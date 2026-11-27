//! Local advanced-control witnesses against the shared global pipeline, not a PS oracle.
use super::*;
use serde_json::{Value, json};

fn solid(value: u8) -> MaskShape {
    MaskShape::Raster { width: 1, height: 1, values: vec![value] }
}
fn image(w: usize, h: usize) -> Vec<[f32; 4]> {
    (0..w * h)
        .map(|i| {
            let x = i % w;
            let y = i / w;
            let noise = ((i * 113 + 37) % 97) as f32 / 97.0;
            [
                0.12 + 0.61 * x as f32 / w as f32 + 0.10 * noise,
                0.08 + 0.56 * y as f32 / h as f32 + 0.09 * noise,
                0.11 + 0.34 * ((x / 3 + y / 2) % 7) as f32 / 7.0 + 0.11 * noise,
                if i % 29 == 0 { 0.0 } else { 0.3 + 0.7 * noise },
            ]
        })
        .collect()
}
fn bits(px: &[[f32; 4]]) -> Vec<[u32; 4]> {
    px.iter().map(|q| q.map(f32::to_bits)).collect()
}
fn adjustment(changes: Value) -> LocalAdjustment {
    let mut value = json!({"mask":{"kind":"raster","width":1,"height":1,"values":[255]}});
    value.as_object_mut().unwrap().extend(changes.as_object().unwrap().clone());
    serde_json::from_value(value).unwrap()
}

#[test]
fn old_json_keeps_advanced_defaults_neutral_and_shared_editor_roundtrip_preserves_base_geometry() {
    let mut a = adjustment(json!({"exposure":0.4,"hue":23}));
    assert!(!a.has_advanced());
    assert_eq!(a.advanced_params(1.0, 0), CameraRaw::default());
    let original_mask = a.mask.clone();
    let params = CameraRaw {
        exposure: 4.0,
        temperature: 42.0,
        saturation: 21.0,
        vibrance: 12.0,
        texture: 13.0,
        clarity: -14.0,
        dehaze: 15.0,
        grain_amount: 16.0,
        grain_size: 17.0,
        grain_roughness: 18.0,
        curve_highlights: -19.0,
        curve_lights: 20.0,
        curve_darks: 21.0,
        curve_shadows: -22.0,
        curve_splits: [22.0, 45.0, 78.0],
        point_curve: vec![[0.0, 0.0], [255.0, 245.0]],
        point_curve_red: vec![[0.0, 5.0], [255.0, 255.0]],
        point_curve_green: vec![[0.0, 0.0], [255.0, 249.0]],
        point_curve_blue: vec![[0.0, 0.0], [255.0, 241.0]],
        hsl_hue: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
        hsl_sat: [8.0; 8],
        hsl_lum: [-9.0; 8],
        grade_shadows: Wheel { hue: 15.0, sat: 16.0, lum: 17.0 },
        grade_midtones: Wheel { hue: 115.0, sat: 26.0, lum: -17.0 },
        grade_highlights: Wheel { hue: 215.0, sat: 36.0, lum: 27.0 },
        grade_global: Wheel { hue: 315.0, sat: 46.0, lum: -27.0 },
        grade_blending: 71.0,
        grade_balance: -12.0,
        sharpen_amount: 81.0,
        sharpen_radius: 2.1,
        sharpen_detail: 61.0,
        sharpen_masking: 51.0,
        noise_luminance: 41.0,
        noise_luminance_detail: 31.0,
        noise_color: 21.0,
        noise_color_detail: 11.0,
        pixel_scale: 0.25,
        seed: 123,
        local_adjustments: vec![LocalAdjustment::default()],
        ..Default::default()
    };
    a.update_advanced(&params);
    let advanced = a.advanced_params(0.25, 123);
    let expected = CameraRaw { exposure: 0.0, temperature: 0.0, saturation: 0.0, local_adjustments: Vec::new(), ..params };
    assert_eq!(advanced, expected);
    assert_eq!(a.exposure, 0.4);
    assert_eq!(a.hue, 23.0);
    assert_eq!(a.mask, original_mask);
    assert!(advanced.local_adjustments.is_empty());
    assert!(validate(std::slice::from_ref(&a)).is_ok());
    assert_eq!(serde_json::from_value::<LocalAdjustment>(serde_json::to_value(&a).unwrap()).unwrap(), a);
}

