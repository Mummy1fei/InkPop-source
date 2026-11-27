//! Actual egui RawInput confirmation clicks on a synthetic RGBA smart object.
//! No ui.pointer, direct tool_event or direct confirm call is used in these cases.

use std::sync::Arc;

use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::{PhotocraftApp, Tool, canvas::ViewXform};

fn set_tool(app: &mut PhotocraftApp, tool: Tool) {
    app.ui.tool = tool;
    app.sync_tool_brush();
    app.run(
        "tools.setBrush",
        json!({"brush":{
            "size":12.0,"hardness":1.0,"spacing":0.1,"pressureSize":false,
            "opacity":1.0,"flow":1.0,"smoothing":{"amount":0.0}
        }}),
    )
    .unwrap();
}

fn fixture(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.background_jobs = false;
    app.run("file.new", json!({"width":120,"height":80})).unwrap();
    app.run("layer.new.layer", json!({"name":"opaque green subject"})).unwrap();
    app.run("edit.fill", json!({"color":"#00ff00"})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground":"#0000ff"})).unwrap();
    app.ui.extras.rulers = false;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    app.sync_views();
    app.ui.views[0] = crate::state::View { zoom: 3.0, center: [60.0, 40.0], rotation: 0.0, doc_size: [120, 80], fit_pending: false };
    set_tool(&mut app, tool);
    let mut h = Harness::builder().with_size(vec2(1000.0, 720.0)).with_max_steps(64).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::i18n::with_language(crate::i18n::Lang::from_code("en").unwrap(), || {
                // Same relative order as ROOT: the canvas first, modal afterward.
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
                crate::dialogs::show(app, ui.ctx());
            });
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::StudioLight);
    h.ctx.set_embed_viewports(true);
    h.run_steps(4);
    assert_eq!(kind(h.state()), "Smart Object");
    assert_eq!(rgba(h.state(), 30, 20), [0.0, 1.0, 0.0, 1.0], "fixture must contain opaque green source pixels");
    h
}

fn history(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

fn kind(app: &PhotocraftApp) -> &'static str {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().content.kind_name()
}

fn pixels(app: &PhotocraftApp) -> Vec<u8> {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().to_interleaved(st.doc.bounds())
}

fn rgba(app: &PhotocraftApp, x: i32, y: i32) -> [f32; 4] {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)
}

fn screen(h: &Harness<'_, PhotocraftApp>, point: [f32; 2]) -> Pos2 {
    ViewXform::active(h.state()).unwrap().to_screen(point[0], point[1])
}

fn button(h: &mut Harness<'_, PhotocraftApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, pos: Pos2) {
    h.hover_at(pos);
    h.run_steps(1);
    button(h, pos, true);
    button(h, pos, false);
    h.run_steps(3);
}

fn trigger(h: &mut Harness<'_, PhotocraftApp>) {
    let point = screen(h, [30.0, 20.0]);
    click(h, point);
    assert_eq!(h.state().ui.dialogs.len(), 1);
    assert!(crate::rasterize_prompt::owns(&h.state().ui.dialogs[0].fields));
    assert_eq!(kind(h.state()), "Smart Object");
    assert!(h.state().drag.is_none() && h.state().live_stroke.is_none());
}

fn answer(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    let rect = h.get_by_label(label).rect();
    assert!(rect.is_positive() && h.ctx.content_rect().contains_rect(rect), "answer must target the real visible modal button");
    click(h, rect.center());
    assert!(h.state().ui.dialogs.is_empty());
}

fn assert_converted_without_paint(h: &Harness<'_, PhotocraftApp>, original: &[u8], before: usize, tool: Tool) {
    assert_eq!(kind(h.state()), "Pixel");
    assert_eq!(pixels(h.state()), original, "conversion/confirmation cannot erase or paint any source RGBA sample");
    assert_eq!(history(h.state()), before + 1, "confirmation adds only the conversion, no deferred paint history");
    assert_eq!(h.state().ui.tool, tool);
    assert!(h.state().drag.is_none() && h.state().live_stroke.is_none());
}

#[test]
fn real_ok_click_converts_without_replaying_pixel_tools_v89() {
    for tool in [Tool::Eraser, Tool::Brush, Tool::Pencil, Tool::PaintBucket, Tool::MagicEraser] {
        let mut h = fixture(tool);
        let original = pixels(h.state());
        let before = history(h.state());
        trigger(&mut h);
        assert_eq!(pixels(h.state()), original);
        assert_eq!(history(h.state()), before);
        answer(&mut h, "OK");
        assert_converted_without_paint(&h, &original, before, tool);
        // Merely moving back over the image, and a redundant release, are not
        // permission to resume either the original stroke or the answer click.
        let at = screen(&h, [75.0, 55.0]);
        h.hover_at(at);
        h.run_steps(2);
        button(&mut h, at, false);
        h.run_steps(2);
        assert_converted_without_paint(&h, &original, before, tool);
        assert!(h.state_mut().session.undo());
        assert_eq!(kind(h.state()), "Smart Object");
        assert_eq!(pixels(h.state()), original, "one undo after confirmation restores the unchanged editable source");
    }
}

