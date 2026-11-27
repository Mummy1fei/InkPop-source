//! SAM 2.1 adapter for the pinned onnx-community encoder/prompt decoder.
//!
//! Unlike MobileSAM, this export stretches the source to 1024 square without
//! padding. Its box tensor is empty for point-only prompts; that exported branch
//! has been independently exercised with Python ORT on CPU and DirectML.
use crate::onnx_segment::{Execution, Mask, OutputKind, guarded, load_session, named_input, named_output, prepare_rgb, probabilities};
use ort::session::Session;
use ort::tensor::TensorElementType;
use ort::value::{DynValue, Tensor};
use std::path::Path;

const SIDE: usize = 1024;
const EMBEDDINGS: [(&str, [i64; 4]); 3] =
    [("image_embeddings.0", [1, 32, 256, 256]), ("image_embeddings.1", [1, 64, 128, 128]), ("image_embeddings.2", [1, 256, 64, 64])];

struct Embedding {
    key: u128,
    high0: DynValue,
    high1: DynValue,
    image: DynValue,
}

pub struct Segmenter {
    encoder: Session,
    decoder: Session,
    embedding: Option<Embedding>,
    execution: Execution,
}

fn checked_embedding(value: &DynValue, expected: &[i64; 4]) -> Result<(), String> {
    let (shape, data) = value.try_extract_tensor::<f32>().map_err(|error| error.to_string())?;
    if shape.as_ref() != expected || data.iter().any(|value| !value.is_finite()) {
        return Err("SAM 2 returned invalid image features".into());
    }
    Ok(())
}

impl Segmenter {
    pub fn load(runtime: &Path, encoder: &Path, decoder: &Path, execution: Execution) -> Result<Self, String> {
        let encoder = load_session(runtime, encoder, execution)?;
        let decoder = load_session(runtime, decoder, execution)?;
        if encoder.inputs.len() != 1 || encoder.outputs.len() != 3 || decoder.inputs.len() != 6 || decoder.outputs.len() != 3 {
            return Err("Unsupported SAM 2 encoder/decoder contract".into());
        }
        named_input(&encoder, "pixel_values", &[1, 3, 1024, 1024], TensorElementType::Float32)?;
        for (name, shape) in EMBEDDINGS {
            named_output(&encoder, name, &shape, TensorElementType::Float32)?;
            named_input(&decoder, name, &shape, TensorElementType::Float32)?;
        }
        named_input(&decoder, "input_points", &[1, 1, -1, 2], TensorElementType::Float32)?;
        named_input(&decoder, "input_labels", &[1, 1, -1], TensorElementType::Int64)?;
        named_input(&decoder, "input_boxes", &[1, -1, 4], TensorElementType::Float32)?;
        named_output(&decoder, "iou_scores", &[1, 1, 3], TensorElementType::Float32)?;
        named_output(&decoder, "pred_masks", &[1, 1, 3, -1, -1], TensorElementType::Float32)?;
        named_output(&decoder, "object_score_logits", &[1, 1, 1], TensorElementType::Float32)?;
        Ok(Self { encoder, decoder, embedding: None, execution })
    }

    pub fn execution(&self) -> Execution {
        self.execution
    }

    pub fn has_image(&self, key: u128) -> bool {
        self.embedding.as_ref().is_some_and(|embedding| embedding.key == key)
    }

    pub fn set_image(&mut self, key: u128, rgb: &[[f32; 3]], width: usize, height: usize) -> Result<(), String> {
        let values = prepare_rgb(rgb, width, height, SIDE)?;
        // A failed replacement cannot leave old features associated with a new
        // source. Upper layers bind the key to exact analysis pixels/geometry.
        self.embedding = None;
        let embedding = guarded(|| {
            let input = Tensor::from_array(([1usize, 3, SIDE, SIDE], values.into_boxed_slice())).map_err(|error| error.to_string())?;
            let mut outputs = self.encoder.run(ort::inputs!["pixel_values" => input]).map_err(|error| error.to_string())?;
            let high0 = outputs.remove("image_embeddings.0").ok_or("SAM 2 feature 0 is missing")?;
            let high1 = outputs.remove("image_embeddings.1").ok_or("SAM 2 feature 1 is missing")?;
            let image = outputs.remove("image_embeddings.2").ok_or("SAM 2 image embedding is missing")?;
            checked_embedding(&high0, &[1, 32, 256, 256])?;
            checked_embedding(&high1, &[1, 64, 128, 128])?;
            checked_embedding(&image, &[1, 256, 64, 64])?;
            Ok(Embedding { key, high0, high1, image })
        })?;
        self.embedding = Some(embedding);
        Ok(())
    }

