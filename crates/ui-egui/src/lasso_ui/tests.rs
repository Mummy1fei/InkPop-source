use super::*;
use crate::canvas::tool_event;
use serde_json::json;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Lasso;
    app
}

fn event(app: &mut PhotocraftApp, kind: &str, x: f64, y: f64, mods: Modifiers) {
    let ev = match kind {
        "down" => ToolEvent::Down { x, y, pressure: 1.0 },
        "up" => ToolEvent::Up { x, y },
        _ => ToolEvent::Move { x, y, pressure: 1.0 },
    };
    tool_event(app, ev, mods);
}

fn begin(app: &mut PhotocraftApp, mods: Modifiers) {
    event(app, "down", 50.0, 50.0, mods);
    event(app, "move", 90.0, 40.0, mods);
    event(app, "move", 120.0, 50.0, mods);
}

#[test]
fn ctrl_clicks_preserve_outline_and_hover_does_not_draw() {
    let mut app = app();
    begin(&mut app, Modifiers::NONE);
    event(&mut app, "up", 120.0, 50.0, Modifiers::CTRL);
    assert!(app.session.active().unwrap().doc.selection.is_none());
    event(&mut app, "move", 170.0, 10.0, Modifiers::CTRL);
    event(&mut app, "move", 180.0, 100.0, Modifiers::CTRL);
    assert_eq!(app.drag.as_ref().unwrap().points.len(), 3);
    event(&mut app, "down", 180.0, 100.0, Modifiers::CTRL);
    event(&mut app, "up", 180.0, 100.0, Modifiers::CTRL);
    assert_eq!(app.drag.as_ref().unwrap().points.len(), 4);
    assert_eq!(crate::tool_feedback::badge(&app, Tool::Lasso, Modifiers::CTRL), None);
    // Release Ctrl between clicks, then continue a freehand stroke in the same outline.
    event(&mut app, "move", 140.0, 150.0, Modifiers::NONE);
    assert_eq!(app.drag.as_ref().unwrap().points.len(), 4);
    event(&mut app, "down", 130.0, 140.0, Modifiers::NONE);
    event(&mut app, "move", 90.0, 130.0, Modifiers::NONE);
    event(&mut app, "up", 50.0, 120.0, Modifiers::NONE);
    assert!(!active(&app));
    let sel = app.session.active().unwrap().doc.selection.as_ref().unwrap();
    assert!(sel.sample_channel(100, 80, 0) > 0.9);
    assert!(sel.sample_channel(160, 20, 0) < 0.1, "hover detour must stay outside");
    app.run("edit.undo", json!({})).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_none(), "one selection undo");
}

#[test]
fn switching_back_while_pressed_fixes_the_straight_endpoint() {
    let mut app = app();
    begin(&mut app, Modifiers::NONE);
    event(&mut app, "move", 180.0, 100.0, Modifiers::CTRL);
    assert_eq!(app.drag.as_ref().unwrap().points.len(), 3);
    event(&mut app, "move", 170.0, 130.0, Modifiers::NONE);
    let pts = &app.drag.as_ref().unwrap().points;
    assert_eq!(&pts[3..], &[[180.0, 100.0, 1.0], [170.0, 130.0, 1.0]]);
    event(&mut app, "up", 50.0, 120.0, Modifiers::NONE);
    assert!(app.session.active().unwrap().doc.selection.is_some());
}

#[test]
fn initial_alt_still_subtracts_and_initial_shift_still_adds() {
    for initial in [Modifiers::ALT, Modifiers::SHIFT, Modifiers::SHIFT | Modifiers::ALT] {
        let mut app = app();
        app.run("select.rect", json!({"x": 0, "y": 0, "width": 150, "height": 200})).unwrap();
        begin(&mut app, initial);
        event(&mut app, "up", 120.0, 50.0, Modifiers::CTRL);
        event(&mut app, "down", 180.0, 150.0, Modifiers::CTRL);
        event(&mut app, "up", 180.0, 150.0, Modifiers::CTRL);
        commit(&mut app);
        let sel = app.session.active().unwrap().doc.selection.as_ref().unwrap();
        assert_eq!(sel.sample_channel(120, 90, 0) > 0.5, initial.shift);
        assert_eq!(sel.sample_channel(10, 10, 0) > 0.5, !(initial.shift && initial.alt));
    }
}

