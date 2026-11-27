use super::*;
use crate::theme::ThemeKind;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::Harness;

fn fixture() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"name":"Ctrl pickup outline","width":320,"height":240,"background":"transparent"})).unwrap();
    for (name, rect, color) in [
        ("Red target", DRect::new(40, 40, 100, 100), [0.95, 0.15, 0.1, 1.0]),
        ("Blue previous selection", DRect::new(200, 150, 250, 200), [0.1, 0.3, 0.95, 1.0]),
    ] {
        app.run("layer.new.layer", json!({"name":name})).unwrap();
        app.session
            .edit("synthetic rectangle", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(rect, &color);
                Ok(())
            })
            .unwrap();
    }
    app.sync_views();
    app
}

fn harness(kind: ThemeKind, gpu: bool) -> Harness<'static, PhotocraftApp> {
    let builder = Harness::builder().with_size(vec2(1080.0, 800.0)).with_step_dt(1.0 / 60.0).with_max_steps(64);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    let mut h = builder.build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
            crate::panels::toolbar(app, ui);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        fixture(),
    );
    PhotocraftApp::setup_context(&h.ctx, kind);
    let ctx = h.ctx.clone();
    h.state_mut().run("prefs.set", json!({"path":"interface.theme","value":kind.id()})).unwrap();
    crate::prefs_ui::load(h.state_mut());
    h.state_mut().set_theme(&ctx, kind);
    let app = h.state_mut();
    app.ui.tool = Tool::Brush;
    app.sync_tool_brush();
    app.ui.tool_options.move_auto_select = false;
    app.ui.tool_options.move_show_transform = true;
    app.ui.extras.snap = false;
    app.ui.extras.rulers = false;
    app.ui.view.show.smart_guides = false;
    app.ui.view.show.layer_edges = true;
    h.run_steps(5);
    let view = &mut h.state_mut().ui.views[0];
    view.zoom = 1.0;
    view.center = [160.0, 120.0];
    view.fit_pending = false;
    h.run_steps(3);
    h
}

fn xf(h: &Harness<'_, PhotocraftApp>) -> ViewXform {
    let app = h.state();
    let view = &app.ui.views[0];
    ViewXform {
        rotation: view.rotation,
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: view.zoom,
        center: view.center,
        flip: app.ui.view.flip_horizontal,
    }
}

fn motion(h: &mut Harness<'_, PhotocraftApp>, at: [f32; 2], modifiers: Modifiers) {
    let point = xf(h).to_screen(at[0], at[1]);
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(point));
    h.run_steps(1);
}

fn button(h: &mut Harness<'_, PhotocraftApp>, at: [f32; 2], pressed: bool, modifiers: Modifiers) {
    let point = xf(h).to_screen(at[0], at[1]);
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(point));
    h.event(Event::PointerButton { pos: point, button: PointerButton::Primary, pressed, modifiers });
    h.run_steps(1);
}

fn has_outline(h: &Harness<'_, PhotocraftApp>, bounds: DRect) -> bool {
    let expected = xf(h).doc_corners(bounds);
    fn contains(shape: &egui::Shape, expected: &[Pos2; 4]) -> bool {
        match shape {
            egui::Shape::Path(path) => {
                path.closed && path.points.len() == 4 && expected.iter().all(|point| path.points.iter().any(|p| p.distance(*point) < 0.8))
            }
            egui::Shape::Rect(shape) if shape.stroke.width > 0.0 => {
                let corners = [shape.rect.left_top(), shape.rect.right_top(), shape.rect.right_bottom(), shape.rect.left_bottom()];
                expected.iter().all(|point| corners.iter().any(|p| p.distance(*point) < 0.8))
            }
            egui::Shape::Vec(items) => items.iter().any(|shape| contains(shape, expected)),
            _ => false,
        }
    }
    h.output().shapes.iter().any(|shape| contains(&shape.shape, &expected))
}

fn ids(app: &PhotocraftApp) -> (LayerId, LayerId) {
    let doc = &app.session.active().unwrap().doc;
    (doc.layers[1].id, doc.layers[2].id)
}

