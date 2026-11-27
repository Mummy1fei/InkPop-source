use super::*;
use crate::theme::ThemeKind;

fn cube() -> String {
    photocraft_cms::lutfile::write_cube(&photocraft_cms::lutfile::LutFile::from_fn("Invert", 2, |rgb| rgb.map(|v| 1.0 - v)))
}
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":64,"height":48,"depth":32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color":"#306090"})).unwrap();
    app
}

#[test]
fn raw_lut_v75_import_formats_domains_adaptation_and_rejection() {
    let (one_dimensional, _) = lut::parse("float-1d.cube", b"LUT_1D_SIZE 2\n0 -1 0\n2 1 2\n").unwrap();
    for (actual, expected) in one_dimensional.rgb([0.75, 0.25, 0.75]).into_iter().zip([1.5, -0.5, 1.5]) {
        assert!((actual - expected).abs() < 1e-6);
    }
    let (with_domain, _) = lut::parse("float-domain.cube", b"DOMAIN_MAX 2 2 2\nLUT_1D_SIZE 2\n0 0 0\n2 2 2\n").unwrap();
    assert_eq!(with_domain.rgb([1.5; 3]), [1.5; 3]);
    assert!(lut::parse("combined.cube", b"LUT_1D_SIZE 2\nLUT_3D_SIZE 2\n").is_err());
    let (lut, adapted) = lut::parse("C:/private-folder/invert.cube", cube().as_bytes()).unwrap();
    assert_eq!(lut.name, "invert.cube");
    assert!(!adapted);
    for (a, b) in lut.rgb([0.2, 0.3, 0.8]).into_iter().zip([0.8, 0.7, 0.2]) {
        assert!((a - b).abs() < 1e-6);
    }
    let (domain, _) = lut::parse("domain.cube", format!("DOMAIN_MAX 2 2 2\n{}", cube()).as_bytes()).unwrap();
    assert_eq!(domain.rgb([1.0; 3]), [0.5; 3]);
    let large = photocraft_cms::lutfile::LutFile::from_fn("Large", 65, |rgb| rgb.map(|v| v * 0.8 + 0.1));
    let (lut, adapted) = lut::parse("large.cube", photocraft_cms::lutfile::write_cube(&large).as_bytes()).unwrap();
    assert!(adapted);
    assert_eq!(lut.size, 33);
    for (a, b) in lut.rgb([0.2, 0.3, 0.8]).into_iter().zip([0.26, 0.34, 0.74]) {
        assert!((a - b).abs() < 2e-6);
    }
    let identity = photocraft_cms::lutfile::LutFile::identity(2);
    let hex: String = identity.data.iter().flat_map(|v| v.to_le_bytes()).map(|b| format!("{b:02X}")).collect();
    assert_eq!(lut::parse("test.look", format!("<look><size>2</size><data>{hex}</data></look>").as_bytes()).unwrap().0.size, 2);
    let mut three = String::from("0 1023\n");
    for r in 0..2 {
        for g in 0..2 {
            for b in 0..2 {
                three.push_str(&format!("{} {} {}\n", r * 1023, g * 1023, b * 1023));
            }
        }
    }
    assert_eq!(lut::parse("test.3dl", three.as_bytes()).unwrap().0.values.as_ref(), &identity.data);
    assert!(lut::parse("invalid.cube", b"bad").is_err());
    assert!(lut::parse("invalid.cube", format!("DOMAIN_MAX -1 1 1\n{}", cube()).as_bytes()).is_err());
}

#[test]
fn raw_lut_v75_services_preview_commit_preset_and_cache_are_consistent() {
    let mut app = app();
    let ctx = egui::Context::default();
    open(&mut app, &ctx).unwrap();
    assert!(lut::execute(&mut app, &ctx, None).unwrap_err().contains("unavailable"));
    app.services.pick_lut = Some(Box::new(|| None));
    assert_eq!(lut::execute(&mut app, &ctx, None).unwrap()["cancelled"], true);
    app.services.pick_lut = Some(Box::new(|| Some(("test.cube".into(), Ok(cube().into_bytes())))));
    lut::execute(&mut app, &ctx, None).unwrap();
    let d = app.camera_raw.as_mut().unwrap();
    d.render(&ctx);
    let preview = d.processed[0];
    let source = d.proxy.clone();
    let mut p = d.params.clone();
    let first = local_cache::render(&source, d.pw, d.ph, &p, true, None);
    p.local_adjustments.push(photocraft_algo::camera_raw_masks::LocalAdjustment { exposure: 0.4, ..Default::default() });
    let second = local_cache::render(&source, d.pw, d.ph, &p, true, first.global.as_ref());
    let mut exact = source.to_vec();
    photocraft_algo::camera_raw::develop(&mut exact, d.pw, d.ph, &p, true);
    assert_eq!(second.pixels, exact);
    p.color_lookup.amount = 20.0;
    let changed = local_cache::render(&source, d.pw, d.ph, &p, true, second.global.as_ref());
    assert!(!changed.hit);
    let saved = d.params.clone();
    presets::execute(&mut app, &ctx, presets::Action::Save("LUT".into())).unwrap();
    let exported = presets::execute(&mut app, &ctx, presets::Action::ExportData("LUT".into())).unwrap();
    assert_eq!(exported["params"]["colorLookup"], json!(saved.color_lookup));
    let previous = app.camera_raw.as_ref().unwrap().params.clone();
    assert!(menu(&mut app, &ctx, "filter.cameraRaw", &json!({"ui":{"lut":{"fileName":"x.cube","data":"invalid"}}})).unwrap().is_err());
    assert_eq!(app.camera_raw.as_ref().unwrap().params, previous);
    commit(&mut app).unwrap();
    let st = app.session.active().unwrap();
    let actual = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(0, 0);
    for (a, b) in preview.iter().zip(actual) {
        assert!((a - b).abs() < 2e-6, "{preview:?} {actual:?}");
    }
}

#[test]
#[ignore = "writes five production Camera Raw LUT screenshots for root visual review"]
fn raw_lut_gpu_visual_v75() {
    use egui_kittest::kittest::Queryable;
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v75/raw-lut-gpu");
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
        menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui":{"lut":{"fileName":"创意反相.cube","data":cube()}}})).unwrap().unwrap();
        h.state_mut().camera_raw.as_mut().unwrap().render(&ctx);
        h.run_steps(8);
        h.get_by_label("色彩空间").click();
        h.run_steps(16);
        let tokens = crate::theme::Tokens::get(&ctx);
        assert_eq!(h.state().ui.theme, kind);
        assert_eq!(tokens.kind, kind);
        assert_eq!(h.state().session.prefs_value()["interface"]["theme"], json!(kind.id()));
        let name = format!("{}.png", kind.id());
        h.render().unwrap().save(out.join(&name)).unwrap();
        assert_eq!(crate::theme::Tokens::get(&ctx).kind, kind);
        manifest.push(json!({"file":name,"requested":kind.id(),"uiTheme":h.state().ui.theme.id(),"tokenTheme":tokens.kind.id(),"preferenceTheme":h.state().session.prefs_value()["interface"]["theme"],"chromeRgba":tokens.chrome.to_array()}));
    }
    std::fs::write(out.join("theme-manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}
