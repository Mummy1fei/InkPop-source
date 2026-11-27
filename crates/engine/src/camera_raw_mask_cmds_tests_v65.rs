use super::*;
use photocraft_algo::camera_raw_masks::BrushStroke;
use serde_json::json;

fn group(name: &str) -> LocalAdjustment {
    LocalAdjustment {
        name: name.into(),
        exposure: 0.7,
        mask: MaskShape::Radial { center: [0.4, 0.6], radius: [0.2, 0.3], feather: 0.4, rotation: 27.0 },
        components: vec![MaskComponent { name: "addition".into(), ..Default::default() }],
        ..Default::default()
    }
}
fn empty(name: &str) -> LocalAdjustment {
    LocalAdjustment { name: name.into(), exposure: 0.5, mask: MaskShape::Brush { strokes: vec![] }, ..Default::default() }
}
fn unchanged_on_error(items: &[LocalAdjustment], operation: MaskOperation) {
    let before = items.to_vec();
    assert!(apply(items, &operation).is_err(), "{operation:?}");
    assert_eq!(items, before);
}

#[test]
fn duplicate_copies_all_content_inserts_after_source_and_maps_original_identity() {
    let items = vec![group("A"), group("B"), group("C")];
    for invert in [false, true] {
        let result = apply(&items, &MaskOperation::Duplicate { index: 1, invert }).unwrap();
        assert_eq!(result.old_to_new, vec![Some(0), Some(1), Some(3)]);
        assert_eq!(result.preferred_selection, Some(2));
        assert_eq!(result.items[1], items[1]);
        let mut expected = items[1].clone();
        expected.name = result.items[2].name.clone();
        expected.invert ^= invert;
        assert_eq!(result.items[2], expected);
        assert_ne!(result.items[2].name, items[1].name);
        let mut independent = result.items;
        independent[2].components[0].name = "changed clone".into();
        assert_eq!(independent[1], items[1]);
    }
}

#[test]
fn duplicate_names_preserve_utf8_byte_limit_and_avoid_collisions() {
    let mut source = group(&"蒙".repeat(85));
    source.name.push('a');
    assert_eq!(source.name.len(), 256);
    let first = apply(&[source], &MaskOperation::Duplicate { index: 0, invert: false }).unwrap();
    let second = apply(&first.items, &MaskOperation::Duplicate { index: 0, invert: false }).unwrap();
    assert!(second.items.iter().all(|item| item.name.len() <= 256 && !item.name.trim().is_empty()));
    assert_ne!(second.items[1].name, second.items[2].name);
    assert!(second.items[1].name.ends_with("副本 2"));
    masks::validate(&second.items).unwrap();
}

#[test]
fn reset_restores_all_controls_but_retains_name_hidden_inversion_and_full_geometry() {
    let mut original = group("保留蒙版");
    original.enabled = false;
    original.invert = true;
    original.mask_enabled = false;
    original.vibrance = 35.0;
    original.texture = 29.0;
    original.grain_amount = 21.0;
    original.hsl_sat[3] = 17.0;
    original.point_curve_blue = vec![[0.0, 9.0], [255.0, 255.0]];
    original.grade_midtones.sat = 13.0;
    original.noise_color = 23.0;
    original.sharpen_radius = 2.1;
    original.components[0].enabled = false;
    original.components[0].intersect = true;
    let expected = LocalAdjustment {
        name: original.name.clone(),
        enabled: original.enabled,
        invert: original.invert,
        mask: original.mask.clone(),
        mask_enabled: original.mask_enabled,
        components: original.components.clone(),
        ..Default::default()
    };
    let result = apply(&[original], &MaskOperation::ResetAdjustments { index: 0 }).unwrap();
    assert_eq!(result.items, vec![expected]);
    assert_eq!(result.old_to_new, vec![Some(0)]);
    assert_eq!(result.preferred_selection, Some(0));
}

