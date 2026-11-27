use super::*;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, LayerMask};

fn fixture(mode: &str, depth: u8) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":128,"height":96,"mode":mode,"depth":depth,"background":"transparent"})).unwrap();
    s.execute("edit.fill", json!({"color":"#2d648a"})).unwrap();
    s.tools.brush = BrushSettings {
        size: 23.,
        opacity: 0.73,
        spacing: 0.13,
        tip: TipShape::Sampled(GrayTile::from_fn(17, 11, |x, y| ((x * 13 + y * 7) % 19) as f32 / 18.)),
        ..Default::default()
    };
    s.tools.foreground = [0.8, 0.3, 0.15, 1.];
    s.tools.background = [0.1, 0.8, 0.4, 1.];
    s
}

fn replica(s: &Session) -> Session {
    Session { docs: s.docs.clone(), active: s.active, tools: s.tools.clone(), journal: s.journal.clone(), ..Default::default() }
}

fn points() -> Value {
    json!([
        [12., 44., 0.4, 3., -4., 8., 0., 0.3],
        [31., 22., 0.7, 8., 2., 14., 10., 0.6],
        [63., 67., 0.9, -2., 4., 23., 20., 0.7],
        [109., 48., 0.5, -5., 6., 36., 30., 0.9]
    ])
}

fn prepare(s: &mut Session, cmd: &str, p: &mut Value) {
    let mut initial = p.clone();
    initial["points"] = json!([p["points"][0]]);
    let mut live = LiveStroke::begin_with(s, cmd, &initial).unwrap();
    let middle = json!({"points":[p["points"][1],p["points"][2]]});
    live.push(&parse_points(&middle, cmd).unwrap()).unwrap();
    p["seed"] = json!(live.seed);
    let before = s.active().unwrap().doc.clone();
    live.prepare(s, p).unwrap(); // Must append the as-yet-unfed final point and its complete tail.
    assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
    assert!(s.prepared_stroke.is_some());
}

fn assert_commit_equivalent(s: &mut Session, cmd: &str, p: Value) {
    let mut replay = replica(s);
    let before = s.active().unwrap().doc.clone();
    let revision = s.active().unwrap().revision;
    let context = format!("{cmd} {:?}/{:?} symmetry={} params={p}", before.mode, before.depth, s.active().unwrap().symmetry_path.is_some());
    let out = s.execute(cmd, p.clone()).unwrap();
    let expected = replay.execute(cmd, p).unwrap();
    assert!(s.prepared_stroke.is_none());
    assert_eq!(out, expected, "damage rectangle: {context}");
    assert!(s.active().unwrap().doc == replay.active().unwrap().doc, "complete document and every native tile byte: {context}");
    assert_eq!(s.journal, replay.journal);
    assert_eq!(s.active().unwrap().revision, revision + 1);
    assert_eq!(s.active().unwrap().last_damage, replay.active().unwrap().last_damage);
    assert_eq!(s.active().unwrap().history.entries(), replay.active().unwrap().history.entries());
    assert!(s.active().unwrap().is_dirty());
    let after = s.active().unwrap().doc.clone();
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc, before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc, after);
}

