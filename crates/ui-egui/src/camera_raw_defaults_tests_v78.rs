use super::*;
use photocraft_algo::camera_raw_color::RgbSpace;
use photocraft_color::SampleType;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":16,"height":12,"depth":32,"background":"transparent"})).unwrap();
    app.run("edit.fill", json!({"color":"#305080"})).unwrap();
    app
}

fn defaults(app: &mut PhotocraftApp, space: &str, depth: &str, dpi: f64) {
    for (path, value) in [("rawDefaults.colorSpace", json!(space)), ("rawDefaults.bitDepth", json!(depth)), ("rawDefaults.resolution", json!(dpi))] {
        app.run("prefs.set", json!({"path":path,"value":value})).unwrap();
    }
}

#[test]
fn raw_defaults_v78_new_dialog_uses_output_defaults_and_keeps_document_working_space() {
    for (name, expected) in
        [("srgb", RgbSpace::Srgb), ("adobeRgb", RgbSpace::AdobeRgb), ("proPhoto", RgbSpace::ProPhotoRgb), ("displayP3", RgbSpace::DisplayP3)]
    {
        let mut app = app();
        defaults(&mut app, name, "8", 245.5);
        open(&mut app, &egui::Context::default()).unwrap();
        let d = app.camera_raw.as_ref().unwrap();
        assert_eq!(d.params.color_spaces.working, RgbSpace::Document);
        assert_eq!(d.params.color_spaces.output, expected);
        assert_eq!(d.output_options.depth, 8);
        assert_eq!(d.output_options.resolution, 245.5);
    }
}

#[test]
fn raw_defaults_v78_dialog_output_edits_are_retained_and_forwarded_to_real_output() {
    let mut app = app();
    defaults(&mut app, "srgb", "16", 300.0);
    open(&mut app, &egui::Context::default()).unwrap();
    let d = app.camera_raw.as_mut().unwrap();
    d.output_options.depth = 8;
    d.output_options.resolution = 180.0;
    // Later preference changes must not overwrite an open dialog's choices.
    defaults(&mut app, "displayP3", "16", 500.0);
    let original = app.session.active().unwrap().doc.clone();
    let result = create_output(&mut app).unwrap();
    assert_eq!(result["depth"], 8);
    assert_eq!(result["resolution"], 180.0);
    assert_eq!(app.session.active().unwrap().doc.depth, SampleType::U8);
    assert_eq!(app.session.active().unwrap().doc.resolution_dpi, 180.0);
    assert_eq!(app.session.active().unwrap().doc.icc_profile.as_ref().unwrap(), &photocraft_cms::Builtin::Srgb.profile().to_bytes());
    assert!(std::sync::Arc::ptr_eq(&app.session.documents()[0].doc, &original));
}

#[test]
fn raw_defaults_v78_existing_smart_filter_settings_are_not_replaced_by_defaults() {
    let mut app = app();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("filter.cameraRaw", json!({"exposure":0.25,"colorSpaces":{"working":"proPhotoRgb","output":"displayP3"}})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    defaults(&mut app, "srgb", "8", 240.0);
    open_smart_filter(&mut app, &egui::Context::default(), layer, 0).unwrap();
    let d = app.camera_raw.as_ref().unwrap();
    assert_eq!(d.params.exposure, 0.25);
    assert_eq!(d.params.color_spaces.working, RgbSpace::ProPhotoRgb);
    assert_eq!(d.params.color_spaces.output, RgbSpace::DisplayP3);
    assert_eq!(d.original.color_spaces, d.params.color_spaces);
}

#[test]
fn raw_defaults_v78_normal_filter_commit_does_not_change_document_depth_or_dpi() {
    let mut app = app();
    let original = app.session.active().unwrap().doc.clone();
    defaults(&mut app, "displayP3", "8", 450.0);
    let ctx = egui::Context::default();
    open(&mut app, &ctx).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"set":{"exposure":0.1},"commit":true}})).unwrap().unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active().unwrap().doc.depth, SampleType::F32);
    assert_eq!(app.session.active().unwrap().doc.resolution_dpi, original.resolution_dpi);
    assert_eq!(app.session.active().unwrap().doc.icc_profile, original.icc_profile);
}
