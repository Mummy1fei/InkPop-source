use super::*;
use crate::Session;
use serde_json::json;

#[test]
fn no_confident_model_foreground_remains_empty_instead_of_stretching_noise() {
    let mut no_sky = vec![0.0001, 0.13, 0.2, 0.01];
    retain_confident_foreground(&mut no_sky);
    assert_eq!(no_sky, vec![0.0; 4]);
    let mut undecided = vec![0.5; 4];
    retain_confident_foreground(&mut undecided);
    assert_eq!(undecided, vec![0.0; 4]);
    let mut real_edge = vec![0.01, 0.25, 0.5, 0.99];
    let before = real_edge.clone();
    retain_confident_foreground(&mut real_edge);
    assert_eq!(real_edge, before, "valid boundary probabilities must remain unchanged");
}

fn classical_options() -> Options {
    Options { backend: BackendPreference::Classical, ..Default::default() }
}

fn landscape(width: usize, height: usize) -> Vec<[f32; 4]> {
    (0..width * height)
        .map(|i| {
            let y = i / width;
            if y < height * 3 / 5 { [0.25, 0.55, 0.95, 1.0] } else { [0.2, 0.3, 0.08, 1.0] }
        })
        .collect()
}

#[test]
fn hostile_dimensions_samples_and_sky_options_return_errors_before_analysis() {
    let ctx = JobCtx::new();
    for (width, height) in [(0, 1), (1, 0), (usize::MAX, 2), (10_001, 10_000)] {
        assert!(recognize_with_options(&[], width, height, Kind::Subject, classical_options(), &ctx).is_err());
    }
    assert!(recognize_with_options(&[[0.0; 4]], 2, 2, Kind::Sky, classical_options(), &ctx).is_err());
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut pixel = [0.0, 0.0, 0.0, 1.0];
        pixel[1] = value;
        assert!(recognize_with_options(&[pixel], 1, 1, Kind::Subject, classical_options(), &ctx).is_err());
    }
    let options = Options { sky: segment::sky::SkyParams { threshold: f32::NAN, ..Default::default() }, ..classical_options() };
    assert!(recognize_with_options(&[[0.0; 4]], 1, 1, Kind::Sky, options, &ctx).is_err());
}

#[test]
fn reference_composites_hidden_rgb_and_antialiases_small_details() {
    let ctx = JobCtx::new();
    let a = reference(&[[1.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 0.5]], 2, 1, &ctx).unwrap();
    let b = reference(&[[0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.5]], 2, 1, &ctx).unwrap();
    assert_eq!(a.image, b.image, "invisible RGB must not become object cues");
    assert_eq!(a.image.px, vec![[0.5; 3], [0.25; 3]]);
    assert_eq!(a.alpha, vec![0.0, 0.5]);
    let stripes: Vec<_> = (0..2048 * 8).map(|i| [((i % 2048) % 2) as f32, 0.0, 0.0, 1.0]).collect();
    let small = reference(&stripes, 2048, 8, &ctx).unwrap();
    assert_eq!([small.image.w, small.image.h], [1024, 4]);
    assert!(small.image.px.iter().all(|p| p[0] == 0.5), "box analysis must retain energy of one-pixel stripes");
}

#[test]
fn explicit_cancel_never_begins_model_or_classical_work() {
    let ctx = JobCtx::new();
    ctx.cancel();
    assert!(matches!(recognize(&landscape(32, 24), 32, 24, Kind::Sky, &ctx), Err(EngineError::Cancelled)));
}

#[test]
fn classical_sky_is_shared_and_high_resolution_output_is_not_a_256_raster() {
    let ctx = JobCtx::new();
    let pixels = landscape(600, 400);
    let result = recognize_with_options(&pixels, 600, 400, Kind::Sky, classical_options(), &ctx).unwrap();
    assert_eq!(result.backend, "ClassicalSky");
    assert_eq!([result.width, result.height], [600, 400]);
    assert_eq!(result.diagnostics.analysis_size, [400, 267]);
    assert_eq!(result.values.len(), 600 * 400);
    assert!(result.values[10 * 600 + 10] > 240);
    assert!(result.values[350 * 600 + 10] < 15);
    assert!(result.diagnostics.fallback_reason.is_none(), "an explicitly selected classical backend is not a fallback");
}