#[test]
fn prepared_matches_full_replay_for_native_depths_targets_smoothing_and_symmetry() {
    for (mode, depth) in [("rgb", 8), ("rgb", 16), ("rgb", 32), ("grayscale", 16), ("cmyk", 8)] {
        for target in ["pixels", "mask", "alpha", "quickMask", "color"] {
            for command in ["paint.stroke", "paint.pencil"] {
                let mut s = fixture(mode, depth);
                let gray = PixelFormat::new(ColorMode::Grayscale, s.active().unwrap().doc.depth, false);
                let st = s.active_mut().unwrap();
                let id = st.active_layer.unwrap();
                let doc = Arc::make_mut(&mut st.doc);
                doc.layer_mut(id).unwrap().mask = Some(LayerMask { surface: Surface::with_default(gray, &[0.6]), ..LayerMask::reveal_all() });
                doc.channels.push(AlphaChannel::new("Test Alpha", Surface::with_default(gray, &[0.4])));
                doc.quick_mask = Some(AlphaChannel::new("Quick Mask", Surface::with_default(gray, &[0.3])));
                let mut selection = Surface::new(gray);
                selection.fill_rect(Rect::new(3, 3, 115, 90), &[0.8]);
                doc.selection = Some(selection);
                if target == "color" {
                    st.channel_view.target = crate::channel_cmds::ChannelTarget::Color(0);
                }
                let target = match target {
                    "alpha" => json!({"channel":0}),
                    "mask" => json!("mask"),
                    "quickMask" => json!("quickMask"),
                    _ => json!("pixels"),
                };
                let mut p = json!({"points":points(),"target":target,"zoom":0.4,"smoothing":0.3,"erase":command=="paint.stroke"});
                prepare(&mut s, command, &mut p);
                assert_commit_equivalent(&mut s, command, p);
            }
        }
    }
    for mirror in [false, true] {
        let mut s = fixture("rgb", 16);
        s.active_mut().unwrap().symmetry_path =
            Some(crate::symmetry_cmds::SymmetryAxis { source: "Test axis".into(), segments: vec![[[64., 0.], [64., 96.]]] });
        let pts = if mirror { points() } else { json!([[64., 14., 0.4], [64., 28., 0.7], [64., 42., 0.9], [64., 70., 0.5]]) };
        let mut p = json!({"points":pts,"target":"pixels","zoom":1.7,"smoothing":0.4,"mode":"multiply"});
        prepare(&mut s, "paint.stroke", &mut p);
        assert_commit_equivalent(&mut s, "paint.stroke", p);
    }
}

#[test]
fn prepared_pencil_auto_erase_and_locked_background_keep_history_and_pixels() {
    for command in ["paint.stroke", "paint.pencil"] {
        let mut s = fixture("rgb", 8);
        let id = s.active().unwrap().active_layer.unwrap();
        Arc::make_mut(&mut s.active_mut().unwrap().doc).layer_mut(id).unwrap().locks.transparency = true;
        s.tools.foreground = [45. / 255., 100. / 255., 138. / 255., 1.];
        let mut p = json!({"points":points(),"target":"pixels","erase":command=="paint.stroke","autoErase":true});
        prepare(&mut s, command, &mut p);
        assert_commit_equivalent(&mut s, command, p);
    }
}

#[test]
fn changed_source_settings_target_points_or_command_discard_prepared_pixels() {
    for change in 0..9 {
        let mut s = fixture("rgb", 8);
        let mut p = json!({"points":points(),"target":"pixels"});
        prepare(&mut s, "paint.stroke", &mut p);
        let mut command = "paint.stroke";
        match change {
            0 => p["points"][2][0] = json!(66.),
            1 => p["brush"] = json!({"size":9.}),
            2 => s.tools.foreground = [0., 1., 0., 1.],
            3 => s.tools.background = [0., 0., 0., 1.],
            4 => {
                s.execute("layer.new.layer", json!({})).unwrap();
            }
            5 => {
                let doc = s.active().unwrap().doc.clone();
                s.active_mut().unwrap().doc = Arc::new((*doc).clone());
            }
            6 => s.active_mut().unwrap().revision += 1,
            7 => command = "paint.pencil",
            _ => {
                s.active_mut().unwrap().symmetry_path =
                    Some(crate::symmetry_cmds::SymmetryAxis { source: "Changed".into(), segments: vec![[[64., 0.], [64., 96.]]] })
            }
        }
        if let Some(pending) = s.prepared_stroke.as_ref() {
            assert!(!pending.guard.matches(&s, command, &p) || parse_points(&p, command).unwrap() != pending.points);
        } else {
            assert_eq!(change, 4, "only the intervening non-paint command clears before dispatch");
        }
        assert_commit_equivalent(&mut s, command, p);
    }
}

