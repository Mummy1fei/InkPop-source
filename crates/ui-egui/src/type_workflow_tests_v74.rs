//! Empty type insertion, all-theme options and the production shell's text workflow.

use super::*;
use crate::state::Tool;
use crate::theme::ThemeKind;
use egui::{Shape, vec2};
use egui_kittest::Harness;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 640, "height": 480})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Type;
    app.ui.tool_options.type_size = 36.0;
    app
}

fn raw_text(app: &PhotocraftApp, id: u64) -> &TextLayer {
    text_layer(&app.session.active().unwrap().doc, LayerId(id)).unwrap()
}

#[test]
fn empty_point_and_paragraph_start_at_zero_with_real_font_caret_v74() {
    for tool in [Tool::Type, Tool::VerticalType] {
        for end in [[100.0, 160.0], [360.0, 320.0]] {
            let mut app = app();
            app.ui.tool = tool;
            app.ui.tool_options.type_size = 72.0;
            let before = app.session.active().unwrap().history.past_len();
            assert!(!pointer_down(&mut app, 100.0, 160.0, false));
            pointer_up(&mut app, [100.0, 160.0], end);
            let ed = app.ui.text_edit.clone().unwrap();
            assert_eq!((ed.anchor, ed.caret), (0, 0));
            assert!(ed.created);
            assert!(raw_text(&app, ed.layer).text.is_empty());
            assert!(raw_text(&app, ed.layer).cache.as_ref().unwrap().content_bounds().is_empty());
            let (layout, _, _) = layout(&mut app, LayerId(ed.layer)).unwrap();
            assert!(layout.glyphs.is_empty(), "no placeholder glyphs are rendered");
            let (_, top, bottom) = caret_geometry(&app, LayerId(ed.layer), &layout, 0);
            assert!(bottom - top >= 72.0 * layout.px_per_pt * 0.6, "caret follows 72pt font metrics: {top}..{bottom}");
            assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
            insert(&mut app, "Hello 世界");
            assert_eq!(raw_text(&app, ed.layer).text, "Hello 世界");
            assert_eq!(app.session.active().unwrap().history.past_len(), before + 1, "creation and typing form one transaction");
            commit(&mut app);
            app.run("edit.undo", json!({})).unwrap();
            assert!(app.session.active().unwrap().doc.layer(LayerId(ed.layer)).is_none());
        }
    }
}

#[test]
fn finishing_or_canceling_new_blank_text_leaves_no_layer_or_extra_delete_step_v74() {
    for cancel_edit in [false, true] {
        let mut app = app();
        let count = app.session.active().unwrap().doc.layers.len();
        let before = app.session.active().unwrap().history.past_len();
        pointer_up(&mut app, [100.0, 160.0], [100.0, 160.0]);
        let id = app.ui.text_edit.as_ref().unwrap().layer;
        if cancel_edit {
            cancel(&mut app)
        } else {
            commit(&mut app)
        }
        assert!(app.ui.text_edit.is_none());
        assert!(app.session.active().unwrap().doc.layer(LayerId(id)).is_none());
        assert_eq!(app.session.active().unwrap().doc.layers.len(), count);
        assert_eq!(app.session.active().unwrap().history.past_len(), before, "no standalone Delete operation follows an empty insertion");
    }
}

fn shell(kind: ThemeKind, size: egui::Vec2, gpu: bool, empty: bool, vertical: bool) -> Harness<'static, PhotocraftApp> {
    let builder = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).with_max_steps(64);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    let mut h = builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, kind);
        let mut app = app();
        // The production shell's first preferences load is authoritative. Seed it
        // before configuring the UI, otherwise first-frame tick restores Pro.
        app.run("prefs.set", json!({"path":"interface.theme","value":kind.id()})).unwrap();
        crate::prefs_ui::load(&mut app);
        app.custom_titlebar = true;
        app.set_theme(&cc.egui_ctx, kind);
        app.ui.tool = if vertical { Tool::VerticalType } else { Tool::Type };
        app.ui.panels.properties = true;
        app.ui.panels.layers = true;
        // Give Properties the real right-dock surface, with both panels expanded.
        let (request, _reply) = crate::control::ControlRequest::new("ui.set", json!({"dock":{"collapsed":["color","history","navigator"]}}));
        crate::control::handle(&mut app, &cc.egui_ctx, &request);
        pointer_up(&mut app, [160.0, 180.0], [160.0, 180.0]);
        if !empty {
            insert(&mut app, "InkPop\nEditable text");
        }
        app
    });
    h.run_steps(8);
    assert_shell_theme(&h, kind);
    h
}

