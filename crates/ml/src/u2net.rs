//! U²-Net's small U2NETP salient-object model, evaluated locally with Candle.
//!
//! Native implementation of the published residual U-block architecture, using
//! the original Apache-2.0 checkpoint names. This is a saliency model, not a
//! semantic classifier: its output does not identify sky, people or object names.
//! Architecture: <https://github.com/xuebinqin/U-2-Net>. The separate MIT sky
//! checkpoint uses this same graph with folded normalization and sky training.

use candle_core::{DType, Device, Tensor};
use candle_nn::{BatchNorm, Conv2d, Conv2dConfig, Module, ModuleT, VarBuilder};

/// Fixed model input/output edge. Callers resize the analysis image to this size.
pub const INPUT_SIDE: usize = 320;
/// Input/output edge of the independently trained small sky checkpoint.
pub const SKY_INPUT_SIDE: usize = 384;
const MID_CHANNELS: usize = 16;
const OUT_CHANNELS: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Flavor {
    Foreground,
    Sky,
}

impl Flavor {
    fn side(self) -> usize {
        match self {
            Self::Foreground => INPUT_SIDE,
            Self::Sky => SKY_INPUT_SIDE,
        }
    }
}

struct RebnConv {
    conv: Conv2d,
    norm: Option<BatchNorm>,
}

impl RebnConv {
    fn new(input: usize, output: usize, dilation: usize, folded: bool, vb: VarBuilder<'_>) -> candle_core::Result<Self> {
        let config = Conv2dConfig { padding: dilation, dilation, ..Default::default() };
        Ok(Self {
            conv: candle_nn::conv2d(input, output, 3, config, vb.pp("conv_s1"))?,
            norm: if folded { None } else { Some(candle_nn::batch_norm(output, 1e-5, vb.pp("bn_s1"))?) },
        })
    }

    fn forward(&self, image: &Tensor) -> candle_core::Result<Tensor> {
        let image = self.conv.forward(image)?;
        match &self.norm {
            Some(norm) => norm.forward_t(&image, false)?.relu(),
            None => image.relu(),
        }
    }
}

/// Half-pixel-centred bilinear resizing, matching PyTorch align_corners=false.
/// Candle 0.9's interpolate2d is nearest-neighbour and must not be used here.
fn resize_bilinear(image: &Tensor, height: usize, width: usize) -> candle_core::Result<Tensor> {
    let (_, _, input_height, input_width) = image.dims4()?;
    if input_width == 0 || input_height == 0 || width == 0 || height == 0 {
        return Err(candle_core::Error::Msg("U2NETP resize needs nonempty dimensions".into()));
    }
    if (height, width) == (input_height, input_width) {
        return Ok(image.clone());
    }
    let axis = |source: usize, target: usize| -> candle_core::Result<(Tensor, Tensor, Tensor)> {
        let mut lower = Vec::with_capacity(target);
        let mut upper = Vec::with_capacity(target);
        let mut fraction = Vec::with_capacity(target);
        for output in 0..target {
            let position = ((output as f64 + 0.5) * source as f64 / target as f64 - 0.5).max(0.0);
            let low = (position.floor() as usize).min(source - 1);
            lower.push(low as u32);
            upper.push((low + 1).min(source - 1) as u32);
            fraction.push((position - low as f64) as f32);
        }
        Ok((
            Tensor::from_vec(lower, target, image.device())?,
            Tensor::from_vec(upper, target, image.device())?,
            Tensor::from_vec(fraction, target, image.device())?,
        ))
    };
    let (left, right, horizontal_weight) = axis(input_width, width)?;
    let horizontal_weight = horizontal_weight.reshape((1, 1, 1, width))?;
    let left = image.index_select(&left, 3)?;
    let right = image.index_select(&right, 3)?;
    let horizontal = (&left + (&right - &left)?.broadcast_mul(&horizontal_weight)?)?;
    let (top, bottom, vertical_weight) = axis(input_height, height)?;
    let vertical_weight = vertical_weight.reshape((1, 1, height, 1))?;
    let top = horizontal.index_select(&top, 2)?;
    let bottom = horizontal.index_select(&bottom, 2)?;
    (&top + (&bottom - &top)?.broadcast_mul(&vertical_weight)?)?.contiguous()
}

