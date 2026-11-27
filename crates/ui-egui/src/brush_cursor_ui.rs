//! Interpolated tip contours, cached independently of document edits and painted as paths.
use egui::{Color32, Painter, Pos2, Stroke, Vec2, vec2};
use photocraft_engine::{BrushSettings, paint::TipShape};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
};
#[derive(Clone)]
pub(crate) struct Contour {
    pub(crate) signature: u64,
    pub(crate) paths: Vec<Vec<Vec2>>,
}
pub(crate) const RESOLUTION: usize = 192;

// Marching squares: interpolate alpha crossings, rather than outlining square pixels.
// Shared grid-edge IDs connect crossings into paths (including holes and disconnected islands).
fn paths(alpha: &[f32], n: usize, threshold: f32) -> Vec<Vec<Vec2>> {
    if n < 2 || alpha.len() != n.saturating_mul(n) {
        return vec![];
    }
    let mut ids = HashMap::new();
    let mut points = Vec::new();
    let mut links: Vec<Vec<usize>> = Vec::new();
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            let corners = [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)];
            let values: Vec<_> = corners.iter().map(|&(x, y)| alpha.get(y * n + x).copied().unwrap_or(0.)).collect();
            let mut crossings = Vec::new();
            for i in 0..4 {
                let j = (i + 1) % 4;
                let (Some(&a), Some(&b), Some(&(ax, ay)), Some(&(bx, by))) = (values.get(i), values.get(j), corners.get(i), corners.get(j)) else { continue };
                if (a >= threshold) == (b >= threshold) {
                    continue;
                }
                let key = (ax.min(bx), ay.min(by), ax == bx);
                let id = *ids.entry(key).or_insert_with(|| {
                    let t = ((threshold - a) / (b - a)).clamp(0., 1.);
                    points.push(vec2(
                        (ax as f32 + (bx as f32 - ax as f32) * t + 0.5) / n as f32 - 0.5,
                        (ay as f32 + (by as f32 - ay as f32) * t + 0.5) / n as f32 - 0.5,
                    ));
                    links.push(Vec::new());
                    points.len() - 1
                });
                crossings.push(id);
            }
            // Resolve a saddle with the cell centre's coverage, retaining separate islands.
            if crossings.len() == 4 && values.iter().sum::<f32>() * 0.25 >= threshold {
                crossings.rotate_left(1);
            }
            for [a, b] in crossings.as_chunks::<2>().0 {
                if let Some(v) = links.get_mut(*a) {
                    v.push(*b);
                }
                if let Some(v) = links.get_mut(*b) {
                    v.push(*a);
                }
            }
        }
    }
    let mut out = Vec::new();
    for start in 0..points.len() {
        while links.get(start).is_some_and(|v| !v.is_empty()) {
            let mut path = Vec::new();
            let mut at = start;
            for _ in 0..=points.len() {
                let Some(&p) = points.get(at) else { break };
                path.push(p);
                let Some(next) = links.get_mut(at).and_then(Vec::pop) else { break };
                if let Some(v) = links.get_mut(next) {
                    v.retain(|i| *i != at);
                }
                at = next;
                if at == start {
                    if let Some(&p) = points.get(start) {
                        path.push(p);
                    }
                    break;
                }
            }
            // Remove straight runs; preserve the subpixel positions on curved edges.
            let mut simple = Vec::new();
            for &p in &path {
                while simple.len() >= 2 {
                    let a: Vec2 = simple.get(simple.len() - 2).copied().unwrap_or(p);
                    let b: Vec2 = simple.last().copied().unwrap_or(p);
                    let ab = b - a;
                    let bc = p - b;
                    if ab.dot(bc) > 0. && (ab.x * bc.y - ab.y * bc.x).abs() < 0.0000001 {
                        simple.pop();
                    } else {
                        break;
                    }
                }
                simple.push(p);
            }
            if simple.len() > 1 {
                out.push(simple);
            }
        }
    }
    out
}
pub(crate) fn contour(ctx: &egui::Context, b: &BrushSettings, normal: bool) -> Arc<Contour> {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    match &b.tip {
        TipShape::Round => 0usize.hash(&mut hash),
        TipShape::Sampled(t) => {
            (t.data.as_ptr() as usize, t.width, t.height, t.data.len()).hash(&mut hash);
            // A small content fingerprint catches in-place updates without scanning large tips.
            for i in (0..t.data.len()).step_by((t.data.len() / 64).max(1)) {
                t.data.get(i).hash(&mut hash);
            }
        }
    }
    (b.hardness.to_bits(), b.angle.to_bits(), b.roundness.to_bits(), b.flip_x, b.flip_y, normal).hash(&mut hash);
    let signature = hash.finish();
    let id = egui::Id::new("brush-tip-contour");
    let cached: Option<Arc<Contour>> = ctx.data(|d| d.get_temp(id));
    cached.filter(|c| c.signature == signature).unwrap_or_else(|| {
        let alpha = crate::brush_preview::tip_alpha(&b.tip, b.hardness, b.angle, b.roundness, (b.flip_x, b.flip_y), RESOLUTION as u32);
        let c = Arc::new(Contour { signature, paths: paths(&alpha, RESOLUTION, if normal { 0.5 } else { 0.01 }) });
        ctx.data_mut(|d| d.insert_temp(id, c.clone()));
        c
    })
}
pub(crate) fn paint(painter: &Painter, at: Pos2, b: &BrushSettings, xf: &crate::canvas::ViewXform, normal: bool, _revision: usize) -> bool {
    let contour = contour(painter.ctx(), b, normal);
    if contour.paths.is_empty() {
        return false;
    }
    let size = (b.size * xf.zoom).clamp(1., 100000.) * RESOLUTION as f32 / (RESOLUTION - 2) as f32;
    let rotation = egui::emath::Rot2::from_angle(xf.rotation.to_radians());
    for (width, color) in [(2., Color32::from_black_alpha(160)), (1., Color32::from_white_alpha(230))] {
        for path in &contour.paths {
            painter.add(egui::Shape::line(
                path.iter().map(|p| at + rotation * vec2(if xf.flip { -p.x } else { p.x }, p.y) * size).collect(),
                Stroke::new(width, color),
            ));
        }
    }
    true
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonround_cursor_contour_rotates_and_mirrors_with_the_canvas() {
        let ctx = egui::Context::default();
        let at = egui::pos2(200.0, 200.0);
        let brush = BrushSettings { size: 80.0, roundness: 0.3, angle: 17.0, ..Default::default() };
        let flat = crate::canvas::ViewXform {
            rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0)),
            center: [100.0, 50.0],
            zoom: 1.5,
            rotation: 0.0,
            flip: false,
        };
        let points = |xf: &crate::canvas::ViewXform| {
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(flat.rect), ..Default::default() }, |ui| {
                assert!(paint(ui.painter(), at, &brush, xf, true, 0));
            });
            // This geometry-only test has no renderer to consume font/texture uploads.
            out.textures_delta.clear();
            out.shapes
                .into_iter()
                .find_map(|s| match s.shape {
                    egui::Shape::Path(p) => Some(p.points),
                    _ => None,
                })
                .expect("contour path")
        };
        let original = points(&flat);
        for rotation in [23.0_f32, 90.0] {
            for flip in [false, true] {
                let rotated = points(&crate::canvas::ViewXform { rotation, flip, ..flat });
                let matrix = egui::emath::Rot2::from_angle(rotation.to_radians());
                assert_eq!(rotated.len(), original.len());
                for (a, b) in original.iter().zip(rotated) {
                    let d = *a - at;
                    let expected = at + matrix * vec2(if flip { -d.x } else { d.x }, d.y);
                    assert!(b.distance(expected) < 0.001);
                }
            }
        }
    }
    #[test]
    fn interpolated_contours_keep_holes_and_separate_islands() {
        assert!(paths(&[0.; 16], 4, 0.5).is_empty());
        assert!(paths(&[], 0, 0.5).is_empty());
        let alpha: Vec<_> = (0..81)
            .map(|i| {
                if (1..8).contains(&(i % 9)) && (1..8).contains(&(i / 9)) && !(3..6).contains(&(i % 9))
                    || (1..8).contains(&(i % 9)) && (1..8).contains(&(i / 9)) && !(3..6).contains(&(i / 9))
                {
                    1.
                } else {
                    0.
                }
            })
            .collect();
        let p = paths(&alpha, 9, 0.5);
        assert_eq!(p.len(), 2);
        assert!(p.iter().all(|p| p.first() == p.last()));
        assert!(p.iter().flatten().all(|p| p.x.abs() <= 0.5 && p.y.abs() <= 0.5));
    }
    #[test]
    fn round_tip_is_one_smooth_path_and_drawing_does_not_invalidate_it() {
        let alpha = crate::brush_preview::tip_alpha(&TipShape::Round, 1., 33., 0.7, (false, false), 192);
        let p = paths(&alpha, 192, 0.5);
        assert_eq!(p.len(), 1);
        assert!(p[0].windows(2).any(|v| (v[1].x - v[0].x).abs() > 0.00001 && (v[1].y - v[0].y).abs() > 0.00001));
        let ctx = egui::Context::default();
        let painter = ctx.layer_painter(egui::LayerId::background());
        let b = BrushSettings::default();
        let xf = crate::canvas::ViewXform {
            rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(100.0, 100.0)),
            center: [50.0, 50.0],
            zoom: 1.0,
            flip: false,
            rotation: 0.0,
        };
        paint(&painter, Pos2::ZERO, &b, &xf, true, 0);
        let a: Arc<Contour> = ctx.data(|d| d.get_temp(egui::Id::new("brush-tip-contour"))).unwrap();
        paint(&painter, Pos2::ZERO, &b, &crate::canvas::ViewXform { rotation: 45.0, flip: true, ..xf }, true, 500);
        let c: Arc<Contour> = ctx.data(|d| d.get_temp(egui::Id::new("brush-tip-contour"))).unwrap();
        assert!(Arc::ptr_eq(&a, &c));
    }
}
