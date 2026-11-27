//! v65 ordered intersection, stored geometry and pre-intersection compatibility witnesses.
//! These compare PhotoCraft arithmetic, not Photoshop implementation or formulas.
use super::*;
use serde_json::json;

fn constant(value: u8) -> MaskShape {
    MaskShape::Raster { width: 1, height: 1, values: vec![value] }
}
fn leaf(mask: MaskShape, subtract: bool, intersect: bool) -> MaskComponent {
    MaskComponent { name: "Leaf".into(), subtract, intersect, mask, ..Default::default() }
}
fn bits(pixels: &[[f32; 4]]) -> Vec<[u32; 4]> {
    pixels.iter().map(|pixel| pixel.map(f32::to_bits)).collect()
}
fn image(w: usize, h: usize) -> Vec<[f32; 4]> {
    (0..w * h)
        .map(|i| {
            [
                ((i * 37 % 251) as f32 - 17.0) / 173.0,
                ((i * 13 % 229) as f32 - 11.0) / 199.0,
                ((i * 61 % 257) as f32 - 19.0) / 183.0,
                if i % 17 == 0 { 0.0 } else { 0.67 },
            ]
        })
        .collect()
}
fn legacy_weight(a: &LocalAdjustment, uv: [f32; 2], source: [f32; 4], aspect: [f32; 2]) -> f32 {
    // Frozen pre-v65 add/subtract expressions, evaluated independently of PreparedLocalMask.
    let mut coverage = if a.mask_enabled { weight(&a.mask, uv, source, aspect) } else { 0.0 };
    for c in a.components.iter().filter(|c| c.enabled) {
        assert!(!c.intersect);
        let next = weight(&c.mask, uv, source, aspect);
        coverage = if c.subtract { coverage * (1.0 - next) } else { coverage.max(next) };
    }
    if a.invert { 1.0 - coverage } else { coverage }
}

#[test]
fn old_component_json_defaults_false_and_new_intersection_roundtrips_without_recursion() {
    let old = json!({"name":"Old group","exposure":0.4,"mask":{"kind":"raster","width":1,"height":1,"values":[128]},
        "components":[{"name":"Old addition","mask":{"kind":"linear","start":[0,0.5],"end":[1,0.5]}},
        {"name":"Old subtraction","subtract":true,"mask":{"kind":"raster","width":1,"height":1,"values":[64]}}]});
    let legacy: LocalAdjustment = serde_json::from_value(old.clone()).unwrap();
    assert!(legacy.components.iter().all(|c| !c.intersect && !c.invert_previous));
    assert!(validate(std::slice::from_ref(&legacy)).is_ok());
    let mut explicit = old;
    for c in explicit["components"].as_array_mut().unwrap() {
        c["intersect"] = json!(false);
        c["invertPrevious"] = json!(false);
    }
    assert_eq!(legacy, serde_json::from_value::<LocalAdjustment>(explicit).unwrap());
    let mut a = legacy;
    a.components.push(leaf(constant(192), false, true));
    assert!(validate(std::slice::from_ref(&a)).is_ok());
    let decoded: LocalAdjustment = serde_json::from_value(serde_json::to_value(&a).unwrap()).unwrap();
    assert_eq!(decoded, a);
    assert!(decoded.components.last().unwrap().intersect);
    for enabled in [false, true] {
        let bad = LocalAdjustment { components: vec![MaskComponent { enabled, subtract: true, intersect: true, ..Default::default() }], ..Default::default() };
        assert!(validate(std::slice::from_ref(&bad)).is_err());
        let mut actual = image(3, 2);
        let original = actual.clone();
        apply(&mut actual, 3, 2, &[bad], false);
        assert_eq!(bits(&actual), bits(&original));
        for subtract in [false, true] {
            let transferred = LocalAdjustment {
                exposure: 1.0,
                components: vec![MaskComponent { enabled, subtract, invert_previous: true, intersect: false, ..Default::default() }],
                ..Default::default()
            };
            assert!(validate(std::slice::from_ref(&transferred)).is_ok(), "deletion may transfer unary boundary to any successor");
        }
    }
    assert!(serde_json::from_value::<MaskComponent>(json!({"intersect":true,"mask":{"kind":"group","components":[]}})).is_err());
}

