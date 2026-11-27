use super::*;
use crate::theme::ThemeKind;
use egui_kittest::{Harness, kittest::Queryable};

fn preferences(app: &mut PhotocraftApp) -> Dialog {
    let id = crate::prefs_ui::open_preferences(app, "interface");
    app.ui.dialog_mut(id).unwrap().clone()
}

fn build_with_viewport(monitor: Option<egui::Vec2>, inner: egui::Rect, maximized: bool, scale: f32) -> (egui::ViewportBuilder, egui::ViewportBuilder) {
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let dialog = preferences(&mut app);
    let mut input = egui::RawInput::default();
    let viewport = input.viewports.get_mut(&ViewportId::ROOT).unwrap();
    viewport.monitor_size = monitor;
    viewport.inner_rect = Some(inner);
    viewport.maximized = Some(maximized);
    viewport.native_pixels_per_point = Some(scale);
    let mut result = None;
    let mut output = ctx.run_ui(input, |ui| {
        result = Some((builder(ui.ctx(), &dialog, "Preferences", true), builder(ui.ctx(), &dialog, "Preferences", false)));
    });
    output.textures_delta.clear();
    result.unwrap()
}

#[test]
fn preferences_defaults_use_monitor_room_instead_of_a_small_owner_and_only_open_once_v70() {
    let owner = egui::Rect::from_min_size(egui::pos2(180.0, 160.0), vec2(620.0, 360.0));
    let (first, later) = build_with_viewport(Some(vec2(1920.0, 1080.0)), owner, false, 1.0);
    assert_eq!(first.inner_size, Some(vec2(920.0, 740.0)));
    assert_eq!(first.position, Some(egui::pos2(30.0, 32.0)), "the parent-centred client must not open with an unreachable negative title position");
    assert_eq!(first.window_level, Some(egui::WindowLevel::Normal));
    assert_eq!(first.resizable, Some(true));
    assert_eq!(first.active, Some(true));
    assert!(later.inner_size.is_none() && later.position.is_none() && later.active.is_none(), "subsequent frames must not reset user resizing/restoring");
}

#[test]
fn preferences_size_respects_logical_monitor_scale_and_known_maximized_owner_bounds_v70() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let monitor = vec2(1920.0, 1080.0) / scale;
        let owner = egui::Rect::from_min_size(egui::pos2(-monitor.x, 0.0), monitor - vec2(0.0, crate::work_area::TASKBAR));
        let (first, later) = build_with_viewport(Some(monitor), owner, true, scale);
        let room = monitor - vec2(32.0, crate::work_area::TASKBAR + 64.0);
        let expected = vec2(920.0, 740.0).min(room);
        assert_eq!(first.inner_size, Some(expected), "monitor dimensions already include {scale}x logical scale");
        let rectangle = egui::Rect::from_min_size(first.position.unwrap(), expected);
        assert!(owner.contains_rect(rectangle));
        assert!(first.min_inner_size.unwrap().x <= expected.x && first.min_inner_size.unwrap().y <= expected.y);
        assert!(later.inner_size.is_none() && later.position.is_none());
    }
    // A tiny display must not be forced beyond its monitor by the ordinary
    // 360x200 resize minimum. Unknown monitors keep the existing root fallback.
    let owner = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(320.0, 240.0));
    let (tiny, _) = build_with_viewport(Some(owner.size()), owner, true, 1.0);
    assert!(tiny.inner_size.unwrap().x <= 288.0 && tiny.inner_size.unwrap().y <= 128.0);
    assert!(tiny.min_inner_size.unwrap().x <= tiny.inner_size.unwrap().x);
    let (fallback, _) = build_with_viewport(None, egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 900.0)), false, 1.0);
    assert_eq!(fallback.inner_size, Some(vec2(920.0, 740.0)));
}

struct PreferenceBody {
    app: PhotocraftApp,
    dialog: Dialog,
    fields: serde_json::Map<String, Value>,
    geometry: (egui::Vec2, egui::Vec2, egui::Vec2),
}

impl eframe::App for PreferenceBody {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let language = crate::i18n::Lang::from_pref(
            self.fields.get("values").and_then(|value| value.pointer("/interface/language")).and_then(Value::as_str).unwrap_or("en"),
        );
        let _language = crate::i18n::language_scope(language);
        let mut footer = egui::Panel::bottom("preferences-v70-footer")
            .exact_size(64.0)
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(16.0))
            .show(ui, |ui| ui.new_child(egui::UiBuilder::new().id_salt("preferences-v70-actions").max_rect(ui.max_rect())))
            .inner;
        egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style()).inner_margin(16.0)).show(ui, |ui| {
            let output = form_body_scroll(ui, &mut self.app, &self.dialog, &mut self.fields);
            self.geometry = (output.content_size, output.inner_rect.size(), output.state.offset);
        });
        let _ = super::super::buttons(&mut self.app, &mut footer, &self.dialog, &self.fields);
    }
}

fn body(theme: ThemeKind, language: &str, size: egui::Vec2, category: Option<usize>) -> Harness<'static, PreferenceBody> {
    body_with_gpu(theme, language, size, category, false)
}

