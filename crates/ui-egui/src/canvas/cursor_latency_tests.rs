//! Real canvas event regressions. The ignored benchmark measures CPU frame work only:
//! it does not measure the OS cursor, GPU presentation, display scanout, or a physical pen.
use super::*;
use egui::{Event, Modifiers, ViewportId};

#[path = "cursor_identity_tests_v67.rs"]
mod cursor_identity_tests_v67;

#[path = "cursor_area_tests_v78.rs"]
mod cursor_area_tests_v78;

#[path = "eraser_cursor_tests_v83.rs"]
mod eraser_cursor_tests_v83;

#[path = "eyedropper_alt_tests_v102.rs"]
mod eyedropper_alt_tests_v102;

struct CanvasHarness {
    app: PhotocraftApp,
    ctx: egui::Context,
    view: View,
    rect: Rect,
    time: f64,
    native_pixels_per_point: f32,
}

impl CanvasHarness {
    fn new(width: u32, height: u32, native: bool) -> Self {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        PhotocraftApp::enable_native_tool_cursors(&ctx);
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("photocraft-native-tool-cursors"), native));
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":width,"height":height,"background":"transparent"})).unwrap();
        app.ui.extras.rulers = false;
        app.session.tools.brush.size = 20.0;
        let rect = Rect::from_min_size(pos2(32.0, 32.0), vec2(320.0, 200.0));
        let view = View {
            rotation: 0.0,
            center: [width as f32 / 2.0, height as f32 / 2.0],
            zoom: (320.0 / width as f32).min(200.0 / height as f32),
            doc_size: [width, height],
            fit_pending: false,
        };
        Self { app, ctx, view, rect, time: 0.0, native_pixels_per_point: 1.0 }
    }

    fn frame(&mut self, viewport: ViewportId, modifiers: Modifiers, events: Vec<Event>) -> egui::FullOutput {
        self.frame_document(viewport, 0, true, modifiers, events)
    }

    fn frame_document(&mut self, viewport: ViewportId, document: usize, primary: bool, modifiers: Modifiers, mut events: Vec<Event>) -> egui::FullOutput {
        self.time += 1.0 / 120.0;
        events.insert(0, Event::ModifiersChanged(modifiers));
        let mut input = egui::RawInput {
            viewport_id: viewport,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.rect.max.to_vec2() + vec2(32.0, 32.0))),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        input.viewports.entry(viewport).or_default().focused = Some(true);
        input.viewports.entry(viewport).or_default().native_pixels_per_point = Some(self.native_pixels_per_point);
        let mut output = self.ctx.run_ui(input, |ui| {
            self.view = canvas_view(&mut self.app, ui, document, self.rect, self.view.clone(), primary);
        });
        output.textures_delta.clear();
        output
    }

    fn hover(&mut self, viewport: ViewportId, modifiers: Modifiers, p: Pos2) -> egui::FullOutput {
        self.frame(viewport, modifiers, vec![Event::PointerMoved(p)])
    }

    fn doc_point(&self, x: f32, y: f32) -> Pos2 {
        ViewXform { rotation: self.view.rotation, rect: self.rect, zoom: self.view.zoom, center: self.view.center, flip: false }.to_screen(x, y)
    }
}

fn button(p: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers }
}

#[test]
fn all_fifty_tool_hovers_use_native_pointer_output() {
    assert_eq!(Tool::ALL.len(), 50);
    for tool in Tool::ALL {
        let mut h = CanvasHarness::new(320, 200, true);
        h.app.ui.tool = tool;
        let p = h.doc_point(110.0, 90.0);
        h.hover(ViewportId::ROOT, Modifiers::NONE, p);
        let output = h.hover(ViewportId::ROOT, Modifiers::NONE, p + vec2(1.0, 0.0));
        if tool == Tool::Crop {
            // A crop frame's interior already uses the OS Move pointer, not a tool glyph.
            assert_eq!(output.platform_output.cursor_icon, egui::CursorIcon::Move);
        } else {
            let image = output.platform_output.cursor_image.as_ref().unwrap_or_else(|| panic!("{tool:?} has no native pointer image"));
            assert!(image.size[0] > 0 && image.size[1] > 0, "{tool:?}");
            assert!(image.hotspot[0] < image.size[0] && image.hotspot[1] < image.size[1], "{tool:?}");
        }
    }
}