#[test]
fn ordered_intersection_multiplies_feather_before_later_operations_and_group_inversion() {
    let radial = MaskShape::Radial { center: [0.5; 2], radius: [0.43, 0.37], feather: 0.7, rotation: 0.0 };
    let horizontal = MaskShape::Linear { start: [0.0, 0.5], end: [1.0, 0.5] };
    let mut a = LocalAdjustment {
        mask: constant(192),
        components: vec![
            leaf(constant(128), false, false),
            leaf(horizontal.clone(), false, true),
            leaf(constant(64), true, false),
            leaf(radial.clone(), false, true),
        ],
        ..Default::default()
    };
    let aspect = [1.5, 1.0];
    let mut saw_fractional = false;
    for inverse in [false, true] {
        a.invert = inverse;
        let prepared = PreparedLocalMask::new(&a, aspect);
        for y in 0..31 {
            for x in 0..47 {
                let uv = [(x as f32 + 0.5) / 47.0, (y as f32 + 0.5) / 31.0];
                let h = weight(&horizontal, uv, [0.4; 4], aspect);
                let r = weight(&radial, uv, [0.4; 4], aspect);
                // Even a constant raster follows bilinear interpolation's f32
                // arithmetic at each UV. Use scalar shape samples, then verify
                // the independent ordered combination bit-for-bit.
                let base = weight(&a.mask, uv, [0.4; 4], aspect);
                let added = weight(&constant(128), uv, [0.4; 4], aspect);
                let subtracted = weight(&constant(64), uv, [0.4; 4], aspect);
                let coverage = base.max(added) * h * (1.0 - subtracted) * r;
                let expected = if inverse { 1.0 - coverage } else { coverage };
                assert_eq!(prepared.weight(uv, [0.4; 4]).to_bits(), expected.to_bits());
                saw_fractional |= expected > 0.0 && expected < 1.0;
            }
        }
    }
    assert!(saw_fractional);
    let mut order =
        LocalAdjustment { mask_enabled: false, components: vec![leaf(constant(0), false, true), leaf(constant(128), false, false)], ..Default::default() };
    assert_eq!(group_weight(&order, [0.5; 2], [0.4; 4], [1.0; 2]), 128.0 / 255.0);
    order.components.swap(0, 1);
    assert_eq!(group_weight(&order, [0.5; 2], [0.4; 4], [1.0; 2]), 0.0);
}

#[test]
fn intersection_is_not_a_positive_leaf_and_hidden_intersection_does_not_remove_primary() {
    let mut a = LocalAdjustment { exposure: 1.0, mask_enabled: false, components: vec![leaf(constant(128), false, true)], ..Default::default() };
    assert!(a.is_identity());
    assert_eq!(group_weight(&a, [0.5; 2], [0.2; 4], [1.0; 2]), 0.0);
    a.invert = true;
    assert!(!a.is_identity());
    assert_eq!(group_weight(&a, [0.5; 2], [0.2; 4], [1.0; 2]), 1.0);
    a.invert = false;
    a.mask_enabled = true;
    a.mask = constant(192);
    a.components[0].enabled = false;
    assert_eq!(group_weight(&a, [0.5; 2], [0.2; 4], [1.0; 2]), 192.0 / 255.0);
    a.components[0].enabled = true;
    assert_eq!(group_weight(&a, [0.5; 2], [0.2; 4], [1.0; 2]), 192.0 / 255.0 * (128.0 / 255.0));
    a.components.push(leaf(constant(64), false, false));
    a.mask_enabled = false;
    assert!(!a.is_identity(), "a later add remains a positive leaf");
}

#[test]
fn old_add_sub_coverage_and_global_output_remain_bit_exact_with_false_intersections() {
    let (w, h) = (31, 23);
    let source = image(w, h);
    let aspect = [w as f32 / h as f32, 1.0];
    let mut a = LocalAdjustment {
        exposure: 0.4,
        mask: constant(128),
        components: vec![
            leaf(MaskShape::Linear { start: [0.1, 0.2], end: [0.8, 0.6] }, false, false),
            leaf(MaskShape::Radial { center: [0.51, 0.43], radius: [0.23, 0.37], feather: 0.7, rotation: 0.0 }, true, false),
        ],
        ..Default::default()
    };
    for invert in [false, true] {
        a.invert = invert;
        let prepared = PreparedLocalMask::new(&a, aspect);
        for (i, q) in source.iter().enumerate() {
            let uv = [((i % w) as f32 + 0.5) / w as f32, ((i / w) as f32 + 0.5) / h as f32];
            assert_eq!(prepared.weight(uv, *q).to_bits(), legacy_weight(&a, uv, *q, aspect).to_bits());
        }
        let mut old_json = serde_json::to_value(&a).unwrap();
        for component in old_json["components"].as_array_mut().unwrap() {
            component.as_object_mut().unwrap().remove("intersect");
            component.as_object_mut().unwrap().remove("invertPrevious");
        }
        let old_a: LocalAdjustment = serde_json::from_value(old_json).unwrap();
        for float in [false, true] {
            let mut actual = source.clone();
            let mut legacy = source.clone();
            let settings = CameraRaw { exposure: 0.3, vibrance: 17.0, local_adjustments: vec![a.clone()], ..Default::default() };
            let old_settings = CameraRaw { local_adjustments: vec![old_a.clone()], ..settings.clone() };
            crate::camera_raw::develop(&mut actual, w, h, &settings, float);
            crate::camera_raw::develop(&mut legacy, w, h, &old_settings, float);
            assert_eq!(bits(&actual), bits(&legacy));
        }
    }
}