#[test]
fn tool_or_document_switch_discards_pending_outline() {
    let mut app = app();
    begin(&mut app, Modifiers::NONE);
    event(&mut app, "up", 120.0, 50.0, Modifiers::CTRL);
    app.ui.tool = Tool::Brush;
    app.sync_views();
    assert!(!active(&app));
    app.ui.tool = Tool::Lasso;
    begin(&mut app, Modifiers::NONE);
    event(&mut app, "up", 120.0, 50.0, Modifiers::CTRL);
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    commit(&mut app);
    assert!(!active(&app));
    assert!(app.session.active().unwrap().doc.selection.is_none());
}

fn harness() -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::shortcuts::handle(app, &ctx);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.fit_pending = false;
    h.run_steps(2);
    h
}

fn mouse(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, kind: &str, x: f32, y: f32, mods: Modifiers) {
    let app = h.state();
    let v = &app.ui.views[0];
    let xf = ViewXform { rotation: v.rotation, rect: crate::rulers::content_rect(app, app.last_canvas_rect), zoom: v.zoom, center: v.center, flip: false };
    let pos = xf.to_screen(x, y);
    h.event(Event::ModifiersChanged(mods));
    h.event(Event::PointerMoved(pos));
    if kind != "move" {
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: kind == "down", modifiers: mods });
    }
    h.run_steps(2);
}

#[test]
fn real_canvas_ctrl_clicks_enter_escape_and_resume() {
    for finish in ["enter", "escape", "resume", "close"] {
        let mut h = harness();
        mouse(&mut h, "down", 50.0, 50.0, Modifiers::NONE);
        mouse(&mut h, "move", 120.0, 50.0, Modifiers::NONE);
        mouse(&mut h, "up", 120.0, 50.0, Modifiers::CTRL);
        assert!(active(h.state()), "Ctrl-held pointer release must keep gesture");
        mouse(&mut h, "move", 180.0, 100.0, Modifiers::CTRL);
        mouse(&mut h, "down", 180.0, 100.0, Modifiers::CTRL);
        mouse(&mut h, "up", 180.0, 100.0, Modifiers::CTRL);
        assert_eq!(h.state().drag.as_ref().unwrap().points.len(), 3);
        match finish {
            "resume" => {
                mouse(&mut h, "down", 150.0, 150.0, Modifiers::NONE);
                mouse(&mut h, "move", 90.0, 130.0, Modifiers::NONE);
                mouse(&mut h, "up", 50.0, 120.0, Modifiers::NONE);
            }
            "close" => {
                mouse(&mut h, "down", 50.0, 50.0, Modifiers::CTRL);
                mouse(&mut h, "up", 50.0, 50.0, Modifiers::CTRL);
            }
            _ => {
                h.event(Event::Key {
                    key: if finish == "escape" { egui::Key::Escape } else { egui::Key::Enter },
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::CTRL,
                });
                h.run_steps(2);
            }
        }
        assert!(!active(h.state()), "{finish}");
        assert_eq!(h.state().session.active().unwrap().doc.selection.is_some(), finish != "escape", "{finish}");
    }
}

/// A lasso press inside the selection still drags the selection (`canvas::selection_drag_kind`),
/// both through tool events and through the real canvas.
#[test]
fn a_drag_inside_the_selection_moves_it() {
    let mut app = app();
    app.ui.extras.snap = false;
    app.run("select.rect", json!({"x": 100, "y": 100, "width": 50, "height": 40})).unwrap();
    let bounds = |app: &PhotocraftApp| app.session.active().unwrap().doc.selection.as_ref().map(|s| s.content_bounds());
    event(&mut app, "down", 120.0, 120.0, Modifiers::NONE);
    event(&mut app, "move", 130.0, 125.0, Modifiers::NONE);
    event(&mut app, "up", 130.0, 125.0, Modifiers::NONE);
    assert!(!active(&app));
    assert_eq!(bounds(&app), Some(photocraft_geom::Rect::new(110, 105, 160, 145)));

    let mut h = harness();
    h.state_mut().ui.extras.snap = false;
    h.state_mut().run("select.rect", json!({"x": 100, "y": 100, "width": 50, "height": 40})).unwrap();
    h.run_steps(1);
    mouse(&mut h, "down", 120.0, 120.0, Modifiers::NONE);
    mouse(&mut h, "move", 125.0, 122.0, Modifiers::NONE);
    mouse(&mut h, "move", 130.0, 125.0, Modifiers::NONE);
    mouse(&mut h, "up", 130.0, 125.0, Modifiers::NONE);
    assert!(h.state().drag.is_none());
    assert_eq!(bounds(h.state()), Some(photocraft_geom::Rect::new(110, 105, 160, 145)));
}

