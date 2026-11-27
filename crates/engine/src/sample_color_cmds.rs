//! Bounded, document-space color sampling shared by Eyedropper and Mixer Brush loading.
use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use photocraft_geom::Rect;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{EngineError, Result, Session, commands::CommandSpec};

pub const COMMAND: &str = "document.sampleColor";
pub const SIZES: [u32; 7] = [1, 3, 5, 11, 31, 51, 101];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Source {
    Current,
    CurrentBelow,
    All,
    AllNoAdjustments,
    CurrentBelowNoAdjustments,
}

fn bad(message: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: COMMAND.into(), msg: message.into() }
}

fn validate_tree(doc: &Document) -> Result<()> {
    if doc.max_group_depth() > photocraft_doc::MAX_GROUP_DEPTH {
        return Err(bad("sample document exceeds the supported layer nesting depth"));
    }
    Ok(())
}

pub(crate) fn size(value: &Value) -> Result<u32> {
    value.as_u64().and_then(|n| u32::try_from(n).ok()).filter(|n| SIZES.contains(n)).ok_or_else(|| bad("sample size must be 1, 3, 5, 11, 31, 51 or 101"))
}

pub(crate) fn source(value: &Value) -> Result<Source> {
    serde_json::from_value(value.clone()).map_err(|_| bad("source must be current, currentBelow, all, allNoAdjustments or currentBelowNoAdjustments"))
}

/// Keep the target and lower siblings at every ancestor level. A selected group
/// includes its children; flattening the walk and hiding everything after the
/// group would incorrectly erase those children from its own sample.
fn keep_through(layers: &mut Vec<Layer>, path: &[usize]) -> Result<()> {
    let (index, rest) = path.split_first().ok_or_else(|| bad("no current layer"))?;
    if *index >= layers.len() {
        return Err(bad("invalid current layer path"));
    }
    layers.truncate(*index + 1);
    if !rest.is_empty() {
        let children = layers.get_mut(*index).and_then(Layer::children_mut).ok_or_else(|| bad("invalid current layer parent"))?;
        keep_through(children, rest)?;
    }
    Ok(())
}

fn hide_adjustments(layers: &mut [Layer]) {
    for layer in layers {
        if matches!(layer.content, LayerContent::Adjustment(_)) {
            layer.visible = false;
        }
        if let Some(children) = layer.children_mut() {
            hide_adjustments(children);
        }
    }
}

/// Current-layer pixel data can be sampled directly without materializing a
/// brush-sized region; group/fill content uses the compositor fallback.
pub(crate) fn raw_surface(doc: &Document, layer: Option<LayerId>, source: Source) -> Result<Option<&photocraft_raster::Surface>> {
    validate_tree(doc)?;
    if source == Source::Current {
        let layer = layer.and_then(|id| doc.layer(id)).ok_or_else(|| bad("no current layer"))?;
        Ok(layer.surface())
    } else {
        Ok(None)
    }
}

/// Read straight RGBA values in the same document/compositor encoding used by
/// document.pixel. Native sample depth and HDR values are retained.
pub(crate) fn read(doc: &Document, layer: Option<LayerId>, source: Source, area: Rect) -> Result<Vec<[f32; 4]>> {
    validate_tree(doc)?;
    let count = u64::from(area.width())
        .checked_mul(u64::from(area.height()))
        .filter(|n| *n <= 16_000_000)
        .ok_or_else(|| bad("sample region exceeds 16 million pixels"))? as usize;
    if area.is_empty() {
        return Ok(Vec::new());
    }
    let current = || layer.and_then(|id| doc.layer(id)).ok_or_else(|| bad("no current layer"));
    if source == Source::Current {
        let selected = current()?;
        if let Some(surface) = raw_surface(doc, layer, source)? {
            let mut pixels = vec![[0.0; 4]; count];
            let cmyk = photocraft_compose::cmyk_space(doc);
            photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || surface.read_rgba_into(area, &mut pixels));
            return Ok(pixels);
        }
        // Fill/adjustment/group sources need their actual document color context,
        // not render_layer's thumbnail context with an assumed sRGB transfer.
        let mut isolated = doc.clone();
        let mut selected = selected.clone();
        selected.visible = true;
        isolated.layers = vec![selected];
        return Ok(photocraft_compose::render(&isolated, area).px);
    }
    if source == Source::All {
        return Ok(photocraft_compose::render(doc, area).px);
    }
    let mut projected = doc.clone();
    if matches!(source, Source::CurrentBelow | Source::CurrentBelowNoAdjustments) {
        let id = current()?.id;
        let path = doc.path_of(id).ok_or(EngineError::NoLayer(id))?;
        keep_through(&mut projected.layers, &path)?;
    }
    if matches!(source, Source::AllNoAdjustments | Source::CurrentBelowNoAdjustments) {
        hide_adjustments(&mut projected.layers);
    }
    Ok(photocraft_compose::render(&projected, area).px)
}