fn bounds(app: &PhotocraftApp, layer: LayerId) -> DRect {
    app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().content_bounds()
}

#[test]
fn ctrl_move_outline_v75_hover_and_first_pick_frame_never_paint_previous_layer_frame() {
    for saved in [Tool::Brush, Tool::Move] {
        let mut h = harness(ThemeKind::StudioLight, false);
        h.state_mut().ui.tool = saved;
        let (red, blue) = ids(h.state());
        let history = h.state().session.active().unwrap().history.past_len();
        motion(&mut h, [220.0, 170.0], Modifiers::CTRL);
        assert!(controls_match_pick(h.state(), &h.ctx, &xf(&h)), "the current target keeps its proper controls");
        assert!(has_outline(&h, DRect::new(200, 150, 250, 200)));
        motion(&mut h, [150.0, 110.0], Modifiers::CTRL);
        assert!(!controls_match_pick(h.state(), &h.ctx, &xf(&h)));
        assert!(!has_outline(&h, DRect::new(200, 150, 250, 200)), "empty pickup space has no previous-layer frame");
        motion(&mut h, [60.0, 60.0], Modifiers::CTRL);
        assert_eq!(h.state().session.active().unwrap().active_layer, Some(blue), "hover never changes selection/history");
        assert!(has_outline(&h, DRect::new(40, 40, 100, 100)), "Ctrl candidate receives the read-only edge");
        assert!(!has_outline(&h, DRect::new(200, 150, 250, 200)), "the previous layer cannot keep its frame");
        assert!(!controls_match_pick(h.state(), &h.ctx, &xf(&h)));
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
        button(&mut h, [60.0, 60.0], true, Modifiers::CTRL);
        motion(&mut h, [80.0, 70.0], Modifiers::CTRL);
        assert_eq!(h.state().session.active().unwrap().active_layer, Some(red));
        assert!(h.state().ui.transform.is_none(), "a different layer's handles cannot intercept pickup");
        assert!(has_outline(&h, DRect::new(60, 50, 120, 110)), "even the first Auto-Select/drag frame paints current translated bounds");
        assert!(!has_outline(&h, DRect::new(200, 150, 250, 200)));
        assert!(!has_outline(&h, DRect::new(220, 160, 270, 210)), "the pre-pick layer cannot leave a shifted snap rectangle");
        button(&mut h, [80.0, 70.0], false, Modifiers::NONE);
        h.run_steps(3);
        assert_eq!(h.state().ui.tool, saved);
        assert!(!h.state().ui.tool_options.move_auto_select);
    }
}

#[test]
fn ctrl_move_outline_v75_edge_tracks_drag_and_ctrl_release_preserves_tool_and_brush() {
    let mut h = harness(ThemeKind::Pro, false);
    let (red, blue) = ids(h.state());
    let brush = h.state().session.tools.brush.clone();
    button(&mut h, [60.0, 60.0], true, Modifiers::CTRL);
    for (at, expected, modifiers) in
        [([80.0, 70.0], DRect::new(60, 50, 120, 110), Modifiers::CTRL), ([110.0, 90.0], DRect::new(90, 70, 150, 130), Modifiers::NONE)]
    {
        motion(&mut h, at, modifiers);
        assert_eq!(crate::hold_keys::effective_tool(h.state(), &h.ctx), Tool::Move);
        assert!(has_outline(&h, expected));
        assert!(!has_outline(&h, DRect::new(40, 40, 100, 100)), "the source location must not remain outlined");
        assert!(!has_outline(&h, DRect::new(200, 150, 250, 200)));
        let previous_moved = DRect::new(200, 150, 250, 200).translate(expected.x0 - 40, expected.y0 - 40);
        assert!(!has_outline(&h, previous_moved), "neither Path nor Rect may outline the old layer at this drag offset");
    }
    button(&mut h, [110.0, 90.0], false, Modifiers::NONE);
    h.run_steps(3);
    assert_eq!(bounds(h.state(), red), DRect::new(90, 70, 150, 130));
    assert_eq!(bounds(h.state(), blue), DRect::new(200, 150, 250, 200));
    assert_eq!(crate::hold_keys::effective_tool(h.state(), &h.ctx), Tool::Brush);
    assert_eq!(h.state().session.tools.brush, brush);
    assert!(h.state().drag.is_none());
    assert!(!h.state().session.journal.iter().any(|(id, _)| id.starts_with("paint.")));
}

