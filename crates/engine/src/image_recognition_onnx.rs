//! Shared local ONNX queue and bounded model/result caches. No network access.
use super::{Result, bilinear, invalid, refined, resize_field, retain_confident_foreground};
use crate::jobs::JobCtx;
use photocraft_algo::segment::{Region, RgbImage};
use photocraft_geom::Rect;
use photocraft_ml::onnx_segment::{Execution, ForegroundModel, OutputKind};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError};
use std::time::SystemTime;

static QUEUE: Mutex<()> = Mutex::new(());
static SUBJECT: OnceLock<Mutex<Option<SubjectCache>>> = OnceLock::new();
static OBJECT: OnceLock<Mutex<Option<ObjectCache>>> = OnceLock::new();
// Provider failures outlive a heavy-session eviction. Retain only the last
// signature, so this recovery metadata cannot grow with opened documents.
static OBJECT_DML_REJECTED: Mutex<Option<Signature>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Preference {
    Auto,
    Cpu,
    DirectMl,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileSignature {
    path: PathBuf,
    length: u64,
    modified: Option<SystemTime>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Signature {
    files: Vec<FileSignature>,
    execution: Preference,
}

pub(super) struct SubjectPrediction {
    pub values: Vec<u8>,
    pub model_hit: bool,
    pub result_hit: bool,
    pub execution: &'static str,
    pub fallback: Option<String>,
}

pub(crate) struct ObjectPrediction {
    pub region: Region,
    pub model_hit: bool,
    pub embedding_hit: bool,
    pub execution: &'static str,
    pub fallback: Option<String>,
}

struct SubjectCache {
    signature: Signature,
    model: ForegroundModel,
    fallback: Option<String>,
    prediction: Option<(u128, Vec<u8>)>,
}

struct ObjectCache {
    signature: Signature,
    model: photocraft_ml::sam2_onnx::Segmenter,
    fallback: Option<String>,
}

fn lock<'a, T>(mutex: &'a Mutex<T>, ctx: &JobCtx) -> Result<MutexGuard<'a, T>> {
    loop {
        ctx.check()?;
        match mutex.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(error)) => return Ok(error.into_inner()),
            Err(TryLockError::WouldBlock) => std::thread::sleep(std::time::Duration::from_millis(4)),
        }
    }
}

fn sibling(relative: &str) -> PathBuf {
    std::env::current_exe().ok().and_then(|path| path.parent().map(|dir| dir.join(relative))).unwrap_or_default()
}

fn configured(variable: &str, relative: &str) -> PathBuf {
    std::env::var_os(variable).map(PathBuf::from).unwrap_or_else(|| sibling(relative))
}

fn runtime() -> PathBuf {
    let relative = if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "runtime/libonnxruntime.dylib"
    } else {
        "runtime/libonnxruntime.so"
    };
    configured("PHOTOCRAFT_ONNX_RUNTIME", relative)
}

fn subject_path() -> PathBuf {
    configured("PHOTOCRAFT_BIREFNET_MODEL", "models/birefnet-lite.onnx")
}

fn object_dir() -> PathBuf {
    configured("PHOTOCRAFT_SAM2_MODEL_DIR", "models/sam2")
}

pub(super) fn subject_available() -> bool {
    runtime().is_file() && subject_path().is_file()
}

pub(crate) fn object_available() -> bool {
    let dir = object_dir();
    runtime().is_file()
        && ["vision_encoder.onnx", "vision_encoder.onnx_data", "prompt_encoder_mask_decoder.onnx", "prompt_encoder_mask_decoder.onnx_data"]
            .iter()
            .all(|name| dir.join(name).is_file())
}

fn preference() -> Result<Preference> {
    match std::env::var("PHOTOCRAFT_ML_EXECUTION").as_deref().unwrap_or("auto") {
        "auto" => Ok(Preference::Auto),
        "cpu" => Ok(Preference::Cpu),
        "directml" => Ok(Preference::DirectMl),
        _ => Err(invalid("PHOTOCRAFT_ML_EXECUTION must be auto, cpu or directml")),
    }
}

