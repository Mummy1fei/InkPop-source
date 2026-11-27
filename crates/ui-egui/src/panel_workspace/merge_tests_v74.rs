//! Header-only panel merge preview/drop through production floating containers.

use super::*;
use egui::{Event, Modifiers, PointerButton, Pos2, Shape, ViewportId};
use egui_kittest::Harness;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", serde_json::json!({"width":80,"height":60})).unwrap();
    app.ui.panels.layers = true;
    app.ui.panels.color = true;
    app.ui.dock.containers = vec![
        Container {
            id: 10,
            tabs: vec![Tab { group: Group::Layers, index: 0 }, Tab { group: Group::Layers, index: 1 }],
            floating: true,
            position: [20.0, 80.0],
            size: [380.0, 500.0],
            ..Default::default()
        },
        Container {
            id: 11,
            tabs: vec![Tab { group: Group::Color, index: 1 }],
            floating: true,
            position: [570.0, 80.0],
            size: [320.0, 500.0],
            ..Default::default()
        },
    ];
    app
}

fn harness(kind: crate::theme::ThemeKind, gpu: bool) -> Harness<'static, PhotocraftApp> {
    let builder = Harness::builder().with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).with_max_steps(64);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    let mut h = builder.build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            app.frame += 1;
            egui::CentralPanel::default().show(ui, |_| {});
            floating(app, &ctx);
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, kind);
    h.run_steps(8);
    h
}

fn header(h: &Harness<'_, PhotocraftApp>, id: u64) -> Rect {
    h.ctx.data(|data| data.get_temp::<Vec<Zone>>(zones_id())).unwrap().iter().find(|zone| zone.id == id).unwrap().rect
}

fn window(h: &Harness<'_, PhotocraftApp>, id: u64) -> Rect {
    h.ctx.memory(|memory| memory.area_rect(egui::Id::new(("panel-container", id)))).unwrap()
}

fn drag(h: &mut Harness<'_, PhotocraftApp>, to_header: bool) -> Pos2 {
    let source = window(h, 10);
    let start = pos2(source.center().x, source.top() + 14.0);
    let end = if to_header { header(h, 11).center() } else { window(h, 11).center() };
    h.hover_at(start);
    h.step();
    h.event(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for step in 1..=12 {
        h.hover_at(start + (end - start) * (step as f32 / 12.0));
        h.run_steps(2);
    }
    h.event(Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(4);
    end
}

fn highlighted(h: &Harness<'_, PhotocraftApp>, rect: Rect) -> bool {
    let tint = Tokens::get(&h.ctx).accent.gamma_multiply(0.18);
    fn painted(shape: &Shape, rect: Rect, tint: egui::Color32) -> bool {
        match shape {
            Shape::Rect(shape) => shape.rect == rect && shape.fill == tint,
            Shape::Vec(shapes) => shapes.iter().any(|shape| painted(shape, rect, tint)),
            _ => false,
        }
    }
    h.output().shapes.iter().any(|shape| painted(&shape.shape, rect, tint))
}

#[test]
fn real_floating_title_merges_only_on_the_actual_tab_header_v74() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = harness(theme, false);
        drag(&mut h, true);
        assert_eq!(h.state().ui.dock.containers.len(), 1, "{theme:?}: header combines tabs");
        let target = &h.state().ui.dock.containers[0];
        assert_eq!(target.id, 11);
        assert_eq!(target.tabs.len(), 3);
        assert!(target.stack.is_none());
        assert!(target.tabs.contains(&Tab { group: Group::Layers, index: 0 }));
        assert!(target.tabs.contains(&Tab { group: Group::Layers, index: 1 }));
    }
}

#[test]
fn real_floating_title_over_content_does_not_merge_or_reorder_v74() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight, false);
    let before = h.state().ui.dock.containers.clone();
    let origin = window(&h, 10).min;
    drag(&mut h, false);
    assert_eq!(h.state().ui.dock.containers.len(), 2);
    for (old, current) in before.iter().zip(&h.state().ui.dock.containers) {
        assert_eq!(old.id, current.id);
        assert_eq!(old.tabs, current.tabs);
        assert_eq!(old.stack, current.stack);
        assert!(current.floating);
    }
    let placed = window(&h, 10);
    assert!(placed.min.distance(origin) > 80.0, "the window still follows the drag");
    h.run_steps(20);
    assert!(window(&h, 10).min.distance(placed.min) < 1.0, "content drop keeps the placed floating position");
}