#[test]
fn lasso_pointer_tip_matches_input_at_multiple_dpis_and_transformed_views() {
    for ppp in [1.0, 1.25, 2.0] {
        for (zoom, rotation, flip) in [(1.0, 0.0, false), (2.75, 37.0, false), (0.8, -23.0, true)] {
            let mut h = CanvasHarness::new(320, 200, true);
            h.app.ui.tool = Tool::Lasso;
            h.app.ui.view.flip_horizontal = flip;
            h.view.zoom = zoom;
            h.view.rotation = rotation;
            h.native_pixels_per_point = ppp;
            let point = [155.25, 95.5];
            let xf = ViewXform { rotation, rect: h.rect, zoom, center: h.view.center, flip };
            let p = xf.to_screen(point[0] as f32, point[1] as f32);
            h.hover(ViewportId::ROOT, Modifiers::NONE, p);
            let output = h.frame(ViewportId::ROOT, Modifiers::NONE, vec![button(p, true, Modifiers::NONE)]);
            assert_eq!(h.ctx.pixels_per_point(), ppp);
            let start = h.app.drag.as_ref().expect("lasso outline").start;
            assert!((start[0] - point[0]).abs() < 0.001 && (start[1] - point[1]).abs() < 0.001, "{ppp}x/{zoom}/{rotation}/{flip}: {start:?}");
            let image = output.platform_output.cursor_image.expect("OS lasso pointer");
            let side = (28.0 * ppp).round();
            let tip = ppp.ceil() + side * 2.0 / 32.0;
            for component in image.hotspot {
                assert!((f32::from(component) - tip).abs() <= 0.5, "hotspot must be the arrow tip, not the lasso loop");
            }
            let x = usize::from(image.hotspot[0]);
            let y = usize::from(image.hotspot[1]);
            let index = (y * usize::from(image.size[0]) + x) * 4;
            assert!(image.rgba[index + 3] > 0, "arrow tip is visible at the exact hotspot");
        }
    }
}

#[test]
fn ctrl_moves_an_idle_lasso_but_keeps_polygon_segments_during_its_outline_v74() {
    let mut h = CanvasHarness::new(320, 200, true);
    h.app.ui.tool = Tool::Lasso;
    let p = h.doc_point(120.0, 80.0);
    h.hover(ViewportId::ROOT, Modifiers::NONE, p);
    let freehand = h.hover(ViewportId::ROOT, Modifiers::NONE, p).platform_output.cursor_image.unwrap();
    let temporary_move = h.hover(ViewportId::ROOT, Modifiers::CTRL, p).platform_output.cursor_image.unwrap();
    h.app.ui.tool = Tool::Move;
    let move_pointer = h.hover(ViewportId::ROOT, Modifiers::NONE, p).platform_output.cursor_image.unwrap();
    assert_eq!(temporary_move, move_pointer, "idle Ctrl uses the Move cursor");
    h.app.ui.tool = Tool::Lasso;
    assert_eq!(h.hover(ViewportId::ROOT, Modifiers::NONE, p).platform_output.cursor_image.unwrap(), freehand);
    h.frame(ViewportId::ROOT, Modifiers::NONE, vec![button(p, true, Modifiers::NONE)]);
    assert!(crate::lasso_ui::active(&h.app));
    let polygon = h.hover(ViewportId::ROOT, Modifiers::CTRL, p).platform_output.cursor_image.unwrap();
    assert_eq!(h.app.drag.as_ref().unwrap().tool, Tool::Lasso, "Ctrl never steals an existing outline");
    assert_eq!(freehand.hotspot, polygon.hotspot, "changing symbol must never move the input tip");
    assert_ne!(freehand.rgba, polygon.rgba);
    assert_eq!(h.hover(ViewportId::ROOT, Modifiers::NONE, p).platform_output.cursor_image.unwrap(), freehand);
    h.frame(ViewportId::ROOT, Modifiers::NONE, vec![button(p, false, Modifiers::NONE)]);
    h.app.ui.tool = Tool::PolygonLasso;
    assert_eq!(h.hover(ViewportId::ROOT, Modifiers::NONE, p).platform_output.cursor_image.unwrap(), polygon);
}