fn body_with_gpu(theme: ThemeKind, language: &str, size: egui::Vec2, category: Option<usize>, gpu: bool) -> Harness<'static, PreferenceBody> {
    let language = language.to_string();
    let builder = Harness::builder().with_size(size).with_max_steps(64);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("prefs.set", serde_json::json!({"values":{"interface.theme":theme.id(),"interface.language":language}})).unwrap();
        let mut dialog = preferences(&mut app);
        if let Some(category) = category {
            dialog.fields.insert("__interfacePaletteTab".into(), serde_json::json!(true));
            dialog.fields.insert("__paletteCategory".into(), serde_json::json!(category));
        }
        PreferenceBody { app, fields: dialog.fields.clone(), dialog, geometry: (egui::Vec2::ZERO, egui::Vec2::ZERO, egui::Vec2::ZERO) }
    })
}

#[test]
#[ignore = "writes five-theme actual Preferences plus small/resized GPU captures for root review"]
fn preferences_gpu_visual_v70() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v70/preferences-gpu");
    assert!(!out.exists(), "preserve previous visual evidence");
    std::fs::create_dir_all(&out).unwrap();
    for theme in ThemeKind::ALL {
        for (category, name) in [(None, "interface"), (Some(0), "module-colors")] {
            let mut h = body_with_gpu(theme, "zh-hans", vec2(920.0, 740.0), category, true);
            h.run_steps(10);
            let (content, viewport, offset) = h.state().geometry;
            assert!(content.x <= viewport.x + 1.0 && content.y <= viewport.y + 1.0);
            assert_eq!(offset, egui::Vec2::ZERO);
            h.render().unwrap().save(out.join(format!("{}-{name}.png", theme.id()))).unwrap();
        }
    }
    let mut h = body_with_gpu(ThemeKind::StudioLight, "zh-hans", vec2(920.0, 740.0), Some(6), true);
    h.run_steps(10);
    h.render().unwrap().save(out.join("studioLight-progress-colors.png")).unwrap();
    h.set_size(vec2(600.0, 420.0));
    h.run_steps(10);
    h.render().unwrap().save(out.join("small-needed-scrollbars.png")).unwrap();
    h.set_size(vec2(920.0, 420.0));
    h.run_steps(10);
    let (content, viewport, _) = h.state().geometry;
    assert!(content.x <= viewport.x + 1.0 && content.y > viewport.y + 1.0, "a short but wide form needs only vertical scrolling");
    h.set_size(vec2(920.0, 740.0));
    h.run_steps(10);
    let (content, viewport, offset) = h.state().geometry;
    assert!(content.x <= viewport.x + 1.0 && content.y <= viewport.y + 1.0);
    assert_eq!(offset, egui::Vec2::ZERO);
    h.render().unwrap().save(out.join("restored-no-unnecessary-scrollbars.png")).unwrap();
}

fn text_inside_clip(shapes: &[egui::epaint::ClippedShape], wanted: &str) -> bool {
    fn contains(shape: &egui::Shape, clip: egui::Rect, wanted: &str) -> bool {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text == wanted => clip.expand(0.51).contains_rect(text.galley.mesh_bounds.translate(text.pos.to_vec2())),
            egui::Shape::Vec(children) => children.iter().any(|shape| contains(shape, clip, wanted)),
            _ => false,
        }
    }
    shapes.iter().any(|shape| contains(&shape.shape, shape.clip_rect, wanted))
}

#[test]
fn actual_preferences_form_fits_default_size_and_last_navigation_item_in_all_themes_v70() {
    for theme in ThemeKind::ALL {
        for language in ["en", "zh-hans"] {
            for category in [None, Some(0), Some(6)] {
                let mut h = body(theme, language, vec2(920.0, 740.0), category);
                h.run_steps(10);
                let (content, viewport, offset) = h.state().geometry;
                assert!(content.x <= viewport.x + 1.0, "{theme:?} {language} {category:?}: unnecessary horizontal overflow {content:?}/{viewport:?}");
                assert!(content.y <= viewport.y + 1.0, "{theme:?} {language} {category:?}: unnecessary vertical overflow {content:?}/{viewport:?}");
                assert_eq!(offset, egui::Vec2::ZERO);
                let lang = crate::i18n::Lang::from_code(language).unwrap();
                assert!(
                    text_inside_clip(&h.output().shapes, crate::i18n::tr(lang, "Integrations")),
                    "the last actual navigation label must paint inside its clip"
                );
                for label in ["OK", "Cancel", "Apply"] {
                    let node = h.get_by_label(crate::i18n::tr(lang, label));
                    assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(920.0, 740.0)).contains_rect(node.rect()));
                }
            }
        }
    }
}

#[test]
fn preferences_small_window_keeps_needed_scroll_and_footer_then_resize_removes_both_bars_v70() {
    let mut h = body(ThemeKind::StudioLight, "en", vec2(600.0, 420.0), Some(6));
    h.run_steps(10);
    let (content, viewport, _) = h.state().geometry;
    assert!(content.x > viewport.x + 1.0 && content.y > viewport.y + 1.0, "a deliberately small form must keep real overflow scrollable");
    let small = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(600.0, 420.0));
    for label in ["OK", "Cancel", "Apply"] {
        assert!(small.contains_rect(h.get_by_label(label).rect()), "fixed footer must remain reachable at small size");
    }
    h.set_size(vec2(920.0, 740.0));
    h.run_steps(10);
    let (content, viewport, offset) = h.state().geometry;
    assert!(
        content.x <= viewport.x + 1.0 && content.y <= viewport.y + 1.0,
        "scrollbars must disappear after actual content fits, without a width feedback loop"
    );
    assert_eq!(offset, egui::Vec2::ZERO);
}
