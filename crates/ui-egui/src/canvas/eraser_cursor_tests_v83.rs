//! Regular Eraser retains the painting preset; area retouch tools remain circular.
use super::*;
use photocraft_engine::paint::{BrushSettings, GrayTile, TipShape};

fn custom_tip() -> BrushSettings {
    let mut b = BrushSettings {
        size: 48.0,
        hardness: 1.0,
        spacing: 0.1,
        roundness: 0.7,
        angle: 23.0,
        tip: TipShape::Sampled(GrayTile::from_fn(17, 17, |x, y| if (3..14).contains(&x) && (6..11).contains(&y) { 1.0 } else { 0.0 })),
        pressure_size: true,
        pressure_opacity: false,
        ..Default::default()
    };
    b.smoothing.amount = 0.0;
    b
}
fn prepare(h: &mut CanvasHarness) {
    h.app.ui.tool = Tool::Eraser;
    h.app.sync_tool_brush();
    h.app.run("tools.setBrush", json!({"brush":custom_tip(),"summary":true})).unwrap();
}
fn cursor(h: &mut CanvasHarness) -> egui::FullOutput {
    let p = h.doc_point(120.0, 80.0);
    let mut warmup = h.hover(ViewportId::ROOT, Modifiers::NONE, p);
    warmup.textures_delta.clear();
    let mut output = h.hover(ViewportId::ROOT, Modifiers::NONE, p);
    output.textures_delta.clear();
    output
}
#[test]
fn eraser_native_outline_matches_selected_tip_across_dpi_views_and_preferences_v83() {
    for ppp in [1.0, 1.25, 2.0] {
        for (rotation, flip) in [(0.0, false), (37.0, true)] {
            for normal in [true, false] {
                let mut h = CanvasHarness::new(320, 200, true);
                prepare(&mut h);
                h.native_pixels_per_point = ppp;
                h.view.rotation = rotation;
                h.app.ui.view.flip_horizontal = flip;
                h.app.run("prefs.set", json!({"path":"cursors.painting","value":if normal {"normalTip"} else {"fullSizeTip"}})).unwrap();
                let before = h.app.session.tools.brush.clone();
                let output = cursor(&mut h);
                let actual = output.platform_output.cursor_image.unwrap();
                let ctx = egui::Context::default();
                PhotocraftApp::enable_native_tool_cursors(&ctx);
                let mut input = egui::RawInput::default();
                input.viewports.entry(ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
                let xf = ViewXform { rect: h.rect, center: h.view.center, zoom: h.view.zoom, rotation, flip };
                let mut expected_output = ctx.run_ui(input, |ui| assert!(crate::native_cursor::brush(ui.ctx(), &before, &xf, normal, None)));
                expected_output.textures_delta.clear();
                let expected = expected_output.platform_output.cursor_image.unwrap();
                assert_eq!(actual, expected, "Eraser must use the full imported brush cursor path");
                assert_eq!(h.app.session.tools.brush, before);
            }
        }
    }
}
#[test]
fn eraser_software_fallback_keeps_sampled_paths_instead_of_area_circle_v83() {
    let mut h = CanvasHarness::new(320, 200, false);
    prepare(&mut h);
    let p = h.doc_point(120.0, 80.0);
    let out = cursor(&mut h);
    assert!(out.platform_output.cursor_image.is_none());
    assert!(out.shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Path(path) if path.points.iter().any(|q|q.distance(p)<40.0))));
    assert!(!out.shapes.iter().any(|s| matches!(&s.shape,egui::Shape::Circle(circle) if circle.center==p)));
}
#[test]
fn eraser_actual_sampled_footprint_and_pressure_survive_retouch_switch_and_undo_v83() {
    let mut h = CanvasHarness::new(320, 200, true);
    prepare(&mut h);
    h.app.run("edit.fill", json!({"color":"#5588aa"})).unwrap();
    let id = h.app.session.active().unwrap().active_layer.unwrap();
    let before = h.app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap().clone();
    let settings = h.app.session.tools.brush.clone();
    h.app.ui.tool = Tool::SpotHealing;
    h.app.sync_tool_brush();
    h.app.session.tools.brush.size = 19.0;
    h.app.ui.tool = Tool::Eraser;
    h.app.sync_tool_brush();
    assert_eq!(h.app.session.tools.brush, settings);
    let mut counts = Vec::new();
    for (x, pressure) in [(80.0, 0.25), (220.0, 1.0)] {
        tool_event(&mut h.app, ToolEvent::Down { x, y: 100.0, pressure }, Modifiers::NONE);
        tool_event(&mut h.app, ToolEvent::Up { x, y: 100.0 }, Modifiers::NONE);
        let st = h.app.session.active().unwrap();
        let surface = st.doc.layer(id).unwrap().surface().unwrap();
        let count =
            ((x as i32 - 32)..(x as i32 + 32)).flat_map(|xx| (68..132).map(move |yy| (xx, yy))).filter(|(xx, yy)| surface.rgba(*xx, *yy)[3] < 0.99).count();
        counts.push(count);
        assert_eq!(surface.rgba(x as i32, 120)[3], 1.0, "thin sampled eraser does not erase the whole nominal circle");
        let (_, params) = h.app.session.journal.iter().rev().find(|(cmd, _)| cmd == "paint.stroke").unwrap();
        assert_eq!(params["erase"], true);
        assert_eq!(params["points"][0][2], json!(pressure as f64));
    }
    assert!(counts[0] > 0 && counts[1] > counts[0] * 4, "pen pressure changes real erased area: {counts:?}");
    h.app.run("edit.undo", json!({})).unwrap();
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.active().unwrap().doc.layer(id).unwrap().surface().unwrap(), &before);
    assert_eq!(h.app.session.tools.brush, settings, "drawing never overwrites the eraser preset");
}
#[test]
#[ignore = "writes synthetic actual cursor bitmaps and preset for root native QA"]
fn export_eraser_cursor_evidence_v83() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v83/eraser-cursors");
    std::fs::create_dir(&root).unwrap();
    let mut records = Vec::new();
    for (name, tool, angle) in [("eraser-23", Tool::Eraser, 23.0), ("eraser-91", Tool::Eraser, 91.0), ("healing-circle", Tool::SpotHealing, 23.0)] {
        let mut h = CanvasHarness::new(320, 200, true);
        prepare(&mut h);
        h.app.ui.tool = tool;
        h.app.session.tools.brush.size = 160.0;
        h.app.session.tools.brush.angle = angle;
        let im = cursor(&mut h).platform_output.cursor_image.unwrap();
        let mut pix = tiny_skia::Pixmap::new(u32::from(im.size[0]), u32::from(im.size[1])).unwrap();
        for (src, dst) in im.rgba.as_chunks::<4>().0.iter().zip(pix.data_mut().as_chunks_mut::<4>().0) {
            for (d, s) in dst.iter_mut().zip(src).take(3) {
                *d = ((u16::from(*s) * u16::from(src[3]) + 127) / 255) as u8;
            }
            dst[3] = src[3];
        }
        pix.save_png(root.join(format!("{name}.png"))).unwrap();
        records.push(json!({"path":format!("evidence/v83/eraser-cursors/{name}.png"),"scope":"actual shared-canvas native cursor bitmap, synthetic preset","size":im.size,"hotspot":im.hotspot}));
    }
    std::fs::write(root.join("synthetic-brush.json"), serde_json::to_vec_pretty(&custom_tip()).unwrap()).unwrap();
    std::fs::write(root.join("manifest.json"), serde_json::to_vec_pretty(&records).unwrap()).unwrap();
}

