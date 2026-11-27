//! Exercise the real detached frame/header/body and canvas view update, not just the size helper.
use super::*;
use crate::state::{Tool, View};
use egui::{Event, Modifiers, PointerButton};
use serde_json::json;

fn fixture(theme: crate::theme::ThemeKind) -> (PhotocraftApp, egui::Context, Vec<DocWindow>) {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, theme);
    ctx.set_embed_viewports(false);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    for size in [[800, 600], [320, 240]] {
        app.run("file.new", json!({"width":size[0],"height":size[1],"background":"transparent"})).unwrap();
    }
    app.sync_views();
    for document in 0..2 {
        crate::view_cmds::float_window(&mut app, document, document);
    }
    app.ui.views[0] = View { zoom: 0.405, rotation: 23.0, center: [99.0, 77.0], fit_pending: false, doc_size: [800, 600] };
    app.ui.views[1] = View { zoom: 1.25, rotation: -17.0, center: [42.0, 53.0], fit_pending: false, doc_size: [320, 240] };
    app.ui.tool = Tool::Hand;
    app.ui.extras.rulers = false;
    let windows = app.ui.windows.clone();
    (app, ctx, windows)
}

#[allow(clippy::too_many_arguments)]
fn frame(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    window: &DocWindow,
    size: Vec2,
    origin: Pos2,
    root_minimized: bool,
    child_minimized: bool,
    events: Vec<Event>,
) {
    let viewport = ViewportId::from_hash_of(("docwin", window.id));
    let mut input = egui::RawInput { viewport_id: viewport, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() };
    let info = input.viewports.entry(viewport).or_default();
    info.inner_rect = Some(Rect::from_min_size(origin, size));
    info.outer_rect = info.inner_rect;
    info.focused = Some(true);
    info.minimized = Some(child_minimized);
    info.native_pixels_per_point = Some(1.0);
    let root = input.viewports.entry(ViewportId::ROOT).or_default();
    root.minimized = Some(root_minimized);
    root.inner_rect = Some(Rect::from_min_size(Pos2::ZERO, if root_minimized { Vec2::ZERO } else { vec2(1400.0, 900.0) }));
    let mut output = ctx.run_ui(input, |ui| {
        if size.x < 32.0 || size.y < 64.0 {
            // Some native backends enter the old callback with an empty client rect.
            body(app, ui, window.document, true);
            return;
        }
        let frame = detached_frame(ui.style(), crate::theme::Tokens::get(ui.ctx()));
        let card = ui.max_rect().shrink(FRAME_GUTTER);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(card));
        frame.show(&mut child, |ui| {
            header(app, ui, window, "Resize regression");
            body(app, ui, window.document, true);
        });
    });
    output.textures_delta.clear();
}

fn ordinary_frame(app: &mut PhotocraftApp, ctx: &egui::Context, window: &DocWindow, size: Vec2) {
    frame(app, ctx, window, size, pos2(300.0, 200.0), false, false, Vec::new());
}

#[test]
fn actual_detached_body_resize_recenters_at_constant_manual_zoom_and_rotation_five_themes_v79() {
    for theme in crate::theme::ThemeKind::ALL {
        let (mut app, ctx, windows) = fixture(theme);
        let other = app.ui.views[1].clone();
        let window = &windows[0];
        let initial = app.ui.views[0].clone();
        ordinary_frame(&mut app, &ctx, window, vec2(500.0, 400.0));
        assert_eq!(app.ui.views[0], initial, "{theme:?}: first detach preserves existing pan/zoom");
        for size in [vec2(560.0, 430.0), vec2(720.0, 500.0), vec2(420.0, 360.0)] {
            app.ui.views[0].center = [99.0, 77.0];
            ordinary_frame(&mut app, &ctx, window, size);
            assert_eq!(app.ui.views[0].center, [400.0, 300.0], "{theme:?}: resize recenters document in the actual body");
            assert_eq!(app.ui.views[0].zoom, 0.405, "do not invoke Fit and change40.5% zoom");
            assert_eq!(app.ui.views[0].rotation, 23.0);
            assert!(!app.ui.views[0].fit_pending);
            assert_eq!(app.ui.views[1], other, "another document's view stays independent");
            let xf = canvas::ViewXform {
                rect: app.last_canvas_rect,
                zoom: app.ui.views[0].zoom,
                rotation: app.ui.views[0].rotation,
                center: app.ui.views[0].center,
                flip: false,
            };
            assert!(
                xf.to_screen(400.0, 300.0).distance(app.last_canvas_rect.center()) < 0.001,
                "actual post-body transform maps the document centre to the new canvas centre"
            );
        }
    }
}

