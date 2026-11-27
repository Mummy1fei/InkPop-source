//! Production canvas identity/geometry regressions; no physical pen/OS timing claims.
use super::*;
use egui::{CustomCursorImage, Key};
use photocraft_engine::paint::{BrushSettings, GrayTile, TipShape};

fn cursor(h: &mut CanvasHarness, viewport: ViewportId, modifiers: Modifiers) -> CustomCursorImage {
    let point = h.doc_point(120.0, 80.0);
    h.hover(viewport, modifiers, point);
    h.hover(viewport, modifiers, point).platform_output.cursor_image.expect("production native cursor")
}

fn painting_preference(h: &mut CanvasHarness, value: &str) {
    h.app.run("prefs.set", json!({"path":"cursors.painting","value":value})).unwrap();
}

fn sampled(size: f32) -> BrushSettings {
    BrushSettings {
        size,
        hardness: 0.07,
        angle: 34.0,
        roundness: 0.2,
        tip: TipShape::Sampled(GrayTile::from_fn(7, 13, |x, y| if x == 2 || y == 8 { 1.0 } else { 0.0 })),
        ..Default::default()
    }
}

#[test]
fn quick_selection_keeps_a_circle_after_sampled_brush_and_view_changes_v67() {
    let mut h = CanvasHarness::new(320, 200, true);
    h.app.ui.tool = Tool::QuickSelection;
    h.app.sync_tool_brush();
    h.app.run("tools.setBrush", json!({"brush":BrushSettings {size:40.0,..Default::default()},"summary":true})).unwrap();
    let round = cursor(&mut h, ViewportId::ROOT, Modifiers::NONE);
    h.app.run("tools.setBrush", json!({"brush":sampled(40.0),"summary":true})).unwrap();
    let brush_before = h.app.session.tools.brush.clone();
    h.view.rotation = 37.0;
    h.app.ui.view.flip_horizontal = true;
    assert_eq!(cursor(&mut h, ViewportId::ROOT, Modifiers::NONE), round, "selection seed is circular regardless of imported painting tip or canvas rotation");
    painting_preference(&mut h, "fullSizeTip");
    assert_eq!(cursor(&mut h, ViewportId::ROOT, Modifiers::NONE), round, "selection has no hardness-dependent half contour");
    assert_eq!(h.app.session.tools.brush, brush_before, "cursor drawing never rewrites the saved painting preset");
}

#[test]
fn quick_selection_native_seed_diameter_tracks_zoom_dpi_and_clamps_v67() {
    for ppp in [1.0, 1.25, 2.0] {
        for zoom in [0.5, 1.0, 2.75] {
            let mut h = CanvasHarness::new(320, 200, true);
            h.app.ui.tool = Tool::QuickSelection;
            h.app.sync_tool_brush();
            h.app.session.tools.brush.size = 40.0;
            h.view.zoom = zoom;
            h.native_pixels_per_point = ppp;
            let image = cursor(&mut h, ViewportId::ROOT, Modifiers::NONE);
            assert_eq!(h.ctx.pixels_per_point(), ppp);
            let radius_px = 20.0 * zoom * ppp;
            let half = (radius_px + 2.0 * ppp).ceil() as u16;
            assert_eq!(image.size, [half * 2 + 1; 2]);
            assert_eq!(image.hotspot, [half; 2], "circle and add badge use the actual input centre");
            let alpha = |x: u16, y: u16| image.rgba[(usize::from(y) * usize::from(image.size[0]) + usize::from(x)) * 4 + 3];
            assert!(alpha(half, half) > 0, "add sign at hotspot");
            assert!(alpha(0, 0) == 0 && alpha(half, 0) == 0, "transparent padding remains outside footprint");
        }
    }
    assert_eq!(crate::tool_cursor_ui::quick_selection_radius(9999.0, 0.1), Some(250.0));
    assert_eq!(crate::tool_cursor_ui::quick_selection_radius(0.0, 0.1), Some(1.0));
    for invalid in [f32::NAN, f32::INFINITY] {
        assert!(crate::tool_cursor_ui::quick_selection_radius(invalid, 1.0).is_none());
        assert!(crate::tool_cursor_ui::quick_selection_radius(40.0, invalid).is_none());
    }
}

