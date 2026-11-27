//! Full-resolution refinement of Camera Raw's interactive proxy. Native work is off the
//! UI thread; only one revision runs at a time, and obsolete results never replace new edits.
use egui::{Color32, ColorImage, Context, Pos2, Rect, TextureHandle, TextureOptions, pos2, vec2};
use photocraft_algo::camera_raw::CameraRaw;
use photocraft_geom::Rect as PixelRect;
use photocraft_raster::Surface;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{Receiver, TryRecvError},
};

#[derive(Clone)]
pub(crate) enum Coverage {
    Selection(Surface),
    Mask(photocraft_doc::LayerMask),
    None,
}

type DetailResult = Result<(u64, Surface), String>;
type OverlayKey = (u64, bool, PixelRect, crate::camera_raw_scope_ui::ClippingMode, crate::theme::ThemeKind);

/// Display-resolution processing carries the current colour adjustment. Reconstructing its
/// low-frequency difference over original source pixels preserves visible fine structure
/// during a global adjustment. This is an approximation, never an engine result/readout.
#[derive(Clone)]
struct InteractiveProxy {
    revision: u64,
    width: usize,
    height: usize,
    original: Arc<[[f32; 4]]>,
    processed: Arc<[[f32; 4]]>,
}

struct InteractiveFrame {
    revision: u64,
    crop: PixelRect,
    image: ColorImage,
    ms: f64,
}

type InteractiveResult = Result<InteractiveFrame, String>;

pub(crate) struct DetailPreview {
    source: Surface,
    area: PixelRect,
    coverage: Coverage,
    result: Option<(u64, Surface)>,
    result_area: PixelRect,
    pending_area: PixelRect,
    pending_work_area: PixelRect,
    pending_local: bool,
    pending: Option<(u64, Receiver<DetailResult>)>,
    cancel: Arc<AtomicBool>,
    pub(crate) error: Option<String>,
    failed_revision: Option<u64>,
    crop: Option<(u64, bool, PixelRect)>,
    texture: Option<TextureHandle>,
    before_texture: Option<TextureHandle>,
    before_crop: Option<(u64, bool, PixelRect)>,
    after_crop: Option<(u64, bool, PixelRect)>,
    overlay_texture: Option<TextureHandle>,
    overlay_key: Option<OverlayKey>,
    interactive_proxy: Option<InteractiveProxy>,
    interactive_pending: Option<Receiver<InteractiveResult>>,
    interactive_texture: Option<TextureHandle>,
    interactive_key: Option<(u64, PixelRect)>,
    interactive_failed: Option<(u64, PixelRect)>,
    interactive_floor: u64,
    interactive_ms: f64,
    interactive_updated_at: f64,
    approximate: bool,
    pub(crate) ready: bool,
}

impl Drop for DetailPreview {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl DetailPreview {
    pub(crate) fn new(source: Surface, area: PixelRect, coverage: Coverage) -> Self {
        Self {
            source,
            area,
            coverage,
            result: None,
            result_area: area,
            pending_area: area,
            pending_work_area: area,
            pending_local: false,
            pending: None,
            cancel: Arc::new(AtomicBool::new(false)),
            error: None,
            failed_revision: None,
            crop: None,
            texture: None,
            before_texture: None,
            before_crop: None,
            after_crop: None,
            overlay_texture: None,
            overlay_key: None,
            interactive_proxy: None,
            interactive_pending: None,
            interactive_texture: None,
            interactive_key: None,
            interactive_failed: None,
            interactive_floor: 0,
            interactive_ms: 0.0,
            interactive_updated_at: 0.0,
            approximate: false,
            ready: false,
        }
    }

    pub(crate) fn pending(&self) -> bool {
        self.pending.is_some()
    }

    pub(crate) fn displayed_revision(&self) -> Option<u64> {
        self.crop.map(|(revision, _, _)| revision)
    }

    pub(crate) fn approximate(&self) -> bool {
        self.approximate
    }

    pub(crate) fn interactive_pending(&self) -> bool {
        self.interactive_pending.is_some()
    }

    pub(crate) fn interactive_ms(&self) -> f64 {
        self.interactive_ms
    }

    pub(crate) fn clear_display(&mut self) {
        self.ready = false;
        self.approximate = false;
        self.crop = None;
    }

    pub(crate) fn poll_completed(&mut self, ctx: &Context, revision: u64) {
        self.poll(revision);
        self.poll_interactive(ctx);
    }

    pub(crate) fn set_interactive_proxy(&mut self, revision: u64, width: usize, height: usize, original: Arc<[[f32; 4]]>, processed: Arc<[[f32; 4]]>) {
        let replaced_quality = self.interactive_proxy.as_ref().is_some_and(|previous| {
            previous.revision == revision
                && (previous.width != width
                    || previous.height != height
                    || !Arc::ptr_eq(&previous.original, &original)
                    || !Arc::ptr_eq(&previous.processed, &processed))
        });
        if replaced_quality {
            // A reduced proxy and its restored display-resolution proxy share the
            // settings revision. Retire both completed and in-flight reconstruction
            // from the older source so (revision,crop) cannot keep it on top forever.
            // This drops only the bounded live worker's receiver, never the exact job.
            self.interactive_pending = None;
            self.interactive_key = None;
            self.interactive_failed = None;
            self.crop = None;
            self.ready = false;
            self.approximate = false;
        }
        self.interactive_proxy = Some(InteractiveProxy { revision, width, height, original, processed });
    }

    pub(crate) fn work_pixels(&self) -> u64 {
        if self.pending.is_some() { self.pending_work_area.width() as u64 * self.pending_work_area.height() as u64 } else { 0 }
    }

    /// Invalidate an in-flight refinement when interactive settings change.
    ///
    /// Retain the single worker and its actual revision. Full-image engine operations are
    /// not interruptible once entered, so dropping the receiver would only spawn more
    /// cancelled work and starve useful completion. Paint chooses the newest visible source.
    pub(crate) fn invalidate(&mut self) {
        self.ready = false;
        self.crop = None;
        self.overlay_key = None;
    }

    /// Moving back to finite-support effects must not wait for a previous full-image
    /// operation. Local work has its own serialized worker; detach only on this transition.
    pub(crate) fn prefer_local(&mut self) {
        if self.pending.is_some() && !self.pending_local {
            self.cancel.store(true, Ordering::Relaxed);
            self.pending = None;
            self.cancel = Arc::new(AtomicBool::new(false));
        }
    }

    fn poll(&mut self, revision: u64) {
        let Some((requested_revision, rx)) = &self.pending else { return };
        let requested_revision = *requested_revision;
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("Camera Raw detail preview worker stopped".into()),
        };
        self.pending = None;
        match result {
            Ok((r, surface)) if r <= revision && self.result.as_ref().is_none_or(|(displayed, _)| r >= *displayed) => {
                self.result = Some((r, surface));
                self.result_area = self.pending_area;
                self.crop = None;
                self.error = None;
            }
            Ok(_) => {} // A parameter changed while the worker was developing this revision.
            Err(e) => {
                if requested_revision == revision {
                    self.error = Some(e);
                }
                self.failed_revision = Some(requested_revision);
            }
        }
    }

