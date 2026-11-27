//! Actual shared canvas dispatch and cursor bitmap/shape regressions, not physical pen timing.
use super::*;
use egui::CustomCursorImage;
use photocraft_engine::paint::{BrushSettings, GrayTile, TipShape};

const AREA_TOOLS: [Tool; 10] = [
    Tool::BackgroundEraser,
    Tool::SpotHealing,
    Tool::Healing,
    Tool::CloneStamp,
    Tool::HistoryBrush,
    Tool::Blur,
    Tool::Sharpen,
    Tool::Dodge,
    Tool::Burn,
    Tool::Sponge,
];

fn sampled() -> BrushSettings {
    BrushSettings {
        size: 40.0,
        hardness: 0.07,
        angle: 34.0,
        roundness: 0.2,
        tip: TipShape::Sampled(GrayTile::from_fn(13, 17, |x, y| if x == 3 || y == 8 { 1.0 } else { 0.0 })),
        ..Default::default()
    }
}

fn prepare(h: &mut CanvasHarness, tool: Tool) {
    h.app.ui.tool = tool;
    h.app.sync_tool_brush();
    h.app.session.tools.brush = sampled();
}

fn hover(h: &mut CanvasHarness, viewport: ViewportId) -> egui::FullOutput {
    let p = h.doc_point(120.0, 80.0);
    h.hover(viewport, Modifiers::NONE, p);
    h.hover(viewport, Modifiers::NONE, p)
}

fn expected_circle(ppp: f32, radius: f32, centre: bool) -> CustomCursorImage {
    let ctx = egui::Context::default();
    PhotocraftApp::enable_native_tool_cursors(&ctx);
    let mut input = egui::RawInput::default();
    input.viewports.entry(ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
    let mut output = ctx.run_ui(input, |ui| {
        assert!(crate::native_cursor::area_circle(ui.ctx(), radius, centre.then_some(3.0)));
    });
    output.textures_delta.clear();
    output.platform_output.cursor_image.unwrap()
}

#[test]
fn area_tools_ignore_inherited_sampled_silhouette_in_shared_native_dispatch_v78() {
    let expected = expected_circle(1.0, 20.0, false);
    for tool in AREA_TOOLS {
        let mut h = CanvasHarness::new(320, 200, true);
        prepare(&mut h, tool);
        let before = h.app.session.tools.brush.clone();
        for preference in ["normalTip", "fullSizeTip"] {
            h.app.run("prefs.set", json!({"path":"cursors.painting","value":preference})).unwrap();
            for viewport in [ViewportId::ROOT, ViewportId::from_hash_of("v78-area-cursor")] {
                h.view.rotation = 37.0;
                h.app.ui.view.flip_horizontal = true;
                let actual = hover(&mut h, viewport).platform_output.cursor_image.unwrap();
                assert_eq!(actual, expected, "{tool:?}/{preference}: round area must ignore sampled tip, hardness, angle and view rotation");
            }
        }
        assert_eq!(h.app.session.tools.brush, before, "cursor dispatch must not reset {tool:?}'s saved preset");
    }
}

#[test]
fn area_tools_software_shapes_match_native_radius_and_never_draw_sampled_paths_v78() {
    for tool in AREA_TOOLS {
        for zoom in [0.5, 1.0, 2.75] {
            let mut h = CanvasHarness::new(320, 200, false);
            prepare(&mut h, tool);
            h.view.zoom = zoom;
            let p = h.doc_point(120.0, 80.0);
            let output = hover(&mut h, ViewportId::ROOT);
            let circles: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle) if circle.center == p => Some(circle.radius),
                    _ => None,
                })
                .collect();
            assert_eq!(circles, [20.0 * zoom; 2], "{tool:?}: both contrast outlines use the full circular tool area");
            assert!(output.platform_output.cursor_image.is_none());
            assert!(
                !output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Path(path) if path.points.iter().any(|q|q.distance(p)<60.0))),
                "{tool:?}: no inherited sampled contour path"
            );
        }
    }
}

#[test]
fn area_native_diameter_tracks_actual_size_zoom_dpi_and_crosshair_preference_v78() {
    for ppp in [1.0, 1.25, 2.0] {
        for zoom in [0.5, 1.0, 2.75] {
            let mut h = CanvasHarness::new(320, 200, true);
            prepare(&mut h, Tool::SpotHealing);
            h.native_pixels_per_point = ppp;
            h.view.zoom = zoom;
            for centre in [false, true] {
                h.app.run("prefs.set", json!({"path":"cursors.showCrosshairInBrushTip","value":centre})).unwrap();
                let actual = hover(&mut h, ViewportId::ROOT).platform_output.cursor_image.unwrap();
                assert_eq!(actual, expected_circle(ppp, 20.0 * zoom, centre));
                let half = (20.0 * zoom * ppp + 2.0 * ppp).ceil() as u16;
                assert_eq!(actual.size, [half * 2 + 1; 2]);
                assert_eq!(actual.hotspot, [half; 2]);
            }
        }
    }
    for invalid in [f32::NAN, f32::INFINITY] {
        assert!(crate::tool_cursor_ui::area_radius(Tool::SpotHealing, invalid, 1.0).is_none());
        assert!(crate::tool_cursor_ui::area_radius(Tool::SpotHealing, 40.0, invalid).is_none());
    }
}

