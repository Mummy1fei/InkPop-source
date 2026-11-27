//! Shared, document-independent recognition for selections and Camera Raw drafts.
//!
//! Inputs are straight sRGB RGBA; only the bounded analysis copy is transformed or
//! composited. Classical fallback is explicit in diagnostics, never called a model.
use photocraft_algo::segment::{ImageSampler, RgbImage};
use photocraft_algo::{matting, segment};
use photocraft_geom::Rect;
use serde::Serialize;

use crate::{EngineError, Result, jobs::JobCtx};

pub const MAX_INPUT_PIXELS: usize = 100_000_000;
pub const REFERENCE_SIDE: usize = 1024;
const CLASSICAL_SIDE: usize = 400;

#[cfg(not(target_arch = "wasm32"))]
#[path = "image_recognition_onnx.rs"]
pub(crate) mod onnx;

/// Availability of the optional precise foreground assets, without loading them.
pub fn precise_subject_available() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        onnx::subject_available()
    }
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Subject,
    Sky,
    Background,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BackendPreference {
    #[default]
    Auto,
    Classical,
    U2Net,
    BiRefNet,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    pub backend: BackendPreference,
    pub sky: segment::sky::SkyParams,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostics {
    pub input_size: [usize; 2],
    pub analysis_size: [usize; 2],
    pub output_size: [usize; 2],
    pub model_cache_hit: bool,
    pub result_cache_hit: bool,
    pub fallback_reason: Option<String>,
    pub inference_provider: Option<&'static str>,
}

#[derive(Clone, Debug)]
pub struct Recognition {
    pub width: usize,
    pub height: usize,
    pub values: Vec<u8>,
    pub backend: &'static str,
    pub diagnostics: Diagnostics,
}

fn invalid(message: &str) -> EngineError {
    EngineError::Other(format!("Image recognition: {message}"))
}

fn checked_area(width: usize, height: usize) -> Result<usize> {
    width
        .checked_mul(height)
        .filter(|n| width > 0 && height > 0 && *n <= MAX_INPUT_PIXELS && width <= i32::MAX as usize && height <= i32::MAX as usize)
        .ok_or_else(|| invalid("dimensions must be nonzero and at most 100 megapixels"))
}

fn scaled_size(width: usize, height: usize, side: usize) -> [usize; 2] {
    let scale = (side as f64 / width.max(height).max(1) as f64).min(1.0);
    [((width as f64 * scale).round() as usize).max(1), ((height as f64 * scale).round() as usize).max(1)]
}

fn bin_start(bin: usize, source: usize, bins: usize) -> usize {
    (bin as u64 * source as u64 / bins.max(1) as u64) as usize
}

struct Reference {
    image: RgbImage,
    alpha: Vec<f32>,
}

// Box bins cover every input pixel once. Compose before averaging so hidden RGB
// never becomes an object or a sky seed; keep alpha separately for output bounds.
fn reference(rgba: &[[f32; 4]], width: usize, height: usize, ctx: &JobCtx) -> Result<Reference> {
    let n = checked_area(width, height)?;
    if rgba.len() != n {
        return Err(invalid("RGBA buffer does not match its dimensions"));
    }
    let [rw, rh] = scaled_size(width, height, REFERENCE_SIDE);
    let mut pixels = Vec::with_capacity(rw * rh);
    let mut alpha = Vec::with_capacity(rw * rh);
    for y in 0..rh {
        ctx.check()?;
        let (y0, y1) = (bin_start(y, height, rh), bin_start(y + 1, height, rh));
        for x in 0..rw {
            let (x0, x1) = (bin_start(x, width, rw), bin_start(x + 1, width, rw));
            let mut sum = [0.0f64; 4];
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = rgba.get(sy * width + sx).ok_or_else(|| invalid("invalid analysis bounds"))?;
                    if p.iter().any(|v| !v.is_finite()) {
                        return Err(invalid("RGBA samples must be finite"));
                    }
                    let a = p[3].clamp(0.0, 1.0) as f64;
                    for (sum, value) in sum.iter_mut().take(3).zip(p) {
                        *sum += value.clamp(0.0, 1.0) as f64 * a + 0.5 * (1.0 - a);
                    }
                    sum[3] += a;
                }
            }
            let count = ((y1 - y0) * (x1 - x0)).max(1) as f64;
            pixels.push([sum[0], sum[1], sum[2]].map(|v| (v / count) as f32));
            alpha.push((sum[3] / count) as f32);
        }
    }
    Ok(Reference { image: RgbImage { w: rw, h: rh, px: pixels }, alpha })
}

