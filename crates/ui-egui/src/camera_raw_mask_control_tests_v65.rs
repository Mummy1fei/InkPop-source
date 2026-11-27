use super::*;

fn fixture() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width":16,"height":16,"depth":32})).unwrap();
    app.run("edit.fill", json!({"color":"#808080"})).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({})).unwrap().unwrap();
    menu(
        &mut app,
        &ctx,
        "filter.cameraRaw",
        &json!({"ui":{"set":{"localAdjustments":[{
        "name":"蒙版 1","exposure":0.7,"vibrance":12,"invert":true,
        "mask":{"kind":"radial","center":[0.5,0.5],"radius":[0.3,0.2],"feather":0.5,"rotation":28}
    }]},"tab":"masks","maskEditor":{"selected":0,"editing":true,"overlay":true}}}),
    )
    .unwrap()
    .unwrap();
    (app, ctx)
}
fn action(app: &mut PhotocraftApp, ctx: &egui::Context, operation: Value) -> Value {
    menu(app, ctx, "filter.cameraRaw", &json!({"ui":{"maskEditor":{"action":operation}}})).unwrap().unwrap()
}

#[test]
fn mask_actions_route_engine_drafts_preserve_document_and_keep_inverted_intersections() {
    let (mut app, ctx) = fixture();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let before = app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().rgba(0, 0);
    let original = app.camera_raw.as_ref().unwrap().params.local_adjustments[0].clone();
    assert_eq!(action(&mut app, &ctx, json!({"kind":"rename","index":0,"name":"局部颜色"}))["params"]["localAdjustments"][0]["name"], "局部颜色");
    let copy = action(&mut app, &ctx, json!({"kind":"duplicate","index":0,"invert":true}));
    assert_eq!(copy["params"]["localAdjustments"].as_array().unwrap().len(), 2);
    assert_eq!(copy["params"]["localAdjustments"][1]["invert"], false);
    assert_eq!(copy["maskEditor"]["selected"], 1);
    action(&mut app, &ctx, json!({"kind":"setEnabled","index":0,"enabled":false}));
    assert_eq!(app.camera_raw.as_ref().unwrap().masks.selected, Some(1), "eye does not select another row");
    action(&mut app, &ctx, json!({"kind":"setEnabled","index":0,"enabled":true}));
    action(&mut app, &ctx, json!({"kind":"intersect","index":0,"shape":{"kind":"linear","start":[0.1,0.2],"end":[0.9,0.8]}}));
    let draft = &app.camera_raw.as_ref().unwrap().params.local_adjustments[0];
    assert!(!draft.invert);
    assert!(draft.components[0].intersect && draft.components[0].invert_previous);
    assert_eq!(draft.mask, original.mask);
    assert_eq!(app.camera_raw.as_ref().unwrap().masks.selected_component, Some(0));
    let geometry = draft.clone();
    action(&mut app, &ctx, json!({"kind":"resetAdjustments","index":0}));
    let reset = &app.camera_raw.as_ref().unwrap().params.local_adjustments[0];
    assert_eq!(reset.exposure, 0.0);
    assert_eq!(reset.vibrance, 0.0);
    assert_eq!(reset.mask, geometry.mask);
    assert_eq!(reset.components, geometry.components);
    assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().rgba(0, 0), before, "draft commands must not edit document pixels");
    action(&mut app, &ctx, json!({"kind":"deleteAll"}));
    assert!(app.camera_raw.as_ref().unwrap().params.local_adjustments.is_empty());
    assert_eq!(app.camera_raw.as_ref().unwrap().masks.selected, None);
}

#[test]
fn invalid_mixed_or_generating_mask_actions_leave_draft_and_selection_unchanged() {
    let (mut app, ctx) = fixture();
    let before = app.camera_raw.as_ref().unwrap().params.clone();
    for request in [
        json!({"ui":{"maskEditor":{"action":{"kind":"delete","index":99}}}}),
        json!({"ui":{"maskEditor":{"action":{"kind":"rename","index":0,"name":""}}}}),
        json!({"ui":{"maskEditor":{"action":{"kind":"deleteAll","extra":true}}}}),
        json!({"ui":{"maskEditor":{"action":{"kind":"deleteAll"},"selected":0}}}),
        json!({"ui":{"maskEditor":{"action":{"kind":"deleteAll"}},"set":{"exposure":1}}}),
    ] {
        assert!(menu(&mut app, &ctx, "filter.cameraRaw", &request).unwrap().is_err());
        let d = app.camera_raw.as_ref().unwrap();
        assert_eq!(d.params, before);
        assert_eq!(d.masks.selected, Some(0));
    }
    app.camera_raw.as_mut().unwrap().masks.generate("sky").unwrap();
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"maskEditor":{"action":{"kind":"deleteAll"}}}})).unwrap().is_err());
    let d = app.camera_raw.as_ref().unwrap();
    assert_eq!(d.params, before);
    assert!(d.masks.generating());
    assert_eq!(d.masks.selected, Some(0));
}
