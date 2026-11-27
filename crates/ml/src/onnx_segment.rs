//! Bounded local ONNX segmentation adapters. No downloads, document state or UI.
//!
//! The caller selects a vetted graph contract and checks cancellation before and
//! after inference. ORT's Rust wrapper can panic during dynamic-library loading;
//! those panics are contained here. Native access violations are not catchable.
use ort::execution_providers::{CPUExecutionProvider, DirectMLExecutionProvider};
use ort::session::{Session, builder::GraphOptimizationLevel};
use ort::tensor::TensorElementType;
use ort::value::{Tensor, ValueType};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const MAX_SIDE: usize = 1536;
const MAX_RGB_PIXELS: usize = 1024 * 1024;
const MAX_MODEL_BYTES: u64 = 1536 * 1024 * 1024;
const MAX_RUNTIME_BYTES: u64 = 256 * 1024 * 1024;
const MAX_TENSOR_VALUES: usize = 64 * 1024 * 1024;
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// A requested provider. DirectML registration does not imply every graph node
/// runs on the GPU; unsupported operators may execute in ORT's CPU provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Execution {
    Cpu,
    DirectMl,
}

impl Execution {
    pub fn name(self) -> &'static str {
        match self {
            Self::Cpu => "ONNX-CPU",
            Self::DirectMl => "ONNX-DirectML",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum OutputKind {
    Logits,
    Probabilities,
}

#[derive(Debug)]
pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub probabilities: Vec<f32>,
}

pub(crate) fn guarded<T>(run: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).map_err(|_| "Local ONNX inference failed safely".to_owned())?
}

fn checked_file(path: &Path, max_bytes: u64) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Local model/runtime paths must be absolute".into());
    }
    let path = path.canonicalize().map_err(|_| "Local model/runtime file is missing".to_owned())?;
    let meta = std::fs::metadata(&path).map_err(|error| error.to_string())?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > max_bytes {
        return Err("Local model/runtime file exceeds its size limit".into());
    }
    Ok(path)
}

fn runtime(path: &Path) -> Result<(), String> {
    static RUNTIME: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    let requested = checked_file(path, MAX_RUNTIME_BYTES)?;
    let initialized = RUNTIME.get_or_init(|| {
        guarded(|| {
            let text = requested.to_str().ok_or("Runtime path must be valid Unicode")?;
            let committed = ort::init_from(text).with_name("InkPop-local-segmentation").with_telemetry(false).commit().map_err(|error| error.to_string())?;
            if !committed {
                return Err("ONNX Runtime was initialized outside the local model service".into());
            }
            Ok(requested.clone())
        })
    });
    match initialized {
        Ok(loaded) if *loaded == requested => Ok(()),
        Ok(_) => Err("A different local ONNX Runtime is already initialized; restart to switch it".into()),
        Err(error) => Err(error.clone()),
    }
}

pub(crate) fn load_session(runtime_path: &Path, model_path: &Path, execution: Execution) -> Result<Session, String> {
    session_with_memory_policy(runtime_path, model_path, execution, false)
}

fn session_with_memory_policy(runtime_path: &Path, model_path: &Path, execution: Execution, bounded_cpu: bool) -> Result<Session, String> {
    runtime(runtime_path)?;
    let model = checked_file(model_path, MAX_MODEL_BYTES)?;
    guarded(|| {
        let threads = std::thread::available_parallelism().map_or(1, |count| count.get().min(4));
        let mut builder = Session::builder()
            .and_then(|builder| builder.with_optimization_level(GraphOptimizationLevel::Level3))
            .and_then(|builder| builder.with_intra_threads(threads))
            .and_then(|builder| builder.with_inter_threads(1))
            .and_then(|builder| builder.with_parallel_execution(false))
            .map_err(|error| error.to_string())?;
        if execution == Execution::DirectMl {
            // DML requires sequential execution and memory-pattern optimizations
            // disabled. Registration failures must not masquerade as GPU use.
            builder = builder
                .with_memory_pattern(false)
                .and_then(|builder| builder.with_execution_providers([DirectMLExecutionProvider::default().build().error_on_failure()]))
                .map_err(|error| error.to_string())?;
        } else if bounded_cpu {
            // Foreground inference has large transient feature maps. Do not
            // retain an arena sized to the worst run in its idle model cache.
            // This limits allocator retention, not the application's total RAM.
            builder = builder
                .with_memory_pattern(false)
                .and_then(|builder| builder.with_execution_providers([CPUExecutionProvider::default().with_arena_allocator(false).build().error_on_failure()]))
                .map_err(|error| error.to_string())?;
        }
        builder.commit_from_file(model).map_err(|error| error.to_string())
    })
}

pub(crate) fn shape_matches(value: &ValueType, expected: &[i64], element: TensorElementType) -> bool {
    let ValueType::Tensor { ty, shape, .. } = value else { return false };
    *ty == element
        && shape.len() == expected.len()
        && shape.iter().zip(expected).all(|(actual, expected)| *actual == -1 || *expected == -1 || *actual == *expected)
}

