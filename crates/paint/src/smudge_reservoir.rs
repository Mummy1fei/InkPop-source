//! Separate sampled-tip deposition and image pickup. Clean-room contact
//! experiments distinguish this from both masked displacement and feedback
//! from the pixel that has just been deposited.
use super::{Footprint, Region, Smudge};
use photocraft_color::{BlendMode, PixelFormat};
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

/// Visit matching contiguous scanlines. Saturating coordinates can repeat the
/// edge pixel at extreme i32 centres; retain their original sequential order.
fn matching_rows(work: &mut Region, carry: &mut [f32], side: usize, center: (i32, i32), r: i32, mut visit: impl FnMut(&mut [f32], &mut [f32], i32, i32)) {
    let n = work.ch;
    let Some(stride) = side.checked_mul(n).filter(|s| *s > 0) else { return };
    let Some(work_stride) = work.width().checked_mul(n) else { return };
    let Some(last) = side.checked_sub(1).and_then(|v| i32::try_from(v).ok()) else { return };
    let linear_x = center.0.checked_add(last).and_then(|_| center.0.checked_sub(r));
    for (qy, held_row) in carry.chunks_exact_mut(stride).enumerate() {
        let Ok(qy) = i32::try_from(qy) else { return };
        let y = center.1.saturating_add(qy).saturating_sub(r);
        if y < work.rect.y0 || y >= work.rect.y1 {
            continue;
        }
        let Some(row_start) = usize::try_from(i64::from(y) - i64::from(work.rect.y0)).ok().and_then(|row| row.checked_mul(work_stride)) else { continue };
        let Some(row_end) = row_start.checked_add(work_stride) else { continue };
        let Some(work_row) = work.data.get_mut(row_start..row_end) else { continue };
        if let Some(x0) = linear_x {
            let begin = (i64::from(work.rect.x0) - i64::from(x0)).clamp(0, side as i64) as usize;
            let end = (i64::from(work.rect.x1) - i64::from(x0)).clamp(0, side as i64) as usize;
            if begin >= end {
                continue;
            }
            let x = (i64::from(x0) + begin as i64) as i32;
            let Some(work_begin) = usize::try_from(i64::from(x) - i64::from(work.rect.x0)).ok().and_then(|v| v.checked_mul(n)) else { continue };
            let Some(work_end) = (end - begin).checked_mul(n).and_then(|count| work_begin.checked_add(count)) else { continue };
            if let (Some(pixels), Some(held)) = (work_row.get_mut(work_begin..work_end), held_row.get_mut(begin * n..end * n)) {
                visit(pixels, held, x, y);
            }
        } else {
            for (qx, held) in held_row.chunks_exact_mut(n).enumerate() {
                let x = center.0.saturating_add(qx as i32).saturating_sub(r);
                if x < work.rect.x0 || x >= work.rect.x1 {
                    continue;
                }
                let Some(start) = usize::try_from(i64::from(x) - i64::from(work.rect.x0)).ok().and_then(|v| v.checked_mul(n)) else { continue };
                let Some(end) = start.checked_add(n) else { continue };
                if let Some(pixel) = work_row.get_mut(start..end) {
                    visit(pixel, held, x, y);
                }
            }
        }
    }
}