#[test]
fn real_hand_pan_and_native_title_position_changes_survive_same_size_frames_v79() {
    let (mut app, ctx, windows) = fixture(crate::theme::ThemeKind::StudioLight);
    let window = &windows[0];
    let size = vec2(500.0, 400.0);
    ordinary_frame(&mut app, &ctx, window, size);
    let at = pos2(250.0, 210.0);
    let shifted = at + vec2(45.0, 25.0);
    for events in [
        vec![Event::PointerMoved(at)],
        vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }],
        vec![Event::PointerMoved(shifted)],
        vec![Event::PointerButton { pos: shifted, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }],
    ] {
        frame(&mut app, &ctx, window, size, pos2(300.0, 200.0), false, false, events);
    }
    let panned = app.ui.views[0].clone();
    assert_ne!(panned.center, [99.0, 77.0], "real Hand gesture changes the view");
    assert_ne!(panned.center, [400.0, 300.0]);
    for origin in [pos2(300.0, 200.0), pos2(420.0, 250.0), pos2(150.0, 100.0)] {
        frame(&mut app, &ctx, window, size, origin, false, false, Vec::new());
        assert_eq!(app.ui.views[0], panned, "moving a native window/ordinary repaint is not a resize");
    }
    ordinary_frame(&mut app, &ctx, window, vec2(640.0, 480.0));
    assert_eq!(app.ui.views[0].center, [400.0, 300.0]);
    assert_eq!((app.ui.views[0].zoom, app.ui.views[0].rotation), (panned.zoom, panned.rotation));
}

#[test]
fn minimize_zero_and_restore_placeholder_frames_do_not_poison_view_or_resize_baseline_v79() {
    let (mut app, ctx, windows) = fixture(crate::theme::ThemeKind::ProMedium);
    let window = &windows[0];
    let size = vec2(500.0, 400.0);
    ordinary_frame(&mut app, &ctx, window, size);
    let before = app.ui.views[0].clone();
    frame(&mut app, &ctx, window, Vec2::ZERO, pos2(0.0, 0.0), true, true, Vec::new());
    assert_eq!(app.ui.views[0], before);
    ctx.data_mut(|data| {
        data.insert_temp(root_hidden_id(window.id), true);
        data.insert_temp(geometry_restore_pending_id(window.id), true);
    });
    ordinary_frame(&mut app, &ctx, window, vec2(160.0, 96.0));
    assert_eq!(app.ui.views[0], before, "valid-looking native restore fallback is not a user resize");
    ctx.data_mut(|data| {
        data.remove::<bool>(root_hidden_id(window.id));
        data.remove::<bool>(geometry_restore_pending_id(window.id));
    });
    ordinary_frame(&mut app, &ctx, window, size);
    assert_eq!(app.ui.views[0], before, "restored original size retains user pan and zoom");
    ordinary_frame(&mut app, &ctx, window, vec2(600.0, 450.0));
    assert_eq!(app.ui.views[0].center, [400.0, 300.0], "next genuine resize still works");
}

#[test]
fn first_frame_and_resize_tracking_are_isolated_by_viewport_and_document_v79() {
    let (mut app, ctx, windows) = fixture(crate::theme::ThemeKind::Classic);
    let first = app.ui.views[0].clone();
    let second = app.ui.views[1].clone();
    ordinary_frame(&mut app, &ctx, &windows[0], vec2(500.0, 400.0));
    ordinary_frame(&mut app, &ctx, &windows[1], vec2(640.0, 480.0));
    assert_eq!(app.ui.views[0], first);
    assert_eq!(app.ui.views[1], second, "another viewport's first body must not inherit the previous size stamp");
    ordinary_frame(&mut app, &ctx, &windows[1], vec2(700.0, 520.0));
    assert_eq!(app.ui.views[1].center, [160.0, 120.0]);
    assert_eq!((app.ui.views[1].zoom, app.ui.views[1].rotation), (second.zoom, second.rotation));
    assert_eq!(app.ui.views[0], first);
}
