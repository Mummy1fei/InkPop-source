//! The translucent footprint trail the retouching tools (Dodge, Blur, Clone Stamp, …) and Quick
//! Selection show while dragging: the union of the brush's round footprint along the drag.
//!
//! It is a coverage mask in document space, stamped segment by segment as the pointer moves and
//! drawn as one texture over the document. It used to be an egui polyline as wide as the brush:
//! egui tessellates a polyline whose width is much larger than its steps (any zoomed-in drag)
//! into hard pie wedges fanning out across the canvas (#189), and translucent overlaps doubled up.

use egui::{Color32, ColorImage, Rect, TextureHandle, TextureOptions, pos2};

/// Longest side of the mask, in mask pixels (big documents are stamped at a reduced scale).
const MAX_SIDE: u32 = 1024;
const LOCAL_MAX_SIDE: u32 = 2048;
const LOCAL_BLOCK: f64 = 128.0;

/// A drag's footprint mask.
pub struct Trail {
    /// Document size the mask covers.
    doc: [u32; 2],
    /// Local trails grow in block-sized regions rather than downsampling the whole canvas.
    origin: [u32; 2],
    local: bool,
    /// Document pixels per mask pixel.
    k: f32,
    w: usize,
    h: usize,
    /// Coverage 0..=255 per mask pixel.
    cov: Vec<u8>,
    /// Drag points stamped so far.
    fed: usize,
    /// Mask pixels changed since the last upload (x0, y0, x1, y1), if any.
    dirty: Option<[usize; 4]>,
    tex: Option<TextureHandle>,
    diameter: Option<f32>,
    color: Option<Color32>,
}

impl Trail {
    pub fn new(doc: [u32; 2]) -> Self {
        let k = doc[0].max(doc[1]).div_ceil(MAX_SIDE).max(1);
        let (w, h) = (doc[0].div_ceil(k).max(1) as usize, doc[1].div_ceil(k).max(1) as usize);
        Self { doc, origin: [0, 0], local: false, k: k as f32, w, h, cov: vec![0; w * h], fed: 0, dirty: None, tex: None, diameter: None, color: None }
    }

    /// Fine round-area feedback for healing and quick selection. A short stroke on
    /// a 24MP image keeps document-pixel accuracy without uploading a 24MP texture.
    pub fn local(doc: [u32; 2]) -> Self {
        Self { doc, origin: [0, 0], local: true, k: 1.0, w: 0, h: 0, cov: Vec::new(), fed: 0, dirty: None, tex: None, diameter: None, color: None }
    }

    fn fit_local(&mut self, points: &[[f64; 3]], diameter: f32) {
        let radius = f64::from(diameter) * 0.5 + 1.0;
        let incremental = self.w > 0 && self.h > 0 && self.diameter == Some(diameter) && self.fed <= points.len();
        let mut extent = if incremental {
            [
                f64::from(self.origin[0]),
                f64::from(self.origin[1]),
                f64::from(self.origin[0]) + self.w as f64 * f64::from(self.k),
                f64::from(self.origin[1]) + self.h as f64 * f64::from(self.k),
            ]
        } else {
            [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY]
        };
        for point in points.iter().skip(if incremental { self.fed } else { 0 }).filter(|point| point[0].is_finite() && point[1].is_finite()) {
            extent[0] = extent[0].min(point[0] - radius);
            extent[1] = extent[1].min(point[1] - radius);
            extent[2] = extent[2].max(point[0] + radius);
            extent[3] = extent[3].max(point[1] + radius);
        }
        if !extent.iter().all(|value| value.is_finite()) {
            return;
        }
        let bounds = [
            (extent[0] / LOCAL_BLOCK).floor().mul_add(LOCAL_BLOCK, 0.0).clamp(0.0, f64::from(self.doc[0])) as u32,
            (extent[1] / LOCAL_BLOCK).floor().mul_add(LOCAL_BLOCK, 0.0).clamp(0.0, f64::from(self.doc[1])) as u32,
            (extent[2] / LOCAL_BLOCK).ceil().mul_add(LOCAL_BLOCK, 0.0).clamp(0.0, f64::from(self.doc[0])) as u32,
            (extent[3] / LOCAL_BLOCK).ceil().mul_add(LOCAL_BLOCK, 0.0).clamp(0.0, f64::from(self.doc[1])) as u32,
        ];
        let width = bounds[2].saturating_sub(bounds[0]);
        let height = bounds[3].saturating_sub(bounds[1]);
        if width == 0 || height == 0 {
            return;
        }
        let k = width.max(height).div_ceil(LOCAL_MAX_SIDE).max(1);
        let (w, h) = (width.div_ceil(k) as usize, height.div_ceil(k) as usize);
        if self.origin == [bounds[0], bounds[1]] && self.w == w && self.h == h && self.k == k as f32 {
            return;
        }
        self.origin = [bounds[0], bounds[1]];
        self.k = k as f32;
        self.w = w;
        self.h = h;
        self.cov = vec![0; w * h];
        self.fed = 0;
        self.dirty = None;
        self.tex = None;
    }

