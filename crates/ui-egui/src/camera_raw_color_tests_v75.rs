use super::*;
use crate::theme::ThemeKind;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":64,"height":48,"depth":32,"background":"transparent"})).unwrap();
    app.run("edit.fill", json!({"color":"#306090"})).unwrap();
    app
}

#[test]
fn raw_color_v75_ui_preview_preset_rebind_and_output_copy_keep_source_intact() {
    let mut app = app();
    let ctx = egui::Context::default();
    open(&mut app, &ctx).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"set":{"exposure":0.4,"colorSpaces":{"working":"proPhotoRgb","output":"displayP3"}}}}))
        .unwrap()
        .unwrap();
    let d = app.camera_raw.as_ref().unwrap();
    assert_eq!(d.params.color_spaces.source_profile, "srgb");
    let mut expected = d.proxy.to_vec();
    photocraft_algo::camera_raw::develop(&mut expected, d.pw, d.ph, &d.params, true);
    for (a, b) in d.processed.iter().flatten().zip(expected.iter().flatten()) {
        assert!((a - b).abs() < 2e-6);
    }
    presets::execute(&mut app, &ctx, presets::Action::Save("Spaces".into())).unwrap();
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"cancel":true}})).unwrap().unwrap();
    app.run("edit.assignProfile", json!({"profile":"display-p3"})).unwrap();
    open(&mut app, &ctx).unwrap();
    presets::execute(&mut app, &ctx, presets::Action::Load("Spaces".into())).unwrap();
    assert_eq!(app.camera_raw.as_ref().unwrap().params.color_spaces.source_profile, "display-p3");
    assert_eq!(app.camera_raw.as_ref().unwrap().params.color_spaces.output, photocraft_algo::camera_raw_color::RgbSpace::DisplayP3);
    let original = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    let result = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"createOutput":true}})).unwrap().unwrap();
    assert_eq!(result["originalUnchanged"], true);
    assert!(app.camera_raw.is_none());
    assert_eq!(app.session.documents().len(), 2);
    assert!(std::sync::Arc::ptr_eq(&app.session.documents()[0].doc, &original));
    assert_eq!(app.session.documents()[0].history.past_len(), history);
    assert!(app.session.active().unwrap().floating.is_none());
    assert_eq!(app.session.active().unwrap().doc.icc_profile.as_ref().unwrap(), &photocraft_cms::Builtin::DisplayP3.profile().to_bytes());
    assert_eq!(app.ui.views.len(), 2);
}

#[test]
fn raw_color_v75_normal_ok_retains_document_profile_and_working_cache_is_correct() {
    let mut app = app();
    let ctx = egui::Context::default();
    open(&mut app, &ctx).unwrap();
    let original_profile = app.session.active().unwrap().doc.icc_profile.clone();
    menu(
        &mut app,
        &ctx,
        "filter.cameraRaw",
        &json!({"ui":{"set":{"contrast":20,"colorSpaces":{"working":"adobeRgb","output":"proPhotoRgb"},"localAdjustments":[{"exposure":0.3}]}}}),
    )
    .unwrap()
    .unwrap();
    let d = app.camera_raw.as_ref().unwrap();
    let cached = local_cache::render(&d.proxy, d.pw, d.ph, &d.params, true, None);
    assert!(cached.global.is_none(), "work-domain local stages must not reuse a document-domain prefix");
    let mut exact = d.proxy.to_vec();
    photocraft_algo::camera_raw::develop(&mut exact, d.pw, d.ph, &d.params, true);
    assert_eq!(cached.pixels, exact);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"commit":true}})).unwrap().unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.active().unwrap().doc.icc_profile, original_profile);
}