#[test]
fn idle_ctrl_temporarily_moves_selected_pixels_and_restores_lasso_v74() {
    let mut h = harness();
    h.state_mut().ui.extras.snap = false;
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    h.state_mut().run("select.rect", json!({"x": 100, "y": 100, "width": 50, "height": 40})).unwrap();
    h.state_mut().run("edit.fill", json!({"color":"#ee2244"})).unwrap();
    let layer = h.state().session.active().unwrap().active_layer.unwrap();
    let before = h.state().session.active().unwrap().history.past_len();
    let alpha = |app: &PhotocraftApp, x, y| app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().rgba(x, y)[3];
    let ctrl = Modifiers::CTRL | Modifiers::COMMAND; // Windows event mapping.
    mouse(&mut h, "down", 120.0, 120.0, ctrl);
    mouse(&mut h, "move", 160.0, 120.0, ctrl);
    mouse(&mut h, "move", 180.0, 140.0, ctrl);
    assert!(!active(h.state()), "an idle Ctrl press belongs to temporary Move, not a polygonal outline");
    assert_eq!(h.state().drag.as_ref().unwrap().tool, Tool::Move);
    assert!(h.state().drag.as_ref().unwrap().lasso.is_none());
    mouse(&mut h, "up", 180.0, 140.0, ctrl);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
    assert!(h.state().drag.is_none());
    assert!(photocraft_engine::float_cmds::floating(h.state().session.active().unwrap()).is_none(), "Move release commits the selected pixels");
    assert_eq!(alpha(h.state(), 120, 120), 0.0);
    assert_eq!(alpha(h.state(), 180, 140), 1.0);
    assert_eq!(h.state().ui.tool, Tool::Lasso);
    assert_eq!(crate::hold_keys::effective_tool(h.state(), &h.ctx), Tool::Lasso);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before + 1);
    let sel = h.state().session.active().unwrap().doc.selection.as_ref().unwrap();
    assert_eq!(sel.content_bounds(), photocraft_geom::Rect::new(160, 120, 210, 160));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.active().unwrap().doc.selection.as_ref().unwrap().content_bounds(), photocraft_geom::Rect::new(100, 100, 150, 140));
    assert_eq!(alpha(h.state(), 120, 120), 1.0);
    assert_eq!(alpha(h.state(), 180, 140), 0.0);
}

#[test]
fn alt_subtracts_without_suppressing_freehand_samples() {
    let mut app = app();
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 250, "height": 250})).unwrap();
    begin(&mut app, Modifiers::ALT);
    event(&mut app, "move", 120.0, 150.0, Modifiers::ALT);
    assert_eq!(app.drag.as_ref().unwrap().points.len(), 4);
    assert!(!app.drag.as_ref().unwrap().lasso.as_ref().unwrap().polygonal);
    event(&mut app, "up", 50.0, 150.0, Modifiers::ALT);
    assert!(!active(&app), "Alt is only subtract; releasing the pointer completes freehand");
    let selection = app.session.active().unwrap().doc.selection.as_ref().unwrap();
    assert!(selection.sample_channel(80, 100, 0) < 0.1);
    assert!(selection.sample_channel(10, 10, 0) > 0.9);
}

fn options_harness(theme: crate::theme::ThemeKind, tool: Tool, gpu: bool) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut app = app();
    app.ui.tool = tool;
    app.ui.extras.rulers = false;
    let builder = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 650.0));
    let builder = if gpu { builder.wgpu() } else { builder };
    let mut h = builder.build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(8);
    h
}

fn mode_labels() -> [String; 4] {
    [
        "New selection".into(),
        format!("Add to selection  ({})", crate::shortcuts::pretty("Shift")),
        format!("Subtract from selection  ({})", crate::shortcuts::pretty("Alt")),
        format!("Intersect with selection  ({})", crate::shortcuts::pretty("Shift+Alt")),
    ]
}