#[test]
fn multiple_intersection_groups_match_independent_masked_candidates_and_global_pipeline() {
    let (w, h) = (29, 19);
    let source = image(w, h);
    let aspect = [w as f32 / h as f32, 1.0];
    let groups = vec![
        LocalAdjustment {
            mask: constant(192),
            vibrance: 31.0,
            point_curve_blue: vec![[0.0, 7.0], [127.0, 157.0], [255.0, 255.0]],
            components: vec![leaf(MaskShape::Linear { start: [0.0, 0.5], end: [1.0, 0.5] }, false, true)],
            ..Default::default()
        },
        LocalAdjustment {
            mask: MaskShape::Luminance { min: 0.15, max: 0.65, feather: 0.2 },
            grain_amount: 27.0,
            sharpen_amount: 51.0,
            components: vec![leaf(constant(128), false, true), leaf(constant(64), true, false)],
            ..Default::default()
        },
    ];
    for float in [false, true] {
        let mut expected = source.clone();
        for a in &groups {
            let mut candidate = expected.clone();
            crate::camera_raw::pointwise_tests_v64::frozen_develop(&mut candidate, w, h, &a.advanced_params(0.4, 129), float);
            for (i, q) in expected.iter_mut().enumerate() {
                if q[3] <= 0.0 {
                    continue;
                }
                let uv = [((i % w) as f32 + 0.5) / w as f32, ((i / w) as f32 + 0.5) / h as f32];
                let mut m = weight(&a.mask, uv, source[i], aspect);
                for c in &a.components {
                    let next = weight(&c.mask, uv, source[i], aspect);
                    m = if c.subtract {
                        m * (1.0 - next)
                    } else if c.intersect {
                        m * next
                    } else {
                        m.max(next)
                    };
                }
                if m <= 0.0 {
                    continue;
                }
                for (old, value) in q.iter_mut().take(3).zip(candidate[i]) {
                    if m >= 1.0 {
                        *old = value;
                    } else {
                        *old += (value - *old) * m;
                    }
                }
            }
        }
        let mut actual = source.clone();
        apply_scaled(&mut actual, w, h, &groups, float, 0.4, 129);
        assert_eq!(bits(&actual), bits(&expected));
        let mut global = source.clone();
        crate::camera_raw::develop(
            &mut global,
            w,
            h,
            &CameraRaw { local_adjustments: groups.clone(), pixel_scale: 0.4, seed: 129, ..Default::default() },
            float,
        );
        // A nonfloat global develop clamps its global output before local groups. The
        // direct apply API deliberately accepts an already-developed input, including HDR.
        let mut global_expected = source.clone();
        if !float {
            for q in &mut global_expected {
                for channel in q.iter_mut().take(3) {
                    *channel = channel.clamp(0.0, 1.0);
                }
            }
        }
        apply_scaled(&mut global_expected, w, h, &groups, float, 0.4, 129);
        assert_eq!(bits(&global), bits(&global_expected));
        for split in 0..=groups.len() {
            let mut split_result = source.clone();
            apply_scaled(&mut split_result, w, h, &groups[..split], float, 0.4, 129);
            apply_scaled_anchored(&mut split_result, &source, w, h, &groups[split..], float, 0.4, 129);
            assert_eq!(bits(&split_result), bits(&expected));
        }
    }
}

