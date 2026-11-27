//! Frozen v63 candidate construction with the independent pre-v64 global color oracle.
use super::*;

fn image(w: usize, h: usize) -> Vec<[f32; 4]> {
    (0..w * h)
        .map(|i| {
            [
                ((i * 37 % 251) as f32 - 35.0) / 140.0,
                ((i * 13 % 229) as f32 - 11.0) / 180.0,
                ((i * 61 % 257) as f32 - 19.0) / 170.0,
                if i % 23 == 0 { 0.0 } else { 0.73 },
            ]
        })
        .collect()
}
fn bits(p: &[[f32; 4]]) -> Vec<[u32; 4]> {
    p.iter().map(|p| p.map(f32::to_bits)).collect()
}
fn groups() -> Vec<LocalAdjustment> {
    let mut first: LocalAdjustment = serde_json::from_value(serde_json::json!({
        "exposure":0.4,"contrast":13,"temperature":19,"tint":-17,"hue":37,"highlights":23,"shadows":-21,"whites":-17,"blacks":11,"saturation":-27,
        "vibrance":53,"curveHighlights":-11,"curveLights":23,"curveDarks":-19,"curveShadows":17,
        "pointCurve":[[0,12],[91,120],[255,249]],"pointCurveRed":[[0,8],[125,157],[255,255]],
        "pointCurveGreen":[[0,0],[123,141],[255,249]],"pointCurveBlue":[[0,7],[121,109],[255,255]],
        "hslHue":[15,-21,28,33,-12,-25,37,-43],"hslSat":[-18,29,-32,13,28,47,-31,21],"hslLum":[29,-27,11,42,-13,17,23,-33],
        "gradeShadows":{"hue":39,"sat":53,"lum":-17},"gradeMidtones":{"hue":137,"sat":35,"lum":11},"gradeHighlights":{"hue":234,"sat":19,"lum":23},"gradeGlobal":{"hue":319,"sat":17,"lum":-7},
        "gradeBalance":-23,"gradeBlending":79,"grainAmount":29,"grainSize":43,"grainRoughness":71
    })).unwrap();
    first.mask = MaskShape::Radial { center: [0.4, 0.5], radius: [0.31, 0.37], feather: 0.45, rotation: 31.0 };
    first.components =
        vec![MaskComponent { name: "subtract".into(), subtract: true, mask: MaskShape::Linear { start: [0.2, 0.3], end: [0.8, 0.6] }, ..Default::default() }];
    let second = LocalAdjustment {
        mask: MaskShape::Luminance { min: 0.15, max: 0.6, feather: 0.17 },
        exposure: -0.4,
        vibrance: -41.0,
        grain_amount: 31.0,
        ..Default::default()
    };
    let third = LocalAdjustment {
        mask: MaskShape::Color { color: [0.5, 0.3, 0.1], tolerance: 0.48, feather: 0.21 },
        exposure: 0.7,
        contrast: 13.0,
        ..Default::default()
    };
    let fourth = LocalAdjustment {
        mask: MaskShape::Raster { width: 3, height: 2, values: vec![0, 128, 255, 89, 231, 31] },
        invert: true,
        point_curve_red: vec![[0.0, 0.0], [128.0, 183.0], [255.0, 255.0]],
        ..Default::default()
    };
    vec![first, second, third, fourth]
}
fn frozen(source: &[[f32; 4]], w: usize, h: usize, items: &[LocalAdjustment], float: bool, scale: f32, seed: u32) -> Vec<[f32; 4]> {
    let aspect = [w as f32 / w.min(h) as f32, h as f32 / w.min(h) as f32];
    let prepared: Vec<_> = items.iter().filter(|a| !a.is_identity()).map(|a| PreparedAdjustment::new(a, aspect)).collect();
    let mut out = source.to_vec();
    frozen_v63_advanced(&mut out, w, h, &prepared, float, scale, seed);
    out
}
#[test]
fn fused_pointwise_masks_match_frozen_v63_candidate_bits_in_hdr_and_fractional_coverage() {
    let (w, h) = (41, 29);
    let source = image(w, h);
    let mut items = groups();
    for float in [false, true] {
        for scale in [0.01, 0.25, 1.0] {
            for seed in [0, 127] {
                let expected = frozen(&source, w, h, &items, float, scale, seed);
                let mut actual = source.clone();
                apply_scaled(&mut actual, w, h, &items, float, scale, seed);
                assert_eq!(bits(&actual), bits(&expected), "float={float} scale={scale} seed={seed}");
            }
        }
    }
    items[0].enabled = false;
    items[1].mask_enabled = false;
    let mut actual = source.clone();
    apply_scaled(&mut actual, w, h, &items, false, 0.25, 127);
    assert_eq!(bits(&actual), bits(&frozen(&source, w, h, &items, false, 0.25, 127)));
}
#[test]
fn mixed_point_spatial_groups_and_every_anchored_prefix_match_frozen_v63_bits() {
    let (w, h) = (37, 31);
    let source = image(w, h);
    for spatial in [false, true] {
        let mut items = groups();
        if spatial {
            items[1].texture = 47.0;
            items[1].clarity = -29.0;
            items[1].noise_color = 23.0;
            items[3].sharpen_amount = 71.0;
        }
        for float in [false, true] {
            let expected = frozen(&source, w, h, &items, float, 0.25, 319);
            for split in 0..=items.len() {
                let mut actual = source.clone();
                apply_scaled(&mut actual, w, h, &items[..split], float, 0.25, 319);
                apply_scaled_anchored(&mut actual, &source, w, h, &items[split..], float, 0.25, 319);
                assert_eq!(bits(&actual), bits(&expected), "spatial={spatial}; float={float}; prefix={split}");
            }
        }
    }
}
#[test]
fn anchored_api_rejects_wrong_domain_or_nonfinite_scale_without_partial_changes() {
    let original = image(5, 4);
    let items = groups();
    for (w, h, anchor_len, scale) in [(5, 4, 19, 1.0), (4, 4, 20, 1.0), (usize::MAX, 2, 20, 1.0), (5, 4, 20, f32::NAN), (0, 4, 20, 1.0)] {
        let mut actual = original.clone();
        apply_scaled_anchored(&mut actual, &original[..anchor_len], w, h, &items, false, scale, 0);
        assert_eq!(bits(&actual), bits(&original));
    }
}