#[test]
fn standard_painting_cursors_render_the_actual_toolbar_glyph_v67() {
    for tool in Tool::ALL.into_iter().filter(|tool| tool.is_brushlike()) {
        let mut h = CanvasHarness::new(320, 200, true);
        h.app.ui.tool = tool;
        painting_preference(&mut h, "standard");
        let image = cursor(&mut h, ViewportId::ROOT, Modifiers::NONE);
        let expected_ctx = egui::Context::default();
        PhotocraftApp::enable_native_tool_cursors(&expected_ctx);
        let hotspot = match tool {
            Tool::Brush => vec2(3.0, 21.0) / 24.0,
            Tool::Pencil => vec2(2.0, 22.0) / 24.0,
            _ => egui::Vec2::splat(0.5),
        };
        crate::icons::cursor(&expected_ctx, crate::icons::tool_icon(tool), Pos2::ZERO, hotspot, 22.0);
        let expected = expected_ctx.output(|output| output.cursor_image.clone()).expect("toolbar symbol rendered as native image");
        assert_eq!(image, expected, "{tool:?}: Standard means this tool's glyph, not a generic system arrow");
    }
}

#[test]
fn standard_brush_pencil_and_selection_hotspots_stay_on_the_working_point_v67() {
    for ppp in [1.0_f32, 1.25, 2.0, 4.0, 8.0, 16.0] {
        for (tool, units) in [(Tool::Brush, vec2(3.0, 21.0)), (Tool::Pencil, vec2(2.0, 22.0)), (Tool::QuickSelection, vec2(9.0, 15.0))] {
            let ctx = egui::Context::default();
            PhotocraftApp::enable_native_tool_cursors(&ctx);
            let mut input = egui::RawInput::default();
            input.viewports.entry(ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
            let mut output = ctx.run_ui(input, |ui| {
                assert_eq!(tool_glyph_cursor(ui.ctx(), tool, pos2(50.0, 50.0)), egui::CursorIcon::None);
            });
            output.textures_delta.clear();
            assert_eq!(ctx.pixels_per_point(), ppp);
            let image = output.platform_output.cursor_image.expect("working-point glyph");
            let expected = egui::Vec2::splat(ppp.ceil()) + units / 24.0 * (22.0 * ppp).round();
            for (actual, expected) in image.hotspot.into_iter().zip([expected.x, expected.y]) {
                assert!((f32::from(actual) - expected).abs() <= 0.5, "{tool:?} at {ppp}× shifted off its SVG working point");
            }
            assert!(image.hotspot[0] < image.size[0] && image.hotspot[1] < image.size[1]);
            assert!(image.size[0] <= 512 && image.size[1] <= 512);
        }
    }
}

#[test]
fn quick_selection_restores_identity_after_other_tools_and_temporary_keys_v67() {
    let mut h = CanvasHarness::new(320, 200, true);
    h.app.ui.tool = Tool::QuickSelection;
    let root = cursor(&mut h, ViewportId::ROOT, Modifiers::NONE);
    let subtract = cursor(&mut h, ViewportId::ROOT, Modifiers::ALT);
    assert_ne!(root, subtract, "Alt replaces the centre add sign by subtract");
    assert_eq!(cursor(&mut h, ViewportId::ROOT, Modifiers::NONE), root);
    let point = h.doc_point(120.0, 80.0);
    let key = |pressed| Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
    let hand = h.frame(ViewportId::ROOT, Modifiers::NONE, vec![key(true), Event::PointerMoved(point)]).platform_output.cursor_image.unwrap();
    assert_ne!(hand, root);
    h.frame(ViewportId::ROOT, Modifiers::NONE, vec![key(false)]);
    assert_eq!(cursor(&mut h, ViewportId::ROOT, Modifiers::NONE), root);
    assert_eq!(h.app.ui.tool, Tool::QuickSelection, "temporary Hand never changes the selected toolbar tool");
    let detached = ViewportId::from_hash_of("v67-cursor-identity-detached");
    for intermediate in [Tool::Brush, Tool::Eyedropper, Tool::EllipseMarquee] {
        h.app.ui.tool = intermediate;
        assert_ne!(cursor(&mut h, detached, Modifiers::NONE), root, "{intermediate:?} has its own pointer");
        h.app.ui.tool = Tool::QuickSelection;
        assert_eq!(cursor(&mut h, detached, Modifiers::NONE), root);
        assert_eq!(cursor(&mut h, ViewportId::ROOT, Modifiers::NONE), root);
    }
}

#[test]
fn quick_selection_software_fallback_keeps_only_the_seed_circle_v67() {
    let mut h = CanvasHarness::new(320, 200, false);
    h.app.ui.tool = Tool::QuickSelection;
    h.app.sync_tool_brush();
    h.app.run("tools.setBrush", json!({"brush":sampled(40.0),"summary":true})).unwrap();
    let point = h.doc_point(120.0, 80.0);
    h.hover(ViewportId::ROOT, Modifiers::NONE, point);
    let output = h.hover(ViewportId::ROOT, Modifiers::NONE, point);
    assert!(output.platform_output.cursor_image.is_none());
    let circles: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Circle(circle) if circle.center == point => Some(circle.radius),
            _ => None,
        })
        .collect();
    assert_eq!(circles, [20.0, 20.0], "software fallback must match native circular diameter, with contrasting outlines");
}