#[test]
fn structural_empty_helpers_keep_hidden_positive_inverted_cancelled_and_invalid_geometry() {
    let empty_brush = MaskShape::Brush { strokes: Vec::new() };
    let stroke = BrushStroke { points: vec![[0.4, 0.5]], radius: 0.2, feather: 0.5, erase: false };
    let eraser = BrushStroke { erase: true, ..stroke.clone() };
    let all_erase = MaskShape::Brush { strokes: vec![eraser.clone()] };
    let draw_then_erase = MaskShape::Brush { strokes: vec![stroke, eraser] };
    assert!(shape_is_empty(&empty_brush));
    assert!(shape_is_empty(&all_erase));
    assert!(shape_is_empty(&constant(0)));
    assert!(!shape_is_empty(&constant(1)));
    assert!(!shape_is_empty(&draw_then_erase), "avoid inferring exact cancellation");
    let mut empty = LocalAdjustment {
        enabled: false,
        mask_enabled: false,
        mask: empty_brush.clone(),
        components: vec![leaf(all_erase, false, false), leaf(constant(0), false, false), leaf(constant(255), true, false), leaf(constant(255), false, true)],
        ..Default::default()
    };
    assert!(group_is_empty(&empty), "subtract/intersect cannot add stored positive coverage");
    let mut inverted = empty.clone();
    inverted.invert = true;
    assert!(!group_is_empty(&inverted));
    let mut hidden_primary = empty.clone();
    hidden_primary.mask = constant(1);
    assert!(!group_is_empty(&hidden_primary), "hidden primary is not deleted");
    empty.components.push(MaskComponent { enabled: false, mask: constant(1), ..Default::default() });
    assert!(!group_is_empty(&empty), "hidden positive component is not deleted");
    let analytic = [
        MaskShape::Linear { start: [0.1; 2], end: [0.9; 2] },
        MaskShape::Radial { center: [0.5; 2], radius: [0.2; 2], feather: 0.5, rotation: 0.0 },
        MaskShape::Luminance { min: 0.0, max: 0.0, feather: 0.0 },
        MaskShape::Color { color: [0.0; 3], tolerance: 0.1, feather: 0.0 },
    ];
    for shape in analytic {
        assert!(!shape_is_empty(&shape));
    }
    let invalid = [
        MaskShape::Raster { width: 0, height: 1, values: vec![] },
        MaskShape::Raster { width: 2, height: 1, values: vec![0] },
        MaskShape::Brush { strokes: vec![BrushStroke { points: vec![], radius: 0.2, feather: 0.5, erase: true }] },
    ];
    for mask in invalid {
        assert!(!shape_is_empty(&mask), "invalid shape is retained");
        assert!(!group_is_empty(&LocalAdjustment { mask, ..Default::default() }));
    }
    let bad_group = LocalAdjustment { mask: empty_brush, components: vec![leaf(constant(255), true, true)], ..Default::default() };
    assert!(!group_is_empty(&bad_group));
}

fn append_intersection(a: &mut LocalAdjustment, mask: MaskShape) {
    let invert_previous = a.invert;
    a.invert = false;
    a.components.push(MaskComponent { mask, intersect: true, invert_previous, ..Default::default() });
}

#[test]
fn appending_intersection_consumes_current_inversion_before_new_leaf_with_later_union() {
    let mut a = LocalAdjustment { mask: constant(192), invert: true, exposure: 1.0, ..Default::default() };
    let p = 192.0f32 / 255.0;
    let first = 128.0f32 / 255.0;
    let second = 64.0f32 / 255.0;
    append_intersection(&mut a, constant(128));
    assert!(!a.invert && a.components[0].invert_previous);
    let after_first = (1.0 - p) * first;
    assert_eq!(group_weight(&a, [0.5; 2], [0.3; 4], [1.0; 2]).to_bits(), after_first.to_bits());
    assert_ne!(after_first.to_bits(), (1.0 - p * first).to_bits(), "complement must precede the appended intersection");
    a.invert = true;
    append_intersection(&mut a, constant(64));
    let after_second = (1.0 - after_first) * second;
    assert_eq!(group_weight(&a, [0.5; 2], [0.3; 4], [1.0; 2]).to_bits(), after_second.to_bits());
    assert_eq!(a.components.iter().filter(|c| c.invert_previous).count(), 2);
    a.components.push(leaf(constant(128), false, false));
    let after_union = after_second.max(first);
    assert_eq!(group_weight(&a, [0.5; 2], [0.3; 4], [1.0; 2]).to_bits(), after_union.to_bits());
    a.invert = true;
    let after_final_invert = 1.0 - after_union;
    assert_eq!(group_weight(&a, [0.5; 2], [0.3; 4], [1.0; 2]).to_bits(), after_final_invert.to_bits());
    append_intersection(&mut a, constant(192));
    let final_coverage = after_final_invert * p;
    assert_eq!(group_weight(&a, [0.5; 2], [0.3; 4], [1.0; 2]).to_bits(), final_coverage.to_bits());
    assert!(validate(std::slice::from_ref(&a)).is_ok());
    for float in [false, true] {
        let mut actual = [[0.2, 0.3, 0.4, 0.7]];
        let mut expected = actual;
        let prepared = PreparedAdjustment::new(&a, [1.0; 2]);
        let rgb = prepared.rgb([0.2, 0.3, 0.4]);
        for (old, value) in expected[0].iter_mut().take(3).zip(rgb) {
            let value = if float { value } else { value.clamp(0.0, 1.0) };
            *old += (value - *old) * final_coverage;
        }
        apply(&mut actual, 1, 1, std::slice::from_ref(&a), float);
        assert_eq!(bits(&actual), bits(&expected));
    }
}