#[test]
#[ignore = "release warm 24MP Eraser cursor CPU work only; not hardware latency"]
#[allow(clippy::assertions_on_constants)] // Keep debug builds compilable while refusing misleading benchmark results.
fn benchmark_eraser_cursor_cpu_24mp_v83() {
    assert!(!cfg!(debug_assertions));
    let mut records = Vec::new();
    for native in [false, true] {
        let mut h = CanvasHarness::new(6000, 4000, native);
        prepare(&mut h);
        h.rect = Rect::from_min_size(pos2(32.0, 32.0), vec2(900.0, 600.0));
        h.view.zoom = 0.15;
        h.app.session.tools.brush.size = 160.0;
        let mut times = Vec::new();
        for i in 0..280 {
            let p = pos2(180.0 + (i % 96) as f32 * 4.0, 150.0 + (i % 63) as f32 * 3.0);
            let start = std::time::Instant::now();
            let mut out = h.hover(ViewportId::ROOT, Modifiers::NONE, p);
            if i >= 40 {
                times.push(start.elapsed().as_secs_f64() * 1000.0);
                out.textures_delta.clear();
                assert_eq!(out.platform_output.cursor_image.is_some(), native, "measured warm frame {i}");
            } else {
                out.textures_delta.clear();
            }
        }
        times.sort_by(f64::total_cmp);
        records.push(json!({"native":native,"samples":times.len(),"cpuFrameMsP50":times[times.len()/2],"cpuFrameMsP95":times[times.len()*95/100]}));
    }
    println!(
        "V83_ERASER_CURSOR_CPU={}",
        json!({"documentPixels":[6000,4000],"warmupFrames":40,"records":records,"physicalTabletTested":false,"scope":"warm egui canvas CPU only; excludes OS/GPU/display/hardware"})
    );
}