/// A contact only reads/writes its own pixel and reservoir entry. Parallel rows
/// are therefore bit-equivalent, provided coordinate saturation cannot make
/// distinct reservoir entries alias the same work pixel.
#[cfg(not(target_arch = "wasm32"))]
fn parallel_rows(
    work: &mut Region,
    carry: &mut [f32],
    side: usize,
    center: (i32, i32),
    r: i32,
    min_area: usize,
    visit: &(impl Fn(&mut [f32], &mut [f32], i32, i32) + Sync),
) -> bool {
    let Some(last) = side.checked_sub(1).and_then(|v| i32::try_from(v).ok()) else { return false };
    let Some(x0) = center.0.checked_add(last).and_then(|_| center.0.checked_sub(r)) else { return false };
    let Some(y0) = center.1.checked_add(last).and_then(|_| center.1.checked_sub(r)) else { return false };
    let x_begin = (i64::from(work.rect.x0) - i64::from(x0)).clamp(0, side as i64) as usize;
    let x_end = (i64::from(work.rect.x1) - i64::from(x0)).clamp(0, side as i64) as usize;
    let y_begin = (i64::from(work.rect.y0) - i64::from(y0)).clamp(0, side as i64) as usize;
    let y_end = (i64::from(work.rect.y1) - i64::from(y0)).clamp(0, side as i64) as usize;
    if x_begin >= x_end || y_begin >= y_end {
        return true;
    }
    let Some(area) = (x_end - x_begin).checked_mul(y_end - y_begin) else { return false };
    if area < min_area {
        return false;
    }
    let n = work.ch;
    let Some(stride) = side.checked_mul(n).filter(|s| *s > 0) else { return false };
    let Some(work_stride) = work.width().checked_mul(n).filter(|s| *s > 0) else { return false };
    let x = (i64::from(x0) + x_begin as i64) as i32;
    let y = (i64::from(y0) + y_begin as i64) as i32;
    let Some(work_row_begin) = usize::try_from(i64::from(y) - i64::from(work.rect.y0)).ok().and_then(|v| v.checked_mul(work_stride)) else { return false };
    let Some(work_row_end) = (y_end - y_begin).checked_mul(work_stride).and_then(|v| work_row_begin.checked_add(v)) else { return false };
    let Some(work_x_begin) = usize::try_from(i64::from(x) - i64::from(work.rect.x0)).ok().and_then(|v| v.checked_mul(n)) else { return false };
    let Some(work_x_end) = (x_end - x_begin).checked_mul(n).and_then(|v| work_x_begin.checked_add(v)) else { return false };
    let Some(held_begin) = y_begin.checked_mul(stride) else { return false };
    let Some(held_end) = y_end.checked_mul(stride) else { return false };
    let Some(held_x_begin) = x_begin.checked_mul(n) else { return false };
    let Some(held_x_end) = x_end.checked_mul(n) else { return false };
    let (Some(pixels), Some(held)) = (work.data.get_mut(work_row_begin..work_row_end), carry.get_mut(held_begin..held_end)) else { return false };
    pixels.par_chunks_exact_mut(work_stride).zip(held.par_chunks_exact_mut(stride)).enumerate().with_min_len(8).for_each(|(row, (pixels, held))| {
        if let (Some(pixels), Some(held)) = (pixels.get_mut(work_x_begin..work_x_end), held.get_mut(held_x_begin..held_x_end)) {
            visit(pixels, held, x, (i64::from(y) + row as i64) as i32);
        }
    });
    true
}

fn independent_rows(
    work: &mut Region,
    carry: &mut [f32],
    side: usize,
    center: (i32, i32),
    r: i32,
    min_area: usize,
    visit: impl Fn(&mut [f32], &mut [f32], i32, i32) + Sync,
) {
    // The dispatch threshold is unused by the sequential WebAssembly build.
    let _ = min_area;
    #[cfg(not(target_arch = "wasm32"))]
    if parallel_rows(work, carry, side, center, r, min_area, &visit) {
        return;
    }
    matching_rows(work, carry, side, center, r, visit);
}

struct SampledKernel {
    n: usize,
    alpha: Option<usize>,
    strength: f32,
    pickup: f32,
    mode: BlendMode,
    format: Option<PixelFormat>,
}

impl SampledKernel {
    #[inline]
    fn pickup(&self, pixels: &[f32], held: &mut [f32]) {
        // Retain even zero-rate arithmetic: native f32 may contain signed zero
        // or exceptional values; preserve the original recurrence bit for bit.
        let opaque = self.alpha.is_none_or(|alpha| pixels.chunks_exact(self.n).all(|pixel| pixel.get(alpha) == Some(&1.0)));
        if opaque {
            for (old, carried) in pixels.iter().zip(held) {
                *carried += (*old - *carried) * self.pickup;
            }
            return;
        }
        for (pixel, held) in pixels.chunks_exact(self.n).zip(held.chunks_exact_mut(self.n)) {
            let a = self.alpha.and_then(|a| pixel.get(a).copied()).unwrap_or(1.0);
            for (c, (value, carried)) in pixel.iter().zip(held).enumerate() {
                let old = if self.alpha.is_some() && self.alpha != Some(c) { *value * a } else { *value };
                *carried += (old - *carried) * self.pickup;
            }
        }
    }