fn box_resize(image: &RgbImage, width: usize, height: usize) -> RgbImage {
    RgbImage::from_fn(width, height, |x, y| {
        let mut sum = [0.0f64; 3];
        let (x0, x1) = (bin_start(x, image.w, width), bin_start(x + 1, image.w, width));
        let (y0, y1) = (bin_start(y, image.h, height), bin_start(y + 1, image.h, height));
        for sy in y0..y1 {
            for sx in x0..x1 {
                if let Some(p) = image.px.get(sy * image.w + sx) {
                    for (s, v) in sum.iter_mut().zip(p) {
                        *s += *v as f64;
                    }
                }
            }
        }
        let n = ((x1 - x0) * (y1 - y0)).max(1) as f64;
        sum.map(|v| (v / n) as f32)
    })
}

fn bilinear(values: &[f32], width: usize, height: usize, x: f32, y: f32) -> f32 {
    let x = x.clamp(0.0, width.saturating_sub(1) as f32);
    let y = y.clamp(0.0, height.saturating_sub(1) as f32);
    let (ix, iy) = (x.floor() as usize, y.floor() as usize);
    let (fx, fy) = (x - ix as f32, y - iy as f32);
    let at = |x, y: usize| values.get(y * width + x).copied().unwrap_or(0.0);
    let right = (ix + 1).min(width.saturating_sub(1));
    let below = (iy + 1).min(height.saturating_sub(1));
    let top = at(ix, iy) * (1.0 - fx) + at(right, iy) * fx;
    let bottom = at(ix, below) * (1.0 - fx) + at(right, below) * fx;
    top * (1.0 - fy) + bottom * fy
}

fn resize_field(values: &[f32], width: usize, height: usize, ow: usize, oh: usize, ctx: &JobCtx) -> Result<Vec<f32>> {
    let mut output = Vec::with_capacity(ow * oh);
    for y in 0..oh {
        ctx.check()?;
        for x in 0..ow {
            output.push(bilinear(values, width, height, (x as f32 + 0.5) * width as f32 / ow as f32 - 0.5, (y as f32 + 0.5) * height as f32 / oh as f32 - 0.5));
        }
    }
    Ok(output)
}

fn refined(values: &[f32], image: &RgbImage, ctx: &JobCtx) -> Result<Vec<u8>> {
    ctx.check()?;
    let bounds = Rect::new(0, 0, image.w as i32, image.h as i32);
    let coarse = segment::Region { bbox: bounds, mask: values.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect() };
    let params = matting::RefineParams { radius: 3.0, smart_radius: true, smooth: 0.0, feather: 0.0, contrast: 0.0, shift_edge: 0.0 };
    let result = matting::refine_mask(&ImageSampler { img: image, origin: (0, 0) }, &matting::region_reader(&coarse), bounds, bounds, &params);
    ctx.check()?;
    Ok((0..image.w * image.h).map(|i| result.as_ref().map_or(0, |r| (r.at((i % image.w) as i32, (i / image.w) as i32) * 255.0).round() as u8)).collect())
}

/// A binary model with no pixel favouring foreground has found no region.
/// Never stretch low-confidence indoor/no-sky noise into a visible selection.
#[cfg(any(not(target_arch = "wasm32"), test))]
fn retain_confident_foreground(values: &mut [f32]) {
    if !values.iter().any(|value| *value > 0.5) {
        values.fill(0.0);
    }
}