pub(crate) fn named_input(session: &Session, name: &str, expected: &[i64], element: TensorElementType) -> Result<(), String> {
    if name.is_empty() || !session.inputs.iter().any(|input| input.name == name && shape_matches(&input.input_type, expected, element)) {
        return Err("Local model input signature does not match the vetted contract".into());
    }
    Ok(())
}

pub(crate) fn named_output(session: &Session, name: &str, expected: &[i64], element: TensorElementType) -> Result<(), String> {
    if name.is_empty() || !session.outputs.iter().any(|output| output.name == name && shape_matches(&output.output_type, expected, element)) {
        return Err("Local model output signature does not match the vetted contract".into());
    }
    Ok(())
}

pub(crate) fn tensor_count(shape: &[usize]) -> Result<usize, String> {
    if shape.is_empty() || shape.len() > 6 {
        return Err("Invalid local model tensor rank".into());
    }
    shape
        .iter()
        .try_fold(1usize, |count, dimension| if *dimension == 0 { None } else { count.checked_mul(*dimension) })
        .filter(|count| *count <= MAX_TENSOR_VALUES)
        .ok_or_else(|| "Local model tensor exceeds its value budget".into())
}

/// Centre-aligned bilinear resize and ImageNet normalization. The shared engine
/// provides an already antialiased reference no larger than 1024 on either side;
/// this stage expands it to the model square and never downsamples it again.
pub fn prepare_rgb(rgb: &[[f32; 3]], width: usize, height: usize, side: usize) -> Result<Vec<f32>, String> {
    let pixels = width.checked_mul(height).filter(|count| *count > 0 && *count <= MAX_RGB_PIXELS).ok_or("Invalid analysis image size")?;
    if rgb.len() != pixels || side == 0 || side > MAX_SIDE || width > side || height > side {
        return Err("Local model requires a bounded analysis reference no larger than its input".into());
    }
    if rgb.iter().flatten().any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)) {
        return Err("Analysis RGB samples must be finite sRGB values in 0..1".into());
    }
    let count = tensor_count(&[1, 3, side, side])?;
    let plane = side * side;
    let mut output = vec![0.0; count];
    for y in 0..side {
        let sy = ((y as f64 + 0.5) * height as f64 / side as f64 - 0.5).clamp(0.0, (height - 1) as f64);
        let y0 = sy.floor() as usize;
        let y1 = (y0 + 1).min(height - 1);
        let fy = (sy - y0 as f64) as f32;
        for x in 0..side {
            let sx = ((x as f64 + 0.5) * width as f64 / side as f64 - 0.5).clamp(0.0, (width - 1) as f64);
            let x0 = sx.floor() as usize;
            let x1 = (x0 + 1).min(width - 1);
            let fx = (sx - x0 as f64) as f32;
            let a = rgb.get(y0 * width + x0).ok_or("Invalid analysis image offset")?;
            let b = rgb.get(y0 * width + x1).ok_or("Invalid analysis image offset")?;
            let c = rgb.get(y1 * width + x0).ok_or("Invalid analysis image offset")?;
            let d = rgb.get(y1 * width + x1).ok_or("Invalid analysis image offset")?;
            for (channel, (((a, b), c), d)) in a.iter().zip(b).zip(c).zip(d).enumerate() {
                let mean = MEAN.get(channel).copied().ok_or("Invalid RGB channel")?;
                let std = STD.get(channel).copied().ok_or("Invalid RGB channel")?;
                let top = *a * (1.0 - fx) + *b * fx;
                let bottom = *c * (1.0 - fx) + *d * fx;
                let target = output.get_mut(channel * plane + y * side + x).ok_or("Invalid model input offset")?;
                *target = (top * (1.0 - fy) + bottom * fy - mean) / std;
            }
        }
    }
    Ok(output)
}

pub(crate) fn probabilities(values: &[f32], kind: OutputKind) -> Result<Vec<f32>, String> {
    if values.is_empty() || values.len() > MAX_TENSOR_VALUES || values.iter().any(|value| !value.is_finite()) {
        return Err("Local model returned an invalid mask".into());
    }
    if matches!(kind, OutputKind::Probabilities) && values.iter().any(|value| !(0.0..=1.0).contains(value)) {
        return Err("Local model probability output is outside 0..1".into());
    }
    Ok(values
        .iter()
        .map(|value| match kind {
            OutputKind::Probabilities => *value,
            OutputKind::Logits if *value >= 0.0 => 1.0 / (1.0 + (-*value).exp()),
            OutputKind::Logits => {
                let e = value.exp();
                e / (1.0 + e)
            }
        })
        .collect())
}

/// BiRefNet-family adapter. Names and output activation are supplied from the
/// recorded export contract; output order or value range never guesses them.
pub struct ForegroundModel {
    session: Session,
    input_name: String,
    output_name: String,
    side: usize,
    kind: OutputKind,
    execution: Execution,
}

