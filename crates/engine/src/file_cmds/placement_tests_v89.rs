use super::*;
use photocraft_color::SampleType;
use photocraft_geom::Size;

fn encoded_source(mode: ColorMode, sample: SampleType) -> Vec<u8> {
    let mut source = Document::new("Placement v89", Size::new(9, 7), mode, sample);
    let fmt = source.pixel_format();
    let mut pixels = Surface::new(fmt);
    let color = photocraft_raster::from_rgba(&fmt, [0.2, 0.6, 0.9, 1.0]);
    pixels.write_region(source.bounds(), &color.repeat(63));
    source.layers.push(Layer::new("Pixels", LayerContent::Raster(pixels)));
    crate::smart_cmds::encode_source(&source).unwrap()
}

#[test]
fn pixel_and_smart_placement_match_and_undo_as_one_step_v89() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk] {
        for (sample, depth) in [(SampleType::U8, 8), (SampleType::U16, 16), (SampleType::F32, 32)] {
            let bytes = encoded_source(mode, sample);
            let mut session = Session::new();
            session.execute("file.new", json!({"width":19,"height":17,"depth":depth})).unwrap();
            let original = session.active().unwrap().doc.clone();
            let fmt = original.pixel_format();
            let decoded = crate::smart_cmds::decode_source_image("Source.pcraft", &bytes, fmt).unwrap();
            let expected = translate_placed(&decoded.surface, 5, 5).unwrap();
            for smart in [false, true] {
                let result = place_bytes(&mut session, "Source.pcraft", bytes.clone(), None, &json!({"asSmartObject":smart})).unwrap();
                assert_eq!(result["asSmartObject"], json!(smart));
                assert_eq!(result["bounds"], json!([5.0, 5.0, 14.0, 12.0]));
                let state = session.active().unwrap();
                let layer = state.doc.layer(state.active_layer.unwrap()).unwrap();
                match &layer.content {
                    LayerContent::Raster(surface) => {
                        assert!(!smart);
                        assert_eq!(surface, &expected);
                    }
                    LayerContent::Smart(object) => {
                        assert!(smart);
                        assert_eq!(object.cache.as_ref().unwrap(), &expected);
                        let SmartSource::Embedded { bytes: retained, .. } = &object.source else { panic!("expected embedded") };
                        assert_eq!(retained.as_ref(), &bytes);
                    }
                    other => panic!("unexpected content {}", other.kind_name()),
                }
                assert!(session.undo());
                assert_eq!(session.active().unwrap().doc.layers, original.layers);
                assert!(session.redo());
                let state = session.active().unwrap();
                assert_eq!(matches!(state.doc.layer(state.active_layer.unwrap()).unwrap().content, LayerContent::Smart(_)), smart);
                assert!(session.undo());
            }
        }
    }
}

#[test]
fn linked_pixels_and_malformed_options_fail_before_document_edit_v89() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":19,"height":17})).unwrap();
    for params in [
        json!({"asSmartObject":false}),
        json!({"asSmartObject":"false"}),
        json!({"scale":0}),
        json!({"scale":-1}),
        json!({"center":["x",2]}),
        json!({"center":[1]}),
        json!({"fit":1}),
    ] {
        let before = session.active().unwrap().doc.clone();
        let err = place_bytes(&mut session, "bad.dat", vec![0, 1, 2], Some("bad.dat".into()), &params).unwrap_err();
        assert!(matches!(err, EngineError::BadParams { .. }), "{params}: {err}");
        assert!(Arc::ptr_eq(&before, &session.active().unwrap().doc));
    }
}

#[test]
fn explicit_defaults_remain_smart_and_pixel_place_does_not_seed_source_cache_v89() {
    let mut unique = import("source.pcraft", &encoded_source(ColorMode::Rgb, SampleType::U8)).unwrap();
    unique.name = "v89 independent cache seeding fixture".into();
    let bytes = crate::smart_cmds::encode_source(&unique).unwrap();
    let mut session = Session::new();
    session.execute("file.new", json!({"width":9,"height":7})).unwrap();
    let fmt = session.active().unwrap().doc.pixel_format();
    assert_eq!(crate::smart_cmds::source_decode_count(&bytes, fmt), 0);
    place_bytes(&mut session, "Cache.pcraft", bytes.clone(), None, &json!({"asSmartObject":false})).unwrap();
    assert_eq!(crate::smart_cmds::source_decode_count(&bytes, fmt), 0);
    place_bytes(&mut session, "Cache.pcraft", bytes.clone(), None, &json!({})).unwrap();
    assert_eq!(crate::smart_cmds::source_decode_count(&bytes, fmt), 1);
    crate::smart_cmds::source_image("Cache.pcraft", &bytes, fmt).unwrap();
    assert_eq!(crate::smart_cmds::source_decode_count(&bytes, fmt), 1);
    place_bytes(&mut session, "Cache.pcraft", bytes, Some("linked.pcraft".into()), &json!({})).unwrap();
    let state = session.active().unwrap();
    let LayerContent::Smart(object) = &state.doc.layer(state.active_layer.unwrap()).unwrap().content else { panic!("expected smart") };
    assert!(matches!(&object.source, SmartSource::Linked { path } if path == "linked.pcraft"));
}