#[test]
fn hidden_or_zero_adjustment_content_is_not_deleted_as_empty_and_bulk_mapping_is_exact() {
    let mut hidden = group("hidden");
    hidden.enabled = false;
    hidden.mask_enabled = false;
    hidden.components[0].enabled = false;
    let mut neutral = group("neutral");
    neutral.exposure = 0.0;
    let items = vec![hidden, empty("empty1"), neutral, empty("empty2")];
    assert!(items[0].is_identity() && items[2].is_identity());
    let result = apply(&items, &MaskOperation::DeleteAllEmpty).unwrap();
    assert_eq!(result.items, vec![items[0].clone(), items[2].clone()]);
    assert_eq!(result.old_to_new, vec![Some(0), None, Some(1), None]);
    assert_eq!(result.preferred_selection, None);
    let keep = apply(&items, &MaskOperation::DeleteCurrentIfEmpty { index: 0 }).unwrap();
    assert_eq!(keep.items, items);
    assert_eq!(keep.preferred_selection, Some(0));
    let delete = apply(&items, &MaskOperation::DeleteCurrentIfEmpty { index: 1 }).unwrap();
    assert_eq!(delete.old_to_new, vec![Some(0), None, Some(1), Some(2)]);
    let all = apply(&items, &MaskOperation::DeleteAll).unwrap();
    assert!(all.items.is_empty());
    assert_eq!(all.old_to_new, vec![None; 4]);
}

#[test]
fn row_actions_touch_only_the_explicit_group_or_component() {
    let original = vec![group("A"), group("B")];
    let named = apply(&original, &MaskOperation::Rename { index: 1, name: "新名称".into() }).unwrap();
    assert_eq!(named.items[0], original[0]);
    assert_eq!(named.items[1].name, "新名称");
    let inverted = apply(&original, &MaskOperation::Invert { index: 1 }).unwrap();
    assert!(inverted.items[1].invert);
    assert_eq!(inverted.items[1].components, original[1].components);
    let hidden = apply(&original, &MaskOperation::SetEnabled { index: 1, enabled: false }).unwrap();
    assert!(!hidden.items[1].enabled);
    assert_eq!(hidden.items[1].mask, original[1].mask);
    let primary = apply(&original, &MaskOperation::SetPrimaryEnabled { index: 1, enabled: false }).unwrap();
    assert!(!primary.items[1].mask_enabled && primary.items[1].enabled);
    assert_eq!(primary.items[1].mask, original[1].mask);
    assert_eq!(primary.items[1].components, original[1].components);
    assert_eq!(primary.items[0], original[0]);
    let component = apply(&original, &MaskOperation::RenameComponent { index: 1, component: 0, name: "组件新名".into() }).unwrap();
    assert_eq!(component.items[1].name, original[1].name);
    assert_eq!(component.items[1].components[0].name, "组件新名");
    let hidden = apply(&original, &MaskOperation::SetComponentEnabled { index: 1, component: 0, enabled: false }).unwrap();
    assert!(!hidden.items[1].components[0].enabled && hidden.items[1].enabled);
    let component = apply(&original, &MaskOperation::DeleteComponent { index: 1, component: 0 }).unwrap();
    assert!(component.items[1].components.is_empty());
    assert_eq!(component.items[1].mask, original[1].mask);
    let deleted = apply(&original, &MaskOperation::Delete { index: 0 }).unwrap();
    assert_eq!(deleted.items, vec![original[1].clone()]);
    assert_eq!(deleted.old_to_new, vec![None, Some(0)]);
}