#[test]
fn ctrl_move_outline_v75_snap_rect_and_exclusion_follow_pick_when_layer_edges_are_hidden() {
    use photocraft_engine::snap::SnapKind;

    for saved in [Tool::Brush, Tool::Move] {
        let mut h = harness(ThemeKind::ProMedium, false);
        h.state_mut().ui.tool = saved;
        h.state_mut().ui.view.show.layer_edges = false;
        h.state_mut().ui.extras.snap = true;
        h.state_mut().ui.view.snap_to.layers = true;
        let (red, blue) = ids(h.state());
        button(&mut h, [60.0, 60.0], true, Modifiers::CTRL);
        motion(&mut h, [100.0, 80.0], Modifiers::CTRL);
        assert_eq!(h.state().session.active().unwrap().active_layer, Some(red));
        let snap = h.state().prefs_rt.snap.as_ref().expect("the real Move input creates snap state");
        assert_eq!(snap.gesture, crate::snap_ui::Gesture::Move { rect: [40.0, 40.0, 100.0, 100.0] });
        assert_eq!(snap.start, [60.0, 60.0]);
        let layer_edges: Vec<_> = snap.targets.x.iter().filter(|target| target.kind == SnapKind::LayerEdge).map(|target| target.pos).collect();
        assert!(layer_edges.contains(&200.0) && layer_edges.contains(&250.0), "the stationary old layer remains a snap target");
        assert!(!layer_edges.contains(&40.0) && !layer_edges.contains(&100.0), "the newly picked moving layer is excluded");
        assert!(has_outline(&h, DRect::new(80, 60, 140, 120)), "normal snap drag bounds remain visible with Layer Edges off");
        assert!(!has_outline(&h, DRect::new(240, 170, 290, 220)), "the old target's shifted rectangle is forbidden");
        assert_eq!(bounds(h.state(), blue), DRect::new(200, 150, 250, 200));
        button(&mut h, [100.0, 80.0], false, Modifiers::NONE);
        h.run_steps(3);
        assert_eq!(bounds(h.state(), red), DRect::new(80, 60, 140, 120));
        assert!(h.state().prefs_rt.snap.is_none());
        assert_eq!(h.state().ui.tool, saved);
    }
}

#[test]
fn ctrl_move_outline_v75_snap_rect_uses_full_selected_region_and_existing_float_offset() {
    let mut h = harness(ThemeKind::StudioLight, false);
    let (red, blue) = ids(h.state());
    h.state_mut().run("layer.select", json!({"layer":red.0})).unwrap();
    h.state_mut().run("select.rect", json!({"x":30,"y":30,"width":100,"height":100})).unwrap();
    h.state_mut().run("select.float", json!({"dx":20,"dy":10})).unwrap();
    h.state_mut().ui.tool_options.move_show_transform = false;
    button(&mut h, [90.0, 80.0], true, Modifiers::CTRL);
    motion(&mut h, [100.0, 90.0], Modifiers::CTRL);
    assert!(h.state().drag.as_ref().is_some_and(|drag| drag.tool == Tool::Move && drag.sel_move == Some(true)));
    let snap = h.state().prefs_rt.snap.as_ref().unwrap();
    assert_eq!(snap.gesture, crate::snap_ui::Gesture::Move { rect: [50.0, 40.0, 150.0, 140.0] });
    assert_eq!(layer_edge_bounds(h.state(), &h.ctx, &xf(&h), 0, true), Some(DRect::new(60, 50, 160, 150)));
    assert!(has_outline(&h, DRect::new(60, 50, 160, 150)), "the selected region retains its existing float offset and this drag's delta");
    assert!(!has_outline(&h, DRect::new(70, 60, 130, 120)), "a pre-pick layer-content box cannot accompany the selection frame");
    assert_eq!(bounds(h.state(), blue), DRect::new(200, 150, 250, 200));
    button(&mut h, [100.0, 90.0], false, Modifiers::NONE);
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().doc.selection.as_ref().unwrap().content_bounds(), DRect::new(60, 50, 160, 150));
}