fn signature(paths: &[&Path]) -> Result<Signature> {
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let path = path.canonicalize().map_err(|_| invalid("local ONNX model or runtime is missing"))?;
        let metadata = std::fs::metadata(&path).map_err(|error| invalid(&error.to_string()))?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 1536 * 1024 * 1024 {
            return Err(invalid("local ONNX file exceeds its size budget"));
        }
        files.push(FileSignature { path, length: metadata.len(), modified: metadata.modified().ok() });
    }
    Ok(Signature { files, execution: preference()? })
}

fn load<T>(preference: Preference, ctx: &JobCtx, mut create: impl FnMut(Execution) -> std::result::Result<T, String>) -> Result<(T, Option<String>)> {
    ctx.check()?;
    if preference == Preference::Cpu || (preference == Preference::Auto && !cfg!(target_os = "windows")) {
        return create(Execution::Cpu).map(|model| (model, None)).map_err(|error| invalid(&error));
    }
    match create(Execution::DirectMl) {
        Ok(model) => Ok((model, None)),
        Err(error) if preference == Preference::Auto => {
            // The preceding native call has returned. Do not start a second
            // expensive model initialization after the caller cancelled it.
            ctx.check()?;
            let model = create(Execution::Cpu).map_err(|cpu| invalid(&format!("DirectML: {error}; CPU: {cpu}")))?;
            Ok((model, Some(format!("DirectML unavailable; using CPU: {error}"))))
        }
        Err(error) => Err(invalid(&error)),
    }
}

fn quarantined(signature: &Signature, rejected: &Option<Signature>) -> bool {
    signature.execution == Preference::Auto && rejected.as_ref() == Some(signature)
}

fn load_object<T>(
    signature: &Signature,
    rejected: &mut Option<Signature>,
    ctx: &JobCtx,
    mut create: impl FnMut(Execution) -> std::result::Result<T, String>,
) -> Result<(T, Option<String>)> {
    let already_rejected = quarantined(signature, rejected);
    let preference = if already_rejected { Preference::Cpu } else { signature.execution };
    let (model, fallback) = load(preference, ctx, |execution| {
        let result = create(execution);
        if execution == Execution::DirectMl && signature.execution == Preference::Auto && result.is_err() {
            *rejected = Some(signature.clone());
        }
        result
    })?;
    Ok((model, if already_rejected { Some("SAM 2 DirectML previously failed for these model files; using CPU".into()) } else { fallback }))
}

fn image_key(image: &RgbImage) -> u128 {
    let mut hash = blake3::Hasher::new();
    hash.update(&(image.w as u64).to_le_bytes());
    hash.update(&(image.h as u64).to_le_bytes());
    for pixel in &image.px {
        for value in pixel {
            hash.update(&value.to_bits().to_le_bytes());
        }
    }
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&hash.finalize().as_bytes()[..16]);
    u128::from_le_bytes(bytes)
}

pub(super) fn subject(image: &RgbImage, ctx: &JobCtx) -> Result<SubjectPrediction> {
    let _queue = lock(&QUEUE, ctx)?;
    // Keep one heavy ONNX model family resident. Old lightweight fallback
    // models have independent small caches; this is not a whole-app RAM cap.
    if let Some(objects) = OBJECT.get() {
        *lock(objects, ctx)? = None;
    }
    let runtime = runtime();
    let path = subject_path();
    let mut signature = signature(&[&runtime, &path])?;
    if signature.execution == Preference::DirectMl {
        return Err(invalid("BiRefNet DirectML is disabled after a device error; use the CPU precise mode"));
    }
    // An earlier BiRefNet export failed a real DirectML test. This new
    // CPU-validated export has not been approved for DML. A working SAM 2
    // provider does not establish that a BiRefNet graph is safe on that device.
    signature.execution = Preference::Cpu;
    let mut cache = lock(SUBJECT.get_or_init(|| Mutex::new(None)), ctx)?;
    let model_hit = cache.as_ref().is_some_and(|cache| cache.signature == signature);
    if !model_hit {
        *cache = None;
        ctx.progress(0.2, "Loading local BiRefNet subject model");
        // This is the vetted v93 Tiny 1024 export contract; source weights and
        // the equivalent local export are recorded separately in provenance.
        let (model, fallback) = load(signature.execution, ctx, |execution| {
            ForegroundModel::load(&runtime, &path, "input_image", "output_image", 1024, OutputKind::Logits, execution)
        })?;
        *cache = Some(SubjectCache { signature, model, fallback, prediction: None });
    }
    ctx.check()?;
    let cache = cache.as_mut().ok_or_else(|| invalid("BiRefNet model is unavailable"))?;
    let key = image_key(image);
    if let Some((_, values)) = cache.prediction.as_ref().filter(|(previous, _)| *previous == key) {
        return Ok(SubjectPrediction {
            values: values.clone(),
            model_hit,
            result_hit: true,
            execution: cache.model.execution().name(),
            fallback: cache.fallback.clone(),
        });
    }
    ctx.progress(0.35, "Recognizing foreground with BiRefNet");
    let mut mask = cache.model.predict_rgb(&image.px, image.w, image.h).map_err(|error| invalid(&error))?;
    ctx.check()?;
    retain_confident_foreground(&mut mask.probabilities);
    let values = resize_field(&mask.probabilities, mask.width, mask.height, image.w, image.h, ctx)?;
    let values = refined(&values, image, ctx)?;
    ctx.check()?;
    cache.prediction = Some((key, values.clone()));
    Ok(SubjectPrediction { values, model_hit, result_hit: false, execution: cache.model.execution().name(), fallback: cache.fallback.clone() })
}