#[test]
#[ignore = "writes native lasso cursor bitmaps and hotspot metadata at three DPIs"]
fn export_lasso_pointer_assets_v60() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v60/lasso-native-cursors");
    assert!(!root.exists(), "preserve previous cursor evidence");
    std::fs::create_dir_all(&root).unwrap();
    let mut records = Vec::new();
    for ppp in [1.0, 1.25, 2.0] {
        let mut h = CanvasHarness::new(320, 200, true);
        h.native_pixels_per_point = ppp;
        h.app.ui.tool = Tool::Lasso;
        for (name, mods) in [
            ("freehand", Modifiers::NONE),
            ("polygon", Modifiers::CTRL),
            ("add", Modifiers::SHIFT),
            ("subtract", Modifiers::ALT),
            ("intersect", Modifiers::ALT | Modifiers::SHIFT),
        ] {
            let p = h.doc_point(120.0, 80.0);
            h.hover(ViewportId::ROOT, mods, p);
            let image = h.hover(ViewportId::ROOT, mods, p).platform_output.cursor_image.unwrap();
            assert_eq!(h.ctx.pixels_per_point(), ppp);
            let mut pixmap = tiny_skia::Pixmap::new(u32::from(image.size[0]), u32::from(image.size[1])).unwrap();
            for (src, dst) in image.rgba.as_chunks::<4>().0.iter().zip(pixmap.data_mut().as_chunks_mut::<4>().0.iter_mut()) {
                let a = u16::from(src[3]);
                for i in 0..3 {
                    dst[i] = ((u16::from(src[i]) * a + 127) / 255) as u8;
                }
                dst[3] = src[3];
            }
            let filename = format!("lasso-{name}-{ppp}x.png");
            pixmap.save_png(root.join(&filename)).unwrap();
            records.push(json!({"file":filename,"pixelsPerPoint":h.ctx.pixels_per_point(),"size":image.size,"hotspot":image.hotspot}));
        }
    }
    std::fs::write(root.join("metadata.json"), serde_json::to_vec_pretty(&records).unwrap()).unwrap();
}

#[test]
fn alt_hover_switches_brush_and_mixer_to_pipette_then_restores_tip() {
    for tool in [Tool::Brush, Tool::MixerBrush, Tool::Pencil] {
        let mut h = CanvasHarness::new(320, 200, true);
        h.app.ui.tool = tool;
        let p = h.doc_point(120.0, 80.0);
        h.hover(ViewportId::ROOT, Modifiers::NONE, p);
        let tip = h.hover(ViewportId::ROOT, Modifiers::NONE, p).platform_output.cursor_image.unwrap();
        let sample = h.hover(ViewportId::ROOT, Modifiers::ALT, p).platform_output.cursor_image.unwrap();
        assert_ne!(tip, sample, "{tool:?} must visibly become the eyedropper");
        let restored = h.hover(ViewportId::ROOT, Modifiers::NONE, p).platform_output.cursor_image.unwrap();
        assert_eq!(tip, restored, "{tool:?} must restore the exact tip when Alt is released");
        assert!(h.app.drag.is_none() && !h.app.alt_sampling, "hover must not start a gesture");
    }
}

