use super::*;
use egui::{Event, Key, Modifiers, Rect, Vec2, pos2, vec2};
use photocraft_doc::LayerContent;

const SMART: &str = "layer.smartObjects.convertToSmartObject";

fn app() -> crate::PhotocraftApp {
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":16,"height":16})).unwrap();
    app.run("layer.new.layer", json!({"name":"Child"})).unwrap();
    app.run("edit.fill", json!({"color":"#6b8fc9"})).unwrap();
    app.run("layer.groupLayers", json!({"name":"Group"})).unwrap();
    app.run("edit.keyboardShortcuts", json!({"set":{SMART:"S"}})).unwrap();
    app
}

#[test]
fn native_layer_menu_collects_bound_s_for_root_execution_on_a_group_v70() {
    let ctx = egui::Context::default();
    crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
    let mut app = app();
    let source = egui::ViewportId::from_hash_of("layer-context-shortcut-v70");
    let mut actions = Vec::new();
    let active = app.session.active().unwrap();
    let group = active.doc.layer(active.active_layer.unwrap()).unwrap().clone();
    let mut input = egui::RawInput {
        viewport_id: source,
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(430.0, 1000.0))),
        events: vec![Event::Key { key: Key::S, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }],
        ..Default::default()
    };
    input.viewports.entry(source).or_default();
    ctx.run_ui(input, |ui| {
        contents(&app, ui, &group, false, &mut actions, true);
        assert!(!ui.input(|input| input.events.iter().any(|event| matches!(event, Event::Key { key: Key::S, pressed: true, .. }))));
    })
    .textures_delta
    .clear();
    assert_eq!(actions, [("layer.select".into(), json!({"layer":group.id.0})), (SMART.into(), Value::Null)]);
    assert!(
        matches!(app.session.active().unwrap().doc.layer(group.id).unwrap().content, LayerContent::Group(_)),
        "child collection never mutates the document"
    );
    ctx.run_ui(Default::default(), |ui| {
        assert_eq!(ui.ctx().viewport_id(), egui::ViewportId::ROOT);
        for (id, params) in &actions {
            if params.is_null() {
                crate::menus::invoke(&mut app, ui.ctx(), id, json!({})).unwrap();
            } else {
                app.run(id, params.clone()).unwrap();
            }
        }
    })
    .textures_delta
    .clear();
    let active = app.session.active().unwrap();
    assert!(matches!(active.doc.layer(active.active_layer.unwrap()).unwrap().content, LayerContent::Smart(_)));
}

fn contains_text(shapes: &[egui::epaint::ClippedShape], expected: &str) -> bool {
    fn has(shape: &egui::Shape, expected: &str) -> bool {
        match shape {
            egui::Shape::Text(text) => text.galley.job.text == expected,
            egui::Shape::Vec(children) => children.iter().any(|shape| has(shape, expected)),
            _ => false,
        }
    }
    shapes.iter().any(|shape| has(&shape.shape, expected))
}

#[test]
fn layer_menu_paints_custom_shortcut_and_invalidates_native_width_cache_v70() {
    for native in [false, true] {
        let ctx = egui::Context::default();
        crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
        let mut app = app();
        let group = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().clone();
        let list = vec![Some(("Convert to Smart Object", SMART))];
        let mut previous = None;
        for binding in ["S", "Cmd+Alt+Shift+F12", ""] {
            app.run("edit.keyboardShortcuts", json!({"set":{SMART:binding}})).unwrap();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                let layout = native_layout_with_shortcuts(ui.ctx(), list.clone(), command_shortcuts(&app, &list));
                if let Some(old) = previous.take() {
                    assert!(!std::sync::Arc::ptr_eq(&layout.entries, &old), "binding changes retire cached host widths");
                }
                previous = Some(layout.entries.clone());
                contents(&app, ui, &group, true, &mut Vec::new(), native);
            });
            output.textures_delta.clear();
            if binding.is_empty() {
                assert!(!contains_text(&output.shapes, "S"));
            } else {
                assert!(contains_text(&output.shapes, &crate::shortcuts::pretty(binding)), "menu must paint effective binding {binding}");
            }
        }
    }
}

#[test]
#[ignore = "writes actual layer context contents in Pro and Studio Light warm-gray-orange for root visual review"]
fn smart_object_shortcut_gpu_visual_v70() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v70/shortcut-menu-gpu");
    assert!(!out.exists(), "preserve previous visual evidence");
    std::fs::create_dir_all(&out).unwrap();
    for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::StudioLight] {
        let mut app = app();
        app.ui.theme = theme;
        if theme == crate::theme::ThemeKind::StudioLight {
            let colors = photocraft_engine::prefs::palette::builtin::BUILTIN_PALETTES[0].colors();
            app.session.prefs.edit(|prefs| prefs.interface.studio_light_colors.colors = colors);
        }
        let active = app.session.active().unwrap();
        let layer = active.doc.layer(active.active_layer.unwrap()).unwrap().clone();
        let mut initialized = false;
        let mut harness = egui_kittest::Harness::builder().with_size(Vec2::new(450.0, 940.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_ui_state(
            move |ui, actions| {
                let _language = crate::i18n::language_scope(crate::i18n::Lang::from_pref("zh-hans"));
                if !initialized {
                    crate::PhotocraftApp::setup_context(ui.ctx(), theme);
                    initialized = true;
                }
                crate::theme::palette::apply(ui.ctx(), theme, &app.session.prefs().interface.studio_light_colors);
                crate::theme::palette::menu_scope(ui);
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    contents(&app, ui, &layer, true, actions, true);
                });
            },
            Vec::<(String, Value)>::new(),
        );
        harness.run_steps(8);
        harness.render().unwrap().save(out.join(format!("{}-smart-object-S.png", theme.id()))).unwrap();
    }
}