#[test]
fn intersect_retains_existing_geometry_and_adjustments_and_rejects_invalid_shape() {
    let items = vec![group("original")];
    let shape = MaskShape::Luminance { min: 0.2, max: 0.8, feather: 0.1 };
    let result = apply(&items, &MaskOperation::Intersect { index: 0, shape: shape.clone() }).unwrap();
    assert_eq!(result.items[0].mask, items[0].mask);
    assert!(result.items[0].same_color_settings(&items[0]));
    assert_eq!(result.items[0].components[0], items[0].components[0]);
    let extra = &result.items[0].components[1];
    assert!(extra.enabled && extra.intersect && !extra.subtract);
    assert!(!extra.invert_previous);
    assert_eq!(extra.mask, shape);
    unchanged_on_error(
        &items,
        MaskOperation::Intersect { index: 0, shape: MaskShape::Radial { center: [0.5; 2], radius: [f32::NAN, 0.2], feather: 0.5, rotation: 0.0 } },
    );
}

#[test]
fn intersect_transfers_existing_inversion_and_preserves_repeated_operations_and_later_union() {
    let mut original = group("inverted");
    original.components.clear();
    original.invert = true;
    let intersect = MaskShape::Linear { start: [0.1, 0.3], end: [0.9, 0.6] };
    let once = apply(&[original.clone()], &MaskOperation::Intersect { index: 0, shape: intersect.clone() }).unwrap();
    assert!(!once.items[0].invert && once.items[0].components[0].invert_previous);
    let flipped = apply(&once.items, &MaskOperation::Invert { index: 0 }).unwrap();
    let second_shape = MaskShape::Radial { center: [0.6, 0.4], radius: [0.4, 0.35], feather: 0.5, rotation: 18.0 };
    let mut twice = apply(&flipped.items, &MaskOperation::Intersect { index: 0, shape: second_shape.clone() }).unwrap();
    assert!(!twice.items[0].invert && twice.items[0].components[1].invert_previous);
    let union = MaskShape::Linear { start: [0.7, 0.2], end: [0.9, 0.8] };
    twice.items[0].components.push(MaskComponent { name: "later union".into(), mask: union.clone(), ..Default::default() });
    masks::validate(&twice.items).unwrap();
    for y in 0..19 {
        for x in 0..23 {
            let uv = [(x as f32 + 0.5) / 23.0, (y as f32 + 0.5) / 19.0];
            let rgba = [0.3, 0.4, 0.5, 1.0];
            let old = masks::group_weight(&original, uv, rgba, [1.0; 2]);
            let leaf = masks::weight(&intersect, uv, rgba, [1.0; 2]);
            let expected_once = old * leaf;
            assert!((masks::group_weight(&once.items[0], uv, rgba, [1.0; 2]) - expected_once).abs() < 1e-6);
            let expected_twice = (1.0 - expected_once) * masks::weight(&second_shape, uv, rgba, [1.0; 2]);
            let expected_union = expected_twice.max(masks::weight(&union, uv, rgba, [1.0; 2]));
            assert!((masks::group_weight(&twice.items[0], uv, rgba, [1.0; 2]) - expected_union).abs() < 1e-6);
        }
    }
}

#[test]
fn hiding_or_deleting_intersection_preserves_prior_inversion_boundary() {
    let mut original = group("inverted");
    original.components.clear();
    original.invert = true;
    let intersect = MaskShape::Linear { start: [0.1, 0.3], end: [0.9, 0.6] };
    let once = apply(&[original.clone()], &MaskOperation::Intersect { index: 0, shape: intersect }).unwrap();
    let hidden = apply(&once.items, &MaskOperation::SetComponentEnabled { index: 0, component: 0, enabled: false }).unwrap();
    let deleted = apply(&once.items, &MaskOperation::DeleteComponent { index: 0, component: 0 }).unwrap();
    assert!(deleted.items[0].invert && deleted.items[0].components.is_empty());
    let unhidden = apply(&hidden.items, &MaskOperation::SetComponentEnabled { index: 0, component: 0, enabled: true }).unwrap();
    assert_eq!(unhidden.items, once.items);
    for y in 0..17 {
        for x in 0..19 {
            let uv = [(x as f32 + 0.5) / 19.0, (y as f32 + 0.5) / 17.0];
            let rgba = [0.3, 0.4, 0.5, 1.0];
            let expected = masks::group_weight(&original, uv, rgba, [1.0; 2]);
            assert_eq!(masks::group_weight(&hidden.items[0], uv, rgba, [1.0; 2]), expected);
            assert_eq!(masks::group_weight(&deleted.items[0], uv, rgba, [1.0; 2]), expected);
        }
    }
}