#[test]
fn ctrl_move_outline_v75_preview_key_and_targets_change_even_without_document_revision() {
    let mut app = fixture();
    let (red, blue) = ids(&app);
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = false;
    app.run("layer.select", json!({"layer":red.0})).unwrap();
    app.drag = Some(Drag::new(Tool::Move, [50.0, 50.0], vec![[50.0, 50.0, 1.0], [60.0, 55.0, 1.0]], Modifiers::NONE, false));
    let (old, old_key) = crate::move_ui::display_doc(&mut app, 0).unwrap();
    assert_eq!(old.layer(red).unwrap().surface().unwrap().content_bounds(), DRect::new(50, 45, 110, 105));
    let revision = app.session.active().unwrap().revision;
    // Deliberately model an external/same-revision target replacement. The real
    // layer.select command does publish a new revision and is covered elsewhere.
    let state = app.session.active_mut().unwrap();
    state.active_layer = Some(blue);
    state.selected_layers = vec![blue];
    state.layer_anchor = Some(blue);
    assert_eq!(app.session.active().unwrap().revision, revision);
    let (current, current_key) = crate::move_ui::display_doc(&mut app, 0).unwrap();
    assert_ne!(old_key, current_key, "GPU/CPU cache identity must change with pickup targets");
    assert_eq!(current.layer(red).unwrap().surface().unwrap().content_bounds(), DRect::new(40, 40, 100, 100));
    assert_eq!(current.layer(blue).unwrap().surface().unwrap().content_bounds(), DRect::new(210, 155, 260, 205));
    assert_eq!(crate::move_ui::damage(&app, current.id, revision, old_key, current_key), None, "old target key requires full fresh composite");
}

#[test]
fn ctrl_move_outline_v75_multi_layer_and_group_drag_keep_current_union() {
    for group in [false, true] {
        let mut h = harness(ThemeKind::Classic, false);
        let (red, blue) = ids(h.state());
        h.state_mut().run("layer.select", json!({"layer":red.0})).unwrap();
        h.state_mut().run("layer.select", json!({"layer":blue.0,"mode":"add"})).unwrap();
        if group {
            h.state_mut().run("layer.new.group", json!({"name":"Two subjects"})).unwrap();
            h.state_mut().ui.tool_options.move_target = "group".into();
        }
        h.state_mut().ui.tool_options.move_show_transform = false;
        button(&mut h, [60.0, 60.0], true, Modifiers::CTRL);
        motion(&mut h, [80.0, 70.0], Modifiers::CTRL);
        assert!(has_outline(&h, DRect::new(60, 50, 270, 210)));
        button(&mut h, [80.0, 70.0], false, Modifiers::NONE);
        h.run_steps(3);
        assert_eq!(bounds(h.state(), red), DRect::new(60, 50, 120, 110));
        assert_eq!(bounds(h.state(), blue), DRect::new(220, 160, 270, 210));
    }
}