    /// Normalized document points/box. Point-only prompts use an empty box
    /// tensor; no full-image box or foreground centre is guessed.
    pub fn predict(&mut self, points: &[(f64, f64, bool)], rect: Option<[f64; 4]>) -> Result<Mask, String> {
        let box_count = usize::from(rect.is_some());
        let (coordinates, labels, rectangle) = prompt_tensors(points, rect)?;
        let embedding = self.embedding.as_ref().ok_or("No SAM 2 image is loaded")?;
        guarded(|| {
            let count = labels.len();
            let coordinates = Tensor::from_array(([1usize, 1, count, 2], coordinates.into_boxed_slice())).map_err(|error| error.to_string())?;
            let labels = Tensor::from_array(([1usize, 1, count], labels.into_boxed_slice())).map_err(|error| error.to_string())?;
            let rectangle = if box_count == 0 {
                // rc.10's raw-array constructor rejects zero dimensions. The
                // safe allocator constructor supports this real empty tensor;
                // there are no elements to initialize or expose to the model.
                Tensor::<f32>::new(&ort::memory::Allocator::default(), [1usize, 0, 4]).map_err(|error| error.to_string())?
            } else {
                Tensor::from_array(([1usize, 1, 4], rectangle.into_boxed_slice())).map_err(|error| error.to_string())?
            };
            let outputs = self
                .decoder
                .run(ort::inputs![
                    "input_points" => coordinates,
                    "input_labels" => labels,
                    "input_boxes" => rectangle,
                    "image_embeddings.0" => &embedding.high0,
                    "image_embeddings.1" => &embedding.high1,
                    "image_embeddings.2" => &embedding.image
                ])
                .map_err(|error| error.to_string())?;
            let mask = outputs.get("pred_masks").ok_or("SAM 2 masks are missing")?;
            let (shape, masks) = mask.try_extract_tensor::<f32>().map_err(|error| error.to_string())?;
            let dimensions: &[i64] = shape.as_ref();
            let [1, 1, 3, height, width] = dimensions else { return Err("SAM 2 mask rank/count differs from the vetted export".into()) };
            if *width <= 0 || *height <= 0 || *width > SIDE as i64 || *height > SIDE as i64 {
                return Err("SAM 2 mask geometry exceeds its limit".into());
            }
            let (width, height) = (*width as usize, *height as usize);
            let scores = outputs.get("iou_scores").ok_or("SAM 2 scores are missing")?;
            let (score_shape, scores) = scores.try_extract_tensor::<f32>().map_err(|error| error.to_string())?;
            if score_shape.as_ref() != [1, 1, 3] {
                return Err("SAM 2 scores have the wrong shape".into());
            }
            select_mask(masks, scores, width, height, points)
        })
    }
}

type PromptTensors = (Vec<f32>, Vec<i64>, Vec<f32>);

fn prompt_tensors(points: &[(f64, f64, bool)], rect: Option<[f64; 4]>) -> Result<PromptTensors, String> {
    if points.len() > 64
        || points.iter().any(|(x, y, _)| !x.is_finite() || !y.is_finite() || !(0.0..=1.0).contains(x) || !(0.0..=1.0).contains(y))
        || rect.is_none() && !points.iter().any(|point| point.2)
        || rect.is_some_and(|rect| {
            rect.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                || rect.first().zip(rect.get(2)).is_none_or(|(x0, x1)| x0 >= x1)
                || rect.get(1).zip(rect.get(3)).is_none_or(|(y0, y1)| y0 >= y1)
        })
    {
        return Err("Invalid SAM 2 point/box prompt".into());
    }
    let (coordinates, labels) = if points.is_empty() {
        // -1 is the upstream padding/not-a-point label, never foreground.
        (vec![0.0, 0.0], vec![-1])
    } else {
        (points.iter().flat_map(|(x, y, _)| [(*x * SIDE as f64) as f32, (*y * SIDE as f64) as f32]).collect(), points.iter().map(|p| i64::from(p.2)).collect())
    };
    Ok((coordinates, labels, rect.into_iter().flatten().map(|value| (value * SIDE as f64) as f32).collect()))
}