    /// Coverage at a mask pixel (tests).
    pub fn coverage(&self, x: usize, y: usize) -> u8 {
        if x < self.w && y < self.h { self.cov.get(y * self.w + x).copied().unwrap_or(0) } else { 0 }
    }

    /// Document pixels per mask pixel.
    pub fn scale(&self) -> f32 {
        self.k
    }

    /// Stamp the drag points not seen yet: round footprints of `diameter` document pixels joined
    /// into a capsule per segment (coverage is the max, so overlaps don't add up).
    pub fn feed(&mut self, points: &[[f64; 3]], diameter: f32) {
        let diameter = if diameter.is_finite() { diameter.clamp(1.0, photocraft_engine::paint::MAX_BRUSH_SIZE) } else { 1.0 };
        if self.fed >= points.len() && self.diameter == Some(diameter) {
            return;
        }
        if self.local {
            self.fit_local(points, diameter);
        }
        if self.diameter != Some(diameter) {
            self.cov.fill(0);
            self.fed = 0;
            self.dirty = None;
            self.tex = None;
            self.diameter = Some(diameter);
        }
        if self.fed >= points.len() {
            return;
        }
        let k = f64::from(self.k);
        let r = (f64::from(diameter.max(1.0)) / 2.0) / k;
        let origin = self.origin;
        let mask = |p: &[f64; 3]| [(p[0] - f64::from(origin[0])) / k, (p[1] - f64::from(origin[1])) / k];
        for i in self.fed..points.len() {
            let Some(b) = points.get(i).map(mask) else { break };
            let a = i.checked_sub(1).and_then(|j| points.get(j)).map_or(b, mask);
            self.capsule(a, b, r);
        }
        self.fed = points.len();
    }

