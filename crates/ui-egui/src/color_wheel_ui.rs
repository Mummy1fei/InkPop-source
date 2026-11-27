//! Hue ring and a saturation/value triangle.
use egui::{Color32, Pos2, Sense, Stroke, Ui, Vec2, ecolor::Hsva, vec2};

/// Custom meshes do not get epaint's path antialiasing. Keep a one-physical-pixel
/// coverage fringe on both ring edges and the SV triangle, independent of DPI.
fn pixel_size(pixels_per_point: f32) -> f32 {
    if pixels_per_point.is_finite() && pixels_per_point > 0.0 { pixels_per_point.recip() } else { 1.0 }
}

fn ring_segments(radius: f32, pixels_per_point: f32) -> u32 {
    // Bound the polygon's radial error to about 0.1 physical pixel. The minimum
    // also retains the old hue interpolation density on the smallest panels.
    let physical_radius = radius / pixel_size(pixels_per_point);
    (std::f32::consts::TAU * (physical_radius.max(0.0) / 0.8).sqrt()).ceil().clamp(96.0, 1024.0) as u32
}

fn ring_mesh(center: Pos2, radius: f32, inner: f32, pixels_per_point: f32) -> egui::Mesh {
    let fringe = pixel_size(pixels_per_point) * 0.5;
    let segments = ring_segments(radius, pixels_per_point);
    let mut mesh = egui::Mesh::default();
    for i in 0..=segments {
        let h = i as f32 / segments as f32;
        let dir = Vec2::angled(h * std::f32::consts::TAU);
        let color = Color32::from(Hsva::new(h, 1.0, 1.0, 1.0));
        for (r, c) in [(radius + fringe, Color32::TRANSPARENT), (radius - fringe, color), (inner + fringe, color), (inner - fringe, Color32::TRANSPARENT)] {
            mesh.colored_vertex(center + dir * r, c);
        }
        if i > 0 {
            let previous = (i - 1) * 4;
            for band in 0..3 {
                let a = previous + band;
                mesh.add_triangle(a, a + 1, a + 5);
                mesh.add_triangle(a, a + 5, a + 4);
            }
        }
    }
    mesh
}

fn triangle_mesh(triangle: [Pos2; 3], hue: f32, pixels_per_point: f32) -> egui::Mesh {
    let colors = [Color32::from(Hsva::new(hue, 1.0, 1.0, 1.0)), Color32::WHITE, Color32::BLACK];
    let [a, b, c] = triangle;
    let orientation = ((b - a).x * (c - a).y - (b - a).y * (c - a).x).signum();
    let normal = |from: Pos2, to: Pos2| {
        let edge = (to - from).normalized();
        vec2(edge.y, -edge.x) * orientation
    };
    let mut mesh = egui::Mesh::default();
    let mut outer = [Pos2::ZERO; 3];
    for i in 0..3 {
        let p = triangle[i];
        let before = normal(triangle[(i + 2) % 3], p);
        let after = normal(p, triangle[(i + 1) % 3]);
        let bisector = (before + after).normalized();
        let offset = bisector * (pixel_size(pixels_per_point) * 0.5 / bisector.dot(after).max(0.25));
        let inset = p - offset;
        // Preserve the original triangle's affine RGB field as its opaque core
        // moves inward by half a pixel. The pointer/HSVA mapping stays unchanged.
        let w = weights(inset, triangle);
        let channel = |component: usize| colors.iter().zip(w).map(|(color, weight)| f32::from(color.to_array()[component]) * weight).sum::<f32>().round() as u8;
        mesh.colored_vertex(inset, Color32::from_rgb(channel(0), channel(1), channel(2)));
        outer[i] = p + offset;
    }
    for p in outer {
        mesh.colored_vertex(p, Color32::TRANSPARENT);
    }
    mesh.add_triangle(0, 1, 2);
    for i in 0..3 {
        let next = (i + 1) % 3;
        mesh.add_triangle(i, next, next + 3);
        mesh.add_triangle(i, next + 3, i + 3);
    }
    mesh
}

