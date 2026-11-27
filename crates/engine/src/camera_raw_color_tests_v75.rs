use super::*;
use photocraft_algo::camera_raw_color::{ColorSpaces, RgbSpace};
use std::sync::Arc;

fn fixture(depth: u32, gray: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":16,"height":12,"depth":depth,"mode":if gray {"grayscale"} else {"rgb"},"background":"transparent"})).unwrap();
    s.execute("edit.fill", json!({"color":"#305080"})).unwrap();
    s.edit("Synthetic translucent sample", |doc, active| {
        let surface = doc.layer_mut(active.ok_or(EngineError::NoDocument)?).ok_or(EngineError::NoDocument)?.surface_mut().ok_or(EngineError::NoDocument)?;
        let pixel = photocraft_raster::from_rgba(&surface.format(), [if depth == 32 { 1.7 } else { 0.7 }, 0.3, 0.5, 0.4]);
        surface.write_pixel(0, 0, &pixel);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn raw_color_v75_output_is_real_icc_conversion_and_original_is_unchanged() {
    for depth in [8, 16, 32] {
        for gray in [false, true] {
            let mut s = fixture(depth, gray);
            s.execute("layer.new.layer", json!({"name":"Visible overlay"})).unwrap();
            s.execute("edit.fill", json!({"color":"#804020"})).unwrap();
            s.edit("Synthetic opacity", |doc, active| {
                doc.layer_mut(active.ok_or(EngineError::NoDocument)?).ok_or(EngineError::NoDocument)?.opacity = 0.4;
                Ok(())
            })
            .unwrap();
            s.execute("select.rect", json!({"x":0,"y":0,"width":8,"height":12})).unwrap();
            let original = s.active().unwrap().doc.clone();
            let history = s.active().unwrap().history.past_len();
            let revision = s.active().unwrap().revision;
            let dirty = s.active().unwrap().is_dirty();
            let params = CameraRaw {
                exposure: 0.4,
                color_spaces: ColorSpaces { working: RgbSpace::ProPhotoRgb, output: RgbSpace::DisplayP3, ..Default::default() },
                ..Default::default()
            };
            let mut oracle = Session::new();
            oracle.add_document((*original).clone(), None);
            oracle.select_layer(s.active().unwrap().active_layer.unwrap()).unwrap();
            oracle.execute(RAW, json!(params)).unwrap();
            let reference = crate::file_cmds::flattened(&oracle.active().unwrap().doc, oracle.active().unwrap().doc.pixel_format());
            let source_profile = crate::color_cmds::document_profile(&original);
            let transform = photocraft_cms::Transform::new(
                &source_profile,
                photocraft_cms::Builtin::DisplayP3.profile(),
                photocraft_cms::Intent::RelativeColorimetric,
                true,
            )
            .unwrap();
            let converted = crate::color_cmds::convert_surface(
                &reference,
                original.mode,
                photocraft_color::PixelFormat::new(ColorMode::Rgb, original.depth, true),
                &transform,
            );
            let result = s.execute(OUTPUT, json!({"settings":params})).unwrap();
            assert_eq!(result["merged"], true);
            assert_eq!(s.documents().len(), 2);
            assert!(Arc::ptr_eq(&s.documents()[0].doc, &original));
            assert_eq!(s.documents()[0].history.past_len(), history);
            assert_eq!(s.documents()[0].revision, revision);
            assert_eq!(s.documents()[0].is_dirty(), dirty);
            let out = s.active().unwrap();
            assert!(out.is_dirty());
            assert!(out.path.is_none());
            assert!(out.floating.is_none());
            assert_eq!(out.doc.mode, ColorMode::Rgb);
            assert_eq!(out.doc.depth, original.depth);
            assert_eq!(out.doc.layers.len(), 1);
            assert_eq!(out.doc.icc_profile.as_ref().unwrap(), &photocraft_cms::Builtin::DisplayP3.profile().to_bytes());
            let pixel = out.doc.layers[0].surface().unwrap().rgba(0, 0);
            let expected = converted.rgba(0, 0);
            for (a, b) in pixel.into_iter().zip(expected) {
                assert!((a - b).abs() < 0.006, "{depth} {gray}: {pixel:?} {expected:?}");
            }
            assert!(pixel[3] > 0.4 && pixel[3] < 0.9, "alpha retained: {pixel:?}");
        }
    }
}

#[test]
fn raw_color_v75_smart_replay_source_capture_preset_and_output_replace() {
    let mut s = fixture(32, false);
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let settings = json!({"exposure":0.4,"colorSpaces":{"working":"proPhotoRgb","output":"adobeRgb"}});
    s.execute(RAW, settings).unwrap();
    let layer = s.active().unwrap().active_layer.unwrap();
    let LayerContent::Smart(smart) = &s.active().unwrap().doc.layer(layer).unwrap().content else {
        panic!("smart");
    };
    assert_eq!(smart.smart_filters[0].params["colorSpaces"]["sourceProfile"], "srgb");
    let mut params = raw_params(RAW, &smart.smart_filters[0].params).unwrap();
    params.exposure = 0.7;
    s.execute("cameraRaw.presets.save", json!({"name":"Working and output","params":params})).unwrap();
    let saved = s.execute("cameraRaw.presets.get", json!({"name":"Working and output"})).unwrap();
    assert_eq!(saved["params"]["colorSpaces"]["working"], "proPhotoRgb");
    let original = s.active().unwrap().doc.clone();
    s.execute(OUTPUT, json!({"settings":params,"layer":layer.0,"smartFilter":0})).unwrap();
    assert!(Arc::ptr_eq(&s.documents()[0].doc, &original));
    assert_eq!(s.active().unwrap().doc.icc_profile.as_ref().unwrap(), &photocraft_cms::Builtin::AdobeRgbCompat.profile().to_bytes());
    assert_eq!(s.active().unwrap().doc.layers.len(), 1);
}

#[test]
fn raw_color_v75_invalid_output_is_atomic_and_does_not_create_documents() {
    let mut s = fixture(8, false);
    let original = s.active().unwrap().doc.clone();
    for p in [
        json!({}),
        json!({"settings":false}),
        json!({"settings":{},"smartFilter":-1}),
        json!({"settings":{},"layer":u64::MAX}),
        json!({"settings":{"colorSpaces":{"working":"unknown"}}}),
        json!({"settings":{},"unexpected":true}),
    ] {
        assert!(s.execute(OUTPUT, p).is_err());
        assert_eq!(s.documents().len(), 1);
        assert!(Arc::ptr_eq(&s.active().unwrap().doc, &original));
    }
}

#[test]
fn raw_color_v75_table_icc_hdr_working_and_output_fail_atomically() {
    for gray in [false, true] {
        let mut profile = if gray { photocraft_cms::Builtin::SGray.profile().clone() } else { photocraft_cms::Builtin::Srgb.profile().clone() };
        let table = photocraft_cms::Curve::Table(vec![0., 0.214, 1.]);
        if gray {
            profile.gray_trc = Some(table);
        } else {
            profile.trc = Some(std::array::from_fn(|_| table.clone()));
        }
        let profile = profile.with_encoded_bytes();
        let mut s = fixture(32, gray);
        s.edit("Synthetic sampled ICC and HDR", |doc, active| {
            doc.icc_profile = Some(profile.to_bytes());
            let surface = doc.layer_mut(active.ok_or(EngineError::NoDocument)?).ok_or(EngineError::NoDocument)?.surface_mut().ok_or(EngineError::NoDocument)?;
            surface.write_pixel(0, 0, &photocraft_raster::from_rgba(&surface.format(), [1.8, 1.8, 1.8, 0.61]));
            Ok(())
        })
        .unwrap();
        let original = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let history = s.active().unwrap().history.past_len();
        let error = s.execute(RAW, json!({"contrast":20,"colorSpaces":{"working":"adobeRgb"}})).unwrap_err().to_string();
        assert!(error.contains(photocraft_algo::camera_raw_color::HDR_TABLE_ERROR));
        let error = s.execute(OUTPUT, json!({"settings":{"contrast":20,"colorSpaces":{"output":"displayP3"}}})).unwrap_err().to_string();
        assert!(error.contains(photocraft_algo::camera_raw_color::HDR_TABLE_ERROR));
        assert_eq!(s.documents().len(), 1);
        assert!(Arc::ptr_eq(&s.active().unwrap().doc, &original));
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(s.active().unwrap().history.past_len(), history);
        let source = original.layer(original.top_layer().unwrap()).unwrap().surface().unwrap();
        assert!(source.rgba(0, 0)[0] > 1.79);
        let replay = json!({"contrast":20,"colorSpaces":{"working":"adobeRgb","sourceProfile":encode_source(&profile).unwrap()}});
        assert!(super::super::apply_to_surface(RAW, &replay, source, original.bounds()).is_none(), "stored unsafe HDR replay is rejected");
        for depth in [8, 16] {
            let mut integer = fixture(depth, gray);
            integer
                .edit("Synthetic sampled ICC", |doc, _| {
                    doc.icc_profile = Some(profile.to_bytes());
                    Ok(())
                })
                .unwrap();
            assert!(integer.execute(RAW, json!({"contrast":20,"colorSpaces":{"working":"adobeRgb"}})).is_ok());
        }
    }
}
