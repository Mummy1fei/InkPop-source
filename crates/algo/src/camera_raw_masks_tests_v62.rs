//! Compatibility and geometry witnesses for v62 local mask groups. No PS oracle claim.
use super::*;
use serde_json::json;

fn constant(value: u8) -> MaskShape {
    MaskShape::Raster { width: 1, height: 1, values: vec![value] }
}
fn component(mask: MaskShape, subtract: bool) -> MaskComponent {
    MaskComponent { name: "Shape".into(), mask, subtract, ..Default::default() }
}
fn radial(rotation: f32) -> MaskShape {
    MaskShape::Radial { center: [0.5, 0.5], radius: [0.18, 0.05], feather: 0.4, rotation }
}
fn assert_bits(actual: &[[f32; 4]], expected: &[[f32; 4]]) {
    assert_eq!(actual.len(), expected.len());
    for (i, (a, b)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits), "pixel={i}");
    }
}

#[test]
fn old_local_mask_json_has_identity_defaults_and_roundtrips() {
    let old = json!({"name":"Old radial","exposure":0.5,"mask":{"kind":"radial","center":[0.5,0.5],"radius":[0.2,0.1],"feather":0.5}});
    let a: LocalAdjustment = serde_json::from_value(old).unwrap();
    assert!(a.mask_enabled);
    assert!(a.components.is_empty());
    assert_eq!([a.highlights, a.shadows, a.whites, a.blacks, a.hue], [0.0; 5]);
    assert!(matches!(a.mask, MaskShape::Radial { rotation: 0.0, .. }));
    assert!(validate(std::slice::from_ref(&a)).is_ok());
    let decoded: LocalAdjustment = serde_json::from_value(serde_json::to_value(&a).unwrap()).unwrap();
    assert_eq!(decoded, a);
    // Shapes are leaves, so a hostile recursive group cannot deserialize.
    assert!(serde_json::from_value::<MaskShape>(json!({"kind":"group","components":[]})).is_err());
}

#[test]
fn rotated_radial_uses_physical_aspect_clockwise_and_preserves_zero_angle() {
    for aspect in [[1.0, 1.0], [2.0, 1.0], [1.0, 3.0]] {
        for rotation in [-135.0f32, -90.0, -35.0, 0.0, 45.0, 90.0, 180.0] {
            let shape = radial(rotation);
            let group = LocalAdjustment { mask: shape.clone(), ..Default::default() };
            let prepared = PreparedLocalMask::new(&group, aspect);
            let (sin, cos) = rotation.to_radians().sin_cos();
            for (t, expected) in [(0.0, 1.0), (0.4, 1.0), (0.8, 0.5), (1.2, 0.0)] {
                // Independent forward rotation of a point on the ellipse's long physical axis.
                let uv = [0.5 + 0.18 * t * cos, 0.5 + 0.18 * aspect[0] * t * sin / aspect[1]];
                assert!((weight(&shape, uv, [0.0; 4], aspect) - expected).abs() < 2e-6, "{aspect:?} {rotation} {t}");
                assert!((prepared.weight(uv, [0.0; 4]) - expected).abs() < 2e-6);
            }
        }
        let zero = radial(0.0);
        for y in 0..31 {
            for x in 0..41 {
                let uv = [x as f32 / 40.0, y as f32 / 30.0];
                let d = ((uv[0] - 0.5) / 0.18).hypot((uv[1] - 0.5) / 0.05);
                assert_eq!(weight(&zero, uv, [0.0; 4], aspect).to_bits(), falloff(d, 1.0, 0.4).to_bits());
            }
        }
    }
    let ninety = radial(90.0);
    assert!(weight(&ninety, [0.5, 0.7], [0.0; 4], [2.0, 1.0]) > 0.99, "positive rotation points the long axis downward");
    assert_eq!(weight(&ninety, [0.6, 0.5], [0.0; 4], [2.0, 1.0]), 0.0);
}

