//! Rotate the camera, never the image. Gesture state is private to each viewport/document.
use crate::{PhotocraftApp, canvas::ViewXform, state::View};
use egui::{Pos2, Response};
use serde_json::{Value, json};

pub fn normalized(degrees: f32) -> f32 {
    if degrees.is_finite() { (degrees + 180.0).rem_euclid(360.0) - 180.0 } else { 0.0 }
}

pub fn set(app: &mut PhotocraftApp, angle: f32) -> Result<Value, String> {
    let idx = app.session.active_index().ok_or("no document")?;
    let angle = normalized(angle);
    if let Some(v) = app.ui.views.get_mut(idx) {
        v.rotation = angle;
    }
    for w in &mut app.ui.windows {
        if w.document == idx || app.ui.rotate_all_windows {
            w.view.rotation = angle;
        }
    }
    if app.ui.rotate_all_windows {
        for v in &mut app.ui.views {
            v.rotation = angle;
        }
    }
    Ok(json!({"rotation": angle}))
}

pub fn invoke(app: &mut PhotocraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    match id {
        "view.rotate" if p.get("angle").is_none() => {
            app.ui.tool = crate::state::Tool::RotateView;
            Some(Ok(json!({"tool": "RotateView"})))
        }
        "view.rotate" => Some(match p.get("angle").and_then(Value::as_f64).filter(|n| n.is_finite() && n.abs() <= 1_000_000.0) {
            Some(a) => set(app, a as f32),
            None => Err("angle must be a finite number of degrees".into()),
        }),
        "view.resetRotation" => Some(set(app, 0.0)),
        _ => None,
    }
}

pub fn options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let mut angle = app.session.active_index().and_then(|i| app.ui.views.get(i)).map_or(0.0, |v| v.rotation);
    ui.label(tl!("Rotation Angle"));
    if ui.add(egui::DragValue::new(&mut angle).speed(0.5).suffix("°").range(-180.0..=180.0)).changed() {
        let _ = set(app, angle);
    }
    if crate::widgets::secondary_button(ui, tl!("Reset View"), 0.0).clicked() {
        let _ = set(app, 0.0);
    }
    crate::widgets::checkbox(ui, &mut app.ui.rotate_all_windows, tl!("Rotate All Windows"));
}

#[derive(Clone, Copy)]
struct Drag {
    pointer_angle: f32,
    rotation: f32,
}

fn bearing(p: Pos2, center: Pos2) -> Option<f32> {
    let d = p - center;
    (d.length_sq() >= 64.0).then(|| d.y.atan2(d.x).to_degrees())
}