#[test]
fn prior_inversion_json_and_geometry_keys_preserve_positive_coverage_and_safe_cleanup() {
    let a = LocalAdjustment {
        exposure: 1.0,
        mask_enabled: false,
        mask: MaskShape::Brush { strokes: vec![] },
        components: vec![MaskComponent { intersect: true, invert_previous: true, mask: constant(128), ..Default::default() }],
        ..Default::default()
    };
    assert!(!a.is_identity(), "inverting previous zero before intersection establishes positive coverage");
    assert!(!group_is_empty(&a));
    assert_eq!(group_weight(&a, [0.5; 2], [0.3; 4], [1.0; 2]), 128.0 / 255.0);
    let mut old_geometry = a.clone();
    old_geometry.components[0].invert_previous = false;
    assert!(old_geometry.is_identity());
    assert!(group_is_empty(&old_geometry));
    assert_ne!(a, old_geometry, "prefix/settings equality must notice operation algebra");
    assert!(a.same_color_settings(&old_geometry), "unmasked spatial candidate key excludes geometry algebra");
    let encoded = serde_json::to_value(&a).unwrap();
    assert_eq!(encoded["components"][0]["invertPrevious"], true);
    let decoded: LocalAdjustment = serde_json::from_value(encoded).unwrap();
    assert_eq!(a, decoded);
    for float in [false, true] {
        let mut first = image(3, 2);
        let mut second = first.clone();
        apply(&mut first, 3, 2, std::slice::from_ref(&a), float);
        apply(&mut second, 3, 2, std::slice::from_ref(&decoded), float);
        assert_eq!(bits(&first), bits(&second));
    }
    let mut hidden = a;
    hidden.components[0].enabled = false;
    assert!(!hidden.is_identity());
    assert_eq!(group_weight(&hidden, [0.5; 2], [0.3; 4], [1.0; 2]), 1.0);
    assert!(!group_is_empty(&hidden), "prior-inversion groups are conservatively retained even when hidden");
}

#[test]
fn hidden_leaf_preserves_prefix_inversion_and_transferred_boundary_works_on_any_operation() {
    let p = 192.0f32 / 255.0;
    let mut a = LocalAdjustment { mask: constant(192), invert: true, exposure: 1.0, ..Default::default() };
    append_intersection(&mut a, constant(128));
    a.components[0].enabled = false;
    assert_eq!(group_weight(&a, [0.5; 2], [0.3; 4], [1.0; 2]).to_bits(), (1.0 - p).to_bits(), "hiding intersection restores the inverted old coverage");
    a.components[0].intersect = false;
    for subtract in [false, true] {
        a.components[0].subtract = subtract;
        assert!(validate(std::slice::from_ref(&a)).is_ok());
        assert_eq!(group_weight(&a, [0.5; 2], [0.3; 4], [1.0; 2]).to_bits(), (1.0 - p).to_bits());
        a.components[0].enabled = true;
        let next = 128.0f32 / 255.0;
        let expected = if subtract { (1.0 - p) * (1.0 - next) } else { (1.0 - p).max(next) };
        assert_eq!(group_weight(&a, [0.5; 2], [0.3; 4], [1.0; 2]).to_bits(), expected.to_bits());
        a.components[0].enabled = false;
    }
    a.mask_enabled = false;
    assert!(!a.is_identity(), "a hidden unary boundary still turns empty prefix into full coverage");
    assert_eq!(group_weight(&a, [0.5; 2], [0.3; 4], [1.0; 2]), 1.0);
    let mut pixels = [[0.2, 0.3, 0.4, 0.7]];
    let expected = [[0.4, 0.6, 0.8, 0.7]];
    apply(&mut pixels, 1, 1, std::slice::from_ref(&a), true);
    assert_eq!(bits(&pixels), bits(&expected));
    a.components.push(MaskComponent { enabled: false, invert_previous: true, ..Default::default() });
    assert_eq!(group_weight(&a, [0.5; 2], [0.3; 4], [1.0; 2]), 0.0, "two retained unary boundaries cancel even when both leaves are hidden");
}