#[test]
fn spatial_candidate_reuse_preserves_immutable_anchor_and_all_geometry_edits() {
    let (w, h) = (31, 23);
    let anchor = image(w, h);
    let first = groups().remove(0);
    let mut prefix = anchor.clone();
    apply_scaled(&mut prefix, w, h, &[first], false, 0.4, 129);
    let a = LocalAdjustment {
        mask: MaskShape::Luminance { min: 0.1, max: 0.6, feather: 0.2 },
        exposure: 0.5,
        texture: 37.0,
        clarity: 29.0,
        grain_amount: 23.0,
        point_curve_blue: vec![[0.0, 9.0], [128.0, 151.0], [255.0, 255.0]],
        ..Default::default()
    };
    let candidate = build_spatial_candidate(&prefix, w, h, &a, false, 0.4, 129).unwrap();
    let mut changed = a.clone();
    changed.mask = MaskShape::Radial { center: [0.51, 0.47], radius: [0.23, 0.37], feather: 0.6, rotation: 53.0 };
    changed.invert = true;
    changed.components.push(MaskComponent { mask: MaskShape::Linear { start: [0.1, 0.2], end: [0.7, 0.8] }, ..Default::default() });
    let mut disabled = changed.clone();
    disabled.enabled = false;
    for a in [a, changed, disabled] {
        let mut expected = prefix.clone();
        apply_scaled_anchored(&mut expected, &anchor, w, h, std::slice::from_ref(&a), false, 0.4, 129);
        let mut actual = prefix.clone();
        assert!(blend_spatial_candidate(&mut actual, &anchor, &candidate, w, h, &a));
        assert_eq!(bits(&actual), bits(&expected));
    }
    let original = prefix.clone();
    assert!(!blend_spatial_candidate(&mut prefix, &anchor[..anchor.len() - 1], &candidate, w, h, &LocalAdjustment { texture: 1.0, ..Default::default() }));
    assert_eq!(bits(&prefix), bits(&original));
    assert!(build_spatial_candidate(&anchor, 0, h, &LocalAdjustment { texture: 1.0, ..Default::default() }, false, 1.0, 0).is_none());
    assert!(build_spatial_candidate(&anchor, w, h, &LocalAdjustment { vibrance: 1.0, ..Default::default() }, false, 1.0, 0).is_none());
}

