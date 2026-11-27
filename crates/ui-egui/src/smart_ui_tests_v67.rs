use super::*;

fn filter(command: &str, params: Value) -> photocraft_doc::SmartFilter {
    photocraft_doc::SmartFilter { command: command.into(), params, blend: photocraft_color::BlendMode::Normal, opacity: 1.0, visible: true }
}

struct Fixture {
    app: PhotocraftApp,
    layer: Layer,
}

impl eframe::App for Fixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| filter_rows(&mut self.app, ui, &self.layer, 0, &mut Vec::new()));
    }
}

fn text(shape: &egui::Shape, output: &mut Vec<String>) {
    match shape {
        egui::Shape::Text(shape) => output.push(shape.galley.job.text.clone()),
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| text(shape, output)),
        _ => {}
    }
}

#[test]
fn smart_filter_rows_translate_live_without_renaming_imported_filter_names() {
    for code in ["en", "zh-hans", "zh-hant"] {
        let lang = crate::i18n::Lang::from_code(code).unwrap();
        crate::i18n::with_language(lang, || {
            let mut h = egui_kittest::Harness::builder().with_size(vec2(700.0, 250.0)).build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                let mut smart = SmartObject::new(
                    photocraft_doc::SmartSource::Embedded { file_name: "User source.png".into(), bytes: std::sync::Arc::new(Vec::new()) },
                    photocraft_geom::Affine::IDENTITY,
                    None,
                );
                smart.smart_filters = vec![
                    filter(photocraft_engine::lens_cmds::RAW, json!({})),
                    filter(photocraft_engine::smart_cmds::UNSUPPORTED_FILTER, json!({"name":"Background"})),
                ];
                Fixture {
                    app: PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default()),
                    layer: Layer::new("Background 用户图层", LayerContent::Smart(smart)),
                }
            });
            h.run_steps(4);
            let mut drawn = Vec::new();
            for shape in &h.output().shapes {
                text(&shape.shape, &mut drawn);
            }
            assert!(drawn.iter().any(|s| s == crate::i18n::tr(lang, "Smart Filters")), "{code}: {drawn:?}");
            let expected = crate::i18n::fmt(crate::i18n::tr(lang, "{name} (kept, not editable)"), &[("name", "Background")]);
            assert!(drawn.contains(&expected), "{code}: imported filter name must be retained while its suffix is translated: {drawn:?}");
            if code != "en" {
                assert!(!drawn.iter().any(|s| s == "Smart Filters" || s.ends_with("(kept, not editable)")), "{code}: {drawn:?}");
            }
            assert_eq!(h.state().layer.name, "Background 用户图层");
        });
    }
}
