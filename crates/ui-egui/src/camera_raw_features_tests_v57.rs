use super::*;

fn mask() -> Value {
    json!({"name":"中心曝光","exposure":1.0,"mask":{"kind":"radial","center":[0.5,0.5],"radius":[0.2,0.2],"feather":0.25}})
}

#[test]
fn local_mask_preview_commit_history_and_cancel_at_all_depths() {
    for depth in [8, 16, 32] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width":32,"height":32,"depth":depth})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color":"#404040"})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let read = |app: &PhotocraftApp, x, y| app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().rgba(x, y);
        let before = read(&app, 16, 16);
        let steps = app.session.active().unwrap().history.past_len();
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
        menu(
            &mut app,
            &ctx,
            "filter.cameraRaw",
            &json!({"ui":{"set":{"localAdjustments":[mask()]},"maskEditor":{"selected":0,"editing":false,"overlay":true}}}),
        )
        .unwrap()
        .unwrap();
        let d = app.camera_raw.as_ref().unwrap();
        assert!(d.processed[16 * 32 + 16][0] > before[0]);
        assert_eq!(d.processed[0], d.proxy[0]);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        assert_eq!(read(&app, 16, 16), before);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"cancel":true}})).unwrap().unwrap();
        assert_eq!(read(&app, 16, 16), before);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
        menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"set":{"localAdjustments":[mask()]},"commit":true}})).unwrap().unwrap();
        let after = read(&app, 16, 16);
        assert!(after[0] > before[0]);
        assert_eq!(after[3], before[3]);
        assert_eq!(read(&app, 0, 0), before);
        assert_eq!(app.session.active().unwrap().history.past_len(), steps + 1);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(read(&app, 16, 16), before);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(read(&app, 16, 16), after);
    }
}

#[test]
fn raw_preset_save_load_keeps_pending_masks_and_invalid_control_is_atomic() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width":16,"height":16})).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"set":{"exposure":0.5,"localAdjustments":[mask()]}}})).unwrap().unwrap();
    let pending = app.camera_raw.as_ref().unwrap().params.clone();
    let steps = app.session.active().unwrap().history.past_len();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"preset":{"action":"save","name":"局部暖色"}}})).unwrap().unwrap();
    assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    for update in [
        json!({"set":{"exposure":2},"maskEditor":{"selected":999}}),
        json!({"set":{"localAdjustments":[{"name":"bad","mask":{"kind":"linear","start":[0,0],"end":[0,0]}}]}}),
        json!({"preset":{"action":"save","name":"no"},"set":{"exposure":2}}),
    ] {
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":update})).unwrap().is_err());
        assert_eq!(app.camera_raw.as_ref().unwrap().params, pending);
    }
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"cancel":true}})).unwrap().unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"preset":{"action":"load","name":"局部暖色"}}})).unwrap().unwrap();
    assert_eq!(app.camera_raw.as_ref().unwrap().params, pending);
    let prefs = app.session.prefs_to_json();
    let mut loaded = photocraft_engine::Session::new();
    loaded.load_prefs_json(&prefs).unwrap();
    assert_eq!(loaded.presets.camera_raw[0].params, pending);
}

#[test]
fn raw_recipe_envelope_roundtrip_and_generation_requests_validate_atomically() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width":16,"height":16})).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"set":{"localAdjustments":[mask()]}}})).unwrap().unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"preset":{"action":"save","name":"Original"}}})).unwrap().unwrap();
    let mut data = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"preset":{"action":"export","name":"Original"}}})).unwrap().unwrap();
    assert_eq!(data["format"], "photocraft-camera-raw-v1");
    data["name"] = json!("Imported");
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"preset":{"action":"import","data":data}}})).unwrap().unwrap();
    assert_eq!(app.session.presets.camera_raw[0].params, app.session.presets.camera_raw[1].params);
    let saved = app.session.presets.camera_raw.clone();
    let params = app.camera_raw.as_ref().unwrap().params.clone();
    for ui in [
        json!({"preset":{"action":"import","data":{"format":"wrong","name":"x","params":{}}}}),
        json!({"preset":{"action":"import","data":data,"name":"extra"}}),
        json!({"preset":{"action":"export","name":"Original"},"set":{"exposure":4}}),
        json!({"maskEditor":{"generate":"unknown"}}),
        json!({"maskEditor":{"generate":"sky","cancelGeneration":true}}),
        json!({"maskEditor":{"generate":"sky"},"set":{"exposure":4}}),
        json!({"maskEditor":{"cancelGeneration":false}}),
    ] {
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":ui})).unwrap().is_err());
        assert_eq!(app.session.presets.camera_raw, saved);
        assert_eq!(app.camera_raw.as_ref().unwrap().params, params);
        assert!(!app.camera_raw.as_ref().unwrap().masks.generating());
    }
}
