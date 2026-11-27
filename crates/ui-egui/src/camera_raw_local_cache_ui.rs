//! Exact full-proxy global stage reuse while only local masks change.
#[cfg(test)]
#[path = "camera_raw_local_cache_tests_v64.rs"]
mod tests_v64;
use photocraft_algo::camera_raw::{CameraRaw, develop};
use photocraft_algo::camera_raw_masks::{self as masks, LocalAdjustment};
use std::sync::Arc;

// At most one prefix and one unmasked spatial candidate. Large display proxies
// keep the exact uncached path rather than growing persistent working memory.
const LOCAL_CACHE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
struct LocalStage {
    items: Vec<LocalAdjustment>,
    index: usize,
    before: Arc<[[f32; 4]]>,
    candidate: Option<Arc<[[f32; 4]]>>,
}

/// Clone bounded global settings without copying any raster/brush mask payload.
/// Exhaustive destructuring makes newly added CameraRaw settings a compile-time obligation.
pub(super) fn global_params(params: &CameraRaw) -> CameraRaw {
    let CameraRaw {
        temperature,
        tint,
        exposure,
        contrast,
        highlights,
        shadows,
        whites,
        blacks,
        texture,
        clarity,
        dehaze,
        vibrance,
        saturation,
        curve_highlights,
        curve_lights,
        curve_darks,
        curve_shadows,
        curve_splits,
        point_curve,
        point_curve_red,
        point_curve_green,
        point_curve_blue,
        hsl_hue,
        hsl_sat,
        hsl_lum,
        grade_shadows,
        grade_midtones,
        grade_highlights,
        grade_global,
        grade_blending,
        grade_balance,
        sharpen_amount,
        sharpen_radius,
        sharpen_detail,
        sharpen_masking,
        noise_luminance,
        noise_luminance_detail,
        noise_color,
        noise_color_detail,
        grain_amount,
        grain_size,
        grain_roughness,
        vignette_amount,
        vignette_midpoint,
        vignette_roundness,
        vignette_feather,
        vignette_highlights,
        vignette_style,
        seed,
        color_lookup,
        color_spaces,
        local_adjustments: _,
        pixel_scale,
    } = params;
    CameraRaw {
        temperature: *temperature,
        tint: *tint,
        exposure: *exposure,
        contrast: *contrast,
        highlights: *highlights,
        shadows: *shadows,
        whites: *whites,
        blacks: *blacks,
        texture: *texture,
        clarity: *clarity,
        dehaze: *dehaze,
        vibrance: *vibrance,
        saturation: *saturation,
        curve_highlights: *curve_highlights,
        curve_lights: *curve_lights,
        curve_darks: *curve_darks,
        curve_shadows: *curve_shadows,
        curve_splits: *curve_splits,
        point_curve: point_curve.clone(),
        point_curve_red: point_curve_red.clone(),
        point_curve_green: point_curve_green.clone(),
        point_curve_blue: point_curve_blue.clone(),
        hsl_hue: *hsl_hue,
        hsl_sat: *hsl_sat,
        hsl_lum: *hsl_lum,
        grade_shadows: *grade_shadows,
        grade_midtones: *grade_midtones,
        grade_highlights: *grade_highlights,
        grade_global: *grade_global,
        grade_blending: *grade_blending,
        grade_balance: *grade_balance,
        sharpen_amount: *sharpen_amount,
        sharpen_radius: *sharpen_radius,
        sharpen_detail: *sharpen_detail,
        sharpen_masking: *sharpen_masking,
        noise_luminance: *noise_luminance,
        noise_luminance_detail: *noise_luminance_detail,
        noise_color: *noise_color,
        noise_color_detail: *noise_color_detail,
        grain_amount: *grain_amount,
        grain_size: *grain_size,
        grain_roughness: *grain_roughness,
        vignette_amount: *vignette_amount,
        vignette_midpoint: *vignette_midpoint,
        vignette_roundness: *vignette_roundness,
        vignette_feather: *vignette_feather,
        vignette_highlights: *vignette_highlights,
        vignette_style: vignette_style.clone(),
        seed: *seed,
        color_lookup: color_lookup.clone(),
        color_spaces: color_spaces.clone(),
        local_adjustments: Vec::new(),
        pixel_scale: *pixel_scale,
    }
}

#[derive(Clone)]
pub(super) struct GlobalStage {
    params: CameraRaw,
    source: Arc<[[f32; 4]]>,
    pixels: Arc<[[f32; 4]]>,
    size: [usize; 2],
    float: bool,
    local: Option<LocalStage>,
    prefix_hit: bool,
    candidate_hit: bool,
}
impl GlobalStage {
    pub(super) fn bytes(&self) -> usize {
        self.pixels.len().saturating_mul(std::mem::size_of::<[f32; 4]>())
    }
    pub(super) fn local_bytes(&self) -> usize {
        self.local.as_ref().map_or(0, |cache| {
            let prefix = if Arc::ptr_eq(&cache.before, &self.pixels) { 0 } else { cache.before.len().saturating_mul(16) };
            prefix.saturating_add(cache.candidate.as_ref().map_or(0, |pixels| pixels.len().saturating_mul(16)))
        })
    }
    pub(super) fn prefix_hit(&self) -> bool {
        self.prefix_hit
    }
    pub(super) fn candidate_hit(&self) -> bool {
        self.candidate_hit
    }
}

