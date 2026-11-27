//! Filter › Camera Raw Filter…: a large dialog like Adobe Camera Raw's (preview on the left,
//! edit panels on the right: Light, Color, Effects, Curve, Color Mixer, Color Grading, Detail).
//!
//! The preview runs [`photocraft_algo::camera_raw::develop`] on a CPU proxy of the layer
//! (at least fit-view display resolution within a bounded allocation; `pixel_scale` keeps pixel radii true to the full image), recomputed when a
//! control changes. OK runs `filter.cameraRaw` with the non-default settings, so the result is
//! one history step (or a smart filter on a smart object) and exactly replayable.
//!
//! Control channel: `ui.menu.invoke {"id":"filter.cameraRaw","params":{"ui":{"set":{…},
//! "commit":true | "cancel":true}}}`; the reply describes the dialog.

use crate::theme::palette::ScrollAreaPaletteExt;
use egui::{Align2, Color32, FontId, Pos2, Rect as ERect, Sense, Stroke, TextureHandle, pos2, vec2};
use photocraft_algo::camera_raw::{CameraRaw, Wheel, curve_lut};
use photocraft_algo::histogram::RgbHistogram;

use crate::camera_raw_scope_ui::{MAX_SAMPLERS, ToneZone};
use photocraft_doc::{DocId, LayerId};
use photocraft_geom::Rect;
use serde_json::{Value, json};
use std::sync::mpsc::{Receiver, TryRecvError};

use crate::PhotocraftApp;
use crate::theme::Tokens;
use crate::widgets;

#[path = "camera_raw_color_ui.rs"]
mod color_spaces;
#[path = "camera_raw_frame_ui.rs"]
mod frame;
#[path = "camera_raw_live_ui.rs"]
mod live;
#[path = "camera_raw_local_cache_ui.rs"]
mod local_cache;
#[path = "camera_raw_lut_ui.rs"]
mod lut;
#[path = "camera_raw_mask_ui.rs"]
mod masks;
#[path = "camera_raw_presets_ui.rs"]
mod presets;
#[path = "camera_raw_recognition_ui.rs"]
mod recognition;
#[path = "camera_raw_tabs_ui.rs"]
mod tabs;

/// Camera Raw uses one square check control for mask, preview and scope switches.
pub(crate) fn square_checkbox(ui: &mut egui::Ui, value: &mut bool, label: &str) -> egui::Response {
    masks::square_checkbox(ui, value, label)
}

// A 900 px floor is retained for small windows, but normal/high-DPI fit views must not
// enlarge a tiny proxy after every slider edit. Keep the allocation explicitly bounded.
const PROXY_SIDE: usize = 900;
const PROXY_PIXELS: usize = 4_194_304;
const PANEL_W: f32 = 330.0;
pub(crate) const NATIVE_DIALOG_KEY: &str = "camera-raw";
const BANDS: [&str; 8] = ["Red", "Orange", "Yellow", "Green", "Aqua", "Blue", "Purple", "Magenta"];

/// Presentation only: all four curves remain in CameraRaw and are rendered together.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
enum CurveChannel {
    #[default]
    Rgb,
    Red,
    Green,
    Blue,
}

impl CurveChannel {
    const ALL: [Self; 4] = [Self::Rgb, Self::Red, Self::Green, Self::Blue];

    fn name(self) -> &'static str {
        match self {
            Self::Rgb => "RGB",
            Self::Red => "Red",
            Self::Green => "Green",
            Self::Blue => "Blue",
        }
    }

    fn color(self, t: &Tokens) -> Color32 {
        let name = match self {
            Self::Rgb => None,
            Self::Red => Some("red"),
            Self::Green => Some("green"),
            Self::Blue => Some("blue"),
        };
        name.map(|name| crate::tone::channel_color(name, t)).unwrap_or(t.text)
    }

    fn points(self, p: &CameraRaw) -> &Vec<[f32; 2]> {
        match self {
            Self::Rgb => &p.point_curve,
            Self::Red => &p.point_curve_red,
            Self::Green => &p.point_curve_green,
            Self::Blue => &p.point_curve_blue,
        }
    }

    fn points_mut(self, p: &mut CameraRaw) -> &mut Vec<[f32; 2]> {
        match self {
            Self::Rgb => &mut p.point_curve,
            Self::Red => &mut p.point_curve_red,
            Self::Green => &mut p.point_curve_green,
            Self::Blue => &mut p.point_curve_blue,
        }
    }
}

fn proxy_size(w: usize, h: usize, screen: egui::Vec2, ppp: f32) -> (usize, usize) {
    let baseline = (PROXY_SIDE as f32 / w.max(h).max(1) as f32).min(1.0);
    let display = if screen.is_finite() && ppp.is_finite() && ppp > 0.0 {
        let visible = (screen - vec2(PANEL_W + 64.0, 160.0)).max(vec2(1.0, 1.0)) * ppp;
        (visible.x / w.max(1) as f32).min(visible.y / h.max(1) as f32).min(1.0)
    } else {
        baseline
    };
    let limit = (PROXY_PIXELS as f64 / (w.max(1) as f64 * h.max(1) as f64)).sqrt() as f32;
    let scale = baseline.max(display).min(limit).min(1.0);
    let (mut pw, mut ph) = ((w as f32 * scale).ceil() as usize, (h as f32 * scale).ceil() as usize);
    pw = pw.clamp(1, w.max(1));
    ph = ph.clamp(1, h.max(1));
    // Ceiling the two dimensions may add a row beyond the explicit allocation budget.
    while pw.saturating_mul(ph) > PROXY_PIXELS {
        if pw >= ph {
            pw -= 1;
        } else {
            ph -= 1;
        }
    }
    (pw, ph)
}

fn proxy_bin_start(bin: usize, source: usize, bins: usize) -> usize {
    // A very wide sparse document is valid on wasm32 too; avoid usize multiplication
    // overflow even when the total image/preview allocations are already bounded.
    (bin as u64 * source as u64).div_ceil(bins.max(1) as u64) as usize
}

struct ProxyFrame {
    revision: u64,
    params: CameraRaw,
    pixels: std::sync::Arc<[[f32; 4]]>,
    unmasked: Option<std::sync::Arc<[[f32; 4]]>>,
    image: egui::ColorImage,
    histogram: RgbHistogram,
    histogram_ms: f64,
    render_ms: f64,
    /// Low-resolution computation is display feedback, not a refined proxy.
    live: Option<(live::Source, std::sync::Arc<[[f32; 4]]>)>,
    global_stage: Option<local_cache::GlobalStage>,
    global_cache_hit: bool,
}
type ProxyResult = Result<ProxyFrame, String>;

fn fold_grayscale(pixels: &mut [[f32; 4]]) {
    for pixel in pixels {
        let gray = photocraft_color::convert::rgb_to_gray([pixel[0], pixel[1], pixel[2]]);
        pixel[..3].fill(gray);
    }
}

pub struct CameraRawDialog {
    recognition_source: recognition::Source,
    document: DocId,
    native_close: Option<(DialogAction, u64)>,
    pub layer: LayerId,
    layer_name: String,
    pub params: CameraRaw,
    /// Settings when the dialog opened (stored settings of an edited smart filter).
    original: CameraRaw,
    target: Target,
    curve_state: crate::state::PointCurveState,
    curve_channel: CurveChannel,
    curve_rect: Option<ERect>,
    curve_visible_rect: Option<ERect>,
    /// Proxy pixels (straight RGBA) and size.
    pub(crate) proxy: std::sync::Arc<[[f32; 4]]>,
    pub(crate) selection: Option<std::sync::Arc<[f32]>>,
    pub(crate) pw: usize,
    pub(crate) ph: usize,
    full_w: usize,
    full_h: usize,
    preview_ppp: f32,
    pub(crate) detail: super::camera_raw_detail_ui::DetailPreview,
    pub(crate) viewport: Option<ERect>,
    pub(crate) float: bool,
    grayscale: bool,
    tex: Option<TextureHandle>,
    before_tex: Option<TextureHandle>,
    pub(crate) before_histogram: RgbHistogram,
    pub(crate) processed: std::sync::Arc<[[f32; 4]]>,
    pub(crate) lab_transform: std::sync::Arc<photocraft_cms::Transform>,
    pub(crate) scope_transform: std::sync::Arc<photocraft_cms::Transform>,
    pub(crate) scope: super::camera_raw_scope_ui::ScopeView,
    pub(crate) after_histogram: RgbHistogram,
    pub(crate) preview_revision: u64,
    histogram_ms: f64,
    pub(crate) dirty: bool,
    pub show_before: bool,
    mixer_band: usize,
    pub render_ms: f64,
    /// At most one proxy render is in flight. New slider values replace the requested revision
    /// and are rendered immediately after the current worker finishes. Completed intermediate
    /// frames advance the displayed revision so a sustained drag cannot starve the preview.
    proxy_pending: Option<Receiver<ProxyResult>>,
    proxy_requested: bool,
    global_stage: Option<local_cache::GlobalStage>,
    global_cache_hit: bool,
    live_source: live::Source,
    proxy_reduced: bool,
    pub(crate) requested_revision: u64,
    pub(crate) rendered_params: CameraRaw,
    last_edit_time: f64,
    preview_error: Option<String>,
    masks: masks::State,
    presets: presets::State,
    lut_message: Option<String>,
    source_profile: Result<String, String>,
    color_geometry: color_spaces::Geometry,
    output_options: color_spaces::OutputOptions,
    active_tab: tabs::Tab,
    tab_geometry: tabs::Geometry,
}

#[derive(Clone, Copy)]
enum DialogAction {
    Cancel,
    Apply,
    Output,
}

impl CameraRawDialog {
    pub(crate) fn native_closing(&self) -> bool {
        self.native_close.is_some()
    }
    fn bind_color_source(&self, params: &mut CameraRaw, force: bool) -> Result<(), String> {
        use photocraft_algo::camera_raw_color::RgbSpace;
        if params.color_spaces.working == RgbSpace::Document {
            if force {
                params.color_spaces.source_profile.clear();
            }
        } else if force || params.color_spaces.source_profile.is_empty() {
            params.color_spaces.source_profile = self.source_profile.as_ref().map_err(Clone::clone)?.clone();
        }
        params.color_spaces.validate_depth(self.float).map_err(color_spaces::localize_error)
    }

    fn prepare_color_source(&mut self) -> Result<(), String> {
        use photocraft_algo::camera_raw_color::RgbSpace;
        let force = matches!(self.target, Target::NewFilter);
        let spaces = &mut self.params.color_spaces;
        if spaces.working == RgbSpace::Document {
            if force {
                spaces.source_profile.clear();
            }
        } else if force || spaces.source_profile.is_empty() {
            spaces.source_profile = self.source_profile.as_ref().map_err(Clone::clone)?.clone();
        }
        spaces.validate_depth(self.float).map_err(color_spaces::localize_error)
    }
    fn select_tab(&mut self, tab: tabs::Tab) {
        if self.active_tab != tab {
            masks::end_gesture(&mut self.masks);
            self.active_tab = tab;
            self.curve_rect = None;
            self.curve_visible_rect = None;
        }
    }