#[test]
fn dispatch_rejection_intervening_commands_and_close_release_prepared_pixels() {
    for case in 0..7 {
        let mut s = fixture("rgb", 8);
        let mut p = json!({"points":points(),"target":"pixels"});
        prepare(&mut s, "paint.stroke", &mut p);
        let source = Arc::downgrade(&s.prepared_stroke.as_ref().unwrap().guard.source);
        let mut command = "paint.stroke";
        match case {
            0 => p = json!([]),
            1 => command = "missing.command",
            2 => command = "brush.get",
            3 => s.authorize = Some(|_, _| Err(EngineError::Other("test denied".into()))),
            4 => {
                let id = s.active().unwrap().active_layer.unwrap();
                Arc::make_mut(&mut s.active_mut().unwrap().doc).layer_mut(id).unwrap().locks.all = true;
            }
            5 => p["points"] = json!([]),
            _ => command = "file.close",
        }
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.entries();
        let journal = s.journal.clone();
        let retained = source.strong_count();
        let result = s.execute(command, p);
        assert_eq!(result.is_ok(), matches!(case, 2 | 6), "case={case}");
        assert!(s.prepared_stroke.is_none(), "case={case}");
        assert!(source.strong_count() < retained, "guard must release its source snapshot: case={case}");
        if case == 6 {
            assert!(s.active().is_none());
        } else {
            assert!(Arc::ptr_eq(&s.active().unwrap().doc, &before));
            assert_eq!(s.active().unwrap().history.entries(), history);
            assert_eq!(s.journal, journal);
        }
    }
    let s = fixture("rgb", 8);
    let initial = json!({"points":[[12.,44.,0.4]],"target":"pixels"});
    let source = Arc::downgrade(&s.active().unwrap().doc);
    let live = LiveStroke::begin(&s, &initial).unwrap();
    assert_eq!(source.strong_count(), 2);
    drop(live);
    assert_eq!(source.strong_count(), 1);
    assert!(s.prepared_stroke.is_none());
}

#[test]
fn shrinking_symmetry_tail_does_not_enlarge_final_command_damage() {
    let mut s = fixture("rgb", 16);
    s.active_mut().unwrap().symmetry_path = Some(crate::symmetry_cmds::SymmetryAxis { source: "Test axis".into(), segments: vec![[[64., 0.], [64., 96.]]] });
    let mut p = json!({"points":points(),"target":"pixels","zoom":1.7,"smoothing":0.4,"mode":"multiply"});
    let mut initial = p.clone();
    initial["points"] = json!([p["points"][0]]);
    let mut live = LiveStroke::begin(&s, &initial).unwrap();
    live.push(&parse_points(&json!({"points":[p["points"][1],p["points"][2]]}), "paint.stroke").unwrap()).unwrap();
    live.push(&parse_points(&json!({"points":[p["points"][3]]}), "paint.stroke").unwrap()).unwrap();
    assert!(live.bounds().y1 > live.commit_damage.y1, "preview restoration and final stroke bounds are separate");
    p["seed"] = json!(live.seed);
    live.prepare(&mut s, &p).unwrap();
    assert_commit_equivalent(&mut s, "paint.stroke", p);
}

