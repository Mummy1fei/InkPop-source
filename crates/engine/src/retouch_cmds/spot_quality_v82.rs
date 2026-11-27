//! Prefer one coherent exemplar only when its known surroundings agree strongly.
//!
//! PatchMatch voting remains the general fill. For repeated structures, averaging
//! individually plausible patches can soften or break a line even when a complete
//! matching neighbour exists. A conservative whole-patch donor avoids that case.
use photocraft_algo::inpaint;
use photocraft_paint::retouch::Region;

pub(super) fn coherent_fill(image: &Region, hole: &[bool], size: f32, radius: i32) -> Option<Vec<f32>> {
    let (w, h, ch) = (image.width(), image.height(), image.ch);
    let pixels = w.checked_mul(h)?;
    // This optional fast path stays local and bounded. Large strokes retain the
    // general completion path rather than adding another full search first.
    if pixels == 0 || pixels > 1_048_576 || ch == 0 || ch > 8 || hole.len() != pixels || image.data.len() != pixels.checked_mul(ch)? || !size.is_finite() {
        return None;
    }
    let ring = (size / 8.0).clamp(3.0, 12.0) as usize;
    let offset = inpaint::best_offset(w, h, ch, &image.data, hole, ring, radius.clamp(1, 256))?;
    // Use the immediately known neighbourhood, never the contaminated values
    // under the stroke. The source sampler has already rejected overlapping holes.
    let mut count = 0usize;
    let mut error = 0.0f64;
    let mut sum = vec![0.0f64; ch];
    let mut sum_sq = vec![0.0f64; ch];
    let near_hole = super::dilate(w, h, hole, ring);
    for y in 0..h {
        for x in 0..w {
            if *hole.get(y * w + x)? {
                continue;
            }
            if !*near_hole.get(y * w + x)? {
                continue;
            }
            let sx = x as i32 + offset.0;
            let sy = y as i32 + offset.1;
            if sx < 0 || sy < 0 || sx as usize >= w || sy as usize >= h || *hole.get(sy as usize * w + sx as usize)? {
                return None;
            }
            let (a, b) = ((y * w + x) * ch, (sy as usize * w + sx as usize) * ch);
            for ((value, donor), (sum, sum_sq)) in image.data.get(a..a + ch)?.iter().zip(image.data.get(b..b + ch)?).zip(sum.iter_mut().zip(&mut sum_sq)) {
                let v = f64::from(*value);
                let d = v - f64::from(*donor);
                if !v.is_finite() || !d.is_finite() {
                    return None;
                }
                error += d * d;
                *sum += v;
                *sum_sq += v * v;
            }
            count += 1;
        }
    }
    if count < 16 {
        return None;
    }
    let n = count as f64;
    let variance = sum.iter().zip(&sum_sq).map(|(sum, sq)| (sq / n - (sum / n).powi(2)).max(0.0)).sum::<f64>() / ch as f64;
    let mse = error / (n * ch as f64);
    // Require ~1% of the actual surrounding signal variance, with only a
    // sub-integer-quantisation floor for a flat field. Uncertain matches use EM.
    if mse > variance * 0.01 + 1e-6 {
        return None;
    }
    let mut out = image.data.clone();
    for (i, &masked) in hole.iter().enumerate() {
        if !masked {
            continue;
        }
        let (x, y) = ((i % w) as i32 + offset.0, (i / w) as i32 + offset.1);
        if x < 0 || y < 0 || x as usize >= w || y as usize >= h || *hole.get(y as usize * w + x as usize)? {
            return None;
        }
        let source = (y as usize * w + x as usize) * ch;
        out.get_mut(i * ch..(i + 1) * ch)?.copy_from_slice(image.data.get(source..source + ch)?);
    }
    Some(out)
}

#[cfg(test)]
#[path = "spot_quality_tests_v82.rs"]
mod tests;