    pub fn describe(&self, view: &crate::state::CameraRawScopeState, navigation: &crate::state::CameraRawPreviewState) -> Value {
        let h = self.histogram();
        let mut result = json!({"layer": self.layer.0, "params": serde_json::to_value(&self.params).unwrap_or(Value::Null), "proxy": [self.pw, self.ph],
            "renderMs": self.render_ms, "histogramMs": self.histogram_ms, "previewRevision": self.preview_revision, "before": self.show_before,
            "histogram": {"source": if self.show_before { "before" } else { "after" }, "approximate": true, "size": [self.pw, self.ph],
                "sampleDomain": "preview-rgb", "range": [0, 1], "red": h.channels[0].as_slice(), "green": h.channels[1].as_slice(), "blue": h.channels[2].as_slice(),
                "samples": h.samples, "transparent": h.transparent, "invalid": h.invalid, "underflow": h.underflow, "overflow": h.overflow, "shadows": h.shadows, "highlights": h.highlights}});
        result["smartFilter"] = json!(match self.target {
            Target::SmartFilter(index) => Some(index),
            Target::NewFilter => None,
        });
        result["view"] = json!(navigation);
        result["tab"] = json!(self.active_tab);
        result["tabs"] = self.tab_geometry.describe(self.active_tab);
        result["outputOptions"] = json!({"depth":self.output_options.depth,"resolution":self.output_options.resolution});
        result["sourceSize"] = json!([self.full_w, self.full_h]);
        result["previewComputeBackend"] = json!("cpu");
        result["previewApproximate"] =
            json!(self.proxy_reduced || self.detail.approximate() || !self.detail.ready && (self.pw != self.full_w || self.ph != self.full_h));
        result["proxyReduced"] = json!(self.proxy_reduced);
        result["globalStageCacheHit"] = json!(self.global_cache_hit);
        result["globalStageCacheBytes"] = json!(self.global_stage.as_ref().map_or(0, local_cache::GlobalStage::bytes));
        result["localStageCacheBytes"] = json!(self.global_stage.as_ref().map_or(0, local_cache::GlobalStage::local_bytes));
        result["localPrefixCacheHit"] = json!(self.global_stage.as_ref().is_some_and(local_cache::GlobalStage::prefix_hit));
        result["localCandidateCacheHit"] = json!(self.global_stage.as_ref().is_some_and(local_cache::GlobalStage::candidate_hit));
        result["interactionProxy"] = json!([self.live_source.width, self.live_source.height]);
        result["detailPending"] = json!(self.detail.pending());
        result["detailDisplayedRevision"] = json!(self.detail.displayed_revision());
        result["detailApproximate"] = json!(self.detail.approximate());
        result["interactiveDetailPending"] = json!(self.detail.interactive_pending());
        result["interactiveDetailMs"] = json!(self.detail.interactive_ms());
        result["detailWorkPixels"] = json!(self.detail.work_pixels());
        result["detailError"] = json!(self.detail.error);
        result["requestedRevision"] = json!(self.requested_revision);
        result["previewPending"] =
            json!(self.proxy_pending.is_some() || self.proxy_requested || self.dirty || self.proxy_reduced && self.preview_error.is_none());
        result["previewError"] = json!(self.preview_error);
        result["viewportRect"] = json!(self.viewport.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["zoom"] = json!(self.viewport.map(|r| navigation.scale(r, vec2(self.full_w as f32, self.full_h as f32) / self.preview_ppp)));
        result["scope"] = serde_json::to_value(view).unwrap_or(Value::Null);
        result["hasScopeSelection"] = json!(self.selection.is_some());
        result["vectorscope"] = json!(
            self.scope
                .cache
                .as_ref()
                .map(|(_, before, selected, data)| json!({"samples":data.samples,"before":before,"selectedRegion":selected,"bins":data.counts}))
        );
        result["scopeMs"] = json!(self.scope.ms);
        result["scopeRevision"] = json!(self.scope.revision);
        result["overlayRevision"] = json!(self.scope.overlay_key.map(|k| k.0));
        result["pointerReadout"] = self.scope.pointer_sample.and_then(|p| self.readout(p, view.lab)).unwrap_or(Value::Null);
        result["samplerReadouts"] = json!(view.samplers.iter().take(MAX_SAMPLERS).map(|p| self.readout(*p, view.lab)).collect::<Vec<_>>());
        result["hoveredZone"] = json!(self.scope.hovered_zone);
        result["previewRect"] = json!(self.scope.preview_rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["curveState"] = json!(self.curve_state);
        result["maskEditor"] = self.masks.describe(&self.params.local_adjustments, self.scope.preview_rect);
        result["presetMessage"] = json!(self.presets.message);
        result["curveChannel"] = json!(self.curve_channel);
        result["curveRect"] = json!(self.curve_rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["curveVisibleRect"] = json!(self.curve_visible_rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["hoverSample"] = json!(self.hover_sample(view.vectorscope));
        result["vectorscopeRect"] = json!(self.scope.vectorscope_rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result["scopeRect"] = json!(self.scope.rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]));
        result
    }

    pub(crate) fn histogram(&self) -> &RgbHistogram {
        if self.show_before { &self.before_histogram } else { &self.after_histogram }
    }

    fn detail_allowed(&self, ctx: &egui::Context) -> bool {
        if self.dirty || self.proxy_pending.is_some() || self.proxy_requested || self.proxy_reduced || self.preview_revision != self.requested_revision {
            return false;
        }
        self.interaction_settled(ctx)
    }

    fn interaction_settled(&self, ctx: &egui::Context) -> bool {
        let remaining = 0.2 - (ctx.input(|i| i.time) - self.last_edit_time);
        if remaining > 0.0 {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(remaining.min(0.2)));
            return false;
        }
        !ctx.input(|i| i.pointer.any_down())
    }

    /// Params for the engine: only the settings that differ from the defaults.
    pub fn command_params(&self) -> Value {
        let full = serde_json::to_value(&self.params).unwrap_or(json!({}));
        let def = serde_json::to_value(CameraRaw::default()).unwrap_or(json!({}));
        let mut out = serde_json::Map::new();
        if let (Value::Object(f), Value::Object(d)) = (full, def) {
            for (k, v) in f {
                if k != "pixelScale" && d.get(&k) != Some(&v) {
                    out.insert(k, v);
                }
            }
        }
        Value::Object(out)
    }

    fn image(px: &[[f32; 4]], w: usize, h: usize) -> egui::ColorImage {
        frame::image(px, w, h)
    }

    fn install_proxy(&mut self, ctx: &egui::Context, frame: ProxyFrame) {
        self.global_stage = frame.global_stage;
        self.global_cache_hit = frame.global_cache_hit;
        self.proxy_reduced = frame.live.is_some();
        if let Some((source, processed)) = frame.live {
            self.detail.set_interactive_proxy(frame.revision, source.width, source.height, source.pixels, processed);
        } else {
            self.detail.set_interactive_proxy(
                frame.revision,
                self.pw,
                self.ph,
                self.proxy.clone(),
                frame.unmasked.clone().unwrap_or_else(|| frame.pixels.clone()),
            );
        }
        match &mut self.tex {
            Some(t) => t.set(frame.image, egui::TextureOptions::LINEAR),
            None => self.tex = Some(ctx.load_texture("camera-raw-after", frame.image, egui::TextureOptions::LINEAR)),
        }
        if self.before_tex.is_none() {
            self.before_tex = Some(ctx.load_texture("camera-raw-before", Self::image(&self.proxy, self.pw, self.ph), egui::TextureOptions::LINEAR));
        }
        self.processed = frame.pixels;
        self.after_histogram = frame.histogram;
        self.scope.cache = None;
        self.render_ms = frame.render_ms;
        self.histogram_ms = frame.histogram_ms;
        self.preview_revision = frame.revision;
        self.rendered_params = frame.params;
        self.preview_error = None;
        // Installing a result can also happen after the preview has already been painted.
        ctx.request_repaint();
    }

    pub(crate) fn render(&mut self, ctx: &egui::Context) {
        if let Err(error) = self.prepare_color_source() {
            self.preview_error = Some(error);
            self.dirty = false;
            return;
        }
        let t0 = crate::gpu_canvas::now_ms();
        let mut p = self.params.clone();
        p.pixel_scale = (self.pw as f32 / self.full_w.max(1) as f32).min(1.0);
        let cached = local_cache::render(&self.proxy, self.pw, self.ph, &p, self.float, self.global_stage.as_ref());
        let mut px = cached.pixels;
        if self.grayscale {
            fold_grayscale(&mut px);
        }
        let unmasked = self.selection.as_ref().map(|_| std::sync::Arc::from(px.clone()));
        if let Some(mask) = &self.selection {
            for ((out, original), coverage) in px.iter_mut().zip(self.proxy.iter()).zip(mask.iter()) {
                for (out, original) in out.iter_mut().zip(original) {
                    *out = *original + (*out - *original) * coverage;
                }
            }
        }
        let histogram_start = crate::gpu_canvas::now_ms();
        let histogram = frame::histogram(&px);
        self.histogram_ms = crate::gpu_canvas::now_ms() - histogram_start;
        self.requested_revision = self.requested_revision.saturating_add(1);
        let image = Self::image(&px, self.pw, self.ph);
        self.install_proxy(
            ctx,
            ProxyFrame {
                revision: self.requested_revision,
                params: self.params.clone(),
                pixels: px.into(),
                unmasked,
                image,
                histogram,
                histogram_ms: self.histogram_ms,
                render_ms: crate::gpu_canvas::now_ms() - t0,
                live: None,
                global_stage: cached.global,
                global_cache_hit: cached.hit,
            },
        );
        self.proxy_requested = false;
        self.dirty = false;
    }

    /// Queue the newest settings without blocking the egui event loop. This is deliberately
    /// latest-wins: a long slider drag may produce dozens of values, but only the current one is
    /// useful to the artist.
    pub(crate) fn queue_render(&mut self, ctx: &egui::Context) {
        if let Err(error) = self.prepare_color_source() {
            self.preview_error = Some(error);
            self.dirty = false;
            return;
        }
        self.requested_revision = self.requested_revision.saturating_add(1);
        self.last_edit_time = ctx.input(|i| i.time);
        self.preview_error = None;
        // Keep one exact worker instead of cancelling and spawning full-image work on every
        // edit. The live display follows completed proxy revisions, independently of refinement.
        self.detail.invalidate();
        if super::camera_raw_detail_ui::local_preview(&self.params) {
            self.detail.prefer_local();
        }
        self.proxy_requested = true;
        self.dirty = false;
        if self.proxy_pending.is_none() {
            self.start_proxy_worker(ctx);
        }
        ctx.request_repaint();
    }

    fn start_proxy_worker(&mut self, ctx: &egui::Context) {
        if !self.proxy_requested || self.proxy_pending.is_some() {
            return;
        }
        self.proxy_requested = false;
        let revision = self.requested_revision;
        let original = self.proxy.clone();
        let live = (!self.params.is_identity()
            && live::should_reduce(&self.params)
            && !self.interaction_settled(ctx)
            && self.live_source.pixels.len() < original.len())
        .then(|| self.live_source.clone());
        let params = self.params.clone();
        let global_stage = self.global_stage.clone();
        let selection = self.selection.clone();
        let float = self.float;
        let grayscale = self.grayscale;
        let (tx, rx) = std::sync::mpsc::channel();
        let repaint = ctx.clone();
        let w = self.pw;
        let h = self.ph;
        let full_w = self.full_w;
        let handle = move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let t0 = std::time::Instant::now();
                let (work_w, work_h) = live.as_ref().map_or((w, h), |source| (source.width, source.height));
                let mut px = live.as_ref().map_or_else(Vec::new, |source| source.pixels.to_vec());
                let mut p = params.clone();
                p.pixel_scale = (work_w as f32 / full_w.max(1) as f32).min(1.0);
                let (global_stage, global_cache_hit) = if live.is_none() {
                    let cached = local_cache::render(&original, work_w, work_h, &p, float, global_stage.as_ref());
                    px = cached.pixels;
                    (cached.global, cached.hit)
                } else {
                    photocraft_algo::camera_raw::develop(&mut px, work_w, work_h, &p, float);
                    (None, false)
                };
                if grayscale {
                    fold_grayscale(&mut px);
                }
                let live = live.map(|source| {
                    let processed: std::sync::Arc<[[f32; 4]]> = std::mem::take(&mut px).into();
                    px = source.expand_preserving_detail(&processed, &original, w, h);
                    // Filtering changes colour, never alpha. Keep full-proxy coverage,
                    // including thin/transparent edges, rather than expanded alpha.
                    for (pixel, before) in px.iter_mut().zip(original.iter()) {
                        pixel[3] = before[3];
                        if !float {
                            for value in &mut pixel[..3] {
                                *value = value.clamp(0.0, 1.0);
                            }
                        }
                    }
                    (source, processed)
                });
                // The live frame already carries its unmasked low-resolution output.
                // Copying another display-sized unmasked surface would never be used.
                let unmasked = (selection.is_some() && live.is_none()).then(|| std::sync::Arc::from(px.clone()));
                if let Some(mask) = selection {
                    for ((out, original), coverage) in px.iter_mut().zip(original.iter()).zip(mask.iter()) {
                        for (out, original) in out.iter_mut().zip(original) {
                            *out = *original + (*out - *original) * coverage;
                        }
                    }
                }
                let histogram_start = std::time::Instant::now();
                let histogram = frame::histogram(&px);
                let histogram_ms = histogram_start.elapsed().as_secs_f64() * 1000.0;
                let image = Self::image(&px, w, h);
                let elapsed = t0.elapsed().as_secs_f64() * 1000.0;
                ProxyFrame {
                    revision,
                    params,
                    pixels: px.into(),
                    unmasked,
                    image,
                    histogram,
                    histogram_ms,
                    render_ms: elapsed,
                    live,
                    global_stage,
                    global_cache_hit,
                }
            }))
            .map_err(|_| "Camera Raw preview worker failed".to_owned());
            let _ = tx.send(result);
            repaint.request_repaint_of(egui::ViewportId::ROOT);
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            match std::thread::Builder::new().name("camera-raw-proxy".into()).spawn(handle) {
                Ok(_) => self.proxy_pending = Some(rx),
                Err(e) => self.preview_error = Some(format!("Cannot start Camera Raw preview: {e}")),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            drop(handle);
            drop(rx);
            self.render(ctx);
        }
    }

    pub(crate) fn poll_render(&mut self, ctx: &egui::Context) {
        // A refinement replaces the reduced frame at the *same* settings revision.
        // No new input is needed to restore the original display-proxy resolution.
        if self.proxy_reduced && self.preview_error.is_none() && !self.proxy_requested && self.proxy_pending.is_none() && self.interaction_settled(ctx) {
            self.proxy_requested = true;
        }
        let Some(rx) = self.proxy_pending.take() else {
            if self.proxy_requested {
                self.start_proxy_worker(ctx);
            }
            return;
        };
        match rx.try_recv() {
            Ok(Ok(frame)) => {
                if (frame.revision > self.preview_revision
                    || frame.revision == self.preview_revision && frame.revision == self.requested_revision && self.proxy_reduced && frame.live.is_none())
                    && frame.revision <= self.requested_revision
                    && !self.dirty
                {
                    self.install_proxy(ctx, frame);
                }
                if self.proxy_requested {
                    self.start_proxy_worker(ctx);
                }
            }
            Err(TryRecvError::Empty) => {
                self.proxy_pending = Some(rx);
                // Watchdog for missed/coalesced native wakeups after the last pointer event.
                ctx.request_repaint_after(std::time::Duration::from_millis(16));
            }
            Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                if self.proxy_requested {
                    self.start_proxy_worker(ctx);
                } else {
                    self.preview_error = Some("Camera Raw preview worker stopped; change a setting to retry".into());
                }
            }
        }
    }
}

/// Opens the dialog on the active layer.
/// Where OK writes: a new filter on the layer, or an existing smart filter (by index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    NewFilter,
    SmartFilter(usize),
}

/// What limits the preview: the document selection (a new filter applies through it, as the
/// engine does), a smart object's filter mask, or nothing.
enum Coverage {
    Selection,
    Mask(photocraft_doc::LayerMask),
    None,
}

pub fn open(app: &mut PhotocraftApp, ctx: &egui::Context) -> Result<(), String> {
    photocraft_engine::commands::find("filter.cameraRaw").map(|c| (c.enabled)(&app.session)).unwrap_or(Err("unknown command".into()))?;
    let (layer, surf, _) = crate::distort_ui::active_pixels(app)?;
    let mut params = CameraRaw::default();
    // Output defaults must not change the document-domain preview/working space.
    params.color_spaces.output = color_spaces::default_output_space(&app.session.prefs().raw_defaults);
    open_pixels(app, ctx, layer, surf, params, Coverage::Selection, Target::NewFilter)
}

/// Opens Camera Raw on smart filter `index` of smart layer `layer`, with its stored settings and
/// the pixels below it; OK updates that filter (`layer.smartFilter.setParams`), as double-clicking
/// a Camera Raw smart filter does in Photoshop.
pub fn open_smart_filter(app: &mut PhotocraftApp, ctx: &egui::Context, layer: LayerId, index: usize) -> Result<(), String> {
    use photocraft_engine::lens_cmds::{RAW, raw_params};
    if crate::ps_collaboration_ui::intercept_editor(app, layer, index)? {
        ctx.request_repaint();
        return Ok(());
    }
    let st = app.session.active().ok_or("no document")?;
    let Some(photocraft_doc::LayerContent::Smart(sm)) = st.doc.layer(layer).map(|l| &l.content) else {
        return Err("the layer is not a smart object".into());
    };
    let filter = sm.smart_filters.get(index).ok_or("no such smart filter")?;
    if filter.command != RAW {
        return Err("the smart filter is not a Camera Raw filter".into());
    }
    let params = raw_params(RAW, &filter.params).map_err(|e| e.to_string())?;
    let surf = photocraft_engine::smart_cmds::render_below_filter(&st.doc, sm, index)
        .map_err(|e| e.to_string())?
        .ok_or("the smart object's contents are unavailable (missing linked file?)")?;
    let coverage = sm.filter_mask.clone().filter(|m| m.enabled).map_or(Coverage::None, Coverage::Mask);
    open_pixels(app, ctx, layer, surf, params, coverage, Target::SmartFilter(index))
}