pub(crate) fn object(image: &RgbImage, bounds: Rect, points: &[(f64, f64, bool)], rect: Option<[f64; 4]>, ctx: &JobCtx) -> Result<ObjectPrediction> {
    let _queue = lock(&QUEUE, ctx)?;
    if let Some(subjects) = SUBJECT.get() {
        *lock(subjects, ctx)? = None;
    }
    let runtime = runtime();
    let dir = object_dir();
    let encoder = dir.join("vision_encoder.onnx");
    let decoder = dir.join("prompt_encoder_mask_decoder.onnx");
    let signature = signature(&[&runtime, &encoder, &dir.join("vision_encoder.onnx_data"), &decoder, &dir.join("prompt_encoder_mask_decoder.onnx_data")])?;
    let mut rejected = lock(&OBJECT_DML_REJECTED, ctx)?;
    let mut cache = lock(OBJECT.get_or_init(|| Mutex::new(None)), ctx)?;
    let model_hit = cache.as_ref().is_some_and(|cache| cache.signature == signature);
    if !model_hit {
        *cache = None;
        ctx.progress(0.2, "Loading local SAM 2 object model");
        let (model, fallback) =
            load_object(&signature, &mut rejected, ctx, |execution| photocraft_ml::sam2_onnx::Segmenter::load(&runtime, &encoder, &decoder, execution))?;
        *cache = Some(ObjectCache { signature, model, fallback });
    }
    ctx.check()?;
    let cache = cache.as_mut().ok_or_else(|| invalid("SAM 2 model is unavailable"))?;
    let key = image_key(image);
    if quarantined(&cache.signature, &rejected) && cache.model.execution() == Execution::DirectMl {
        cache.model = photocraft_ml::sam2_onnx::Segmenter::load(&runtime, &encoder, &decoder, Execution::Cpu).map_err(|error| invalid(&error))?;
    }
    let mut embedding_hit = cache.model.has_image(key);
    let run = |model: &mut photocraft_ml::sam2_onnx::Segmenter| -> Result<photocraft_ml::onnx_segment::Mask> {
        if !model.has_image(key) {
            ctx.progress(0.35, "Recognizing image objects with SAM 2");
            model.set_image(key, &image.px, image.w, image.h).map_err(|error| invalid(&error))?;
        }
        ctx.check()?;
        model.predict(points, rect).map_err(|error| invalid(&error))
    };
    let mask = match run(&mut cache.model) {
        Ok(mask) => mask,
        Err(error) if cache.signature.execution == Preference::Auto && cache.model.execution() == Execution::DirectMl => {
            ctx.check()?;
            // Quarantine this provider for the current file signature. Reusing
            // a failed DML session on each click can repeat a device failure.
            *rejected = Some(cache.signature.clone());
            cache.fallback = Some(format!("SAM 2 DirectML request failed; using CPU for this model session: {error}"));
            cache.model = photocraft_ml::sam2_onnx::Segmenter::load(&runtime, &encoder, &decoder, Execution::Cpu).map_err(|error| invalid(&error))?;
            embedding_hit = false;
            run(&mut cache.model)?
        }
        Err(error) => return Err(error),
    };
    ctx.check()?;
    let region = object_coverage(mask, bounds, ctx)?;
    Ok(ObjectPrediction { region, model_hit, embedding_hit, execution: cache.model.execution().name(), fallback: cache.fallback.clone() })
}