    fn capsule(&mut self, a: [f64; 2], b: [f64; 2], r: f64) {
        if !(a.iter().chain(&b).all(|v| v.is_finite()) && r.is_finite()) {
            return;
        }
        let x0 = (a[0].min(b[0]) - r - 1.0).floor().max(0.0);
        let y0 = (a[1].min(b[1]) - r - 1.0).floor().max(0.0);
        let x1 = (a[0].max(b[0]) + r + 1.0).ceil().min(self.w as f64);
        let y1 = (a[1].max(b[1]) + r + 1.0).ceil().min(self.h as f64);
        if x1 <= x0 || y1 <= y0 {
            return;
        }
        let (x0, y0, x1, y1) = (x0 as usize, y0 as usize, x1 as usize, y1 as usize);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (x as f64 + 0.5 - a[0], y as f64 + 0.5 - a[1]);
                let t = if len2 > 0.0 { ((px * dx + py * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
                let d = (px - t * dx).hypot(py - t * dy);
                // One mask pixel of anti-aliasing at the edge.
                let c = ((r + 0.5 - d).clamp(0.0, 1.0) * 255.0).round() as u8;
                if let Some(v) = self.cov.get_mut(y * self.w + x)
                    && c > *v
                {
                    *v = c;
                }
            }
        }
        let d = self.dirty.get_or_insert([x0, y0, x1, y1]);
        *d = [d[0].min(x0), d[1].min(y0), d[2].max(x1), d[3].max(y1)];
    }

    fn image(&self, r: [usize; 4], color: Color32) -> ColorImage {
        let [x0, y0, x1, y1] = r;
        let [red, green, blue, alpha] = color.to_srgba_unmultiplied();
        let mut px = Vec::with_capacity((x1 - x0) * (y1 - y0));
        for y in y0..y1 {
            for x in x0..x1 {
                let c = self.coverage(x, y);
                px.push(Color32::from_rgba_unmultiplied(red, green, blue, ((u16::from(c) * u16::from(alpha)) / 255) as u8));
            }
        }
        ColorImage::new([x1 - x0, y1 - y0], px)
    }

    /// Upload only changed mask pixels, then map the texture's four document corners through
    /// the same view transform as the canvas, including rotation and mirroring.
    pub fn draw(&mut self, painter: &egui::Painter, xf: &crate::canvas::ViewXform, color: Color32) {
        if self.w == 0 || self.h == 0 {
            return;
        }
        if self.color != Some(color) {
            self.tex = None;
            self.color = Some(color);
        }
        let opts = TextureOptions::LINEAR;
        let dirty = self.dirty.take();
        if self.tex.is_none() {
            let img = self.image([0, 0, self.w, self.h], color);
            self.tex = Some(painter.ctx().load_texture("stroke-trail", img, opts));
        } else if let Some(r) = dirty {
            let img = self.image(r, color);
            if let Some(t) = self.tex.as_mut() {
                t.set_partial([r[0], r[1]], img, opts);
            }
        }
        let Some(t) = &self.tex else { return };
        // The mask's last column/row may reach past the document (sizes rounded up).
        let right = (self.origin[0] as f32 + self.w as f32 * self.k).min(self.doc[0] as f32);
        let bottom = (self.origin[1] as f32 + self.h as f32 * self.k).min(self.doc[1] as f32);
        let (u, v) = ((right - self.origin[0] as f32) / (self.w as f32 * self.k), (bottom - self.origin[1] as f32) / (self.h as f32 * self.k));
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(u, v));
        let bounds = photocraft_geom::Rect::new(
            self.origin[0].min(i32::MAX as u32) as i32,
            self.origin[1].min(i32::MAX as u32) as i32,
            right.min(i32::MAX as f32) as i32,
            bottom.min(i32::MAX as f32) as i32,
        );
        crate::rotate_view::image(painter, xf, bounds, t.id(), uv);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotated_trail_mesh_follows_document_corners_without_double_mirroring() {
        let ctx = egui::Context::default();
        let mut trail = Trail::new([200, 100]);
        trail.feed(&[[20.0, 30.0, 1.0], [100.0, 70.0, 1.0]], 20.0);
        for rotation in [23.0, 90.0] {
            for flip in [false, true] {
                let xf = crate::canvas::ViewXform {
                    rect: Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(500.0, 400.0)),
                    center: [100.0, 50.0],
                    zoom: 1.5,
                    rotation,
                    flip,
                };
                let mut output =
                    ctx.run_ui(egui::RawInput { screen_rect: Some(xf.rect), ..Default::default() }, |ui| trail.draw(ui.painter(), &xf, Color32::WHITE));
                // Inspect the submitted mesh without rendering its texture uploads.
                output.textures_delta.clear();
                let mesh = output
                    .shapes
                    .iter()
                    .find_map(|s| match &s.shape {
                        egui::Shape::Mesh(m) if m.vertices.len() == 4 => Some(m),
                        _ => None,
                    })
                    .expect("trail texture mesh");
                let expected = xf.doc_corners(photocraft_geom::Rect::new(0, 0, 200, 100));
                for (vertex, pos) in mesh.vertices.iter().zip(expected) {
                    assert!(vertex.pos.distance(pos) < 0.001);
                }
                assert_eq!(mesh.vertices[0].uv, pos2(0.0, 0.0));
                assert_eq!(mesh.vertices[2].uv, pos2(1.0, 1.0));
            }
        }
    }

    #[test]
    fn the_trail_is_the_union_of_round_footprints_along_the_drag() {
        let mut t = Trail::new([200, 100]);
        // Many tiny steps with a wide brush (what zoomed-in drags send).
        let pts: Vec<[f64; 3]> = (0..200).map(|i| [40.0 + f64::from(i) * 0.5, 50.0 + (f64::from(i) * 0.3).sin(), 1.0]).collect();
        t.feed(&pts[..100], 40.0);
        t.feed(&pts, 40.0);
        assert_eq!(t.coverage(90, 50), 255, "on the path");
        assert_eq!(t.coverage(90, 50 + 15), 255, "inside the footprint");
        assert_eq!(t.coverage(90, 50 + 25), 0, "outside the footprint: no wedges reaching out");
        assert_eq!(t.coverage(5, 50), 0, "nothing behind the start");
        assert_eq!(t.coverage(170, 50), 0, "nothing past the end");
        // Every covered pixel lies within the brush radius of the path.
        for y in 0..100 {
            for x in 0..200 {
                if t.coverage(x, y) > 0 {
                    let d = pts.iter().map(|p| (x as f64 + 0.5 - p[0]).hypot(y as f64 + 0.5 - p[1])).fold(f64::MAX, f64::min);
                    assert!(d <= 21.5, "({x}, {y}) is {d} px from the path");
                }
            }
        }
    }

    #[test]
    fn hostile_points_and_huge_documents_are_safe() {
        let mut t = Trail::new([300_000, 2]);
        assert!(t.scale() > 1.0 && t.w <= MAX_SIDE as usize);
        t.feed(&[[f64::NAN, 1.0, 1.0], [f64::INFINITY, -1e300, 1.0], [-5.0, -5.0, 1.0], [1e12, 1.0, 1.0]], 1e9);
        t.feed(&[], 10.0);
        let mut e = Trail::new([0, 0]);
        e.feed(&[[0.0, 0.0, 1.0]], 0.0);
        assert!(e.coverage(0, 0) > 0 && e.coverage(5, 5) == 0);
    }