fn sparse_allocation_cases(report: bool) {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for kind in ["symmetric-brush", "symmetric-eraser", "symmetric-mask", "shrinking-smoothing"] {
            let mut doc = Document::new("Sparse allocation fixture", photocraft_geom::Size::new(4096, 2048), ColorMode::Rgb, depth);
            let mut layer = photocraft_doc::Layer::raster("Content and off-canvas pixels", doc.pixel_format());
            let surface = layer.surface_mut().unwrap();
            surface.fill_rect(Rect::new(32, 32, 72, 72), &[0.2, 0.4, 0.6, 1.]);
            surface.fill_rect(Rect::new(-500, 2300, -480, 2320), &[0.7, 0.3, 0.1, 0.8]);
            let mask = kind == "symmetric-mask";
            if mask {
                let gray = PixelFormat::new(ColorMode::Grayscale, depth, false);
                let mut surface = Surface::with_default(gray, &[0.6]);
                surface.fill_rect(Rect::new(32, 32, 72, 72), &[0.3]);
                surface.fill_rect(Rect::new(-500, 2300, -480, 2320), &[0.4]);
                layer.mask = Some(LayerMask { surface, ..LayerMask::reveal_all() });
            }
            doc.layers.push(layer);
            let mut s = Session::new();
            s.add_document(doc, None);
            s.tools.brush = BrushSettings {
                size: 64.,
                spacing: 0.2,
                opacity: 0.73,
                tip: TipShape::Sampled(GrayTile::from_fn(31, 19, |x, y| ((x * 7 + y * 11) % 23) as f32 / 22.)),
                ..Default::default()
            };
            s.tools.foreground = [0.8, 0.2, 0.1, 1.];
            let symmetric = kind != "shrinking-smoothing";
            if symmetric {
                s.active_mut().unwrap().symmetry_path =
                    Some(crate::symmetry_cmds::SymmetryAxis { source: "Wide symmetric gap".into(), segments: vec![[[2048., 0.], [2048., 2048.]]] });
            }
            let points = if symmetric {
                json!([[320., 300., 0.4], [360., 700., 0.8], [400., 330., 0.5], [480., 340., 1.]])
            } else {
                json!([[320., 300., 0.4], [340., 1800., 0.8], [380., 300., 0.5], [1200., 350., 1.]])
            };
            let mut p = json!({"points":points,"seed":18674,"target":if mask {"mask"}else{"pixels"},"erase":kind=="symmetric-eraser","smoothing":if symmetric {0.3}else{0.85}});
            let all = parse_points(&p, "paint.stroke").unwrap();
            let mut initial = p.clone();
            initial["points"] = json!([p["points"][0]]);
            let mut live = LiveStroke::begin(&s, &initial).unwrap();
            for point in all.iter().skip(1) {
                live.push(&[*point]).unwrap();
            }
            p["seed"] = json!(live.seed);
            let mut live_doc = (*live.doc).clone();
            let direct_copy = crate::channel_cmds::target_surface(&mut live_doc, live.layer, &p).unwrap().0.clone();
            let mut replay = replica(&s);
            replay.execute("paint.stroke", p.clone()).unwrap();
            let mut legacy_doc = (*replay.active().unwrap().doc).clone();
            let legacy = crate::channel_cmds::target_surface(&mut legacy_doc, live.layer, &p).unwrap().0.clone();
            assert!(direct_copy.tile_count() > legacy.tile_count(), "fixture must exercise restoration-only tiles: {depth:?} {kind}");
            live.prepare(&mut s, &p).unwrap();
            let prepared = &s.prepared_stroke.as_ref().unwrap().surface;
            assert!(prepared == &legacy, "exact native sparse allocation and bytes: {depth:?} {kind}");
            if report {
                println!(
                    "{}",
                    json!({"depth":format!("{depth:?}"),"case":kind,
                    "previewDirectCopyTileCount":direct_copy.tile_count(),"legacyTileCount":legacy.tile_count(),
                    "preparedTileCount":prepared.tile_count(),"tileBytes":256*256*prepared.format().bytes_per_pixel(),
                    "sameNativeTileCoordinatesAndBytes":true,"sourceOffCanvasTilePreserved":true})
                );
            }
            assert_commit_equivalent(&mut s, "paint.stroke", p);
        }
    }
}

#[test]
fn sparse_symmetry_and_shrinking_tail_keep_legacy_exact_tile_allocation() {
    sparse_allocation_cases(false);
}

