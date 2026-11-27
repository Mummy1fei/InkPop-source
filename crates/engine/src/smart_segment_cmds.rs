//! Promptable local object masks; inference is a cancellable document job.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use photocraft_algo::selection::Region;
use photocraft_geom::Rect;
use serde_json::Value;
#[cfg(any(not(target_arch = "wasm32"), test))]
use serde_json::json;

/// A small guided-filter pass after MobileSAM. The model supplies the coarse object mask; the
/// image-guided band snaps that mask to local colour edges without blurring the interior.
const SMART_EDGE: photocraft_algo::matting::RefineParams =
    photocraft_algo::matting::RefineParams { radius: 3.0, smart_radius: true, smooth: 6.0, feather: 0.0, contrast: 18.0, shift_edge: 0.0 };

const CMD: &str = "select.smart";
fn bad(message: &str) -> EngineError {
    EngineError::BadParams { cmd: CMD.into(), msg: message.into() }
}
fn enabled(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn prompts(p: &Value, width: f64, height: f64) -> Result<Vec<(f64, f64, bool)>> {
    let mut out = Vec::new();
    for (key, positive) in [("points", true), ("negativePoints", false)] {
        if let Some(value) = p.get(key) {
            let points = value.as_array().ok_or_else(|| bad("points must be arrays of [x,y]"))?;
            if points.len() > 64 {
                return Err(bad("at most 64 prompt points"));
            }
            for point in points {
                let xy = point.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad("each point is [x,y]"))?;
                let x = xy.first().and_then(Value::as_f64).ok_or_else(|| bad("point x must be numeric"))?;
                let y = xy.get(1).and_then(Value::as_f64).ok_or_else(|| bad("point y must be numeric"))?;
                if !x.is_finite() || !y.is_finite() || x < 0. || y < 0. || x >= width || y >= height {
                    return Err(bad("prompt points must lie inside the image"));
                }
                out.push((x / width, y / height, positive));
            }
        }
    }
    if out.len() > 64 || (!out.iter().any(|p| p.2) && p.get("rect").is_none()) {
        return Err(bad("needs at least one foreground point, at most 64 total"));
    }
    Ok(out)
}

fn box_prompt(p: &Value, width: f64, height: f64) -> Result<Option<[f64; 4]>> {
    let Some(value) = p.get("rect") else { return Ok(None) };
    let array = value.as_array().filter(|v| v.len() == 4).ok_or_else(|| bad("rect must be [x,y,width,height]"))?;
    let mut r = [0.; 4];
    for (dst, v) in r.iter_mut().zip(array) {
        *dst = v.as_f64().filter(|v| v.is_finite()).ok_or_else(|| bad("rectangle coordinates must be finite numbers"))?;
    }
    let (x, y) = (r[0] + r[2], r[1] + r[3]);
    if !x.is_finite() || !y.is_finite() || width <= 0. || height <= 0. {
        return Err(bad("invalid rectangle"));
    }
    let result = [
        r[0].min(x).clamp(0., width) / width,
        r[1].min(y).clamp(0., height) / height,
        r[0].max(x).clamp(0., width) / width,
        r[1].max(y).clamp(0., height) / height,
    ];
    if result[0] >= result[2] || result[1] >= result[3] {
        return Err(bad("rectangle must intersect the image"));
    }
    Ok(Some(result))
}

/// A lasso is an allowed region, never a sequence of foreground clicks.
/// Point count and coordinate bounds also cap the scan-line rasterizer's work.
struct PolygonConstraint {
    points: Vec<(f32, f32)>,
    bounds: Rect,
}