/// Premultiplied averaging prevents invisible RGB from contaminating translucent
/// samples; alpha is averaged over the actual in-canvas sample area.
pub(crate) fn average(pixels: &[[f32; 4]]) -> [f32; 4] {
    let mut total = [0.0f64; 4];
    for pixel in pixels {
        if pixel.iter().any(|value| !value.is_finite()) {
            continue;
        }
        let alpha = f64::from(pixel[3].clamp(0.0, 1.0));
        for channel in 0..3 {
            total[channel] += f64::from(pixel[channel]) * alpha;
        }
        total[3] += alpha;
    }
    if pixels.is_empty() || total[3] <= 0.0 {
        return [0.0; 4];
    }
    [(total[0] / total[3]) as f32, (total[1] / total[3]) as f32, (total[2] / total[3]) as f32, (total[3] / pixels.len() as f64) as f32]
}

fn sample(s: &mut Session, p: &Value) -> Result<Value> {
    let object = p.as_object().ok_or_else(|| bad("expected an object"))?;
    if object.keys().any(|key| !matches!(key.as_str(), "x" | "y" | "size" | "source" | "layer" | "coalesce")) {
        return Err(bad("unknown sample property"));
    }
    let size = p.get("size").map(size).transpose()?.unwrap_or(1);
    let source = p.get("source").map(source).transpose()?.unwrap_or(Source::All);
    let coordinate = |name| {
        p.get(name)
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite() && x.floor() >= f64::from(i32::MIN) + 101.0 && x.floor() <= f64::from(i32::MAX) - 102.0)
            .map(|x| x.floor() as i32)
            .ok_or_else(|| bad(format!("{name} must be a finite document coordinate")))
    };
    let (x, y) = (coordinate("x")?, coordinate("y")?);
    let st = s.active().ok_or(EngineError::NoDocument)?;
    validate_tree(&st.doc)?;
    let layer = match p.get("layer") {
        Some(value) => Some(LayerId(value.as_u64().ok_or_else(|| bad("invalid layer id"))?)),
        None => st.active_layer,
    };
    if let Some(id) = layer
        && st.doc.layer(id).is_none()
    {
        return Err(EngineError::NoLayer(id));
    }
    let radius = (size / 2) as i32;
    let area = Rect::from_xywh(x - radius, y - radius, size, size).intersect(&st.doc.bounds());
    Ok(json!(average(&read(&st.doc, layer, source, area)?)))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: COMMAND,
        label: "Sample Document Color",
        menu: &[],
        shortcut: None,
        params: r#"{"x":number,"y":number,"size":1|3|5|11|31|51|101=1,"source":"current|currentBelow|all|allNoAdjustments|currentBelowNoAdjustments"="all","layer":id?} -> straight RGBA; read-only"#,
        enabled: |s| s.active().map(|_| ()).ok_or_else(|| "no document open".into()),
        run: sample,
        journal: false,
    }]
}

#[cfg(test)]
#[path = "sample_color_tests_v79.rs"]
mod tests;