fn open_pixels(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    layer: LayerId,
    surf: photocraft_raster::Surface,
    mut params: CameraRaw,
    coverage: Coverage,
    target: Target,
) -> Result<(), String> {
    let st = app.session.active().ok_or("no document")?;
    let source_profile = photocraft_algo::camera_raw_color::encode_source(&photocraft_engine::color_cmds::document_profile(&st.doc));
    photocraft_engine::lens_cmds::color::bind_source(&mut params, &st.doc, matches!(target, Target::NewFilter))
        .map_err(|e| color_spaces::localize_error(e.to_string()))?;
    let canvas = st.doc.bounds();
    let name = st.doc.layer(layer).map(|l| l.name.clone()).unwrap_or_default();
    // Use the engine filter domain, including pixels outside the canvas: spatial Camera Raw
    // stages (vignette, grain, local contrast) must agree with the committed result.
    let area = canvas.union(&surf.content_bounds());
    let width = area.x1.checked_sub(area.x0).filter(|n| (1..=1_048_576).contains(n));
    let height = area.y1.checked_sub(area.y0).filter(|n| (1..=1_048_576).contains(n));
    let (Some(w), Some(h)) = (width, height) else {
        return Err("Camera Raw preview bounds are too large".into());
    };
    let (w, h) = (w as usize, h as usize);
    if w.checked_mul(h).is_none_or(|pixels| pixels > 512_000_000) {
        return Err("Camera Raw preview bounds are too large".into());
    }
    let (pw, ph) = proxy_size(w, h, ctx.content_rect().size(), ctx.pixels_per_point());
    let mut proxy = vec![[0.0f32; 4]; pw * ph];
    let mut recognition_source = recognition::Builder::new(w, h);
    let mut row = vec![[0.0f32; 4]; w];
    for py in 0..ph {
        // Average complete source bins at the requested display size. Integer k×k
        // steps can jump from 2.7 MP to 6 MP for a small DPI change; variable bins
        // cover the screen accurately without that memory/processing jump.
        let mut acc = vec![[0.0f32; 5]; pw];
        for sy in proxy_bin_start(py, h, ph)..proxy_bin_start(py + 1, h, ph) {
            let y = area.y0 + sy as i32;
            surf.read_rgba_into(Rect::new(area.x0, y, area.x1, y + 1), &mut row);
            recognition_source.add_row(&row, sy);
            for (x, q) in row.iter().enumerate() {
                let a = &mut acc[(x as u64 * pw as u64 / w as u64) as usize];
                for c in 0..4 {
                    a[c] += q[c];
                }
                a[4] += 1.0;
            }
        }
        for (x, a) in acc.iter().enumerate() {
            proxy[py * pw + x] = [a[0] / a[4], a[1] / a[4], a[2] / a[4], a[3] / a[4]];
        }
    }
    // Coverage at the centre of each source bin used above.
    let sample = |value: &dyn Fn(i32, i32) -> f32| {
        (0..ph)
            .flat_map(|y| {
                (0..pw).map(move |x| {
                    (
                        area.x0 + ((proxy_bin_start(x, w, pw) + proxy_bin_start(x + 1, w, pw)) / 2).min(w - 1) as i32,
                        area.y0 + ((proxy_bin_start(y, h, ph) + proxy_bin_start(y + 1, h, ph)) / 2).min(h - 1) as i32,
                    )
                })
            })
            .map(|(x, y)| value(x, y).clamp(0.0, 1.0))
            .collect::<Vec<_>>()
    };
    let selection = match &coverage {
        Coverage::Selection => st.doc.selection.as_ref().map(|mask| sample(&|x, y| mask.sample_channel(x, y, 0))),
        Coverage::Mask(mask) => Some(sample(&|x, y| mask.value(x, y))),
        Coverage::None => None,
    };
    let source = photocraft_engine::color_cmds::composite_profile(&st.doc);
    let lab_transform = photocraft_cms::cached(&source, photocraft_cms::Builtin::LabD50.profile(), Default::default()).map_err(|e| e.to_string())?;
    let scope_transform = photocraft_cms::cached(&source, photocraft_cms::Builtin::Srgb.profile(), Default::default()).map_err(|e| e.to_string())?;
    if let Some(saved) = app.session.prefs().dialogs.get("filter.cameraRaw.scope")
        && saved.get("samplers").and_then(Value::as_array).is_none_or(|a| a.len() <= MAX_SAMPLERS)
        && let Ok(view) = serde_json::from_value(saved.clone())
    {
        app.ui.camera_raw_scope = view;
    }
    app.ui.camera_raw_preview = crate::state::CameraRawPreviewState {
        comparison: app.ui.camera_raw_preview.comparison,
        swapped: app.ui.camera_raw_preview.swapped,
        ..Default::default()
    };
    app.ui.camera_raw_scope.samplers.clear();
    // Keep Camera Raw compact on open; vectorscope is an opt-in context-menu view.
    app.ui.camera_raw_scope.vectorscope = false;
    if selection.is_none() {
        app.ui.camera_raw_scope.selected_region = false;
    }
    let float = surf.format().sample == photocraft_color::SampleType::F32;
    params.color_spaces.validate_depth(float).map_err(color_spaces::localize_error)?;
    let grayscale = surf.format().mode == photocraft_color::ColorMode::Grayscale;
    let proxy: std::sync::Arc<[[f32; 4]]> = proxy.into();
    let live_source = live::Source::new(proxy.clone(), pw, ph);
    let mut d = CameraRawDialog {
        recognition_source: recognition_source.finish(),
        document: st.doc.id,
        native_close: None,
        layer,
        layer_name: name,
        original: params.clone(),
        params,
        target,
        before_histogram: frame::histogram(&proxy),
        processed: Vec::new().into(),
        lab_transform,
        scope_transform,
        scope: Default::default(),
        after_histogram: RgbHistogram::default(),
        preview_revision: 0,
        histogram_ms: 0.0,
        proxy,
        selection: selection.map(std::sync::Arc::from),
        pw,
        ph,
        full_w: w,
        full_h: h,
        preview_ppp: ctx.pixels_per_point(),
        detail: super::camera_raw_detail_ui::DetailPreview::new(
            surf,
            area,
            match coverage {
                Coverage::Selection => {
                    st.doc.selection.clone().map(super::camera_raw_detail_ui::Coverage::Selection).unwrap_or(super::camera_raw_detail_ui::Coverage::None)
                }
                Coverage::Mask(mask) => super::camera_raw_detail_ui::Coverage::Mask(mask),
                Coverage::None => super::camera_raw_detail_ui::Coverage::None,
            },
        ),
        viewport: None,
        float,
        grayscale,
        tex: None,
        before_tex: None,
        dirty: true,
        show_before: false,
        mixer_band: 0,
        curve_state: Default::default(),
        curve_channel: Default::default(),
        curve_rect: None,
        curve_visible_rect: None,
        render_ms: 0.0,
        proxy_pending: None,
        proxy_requested: false,
        global_stage: None,
        global_cache_hit: false,
        live_source,
        proxy_reduced: false,
        requested_revision: 0,
        rendered_params: CameraRaw::default(),
        last_edit_time: ctx.input(|i| i.time),
        preview_error: None,
        masks: Default::default(),
        presets: Default::default(),
        lut_message: None,
        source_profile,
        color_geometry: Default::default(),
        output_options: color_spaces::OutputOptions::from_prefs(&app.session.prefs().raw_defaults),
        active_tab: Default::default(),
        tab_geometry: Default::default(),
    };
    d.render(ctx);
    app.camera_raw = Some(d);
    Ok(())
}

/// Menu / control-channel entry point. `None` when the call isn't for this dialog.
pub fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id != "filter.cameraRaw" {
        return None;
    }
    let Some(fields) = params.as_object() else {
        return Some(Err("Camera Raw params must be an object".into()));
    };
    if fields.is_empty() {
        return Some(
            open(app, ctx).map(|_| app.camera_raw.as_ref().map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)).unwrap_or(Value::Null)),
        );
    }
    if let Some(sf) = params.get("smartFilter") {
        let opened = (|| {
            if fields.len() > 1 {
                return Err("smartFilter opens the dialog; send ui requests separately".to_string());
            }
            let layer = sf.get("layer").and_then(Value::as_u64).ok_or("smartFilter.layer must be a layer id")?;
            let index = sf.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).ok_or("smartFilter.index must be a filter index")?;
            open_smart_filter(app, ctx, LayerId(layer), index)?;
            Ok(app.camera_raw.as_ref().map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)).unwrap_or(Value::Null))
        })();
        return Some(opened);
    }
    let ui = params.get("ui")?;
    let Some(fields) = ui.as_object() else {
        return Some(Err("Camera Raw ui must be an object".into()));
    };
    for (key, value) in fields {
        match key.as_str() {
            "set" | "scope" | "view" | "preset" | "maskEditor" | "lut" => {}
            "tab" => {
                if serde_json::from_value::<tabs::Tab>(value.clone()).is_err() {
                    return Some(Err("Camera Raw tab must be edit, presets, masks, colorSpaces or colorLookup".into()));
                }
            }
            "curveChannel" => {
                if serde_json::from_value::<CurveChannel>(value.clone()).is_err() {
                    return Some(Err("Camera Raw curveChannel must be rgb, red, green or blue".into()));
                }
            }
            "before" | "commit" | "cancel" | "async" | "createOutput" if value.is_boolean() => {}
            "before" | "commit" | "cancel" | "async" | "createOutput" => return Some(Err(format!("Camera Raw {key} must be a boolean"))),
            _ => return Some(Err(format!("unknown Camera Raw ui property {key}"))),
        }
    }
    if ui.get("commit").and_then(Value::as_bool) == Some(true) && ui.get("cancel").and_then(Value::as_bool) == Some(true) {
        return Some(Err("Camera Raw cannot commit and cancel together".into()));
    }
    if ui.get("createOutput").and_then(Value::as_bool) == Some(true)
        && (ui.get("commit").and_then(Value::as_bool) == Some(true) || ui.get("cancel").and_then(Value::as_bool) == Some(true))
    {
        return Some(Err("Create output cannot commit or cancel the original".into()));
    }
    let was_closed = app.camera_raw.is_none();
    let previous_view = app.ui.camera_raw_scope.clone();
    let previous_navigation = app.ui.camera_raw_preview;
    if was_closed && let Err(e) = open(app, ctx) {
        return Some(Err(e));
    }
    let result = menu_update(app, ctx, ui);
    if was_closed && result.as_ref().is_some_and(Result::is_err) {
        app.camera_raw = None;
        app.ui.camera_raw_scope = previous_view;
        app.ui.camera_raw_preview = previous_navigation;
    }
    result
}

fn execute_mask_operation(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    operation: &photocraft_engine::camera_raw_mask_cmds::MaskOperation,
) -> Result<(), String> {
    let d = app.camera_raw.as_ref().ok_or("Camera Raw dialog is closed")?;
    if d.masks.generating() {
        return Err("Finish automatic mask generation before editing the mask list".into());
    }
    let request = json!({"items": d.params.local_adjustments, "operation": operation});
    let response = app.run(photocraft_engine::camera_raw_mask_cmds::EDIT, request)?;
    let result: photocraft_engine::camera_raw_mask_cmds::EditResult = serde_json::from_value(response).map_err(|error| error.to_string())?;
    let d = app.camera_raw.as_mut().ok_or("Camera Raw dialog is closed")?;
    d.dirty |= d.masks.install_result(&mut d.params.local_adjustments, operation, result);
    if d.dirty {
        d.queue_render(ctx);
    }
    ctx.request_repaint();
    Ok(())
}

