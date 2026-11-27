use super::*;
use photocraft_algo::camera_raw_masks::{LocalAdjustment, MaskComponent, MaskShape};

fn recipe(name: &str, pixels: usize) -> RawPreset {
    let mut params = CameraRaw::default();
    if pixels != 0 {
        params
            .local_adjustments
            .push(LocalAdjustment { mask: MaskShape::Raster { width: pixels as u32, height: 1, values: vec![127; pixels] }, ..Default::default() });
    }
    RawPreset { name: name.into(), params }
}

#[test]
fn collection_count_is_checked_before_recipe_decoding_and_budget_counts_all_leaves() {
    assert!(collection_values(json!(vec![Value::Null; MAX_PRESETS + 1])).is_none());
    assert_eq!(collection_values(json!(vec![Value::Null; MAX_PRESETS])).unwrap().len(), MAX_PRESETS);
    assert!(decode_collection_with_budget(json!({"not":"an array"}), 6).is_none());
    let mut first = recipe("a", 4);
    first.params.local_adjustments[0]
        .components
        .push(MaskComponent { mask: MaskShape::Raster { width: 2, height: 1, values: vec![9; 2] }, ..Default::default() });
    assert_eq!(first.raster_pixels(), 6);
    let second = recipe("b", 3);
    let encoded = json!([first, second]);
    assert!(decode_collection_with_budget(encoded.clone(), 8).is_none());
    let exact = decode_collection_with_budget(encoded, 9).unwrap();
    assert_eq!(exact, vec![first.clone(), second]);
    assert!(decode_collection_with_budget(json!([first.clone(), first]), 100).is_none(), "duplicate names retain the existing rejection policy");
}

#[test]
fn save_budget_is_atomic_and_replacement_subtracts_the_replaced_recipe() {
    let mut session = Session::new();
    let first = recipe("a", 4);
    save_with_budget(&mut session, &json!({"name":first.name,"params":first.params}), 6).unwrap();
    let before = session.presets.camera_raw.clone();
    let second = recipe("b", 3);
    assert!(save_with_budget(&mut session, &json!({"name":second.name,"params":second.params}), 6).is_err());
    assert_eq!(session.presets.camera_raw, before);
    let replacement = recipe("a", 2);
    save_with_budget(&mut session, &json!({"name":replacement.name,"params":replacement.params,"replace":true}), 3).unwrap();
    assert_eq!(session.presets.camera_raw, vec![replacement]);
    let second = recipe("b", 1);
    save_with_budget(&mut session, &json!({"name":second.name,"params":second.params}), 3).unwrap();
    let exact = session.presets.camera_raw.clone();
    let excessive = recipe("a", 3);
    assert!(save_with_budget(&mut session, &json!({"name":excessive.name,"params":excessive.params,"replace":true}), 3).is_err());
    assert_eq!(session.presets.camera_raw, exact);
}

#[test]
fn invalid_local_library_keeps_previous_camera_raw_and_other_preferences_still_load() {
    let mut session = Session::new();
    session.execute("cameraRaw.presets.save", json!({"name":"Existing","params":{"exposure":0.2}})).unwrap();
    let previous = session.presets.camera_raw.clone();
    session.load_presets_json(json!({"camera_raw":vec![Value::Null;MAX_PRESETS+1],"layer_defaults":{"bounded-load":{"opacity":50}}}));
    assert_eq!(session.presets.camera_raw, previous);
    assert_eq!(session.presets.layer_defaults.get("bounded-load"), Some(&json!({"opacity":50})));
    let legacy = recipe("Legacy", 2);
    let encoded = json!([legacy]);
    session.load_presets_json(json!({"camera_raw":encoded}));
    assert_eq!(session.presets.camera_raw, vec![legacy]);
}