impl ForegroundModel {
    pub fn load(
        runtime: &Path,
        model: &Path,
        input_name: &str,
        output_name: &str,
        side: usize,
        kind: OutputKind,
        execution: Execution,
    ) -> Result<Self, String> {
        if side == 0 || side > MAX_SIDE {
            return Err("Invalid foreground model side".into());
        }
        let session = session_with_memory_policy(runtime, model, execution, true)?;
        if session.inputs.len() != 1 || session.outputs.is_empty() || session.outputs.len() > 16 {
            return Err("Unsupported foreground model input/output count".into());
        }
        let side_i64 = side as i64;
        named_input(&session, input_name, &[1, 3, side_i64, side_i64], TensorElementType::Float32)?;
        named_output(&session, output_name, &[1, 1, side_i64, side_i64], TensorElementType::Float32)?;
        Ok(Self { session, input_name: input_name.into(), output_name: output_name.into(), side, kind, execution })
    }

    pub fn execution(&self) -> Execution {
        self.execution
    }

    pub fn predict_rgb(&mut self, rgb: &[[f32; 3]], width: usize, height: usize) -> Result<Mask, String> {
        let values = prepare_rgb(rgb, width, height, self.side)?;
        guarded(|| {
            let input = Tensor::from_array(([1usize, 3, self.side, self.side], values.into_boxed_slice())).map_err(|error| error.to_string())?;
            let outputs = self.session.run(ort::inputs![self.input_name.as_str() => input]).map_err(|error| error.to_string())?;
            let value = outputs.get(self.output_name.as_str()).ok_or("Foreground model output is missing")?;
            let (shape, values) = value.try_extract_tensor::<f32>().map_err(|error| error.to_string())?;
            if shape.as_ref() != [1, 1, self.side as i64, self.side as i64] || values.len() != self.side * self.side {
                return Err("Foreground model returned the wrong mask shape".into());
            }
            Ok(Mask { width: self.side, height: self.side, probabilities: probabilities(values, self.kind)? })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preparation_keeps_rgb_planes_and_centred_interpolation() {
        let values = prepare_rgb(&[[0.0, 0.5, 1.0], [1.0, 0.5, 0.0]], 2, 1, 4).unwrap();
        assert_eq!(values.len(), 48);
        assert!((values[0] - -0.485 / 0.229).abs() < 1e-5);
        assert!((values[1] - (0.25 - 0.485) / 0.229).abs() < 1e-5);
        assert!((values[2] - (0.75 - 0.485) / 0.229).abs() < 1e-5);
        assert!((values[16] - (0.5 - 0.456) / 0.224).abs() < 1e-5);
        assert!((values[32] - (1.0 - 0.406) / 0.225).abs() < 1e-5);
        assert_eq!(&values[..4], &values[12..16]);
    }

    #[test]
    fn preparation_rejects_nonfinite_oversized_or_mismatched_input_before_allocating() {
        assert!(prepare_rgb(&[[f32::NAN, 0.0, 0.0]], 1, 1, 1).is_err());
        assert!(prepare_rgb(&[[1.1, 0.0, 0.0]], 1, 1, 1).is_err());
        assert!(prepare_rgb(&[], usize::MAX, 2, 1024).is_err());
        assert!(prepare_rgb(&[[0.0; 3]], 1, 1, usize::MAX).is_err());
        assert!(prepare_rgb(&[[0.0; 3]], 2, 1, 2).is_err());
        assert!(prepare_rgb(&[[0.0; 3]; 4], 4, 1, 2).is_err());
    }

    #[test]
    fn output_activation_is_explicit_and_stable_without_imagewise_normalization() {
        assert_eq!(probabilities(&[-1000.0, 0.0, 1000.0], OutputKind::Logits).unwrap(), [0.0, 0.5, 1.0]);
        assert_eq!(probabilities(&[0.1, 0.3], OutputKind::Probabilities).unwrap(), [0.1, 0.3]);
        assert!(probabilities(&[-0.1], OutputKind::Probabilities).is_err());
        assert!(probabilities(&[f32::NAN], OutputKind::Logits).is_err());
        assert!(probabilities(&[f32::INFINITY], OutputKind::Logits).is_err());
    }

    #[test]
    fn missing_runtime_returns_error_without_initializing_ort() {
        assert!(runtime(Path::new("relative.dll")).is_err());
        let missing = std::env::temp_dir().join("inkpop-no-such-runtime-v93.dll");
        if !missing.exists() {
            assert!(runtime(&missing).is_err());
        }
    }

    #[test]
    fn tensor_shapes_are_checked_without_overflow() {
        assert_eq!(tensor_count(&[1, 3, 1024, 1024]).unwrap(), 3 * 1024 * 1024);
        assert!(tensor_count(&[0, 1024]).is_err());
        assert!(tensor_count(&[usize::MAX, 2]).is_err());
        assert!(tensor_count(&[1; 7]).is_err());
        assert!(tensor_count(&[65 * 1024 * 1024]).is_err());
    }
}
