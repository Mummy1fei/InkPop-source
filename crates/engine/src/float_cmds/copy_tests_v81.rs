//! Selected-pixel copy invariants, independent of keyboard or pointer event routing.
use photocraft_color::{ColorMode, SampleType};
use photocraft_geom::Rect;
use serde_json::json;

use super::*;

fn fixture(format: PixelFormat, soft: bool) -> (Session, LayerId) {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":32,"height":24,"background":"transparent"})).unwrap();
    let layer = session.active().unwrap().active_layer.unwrap();
    session
        .edit("Copy fixture", |doc, _| {
            doc.mode = format.mode;
            doc.depth = format.sample;
            let mut pixels = vec![0.25; format.channels()];
            if format.alpha {
                *pixels.last_mut().unwrap() = 0.6;
            }
            let mut surface = Surface::new(format);
            surface.fill_rect(Rect::new(4, 4, 12, 12), &pixels);
            *doc.layer_mut(layer).unwrap().surface_mut().unwrap() = surface;
            let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
            selection.fill_rect(Rect::new(4, 4, 12, 12), &[if soft { 0.5 } else { 1.0 }]);
            selection.fill_rect(Rect::new(6, 6, 8, 8), &[0.0]);
            doc.selection = Some(selection);
            Ok(())
        })
        .unwrap();
    (session, layer)
}

fn native(doc: &Document, layer: LayerId, region: Rect) -> Vec<u32> {
    doc.layer(layer).unwrap().surface().unwrap().read_region(region).iter().map(|value| value.to_bits()).collect()
}

fn formats() -> impl Iterator<Item = PixelFormat> {
    [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab]
        .into_iter()
        .flat_map(|mode| SampleType::ALL.into_iter().flat_map(move |sample| [false, true].into_iter().map(move |alpha| PixelFormat::new(mode, sample, alpha))))
}

#[test]
fn selected_pixel_copy_keeps_source_and_layer_count_and_commits_as_one_undoable_step_v81() {
    let (mut session, layer) = fixture(PixelFormat::RGBA16, false);
    let before = session.active().unwrap().doc.clone();
    let history = session.active().unwrap().history.past_len();
    let revision = session.active().unwrap().revision;
    session.execute("select.float", json!({"copy":true,"dx":8,"dy":0})).unwrap();
    let parts = floating(session.active().unwrap()).unwrap().parts.clone();
    session.execute("select.float", json!({"dx":4,"dy":2})).unwrap();
    let state = session.active().unwrap();
    assert!(Arc::ptr_eq(&parts, &floating(state).unwrap().parts), "every drag update uses the original captured piece");
    assert_eq!(state.doc, before);
    assert_eq!(state.revision, revision);
    assert_eq!(state.history.past_len(), history);
    let shown = displayed(state, (0, 0)).unwrap();
    assert_eq!(shown.layers.len(), before.layers.len(), "the copy belongs to the existing pixel layer");
    assert_eq!(native(&shown, layer, Rect::new(4, 4, 12, 12)), native(&before, layer, Rect::new(4, 4, 12, 12)));
    assert_eq!(native(&shown, layer, Rect::new(16, 6, 18, 8)), native(&before, layer, Rect::new(4, 4, 6, 6)));
    assert_eq!(shown.layer(layer).unwrap().surface().unwrap().rgba(19, 9)[3], 0.0, "the unselected hole is not copied");
    session.execute("select.drop", json!({})).unwrap();
    let state = session.active().unwrap();
    assert!(floating(state).is_none());
    assert_eq!(state.history.past_len(), history + 1);
    assert_eq!(state.history.undo_label(), Some("Copy Selected Pixels"));
    assert_eq!(state.doc.selection.as_ref().unwrap().content_bounds(), Rect::new(16, 6, 24, 14));
    assert_eq!(native(&state.doc, layer, Rect::new(0, 0, 32, 24)), native(&shown, layer, Rect::new(0, 0, 32, 24)));
    let committed = state.doc.clone();
    assert!(session.undo());
    assert_eq!(session.active().unwrap().doc, before, "one undo restores both pixels and selection");
    assert!(session.redo());
    assert_eq!(session.active().unwrap().doc, committed);
}