#[test]
fn components_add_subtract_in_order_and_invert_after_composition() {
    let a = LocalAdjustment {
        mask: constant(128),
        components: vec![component(constant(128), true), component(constant(192), false), component(constant(255), true)],
        ..Default::default()
    };
    let mut group = a.clone();
    group.components.last_mut().unwrap().enabled = false;
    let expected = (128.0f32 / 255.0 * (1.0 - 128.0 / 255.0)).max(192.0 / 255.0);
    for group_enabled in [true, false] {
        group.enabled = group_enabled;
        assert_eq!(group_weight(&group, [0.5; 2], [0.2; 4], [2.0, 1.0]), expected, "group visibility must not erase editable geometry");
    }
    group.invert = true;
    assert_eq!(group_weight(&group, [0.5; 2], [0.2; 4], [2.0, 1.0]), 1.0 - expected);
    let mut reordered = group.clone();
    reordered.invert = false;
    reordered.components.swap(0, 1);
    assert!(group_weight(&reordered, [0.5; 2], [0.2; 4], [1.0; 2]) < expected);
    assert_eq!(group_weight(&a, [0.5; 2], [0.2; 4], [1.0; 2]), 0.0, "enabled subtract-all removes prior coverage");
}

#[test]
fn disabled_primary_empty_groups_and_inversion_have_correct_identity() {
    let mut a = LocalAdjustment { exposure: 1.0, mask_enabled: false, components: vec![component(constant(128), true)], ..Default::default() };
    assert!(a.is_identity());
    assert_eq!(group_weight(&a, [0.5; 2], [0.0; 4], [1.0; 2]), 0.0);
    a.invert = true;
    assert!(!a.is_identity());
    assert_eq!(group_weight(&a, [0.5; 2], [0.0; 4], [1.0; 2]), 1.0);
    let mut p = [[0.2, 0.3, 0.4, 0.5]];
    apply(&mut p, 1, 1, &[a.clone()], true);
    assert_eq!(p[0], [0.4, 0.6, 0.8, 0.5]);
    a.enabled = false;
    assert!(a.is_identity());
    a.enabled = true;
    a.invert = false;
    a.components.push(component(constant(128), false));
    assert!(!a.is_identity());
    assert_eq!(group_weight(&a, [0.5; 2], [0.0; 4], [1.0; 2]), 128.0 / 255.0);
}

#[test]
fn group_limits_validate_disabled_geometry_and_aggregate_work() {
    let a = LocalAdjustment { mask: radial(0.0), ..Default::default() };
    for rotation in [f32::NAN, f32::INFINITY, 180.01, -180.01] {
        let mut bad = a.clone();
        if let MaskShape::Radial { rotation: r, .. } = &mut bad.mask {
            *r = rotation;
        }
        assert!(validate(&[bad]).is_err());
    }
    for field in ["highlights", "shadows", "whites", "blacks", "hue"] {
        let mut json = serde_json::to_value(&a).unwrap();
        json[field] = json!(if field == "hue" { 180.01 } else { 100.01 });
        assert!(validate(&[serde_json::from_value(json).unwrap()]).is_err());
    }
    for parameter in 0..5 {
        let mut bad = a.clone();
        *match parameter {
            0 => &mut bad.highlights,
            1 => &mut bad.shadows,
            2 => &mut bad.whites,
            3 => &mut bad.blacks,
            _ => &mut bad.hue,
        } = f32::NAN;
        assert!(validate(&[bad]).is_err());
    }
    let mut bad = a.clone();
    bad.components = vec![component(radial(0.0), false); MAX_COMPONENTS + 1];
    assert!(validate(&[bad]).is_err());
    let mut group = a.clone();
    group.components = vec![component(radial(0.0), false); MAX_COMPONENTS];
    let mut groups = vec![group; 7];
    let mut last = a.clone();
    last.components = vec![component(radial(0.0), false); 24];
    groups.push(last);
    assert!(validate(&groups).is_ok(), "exactly 256 total shapes");
    groups.last_mut().unwrap().components.push(component(radial(0.0), false));
    assert!(validate(&groups).is_err());
    let mut bad = a;
    bad.mask_enabled = false;
    bad.components = vec![MaskComponent { name: "\n".into(), enabled: false, ..Default::default() }];
    assert!(validate(&[bad]).is_err());
}

#[test]
fn brush_points_and_raster_pixels_share_one_budget_across_group_components() {
    let stroke = |n| MaskShape::Brush { strokes: vec![BrushStroke { points: vec![[0.5; 2]; n], radius: 0.1, feather: 0.5, erase: false }] };
    let mut a = LocalAdjustment { mask: stroke(2048), components: vec![component(stroke(2048), false)], ..Default::default() };
    assert!(validate(std::slice::from_ref(&a)).is_ok());
    a.components.push(component(stroke(1), true));
    assert!(validate(&[a]).is_err());
    let side = MAX_RASTER_SIDE;
    let raster = MaskShape::Raster { width: side, height: side, values: vec![0; side as usize * side as usize] };
    let slots = MAX_RASTER_PIXELS / (side as usize * side as usize);
    let mut a = LocalAdjustment { mask: raster.clone(), components: vec![component(raster.clone(), false); slots - 1], ..Default::default() };
    assert!(validate(std::slice::from_ref(&a)).is_ok());
    a.components.push(component(raster, true));
    assert!(validate(&[a]).is_err());
}