#[test]
fn only_a_new_canvas_down_after_confirmation_can_start_eraser_v89() {
    let mut h = fixture(Tool::Eraser);
    let original = pixels(h.state());
    let before = history(h.state());
    trigger(&mut h);
    answer(&mut h, "OK");
    assert_converted_without_paint(&h, &original, before, Tool::Eraser);
    let from = screen(&h, [75.0, 55.0]);
    let to = screen(&h, [83.0, 55.0]);
    h.hover_at(from);
    h.run_steps(1);
    button(&mut h, from, true);
    h.event(Event::PointerMoved(to));
    h.run_steps(1);
    button(&mut h, to, false);
    h.run_steps(3);
    assert!(rgba(h.state(), 75, 55)[3] < 0.05, "the fresh gesture still erases normally");
    assert_eq!(rgba(h.state(), 30, 20), [0.0, 1.0, 0.0, 1.0], "the original prompt point stays intact");
    assert_eq!(history(h.state()), before + 2, "conversion and the new stroke are separate history states");
    assert!(h.state_mut().session.undo());
    assert_eq!(kind(h.state()), "Pixel");
    assert_eq!(pixels(h.state()), original);
    assert!(h.state_mut().session.undo());
    assert_eq!(kind(h.state()), "Smart Object");
    assert_eq!(pixels(h.state()), original);
}

#[test]
fn enter_while_original_pointer_is_held_does_not_resume_that_drag_v89() {
    let mut h = fixture(Tool::Eraser);
    let original = pixels(h.state());
    let before = history(h.state());
    let down = screen(&h, [30.0, 20.0]);
    let moved = screen(&h, [40.0, 20.0]);
    h.hover_at(down);
    h.run_steps(1);
    button(&mut h, down, true);
    h.event(Event::PointerMoved(moved));
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.len(), 1, "a real dragging press must ask before painting");
    assert_eq!(history(h.state()), before);
    assert_eq!(pixels(h.state()), original);
    h.event(Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::Key { key: Key::Enter, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().ui.dialogs.is_empty());
    assert_converted_without_paint(&h, &original, before, Tool::Eraser);
    let released = screen(&h, [83.0, 55.0]);
    h.event(Event::PointerMoved(released));
    h.run_steps(1);
    button(&mut h, released, false);
    h.run_steps(3);
    assert_converted_without_paint(&h, &original, before, Tool::Eraser);
    let fresh = screen(&h, [75.0, 55.0]);
    click(&mut h, fresh);
    assert!(rgba(h.state(), 75, 55)[3] < 0.05, "a later fresh canvas press is not swallowed");
    assert_eq!(history(h.state()), before + 2);
}

#[test]
fn real_cancel_preserves_source_and_allows_a_later_new_prompt_v89() {
    let mut h = fixture(Tool::Eraser);
    let original = pixels(h.state());
    let before = history(h.state());
    let doc = Arc::clone(&h.state().session.active().unwrap().doc);
    trigger(&mut h);
    answer(&mut h, "Cancel");
    assert_eq!(kind(h.state()), "Smart Object");
    assert_eq!(pixels(h.state()), original);
    assert_eq!(history(h.state()), before);
    assert!(Arc::ptr_eq(&doc, &h.state().session.active().unwrap().doc));
    assert!(h.state().drag.is_none() && h.state().live_stroke.is_none());
    trigger(&mut h);
    answer(&mut h, "Cancel");
    assert_eq!(pixels(h.state()), original);
    assert_eq!(history(h.state()), before);
}

#[test]
fn confirmation_keeps_current_tool_and_ignores_stored_input_provenance_v89() {
    let mut h = fixture(Tool::Eraser);
    let original = pixels(h.state());
    let before = history(h.state());
    trigger(&mut h);
    let id = h.state().ui.dialogs[0].id;
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("at".into(), json!([90.0, 60.0, 1.0]));
    set_tool(h.state_mut(), Tool::Brush);
    answer(&mut h, "OK");
    assert_converted_without_paint(&h, &original, before, Tool::Brush);
    let fresh = screen(&h, [75.0, 55.0]);
    click(&mut h, fresh);
    let color = rgba(h.state(), 75, 55);
    assert!(color[2] > 0.99 && color[0] < 0.01 && color[3] > 0.99, "the current brush, not the stored eraser, owns the next press: {color:?}");
    assert_eq!(rgba(h.state(), 30, 20), [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(rgba(h.state(), 90, 60), [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(history(h.state()), before + 2);
}

#[test]
fn a_deleted_prompt_target_cannot_rasterize_or_paint_another_layer_v89() {
    let mut h = fixture(Tool::Eraser);
    trigger(&mut h);
    let target = h.state().ui.dialogs[0].fields["layer"].as_u64().unwrap();
    h.state_mut().run("layer.delete", json!({"layer":target})).unwrap();
    h.run_steps(2);
    let document = Arc::clone(&h.state().session.active().unwrap().doc);
    let before = history(h.state());
    answer(&mut h, "OK");
    assert_eq!(h.state().session.active().unwrap().doc.as_ref(), document.as_ref(), "a stale prompt may not act on its replacement active layer");
    assert_eq!(history(h.state()), before);
    assert!(h.state().ui.status_error);
    assert!(h.state().drag.is_none() && h.state().live_stroke.is_none());
}