fn menu_update(app: &mut PhotocraftApp, ctx: &egui::Context, ui: &Value) -> Option<Result<Value, String>> {
    if let Some(request) = ui.get("lut") {
        if ui.as_object().is_none_or(|object| object.len() != 1) {
            return Some(Err("LUT import must be a separate request".into()));
        }
        return Some(lut::execute(app, ctx, Some(request)));
    }
    if let Some(kind) = ui.get("maskEditor").and_then(|editor| editor.get("create")) {
        if ui.as_object().is_none_or(|object| object.len() != 1) || ui.get("maskEditor").and_then(Value::as_object).is_none_or(|object| object.len() != 1) {
            return Some(Err("Mask creation must be sent separately from other dialog settings".into()));
        }
        let Some(kind) = kind.as_str() else { return Some(Err("create must be linear or radial".into())) };
        let d = app.camera_raw.as_mut()?;
        if let Err(error) = d.masks.create_gradient(&mut d.params.local_adjustments, kind) {
            return Some(Err(error));
        }
        // Creating the draft already ends the previous gesture. Select its tab
        // directly so select_tab's cancellation does not discard the new draft.
        d.active_tab = tabs::Tab::Masks;
        d.curve_rect = None;
        d.curve_visible_rect = None;
        ctx.request_repaint();
        return Some(Ok(d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)));
    }
    if let Some(operation) = ui.get("maskEditor").and_then(|editor| editor.get("action")) {
        if ui.as_object().is_none_or(|object| object.len() != 1) || ui.get("maskEditor").and_then(Value::as_object).is_none_or(|object| object.len() != 1) {
            return Some(Err("Mask actions must be sent separately from other dialog settings".into()));
        }
        let operation = match photocraft_engine::camera_raw_mask_cmds::parse_operation(operation.clone()) {
            Ok(operation) => operation,
            Err(error) => return Some(Err(format!("Invalid mask action: {error}"))),
        };
        if let Err(error) = execute_mask_operation(app, ctx, &operation) {
            return Some(Err(error));
        }
        return Some(Ok(app.camera_raw.as_ref()?.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)));
    }
    if let Some(preset) = ui.get("preset") {
        if ui.as_object().is_none_or(|o| o.len() != 1) {
            return Some(Err("Preset actions must be sent separately from filter changes".into()));
        }
        let name = preset.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
        let Some(object) = preset.as_object() else {
            return Some(Err("preset must be an object".into()));
        };
        let importing = preset.get("action").and_then(Value::as_str) == Some("import");
        if object.keys().any(|k| k != "action" && k != if importing { "data" } else { "name" }) {
            return Some(Err("Unknown or mixed preset action property".into()));
        }
        let action = match preset.get("action").and_then(Value::as_str) {
            Some("save") => presets::Action::Save(name),
            Some("load") => presets::Action::Load(name),
            Some("delete") => presets::Action::Delete(name),
            Some("export") => presets::Action::ExportData(name),
            Some("import") => presets::Action::ImportData(preset.get("data").cloned().unwrap_or(Value::Null)),
            _ => return Some(Err("Preset action must be save, load, delete, export or import".into())),
        };
        return Some(presets::execute(app, ctx, action));
    }
    if let Some(editor) = ui.get("maskEditor")
        && (editor.get("generate").is_some() || editor.get("cancelGeneration").is_some())
    {
        if ui.as_object().is_none_or(|o| o.len() != 1) || editor.as_object().is_none_or(|o| o.len() != 1) {
            return Some(Err("Mask generation actions must be sent separately".into()));
        }
        let subject_quality = app.session.prefs().performance.subject_recognition_quality;
        let d = app.camera_raw.as_mut()?;
        if let Some(kind) = editor.get("generate") {
            let Some(kind) = kind.as_str() else {
                return Some(Err("generate must be a string".into()));
            };
            if d.params.local_adjustments.len() >= photocraft_algo::camera_raw_masks::MAX_MASKS {
                return Some(Err("Mask limit reached".into()));
            }
            if let Err(error) = d.masks.generate(kind) {
                return Some(Err(error));
            }
            d.dirty |= masks::poll_and_start(
                ctx,
                &mut d.masks,
                &mut d.params.local_adjustments,
                &d.recognition_source.pixels,
                d.recognition_source.width,
                d.recognition_source.height,
                masks::RecognitionOptions { transform: Some(&d.scope_transform), quality: subject_quality },
            );
        } else {
            if editor.get("cancelGeneration").and_then(Value::as_bool) != Some(true) {
                return Some(Err("cancelGeneration must be true".into()));
            }
            d.masks.cancel_generation();
        }
        return Some(Ok(d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)));
    }
    // Stage both filter and presentation updates. No texture, preference or dialog mutation
    // occurs until every part of the request has passed validation.
    let d = app.camera_raw.as_ref()?;
    let next_tab = ui.get("tab").map(|value| serde_json::from_value::<tabs::Tab>(value.clone())).transpose();
    let next_tab = match next_tab {
        Ok(tab) => tab,
        Err(_) => return Some(Err("Camera Raw tab must be edit, presets, masks, colorSpaces or colorLookup".into())),
    };
    let next_channel = match ui.get("curveChannel") {
        Some(value) => match serde_json::from_value::<CurveChannel>(value.clone()) {
            Ok(channel) => channel,
            Err(e) => return Some(Err(format!("bad Camera Raw curve channel: {e}"))),
        },
        None => d.curve_channel,
    };
    let mut next_params = d.params.clone();
    if let Some(set) = ui.get("set") {
        if !set.is_object() {
            return Some(Err("bad Camera Raw settings: expected an object".into()));
        }
        let mut cur = serde_json::to_value(&d.params).unwrap_or(json!({}));
        if let (Value::Object(c), Value::Object(s)) = (&mut cur, set) {
            for (k, v) in s {
                if !c.contains_key(k) {
                    return Some(Err(format!("unknown Camera Raw setting {k}")));
                }
                c.insert(k.clone(), v.clone());
            }
        }
        next_params = match serde_json::from_value::<CameraRaw>(cur) {
            Ok(p) => p,
            Err(e) => return Some(Err(format!("bad Camera Raw settings: {e}"))),
        };
        if let Err(e) = d.bind_color_source(&mut next_params, matches!(d.target, Target::NewFilter)) {
            return Some(Err(e));
        }
        if let Err(e) = next_params.validate_changes(&d.original) {
            return Some(Err(format!("bad Camera Raw settings: {e}")));
        }
    }
    let scope_update = match ui.get("scope") {
        Some(scope) => match super::camera_raw_scope_ui::prepare_control(&next_params, d.selection.is_some(), &app.ui.camera_raw_scope, scope) {
            Ok(update) => Some(update),
            Err(e) => return Some(Err(e)),
        },
        None => None,
    };
    let navigation = match ui.get("view") {
        Some(view) => match super::camera_raw_preview_ui::prepare(app.ui.camera_raw_preview, view) {
            Ok(next) => Some(next),
            Err(e) => return Some(Err(e)),
        },
        None => None,
    };
    let mask_editor = if let Some(editor) = ui.get("maskEditor") {
        let Some(object) = editor.as_object() else {
            return Some(Err("maskEditor must be an object".into()));
        };
        if object.keys().any(|k| !matches!(k.as_str(), "selected" | "selectedComponent" | "editing" | "overlay")) {
            return Some(Err("Unknown maskEditor setting".into()));
        }
        let selected = match editor.get("selected") {
            Some(Value::Null) => None,
            Some(v) => match v.as_u64().and_then(|n| usize::try_from(n).ok()).filter(|n| *n < next_params.local_adjustments.len()) {
                Some(n) => Some(n),
                None => return Some(Err("Mask index out of range".into())),
            },
            None => d.masks.selected,
        };
        let selected_component = match editor.get("selectedComponent") {
            Some(Value::Null) => None,
            Some(value) => match value
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| selected.and_then(|group| next_params.local_adjustments.get(group)).is_some_and(|a| *n < a.components.len()))
            {
                Some(index) => Some(index),
                None => return Some(Err("Mask component index out of range".into())),
            },
            None if selected != d.masks.selected => None,
            None => d.masks.selected_component,
        };
        let mut flags = [d.masks.editing_after_cancel(), d.masks.overlay];
        for (i, key) in ["editing", "overlay"].iter().enumerate() {
            if let Some(value) = editor.get(key) {
                let Some(value) = value.as_bool() else {
                    return Some(Err(format!("{key} must be boolean")));
                };
                flags[i] = value;
            }
        }
        Some((selected, selected_component, flags))
    } else {
        None
    };
    let d = app.camera_raw.as_mut()?;
    if let Some(tab) = next_tab {
        d.select_tab(tab);
    }
    if let Some((selected, selected_component, flags)) = mask_editor {
        d.masks.reset_geometry();
        d.masks.selected = selected;
        d.masks.selected_component = selected_component;
        d.masks.edit = flags[0];
        d.masks.overlay = flags[1];
    }
    if let Some(view) = navigation {
        app.ui.camera_raw_preview = view;
    }
    if d.curve_channel != next_channel || next_channel.points(&d.params) != next_channel.points(&next_params) {
        d.curve_state = Default::default();
    }
    d.curve_channel = next_channel;
    if d.params.local_adjustments != next_params.local_adjustments {
        d.masks.reset_geometry();
    }
    d.dirty |= d.params != next_params;
    d.params = next_params;
    if d.masks.selected.is_some_and(|i| i >= d.params.local_adjustments.len()) {
        d.masks.selected = None;
        d.masks.selected_component = None;
    } else if d
        .masks
        .selected_component
        .is_some_and(|component| d.masks.selected.and_then(|group| d.params.local_adjustments.get(group)).is_none_or(|a| component >= a.components.len()))
    {
        d.masks.selected_component = None;
    }
    if let Some(b) = ui.get("before").and_then(Value::as_bool) {
        d.show_before = b;
    }
    if let Some(update) = scope_update {
        update.apply(d, &mut app.ui.camera_raw_scope);
    }
    if d.dirty {
        if ui.get("async").and_then(Value::as_bool) == Some(true) {
            d.queue_render(ctx);
        } else {
            // Preserve the control channel's synchronous snapshot contract.
            d.render(ctx);
        }
    }
    if app.ui.camera_raw_scope.vectorscope
        && let Some(d) = app.camera_raw.as_mut()
    {
        d.ensure_scope(app.ui.camera_raw_scope.selected_region);
    }
    super::camera_raw_scope_ui::persist(app, ctx);
    if ui.get("cancel").and_then(Value::as_bool) == Some(true) {
        app.camera_raw = None;
        return Some(Ok(json!({"cancelled": true})));
    }
    if ui.get("commit").and_then(Value::as_bool) == Some(true) {
        return Some(commit(app));
    }
    if ui.get("createOutput").and_then(Value::as_bool) == Some(true) {
        return Some(create_output(app));
    }
    Some(Ok(app.camera_raw.as_ref().map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)).unwrap_or(Value::Null)))
}

fn commit(app: &mut PhotocraftApp) -> Result<Value, String> {
    let d = app.camera_raw.as_ref().ok_or(tl!("Camera Raw isn't open"))?;
    let result = match d.target {
        Target::NewFilter => {
            let mut p = d.command_params();
            p["layer"] = json!(d.layer.0);
            app.run("filter.cameraRaw", p)?
        }
        Target::SmartFilter(index) => {
            d.params.validate_changes(&d.original)?;
            // Every setting, not just the non-defaults: setParams merges into the stored ones,
            // so a control reset to its default must overwrite the old value.
            let mut params = serde_json::to_value(&d.params).map_err(|e| e.to_string())?;
            if let Value::Object(m) = &mut params {
                m.remove("pixelScale");
            }
            app.run("layer.smartFilter.setParams", json!({"layer": d.layer.0, "index": index, "params": params}))?
        }
    };
    app.camera_raw = None;
    Ok(result)
}

fn create_output(app: &mut PhotocraftApp) -> Result<Value, String> {
    let d = app.camera_raw.as_ref().ok_or(tl!("Camera Raw isn't open"))?;
    let mut params = d.params.clone();
    d.bind_color_source(&mut params, matches!(d.target, Target::NewFilter))?;
    let mut request = json!({"layer":d.layer.0,"settings":params,"outputDepth":d.output_options.depth,"resolution":d.output_options.resolution});
    if let Target::SmartFilter(index) = d.target {
        request["smartFilter"] = json!(index);
    }
    let output = app.run(photocraft_engine::lens_cmds::color::OUTPUT, request).map_err(color_spaces::localize_error)?;
    app.camera_raw = None;
    app.sync_views();
    Ok(output)
}

fn temp_stops() -> Vec<Color32> {
    vec![Color32::from_rgb(70, 120, 230), Color32::from_rgb(200, 200, 200), Color32::from_rgb(235, 200, 60)]
}
fn tint_stops() -> Vec<Color32> {
    vec![Color32::from_rgb(70, 190, 80), Color32::from_rgb(200, 200, 200), Color32::from_rgb(210, 80, 200)]
}

/// One labelled slider; marks the dialog dirty when it moves.
fn row(ui: &mut egui::Ui, dirty: &mut bool, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, grad: Option<&[Color32]>) {
    let _ = slider(ui, dirty, label, v, range, grad);
}

fn slider(ui: &mut egui::Ui, dirty: &mut bool, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>, grad: Option<&[Color32]>) -> egui::Response {
    let reset = 0.0f32.clamp(*range.start(), *range.end());
    let r = widgets::slider_row(ui, crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", label), v, range, "", grad);
    if r.changed() {
        *dirty = true;
    }
    if r.double_clicked() {
        *v = reset;
        *dirty = true;
    }
    r
}

/// A Light tone slider; Alt-dragging it shows that zone's channel clipping on the preview.
fn tone_row(ui: &mut egui::Ui, dirty: &mut bool, alt: &mut Option<ToneZone>, zone: ToneZone, v: &mut f32) {
    let r = slider(ui, dirty, zone.name(), v, -zone.limit()..=zone.limit(), None);
    if r.dragged() && ui.input(|i| i.modifiers.alt) {
        *alt = Some(zone);
    }
}

fn section(ui: &mut egui::Ui, title: &str, open: bool, body: impl FnOnce(&mut egui::Ui)) {
    // Use the same theme border as editable fields: the former header-only layout
    // had no visible boundary at all, especially against a custom light dock.
    let tokens = Tokens::get(ui.ctx());
    let start = ui.next_widget_position();
    ui.painter().line_segment([start, start + vec2(ui.available_width(), 0.0)], Stroke::new(1.0, tokens.field_border));
    ui.add_space(5.0);
    egui::CollapsingHeader::new(egui::RichText::new(crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", title)).font(FontId::proportional(13.0)))
        .id_salt(("camera-raw-section", title))
        .default_open(open)
        .show(ui, body);
}

fn subheading(ui: &mut egui::Ui, title: &str) {
    let tokens = Tokens::get(ui.ctx());
    ui.add_space(3.0);
    ui.label(egui::RichText::new(title).font(crate::theme::semibold(13.0)).color(tokens.text));
}

/// The active band is dialog state; choosing it never changes the filter or preview.
fn color_mixer(ui: &mut egui::Ui, p: &mut CameraRaw, band: &mut usize, dirty: &mut bool) {
    let tokens = Tokens::get(ui.ctx());
    let hues = &photocraft_algo::camera_raw::HSL_BANDS;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (index, source) in BANDS.iter().enumerate() {
            let name = crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", source);
            let (rect, response) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
            if response.clicked() {
                *band = index;
            }
            let selected = *band == index;
            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::RadioButton, ui.is_enabled(), selected, name));
            let center = rect.center();
            let color = hue_color(hues[index]);
            if selected {
                ui.painter().circle_filled(center, 8.0, color);
                ui.painter().circle_filled(center, 2.7, Color32::WHITE);
                ui.painter().circle_stroke(center, 2.7, Stroke::new(0.8, Color32::from_black_alpha(150)));
            } else {
                ui.painter().circle_stroke(center, 7.0, Stroke::new(2.2, color));
            }
            if response.hovered() || response.has_focus() {
                ui.painter().circle_stroke(center, 10.0, Stroke::new(1.0, tokens.text));
            }
            response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(name);
        }
    });
    let index = *band;
    let hue = hues[index];
    let base = hue_color(hue);
    let hue_track = [hue_color(hue - 30.0), base, hue_color(hue + 30.0)];
    let saturation_track = [Color32::from_gray(128), base];
    let luminance_track = [base.lerp_to_gamma(Color32::BLACK, 0.65), base, base.lerp_to_gamma(Color32::WHITE, 0.75)];
    // Each band's numerical editors keep their own focus and uncommitted text.
    ui.push_id(index, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        row(ui, dirty, "Hue", &mut p.hsl_hue[index], -100.0..=100.0, Some(&hue_track));
        row(ui, dirty, "Saturation", &mut p.hsl_sat[index], -100.0..=100.0, Some(&saturation_track));
        row(ui, dirty, "Luminance", &mut p.hsl_lum[index], -100.0..=100.0, Some(&luminance_track));
    });
}

