//! Synthetic native capture and mixed-DPI regression coverage; no hardware claims.
use super::*;
use crate::{dock::Group, panel_workspace::Tab};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.frame = 20;
    app.ui.panels.layers = true;
    app.ui.panels.color = true;
    app.ui.dock.containers = vec![
        Container { id: 10, floating: true, tabs: vec![Tab { group: Group::Layers, index: 0 }, Tab { group: Group::Layers, index: 1 }], ..Default::default() },
        Container { id: 11, floating: true, tabs: vec![Tab { group: Group::Color, index: 1 }], ..Default::default() },
    ];
    app
}

fn input(id: ViewportId, origin: Pos2, events: Vec<egui::Event>) -> egui::RawInput {
    let mut input = egui::RawInput { viewport_id: id, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 500.0))), events, ..Default::default() };
    let root = input.viewports.entry(ViewportId::ROOT).or_default();
    root.native_pixels_per_point = Some(2.0);
    root.inner_rect = Some(Rect::from_min_size(pos2(100.0, 50.0), vec2(1000.0, 800.0)));
    let child = input.viewports.entry(id).or_default();
    child.native_pixels_per_point = Some(1.0);
    child.outer_rect = Some(Rect::from_min_size(origin, vec2(400.0, 500.0)));
    child.inner_rect = child.outer_rect;
    input
}

fn touch(phase: egui::TouchPhase) -> egui::Event {
    // Deliberately stale local input: captured desktop samples must drive the target.
    egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(4), phase, pos: pos2(20.0, 20.0), force: Some(0.5) }
}

#[test]
fn captured_pen_header_drop_uses_shared_desktop_point_across_dpi_and_keeps_every_tab_v74() {
    use std::{cell::Cell, rc::Rc};
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = app();
    let source = app.ui.dock.containers[0].clone();
    let source_id = viewport(10, 0);
    let target_id = viewport(11, 0);
    let sample = Rc::new(Cell::new(pos2(370.0, 220.0)));
    let read = Rc::clone(&sample);
    app.services.contact_screen_pos = Some(Box::new(move |_, _| Some(read.get())));
    let title = Rect::from_min_size(pos2(12.0, 12.0), vec2(300.0, 24.0));
    let mut start = ctx.run_ui(input(source_id, pos2(350.0, 200.0), vec![touch(egui::TouchPhase::Start)]), |ui| drag_title(&mut app, ui, &source, title, true));
    start.textures_delta.clear();
    // Register the target's real local menu strip using its independent window origin.
    let mut target = ctx.run_ui(input(target_id, pos2(550.0, 400.0), vec![]), |ui| {
        super::super::register(&app, ui.ctx(), 11, Rect::from_min_size(pos2(10.0, 0.0), vec2(200.0, 28.0)));
    });
    target.textures_delta.clear();
    sample.set(pos2(580.0, 410.0));
    let mut moved = ctx.run_ui(input(source_id, pos2(350.0, 200.0), vec![touch(egui::TouchPhase::Move)]), |ui| {
        drag_title(&mut app, ui, &source, title, true);
        let payload = Payload { container: 10, tab: None };
        let point = super::super::current_drag_point(ui.ctx(), &payload).unwrap();
        assert_eq!(point, pos2(190.0, 155.0));
        assert_eq!(super::super::merge_target(&app, ui.ctx(), &payload, point).unwrap().id, 11);
    });
    moved.textures_delta.clear();
    let mut ended = ctx.run_ui(input(source_id, pos2(350.0, 200.0), vec![touch(egui::TouchPhase::End)]), |ui| {
        drag_title(&mut app, ui, &source, title, true);
        queue_drop(ui.ctx(), pos2(900.0, 700.0)); // a later synthetic mouse release cannot replace the pen endpoint
    });
    ended.textures_delta.clear();
    assert_eq!(ctx.data(|d| d.get_temp::<PendingDrop>(Id::new("native-panel-pending-drop"))).unwrap().point, pos2(190.0, 155.0));
    finish_pending(&mut app, &ctx);
    assert_eq!(app.ui.dock.containers.len(), 1);
    let target = &app.ui.dock.containers[0];
    assert_eq!(target.id, 11);
    assert_eq!(target.tabs.len(), 3);
    assert!(target.tabs.contains(&Tab { group: Group::Layers, index: 0 }));
    assert!(target.tabs.contains(&Tab { group: Group::Layers, index: 1 }));
    assert!(target.tabs.contains(&Tab { group: Group::Color, index: 1 }));
    assert!(egui::DragAndDrop::payload::<Payload>(&ctx).is_none());
    assert!(ctx.data(|d| d.get_temp::<super::super::DragPoint>(super::super::drag_point_id())).is_none());
}

#[test]
fn escape_and_pen_cancel_clear_preview_and_do_not_mutate_layout_v74() {
    for cancel in [
        touch(egui::TouchPhase::Cancel),
        egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
    ] {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = app();
        let before = app.ui.dock.containers.clone();
        let source = before[0].clone();
        let source_id = viewport(10, 0);
        let title = Rect::from_min_size(pos2(12.0, 12.0), vec2(300.0, 24.0));
        let mut started =
            ctx.run_ui(input(source_id, pos2(350.0, 200.0), vec![touch(egui::TouchPhase::Start)]), |ui| drag_title(&mut app, ui, &source, title, true));
        started.textures_delta.clear();
        egui::DragAndDrop::set_payload(&ctx, Payload { container: 10, tab: None });
        super::super::set_drag_point(&ctx, 10, pos2(190.0, 155.0), true);
        ctx.data_mut(|d| d.insert_temp(Id::new("panel-drag-whole-stack"), 10u64));
        let mut cancelled = ctx.run_ui(input(source_id, pos2(350.0, 200.0), vec![cancel]), |ui| drag_title(&mut app, ui, &source, title, true));
        cancelled.textures_delta.clear();
        finish_pending(&mut app, &ctx);
        assert_eq!(app.ui.dock.containers, before);
        assert!(egui::DragAndDrop::payload::<Payload>(&ctx).is_none());
        assert!(ctx.data(|d| d.get_temp::<super::super::DragPoint>(super::super::drag_point_id())).is_none());
        assert!(ctx.data(|d| d.get_temp::<u64>(Id::new("panel-drag-whole-stack"))).is_none());
        assert!(ctx.data(|d| d.get_temp::<Drag>(Id::new(("native-panel-drag", source_id)))).is_none());
    }
}
