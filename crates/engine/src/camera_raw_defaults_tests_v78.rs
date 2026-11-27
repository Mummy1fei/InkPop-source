use super::*;
use std::sync::Arc;

fn fixture(depth: u32, gray: bool) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":8,"height":6,"depth":depth,"mode":if gray {"grayscale"} else {"rgb"},"background":"transparent"})).unwrap();
    s.edit("Synthetic output-depth sample", |doc, active| {
        doc.resolution_dpi = 123.0;
        let surface = doc.layer_mut(active.ok_or(EngineError::NoDocument)?).ok_or(EngineError::NoDocument)?.surface_mut().ok_or(EngineError::NoDocument)?;
        let pixel = photocraft_raster::from_rgba(&surface.format(), [0.337, 0.513, 0.721, 0.413]);
        surface.write_pixel(2, 2, &pixel);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn raw_defaults_v78_explicit_output_converts_pixels_depth_and_dpi_without_editing_source() {
    for source_depth in [8, 16, 32] {
        for gray in [false, true] {
            for output_depth in [8, 16] {
                let mut s = fixture(source_depth, gray);
                let original = s.active().unwrap().doc.clone();
                let history = s.active().unwrap().history.past_len();
                let expected_format = photocraft_color::PixelFormat::new(original.mode, if output_depth == 8 { SampleType::U8 } else { SampleType::U16 }, true);
                let expected = crate::file_cmds::flattened(&original, original.pixel_format()).convert(expected_format);
                let result = s.execute(OUTPUT, json!({"settings":{},"outputDepth":output_depth,"resolution":267.5})).unwrap();
                assert_eq!(result["depth"], output_depth);
                assert_eq!(result["resolution"], 267.5);
                let out = &s.active().unwrap().doc;
                assert_eq!(out.depth, expected_format.sample);
                assert_eq!(out.resolution_dpi, 267.5);
                let surface = out.layers[0].surface().unwrap();
                assert_eq!(surface.format(), expected_format);
                for (a, b) in surface.rgba(2, 2).into_iter().zip(expected.rgba(2, 2)) {
                    assert!((a - b).abs() < 0.005, "{source_depth}/{output_depth}/{gray}: {a} {b}");
                }
                assert!((surface.rgba(2, 2)[3] - 0.413).abs() < 0.005);
                assert!(Arc::ptr_eq(&s.documents()[0].doc, &original));
                assert_eq!(s.documents()[0].history.past_len(), history);
            }
        }
    }
}

#[test]
fn raw_defaults_v78_omitted_output_options_preserve_legacy_depth_and_resolution() {
    for depth in [8, 16, 32] {
        let mut s = fixture(depth, false);
        let original = s.active().unwrap().doc.clone();
        s.execute("prefs.set", json!({"path":"rawDefaults.bitDepth","value":"8"})).unwrap();
        s.execute("prefs.set", json!({"path":"rawDefaults.resolution","value":500})).unwrap();
        s.execute(OUTPUT, json!({"settings":{}})).unwrap();
        assert_eq!(s.active().unwrap().doc.depth, original.depth);
        assert_eq!(s.active().unwrap().doc.resolution_dpi, 123.0);
    }
}

#[test]
fn raw_defaults_v78_invalid_output_format_is_atomic() {
    let mut s = fixture(32, false);
    let original = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    for invalid in [
        json!({"outputDepth":32}),
        json!({"outputDepth":8.5}),
        json!({"outputDepth":"8"}),
        json!({"outputDepth":null}),
        json!({"outputDepth":true}),
        json!({"resolution":0}),
        json!({"resolution":10001}),
        json!({"resolution":"300"}),
        json!({"resolution":null}),
        json!({"resolution":true}),
    ] {
        let mut request = invalid;
        request["settings"] = json!({"exposure":0.4});
        assert!(s.execute(OUTPUT, request).is_err());
        assert_eq!(s.documents().len(), 1);
        assert!(Arc::ptr_eq(&s.active().unwrap().doc, &original));
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}
