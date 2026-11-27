use super::*;
use std::sync::{Arc, Mutex};

fn stored_app(text: Option<String>) -> (PhotocraftApp, Arc<Mutex<Option<String>>>) {
    let store = Arc::new(Mutex::new(text));
    let read = store.clone();
    let write = store.clone();
    let services = crate::Services {
        load_prefs: Some(Box::new(move || read.lock().unwrap().clone())),
        save_prefs: Some(Box::new(move |text| {
            *write.lock().unwrap() = Some(text.to_string());
            Ok(())
        })),
        ..Default::default()
    };
    (PhotocraftApp::new(photocraft_engine::Session::new(), services), store)
}

#[test]
fn legacy_place_default_migrates_after_saved_baseline_and_persists_v89() {
    let (mut app, store) = stored_app(Some(r#"{"general":{"alwaysCreateSmartObjectsWhenPlacing":true}}"#.into()));
    assert!(!app.session.prefs().general.always_create_smart_objects_when_placing);
    assert_eq!(app.session.prefs().general.place_mode_version, 1);
    assert_eq!(app.prefs_rt.saved_value.as_ref().unwrap()["general"]["placeModeVersion"], json!(0));
    assert_eq!(app.prefs_rt.saved_value.as_ref().unwrap()["general"]["alwaysCreateSmartObjectsWhenPlacing"], json!(true));
    assert_ne!(app.prefs_rt.saved_rev, app.session.prefs.rev(), "migration must remain pending for real persistence");
    assert_eq!(persist(&mut app, 1.0), None);
    let saved = store.lock().unwrap().clone().unwrap();
    let json: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(json["general"]["placeModeVersion"], 1);
    assert_eq!(json["general"]["alwaysCreateSmartObjectsWhenPlacing"], false);
    let (restored, _) = stored_app(Some(saved));
    assert!(!restored.session.prefs().general.always_create_smart_objects_when_placing);
    assert_eq!(restored.session.prefs().general.place_mode_version, 1);
}

#[test]
fn later_explicit_smart_choice_survives_restart_and_repeated_load_v89() {
    let (mut app, store) = stored_app(Some(r#"{"general":{"alwaysCreateSmartObjectsWhenPlacing":true}}"#.into()));
    app.run("prefs.set", json!({"path":"general.alwaysCreateSmartObjectsWhenPlacing","value":true})).unwrap();
    assert_eq!(persist(&mut app, 1.0), None);
    let (mut restored, _) = stored_app(store.lock().unwrap().clone());
    assert!(restored.session.prefs().general.always_create_smart_objects_when_placing);
    assert_eq!(restored.session.prefs().general.place_mode_version, 1);
    load(&mut restored);
    assert!(restored.session.prefs().general.always_create_smart_objects_when_placing);
}

#[test]
fn general_editor_has_place_choice_without_migration_marker_field_v89() {
    assert!(prefs::is_hidden("general.placeModeVersion"));
    assert!(!prefs::is_hidden("general.alwaysCreateSmartObjectsWhenPlacing"));
    let mut obj = prefs::Preferences::default().to_json()["general"].as_object().unwrap().clone();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::StudioLight);
    let mut output = ctx.run_ui(Default::default(), |ui| {
        section_fields(ui, "general", &mut obj, &[], crate::i18n::current());
    });
    output.textures_delta.clear();
    assert_eq!(obj["alwaysCreateSmartObjectsWhenPlacing"], false);
    assert_eq!(obj["placeModeVersion"], 1);
}