    fn doc_coverage(trail: &Trail, x: u32, y: u32) -> u8 {
        if x < trail.origin[0] || y < trail.origin[1] {
            return 0;
        }
        trail.coverage(((x - trail.origin[0]) as f32 / trail.k) as usize, ((y - trail.origin[1]) as f32 / trail.k) as usize)
    }

    #[test]
    fn local_healing_feedback_matches_circle_radius_on_large_document_and_survives_growth_v82() {
        let mut trail = Trail::local([6000, 4000]);
        let points = [[3010.0, 2050.0, 0.05], [3180.0, 2050.0, 1.0], [3400.0, 2080.0, 0.3]];
        trail.feed(&points[..1], 40.0);
        assert_eq!(trail.scale(), 1.0, "a local stroke retains native resolution on 24MP");
        assert!(trail.w <= 256 && trail.h <= 256, "short feedback never allocates a full-document mask");
        assert_eq!(doc_coverage(&trail, 3010, 2050), 255);
        assert!(doc_coverage(&trail, 3029, 2050) >= 240, "last inside pixel agrees with the 20px cursor radius");
        assert_eq!(doc_coverage(&trail, 3031, 2050), 0);
        trail.feed(&points[..2], 40.0);
        trail.feed(&points, 40.0);
        assert_eq!(trail.scale(), 1.0);
        assert_eq!(doc_coverage(&trail, 3010, 2050), 255, "growing the crop retains the source endcap");
        assert_eq!(doc_coverage(&trail, 3400, 2080), 255);
        assert_eq!(doc_coverage(&trail, 3400, 2102), 0);
        trail.feed(&points, 10.0);
        assert_eq!(doc_coverage(&trail, 3010, 2060), 0, "resizing does not retain the old wider coverage");
    }

    #[test]
    fn feedback_tint_keeps_unmultiplied_color_and_thirty_percent_opacity_in_all_themes_v82() {
        for kind in crate::theme::ThemeKind::ALL {
            let tokens = crate::theme::Tokens::for_kind(kind);
            let color = tokens.area_feedback();
            let mut trail = Trail::local([200, 100]);
            trail.feed(&[[40.0, 50.0, 0.01], [100.0, 50.0, 1.0], [40.0, 50.0, 0.2]], 30.0);
            let image = trail.image([0, 0, trail.w, trail.h], color);
            let pixel = image.pixels[50 * trail.w + 60];
            assert_eq!(pixel.a(), 77, "overlapping feedback never builds up alpha: {kind:?}");
            let expected = color.to_srgba_unmultiplied();
            let actual = pixel.to_srgba_unmultiplied();
            for (actual, expected) in actual.into_iter().zip(expected) {
                assert!(actual.abs_diff(expected) <= 2, "{kind:?}: avoid applying premultiplication twice");
            }
            assert_eq!(image.pixels[0].a(), 0);
            assert!(expected[0] > expected[1] && expected[0] > expected[2], "healing and selection use a visible red edit mask");
        }
    }

    #[test]
    fn local_feedback_texture_bounds_preserve_document_origin_rotation_and_flip_v82() {
        let ctx = egui::Context::default();
        let mut trail = Trail::local([6000, 4000]);
        trail.feed(&[[3010.0, 2050.0, 1.0], [3100.0, 2080.0, 1.0]], 30.0);
        for rotation in [0.0, 37.0] {
            for flip in [false, true] {
                let xf = crate::canvas::ViewXform {
                    rect: Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(800.0, 600.0)),
                    center: [3050.0, 2050.0],
                    zoom: 2.0,
                    rotation,
                    flip,
                };
                let mut output =
                    ctx.run_ui(egui::RawInput { screen_rect: Some(xf.rect), ..Default::default() }, |ui| trail.draw(ui.painter(), &xf, Color32::WHITE));
                let mesh = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Mesh(mesh) if mesh.vertices.len() == 4 => Some(mesh),
                        _ => None,
                    })
                    .unwrap();
                let bounds = photocraft_geom::Rect::new(
                    trail.origin[0] as i32,
                    trail.origin[1] as i32,
                    trail.origin[0] as i32 + trail.w as i32,
                    trail.origin[1] as i32 + trail.h as i32,
                );
                for (vertex, expected) in mesh.vertices.iter().zip(xf.doc_corners(bounds)) {
                    assert!(vertex.pos.distance(expected) < 0.001);
                }
                output.textures_delta.clear();
            }
        }
    }
}