#[test]
fn subject_and_background_are_complementary_only_inside_source_alpha() {
    let ctx = JobCtx::new();
    let pixels: Vec<_> = (0i32..64 * 48)
        .map(|i| {
            let (x, y) = (i % 64, i / 64);
            if x < 6 {
                [1.0, 0.0, 0.0, 0.0]
            } else if (x - 32).pow(2) + (y - 24).pow(2) < 150 {
                [0.9, 0.2, 0.1, 1.0]
            } else {
                [0.2, 0.45, 0.7, 1.0]
            }
        })
        .collect();
    let subject = recognize_with_options(&pixels, 64, 48, Kind::Subject, classical_options(), &ctx).unwrap();
    let background = recognize_with_options(&pixels, 64, 48, Kind::Background, classical_options(), &ctx).unwrap();
    for (i, (subject, background)) in subject.values.iter().zip(&background.values).enumerate() {
        if i % 64 < 6 {
            assert_eq!((*subject, *background), (0, 0), "transparent RGB is neither subject nor background");
        } else {
            assert_eq!(*subject as u16 + *background as u16, 255, "background complements the same subject shape");
        }
    }
}

#[test]
fn reconstruction_uses_pixel_centers_and_preserves_document_offset() {
    let result = Recognition {
        width: 2,
        height: 1,
        values: vec![0, 255],
        backend: "Test",
        diagnostics: Diagnostics {
            input_size: [2, 1],
            analysis_size: [2, 1],
            output_size: [2, 1],
            model_cache_hit: false,
            result_cache_hit: false,
            fallback_reason: None,
            inference_provider: None,
        },
    };
    let bounds = Rect::new(-4, -3, 4, -1);
    let region = result.region(bounds, &JobCtx::new()).unwrap().unwrap();
    let row = (-4..4).map(|x| (region.at(x, -3) * 255.0).round() as u8).collect::<Vec<_>>();
    assert_eq!(row, vec![0, 0, 32, 96, 159, 223, 255, 255]);
    assert_eq!(region.bbox.y0, -3);
    assert!(result.region(Rect::EMPTY, &JobCtx::new()).is_err());
}

#[test]
fn foreground_model_is_not_a_sky_classifier_v93() {
    let pixels = vec![[0.1, 0.2, 0.8, 1.0]; 16];
    let result = recognize_with_options(&pixels, 4, 4, Kind::Sky, Options { backend: BackendPreference::BiRefNet, ..Default::default() }, &JobCtx::new());
    assert!(result.is_err());
    let ctx = JobCtx::new();
    ctx.cancel();
    assert!(matches!(
        recognize_with_options(&pixels, 4, 4, Kind::Subject, Options { backend: BackendPreference::BiRefNet, ..Default::default() }, &ctx),
        Err(EngineError::Cancelled)
    ));
}

#[test]
fn sky_with_nondefault_options_preserves_classical_meaning_and_reports_it() {
    let pixels = landscape(32, 24);
    let options = Options { sky: segment::sky::SkyParams { threshold: 60.0, softness: 25.0 }, ..Default::default() };
    let result = recognize_with_options(&pixels, 32, 24, Kind::Sky, options, &JobCtx::new()).unwrap();
    assert_eq!(result.backend, "ClassicalSky");
    assert!(result.diagnostics.fallback_reason.as_deref().unwrap().contains("threshold/softness"));
    let forced = Options { backend: BackendPreference::U2Net, ..options };
    assert!(recognize_with_options(&pixels, 32, 24, Kind::Sky, forced, &JobCtx::new()).is_err());
}