#[test]
fn pencil_keeps_pixel_aligned_footprint_beside_native_hotspot() {
    let mut h = CanvasHarness::new(320, 200, true);
    h.app.ui.tool = Tool::Pencil;
    let p = h.doc_point(120.4, 80.6);
    h.hover(ViewportId::ROOT, Modifiers::NONE, p);
    let output = h.hover(ViewportId::ROOT, Modifiers::NONE, p);
    assert!(output.platform_output.cursor_image.is_some(), "the precise pencil has an independent native hotspot");
    let xf = ViewXform { rotation: h.view.rotation, rect: h.rect, zoom: h.view.zoom, center: h.view.center, flip: false };
    let expected = pencil_cursor_rect(&xf, xf.to_doc(p), h.app.session.tools.brush.size, h.ctx.pixels_per_point());
    let footprint_strokes = output.shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.rect == expected)).count();
    assert_eq!(footprint_strokes, 2, "native movement must preserve the black/white pixel-grid footprint");
}

#[test]
fn leaving_canvas_clears_custom_pointer_in_each_viewport() {
    let mut h = CanvasHarness::new(320, 200, true);
    let detached = ViewportId::from_hash_of("v44-detached-cursor-regression");
    let p = h.doc_point(120.0, 80.0);
    for viewport in [ViewportId::ROOT, detached] {
        h.app.ui.tool = Tool::Brush;
        h.hover(viewport, Modifiers::NONE, p);
        assert!(h.hover(viewport, Modifiers::NONE, p).platform_output.cursor_image.is_some());
        let outside = h.hover(viewport, Modifiers::NONE, pos2(12.0, 12.0));
        assert!(outside.platform_output.cursor_image.is_none(), "{viewport:?}: a panel must not inherit the brush image");
        h.hover(viewport, Modifiers::NONE, p);
        let gone = h.frame(viewport, Modifiers::NONE, vec![Event::PointerGone]);
        assert!(gone.platform_output.cursor_image.is_none(), "{viewport:?}: leave-window must clear the native image");
    }
}

#[test]
fn root_and_detached_cursor_updates_do_not_reuse_the_other_tool_image() {
    let mut h = CanvasHarness::new(320, 200, true);
    let detached = ViewportId::from_hash_of("v44-independent-native-cursor");
    let p = h.doc_point(120.0, 80.0);
    h.app.ui.tool = Tool::Brush;
    h.hover(ViewportId::ROOT, Modifiers::NONE, p);
    let root = h.hover(ViewportId::ROOT, Modifiers::NONE, p).platform_output.cursor_image.unwrap();
    h.app.ui.tool = Tool::Eyedropper;
    h.hover(detached, Modifiers::NONE, p);
    let child = h.hover(detached, Modifiers::NONE, p).platform_output.cursor_image.unwrap();
    assert_ne!(root, child);
    assert!(h.hover(detached, Modifiers::NONE, pos2(12.0, 12.0)).platform_output.cursor_image.is_none());
    h.app.ui.tool = Tool::Brush;
    let root_again = h.hover(ViewportId::ROOT, Modifiers::NONE, p).platform_output.cursor_image.unwrap();
    assert_eq!(root, root_again, "the detached output must not replace the root's brush");
}