    fn poll_interactive(&mut self, ctx: &Context) {
        let Some(rx) = self.interactive_pending.take() else { return };
        match rx.try_recv() {
            Ok(Ok(frame)) => {
                if frame.revision >= self.interactive_floor && self.interactive_key.is_none_or(|(revision, _)| frame.revision >= revision) {
                    match &mut self.interactive_texture {
                        Some(texture) => texture.set(frame.image, TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR }),
                        None => {
                            self.interactive_texture = Some(ctx.load_texture(
                                "cr-interactive-detail",
                                frame.image,
                                TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR },
                            ))
                        }
                    }
                    self.interactive_key = Some((frame.revision, frame.crop));
                    self.interactive_ms = frame.ms;
                    self.interactive_updated_at = ctx.input(|input| input.time);
                    self.interactive_failed = None;
                    ctx.request_repaint();
                }
            }
            Err(TryRecvError::Empty) => self.interactive_pending = Some(rx),
            Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                // The normal proxy remains available; do not turn an approximate display
                // failure into a permanent pending loop or a false exact-preview error.
            }
        }
    }

    fn request_interactive(&mut self, ctx: &Context, crop: PixelRect) {
        let Some(proxy) = &self.interactive_proxy else { return };
        let key = (proxy.revision, crop);
        if self.interactive_pending.is_some() || self.interactive_key == Some(key) || self.interactive_failed == Some(key) {
            return;
        }
        // An unusually large zoomed viewport keeps its existing display-sized proxy.
        // Neither this worker nor the UI ever allocates the full source image.
        if crop.width() as u64 * crop.height() as u64 > 4_194_304 {
            self.interactive_failed = Some(key);
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let source = self.source.clone();
            let domain = self.area;
            let coverage = self.coverage.clone();
            let proxy = proxy.clone();
            let repaint = ctx.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            match std::thread::Builder::new().name("camera-raw-live-detail".into()).spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let start = std::time::Instant::now();
                    let image = reconstruct(&source, domain, crop, &coverage, &proxy);
                    InteractiveFrame { revision: proxy.revision, crop, image, ms: start.elapsed().as_secs_f64() * 1000.0 }
                }))
                .map_err(|_| "Camera Raw interactive detail failed".to_owned());
                let _ = tx.send(result);
                repaint.request_repaint_of(egui::ViewportId::ROOT);
            }) {
                Ok(_) => {
                    self.interactive_pending = Some(rx);
                    self.interactive_failed = Some(key);
                }
                Err(_) => self.interactive_failed = Some(key),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            self.interactive_failed = Some(key);
        }
    }

    #[cfg(test)]
    fn request(&mut self, ctx: &Context, revision: u64, params: &CameraRaw) {
        self.request_region(ctx, revision, params, self.area);
    }

    fn request_region(&mut self, ctx: &Context, revision: u64, params: &CameraRaw, visible: PixelRect) {
        let mut params = params.clone();
        params.pixel_scale = 1.0;
        let work_area = local_work_area(&params, visible, self.area);
        let area = if work_area.is_some() { visible } else { self.area };
        let work_area = work_area.unwrap_or(self.area);
        if self.pending.is_some()
            || self.result.as_ref().is_some_and(|(r, _)| *r == revision && self.result_area.intersect(&area) == area)
            || self.failed_revision == Some(revision)
        {
            return;
        }
        // Keep the allocation and its real revision until its replacement is ready.
        // An old surface must never be uploaded using a newer revision's texture cache key.
        self.crop = None;
        self.error = None;
        // The engine's full-image pipeline needs multiple temporary float buffers. Never start
        // an unbounded allocation just because a huge sparse document was zoomed into.
        if work_area.width() as u64 * work_area.height() as u64 > 64_000_000 {
            self.error = Some("Full-resolution Camera Raw preview is limited to 64 megapixels; the proxy remains available".into());
            self.failed_revision = Some(revision);
            return;
        }
        let source = self.source.clone();
        self.pending_area = area;
        self.pending_work_area = work_area;
        self.pending_local = local_preview(&params);
        let coverage = self.coverage.clone();
        let cancel = self.cancel.clone();
        let ctx = ctx.clone();
        let (tx, rx) = std::sync::mpsc::channel::<DetailResult>();
        #[cfg(not(target_arch = "wasm32"))]
        let region_only = self.pending_local;
        let work = move || {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| develop(&source, work_area, &params, &coverage)))
                .map(|surface| (revision, surface))
                .map_err(|_| "Camera Raw detail preview failed; the proxy remains available".into());
            if !cancel.load(Ordering::Relaxed) {
                let _ = tx.send(result);
                ctx.request_repaint_of(egui::ViewportId::ROOT);
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            // A closed dialog can leave one non-interruptible engine stage finishing. Serialize
            // that work with the next dialog instead of multiplying its peak memory.
            static WORKER: std::sync::Mutex<()> = std::sync::Mutex::new(());
            static REGION_WORKER: std::sync::Mutex<()> = std::sync::Mutex::new(());
            match std::thread::Builder::new().name("camera-raw-preview".into()).spawn(move || {
                let _guard = if region_only { &REGION_WORKER } else { &WORKER }.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                work();
            }) {
                Ok(_) => self.pending = Some((revision, rx)),
                Err(e) => {
                    self.error = Some(format!("Cannot start Camera Raw preview: {e}"));
                    self.failed_revision = Some(revision);
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            // No browser worker/executor is available here. Preserve the interactive proxy
            // instead of blocking its UI with the full-image engine pipeline.
            drop(work);
            drop(rx);
            self.error = Some("Full-resolution filtered preview requires a native Camera Raw worker; the web preview keeps the proxy".into());
            self.failed_revision = Some(revision);
        }
    }

    /// Draw only the visible pixel rectangle. No full-image GPU texture and no GPU-size limit
    /// on the source image; uploads are capped to the current viewport plus a one-pixel border.
    pub(crate) fn paint(&mut self, ui: &egui::Ui, image: Rect, revision: u64, before: bool, params: &CameraRaw, allow_request: bool) -> bool {
        self.ready = false;
        self.approximate = false;
        self.poll(revision);
        self.poll_interactive(ui.ctx());
        let Some(crop) = visible_pixels(image, ui.clip_rect(), self.area, ui.input(|i| i.max_texture_side)) else { return false };
        let original = before || params.is_identity();
        if !before && params.is_identity() && self.result.as_ref().is_none_or(|(r, _)| *r != revision) {
            // A reset is itself the newest sharp after-frame. Retaining an older adjusted
            // surface here would flash back to it at the next non-neutral slider change.
            self.result = Some((revision, self.source.clone()));
            self.result_area = self.area;
            self.interactive_floor = revision;
            self.interactive_key = None;
        }
        // Finite-support processing needs only the visible crop plus its halo. Keep
        // this bounded native work flowing while held; full-domain work still waits
        // for the interaction to settle instead of competing with every proxy edit.
        let bounded_local = cfg!(not(target_arch = "wasm32"))
            && local_work_area(params, crop, self.area).is_some_and(|work| work.width() as u64 * work.height() as u64 <= 4_194_304);
        if !original && (allow_request || bounded_local) {
            self.request_region(ui.ctx(), revision, params, crop);
            self.poll(revision);
        }
        // A newer display-sized result must not be hidden by an older exact surface.
        // Until the full-domain engine catches up, reconstruct over source-resolution
        // pixels in one bounded worker, and keep its honest approximate/revision metadata.
        let proxy_revision = self.interactive_proxy.as_ref().map_or(0, |proxy| proxy.revision);
        let exact_current = self.result.as_ref().is_some_and(|(r, _)| *r >= proxy_revision);
        let preserve_source_detail = params.sharpen_amount == 0.0
            && params.noise_luminance == 0.0
            && params.noise_color == 0.0
            && params.grain_amount == 0.0
            && params.local_adjustments.is_empty();
        if !original && !exact_current && self.interactive_proxy.is_some() && preserve_source_detail {
            self.request_interactive(ui.ctx(), crop);
            if self.interactive_pending() {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
            }
            if let Some((r, live_crop)) = self.interactive_key
                && r <= revision
                && (r == proxy_revision || self.interactive_pending() && ui.input(|input| input.time) - self.interactive_updated_at <= 0.2)
                && self.result.as_ref().is_none_or(|(exact_revision, _)| r >= *exact_revision)
                && let Some(texture) = &self.interactive_texture
            {
                let available = crop.intersect(&live_crop);
                if !available.is_empty() {
                    let scale = image.size() / vec2(self.area.width() as f32, self.area.height() as f32);
                    let screen = Rect::from_min_size(
                        image.min + vec2((live_crop.x0 - self.area.x0) as f32, (live_crop.y0 - self.area.y0) as f32) * scale,
                        vec2(live_crop.width() as f32, live_crop.height() as f32) * scale,
                    );
                    ui.painter().image(texture.id(), screen, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
                    self.crop = Some((r, false, available));
                    self.approximate = true;
                    return true;
                }
            }
            self.crop = None;
            return false;
        }
        // Keep the last sharp frame while the next revision is computing. Its real revision
        // remains in the texture key; it is never labelled/sampled as the latest settings.
        let requested_crop = crop;
        let (surface, displayed_revision, crop) = if original {
            (&self.source, 0, crop)
        } else if let Some((r, surface)) = &self.result {
            let available = crop.intersect(&self.result_area);
            if available.is_empty() {
                return false;
            }
            (surface, *r, available)
        } else if self.before_texture.is_some() || !preserve_source_detail {
            (&self.source, 0, crop)
        } else {
            return false;
        };
        let key = (displayed_revision, original, crop);
        let cached_crop = if original { &mut self.before_crop } else { &mut self.after_crop };
        let texture = if original { &mut self.before_texture } else { &mut self.texture };
        if *cached_crop != Some(key) {
            let color = crop_image(surface, crop);
            let options = TextureOptions { magnification: egui::TextureFilter::Nearest, ..TextureOptions::LINEAR };
            match texture {
                Some(t) => t.set(color, options),
                None => *texture = Some(ui.ctx().load_texture(if original { "cr-detail-before" } else { "cr-detail-after" }, color, options)),
            }
            *cached_crop = Some(key);
        }
        self.crop = Some(key);
        if let Some(texture) = texture {
            let scale = image.size() / vec2(self.area.width() as f32, self.area.height() as f32);
            let screen = Rect::from_min_size(
                image.min + vec2((crop.x0 - self.area.x0) as f32, (crop.y0 - self.area.y0) as f32) * scale,
                vec2(crop.width() as f32, crop.height() as f32) * scale,
            );
            ui.painter().image(texture.id(), screen, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
            self.ready = original || displayed_revision == revision && crop == requested_crop;
        }
        true
    }
    pub(crate) fn sample(&self, p: [f32; 2], revision: u64, before: bool, params: &CameraRaw) -> Option<[f32; 4]> {
        if !self.ready || !p.iter().all(|v| v.is_finite() && (0.0..=1.0).contains(v)) {
            return None;
        }
        let source = if before || params.is_identity() {
            &self.source
        } else if let Some((r, source)) = &self.result
            && *r == revision
        {
            source
        } else {
            return None;
        };
        let x = self.area.x0 + ((p[0] * self.area.width() as f32) as i32).min((self.area.width() - 1) as i32);
        let y = self.area.y0 + ((p[1] * self.area.height() as f32) as i32).min((self.area.height() - 1) as i32);
        if !before && !params.is_identity() && !(self.result_area.x0..self.result_area.x1).contains(&x) {
            return None;
        }
        if !before && !params.is_identity() && !(self.result_area.y0..self.result_area.y1).contains(&y) {
            return None;
        }
        Some(source.rgba(x, y))
    }

    pub(crate) fn overlay(&mut self, ui: &egui::Ui, image: Rect, revision: u64, mode: crate::camera_raw_scope_ui::ClippingMode) -> bool {
        use crate::camera_raw_scope_ui::ClippingMode;
        if !self.ready {
            return false;
        }
        let Some((r, original, crop)) = self.crop else { return false };
        let t = crate::theme::Tokens::get(ui.ctx());
        let key = (r, original, crop, mode, t.kind);
        if self.overlay_key != Some(key) {
            let surface = if original {
                &self.source
            } else if let Some((r, surface)) = &self.result
                && *r == revision
            {
                surface
            } else {
                return false;
            };
            let mut row = vec![[0.0; 4]; crop.width() as usize];
            let mut pixels = Vec::with_capacity(crop.width() as usize * crop.height() as usize);
            for y in crop.y0..crop.y1 {
                surface.read_rgba_into(PixelRect::new(crop.x0, y, crop.x1, y + 1), &mut row);
                pixels.extend(row.iter().map(|p| {
                    if p[3] <= 0.0 || !p.iter().all(|v| v.is_finite()) {
                        return Color32::TRANSPARENT;
                    }
                    let (lo, hi) = photocraft_algo::histogram::clipping(*p);
                    match mode {
                        ClippingMode::ShadowChannels => t.histogram_color(7 ^ lo),
                        ClippingMode::HighlightChannels => {
                            if hi == 0 {
                                t.histogram_background()
                            } else {
                                t.histogram_color(hi)
                            }
                        }
                        ClippingMode::Warnings(w) => {
                            if w & 2 != 0 && hi != 0 {
                                t.histogram_color(1)
                            } else if w & 1 != 0 && lo != 0 {
                                t.histogram_color(4)
                            } else {
                                Color32::TRANSPARENT
                            }
                        }
                        ClippingMode::None => Color32::TRANSPARENT,
                    }
                }));
            }
            let color = ColorImage::new([crop.width() as usize, crop.height() as usize], pixels);
            match &mut self.overlay_texture {
                Some(t) => t.set(color, TextureOptions::NEAREST),
                None => self.overlay_texture = Some(ui.ctx().load_texture("cr-detail-clipping", color, TextureOptions::NEAREST)),
            }
            self.overlay_key = Some(key);
        }
        if let Some(texture) = &self.overlay_texture {
            let scale = image.size() / vec2(self.area.width() as f32, self.area.height() as f32);
            let screen = Rect::from_min_size(
                image.min + vec2((crop.x0 - self.area.x0) as f32, (crop.y0 - self.area.y0) as f32) * scale,
                vec2(crop.width() as f32, crop.height() as f32) * scale,
            );
            ui.painter().image(texture.id(), screen, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        }
        true
    }
}

/// These stages depend only on each pixel's colour. A visible crop therefore produces
/// exactly the same pixels as the full-image engine, including depth and coverage.
/// Spatial/global stages deliberately keep the complete domain (e.g. dehaze atmosphere,
/// guided-filter radii, vignette centre and grain coordinates).
#[cfg(test)]
fn pointwise(params: &CameraRaw) -> bool {
    params.dehaze == 0.0
        && params.highlights == 0.0
        && params.shadows == 0.0
        && params.texture == 0.0
        && params.clarity == 0.0
        && params.sharpen_amount == 0.0
        && params.noise_luminance == 0.0
        && params.noise_color == 0.0
        && params.grain_amount == 0.0
        && params.vignette_amount == 0.0
        && params.local_adjustments.is_empty()
}

/// Local spatial stages retain all source pixels they need. They can update while a slider
/// is held just like pointwise stages; dimension-dependent/global effects remain full-domain.
pub(crate) fn local_preview(params: &CameraRaw) -> bool {
    photocraft_algo::camera_raw::preview_halo(params).is_some()
}

fn local_work_area(params: &CameraRaw, visible: PixelRect, domain: PixelRect) -> Option<PixelRect> {
    let halo = i32::try_from(photocraft_algo::camera_raw::preview_halo(params)?).ok()?;
    Some(
        PixelRect::new(visible.x0.saturating_sub(halo), visible.y0.saturating_sub(halo), visible.x1.saturating_add(halo), visible.y1.saturating_add(halo))
            .intersect(&domain),
    )
}

fn develop(source: &Surface, area: PixelRect, params: &CameraRaw, coverage: &Coverage) -> Surface {
    let mut result = photocraft_engine::lens_cmds::camera_raw_surface(source, area, params);
    if matches!(coverage, Coverage::None) {
        return result;
    }
    let width = area.width() as usize;
    let channels = source.channels();
    let mut original = Vec::new();
    let mut row = Vec::new();
    for y in area.y0..area.y1 {
        let rect = PixelRect::new(area.x0, y, area.x1, y + 1);
        source.read_region_into(rect, &mut original);
        result.read_region_into(rect, &mut row);
        for (x, (out, original)) in row.chunks_exact_mut(channels).zip(original.chunks_exact(channels)).take(width).enumerate() {
            let x = area.x0 + x as i32;
            let alpha = match coverage {
                Coverage::Selection(mask) => mask.sample_channel(x, y, 0),
                Coverage::Mask(mask) => mask.value(x, y),
                Coverage::None => 1.0,
            }
            .clamp(0.0, 1.0);
            for (out, original) in out.iter_mut().zip(original) {
                *out = *original + (*out - *original) * alpha;
            }
        }
        result.write_region(rect, &row);
    }
    result
}

/// Lift the newest display-resolution colour difference onto original high-frequency
/// source pixels. This does not recompute full-domain kernels: it is deliberately a live
/// approximation, replaced by `develop` when the exact revision completes. Selection and
/// mask coverage are applied at the real source pixel, never interpolated across a mask edge.
fn reconstruct(source: &Surface, domain: PixelRect, crop: PixelRect, coverage: &Coverage, proxy: &InteractiveProxy) -> ColorImage {
    reconstruct_with_parallel(source, domain, crop, coverage, proxy, true)
}

fn reconstruct_with_parallel(
    source: &Surface,
    domain: PixelRect,
    crop: PixelRect,
    coverage: &Coverage,
    proxy: &InteractiveProxy,
    parallel: bool,
) -> ColorImage {
    #[cfg(target_arch = "wasm32")]
    let _ = parallel;
    if proxy.width == 0
        || proxy.height == 0
        || proxy.width.checked_mul(proxy.height) != Some(proxy.original.len())
        || proxy.original.len() != proxy.processed.len()
    {
        return crop_image(source, crop);
    }
    let axis = |coordinate: i32, origin: i32, extent: u32, bins: usize| {
        let position = (((coordinate as f64 - origin as f64 + 0.5) * bins as f64 / extent.max(1) as f64) - 0.5).clamp(0.0, (bins - 1) as f64);
        let lo = position.floor() as usize;
        (lo, (lo + 1).min(bins - 1), (position - lo as f64) as f32)
    };
    let horizontal: Vec<_> = (crop.x0..crop.x1).map(|x| axis(x, domain.x0, domain.width(), proxy.width)).collect();
    let delta = |x: usize, y: usize| {
        let index = y * proxy.width + x;
        match (proxy.original.get(index), proxy.processed.get(index)) {
            (Some(before), Some(after)) => [after[0] - before[0], after[1] - before[1], after[2] - before[2]],
            _ => [0.0; 3],
        }
    };
    let width = crop.width() as usize;
    let mut pixels = vec![Color32::TRANSPARENT; width.saturating_mul(crop.height() as usize)];
    let enc = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let fill = |row_index: usize, output: &mut [Color32], row: &mut Vec<[f32; 4]>| {
        let y = crop.y0 + row_index as i32;
        source.read_rgba_into(PixelRect::new(crop.x0, y, crop.x1, y + 1), row);
        let (y0, y1, ty) = axis(y, domain.y0, domain.height(), proxy.height);
        for (offset, ((pixel, &(x0, x1, tx)), out)) in row.iter().zip(&horizontal).zip(output).enumerate() {
            let lo0 = delta(x0, y0);
            let lo1 = delta(x1, y0);
            let hi0 = delta(x0, y1);
            let hi1 = delta(x1, y1);
            let x = crop.x0 + offset as i32;
            let coverage = match coverage {
                Coverage::None => 1.0,
                Coverage::Selection(mask) => mask.sample_channel(x, y, 0),
                Coverage::Mask(mask) => mask.value(x, y),
            }
            .clamp(0.0, 1.0);
            let mut color = [0.0; 3];
            for (channel, value) in color.iter_mut().enumerate() {
                let low = lo0[channel] + (lo1[channel] - lo0[channel]) * tx;
                let high = hi0[channel] + (hi1[channel] - hi0[channel]) * tx;
                *value = pixel[channel] + (low + (high - low) * ty) * coverage;
            }
            if source.format().mode == photocraft_color::ColorMode::Grayscale {
                color = [photocraft_color::convert::rgb_to_gray(color); 3];
            }
            *out = Color32::from_rgba_unmultiplied(enc(color[0]), enc(color[1]), enc(color[2]), enc(pixel[3]));
        }
    };
    if width == 0 {
        return ColorImage::new([width, crop.height() as usize], pixels);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if parallel && pixels.len() > 65_536 {
        use rayon::prelude::*;
        pixels.par_chunks_mut(width).with_min_len(16).enumerate().for_each_init(|| vec![[0.0; 4]; width], |row, (index, output)| fill(index, output, row));
        return ColorImage::new([width, crop.height() as usize], pixels);
    }
    let mut row = vec![[0.0; 4]; width];
    for (index, output) in pixels.chunks_mut(width).enumerate() {
        fill(index, output, &mut row);
    }
    ColorImage::new([width, crop.height() as usize], pixels)
}

fn visible_pixels(image: Rect, viewport: Rect, area: PixelRect, max_side: usize) -> Option<PixelRect> {
    let visible = image.intersect(viewport);
    if !image.is_finite() || image.width() <= 0.0 || image.height() <= 0.0 || !visible.is_positive() || area.is_empty() {
        return None;
    }
    let scale = vec2(area.width() as f32, area.height() as f32) / image.size();
    let lo = (visible.min - image.min) * scale;
    let hi = (visible.max - image.min) * scale;
    let crop = PixelRect::new(
        area.x0.saturating_add((lo.x.floor() as i32).saturating_sub(1)),
        area.y0.saturating_add((lo.y.floor() as i32).saturating_sub(1)),
        area.x0.saturating_add((hi.x.ceil() as i32).saturating_add(1)),
        area.y0.saturating_add((hi.y.ceil() as i32).saturating_add(1)),
    )
    .intersect(&area);
    // Proxy is sufficient below its native scale; a larger intermediate should never be
    // allocated if an unusually large viewport exceeds the device's texture limit.
    (crop.width() as usize <= max_side && crop.height() as usize <= max_side && crop.width() as u64 * crop.height() as u64 <= 16_777_216).then_some(crop)
}

fn crop_image(surface: &Surface, area: PixelRect) -> ColorImage {
    let width = area.width() as usize;
    let mut row = vec![[0.0; 4]; width];
    let mut pixels = Vec::with_capacity(width * area.height() as usize);
    let enc = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    for y in area.y0..area.y1 {
        surface.read_rgba_into(PixelRect::new(area.x0, y, area.x1, y + 1), &mut row);
        pixels.extend(row.iter().map(|q| Color32::from_rgba_unmultiplied(enc(q[0]), enc(q[1]), enc(q[2]), enc(q[3]))));
    }
    ColorImage::new([width, area.height() as usize], pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::PixelFormat;

    #[test]
    fn held_detail_edits_request_bounded_exact_pixels_and_never_restore_original_noise() {
        let area = PixelRect::new(0, 0, 32, 24);
        let mut source = Surface::new(PixelFormat::RGBA8);
        let values: Vec<f32> = (0..32 * 24)
            .flat_map(|i| {
                let v = if i % 2 == 0 { 0.2 } else { 0.7 };
                [v, v, v, 1.0]
            })
            .collect();
        source.write_region(area, &values);
        let params = CameraRaw { noise_luminance: 80.0, ..Default::default() };
        let mut preview = DetailPreview::new(source.clone(), area, Coverage::None);
        let old = develop(&source, area, &params, &Coverage::None);
        preview.result = Some((1, old.clone()));
        preview.result_area = area;
        preview.set_interactive_proxy(2, 1, 1, vec![[0.5; 4]].into(), vec![[0.6; 4]].into());
        let ctx = Context::default();
        let image = Rect::from_min_size(pos2(10.0, 10.0), vec2(320.0, 240.0));
        let mut output = ctx.run_ui(Default::default(), |ui| {
            assert!(preview.paint(ui, image, 2, false, &params, false));
            assert!(!preview.approximate, "NR must not restore original high-frequency noise");
        });
        output.textures_delta.clear();
        assert!(preview.pending() || preview.result.as_ref().is_some_and(|(r, _)| *r == 2), "finite crop starts even while the pointer is held");
        assert!(!preview.interactive_pending());
        assert!(preview.work_pixels() <= 4_194_304);
        assert!(preview.after_crop.is_some_and(|(r, _, _)| r == 1 || r == 2), "sharp frame keeps its true revision");
    }

    #[test]
    fn native_detail_reconstruction_rows_match_serial_for_source_alpha_gray_and_mask_edges() {
        let area = PixelRect::new(10, -20, 810, 580);
        for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::GRAY8] {
            let mut source = Surface::with_default(format, &[0.2, 0.4, 0.6, 0.75]);
            source.fill_rect(PixelRect::new(100, 100, 500, 400), &[0.7, 0.2, 0.1, 0.5]);
            let mut mask = Surface::new(PixelFormat::GRAY8);
            mask.fill_rect(PixelRect::new(400, -20, 810, 580), &[0.5]);
            let proxy = InteractiveProxy {
                revision: 2,
                width: 3,
                height: 2,
                original: vec![[0.25, 0.5, 1.5, 0.7]; 6].into(),
                processed: (0..6).map(|i| [0.4 + i as f32 * 0.1, 0.3, 1.6, 0.0]).collect::<Vec<_>>().into(),
            };
            let coverage = Coverage::Selection(mask);
            let parallel = reconstruct(&source, area, area, &coverage, &proxy);
            let serial = reconstruct_with_parallel(&source, area, area, &coverage, &proxy, false);
            assert_eq!(parallel, serial, "{format:?}");
        }
    }

    #[test]
    fn actual_dialog_sync_and_async_proxy_reconstruction_apply_half_selection_once_and_keep_gray_gray() {
        use serde_json::json;
        for gray in [false, true] {
            let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            let ctx = Context::default();
            app.run("file.new", json!({"width":64,"height":48})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("edit.fill", json!({"color":"#405060"})).unwrap();
            if gray {
                app.run("image.mode.grayscale", json!({})).unwrap();
            }
            let mut selection = Surface::new(PixelFormat::GRAY8);
            selection.fill_rect(PixelRect::new(0, 0, 32, 48), &[0.5]);
            Arc::make_mut(&mut app.session.active_mut().unwrap().doc).selection = Some(selection);
            crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
            for asynchronous in [false, true] {
                crate::camera_raw_ui::menu(
                    &mut app,
                    &ctx,
                    "filter.cameraRaw",
                    &json!({"ui":{"async":asynchronous,"set":{
                        "exposure":if asynchronous {0.8} else {0.5},"temperature":30.0,"clarity":15.0
                    }}}),
                )
                .unwrap()
                .unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                let d = app.camera_raw.as_mut().unwrap();
                while d.preview_revision != d.requested_revision {
                    assert!(std::time::Instant::now() < deadline);
                    d.poll_render(&ctx);
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                let preview = &d.detail;
                let proxy = preview.interactive_proxy.as_ref().unwrap();
                assert_eq!(proxy.revision, d.preview_revision);
                let live = reconstruct(&preview.source, preview.area, preview.area, &preview.coverage, proxy);
                let expected = develop(&preview.source, preview.area, &d.params, &preview.coverage);
                let expected = crop_image(&expected, preview.area);
                for (a, b) in live.pixels.iter().zip(&expected.pixels) {
                    for (a, b) in a.to_array().iter().zip(b.to_array()) {
                        assert!(a.abs_diff(b) <= 2, "gray={gray}, async={asynchronous}: coverage must be applied once, actual={a}, expected={b}");
                    }
                }
                assert_eq!(live.pixels[48], crop_image(&preview.source, preview.area).pixels[48], "zero coverage remains exactly original");
                assert!(live.pixels[8].r() > crop_image(&preview.source, preview.area).pixels[8].r() + 5);
                if gray {
                    assert!(live.pixels.iter().all(|p| p.r() == p.g() && p.g() == p.b()));
                    assert!(d.processed.iter().all(|p| p[0] == p[1] && p[1] == p[2]));
                }
            }
        }
    }

    #[test]
    fn live_reconstruction_preserves_source_lines_alpha_and_exact_coverage_across_depths() {
        use photocraft_color::{ColorMode, SampleType};
        let domain = PixelRect::new(-4, 7, 4, 13);
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
                let format = PixelFormat { mode, sample, alpha: true };
                let channels = format.channels();
                let values: Vec<f32> = (0..48)
                    .flat_map(|i| {
                        (0..channels).map(move |channel| {
                            if channel + 1 == channels {
                                if i % 5 == 0 { 0.0 } else { 0.5 }
                            } else if i % 2 == 0 {
                                0.15
                            } else {
                                0.65
                            }
                        })
                    })
                    .collect();
                let mut source = Surface::new(format);
                source.write_region(domain, &values);
                let mut selection = Surface::new(PixelFormat::GRAY8);
                selection.fill_rect(PixelRect::new(0, 7, 4, 13), &[1.0]);
                let proxy = InteractiveProxy {
                    revision: 2,
                    width: 2,
                    height: 2,
                    original: vec![[0.4, 0.4, 0.4, 0.5]; 4].into(),
                    processed: vec![[0.5, 0.5, 0.5, 0.5]; 4].into(),
                };
                for coverage in [
                    Coverage::None,
                    Coverage::Selection(selection.clone()),
                    Coverage::Mask(photocraft_doc::LayerMask { surface: selection.clone(), ..photocraft_doc::LayerMask::reveal_all() }),
                ] {
                    let live = reconstruct(&source, domain, domain, &coverage, &proxy);
                    for y in domain.y0..domain.y1 {
                        for x in domain.x0..domain.x1 {
                            let src = source.rgba(x, y);
                            let c = match &coverage {
                                Coverage::None => 1.0,
                                Coverage::Selection(mask) => mask.sample_channel(x, y, 0),
                                Coverage::Mask(mask) => mask.value(x, y),
                            };
                            let enc = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                            let expected = Color32::from_rgba_unmultiplied(enc(src[0] + 0.1 * c), enc(src[1] + 0.1 * c), enc(src[2] + 0.1 * c), enc(src[3]));
                            let actual = live.pixels[(y - domain.y0) as usize * 8 + (x - domain.x0) as usize];
                            for (a, b) in actual.to_array().iter().zip(expected.to_array()) {
                                assert!(a.abs_diff(b) <= 1, "{mode:?}/{sample:?} at {x},{y}: {actual:?} vs {expected:?}");
                            }
                        }
                    }
                    let dark = live.pixels[6].to_srgba_unmultiplied();
                    let light = live.pixels[7].to_srgba_unmultiplied();
                    assert!(light[0] as i16 - dark[0] as i16 > 120, "alternating source pixels survive the low-frequency adjustment");
                }
                let neutral = InteractiveProxy { processed: proxy.original.clone(), ..proxy };
                assert_eq!(reconstruct(&source, domain, domain, &Coverage::None, &neutral), crop_image(&source, domain));
            }
        }
    }

    #[test]
    fn fresh_proxy_is_not_hidden_by_stale_exact_and_live_never_overrides_reset_or_newer_exact() {
        let domain = PixelRect::new(0, 0, 16, 16);
        let source = Surface::with_default(PixelFormat::RGBA8, &[0.2, 0.4, 0.6, 1.0]);
        let mut preview = DetailPreview::new(source.clone(), domain, Coverage::None);
        let original: Arc<[[f32; 4]]> = vec![[0.2, 0.4, 0.6, 1.0]; 4].into();
        preview.result = Some((1, source.clone()));
        preview.set_interactive_proxy(2, 2, 2, original.clone(), vec![[0.3, 0.5, 0.7, 1.0]; 4].into());
        let (tx, rx) = std::sync::mpsc::channel();
        preview.interactive_pending = Some(rx);
        let params = CameraRaw { clarity: 20.0, exposure: 0.5, ..Default::default() };
        let ctx = Context::default();
        let image = Rect::from_min_size(pos2(10.0, 10.0), vec2(200.0, 200.0));
        let mut output = ctx.run_ui(Default::default(), |ui| {
            assert!(!preview.paint(ui, image, 2, false, &params, false), "the freshly painted proxy must remain visible until live detail is ready");
        });
        output.textures_delta.clear();
        tx.send(Ok(InteractiveFrame {
            revision: 2,
            crop: domain,
            image: reconstruct(&source, domain, domain, &Coverage::None, preview.interactive_proxy.as_ref().unwrap()),
            ms: 1.0,
        }))
        .unwrap();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            assert!(preview.paint(ui, image, 2, false, &params, false));
            assert!(preview.approximate());
            assert_eq!(preview.displayed_revision(), Some(2));
            assert!(!preview.ready);
            assert!(preview.sample([0.5, 0.5], 2, false, &params).is_none(), "scope probes never read reconstructed pixels as exact");
            assert!(preview.paint(ui, image, 2, true, &params, false));
            assert!(!preview.approximate());
        });
        output.textures_delta.clear();
        preview.result = Some((3, source.clone()));
        preview.set_interactive_proxy(3, 2, 2, original.clone(), original.clone());
        let mut output = ctx.run_ui(Default::default(), |ui| {
            assert!(preview.paint(ui, image, 3, false, &params, false));
            assert!(preview.ready);
            assert!(!preview.approximate());
            assert_eq!(preview.displayed_revision(), Some(3));
            assert!(preview.paint(ui, image, 4, false, &CameraRaw::default(), false));
        });
        output.textures_delta.clear();
        assert!(preview.interactive_key.is_none());
        assert_eq!(preview.interactive_floor, 4);
        let (tx, rx) = std::sync::mpsc::channel();
        preview.interactive_pending = Some(rx);
        tx.send(Ok(InteractiveFrame { revision: 3, crop: domain, image: crop_image(&source, domain), ms: 1.0 })).unwrap();
        preview.poll_interactive(&ctx);
        assert!(preview.interactive_key.is_none(), "a late approximate worker cannot resurrect pixels older than Reset");
    }

    #[test]
    fn switching_from_global_back_to_local_does_not_wait_for_the_global_worker() {
        let source = Surface::new(PixelFormat::RGBA8);
        let mut preview = DetailPreview::new(source, PixelRect::new(0, 0, 6000, 4000), Coverage::None);
        let (_tx, rx) = std::sync::mpsc::channel();
        preview.pending = Some((1, rx));
        let token = preview.cancel.clone();
        preview.invalidate();
        assert!(preview.pending());
        preview.prefer_local();
        assert!(!preview.pending());
        assert!(token.load(Ordering::Relaxed));
        assert!(!preview.cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn same_revision_source_quality_retires_cached_and_pending_live_reconstruction() {
        for old_pending in [false, true] {
            let domain = PixelRect::new(0, 0, 16, 16);
            let source = Surface::with_default(PixelFormat::RGBA8, &[0.2, 0.4, 0.6, 1.0]);
            let mut preview = DetailPreview::new(source.clone(), domain, Coverage::None);
            preview.set_interactive_proxy(7, 2, 2, vec![[0.2, 0.4, 0.6, 1.0]; 4].into(), vec![[0.3, 0.5, 0.7, 1.0]; 4].into());
            let ctx = Context::default();
            let old_image = reconstruct(&source, domain, domain, &Coverage::None, preview.interactive_proxy.as_ref().unwrap());
            preview.interactive_texture = Some(ctx.load_texture("old-live", old_image.clone(), TextureOptions::LINEAR));
            preview.interactive_key = Some((7, domain));
            preview.interactive_failed = Some((7, domain));
            preview.crop = Some((7, false, domain));
            let (tx, rx) = std::sync::mpsc::channel();
            if old_pending {
                preview.interactive_pending = Some(rx);
            }
            preview.set_interactive_proxy(7, 4, 4, vec![[0.2, 0.4, 0.6, 1.0]; 16].into(), vec![[0.5, 0.7, 0.9, 1.0]; 16].into());
            assert!(preview.interactive_key.is_none());
            assert!(!preview.interactive_pending());
            assert!(preview.interactive_failed.is_none());
            assert!(preview.displayed_revision().is_none());
            if old_pending {
                assert!(
                    tx.send(Ok(InteractiveFrame { revision: 7, crop: domain, image: old_image.clone(), ms: 1.0 })).is_err(),
                    "late low-quality worker must have no receiver"
                );
            }
            let expected = reconstruct(&source, domain, domain, &Coverage::None, preview.interactive_proxy.as_ref().unwrap());
            assert_ne!(expected, old_image);
            let image = Rect::from_min_size(pos2(10.0, 10.0), vec2(200.0, 200.0));
            let params = CameraRaw { exposure: 0.5, clarity: 10.0, ..Default::default() };
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut uploaded = false;
            while preview.interactive_key != Some((7, domain)) {
                assert!(std::time::Instant::now() < deadline, "restored source quality remained stuck behind old reconstruction");
                let mut output = ctx.run_ui(Default::default(), |ui| {
                    preview.paint(ui, image, 7, false, &params, false);
                });
                for updates in output.textures_delta.set.values() {
                    for delta in updates {
                        let egui::ImageData::Color(pixels) = &delta.image;
                        uploaded |= pixels.as_ref() == &expected;
                    }
                }
                output.textures_delta.clear();
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            assert!(uploaded, "the restored proxy must cause a new texture upload at the same settings revision");
            assert!(preview.approximate());
            assert!(!preview.ready, "restored live detail is still an approximation");
            assert!(!preview.pending(), "this regression must not be rescued by exact refinement");
        }
    }

    #[test]
    fn failed_over_budget_or_slow_live_reconstruction_cannot_permanently_cover_a_fresher_proxy() {
        let domain = PixelRect::new(0, 0, 2100, 2100);
        let source = Surface::with_default(PixelFormat::RGBA8, &[0.2, 0.4, 0.6, 1.0]);
        let mut preview = DetailPreview::new(source, domain, Coverage::None);
        let original: Arc<[[f32; 4]]> = vec![[0.2, 0.4, 0.6, 1.0]; 4].into();
        preview.set_interactive_proxy(2, 2, 2, original.clone(), original);
        let ctx = Context::default();
        preview.interactive_texture = Some(ctx.load_texture("test-live-fallback", ColorImage::filled([2, 2], Color32::GRAY), TextureOptions::LINEAR));
        preview.interactive_key = Some((1, domain));
        let params = CameraRaw { clarity: 10.0, ..Default::default() };
        let image = Rect::from_min_size(pos2(10.0, 10.0), vec2(2100.0, 2100.0));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(2200.0, 2200.0))),
                max_texture_side: Some(4096),
                time: Some(1.0),
                ..Default::default()
            },
            |ui| {
                assert!(!preview.paint(ui, image, 2, false, &params, false), "over-budget reconstruction leaves the newest ordinary proxy visible");
            },
        );
        output.textures_delta.clear();
        assert!(preview.interactive_failed.is_some());
        assert!(!preview.interactive_pending());

        // Keep a worker unfinished while parameters and UI time advance. The sharp old
        // approximation is useful for at most 200 ms; it must then reveal the fresh proxy.
        let (_tx, rx) = std::sync::mpsc::channel();
        preview.interactive_pending = Some(rx);
        preview.interactive_updated_at = 1.0;
        let mut output = ctx.run_ui(egui::RawInput { time: Some(1.1), max_texture_side: Some(4096), ..Default::default() }, |ui| {
            assert!(preview.paint(ui, image, 2, false, &params, false));
            assert!(preview.approximate());
        });
        output.textures_delta.clear();
        let mut output = ctx.run_ui(egui::RawInput { time: Some(1.3), max_texture_side: Some(4096), ..Default::default() }, |ui| {
            assert!(!preview.paint(ui, image, 2, false, &params, false), "a slow worker cannot freeze a newer proxy behind old live pixels");
        });
        output.textures_delta.clear();
        preview.interactive_pending = None;
        let mut output = ctx.run_ui(egui::RawInput { time: Some(1.4), max_texture_side: Some(4096), ..Default::default() }, |ui| {
            assert!(!preview.paint(ui, image, 2, false, &params, false), "failure must fall back too");
        });
        output.textures_delta.clear();
    }
    #[test]
    fn pointwise_visible_region_matches_full_engine_across_depths_and_selection() {
        use photocraft_color::{ColorMode, SampleType};
        let area = PixelRect::new(-20, 11, 76, 91);
        let crop = PixelRect::new(5, 26, 29, 46);
        let params = CameraRaw {
            exposure: 0.7,
            contrast: 20.0,
            temperature: -18.0,
            whites: 12.0,
            blacks: -10.0,
            saturation: 8.0,
            vibrance: 14.0,
            point_curve: vec![[0.0, 0.0], [100.0, 120.0], [255.0, 255.0]],
            point_curve_red: vec![[0.0, 0.0], [128.0, 170.0], [255.0, 255.0]],
            point_curve_green: vec![[0.0, 0.0], [128.0, 90.0], [255.0, 255.0]],
            point_curve_blue: vec![[0.0, 15.0], [128.0, 115.0], [255.0, 245.0]],
            ..Default::default()
        };
        assert!(pointwise(&params));
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
                let format = PixelFormat { mode, sample, alpha: true };
                let channels = format.channels();
                let mut source = Surface::new(format);
                let pixels: Vec<_> = (0..96 * 80)
                    .flat_map(|i| (0..channels).map(move |c| if c + 1 == channels { 0.8 } else { ((i * 17 + c * 31) % 251) as f32 / 250.0 }))
                    .collect();
                source.write_region(area, &pixels);
                for coverage in [Coverage::None, Coverage::Selection(Surface::with_default(PixelFormat::GRAY8, &[0.5]))] {
                    let full = develop(&source, area, &params, &coverage);
                    let region = develop(&source, crop, &params, &coverage);
                    assert_eq!(region.read_region(crop), full.read_region(crop), "{mode:?}/{sample:?}");
                    assert_eq!(region.rgba(area.x0, area.y0), source.rgba(area.x0, area.y0), "outside the region stays untouched");
                }
            }
        }
    }

    #[test]
    fn spatial_parameters_never_take_the_pointwise_crop_shortcut() {
        for field in ["dehaze", "highlights", "shadows", "texture", "clarity", "sharpenAmount", "noiseLuminance", "noiseColor", "grainAmount", "vignetteAmount"]
        {
            let params: CameraRaw = serde_json::from_value(serde_json::json!({field: 1.0})).unwrap();
            assert!(!pointwise(&params), "{field} needs its full spatial domain");
        }
    }

    #[test]
    fn local_detail_region_matches_full_engine_across_depths_coverage_and_edges() {
        use photocraft_color::{ColorMode, SampleType};
        let domain = PixelRect::new(-20, 11, 108, 107);
        let params = CameraRaw {
            exposure: 0.4,
            noise_luminance: 70.0,
            noise_color: 90.0,
            sharpen_amount: 120.0,
            sharpen_radius: 3.0,
            sharpen_masking: 80.0,
            ..Default::default()
        };
        assert!(local_preview(&params));
        assert!(!pointwise(&params));
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
                let format = PixelFormat { mode, sample, alpha: true };
                let channels = format.channels();
                let mut source = Surface::new(format);
                let pixels: Vec<_> = (0..128 * 96)
                    .flat_map(|i| (0..channels).map(move |c| if c + 1 == channels { 0.7 } else { ((i * 37 + c * 71) % 251) as f32 / 250.0 }))
                    .collect();
                source.write_region(domain, &pixels);
                let mask = Surface::with_default(PixelFormat::GRAY8, &[0.4]);
                for coverage in [
                    Coverage::None,
                    Coverage::Selection(mask.clone()),
                    Coverage::Mask(photocraft_doc::LayerMask { surface: mask, density: 0.6, ..photocraft_doc::LayerMask::reveal_all() }),
                ] {
                    let full = develop(&source, domain, &params, &coverage);
                    for crop in [PixelRect::new(30, 40, 58, 60), PixelRect::new(-20, 11, 2, 31), PixelRect::new(84, 87, 108, 107)] {
                        let work = local_work_area(&params, crop, domain).unwrap();
                        assert_ne!(work, domain);
                        assert_eq!(work.intersect(&domain), work);
                        let local = develop(&source, work, &params, &coverage);
                        // Different running-sum origins may round at the final depth conversion.
                        let tolerance = match sample {
                            SampleType::U8 => 1.0 / 255.0 + 1e-7,
                            SampleType::U16 => 1.0 / 65535.0 + 2e-6,
                            SampleType::F32 => 2e-6,
                        };
                        let max_error = full.read_region(crop).iter().zip(local.read_region(crop)).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
                        assert!(max_error <= tolerance, "{mode:?}/{sample:?}: {max_error}");
                    }
                }
            }
        }
    }

    #[test]
    fn spatial_crop_work_publishes_only_valid_interior_and_coalesces_drag_revisions() {
        let domain = PixelRect::new(0, 0, 6000, 4000);
        let visible = PixelRect::new(2000, 1500, 2800, 2100);
        let params = CameraRaw { noise_luminance: 80.0, noise_color: 90.0, sharpen_amount: 100.0, sharpen_radius: 3.0, ..Default::default() };
        let work = local_work_area(&params, visible, domain).unwrap();
        assert!(work.width() > visible.width() && work.width() < 850);
        let source = Surface::with_default(PixelFormat::RGBA8, &[0.2, 0.4, 0.6, 1.0]);
        let mut preview = DetailPreview::new(source.clone(), domain, Coverage::None);
        let (tx, rx) = std::sync::mpsc::channel();
        preview.pending = Some((2, rx));
        preview.pending_local = true;
        preview.pending_area = visible;
        preview.pending_work_area = work;
        assert_eq!(preview.work_pixels(), work.width() as u64 * work.height() as u64);
        preview.invalidate();
        assert!(preview.pending());
        tx.send(Ok((2, source.clone()))).unwrap();
        preview.poll(8);
        assert_eq!(preview.result_area, visible, "halo boundary pixels must never be displayed or sampled");
        assert_eq!(preview.result, Some((2, source)));
        assert!(!preview.ready, "an intermediate result must retain its actual revision");
    }

    #[test]
    fn pointwise_jobs_coalesce_without_starving_during_a_drag() {
        let source = Surface::with_default(PixelFormat::RGBA8, &[0.2, 0.4, 0.6, 1.0]);
        let mut preview = DetailPreview::new(source.clone(), PixelRect::new(0, 0, 6000, 4000), Coverage::None);
        let crop = PixelRect::new(2000, 1500, 2800, 2100);
        let (tx, rx) = std::sync::mpsc::channel();
        preview.pending = Some((1, rx));
        preview.pending_local = true;
        preview.pending_area = crop;
        let token = preview.cancel.clone();
        preview.invalidate();
        assert!(preview.pending(), "latest settings must not cancel every intermediate frame during a continuous drag");
        assert!(!token.load(Ordering::Relaxed));
        tx.send(Ok((1, source))).unwrap();
        preview.poll(8);
        assert_eq!(preview.result.as_ref().map(|(revision, _)| *revision), Some(1));
        assert_eq!(preview.result_area, crop);
        assert!(!preview.ready, "intermediate frame is retained without claiming latest-revision completion");
        preview.request_region(&Context::default(), 8, &CameraRaw { exposure: 0.5, ..Default::default() }, crop);
        assert_eq!(preview.pending_area, crop, "only 800×600 visible pixels are developed, not the 24 MP document");
        assert!(preview.pending());
    }

    #[test]
    fn old_detail_never_poison_new_revision_texture_cache() {
        let area = PixelRect::new(0, 0, 16, 16);
        let source = Surface::with_default(PixelFormat::RGBA8, &[0.2, 0.4, 0.6, 1.0]);
        let params = CameraRaw { exposure: 1.0, ..Default::default() };
        let mut preview = DetailPreview::new(source.clone(), area, Coverage::None);
        preview.result = Some((1, source.clone()));
        let ctx = Context::default();
        let image = Rect::from_min_size(pos2(10., 10.), vec2(200., 200.));
        let mut out = ctx.run_ui(Default::default(), |ui| {
            assert!(preview.paint(ui, image, 1, false, &params, true));
        });
        out.textures_delta.clear();
        let texture = preview.texture.as_ref().unwrap().id();
        let (tx, rx) = std::sync::mpsc::channel();
        preview.pending = Some((2, rx));
        let mut out = ctx.run_ui(Default::default(), |ui| {
            assert!(preview.paint(ui, image, 2, false, &params, true));
            assert!(!preview.ready, "retained sharp pixels are not the new settings");
        });
        out.textures_delta.clear();
        assert_eq!(preview.after_crop.unwrap().0, 1, "old pixels must not receive the new cache key");
        let fresh = develop(&source, area, &params, &Coverage::None);
        tx.send(Ok((2, fresh.clone()))).unwrap();
        let mut out = ctx.run_ui(Default::default(), |ui| {
            assert!(preview.paint(ui, image, 2, false, &params, true));
        });
        assert!(out.textures_delta.set.iter().any(|(id, _)| *id == texture), "new pixels upload without another edit");
        out.textures_delta.clear();
        assert_eq!(preview.after_crop.unwrap().0, 2);
        assert_eq!(preview.sample([0.5, 0.5], 2, false, &params), Some(fresh.rgba(8, 8)));
    }
    #[test]
    fn reset_replaces_the_retained_sharp_frame_before_the_next_edit() {
        let source = Surface::with_default(PixelFormat::RGBA8, &[0.2, 0.4, 0.6, 1.0]);
        let area = PixelRect::new(0, 0, 16, 16);
        let mut preview = DetailPreview::new(source.clone(), area, Coverage::None);
        preview.result = Some((1, Surface::with_default(PixelFormat::RGBA8, &[0.8, 0.9, 1.0, 1.0])));
        let ctx = Context::default();
        let image = Rect::from_min_size(pos2(10.0, 10.0), vec2(200.0, 200.0));
        let mut output = ctx.run_ui(Default::default(), |ui| {
            assert!(preview.paint(ui, image, 2, false, &CameraRaw::default(), true));
        });
        output.textures_delta.clear();
        assert_eq!(preview.result, Some((2, source)));
        let mut output = ctx.run_ui(Default::default(), |ui| {
            assert!(preview.paint(ui, image, 3, false, &CameraRaw { exposure: 0.2, ..Default::default() }, false));
            assert!(!preview.ready);
        });
        output.textures_delta.clear();
        assert_eq!(preview.after_crop.unwrap().0, 2, "pending edit retains the reset frame instead of flashing back to revision 1");
    }

    #[test]
    fn before_and_after_refinements_keep_separate_gpu_textures() {
        let area = PixelRect::new(0, 0, 16, 16);
        let source = Surface::with_default(PixelFormat::RGBA8, &[0.2, 0.4, 0.6, 1.0]);
        let params = CameraRaw { exposure: 1.0, ..Default::default() };
        let mut preview = DetailPreview::new(source.clone(), area, Coverage::None);
        preview.result = Some((1, develop(&source, area, &params, &Coverage::None)));
        let ctx = Context::default();
        let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 300.0))), ..Default::default() }, |ui| {
            let before = Rect::from_min_size(pos2(10.0, 10.0), vec2(200.0, 200.0));
            let after = before.translate(vec2(230.0, 0.0));
            assert!(preview.paint(ui, before, 1, true, &params, true));
            assert!(preview.paint(ui, after, 1, false, &params, true));
        });
        output.textures_delta.clear();
        assert_ne!(preview.before_texture.as_ref().unwrap().id(), preview.texture.as_ref().unwrap().id());
    }
    #[test]
    fn refinement_uses_the_engine_at_source_resolution_and_respects_selection() {
        let area = PixelRect::new(10, -5, 42, 19);
        let mut source = Surface::new(PixelFormat::RGBA8);
        let mut values = vec![0.0; 32 * 24 * 4];
        for (i, p) in values.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            p.copy_from_slice(&[if i % 2 == 0 { 0.1 } else { 0.8 }, 0.3, 0.6, 1.0]);
        }
        source.write_region(area, &values);
        let params = CameraRaw { exposure: 0.5, texture: 15.0, grain_amount: 30.0, ..Default::default() };
        let expected = photocraft_engine::lens_cmds::camera_raw_surface(&source, area, &params);
        assert_eq!(develop(&source, area, &params, &Coverage::None), expected);
        let mask = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        assert_eq!(develop(&source, area, &params, &Coverage::Selection(mask)), source);
    }
    #[test]
    fn stale_refinements_and_failed_old_revisions_do_not_replace_new_settings() {
        let source = Surface::with_default(PixelFormat::RGBA8, &[0.2, 0.4, 0.6, 1.0]);
        let mut preview = DetailPreview::new(source.clone(), PixelRect::new(0, 0, 16, 16), Coverage::None);
        let (tx, rx) = std::sync::mpsc::channel();
        preview.pending = Some((1, rx));
        tx.send(Ok((1, source.clone()))).unwrap();
        preview.poll(2);
        assert_eq!(preview.result.as_ref().map(|(revision, _)| *revision), Some(1), "completed global work retains its honest intermediate revision");
        assert!(!preview.pending());
        let (tx, rx) = std::sync::mpsc::channel();
        preview.pending = Some((1, rx));
        tx.send(Err("old failure".into())).unwrap();
        preview.poll(2);
        assert_eq!(preview.failed_revision, Some(1), "a failure must not suppress the new revision");
        let (tx, rx) = std::sync::mpsc::channel();
        preview.pending = Some((2, rx));
        tx.send(Ok((2, source))).unwrap();
        preview.poll(2);
        assert_eq!(preview.result.as_ref().map(|(r, _)| *r), Some(2));
        assert!(preview.error.is_none());
    }

    #[test]
    fn interactive_revision_keeps_one_full_domain_worker_instead_of_spawning_cancelled_work() {
        let source = Surface::with_default(PixelFormat::RGBA8, &[0.2, 0.4, 0.6, 1.0]);
        let mut preview = DetailPreview::new(source, PixelRect::new(0, 0, 16, 16), Coverage::None);
        let (_tx, rx) = std::sync::mpsc::channel();
        let old_cancel = preview.cancel.clone();
        preview.pending = Some((1, rx));
        preview.ready = true;
        preview.invalidate();
        assert!(!old_cancel.load(Ordering::Relaxed));
        assert!(preview.pending());
        assert!(!preview.ready);
        assert!(preview.crop.is_none());
    }

    #[test]
    fn full_resolution_sampling_and_crops_keep_fine_detail_at_every_supported_depth() {
        use photocraft_color::{ColorMode, PixelFormat, SampleType};
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
                let format = PixelFormat { mode, sample: depth, alpha: true };
                let area = PixelRect::new(-12, 8, 28, 38);
                let mut source = Surface::new(format);
                let channels = format.channels();
                let mut pixels = vec![0.0; 40 * 30 * channels];
                for (i, pixel) in pixels.chunks_exact_mut(channels).enumerate() {
                    pixel.fill(if i % 2 == 0 { 0.1 } else { 0.9 });
                    pixel[channels - 1] = 1.0;
                }
                source.write_region(area, &pixels);
                let params = CameraRaw { exposure: 0.5, sharpen_amount: 20.0, grain_amount: 30.0, ..Default::default() };
                let full = develop(&source, area, &params, &Coverage::None);
                let expected = photocraft_engine::lens_cmds::camera_raw_surface(&source, area, &params);
                assert_eq!(full, expected);
                let mut preview = DetailPreview::new(source, area, Coverage::None);
                preview.result = Some((1, full));
                preview.ready = true;
                let dark = preview.sample([0.0, 0.0], 1, false, &params).unwrap();
                let light = preview.sample([1.0 / 40.0, 0.0], 1, false, &params).unwrap();
                assert!(light[0] - dark[0] > 0.5, "native alternating pixels survive: {mode:?}/{depth:?}");
                assert_eq!(crop_image(&expected, PixelRect::new(-12, 8, -10, 9)).size, [2, 1]);
                assert!(preview.sample([f32::NAN, 0.0], 1, false, &params).is_none());
                assert!(preview.sample([0.5, 0.5], 2, false, &params).is_none());
            }
        }
    }

    #[test]
    fn huge_sparse_images_keep_the_proxy_without_starting_an_unbounded_worker() {
        let source = Surface::new(PixelFormat::RGBA8);
        let mut preview = DetailPreview::new(source, PixelRect::new(0, 0, 8001, 8000), Coverage::None);
        preview.request(&Context::default(), 1, &CameraRaw { exposure: 1.0, ..Default::default() });
        assert!(!preview.pending());
        assert!(preview.error.as_deref().unwrap().contains("64 megapixels"));
        assert_eq!(preview.failed_revision, Some(1));
    }

    #[test]
    fn zoom_uploads_only_visible_pixels_even_on_a_huge_offset_image() {
        let area = PixelRect::new(-200, 10, 15800, 10010);
        let image = Rect::from_min_size(pos2(-7800.0, -4800.0), vec2(16000.0, 10000.0));
        let visible = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let crop = visible_pixels(image, visible, area, 4096).unwrap();
        assert_eq!((crop.width(), crop.height()), (802, 602));
        assert!(visible_pixels(image, Rect::from_min_size(Pos2::ZERO, vec2(8000.0, 6000.0)), area, 4096).is_none());
        assert!(visible_pixels(Rect::NOTHING, visible, area, 4096).is_none());
    }
}