fn select_mask(masks: &[f32], scores: &[f32], width: usize, height: usize, points: &[(f64, f64, bool)]) -> Result<Mask, String> {
    let plane = width.checked_mul(height).filter(|plane| *plane > 0 && *plane <= SIDE * SIDE).ok_or("Invalid SAM 2 mask size")?;
    if scores.len() != 3 || masks.len() != plane * 3 || masks.iter().any(|v| !v.is_finite()) {
        return Err("Invalid SAM 2 masks/scores".into());
    }
    let mut best = None;
    for (index, score) in scores.iter().copied().enumerate().filter(|(_, score)| score.is_finite()) {
        let begin = index * plane;
        let mask = masks.get(begin..begin + plane).ok_or("Invalid SAM 2 candidate bounds")?;
        let agrees = points.iter().all(|(x, y, positive)| {
            let x = (*x * width as f64).clamp(0.0, (width - 1) as f64) as usize;
            let y = (*y * height as f64).clamp(0.0, (height - 1) as f64) as usize;
            mask.get(y * width + x).is_some_and(|value| (*value > 0.0) == *positive)
        });
        if agrees && best.is_none_or(|(_, previous)| score > previous) {
            best = Some((index, score));
        }
    }
    let (index, _) = best.ok_or("No SAM 2 candidate satisfies the supplied point constraints")?;
    let begin = index * plane;
    let logits = masks.get(begin..begin + plane).ok_or("Invalid SAM 2 chosen mask")?;
    Ok(Mask { width, height, probabilities: probabilities(logits, OutputKind::Logits)? })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_only_adds_only_a_padding_point_and_stretches_coordinates_without_padding() {
        let (points, labels, rect) = prompt_tensors(&[], Some([0.25, 0.1, 0.75, 1.0])).unwrap();
        assert_eq!(points, [0.0, 0.0]);
        assert_eq!(labels, [-1]);
        assert_eq!(rect, [256.0, 102.4, 768.0, 1024.0]);
        let (points, labels, _) = prompt_tensors(&[(0.5, 0.25, true), (0.1, 0.9, false)], Some([0.0, 0.0, 1.0, 1.0])).unwrap();
        assert_eq!(points, [512.0, 256.0, 102.4, 921.6]);
        assert_eq!(labels, [1, 0]);
        let (_, labels, rectangle) = prompt_tensors(&[(0.5, 0.5, true)], None).unwrap();
        assert_eq!(labels, [1]);
        assert!(rectangle.is_empty(), "point-only does not invent a full-image box");
    }

    #[test]
    fn invalid_points_boxes_and_oversized_prompts_are_rejected() {
        assert!(prompt_tensors(&[], Some([0.4, 0.2, 0.3, 0.9])).is_err());
        assert!(prompt_tensors(&[], Some([0.0, f64::NAN, 1.0, 1.0])).is_err());
        assert!(prompt_tensors(&[(f64::INFINITY, 0.5, true)], Some([0.0, 0.0, 1.0, 1.0])).is_err());
        assert!(prompt_tensors(&[(0.5, 0.5, true); 65], Some([0.0, 0.0, 1.0, 1.0])).is_err());
        assert!(prompt_tensors(&[], None).is_err());
        assert!(prompt_tensors(&[(0.5, 0.5, false)], None).is_err());
    }

    #[test]
    fn candidate_selection_obeys_points_and_not_mask_area() {
        let masks = [-4.0, 4.0, -4.0, -4.0, 4.0, 4.0, 4.0, 4.0, -4.0, 4.0, 4.0, 4.0];
        let mask = select_mask(&masks, &[0.8, 0.9, 0.7], 2, 2, &[(0.9, 0.1, true), (0.1, 0.9, false)]).unwrap();
        assert!(mask.probabilities[1] > 0.9);
        assert!(mask.probabilities[2] < 0.1);
        assert!(select_mask(&masks, &[f32::NAN; 3], 2, 2, &[]).is_err());
        assert!(select_mask(&masks, &[0.8, 0.9, 0.7], 2, 2, &[(0.9, 0.1, false)]).is_err());
    }
}