pub(super) struct Output {
    pub pixels: Vec<[f32; 4]>,
    pub global: Option<GlobalStage>,
    pub hit: bool,
}

pub(super) fn render(source: &Arc<[[f32; 4]]>, w: usize, h: usize, params: &CameraRaw, float: bool, previous: Option<&GlobalStage>) -> Output {
    if params.local_adjustments.is_empty() || params.is_identity() || params.color_spaces.working != photocraft_algo::camera_raw_color::RgbSpace::Document {
        let mut pixels = source.to_vec();
        develop(&mut pixels, w, h, params, float);
        return Output { pixels, global: None, hit: false };
    }
    let global = global_params(params);
    let cached = previous.filter(|cache| cache.params == global && cache.size == [w, h] && cache.float == float && Arc::ptr_eq(&cache.source, source));
    let mut stage = if let Some(cached) = cached {
        cached.clone()
    } else {
        let mut pixels = source.to_vec();
        develop(&mut pixels, w, h, &global, float);
        // develop returns early for a neutral global configuration, whereas a non-neutral
        // local pipeline reaches its integer-domain clamp immediately before local masks.
        if global.is_identity() && !float {
            for pixel in &mut pixels {
                for channel in &mut pixel[..3] {
                    *channel = channel.clamp(0.0, 1.0);
                }
            }
        }
        GlobalStage { params: global, source: source.clone(), pixels: pixels.into(), size: [w, h], float, local: None, prefix_hit: false, candidate_hit: false }
    };
    let pixels = render_local(&mut stage, params, w, h, float);
    Output { pixels, global: Some(stage), hit: cached.is_some() }
}