fn wheel(ui: &mut egui::Ui, dirty: &mut bool, title: &str, w: &mut Wheel) {
    subheading(ui, crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", title));
    let hs = widgets::hue_stops();
    row(ui, dirty, "Hue", &mut w.hue, 0.0..=360.0, Some(&hs));
    row(ui, dirty, "Saturation", &mut w.sat, 0.0..=100.0, None);
    row(ui, dirty, "Luminance", &mut w.lum, -100.0..=100.0, None);
}

// The shared Curve editor must never produce a curve `CameraRaw::validate` rejects.
const _: () = assert!(crate::point_curve::MAX_POINTS == photocraft_algo::camera_raw::MAX_CURVE_POINTS && crate::point_curve::MIN_GAP >= 1.0);

fn curve_controls(ui: &mut egui::Ui, p: &mut CameraRaw, dirty: &mut bool, state: &mut crate::state::PointCurveState, active: &mut CurveChannel) {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        for channel in CurveChannel::ALL {
            let selected = *active == channel;
            let name = crate::i18n::tr_ctx(crate::i18n::current(), "cameraRaw", channel.name());
            let response =
                ui.add(egui::Button::new(egui::RichText::new("●").color(channel.color(&t)).size(18.0)).selected(selected).min_size(vec2(28.0, 24.0)));
            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), selected, name));
            if response.on_hover_text(name).clicked() && !selected {
                *active = channel;
                *state = Default::default();
            }
        }
        if ui.button(tl!("Reset")).clicked() {
            let curve = active.points_mut(p);
            *dirty |= !curve.is_empty();
            curve.clear();
            *state = Default::default();
        }
    });
}

/// Click to add, drag to move, right-click to delete on the selected channel.
fn curve_editor(ui: &mut egui::Ui, p: &mut CameraRaw, dirty: &mut bool, state: &mut crate::state::PointCurveState, channel: CurveChannel) -> ERect {
    let t = Tokens::get(ui.ctx());
    let side = ui.available_width().min(260.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), tl!("Curve")));
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, t.radius_sm, t.field);
    for i in 1..4 {
        let f = i as f32 / 4.0;
        painter.line_segment([pos2(rect.left() + f * side, rect.top()), pos2(rect.left() + f * side, rect.bottom())], Stroke::new(1.0, t.separator));
        painter.line_segment([pos2(rect.left(), rect.top() + f * side), pos2(rect.right(), rect.top() + f * side)], Stroke::new(1.0, t.separator));
    }
    let to_screen = |x: f32, y: f32| pos2(rect.left() + x / 255.0 * side, rect.bottom() - y / 255.0 * side);
    // The identity handles are presentation until a gesture actually edits them.
    let stored = channel.points(p);
    let mut points = if stored.is_empty() { vec![[0.0, 0.0], [255.0, 255.0]] } else { stored.clone() };
    if crate::point_curve::interact(ui, &resp, rect, &mut points, state).changed {
        *channel.points_mut(p) = points.clone();
        *dirty = true;
    }
    // Curve (point curve after the parametric one).
    let lut = curve_lut(&points, 256);
    let pts: Vec<Pos2> = lut.iter().enumerate().map(|(i, value)| to_screen(i as f32, value * 255.0)).collect();
    let color = channel.color(&t);
    painter.add(egui::Shape::line(pts, Stroke::new(1.5, color)));
    for (i, c) in points.iter().enumerate() {
        let center = to_screen(c[0], c[1]);
        if state.selected == Some(i) {
            painter.circle_filled(center, 3.0, color);
        }
        painter.circle_stroke(center, 4.0, Stroke::new(1.5, color));
    }
    rect
}

/// The native viewport owns a render surface. Complete an accepted close from
/// ROOT only after the backend has retired that surface and restored its owners.
pub(crate) fn finish_native_close(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some((action, queued_frame)) = app.camera_raw.as_ref().and_then(|dialog| dialog.native_close) else { return };
    let viewport = crate::dialogs::auxiliary_viewport(NATIVE_DIALOG_KEY);
    if ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT) <= queued_frame || ctx.input(|input| input.raw.viewports.contains_key(&viewport)) {
        ctx.request_repaint();
        return;
    }
    let Some(dialog) = app.camera_raw.as_mut() else { return };
    dialog.native_close = None;
    if !matches!(action, DialogAction::Cancel) && app.session.active().is_none_or(|state| state.doc.id != dialog.document) {
        app.camera_raw = None;
        app.ui.status = if crate::i18n::current().code() == "zh-hans" {
            "原文档已不在当前编辑状态，未应用 Camera Raw。"
        } else {
            "The original document is no longer active. Camera Raw was not applied."
        }
        .into();
        app.ui.status_error = true;
        return;
    }
    apply_dialog_action(app, action);
    ctx.request_repaint();
}

fn handle_dialog_action(app: &mut PhotocraftApp, ctx: &egui::Context, action: DialogAction) {
    if !ctx.embed_viewports() && ctx.viewport_id() != egui::ViewportId::ROOT {
        if let Some(dialog) = app.camera_raw.as_mut() {
            dialog.native_close = Some((action, ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT)));
        }
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    } else {
        apply_dialog_action(app, action);
    }
}

