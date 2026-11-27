//! Real shell shortcuts, pointer rotation and painting through the rotated transform.
use egui::{Event, Key, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, canvas::ViewXform, menus, state::Tool, theme::ThemeKind};
use serde_json::json;

fn fixture() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Pro);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.prefs.edit(|p| p.interface.language = "en".into());
        app.run("file.new", json!({"width":300,"height":200,"background":"transparent"})).unwrap();
        app.run("tools.setColors", json!({"foreground":"#ff0000"})).unwrap();
        app.run("tools.setBrush", json!({"brush":{"size":6,"hardness":1,"pressureSize":false,"smoothing":{"amount":0}}})).unwrap();
        app
    });
    h.run_steps(6);
    {
        let view = &mut h.state_mut().ui.views[0];
        view.zoom = 1.0;
        view.center = [150.0, 100.0];
        view.fit_pending = false;
    }
    h.run_steps(3);
    h
}

fn key(h: &mut Harness<'_, PhotocraftApp>, key: Key) {
    h.event(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    h.event(Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

fn press(h: &mut Harness<'_, PhotocraftApp>, pos: Pos2, pressed: bool, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers });
    h.run_steps(2);
}

fn rotate(h: &mut Harness<'_, PhotocraftApp>, degrees: f32, modifiers: Modifiers) {
    let center = ViewXform::active(h.state()).unwrap().rect.center();
    let radius = 95.0;
    let start = center + vec2(radius, 0.0);
    h.event(Event::PointerMoved(start));
    h.run_steps(2);
    press(h, start, true, modifiers);
    let mut end = start;
    for step in 1..=12 {
        let angle = (degrees * step as f32 / 12.0).to_radians();
        end = center + vec2(angle.cos(), angle.sin()) * radius;
        h.event(Event::PointerMoved(end));
        h.run_steps(2);
    }
    press(h, end, false, modifiers);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

#[test]
fn r_drags_rotate_without_editing_shift_snaps_and_reset_restores_view() {
    let mut h = fixture();
    let revision = h.state().session.active().unwrap().revision;
    let history = h.state().session.active().unwrap().history.past_len();
    let zoom = h.state().ui.views[0].zoom;
    key(&mut h, Key::R);
    assert_eq!(h.state().ui.tool, Tool::RotateView);
    h.get_by_label("Rotation Angle");
    rotate(&mut h, 43.0, Modifiers::NONE);
    assert!((h.state().ui.views[0].rotation - 43.0).abs() < 0.2, "drag angle: {}", h.state().ui.views[0].rotation);
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    h.get_by_label("Reset View").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.views[0].rotation, 0.0);
    assert_eq!(h.state().ui.views[0].zoom, zoom);
    h.run_steps(30);
    rotate(&mut h, 23.0, Modifiers::SHIFT);
    assert!((h.state().ui.views[0].rotation - 30.0).abs() < 0.01);
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
}

#[test]
fn painting_after_rotation_hits_document_pixels_under_the_pointer() {
    let mut h = fixture();
    // Setting the view angle is independent of tool selection. Leave the fixture's initial
    // Brush first; otherwise B legitimately cycles the already-selected brush group to Pencil.
    key(&mut h, Key::R);
    let ctx = h.ctx.clone();
    menus::invoke(h.state_mut(), &ctx, "view.rotate", json!({"angle":90})).unwrap();
    key(&mut h, Key::B);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    let xf = ViewXform::active(h.state()).unwrap();
    let start = xf.to_screen(80.5, 60.5);
    let end = xf.to_screen(100.5, 60.5);
    press(&mut h, start, true, Modifiers::NONE);
    for i in 1..=10 {
        h.event(Event::PointerMoved(start + (end - start) * i as f32 / 10.0));
        h.run_steps(2);
    }
    press(&mut h, end, false, Modifiers::NONE);
    h.run_steps(4);
    let state = h.state().session.active().unwrap();
    let surface = state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap();
    let painted = surface.pixel(90, 60);
    assert!(painted[0] > 0.9 && painted[1] < 0.1 && painted[2] < 0.1 && painted[3] > 0.9, "rotated hit: {painted:?}");
    let unrotated_position = surface.pixel(190, 40);
    assert_eq!(unrotated_position[3], 0.0, "screen coordinates must be unrotated before painting");
    assert_eq!(h.state().ui.views[0].rotation, 90.0);
}

#[test]
fn rotation_cursor_replaces_hand_on_hover_and_during_captured_drag() {
    let mut h = fixture();
    key(&mut h, Key::R);
    let rect = ViewXform::active(h.state()).unwrap().rect;
    let start = rect.center() + vec2(95.0, 0.0);
    h.event(Event::PointerMoved(start));
    h.run_steps(3);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "rotation glyph replaces the system hand");
    let cursor_layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new(("pc-icon-cursor", h.ctx.viewport_id())));
    assert!(h.ctx.memory(|m| m.areas().visible_last_frame(&cursor_layer)), "the hidden system cursor must have a visible rotation overlay");

    // Keep the pointer stationary through theme changes: one passing frame would miss a
    // cursor Area alternately taking canvas hover and disappearing on the following frame.
    let mut missing_cursor_frames = Vec::new();
    for theme in [ThemeKind::Studio, ThemeKind::StudioLight, ThemeKind::ProMedium, ThemeKind::Pro, ThemeKind::Classic] {
        let ctx = h.ctx.clone();
        h.state_mut().set_theme(&ctx, theme);
        h.run_steps(1);
        if h.output().platform_output.cursor_icon != egui::CursorIcon::None {
            let viewport = h.output().viewport_output.get(&h.ctx.viewport_id()).unwrap();
            assert!(viewport.repaint_delay.is_zero(), "{theme:?}: changing widget IDs must request another frame without requiring pointer motion");
        }
        // A different theme family rebuilds the widget hierarchy. egui resolves hit testing
        // from the previous pass, so let the new canvas ID enter that map before checking.
        h.run_steps(2);
        for frame in 0..12 {
            h.run_steps(1);
            let icon = h.output().platform_output.cursor_icon;
            let visible = h.ctx.memory(|m| m.areas().visible_last_frame(&cursor_layer));
            if icon != egui::CursorIcon::None || !visible {
                missing_cursor_frames.push(format!("{theme:?} idle frame {frame}: {icon:?}, overlay visible = {visible}"));
            }
            let glyph = h.output().shapes.iter().any(|shape| {
                if let egui::Shape::Rect(rect) = &shape.shape {
                    rect.rect.contains(start) && (rect.rect.width() - 22.0).abs() < 0.1 && rect.fill.a() > 0 && rect.brush.is_some()
                } else {
                    false
                }
            });
            assert!(glyph, "{theme:?} idle frame {frame}: overlay has no visible cursor image");
        }
    }
    assert!(missing_cursor_frames.is_empty(), "{}", missing_cursor_frames.join("\n"));

    press(&mut h, start, true, Modifiers::NONE);
    let outside = egui::pos2(rect.right() + 8.0, start.y + 45.0);
    h.event(Event::PointerMoved(outside));
    h.run_steps(3);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "captured rotation keeps its glyph outside the canvas");
    assert!(h.ctx.memory(|m| m.areas().visible_last_frame(&cursor_layer)));
    press(&mut h, outside, false, Modifiers::NONE);
    h.event(Event::PointerMoved(egui::pos2(4.0, 4.0)));
    h.run_steps(3);
    assert_ne!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "ordinary UI restores the system pointer after release");
    assert!(!h.ctx.memory(|m| m.areas().visible_last_frame(&cursor_layer)), "rotation overlay must not remain over other UI");

    h.event(Event::WindowFocused(false));
    h.event(Event::PointerMoved(start));
    h.run_steps(3);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None, "hover identifies the tool before a floating view gains keyboard focus");
    assert!(h.ctx.memory(|m| m.areas().visible_last_frame(&cursor_layer)));
}
