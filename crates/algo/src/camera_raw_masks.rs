//! Resolution-independent Camera Raw local masks. Point adjustments have no image-size mask allocation.
use crate::camera_raw::{CameraRaw, Wheel};
use serde::{Deserialize, Serialize};

#[path = "camera_raw_mask_serde.rs"]
mod bounded_serde;
#[path = "camera_raw_raster_codec.rs"]
mod raster_codec;
pub use bounded_serde::deserialize_items;

pub const MAX_MASKS: usize = 64;
pub const MAX_POINTS: usize = 4096;
/// Additional (non-recursive) shapes in one mask group.
pub const MAX_COMPONENTS: usize = 32;
/// Bounds preparation/index overhead even when all shapes are empty brushes.
pub const MAX_TOTAL_SHAPES: usize = 256;
/// Automatic masks retain the shared service's reference resolution. The total
/// storage budget stays bounded across every group and component (16 MiB u8).
pub const MAX_RASTER_SIDE: u32 = 1024;
pub const MAX_RASTER_PIXELS: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrushStroke {
    pub points: Vec<[f32; 2]>,
    /// Radius as a fraction of the image's shorter side.
    pub radius: f32,
    pub feather: f32,
    #[serde(default)]
    pub erase: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum MaskShape {
    Brush {
        strokes: Vec<BrushStroke>,
    },
    Linear {
        start: [f32; 2],
        end: [f32; 2],
    },
    Radial {
        center: [f32; 2],
        radius: [f32; 2],
        feather: f32,
        /// Clockwise degrees in image pixel coordinates, not stretched UV coordinates.
        #[serde(default)]
        rotation: f32,
    },
    Luminance {
        min: f32,
        max: f32,
        feather: f32,
    },
    Color {
        color: [f32; 3],
        tolerance: f32,
        feather: f32,
    },
    /// Bounded generated selection, sampled in normalized image coordinates.
    Raster {
        width: u32,
        height: u32,
        #[serde(with = "raster_codec")]
        values: Vec<u8>,
    },
}

/// A leaf shape added to, subtracted from, or intersected with the group's primary `mask`.
/// Components deliberately cannot contain groups, so validation and evaluation are bounded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct MaskComponent {
    pub name: String,
    pub enabled: bool,
    pub subtract: bool,
    /// Multiply current group coverage by this shape. Mutually exclusive with `subtract`.
    /// Missing in older projects and presets, where the default remains false.
    pub intersect: bool,
    /// Complement accumulated prior coverage before this leaf, independently of its visibility.
    /// Appending an intersection consumes group inversion here. Deleting that leaf transfers
    /// this unary boundary to its successor (or to final group inversion). Default is false.
    pub invert_previous: bool,
    pub mask: MaskShape,
}
impl Default for MaskComponent {
    fn default() -> Self {
        Self {
            name: "Component".into(),
            enabled: true,
            subtract: false,
            intersect: false,
            invert_previous: false,
            mask: MaskShape::Linear { start: [0.5, 0.25], end: [0.5, 0.75] },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct LocalAdjustment {
    pub name: String,
    pub enabled: bool,
    pub invert: bool,
    pub mask: MaskShape,
    /// Primary leaf visibility, distinct from the whole adjustment group's `enabled`.
    pub mask_enabled: bool,
    #[serde(deserialize_with = "bounded_serde::deserialize_components")]
    pub components: Vec<MaskComponent>,
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub temperature: f32,
    pub tint: f32,
    /// Uniform hue shift in degrees, -180..=180.
    pub hue: f32,
    pub saturation: f32,
    pub vibrance: f32,
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub grain_amount: f32,
    pub grain_size: f32,
    pub grain_roughness: f32,
    pub curve_highlights: f32,
    pub curve_lights: f32,
    pub curve_darks: f32,
    pub curve_shadows: f32,
    pub curve_splits: [f32; 3],
    pub point_curve: Vec<[f32; 2]>,
    pub point_curve_red: Vec<[f32; 2]>,
    pub point_curve_green: Vec<[f32; 2]>,
    pub point_curve_blue: Vec<[f32; 2]>,
    pub hsl_hue: [f32; 8],
    pub hsl_sat: [f32; 8],
    pub hsl_lum: [f32; 8],
    pub grade_shadows: Wheel,
    pub grade_midtones: Wheel,
    pub grade_highlights: Wheel,
    pub grade_global: Wheel,
    pub grade_blending: f32,
    pub grade_balance: f32,
    pub sharpen_amount: f32,
    pub sharpen_radius: f32,
    pub sharpen_detail: f32,
    pub sharpen_masking: f32,
    pub noise_luminance: f32,
    pub noise_luminance_detail: f32,
    pub noise_color: f32,
    pub noise_color_detail: f32,
}
impl Default for LocalAdjustment {
    fn default() -> Self {
        Self {
            name: "Mask".into(),
            enabled: true,
            invert: false,
            mask: MaskShape::Linear { start: [0.5, 0.25], end: [0.5, 0.75] },
            mask_enabled: true,
            components: Vec::new(),
            exposure: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
            temperature: 0.0,
            tint: 0.0,
            hue: 0.0,
            saturation: 0.0,
            vibrance: 0.0,
            texture: 0.0,
            clarity: 0.0,
            dehaze: 0.0,
            grain_amount: 0.0,
            grain_size: 25.0,
            grain_roughness: 50.0,
            curve_highlights: 0.0,
            curve_lights: 0.0,
            curve_darks: 0.0,
            curve_shadows: 0.0,
            curve_splits: [25.0, 50.0, 75.0],
            point_curve: Vec::new(),
            point_curve_red: Vec::new(),
            point_curve_green: Vec::new(),
            point_curve_blue: Vec::new(),
            hsl_hue: [0.0; 8],
            hsl_sat: [0.0; 8],
            hsl_lum: [0.0; 8],
            grade_shadows: Wheel::default(),
            grade_midtones: Wheel::default(),
            grade_highlights: Wheel::default(),
            grade_global: Wheel::default(),
            grade_blending: 50.0,
            grade_balance: 0.0,
            sharpen_amount: 0.0,
            sharpen_radius: 1.0,
            sharpen_detail: 25.0,
            sharpen_masking: 0.0,
            noise_luminance: 0.0,
            noise_luminance_detail: 50.0,
            noise_color: 0.0,
            noise_color_detail: 50.0,
        }
    }
}
impl LocalAdjustment {
    /// Whether this group needs full-image neighbourhoods, rather than independent pixels.
    pub fn is_spatial(&self) -> bool {
        self.texture != 0.0 || self.clarity != 0.0 || self.dehaze != 0.0 || self.sharpen_amount > 0.0 || self.noise_luminance > 0.0 || self.noise_color > 0.0
    }
    /// Compare all color controls without copying masks, raster data, or point curves.
    /// Geometry, group/component enable state, inversion, and names are intentionally excluded.
    pub fn same_color_settings(&self, other: &Self) -> bool {
        self.exposure == other.exposure
            && self.contrast == other.contrast
            && self.highlights == other.highlights
            && self.shadows == other.shadows
            && self.whites == other.whites
            && self.blacks == other.blacks
            && self.temperature == other.temperature
            && self.tint == other.tint
            && self.hue == other.hue
            && self.saturation == other.saturation
            && self.vibrance == other.vibrance
            && self.texture == other.texture
            && self.clarity == other.clarity
            && self.dehaze == other.dehaze
            && self.grain_amount == other.grain_amount
            && self.grain_size == other.grain_size
            && self.grain_roughness == other.grain_roughness
            && self.curve_highlights == other.curve_highlights
            && self.curve_lights == other.curve_lights
            && self.curve_darks == other.curve_darks
            && self.curve_shadows == other.curve_shadows
            && self.curve_splits == other.curve_splits
            && self.point_curve == other.point_curve
            && self.point_curve_red == other.point_curve_red
            && self.point_curve_green == other.point_curve_green
            && self.point_curve_blue == other.point_curve_blue
            && self.hsl_hue == other.hsl_hue
            && self.hsl_sat == other.hsl_sat
            && self.hsl_lum == other.hsl_lum
            && self.grade_shadows == other.grade_shadows
            && self.grade_midtones == other.grade_midtones
            && self.grade_highlights == other.grade_highlights
            && self.grade_global == other.grade_global
            && self.grade_blending == other.grade_blending
            && self.grade_balance == other.grade_balance
            && self.sharpen_amount == other.sharpen_amount
            && self.sharpen_radius == other.sharpen_radius
            && self.sharpen_detail == other.sharpen_detail
            && self.sharpen_masking == other.sharpen_masking
            && self.noise_luminance == other.noise_luminance
            && self.noise_luminance_detail == other.noise_luminance_detail
            && self.noise_color == other.noise_color
            && self.noise_color_detail == other.noise_color_detail
    }
    pub fn is_identity(&self) -> bool {
        !self.enabled
            || (!self.invert && !self.mask_enabled && self.components.iter().all(|c| !c.invert_previous && (!c.enabled || c.subtract || c.intersect)))
            || (self.base_is_identity() && !self.has_advanced())
    }
    fn base_is_identity(&self) -> bool {
        [self.exposure, self.contrast, self.highlights, self.shadows, self.whites, self.blacks, self.temperature, self.tint, self.hue, self.saturation]
            .iter()
            .all(|v| *v == 0.0)
    }
    /// Extra local controls use the same Camera Raw implementation as the global Edit page.
    /// Existing base local controls stay neutral here to retain their v62 arithmetic.
    /// No local mask list is copied or nested, so a develop pass cannot recurse.
    pub fn advanced_params(&self, pixel_scale: f32, seed: u32) -> CameraRaw {
        CameraRaw {
            vibrance: self.vibrance,
            texture: self.texture,
            clarity: self.clarity,
            dehaze: self.dehaze,
            grain_amount: self.grain_amount,
            grain_size: self.grain_size,
            grain_roughness: self.grain_roughness,
            curve_highlights: self.curve_highlights,
            curve_lights: self.curve_lights,
            curve_darks: self.curve_darks,
            curve_shadows: self.curve_shadows,
            curve_splits: self.curve_splits,
            point_curve: self.point_curve.clone(),
            point_curve_red: self.point_curve_red.clone(),
            point_curve_green: self.point_curve_green.clone(),
            point_curve_blue: self.point_curve_blue.clone(),
            hsl_hue: self.hsl_hue,
            hsl_sat: self.hsl_sat,
            hsl_lum: self.hsl_lum,
            grade_shadows: self.grade_shadows,
            grade_midtones: self.grade_midtones,
            grade_highlights: self.grade_highlights,
            grade_global: self.grade_global,
            grade_blending: self.grade_blending,
            grade_balance: self.grade_balance,
            sharpen_amount: self.sharpen_amount,
            sharpen_radius: self.sharpen_radius,
            sharpen_detail: self.sharpen_detail,
            sharpen_masking: self.sharpen_masking,
            noise_luminance: self.noise_luminance,
            noise_luminance_detail: self.noise_luminance_detail,
            noise_color: self.noise_color,
            noise_color_detail: self.noise_color_detail,
            pixel_scale,
            seed,
            ..Default::default()
        }
    }
    /// Copy supported extra controls from a temporary shared editor, without changing geometry
    /// or base local parameters. The caller still validates the complete adjustment transaction.
    pub fn update_advanced(&mut self, params: &CameraRaw) {
        self.vibrance = params.vibrance;
        self.texture = params.texture;
        self.clarity = params.clarity;
        self.dehaze = params.dehaze;
        self.grain_amount = params.grain_amount;
        self.grain_size = params.grain_size;
        self.grain_roughness = params.grain_roughness;
        self.curve_highlights = params.curve_highlights;
        self.curve_lights = params.curve_lights;
        self.curve_darks = params.curve_darks;
        self.curve_shadows = params.curve_shadows;
        self.curve_splits = params.curve_splits;
        self.point_curve.clone_from(&params.point_curve);
        self.point_curve_red.clone_from(&params.point_curve_red);
        self.point_curve_green.clone_from(&params.point_curve_green);
        self.point_curve_blue.clone_from(&params.point_curve_blue);
        self.hsl_hue = params.hsl_hue;
        self.hsl_sat = params.hsl_sat;
        self.hsl_lum = params.hsl_lum;
        self.grade_shadows = params.grade_shadows;
        self.grade_midtones = params.grade_midtones;
        self.grade_highlights = params.grade_highlights;
        self.grade_global = params.grade_global;
        self.grade_blending = params.grade_blending;
        self.grade_balance = params.grade_balance;
        self.sharpen_amount = params.sharpen_amount;
        self.sharpen_radius = params.sharpen_radius;
        self.sharpen_detail = params.sharpen_detail;
        self.sharpen_masking = params.sharpen_masking;
        self.noise_luminance = params.noise_luminance;
        self.noise_luminance_detail = params.noise_luminance_detail;
        self.noise_color = params.noise_color;
        self.noise_color_detail = params.noise_color_detail;
    }
    fn has_advanced(&self) -> bool {
        [
            self.vibrance,
            self.texture,
            self.clarity,
            self.dehaze,
            self.grain_amount,
            self.curve_highlights,
            self.curve_lights,
            self.curve_darks,
            self.curve_shadows,
            self.sharpen_amount,
            self.noise_luminance,
            self.noise_color,
        ]
        .iter()
        .any(|v| *v != 0.0)
            || self.hsl_hue.iter().chain(&self.hsl_sat).chain(&self.hsl_lum).any(|v| *v != 0.0)
            || [&self.point_curve, &self.point_curve_red, &self.point_curve_green, &self.point_curve_blue]
                .iter()
                .any(|curve| curve.len() >= 2 && curve.iter().any(|p| (p[0] - p[1]).abs() >= 1e-3))
            || [self.grade_shadows, self.grade_midtones, self.grade_highlights, self.grade_global].iter().any(|wheel| wheel.sat != 0.0 || wheel.lum != 0.0)
    }
}
fn bounded(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}
fn point(p: &[f32; 2]) -> bool {
    p.iter().all(|v| bounded(*v, 0.0, 1.0))
}
fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
}
fn valid_advanced(a: &LocalAdjustment) -> bool {
    let signed = [a.vibrance, a.texture, a.clarity, a.dehaze, a.curve_highlights, a.curve_lights, a.curve_darks, a.curve_shadows, a.grade_balance];
    let unsigned = [
        a.grain_amount,
        a.grain_size,
        a.grain_roughness,
        a.grade_blending,
        a.sharpen_detail,
        a.sharpen_masking,
        a.noise_luminance,
        a.noise_luminance_detail,
        a.noise_color,
        a.noise_color_detail,
    ];
    signed.iter().all(|v| bounded(*v, -100.0, 100.0))
        && unsigned.iter().all(|v| bounded(*v, 0.0, 100.0))
        && bounded(a.sharpen_amount, 0.0, 150.0)
        && bounded(a.sharpen_radius, 0.5, 3.0)
        && a.hsl_hue.iter().chain(&a.hsl_sat).chain(&a.hsl_lum).all(|v| bounded(*v, -100.0, 100.0))
        && a.curve_splits.iter().all(|v| bounded(*v, 0.0, 100.0))
        && a.curve_splits.windows(2).all(|p| p[1] - p[0] >= 1.0)
        && [&a.point_curve, &a.point_curve_red, &a.point_curve_green, &a.point_curve_blue].iter().all(|curve| crate::camera_raw::validate_curve(curve).is_ok())
        && [a.grade_shadows, a.grade_midtones, a.grade_highlights, a.grade_global]
            .iter()
            .all(|wheel| bounded(wheel.hue, 0.0, 360.0) && bounded(wheel.sat, 0.0, 100.0) && bounded(wheel.lum, -100.0, 100.0))
}
#[derive(Default)]
struct ValidationBudget {
    shapes: usize,
    points: usize,
    raster_pixels: usize,
}
fn valid_shape(shape: &MaskShape, budget: &mut ValidationBudget) -> bool {
    budget.shapes = budget.shapes.saturating_add(1);
    if budget.shapes > MAX_TOTAL_SHAPES {
        return false;
    }
    match shape {
        MaskShape::Brush { strokes } => {
            if strokes.len() > MAX_POINTS {
                return false;
            }
            strokes.iter().all(|s| {
                budget.points = budget.points.saturating_add(s.points.len());
                budget.points <= MAX_POINTS
                    && !s.points.is_empty()
                    && s.points.iter().all(point)
                    && bounded(s.radius, 0.001, 1.0)
                    && bounded(s.feather, 0.0, 1.0)
            })
        }
        MaskShape::Linear { start, end } => point(start) && point(end) && start != end,
        MaskShape::Radial { center, radius, feather, rotation } => {
            point(center) && radius.iter().all(|v| bounded(*v, 0.001, 1.0)) && bounded(*feather, 0.0, 1.0) && bounded(*rotation, -180.0, 180.0)
        }
        MaskShape::Luminance { min, max, feather } => bounded(*min, 0.0, 1.0) && bounded(*max, 0.0, 1.0) && min <= max && bounded(*feather, 0.0, 1.0),
        MaskShape::Color { color, tolerance, feather } => {
            color.iter().all(|v| bounded(*v, 0.0, 1.0)) && bounded(*tolerance, 0.001, 1.0) && bounded(*feather, 0.0, 1.0)
        }
        MaskShape::Raster { width, height, values } => {
            budget.raster_pixels = budget.raster_pixels.saturating_add(values.len());
            *width > 0
                && *height > 0
                && *width <= MAX_RASTER_SIDE
                && *height <= MAX_RASTER_SIDE
                && values.len() == (*width as usize).saturating_mul(*height as usize)
                && budget.raster_pixels <= MAX_RASTER_PIXELS
        }
    }
}
pub fn validate(items: &[LocalAdjustment]) -> Result<(), String> {
    if items.len() > MAX_MASKS {
        return Err("Too many local masks (maximum 64)".into());
    }
    let mut budget = ValidationBudget::default();
    for a in items {
        if !valid_name(&a.name) {
            return Err("Mask name must contain 1–256 bytes without control characters".into());
        }
        if !bounded(a.exposure, -5.0, 5.0)
            || !bounded(a.hue, -180.0, 180.0)
            || [a.contrast, a.highlights, a.shadows, a.whites, a.blacks, a.temperature, a.tint, a.saturation].iter().any(|v| !bounded(*v, -100.0, 100.0))
            || !valid_advanced(a)
        {
            return Err("Local adjustment is outside its supported range".into());
        }
        if a.components.len() > MAX_COMPONENTS {
            return Err("Too many components in one local mask (maximum 32)".into());
        }
        if a.components.iter().any(|component| component.subtract && component.intersect) {
            return Err("A mask component cannot both subtract and intersect".into());
        }
        if !valid_shape(&a.mask, &mut budget) || a.components.iter().any(|component| !valid_name(&component.name) || !valid_shape(&component.mask, &mut budget))
        {
            return Err("Invalid mask geometry or aggregate shape/point/raster limit exceeded".into());
        }
    }
    Ok(())
}

/// Conservatively detect a structurally empty stored shape without sampling an image.
/// Empty/all-erase brushes and all-zero bounded rasters are known empty. Analytic/range
/// shapes and invalid data return false; cancellation of previously drawn strokes is not
/// inferred. This helper does not depend on UI visibility or source-image content.
pub fn shape_is_empty(shape: &MaskShape) -> bool {
    if !valid_shape(shape, &mut ValidationBudget::default()) {
        return false;
    }
    stored_shape_is_empty(shape)
}

fn stored_shape_is_empty(shape: &MaskShape) -> bool {
    match shape {
        MaskShape::Brush { strokes } => strokes.iter().all(|stroke| stroke.erase),
        MaskShape::Raster { values, .. } => values.iter().all(|value| *value == 0),
        _ => false,
    }
}

/// A safe cleanup predicate for groups with no stored positive geometry. Hidden group,
/// primary, or component switches do not make content empty: nonempty hidden shapes are
/// retained. Intersections/subtractions cannot establish positive coverage; additions can.
/// Inverted, prior-inverting, or invalid groups are always retained. Disjoint/cancelled
/// shapes are deliberately not inferred empty, so this is not a source-dependent coverage test.
pub fn group_is_empty(adjustment: &LocalAdjustment) -> bool {
    if adjustment.invert || adjustment.components.iter().any(|component| component.invert_previous) || validate(std::slice::from_ref(adjustment)).is_err() {
        return false;
    }
    stored_shape_is_empty(&adjustment.mask)
        && adjustment.components.iter().all(|component| component.subtract || component.intersect || stored_shape_is_empty(&component.mask))
}

fn falloff(distance: f32, radius: f32, feather: f32) -> f32 {
    let inner = radius * (1.0 - feather);
    if distance <= inner {
        1.0
    } else if distance >= radius {
        0.0
    } else {
        let t = ((radius - distance) / (radius - inner).max(1e-6)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}
/// Pixel-center normalized coordinates and image aspect relative to its shorter side.
pub fn weight(shape: &MaskShape, uv: [f32; 2], rgba: [f32; 4], aspect: [f32; 2]) -> f32 {
    match shape {
        MaskShape::Linear { start, end } => {
            let d = [(end[0] - start[0]) * aspect[0], (end[1] - start[1]) * aspect[1]];
            let q = [(uv[0] - start[0]) * aspect[0], (uv[1] - start[1]) * aspect[1]];
            ((q[0] * d[0] + q[1] * d[1]) / (d[0] * d[0] + d[1] * d[1]).max(1e-12)).clamp(0.0, 1.0)
        }
        MaskShape::Radial { center, radius, feather, rotation } => {
            radial_weight(*center, *radius, *feather, uv, aspect, (*rotation != 0.0).then(|| rotation.to_radians().sin_cos()))
        }
        MaskShape::Luminance { min, max, feather } => {
            let y = 0.2126 * rgba[0] + 0.7152 * rgba[1] + 0.0722 * rgba[2];
            if y >= *min && y <= *max { 1.0 } else { falloff(if y < *min { min - y } else { y - max }, feather.max(1e-6), 1.0) }
        }
        MaskShape::Color { color, tolerance, feather } => {
            falloff(((rgba[0] - color[0]).powi(2) + (rgba[1] - color[1]).powi(2) + (rgba[2] - color[2]).powi(2)).sqrt() / 3f32.sqrt(), *tolerance, *feather)
        }
        MaskShape::Raster { width, height, values } => {
            let x = (uv[0] * *width as f32 - 0.5).max(0.0);
            let y = (uv[1] * *height as f32 - 0.5).max(0.0);
            let ix = x.floor() as usize;
            let iy = y.floor() as usize;
            let fx = x - x.floor();
            let fy = y - y.floor();
            let at = |x: usize, y: usize| {
                values
                    .get(y.min(height.saturating_sub(1) as usize).saturating_mul(*width as usize).saturating_add(x.min(width.saturating_sub(1) as usize)))
                    .copied()
                    .unwrap_or(0) as f32
                    / 255.0
            };
            (1.0 - fy) * ((1.0 - fx) * at(ix, iy) + fx * at(ix + 1, iy)) + fy * ((1.0 - fx) * at(ix, iy + 1) + fx * at(ix + 1, iy + 1))
        }
        MaskShape::Brush { strokes } => {
            let mut coverage = 0.0f32;
            for stroke in strokes {
                let mut distance = f32::INFINITY;
                let q = [uv[0] * aspect[0], uv[1] * aspect[1]];
                for (i, p) in stroke.points.iter().enumerate() {
                    let a = [p[0] * aspect[0], p[1] * aspect[1]];
                    let b = stroke.points.get(i + 1).map(|p| [p[0] * aspect[0], p[1] * aspect[1]]).unwrap_or(a);
                    let d = [b[0] - a[0], b[1] - a[1]];
                    let t = (((q[0] - a[0]) * d[0] + (q[1] - a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]).max(1e-12)).clamp(0.0, 1.0);
                    distance = distance.min((q[0] - a[0] - t * d[0]).hypot(q[1] - a[1] - t * d[1]));
                }
                let v = falloff(distance, stroke.radius, stroke.feather);
                coverage = if stroke.erase { coverage * (1.0 - v) } else { coverage.max(v) };
            }
            coverage
        }
    }
}
fn radial_weight(center: [f32; 2], radius: [f32; 2], feather: f32, uv: [f32; 2], aspect: [f32; 2], rotation: Option<(f32, f32)>) -> f32 {
    let distance = if let Some((sin, cos)) = rotation {
        let q = [(uv[0] - center[0]) * aspect[0], (uv[1] - center[1]) * aspect[1]];
        let local = [cos * q[0] + sin * q[1], -sin * q[0] + cos * q[1]];
        (local[0] / (radius[0].max(0.001) * aspect[0])).hypot(local[1] / (radius[1].max(0.001) * aspect[1]))
    } else {
        // Keep the original zero-angle arithmetic for existing presets and projects.
        ((uv[0] - center[0]) / radius[0].max(0.001)).hypot((uv[1] - center[1]) / radius[1].max(0.001))
    };
    falloff(distance, 1.0, feather)
}
struct Segment {
    a: [f32; 2],
    b: [f32; 2],
    stroke: usize,
    radius: f32,
    feather: f32,
    erase: bool,
}
/// A bounded spatial index keeps a long brush path from scanning every segment for
/// every full-resolution pixel. Index lists preserve stroke order for erasing.
struct BrushIndex {
    segments: Vec<Segment>,
    cells: Vec<Vec<usize>>,
}
impl BrushIndex {
    fn new(strokes: &[BrushStroke], aspect: [f32; 2]) -> Self {
        let mut out = Self { segments: Vec::new(), cells: vec![Vec::new(); 32 * 32] };
        for (stroke, s) in strokes.iter().enumerate() {
            for (i, p) in s.points.iter().enumerate() {
                let q = s.points.get(i + 1).unwrap_or(p);
                let lo = [(p[0].min(q[0]) - s.radius / aspect[0]).max(0.0), (p[1].min(q[1]) - s.radius / aspect[1]).max(0.0)];
                let hi = [(p[0].max(q[0]) + s.radius / aspect[0]).min(1.0), (p[1].max(q[1]) + s.radius / aspect[1]).min(1.0)];
                let index = out.segments.len();
                out.segments.push(Segment {
                    a: [p[0] * aspect[0], p[1] * aspect[1]],
                    b: [q[0] * aspect[0], q[1] * aspect[1]],
                    stroke,
                    radius: s.radius,
                    feather: s.feather,
                    erase: s.erase,
                });
                for y in ((lo[1] * 32.0) as usize).min(31)..=((hi[1] * 32.0) as usize).min(31) {
                    for x in ((lo[0] * 32.0) as usize).min(31)..=((hi[0] * 32.0) as usize).min(31) {
                        if let Some(cell) = out.cells.get_mut(y * 32 + x) {
                            cell.push(index);
                        }
                    }
                }
            }
        }
        out
    }
    fn weight(&self, uv: [f32; 2], aspect: [f32; 2]) -> f32 {
        let cell = ((uv[1] * 32.0) as usize).min(31) * 32 + ((uv[0] * 32.0) as usize).min(31);
        let Some(indices) = self.cells.get(cell) else {
            return 0.0;
        };
        let q = [uv[0] * aspect[0], uv[1] * aspect[1]];
        let mut coverage = 0.0f32;
        let mut current = None;
        let mut value = 0.0f32;
        let mut erase = false;
        for index in indices {
            let Some(s) = self.segments.get(*index) else {
                continue;
            };
            if current != Some(s.stroke) {
                coverage = if erase { coverage * (1.0 - value) } else { coverage.max(value) };
                value = 0.0;
                erase = s.erase;
                current = Some(s.stroke);
            }
            if q[0] < s.a[0].min(s.b[0]) - s.radius
                || q[0] > s.a[0].max(s.b[0]) + s.radius
                || q[1] < s.a[1].min(s.b[1]) - s.radius
                || q[1] > s.a[1].max(s.b[1]) + s.radius
            {
                continue;
            }
            let d = [s.b[0] - s.a[0], s.b[1] - s.a[1]];
            let t = (((q[0] - s.a[0]) * d[0] + (q[1] - s.a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]).max(1e-12)).clamp(0.0, 1.0);
            value = value.max(falloff((q[0] - s.a[0] - t * d[0]).hypot(q[1] - s.a[1] - t * d[1]), s.radius, s.feather));
        }
        if erase { coverage * (1.0 - value) } else { coverage.max(value) }
    }
}

enum PreparedShape<'a> {
    Brush(BrushIndex),
    Radial { center: [f32; 2], radius: [f32; 2], feather: f32, rotation: Option<(f32, f32)> },
    Other(&'a MaskShape),
}
impl<'a> PreparedShape<'a> {
    fn new(shape: &'a MaskShape, aspect: [f32; 2]) -> Self {
        match shape {
            MaskShape::Brush { strokes } => Self::Brush(BrushIndex::new(strokes, aspect)),
            MaskShape::Radial { center, radius, feather, rotation } => {
                Self::Radial { center: *center, radius: *radius, feather: *feather, rotation: (*rotation != 0.0).then(|| rotation.to_radians().sin_cos()) }
            }
            _ => Self::Other(shape),
        }
    }
    fn weight(&self, uv: [f32; 2], rgba: [f32; 4], aspect: [f32; 2]) -> f32 {
        match self {
            Self::Brush(index) => index.weight(uv, aspect),
            Self::Radial { center, radius, feather, rotation } => radial_weight(*center, *radius, *feather, uv, aspect, *rotation),
            Self::Other(shape) => weight(shape, uv, rgba, aspect),
        }
    }
}

/// Reusable coverage evaluator for one non-recursive mask group. Construct once per
/// thumbnail/overlay/render loop so rotation trigonometry and brush indices are not per pixel.
/// Whole-group `enabled` is a colour-adjustment switch, not geometry visibility. Component
/// visibility and primary `mask_enabled` are included here; whole-group inversion is last.
pub struct PreparedLocalMask<'a> {
    primary: Option<PreparedShape<'a>>,
    components: Vec<(bool, bool, bool, bool, PreparedShape<'a>)>,
    aspect: [f32; 2],
    invert: bool,
    valid: bool,
}
impl<'a> PreparedLocalMask<'a> {
    pub fn new(adjustment: &'a LocalAdjustment, aspect: [f32; 2]) -> Self {
        if validate(std::slice::from_ref(adjustment)).is_err() || aspect.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Self { primary: None, components: Vec::new(), aspect: [1.0; 2], invert: false, valid: false };
        }
        Self::validated(adjustment, aspect)
    }
    fn validated(adjustment: &'a LocalAdjustment, aspect: [f32; 2]) -> Self {
        Self {
            primary: adjustment.mask_enabled.then(|| PreparedShape::new(&adjustment.mask, aspect)),
            components: adjustment
                .components
                .iter()
                .filter(|component| component.enabled || component.invert_previous)
                .map(|component| {
                    (component.enabled, component.subtract, component.intersect, component.invert_previous, PreparedShape::new(&component.mask, aspect))
                })
                .collect(),
            aspect,
            invert: adjustment.invert,
            valid: true,
        }
    }
    pub fn weight(&self, uv: [f32; 2], rgba: [f32; 4]) -> f32 {
        if !self.valid || uv.iter().any(|v| !v.is_finite()) {
            return 0.0;
        }
        let mut coverage = self.primary.as_ref().map_or(0.0, |shape| shape.weight(uv, rgba, self.aspect));
        for (enabled, subtract, intersect, invert_previous, shape) in &self.components {
            if *invert_previous {
                coverage = 1.0 - coverage;
            }
            if !enabled {
                continue;
            }
            let next = shape.weight(uv, rgba, self.aspect);
            coverage = if *subtract {
                coverage * (1.0 - next)
            } else if *intersect {
                coverage * next
            } else {
                coverage.max(next)
            };
        }
        if self.invert { 1.0 - coverage } else { coverage }
    }
}

/// Scalar convenience API. Prefer [`PreparedLocalMask`] when sampling many pixels.
pub fn group_weight(adjustment: &LocalAdjustment, uv: [f32; 2], rgba: [f32; 4], aspect: [f32; 2]) -> f32 {
    PreparedLocalMask::new(adjustment, aspect).weight(uv, rgba)
}

fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

struct PreparedAdjustment<'a> {
    adjustment: &'a LocalAdjustment,
    mask: PreparedLocalMask<'a>,
    gain: f32,
    temperature: [f32; 2],
    tint: f32,
    contrast: f32,
    saturation: f32,
    tone: bool,
    base_identity: bool,
}
impl<'a> PreparedAdjustment<'a> {
    fn new(adjustment: &'a LocalAdjustment, aspect: [f32; 2]) -> Self {
        Self {
            adjustment,
            mask: PreparedLocalMask::validated(adjustment, aspect),
            gain: 2f32.powf(adjustment.exposure),
            temperature: [1.0 + adjustment.temperature * 0.002, 1.0 - adjustment.temperature * 0.002],
            tint: 1.0 - adjustment.tint * 0.002,
            contrast: 1.0 + adjustment.contrast / 100.0,
            saturation: 1.0 + adjustment.saturation / 100.0,
            tone: [adjustment.highlights, adjustment.shadows, adjustment.whites, adjustment.blacks].iter().any(|v| *v != 0.0),
            base_identity: adjustment.base_is_identity(),
        }
    }
    fn rgb(&self, original: [f32; 3]) -> [f32; 3] {
        let a = self.adjustment;
        // Preserve the existing local exposure/white-balance/contrast operation ordering.
        let mut rgb = [original[0] * self.gain * self.temperature[0], original[1] * self.gain * self.tint, original[2] * self.gain * self.temperature[1]];
        for v in &mut rgb {
            *v = (*v - 0.18) * self.contrast + 0.18;
        }
        if self.tone {
            let luma = (0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]).max(0.0);
            let unit = luma.min(1.0);
            let mut value = unit;
            // Same luminance ranges/end-point coefficients as the global controls. This local
            // point adjustment deliberately does not add a full-image guided-filter pass.
            if a.highlights != 0.0 || a.shadows != 0.0 {
                let highlights = smoothstep(0.45, 0.95, unit);
                let shadows = 1.0 - smoothstep(0.05, 0.5, unit);
                value *= 2f32.powf(a.highlights / 100.0 * 1.2 * highlights + a.shadows / 100.0 * 1.4 * shadows);
            }
            if a.whites != 0.0 {
                value += a.whites / 100.0 * 0.25 * smoothstep(0.4, 1.0, value);
            }
            if a.blacks != 0.0 {
                value += a.blacks / 100.0 * 0.15 * (1.0 - smoothstep(0.0, 0.45, value));
            }
            let value = value.max(0.0) + (luma - unit);
            rgb = if luma > 1e-5 { rgb.map(|v| v * (value / luma)) } else { rgb.map(|v| v + (value - luma)) };
        }
        if a.hue != 0.0 {
            // Preserve HDR value and a possible negative-channel offset when rotating hue.
            let offset = rgb[0].min(rgb[1]).min(rgb[2]).min(0.0);
            let [h, s, v] = crate::other2::rgb_to_hsb(rgb.map(|value| value - offset));
            rgb = crate::other2::hsb_to_rgb([h + a.hue / 360.0, s, v]).map(|value| value + offset);
        }
        let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
        rgb.map(|v| luma + (v - luma) * self.saturation)
    }
}

pub fn apply(px: &mut [[f32; 4]], w: usize, h: usize, items: &[LocalAdjustment], float: bool) {
    apply_scaled(px, w, h, items, float, 1.0, 0);
}

/// Apply local controls, carrying the parent's preview scale and deterministic grain seed.
/// Independent color stages are fused after coverage evaluation in one pass. Spatial filters
/// use a complete unmasked candidate, then blend once; neighbours retain the full image domain.
pub fn apply_scaled(px: &mut [[f32; 4]], w: usize, h: usize, items: &[LocalAdjustment], float: bool, pixel_scale: f32, seed: u32) {
    apply_inner(px, None, w, h, items, float, pixel_scale, seed);
}

/// Apply a suffix of local adjustments to an existing prefix result, while all range masks
/// continue sampling the immutable globally developed image. `anchor` must cover exactly
/// the full `w × h` image, with the same dimensions and pixel order as `px`.
/// Invalid dimensions/settings leave the entire destination unchanged.
#[allow(clippy::too_many_arguments)]
pub fn apply_scaled_anchored(
    px: &mut [[f32; 4]],
    anchor: &[[f32; 4]],
    w: usize,
    h: usize,
    items: &[LocalAdjustment],
    float: bool,
    pixel_scale: f32,
    seed: u32,
) {
    if w.checked_mul(h) != Some(px.len()) || anchor.len() != px.len() {
        return;
    }
    apply_inner(px, Some(anchor), w, h, items, float, pixel_scale, seed);
}

struct PreparedAdvanced {
    params: CameraRaw,
    color: crate::camera_raw::PreparedColor,
    grain: Option<crate::camera_raw::PreparedGrain>,
    spatial: bool,
}
impl PreparedAdvanced {
    fn new(a: &LocalAdjustment, scale: f32, seed: u32) -> Self {
        let params = a.advanced_params(scale, seed);
        Self {
            color: crate::camera_raw::PreparedColor::new(&params),
            grain: crate::camera_raw::PreparedGrain::new(&params, scale),
            spatial: a.is_spatial(),
            params,
        }
    }
}

fn copied_pixels(pixels: &[[f32; 4]]) -> Option<Vec<[f32; 4]>> {
    let mut result = Vec::new();
    result.try_reserve_exact(pixels.len()).ok()?;
    result.extend_from_slice(pixels);
    Some(result)
}

/// Build the complete unmasked candidate for one spatial group. The caller may reuse this
/// image while only group geometry changes, provided the input prefix, every color setting,
/// float mode, dimensions, preview scale, and seed remain equal. This is a full-domain
/// filter, with no masked/cropped neighbourhoods. Validation and allocation happen first.
pub fn build_spatial_candidate(
    input: &[[f32; 4]],
    w: usize,
    h: usize,
    adjustment: &LocalAdjustment,
    float: bool,
    pixel_scale: f32,
    seed: u32,
) -> Option<Vec<[f32; 4]>> {
    if w == 0
        || h == 0
        || w.checked_mul(h) != Some(input.len())
        || !pixel_scale.is_finite()
        || !adjustment.is_spatial()
        || validate(std::slice::from_ref(adjustment)).is_err()
    {
        return None;
    }
    let short = w.min(h) as f32;
    let prepared = PreparedAdjustment::new(adjustment, [w as f32 / short, h as f32 / short]);
    let params = adjustment.advanced_params(pixel_scale.clamp(0.01, 1.0), seed);
    let mut candidate = copied_pixels(input)?;
    develop_candidate(&mut candidate, w, h, &prepared, &params, float);
    Some(candidate)
}

/// Blend a previously built full-domain spatial candidate through the current geometry.
/// `anchor` is the immutable global-developed source used for range mask membership,
/// while `px` holds the input prefix. Returns false without mutating on invalid domains.
pub fn blend_spatial_candidate(px: &mut [[f32; 4]], anchor: &[[f32; 4]], candidate: &[[f32; 4]], w: usize, h: usize, adjustment: &LocalAdjustment) -> bool {
    if w == 0
        || h == 0
        || w.checked_mul(h) != Some(px.len())
        || anchor.len() != px.len()
        || candidate.len() != px.len()
        || !adjustment.is_spatial()
        || validate(std::slice::from_ref(adjustment)).is_err()
    {
        return false;
    }
    if adjustment.is_identity() {
        return true;
    }
    let short = w.min(h) as f32;
    let prepared = PreparedAdjustment::new(adjustment, [w as f32 / short, h as f32 / short]);
    blend_candidate(px, anchor, candidate, w, h, &prepared);
    true
}

fn develop_candidate(candidate: &mut [[f32; 4]], w: usize, h: usize, a: &PreparedAdjustment<'_>, params: &CameraRaw, float: bool) {
    if !a.base_identity {
        for_rows(candidate, w, |(_, row)| {
            for q in row.iter_mut() {
                if q[3] <= 0.0 {
                    continue;
                }
                let rgb = a.rgb([q[0], q[1], q[2]]);
                for (old, value) in q.iter_mut().take(3).zip(rgb) {
                    let value = if float { value } else { value.clamp(0.0, 1.0) };
                    *old += value - *old;
                }
            }
        });
    }
    crate::camera_raw::develop(candidate, w, h, params, float);
}

fn blend_candidate(px: &mut [[f32; 4]], anchor: &[[f32; 4]], candidate: &[[f32; 4]], w: usize, h: usize, a: &PreparedAdjustment<'_>) {
    for_rows(px, w, |(y, row)| {
        for (x, q) in row.iter_mut().enumerate() {
            let i = y.saturating_mul(w).saturating_add(x);
            let (Some(original), Some(developed)) = (anchor.get(i), candidate.get(i)) else { continue };
            if q[3] <= 0.0 {
                continue;
            }
            let m = a.mask.weight([(x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32], *original);
            if m <= 0.0 {
                continue;
            }
            for (old, value) in q.iter_mut().take(3).zip(developed) {
                if m >= 1.0 {
                    *old = *value;
                } else {
                    *old += (*value - *old) * m;
                }
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn apply_inner(px: &mut [[f32; 4]], anchor: Option<&[[f32; 4]]>, w: usize, h: usize, items: &[LocalAdjustment], float: bool, pixel_scale: f32, seed: u32) {
    if w == 0 || h == 0 || validate(items).is_err() {
        return;
    }
    let short = w.min(h) as f32;
    let aspect = [w as f32 / short, h as f32 / short];
    let prepared: Vec<_> = items.iter().filter(|a| !a.is_identity()).map(|a| PreparedAdjustment::new(a, aspect)).collect();
    if prepared.is_empty() {
        return;
    }
    let advanced = prepared.iter().any(|a| a.adjustment.has_advanced());
    if advanced && (w.checked_mul(h) != Some(px.len()) || !pixel_scale.is_finite()) {
        return;
    }
    let scale = pixel_scale.clamp(0.01, 1.0);
    let extras: Vec<_> = prepared.iter().map(|a| a.adjustment.has_advanced().then(|| PreparedAdvanced::new(a.adjustment, scale, seed))).collect();
    if extras.iter().flatten().any(|a| a.spatial) {
        // Spatial filters must see the complete, unmasked current prefix image. Only this
        // branch needs image-sized copies; pure point groups never allocate either buffer.
        let owned_anchor;
        let source = match anchor {
            Some(anchor) => anchor,
            None => {
                let Some(copy) = copied_pixels(px) else { return };
                owned_anchor = copy;
                &owned_anchor
            }
        };
        let Some(mut candidate) = copied_pixels(px) else { return };
        for (a, extra) in prepared.iter().zip(&extras) {
            if let Some(extra) = extra.as_ref().filter(|e| e.spatial) {
                candidate.copy_from_slice(px);
                develop_candidate(&mut candidate, w, h, a, &extra.params, float);
                blend_candidate(px, source, &candidate, w, h, a);
            } else {
                for_rows(px, w, |(y, row)| {
                    for (x, q) in row.iter_mut().enumerate() {
                        let Some(original) = source.get(y.saturating_mul(w).saturating_add(x)) else { continue };
                        apply_point(a, extra.as_ref(), q, *original, [x, y], [w, h], float);
                    }
                });
            }
        }
        return;
    }
    let count = px.len().min(w.saturating_mul(h));
    let Some(pixels) = px.get_mut(..count) else { return };
    for_rows(pixels, w, |(y, row)| {
        for (x, q) in row.iter_mut().enumerate() {
            let source = anchor.and_then(|a| a.get(y.saturating_mul(w).saturating_add(x))).copied().unwrap_or(*q);
            for (a, extra) in prepared.iter().zip(&extras) {
                apply_point(a, extra.as_ref(), q, source, [x, y], [w, h], float);
            }
        }
    });
}

#[inline]
fn apply_point(
    a: &PreparedAdjustment<'_>,
    extra: Option<&PreparedAdvanced>,
    q: &mut [f32; 4],
    source: [f32; 4],
    xy: [usize; 2],
    size: [usize; 2],
    float: bool,
) {
    if q[3] <= 0.0 {
        return;
    }
    let m = a.mask.weight([(xy[0] as f32 + 0.5) / size[0] as f32, (xy[1] as f32 + 0.5) / size[1] as f32], source);
    if m <= 0.0 {
        return;
    }
    let mut rgb = [q[0], q[1], q[2]];
    if let Some(extra) = extra {
        if !a.base_identity {
            let adjusted = a.rgb(rgb);
            // Preserve v63 candidate arithmetic, including nonfloat intermediate clamping.
            for (old, value) in rgb.iter_mut().zip(adjusted) {
                let value = if float { value } else { value.clamp(0.0, 1.0) };
                *old += value - *old;
            }
        }
        rgb = extra.color.rgb(rgb);
        if let Some(grain) = &extra.grain {
            rgb = grain.rgb(rgb, xy[0], xy[1]);
        }
    } else {
        rgb = a.rgb(rgb);
    }
    for (old, value) in q.iter_mut().take(3).zip(rgb) {
        let value = if float { value } else { value.clamp(0.0, 1.0) };
        if extra.is_some() && m >= 1.0 {
            *old = value;
        } else {
            *old += (value - *old) * m;
        }
    }
}

fn for_rows(pixels: &mut [[f32; 4]], w: usize, row: impl Fn((usize, &mut [[f32; 4]])) + Sync + Send) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        pixels.par_chunks_mut(w).enumerate().for_each(row);
    }
    #[cfg(target_arch = "wasm32")]
    pixels.chunks_mut(w).enumerate().for_each(row);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mask_validation_is_bounded_and_geometry_is_resolution_independent() {
        let a = LocalAdjustment {
            exposure: 1.0,
            mask: MaskShape::Radial { center: [0.5, 0.5], radius: [0.2, 0.2], feather: 0.5, rotation: 0.0 },
            ..Default::default()
        };
        assert!(validate(std::slice::from_ref(&a)).is_ok());
        for float in [false, true] {
            let mut p = vec![[0.2, 0.3, 0.4, 0.7]; 100];
            apply(&mut p, 10, 10, std::slice::from_ref(&a), float);
            assert_eq!(p[0], [0.2, 0.3, 0.4, 0.7]);
            assert!(p[55][0] > 0.2);
            assert!(p.iter().all(|p| p[3] == 0.7));
        }
        let mut bad = a.clone();
        bad.exposure = f32::NAN;
        assert!(validate(&[bad]).is_err());
        assert!(validate(&vec![a; MAX_MASKS + 1]).is_err());
    }
    #[test]
    fn brush_erase_and_inversion_do_not_change_alpha() {
        let stroke = BrushStroke { points: vec![[0.5, 0.5]], radius: 0.2, feather: 0.0, erase: false };
        let mut erase = stroke.clone();
        erase.erase = true;
        let mask = MaskShape::Brush { strokes: vec![stroke, erase] };
        assert_eq!(weight(&mask, [0.5, 0.5], [0.4; 4], [1.0, 1.0]), 0.0);
        let a = LocalAdjustment { mask, invert: true, exposure: 1.0, ..Default::default() };
        let mut px = [[0.4, 0.3, 0.2, 0.5]];
        apply(&mut px, 1, 1, &[a], true);
        assert_eq!(px[0], [0.8, 0.6, 0.4, 0.5]);
    }
    #[test]
    fn indexed_brush_is_identical_to_direct_geometry_across_aspects_and_erase_order() {
        let strokes = vec![
            BrushStroke { points: vec![[0.1, 0.2], [0.8, 0.7], [0.7, 0.9]], radius: 0.09, feather: 0.6, erase: false },
            BrushStroke { points: vec![[0.4, 0.4], [0.5, 0.6]], radius: 0.1, feather: 0.4, erase: true },
            BrushStroke { points: vec![[0.5, 0.55]], radius: 0.04, feather: 0.5, erase: false },
        ];
        let shape = MaskShape::Brush { strokes: strokes.clone() };
        for aspect in [[1.0, 1.0], [1.0, 2.0], [3.0, 1.0]] {
            let index = BrushIndex::new(&strokes, aspect);
            for y in 0..67 {
                for x in 0..79 {
                    let uv = [(x as f32 + 0.5) / 79.0, (y as f32 + 0.5) / 67.0];
                    assert!((index.weight(uv, aspect) - weight(&shape, uv, [0.5; 4], aspect)).abs() < 2e-6);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "camera_raw_masks_tests_v62.rs"]
mod tests_v62;

#[cfg(test)]
#[path = "camera_raw_masks_tests_v63.rs"]
mod tests_v63;

#[cfg(test)]
#[path = "camera_raw_masks_tests_v64.rs"]
mod tests_v64;

#[cfg(test)]
#[path = "camera_raw_masks_tests_v65.rs"]
mod tests_v65;

#[cfg(test)]
#[path = "camera_raw_masks_tests_v66.rs"]
mod tests_v66;
