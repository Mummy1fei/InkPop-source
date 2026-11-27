use super::*;

fn fixture() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width":16,"height":16,"depth":32})).unwrap();
    app.run("edit.fill", json!({"color":"#808080"})).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
    (app, ctx)
}

#[test]
fn gradient_creation_control_arms_the_real_view_state_without_filter_or_document_edits() {
    let (mut app, ctx) = fixture();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let pixel = app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().rgba(0, 0);
    let before = app.camera_raw.as_ref().unwrap().params.clone();
    for (kind, label) in [("linear", "线性渐变"), ("radial", "径向渐变")] {
        let actual = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"maskEditor":{"create":kind}}})).unwrap().unwrap();
        assert_eq!(actual["maskEditor"]["creating"], label);
        assert_eq!(actual["maskEditor"]["placing"], true);
        assert!(actual["maskEditor"]["shape"].is_null());
        assert_eq!(actual["maskEditor"]["handles"], json!([]));
        assert_eq!(actual["tab"], "masks");
        assert_eq!(app.camera_raw.as_ref().unwrap().params, before);
        assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().rgba(0, 0), pixel);
    }
}

#[test]
fn invalid_or_mixed_gradient_creation_requests_leave_the_open_draft_unchanged() {
    let (mut app, ctx) = fixture();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"maskEditor":{"create":"linear"}}})).unwrap().unwrap();
    let expected = app.camera_raw.as_ref().unwrap().describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview);
    for request in [
        json!({"ui":{"maskEditor":{"create":"brush"}}}),
        json!({"ui":{"maskEditor":{"create":null}}}),
        json!({"ui":{"maskEditor":{"create":true}}}),
        json!({"ui":{"maskEditor":{"create":"radial","selected":0}}}),
        json!({"ui":{"maskEditor":{"create":"radial"},"set":{"exposure":1}}}),
        json!({"ui":{"maskEditor":{"create":"radial"},"tab":"edit"}}),
    ] {
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &request).unwrap().is_err());
        assert_eq!(app.camera_raw.as_ref().unwrap().describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview), expected);
    }
}

#[test]
fn cancelling_gradient_by_view_controls_restores_the_original_edit_mode() {
    for cancellation in [json!({"tab":"edit"}), json!({"maskEditor":{"selected":0}}), json!({"maskEditor":{"overlay":false}})] {
        let (mut app, ctx) = fixture();
        menu(
            &mut app,
            &ctx,
            "filter.cameraRaw",
            &json!({"ui":{"set":{"localAdjustments":[{
            "name":"原蒙版","mask":{"kind":"linear","start":[0.2,0.2],"end":[0.8,0.8]}
        }]},"tab":"masks","maskEditor":{"selected":0,"editing":false}}}),
        )
        .unwrap()
        .unwrap();
        let before = app.camera_raw.as_ref().unwrap().params.clone();
        for kind in ["linear", "radial"] {
            let armed = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"maskEditor":{"create":kind}}})).unwrap().unwrap();
            assert_eq!(armed["maskEditor"]["editing"], true);
        }
        let cancelled = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":cancellation})).unwrap().unwrap();
        assert_eq!(cancelled["maskEditor"]["editing"], false);
        assert!(cancelled["maskEditor"]["creating"].is_null());
        assert_eq!(app.camera_raw.as_ref().unwrap().params, before);
    }
}

#[test]
fn cancelling_an_empty_list_draft_restores_false_but_an_explicit_edit_setting_wins() {
    let (mut app, ctx) = fixture();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"maskEditor":{"create":"radial"}}})).unwrap().unwrap();
    let cancelled = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"tab":"edit"}})).unwrap().unwrap();
    assert_eq!(cancelled["maskEditor"]["editing"], false);
    assert!(cancelled["maskEditor"]["selected"].is_null() && cancelled["params"]["localAdjustments"].as_array().unwrap().is_empty());
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"maskEditor":{"create":"linear"}}})).unwrap().unwrap();
    let explicit = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"maskEditor":{"editing":true}}})).unwrap().unwrap();
    assert_eq!(explicit["maskEditor"]["editing"], true);
    assert!(explicit["maskEditor"]["creating"].is_null());
}