#[test]
#[ignore = "isolated 24MP placement measurement; run with INKPOP_PLACE_BENCH=legacy|pixel|smart"]
fn placement_24mp_measurement_v89() {
    let mode = std::env::var("INKPOP_PLACE_BENCH").unwrap();
    let sample = match std::env::var("INKPOP_BENCH_DEPTH").as_deref() {
        Ok("16") => SampleType::U16,
        Ok("32") => SampleType::F32,
        _ => SampleType::U8,
    };
    let fit = std::env::var("INKPOP_BENCH_FIT").as_deref() == Ok("1");
    let mut source = Document::new("24MP synthetic", Size::new(6000, 4000), ColorMode::Rgb, sample);
    let fmt = source.pixel_format();
    let mut layer = Layer::raster("opaque source", fmt);
    layer.surface_mut().unwrap().fill_rect(source.bounds(), &[0.2, 0.6, 0.9, 1.0]);
    source.layers.push(layer);
    let bytes = crate::smart_cmds::encode_source(&source).unwrap();
    drop(source);
    let mut session = Session::new();
    let (width, height) = if fit { (3000, 2000) } else { (6020, 4020) };
    session.execute("file.new", json!({"width":width,"height":height,"depth":if sample==SampleType::F32 {32} else if sample==SampleType::U16 {16} else {8},"background":"transparent"})).unwrap();
    println!("BENCH_READY");
    use std::io::Write;
    std::io::stdout().flush().unwrap();
    let started = std::time::Instant::now();
    let result = if mode == "legacy" {
        let decoded = import("source.pcraft", &bytes).unwrap();
        let mut surface = flattened(&decoded, fmt);
        if fit {
            surface = resize_surface(&surface, 0.5, 0.5, Resample::Bicubic);
        }
        let surface = photocraft_algo::resample::translate_surface(&surface, if fit { 0 } else { 10 }, if fit { 0 } else { 10 });
        let object = SmartObject::new(SmartSource::Embedded { file_name: "source.pcraft".into(), bytes: Arc::new(bytes) }, Affine::IDENTITY, Some(surface));
        session
            .edit("legacy place", |doc, active| {
                *active = Some(doc.insert_above(*active, Layer::new("placed", LayerContent::Smart(object))));
                Ok(())
            })
            .unwrap();
        json!({"legacy":true})
    } else {
        place_bytes(&mut session, "source.pcraft", bytes, None, &json!({"asSmartObject":mode=="smart","fit":fit})).unwrap()
    };
    let millis = started.elapsed().as_secs_f64() * 1000.0;
    let state = session.active().unwrap();
    let surface = state.doc.layer(state.active_layer.unwrap()).unwrap().surface().unwrap();
    let p = surface.pixel(if fit { 20 } else { 30 }, if fit { 20 } else { 30 });
    assert!((p[0] - 0.2).abs() < 0.01 && (p[2] - 0.9).abs() < 0.01, "{p:?}");
    println!(
        "BENCH_RESULT {}",
        json!({"mode":mode,"depth":format!("{sample:?}"),"fit":fit,"milliseconds":millis,"nativeBytes":surface.tiles().map(|(_,t)|t.bytes().len()).sum::<usize>(),"result":result})
    );
    assert!(session.undo());
    assert!(session.redo());
}

#[test]
fn geometry_and_working_memory_preflight_reject_extremes_without_allocating_v89() {
    let input = placement_input(&json!({}), false).unwrap();
    assert!(placement_plan(Size::new(64, 64), Rect::new(0, 0, 9, 7), PixelFormat::RGBA8, input, false, 4096, 1024).is_err());
    for p in [json!({"scale":1e300}), json!({"center":[1e100,1]}), json!({"center":[-1e100,1]})] {
        let input = placement_input(&p, false).unwrap();
        assert!(placement_plan(Size::new(64, 64), Rect::new(0, 0, 9, 7), PixelFormat::RGBA8, input, false, 4096, usize::MAX).is_err());
    }
}

#[test]
fn native_translation_matches_legacy_bytes_across_tiles_and_formats_v89() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk] {
        for sample in SampleType::ALL {
            let fmt = PixelFormat::new(mode, sample, true);
            let mut source = Surface::new(fmt);
            let pixel = photocraft_raster::from_rgba(&fmt, [0.2, 0.7, 0.4, 0.8]);
            source.write_region(Rect::new(-2, 3, 260, 8), &pixel.repeat(262 * 5));
            source.write_region(Rect::new(0, 256, 256, 512), &pixel.repeat(256 * 256));
            for (dx, dy) in [(0, 0), (5, -7), (-261, 257)] {
                let expected = photocraft_algo::resample::translate_surface(&source, dx, dy);
                let got = translate_placed(&source, dx, dy).unwrap();
                assert_eq!(got.content_bounds(), expected.content_bounds());
                let area = got.content_bounds().union(&expected.content_bounds());
                assert_eq!(got.to_interleaved(area), expected.to_interleaved(area), "{mode:?}/{sample:?}/{dx}/{dy}");
            }
        }
    }
}