#[test]
fn spatial_color_key_compares_every_serialized_color_field_and_excludes_geometry() {
    let source = LocalAdjustment::default();
    let json = serde_json::to_value(&source).unwrap();
    let excluded = ["name", "enabled", "invert", "mask", "maskEnabled", "components"];
    for (name, value) in json.as_object().unwrap() {
        if excluded.contains(&name.as_str()) {
            continue;
        }
        let mut changed = json.clone();
        let replacement = match value {
            serde_json::Value::Number(_) => serde_json::json!(value.as_f64().unwrap() + 0.125),
            serde_json::Value::Array(items) if items.is_empty() => serde_json::json!([[0, 1], [255, 255]]),
            serde_json::Value::Array(items) => {
                let mut items = items.clone();
                items[0] = serde_json::json!(items[0].as_f64().unwrap() + 0.125);
                serde_json::json!(items)
            }
            serde_json::Value::Object(_) => serde_json::json!({"hue":17,"sat":31,"lum":-11}),
            other => panic!("unexpected color field type {name}: {other}"),
        };
        changed[name] = replacement;
        let changed: LocalAdjustment = serde_json::from_value(changed).unwrap();
        assert!(!source.same_color_settings(&changed), "color cache key omitted {name}");
    }
    let mut geometry = source.clone();
    geometry.name = "new name".into();
    geometry.enabled = false;
    geometry.invert = true;
    geometry.mask_enabled = false;
    geometry.mask = MaskShape::Color { color: [0.4, 0.6, 0.8], tolerance: 0.3, feather: 0.4 };
    geometry.components.push(MaskComponent::default());
    assert!(source.same_color_settings(&geometry));
    assert!(!source.is_spatial());
    for name in ["texture", "clarity", "dehaze", "sharpenAmount", "noiseLuminance", "noiseColor"] {
        let mut value = json.clone();
        value[name] = serde_json::json!(1);
        let spatial: LocalAdjustment = serde_json::from_value(value).unwrap();
        assert!(spatial.is_spatial(), "{name}");
    }
}

fn frozen_v63_advanced(px: &mut [[f32; 4]], w: usize, h: usize, prepared: &[PreparedAdjustment<'_>], float: bool, pixel_scale: f32, seed: u32) {
    // The reused global spatial stages require a complete rectangular domain. Validate
    // it before allocating or mutating anything; saturated/partial hostile dimensions
    // keep their source unchanged instead of entering those array-based stages.
    if w.checked_mul(h) != Some(px.len()) || !pixel_scale.is_finite() {
        return;
    }
    let pixel_scale = pixel_scale.clamp(0.01, 1.0);
    let Some(source) = copied_pixels(px) else { return };
    // Reserve both buffers before any group commits, so a failed local allocation
    // cannot leave an edit partially applied. They are reused across all groups.
    let Some(mut candidate) = copied_pixels(px) else { return };
    for a in prepared {
        if !a.adjustment.has_advanced() {
            let row = |(y, pixels): (usize, &mut [[f32; 4]])| {
                let v = (y as f32 + 0.5) / h as f32;
                let original_row = source.get(y.saturating_mul(w)..).unwrap_or_default();
                for (x, (q, original)) in pixels.iter_mut().zip(original_row).enumerate() {
                    if q[3] <= 0.0 {
                        continue;
                    }
                    let m = a.mask.weight([(x as f32 + 0.5) / w as f32, v], *original);
                    if m <= 0.0 {
                        continue;
                    }
                    let rgb = a.rgb([q[0], q[1], q[2]]);
                    for (old, value) in q.iter_mut().take(3).zip(rgb) {
                        let value = if float { value } else { value.clamp(0.0, 1.0) };
                        *old += (value - *old) * m;
                    }
                }
            };
            for_rows(px, w, row);
            continue;
        }
        candidate.copy_from_slice(px);
        if !a.adjustment.base_is_identity() {
            for_rows(&mut candidate, w, |(_, pixels)| {
                for q in pixels {
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
        let params = a.adjustment.advanced_params(pixel_scale, seed);
        // advanced_params creates a leaf: local_adjustments is empty, preventing recursion.
        crate::camera_raw::pointwise_tests_v64::frozen_develop(&mut candidate, w, h, &params, float);
        for_rows(px, w, |(y, pixels)| {
            let v = (y as f32 + 0.5) / h as f32;
            let start = y.saturating_mul(w);
            let original_row = source.get(start..).unwrap_or_default();
            let candidate_row = candidate.get(start..).unwrap_or_default();
            for (x, ((q, original), developed)) in pixels.iter_mut().zip(original_row).zip(candidate_row).enumerate() {
                if q[3] <= 0.0 {
                    continue;
                }
                let m = a.mask.weight([(x as f32 + 0.5) / w as f32, v], *original);
                if m <= 0.0 {
                    continue;
                }
                // Blend once, preserving alpha and exact outside/transparent pixels.
                for (old, value) in q.iter_mut().take(3).zip(developed.iter()) {
                    if m >= 1.0 {
                        *old = *value;
                    } else {
                        *old += (*value - *old) * m;
                    }
                }
            }
        });
    }
}