#[test]
fn new_local_tone_and_hue_controls_affect_their_ranges_and_preserve_alpha() {
    let evaluate = |a: LocalAdjustment, rgb: [f32; 3]| {
        let mut p = [[rgb[0], rgb[1], rgb[2], 0.37]];
        apply(&mut p, 1, 1, &[LocalAdjustment { mask: constant(255), ..a }], true);
        assert_eq!(p[0][3].to_bits(), 0.37f32.to_bits());
        p[0]
    };
    assert!(evaluate(LocalAdjustment { highlights: -100.0, ..Default::default() }, [0.9; 3])[0] < 0.9);
    assert!(evaluate(LocalAdjustment { shadows: 100.0, ..Default::default() }, [0.05; 3])[0] > 0.05);
    assert!(evaluate(LocalAdjustment { whites: 100.0, ..Default::default() }, [0.8; 3])[0] > 0.8);
    assert!(evaluate(LocalAdjustment { blacks: 100.0, ..Default::default() }, [0.05; 3])[0] > 0.05);
    let hue = evaluate(LocalAdjustment { hue: 120.0, ..Default::default() }, [0.8, 0.1, 0.1]);
    assert!((hue[0] - 0.1).abs() < 1e-6 && (hue[1] - 0.8).abs() < 1e-6 && (hue[2] - 0.1).abs() < 1e-6);
    let hdr = evaluate(LocalAdjustment { exposure: 1.0, hue: 120.0, ..Default::default() }, [-0.2, 2.0, 0.4]);
    assert!(hdr.iter().take(3).any(|v| *v > 1.0));
    assert!(hdr.iter().take(3).any(|v| *v < 0.0));
}

#[test]
fn range_components_use_the_global_source_instead_of_earlier_local_colour_edits() {
    let first = LocalAdjustment { mask: constant(255), hue: 120.0, ..Default::default() };
    let second = LocalAdjustment { mask: MaskShape::Color { color: [0.1, 0.8, 0.1], tolerance: 0.02, feather: 0.5 }, exposure: 1.0, ..Default::default() };
    let mut only_first = [[0.8, 0.1, 0.1, 0.5]];
    let mut both = only_first;
    apply(&mut only_first, 1, 1, std::slice::from_ref(&first), true);
    apply(&mut both, 1, 1, &[first, second], true);
    assert_bits(&both, &only_first);
}

// Frozen v61 scalar tone/coverage model guards the compatibility of gain hoisting and row
// scheduling. It is deliberately the old arithmetic, not a Photoshop correctness oracle.
fn legacy_scalar(px: &mut [[f32; 4]], w: usize, h: usize, items: &[LocalAdjustment], float: bool) {
    if w == 0 || h == 0 {
        return;
    }
    let aspect = [w as f32 / w.min(h) as f32, h as f32 / w.min(h) as f32];
    for (i, c) in px.iter_mut().take(w.saturating_mul(h)).enumerate() {
        if c[3] <= 0.0 {
            continue;
        }
        let source = *c;
        let uv = [((i % w) as f32 + 0.5) / w as f32, ((i / w) as f32 + 0.5) / h as f32];
        for a in items.iter().filter(|a| !a.is_identity()) {
            let mut m = weight(&a.mask, uv, source, aspect);
            if a.invert {
                m = 1.0 - m;
            }
            if m <= 0.0 {
                continue;
            }
            let gain = 2f32.powf(a.exposure);
            let mut rgb = [c[0] * gain * (1.0 + a.temperature * 0.002), c[1] * gain * (1.0 - a.tint * 0.002), c[2] * gain * (1.0 - a.temperature * 0.002)];
            for v in &mut rgb {
                *v = (*v - 0.18) * (1.0 + a.contrast / 100.0) + 0.18;
            }
            let luma = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
            for ch in 0..3 {
                let v = luma + (rgb[ch] - luma) * (1.0 + a.saturation / 100.0);
                let v = if float { v } else { v.clamp(0.0, 1.0) };
                c[ch] += (v - c[ch]) * m;
            }
        }
    }
}