#[test]
fn header_preview_and_drop_share_the_same_hit_test_in_all_themes_v74() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = harness(theme, false);
        for over_header in [false, true] {
            let point = if over_header { header(&h, 11).center() } else { window(&h, 11).center() };
            egui::DragAndDrop::set_payload(&h.ctx, Payload { container: 10, tab: None });
            h.hover_at(point);
            h.run_steps(2);
            let target = merge_target(h.state(), &h.ctx, &Payload { container: 10, tab: None }, point);
            assert_eq!(target.is_some(), over_header, "{theme:?}: only the menu strip accepts merge");
            assert_eq!(highlighted(&h, header(&h, 11)), over_header, "{theme:?}: visible tint matches drop eligibility");
        }
    }
}

#[test]
fn stale_closed_collapsed_and_same_stack_targets_never_accept_merge_v74() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.frame = 9;
    let rect = Rect::from_min_size(pos2(100.0, 100.0), vec2(240.0, 28.0));
    let point = rect.center();
    register(&app, &ctx, 11, rect);
    let payload = Payload { container: 10, tab: None };
    assert_eq!(merge_target(&app, &ctx, &payload, point).unwrap().id, 11);
    app.frame += 1;
    assert!(merge_target(&app, &ctx, &payload, point).is_none());
    register(&app, &ctx, 11, rect);
    for field in [0, 1, 2] {
        app.ui.dock.containers[1].closed = field == 0;
        app.ui.dock.containers[1].collapsed = field == 1;
        app.ui.dock.containers[1].stack = if field == 2 { Some(10) } else { None };
        assert!(merge_target(&app, &ctx, &payload, point).is_none());
    }
    app.ui.dock.containers[1].stack = None;
    app.ui.dock.containers[1].collapsed = false;
    app.ui.dock.containers[1].closed = false;
    assert!(merge_target(&app, &ctx, &Payload { container: 10, tab: Some(100) }, point).is_none());
    app.ui.panels.color = false;
    assert!(merge_target(&app, &ctx, &payload, point).is_none());
}

#[test]
fn rightmost_content_is_not_an_implicit_dock_target_and_body_drop_preserves_stack_v74() {
    let ctx = egui::Context::default();
    let mut app = app();
    app.frame = 4;
    app.ui.dock.containers.push(Container {
        id: 12,
        tabs: vec![Tab { group: Group::Layers, index: 2 }],
        floating: true,
        stack: Some(10),
        ..Default::default()
    });
    let before = app.ui.dock.containers.clone();
    egui::DragAndDrop::set_payload(&ctx, Payload { container: 10, tab: None });
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("panel-drag-whole-stack"), 10u64));
    register(&app, &ctx, 11, Rect::from_min_size(pos2(950.0, 20.0), vec2(200.0, 28.0)));
    finish_drag(&mut app, &ctx, true, Some(pos2(1150.0, 350.0)));
    assert_eq!(app.ui.dock.containers, before, "no header/rail hit: even the old rightmost band must leave the floating stack intact");
    assert!(egui::DragAndDrop::payload::<Payload>(&ctx).is_none());
    assert!(ctx.data(|data| data.get_temp::<DragPoint>(drag_point_id())).is_none());
}

#[test]
fn native_precise_shared_hover_is_not_overwritten_by_target_or_synthetic_mouse_v74() {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = app();
    app.frame = 4;
    let target_viewport = ViewportId::from_hash_of("merge-target-v74");
    // Drain initial viewport repaint requests before observing cross-window wakeups.
    for viewport_id in [ViewportId::ROOT, target_viewport] {
        for step in 0..6 {
            let mut input = egui::RawInput { viewport_id, time: Some(step as f64 / 60.0), ..Default::default() };
            // A backend must list the current child even when there are no widgets.
            input.viewports.entry(target_viewport).or_default();
            let mut output = ctx.run_ui(input, |_| {});
            output.textures_delta.clear();
        }
    }
    ctx.data_mut(|data| {
        data.insert_temp(
            zones_id(),
            vec![Zone { id: 11, rect: Rect::from_min_size(pos2(100.0, 100.0), vec2(200.0, 28.0)), frame: 4, viewport: target_viewport }],
        )
    });
    let repaints = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let events = std::sync::Arc::clone(&repaints);
    ctx.set_request_repaint_callback(move |request| events.lock().unwrap().push(request.viewport_id));
    set_drag_point(&ctx, 10, pos2(140.0, 114.0), true);
    set_drag_point(&ctx, 10, pos2(900.0, 600.0), false);
    let point = current_drag_point(&ctx, &Payload { container: 10, tab: None }).unwrap();
    assert_eq!(point, pos2(140.0, 114.0));
    assert!(merge_target(&app, &ctx, &Payload { container: 10, tab: None }, point).is_some());
    assert!(repaints.lock().unwrap().contains(&ViewportId::ROOT));
    assert!(repaints.lock().unwrap().contains(&target_viewport));
    clear_drag_point(&ctx);
    assert!(ctx.data(|data| data.get_temp::<DragPoint>(drag_point_id())).is_none());
}

