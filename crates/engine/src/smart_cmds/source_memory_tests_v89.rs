use super::*;
use photocraft_color::{ColorMode, SampleType};

fn document(mode: ColorMode, sample: SampleType, alpha: f32) -> Document {
    let mut doc = Document::new("Source memory v89", Size::new(11, 9), mode, sample);
    let fmt = doc.pixel_format();
    let mut surface = Surface::new(fmt);
    let pixel = photocraft_raster::from_rgba(&fmt, [0.15, 0.6, 0.8, alpha]);
    surface.write_region(doc.bounds(), &pixel.repeat(99));
    doc.layers.push(Layer::new("Pixel", LayerContent::Raster(surface)));
    doc
}

fn assert_legacy_pixels(doc: &Document, fmt: PixelFormat) {
    let expected = buffer_to_surface(&photocraft_compose::flatten(doc), fmt);
    let actual = source_from_document(doc, fmt);
    assert_eq!(actual.bounds, doc.bounds());
    assert_eq!(actual.surface.as_ref(), &expected);
}

#[test]
fn opaque_rgb_source_shares_native_tiles_at_all_depths_v89() {
    for sample in SampleType::ALL {
        let doc = document(ColorMode::Rgb, sample, 1.0);
        let fmt = doc.pixel_format();
        let actual = source_from_document(&doc, fmt);
        let original = doc.layers[0].surface().unwrap();
        for (coord, tile) in original.tiles() {
            assert!(Arc::ptr_eq(tile, actual.surface.tile(*coord).unwrap()));
        }
        assert_legacy_pixels(&doc, fmt);
    }
}

#[test]
fn advanced_and_non_rgb_sources_match_banded_compositor_v89() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk] {
        for sample in SampleType::ALL {
            for alpha in [0.0, 0.5, 1.0] {
                let doc = document(mode, sample, alpha);
                assert_legacy_pixels(&doc, doc.pixel_format());
                let mut faded = doc.clone();
                faded.layers[0].opacity = 0.5;
                assert!(native_source(&faded, faded.pixel_format()).is_none());
                assert_legacy_pixels(&faded, faded.pixel_format());
            }
        }
    }
    let mut doc = document(ColorMode::Rgb, SampleType::U8, 1.0);
    doc.layers[0].mask = Some(LayerMask::reveal_all());
    assert!(native_source(&doc, doc.pixel_format()).is_none());
    assert_legacy_pixels(&doc, doc.pixel_format());
}

#[test]
fn nonzero_default_and_off_canvas_sources_are_composited_and_cropped_v89() {
    let mut doc = document(ColorMode::Rgb, SampleType::F32, 1.0);
    let fmt = doc.pixel_format();
    doc.layers[0].content = LayerContent::Raster(Surface::with_default(fmt, &[0.2, 0.3, 0.4, 1.0]));
    assert!(native_source(&doc, fmt).is_none());
    assert_legacy_pixels(&doc, fmt);
    doc.layers[0].surface_mut().unwrap().write_region(Rect::new(-2, -2, 2, 2), &[0.8, 0.1, 0.5, 1.0].repeat(16));
    assert!(native_source(&doc, fmt).is_none());
    assert_legacy_pixels(&doc, fmt);
}

#[test]
fn source_cache_enforces_native_byte_limit_even_for_singleton_v89() {
    let fmt = PixelFormat::RGBA8;
    let small = source_from_document(&document(ColorMode::Rgb, SampleType::U8, 1.0), fmt);
    let one_tile = source_storage_bytes(&small);
    let mut large_surface = small.surface.as_ref().clone();
    large_surface.write_region(Rect::new(256, 0, 257, 1), &[0.3, 0.2, 0.1, 1.0]);
    let large = SourceImage { surface: Arc::new(large_surface), bounds: Rect::new(0, 0, 257, 9) };
    let mut cache = Vec::new();
    assert!(!cache_put_limited(&mut cache, ([1; 32], fmt), large.clone(), 8, one_tile));
    assert!(cache.is_empty());
    assert!(cache_put_limited(&mut cache, ([2; 32], fmt), small.clone(), 8, one_tile));
    assert!(!cache_put_limited(&mut cache, ([3; 32], fmt), large, 8, one_tile));
    assert_eq!(cache.len(), 1);
    assert_eq!(cache[0].key, ([2; 32], fmt));
    assert!(cache_put_limited(&mut cache, ([4; 32], fmt), small.clone(), 8, one_tile));
    assert_eq!(cache.len(), 1);
    assert_eq!(cache[0].key, ([4; 32], fmt));
    assert!(cache_put_limited(&mut cache, ([5; 32], fmt), small, 1, one_tile * 8));
    assert_eq!(cache.len(), 1);
    assert_eq!(cache[0].key, ([5; 32], fmt));
}