fn polygon_constraint(p: &Value) -> Result<Option<PolygonConstraint>> {
    let Some(value) = p.get("polygon") else { return Ok(None) };
    let array = value.as_array().filter(|a| (3..=256).contains(&a.len())).ok_or_else(|| bad("polygon needs 3 to 256 [x,y] points"))?;
    let mut points = Vec::with_capacity(array.len());
    let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    for value in array {
        let xy = value.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad("each polygon point is [x,y]"))?;
        let number = |index| {
            xy.get(index)
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && v.abs() <= 10_000_000.0)
                .ok_or_else(|| bad("polygon coordinates must be finite and within ±10000000"))
        };
        let (x, y) = (number(0)?, number(1)?);
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        points.push((x as f32, y as f32));
    }
    let first = points.first().copied().ok_or_else(|| bad("polygon is empty"))?;
    let non_collinear = points.iter().find(|point| **point != first).is_some_and(|second| {
        points.iter().any(|third| {
            let cross =
                (second.0 as f64 - first.0 as f64) * (third.1 as f64 - first.1 as f64) - (second.1 as f64 - first.1 as f64) * (third.0 as f64 - first.0 as f64);
            cross.abs() > 1e-6
        })
    });
    if !non_collinear {
        return Err(bad("polygon must enclose an area"));
    }
    Ok(Some(PolygonConstraint { points, bounds: Rect::new(x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32) }))
}

/// Clip only the new, refined model result. Existing add/subtract selections
/// keep their own geometry. Small scan-line bands avoid a second full-size mask.
fn constrain_region(mut region: Region, polygon: &PolygonConstraint, canvas: Rect, ctx: &crate::jobs::JobCtx) -> Result<Region> {
    let bounds = region.bbox;
    let width = bounds.width() as usize;
    let height = bounds.height() as usize;
    let area = width
        .checked_mul(height)
        .filter(|n| *n > 0 && *n <= crate::image_recognition::MAX_INPUT_PIXELS)
        .ok_or_else(|| bad("invalid object mask dimensions"))?;
    if region.mask.len() != area {
        return Err(bad("invalid object mask coverage"));
    }
    let allowed = bounds.intersect(&canvas).intersect(&polygon.bounds);
    if allowed.is_empty() {
        return Err(bad("lasso does not intersect the selected object"));
    }
    for y in 0..height {
        if y % 32 == 0 {
            ctx.check()?;
        }
        let row = region.mask.get_mut(y * width..(y + 1) * width).ok_or_else(|| bad("invalid mask row"))?;
        let doc_y = bounds.y0 + y as i32;
        if doc_y < allowed.y0 || doc_y >= allowed.y1 {
            row.fill(0);
        } else {
            let left = (allowed.x0 - bounds.x0) as usize;
            let right = (allowed.x1 - bounds.x0) as usize;
            row.get_mut(..left).ok_or_else(|| bad("invalid lasso bounds"))?.fill(0);
            row.get_mut(right..).ok_or_else(|| bad("invalid lasso bounds"))?.fill(0);
        }
    }
    for y0 in (allowed.y0..allowed.y1).step_by(32) {
        let y1 = (y0 + 32).min(allowed.y1);
        let columns = (262_144 / (y1 - y0) as usize).max(1);
        for x0 in (allowed.x0..allowed.x1).step_by(columns) {
            ctx.check()?;
            let x1 = (x0 + columns as i32).min(allowed.x1);
            let stripe = Rect::new(x0, y0, x1, y1);
            let coverage = photocraft_algo::selection::polygon(&polygon.points, stripe, true);
            let stripe_width = stripe.width() as usize;
            for (index, value) in coverage.into_iter().enumerate() {
                let x = x0 + (index % stripe_width) as i32;
                let y = y0 + (index / stripe_width) as i32;
                let offset = (y - bounds.y0) as usize * width + (x - bounds.x0) as usize;
                let dest = region.mask.get_mut(offset).ok_or_else(|| bad("invalid lasso mask offset"))?;
                *dest = (f32::from(*dest) * value).round() as u8;
            }
        }
    }
    ctx.check()?;
    photocraft_algo::segment::trim_region(region).ok_or_else(|| bad("no object was found inside the lasso"))
}

#[cfg(not(target_arch = "wasm32"))]
fn model_path() -> std::path::PathBuf {
    std::env::var_os("PHOTOCRAFT_SMART_SELECTION_MODEL")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.join("models/mobile_sam-tiny-vitt.safetensors"))).unwrap_or_default())
}

