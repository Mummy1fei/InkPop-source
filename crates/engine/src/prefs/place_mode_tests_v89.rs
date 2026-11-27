use super::*;

#[test]
fn new_place_default_is_pixels_but_missing_serialized_marker_is_legacy_v89() {
    let fresh = General::default();
    assert!(!fresh.always_create_smart_objects_when_placing);
    assert_eq!(fresh.place_mode_version, 1);
    let legacy: General = serde_json::from_value(json!({"alwaysCreateSmartObjectsWhenPlacing":true})).unwrap();
    assert!(legacy.always_create_smart_objects_when_placing, "deserialization itself preserves the old value until shell migration");
    assert_eq!(legacy.place_mode_version, 0);
    let roundtrip: General = serde_json::from_value(serde_json::to_value(fresh).unwrap()).unwrap();
    assert_eq!(roundtrip.place_mode_version, 1);
}

#[test]
fn placement_choice_is_visible_and_internal_marker_is_hidden_v89() {
    assert!(!is_hidden("general.alwaysCreateSmartObjectsWhenPlacing"));
    assert!(!HIDDEN_UNTIL_IMPLEMENTED.contains(&"general.alwaysCreateSmartObjectsWhenPlacing"));
    assert!(is_hidden("general.placeModeVersion"));
}

#[test]
fn explicit_smart_preference_marks_intent_and_survives_serialization_v89() {
    let mut session = Session::new();
    session.load_prefs_json(r#"{"general":{"alwaysCreateSmartObjectsWhenPlacing":true}}"#).unwrap();
    assert_eq!(session.prefs().general.place_mode_version, 0);
    session.execute("prefs.set", json!({"path":"general.alwaysCreateSmartObjectsWhenPlacing","value":true})).unwrap();
    assert_eq!(session.prefs().general.place_mode_version, 1);
    let saved = session.prefs_to_json();
    let mut restored = Session::new();
    restored.load_prefs_json(&saved).unwrap();
    assert!(restored.prefs().general.always_create_smart_objects_when_placing);
    assert_eq!(restored.prefs().general.place_mode_version, 1);
    restored.execute("prefs.reset", json!({"path":"general"})).unwrap();
    assert!(!restored.prefs().general.always_create_smart_objects_when_placing);
    assert_eq!(restored.prefs().general.place_mode_version, 1);
}
