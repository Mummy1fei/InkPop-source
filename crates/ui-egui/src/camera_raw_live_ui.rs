//! Cached, bounded source for slider feedback. The normal display proxy is restored
//! after interaction; neither this source nor its output enters the engine commit.
use std::sync::Arc;

pub(super) const PIXELS: usize = 524_288;
const SIDE: usize = 1024;

/// Only large-scale spatial operations benefit from a reduced work grid. Pointwise
/// colour changes keep the display grid; detail/noise/local-mask edits must not have
/// their intended high-frequency changes replaced with the original source detail.
pub(super) fn should_reduce(params: &photocraft_algo::camera_raw::CameraRaw) -> bool {
    (params.dehaze != 0.0 || params.highlights != 0.0 || params.shadows != 0.0 || params.texture != 0.0 || params.clarity != 0.0)
        && params.sharpen_amount == 0.0
        && params.noise_luminance == 0.0
        && params.noise_color == 0.0
        && params.grain_amount == 0.0
        && params.local_adjustments.is_empty()
}

#[derive(Clone)]
pub(super) struct Source {
    pub width: usize,
    pub height: usize,
    pub pixels: Arc<[[f32; 4]]>,
}

impl Source {
    pub fn new(pixels: Arc<[[f32; 4]]>, width: usize, height: usize) -> Self {
        // The normal 900 px floor is already only ~0.54 MP for a 3:2 image.
        // Reducing that to 0.50 MP would barely help while still paying expansion.
        // Require at least four times fewer work pixels: a half-size pixel budget
        // can cross dehaze/guided-filter subsampling thresholds and increase their
        // internal work at ordinary DPI instead of making the feedback cheaper.
        let count = width.saturating_mul(height);
        let budget = if count > PIXELS || width.max(height) > SIDE { PIXELS.min((count / 4).max(1)) } else { count.max(1) };
        let scale = (SIDE as f64 / width.max(height).max(1) as f64).min((budget as f64 / (width.max(1) as f64 * height.max(1) as f64)).sqrt()).min(1.0);
        let w = ((width as f64 * scale).floor() as usize).max(1);
        let h = ((height as f64 * scale).floor() as usize).max(1);
        if w == width && h == height {
            return Self { width, height, pixels };
        }
        // This runs once when the dialog opens, never on each slider edit. Every
        // source pixel belongs to exactly one bin, including thin/wide images.
        let edge = |bin: usize, extent: usize, bins: usize| (bin as u64 * extent as u64 / bins as u64) as usize;
        let mut reduced = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0.0f64; 4];
                let mut count = 0usize;
                for sy in edge(y, height, h)..edge(y + 1, height, h) {
                    for sx in edge(x, width, w)..edge(x + 1, width, w) {
                        if let Some(pixel) = pixels.get(sy.saturating_mul(width).saturating_add(sx)) {
                            for (out, value) in sum.iter_mut().zip(pixel) {
                                *out += *value as f64;
                            }
                            count += 1;
                        }
                    }
                }
                reduced.push(sum.map(|v| (v / count.max(1) as f64) as f32));
            }
        }
        Self { width: w, height: h, pixels: reduced.into() }
    }

    /// Bilinear display expansion; exact mask/selection coverage is applied afterwards
    /// on the normal proxy grid. Histogram and scope keep that same grid and revision.
    #[cfg(test)]
    pub fn expand(&self, pixels: &[[f32; 4]], width: usize, height: usize) -> Vec<[f32; 4]> {
        self.expand_with_parallel(pixels, None, width, height, true)
    }

    /// Add the interpolated adjustment difference to the original display pixels.
    /// Unlike upscaling the developed low-resolution image, this keeps fine display
    /// structure during a drag. This is an approximation of global spatial processing;
    /// the settled display/full-resolution engine result remains authoritative.
    pub fn expand_preserving_detail(&self, pixels: &[[f32; 4]], original: &[[f32; 4]], width: usize, height: usize) -> Vec<[f32; 4]> {
        self.expand_with_parallel(pixels, Some(original), width, height, true)
    }

    #[cfg(test)]
    pub(super) fn expand_serial(&self, pixels: &[[f32; 4]], width: usize, height: usize) -> Vec<[f32; 4]> {
        self.expand_with_parallel(pixels, None, width, height, false)
    }

    fn expand_with_parallel(&self, pixels: &[[f32; 4]], original: Option<&[[f32; 4]]>, width: usize, height: usize, parallel: bool) -> Vec<[f32; 4]> {
        #[cfg(target_arch = "wasm32")]
        let _ = parallel;
        if width == 0 || height == 0 {
            return Vec::new();
        }
        let axis = |coordinate: usize, target: usize, source: usize| {
            let p = (((coordinate as f64 + 0.5) * source as f64 / target.max(1) as f64) - 0.5).clamp(0.0, source.saturating_sub(1) as f64);
            let lo = p.floor() as usize;
            (lo, lo.saturating_add(1).min(source.saturating_sub(1)), (p - lo as f64) as f32)
        };
        let horizontal: Vec<_> = (0..width).map(|x| axis(x, width, self.width)).collect();
        let sample = |x: usize, y: usize| {
            let index = y.saturating_mul(self.width).saturating_add(x);
            let pixel = pixels.get(index).copied().unwrap_or([0.0; 4]);
            if original.is_some() {
                let before = self.pixels.get(index).copied().unwrap_or([0.0; 4]);
                std::array::from_fn(|channel| pixel[channel] - before[channel])
            } else {
                pixel
            }
        };
        let fill = |y: usize, row: &mut [[f32; 4]]| {
            let (y0, y1, ty) = axis(y, height, self.height);
            for (x, (pixel, &(x0, x1, tx))) in row.iter_mut().zip(&horizontal).enumerate() {
                let (a, b, c, d) = (sample(x0, y0), sample(x1, y0), sample(x0, y1), sample(x1, y1));
                let reference = original.and_then(|reference| reference.get(y.saturating_mul(width).saturating_add(x)));
                *pixel = std::array::from_fn(|channel| {
                    if let Some(reference) = reference
                        && channel == 3
                    {
                        return reference[3];
                    }
                    let lo = a[channel] + (b[channel] - a[channel]) * tx;
                    let hi = c[channel] + (d[channel] - c[channel]) * tx;
                    let difference = lo + (hi - lo) * ty;
                    reference.map_or(difference, |reference| reference[channel] + difference)
                });
            }
        };
        let mut output = vec![[0.0; 4]; width.saturating_mul(height)];
        #[cfg(not(target_arch = "wasm32"))]
        if parallel && output.len() > 65_536 {
            use rayon::prelude::*;
            output.par_chunks_mut(width).with_min_len(16).enumerate().for_each(|(y, row)| fill(y, row));
            return output;
        }
        for (y, row) in output.chunks_mut(width).enumerate() {
            fill(y, row);
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_large_scale_spatial_edits_reduce_the_work_grid() {
        use photocraft_algo::camera_raw::CameraRaw;
        assert!(!should_reduce(&CameraRaw { exposure: 1.0, contrast: 30.0, temperature: 20.0, ..Default::default() }));
        let spatial = CameraRaw { clarity: 30.0, ..Default::default() };
        assert!(should_reduce(&spatial));
        for params in [
            CameraRaw { sharpen_amount: 1.0, ..spatial.clone() },
            CameraRaw { noise_luminance: 1.0, ..spatial.clone() },
            CameraRaw { noise_color: 1.0, ..spatial.clone() },
            CameraRaw { grain_amount: 1.0, ..spatial.clone() },
            CameraRaw { local_adjustments: vec![Default::default()], ..spatial },
        ] {
            assert!(!should_reduce(&params), "fine detail and normalized masks need their own pixel grid");
        }
    }

    #[test]
    fn adjustment_expansion_preserves_fine_display_structure_and_alpha_at_both_dpi_budgets() {
        for (width, height) in [(1046, 698), (2092, 1395), (4096, 1), (1, 4096)] {
            let original: Arc<[[f32; 4]]> = (0..width * height)
                .map(|i| {
                    let checker = ((i % width + i / width) % 2) as f32;
                    [checker * 0.5 + 0.125, checker * 0.25, checker + 1.5, (i % 17) as f32 / 16.0]
                })
                .collect::<Vec<_>>()
                .into();
            let source = Source::new(original.clone(), width, height);
            assert!(source.pixels.len() < original.len());
            let processed: Vec<_> = source.pixels.iter().map(|p| [p[0] + 0.125, p[1] - 0.0625, p[2] + 0.25, 0.0]).collect();
            let expanded = source.expand_preserving_detail(&processed, &original, width, height);
            let serial = source.expand_with_parallel(&processed, Some(&original), width, height, false);
            assert_eq!(expanded, serial, "parallel order must not change display pixels");
            for (pixel, before) in expanded.iter().zip(original.iter()) {
                assert!((pixel[0] - before[0] - 0.125).abs() < 2e-7);
                assert!((pixel[1] - before[1] + 0.0625).abs() < 2e-7);
                assert!((pixel[2] - before[2] - 0.25).abs() < 2e-7, "HDR detail must survive too");
                assert_eq!(pixel[3].to_bits(), before[3].to_bits());
            }
            let blurred = source.expand(&source.pixels, width, height);
            let error = blurred.iter().zip(original.iter()).map(|(a, b)| (a[0] - b[0]).abs()).sum::<f32>() / original.len() as f32;
            assert!(error > 0.15, "the old replacement must actually lose the test's fine structure");
        }
    }

    #[test]
    fn parallel_expansion_is_bitwise_equal_to_serial_display_oracle() {
        let source = Source {
            width: 135,
            height: 97,
            pixels: (0..135 * 97)
                .map(|i| [((i * 17) % 251) as f32 / 100.0 - 0.5, ((i * 29) % 251) as f32 / 250.0, (i % 2) as f32, 0.75])
                .collect::<Vec<_>>()
                .into(),
        };
        for (width, height) in [(1046, 698), (2092, 1395), (2049, 1), (1, 2049)] {
            let axis = |coordinate: usize, target: usize, bins: usize| {
                let p = (((coordinate as f64 + 0.5) * bins as f64 / target.max(1) as f64) - 0.5).clamp(0.0, bins.saturating_sub(1) as f64);
                let lo = p.floor() as usize;
                (lo, lo.saturating_add(1).min(bins.saturating_sub(1)), (p - lo as f64) as f32)
            };
            let sample = |x: usize, y: usize| source.pixels.get(y.saturating_mul(source.width).saturating_add(x)).copied().unwrap_or([0.0; 4]);
            let actual = source.expand(&source.pixels, width, height);
            for (index, pixel) in actual.iter().enumerate() {
                let (x0, x1, tx) = axis(index % width, width, source.width);
                let (y0, y1, ty) = axis(index / width, height, source.height);
                let (a, b, c, d) = (sample(x0, y0), sample(x1, y0), sample(x0, y1), sample(x1, y1));
                let expected: [f32; 4] = std::array::from_fn(|channel| {
                    let lo = a[channel] + (b[channel] - a[channel]) * tx;
                    let hi = c[channel] + (d[channel] - c[channel]) * tx;
                    lo + (hi - lo) * ty
                });
                assert_eq!(pixel.map(f32::to_bits), expected.map(f32::to_bits), "{width}x{height} pixel {index}");
            }
        }
    }

    #[test]
    fn cached_source_is_bounded_and_preserves_constant_float_pixels() {
        for (w, h) in [(2508, 1672), (1046, 698), (901, 600), (4096, 1), (1, 4096), (64, 48)] {
            let pixels: Arc<[[f32; 4]]> = vec![[0.25, 0.5, 1.5, 0.75]; w * h].into();
            let source = Source::new(pixels.clone(), w, h);
            assert!(source.width * source.height <= PIXELS);
            assert!(source.width.max(source.height) <= SIDE);
            if w * h > PIXELS || w.max(h) > SIDE {
                // Integer rounding keeps at least one pixel on a thin axis.
                assert!(source.width * source.height <= (w * h / 4).max(source.width.max(source.height)));
            }
            assert!(source.pixels.iter().all(|p| *p == [0.25, 0.5, 1.5, 0.75]));
            assert!(source.expand(&source.pixels, w, h).iter().all(|p| *p == [0.25, 0.5, 1.5, 0.75]));
            if w == 64 {
                assert!(Arc::ptr_eq(&pixels, &source.pixels), "small images must not be resampled");
            }
        }
    }

    #[test]
    fn display_expansion_uses_pixel_centers_and_reaches_both_edges() {
        let source = Source { width: 2, height: 1, pixels: vec![[0.0; 4], [1.0; 4]].into() };
        let pixels = source.expand(&source.pixels, 4, 1);
        assert_eq!(pixels, vec![[0.0; 4], [0.25; 4], [0.75; 4], [1.0; 4]]);
    }
}