/// SAM confidence describes semantic membership, not foreground transparency.
/// Threshold before reconstruction, then integrate the semantic boundary over
/// document subpixels. This confines AA to a document-pixel edge rather than
/// stretching a soft model-confidence band across a large document. Matting
/// models (including BiRefNet) deliberately do not use this conversion.
fn object_coverage(mut mask: photocraft_ml::onnx_segment::Mask, bounds: Rect, ctx: &JobCtx) -> Result<Region> {
    ctx.check()?;
    let source_count = mask
        .width
        .checked_mul(mask.height)
        .filter(|count| mask.width > 0 && mask.height > 0 && mask.width <= 1024 && mask.height <= 1024 && *count <= 1024 * 1024)
        .ok_or_else(|| invalid("invalid SAM 2 coverage shape"))?;
    if mask.probabilities.len() != source_count {
        return Err(invalid("invalid SAM 2 coverage length"));
    }
    let (width, height) = (bounds.width() as usize, bounds.height() as usize);
    let count = width
        .checked_mul(height)
        .filter(|count| width > 0 && height > 0 && width <= i32::MAX as usize && height <= i32::MAX as usize && *count <= super::MAX_INPUT_PIXELS)
        .ok_or_else(|| invalid("invalid object output size"))?;
    // Consume the small model buffer in place: no extra full-resolution float
    // buffer, flood fill or hole filling is needed to reconstruct coverage.
    for row in mask.probabilities.chunks_mut(mask.width) {
        ctx.check()?;
        for value in row {
            if !value.is_finite() || !(0.0..=1.0).contains(value) {
                return Err(invalid("invalid SAM 2 coverage probability"));
            }
            *value = if *value > 0.5 { 1.0 } else { 0.0 };
        }
    }
    ctx.check()?;
    let mut values = Vec::new();
    values.try_reserve_exact(count).map_err(|_| invalid("object coverage allocation exceeds available memory"))?;
    let scale_x = mask.width as f64 / width as f64;
    let scale_y = mask.height as f64 / height as f64;
    for y in 0..height {
        ctx.check()?;
        for x in 0..width {
            if x % 4096 == 0 {
                ctx.check()?;
            }
            // Fixed 2x2 quadrature is bounded and preserves solid interiors,
            // real holes and concavities instead of using confidence as alpha.
            let mut covered = 0u16;
            for dy in [0.25, 0.75] {
                for dx in [0.25, 0.75] {
                    let value = bilinear(
                        &mask.probabilities,
                        mask.width,
                        mask.height,
                        ((x as f64 + dx) * scale_x - 0.5) as f32,
                        ((y as f64 + dy) * scale_y - 0.5) as f32,
                    );
                    covered += u16::from(value > 0.5);
                }
            }
            values.push(((covered * 255 + 2) / 4) as u8);
        }
    }
    ctx.check()?;
    photocraft_algo::segment::trim_region(Region { bbox: bounds, mask: values }).ok_or_else(|| invalid("no object was found inside the box"))
}