pub fn input(app: &mut PhotocraftApp, response: &Response, xf: &ViewXform, idx: usize, view: &mut View, enabled: bool) {
    let ctx = &response.ctx;
    let key = egui::Id::new(("rotate-view-drag", ctx.viewport_id(), idx));
    let hover_retry = key.with("cursor-hover-retry");
    if !enabled {
        ctx.data_mut(|d| {
            d.remove::<Drag>(key);
            d.remove::<bool>(hover_retry);
        });
        return;
    }
    // Changing theme families rebuilds the canvas widget ID. egui hit testing uses the
    // previous pass, so a stationary pointer can miss hover for that first pass. Schedule
    // one follow-up frame, without drawing through an overlapping panel or another drag.
    // The latch prevents an idle repaint loop if hover cannot resolve for another reason.
    let hover_pending = !response.hovered()
        && !response.dragged()
        && response.enabled()
        && !ctx.input(|i| i.pointer.any_down())
        && ctx.rect_contains_pointer(response.layer_id, response.rect);
    if hover_pending {
        if !ctx.data(|d| d.get_temp::<bool>(hover_retry)).unwrap_or(false) {
            ctx.data_mut(|d| d.insert_temp(hover_retry, true));
            ctx.request_repaint();
        }
    } else {
        ctx.data_mut(|d| d.remove::<bool>(hover_retry));
    }
    if (response.hovered() || response.dragged())
        && let Some(p) = ctx.input(|i| i.pointer.hover_pos())
    {
        // Rotate View bypasses the canvas's painting-tool branch, including its generic
        // glyph cursor. Draw here so both primary and passive document windows show the
        // same rotation pointer, also while a captured drag moves beyond the canvas.
        // Hover does not require keyboard focus; an unfocused floating view can be hovered.
        // The shared icon overlay is viewport-scoped and contrasts against light/dark art.
        crate::icons::cursor(ctx, crate::icons::tool_icon(crate::state::Tool::RotateView), p, egui::Vec2::splat(0.5), 22.0);
        ctx.set_cursor_icon(if ctx.output(|o| o.cursor_image.is_some()) { egui::CursorIcon::Crosshair } else { egui::CursorIcon::None });
    }
    if !ctx.input(|i| i.focused) {
        ctx.data_mut(|d| d.remove::<Drag>(key));
        return;
    }
    if response.double_clicked() {
        view.rotation = 0.0;
    }
    if response.drag_started() {
        let start = ctx.input(|i| i.pointer.press_origin());
        if let Some(a) = start.and_then(|p| bearing(p, xf.rect.center())) {
            ctx.data_mut(|d| d.insert_temp(key, Drag { pointer_angle: a, rotation: view.rotation }));
        }
    }
    if (response.dragged() || response.drag_stopped())
        && let Some(p) = response.interact_pointer_pos()
        && let Some(a) = bearing(p, xf.rect.center())
    {
        let mut drag = ctx.data(|d| d.get_temp::<Drag>(key)).unwrap_or(Drag { pointer_angle: a, rotation: view.rotation });
        drag.rotation += normalized(a - drag.pointer_angle);
        drag.pointer_angle = a;
        view.rotation = normalized(if ctx.input(|i| i.modifiers.shift) { (drag.rotation / 15.0).round() * 15.0 } else { drag.rotation });
        ctx.data_mut(|d| d.insert_temp(key, drag));
        ctx.request_repaint();
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
    if response.drag_stopped() || !ctx.input(|i| i.pointer.primary_down()) {
        ctx.data_mut(|d| d.remove::<Drag>(key));
    }
    if view.rotation != xf.rotation {
        ctx.request_repaint();
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
    if view.rotation != xf.rotation && app.ui.rotate_all_windows {
        for v in &mut app.ui.views {
            v.rotation = view.rotation;
        }
        for w in &mut app.ui.windows {
            w.view.rotation = view.rotation;
        }
    }
}

/// Texture coordinates stay in document order, including mirrored and rotated views.
pub fn image(painter: &egui::Painter, xf: &ViewXform, bounds: photocraft_geom::Rect, texture: egui::TextureId, uv: egui::Rect) {
    let mut mesh = egui::Mesh::with_texture(texture);
    for (pos, uv) in xf.doc_corners(bounds).into_iter().zip([uv.left_top(), uv.right_top(), uv.right_bottom(), uv.left_bottom()]) {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: egui::Color32::WHITE });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(mesh);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotated_mirrored_mapping_roundtrips_and_encloses_all_corners() {
        for rotation in [0.0, 23.0, 90.0, -138.0, 180.0] {
            for flip in [false, true] {
                let xf = ViewXform {
                    rect: egui::Rect::from_min_size(egui::pos2(30.0, 50.0), egui::vec2(500.0, 300.0)),
                    zoom: 2.3,
                    center: [80.0, 40.0],
                    rotation,
                    flip,
                };
                let (min, max) = xf.visible_bounds();
                for p in [xf.rect.left_top(), xf.rect.right_top(), xf.rect.right_bottom(), xf.rect.left_bottom()] {
                    let d = xf.to_doc(p);
                    assert!(d[0] >= min[0] && d[0] <= max[0] && d[1] >= min[1] && d[1] <= max[1]);
                    assert!(xf.to_screen(d[0] as f32, d[1] as f32).distance(p) < 0.001);
                }
            }
        }
    }
    #[test]
    fn rotate_commands_leave_pixels_history_and_zoom_unchanged() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":100,"height":80})).unwrap();
        app.ui.views.resize(1, View::default());
        let before = app.session.active().unwrap().revision;
        let view = app.ui.views[0].clone();
        invoke(&mut app, "view.rotate", &json!({"angle":23})).unwrap().unwrap();
        assert_eq!(app.ui.views[0].rotation, 23.0);
        assert_eq!(app.ui.views[0].zoom, view.zoom);
        assert_eq!(app.session.active().unwrap().revision, before);
        invoke(&mut app, "view.resetRotation", &json!({})).unwrap().unwrap();
        assert_eq!(app.ui.views[0], view);
        assert!(invoke(&mut app, "view.rotate", &json!({"angle":"bad"})).unwrap().is_err());
        assert_eq!(crate::state::Tool::RotateView.key(), 'R');
    }
}