#[test]
fn every_advanced_family_changes_pixels_and_full_coverage_matches_global_algorithm_exactly() {
    let cases = [
        ("vibrance", json!({"vibrance":55})),
        ("texture", json!({"texture":65})),
        ("clarity", json!({"clarity":50})),
        ("dehaze", json!({"dehaze":35})),
        ("grain", json!({"grainAmount":55,"grainSize":41,"grainRoughness":73})),
        ("parametric", json!({"curveHighlights":15,"curveLights":40,"curveDarks":-30,"curveShadows":-20,"curveSplits":[20,45,80]})),
        ("curve", json!({"pointCurve":[[0,0],[120,160],[255,255]]})),
        ("red curve", json!({"pointCurveRed":[[0,0],[128,165],[255,255]]})),
        ("green curve", json!({"pointCurveGreen":[[0,0],[128,165],[255,255]]})),
        ("blue curve", json!({"pointCurveBlue":[[0,0],[128,165],[255,255]]})),
        ("mixer hue", json!({"hslHue":[50,40,-30,20,-40,25,50,-35]})),
        ("mixer saturation", json!({"hslSat":[50,40,-30,20,-40,25,50,-35]})),
        ("mixer luminance", json!({"hslLum":[50,40,-30,20,-40,25,50,-35]})),
        ("grade shadows", json!({"gradeShadows":{"hue":35,"sat":40,"lum":12}})),
        ("grade midtones", json!({"gradeMidtones":{"hue":135,"sat":35,"lum":-10},"gradeBlending":65,"gradeBalance":-15})),
        ("grade highlights", json!({"gradeHighlights":{"hue":235,"sat":30,"lum":16}})),
        ("grade global", json!({"gradeGlobal":{"hue":315,"sat":25,"lum":-12}})),
        ("sharpen", json!({"sharpenAmount":110,"sharpenRadius":2.4,"sharpenDetail":65,"sharpenMasking":35})),
        ("luminance denoise", json!({"noiseLuminance":65,"noiseLuminanceDetail":20})),
        ("chroma denoise", json!({"noiseColor":70,"noiseColorDetail":25})),
    ];
    let (w, h) = (47, 35);
    let original = image(w, h);
    for (name, changes) in cases {
        let a = adjustment(changes);
        assert!(!a.is_identity(), "{name}");
        assert!(validate(std::slice::from_ref(&a)).is_ok(), "{name}");
        let mut expected = original.clone();
        crate::camera_raw::develop(&mut expected, w, h, &a.advanced_params(0.5, 72), true);
        for (q, o) in expected.iter_mut().zip(&original) {
            if o[3] <= 0.0 {
                *q = *o;
            }
        }
        assert_ne!(bits(&expected), bits(&original), "Inactive parameter family: {name}");
        let mut actual = original.clone();
        apply_scaled(&mut actual, w, h, std::slice::from_ref(&a), true, 0.5, 72);
        assert_eq!(bits(&actual), bits(&expected), "{name}: full coverage diverges from shared algorithm");
    }
}

#[test]
fn advanced_candidate_is_blended_once_and_spatial_neighbours_are_not_pre_masked() {
    let (w, h) = (31, 23);
    let original = image(w, h);
    let a = adjustment(json!({"exposure":0.4,"texture":65,"clarity":35,"noiseLuminance":25,"grainAmount":20}));
    let base = LocalAdjustment { exposure: a.exposure, mask: solid(255), ..Default::default() };
    let mut candidate = original.clone();
    apply(&mut candidate, w, h, &[base], true);
    crate::camera_raw::develop(&mut candidate, w, h, &a.advanced_params(0.4, 92), true);
    let fractional = LocalAdjustment { mask: solid(128), ..a.clone() };
    let mut actual = original.clone();
    apply_scaled(&mut actual, w, h, &[fractional], true, 0.4, 92);
    let m = 128.0f32 / 255.0;
    for ((q, o), processed) in actual.iter().zip(&original).zip(&candidate) {
        let mut expected = *o;
        if o[3] > 0.0 {
            for (dst, value) in expected.iter_mut().take(3).zip(processed) {
                *dst += (*value - *dst) * m;
            }
        }
        assert_eq!(q.map(f32::to_bits), expected.map(f32::to_bits));
    }
    let radial = LocalAdjustment { mask: MaskShape::Radial { center: [0.5, 0.5], radius: [0.25, 0.27], feather: 0.4, rotation: 23.0 }, ..a };
    let mut radial_pixels = original.clone();
    apply_scaled(&mut radial_pixels, w, h, std::slice::from_ref(&radial), true, 0.4, 92);
    let aspect = [w as f32 / h as f32, 1.0];
    for (i, ((q, o), processed)) in radial_pixels.iter().zip(&original).zip(&candidate).enumerate() {
        let m = group_weight(&radial, [(i % w) as f32 / w as f32 + 0.5 / w as f32, (i / w) as f32 / h as f32 + 0.5 / h as f32], *o, aspect);
        let mut expected = *o;
        if o[3] > 0.0 && m > 0.0 {
            for (dst, value) in expected.iter_mut().take(3).zip(processed) {
                if m >= 1.0 {
                    *dst = *value;
                } else {
                    *dst += (*value - *dst) * m;
                }
            }
        }
        for (ch, (a, b)) in q.iter().zip(expected).enumerate() {
            assert!((*a - b).abs() < 2e-6, "pixel={i},ch={ch}");
        }
    }
}