fn apply_dialog_action(app: &mut PhotocraftApp, action: DialogAction) {
    let result = match action {
        DialogAction::Output => create_output(app),
        DialogAction::Apply => commit(app),
        DialogAction::Cancel => {
            app.camera_raw = None;
            return;
        }
    };
    if let Err(error) = result {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.camera_raw.as_ref().is_none_or(CameraRawDialog::native_closing) {
        return;
    }
    if !ctx.embed_viewports() && ctx.viewport_id() == egui::ViewportId::ROOT {
        let size = (ctx.content_rect().size() * 0.9).max(vec2(900.0, 600.0));
        crate::dialogs::show_auxiliary_viewport(app, ctx, NATIVE_DIALOG_KEY, tl!("Camera Raw Filter"), size, |app, ui| {
            if ui.is_enabled() && ui.input(|input| input.viewport().close_requested()) {
                handle_dialog_action(app, ui.ctx(), DialogAction::Cancel);
                return;
            }
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::MinInnerSize(vec2(740.0, 480.0)));
            show(app, ui.ctx());
        });
        crate::dialogs::finish_auxiliary(app, ctx, NATIVE_DIALOG_KEY);
        return;
    }
    let native_host = !ctx.embed_viewports() && ctx.viewport_id() != egui::ViewportId::ROOT;
    if native_host {
        // Reconcile control/UI requests and native maximize/restore independently.
        // A request is emitted only when it changes; a later native state transition
        // updates the button without replaying stale geometry every frame.
        let state_id = egui::Id::new(("camera-raw-native-maximized", ctx.viewport_id()));
        let desired = app.ui.camera_raw_preview.maximized;
        let actual = ctx.input(|input| input.viewport().maximized.unwrap_or(false));
        let previous = ctx.data(|data| data.get_temp::<(bool, bool)>(state_id));
        if previous.is_none_or(|(requested, _)| requested != desired) {
            if actual != desired {
                ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(desired));
            }
        } else if previous.is_some_and(|(_, observed)| observed != actual) {
            app.ui.camera_raw_preview.maximized = actual;
        }
        ctx.data_mut(|data| data.insert_temp(state_id, (app.ui.camera_raw_preview.maximized, actual)));
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let mut action = None;
    let mut preset_action = None;
    let mut import_lut = false;
    let mut output_copy = false;
    let maximized = app.ui.camera_raw_preview.maximized;
    let mut subject_quality = app.session.prefs().performance.subject_recognition_quality;
    let subject_quality_before = subject_quality;
    let window = egui::Window::new(tl!("Camera Raw Filter"))
        .drag_area(egui::WindowDrag::TitleBar)
        .id(egui::Id::new("camera-raw-dialog"))
        .order(egui::Order::Foreground)
        .resizable(!maximized)
        .default_size(screen.size() * 0.85)
        .min_size(vec2(740.0, 480.0));
    let restore = app.ui.camera_raw_preview.restore_rect;
    let window = if native_host {
        // The native viewport owns moving and resizing. Render only the existing
        // Camera Raw contents here, without a second draggable egui window frame.
        app.ui.camera_raw_preview.restore_rect = false;
        window.title_bar(false).frame(egui::Frame::NONE).resizable(false).fixed_rect(screen)
    } else if maximized {
        window.fixed_rect(screen.shrink(8.0))
    } else if restore {
        app.ui.camera_raw_preview.restore_rect = false;
        if let Some([x, y, w, h]) = app.ui.camera_raw_preview.normal_rect { window.fixed_pos(pos2(x, y)).fixed_size(vec2(w, h)) } else { window }
    } else {
        window
    };
    let response = window.show(ctx, |ui| {
        ui.set_min_size(ui.available_size());
        let Some(d) = app.camera_raw.as_mut() else { return };
        d.dirty |= masks::poll_and_start(
            ctx,
            &mut d.masks,
            &mut d.params.local_adjustments,
            &d.recognition_source.pixels,
            d.recognition_source.width,
            d.recognition_source.height,
            masks::RecognitionOptions { transform: Some(&d.scope_transform), quality: subject_quality },
        );
        masks::prepare_tone(&mut d.masks, &d.params, &d.proxy, d.pw, d.ph, d.float);
        d.curve_rect = None;
        d.curve_visible_rect = None;
        d.scope.vectorscope_rect = None;
        d.poll_render(ctx);
        d.detail.poll_completed(ctx, d.requested_revision);
        if d.dirty {
            d.queue_render(ctx);
        }
        let (full, _) = ui.allocate_exact_size(ui.available_size(), Sense::click());
        let painter = ui.painter().clone();
        painter.rect_filled(full, 0.0, t.chrome);
        let title = ERect::from_min_size(full.min, vec2(full.width(), 30.0));
        painter.rect_filled(title, 0.0, t.dock);
        painter.line_segment([title.left_bottom(), title.right_bottom()], Stroke::new(1.0, t.separator));
        let window_title = crate::i18n::fmt(tl!("Camera Raw Filter ({layer})"), &[("layer", &d.layer_name)]);
        painter.text(title.center(), Align2::CENTER_CENTER, window_title, FontId::proportional(13.0), t.text);
        let toggle = ERect::from_min_size(pos2(title.right() - 100.0, title.top() + 2.0), vec2(94.0, 25.0));
        if ui.put(toggle, egui::Button::new(if maximized { tl!("Restore Window") } else { tl!("Maximize Window") })).clicked() {
            app.ui.camera_raw_preview.maximized = !maximized;
            app.ui.camera_raw_preview.restore_rect = maximized;
            ctx.request_repaint();
        }
        let footer_h = 48.0;
        let body = ERect::from_min_max(pos2(full.left(), title.bottom()), pos2(full.right(), full.bottom() - footer_h));
        let scope = &mut app.ui.camera_raw_scope;
        // Clip both pixels and scope overlays before painting the settings dock/footer.
        let preview_body = ERect::from_min_max(body.min, pos2((body.right() - PANEL_W).max(body.left() + 1.0), body.bottom()));
        let view = ERect::from_min_max(
            preview_body.min + vec2(16.0, 16.0),
            pos2((preview_body.right() - 16.0).max(preview_body.left() + 17.0), (preview_body.bottom() - 48.0).max(preview_body.top() + 17.0)),
        );
        d.preview_ppp = ctx.pixels_per_point();
        let size = vec2(d.full_w as f32, d.full_h as f32) / ctx.pixels_per_point();
        let navigation = &mut app.ui.camera_raw_preview;
        let panes = super::camera_raw_compare_ui::panes(navigation, view, d.show_before);
        let nav_view = panes.first().map_or(view, |p| p.viewport);
        d.viewport = Some(nav_view);
        let original_before = d.show_before;
        for (i, comparison) in panes.iter().copied().enumerate() {
            let (pane, before) = (comparison.viewport, comparison.before);
            let mut preview = ui.new_child(egui::UiBuilder::new().id_salt(("cr-comparison", i)).max_rect(pane));
            preview.set_clip_rect(comparison.clip.intersect(ui.clip_rect()));
            preview.painter().rect_filled(comparison.clip, 0.0, t.canvas);
            let mask_editing = d.active_tab == tabs::Tab::Masks
                && d.masks.edit
                && d.masks.selection_visible(&d.params.local_adjustments)
                && !before
                && !ctx.input(|i| i.key_down(egui::Key::Space));
            let (r, response, panning, zoom_box) = if mask_editing {
                let r = navigation.image_rect(pane, size);
                let response = preview.interact(pane, preview.id().with("raw-mask-input"), Sense::click_and_drag());
                (r, response, false, None)
            } else {
                super::camera_raw_preview_ui::interact(&mut preview, navigation, pane, size, &mut scope.sampler_tool)
            };
            let tex = if before { d.before_tex.as_ref() } else { d.tex.as_ref() };
            if let Some(tex) = tex {
                widgets::checker(preview.painter(), r.intersect(pane), 8.0);
                preview.painter().image(tex.id(), r, ERect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                let detail_needed = r.width() * ctx.pixels_per_point() > d.pw as f32 || r.height() * ctx.pixels_per_point() > d.ph as f32;
                if detail_needed {
                    // At Fit even finite-support detail can cover the full 24 MP source.
                    // Reserve interaction for the bounded proxy instead of competing with it.
                    let allowed = before || d.detail_allowed(ctx);
                    d.detail.paint(&preview, r, d.requested_revision, before, &d.params, allowed);
                } else {
                    d.detail.clear_display();
                }
                if d.detail.pending() {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
                d.show_before = before;
                if !before || panes.len() == 1 {
                    d.scope.preview_rect = Some(r);
                }
                if !mask_editing && (panes.len() == 1 || !before) {
                    super::camera_raw_scope_ui::preview(&mut preview, d, scope, r, &response, panning);
                }
                if !before && d.active_tab == tabs::Tab::Masks {
                    d.dirty |= masks::preview(
                        &preview,
                        &mut d.masks,
                        &mut d.params.local_adjustments,
                        masks::Preview { rect: r, response: &response, source: &d.proxy, size: [d.pw, d.ph], can_edit: mask_editing },
                    );
                }
            }
            if panes.len() > 1 {
                preview.painter().text(
                    comparison.clip.left_top() + vec2(8.0, 8.0),
                    Align2::LEFT_TOP,
                    if before { tl!("Before") } else { tl!("After") },
                    FontId::proportional(12.0),
                    t.text,
                );
            }
            if let Some(region) = zoom_box {
                preview.painter().rect_stroke(region, 0.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
            }
        }
        super::camera_raw_compare_ui::divider(ui, navigation.comparison, view);
        d.show_before = original_before;
        let toolbar = ERect::from_min_max(pos2(preview_body.left() + 16.0, view.bottom() + 8.0), preview_body.right_bottom() - vec2(16.0, 4.0));
        let mut nav_ui = ui.new_child(egui::UiBuilder::new().max_rect(toolbar).layout(egui::Layout::left_to_right(egui::Align::Center)));
        nav_ui.set_clip_rect(preview_body.intersect(ui.clip_rect()));
        super::camera_raw_preview_ui::toolbar(&mut nav_ui, navigation, nav_view, size, &mut scope.sampler_tool);
        if d.detail.pending() {
            nav_ui.spinner();
        }
        if let Some(error) = &d.detail.error {
            nav_ui.label(egui::RichText::new(tl!("Full-resolution preview unavailable")).color(t.text_faint)).on_hover_text(error);
        }
        if let Some(error) = &d.preview_error {
            nav_ui.label(error);
        }
        // Panels.
        let right = ERect::from_min_max(pos2(body.right() - PANEL_W, body.top()), body.max);
        painter.rect_filled(right, 0.0, t.dock);
        painter.line_segment([right.left_top(), right.left_bottom()], Stroke::new(1.0, t.separator));
        if let Some(tab) = tabs::rail(ui, right, d.active_tab, &mut d.tab_geometry) {
            d.select_tab(tab);
            ctx.request_repaint();
        }
        let content = tabs::content_rect(right);
        let mut props = ui.new_child(egui::UiBuilder::new().id_salt("camera-raw-properties").max_rect(content.shrink2(vec2(10.0, 10.0))));
        props.set_clip_rect(content.intersect(ui.clip_rect()));
        // This header is outside the settings scroll area and follows the same Before/After
        // selector as the texture. Both derive from the same completed preview revision.
        super::camera_raw_scope_ui::header(&mut props, d, scope);
        d.scope.alt_tone = None;
        let mut dirty = false;
        d.color_geometry = Default::default();
        egui::ScrollArea::vertical().id_salt(("camera-raw-props", d.active_tab)).show_palette(&mut props, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            match d.active_tab {
                tabs::Tab::Presets => {
                    preset_action = presets::panel(ui, &mut d.presets, &app.session.presets.camera_raw);
                    return;
                }
                tabs::Tab::Masks => {
                    ui.horizontal(|ui| {
                        super::recognition_quality_ui::selector(ui, &mut subject_quality);
                    });
                    dirty |= masks::panel(ui, &mut d.params.local_adjustments, &mut d.masks);
                    return;
                }
                tabs::Tab::ColorSpaces => {
                    widgets::section_label(ui, tl!("Color Spaces"));
                    output_copy = color_spaces::panel(ui, &mut d.params.color_spaces, &mut dirty, &mut d.color_geometry, &mut d.output_options);
                    return;
                }
                tabs::Tab::ColorLookup => {
                    widgets::section_label(ui, tl!("Color Lookup (LUT)"));
                    import_lut = lut::panel(ui, &mut d.params.color_lookup, &mut dirty, d.lut_message.as_deref());
                    return;
                }
                tabs::Tab::Edit => {}
            }
            let p = &mut d.params;
            let alt = &mut d.scope.alt_tone;
            let (ts, tn) = (temp_stops(), tint_stops());
            section(ui, "Light", true, |ui| {
                tone_row(ui, &mut dirty, alt, ToneZone::Exposure, &mut p.exposure);
                row(ui, &mut dirty, "Contrast", &mut p.contrast, -100.0..=100.0, None);
                tone_row(ui, &mut dirty, alt, ToneZone::Highlights, &mut p.highlights);
                tone_row(ui, &mut dirty, alt, ToneZone::Shadows, &mut p.shadows);
                tone_row(ui, &mut dirty, alt, ToneZone::Whites, &mut p.whites);
                tone_row(ui, &mut dirty, alt, ToneZone::Blacks, &mut p.blacks);
            });
            section(ui, "Color", false, |ui| {
                widgets::section_label(ui, tl!("White Balance: As Shot"));
                row(ui, &mut dirty, "Temperature", &mut p.temperature, -100.0..=100.0, Some(&ts));
                row(ui, &mut dirty, "Tint", &mut p.tint, -100.0..=100.0, Some(&tn));
                row(ui, &mut dirty, "Vibrance", &mut p.vibrance, -100.0..=100.0, None);
                row(ui, &mut dirty, "Saturation", &mut p.saturation, -100.0..=100.0, None);
            });
            section(ui, "Effects", false, |ui| {
                row(ui, &mut dirty, "Texture", &mut p.texture, -100.0..=100.0, None);
                row(ui, &mut dirty, "Clarity", &mut p.clarity, -100.0..=100.0, None);
                row(ui, &mut dirty, "Dehaze", &mut p.dehaze, -100.0..=100.0, None);
                subheading(ui, tl!("Vignetting"));
                let mut style = p.vignette_style.clone();
                if widgets::dropdown(
                    ui,
                    "cr-vig-style",
                    &mut style,
                    &[
                        ("highlightPriority".to_string(), tl!("Highlight Priority")),
                        ("colorPriority".to_string(), tl!("Color Priority")),
                        ("paintOverlay".to_string(), tl!("Paint Overlay")),
                    ],
                    200.0,
                ) {
                    p.vignette_style = style;
                    dirty = true;
                }
                row(ui, &mut dirty, "Amount", &mut p.vignette_amount, -100.0..=100.0, None);
                row(ui, &mut dirty, "Midpoint", &mut p.vignette_midpoint, 0.0..=100.0, None);
                row(ui, &mut dirty, "Roundness", &mut p.vignette_roundness, -100.0..=100.0, None);
                row(ui, &mut dirty, "Feather", &mut p.vignette_feather, 0.0..=100.0, None);
                row(ui, &mut dirty, "Highlights", &mut p.vignette_highlights, 0.0..=100.0, None);
                subheading(ui, tl!("Grain"));
                row(ui, &mut dirty, "Amount", &mut p.grain_amount, 0.0..=100.0, None);
                row(ui, &mut dirty, "Size", &mut p.grain_size, 0.0..=100.0, None);
                row(ui, &mut dirty, "Roughness", &mut p.grain_roughness, 0.0..=100.0, None);
            });
            section(ui, "Curve", false, |ui| {
                curve_controls(ui, p, &mut dirty, &mut d.curve_state, &mut d.curve_channel);
                let graph = curve_editor(ui, p, &mut dirty, &mut d.curve_state, d.curve_channel);
                d.curve_rect = Some(graph);
                let visible = graph.intersect(ui.clip_rect());
                d.curve_visible_rect = visible.is_positive().then_some(visible);
                row(ui, &mut dirty, "Highlights", &mut p.curve_highlights, -100.0..=100.0, None);
                row(ui, &mut dirty, "Lights", &mut p.curve_lights, -100.0..=100.0, None);
                row(ui, &mut dirty, "Darks", &mut p.curve_darks, -100.0..=100.0, None);
                row(ui, &mut dirty, "Shadows", &mut p.curve_shadows, -100.0..=100.0, None);
            });
            section(ui, "Color Mixer", false, |ui| {
                color_mixer(ui, p, &mut d.mixer_band, &mut dirty);
            });
            section(ui, "Color Grading", false, |ui| {
                wheel(ui, &mut dirty, "Shadows", &mut p.grade_shadows);
                wheel(ui, &mut dirty, "Midtones", &mut p.grade_midtones);
                wheel(ui, &mut dirty, "Highlights", &mut p.grade_highlights);
                wheel(ui, &mut dirty, "Global", &mut p.grade_global);
                row(ui, &mut dirty, "Blending", &mut p.grade_blending, 0.0..=100.0, None);
                row(ui, &mut dirty, "Balance", &mut p.grade_balance, -100.0..=100.0, None);
            });
            section(ui, "Detail", false, |ui| {
                subheading(ui, tl!("Sharpening"));
                row(ui, &mut dirty, "Amount", &mut p.sharpen_amount, 0.0..=150.0, None);
                row(ui, &mut dirty, "Radius", &mut p.sharpen_radius, 0.5..=3.0, None);
                row(ui, &mut dirty, "Detail", &mut p.sharpen_detail, 0.0..=100.0, None);
                row(ui, &mut dirty, "Masking", &mut p.sharpen_masking, 0.0..=100.0, None);
                subheading(ui, tl!("Noise Reduction"));
                row(ui, &mut dirty, "Luminance", &mut p.noise_luminance, 0.0..=100.0, None);
                row(ui, &mut dirty, "Luminance Detail", &mut p.noise_luminance_detail, 0.0..=100.0, None);
                row(ui, &mut dirty, "Color", &mut p.noise_color, 0.0..=100.0, None);
                row(ui, &mut dirty, "Color Detail", &mut p.noise_color_detail, 0.0..=100.0, None);
            });
        });
        super::camera_raw_scope_ui::floating(ctx, d, scope, right);
        super::camera_raw_scope_ui::shortcuts(ctx, scope);
        d.dirty |= dirty;
        if d.dirty {
            d.queue_render(ctx);
        }
        // Footer.
        let foot = ERect::from_min_max(pos2(full.left(), full.bottom() - footer_h), full.max);
        painter.rect_filled(foot, 0.0, t.dock);
        painter.line_segment([foot.left_top(), foot.right_top()], Stroke::new(1.0, t.separator));
        let mut fu = ui.new_child(egui::UiBuilder::new().max_rect(foot.shrink2(vec2(16.0, 9.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
        if let Some(role) = widgets::dialog_buttons(
            &mut fu,
            &[
                widgets::DialogButton::new(widgets::ButtonRole::Default, tl!("OK"), 90.0),
                widgets::DialogButton::new(widgets::ButtonRole::Cancel, tl!("Cancel"), 90.0),
            ],
        ) {
            action = Some(if role == widgets::ButtonRole::Default { DialogAction::Apply } else { DialogAction::Cancel });
        }
        fu.add_space(12.0);
        super::camera_raw_compare_ui::controls(&mut fu, navigation, &mut d.show_before);
        fu.label(egui::RichText::new(format!("{:.0} ms", d.render_ms)).color(t.text_faint));
    });
    if let Some(operation) = app.camera_raw.as_mut().and_then(|dialog| dialog.masks.take_operation())
        && let Err(error) = execute_mask_operation(app, ctx, &operation)
        && let Some(dialog) = app.camera_raw.as_mut()
    {
        dialog.masks.error = Some(error);
    }
    if output_copy {
        action = Some(DialogAction::Output);
    }
    if import_lut
        && let Err(error) = lut::execute(app, ctx, None)
        && let Some(d) = app.camera_raw.as_mut()
    {
        d.lut_message = Some(format!("{}: {error}", tl!("Error")));
    }
    if let Some(request) = preset_action
        && let Err(error) = presets::execute(app, ctx, request)
        && let Some(d) = app.camera_raw.as_mut()
    {
        d.presets.message = Some(error);
    }
    if !native_host
        && !maximized
        && let Some(response) = response
    {
        let r = response.response.rect;
        app.ui.camera_raw_preview.normal_rect = Some([r.left(), r.top(), r.width(), r.height()]);
    }
    super::camera_raw_scope_ui::persist(app, ctx);
    if subject_quality != subject_quality_before {
        let _ = app.run("prefs.set", json!({"path":"performance.subjectRecognitionQuality","value":subject_quality.name()}));
    }
    if let Some(d) = app.camera_raw.as_mut() {
        super::camera_raw_compare_ui::keys(ctx, &mut app.ui.camera_raw_preview, &mut d.show_before);
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        action = Some(DialogAction::Cancel);
    }
    if let Some(action) = action {
        handle_dialog_action(app, ctx, action);
    }
}

fn hue_color(h: f32) -> Color32 {
    let h6 = (h.rem_euclid(360.0)) / 60.0;
    let x = 1.0 - ((h6 % 2.0) - 1.0).abs();
    let (r, g, b) = match h6 as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    Color32::from_rgb((r * 220.0) as u8, (g * 220.0) as u8, (b * 220.0) as u8)
}

#[cfg(test)]
#[path = "camera_raw_features_tests_v57.rs"]
mod features_tests_v57;
#[cfg(test)]
#[path = "camera_raw_mask_control_tests_v65.rs"]
mod mask_control_tests_v65;
#[cfg(test)]
#[path = "camera_raw_mask_control_tests_v66.rs"]
mod mask_control_tests_v66;
#[cfg(test)]
#[path = "camera_raw_quality_tests_v57.rs"]
mod quality_tests_v57;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_camera_raw_waits_for_its_surface_to_retire_before_applying() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#40a080"})).unwrap();
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"set":{"exposure":0.3}}})).unwrap().unwrap();
        let revision = app.session.active().unwrap().revision;
        let viewport = crate::dialogs::auxiliary_viewport(NATIVE_DIALOG_KEY);
        assert_eq!(crate::dialogs::native_dialog_viewport(&app), Some(viewport));
        let frame = |app: &mut PhotocraftApp, live: bool, queue: bool| {
            let mut input = egui::RawInput::default();
            if live {
                input.viewports.entry(viewport).or_default();
            }
            let mut out = ctx.run_ui(input, |ui| {
                if queue {
                    app.camera_raw.as_mut().unwrap().native_close = Some((DialogAction::Apply, ui.ctx().cumulative_frame_nr_for(egui::ViewportId::ROOT)));
                    assert!(crate::dialogs::native_dialog_viewport(app).is_none(), "the retiring editor must surrender the native modal role");
                }
                finish_native_close(app, ui.ctx());
            });
            out.textures_delta.clear();
        };
        frame(&mut app, true, true);
        frame(&mut app, true, false);
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert!(app.camera_raw.as_ref().unwrap().native_closing());
        frame(&mut app, false, false);
        assert!(app.camera_raw.is_none());
        assert!(app.session.active().unwrap().revision > revision);
    }

    #[test]
    fn curve_channel_control_is_atomic_and_switching_does_not_render_or_edit() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let initial = menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        for channel in ["red", "green", "blue", "rgb"] {
            app.camera_raw.as_mut().unwrap().curve_state.selected = Some(1);
            let value = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"curveChannel": channel}})).unwrap().unwrap();
            assert_eq!(value["curveChannel"], channel);
            assert_eq!(value["params"], initial["params"]);
            assert_eq!(value["previewRevision"], initial["previewRevision"]);
            assert_eq!(value["curveState"]["selected"], Value::Null);
        }
        for invalid in [json!("alpha"), json!(4), Value::Null, json!([])] {
            assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"curveChannel": invalid, "set": {"exposure": 2}}})).unwrap().is_err());
            assert_eq!(app.camera_raw.as_ref().unwrap().params.exposure, 0.0);
        }
    }

    #[test]
    fn individual_curve_channels_preview_commit_and_reset_smart_filter_without_losing_other_channels() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 16, "height": 16, "depth": 32})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        let original = app.camera_raw.as_ref().unwrap().processed[0];
        let curve = json!([[0.0, 0.0], [128.0, 180.0], [255.0, 255.0]]);
        for (index, (channel, key)) in [("red", "pointCurveRed"), ("green", "pointCurveGreen"), ("blue", "pointCurveBlue")].iter().enumerate() {
            let set = json!({(*key): curve});
            let result = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"curveChannel": channel, "set": set}})).unwrap().unwrap();
            assert_eq!(result["params"][*key], curve);
            let displayed = app.camera_raw.as_ref().unwrap().processed[0];
            for c in 0..3 {
                if c <= index {
                    assert!(displayed[c] > original[c] + 0.1, "channel {channel} {displayed:?}");
                } else {
                    assert!((displayed[c] - original[c]).abs() < 2e-6);
                }
            }
        }
        let preview = app.camera_raw.as_ref().unwrap().processed[0];
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let pixel = app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().rgba(0, 0);
        for (a, b) in preview.iter().zip(pixel) {
            assert!((a - b).abs() < 2e-6, "preview {preview:?}, applied {pixel:?}");
        }
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": layer.0, "index": 0}})).unwrap().unwrap();
        let result = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"pointCurveRed": []}, "commit": true}})).unwrap().unwrap();
        assert!(!result.is_null());
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": layer.0, "index": 0}})).unwrap().unwrap();
        let d = app.camera_raw.as_ref().unwrap();
        assert!(d.params.point_curve_red.is_empty());
        assert_eq!(json!(d.params.point_curve_green), curve);
        assert_eq!(json!(d.params.point_curve_blue), curve);
        assert!((d.processed[0][0] - original[0]).abs() < 2e-6);
    }

    #[test]
    fn curve_channel_buttons_route_real_point_edits_and_reset_to_selected_channel_only() {
        use egui_kittest::kittest::Queryable;
        type Editor = (CameraRaw, CurveChannel, crate::state::PointCurveState, bool, ERect);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(320.0, 340.0)).build_ui_state(
            |ui, (params, channel, state, dirty, rect): &mut Editor| {
                curve_controls(ui, params, dirty, state, channel);
                *rect = curve_editor(ui, params, dirty, state, *channel);
            },
            (CameraRaw::default(), CurveChannel::Rgb, Default::default(), false, ERect::NOTHING),
        );
        for (channel, name) in [(CurveChannel::Red, "Red"), (CurveChannel::Green, "Green"), (CurveChannel::Blue, "Blue")] {
            h.get_by_label(name).click();
            h.run_steps(2);
            assert_eq!(h.state().1, channel);
            let rect = h.state().4;
            let at = pos2(rect.left() + rect.width() * 0.5, rect.top() + rect.height() * 0.3);
            h.event(egui::Event::PointerMoved(at));
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.run_steps(2);
            }
            assert_eq!(channel.points(&h.state().0).len(), 3);
            assert!(h.state().0.point_curve.is_empty(), "master is untouched");
        }
        h.get_by_label("Reset").click();
        h.run_steps(2);
        assert!(h.state().0.point_curve_blue.is_empty());
        assert_eq!(h.state().0.point_curve_red.len(), 3);
        assert_eq!(h.state().0.point_curve_green.len(), 3);
    }

    #[test]
    fn fit_proxy_covers_display_pixels_instead_of_enlarging_the_interaction_thumbnail() {
        for (screen, ppp) in [(vec2(1920.0, 1080.0), 1.0), (vec2(1440.0, 900.0), 2.0)] {
            let (pw, ph) = proxy_size(6000, 4000, screen, ppp);
            let display = (screen - vec2(PANEL_W + 64.0, 160.0)) * ppp;
            let scale = (display.x / 6000.0).min(display.y / 4000.0);
            assert!(pw as f32 >= 6000.0 * scale);
            assert!(ph as f32 >= 4000.0 * scale);
            assert!(pw * ph <= PROXY_PIXELS);
        }
        let (pw, ph) = proxy_size(6000, 4000, vec2(16000.0, 10000.0), 4.0);
        assert!(pw * ph <= PROXY_PIXELS);
        assert_eq!(proxy_size(64, 48, vec2(1920.0, 1080.0), 1.0), (64, 48));
    }

    #[test]
    fn reduced_feedback_refines_same_revision_without_new_input_and_preserves_mask_alpha() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":1024,"height":768})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#506080"})).unwrap();
        open(&mut app, &ctx).unwrap();
        let d = app.camera_raw.as_mut().unwrap();
        assert!(d.live_source.pixels.len() < d.proxy.len());
        d.selection = Some((0..d.proxy.len()).map(|index| if index % d.pw < d.pw / 2 { 0.0 } else { 0.5 }).collect());
        d.params.exposure = 0.5;
        d.params.clarity = 5.0;
        let point = pos2(-10.0, -10.0);
        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(0.0),
                events: vec![egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE }],
                ..Default::default()
            },
            |ui| d.queue_render(ui.ctx()),
        );
        output.textures_delta.clear();
        let revision = d.requested_revision;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while d.preview_revision != revision {
            assert!(std::time::Instant::now() < deadline, "reduced feedback did not publish");
            let mut output = ctx.run_ui(egui::RawInput { time: Some(0.05), ..Default::default() }, |ui| d.poll_render(ui.ctx()));
            output.textures_delta.clear();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(d.proxy_reduced);
        assert!(!d.detail_allowed(&ctx), "local detail must not compete while dragging");
        let reduced = d.describe(&Default::default(), &Default::default());
        assert_eq!(reduced["previewApproximate"], true);
        assert_eq!(reduced["previewPending"], true, "refinement is still owed");
        for (index, (processed, original)) in d.processed.iter().zip(d.proxy.iter()).enumerate() {
            assert_eq!(processed[3], original[3]);
            if index % d.pw < d.pw / 2 {
                assert_eq!(processed, original, "zero coverage remains untouched");
            }
        }
        let mut output = ctx.run_ui(
            egui::RawInput {
                time: Some(1.0),
                events: vec![egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }],
                ..Default::default()
            },
            |ui| d.poll_render(ui.ctx()),
        );
        output.textures_delta.clear();
        while d.proxy_reduced || d.proxy_pending.is_some() {
            assert!(std::time::Instant::now() < deadline, "display resolution was never restored");
            let mut output = ctx.run_ui(egui::RawInput { time: Some(1.1), ..Default::default() }, |ui| d.poll_render(ui.ctx()));
            output.textures_delta.clear();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(d.preview_revision, revision, "quality restoration must not create an edit");
        assert_eq!(d.requested_revision, revision);
        assert!(d.detail_allowed(&ctx));
        let mut expected = d.proxy.to_vec();
        let mut params = d.params.clone();
        params.pixel_scale = d.pw as f32 / d.full_w as f32;
        photocraft_algo::camera_raw::develop(&mut expected, d.pw, d.ph, &params, d.float);
        for ((pixel, before), coverage) in expected.iter_mut().zip(d.proxy.iter()).zip(d.selection.as_ref().unwrap().iter()) {
            for (value, source) in pixel.iter_mut().zip(before) {
                *value = *source + (*value - *source) * coverage;
            }
        }
        assert_eq!(d.processed.as_ref(), expected.as_slice());
        assert_eq!(d.after_histogram, RgbHistogram::from_rgba(&expected));
        assert_eq!(d.describe(&Default::default(), &Default::default())["previewPending"], false);
        d.params = CameraRaw::default();
        d.queue_render(&ctx);
        while d.proxy_pending.is_some() || d.proxy_requested {
            assert!(std::time::Instant::now() < deadline, "identity reset never published");
            let mut output = ctx.run_ui(egui::RawInput { time: Some(1.15), ..Default::default() }, |ui| d.poll_render(ui.ctx()));
            output.textures_delta.clear();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(!d.proxy_reduced, "identity needs no heavy develop and should stay sharp immediately");
        assert_eq!(d.processed, d.proxy);
    }

    #[test]
    fn display_sized_bins_cover_each_source_pixel_once_and_keep_wide_images_safe() {
        for (source, bins) in [(97, 33), (6000, 1380), (1_048_576, 4096)] {
            let mut total = 0;
            for bin in 0..bins {
                let start = proxy_bin_start(bin, source, bins);
                let end = proxy_bin_start(bin + 1, source, bins);
                assert!(start < end);
                assert_eq!(start as u64 * bins as u64 / source as u64, bin as u64);
                assert_eq!((end - 1) as u64 * bins as u64 / source as u64, bin as u64);
                total += end - start;
            }
            assert_eq!(total, source);
        }
    }

    #[test]
    fn rapid_edits_coalesce_and_finish_without_another_input_event() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width":64,"height":48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#506080"})).unwrap();
        open(&mut app, &ctx).unwrap();
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = wakes.clone();
        ctx.set_request_repaint_callback(move |_| {
            count.fetch_add(1, Ordering::Relaxed);
        });
        let d = app.camera_raw.as_mut().unwrap();
        let initial = d.processed.clone();
        let initial_revision = d.preview_revision;
        // Hold an older worker in flight while multiple different controls change.
        let (tx, rx) = std::sync::mpsc::channel();
        d.proxy_pending = Some(rx);
        for step in 0..80 {
            d.params.exposure = step as f32 / 80.;
            d.params.contrast = step as f32 / 2.;
            d.params.temperature = -(step as f32);
            d.params.texture = 10.;
            d.queue_render(&ctx);
        }
        assert_eq!(d.preview_revision, initial_revision, "requesting is not displaying");
        assert_eq!(d.processed, initial);
        tx.send(Ok(ProxyFrame {
            revision: initial_revision,
            params: CameraRaw::default(),
            image: CameraRawDialog::image(&initial, d.pw, d.ph),
            histogram: RgbHistogram::from_rgba(&initial),
            pixels: initial.clone(),
            unmasked: None,
            histogram_ms: 0.,
            render_ms: 0.,
            live: None,
            global_stage: None,
            global_cache_hit: false,
        }))
        .unwrap();
        d.poll_render(&ctx);
        assert_eq!(d.processed, initial, "obsolete worker cannot install its result");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        // No pointer event or slider click: only scheduled frames/polling.
        while d.proxy_pending.is_some() || d.proxy_requested {
            assert!(std::time::Instant::now() < deadline);
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| d.poll_render(ui.ctx()));
            output.textures_delta.clear();
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut expected = d.proxy.to_vec();
        let mut params = d.params.clone();
        params.pixel_scale = d.pw as f32 / d.full_w as f32;
        photocraft_algo::camera_raw::develop(&mut expected, d.pw, d.ph, &params, d.float);
        assert_eq!(d.processed.as_ref(), expected.as_slice());
        assert_eq!(d.after_histogram, RgbHistogram::from_rgba(&expected));
        assert_eq!(d.preview_revision, d.requested_revision);
        assert_eq!(d.rendered_params, d.params);
        assert!(wakes.load(Ordering::Relaxed) > 0);
        assert!(d.preview_error.is_none());
    }

    #[test]
    fn continuous_edits_publish_completed_intermediate_proxy_frames() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width":8,"height":8})).unwrap();
        open(&mut app, &ctx).unwrap();
        let d = app.camera_raw.as_mut().unwrap();
        let displayed = d.preview_revision;
        d.requested_revision = displayed + 10;
        let pixels = vec![[0.4, 0.5, 0.6, 1.0]; 64];
        let (tx, rx) = std::sync::mpsc::channel();
        d.proxy_pending = Some(rx);
        tx.send(Ok(ProxyFrame {
            revision: displayed + 1,
            params: CameraRaw { exposure: 0.5, ..Default::default() },
            image: CameraRawDialog::image(&pixels, 8, 8),
            histogram: RgbHistogram::from_rgba(&pixels),
            pixels: pixels.clone().into(),
            unmasked: None,
            histogram_ms: 0.0,
            render_ms: 0.0,
            live: None,
            global_stage: None,
            global_cache_hit: false,
        }))
        .unwrap();
        d.poll_render(&ctx);
        assert_eq!(d.preview_revision, displayed + 1, "completed intermediate frames remain visible during sustained edits");
        assert_eq!(d.requested_revision, displayed + 10, "latest requested settings remain distinct");
        assert_eq!(d.processed.as_ref(), pixels.as_slice());
        assert_eq!(d.rendered_params.exposure, 0.5);
    }

    #[test]
    fn obsolete_same_revision_quality_replacement_cannot_hide_newer_input() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width":8,"height":8})).unwrap();
        open(&mut app, &ctx).unwrap();
        let d = app.camera_raw.as_mut().unwrap();
        let displayed = d.preview_revision;
        d.proxy_reduced = true;
        d.requested_revision = displayed + 1;
        let pixels = vec![[0.4, 0.5, 0.6, 1.0]; 64];
        let previous = d.processed.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        d.proxy_pending = Some(rx);
        tx.send(Ok(ProxyFrame {
            revision: displayed,
            params: CameraRaw { exposure: 0.5, ..Default::default() },
            image: CameraRawDialog::image(&pixels, 8, 8),
            histogram: RgbHistogram::from_rgba(&pixels),
            pixels: pixels.into(),
            unmasked: None,
            histogram_ms: 0.0,
            render_ms: 0.0,
            live: None,
            global_stage: None,
            global_cache_hit: false,
        }))
        .unwrap();
        d.poll_render(&ctx);
        assert_eq!(d.processed, previous, "a same-revision refinement is useful only while its settings are still current");
        assert!(d.proxy_reduced);
        assert_eq!(d.preview_revision, displayed);
        assert_eq!(d.requested_revision, displayed + 1);
    }

    #[test]
    #[ignore = "release CPU benchmark for 24MP source/display and 0.5MP interaction proxies"]
    fn interaction_proxy_24mp_cpu_benchmark() {
        let mut records = Vec::new();
        for ppp in [1.0, 2.0] {
            let (w, h) = proxy_size(6000, 4000, vec2(1440.0, 900.0), ppp);
            let original: std::sync::Arc<[[f32; 4]]> = (0..w * h)
                .map(|i| [(i % w % 256) as f32 / 255.0, (i / w % 256) as f32 / 255.0, ((i % w) ^ (i / w)) as u8 as f32 / 255.0, 1.0])
                .collect::<Vec<_>>()
                .into();
            let start = std::time::Instant::now();
            let source = live::Source::new(original.clone(), w, h);
            let cache_ms = start.elapsed().as_secs_f64() * 1000.0;
            for effect_case in ["spatial-mixed", "pointwise"] {
                let mut full = Vec::new();
                let mut reduced = Vec::new();
                for repetition in 0..6 {
                    let params = if effect_case == "pointwise" {
                        CameraRaw { exposure: 0.25 + repetition as f32 * 0.02, contrast: 20.0, temperature: -15.0, saturation: 10.0, ..Default::default() }
                    } else {
                        CameraRaw {
                            exposure: 0.25 + repetition as f32 * 0.02,
                            clarity: 25.0,
                            texture: 25.0,
                            highlights: -20.0,
                            dehaze: 10.0,
                            ..Default::default()
                        }
                    };
                    let start = std::time::Instant::now();
                    let mut pixels = original.to_vec();
                    let mut p = params.clone();
                    p.pixel_scale = w as f32 / 6000.0;
                    photocraft_algo::camera_raw::develop(&mut pixels, w, h, &p, false);
                    std::hint::black_box(RgbHistogram::from_rgba(&pixels));
                    std::hint::black_box(CameraRawDialog::image(&pixels, w, h));
                    let full_ms = start.elapsed().as_secs_f64() * 1000.0;
                    let start = std::time::Instant::now();
                    let mut pixels = source.pixels.to_vec();
                    p.pixel_scale = source.width as f32 / 6000.0;
                    photocraft_algo::camera_raw::develop(&mut pixels, source.width, source.height, &p, false);
                    let mut pixels = source.expand(&pixels, w, h);
                    for (pixel, before) in pixels.iter_mut().zip(original.iter()) {
                        pixel[3] = before[3];
                    }
                    std::hint::black_box(RgbHistogram::from_rgba(&pixels));
                    std::hint::black_box(CameraRawDialog::image(&pixels, w, h));
                    let reduced_ms = start.elapsed().as_secs_f64() * 1000.0;
                    if repetition > 0 {
                        full.push(full_ms);
                        reduced.push(reduced_ms);
                    }
                }
                records.push(json!({"sourceSize":[6000,4000],"pixelsPerPoint":ppp,"effectCase":effect_case,"displayProxy":[w,h],"interactionProxy":[source.width,source.height],
                "cacheOnceMs":cache_ms,"fullProxyMs":full,"reducedProxyIncludingExpansionMs":reduced}));
            }
        }
        println!(
            "CAMERA_RAW_V51_PROXY_BENCHMARK {}",
            json!({"measurement":"CPU proxy preparation in one process; excludes GPU presentation, physical input, source-open scan and exact refinement; one warmup and five samples", "records":records})
        );
    }

    #[test]
    #[ignore = "release same-pixel CPU comparison of serial and parallel Camera Raw display preparation"]
    fn display_preparation_24mp_cpu_benchmark() {
        let serial_image = |pixels: &[[f32; 4]], w: usize, h: usize| {
            let enc = |value: f32| (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            egui::ColorImage::new([w, h], pixels.iter().map(|q| Color32::from_rgba_unmultiplied(enc(q[0]), enc(q[1]), enc(q[2]), enc(q[3]))).collect())
        };
        let mut records = Vec::new();
        for ppp in [1.0, 2.0] {
            let (w, h) = proxy_size(6000, 4000, vec2(1440.0, 900.0), ppp);
            let pixels: std::sync::Arc<[[f32; 4]]> = (0..w * h)
                .map(|i| [(i % w % 256) as f32 / 255.0, (i / w % 256) as f32 / 255.0, ((i % w) ^ (i / w)) as u8 as f32 / 255.0, 1.0])
                .collect::<Vec<_>>()
                .into();
            let source = live::Source::new(pixels, w, h);
            let serial = source.expand_serial(&source.pixels, w, h);
            let parallel = source.expand(&source.pixels, w, h);
            assert_eq!(serial, parallel);
            assert_eq!(RgbHistogram::from_rgba(&serial), frame::histogram(&parallel));
            assert_eq!(serial_image(&serial, w, h), frame::image(&parallel, w, h));
            let mut samples = Vec::new();
            for repetition in 0..7 {
                // Alternate order so one implementation does not always inherit warmer
                // data/cache or run after the other implementation has heated the CPU.
                for serial in if repetition % 2 == 0 { [true, false] } else { [false, true] } {
                    let start = std::time::Instant::now();
                    let expanded = if serial { source.expand_serial(&source.pixels, w, h) } else { source.expand(&source.pixels, w, h) };
                    let expanded_at = start.elapsed().as_secs_f64() * 1000.0;
                    let histogram = if serial { RgbHistogram::from_rgba(&expanded) } else { frame::histogram(&expanded) };
                    let histogram_at = start.elapsed().as_secs_f64() * 1000.0;
                    let image = if serial { serial_image(&expanded, w, h) } else { frame::image(&expanded, w, h) };
                    let image_at = start.elapsed().as_secs_f64() * 1000.0;
                    std::hint::black_box((histogram, image));
                    if repetition > 0 {
                        samples.push(json!({"serial":serial,"expandMs":expanded_at,"histogramMs":histogram_at-expanded_at,
                            "imageMs":image_at-histogram_at,"totalMs":image_at}));
                    }
                }
            }
            records.push(json!({"sourceSize":[6000,4000],"pixelsPerPoint":ppp,"displayProxy":[w,h],
                "interactionProxy":[source.width,source.height],"pixelsHistogramAndImageIdentical":true,"samples":samples}));
        }
        println!(
            "CAMERA_RAW_V53_DISPLAY_BENCHMARK {}",
            json!({"measurement":"same pixel grid, source, interpolation and encoding; CPU only; excludes develop, upload and physical input", "records":records})
        );
    }

    #[test]
    fn held_pointer_global_edits_keep_publishing_visible_live_revisions_then_exactly_settle() {
        use std::collections::BTreeSet;
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":128,"height":96})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#506080"})).unwrap();
        let screen = ERect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0));
        let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), ..Default::default() }, |_| {});
        output.textures_delta.clear();
        open(&mut app, &ctx).unwrap();
        app.ui.camera_raw_preview.zoom = Some(8.0);
        let pointer = pos2(-10.0, -10.0);
        let mut revisions = BTreeSet::new();
        let start = std::time::Instant::now();
        let mut frames = 0;
        while revisions.len() < 4 {
            assert!(start.elapsed().as_secs() < 10, "continuous held-pointer edits never published four visible revisions: {revisions:?}");
            let d = app.camera_raw.as_mut().unwrap();
            d.params.clarity = 20.0;
            d.params.exposure = frames as f32 * 0.002;
            d.queue_render(&ctx);
            let events = if frames == 0 {
                vec![
                    egui::Event::PointerMoved(pointer),
                    egui::Event::PointerButton { pos: pointer, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
                ]
            } else {
                Vec::new()
            };
            let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(frames as f64 / 60.0), events, ..Default::default() }, |ui| {
                show(&mut app, ui.ctx())
            });
            output.textures_delta.clear();
            assert!(ctx.input(|input| input.pointer.any_down()));
            let d = app.camera_raw.as_ref().unwrap();
            assert!(!d.detail_allowed(&ctx), "the regression exercises live display while exact global refinement is gated");
            if d.detail.approximate() {
                revisions.insert(d.detail.displayed_revision().unwrap());
                assert!(!d.detail.ready);
                assert!(d.detail.displayed_revision().unwrap() <= d.preview_revision);
            }
            frames += 1;
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        let mut release = true;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            assert!(std::time::Instant::now() < deadline, "the exact engine result did not replace the live approximation");
            let events = if release {
                vec![egui::Event::PointerButton { pos: pointer, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE }]
            } else {
                Vec::new()
            };
            release = false;
            let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(frames as f64 / 60.0), events, ..Default::default() }, |ui| {
                show(&mut app, ui.ctx())
            });
            output.textures_delta.clear();
            let d = app.camera_raw.as_ref().unwrap();
            if d.detail.ready && d.preview_revision == d.requested_revision {
                assert!(!d.detail.approximate());
                assert_eq!(d.detail.displayed_revision(), Some(d.requested_revision));
                break;
            }
            frames += 1;
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    #[test]
    fn disconnected_proxy_reports_failure_instead_of_waiting_for_a_click() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width":8,"height":8})).unwrap();
        open(&mut app, &ctx).unwrap();
        let d = app.camera_raw.as_mut().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        d.proxy_pending = Some(rx);
        drop(tx);
        d.poll_render(&ctx);
        assert!(d.proxy_pending.is_none());
        assert!(d.preview_error.is_some());
    }

    #[test]
    fn comparison_and_floating_window_are_stable_without_document_edits() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":200,"height":150})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#808080"})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 950.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    show(app, &ctx);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        let ctx = h.ctx.clone();
        menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        let steps = h.state().session.active().unwrap().history.past_len();
        for mode in ["sideBySide", "topBottom", "single"] {
            menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui":{"view":{"comparison":mode,"swapped":true}}})).unwrap().unwrap();
            h.run_steps(4);
            let first = ctx.memory(|m| m.area_rect(egui::Id::new("camera-raw-dialog"))).unwrap();
            h.run_steps(8);
            let last = ctx.memory(|m| m.area_rect(egui::Id::new("camera-raw-dialog"))).unwrap();
            assert!((last.height() - first.height()).abs() < 1.0, "window grows: {first:?} -> {last:?}");
            assert!(last.width() < 1400.0 && last.height() < 950.0);
        }
        use egui_kittest::kittest::Queryable;
        let normal = ctx.memory(|m| m.area_rect(egui::Id::new("camera-raw-dialog"))).unwrap();
        h.get_by_label("Maximize Window").click();
        h.run_steps(4);
        let large = ctx.memory(|m| m.area_rect(egui::Id::new("camera-raw-dialog"))).unwrap();
        assert!(large.width() > normal.width());
        h.get_by_label("Restore Window").click();
        h.run_steps(4);
        let restored = ctx.memory(|m| m.area_rect(egui::Id::new("camera-raw-dialog"))).unwrap();
        assert!((restored.width() - normal.width()).abs() < 2., "{normal:?} -> {restored:?}");
        assert!((restored.height() - normal.height()).abs() < 2., "{normal:?} -> {restored:?}");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps);
    }

    #[test]
    fn histogram_follows_preview_and_view_changes_do_not_render_or_edit() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        let steps = app.session.active().unwrap().history.past_len();
        let initial = menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        assert_eq!(initial["histogram"]["samples"], 64 * 48);
        assert_eq!(initial["histogram"]["red"][128], 64 * 48);
        let updated = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0}}})).unwrap().unwrap();
        assert_ne!(updated["histogram"]["red"], initial["histogram"]["red"]);
        let before = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"before": true}})).unwrap().unwrap();
        assert_eq!(before["histogram"]["source"], "before");
        assert_eq!(before["histogram"]["red"], initial["histogram"]["red"]);
        assert_eq!(before["previewRevision"], updated["previewRevision"]);
        let after = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"before": false, "set": {"exposure": 1}}})).unwrap().unwrap();
        assert_eq!(after["histogram"], updated["histogram"]);
        assert_eq!(after["previewRevision"], updated["previewRevision"]);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        let pixel = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(0, 0);
        assert!((pixel[0] - 128.0 / 255.0).abs() < 0.0001);
    }

    #[test]
    fn smart_filter_double_click_edits_the_stored_camera_raw_filter_in_its_own_dialog() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        // An older editor could save stacked curve inputs; the filter must stay editable.
        let legacy = json!([[0.0, 0.0], [60.0, 40.0], [60.0, 200.0], [255.0, 255.0]]);
        app.run("filter.cameraRaw", json!({"exposure": 1.0, "contrast": 20})).unwrap();
        app.run("layer.smartFilter.setParams", json!({"index": 0, "params": {"pointCurve": legacy}})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let smart = |app: &PhotocraftApp| match &app.session.active().unwrap().doc.layer(layer).unwrap().content {
            photocraft_doc::LayerContent::Smart(sm) => sm.clone(),
            _ => panic!("not a smart object"),
        };
        let steps = app.session.active().unwrap().history.past_len();

        let sm = smart(&app);
        crate::smart_ui::open_editor(&mut app, &ctx, layer.0, &sm, 0);
        assert!(app.ui.dialogs.is_empty(), "no generic parameter form");
        let opened = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {}})).unwrap().unwrap();
        assert_eq!(opened["smartFilter"], 0);
        assert_eq!(opened["params"]["exposure"], 1.0);
        assert_eq!(opened["params"]["pointCurve"], legacy);
        // The preview starts from the pixels below the filter, not the filtered cache.
        let before = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"before": true}})).unwrap().unwrap();
        assert_eq!(before["histogram"]["red"][128], 64 * 48);

        // Resetting a control to its default must overwrite the stored value.
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 0.0}, "before": false, "commit": true}})).unwrap();
        assert!(r.is_ok(), "{r:?}");
        assert!(app.camera_raw.is_none());
        let sm = smart(&app);
        assert_eq!(sm.smart_filters.len(), 1, "edited in place, not added");
        assert_eq!(sm.smart_filters[0].params["exposure"], 0.0);
        assert_eq!(sm.smart_filters[0].params["contrast"], 20.0);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(smart(&app).smart_filters[0].params["exposure"], 1.0);

        // A broken new curve is still rejected; an unknown index is an error, not a panic.
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": layer.0, "index": 0}})).unwrap().unwrap();
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"pointCurve": [[10, 10]]}}})).unwrap().is_err());
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": layer.0, "index": 7}})).unwrap().is_err());
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"smartFilter": {"layer": "x"}})).unwrap().is_err());
    }

    #[test]
    fn distant_sparse_pixels_cannot_trigger_an_unbounded_proxy_scan() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width":64,"height":48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let mut source = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
        source.write_region(Rect::new(30_000, 30_000, 30_001, 30_001), &[0.5, 0.5, 0.5, 1.0]);
        let result = open_pixels(&mut app, &ctx, layer, source, CameraRaw::default(), Coverage::None, Target::NewFilter);
        assert!(result.unwrap_err().contains("bounds are too large"));
        assert!(app.camera_raw.is_none());
    }

    #[test]
    fn dialog_drives_the_engine() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        assert_eq!(r["proxy"], json!([64, 48]));
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": 1.0, "vignetteAmount": -40}}})).unwrap().unwrap();
        assert_eq!(r["params"]["exposure"], 1.0);
        assert_eq!(
            app.camera_raw.as_ref().unwrap().command_params(),
            json!({"exposure": 1.0, "vignetteAmount": -40.0,
                "colorSpaces":{"working":"document","output":"adobeRgb","sourceProfile":""}})
        );
        let before = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 24);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert!(app.camera_raw.is_none());
        let after = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().rgba(32, 24);
        assert!(after[0] > before[0] + 0.1, "{before:?} → {after:?}");
        // Cancel leaves the document alone.
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        let r = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"cancel": true}})).unwrap().unwrap();
        assert_eq!(r["cancelled"], true);
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui": {"set": {"exposure": "x"}}})).unwrap().is_err());
    }
}

#[cfg(test)]
#[path = "camera_raw_color_tests_v75.rs"]
mod color_tests_v75;
#[cfg(test)]
#[path = "camera_raw_lut_tests_v75.rs"]
mod lut_tests_v75;

#[cfg(test)]
#[path = "camera_raw_defaults_tests_v78.rs"]
mod defaults_tests_v78;

#[cfg(test)]
#[path = "camera_raw_tabs_tests_v78.rs"]
mod tabs_tests_v78;