#[test]
fn ctrl_move_outline_v75_selected_pixels_and_existing_free_transform_keep_their_owner() {
    let mut h = harness(ThemeKind::Studio, false);
    let (red, blue) = ids(h.state());
    h.state_mut().run("layer.select", json!({"layer":red.0})).unwrap();
    // Keep the centre outside the visible transform handles' 12px pickup radius.
    // In a 20px square its centre hits a midpoint handle and correctly starts Free Transform.
    h.state_mut().run("select.rect", json!({"x":50,"y":50,"width":40,"height":40})).unwrap();
    button(&mut h, [70.0, 70.0], true, Modifiers::CTRL);
    motion(&mut h, [90.0, 80.0], Modifiers::CTRL);
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(red));
    assert!(h.state().ui.transform.is_none(), "an interior press must move pixels rather than pick a transform handle");
    assert!(h.state().drag.as_ref().is_some_and(|drag| drag.tool == Tool::Move && drag.sel_move == Some(true)));
    assert_eq!(layer_edge_bounds(h.state(), &h.ctx, &xf(&h), 0, true), Some(DRect::new(70, 60, 110, 100)));
    assert!(has_outline(&h, DRect::new(70, 60, 110, 100)));
    assert!(!has_outline(&h, DRect::new(200, 150, 250, 200)));
    button(&mut h, [90.0, 80.0], false, Modifiers::NONE);
    h.run_steps(3);
    assert_eq!(bounds(h.state(), blue), DRect::new(200, 150, 250, 200));
    assert_eq!(h.state().session.active().unwrap().doc.selection.as_ref().unwrap().content_bounds(), DRect::new(70, 60, 110, 100));
    h.state_mut().run("select.deselect", json!({})).unwrap();
    let ctx = h.ctx.clone();
    crate::transform_tool::begin(h.state_mut(), &ctx).unwrap();
    let quad = h.state().ui.transform.as_ref().unwrap().quad;
    motion(&mut h, [220.0, 170.0], Modifiers::CTRL);
    assert_eq!(h.state().ui.transform.as_ref().unwrap().quad, quad);
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(red));
    assert_eq!(layer_edge_bounds(h.state(), &h.ctx, &xf(&h), 0, true), None, "real transform supplies its own current outline");
}

#[test]
#[ignore = "writes five themes of actual Ctrl hover, pickup/drag, and restored-tool production canvases"]
fn ctrl_move_outline_gpu_visual_v75() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v75/ctrl-move-outline-gpu");
    assert!(!out.exists(), "preserve prior visual candidates");
    std::fs::create_dir_all(&out).unwrap();
    let mut manifest = Vec::new();
    for kind in ThemeKind::ALL {
        let mut h = harness(kind, true);
        motion(&mut h, [60.0, 60.0], Modifiers::CTRL);
        for phase in ["hover", "drag", "released"] {
            match phase {
                "drag" => {
                    button(&mut h, [60.0, 60.0], true, Modifiers::CTRL);
                    motion(&mut h, [100.0, 80.0], Modifiers::CTRL);
                }
                "released" => {
                    button(&mut h, [100.0, 80.0], false, Modifiers::NONE);
                }
                _ => {}
            }
            h.run_steps(3);
            assert_eq!(h.state().ui.theme, kind);
            assert_eq!(crate::theme::Tokens::get(&h.ctx).kind, kind);
            assert_eq!(h.state().session.prefs_value()["interface"]["theme"], json!(kind.id()));
            let expected = if phase == "hover" { DRect::new(40, 40, 100, 100) } else { DRect::new(80, 60, 140, 120) };
            assert!(has_outline(&h, expected), "the current red target must have its actual frame in {phase}");
            assert!(!has_outline(&h, DRect::new(240, 170, 290, 220)), "the shifted previous blue target must never have a ghost frame");
            let file = format!("{}-{phase}.png", kind.id());
            h.render().unwrap().save(out.join(&file)).unwrap();
            assert!(!has_outline(&h, DRect::new(200, 150, 250, 200)));
            manifest.push(json!({"file":file,"phase":phase,"theme":kind.id(),"actualUiTheme":h.state().ui.theme.id(),"actualTokenTheme":crate::theme::Tokens::get(&h.ctx).kind.id(),"preferenceTheme":h.state().session.prefs_value()["interface"]["theme"],"savedTool":h.state().ui.tool,"effectiveTool":crate::hold_keys::effective_tool(h.state(),&h.ctx),"activeLayer":h.state().session.active().unwrap().active_layer.map(|id|id.0)}));
        }
    }
    std::fs::write(out.join("theme-manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}