#[test]
fn subpixel_retouch_and_eraser_cursors_match_their_actual_engine_minima_v78() {
    for tool in AREA_TOOLS {
        // At 8× zoom a one-pixel retouch diameter has radius 4 screen points;
        // a half-pixel Eraser/Background Eraser diameter has radius 2.
        let expected_radius = if matches!(tool, Tool::Eraser | Tool::BackgroundEraser) { 2.0 } else { 4.0 };
        for native in [false, true] {
            let mut h = CanvasHarness::new(320, 200, native);
            prepare(&mut h, tool);
            h.app.session.tools.brush.size = 0.5;
            h.view.zoom = 8.0;
            h.view.center = [120.0, 80.0];
            let before = h.app.session.tools.brush.clone();
            let p = h.doc_point(120.0, 80.0);
            let output = hover(&mut h, ViewportId::ROOT);
            if native {
                let image = output.platform_output.cursor_image.unwrap();
                let half = expected_radius as u16 + 2;
                assert_eq!(image.size, [half * 2 + 1; 2], "{tool:?}: bitmap extent follows the real minimum diameter");
                assert_eq!(image.hotspot, [half; 2]);
                let ring_x = half + expected_radius as u16;
                let index = (usize::from(half) * usize::from(image.size[0]) + usize::from(ring_x)) * 4 + 3;
                assert!(image.rgba[index] > 0, "{tool:?}: rendered pixels reach the expected circular boundary");
                assert_eq!(image.rgba[3], 0, "{tool:?}: corner outside the circle remains transparent");
            } else {
                let radii: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Circle(circle) if circle.center == p => Some(circle.radius),
                        _ => None,
                    })
                    .collect();
                assert_eq!(radii, [expected_radius; 2], "{tool:?}: software outline has the same logical radius");
            }
            assert_eq!(h.app.session.tools.brush, before, "{tool:?}: cursor clamping never changes the saved preset");
        }
    }
}

#[test]
fn painting_tools_keep_sampled_outline_and_restore_it_after_retouch_v78() {
    for tool in [Tool::Brush, Tool::Eraser, Tool::Smudge, Tool::MixerBrush] {
        let mut h = CanvasHarness::new(320, 200, true);
        prepare(&mut h, tool);
        let before = h.app.session.tools.brush.clone();
        let image = hover(&mut h, ViewportId::ROOT).platform_output.cursor_image.unwrap();
        assert_ne!(image, expected_circle(1.0, 20.0, false), "{tool:?} retains its custom sampled contour");
        h.app.ui.tool = Tool::SpotHealing;
        h.app.sync_tool_brush();
        h.app.session.tools.brush = sampled();
        assert_eq!(hover(&mut h, ViewportId::ROOT).platform_output.cursor_image.unwrap(), expected_circle(1.0, 20.0, false));
        h.app.ui.tool = tool;
        h.app.sync_tool_brush();
        assert_eq!(h.app.session.tools.brush, before);
        assert_eq!(hover(&mut h, ViewportId::ROOT).platform_output.cursor_image.unwrap(), image);
    }
}

#[test]
#[ignore = "exports actual cursor bitmaps for root visual inspection; no user document"]
fn export_area_cursor_bitmaps_v78() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v78/cursors");
    assert!(!root.exists(), "preserve previous cursor evidence");
    std::fs::create_dir_all(&root).unwrap();
    let mut records = Vec::new();
    for (tool, name) in [
        (Tool::Brush, "brush-sampled"),
        (Tool::Smudge, "smudge-sampled"),
        (Tool::MixerBrush, "mixer-sampled"),
        (Tool::SpotHealing, "spot-healing-round"),
        (Tool::Blur, "blur-round"),
        (Tool::Eraser, "eraser-sampled"),
    ] {
        let mut h = CanvasHarness::new(320, 200, true);
        prepare(&mut h, tool);
        h.app.session.tools.brush.size = 180.0;
        let image = hover(&mut h, ViewportId::ROOT).platform_output.cursor_image.unwrap();
        let mut pixmap = tiny_skia::Pixmap::new(u32::from(image.size[0]), u32::from(image.size[1])).unwrap();
        for (source, destination) in image.rgba.as_chunks::<4>().0.iter().zip(pixmap.data_mut().as_chunks_mut::<4>().0) {
            for (destination, channel) in destination.iter_mut().zip(source.iter()).take(3) {
                *destination = ((u16::from(*channel) * u16::from(source[3]) + 127) / 255) as u8;
            }
            destination[3] = source[3];
        }
        let file = format!("{name}.png");
        pixmap.save_png(root.join(&file)).unwrap();
        records.push(
            json!({"file":file,"tool":format!("{tool:?}"),"scope":"actual shared canvas native cursor bitmap", "size":image.size,"hotspot":image.hotspot}),
        );
    }
    std::fs::write(root.join("manifest.json"), serde_json::to_vec_pretty(&records).unwrap()).unwrap();
}