    #[inline]
    fn deposit(&self, pixel: &mut [f32], held: &mut [f32], k: f32, source: Option<&[f32]>) {
        let old_alpha = self.alpha.and_then(|a| pixel.get(a).copied()).unwrap_or(1.0);
        let held_alpha = self.alpha.and_then(|a| held.get(a).copied()).unwrap_or(1.0);
        if source.is_none() && old_alpha == 1.0 && held_alpha == 1.0 {
            // No colour conversion, premultiplication or unpremultiplication is
            // necessary for opaque Normal pixels (also native no-alpha data).
            for (value, carried) in pixel.iter_mut().zip(held) {
                let old = *value;
                let deposited = if k == 1.0 { *carried } else { old + (*carried - old) * k };
                let next = *carried + (old - *carried) * self.pickup;
                if old != deposited {
                    *value = deposited;
                }
                *carried = next;
            }
            return;
        }
        let out_alpha = old_alpha + (held_alpha - old_alpha) * k;
        for (c, (value, carried)) in pixel.iter_mut().zip(held).enumerate() {
            let old = if self.alpha.is_some() && self.alpha != Some(c) { *value * old_alpha } else { *value };
            let paint = source.and_then(|v| v.get(c).copied()).unwrap_or(*carried);
            let deposited = if k == 1.0 { paint } else { old + (paint - old) * k };
            let next = *carried + (old - *carried) * self.pickup;
            if !(old == deposited && old_alpha == out_alpha) {
                *value = if self.alpha.is_none() || self.alpha == Some(c) {
                    deposited
                } else if out_alpha > 0.0 {
                    deposited / out_alpha
                } else {
                    0.0
                };
            }
            *carried = next;
        }
    }

    fn row<const BLEND: bool>(&self, pixels: &mut [f32], held: &mut [f32], origin: (i32, i32), fp: &Footprint) {
        let (x, y) = origin;
        let count = pixels.len() / self.n;
        if y < fp.rect.y0 || y >= fp.rect.y1 {
            self.pickup(pixels, held);
            return;
        }
        let begin = (i64::from(fp.rect.x0) - i64::from(x)).clamp(0, count as i64) as usize;
        let end = (i64::from(fp.rect.x1) - i64::from(x)).clamp(0, count as i64) as usize;
        if begin >= end {
            self.pickup(pixels, held);
            return;
        }
        let Some(cov_begin) = usize::try_from(i64::from(y) - i64::from(fp.rect.y0))
            .ok()
            .and_then(|row| row.checked_mul(fp.rect.width() as usize))
            .and_then(|row| usize::try_from(i64::from(x) + begin as i64 - i64::from(fp.rect.x0)).ok().and_then(|col| row.checked_add(col)))
        else {
            return;
        };
        let Some(cov_end) = cov_begin.checked_add(end - begin) else { return };
        let Some(coverage) = fp.cov.get(cov_begin..cov_end) else { return };
        let (prefix, pixels) = pixels.split_at_mut(begin * self.n);
        let (held_prefix, held) = held.split_at_mut(begin * self.n);
        self.pickup(prefix, held_prefix);
        let (pixels, suffix) = pixels.split_at_mut((end - begin) * self.n);
        let (held, held_suffix) = held.split_at_mut((end - begin) * self.n);
        for ((pixel, held), coverage) in pixels.chunks_exact_mut(self.n).zip(held.chunks_exact_mut(self.n)).zip(coverage) {
            let k = coverage.clamp(0.0, 1.0) * self.strength;
            if k > 0.0 {
                if BLEND && let Some(format) = self.format {
                    let source = blended_source(self.mode, format, pixel, held, self.alpha);
                    self.deposit(pixel, held, k, Some(&source));
                } else {
                    self.deposit(pixel, held, k, None);
                }
            } else {
                self.pickup(pixel, held);
            }
        }
        self.pickup(suffix, held_suffix);
    }
}