#[test]
fn advanced_range_masks_are_source_anchored_and_disabled_groups_are_exact_identity() {
    let source = vec![[0.3, 0.2, 0.1, 1.0]; 12];
    let first = adjustment(json!({"pointCurve":[[0,150],[255,255]]}));
    let mut one = source.clone();
    apply(&mut one, 4, 3, std::slice::from_ref(&first), true);
    let second = LocalAdjustment { mask: MaskShape::Luminance { min: 0.55, max: 1.0, feather: 0.001 }, vibrance: 75.0, exposure: 1.0, ..Default::default() };
    assert!(group_weight(&second, [0.5, 0.5], one[0], [4.0 / 3.0, 1.0]) > 0.99, "first group must cross the next mask threshold");
    let mut both = source.clone();
    apply(&mut both, 4, 3, &[first, second], true);
    assert_eq!(bits(&both), bits(&one));
    let disabled = LocalAdjustment { enabled: false, texture: 75.0, ..Default::default() };
    let empty = LocalAdjustment { mask_enabled: false, texture: 75.0, ..Default::default() };
    for group in [disabled, empty] {
        let mut p = source.clone();
        apply(&mut p, 4, 3, &[group], true);
        assert_eq!(bits(&p), bits(&source));
    }
}

#[test]
fn advanced_invalid_fields_and_dimensions_leave_source_unchanged() {
    for changes in [
        json!({"vibrance":101}),
        json!({"grainSize":-1}),
        json!({"sharpenAmount":151}),
        json!({"sharpenRadius":0.1}),
        json!({"curveSplits":[50,25,75]}),
        json!({"pointCurveBlue":[[50,50],[50,80]]}),
        json!({"gradeGlobal":{"hue":361,"sat":10,"lum":0}}),
        json!({"hslHue":[0,0,0,0,0,0,0,101]}),
    ] {
        let a = adjustment(changes);
        assert!(validate(std::slice::from_ref(&a)).is_err());
        let mut px = image(5, 4);
        let before = px.clone();
        apply(&mut px, 5, 4, &[a], false);
        assert_eq!(bits(&px), bits(&before));
    }
    for (w, h, scale) in [(usize::MAX, 2, 1.0), (4, 3, 1.0), (5, 4, f32::NAN)] {
        let mut px = image(5, 4);
        let before = px.clone();
        apply_scaled(&mut px, w, h, &[LocalAdjustment { texture: 35.0, ..Default::default() }], false, scale, 0);
        assert_eq!(bits(&px), bits(&before));
    }
    let invalid = LocalAdjustment { texture: f32::INFINITY, ..Default::default() };
    assert!(validate(&[invalid]).is_err());
    assert!(serde_json::from_value::<LocalAdjustment>(json!({"advanced":{"localAdjustments":[{}]}})).is_err());
}

#[test]
fn advanced_grain_uses_parent_scale_seed_and_is_deterministic() {
    let (w, h) = (19, 17);
    let source = image(w, h);
    let a = adjustment(json!({"grainAmount":60,"grainSize":35,"grainRoughness":45}));
    let mut first = source.clone();
    apply_scaled(&mut first, w, h, std::slice::from_ref(&a), false, 0.25, 122);
    let mut same = source.clone();
    apply_scaled(&mut same, w, h, std::slice::from_ref(&a), false, 0.25, 122);
    assert_eq!(bits(&first), bits(&same));
    let mut scale = source.clone();
    apply_scaled(&mut scale, w, h, std::slice::from_ref(&a), false, 0.5, 122);
    assert_ne!(bits(&scale), bits(&first));
    let mut seed = source;
    apply_scaled(&mut seed, w, h, &[a], false, 0.25, 123);
    assert_ne!(bits(&seed), bits(&first));
}

#[test]
fn advanced_proxy_scale_matches_global_finite_clamping_for_panoramas() {
    let (w, h) = (29, 13);
    let source = image(w, h);
    let a = adjustment(json!({"sharpenAmount":65,"sharpenRadius":1.7,"noiseColor":35,"grainAmount":25}));
    for (provided, clamped) in [(0.001, 0.01), (0.0, 0.01), (-0.1, 0.01), (1.5, 1.0)] {
        let mut actual = source.clone();
        apply_scaled(&mut actual, w, h, std::slice::from_ref(&a), false, provided, 62);
        let mut expected = source.clone();
        apply_scaled(&mut expected, w, h, std::slice::from_ref(&a), false, clamped, 62);
        assert_eq!(bits(&actual), bits(&expected), "{provided}");
    }
}