#[test]
fn real_docked_group_header_drag_reorders_without_merging_tabs_v74() {
    let mut state = app();
    for container in &mut state.ui.dock.containers {
        container.floating = false;
    }
    state.ui.dock.containers.reverse(); // Color above Layers before the real drag.
    let mut h = Harness::builder().with_size(vec2(900.0, 800.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            app.frame += 1;
            egui::CentralPanel::default().show(ui, |ui| docked(app, ui));
            floating(app, &ctx);
        },
        state,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(8);
    let layers = header(&h, 10);
    let start = pos2(layers.right() - 60.0, layers.center().y);
    let end = header(&h, 11).center();
    h.hover_at(start);
    h.step();
    h.event(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    for step in 1..=12 {
        h.hover_at(start + (end - start) * (step as f32 / 12.0));
        h.run_steps(2);
    }
    h.event(Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(4);
    let containers = &h.state().ui.dock.containers;
    assert_eq!(containers.len(), 2, "dock reorder must not merge the two groups");
    assert!(containers.iter().all(|c| !c.floating && c.stack.is_none()));
    assert_eq!(containers[0].tabs, vec![Tab { group: Group::Layers, index: 0 }, Tab { group: Group::Layers, index: 1 }]);
    assert_eq!(containers[1].tabs, vec![Tab { group: Group::Color, index: 1 }]);
    assert_eq!(h.state().ui.dock.order().first(), Some(&Group::Layers));
}

#[test]
fn floating_groups_and_individual_docked_tabs_keep_header_merge_v74() {
    for (source_floating, target_floating, tab) in [(true, true, None), (false, true, None), (true, false, None), (false, false, Some(0))] {
        let ctx = egui::Context::default();
        let mut app = app();
        app.ui.dock.containers[0].floating = source_floating;
        app.ui.dock.containers[1].floating = target_floating;
        let target = Rect::from_min_size(pos2(100.0, 100.0), vec2(240.0, 28.0));
        register(&app, &ctx, 11, target);
        egui::DragAndDrop::set_payload(&ctx, Payload { container: 10, tab });
        finish_drag(&mut app, &ctx, true, Some(target.center()));
        let target = app.ui.dock.containers.iter().find(|c| c.id == 11).unwrap();
        assert_eq!(target.tabs.len(), if tab.is_some() { 2 } else { 3 });
        assert_eq!(target.floating, target_floating);
        assert_eq!(app.ui.dock.containers.len(), if tab.is_some() { 2 } else { 1 });
        if tab.is_some() {
            assert_eq!(app.ui.dock.containers.iter().find(|c| c.id == 10).unwrap().tabs, vec![Tab { group: Group::Layers, index: 1 }]);
        }
    }
}

#[test]
#[ignore = "writes header-highlight/body-no-highlight screenshots for root review"]
fn panel_merge_gpu_visual_v74() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v74/panel-merge-gpu");
    assert!(!out.exists(), "preserve previous candidates");
    std::fs::create_dir_all(&out).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        for over_header in [false, true] {
            let mut h = harness(theme, true);
            let point = if over_header { header(&h, 11).center() } else { window(&h, 11).center() };
            egui::DragAndDrop::set_payload(&h.ctx, Payload { container: 10, tab: None });
            h.hover_at(point);
            h.run_steps(2);
            assert_eq!(highlighted(&h, header(&h, 11)), over_header);
            h.render().unwrap().save(out.join(format!("{}-{}.png", theme.id(), if over_header { "header" } else { "body" }))).unwrap();
        }
    }
}
