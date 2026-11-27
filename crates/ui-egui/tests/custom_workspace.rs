//! Exercise the actual popup resize handle and independently enabled mixer controls.
use egui::{Event, Id, Modifiers, PointerButton, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, theme::ThemeKind};
use serde_json::json;

#[test]
fn picker_resizes_and_mixer_controls_work_without_transfer_dynamics() {
    let mut h = Harness::builder().with_size(vec2(1600., 1000.)).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Studio);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        app.run("file.new", json!({"width":100,"height":100})).unwrap();
        app.ui.brush_picker = Some([700., 100.]);
        app
    });
    h.run_steps(8);
    let before = h.ctx.memory(|m| m.area_rect(Id::new("canvas-brush-picker"))).unwrap();
    let start = before.max - vec2(14., 14.);
    let end = start + vec2(120., 140.);
    h.event(Event::PointerMoved(start));
    h.run_steps(2);
    h.event(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    for i in 1..=10 {
        h.event(Event::PointerMoved(start + (end - start) * (i as f32 / 10.)));
        h.run_steps(2);
    }
    h.event(Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(4);
    let after = h.ctx.memory(|m| m.area_rect(Id::new("canvas-brush-picker"))).unwrap();
    assert!(after.width() > before.width() + 80. && after.height() > before.height() + 80., "{before:?}->{after:?}");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.state_mut().ui.panels.brush_settings = true;
    h.state_mut().ui.panels.brush_floating = true;
    h.state_mut().ui.brush_section = 6;
    h.state_mut().ui.brush_tab = 0;
    h.state_mut().ui.dock =
        serde_json::from_value(json!({"containers":[{"id":91,"tabs":[{"group":"brush","index":0}],"floating":true,"position":[10,60],"size":[850,880]}]}))
            .unwrap();
    h.state_mut().session.tools.brush.transfer.enabled = false;
    h.run_steps(6);
    h.get_by_label("Simulate Paint").click();
    h.run_steps(3);
    assert!(h.state().session.tools.brush.mixer.simulate_paint);
    assert!(!h.state().session.tools.brush.transfer.enabled);
}