fn resize_like(image: &Tensor, target: &Tensor) -> candle_core::Result<Tensor> {
    let (_, _, height, width) = target.dims4()?;
    resize_bilinear(image, height, width)
}

/// Repeating the final row/column gives the same maximum as a partial ceil-mode
/// pool cell, including negative inputs, without padding with a foreground zero.
fn pool_ceil(image: &Tensor) -> candle_core::Result<Tensor> {
    let (_, _, height, width) = image.dims4()?;
    if height == 0 || width == 0 {
        return Err(candle_core::Error::Msg("U2NETP pooling needs nonempty dimensions".into()));
    }
    let mut padded = image.clone();
    if !height.is_multiple_of(2) {
        padded = Tensor::cat(&[&padded, &padded.narrow(2, height - 1, 1)?], 2)?;
    }
    if !width.is_multiple_of(2) {
        padded = Tensor::cat(&[&padded, &padded.narrow(3, width - 1, 1)?], 3)?;
    }
    padded.max_pool2d(2)
}

struct ResidualUBlock {
    input: RebnConv,
    encoder: Vec<RebnConv>,
    bottom: RebnConv,
    decoder: Vec<RebnConv>,
    pooled: bool,
}

impl ResidualUBlock {
    /// Depth 4..7 creates RSU4..RSU7; zero denotes the dilated RSU4F.
    fn new(input: usize, depth: usize, folded: bool, vb: VarBuilder<'_>) -> candle_core::Result<Self> {
        if depth != 0 && !(4..=7).contains(&depth) {
            return Err(candle_core::Error::Msg("Invalid U2NETP residual block depth".into()));
        }
        let pooled = depth != 0;
        let levels = if pooled { depth - 1 } else { 3 };
        let mut encoder = Vec::with_capacity(levels);
        let mut decoder = Vec::with_capacity(levels);
        for level in 1..=levels {
            encoder.push(RebnConv::new(
                if level == 1 { OUT_CHANNELS } else { MID_CHANNELS },
                MID_CHANNELS,
                if pooled { 1 } else { 1 << (level - 1) },
                folded,
                vb.pp(format!("rebnconv{level}")),
            )?);
        }
        for level in (1..=levels).rev() {
            decoder.push(RebnConv::new(
                2 * MID_CHANNELS,
                if level == 1 { OUT_CHANNELS } else { MID_CHANNELS },
                if pooled { 1 } else { 1 << (level - 1) },
                folded,
                vb.pp(format!("rebnconv{level}d")),
            )?);
        }
        Ok(Self {
            input: RebnConv::new(input, OUT_CHANNELS, 1, folded, vb.pp("rebnconvin"))?,
            encoder,
            bottom: RebnConv::new(MID_CHANNELS, MID_CHANNELS, if pooled { 2 } else { 8 }, folded, vb.pp(format!("rebnconv{}", levels + 1)))?,
            decoder,
            pooled,
        })
    }

    fn forward(&self, image: &Tensor) -> candle_core::Result<Tensor> {
        let residual = self.input.forward(image)?;
        let mut current = residual.clone();
        let mut skips = Vec::with_capacity(self.encoder.len());
        for (level, encoder) in self.encoder.iter().enumerate() {
            if self.pooled && level > 0 {
                current = pool_ceil(&current)?;
            }
            current = encoder.forward(&current)?;
            skips.push(current.clone());
        }
        current = self.bottom.forward(&current)?;
        for (decoder, skip) in self.decoder.iter().zip(skips.iter().rev()) {
            current = decoder.forward(&Tensor::cat(&[resize_like(&current, skip)?, skip.clone()], 1)?)?;
        }
        current + residual
    }
}