fn classical(image: &RgbImage, kind: Kind, options: &Options, ctx: &JobCtx) -> Result<(Vec<u8>, [usize; 2])> {
    let [width, height] = scaled_size(image.w, image.h, CLASSICAL_SIDE);
    let small = box_resize(image, width, height);
    let sampler = ImageSampler { img: &small, origin: (0, 0) };
    let bounds = Rect::new(0, 0, width as i32, height as i32);
    ctx.progress(0.4, "Analyzing classical selection cues");
    let region = if kind == Kind::Sky { segment::sky::select_sky(&sampler, bounds, options.sky) } else { segment::subject::select_subject(&sampler, bounds) };
    ctx.check()?;
    let values = (0..width * height).map(|i| region.as_ref().map_or(0.0, |r| r.at((i % width) as i32, (i / width) as i32))).collect::<Vec<_>>();
    if width == image.w && height == image.h {
        // Preserve the established classical result when no enlargement was needed.
        return Ok((values.iter().map(|v| (v * 255.0).round() as u8).collect(), [width, height]));
    }
    let values = resize_field(&values, width, height, image.w, image.h, ctx)?;
    Ok((refined(&values, image, ctx)?, [width, height]))
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    struct Cache {
        path: PathBuf,
        signature: (u64, Option<std::time::SystemTime>),
        model: photocraft_ml::u2net::Segmenter,
        prediction: Option<(u128, Vec<u8>)>,
    }
    static SUBJECT_CACHE: OnceLock<Mutex<Option<Cache>>> = OnceLock::new();
    static SKY_CACHE: OnceLock<Mutex<Option<Cache>>> = OnceLock::new();

    fn model_path(kind: Kind) -> PathBuf {
        let (variable, filename) = if kind == Kind::Sky {
            ("PHOTOCRAFT_SKY_MODEL", "models/skyseg-u2netp.safetensors")
        } else {
            ("PHOTOCRAFT_SUBJECT_MODEL", "models/u2netp.safetensors")
        };
        std::env::var_os(variable)
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.join(filename))).unwrap_or_default())
    }

    pub(super) fn predict(image: &RgbImage, kind: Kind, ctx: &JobCtx) -> Result<(Vec<u8>, bool, bool)> {
        let sky = kind == Kind::Sky;
        let path = model_path(kind);
        let meta = std::fs::metadata(&path).map_err(|_| {
            invalid(if sky {
                "local SkySeg U2NETP model is missing (models/skyseg-u2netp.safetensors)"
            } else {
                "local U2NETP subject model is missing (models/u2netp.safetensors)"
            })
        })?;
        if meta.len() == 0 || meta.len() > 32 * 1024 * 1024 {
            return Err(invalid("recognition model must be nonempty and no larger than 32 MiB"));
        }
        let signature = (meta.len(), meta.modified().ok());
        let cache = if sky { &SKY_CACHE } else { &SUBJECT_CACHE };
        let cache = cache.get_or_init(|| Mutex::new(None));
        let mut cache = loop {
            match cache.try_lock() {
                Ok(guard) => break guard,
                Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    ctx.check()?;
                    std::thread::sleep(std::time::Duration::from_millis(4));
                }
            }
        };
        ctx.check()?;
        let model_hit = cache.as_ref().is_some_and(|c| c.path == path && c.signature == signature);
        if !model_hit {
            ctx.progress(0.2, "Loading local recognition model");
            use std::io::Read;
            let file = std::fs::File::open(&path).map_err(|e| invalid(&format!("could not read recognition model: {e}")))?;
            let mut weights = Vec::new();
            file.take(32 * 1024 * 1024 + 1).read_to_end(&mut weights).map_err(|e| invalid(&format!("could not read recognition model: {e}")))?;
            if weights.len() > 32 * 1024 * 1024 {
                return Err(invalid("recognition model exceeds 32 MiB"));
            }
            let model = if sky { photocraft_ml::u2net::Segmenter::new_sky(weights) } else { photocraft_ml::u2net::Segmenter::new(weights) }
                .map_err(|e| invalid(&format!("invalid recognition model: {e}")))?;
            *cache = Some(Cache { path, signature, model, prediction: None });
        }
        let model = cache.as_mut().ok_or_else(|| invalid("subject model unavailable"))?;
        let mut hash = blake3::Hasher::new();
        hash.update(&(image.w as u64).to_le_bytes());
        hash.update(&(image.h as u64).to_le_bytes());
        for p in &image.px {
            for value in p {
                hash.update(&value.to_bits().to_le_bytes());
            }
        }
        let mut key = [0u8; 16];
        key.copy_from_slice(&hash.finalize().as_bytes()[..16]);
        let key = u128::from_le_bytes(key);
        if let Some((_, values)) = model.prediction.as_ref().filter(|(k, _)| *k == key) {
            return Ok((values.clone(), model_hit, true));
        }
        // U2NET uses a square network input; resizing is intentionally not
        // aspect-padded (the public preprocessing warps the image to that square).
        let channels = (0..3).map(|c| image.px.iter().map(|p| p[c]).collect::<Vec<_>>()).collect::<Vec<_>>();
        let side = if sky { photocraft_ml::u2net::SKY_INPUT_SIDE } else { photocraft_ml::u2net::INPUT_SIDE };
        let mut rgb = Vec::with_capacity(side * side * 3);
        for y in 0..side {
            ctx.check()?;
            for x in 0..side {
                for channel in &channels {
                    rgb.push(bilinear(
                        channel,
                        image.w,
                        image.h,
                        (x as f32 + 0.5) * image.w as f32 / side as f32 - 0.5,
                        (y as f32 + 0.5) * image.h as f32 / side as f32 - 0.5,
                    ));
                }
            }
        }
        ctx.progress(0.35, if sky { "Recognizing sky with SkySeg U2NETP" } else { "Recognizing foreground with U2NETP" });
        let mut values =
            if sky { model.model.predict_sky(&rgb) } else { model.model.predict(&rgb) }.map_err(|e| invalid(&format!("recognition inference failed: {e}")))?;
        ctx.check()?;
        if values.len() != side * side || values.iter().any(|v| !v.is_finite()) {
            return Err(invalid("recognition model returned an invalid probability field"));
        }
        retain_confident_foreground(&mut values);
        let values = resize_field(&values, side, side, image.w, image.h, ctx)?;
        let values = refined(&values, image, ctx)?;
        ctx.check()?;
        model.prediction = Some((key, values.clone()));
        Ok((values, model_hit, false))
    }
}