#[test]
fn oversized_quick_selection_keeps_software_extent_and_native_hotspot_v67() {
    let mut h = CanvasHarness::new(320, 200, true);
    h.app.ui.tool = Tool::QuickSelection;
    h.app.sync_tool_brush();
    h.app.session.tools.brush.size = 50.0;
    h.view.zoom = 64.0;
    let point = h.rect.center();
    h.hover(ViewportId::ROOT, Modifiers::NONE, point);
    let output = h.hover(ViewportId::ROOT, Modifiers::NONE, point);
    let image = output.platform_output.cursor_image.expect("large selection retains independent native hotspot");
    assert!(image.size[0] < 32 && image.size[1] < 32);
    let radii: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Circle(circle) if circle.center == point => Some(circle.radius),
            _ => None,
        })
        .collect();
    assert_eq!(radii, [1600.0, 1600.0], "large footprint is not silently shrunk to fit the OS cursor limit");
}

#[test]
#[ignore = "writes five-theme production GUI pointers/toolbar PNGs, not OS compositor captures"]
fn tool_cursor_gpu_visual_v67() {
    use egui_kittest::Harness;
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v67/tool-cursor-gpu");
    assert!(!out.exists(), "keep previous evidence");
    std::fs::create_dir_all(&out).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        let mut harness = Harness::builder().with_size(vec2(1100.0, 900.0)).with_max_steps(64).wgpu().build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, theme);
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.ui.theme = theme;
            app.run("prefs.set", json!({"path":"interface.theme","value":theme.id()})).unwrap();
            app.run("file.new", json!({"width":600,"height":400,"background":"#697783"})).unwrap();
            app.ui.tool = Tool::QuickSelection;
            app.sync_tool_brush();
            app.run("tools.setBrush", json!({"brush":sampled(60.0),"summary":true})).unwrap();
            app
        });
        harness.run_steps(8);
        let point = harness.state().last_canvas_rect.center();
        harness.event(Event::PointerMoved(point));
        harness.run_steps(4);
        assert_eq!(harness.state().ui.theme, theme);
        harness.render().unwrap().save(out.join(format!("{}-quick-circle.png", theme.id()))).unwrap();
        harness.state_mut().run("prefs.set", json!({"path":"cursors.painting","value":"standard"})).unwrap();
        harness.run_steps(4);
        harness.render().unwrap().save(out.join(format!("{}-quick-standard.png", theme.id()))).unwrap();
        harness.state_mut().ui.tool = Tool::Brush;
        harness.run_steps(4);
        harness.render().unwrap().save(out.join(format!("{}-brush-standard.png", theme.id()))).unwrap();
    }
}
