//! Interactive save targets and dialog suggestions; explicit destinations always win.

use photocraft_color::{BlendMode, ColorMode, SampleType};
use photocraft_doc::{Document, LayerContent};

/// The normal Save command may reuse a flat source while it is still a simple
/// pixel image. New editing structure needs a layered destination instead.
/// This does not grant filesystem authority to automation or change Save As.
pub(crate) fn save_in_place_path(doc: &Document, path: Option<&str>) -> Option<String> {
    let path = path?;
    (photocraft_engine::file_cmds::saves_in_place(path) || can_save_flat(doc, path)).then(|| path.to_string())
}

/// Keep a reusable source format; otherwise suggest PSD to preserve editing
/// structure, or to replace an unsupported/read-only source format.
pub(crate) fn suggested_save_path(doc: &Document, path: Option<&str>) -> String {
    match path {
        Some(path) if save_in_place_path(doc, Some(path)).is_some() => path.to_string(),
        Some(path) => with_psd_extension(path),
        None => with_psd_extension(&doc.name),
    }
}

/// A flat derivative must not replace an established layered master. A JPEG
/// that can't represent the document's editable structure is also a copy.
pub(crate) fn saves_as_flat_copy(doc: &Document, source: Option<&str>, target: &str, tiff_layers: bool) -> bool {
    let Some(format) = photocraft_engine::file_cmds::extension(target).and_then(|ext| photocraft_codecs::from_extension(&ext)) else {
        return false;
    };
    (format == photocraft_codecs::Format::Jpeg && !can_save_flat(doc, target))
        || (source.is_some_and(photocraft_engine::file_cmds::saves_in_place)
            && !photocraft_engine::file_cmds::saves_in_place(target)
            && !(format == photocraft_codecs::Format::Tiff && tiff_layers && photocraft_io::tiff_layers::would_write_layers(doc)))
}

fn can_save_flat(doc: &Document, path: &str) -> bool {
    let [layer] = doc.layers.as_slice() else { return false };
    let LayerContent::Raster(surface) = &layer.content else { return false };
    if surface.format() != doc.pixel_format()
        || !layer.visible
        || layer.opacity != 1.0
        || layer.fill_opacity != 1.0
        || !matches!(layer.blend, BlendMode::Normal | BlendMode::PassThrough)
        || layer.clipped
        || layer.mask.is_some()
        || layer.vector_mask.is_some()
        || !layer.effects.items.is_empty()
        || layer.effects.psd_raw.is_some()
        || !layer.psd_blocks.is_empty()
        || layer.excluded_channels != 0
        || layer.blend_if != Default::default()
        || layer.video.is_some()
        || layer.link_group.is_some()
        || !doc.channels.is_empty()
        || !doc.paths.is_empty()
        || doc.work_path.is_some()
        || doc.clipping_path.is_some()
        || doc.quick_mask.is_some()
        || !doc.patterns.is_empty()
        || !doc.layer_comps.is_empty()
        || !doc.notes.is_empty()
        || doc.timeline.is_some()
        || !doc.wet_paint.is_empty()
    {
        return false;
    }
    // Flat export clips to the canvas; preserve recoverable pixels outside it.
    // content_bounds examines outer tiles rather than allocating a composite.
    let content = surface.content_bounds();
    if !content.is_empty() && !doc.bounds().contains_rect(&content) {
        return false;
    }
    let Some(ext) = photocraft_engine::file_cmds::extension(path) else { return false };
    let Some(format) = photocraft_codecs::from_extension(&ext) else { return false };
    let caps = format.caps();
    let depth = match doc.depth {
        SampleType::U8 => photocraft_codecs::SampleType::U8,
        SampleType::U16 => photocraft_codecs::SampleType::U16,
        SampleType::F32 => photocraft_codecs::SampleType::F32,
    };
    let layouts = match doc.mode {
        ColorMode::Rgb => [photocraft_codecs::ChannelLayout::Rgb, photocraft_codecs::ChannelLayout::Rgba],
        ColorMode::Grayscale => [photocraft_codecs::ChannelLayout::Gray, photocraft_codecs::ChannelLayout::GrayA],
        ColorMode::Cmyk => [photocraft_codecs::ChannelLayout::Cmyk, photocraft_codecs::ChannelLayout::CmykA],
        _ => return false,
    };
    caps.write
        && caps.depths.contains(&depth)
        && layouts.iter().any(|layout| caps.layouts.contains(layout))
        && format.max_dimensions().is_none_or(|(width, height)| doc.size.width <= width && doc.size.height <= height)
}

fn with_psd_extension(path: &str) -> String {
    let start = path.rfind(['/', '\\']).map_or(0, |offset| offset + 1);
    let name = path.get(start..).unwrap_or(path);
    let stem = match name.rfind('.') {
        Some(offset) if offset > 0 => name.get(..offset).unwrap_or(name),
        _ => name,
    };
    let directory = path.get(..start).unwrap_or("");
    format!("{directory}{}.psd", if stem.is_empty() { "Untitled" } else { stem })
}

#[cfg(test)]
#[path = "save_defaults_tests_v74.rs"]
mod tests;