/// Small U2NETP foreground or separately trained sky predictor. No file/network I/O or
/// document mutation occurs here. Keep and reuse this object between requests.
pub struct Segmenter {
    encoder: Vec<ResidualUBlock>,
    decoder: Vec<ResidualUBlock>,
    sides: Vec<Conv2d>,
    output: Conv2d,
    flavor: Flavor,
}

impl Segmenter {
    pub fn new(weights: Vec<u8>) -> Result<Self, String> {
        Self::load(weights, Flavor::Foreground)
    }

    /// Loads the converted MIT small sky checkpoint. Its BatchNorm is already
    /// folded into each convolution and its image normalization is different.
    pub fn new_sky(weights: Vec<u8>) -> Result<Self, String> {
        Self::load(weights, Flavor::Sky)
    }

    fn load(weights: Vec<u8>, flavor: Flavor) -> Result<Self, String> {
        if weights.is_empty() || weights.len() > 16 * 1024 * 1024 {
            return Err("U2NETP checkpoint must be a nonempty safetensors file of at most 16 MiB".into());
        }
        let vb = VarBuilder::from_buffered_safetensors(weights, DType::F32, &Device::Cpu).map_err(|e| e.to_string())?;
        Self::from_builder(vb, flavor).map_err(|e| e.to_string())
    }

    fn from_builder(vb: VarBuilder<'_>, flavor: Flavor) -> candle_core::Result<Self> {
        let folded = flavor == Flavor::Sky;
        let mut encoder = Vec::with_capacity(6);
        for (index, depth) in [7, 6, 5, 4, 0, 0].into_iter().enumerate() {
            encoder.push(ResidualUBlock::new(if index == 0 { 3 } else { OUT_CHANNELS }, depth, folded, vb.pp(format!("stage{}", index + 1)))?);
        }
        let mut decoder = Vec::with_capacity(5);
        for (index, depth) in [0, 4, 5, 6, 7].into_iter().enumerate() {
            decoder.push(ResidualUBlock::new(2 * OUT_CHANNELS, depth, folded, vb.pp(format!("stage{}d", 5 - index)))?);
        }
        let mut sides = Vec::with_capacity(6);
        for level in 1..=6 {
            sides.push(candle_nn::conv2d(OUT_CHANNELS, 1, 3, Conv2dConfig { padding: 1, ..Default::default() }, vb.pp(format!("side{level}")))?);
        }
        Ok(Self { encoder, decoder, sides, output: candle_nn::conv2d(6, 1, 1, Default::default(), vb.pp("outconv"))?, flavor })
    }

    fn forward(&self, image: &Tensor) -> candle_core::Result<Tensor> {
        let mut current = image.clone();
        let mut skips = Vec::with_capacity(self.encoder.len());
        for (index, encoder) in self.encoder.iter().enumerate() {
            if index > 0 {
                current = pool_ceil(&current)?;
            }
            current = encoder.forward(&current)?;
            skips.push(current.clone());
        }
        let side6 = self.sides.last().ok_or_else(|| candle_core::Error::Msg("U2NETP has no output heads".into()))?;
        let side = self.input_side();
        let mut logits = vec![resize_bilinear(&side6.forward(&current)?, side, side)?];
        for ((decoder, skip), side) in self.decoder.iter().zip(skips.iter().rev().skip(1)).zip(self.sides.iter().rev().skip(1)) {
            current = decoder.forward(&Tensor::cat(&[resize_like(&current, skip)?, skip.clone()], 1)?)?;
            logits.push(resize_bilinear(&side.forward(&current)?, self.input_side(), self.input_side())?);
        }
        logits.reverse();
        candle_nn::ops::sigmoid(&self.output.forward(&Tensor::cat(&logits, 1)?)?)
    }

