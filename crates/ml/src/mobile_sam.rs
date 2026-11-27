//! MobileSAM inference through Candle (Apache-2.0/MIT), safe buffered weights.
use candle_core::{DType, Device, Tensor};
use candle_nn::{Module, VarBuilder};
use candle_transformers::models::segment_anything::{
    mask_decoder::MaskDecoder,
    prompt_encoder::PromptEncoder,
    tiny_vit::{TinyViT, tiny_vit_5m},
};

pub struct Segmenter {
    encoder: TinyViT,
    prompts: PromptEncoder,
    decoder: MaskDecoder,
    embedding: Option<(u128, usize, usize, Tensor)>,
}

impl Segmenter {
    pub fn new(weights: Vec<u8>) -> Result<Self, String> {
        let vb = VarBuilder::from_buffered_safetensors(weights, DType::F32, &Device::Cpu).map_err(|e| e.to_string())?;
        // MobileSAM checkpoint architecture: 256-wide prompts, 64² embeddings,
        // 16 mask channels, three ambiguity candidates and a depth-3 IoU head.
        let encoder = tiny_vit_5m(vb.pp("image_encoder")).map_err(|e| e.to_string())?;
        let prompts = PromptEncoder::new(256, (64, 64), (1024, 1024), 16, vb.pp("prompt_encoder")).map_err(|e| e.to_string())?;
        let decoder = MaskDecoder::new(256, 3, 3, 256, vb.pp("mask_decoder")).map_err(|e| e.to_string())?;
        Ok(Self { encoder, prompts, decoder, embedding: None })
    }

    pub fn has_image(&self, key: u128) -> bool {
        self.embedding.as_ref().is_some_and(|(k, _, _, _)| *k == key)
    }

    /// Interleaved RGB 0..255, aspect-preserving resized image, longest side 1024.
    pub fn set_image(&mut self, key: u128, rgb: Vec<f32>, width: usize, height: usize) -> Result<(), String> {
        if width == 0 || height == 0 || width > 1024 || height > 1024 || rgb.len() != width * height * 3 {
            return Err("Invalid segmentation image dimensions".into());
        }
        let run = || -> candle_core::Result<Tensor> {
            let image = Tensor::from_vec(rgb, (height, width, 3), &Device::Cpu)?.permute((2, 0, 1))?.contiguous()?;
            let mean = Tensor::new(&[123.675f32, 116.28, 103.53], &Device::Cpu)?.reshape((3, 1, 1))?;
            let std = Tensor::new(&[58.395f32, 57.12, 57.375], &Device::Cpu)?.reshape((3, 1, 1))?;
            let image =
                image.broadcast_sub(&mean)?.broadcast_div(&std)?.pad_with_zeros(1, 0, 1024 - height)?.pad_with_zeros(2, 0, 1024 - width)?.unsqueeze(0)?;
            self.encoder.forward(&image)
        };
        let embedding = run().map_err(|e| e.to_string())?;
        self.embedding = Some((key, width, height, embedding));
        Ok(())
    }

