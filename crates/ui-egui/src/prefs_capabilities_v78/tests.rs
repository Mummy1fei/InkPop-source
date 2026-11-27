use super::*;
use egui_kittest::kittest::Queryable;
use std::sync::{Arc, Mutex};

type Store = Arc<Mutex<Option<String>>>;

fn app_on(store: &Store) -> PhotocraftApp {
    let reader = store.clone();
    let writer = store.clone();
    let mut app = PhotocraftApp::new(
        photocraft_engine::Session::new(),
        crate::Services {
            load_prefs: Some(Box::new(move || reader.lock().unwrap().clone())),
            save_prefs: Some(Box::new(move |text| {
                *writer.lock().unwrap() = Some(text.to_string());
                Ok(())
            })),
            ..Default::default()
        },
    );
    load(&mut app);
    app
}

fn stored(store: &Store) -> Value {
    serde_json::from_str(store.lock().unwrap().as_deref().unwrap()).unwrap()
}

fn draft(app: &mut PhotocraftApp, id: u64) -> &mut Value {
    app.ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap()
}

const ENABLED: &[&str] = &[
    "general.resizeImageDuringPlace",
    "tools.zoomClickedPointToCenter",
    "tools.varyRoundBrushHardnessOnHud",
    "performance.effectCacheMb",
    "cursors.brushPreviewColor",
    "type.useEscToCommit",
    "type.fillNewTypeLayersWithPlaceholder",
    "enhancedControls.touchGestures",
    "enhancedControls.zoomWithTrackpadPinch",
    "enhancedControls.rotateViewWithTrackpad",
    "rawDefaults.colorSpace",
    "rawDefaults.bitDepth",
    "rawDefaults.resolution",
];

#[test]
fn prefs_capabilities_v78_supported_fields_are_visible_and_pending_capabilities_stay_hidden() {
    let defaults = prefs::Preferences::default();
    for path in ENABLED {
        assert!(defaults.get(path).is_some(), "field exists: {path}");
        assert!(!prefs::is_hidden(path), "supported field visible: {path}");
    }
    for path in ["type.smartQuotes", "scratchDisks.disks", "integrations.allowAgentControl", "rawDefaults.applyAutoTone"] {
        assert!(prefs::is_hidden(path), "unimplemented field remains hidden: {path}");
    }
    for section in ["type", "enhancedControls", "rawDefaults"] {
        assert!(has_visible_fields(&defaults.to_json(), section));
    }
}

#[test]
fn prefs_capabilities_v78_apply_cancel_and_reload_preserve_supported_and_hidden_values() {
    let store = Arc::new(Mutex::new(None));
    let mut app = app_on(&store);
    app.run("prefs.set", json!({"path":"type.smartQuotes","value":false})).unwrap();
    let id = open_preferences(&mut app, "type");
    let values = draft(&mut app, id);
    values["general"]["resizeImageDuringPlace"] = json!(false);
    values["tools"]["zoomClickedPointToCenter"] = json!(true);
    values["tools"]["varyRoundBrushHardnessOnHud"] = json!(false);
    values["performance"]["effectCacheMb"] = json!(48);
    values["cursors"]["brushPreviewColor"] = json!("#4080a0");
    values["type"]["useEscToCommit"] = json!(false);
    values["type"]["fillNewTypeLayersWithPlaceholder"] = json!(true);
    values["enhancedControls"]["touchGestures"] = json!(false);
    values["enhancedControls"]["zoomWithTrackpadPinch"] = json!(false);
    values["enhancedControls"]["rotateViewWithTrackpad"] = json!(true);
    values["rawDefaults"]["colorSpace"] = json!("displayP3");
    values["rawDefaults"]["bitDepth"] = json!("8");
    values["rawDefaults"]["resolution"] = json!(245.5);
    apply(&mut app, id).unwrap();
    let expected = app.session.prefs().clone();
    // Cancel discards only the edits after Apply.
    draft(&mut app, id)["rawDefaults"]["resolution"] = json!(600.0);
    app.ui.close_dialog(id);
    save_preferences(&mut app).unwrap();
    let restarted = app_on(&store);
    for path in ENABLED {
        assert_eq!(restarted.session.prefs().get(path), expected.get(path), "{path}");
    }
    assert!(!restarted.session.prefs().type_.smart_quotes);
    assert_eq!(stored(&store)["version"], 2);
}