/// Analyze an sRGB copy without touching documents, selections, history or presets.
pub fn recognize(rgba: &[[f32; 4]], width: usize, height: usize, kind: Kind, ctx: &JobCtx) -> Result<Recognition> {
    recognize_with_options(rgba, width, height, kind, Options::default(), ctx)
}

pub fn recognize_with_options(rgba: &[[f32; 4]], width: usize, height: usize, kind: Kind, options: Options, ctx: &JobCtx) -> Result<Recognition> {
    ctx.check()?;
    if kind == Kind::Sky && options.backend == BackendPreference::BiRefNet {
        return Err(invalid("BiRefNet selects foreground subjects; use the sky model for sky recognition"));
    }
    if !options.sky.threshold.is_finite() || !options.sky.softness.is_finite() {
        return Err(invalid("sky parameters must be finite"));
    }
    let classical_sky_options = kind == Kind::Sky && (options.sky.threshold != 50.0 || options.sky.softness != 50.0);
    if classical_sky_options && options.backend == BackendPreference::U2Net {
        return Err(invalid("custom sky threshold/softness require the classical backend"));
    }
    ctx.progress(0.05, "Preparing recognition image");
    let reference = reference(rgba, width, height, ctx)?;
    let image = &reference.image;
    let mut diagnostics = Diagnostics {
        input_size: [width, height],
        analysis_size: scaled_size(image.w, image.h, CLASSICAL_SIDE),
        output_size: [image.w, image.h],
        model_cache_hit: false,
        result_cache_hit: false,
        fallback_reason: None,
        inference_provider: None,
    };
    let mut model_values: Option<Vec<u8>> = None;
    let mut model_backend = None;
    if classical_sky_options && options.backend == BackendPreference::Auto {
        diagnostics.fallback_reason = Some("Custom sky threshold/softness use the classical backend to preserve their established meaning".into());
    }
    #[cfg(not(target_arch = "wasm32"))]
    if kind != Kind::Sky && options.backend == BackendPreference::BiRefNet {
        match onnx::subject(image, ctx) {
            Ok(prediction) => {
                diagnostics.analysis_size = [1024, 1024];
                diagnostics.model_cache_hit = prediction.model_hit;
                diagnostics.result_cache_hit = prediction.result_hit;
                diagnostics.inference_provider = Some(prediction.execution);
                diagnostics.fallback_reason = prediction.fallback;
                model_values = Some(prediction.values);
                model_backend = Some("BiRefNet");
            }
            Err(error) => {
                ctx.check()?;
                if options.backend == BackendPreference::BiRefNet {
                    return Err(error);
                }
                diagnostics.fallback_reason = Some(format!("BiRefNet unavailable; trying U2NETP: {error}"));
            }
        }
    }
    if !classical_sky_options && options.backend != BackendPreference::Classical && model_values.is_none() {
        #[cfg(not(target_arch = "wasm32"))]
        match native::predict(image, kind, ctx) {
            Ok((values, model_hit, result_hit)) => {
                let side = if kind == Kind::Sky { photocraft_ml::u2net::SKY_INPUT_SIDE } else { photocraft_ml::u2net::INPUT_SIDE };
                diagnostics.analysis_size = [side, side];
                diagnostics.model_cache_hit = model_hit;
                diagnostics.result_cache_hit = result_hit;
                model_values = Some(values);
                model_backend = Some(if kind == Kind::Sky { "SkySegU2NETP" } else { "U2NETP" });
                diagnostics.inference_provider = Some("Candle-CPU");
            }
            Err(error) => {
                ctx.check()?;
                if options.backend == BackendPreference::U2Net {
                    return Err(error);
                }
                diagnostics.fallback_reason = Some(match diagnostics.fallback_reason.take() {
                    Some(previous) => format!("{previous}; {error}"),
                    None => error.to_string(),
                });
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            if matches!(options.backend, BackendPreference::U2Net | BackendPreference::BiRefNet) {
                return Err(invalid("local recognition models require the desktop app"));
            }
            diagnostics.fallback_reason = Some("Local learned recognition is unavailable on the web; classical fallback used".into());
        }
    }
    let backend = if let Some(backend) = model_backend {
        backend
    } else if kind == Kind::Sky {
        "ClassicalSky"
    } else {
        "ClassicalSaliency"
    };
    let mut values = match model_values {
        Some(values) => values,
        None => {
            let (values, size) = classical(image, kind, &options, ctx)?;
            diagnostics.analysis_size = size;
            values
        }
    };
    for (value, alpha) in values.iter_mut().zip(&reference.alpha) {
        let probability = if kind == Kind::Background { 255 - *value } else { *value };
        *value = (probability as f32 * alpha).round() as u8;
    }
    ctx.check()?;
    ctx.progress(0.9, "Recognition mask ready");
    Ok(Recognition { width: image.w, height: image.h, values, backend, diagnostics })
}

impl Recognition {
    /// Reconstruct coverage at document resolution; coordinates remain relative to
    /// the supplied bounds, including documents whose canvas does not start at zero.
    pub fn region(&self, bounds: Rect, ctx: &JobCtx) -> Result<Option<segment::Region>> {
        let (width, height) = (bounds.width() as usize, bounds.height() as usize);
        checked_area(width, height)?;
        if self.width == 0 || self.height == 0 || self.values.len() != self.width.saturating_mul(self.height) {
            return Err(invalid("invalid recognition result dimensions"));
        }
        let field = self.values.iter().map(|v| *v as f32 / 255.0).collect::<Vec<_>>();
        let mut mask = Vec::with_capacity(width * height);
        for y in 0..height {
            ctx.check()?;
            for x in 0..width {
                let value = bilinear(
                    &field,
                    self.width,
                    self.height,
                    (x as f32 + 0.5) * self.width as f32 / width as f32 - 0.5,
                    (y as f32 + 0.5) * self.height as f32 / height as f32 - 0.5,
                );
                mask.push((value * 255.0).round() as u8);
            }
        }
        Ok(segment::trim_region(segment::Region { bbox: bounds, mask }))
    }
}

/// Read a document into a bounded straight sRGB analysis copy. Alpha-weighted
/// averaging happens before CMS; invisible stored RGB cannot contaminate the copy.
pub(crate) fn document_input(
    doc: &photocraft_doc::Document,
    layer: Option<photocraft_doc::LayerId>,
    all: bool,
    ctx: &JobCtx,
) -> Result<(Vec<[f32; 4]>, usize, usize)> {
    let bounds = doc.bounds();
    let (width, height) = (bounds.width() as usize, bounds.height() as usize);
    checked_area(width, height)?;
    let [ow, oh] = scaled_size(width, height, REFERENCE_SIDE);
    let profile = crate::color_cmds::composite_profile(doc);
    let transform = photocraft_cms::cached(&profile, photocraft_cms::Builtin::Srgb.profile(), Default::default()).map_err(|e| invalid(&e.to_string()))?;
    if !matches!(transform.inputs(), 1 | 3) || transform.outputs() != 3 {
        return Err(invalid("unsupported analysis color transform"));
    }
    let cmyk = photocraft_compose::cmyk_space(doc);
    crate::smartselect_cmds::with_doc_sampler(doc, layer, all, |sampler, _| {
        let mut accumulated = vec![[0.0f64; 5]; ow * oh];
        for y0 in (0..height).step_by(64) {
            ctx.check()?;
            let y1 = (y0 + 64).min(height);
            let columns = (262_144 / (y1 - y0).max(1)).max(1);
            for x0 in (0..width).step_by(columns) {
                ctx.check()?;
                let x1 = (x0 + columns).min(width);
                let stripe = photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
                    sampler.rgba(Rect::new(bounds.x0 + x0 as i32, bounds.y0 + y0 as i32, bounds.x0 + x1 as i32, bounds.y0 + y1 as i32))
                });
                if stripe.len() != (y1 - y0) * (x1 - x0) {
                    return Err(invalid("document sampler returned an invalid stripe"));
                }
                for (i, p) in stripe.iter().enumerate() {
                    if p.iter().any(|v| !v.is_finite()) {
                        return Err(invalid("document contains nonfinite analysis samples"));
                    }
                    let (x, y) = (x0 + i % (x1 - x0), y0 + i / (x1 - x0));
                    let (ox, oy) = ((x as u64 * ow as u64 / width as u64) as usize, (y as u64 * oh as u64 / height as u64) as usize);
                    let sum = accumulated.get_mut(oy * ow + ox).ok_or_else(|| invalid("invalid document proxy bounds"))?;
                    let alpha = p[3].clamp(0.0, 1.0) as f64;
                    for (s, v) in sum.iter_mut().take(3).zip(p) {
                        *s += *v as f64 * alpha;
                    }
                    sum[3] += alpha;
                    sum[4] += 1.0;
                }
            }
        }
        let mut rgba = Vec::with_capacity(ow * oh);
        for sum in accumulated {
            let alpha = sum[3] / sum[4].max(1.0);
            let native = [sum[0], sum[1], sum[2]].map(|v| if sum[3] > 0.0 { (v / sum[3]) as f32 } else { 0.0 });
            let mut rgb = [0.0; 3];
            transform.eval(&native[..transform.inputs()], &mut rgb);
            rgba.push([rgb[0], rgb[1], rgb[2], alpha as f32]);
        }
        Ok((rgba, ow, oh))
    })
}