#[test]
fn nested_immediate_viewport_pass_resets_only_its_own_cursor_output() {
    let mut h = CanvasHarness::new(320, 200, true);
    let detached = ViewportId::from_hash_of("v44-nested-immediate-cursor");
    let p = h.doc_point(120.0, 80.0);
    // Build hover hit-test state for both windows before nesting their real passes.
    for viewport in [ViewportId::ROOT, detached] {
        h.hover(viewport, Modifiers::NONE, p);
        h.hover(viewport, Modifiers::NONE, p);
    }
    let ctx = h.ctx.clone();
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(384.0, 264.0)));
    let raw = |viewport| {
        let mut input = egui::RawInput {
            viewport_id: viewport,
            screen_rect,
            events: vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerMoved(p)],
            ..Default::default()
        };
        let info = input.viewports.entry(viewport).or_default();
        info.focused = Some(true);
        if viewport == detached {
            info.parent = Some(ViewportId::ROOT);
        }
        input
    };
    let mut child_image = None;
    let mut parent_image = None;
    let mut output = ctx.run_ui(raw(ViewportId::ROOT), |ui| {
        h.app.ui.tool = Tool::Brush;
        h.view = canvas_view(&mut h.app, ui, 0, h.rect, h.view.clone(), true);
        let before_child = ctx.output(|o| o.cursor_image.clone()).expect("root brush image before nesting");
        let mut child_view = h.view.clone();
        // eframe's native immediate renderer also calls run_ui recursively with the same Context.
        // The backend must separately forward this output with an ActiveEventLoop to display it.
        let mut child_output = ctx.run_ui(raw(detached), |child_ui| {
            assert_eq!(child_ui.ctx().viewport_id(), detached);
            assert!(child_ui.ctx().output(|o| o.cursor_image.is_none()), "the child begin-pass callback cleared its previous image");
            h.app.ui.tool = Tool::RotateView;
            child_view = canvas_view(&mut h.app, child_ui, 0, h.rect, child_view.clone(), true);
        });
        child_image = child_output.platform_output.cursor_image.take();
        child_output.textures_delta.clear();
        assert_eq!(ctx.viewport_id(), ViewportId::ROOT, "child end-pass must restore the active parent viewport");
        assert_eq!(ctx.output(|o| o.cursor_image.clone()), Some(before_child.clone()), "the child's reset/output extraction must not clear the parent image");
        parent_image = Some(before_child);
    });
    output.textures_delta.clear();
    let root = parent_image.expect("parent cursor was drawn");
    let child = child_image.expect("nested Rotate View emitted a custom image");
    assert_ne!(root, child);
    assert_eq!(output.platform_output.cursor_image, Some(root));
}

#[test]
fn newly_opened_canvas_menus_clear_native_tool_pointer_only_over_the_menu() {
    for (tool, area_name) in [(Tool::Move, "canvas-layer-menu"), (Tool::RectMarquee, "canvas-selection-menu"), (Tool::Brush, "canvas-brush-picker")] {
        let mut h = CanvasHarness::new(320, 200, true);
        // Leave room for the embedded picker and a separate, unobstructed canvas region.
        h.rect = Rect::from_min_size(pos2(32.0, 32.0), vec2(900.0, 600.0));
        h.ctx.set_embed_viewports(true);
        h.app.run("edit.fill", json!({"color":"#4488cc"})).unwrap();
        h.app.ui.tool = tool;
        let p = h.doc_point(128.0, 68.0);
        h.hover(ViewportId::ROOT, Modifiers::NONE, p);
        assert!(h.hover(ViewportId::ROOT, Modifiers::NONE, p).platform_output.cursor_image.is_some(), "{tool:?}: canvas starts with a tool pointer");
        let area_id = egui::Id::new(area_name);
        assert!(h.ctx.memory(|m| m.area_rect(area_id)).is_none(), "this must exercise the menu's first appearance");
        h.frame(
            ViewportId::ROOT,
            Modifiers::NONE,
            vec![Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE }],
        );
        let opened = h.frame(
            ViewportId::ROOT,
            Modifiers::NONE,
            vec![Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE }],
        );
        let open = match tool {
            Tool::Move => h.app.ui.layer_menu.is_some(),
            Tool::RectMarquee => h.app.ui.canvas_tool_menu.is_some(),
            Tool::Brush => h.app.ui.brush_picker.is_some(),
            _ => false,
        };
        assert!(open, "{tool:?}: a real right-click must open the target menu");
        let area = h.ctx.memory(|m| m.area_rect(area_id)).expect("the actual menu area was laid out");
        assert!(area.contains(p), "{tool:?}: the first-frame pointer must be over the menu, p={p:?}, menu={area:?}");
        assert!(opened.platform_output.cursor_image.is_none(), "{tool:?}: later canvas cursor drawing must not overwrite a newly opened menu's pointer");

        let outside = pos2(80.0, 80.0);
        assert!(h.rect.contains(outside) && !area.contains(outside));
        h.hover(ViewportId::ROOT, Modifiers::NONE, outside);
        let canvas_again = h.hover(ViewportId::ROOT, Modifiers::NONE, outside);
        assert!(canvas_again.platform_output.cursor_image.is_some(), "{tool:?}: an open menu must not suppress the tool pointer over unrelated canvas pixels");
    }
}