pub fn available() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        model_path().is_file() || crate::image_recognition::onnx::object_available()
    }
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn infer_object(
    doc: &photocraft_doc::Document,
    active: Option<photocraft_doc::LayerId>,
    all: bool,
    points: &[(f64, f64, bool)],
    rect: Option<[f64; 4]>,
    backend: &str,
    ctx: &crate::jobs::JobCtx,
) -> Result<(photocraft_algo::segment::Region, &'static str, Value)> {
    let mut fallback = None;
    if backend == "sam2"
        || (backend == "auto" && std::env::var_os("PHOTOCRAFT_SMART_SELECTION_MODEL").is_none() && crate::image_recognition::onnx::object_available())
    {
        let (analysis, width, height) = crate::image_recognition::document_input(doc, active, all, ctx)?;
        let image = photocraft_algo::segment::RgbImage::from_fn(width, height, |x, y| {
            let pixel = analysis.get(y * width + x).copied().unwrap_or([0.0; 4]);
            let alpha = pixel[3].clamp(0.0, 1.0);
            [pixel[0], pixel[1], pixel[2]].map(|value| value.clamp(0.0, 1.0) * alpha + 0.5 * (1.0 - alpha))
        });
        match crate::image_recognition::onnx::object(&image, doc.bounds(), points, rect, ctx) {
            Ok(prediction) => {
                return Ok((
                    prediction.region,
                    "SAM2.1",
                    json!({"modelCacheHit":prediction.model_hit,"embeddingCacheHit":prediction.embedding_hit,
                    "inferenceProvider":prediction.execution,"fallbackReason":prediction.fallback}),
                ));
            }
            Err(error) => {
                ctx.check()?;
                if backend == "sam2" {
                    return Err(error);
                }
                fallback = Some(format!("SAM 2 unavailable; using MobileSAM: {error}"));
            }
        }
    }
    let region = native::infer(doc, active, all, points, rect, ctx)?;
    Ok((region, "MobileSAM", json!({"inferenceProvider":"Candle-CPU","fallbackReason":fallback})))
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use photocraft_algo::{segment::RgbImage, selection::Region};
    use photocraft_ml::mobile_sam::Segmenter;
    use std::sync::{Mutex, OnceLock};

    struct Cache {
        path: std::path::PathBuf,
        model: Segmenter,
    }
    static CACHE: OnceLock<Mutex<Option<Cache>>> = OnceLock::new();

    // Centre-aligned bilinear interpolation, including non-square padded masks.
    fn sample(values: &[f32], w: usize, h: usize, x: f32, y: f32) -> f32 {
        let x = x.clamp(0., (w - 1) as f32);
        let y = y.clamp(0., (h - 1) as f32);
        let (ix, iy) = (x.floor() as usize, y.floor() as usize);
        let (fx, fy) = (x - ix as f32, y - iy as f32);
        let at = |x: usize, y: usize| values.get(y * w + x).copied().unwrap_or(0.);
        let top = at(ix, iy) * (1. - fx) + at((ix + 1).min(w - 1), iy) * fx;
        let bottom = at(ix, (iy + 1).min(h - 1)) * (1. - fx) + at((ix + 1).min(w - 1), (iy + 1).min(h - 1)) * fx;
        top * (1. - fy) + bottom * fy
    }

    // SAM restores logits to the padded 1024 canvas first, then removes padding,
    // then resizes that cropped image. A direct 256-to-document sample skips an
    // interpolation step and reads padding at a non-multiple-of-four edge.
    fn crop_restored_logits(logits: &[f32], width: usize, height: usize) -> Vec<f32> {
        let mut cropped = Vec::with_capacity(width * height);
        for y in 0..height {
            for x in 0..width {
                cropped.push(sample(logits, 256, 256, (x as f32 + 0.5) / 4. - 0.5, (y as f32 + 0.5) / 4. - 0.5));
            }
        }
        cropped
    }

    fn resize(image: &RgbImage, w: usize, h: usize) -> Vec<f32> {
        let channels: Vec<Vec<f32>> = (0..3).map(|c| image.px.iter().map(|p| p[c]).collect()).collect();
        let mut out = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                for channel in &channels {
                    let v = sample(
                        channel,
                        image.w,
                        image.h,
                        (x as f32 + 0.5) * image.w as f32 / w as f32 - 0.5,
                        (y as f32 + 0.5) * image.h as f32 / h as f32 - 0.5,
                    );
                    out.push(if v.is_finite() { v.clamp(0., 1.) * 255. } else { 0. });
                }
            }
        }
        out
    }

    pub(super) fn infer(
        doc: &photocraft_doc::Document,
        layer: Option<photocraft_doc::LayerId>,
        all: bool,
        points: &[(f64, f64, bool)],
        rect: Option<[f64; 4]>,
        ctx: &crate::jobs::JobCtx,
    ) -> Result<Region> {
        let path = model_path();
        ctx.progress(0.05, "Loading local selection model");
        let mut cache = CACHE.get_or_init(|| Mutex::new(None)).lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        ctx.check()?;
        if cache.as_ref().is_none_or(|c| c.path != path) {
            let length = std::fs::metadata(&path)
                .map_err(|_| EngineError::Other("Smart selection model is missing. Install models/mobile_sam-tiny-vitt.safetensors beside PhotoCraft.".into()))?
                .len();
            if length == 0 || length > 64 * 1024 * 1024 {
                return Err(bad("invalid smart selection model size"));
            }
            let bytes = std::fs::read(&path).map_err(|e| EngineError::Other(e.to_string()))?;
            let model = Segmenter::new(bytes).map_err(EngineError::Other)?;
            *cache = Some(Cache { path, model });
        }
        let model = &mut cache.as_mut().ok_or_else(|| EngineError::Other("selection model unavailable".into()))?.model;
        let bounds = doc.bounds();
        let (w, h) = (bounds.width() as usize, bounds.height() as usize);
        if w == 0 || h == 0 || w.checked_mul(h).is_none_or(|n| n > 100_000_000) {
            return Err(bad("smart selection supports images up to 100 megapixels"));
        }
        ctx.progress(0.15, "Reading image");
        let (analysis, aw, ah) = crate::image_recognition::document_input(doc, layer, all, ctx)?;
        // Use the same alpha-weighted, colour-managed analysis input as the
        // automatic masks. MobileSAM still owns its prompt encoder/decoder.
        let image = RgbImage::from_fn(aw, ah, |x, y| {
            let pixel = analysis.get(y * aw + x).copied().unwrap_or([0.0; 4]);
            let alpha = pixel[3].clamp(0.0, 1.0);
            [
                pixel[0].clamp(0.0, 1.0) * alpha + 0.5 * (1.0 - alpha),
                pixel[1].clamp(0.0, 1.0) * alpha + 0.5 * (1.0 - alpha),
                pixel[2].clamp(0.0, 1.0) * alpha + 0.5 * (1.0 - alpha),
            ]
        });
        let scale = 1024. / w.max(h) as f64;
        let (rw, rh) = (((w as f64 * scale).round() as usize).max(1), ((h as f64 * scale).round() as usize).max(1));
        let rgb = resize(&image, rw, rh);
        let mut hash = blake3::Hasher::new();
        hash.update(&(rw as u64).to_le_bytes());
        hash.update(&(rh as u64).to_le_bytes());
        for v in &rgb {
            hash.update(&v.to_le_bytes());
        }
        let mut key = [0; 16];
        key.copy_from_slice(&hash.finalize().as_bytes()[..16]);
        let key = u128::from_le_bytes(key);
        ctx.check()?;
        if !model.has_image(key) {
            ctx.progress(0.25, "Recognizing image objects");
            model.set_image(key, rgb, rw, rh).map_err(EngineError::Other)?;
        }
        ctx.check()?;
        ctx.progress(0.8, "Tracing selected object");
        let logits = model.predict(points, rect).map_err(EngineError::Other)?;
        ctx.check()?;
        let cropped_logits = crop_restored_logits(&logits, rw, rh);
        let mut mask = Vec::with_capacity(w * h);
        for y in 0..h {
            if y % 128 == 0 {
                ctx.check()?;
            }
            for x in 0..w {
                let v = sample(&cropped_logits, rw, rh, (x as f32 + 0.5) * rw as f32 / w as f32 - 0.5, (y as f32 + 0.5) * rh as f32 / h as f32 - 0.5);
                mask.push(((v + 0.5).clamp(0., 1.) * 255.).round() as u8);
            }
        }
        photocraft_algo::segment::trim_region(Region { bbox: bounds, mask })
            .ok_or_else(|| EngineError::Other("No object found at that point; try a point farther inside the object".into()))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn sam_reconstruction_crops_padding_before_the_document_resize_v82() {
            let mut logits = vec![-8.0; 256 * 256];
            for y in 0..128 {
                logits[y * 256..(y + 1) * 256].fill(8.0);
            }
            // The short image ends in the transition towards padding. The final
            // document resize clamps to its final image row, not a padded row.
            let cropped = crop_restored_logits(&logits, 1024, 511);
            assert_eq!(cropped.len(), 1024 * 511);
            assert!(cropped.iter().all(|value| value.is_finite()));
            let final_row = cropped[510 * 1024 + 500];
            let restored = sample(&cropped, 1024, 511, 500.0, 510.4);
            let former = sample(&logits, 256, 256, (500.0 + 0.5) / 4. - 0.5, (510.4 + 0.5) / 4. - 0.5);
            assert_eq!(restored, final_row);
            assert!(former < restored, "padded rows must not bleed past the cropped edge");
            assert_eq!(sample(&cropped, 1024, 511, 0.0, 0.0), 8.0);
        }

        #[test]
        fn bilinear_mask_retains_edges_and_padding_coordinates() {
            let values = [0., 1., 0., 1.];
            assert_eq!(sample(&values, 2, 2, 0.5, 0.5), 0.5);
            assert_eq!(sample(&values, 2, 2, -1., 0.), 0.);
            assert_eq!(sample(&values, 2, 2, 8., 9.), 1.);
            let image = RgbImage::from_fn(2, 1, |x, _| [x as f32; 3]);
            assert_eq!(resize(&image, 4, 1), vec![0., 0., 0., 63.75, 63.75, 63.75, 191.25, 191.25, 191.25, 255., 255., 255.]);
        }
    }
}

fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let rect = box_prompt(p, doc.bounds().width() as f64, doc.bounds().height() as f64)?;
    let points = prompts(p, doc.bounds().width() as f64, doc.bounds().height() as f64)?;
    let polygon = polygon_constraint(p)?;
    let backend = p.get("backend").map(|value| value.as_str().ok_or_else(|| bad("backend must be a string"))).transpose()?.unwrap_or("auto");
    if !matches!(backend, "auto" | "mobilesam" | "sam2") {
        return Err(bad("backend must be auto, mobilesam or sam2"));
    }
    let backend = backend.to_owned();
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("replace");
    if !matches!(mode, "replace" | "add" | "subtract" | "intersect") {
        return Err(bad("invalid selection mode"));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mode = photocraft_algo::selection::SelectionMode::parse(mode);
        let all = p.get("sampleAllLayers").and_then(Value::as_bool).unwrap_or(true);
        crate::jobs::edit_job(
            s,
            "Smart Object Selection",
            move |doc, active, ctx| {
                let (region, backend, diagnostics) = infer_object(doc, *active, all, &points, rect, &backend, ctx)?;
                ctx.progress(0.9, "Snapping selection to image edges");
                // Refine only the narrow boundary band. If a pathological image leaves no
                // non-empty refined region, retain the model result rather than failing a valid
                // selection request.
                let refined = crate::smartselect_cmds::with_doc_sampler(doc, *active, all, |sampler, _| {
                    photocraft_algo::matting::refine_mask(sampler, &photocraft_algo::matting::region_reader(&region), region.bbox, doc.bounds(), &SMART_EDGE)
                });
                let region = refined.unwrap_or(region);
                let region = if let Some(polygon) = &polygon { constrain_region(region, polygon, doc.bounds(), ctx)? } else { region };
                ctx.check()?;
                doc.selection = photocraft_algo::selection::combine_region(doc.selection.as_ref(), Some(&region), mode);
                Ok(([region.bbox.x0, region.bbox.y0, region.bbox.width() as i32, region.bbox.height() as i32], doc.selection.is_some(), backend, diagnostics))
            },
            |(bounds, selected, backend, diagnostics)| json!({"selected":selected,"bounds":bounds,"backend":backend,"diagnostics":diagnostics}),
        )
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (points, rect, polygon, backend);
        Err(EngineError::Other("Local smart selection is currently available in the desktop app".into()))
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: CMD,
        label: "Smart Object Selection",
        menu: &[],
        shortcut: None,
        params: r#"{"points":[[x,y],...]?,"negativePoints":[[x,y],...]?,"rect":[x,y,w,h]?,"polygon":[[x,y],...]?(3..256 region constraint, not foreground points),"mode":"replace|add|subtract|intersect"="replace","sampleAllLayers":bool=true,"backend":"auto|mobilesam|sam2"="auto"} → local promptable object selection; response includes backend/diagnostics"#,
        enabled,
        run,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_smart_selection_never_changes_document_at_any_depth() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":80,"height":60,"depth":depth})).unwrap();
            s.execute("select.rect", json!({"x":10,"y":10,"width":20,"height":20})).unwrap();
            let before = s.active().unwrap().doc.clone();
            for p in [json!({}), json!({"points":[[80,10]]}), json!({"rect":[0,0,0,10]}), json!({"points":[[10,10]],"mode":"bogus"})] {
                assert!(s.execute(CMD, p).is_err());
                assert_eq!(*before, *s.active().unwrap().doc);
            }
        }
    }
    #[test]
    fn boxes_normalize_reverse_drag_and_reject_empty_or_hostile_input() {
        assert_eq!(box_prompt(&json!({"rect":[80,90,-60,-50]}), 100., 100.).unwrap(), Some([0.2, 0.4, 0.8, 0.9]));
        for rect in [json!([0, 0, 0, 20]), json!([150, 0, 10, 10]), json!([1, 2]), json!([0, 0, "x", 20]), json!([1e308, 0, 1e308, 20])] {
            assert!(box_prompt(&json!({"rect":rect}), 100., 100.).is_err());
        }
    }
    #[test]
    fn rejects_bad_prompts_without_inference() {
        for p in [
            json!({}),
            json!({"points":[[1]]}),
            json!({"points":[[-1,2]]}),
            json!({"points":[[100,2]]}),
            json!({"points":[[1,"a"]]}),
            json!({"negativePoints":[[1,2]]}),
        ] {
            assert!(prompts(&p, 100., 100.).is_err());
        }
        assert_eq!(prompts(&json!({"points":[[20,30]],"negativePoints":[[1,2]]}), 100., 100.).unwrap(), vec![(0.2, 0.3, true), (0.01, 0.02, false)]);
    }

    #[test]
    fn lasso_constraint_rejects_hostile_and_degenerate_geometry_without_inference_v93() {
        for polygon in [
            json!([]),
            json!([[0, 0], [1, 1]]),
            json!([[0, 0], [1, 1], [2, 2]]),
            json!([[0, 0], [0, 0], [0, 0]]),
            json!([[0, 0], [2, 0], ["x", 2]]),
            json!([[0, 0], [2, 0], [0, 1e100]]),
            json!([[0], [2, 0], [0, 2]]),
            json!(vec![[0, 0]; 257]),
        ] {
            assert!(polygon_constraint(&json!({"polygon":polygon})).is_err());
        }
        // A self-crossing lasso still has an even-odd filled area; a zero signed
        // shoelace sum alone must not reject it.
        assert!(polygon_constraint(&json!({"polygon":[[0,0],[10,10],[0,10],[10,0]]})).unwrap().is_some());
        assert!(polygon_constraint(&json!({})).unwrap().is_none());
    }

    #[test]
    fn lasso_constraint_clips_refined_result_and_keeps_antialiased_edges_v93() {
        let canvas = Rect::new(0, 0, 20, 20);
        let full = Region { bbox: canvas, mask: vec![255; 400] };
        let polygon = polygon_constraint(&json!({"polygon":[[2,2],[18,2],[2,18]]})).unwrap().unwrap();
        let result = constrain_region(full, &polygon, canvas, &crate::jobs::JobCtx::new()).unwrap();
        let surface = photocraft_algo::matting::region_surface(&result);
        assert_eq!(surface.read_region(Rect::new(4, 4, 5, 5)), vec![1.0]);
        assert_eq!(surface.read_region(Rect::new(17, 17, 18, 18)), vec![0.0]);
        assert_eq!(surface.read_region(Rect::new(0, 0, 1, 1)), vec![0.0]);
        let values = surface.read_region(canvas);
        assert!(values.iter().any(|v| *v > 0.0 && *v < 1.0), "diagonal lasso edge remains antialiased");
        assert!(result.bbox.x0 >= 2 && result.bbox.y0 >= 2 && result.bbox.x1 <= 18 && result.bbox.y1 <= 18);
    }

    #[test]
    fn lasso_clips_negative_origin_masks_and_does_not_constrain_prior_selection_v93() {
        let canvas = Rect::new(0, 0, 12, 12);
        let polygon = polygon_constraint(&json!({"polygon":[[-5,-5],[6,-5],[6,6],[-5,6]]})).unwrap().unwrap();
        let region = Region { bbox: Rect::new(-3, -3, 15, 15), mask: vec![255; 18 * 18] };
        let result = constrain_region(region, &polygon, canvas, &crate::jobs::JobCtx::new()).unwrap();
        assert_eq!(result.bbox, Rect::new(0, 0, 6, 6));
        let previous = photocraft_algo::selection::mask_to_surface(&[1.0; 4], Rect::new(10, 10, 12, 12));
        let combined = photocraft_algo::selection::combine_region(Some(&previous), Some(&result), photocraft_algo::selection::SelectionMode::Add).unwrap();
        assert_eq!(combined.read_region(Rect::new(10, 10, 12, 12)), vec![1.0; 4], "add preserves earlier selection outside this lasso");
        assert_eq!(combined.read_region(Rect::new(0, 0, 1, 1)), vec![1.0]);
        let subtracted =
            photocraft_algo::selection::combine_region(Some(&combined), Some(&result), photocraft_algo::selection::SelectionMode::Subtract).unwrap();
        assert_eq!(subtracted.read_region(Rect::new(10, 10, 12, 12)), vec![1.0; 4]);
        assert_eq!(subtracted.read_region(Rect::new(0, 0, 1, 1)), vec![0.0]);
    }

    #[test]
    fn lasso_constraint_cancels_rejects_empty_and_checks_mask_dimensions_v93() {
        let canvas = Rect::new(0, 0, 10, 10);
        let polygon = polygon_constraint(&json!({"polygon":[[0,0],[10,0],[10,10],[0,10]]})).unwrap().unwrap();
        let ctx = crate::jobs::JobCtx::new();
        ctx.cancel();
        assert!(matches!(constrain_region(Region { bbox: canvas, mask: vec![255; 100] }, &polygon, canvas, &ctx), Err(EngineError::Cancelled)));
        assert!(constrain_region(Region { bbox: canvas, mask: vec![0; 100] }, &polygon, canvas, &crate::jobs::JobCtx::new()).is_err());
        assert!(constrain_region(Region { bbox: canvas, mask: vec![255; 99] }, &polygon, canvas, &crate::jobs::JobCtx::new()).is_err());
        let outside = polygon_constraint(&json!({"polygon":[[30,30],[40,30],[40,40],[30,40]]})).unwrap().unwrap();
        assert!(constrain_region(Region { bbox: canvas, mask: vec![255; 100] }, &outside, canvas, &crate::jobs::JobCtx::new()).is_err());
    }

    #[test]
    fn invalid_lasso_preserves_document_selection_and_history_v93() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":40,"height":30})).unwrap();
        s.execute("select.rect", json!({"x":1,"y":2,"width":5,"height":5})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        assert!(s.execute(CMD, json!({"rect":[0,0,20,20],"polygon":[[0,0],[1,1],[2,2]]})).is_err());
        assert_eq!(*before, *s.active().unwrap().doc);
        assert_eq!(history, s.active().unwrap().history.past_len());
    }
}