fn weights(p: Pos2, triangle: [Pos2; 3]) -> [f32; 3] {
    let [a, b, c] = triangle;
    let det = (b.y - c.y) * (a.x - c.x) + (c.x - b.x) * (a.y - c.y);
    if det.abs() < f32::EPSILON {
        return [0.0, 0.0, 1.0];
    }
    let x = ((b.y - c.y) * (p.x - c.x) + (c.x - b.x) * (p.y - c.y)) / det;
    let y = ((c.y - a.y) * (p.x - c.x) + (a.x - c.x) * (p.y - c.y)) / det;
    let mut w = [x.max(0.0), y.max(0.0), (1.0 - x - y).max(0.0)];
    let sum = w.iter().sum::<f32>().max(f32::EPSILON);
    for v in &mut w {
        *v /= sum;
    }
    w
}
pub(crate) fn show(ui: &mut Ui, hsv: &mut Hsva) -> bool {
    let side = ui.available_width().clamp(120.0, 240.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click_and_drag());
    ui.data_mut(|d| d.insert_temp(egui::Id::new("color-wheel-rect"), rect));
    let center = rect.center();
    let radius = side * 0.46;
    let inner = radius * 0.78;
    let tri = [center + vec2(inner * 0.90, 0.0), center + vec2(-inner * 0.45, -inner * 0.7794), center + vec2(-inner * 0.45, inner * 0.7794)];
    let mut changed = false;
    let id = response.id.with("ring-drag");
    if (response.dragged() || response.clicked() || response.drag_started())
        && let Some(p) = response.interact_pointer_pos()
    {
        if response.drag_started() || response.clicked() {
            ui.data_mut(|d| d.insert_temp(id, (p - center).length() > inner));
        }
        let ring = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or((p - center).length() > inner);
        if ring {
            hsv.h = (p - center).angle().rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU;
        } else {
            let [h, w, _] = weights(p, tri);
            hsv.v = (h + w).clamp(0.0, 1.0);
            hsv.s = if hsv.v > 0.0 { (h / hsv.v).clamp(0.0, 1.0) } else { 0.0 };
        }
        changed = true;
    }
    ui.painter().add(ring_mesh(center, radius, inner, ui.pixels_per_point()));
    ui.painter().add(triangle_mesh(tri, hsv.h, ui.pixels_per_point()));
    let hw = hsv.s * hsv.v;
    let ww = (1.0 - hsv.s) * hsv.v;
    let knob = tri[0] + (tri[1] - tri[0]) * ww + (tri[2] - tri[0]) * (1.0 - hw - ww);
    for p in [knob, center + Vec2::angled(hsv.h * std::f32::consts::TAU) * (radius + inner) * 0.5] {
        ui.painter().circle_stroke(p, 4.5, Stroke::new(2.5, Color32::BLACK));
        ui.painter().circle_stroke(p, 4.5, Stroke::new(1.0, Color32::WHITE));
    }
    changed
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vertices_and_outside_points_are_bounded() {
        let t = [egui::pos2(1.0, 0.0), egui::pos2(0.0, 0.0), egui::pos2(0.0, 1.0)];
        assert_eq!(weights(t[0], t), [1.0, 0.0, 0.0]);
        assert_eq!(weights(t[1], t), [0.0, 1.0, 0.0]);
        assert_eq!(weights(t[2], t), [0.0, 0.0, 1.0]);
        for p in [egui::pos2(30.0, -10.0), egui::pos2(-2.0, 3.0)] {
            let w = weights(p, t);
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-6);
            assert!(w.iter().all(|v| (0.0..=1.0).contains(v)));
        }
    }
}

#[cfg(test)]
#[path = "color_wheel_tests_v85.rs"]
mod tests_v85;