#[cfg(test)]
#[path = "object_coverage_tests_v93.rs"]
mod object_coverage_tests_v93;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_fallback_is_explicit_and_forced_directml_never_silently_uses_cpu() {
        let mut requests = Vec::new();
        let (actual, fallback) = load(Preference::Auto, &JobCtx::new(), |execution| {
            requests.push(execution);
            if execution == Execution::DirectMl { Err("test provider unavailable".to_owned()) } else { Ok(execution) }
        })
        .unwrap();
        assert_eq!(actual, Execution::Cpu);
        if cfg!(target_os = "windows") {
            assert_eq!(requests, [Execution::DirectMl, Execution::Cpu]);
            assert!(fallback.unwrap().contains("test provider unavailable"));
        } else {
            assert_eq!(requests, [Execution::Cpu]);
            assert!(fallback.is_none());
        }
        let mut forced_calls = Vec::new();
        assert!(
            load(Preference::DirectMl, &JobCtx::new(), |execution| {
                forced_calls.push(execution);
                Err::<(), _>("unavailable".to_owned())
            })
            .is_err()
        );
        assert_eq!(forced_calls, [Execution::DirectMl]);
    }

    #[test]
    fn image_cache_identity_includes_exact_pixels_and_shape() {
        let image = RgbImage::from_fn(2, 2, |_, _| [0.5; 3]);
        let same = RgbImage::from_fn(2, 2, |_, _| [0.5; 3]);
        let changed = RgbImage::from_fn(2, 2, |x, y| if x == 1 && y == 1 { [0.5001; 3] } else { [0.5; 3] });
        let reshaped = RgbImage::from_fn(4, 1, |_, _| [0.5; 3]);
        assert_eq!(image_key(&image), image_key(&same));
        assert_ne!(image_key(&image), image_key(&changed));
        assert_ne!(image_key(&image), image_key(&reshaped));
    }

    #[test]
    fn cancelled_work_does_not_enter_even_an_available_inference_queue() {
        let queue = Mutex::new(7);
        let context = JobCtx::new();
        context.cancel();
        assert!(matches!(lock(&queue, &context), Err(crate::EngineError::Cancelled)));
        assert_eq!(*queue.lock().unwrap(), 7);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn cancellation_after_failed_directml_initialization_never_starts_cpu_loading_v93() {
        let context = JobCtx::new();
        let mut requests = Vec::new();
        let result = load(Preference::Auto, &context, |execution| {
            requests.push(execution);
            if execution == Execution::DirectMl {
                context.cancel();
                Err("injected DML load failure".to_owned())
            } else {
                Ok(())
            }
        });
        assert!(matches!(result, Err(crate::EngineError::Cancelled)));
        assert_eq!(requests, [Execution::DirectMl], "cancellation stops a new CPU initialization after the completed failure");
    }

    #[test]
    fn cancelled_model_load_never_calls_any_factory_v93() {
        let context = JobCtx::new();
        context.cancel();
        let mut requested = false;
        let result = load(Preference::Cpu, &context, |_| {
            requested = true;
            Ok(())
        });
        assert!(matches!(result, Err(crate::EngineError::Cancelled)));
        assert!(!requested);
    }

    fn model_signature(length: u64) -> Signature {
        Signature {
            files: vec![FileSignature { path: PathBuf::from("v93-fake-model.onnx"), length, modified: Some(SystemTime::UNIX_EPOCH) }],
            execution: Preference::Auto,
        }
    }

    #[test]
    fn provider_quarantine_outlives_heavy_session_eviction_and_is_bounded_v93() {
        let signature = model_signature(1);
        // A failed set_image/predict records this same small signature outside
        // the heavy cache. Clearing that cache cannot erase recovery policy.
        let mut rejected = Some(signature.clone());
        let mut heavyweight = Some("fake object session");
        assert!(heavyweight.take().is_some());
        let mut requests = Vec::new();
        let (_, fallback) = load_object(&signature, &mut rejected, &JobCtx::new(), |execution| {
            requests.push(execution);
            Ok(())
        })
        .unwrap();
        assert_eq!(requests, [Execution::Cpu]);
        assert!(fallback.unwrap().contains("previously failed"));
        assert!(quarantined(&signature, &rejected));
        let changed = model_signature(2);
        assert!(!quarantined(&changed, &rejected), "changed model files are eligible for a fresh provider attempt");
        rejected = Some(changed.clone());
        assert!(quarantined(&changed, &rejected));
        assert!(!quarantined(&signature, &rejected), "the bounded slot does not collect every previously opened model");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn failed_provider_initialization_is_quarantined_even_if_cancelled_before_cpu_fallback_v93() {
        let signature = model_signature(1);
        let context = JobCtx::new();
        let mut rejected = None;
        let mut requests = Vec::new();
        let result = load_object(&signature, &mut rejected, &context, |execution| {
            requests.push(execution);
            context.cancel();
            Err::<(), _>("injected DML model initialization failure".to_owned())
        });
        assert!(matches!(result, Err(crate::EngineError::Cancelled)));
        assert_eq!(requests, [Execution::DirectMl]);
        assert!(quarantined(&signature, &rejected));
        requests.clear();
        load_object(&signature, &mut rejected, &JobCtx::new(), |execution| {
            requests.push(execution);
            Ok(())
        })
        .unwrap();
        assert_eq!(requests, [Execution::Cpu]);
    }
}