    /// Normalized foreground/background clicks. Returns 256² mask logits before
    /// cropping away the padded part, for bilinear contour reconstruction.
    pub fn predict(&self, points: &[(f64, f64, bool)], rect: Option<[f64; 4]>) -> Result<Vec<f32>, String> {
        if (points.is_empty() && rect.is_none())
            || points.len() > 64
            || points.iter().any(|(x, y, _)| !x.is_finite() || !y.is_finite() || !(0.0..=1.0).contains(x) || !(0.0..=1.0).contains(y))
        {
            return Err("Segmentation needs 1–64 valid image points".into());
        }
        if rect.is_some_and(|r| r.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) || r[0] >= r[2] || r[1] >= r[3]) {
            return Err("Invalid segmentation box".into());
        }
        // SAM reserves its three ambiguity masks for a single foreground/background click.
        // Multiple clicks and boxes are an explicit request and use the dedicated single mask.
        let multimask = points.len() == 1 && rect.is_none();
        let (masks, scores) = self.decode(points, rect, multimask)?;
        let (_, width, height, _) = self.embedding.as_ref().ok_or("No segmentation image loaded")?;
        let index = if multimask { select_candidate(&masks, &scores, points, *width, *height)? } else { 0 };
        let begin = index * MASK_PIXELS;
        let mask = masks.get(begin..begin + MASK_PIXELS).ok_or("Invalid segmentation mask dimensions")?.to_vec();
        if mask.iter().any(|v| !v.is_finite()) {
            return Err("Invalid segmentation mask".into());
        }
        Ok(mask)
    }

    fn decode(&self, points: &[(f64, f64, bool)], rect: Option<[f64; 4]>, multimask: bool) -> Result<(Vec<f32>, Vec<f32>), String> {
        let (_, width, height, embedding) = self.embedding.as_ref().ok_or("No segmentation image loaded")?;
        let run = || -> candle_core::Result<(Vec<f32>, Vec<f32>)> {
            let point_tensors = if points.is_empty() {
                None
            } else {
                let xy: Vec<f32> = points.iter().flat_map(|(x, y, _)| [*x as f32 * *width as f32, *y as f32 * *height as f32]).collect();
                let labels: Vec<f32> = points.iter().map(|p| if p.2 { 1. } else { 0. }).collect();
                Some((Tensor::from_vec(xy, (1, points.len(), 2), &Device::Cpu)?, Tensor::from_vec(labels, (1, points.len()), &Device::Cpu)?))
            };
            let boxes = rect
                .map(|r| {
                    Tensor::new(
                        &[[r[0] as f32 * *width as f32, r[1] as f32 * *height as f32, r[2] as f32 * *width as f32, r[3] as f32 * *height as f32]],
                        &Device::Cpu,
                    )
                })
                .transpose()?;
            let (mut sparse, dense) = self.prompts.forward(point_tensors.as_ref().map(|(xy, l)| (xy, l)), None, None)?;
            if let Some(boxes) = boxes {
                // Candle 0.9.1 flattens box corners to [batch,512]. Restore two
                // [batch,2,256] tokens and remove the point-only padding token.
                let (corners, _) = self.prompts.forward(None, Some(&boxes), None)?;
                let corners = corners.reshape((1, 2, 256))?;
                sparse = sparse.narrow(1, 0, points.len())?;
                sparse = Tensor::cat(&[sparse, corners], 1)?;
            }
            let (masks, scores) = self.decoder.forward(embedding, &self.prompts.get_dense_pe()?, &sparse, &dense, multimask)?;
            Ok((masks.flatten_all()?.to_vec1::<f32>()?, scores.flatten_all()?.to_vec1::<f32>()?))
        };
        run().map_err(|error| error.to_string())
    }
}

const MASK_SIDE: usize = 256;
const MASK_PIXELS: usize = MASK_SIDE * MASK_SIDE;

/// Select by the model's predicted IoU after honoring explicit clicks. Mask area
/// is not quality: a background/whole-image candidate can be much larger than the
/// intended object while scoring slightly worse, especially for a small object.
fn select_candidate(masks: &[f32], scores: &[f32], points: &[(f64, f64, bool)], width: usize, height: usize) -> Result<usize, String> {
    if scores.is_empty() || scores.len() > 3 || masks.len() != scores.len() * MASK_PIXELS {
        return Err("Invalid segmentation candidates".into());
    }
    let agrees = |mask: &[f32]| {
        points.iter().all(|(x, y, fg)| {
            let ix = (*x * width as f64 / 4.).clamp(0., 255.) as usize;
            let iy = (*y * height as f64 / 4.).clamp(0., 255.) as usize;
            mask.get(iy * MASK_SIDE + ix).is_some_and(|value| value.is_finite() && (*value > 0.) == *fg)
        })
    };
    let valid = |index: usize| masks.get(index * MASK_PIXELS..(index + 1) * MASK_PIXELS).filter(|mask| mask.iter().all(|value| value.is_finite()));
    let mut best = None;
    let mut fallback = None;
    for (index, score) in scores.iter().copied().enumerate().filter(|(_, score)| score.is_finite()) {
        let Some(mask) = valid(index) else { continue };
        if fallback.is_none_or(|(_, previous)| score > previous) {
            fallback = Some((index, score));
        }
        if agrees(mask) && best.is_none_or(|(_, previous)| score > previous) {
            best = Some((index, score));
        }
    }
    best.or(fallback).map(|(index, _)| index).ok_or_else(|| "No valid segmentation candidate".into())
}

#[cfg(test)]
#[path = "mobile_sam_tests_v82.rs"]
mod tests_v82;