fn render_local(stage: &mut GlobalStage, params: &CameraRaw, w: usize, h: usize, float: bool) -> Vec<[f32; 4]> {
    let items = &params.local_adjustments;
    let old = stage.local.take();
    stage.prefix_hit = false;
    stage.candidate_hit = false;
    let frame_bytes = stage.pixels.len().saturating_mul(16);
    if frame_bytes > LOCAL_CACHE_BYTES || masks::validate(items).is_err() || items.len() == 1 && !items[0].is_spatial() {
        let mut pixels = stage.pixels.to_vec();
        masks::apply_scaled(&mut pixels, w, h, items, float, params.pixel_scale, params.seed);
        return pixels;
    }
    let changed = old.as_ref().map(|previous| previous.items.iter().zip(items).position(|(a, b)| a != b).unwrap_or(previous.items.len().min(items.len())));
    let index = changed.filter(|index| *index < items.len()).unwrap_or_else(|| {
        old.as_ref()
            .map_or_else(|| items.iter().position(LocalAdjustment::is_spatial).unwrap_or(0), |previous| previous.index.min(items.len().saturating_sub(1)))
    });
    let reusable = old.as_ref().filter(|previous| previous.index <= index && previous.items.get(..previous.index) == items.get(..previous.index));
    let (mut pixels, start) = if let Some(previous) = reusable {
        stage.prefix_hit = previous.index > 0;
        (previous.before.to_vec(), previous.index)
    } else {
        (stage.pixels.to_vec(), 0)
    };
    if let Some(prefix) = items.get(start..index) {
        masks::apply_scaled_anchored(&mut pixels, &stage.pixels, w, h, prefix, float, params.pixel_scale, params.seed);
    }
    let before: Arc<[[f32; 4]]> = if index == 0 { stage.pixels.clone() } else { pixels.clone().into() };
    let mut candidate = None;
    if let Some(item) = items.get(index) {
        if item.is_spatial() && !item.is_identity() {
            let prior_candidate = reusable
                .filter(|previous| previous.index == index)
                .filter(|previous| previous.items.get(index).is_some_and(|old_item| old_item.same_color_settings(item)))
                .and_then(|previous| previous.candidate.clone());
            stage.candidate_hit = prior_candidate.is_some();
            candidate = prior_candidate.or_else(|| masks::build_spatial_candidate(&before, w, h, item, float, params.pixel_scale, params.seed).map(Arc::from));
            if let Some(candidate) = &candidate {
                masks::blend_spatial_candidate(&mut pixels, &stage.pixels, candidate, w, h, item);
            } else {
                masks::apply_scaled_anchored(&mut pixels, &stage.pixels, w, h, std::slice::from_ref(item), float, params.pixel_scale, params.seed);
            }
        } else {
            masks::apply_scaled_anchored(&mut pixels, &stage.pixels, w, h, std::slice::from_ref(item), float, params.pixel_scale, params.seed);
        }
    }
    if let Some(suffix) = items.get(index.saturating_add(1)..) {
        masks::apply_scaled_anchored(&mut pixels, &stage.pixels, w, h, suffix, float, params.pixel_scale, params.seed);
    }
    let prefix_bytes = if index == 0 { 0 } else { frame_bytes };
    if prefix_bytes.saturating_add(frame_bytes) > LOCAL_CACHE_BYTES {
        candidate = None;
    }
    stage.local = Some(LocalStage { items: items.clone(), index, before, candidate });
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_algo::camera_raw_masks::{LocalAdjustment, MaskShape};
    fn source(w: usize, h: usize) -> Arc<[[f32; 4]]> {
        (0..w * h)
            .map(|i| [((i * 17) % 301) as f32 / 255., ((i * 31) % 256) as f32 / 255., ((i * 7) % 256) as f32 / 255., if i % 5 == 0 { 0.3 } else { 1. }])
            .collect::<Vec<_>>()
            .into()
    }
    fn params() -> CameraRaw {
        CameraRaw {
            local_adjustments: vec![LocalAdjustment {
                exposure: 0.7,
                contrast: 10.,
                mask: MaskShape::Linear { start: [0.1, 0.2], end: [0.8, 0.7] },
                ..Default::default()
            }],
            ..Default::default()
        }
    }
    #[test]
    fn cached_global_stage_is_exact_and_invalidated_by_global_source_or_format_changes() {
        let (w, h) = (96, 64);
        let source = source(w, h);
        for float in [false, true] {
            for spatial in [false, true] {
                let mut p = params();
                if spatial {
                    p.highlights = -20.;
                    p.texture = 15.;
                    p.noise_luminance = 8.;
                    p.grain_amount = 10.;
                }
                let first = render(&source, w, h, &p, float, None);
                assert!(!first.hit);
                for step in 0..4 {
                    p.local_adjustments[0].exposure = step as f32 * 0.25;
                    let cached = render(&source, w, h, &p, float, first.global.as_ref());
                    assert!(cached.hit);
                    let mut oracle = source.to_vec();
                    develop(&mut oracle, w, h, &p, float);
                    for (a, b) in cached.pixels.iter().flatten().zip(oracle.iter().flatten()) {
                        assert_eq!(a.to_bits(), b.to_bits());
                    }
                }
                p.exposure = 0.3;
                assert!(!render(&source, w, h, &p, float, first.global.as_ref()).hit);
                assert!(!render(&source, w, h, &params(), !float, first.global.as_ref()).hit);
                let other = source.to_vec().into();
                assert!(!render(&other, w, h, &params(), float, first.global.as_ref()).hit);
            }
        }
    }
    #[test]
    fn neutral_masks_skip_the_cache_and_local_edit_never_mutates_retained_global_pixels() {
        let source = source(32, 24);
        let p = params();
        let first = render(&source, 32, 24, &p, true, None);
        let before = first.global.as_ref().unwrap().pixels.clone();
        let second = render(&source, 32, 24, &p, true, first.global.as_ref());
        assert!(second.hit);
        assert_eq!(before, first.global.as_ref().unwrap().pixels);
        let neutral = CameraRaw { local_adjustments: vec![LocalAdjustment::default()], ..Default::default() };
        let output = render(&source, 32, 24, &neutral, true, first.global.as_ref());
        assert!(output.global.is_none());
        assert_eq!(output.pixels, source.as_ref());
    }

    #[test]
    fn advanced_local_proxy_uses_identical_scale_seed_and_global_cache() {
        let (w, h) = (64, 48);
        let source = source(w, h);
        for float in [false, true] {
            let mut p = params();
            p.pixel_scale = 0.3;
            p.seed = 31415;
            p.texture = 12.0;
            p.local_adjustments[0].texture = 17.0;
            p.local_adjustments[0].sharpen_amount = 25.0;
            p.local_adjustments[0].grain_amount = 18.0;
            p.local_adjustments[0].hsl_hue[0] = 15.0;
            p.local_adjustments[0].point_curve_blue = vec![[0.0, 8.0], [255.0, 248.0]];
            let first = render(&source, w, h, &p, float, None);
            let saved = first.global.as_ref().unwrap().pixels.clone();
            for step in 0..3 {
                p.local_adjustments[0].exposure = step as f32 * 0.2;
                let out = render(&source, w, h, &p, float, first.global.as_ref());
                assert!(out.hit);
                let mut expected = source.to_vec();
                develop(&mut expected, w, h, &p, float);
                for (actual, expected) in out.pixels.iter().flatten().zip(expected.iter().flatten()) {
                    assert_eq!(actual.to_bits(), expected.to_bits());
                }
                assert_eq!(saved, first.global.as_ref().unwrap().pixels);
            }
        }
    }
}