#[test]
fn deleting_prefix_inversion_propagates_to_union_or_cancels_next_inversion() {
    let mut original = group("inverted");
    original.components.clear();
    original.invert = true;
    let once = apply(&[original.clone()], &MaskOperation::Intersect { index: 0, shape: MaskShape::Luminance { min: 0.2, max: 0.8, feather: 0.1 } }).unwrap();
    let union = MaskComponent { name: "union".into(), mask: MaskShape::Linear { start: [0.6, 0.2], end: [0.8, 0.9] }, ..Default::default() };
    let mut with_union = once.items.clone();
    with_union[0].components.push(union.clone());
    let propagated = apply(&with_union, &MaskOperation::DeleteComponent { index: 0, component: 0 }).unwrap();
    assert!(propagated.items[0].components[0].invert_previous);
    assert!(!propagated.items[0].components[0].intersect && !propagated.items[0].components[0].subtract);
    let mut cancel = with_union.clone();
    cancel[0].components[1].invert_previous = true;
    let cancelled = apply(&cancel, &MaskOperation::DeleteComponent { index: 0, component: 0 }).unwrap();
    assert!(!cancelled.items[0].components[0].invert_previous);
    for y in 0..17 {
        for x in 0..19 {
            let uv = [(x as f32 + 0.5) / 19.0, (y as f32 + 0.5) / 17.0];
            let rgba = [0.3, 0.4, 0.5, 1.0];
            let old = masks::group_weight(&original, uv, rgba, [1.0; 2]);
            let add = masks::weight(&union.mask, uv, rgba, [1.0; 2]);
            assert_eq!(masks::group_weight(&propagated.items[0], uv, rgba, [1.0; 2]), old.max(add));
            assert!((masks::group_weight(&cancelled.items[0], uv, rgba, [1.0; 2]) - (1.0 - old).max(add)).abs() < 1e-6);
        }
    }
    let delete_last = apply(&propagated.items, &MaskOperation::DeleteComponent { index: 0, component: 0 }).unwrap();
    assert_eq!(delete_last.items, vec![original]);
}

#[test]
fn entire_list_group_shape_point_and_raster_limits_are_atomic() {
    let groups = vec![LocalAdjustment::default(); masks::MAX_MASKS];
    unchanged_on_error(&groups, MaskOperation::Duplicate { index: 0, invert: true });
    let heavy_shape = LocalAdjustment { components: vec![MaskComponent::default(); masks::MAX_COMPONENTS], ..Default::default() };
    let shapes = vec![heavy_shape.clone(); 7]; // 231 shapes, but duplicating a 33-shape group exceeds 256.
    unchanged_on_error(&shapes, MaskOperation::Duplicate { index: 0, invert: false });
    unchanged_on_error(&[heavy_shape], MaskOperation::Intersect { index: 0, shape: LocalAdjustment::default().mask });
    let points = LocalAdjustment {
        mask: MaskShape::Brush { strokes: vec![BrushStroke { points: vec![[0.5; 2]; 3000], radius: 0.1, feather: 0.5, erase: false }] },
        ..Default::default()
    };
    unchanged_on_error(&[points], MaskOperation::Duplicate { index: 0, invert: false });
    let side = masks::MAX_RASTER_SIDE;
    let raster =
        LocalAdjustment { mask: MaskShape::Raster { width: side, height: side, values: vec![127; side as usize * side as usize] }, ..Default::default() };
    let count = masks::MAX_RASTER_PIXELS / (side as usize * side as usize);
    unchanged_on_error(&vec![raster; count], MaskOperation::Duplicate { index: 0, invert: false });
}