#[test]
fn selection_menu_opens_on_press_and_survives_held_button_motion() {
    for (tool, at) in [(Tool::Lasso, [180.0, 155.0]), (Tool::RectMarquee, [885.0, 570.0])] {
        let mut h = CanvasHarness::new(900, 600, false);
        h.rect = Rect::from_min_size(pos2(32.0, 32.0), vec2(900.0, 600.0));
        h.view.zoom = 1.0;
        h.ctx.set_embed_viewports(true);
        h.app.ui.tool = tool;
        h.app.run("select.rect", json!({"x":100,"y":100,"width":800,"height":495})).unwrap();
        let p = h.doc_point(at[0], at[1]);
        let before = h.app.session.active().unwrap().doc.selection.clone();
        let revision = h.app.session.active().unwrap().revision;
        let secondary = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE };

        h.hover(ViewportId::ROOT, Modifiers::NONE, p);
        h.frame(ViewportId::ROOT, Modifiers::NONE, vec![secondary(p, true)]);
        let opened = h.app.ui.canvas_tool_menu.clone().expect("selection context menu opens on press");
        assert_eq!(opened.pos, [p.x, p.y]);
        assert!(opened.has_selection);

        let moved = p - vec2(25.0, 15.0);
        h.time += 1.0;
        h.hover(ViewportId::ROOT, Modifiers::NONE, moved);
        h.frame(ViewportId::ROOT, Modifiers::NONE, vec![secondary(moved, false)]);
        assert_eq!(h.app.ui.canvas_tool_menu, Some(opened));
        assert_eq!(h.app.session.active().unwrap().revision, revision);
        assert_eq!(h.app.session.active().unwrap().doc.selection, before);
    }
}

#[test]
fn raw_alt_drag_batch_samples_latest_color_without_per_move_journal_growth() {
    for tool in [Tool::Brush, Tool::Pencil] {
        let mut h = CanvasHarness::new(320, 200, true);
        // Alternating colours ensure command deduplication alone cannot pass this regression.
        h.app.run("shape.create", json!({"kind":"rect","rect":[0,0,160,200],"fill":"#ff0000"})).unwrap();
        h.app.run("shape.create", json!({"kind":"rect","rect":[160,0,160,200],"fill":"#00ff00"})).unwrap();
        h.app.ui.tool = tool;
        let a = h.doc_point(70.0, 80.0);
        let b = h.doc_point(230.0, 80.0);
        h.hover(ViewportId::ROOT, Modifiers::ALT, a);
        h.frame(ViewportId::ROOT, Modifiers::ALT, vec![button(a, true, Modifiers::ALT)]);
        let revision = h.app.session.active().unwrap().revision;
        let journal = h.app.session.journal.len();
        let moves = (0..64).map(|i| Event::PointerMoved(if i % 2 == 0 { a } else { b })).collect();
        h.frame(ViewportId::ROOT, Modifiers::ALT, moves);
        assert_eq!(h.app.session.tools.foreground, [0.0, 1.0, 0.0, 1.0], "{tool:?}: use the last raw sample");
        let added = &h.app.session.journal[journal..];
        assert!(added.iter().filter(|(id, _)| id == "tools.setColors").count() <= 2, "{tool:?}: only press and final colour are observable");
        assert!(added.len() <= 4, "{tool:?}: bounded reads and writes, got {} entries", added.len());
        h.frame(ViewportId::ROOT, Modifiers::ALT, vec![button(b, false, Modifiers::ALT)]);
        assert_eq!(h.app.session.active().unwrap().revision, revision, "sampling must not paint");
        assert!(!h.app.alt_sampling && h.app.drag.is_none());
    }
}