#[test]
fn prefs_capabilities_v78_reset_section_is_a_draft_then_applies_and_persists() {
    let store = Arc::new(Mutex::new(None));
    let mut app = app_on(&store);
    app.run("prefs.set", json!({"values":{"rawDefaults.colorSpace":"srgb","rawDefaults.bitDepth":"8","rawDefaults.resolution":600.0}})).unwrap();
    let id = open_preferences(&mut app, "rawDefaults");
    let fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    struct Fixture {
        app: PhotocraftApp,
        fields: Map<String, Value>,
    }
    impl eframe::App for Fixture {
        fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
            prefs_body(&mut self.app, ui, &mut self.fields);
        }
    }
    let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::default());
        Fixture { app, fields }
    });
    h.run();
    h.get_by_label(tl!("Reset Section")).click();
    h.run();
    assert_eq!(h.state().fields["values"]["rawDefaults"], prefs::Preferences::default().get("rawDefaults").unwrap());
    assert_eq!(h.state().app.session.prefs().raw_defaults.resolution, 600.0);
    let fixture = h.state_mut();
    fixture.app.ui.dialog_mut(id).unwrap().fields = fixture.fields.clone();
    apply(&mut fixture.app, id).unwrap();
    save_preferences(&mut fixture.app).unwrap();
    let restarted = app_on(&store);
    assert_eq!(restarted.session.prefs().raw_defaults, prefs::RawDefaults::default());
}

#[test]
fn prefs_capabilities_v78_unavailable_pages_explain_the_actual_limitations_with_inkpop_name() {
    for (section, message) in [
        (
            "scratchDisks",
            tl!("InkPop keeps image tiles in memory. Disk paging for image tiles is not implemented yet, so choosing a scratch disk would have no effect."),
        ),
        (
            "integrations",
            tl!(
                "InkPop agent control is configured at launch with --control and explicit automation read/write folders. A Preferences switch cannot grant access or start the server in this session."
            ),
        ),
    ] {
        let mut h = egui_kittest::Harness::builder().with_size(vec2(800.0, 300.0)).build_ui(move |ui| unavailable_section(ui, section));
        h.run();
        assert!(h.query_by_label(message).is_some(), "{section}: actionable explanation");
        assert!(message.contains("InkPop"));
        assert!(!message.contains("PhotoCraft"));
    }
}

#[test]
fn prefs_capabilities_v78_legacy_placeholder_migration_can_be_explicitly_enabled_and_saved() {
    let store = Arc::new(Mutex::new(Some(json!({"version":1,"type":{"fillNewTypeLayersWithPlaceholder":true}}).to_string())));
    let mut app = app_on(&store);
    assert!(!app.session.prefs().type_.fill_new_type_layers_with_placeholder);
    let id = open_preferences(&mut app, "type");
    draft(&mut app, id)["type"]["fillNewTypeLayersWithPlaceholder"] = json!(true);
    apply(&mut app, id).unwrap();
    save_preferences(&mut app).unwrap();
    assert_eq!(stored(&store)["version"], 2);
    assert_eq!(stored(&store)["type"]["fillNewTypeLayersWithPlaceholder"], true);
    assert!(app_on(&store).session.prefs().type_.fill_new_type_layers_with_placeholder);
}

#[test]
fn prefs_capabilities_v78_concurrent_version_two_placeholder_choice_survives_unrelated_edits() {
    let store = Arc::new(Mutex::new(Some(json!({"version":1,"type":{"fillNewTypeLayersWithPlaceholder":true}}).to_string())));
    let mut app = app_on(&store);
    assert!(!app.session.prefs().type_.fill_new_type_layers_with_placeholder);
    let mut newer = app.session.prefs_value();
    newer["version"] = json!(2);
    newer["type"]["fillNewTypeLayersWithPlaceholder"] = json!(true);
    *store.lock().unwrap() = Some(newer.to_string());
    let id = open_preferences(&mut app, "rawDefaults");
    draft(&mut app, id)["rawDefaults"]["resolution"] = json!(240.0);
    apply(&mut app, id).unwrap();
    save_preferences(&mut app).unwrap();
    let restarted = app_on(&store);
    assert!(restarted.session.prefs().type_.fill_new_type_layers_with_placeholder);
    assert_eq!(restarted.session.prefs().raw_defaults.resolution, 240.0);
}
