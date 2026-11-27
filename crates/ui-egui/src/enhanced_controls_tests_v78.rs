//! Synthetic native input through the real canvas; not physical touch/trackpad latency proof.
use super::*;
use egui::{Event, RawInput};

fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, view: &mut View, events: Vec<Event>) {
    let rect = Rect::from_min_size(pos2(32.0, 32.0), vec2(200.0, 150.0));
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 250.0))), events, ..Default::default() };
    ctx.run_ui(input, |ui| {
        *view = canvas_view(app, ui, 0, rect, view.clone(), true);
    })
    .textures_delta
    .clear();
}

fn fixture() -> (PhotocraftApp, egui::Context, View) {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":400,"height":300})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    let mut view = View { rotation: 0.0, center: [200.0, 150.0], zoom: 2.0, doc_size: [400, 300], fit_pending: false };
    frame(&mut app, &ctx, &mut view, vec![Event::PointerMoved(pos2(100.0, 100.0))]);
    (app, ctx, view)
}

#[test]
fn pinch_navigation_honours_both_gates_without_blocking_mouse_wheel_v78() {
    for (touch, pinch) in [(true, true), (false, true), (true, false), (false, false)] {
        let (mut app, ctx, mut view) = fixture();
        app.run("prefs.set", json!({"values":{"enhancedControls.touchGestures":touch,"enhancedControls.zoomWithTrackpadPinch":pinch}})).unwrap();
        let before = view.zoom;
        frame(&mut app, &ctx, &mut view, vec![Event::PointerMoved(pos2(100.0, 100.0)), Event::Zoom(1.25)]);
        assert_eq!(view.zoom > before, touch && pinch, "touch={touch},pinch={pinch}");
        let before = view.center;
        frame(
            &mut app,
            &ctx,
            &mut view,
            vec![
                Event::PointerMoved(pos2(100.0, 100.0)),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, 20.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        assert_ne!(view.center, before, "ordinary wheel pan remains available");
        let before = view.zoom;
        frame(
            &mut app,
            &ctx,
            &mut view,
            vec![
                Event::PointerMoved(pos2(100.0, 100.0)),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: vec2(0.0, 20.0),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::CTRL,
                },
            ],
        );
        assert!(view.zoom > before, "Ctrl-wheel zoom must remain available");
    }
}

#[test]
fn trackpad_rotation_requires_master_and_rotation_option_and_preserves_pixels_v78() {
    for (touch, rotate) in [(true, true), (false, true), (true, false), (false, false)] {
        let (mut app, ctx, mut view) = fixture();
        let revision = app.session.active().unwrap().revision;
        app.run("prefs.set", json!({"values":{"enhancedControls.touchGestures":touch,"enhancedControls.rotateViewWithTrackpad":rotate}})).unwrap();
        frame(&mut app, &ctx, &mut view, vec![Event::PointerMoved(pos2(100.0, 100.0)), Event::Rotate(std::f32::consts::FRAC_PI_6)]);
        if touch && rotate {
            assert!((view.rotation - 30.0).abs() < 1e-4);
        } else {
            assert_eq!(view.rotation, 0.0);
        }
        assert_eq!(app.session.active().unwrap().revision, revision, "view rotation never edits document pixels");
    }
}

#[test]
fn zoom_click_center_preference_uses_actual_clicked_document_point_v78() {
    for center_click in [false, true] {
        for flipped in [false, true] {
            for degrees in [0.0, 37.0] {
                for out in [false, true] {
                    let (mut app, ctx, mut view) = fixture();
                    app.ui.tool = Tool::Zoom;
                    app.ui.view.flip_horizontal = flipped;
                    app.run("prefs.set", json!({"path":"tools.zoomClickedPointToCenter","value":center_click})).unwrap();
                    view.rotation = degrees;
                    let rect = Rect::from_min_size(pos2(32.0, 32.0), vec2(200.0, 150.0));
                    let point = pos2(105.0, 92.0);
                    frame(&mut app, &ctx, &mut view, vec![Event::PointerMoved(point)]);
                    let before = ViewXform { rect, zoom: view.zoom, center: view.center, rotation: degrees, flip: flipped };
                    let doc = before.to_doc(point);
                    let zoom = view.zoom;
                    let mods = if out { egui::Modifiers::ALT } else { egui::Modifiers::NONE };
                    frame(
                        &mut app,
                        &ctx,
                        &mut view,
                        vec![
                            Event::PointerMoved(point),
                            Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: true, modifiers: mods },
                        ],
                    );
                    frame(
                        &mut app,
                        &ctx,
                        &mut view,
                        vec![
                            Event::PointerMoved(point),
                            Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: false, modifiers: mods },
                        ],
                    );
                    assert_eq!(view.zoom > zoom, !out);
                    let after = ViewXform { zoom: view.zoom, center: view.center, ..before };
                    let expected = if center_click { rect.center() } else { point };
                    let actual = after.to_screen(doc[0] as f32, doc[1] as f32);
                    assert!(actual.distance(expected) < 0.001, "center={center_click},flip={flipped},rotation={degrees},out={out}: {actual:?} != {expected:?}");
                }
            }
        }
    }
}