#[test]
fn legacy_zero_rotation_without_components_is_bitwise_compatible_after_row_preparation() {
    let shapes = vec![
        MaskShape::Linear { start: [0.15, 0.1], end: [0.8, 0.7] },
        radial(0.0),
        MaskShape::Color { color: [0.3, 0.5, 0.1], tolerance: 0.7, feather: 0.8 },
        MaskShape::Luminance { min: 0.1, max: 0.7, feather: 0.25 },
        MaskShape::Raster { width: 3, height: 2, values: vec![0, 64, 128, 255, 191, 31] },
        MaskShape::Brush { strokes: vec![BrushStroke { points: vec![[0.1, 0.3], [0.7, 0.8]], radius: 0.12, feather: 0.6, erase: false }] },
    ];
    let groups: Vec<_> = shapes
        .into_iter()
        .enumerate()
        .map(|(i, mask)| LocalAdjustment {
            mask,
            exposure: (i as f32 - 2.3) * 0.3,
            contrast: i as f32 * 7.0 - 12.0,
            temperature: i as f32 * 3.0,
            tint: -17.0,
            saturation: 25.0,
            invert: i == 2,
            ..Default::default()
        })
        .collect();
    for (w, h, count) in [(1, 1, 1), (13, 7, 91), (97, 65, 6305), (7, 3, 11), (usize::MAX, usize::MAX, 2)] {
        let source: Vec<_> = (0..count).map(|i| [(i % 23) as f32 / 17.0 - 0.2, (i % 13) as f32 / 11.0, (i % 7) as f32 / 3.0, (i % 5) as f32 / 4.0]).collect();
        for float in [false, true] {
            let mut expected = source.clone();
            let mut actual = source.clone();
            legacy_scalar(&mut expected, w, h, &groups, float);
            apply(&mut actual, w, h, &groups, float);
            assert_bits(&actual, &expected);
        }
    }
}

#[test]
fn local_groups_preserve_alpha_through_u8_u16_f32_surface_roundtrips() {
    use photocraft_color::{PixelFormat, SampleType};
    use photocraft_geom::Rect;
    use photocraft_raster::Surface;
    let bounds = Rect::new(0, 0, 4, 1);
    for format in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let source = [[0.1, 0.3, 0.8, 0.0], [0.7, 0.2, 0.1, 0.25], [0.02, 0.04, 0.03, 0.5], [1.6, -0.2, 0.7, 1.0]];
        let mut surface = Surface::new(format);
        surface.write_region(bounds, &source.into_iter().flatten().collect::<Vec<_>>());
        let mut pixels = vec![[0.0; 4]; 4];
        surface.read_rgba_into(bounds, &mut pixels);
        let before = pixels.clone();
        let group = LocalAdjustment { mask: radial(45.0), invert: true, shadows: 35.0, whites: 10.0, hue: 25.0, ..Default::default() };
        apply(&mut pixels, 4, 1, &[group], format.sample == SampleType::F32);
        assert_eq!(pixels[0], before[0], "transparent hidden colour must stay untouched");
        assert!(pixels.iter().zip(&before).all(|(a, b)| a[3].to_bits() == b[3].to_bits()));
        surface.write_region(bounds, &pixels.into_iter().flatten().collect::<Vec<_>>());
        for (x, b) in before.iter().enumerate() {
            assert_eq!(surface.rgba(x as i32, 0)[3].to_bits(), b[3].to_bits());
        }
    }
}

#[test]
fn invalid_groups_and_invalid_evaluator_inputs_cannot_modify_pixels() {
    let source = [[0.3, 0.5, 0.7, 0.4]];
    let invalid = LocalAdjustment { highlights: f32::NAN, exposure: 1.0, invert: true, ..Default::default() };
    let mut actual = source;
    apply(&mut actual, 1, 1, std::slice::from_ref(&invalid), true);
    assert_bits(&actual, &source);
    assert_eq!(group_weight(&invalid, [0.5; 2], [0.2; 4], [1.0; 2]), 0.0);
    let valid = LocalAdjustment { invert: true, ..Default::default() };
    assert_eq!(group_weight(&valid, [f32::NAN, 0.5], [0.2; 4], [1.0; 2]), 0.0);
    assert_eq!(group_weight(&valid, [0.5; 2], [0.2; 4], [f32::INFINITY, 1.0]), 0.0);
    assert_eq!(group_weight(&valid, [0.5; 2], [0.2; 4], [0.0, 1.0]), 0.0);
}