#[test]
fn raw_color_v75_ui_sampled_hdr_source_reports_actionable_error() {
    let mut app = app();
    let mut profile = photocraft_cms::Builtin::Srgb.profile().clone();
    profile.trc = Some(std::array::from_fn(|_| photocraft_cms::Curve::Table(vec![0., 0.214, 1.])));
    let profile = profile.with_encoded_bytes();
    app.session
        .edit("Synthetic sampled ICC and HDR", |doc, active| {
            doc.icc_profile = Some(profile.to_bytes());
            let surface = doc
                .layer_mut(active.ok_or(photocraft_engine::EngineError::NoDocument)?)
                .ok_or(photocraft_engine::EngineError::NoDocument)?
                .surface_mut()
                .ok_or(photocraft_engine::EngineError::NoDocument)?;
            surface.write_pixel(0, 0, &[1.8, 1.8, 1.8, 0.61]);
            Ok(())
        })
        .unwrap();
    let original = app.session.active().unwrap().doc.clone();
    let ctx = egui::Context::default();
    open(&mut app, &ctx).unwrap();
    let error = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"set":{"exposure":0.2,"colorSpaces":{"working":"adobeRgb"}}}})).unwrap().unwrap_err();
    assert!(error.contains("32"));
    assert_eq!(app.camera_raw.as_ref().unwrap().params.color_spaces.working, photocraft_algo::camera_raw_color::RgbSpace::Document);
    menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"set":{"colorSpaces":{"output":"displayP3"}}}})).unwrap().unwrap();
    let error = menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"createOutput":true}})).unwrap().unwrap_err();
    assert!(error.contains("32"));
    assert_eq!(app.session.documents().len(), 1);
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &original));
    assert!(app.session.active().unwrap().doc.layers[0].surface().unwrap().rgba(0, 0)[0] > 1.79);
}

#[test]
#[ignore = "writes five production Camera Raw ICC control screenshots for root review"]
fn raw_color_gpu_visual_v75() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v75/raw-color-gpu");
    assert!(!out.exists(), "preserve previous candidates");
    std::fs::create_dir_all(&out).unwrap();
    let mut manifest = Vec::new();
    for kind in ThemeKind::ALL {
        let mut h =
            egui_kittest::Harness::builder().with_size(vec2(1440.0, 1000.0)).with_max_steps(96).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_eframe(
                move |cc| {
                    PhotocraftApp::setup_context(&cc.egui_ctx, kind);
                    let mut app = app();
                    app.run("prefs.set", json!({"path":"interface.theme","value":kind.id()})).unwrap();
                    app.run("prefs.set", json!({"path":"interface.language","value":"zh-hans"})).unwrap();
                    crate::prefs_ui::load(&mut app);
                    app.custom_titlebar = true;
                    app.set_theme(&cc.egui_ctx, kind);
                    app
                },
            );
        let ctx = h.ctx.clone();
        open(h.state_mut(), &ctx).unwrap();
        menu(
            h.state_mut(),
            &ctx,
            "filter.cameraRaw",
            &json!({"ui":{"tab":"colorSpaces","set":{"exposure":0.4,"colorSpaces":{"working":"proPhotoRgb","output":"displayP3"}}}}),
        )
        .unwrap()
        .unwrap();
        h.run_steps(8);
        let d = h.state().camera_raw.as_ref().unwrap();
        for rect in [d.color_geometry.working, d.color_geometry.output, d.color_geometry.create] {
            let rect = rect.unwrap();
            assert!(ctx.content_rect().contains_rect(rect));
            assert!(rect.is_positive());
        }
        let geometry = json!({"workingRect":d.color_geometry.working.map(|r|[r.left(),r.top(),r.right(),r.bottom()]),"outputRect":d.color_geometry.output.map(|r|[r.left(),r.top(),r.right(),r.bottom()]),"createRect":d.color_geometry.create.map(|r|[r.left(),r.top(),r.right(),r.bottom()])});
        let tokens = crate::theme::Tokens::get(&ctx);
        assert_eq!(h.state().ui.theme, kind);
        assert_eq!(tokens.kind, kind);
        assert_eq!(h.state().session.prefs_value()["interface"]["theme"], json!(kind.id()));
        let name = format!("{}.png", kind.id());
        h.render().unwrap().save(out.join(&name)).unwrap();
        assert_eq!(crate::theme::Tokens::get(&ctx).kind, kind);
        manifest.push(json!({"file":name,"requested":kind.id(),"uiTheme":h.state().ui.theme.id(),"tokenTheme":tokens.kind.id(),"preferenceTheme":h.state().session.prefs_value()["interface"]["theme"],"geometry":geometry}));
    }
    std::fs::write(out.join("theme-manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}
