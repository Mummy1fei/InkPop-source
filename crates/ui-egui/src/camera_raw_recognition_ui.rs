//! Independent, alpha-weighted analysis proxy. Raw's display/develop proxy keeps
//! its existing math; invisible stored RGB must not become a recognition cue.
pub(super) struct Source {
    pub pixels: std::sync::Arc<[[f32; 4]]>,
    pub width: usize,
    pub height: usize,
}

pub(super) struct Builder {
    width: usize,
    height: usize,
    input_width: usize,
    input_height: usize,
    sums: Vec<[f64; 5]>,
}
impl Builder {
    pub fn new(width: usize, height: usize) -> Self {
        let scale = (photocraft_engine::image_recognition::REFERENCE_SIDE as f64 / width.max(height).max(1) as f64).min(1.0);
        let output_width = (width as f64 * scale).round().max(1.0) as usize;
        let output_height = (height as f64 * scale).round().max(1.0) as usize;
        Self {
            width: output_width,
            height: output_height,
            input_width: width.max(1),
            input_height: height.max(1),
            sums: vec![[0.0; 5]; output_width * output_height],
        }
    }

    pub fn add_row(&mut self, row: &[[f32; 4]], y: usize) {
        if y >= self.input_height {
            return;
        }
        let output_y = (y as u64 * self.height as u64 / self.input_height as u64) as usize;
        for (x, pixel) in row.iter().take(self.input_width).enumerate() {
            let output_x = (x as u64 * self.width as u64 / self.input_width as u64) as usize;
            let Some(sum) = self.sums.get_mut(output_y * self.width + output_x) else { continue };
            let alpha = if pixel[3].is_finite() { pixel[3].clamp(0.0, 1.0) as f64 } else { 0.0 };
            for (total, value) in sum.iter_mut().take(3).zip(pixel) {
                // Hidden NaN/Inf is ignored just like hidden RGB. Visible invalid
                // input remains invalid and the shared service rejects it.
                if alpha > 0.0 {
                    *total += *value as f64 * alpha;
                }
            }
            sum[3] += alpha;
            sum[4] += 1.0;
        }
    }

    pub fn finish(self) -> Source {
        let pixels = self
            .sums
            .into_iter()
            .map(|sum| {
                if sum[3] > 0.0 {
                    [(sum[0] / sum[3]) as f32, (sum[1] / sum[3]) as f32, (sum[2] / sum[3]) as f32, (sum[3] / sum[4].max(1.0)) as f32]
                } else {
                    [0.0; 4]
                }
            })
            .collect::<Vec<_>>()
            .into();
        Source { pixels, width: self.width, height: self.height }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn analysis_proxy_ignores_hidden_color_before_any_downsample() {
        let mut first = Builder::new(2048, 1);
        let mut second = Builder::new(2048, 1);
        let row_a = (0..2048).map(|x| if x % 2 == 0 { [1.0, 0.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0, 1.0] }).collect::<Vec<_>>();
        let row_b = (0..2048).map(|x| if x % 2 == 0 { [0.0, 1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0, 1.0] }).collect::<Vec<_>>();
        first.add_row(&row_a, 0);
        second.add_row(&row_b, 0);
        let first = first.finish();
        let second = second.finish();
        assert_eq!([first.width, first.height], [1024, 1]);
        assert_eq!(first.pixels, second.pixels);
        assert!(first.pixels.iter().all(|p| *p == [0.0, 0.0, 1.0, 0.5]));
    }

    #[test]
    fn analysis_proxy_preserves_opaque_values_and_averages_rows() {
        let mut builder = Builder::new(1, 2048);
        for y in 0..2048 {
            builder.add_row(&[[y as f32 / 2048.0, 0.25, 0.75, 1.0]], y);
        }
        let source = builder.finish();
        assert_eq!([source.width, source.height], [1, 1024]);
        assert_eq!(source.pixels.first(), Some(&[0.5 / 2048.0, 0.25, 0.75, 1.0]));
        assert!(source.pixels.iter().all(|p| p[3] == 1.0));
    }
}
