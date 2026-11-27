//! Backward image transport; a clean-room model checked on independent
//! Strength25/50/75, transparent-edge and right-angle Photoshop exports.
use super::{Footprint, Region, Smudge};
use photocraft_geom::Rect;

impl Smudge {
    pub(super) fn advect(&mut self, work: &mut Region, fp: &Footprint) {
        let center = (fp.dab.center.x, fp.dab.center.y);
        let Some(previous) = self.last_center.replace(center) else { return };
        let (dx, dy) = (center.0 - previous.0, center.1 - previous.1);
        let controlled = self.strength * fp.dab.opacity;
        if !dx.is_finite() || !dy.is_finite() || !controlled.is_finite() || controlled <= 0.0 || (dx == 0.0 && dy == 0.0) {
            return;
        }
        let wr = work.rect;
        let paint = fp.rect.intersect(&wr);
        if paint.is_empty() || work.ch == 0 {
            return;
        }
        let px = (dx.abs().ceil().min(f64::from(wr.width())) as i32).saturating_add(1);
        let py = (dy.abs().ceil().min(f64::from(wr.height())) as i32).saturating_add(1);
        let read = Rect::new(paint.x0.saturating_sub(px), paint.y0.saturating_sub(py), paint.x1.saturating_add(px), paint.y1.saturating_add(py)).intersect(&wr);
        let (w, h, n) = (read.width() as usize, read.height() as usize, work.ch);
        let Some(row) = w.checked_mul(n) else { return };
        let Some(len) = row.checked_mul(h) else { return };
        if len > work.data.len() {
            return;
        }
        self.scratch.resize(len, 0.0);
        for y in read.y0..read.y1 {
            let src = work.index(read.x0, y).saturating_mul(n);
            let dst = (y - read.y0) as usize * row;
            let Some(source) = work.data.get(src..src.saturating_add(row)) else { return };
            let Some(target) = self.scratch.get_mut(dst..dst.saturating_add(row)) else { return };
            target.copy_from_slice(source);
        }
        let alpha = self.alpha.filter(|a| *a < n);
        if let Some(a) = alpha {
            for pixel in self.scratch.chunks_exact_mut(n) {
                let opacity = pixel.get(a).copied().unwrap_or(0.0);
                if opacity != 1.0 {
                    for (c, value) in pixel.iter_mut().enumerate() {
                        if c != a {
                            *value *= opacity;
                        }
                    }
                }
            }
        }
        for y in paint.y0..paint.y1 {
            for x in paint.x0..paint.x1 {
                let k = (fp.at(x, y) * controlled).clamp(0.0, 1.0);
                if k <= 0.0 {
                    continue;
                }
                let sx = (f64::from(x) - dx * f64::from(k)).clamp(f64::from(read.x0), f64::from(read.x1 - 1));
                let sy = (f64::from(y) - dy * f64::from(k)).clamp(f64::from(read.y0), f64::from(read.y1 - 1));
                let (x0, y0) = (sx.floor() as i32, sy.floor() as i32);
                let (fx, fy) = ((sx - f64::from(x0)) as f32, (sy - f64::from(y0)) as f32);
                let at = |xx: i32, yy: i32| ((yy - read.y0) as usize * w + (xx - read.x0) as usize) * n;
                let offsets = [
                    at(x0, y0),
                    at((x0 + 1).min(read.x1 - 1), y0),
                    at(x0, (y0 + 1).min(read.y1 - 1)),
                    at((x0 + 1).min(read.x1 - 1), (y0 + 1).min(read.y1 - 1)),
                ];
                let weights = [(1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy];
                let sample = |offset: usize, c: usize| self.scratch.get(offset.saturating_add(c)).copied().unwrap_or(0.0);
                let interpolate = |c| {
                    let values = offsets.map(|o| sample(o, c));
                    if values.iter().all(|v| *v == values[0]) { values[0] } else { values.iter().zip(weights).map(|(v, w)| v * w).sum::<f32>() }
                };
                let opacity = alpha.map_or(1.0, interpolate);
                let target = work.px_mut(x, y);
                let old_alpha = alpha.and_then(|a| target.get(a).copied()).unwrap_or(1.0);
                for (c, value) in target.iter_mut().enumerate() {
                    if alpha == Some(c) {
                        *value = opacity;
                        continue;
                    }
                    let v = interpolate(c);
                    if old_alpha == opacity && *value * old_alpha == v {
                        continue;
                    }
                    *value = if alpha.is_none() {
                        v
                    } else if opacity > 0.0 {
                        v / opacity
                    } else {
                        0.0
                    };
                }
            }
        }
    }
}