#[test]
fn no_sky_keeps_existing_selection_and_undo_history_untouched_at_all_depths() {
    for depth in [8, 16, 32] {
        let mut session = Session::new();
        session.execute("file.new", json!({"width":32,"height":24,"depth":depth})).unwrap();
        session.execute("edit.fill", json!({"color":"#504020"})).unwrap();
        session.execute("select.rect", json!({"x":4,"y":4,"width":12,"height":10})).unwrap();
        let before = session.active().unwrap().doc.clone();
        let result = session.execute("select.sky", json!({"backend":"classical"})).unwrap();
        assert_eq!(result["changed"], false);
        assert_eq!(result["backend"], "ClassicalSky");
        assert_eq!(*before, *session.active().unwrap().doc);
        session.execute("edit.undo", json!({})).unwrap();
        assert!(session.active().unwrap().doc.selection.is_none(), "no-sky request must not add an empty undo step");
    }
}

#[test]
fn command_rejects_invalid_recognition_params_without_document_changes() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":32,"height":24})).unwrap();
    let before = session.active().unwrap().doc.clone();
    for (command, params) in [
        ("select.subject", json!({"backend":"unknown"})),
        ("select.subject", json!({"backend":true})),
        ("select.subject", json!({"mode":"unknown"})),
        ("select.subject", json!({"mode":true})),
        ("select.sky", json!({"threshold":"many"})),
        ("select.sky", json!({"softness":true})),
        ("select.sky", json!({"backend":"u2netp","threshold":60})),
    ] {
        assert!(session.execute(command, params).is_err());
        assert_eq!(*before, *session.active().unwrap().doc);
    }
}

#[test]
fn bounded_document_input_keeps_hidden_rgb_out_of_visible_edge_bins() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":2048,"height":8,"background":"transparent"})).unwrap();
    session
        .edit("analysis pattern", |doc, layer| {
            let surface = doc.layer_mut(layer.unwrap()).unwrap().surface_mut().unwrap();
            for y in 0..8 {
                for x in 0..2048 {
                    surface.write_pixel(x, y, if x % 2 == 0 { &[1.0, 0.0, 0.0, 0.0] } else { &[0.0, 0.0, 1.0, 1.0] });
                }
            }
            Ok(())
        })
        .unwrap();
    let state = session.active().unwrap();
    let (pixels, width, height) = document_input(&state.doc, state.active_layer, false, &JobCtx::new()).unwrap();
    assert_eq!([width, height], [1024, 4]);
    for pixel in pixels {
        assert!(pixel[0] < 0.01, "invisible red must not contaminate visible blue");
        assert!(pixel[2] > 0.99);
        assert!((pixel[3] - 0.5).abs() < 0.001);
    }
}

#[test]
fn model_independent_classical_command_and_shared_service_agree_and_undo() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":64,"height":48,"depth":16})).unwrap();
    session
        .edit("landscape", |doc, layer| {
            let pixels = landscape(64, 48).into_iter().flatten().collect::<Vec<_>>();
            doc.layer_mut(layer.unwrap()).unwrap().surface_mut().unwrap().write_region(Rect::new(0, 0, 64, 48), &pixels);
            Ok(())
        })
        .unwrap();
    let state = session.active().unwrap();
    let (pixels, width, height) = document_input(&state.doc, state.active_layer, true, &JobCtx::new()).unwrap();
    let expected = recognize_with_options(&pixels, width, height, Kind::Sky, classical_options(), &JobCtx::new()).unwrap();
    let output = session.execute("select.sky", json!({"backend":"classical"})).unwrap();
    assert_eq!(output["backend"], expected.backend);
    let selected = session.active().unwrap().doc.selection.as_ref().unwrap();
    for (i, coverage) in expected.values.iter().enumerate() {
        assert_eq!((selected.sample_channel((i % width) as i32, (i / width) as i32, 0) * 255.0).round() as u8, *coverage);
    }
    session.execute("edit.undo", json!({})).unwrap();
    assert!(session.active().unwrap().doc.selection.is_none());
}