#[test]
fn invalid_indices_names_and_command_parameters_are_errors_without_session_side_effects() {
    let items = vec![group("A")];
    for operation in [
        MaskOperation::Delete { index: usize::MAX },
        MaskOperation::Invert { index: 1 },
        MaskOperation::SetPrimaryEnabled { index: usize::MAX, enabled: false },
        MaskOperation::Rename { index: 0, name: " ".into() },
        MaskOperation::Rename { index: 0, name: "bad\nname".into() },
        MaskOperation::RenameComponent { index: 0, component: 7, name: "x".into() },
        MaskOperation::DeleteComponent { index: 3, component: 0 },
        MaskOperation::DeleteCurrentIfEmpty { index: 9 },
    ] {
        unchanged_on_error(&items, operation);
    }
    let mut session = Session::new();
    assert!(session.active().is_none());
    let output = run(&mut session, &json!({"items":items,"operation":{"kind":"duplicate","index":0,"invert":true}})).unwrap();
    let parsed: EditResult = serde_json::from_value(output.clone()).unwrap();
    assert_eq!(parsed.preferred_selection, Some(1));
    assert!(output.get("oldToNew").is_some() && output.get("preferredSelection").is_some());
    assert!(session.active().is_none());
    for invalid in [
        Value::Null,
        json!({}),
        json!({"items":[],"operation":{"kind":"delete","index":-1}}),
        json!({"items":[],"operation":{"kind":"updateAI"}}),
        json!({"items":[],"operation":{"kind":"deleteAll","extra":true}}),
        json!({"items":[],"operation":{"kind":"deleteAll"},"extra":true}),
    ] {
        assert!(run(&mut session, &invalid).is_err(), "{invalid}");
    }
    let command = specs().pop().unwrap();
    assert_eq!(command.id, EDIT);
    assert!(!command.journal && (command.enabled)(&session).is_ok());
}

#[test]
fn canonical_parser_rejects_unknown_keys_in_every_unit_and_field_variant() {
    let shape = LocalAdjustment::default().mask;
    let operations = [
        MaskOperation::DeleteAllEmpty,
        MaskOperation::DeleteAll,
        MaskOperation::Rename { index: 0, name: "valid".into() },
        MaskOperation::Invert { index: 0 },
        MaskOperation::Duplicate { index: 0, invert: false },
        MaskOperation::Intersect { index: 0, shape },
        MaskOperation::SetEnabled { index: 0, enabled: false },
        MaskOperation::SetPrimaryEnabled { index: 0, enabled: true },
        MaskOperation::Delete { index: 0 },
        MaskOperation::ResetAdjustments { index: 0 },
        MaskOperation::DeleteCurrentIfEmpty { index: 0 },
        MaskOperation::SetComponentEnabled { index: 0, component: 0, enabled: true },
        MaskOperation::RenameComponent { index: 0, component: 0, name: "valid".into() },
        MaskOperation::DeleteComponent { index: 0, component: 0 },
    ];
    let mut session = Session::new();
    for operation in operations {
        let canonical = serde_json::to_value(&operation).unwrap();
        assert_eq!(parse_operation(canonical.clone()).unwrap(), operation);
        for key in ["extra", "updateAI"] {
            let mut invalid = canonical.clone();
            invalid[key] = json!(true);
            assert!(parse_operation(invalid.clone()).is_err(), "{invalid}");
            assert!(run(&mut session, &json!({"items":[],"operation":invalid})).is_err());
        }
    }
    for invalid in [Value::Null, json!([]), json!("deleteAll"), json!({"kind":"deleteAll","index":0}), json!({"kind":"deleteAllEmpty","enabled":false})] {
        assert!(parse_operation(invalid.clone()).is_err(), "{invalid}");
    }
}