fn paints_text(h: &egui_kittest::Harness<'_, PhotocraftApp>, label: &str) -> bool {
    fn contains(shape: &egui::Shape, label: &str) -> bool {
        match shape {
            egui::Shape::Text(text) => text.galley.job.text == label,
            egui::Shape::Vec(items) => items.iter().any(|shape| contains(shape, label)),
            _ => false,
        }
    }
    h.output().shapes.iter().any(|shape| contains(&shape.shape, label))
}

#[test]
fn all_themes_offer_four_lasso_modes_and_show_effective_modifier_mode() {
    use egui_kittest::kittest::Queryable;
    for theme in crate::theme::ThemeKind::ALL {
        for tool in [Tool::Lasso, Tool::PolygonLasso, Tool::MagneticLasso] {
            let mut h = options_harness(theme, tool, false);
            let labels = mode_labels();
            for (i, label) in labels.iter().enumerate() {
                let rect = h.get_by_label(label).rect();
                assert!(rect.min.x >= 0.0 && rect.max.x <= 1000.0 && rect.min.y >= 0.0 && rect.max.y <= 42.0, "{theme:?} {tool:?} {label}: {rect:?}");
                h.get_by_label(label).click();
                h.run_steps(2);
                assert_eq!(h.state().ui.selection_mode, i as u8, "{theme:?} {tool:?} {label}");
            }
            h.state_mut().ui.selection_mode = 0;
            for (mods, index) in [(Modifiers::SHIFT, 1), (Modifiers::ALT, 2), (Modifiers::SHIFT | Modifiers::ALT, 3)] {
                h.event(Event::ModifiersChanged(mods));
                h.run_steps(2);
                let rect = h.get_by_label(&labels[index]).rect();
                let selected = crate::theme::Tokens::get(&h.ctx).accent_soft;
                assert!(
                    h.output().shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.rect == rect && r.fill == selected)),
                    "{theme:?} {tool:?}: effective mode {index} must be highlighted"
                );
                assert_eq!(h.state().ui.selection_mode, 0, "temporary modifiers must not overwrite the persistent mode");
            }
            h.event(Event::ModifiersChanged(Modifiers::CTRL));
            h.run_steps(2);
            assert!(paints_text(&h, tl!("Auto-Select:")), "idle Ctrl paints the temporary Move options in the real bar");
            assert_eq!(crate::hold_keys::effective_tool(h.state(), &h.ctx), Tool::Move);
            assert_eq!(h.state().ui.selection_mode, 0);
            assert_eq!(h.state().ui.tool, tool, "temporary Move never replaces the saved lasso tool");
            h.event(Event::ModifiersChanged(Modifiers::NONE));
            h.run_steps(2);
            for label in &labels {
                assert!(h.query_by_label(label).is_some(), "releasing Ctrl restores every lasso mode button");
            }
            assert!(!paints_text(&h, tl!("Auto-Select:")), "release removes the temporary Move controls from the painted bar");
            assert_eq!(crate::hold_keys::effective_tool(h.state(), &h.ctx), tool);
            assert_eq!(h.state().ui.selection_mode, 0);
        }
    }
}

#[test]
#[ignore = "writes five-theme lasso options and software cursor GPU artifacts"]
fn lasso_options_cursor_gpu_visual_v60() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v60/lasso-offscreen");
    assert!(!root.exists(), "preserve existing evidence");
    std::fs::create_dir_all(&root).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        for tool in [Tool::Lasso, Tool::PolygonLasso] {
            let mut h = options_harness(theme, tool, true);
            h.state_mut().ui.views[0].fit_pending = false;
            h.state_mut().ui.views[0].zoom = 1.5;
            h.run_steps(3);
            for (name, mods) in [("replace", Modifiers::NONE), ("add", Modifiers::SHIFT), ("subtract", Modifiers::ALT), ("polygon", Modifiers::CTRL)] {
                mouse(&mut h, "move", 150.0, 100.0, mods);
                h.run_steps(4);
                h.render().unwrap().save(root.join(format!("{}-{tool:?}-{name}.png", theme.id()))).unwrap();
            }
        }
    }
}