    /// Required square RGB input edge: 320 for foreground or 384 for sky.
    pub fn input_side(&self) -> usize {
        self.flavor.side()
    }

    /// Interleaved sRGB in 0..1, exactly input_side()²×3, after caller-side
    /// resizing. Foreground applies reference image/max then ImageNet mean/std;
    /// sky applies ImageNet mean/std directly. All-black inputs remain finite.
    /// Returns fused d0 sigmoid probabilities (not per-image min/max stretched).
    pub fn predict(&self, rgb: &[f32]) -> Result<Vec<f32>, String> {
        let image = prepare_input(rgb, self.flavor)?;
        let values = self.forward(&image).and_then(|p| p.flatten_all()?.to_vec1::<f32>()).map_err(|e| e.to_string())?;
        if values.len() != self.input_side() * self.input_side() || values.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
            return Err("U2NETP produced invalid foreground probabilities".into());
        }
        Ok(values)
    }

    pub fn predict_sky(&self, rgb: &[f32]) -> Result<Vec<f32>, String> {
        if self.flavor != Flavor::Sky {
            return Err("Sky prediction needs a sky checkpoint".into());
        }
        self.predict(rgb)
    }
}

fn prepare_input(rgb: &[f32], flavor: Flavor) -> Result<Tensor, String> {
    let side = flavor.side();
    if rgb.len() != side * side * 3 || rgb.iter().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
        return Err(format!("U2NETP input must be {side}×{side} interleaved finite sRGB in 0..1"));
    }
    let maximum = if flavor == Flavor::Foreground { rgb.iter().copied().fold(0.0f32, f32::max) } else { 1.0 };
    let maximum = if maximum > 0.0 { maximum } else { 1.0 };
    let means = [0.485f32, 0.456, 0.406];
    let deviations = [0.229f32, 0.224, 0.225];
    let mut normalized = vec![0.0f32; rgb.len()];
    let plane = side * side;
    for (pixel, channels) in rgb.as_chunks::<3>().0.iter().enumerate() {
        for (channel, (value, (mean, deviation))) in channels.iter().zip(means.iter().zip(&deviations)).enumerate() {
            // The destination is fixed-size and the source is validated above.
            if let Some(output) = normalized.get_mut(channel * plane + pixel) {
                *output = (*value / maximum - *mean) / *deviation;
            }
        }
    }
    Tensor::from_vec(normalized, (1, 3, side, side), &Device::Cpu).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_inputs_and_checkpoints_return_errors() {
        for bytes in [vec![], vec![0; 128]] {
            assert!(Segmenter::new(bytes.clone()).is_err());
            assert!(Segmenter::new_sky(bytes).is_err());
        }
        assert!(prepare_input(&[], Flavor::Foreground).is_err());
        let mut input = vec![0.0; INPUT_SIDE * INPUT_SIDE * 3];
        for value in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
            input[42] = value;
            assert!(prepare_input(&input, Flavor::Foreground).is_err());
        }
    }

    #[test]
    fn normalization_matches_reference_and_black_input_stays_finite() {
        let input = vec![0.0; INPUT_SIDE * INPUT_SIDE * 3];
        let black = prepare_input(&input, Flavor::Foreground).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert!(black.iter().all(|v| v.is_finite()));
        assert_eq!(black[0], -0.485 / 0.229);
        assert_eq!(black[INPUT_SIDE * INPUT_SIDE], -0.456 / 0.224);
        let gray = prepare_input(&vec![0.5; input.len()], Flavor::Foreground).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(gray[0], (1.0 - 0.485) / 0.229);
        assert_eq!(gray[2 * INPUT_SIDE * INPUT_SIDE], (1.0 - 0.406) / 0.225);
    }

    #[test]
    fn sky_input_keeps_exposure_and_uses_its_own_384_grid() {
        let pixels = vec![0.5; SKY_INPUT_SIDE * SKY_INPUT_SIDE * 3];
        let gray = prepare_input(&pixels, Flavor::Sky).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(gray[0], (0.5 - 0.485) / 0.229);
        assert_eq!(gray[2 * SKY_INPUT_SIDE * SKY_INPUT_SIDE], (0.5 - 0.406) / 0.225);
        assert!(prepare_input(&pixels, Flavor::Foreground).is_err());
        assert!(prepare_input(&vec![0.5; INPUT_SIDE * INPUT_SIDE * 3], Flavor::Sky).is_err());
        let black = prepare_input(&vec![0.0; pixels.len()], Flavor::Sky).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert!(black.iter().all(|value| value.is_finite()));
        assert_eq!(black[0], -0.485 / 0.229);
    }

    #[test]
    fn folded_sky_convolution_needs_no_synthetic_batchnorm() {
        let tensors = std::collections::HashMap::from([
            ("conv_s1.weight".into(), Tensor::zeros((16, 3, 3, 3), DType::F32, &Device::Cpu).unwrap()),
            ("conv_s1.bias".into(), Tensor::ones(16, DType::F32, &Device::Cpu).unwrap()),
        ]);
        let vb = VarBuilder::from_tensors(tensors, DType::F32, &Device::Cpu);
        assert!(RebnConv::new(3, 16, 1, false, vb.clone()).is_err(), "the subject variant must require real BN parameters");
        let folded = RebnConv::new(3, 16, 1, true, vb).unwrap();
        let input = Tensor::zeros((1, 3, 2, 2), DType::F32, &Device::Cpu).unwrap();
        let output = folded.forward(&input).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(output, vec![1.0; 16 * 2 * 2]);
    }

    #[test]
    fn bilinear_resize_uses_half_pixel_coordinates() {
        let input = Tensor::from_vec(vec![0f32, 1., 2., 3.], (1, 1, 2, 2), &Device::Cpu).unwrap();
        let resized = resize_bilinear(&input, 4, 4).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(resized, vec![0., 0.25, 0.75, 1., 0.5, 0.75, 1.25, 1.5, 1.5, 1.75, 2.25, 2.5, 2., 2.25, 2.75, 3.]);
        let narrow = resize_bilinear(&input, 1, 1).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(narrow, vec![1.5]);
    }

    #[test]
    fn ceil_pool_does_not_introduce_zero_into_negative_edges() {
        let input = Tensor::from_vec((-9..0).map(|v| v as f32).collect::<Vec<_>>(), (1, 1, 3, 3), &Device::Cpu).unwrap();
        let pooled = pool_ceil(&input).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(pooled, vec![-5., -4., -2., -1.]);
    }

    #[test]
    fn zero_weight_graph_matches_constant_sigmoid_output() {
        let model = Segmenter::from_builder(VarBuilder::zeros(DType::F32, &Device::Cpu), Flavor::Foreground).unwrap();
        // A 32px tensor exercises every encoder/decoder, including ceil-pooling
        // and the final fixed 320px output, without a full-resolution conv test.
        let image = Tensor::zeros((1, 3, 32, 32), DType::F32, &Device::Cpu).unwrap();
        let values = model.forward(&image).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(values.len(), INPUT_SIDE * INPUT_SIDE);
        assert!(values.iter().all(|v| *v == 0.5));
        assert_eq!(model.input_side(), INPUT_SIDE);
        assert!(model.predict_sky(&[]).is_err());
        let sky = Segmenter::from_builder(VarBuilder::zeros(DType::F32, &Device::Cpu), Flavor::Sky).unwrap();
        assert_eq!(sky.input_side(), SKY_INPUT_SIDE);
        let values = sky.forward(&image).unwrap().flatten_all().unwrap().to_vec1::<f32>().unwrap();
        assert_eq!(values.len(), SKY_INPUT_SIDE * SKY_INPUT_SIDE);
        assert!(values.iter().all(|value| *value == 0.5));
    }
}