#[test]
fn copied_soft_selection_preserves_native_source_and_source_over_overlap_in_all_formats_v81() {
    for format in formats() {
        let (mut session, layer) = fixture(format, true);
        let before = session.active().unwrap().doc.clone();
        let original = before.layer(layer).unwrap().surface().unwrap();
        let source = original.read_region(Rect::new(5, 5, 6, 6));
        let n = format.mode.color_channels();
        let alpha = if format.alpha { source[n] } else { 1.0 };
        let tolerance = match format.sample {
            SampleType::U8 => 2.0 / 255.0,
            SampleType::U16 => 2.0 / 65535.0,
            SampleType::F32 => 1e-6,
        };
        session.execute("select.float", json!({"copy":true,"dx":4})).unwrap();
        let shown = displayed(session.active().unwrap(), (0, 0)).unwrap();
        let out = shown.layer(layer).unwrap().surface().unwrap();
        assert_eq!(out.format(), PixelFormat::new(format.mode, format.sample, true));
        let untouched = out.read_region(Rect::new(4, 4, 8, 6));
        let expected = if format.alpha { original.clone() } else { original.convert(out.format()) };
        assert_eq!(
            untouched.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            native_surface(&expected, Rect::new(4, 4, 8, 6)),
            "{format:?}: untouched source samples are exact"
        );
        let overlap = out.read_region(Rect::new(9, 5, 10, 6));
        let destination = out.read_region(Rect::new(13, 5, 14, 6));
        let destination_before = expected.read_region(Rect::new(13, 5, 14, 6));
        let copied_alpha = alpha * 0.5;
        let destination_alpha = copied_alpha + destination_before[n] * (1.0 - copied_alpha);
        assert!((overlap[n] - (alpha * 0.5 + alpha * (1.0 - alpha * 0.5))).abs() <= tolerance, "{format:?}: source-over overlap");
        assert!((destination[n] - destination_alpha).abs() <= tolerance, "{format:?}: selection coverage multiplies source alpha once");
        for channel in 0..n {
            assert!((overlap[channel] - source[channel]).abs() <= tolerance, "{format:?}: overlap colour {channel}");
            let expected_color =
                (source[channel] * copied_alpha + destination_before[channel] * destination_before[n] * (1.0 - copied_alpha)) / destination_alpha;
            assert!((destination[channel] - expected_color).abs() <= tolerance, "{format:?}: destination colour {channel}");
        }
        session.execute("select.drop", json!({})).unwrap();
        assert_eq!(
            native(&session.active().unwrap().doc, layer, Rect::new(0, 0, 32, 24)),
            native(&shown, layer, Rect::new(0, 0, 32, 24)),
            "{format:?}: drop equals preview"
        );
    }
}

fn native_surface(surface: &Surface, region: Rect) -> Vec<u32> {
    surface.read_region(region).iter().map(|value| value.to_bits()).collect()
}

#[test]
fn zero_displacement_copy_preserves_document_format_pixels_and_history_v81() {
    for format in formats() {
        let (mut session, _) = fixture(format, true);
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.past_len();
        let revision = session.active().unwrap().revision;
        session.execute("select.float", json!({"copy":true})).unwrap();
        assert_eq!(displayed(session.active().unwrap(), (0, 0)).unwrap(), *before, "{format:?}: no opacity build-up on press");
        session.execute("select.float", json!({"dx":4})).unwrap();
        session.execute("select.float", json!({"dx":-4})).unwrap();
        session.execute("select.drop", json!({})).unwrap();
        let state = session.active().unwrap();
        assert_eq!(state.doc, before, "{format:?}: dragging back does not commit a duplicate");
        assert_eq!(state.history.past_len(), history);
        assert_eq!(state.revision, revision);
    }
}

#[test]
fn copy_mode_and_bad_parameter_errors_do_not_mutate_captured_piece_or_document_v81() {
    for bad in [Value::Null, json!(1), json!("true"), json!([]), json!({})] {
        let (mut session, _) = fixture(PixelFormat::RGBA8, false);
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.past_len();
        let journal = session.journal.len();
        assert!(matches!(session.execute("select.float", json!({"copy":bad,"dx":5})), Err(EngineError::BadParams { .. })));
        assert!(session.active().unwrap().floating.is_none());
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().history.past_len(), history);
        assert_eq!(session.journal.len(), journal);
    }
    for copy in [false, true] {
        let (mut session, _) = fixture(PixelFormat::RGBA8, false);
        session.execute("select.float", json!({"copy":copy,"dx":5})).unwrap();
        let before = session.active().unwrap().doc.clone();
        let piece = floating(session.active().unwrap()).unwrap().clone();
        let journal = session.journal.len();
        assert!(matches!(session.execute("select.float", json!({"copy":!copy,"dx":50})), Err(EngineError::BadParams { .. })));
        let state = session.active().unwrap();
        assert_eq!(state.doc, before);
        assert_eq!(floating(state).unwrap().offset, (5, 0));
        assert!(Arc::ptr_eq(&piece.parts, &floating(state).unwrap().parts));
        assert_eq!(session.journal.len(), journal);
    }
}

