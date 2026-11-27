//! Camera Raw context binding and a true ICC-tagged, merged output copy.
use super::*;
use photocraft_algo::camera_raw_color::{RgbSpace, encode_source};
use photocraft_doc::{Layer, Size};

pub const OUTPUT: &str = "cameraRaw.createOutput";

pub fn bind_source(params: &mut CameraRaw, doc: &Document, force: bool) -> Result<()> {
    if params.color_spaces.working == RgbSpace::Document {
        if force {
            params.color_spaces.source_profile.clear();
        }
        return Ok(());
    }
    if force || params.color_spaces.source_profile.is_empty() {
        params.color_spaces.source_profile = encode_source(&crate::color_cmds::document_profile(doc)).map_err(|e| bad(RAW, e))?;
    }
    params.color_spaces.validate_depth(doc.depth == SampleType::F32).map_err(|e| bad(RAW, e))
}

pub(super) fn create_output(s: &mut Session, p: &Value) -> Result<Value> {
    let fields = p.as_object().ok_or_else(|| bad(OUTPUT, "expected an object"))?;
    if fields.keys().any(|key| !matches!(key.as_str(), "layer" | "settings" | "smartFilter" | "outputDepth" | "resolution")) {
        return Err(bad(OUTPUT, "unknown output property"));
    }
    // Validate optional output-only settings before rendering or adding a document.
    // Omission preserves the command's original depth/DPI, including 32-bit HDR.
    let output_depth = match p.get("outputDepth") {
        None => None,
        Some(value) => Some(match value.as_u64() {
            Some(8) => SampleType::U8,
            Some(16) => SampleType::U16,
            _ => return Err(bad(OUTPUT, "outputDepth must be 8 or 16")),
        }),
    };
    let resolution = match p.get("resolution") {
        None => None,
        Some(value) => Some(
            value
                .as_f64()
                .filter(|dpi| dpi.is_finite() && (1.0..=10000.0).contains(dpi))
                .ok_or_else(|| bad(OUTPUT, "resolution must be between 1 and 10000 pixels/inch"))?,
        ),
    };
    let settings = p.get("settings").filter(|v| v.is_object()).ok_or_else(|| bad(OUTPUT, "settings must be Camera Raw parameters"))?;
    let layer = target(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let original = st.doc.clone();
    let canvas_pixels = (original.size.width as usize).checked_mul(original.size.height as usize).ok_or_else(|| bad(OUTPUT, "output dimensions overflow"))?;
    if canvas_pixels == 0 || canvas_pixels > 64 * 1024 * 1024 {
        return Err(bad(OUTPUT, "output copy is limited to 64 megapixels"));
    }
    let source = original.layer(layer).and_then(|l| l.surface()).ok_or_else(|| bad(OUTPUT, "target needs rendered pixels"))?;
    let area = original.bounds().union(&source.content_bounds());
    let area_pixels = (area.width() as usize).checked_mul(area.height() as usize).ok_or_else(|| bad(OUTPUT, "filter dimensions overflow"))?;
    if area_pixels > 64 * 1024 * 1024 {
        return Err(bad(OUTPUT, "Camera Raw source exceeds 64 megapixels"));
    }
    let mut params = raw_params(OUTPUT, settings)?;
    params.pixel_scale = 1.0;
    let smart_index = match p.get("smartFilter") {
        Some(value) => Some(value.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(OUTPUT, "smartFilter must be an index"))?),
        None => None,
    };
    let previous = if let Some(index) = smart_index {
        let Some(LayerContent::Smart(smart)) = original.layer(layer).map(|l| &l.content) else {
            return Err(bad(OUTPUT, "target is not a smart object"));
        };
        let filter = smart.smart_filters.get(index).filter(|f| f.command == RAW).ok_or_else(|| bad(OUTPUT, "smartFilter is not a Camera Raw filter"))?;
        Some(raw_params(RAW, &filter.params)?)
    } else {
        None
    };
    bind_source(&mut params, &original, previous.is_none())?;
    if original.depth == SampleType::F32
        && let Some(destination) = params.color_spaces.output.profile()
    {
        photocraft_algo::camera_raw_color::validate_float_profile(&crate::color_cmds::document_profile(&original)).map_err(|e| bad(OUTPUT, e))?;
        photocraft_algo::camera_raw_color::validate_float_profile(destination).map_err(|e| bad(OUTPUT, e))?;
    }
    if let Some(previous) = &previous {
        params.validate_changes(previous).map_err(|e| bad(OUTPUT, e))?;
    } else {
        params.validate().map_err(|e| bad(OUTPUT, e))?;
    }
    // All potentially failing edits happen in an isolated session; the original's
    // layers, Smart sources, selection, dirty state and undo history are untouched.
    let mut scratch = Session::new();
    scratch.add_document((*original).clone(), None);
    scratch.select_layer(layer)?;
    let mut resolved = serde_json::to_value(&params).map_err(|e| bad(OUTPUT, e.to_string()))?;
    if let Some(index) = smart_index {
        scratch.execute("layer.smartFilter.setParams", json!({"layer":layer.0,"index":index,"params":resolved}))?;
    } else {
        resolved["layer"] = json!(layer.0);
        scratch.execute(RAW, resolved)?;
    }
    let developed = scratch.active().ok_or(EngineError::NoDocument)?;
    let merged = crate::file_cmds::flattened(&developed.doc, developed.doc.pixel_format());
    let mut output =
        Document::new(format!("{} - Camera Raw", original.name), Size::new(original.size.width, original.size.height), original.mode, original.depth);
    output.resolution_dpi = original.resolution_dpi;
    output.icc_profile = Some(crate::color_cmds::document_profile(&original).to_bytes());
    output.layers.push(Layer::new("Camera Raw", LayerContent::Raster(merged)));
    if let Some(destination) = params.color_spaces.output.profile() {
        // This document contains only rendered pixels, so no Smart replay or
        // editable colour layer can later reintroduce the old encoding.
        crate::color_cmds::convert_document(&mut output, destination, photocraft_cms::Intent::RelativeColorimetric, true)?;
    }
    // Convert merged ICC-encoded pixels after the profile transform, retaining
    // source precision during color conversion without editing source layers.
    if let Some(depth) = output_depth {
        output.depth = depth;
        let format = output.pixel_format();
        for layer in &mut output.layers {
            if let LayerContent::Raster(surface) = &mut layer.content {
                *surface = surface.convert(format);
            }
        }
    }
    if let Some(dpi) = resolution {
        output.resolution_dpi = dpi as f32;
    }
    let depth = match output.depth {
        SampleType::U8 => 8,
        SampleType::U16 => 16,
        SampleType::F32 => 32,
    };
    let resolution = output.resolution_dpi;
    let mode = output.mode;
    let profile = crate::color_cmds::document_profile(&output).description.clone();
    let index = s.add_document(output, None);
    if let Some(output) = s.active_mut() {
        output.saved_revision = 0;
    }
    Ok(json!({"document":index,"profile":profile,"mode":format!("{mode:?}"),"merged":true,"originalUnchanged":true,"depth":depth,"resolution":resolution}))
}

#[cfg(test)]
#[path = "camera_raw_color_tests_v75.rs"]
mod tests;

#[cfg(test)]
#[path = "camera_raw_defaults_tests_v78.rs"]
mod defaults_tests_v78;
