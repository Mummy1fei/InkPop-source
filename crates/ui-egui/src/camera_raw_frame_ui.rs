//! Display preparation for Camera Raw. Native chunks preserve the serial oracle's
//! pixel/bin arithmetic, dimensions and sample count; no preview quality is traded.
use egui::{Color32, ColorImage};
use photocraft_algo::histogram::RgbHistogram;

#[cfg(any(test, not(target_arch = "wasm32")))]
const CHUNK: usize = 65_536;

#[cfg(any(test, not(target_arch = "wasm32")))]
fn merge(mut left: RgbHistogram, right: RgbHistogram) -> RgbHistogram {
    left.samples = left.samples.saturating_add(right.samples);
    left.transparent = left.transparent.saturating_add(right.transparent);
    left.invalid = left.invalid.saturating_add(right.invalid);
    for (left, right) in left.channels.iter_mut().flatten().zip(right.channels.iter().flatten()) {
        *left = left.saturating_add(*right);
    }
    for (left, right) in [
        (&mut left.underflow, right.underflow),
        (&mut left.overflow, right.overflow),
        (&mut left.shadows, right.shadows),
        (&mut left.highlights, right.highlights),
    ] {
        for (left, right) in left.iter_mut().zip(right) {
            *left = left.saturating_add(right);
        }
    }
    left
}

pub(super) fn histogram(pixels: &[[f32; 4]]) -> RgbHistogram {
    #[cfg(not(target_arch = "wasm32"))]
    if pixels.len() > CHUNK {
        use rayon::prelude::*;
        return pixels.par_chunks(CHUNK).map(RgbHistogram::from_rgba).reduce(RgbHistogram::default, merge);
    }
    RgbHistogram::from_rgba(pixels)
}

fn encode(q: &[f32; 4]) -> Color32 {
    let enc = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    Color32::from_rgba_unmultiplied(enc(q[0]), enc(q[1]), enc(q[2]), enc(q[3]))
}

pub(super) fn image(pixels: &[[f32; 4]], width: usize, height: usize) -> ColorImage {
    #[cfg(not(target_arch = "wasm32"))]
    if pixels.len() > CHUNK {
        use rayon::prelude::*;
        return ColorImage::new([width, height], pixels.par_iter().map(encode).collect());
    }
    ColorImage::new([width, height], pixels.iter().map(encode).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_display_preparation_matches_serial_hdr_nonfinite_and_alpha_oracle() {
        let cases = [
            [0.0001, 0.9999, 0.5, 1.0],
            [-0.2, 1.2, 0.5, 0.25],
            [1.0, 0.0, 0.5, 1.0],
            [f32::NAN, 0.2, 0.3, 1.0],
            [0.3, f32::INFINITY, 0.2, 1.0],
            [0.4, 0.2, 0.0, f32::NAN],
            [0.4, 0.2, 0.0, 0.0],
            [0.4, 0.2, 0.0, -1.0],
        ];
        for count in [0, 7, CHUNK, CHUNK + 1, CHUNK * 3 + 17] {
            let pixels: Vec<_> = (0..count).map(|index| cases[index % cases.len()]).collect();
            assert_eq!(histogram(&pixels), RgbHistogram::from_rgba(&pixels));
            assert_eq!(image(&pixels, count, 1), ColorImage::new([count, 1], pixels.iter().map(encode).collect()));
        }
    }

    #[test]
    fn histogram_chunk_merge_preserves_saturated_counters() {
        let mut left = RgbHistogram { samples: u64::MAX, invalid: u64::MAX, transparent: u64::MAX, ..Default::default() };
        left.channels[0][0] = u64::MAX;
        left.underflow[0] = u64::MAX;
        left.overflow[0] = u64::MAX;
        left.shadows[0] = u64::MAX;
        left.highlights[0] = u64::MAX;
        let merged = merge(left, RgbHistogram::from_rgba(&[[0.0, 0.5, 1.0, 1.0], [-0.5, 1.5, 0.0, 1.0], [0.0; 4], [f32::NAN; 4]]));
        assert_eq!(merged.samples, u64::MAX);
        assert_eq!(merged.invalid, u64::MAX);
        assert_eq!(merged.transparent, u64::MAX);
        assert_eq!(merged.channels[0][0], u64::MAX);
        assert_eq!(merged.underflow[0], u64::MAX);
        assert_eq!(merged.overflow[0], u64::MAX);
        assert_eq!(merged.shadows[0], u64::MAX);
        assert_eq!(merged.highlights[0], u64::MAX);
    }
}