#[test]
fn ordinary_painting_keeps_every_distinct_raw_move_in_the_frame() {
    for tool in [Tool::Brush, Tool::Pencil] {
        let mut h = CanvasHarness::new(320, 200, true);
        h.app.ui.tool = tool;
        h.app.session.tools.brush.smoothing.amount = 0.0;
        let start = h.doc_point(40.0, 80.0);
        h.hover(ViewportId::ROOT, Modifiers::NONE, start);
        h.frame(ViewportId::ROOT, Modifiers::NONE, vec![button(start, true, Modifiers::NONE)]);
        let points: Vec<Pos2> = (1..=24).map(|i| h.doc_point(40.0 + i as f32 * 8.0, 80.0 + (i % 2) as f32 * 8.0)).collect();
        h.frame(ViewportId::ROOT, Modifiers::NONE, points.iter().copied().map(Event::PointerMoved).collect());
        let drag = h.app.drag.as_ref().expect("real pointer drag starts a stroke");
        for point in &points {
            let doc = ViewXform { rotation: 0.0, rect: h.rect, zoom: h.view.zoom, center: h.view.center, flip: false }.to_doc(*point);
            assert!(drag.points.iter().any(|p| (p[0] - doc[0]).abs() < 0.01 && (p[1] - doc[1]).abs() < 0.01), "{tool:?}: raw point {doc:?} dropped");
        }
        h.frame(ViewportId::ROOT, Modifiers::NONE, vec![button(*points.last().unwrap(), false, Modifiers::NONE)]);
        assert!(h.app.drag.is_none());
        let stroke = h.app.session.journal.iter().rev().find(|(id, _)| id == stroke_command(tool)).expect("stroke was committed");
        assert!(stroke.1["points"].as_array().unwrap().len() >= points.len());
    }
}

#[test]
#[ignore = "release-only 24 MP warm CPU frame benchmark; not OS/hardware latency"]
#[allow(clippy::assertions_on_constants)] // Runtime guard: debug builds must compile, but must not publish benchmark numbers.
fn benchmark_cursor_hover_cpu_24mp() {
    assert!(!cfg!(debug_assertions), "run this benchmark with cargo test --release");
    let mut h = CanvasHarness::new(6000, 4000, false);
    h.rect = Rect::from_min_size(pos2(32.0, 32.0), vec2(900.0, 600.0));
    h.view.zoom = 0.15;
    h.app.session.tools.brush.size = 160.0;
    let mut records = Vec::new();
    for (tool, modifiers) in
        [(Tool::Brush, Modifiers::NONE), (Tool::Brush, Modifiers::ALT), (Tool::Pencil, Modifiers::NONE), (Tool::RectMarquee, Modifiers::NONE)]
    {
        h.app.ui.tool = tool;
        for native in [false, true] {
            h.ctx.data_mut(|d| d.insert_temp(egui::Id::new("photocraft-native-tool-cursors"), native));
            let mut samples = Vec::new();
            let mut shapes = Vec::new();
            for frame in 0..280 {
                let p = pos2(180.0 + (frame % 96) as f32 * 4.0, 150.0 + (frame % 63) as f32 * 3.0);
                let start = std::time::Instant::now();
                let output = h.hover(ViewportId::ROOT, modifiers, p);
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                if frame >= 40 {
                    samples.push(elapsed);
                    shapes.push(output.shapes.len());
                }
            }
            samples.sort_by(f64::total_cmp);
            shapes.sort_unstable();
            records.push(json!({
                "tool":format!("{tool:?}"), "alt":modifiers.alt, "native":native,
                "samples":samples.len(), "cpu_frame_ms_p50":samples[samples.len()/2],
                "cpu_frame_ms_p95":samples[(samples.len()*95/100).min(samples.len()-1)],
                "shapes_p50":shapes[shapes.len()/2], "shapes_p95":shapes[(shapes.len()*95/100).min(shapes.len()-1)]
            }));
        }
    }
    println!(
        "V44_CURSOR_CPU_BENCHMARK={}",
        json!({
            "measurement":"warm ctx.run_ui canvas CPU work only; excludes native OS movement, GPU, scanout, physical device",
            "document_pixels":[6000,4000],"warmup_frames":40,"records":records
        })
    );
}
