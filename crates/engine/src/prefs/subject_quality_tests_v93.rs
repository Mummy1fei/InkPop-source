use super::*;

const PATH: &str = "performance.subjectRecognitionQuality";

#[test]
fn subject_quality_defaults_to_fast_and_is_a_supported_typed_preference_v93() {
    let prefs = Preferences::default();
    assert_eq!(prefs.performance.subject_recognition_quality, ImageRecognitionQuality::Fast);
    assert_eq!(prefs.get(PATH), Some(json!("fast")));
    assert_eq!(choices(PATH), Some(ImageRecognitionQuality::NAMES));
    assert_eq!(ImageRecognitionQuality::NAMES, &["fast", "precise"]);
    assert!(!is_hidden(PATH));
    assert_eq!(ImageRecognitionQuality::Fast.backend(), crate::image_recognition::BackendPreference::Auto);
    assert_eq!(ImageRecognitionQuality::Precise.backend(), crate::image_recognition::BackendPreference::BiRefNet);
}

#[test]
fn subject_quality_changes_through_real_preference_commands_and_resets_v93() {
    let mut s = Session::new();
    s.execute("prefs.set", json!({"path":PATH,"value":"precise"})).unwrap();
    assert_eq!(s.execute("prefs.get", json!({"path":PATH})).unwrap(), json!("precise"));
    assert_eq!(s.prefs().performance.subject_recognition_quality.backend(), crate::image_recognition::BackendPreference::BiRefNet);
    let section = s.execute("edit.preferences.performance", json!({})).unwrap();
    assert_eq!(section["values"]["subjectRecognitionQuality"], "precise");
    s.execute("prefs.reset", json!({"path":PATH})).unwrap();
    assert_eq!(s.prefs().performance.subject_recognition_quality, ImageRecognitionQuality::Fast);
    s.execute("prefs.set", json!({"path":"performance","value":{"subjectRecognitionQuality":"precise"}})).unwrap();
    assert_eq!(s.prefs().performance.subject_recognition_quality, ImageRecognitionQuality::Precise);
    s.execute("prefs.reset", json!({"path":"performance"})).unwrap();
    assert_eq!(s.prefs().performance.subject_recognition_quality, ImageRecognitionQuality::Fast);
}

#[test]
fn subject_quality_rejects_bad_values_and_mixed_updates_are_atomic_v93() {
    let mut s = Session::new();
    let before = s.prefs().clone();
    for value in [json!("gpu"), json!("auto"), json!("PRECISE"), json!(true), json!(null), json!(1), json!(["fast"])] {
        assert!(s.execute("prefs.set", json!({"path":PATH,"value":value})).is_err());
        assert_eq!(s.prefs(), &before);
    }
    assert!(s.execute("prefs.set", json!({"values":{"performance.subjectRecognitionQuality":"precise","performance.historyStates":0}})).is_err());
    assert_eq!(s.prefs(), &before);
    assert!(s.execute("prefs.set", json!({"path":"performance","value":{"subjectRecognitionQuality":"other"}})).is_err());
    assert_eq!(s.prefs(), &before);
}

#[test]
fn subject_quality_round_trips_and_missing_legacy_field_stays_fast_v93() {
    let mut s = Session::new();
    s.execute("prefs.set", json!({"path":PATH,"value":"precise"})).unwrap();
    let saved = s.prefs_to_json();
    let mut restored = Session::new();
    restored.load_prefs_json(&saved).unwrap();
    assert_eq!(restored.prefs().performance.subject_recognition_quality, ImageRecognitionQuality::Precise);
    let mut legacy = Session::new();
    legacy.load_prefs_json(r#"{"performance":{"historyStates":12}}"#).unwrap();
    assert_eq!(legacy.prefs().performance.subject_recognition_quality, ImageRecognitionQuality::Fast);
    assert_eq!(legacy.prefs().performance.history_states, 12);
    let before = restored.prefs().clone();
    assert!(restored.load_prefs_json(r#"{"performance":{"subjectRecognitionQuality":"gpu"}}"#).is_err());
    assert_eq!(restored.prefs(), &before);
}
