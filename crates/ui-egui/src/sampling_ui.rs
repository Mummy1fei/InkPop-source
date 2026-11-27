//! Independent eyedropper/Mixer pickup settings; presets and brush diameter stay untouched.
use egui::{Color32, Painter, Pos2, Shape, Stroke, vec2};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{PhotocraftApp, widgets};

const STORAGE_KEY: &str = "tools.sampling";
pub const SIZES: [u16; 7] = [1, 3, 5, 11, 31, 51, 101];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Current,
    CurrentBelow,
    #[default]
    All,
    AllNoAdjustments,
    CurrentBelowNoAdjustments,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::CurrentBelow => "currentBelow",
            Self::All => "all",
            Self::AllNoAdjustments => "allNoAdjustments",
            Self::CurrentBelowNoAdjustments => "currentBelowNoAdjustments",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Options {
    pub size: u16,
    pub source: Source,
    pub show_ring: bool,
    pub mixer_size: u16,
    pub mixer_sample_all_layers: bool,
    #[serde(skip)]
    ring: Option<Ring>,
    #[serde(skip)]
    cache: Option<Cache>,
}

impl Default for Options {
    fn default() -> Self {
        Self { size: 1, source: Source::All, show_ring: true, mixer_size: 1, mixer_sample_all_layers: true, ring: None, cache: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Ring {
    previous: [f32; 3],
    current: [f32; 3],
}

#[derive(Clone, Debug, PartialEq)]
struct Cache {
    document: photocraft_doc::DocId,
    revision: u64,
    layer: Option<photocraft_doc::LayerId>,
    position: [i32; 2],
    source: Source,
    size: u16,
    rgba: [f32; 4],
}

fn valid(options: &Options) -> bool {
    SIZES.contains(&options.size) && SIZES.contains(&options.mixer_size)
}

/// Called once by the preferences lifecycle; an explicit saved false is retained.
pub fn load(app: &mut PhotocraftApp) {
    app.ui.sampling =
        app.session.prefs().dialogs.get(STORAGE_KEY).cloned().and_then(|value| serde_json::from_value::<Options>(value).ok()).filter(valid).unwrap_or_default();
}

pub fn save(app: &mut PhotocraftApp) {
    if let Ok(value) = serde_json::to_value(&app.ui.sampling) {
        app.session.edit_prefs(|prefs| {
            prefs.dialogs.insert(STORAGE_KEY.into(), value);
        });
    }
}

fn size_control(ui: &mut egui::Ui, key: &str, size: &mut u16) -> bool {
    let choices = [
        (1, tl!("Point Sample")),
        (3, tl!("3 by 3 Average")),
        (5, tl!("5 by 5 Average")),
        (11, tl!("11 by 11 Average")),
        (31, tl!("31 by 31 Average")),
        (51, tl!("51 by 51 Average")),
        (101, tl!("101 by 101 Average")),
    ];
    widgets::dropdown(ui, key, size, &choices, 126.0)
}

pub fn eyedropper_options(ui: &mut egui::Ui, options: &mut Options) -> bool {
    let mut changed = false;
    ui.label(tl!("Sample Size:"));
    changed |= size_control(ui, "eyedropper-sample-size", &mut options.size);
    ui.label(tl!("Sample:"));
    changed |= widgets::dropdown(
        ui,
        "eyedropper-source",
        &mut options.source,
        &[
            (Source::Current, tl!("Current Layer")),
            (Source::CurrentBelow, tl!("Current & Below")),
            (Source::All, tl!("All Layers")),
            (Source::AllNoAdjustments, tl!("All Layers (No Adjustments)")),
            (Source::CurrentBelowNoAdjustments, tl!("Current & Below (No Adjustments)")),
        ],
        210.0,
    );
    changed |= widgets::checkbox(ui, &mut options.show_ring, tl!("Show Sampling Ring")).changed();
    changed
}

pub fn mixer_options(ui: &mut egui::Ui, options: &mut Options) -> bool {
    ui.label(tl!("Pickup Size:"));
    let changed = size_control(ui, "mixer-pickup-size", &mut options.mixer_size);
    widgets::checkbox(ui, &mut options.mixer_sample_all_layers, tl!("Sample All Layers")).changed() || changed
}

/// One sample cache is independent of the Info/color-picker one-pixel cache.
pub fn color(app: &mut PhotocraftApp, x: f64, y: f64) -> Option<[f32; 3]> {
    if !x.is_finite() || !y.is_finite() || x.abs() > 1_000_000.0 || y.abs() > 1_000_000.0 {
        return None;
    }
    let options = &app.ui.sampling;
    let active = app.session.active()?;
    let position = [x.floor() as i32, y.floor() as i32];
    let (document, revision, layer) = (active.doc.id, active.revision, active.active_layer);
    let (size, source) = (options.size, options.source);
    let rgba = if let Some(cache) = &options.cache
        && (cache.document, cache.revision, cache.layer, cache.position, cache.size, cache.source) == (document, revision, layer, position, size, source)
    {
        cache.rgba
    } else {
        let result = if size == 1 && source == Source::All {
            app.run("document.pixel", json!({"x":position[0],"y":position[1]})).ok()?
        } else {
            app.run("document.sampleColor", json!({"x":x,"y":y,"size":size,"source":source.as_str()})).ok()?
        };
        let rgba: [f32; 4] = serde_json::from_value(result).ok()?;
        if size == 1 && source == Source::All {
            app.info_sample = Some(((position[0], position[1], revision), rgba));
        }
        app.ui.sampling.cache = Some(Cache { document, revision, layer, position, size, source, rgba });
        rgba
    };
    (rgba[3] > 0.0 && rgba.iter().all(|value| value.is_finite())).then_some([rgba[0], rgba[1], rgba[2]])
}

pub fn begin(app: &mut PhotocraftApp, background: bool) {
    let previous = if background { app.session.tools.background } else { app.session.tools.foreground };
    let previous = [previous[0], previous[1], previous[2]];
    app.ui.sampling.ring = app.ui.sampling.show_ring.then_some(Ring { previous, current: previous });
}

pub fn sampled(app: &mut PhotocraftApp, color: [f32; 3]) {
    if let Some(ring) = &mut app.ui.sampling.ring {
        ring.current = color;
    }
}

pub fn end(app: &mut PhotocraftApp) {
    app.ui.sampling.ring = None;
}

pub fn draw_ring(app: &PhotocraftApp, painter: &Painter, at: Pos2) {
    if !app.ui.sampling.show_ring || !painter.ctx().input(|input| input.pointer.primary_down()) {
        return;
    }
    let Some(ring) = &app.ui.sampling.ring else { return };
    let color = |rgb: [f32; 3]| {
        Color32::from_rgb(
            (rgb[0].clamp(0.0, 1.0) * 255.0).round() as u8,
            (rgb[1].clamp(0.0, 1.0) * 255.0).round() as u8,
            (rgb[2].clamp(0.0, 1.0) * 255.0).round() as u8,
        )
    };
    painter.circle_stroke(at, 33.0, Stroke::new(10.0, Color32::BLACK));
    painter.circle_stroke(at, 33.0, Stroke::new(8.0, Color32::WHITE));
    for (start, rgb) in [(std::f32::consts::PI, ring.current), (0.0, ring.previous)] {
        let points = (0..=48)
            .map(|index| {
                let angle = start + index as f32 / 48.0 * std::f32::consts::PI;
                at + vec2(angle.cos(), angle.sin()) * 33.0
            })
            .collect();
        painter.add(Shape::line(points, Stroke::new(6.0, color(rgb))));
    }
}

#[cfg(test)]
#[path = "sampling_tests_v79.rs"]
mod tests;