/// Command boundary shared by menu subject/sky. Expensive work uses the ordinary
/// cancellable job protocol; only a completed result can append an undo entry.
pub(crate) fn selection(s: &mut crate::Session, p: &serde_json::Value, kind: Kind, cmd: &'static str, label: &'static str) -> Result<serde_json::Value> {
    use photocraft_algo::selection::{SelectionMode, combine_region};
    use serde_json::{Value, json};
    let bad = |message: &str| EngineError::BadParams { cmd: cmd.into(), msg: message.into() };
    let backend_name = match p.get("backend") {
        Some(value) => value.as_str().ok_or_else(|| bad("backend must be a string"))?,
        None => "auto",
    };
    let backend = match backend_name {
        "auto" if kind != Kind::Sky => s.prefs().performance.subject_recognition_quality.backend(),
        "auto" => BackendPreference::Auto,
        "classical" => BackendPreference::Classical,
        "u2net" | "u2netp" => BackendPreference::U2Net,
        "birefnet" => BackendPreference::BiRefNet,
        _ => return Err(bad("backend must be auto, classical, u2netp or birefnet")),
    };
    let mode = match p.get("mode") {
        Some(value) => value.as_str().ok_or_else(|| bad("mode must be a string"))?,
        None => "replace",
    };
    if !matches!(mode, "replace" | "new" | "add" | "subtract" | "intersect") {
        return Err(bad("mode must be replace, add, subtract or intersect"));
    }
    let mode = SelectionMode::parse(if mode == "new" { "replace" } else { mode });
    let scalar = |name: &str, default: f64| -> Result<f32> {
        let value = match p.get(name) {
            Some(value) => value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad("sky parameters must be finite numbers"))?,
            None => default,
        };
        Ok(value.clamp(0.0, 100.0) as f32)
    };
    let options = Options { backend, sky: segment::sky::SkyParams { threshold: scalar("threshold", 50.0)?, softness: scalar("softness", 50.0)? } };
    let all = p.get("sampleAllLayers").and_then(Value::as_bool).unwrap_or(true);
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let (doc, layer) = (st.doc.clone(), st.active_layer);
    crate::jobs::run(
        s,
        label,
        true,
        move |ctx| {
            let (rgba, width, height) = document_input(&doc, layer, all, ctx)?;
            let result = recognize_with_options(&rgba, width, height, kind, options, ctx)?;
            let region = result.region(doc.bounds(), ctx)?;
            ctx.check()?;
            Ok((result, region, doc.bounds()))
        },
        move |s, (result, region, canvas)| {
            let bbox = region.as_ref().map(|r| [r.bbox.x0, r.bbox.y0, r.bbox.width() as i32, r.bbox.height() as i32]);
            let total = region.as_ref().map_or(0.0, |r| r.mask.iter().map(|v| f64::from(*v) / 255.0).sum::<f64>());
            let coverage = total / (canvas.width() as f64 * canvas.height() as f64).max(1.0);
            if region.is_none() && (kind == Kind::Sky || matches!(mode, SelectionMode::Add | SelectionMode::Subtract)) {
                return Ok(json!({"selected":s.active().ok_or(EngineError::NoDocument)?.doc.selection.is_some(),
                    "changed":false,"coverage":0.0,"bounds":bbox,"backend":result.backend,"diagnostics":result.diagnostics,
                    "message":if kind == Kind::Sky {"No sky was detected"} else {"No subject was detected"}}));
            }
            let selected = s.edit(label, |doc, _| {
                doc.selection = combine_region(doc.selection.as_ref(), region.as_ref(), mode);
                Ok(doc.selection.is_some())
            })?;
            Ok(json!({"selected":selected,"changed":true,"coverage":coverage,"bounds":bbox,"backend":result.backend,"diagnostics":result.diagnostics}))
        },
    )
}

#[cfg(test)]
#[path = "image_recognition_tests_v66.rs"]
mod tests;