#[test]
#[ignore = "actual direct-copy preview / legacy replay / canonical prepared allocation report"]
fn sparse_prepared_allocation_report() {
    sparse_allocation_cases(true);
}

#[test]
fn prepare_rejects_changed_prefix_source_and_invalid_points_without_editing() {
    for case in 0..4 {
        let mut s = fixture("rgb", 8);
        let initial = json!({"points":[[12.,44.,0.4]],"target":"pixels"});
        let live = LiveStroke::begin(&s, &initial).unwrap();
        let mut p = initial.clone();
        p["seed"] = json!(live.seed);
        match case {
            0 => p["points"][0][0] = json!(13.),
            1 => {
                s.execute("layer.new.layer", json!({})).unwrap();
            }
            2 => p["points"] = json!([]),
            _ => p["points"] = json!([[12., 44., 0.4], [1e12, 0., 1.]]),
        }
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        assert!(live.prepare(&mut s, &p).is_err());
        assert!(s.prepared_stroke.is_none());
        assert!(Arc::ptr_eq(&before, &s.active().unwrap().doc));
        assert_eq!(s.active().unwrap().revision, revision);
    }
}

#[test]
#[ignore = "release content-bearing 24MP live-release comparison; reports preparation and command separately"]
fn prepared_release_24mp_perf() {
    let mut doc = Document::new("24MP content", photocraft_geom::Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8);
    let mut layer = photocraft_doc::Layer::raster("Dense raster", doc.pixel_format());
    layer.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.2, 0.4, 0.6, 1.]);
    doc.layers.push(layer);
    for i in 0..20 {
        let mut layer = photocraft_doc::Layer::raster(format!("Sparse layer {i}"), doc.pixel_format());
        layer.surface_mut().unwrap().fill_rect(Rect::new(i * 257, i * 127, i * 257 + 70, i * 127 + 90), &[0.8, 0.4, 0.2, 0.6]);
        doc.layers.push(layer);
    }
    let points: Vec<_> = (0..120).map(|i| [1200. + f64::from(i) * 25., 1800. + (f64::from(i) * 0.1).sin() * 400., 0.4 + f64::from(i % 7) * 0.08]).collect();
    let mut s = Session::new();
    s.add_document(doc, None);
    s.tools.brush = BrushSettings {
        size: 120.,
        spacing: 0.12,
        tip: TipShape::Sampled(GrayTile::from_fn(97, 73, |x, y| ((x * 7 + y * 17) % 37) as f32 / 36.)),
        ..Default::default()
    };
    for iteration in 0..5 {
        let mut normal = replica(&s);
        let mut fast = replica(&s);
        let mut p = json!({"points":[points[0]],"target":"pixels","seed":8326});
        let mut live = LiveStroke::begin(&fast, &p).unwrap();
        for chunk in points[1..].chunks(4) {
            live.push(&chunk.iter().map(|p| StrokePoint::new(p[0], p[1], p[2] as f32)).collect::<Vec<_>>()).unwrap();
        }
        p["points"] = json!(points);
        let start = std::time::Instant::now();
        live.prepare(&mut fast, &p).unwrap();
        let prepare_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = std::time::Instant::now();
        normal.execute("paint.stroke", p.clone()).unwrap();
        let replay_ms = start.elapsed().as_secs_f64() * 1000.;
        let start = std::time::Instant::now();
        fast.execute("paint.stroke", p).unwrap();
        let commit_ms = start.elapsed().as_secs_f64() * 1000.;
        assert!(fast.active().unwrap().doc == normal.active().unwrap().doc, "native tile bytes differ: iteration={iteration}");
        println!("iteration={iteration} prepare_ms={prepare_ms:0.4} legacy_replay_ms={replay_ms:0.4} prepared_commit_ms={commit_ms:0.4}");
    }
}