#[test]
fn canceling_or_auto_dropping_a_copy_preserves_existing_floating_command_semantics_v81() {
    let (mut session, layer) = fixture(PixelFormat::RGBA8, false);
    let before = session.active().unwrap().doc.clone();
    let history = session.active().unwrap().history.past_len();
    session.execute("select.float", json!({"copy":true,"dx":12})).unwrap();
    assert_eq!(session.execute("edit.undo", json!({})).unwrap(), json!({"floating":"returned"}));
    assert!(session.active().unwrap().floating.is_none());
    assert_eq!(session.active().unwrap().doc, before);
    assert_eq!(session.active().unwrap().history.past_len(), history);
    session.execute("select.float", json!({"copy":true,"dx":12})).unwrap();
    let shown = displayed(session.active().unwrap(), (0, 0)).unwrap();
    session.execute("select.deselect", json!({})).unwrap();
    let state = session.active().unwrap();
    assert!(state.floating.is_none() && state.doc.selection.is_none());
    assert_eq!(native(&state.doc, layer, Rect::new(0, 0, 32, 24)), native(&shown, layer, Rect::new(0, 0, 32, 24)));
}

#[test]
fn copy_obeys_each_existing_pixel_move_lock_and_rejects_empty_source_v81() {
    for key in ["all", "position", "pixels", "transparency"] {
        let (mut session, layer) = fixture(PixelFormat::RGBA8, false);
        session
            .edit("Lock fixture", |doc, _| {
                let locks = &mut doc.layer_mut(layer).unwrap().locks;
                match key {
                    "all" => locks.all = true,
                    "position" => locks.position = true,
                    "pixels" => locks.pixels = true,
                    _ => locks.transparency = true,
                }
                Ok(())
            })
            .unwrap();
        let before = session.active().unwrap().doc.clone();
        let history = session.active().unwrap().history.past_len();
        assert!(!session.is_enabled("select.float"));
        assert!(session.execute("select.float", json!({"copy":true,"dx":12})).is_err(), "{key}");
        assert_eq!(session.active().unwrap().doc, before);
        assert_eq!(session.active().unwrap().history.past_len(), history);
    }
    let (mut session, layer) = fixture(PixelFormat::RGBA8, false);
    session
        .edit("Empty fixture", |doc, _| {
            doc.layer_mut(layer).unwrap().surface_mut().unwrap().fill_rect(Rect::new(4, 4, 12, 12), &[0.25, 0.25, 0.25, 0.0]);
            Ok(())
        })
        .unwrap();
    let before = session.active().unwrap().doc.clone();
    assert!(session.execute("select.float", json!({"copy":true,"dx":12})).is_err());
    assert!(session.active().unwrap().floating.is_none());
    assert_eq!(session.active().unwrap().doc, before);
}

#[test]
fn first_float_journal_entry_records_copy_mode_for_replay_v81() {
    let (mut session, _) = fixture(PixelFormat::RGBA8, false);
    let first = session.journal.len();
    session.execute("select.float", json!({"copy":true,"dx":8})).unwrap();
    session.execute("select.float", json!({"dx":4})).unwrap();
    session.execute("select.drop", json!({})).unwrap();
    let journal = session.journal[first..].to_vec();
    assert_eq!(journal.iter().map(|item| item.0.as_str()).collect::<Vec<_>>(), ["select.float", "select.float", "select.drop"]);
    assert_eq!(journal[0].1["copy"], true);
    let (mut replayed, _) = fixture(PixelFormat::RGBA8, false);
    for (command, params) in journal {
        replayed.execute(&command, params).unwrap();
    }
    let a = &session.active().unwrap().doc;
    let b = &replayed.active().unwrap().doc;
    assert_eq!(a.layers.len(), b.layers.len());
    let layer_a = session.active().unwrap().active_layer.unwrap();
    let layer_b = replayed.active().unwrap().active_layer.unwrap();
    assert_eq!(native(a, layer_a, Rect::new(0, 0, 32, 24)), native(b, layer_b, Rect::new(0, 0, 32, 24)));
    assert_eq!(a.selection, b.selection);
}

/// Same synthetic 24MP surface and selected area as the existing cut benchmark.
/// This measures capture/preview CPU work, not physical input-to-screen latency.
#[test]
#[ignore]
fn copy_float_drag_bench_v81() {
    let mut session = Session::new();
    session.execute("file.new", json!({"width":6000,"height":4000,"background":"transparent"})).unwrap();
    session.execute("layer.new.layer", json!({})).unwrap();
    session
        .edit("Copy benchmark fixture", |doc, layer| {
            doc.layer_mut(layer.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 6000, 4000), &[0.2, 0.4, 0.6, 1.0]);
            Ok(())
        })
        .unwrap();
    session.execute("select.rect", json!({"x":1000,"y":1000,"width":500,"height":400})).unwrap();
    let history = session.active().unwrap().history.past_len();
    let revision = session.active().unwrap().revision;
    let start = std::time::Instant::now();
    session.execute("select.float", json!({"copy":true})).unwrap();
    let capture = start.elapsed();
    let start = std::time::Instant::now();
    for index in 0..20 {
        displayed(session.active().unwrap(), (index * 7, index * 3)).unwrap();
    }
    eprintln!("6000×4000 layer, 500×400 piece COPY: capture {capture:?}, then {:?} per pointer move", start.elapsed() / 20);
    assert_eq!(session.active().unwrap().history.past_len(), history);
    assert_eq!(session.active().unwrap().revision, revision);
}