fn assert_shell_theme(h: &Harness<'_, PhotocraftApp>, kind: ThemeKind) -> serde_json::Value {
    let tokens = crate::theme::Tokens::get(&h.ctx);
    let pref = h.state().session.prefs_value()["interface"]["theme"].clone();
    assert_eq!(h.state().ui.theme, kind, "production shell retained requested UI theme");
    assert_eq!(tokens.kind, kind, "paint tokens match the requested screenshot theme");
    assert_eq!(pref, json!(kind.id()), "first-frame preference load must not substitute another theme");
    json!({"requested":kind.id(),"uiTheme":h.state().ui.theme.id(),"tokenTheme":tokens.kind.id(),"preferenceTheme":pref,
        "chromeRgba":tokens.chrome.to_array(),"cardRgba":tokens.card.to_array()})
}

fn drawn_text(h: &Harness<'_, PhotocraftApp>) -> Vec<(String, egui::Rect)> {
    fn collect(shape: &Shape, result: &mut Vec<(String, egui::Rect)>) {
        match shape {
            Shape::Text(text) => result.push((text.galley.job.text.clone(), egui::Rect::from_min_size(text.pos, text.galley.size()))),
            Shape::Vec(items) => items.iter().for_each(|item| collect(item, result)),
            _ => {}
        }
    }
    let mut result = Vec::new();
    for shape in &h.output().shapes {
        collect(&shape.shape, &mut result)
    }
    result
}

#[test]
fn basic_type_options_are_present_in_every_theme_for_both_type_tools_v74() {
    for kind in ThemeKind::ALL {
        for vertical in [false, true] {
            let h = shell(kind, vec2(1440.0, 900.0), false, false, vertical);
            let text = drawn_text(&h);
            let top: Vec<_> = text.iter().filter(|(_, rect)| rect.top() < 90.0).collect();
            assert!(
                top.iter().any(|(text, _)| text == &h.state().ui.tool_options.type_font),
                "{kind:?}/{vertical}: font family appears in the options bar: {top:?}"
            );
            assert!(top.iter().any(|(text, _)| text == "pt"), "{kind:?}/{vertical}: font size, not the former generic hint");
            assert!(top.iter().any(|(text, _)| text == crate::i18n::tr(crate::i18n::current(), "Sharp")), "{kind:?}/{vertical}: antialias control");
            assert!(h.state().ui.text_edit.is_some());
        }
    }
}

#[test]
fn blank_insertion_draws_a_visible_ibeam_and_ime_rect_even_in_hidden_blink_phase_v74() {
    let mut h = shell(ThemeKind::StudioLight, vec2(1200.0, 800.0), false, true, false);
    // At 60Hz this is well inside an old absolute blink's hidden half.
    h.run_steps(36);
    let caret = h.output().platform_output.ime.as_ref().expect("IME insertion rectangle").cursor_rect;
    assert!(caret.height() > 20.0 && caret.width() <= 3.0, "a font-sized empty insertion: {caret:?}");
    fn segments(shape: &Shape, lines: &mut Vec<[egui::Pos2; 2]>) {
        match shape {
            Shape::LineSegment { points, .. } => lines.push(*points),
            Shape::Vec(items) => items.iter().for_each(|item| segments(item, lines)),
            _ => {}
        }
    }
    let mut lines = Vec::new();
    for shape in &h.output().shapes {
        segments(&shape.shape, &mut lines)
    }
    assert!(
        lines.iter().any(|line| (line[0].x - caret.center().x).abs() < 2.0 && (line[1].x - line[0].x).abs() < 0.1 && (line[1].y - line[0].y).abs() > 20.0),
        "empty caret is actually painted"
    );
    assert!(
        lines.iter().any(|line| (line[1].x - line[0].x).abs() >= 4.9 && (line[1].y - line[0].y).abs() < 0.1 && caret.expand(4.0).contains(line[0])),
        "I-beam end cap is actually painted"
    );
}

#[test]
#[ignore = "writes production text options, Properties and empty-caret screenshots for root review"]
fn text_workflow_gpu_visual_v74() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v74/text-gpu");
    assert!(!out.exists(), "preserve previous visual candidates");
    std::fs::create_dir_all(&out).unwrap();
    let mut manifest = Vec::new();
    for kind in ThemeKind::ALL {
        for empty in [false, true] {
            let mut h = shell(kind, vec2(1440.0, 1000.0), true, empty, false);
            h.run_steps(4);
            let file = format!("{}-{}.png", kind.id(), if empty { "empty" } else { "text" });
            let mut proof = assert_shell_theme(&h, kind);
            h.render().unwrap().save(out.join(&file)).unwrap();
            assert_shell_theme(&h, kind);
            proof["file"] = json!(file);
            manifest.push(proof);
        }
    }
    let mut h = shell(ThemeKind::StudioLight, vec2(1024.0, 760.0), true, false, true);
    let mut proof = assert_shell_theme(&h, ThemeKind::StudioLight);
    h.render().unwrap().save(out.join("studioLight-small-vertical.png")).unwrap();
    assert_shell_theme(&h, ThemeKind::StudioLight);
    proof["file"] = json!("studioLight-small-vertical.png");
    manifest.push(proof);
    std::fs::write(out.join("theme-manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}
