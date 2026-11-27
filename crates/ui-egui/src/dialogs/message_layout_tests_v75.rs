use super::*;
use crate::theme::ThemeKind;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

struct Fixture {
    app: PhotocraftApp,
    theme: ThemeKind,
    language: crate::i18n::Lang,
    geometry: (egui::Vec2, egui::Vec2),
    initial: egui::ViewportBuilder,
    later: egui::ViewportBuilder,
}

impl eframe::App for Fixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        assert_eq!(self.app.ui.theme, self.theme);
        assert_eq!(crate::theme::Tokens::get(ui.ctx()).kind, self.theme);
        assert_eq!(self.app.session.prefs_value()["interface"]["theme"], json!(self.theme.id()));
        let _language = crate::i18n::language_scope(self.language);
        let dialog = self.app.ui.dialogs.last().unwrap().clone();
        self.initial = builder(ui.ctx(), &dialog, "fixture", true);
        self.later = builder(ui.ctx(), &dialog, "fixture", false);
        let mut fields = dialog.fields.clone();
        let mut footer = egui::Panel::bottom("message-layout-footer-v75")
            .exact_size(super::super::footer_height(&dialog, ui.available_width()))
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(16.0))
            .show(ui, |ui| ui.new_child(egui::UiBuilder::new().id_salt("actions").max_rect(ui.max_rect())))
            .inner;
        egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style()).inner_margin(16.0)).show(ui, |ui| {
            let output = form_body_scroll(ui, &mut self.app, &dialog, &mut fields);
            self.geometry = (output.content_size, output.inner_rect.size());
        });
        super::super::buttons(&mut self.app, &mut footer, &dialog, &fields);
        self.app.ui.dialog_mut(dialog.id).unwrap().fields = fields;
    }
}

fn harness(theme: ThemeKind, language: &str, kind: &str, gpu: bool) -> Harness<'static, Fixture> {
    let language = crate::i18n::Lang::from_code(language).unwrap();
    let kind = kind.to_string();
    let builder = Harness::builder().with_size(vec2(920.0, 740.0)).with_max_steps(64);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("prefs.set", json!({"values":{"interface.theme":theme.id(),"interface.language":language.code()}})).unwrap();
        app.set_theme(&cc.egui_ctx, theme);
        match kind.as_str() {
            "shortcuts" | "menus" | "toolbar" => {
                crate::prefs_ui::open_shortcuts(
                    &mut app,
                    if kind == "menus" {
                        1
                    } else if kind == "toolbar" {
                        2
                    } else {
                        0
                    },
                );
            }
            "warning" => {
                app.ui.open_dialog(
                    DialogKind::Command,
                    serde_json::from_value(json!({
                        "__rasterize":"smartObject", "__label":"Rasterize Smart Object",
                        "message":crate::rasterize_prompt::Kind::SmartObject.message()
                    }))
                    .unwrap(),
                );
            }
            "long" => {
                app.ui.open_dialog(
                    DialogKind::Error,
                    serde_json::from_value(json!({"message":"A longer error message with useful details. ".repeat(80)})).unwrap(),
                );
            }
            _ => {
                app.ui.open_dialog(DialogKind::Error, serde_json::from_value(json!({"message":"Error"})).unwrap());
            }
        }
        Fixture { app, theme, language, geometry: Default::default(), initial: Default::default(), later: Default::default() }
    })
}

fn text_x(shapes: &[egui::epaint::ClippedShape], wanted: &str) -> Option<f32> {
    fn find(shape: &egui::Shape, wanted: &str) -> Option<f32> {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text == wanted => Some(text.pos.x),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, wanted)),
            _ => None,
        }
    }
    shapes.iter().find_map(|shape| find(&shape.shape, wanted))
}

#[test]
fn message_windows_follow_text_and_keep_buttons_visible_without_resetting_geometry_v75() {
    for theme in ThemeKind::ALL {
        for language in ["en", "zh-hans", "zh-hant"] {
            for kind in ["error", "warning"] {
                let mut h = harness(theme, language, kind, false);
                h.run_steps(6);
                let size = h.state().initial.inner_size.unwrap();
                assert!(size.x <= 560.0 && size.y < 200.0, "{theme:?}/{language}/{kind}: {size:?}");
                h.set_size(size);
                h.run_steps(8);
                let (content, visible) = h.state().geometry;
                assert!(content.x <= visible.x + 1.0 && content.y <= visible.y + 1.0, "{theme:?}/{language}/{kind}: {content:?} / {visible:?}");
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                assert!(screen.contains_rect(h.get_by_label(crate::i18n::tr(h.state().language, "OK")).rect()));
                assert!(h.state().later.inner_size.is_none() && h.state().later.position.is_none());
            }
        }
    }
    let mut long = harness(ThemeKind::StudioLight, "en", "long", false);
    long.run_steps(6);
    let size = long.state().initial.inner_size.unwrap();
    assert!(size.y > 200.0 && size.y <= 670.0 && size.x <= 560.0, "long messages remain bounded: {size:?}");
}

#[test]
fn setting_list_names_share_the_left_edge_in_all_three_tabs_v75() {
    for theme in ThemeKind::ALL {
        for language in ["en", "zh-hans"] {
            for kind in ["shortcuts", "menus", "toolbar"] {
                let mut h = harness(theme, language, kind, false);
                h.run_steps(8);
                let lang = h.state().language;
                let names: Vec<_> = crate::prefs_ui::shortcut_items(&h.state().app)
                    .into_iter()
                    .filter(
                        |(id, _, path, _)| {
                            if kind == "toolbar" { id.starts_with("tools.select.") } else { path.len() == 1 && path.first().is_some_and(|p| p == "File") }
                        },
                    )
                    .take(3)
                    .map(|(id, label, _, _)| crate::i18n::tr_id(lang, &id, &label).to_string())
                    .collect();
                assert_eq!(names.len(), 3);
                let positions: Vec<_> = names
                    .iter()
                    .map(|name| text_x(&h.output().shapes, name).unwrap_or_else(|| panic!("missing {name}: {theme:?}/{language}/{kind}")))
                    .collect();
                assert!(positions.iter().all(|x| (*x - positions[0]).abs() <= 0.6), "{theme:?}/{language}/{kind}: {positions:?}");
                assert!(positions[0] < 52.0, "names must remain next to the left edge: {positions:?}");
            }
        }
    }
}

#[test]
#[ignore = "writes message and left-aligned settings GPU images for root review"]
fn dialog_message_and_settings_gpu_visual_v75() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v75/dialog-layout-gpu");
    assert!(!out.exists(), "preserve previous evidence");
    std::fs::create_dir_all(&out).unwrap();
    for theme in ThemeKind::ALL {
        for kind in ["warning", "shortcuts", "menus", "toolbar"] {
            let mut h = harness(theme, "zh-hans", kind, true);
            h.run_steps(8);
            if kind == "warning" {
                let size = h.state().initial.inner_size.unwrap();
                h.set_size(size);
                h.run_steps(8);
            }
            h.render().unwrap().save(out.join(format!("{}-{kind}.png", theme.id()))).unwrap();
        }
    }
}
