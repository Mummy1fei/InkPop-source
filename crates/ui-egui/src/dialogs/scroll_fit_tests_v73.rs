use super::*;
use crate::theme::ThemeKind;
use egui_kittest::{Harness, kittest::Queryable};

struct Fixture {
    app: PhotocraftApp,
    geometry: (egui::Vec2, egui::Vec2, egui::Vec2),
}

impl eframe::App for Fixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let _language = crate::i18n::language_scope(crate::i18n::Lang::from_code("zh-hans").unwrap());
        let Some(dialog) = self.app.ui.dialogs.last().cloned() else { return };
        let mut fields = dialog.fields.clone();
        let mut footer = egui::Panel::bottom("scroll-fit-footer-v73")
            .exact_size(super::super::footer_height(&dialog, ui.available_width()))
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(16.0))
            .show(ui, |ui| ui.new_child(egui::UiBuilder::new().id_salt("fit-actions").max_rect(ui.max_rect())))
            .inner;
        egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style()).inner_margin(16.0)).show(ui, |ui| {
            let out = form_body_scroll(ui, &mut self.app, &dialog, &mut fields);
            self.geometry = (out.content_size, out.inner_rect.size(), out.state.offset);
        });
        let (outcome, apply) = super::super::buttons(&mut self.app, &mut footer, &dialog, &fields);
        super::super::finish(&mut self.app, ui.ctx(), &dialog, fields, outcome, apply);
    }
}

fn harness(theme: ThemeKind, kind: &str, size: egui::Vec2, gpu: bool) -> Harness<'static, Fixture> {
    let kind = kind.to_string();
    let builder = Harness::builder().with_size(size).with_max_steps(64);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", serde_json::json!({"width":32,"height":32})).unwrap();
        match kind.as_str() {
            "about" | "credits" => {
                app.ui.open_dialog(
                    DialogKind::About,
                    serde_json::Map::from_iter([("tab".into(), Value::String(if kind == "credits" { "credits" } else { "about" }.into()))]),
                );
            }
            "preferences" => {
                crate::prefs_ui::open_preferences(&mut app, "interface");
            }
            "shortcuts" | "menus" | "toolbar" | "filtered" => {
                let tab = if kind == "menus" {
                    1
                } else if kind == "toolbar" {
                    2
                } else {
                    0
                };
                let id = crate::prefs_ui::open_shortcuts(&mut app, tab);
                if kind == "filtered" {
                    app.ui.dialog_mut(id).unwrap().fields.insert("filter".into(), Value::String("file.new".into()));
                }
            }
            "new" => {
                app.ui.open_dialog(DialogKind::NewDocument, crate::state::UiState::new_document_fields());
            }
            "color" => {
                crate::color_picker_ui::open(&mut app, "foreground");
            }
            "fill" => {
                crate::fill_ui::open(&mut app);
            }
            "layer-style" => {
                crate::layer_style::open(&mut app, None).unwrap();
            }
            _ => panic!("unknown synthetic fixture"),
        }
        Fixture { app, geometry: Default::default() }
    })
}

/// Use the same preferred client size as the production native viewport builder.
/// The temporary harness size is replaced before the measured settling frames.
fn default_harness(theme: ThemeKind, kind: &str, gpu: bool) -> Harness<'static, Fixture> {
    let mut h = harness(theme, kind, vec2(1100.0, 900.0), gpu);
    let dialog = h.state().app.ui.dialogs.last().unwrap();
    let size = initial_size(&h.ctx, dialog);
    h.set_size(size);
    h
}

#[test]
fn native_forms_do_not_grow_outer_scrollbars_when_their_content_fits_v73() {
    for theme in ThemeKind::ALL {
        for kind in ["about", "credits", "preferences", "shortcuts", "menus", "toolbar", "filtered", "new", "color", "fill", "layer-style"] {
            let mut h = default_harness(theme, kind, false);
            h.run_steps(8);
            let (content, visible, offset) = h.state().geometry;
            assert!(content.x <= visible.x + 1.0 && content.y <= visible.y + 1.0, "{theme:?}/{kind}: unnecessary outer overflow: {content:?} vs {visible:?}");
            assert_eq!(offset, egui::Vec2::ZERO, "{theme:?}/{kind}");
        }
    }
}

#[test]
fn about_and_shortcut_default_geometry_uses_screen_room_and_never_resets_on_repaint_v73() {
    for (kind, preferred) in [("about", vec2(1072.0, 860.0)), ("shortcuts", vec2(920.0, 740.0))] {
        let h = default_harness(ThemeKind::StudioLight, kind, false);
        let dialog = h.state().app.ui.dialogs.last().unwrap();
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        let root = input.viewports.get_mut(&ViewportId::ROOT).unwrap();
        root.inner_rect = Some(egui::Rect::from_min_size(egui::pos2(100.0, 100.0), vec2(620.0, 360.0)));
        root.monitor_size = Some(vec2(1920.0, 1080.0));
        let mut output = ctx.run_ui(input, |ui| {
            assert_eq!(builder(ui.ctx(), dialog, "fixture", true).inner_size, Some(preferred + form_scrollbar_gutter(ui.ctx(), dialog)));
            let subsequent = builder(ui.ctx(), dialog, "fixture", false);
            assert!(subsequent.inner_size.is_none() && subsequent.position.is_none());
        });
        output.textures_delta.clear();
    }
}

#[test]
#[ignore = "writes About and representative dialog scroll-fit screenshots for root review"]
fn about_scroll_fit_gpu_visual_v73() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v73/about-gpu");
    assert!(!out.exists(), "preserve previous images");
    std::fs::create_dir_all(&out).unwrap();
    for theme in ThemeKind::ALL {
        let mut h = default_harness(theme, "about", true);
        h.run_steps(8);
        h.render().unwrap().save(out.join(format!("{}-about.png", theme.id()))).unwrap();
    }
    for kind in ["credits", "preferences", "shortcuts", "menus", "toolbar", "filtered", "new", "color", "fill", "layer-style"] {
        let mut h = default_harness(ThemeKind::StudioLight, kind, true);
        h.run_steps(8);
        h.render().unwrap().save(out.join(format!("{kind}.png"))).unwrap();
    }
    let mut h = harness(ThemeKind::StudioLight, "about", vec2(600.0, 560.0), true);
    h.run_steps(8);
    h.get_by_label("第三方素材与来源索引（ATTRIBUTION.md）").scroll_to_me();
    h.run_steps(5);
    h.render().unwrap().save(out.join("small-about-scrolled.png")).unwrap();
}
