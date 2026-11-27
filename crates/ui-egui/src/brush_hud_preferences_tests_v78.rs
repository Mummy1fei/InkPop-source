use super::*;
use photocraft_engine::paint::{GrayTile, TipShape};

const CTRL_ALT: egui::Modifiers = egui::Modifiers { ctrl: true, alt: true, shift: false, mac_cmd: false, command: false };

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":120,"height":100,"background":"transparent"})).unwrap();
    app.run("tools.setBrush", json!({"brush":{"size":40,"hardness":0.2}})).unwrap();
    app.ui.tool = Tool::Brush;
    app
}

#[test]
fn hud_hardness_preference_changes_only_computed_round_tips_and_keeps_size_drag_v78() {
    for enabled in [false, true] {
        for sampled in [false, true] {
            let mut app = app();
            app.run("prefs.set", json!({"path":"tools.varyRoundBrushHardnessOnHud","value":enabled})).unwrap();
            if sampled {
                app.session.tools.brush.tip = TipShape::Sampled(GrayTile::from_fn(2, 2, |_, _| 1.0));
            }
            let before = app.session.active().unwrap().history.past_len();
            assert!(pointer(&mut app, ToolEvent::Down { x: 40.0, y: 40.0, pressure: 1.0 }, CTRL_ALT, false));
            assert!(pointer(&mut app, ToolEvent::Move { x: 55.0, y: 100.0, pressure: 1.0 }, CTRL_ALT, false));
            assert!(pointer(&mut app, ToolEvent::Up { x: 60.0, y: 140.0 }, CTRL_ALT, false));
            assert_eq!(app.session.tools.brush.size, 80.0);
            assert_eq!(app.session.tools.brush.hardness, if enabled && !sampled { 0.7 } else { 0.2 });
            assert_eq!(app.session.active().unwrap().history.past_len(), before);
            assert!(app.brush_resize.is_none());
        }
    }
}

#[test]
fn hud_preview_mesh_uses_configured_color_and_red_fallback_v78() {
    for (configured, rgb) in [("#1976d2", [25, 118, 210]), ("#77cc33", [119, 204, 51]), ("invalid", [255, 0, 0])] {
        let mut app = app();
        // A defensive fallback also handles an invalid legacy/programmatic preference value.
        app.session.edit_prefs(|prefs| prefs.cursors.brush_preview_color = configured.into());
        pointer(&mut app, ToolEvent::Down { x: 40.0, y: 40.0, pressure: 1.0 }, CTRL_ALT, false);
        let ctx = egui::Context::default();
        crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::StudioLight);
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        let expected = Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], 110);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let xf = ViewXform { rotation: 0.0, rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(300.0, 220.0)), zoom: 1.0, center: [40.0, 40.0], flip: false };
            assert!(draw(&app, ui.painter(), &xf));
        });
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Mesh(mesh) if mesh.vertices.iter().any(|v|v.color==expected))),
            "preview mesh must use {configured}"
        );
        output.textures_delta.clear();
    }
}