/// The same format-aware colour blend primitives as ordinary painting. Apply
/// the mode to deposition only; pickup must still see the unmodified substrate.
fn blended_source(mode: BlendMode, format: PixelFormat, old: &[f32], held: &[f32], alpha: Option<usize>) -> [f32; 16] {
    let mut source = [0.0; 16];
    let n = held.len().min(source.len());
    if let (Some(out), Some(input)) = (source.get_mut(..n), held.get(..n)) {
        out.copy_from_slice(input);
    }
    let held_alpha = alpha.and_then(|a| held.get(a).copied()).unwrap_or(1.0);
    if let Some(a) = alpha {
        for (c, v) in source.iter_mut().take(n).enumerate() {
            if c != a {
                *v = if held_alpha > 0.0 { *v / held_alpha } else { 0.0 };
            }
        }
    }
    let backdrop = photocraft_raster::to_rgba(&format, old);
    let carried = photocraft_raster::to_rgba(&format, &source);
    let color = photocraft_color::blend::blend_rgb(mode, [backdrop[0], backdrop[1], backdrop[2]], [carried[0], carried[1], carried[2]]);
    let source_rgba = [
        (1.0 - backdrop[3]) * carried[0] + backdrop[3] * color[0],
        (1.0 - backdrop[3]) * carried[1] + backdrop[3] * color[1],
        (1.0 - backdrop[3]) * carried[2] + backdrop[3] * color[2],
        carried[3],
    ];
    photocraft_raster::from_rgba_into(&format, source_rgba, &mut source);
    super::premultiply(&mut source[..n], alpha);
    source
}

impl Smudge {
    pub(super) fn sampled_dab(&mut self, work: &mut Region, fp: &Footprint) {
        let center = (fp.dab.center.x, fp.dab.center.y);
        if !center.0.is_finite() || !center.1.is_finite() || !fp.cov.iter().any(|c| c.is_finite() && *c > 0.0) {
            return;
        }
        if self.last_center == Some(center) {
            return;
        }
        let (r, n) = (self.radius, work.ch);
        let Some(side) = r.checked_mul(2).and_then(|r| r.checked_add(1)).and_then(|s| usize::try_from(s).ok()).filter(|s| *s > 0) else { return };
        let Some(len) = side.checked_mul(side).and_then(|area| area.checked_mul(n)) else { return };
        // Native formats have at most sixteen channels. The engine applies a
        // stricter aggregate budget including all working images before entry.
        if n == 0 || n > 16 || len > 134_217_728 || work.rect.is_empty() {
            return;
        }
        let (cx, cy) = (center.0.floor() as i32, center.1.floor() as i32);
        let alpha = self.alpha.filter(|a| *a < n);
        if self.carry.is_none() {
            let mut carry = vec![0.0; len];
            matching_rows(work, &mut carry, side, (cx, cy), r, |pixels, held, _, _| {
                held.copy_from_slice(pixels);
                for pixel in held.chunks_exact_mut(n) {
                    super::premultiply(pixel, alpha);
                }
            });
            self.carry = Some(carry);
            self.last_center = Some(center);
            // Same source and destination at first contact. Do not introduce
            // a premultiply/unpremultiply round trip into untouched native bits.
            return;
        }
        let Some(carry) = self.carry.as_mut().filter(|c| c.len() == len) else { return };
        let blend = self.format.filter(|format| format.channels() == n && self.sampled_mode != BlendMode::Normal);
        let strength = (self.strength * fp.dab.opacity).clamp(0.0, 1.0);
        // Flow is already in footprint coverage, hence is not multiplied into
        // deposition again. It independently controls how quickly colour is
        // replaced in the carried image, including outside a sparse tip mask.
        let pickup = 1.0 - strength * fp.dab.alpha.clamp(0.0, 1.0);
        let kernel = SampledKernel { n, alpha, strength, pickup, mode: self.sampled_mode, format: blend };
        if blend.is_some() {
            independent_rows(work, carry, side, (cx, cy), r, 16_384, |pixels, held, x, y| kernel.row::<true>(pixels, held, (x, y), fp));
        } else {
            // Normal's opaque row loop is cheap; keep medium contacts local
            // rather than paying a parallel dispatch for each spatial dab.
            independent_rows(work, carry, side, (cx, cy), r, 65_536, |pixels, held, x, y| kernel.row::<false>(pixels, held, (x, y), fp));
        }
        self.last_center = Some(center);
    }
}
