//! Mouse buttons on the canvas. Tools follow the left button. The right button never paints with
//! the tool: with a painting tool it opens the Brush Preset picker at the pointer (Photoshop), or,
//! with Preferences › Tools › Right-click with painting tools set to Erase, a right drag with the
//! Brush erases with the current brush (Krita, Paint). Each right stroke is one `paint.stroke`
//! with `"erase": true`, so one undo step, with the pen pressure and tilt of a normal stroke.

use egui::{PointerButton, Response};
use photocraft_engine::prefs::RightClickPaint;

use crate::PhotocraftApp;
use crate::state::Tool;

/// Canvas and direct pointer events use the same complete brush transition as
/// the options bar. Smoothing is part of that snapshot, not a second restore
/// that can accidentally save the incoming value into the outgoing tool.
pub fn sync_tool_smoothing(app: &mut PhotocraftApp) {
    app.sync_tool_brush();
}

/// Which tool events this frame's canvas response produces.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Buttons {
    pub started: bool,
    pub dragged: bool,
    pub stopped: bool,
    pub clicked: bool,
}

/// Tools whose right-click opens the Brush Preset picker (those with the options-bar brush chip).
pub fn has_brush_picker(tool: Tool) -> bool {
    tool.is_brushlike() || tool == Tool::QuickSelection
}

/// Does a right-button drag with `tool` erase (rather than open the picker)?
pub fn right_erases(app: &PhotocraftApp, tool: Tool) -> bool {
    matches!(tool, Tool::Brush | Tool::Eraser) && app.session.prefs().tools.right_click_with_painting_tools == RightClickPaint::Erase
}

/// Route the canvas response's buttons: the left one drives the tool; the right one resizes the
/// brush with Alt held (#297), erases (Erase preference) or opens the Brush Preset picker. Arms
/// `secondary_erase` or `brush_resize_armed` for this frame's `Down`.
pub fn canvas_buttons(app: &mut PhotocraftApp, response: &Response, tool: Tool) -> Buttons {
    let (mods, primary_down, right_down, touch_contact, touch_released) = response.ctx.input(|i| {
        (
            i.modifiers,
            i.pointer.primary_down(),
            i.pointer.secondary_down(),
            // Contact keeps a live pen resize armed between emulated mouse events.
            i.events.iter().any(|event| matches!(event, egui::Event::Touch { .. })),
            i.events.iter().any(|event| matches!(event, egui::Event::Touch { phase: egui::TouchPhase::End | egui::TouchPhase::Cancel, .. })),
        )
    });
    // Alt+right-drag resizes the brush (brush_resize.rs); its events reach `tool_event` like a
    // left drag's, and nothing paints. A resize whose release was missed ends here.
    // A Touch Move may not update egui's emulated primary button on every frame. Treat the
    // contact itself as held so a live pen resize is not cleared while the stylus is still down.
    crate::brush_resize::release_stale_buttons(
        app,
        primary_down || touch_contact,
        right_down || response.drag_stopped_by(PointerButton::Secondary),
        touch_released,
    );
    let resize_start = crate::brush_resize::applies(tool)
        && app.drag.is_none()
        && crate::brush_resize::is_right_gesture(crate::workspace_ui::sticky_mods(app, mods))
        && response.drag_started_by(PointerButton::Secondary);
    let resizing = app.brush_resize.is_some_and(|r| r.secondary);
    app.brush_resize_armed = resize_start;
    // ⌘/Ctrl+right-click lists the layers under the pointer instead (layer_pick_ui.rs, #307).
    let layer_menu = crate::layer_pick_ui::is_gesture(tool, mods);
    let erase = right_erases(app, tool) && !resize_start && !resizing && !layer_menu;
    let right_stroke = erase && app.drag.is_some();
    let right_start = erase && response.drag_started_by(PointerButton::Secondary);
    // Open on the actual button press, not egui's completed click. A stylus can move farther
    // than the click threshold or keep the barrel held while hovering; neither should cancel
    // the picker or delay it until lift. Inspect raw events to exclude touch hold synthesis.
    let right_press = response.contains_pointer()
        && response.ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::PointerButton { button: PointerButton::Secondary, pressed: true, .. })));
    let right_click = response.clicked_by(PointerButton::Secondary);
    if right_press
        && !erase
        && !resize_start
        && !resizing
        && !(crate::brush_resize::applies(tool) && crate::brush_resize::is_right_gesture(crate::workspace_ui::sticky_mods(app, mods)))
        && !layer_menu
        // Free Transform keeps the previously selected painting tool. Its right-click
        // belongs to the transform mode menu, so do not steal focus before button release.
        && app.ui.transform.is_none()
        && has_brush_picker(tool)
        && let Some(p) = response.interact_pointer_pos()
        && let Some(document) = app.session.active().map(|st| st.doc.id)
    {
        crate::brush_picker::open_at(app, &response.ctx, response.ctx.viewport_id(), document, [p.x, p.y]);
    }
    let erase_click = erase && right_click;
    app.secondary_erase = right_start || erase_click;
    let right_drag = right_stroke || resizing;
    Buttons {
        started: response.drag_started_by(PointerButton::Primary) || right_start || resize_start,
        dragged: response.dragged_by(PointerButton::Primary) || (right_drag && response.dragged_by(PointerButton::Secondary)),
        stopped: response.drag_stopped_by(PointerButton::Primary) || (right_drag && response.drag_stopped_by(PointerButton::Secondary)),
        clicked: response.clicked() || erase_click,
    }
}

/// `ui.pointer` with `"button": "secondary"`: true when its events should reach the tool (an
/// Alt+right-drag brush resize, `brush_resize_armed` for its `Down`; or an erasing right stroke,
/// `secondary_erase` armed for its `Down`). Otherwise a right-click with a painting tool opens
/// the Brush Preset picker at screen point `at`, and nothing paints.
pub fn pointer_secondary(app: &mut PhotocraftApp, down: bool, mods: egui::Modifiers, at: [f32; 2]) -> bool {
    let tool = app.ui.tool;
    if crate::brush_resize::applies(tool) && (crate::brush_resize::is_right_gesture(mods) || app.brush_resize.is_some_and(|r| r.secondary)) {
        app.brush_resize_armed = down && app.drag.is_none();
        return true;
    }
    if right_erases(app, tool) {
        app.secondary_erase = down;
        return true;
    }
    if down && has_brush_picker(tool) {
        app.ui.brush_picker = Some(at);
    }
    false
}

/// The Brush Preset picker a right-click opened, at the pointer. It edits the session brush like
/// the options-bar chip's; a click outside, Escape or Enter closes it.
pub fn show_picker(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // The quick picker can be the only visible brush UI. Do not depend on a docked panel
    // enabling background previews before its first imported-tip thumbnail is requested.
    crate::brush_preview::background(ctx, app.background_jobs);
    if !ctx.embed_viewports() {
        crate::brush_picker::show_native(app, ctx);
        return;
    }
    let Some([x, y]) = app.ui.brush_picker else { return };
    if !has_brush_picker(app.ui.tool) || ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter)) {
        app.ui.brush_picker = None;
        return;
    }
    let screen = ctx.content_rect();
    // Keep the whole picker on screen: its last size, or about 320 × 480 points before it shows.
    let id = egui::Id::new("canvas-brush-picker");
    let size = ctx.memory(|m| m.area_rect(id)).map_or(egui::vec2(324.0, 480.0), |r| r.size());
    let pos = egui::pos2(x.min(screen.right() - size.x).max(screen.left()), y.min(screen.bottom() - size.y).max(screen.top()));
    let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        let before = app.session.tools.brush.clone();
        let mut b = before.clone();
        let pick = egui::Frame::popup(ui.style())
            .show(ui, |ui| {
                egui::Resize::default()
                    .id_salt("canvas-brush-picker-resize")
                    .default_size(egui::vec2(300.0, 450.0))
                    .min_size(egui::vec2(250.0, 260.0))
                    .max_size(screen.size() - egui::vec2(24.0, 24.0))
                    .show(ui, |ui| {
                        let available = ui.available_size();
                        crate::brush_picker::body_sized(app, ui, &mut b, available.x, available.y - 155.0)
                    })
            })
            .inner;
        crate::brush_panel::commit_gesture(app, ui.ctx(), &before, &b);
        if pick == Some(crate::brush_picker::Pick::OpenSettings) {
            app.ui.brush_picker = None;
        }
        crate::brush_picker::apply(app, ui.ctx(), pick);
    });
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
    if outside {
        app.ui.brush_picker = None;
    }
}

#[cfg(test)]
mod tests {
    use egui::{Modifiers, PointerButton, Pos2, vec2};
    use egui_kittest::Harness;
    use serde_json::json;

    use super::*;
    use crate::canvas::{ToolEvent, tool_event};

    fn harness(prefs: Option<&str>) -> Harness<'static, PhotocraftApp> {
        harness_with_step_dt(prefs, 1.0 / 4.0)
    }

    fn harness_with_step_dt(prefs: Option<&str>, step_dt: f32) -> Harness<'static, PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 20, "hardness": 1.0}})).unwrap();
        if let Some(v) = prefs {
            app.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": v})).unwrap();
        }
        app.ui.tool = Tool::Brush;
        app.sync_views();
        let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_step_dt(step_dt).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        h
    }

    fn press(h: &mut Harness<'static, PhotocraftApp>, pos: Pos2, button: PointerButton, pressed: bool) {
        h.event(egui::Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }

    /// Drag across the canvas centre with `button`; returns the document points it covered.
    fn drag(h: &mut Harness<'static, PhotocraftApp>, button: PointerButton) -> (Pos2, Pos2) {
        let c = h.state().last_canvas_rect.center();
        let (a, b) = (c - vec2(80.0, 0.0), c + vec2(80.0, 0.0));
        h.event(egui::Event::PointerMoved(a));
        h.run_steps(1);
        press(h, a, button, true);
        for i in 1..=8 {
            h.event(egui::Event::PointerMoved(a + (b - a) * (i as f32 / 8.0)));
            h.run_steps(1);
        }
        press(h, b, button, false);
        (a, b)
    }

    fn alpha_at(h: &Harness<'static, PhotocraftApp>, screen: Pos2) -> f32 {
        let app = h.state();
        let v = app.ui.views[0].clone();
        let r = app.last_canvas_rect;
        let d = (screen - r.center()) / v.zoom;
        let (x, y) = ((d.x + v.center[0]) as i32, (d.y + v.center[1]) as i32);
        let st = app.session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y)[3]
    }

    fn strokes(h: &Harness<'static, PhotocraftApp>) -> Vec<serde_json::Value> {
        h.state().session.journal.iter().filter(|(id, _)| id == "paint.stroke").map(|(_, p)| p.clone()).collect()
    }

    #[test]
    fn left_drag_paints() {
        let mut h = harness(None);
        let (a, b) = drag(&mut h, PointerButton::Primary);
        assert_eq!(strokes(&h).len(), 1);
        assert!(alpha_at(&h, a + (b - a) * 0.5) > 0.9);
    }

    #[test]
    fn shift_click_connects_to_the_previous_brush_stroke() {
        use egui::{Event, Modifiers};

        let mut h = harness(None);
        let (_, end) = drag(&mut h, PointerButton::Primary);
        let previous = h
            .state()
            .session
            .journal
            .iter()
            .rev()
            .find(|(id, _)| id == "paint.stroke")
            .map(|(_, p)| p["points"].as_array().unwrap().last().unwrap().clone())
            .unwrap();
        let target = end + vec2(120.0, 80.0);
        h.event(Event::PointerMoved(target));
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed, modifiers: Modifiers::SHIFT });
            h.run_steps(2);
        }

        let strokes = strokes(&h);
        assert_eq!(strokes.len(), 2, "a Shift-click commits one connected stroke");
        let points = strokes[1]["points"].as_array().unwrap();
        assert_eq!(points.len(), 2, "a click has the previous endpoint and clicked endpoint");
        assert_eq!(points[0][0], previous[0]);
        assert_eq!(points[0][1], previous[1]);
        assert!(alpha_at(&h, end + vec2(60.0, 40.0)) > 0.9, "the segment between the strokes is painted");
    }

    fn modified_press(h: &mut Harness<'static, PhotocraftApp>, pos: Pos2, pressed: bool, modifiers: Modifiers) {
        h.event(egui::Event::ModifiersChanged(modifiers));
        h.event(egui::Event::PointerMoved(pos));
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers });
        h.run_steps(2);
    }

    #[test]
    fn shift_two_clicks_connect_once_for_mouse_and_pen_v73() {
        for pen in [false, true] {
            let mut h = harness_with_step_dt(None, 1.0 / 60.0);
            let history_before = h.state().session.active().unwrap().history.past_len();
            if pen {
                h.state_mut().stylus.feed.set(Some(crate::stylus::PenSample { pressure: 0.7, tilt_x: 18.0, ..Default::default() }));
            }
            let c = h.state().last_canvas_rect.center();
            let a = c - vec2(70.0, 45.0);
            let b = c + vec2(70.0, 45.0);
            modified_press(&mut h, a, true, Modifiers::SHIFT);
            modified_press(&mut h, a, false, Modifiers::SHIFT);
            assert_eq!(strokes(&h).len(), 1, "first Shift click paints its own dab");
            modified_press(&mut h, b, true, Modifiers::SHIFT);
            assert_eq!(strokes(&h).len(), 1, "press never commits the previous-endpoint connector");
            assert_eq!(alpha_at(&h, c), 0.0, "press has not painted the connecting segment");
            modified_press(&mut h, b, false, Modifiers::SHIFT);
            let committed = strokes(&h);
            assert_eq!(committed.len(), 2, "one stroke per click, pen={pen}");
            assert_eq!(committed[0]["points"].as_array().unwrap().len(), 1);
            let line = committed[1]["points"].as_array().unwrap();
            assert_eq!(line.len(), 2, "release commits a single two-point line");
            assert_eq!(line[0][0], committed[0]["points"][0][0]);
            assert_eq!(line[0][1], committed[0]["points"][0][1]);
            assert!(alpha_at(&h, c) > 0.9, "the diagonal is actually painted, pen={pen}");
            if pen {
                assert!((line[1][2].as_f64().unwrap() - 0.7).abs() < 0.001, "line retains pen pressure");
                assert_eq!(line[1][3], json!(18.0), "line retains pen tilt");
            }
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history_before + 2, "one history entry per click");
        }
    }

    #[test]
    fn shift_pointer_drag_chooses_one_axis_without_an_old_connector_v73() {
        for (delta, horizontal) in [(vec2(160.0, 45.0), true), (vec2(40.0, 160.0), false)] {
            let mut h = harness_with_step_dt(None, 1.0 / 60.0);
            let c = h.state().last_canvas_rect.center();
            let old = c - vec2(100.0, 80.0);
            modified_press(&mut h, old, true, Modifiers::NONE);
            modified_press(&mut h, old, false, Modifiers::NONE);
            let start = c - vec2(70.0, 55.0);
            modified_press(&mut h, start, true, Modifiers::SHIFT);
            for step in 1..=8 {
                h.event(egui::Event::PointerMoved(start + delta * (step as f32 / 8.0)));
                h.run_steps(1);
            }
            modified_press(&mut h, start + delta, false, Modifiers::SHIFT);
            let committed = strokes(&h);
            assert_eq!(committed.len(), 2, "one constrained drag after the dab");
            let points = committed[1]["points"].as_array().unwrap();
            let axis = usize::from(horizontal);
            assert!(points.len() >= 2 && points.iter().all(|p| p[axis] == points[0][axis]), "one locked axis: {points:?}");
            let projected_middle = start + if horizontal { vec2(delta.x * 0.5, 0.0) } else { vec2(0.0, delta.y * 0.5) };
            assert!(alpha_at(&h, projected_middle) > 0.9, "the constrained segment is painted");
            assert_eq!(alpha_at(&h, old + (start - old) * 0.5), 0.0, "the drag must never connect to the old dab");
        }
    }

    #[test]
    fn ctrl_pointer_click_temporarily_picks_a_layer_without_painting_v74() {
        for modifiers in [Modifiers::CTRL, Modifiers::CTRL | Modifiers::COMMAND] {
            let mut h = harness_with_step_dt(None, 1.0 / 60.0);
            let history = h.state().session.active().unwrap().history.past_len();
            let brush = serde_json::to_value(&h.state().session.tools.brush).unwrap();
            let background = h.state().session.active().unwrap().doc.layers[0].id;
            h.state_mut().ui.tool_options.move_auto_select = false;
            let c = h.state().last_canvas_rect.center();
            let a = c - vec2(70.0, 45.0);
            let b = c + vec2(70.0, 45.0);
            for point in [a, b] {
                modified_press(&mut h, point, true, modifiers);
                modified_press(&mut h, point, false, modifiers);
            }
            let committed = strokes(&h);
            assert!(committed.is_empty(), "Ctrl is temporary Move; neither click paints a dab or connector");
            let app = h.state();
            assert_eq!(app.session.active().unwrap().active_layer, Some(background));
            assert_eq!(app.session.active().unwrap().history.past_len(), history, "layer pickup is not a pixel edit");
            assert_eq!(app.ui.tool, Tool::Brush);
            assert!(!app.ui.tool_options.move_auto_select, "temporary Auto-Select never changes its saved value");
            assert_eq!(serde_json::to_value(&app.session.tools.brush).unwrap(), brush);
            assert_eq!(app.session.active().unwrap().doc.layers[1].surface().unwrap().content_bounds(), photocraft_geom::Rect::EMPTY);
        }
    }

    #[test]
    fn right_click_opens_the_brush_picker_and_never_paints() {
        let mut h = harness(None);
        let undo = h.state().session.active().unwrap().history.past_len();
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        press(&mut h, c, PointerButton::Secondary, true);
        press(&mut h, c, PointerButton::Secondary, false);
        assert_eq!(h.state().ui.brush_picker, Some([c.x, c.y]), "opened at the pointer");
        assert!(strokes(&h).is_empty());
        // The picker shows over the canvas at the pointer, moved up just enough to stay on screen.
        h.run_steps(2);
        let shown = h.ctx.memory(|m| m.area_rect(egui::Id::new("canvas-brush-picker"))).expect("picker shown");
        let screen = h.ctx.content_rect();
        assert!((shown.min.x - c.x).abs() < 1.0 && shown.min.y <= c.y + 1.0, "{shown:?}");
        assert!(screen.contains_rect(shown.shrink(0.5)) && shown.width() > 250.0 && shown.height() > 150.0, "{shown:?}");
        // It is the real preset library, not a fixed set of round tips (#258).
        let names: Vec<String> = h.state().session.tools.presets.iter().take(3).map(|p| p.name.clone()).collect();
        {
            use egui_kittest::kittest::Queryable;
            for n in &names {
                assert!(h.query_by_label(n).is_some(), "{n} listed");
            }
        }
        // A right drag doesn't paint either.
        drag(&mut h, PointerButton::Secondary);
        assert!(strokes(&h).is_empty(), "right-drag never paints with the brush picker preference");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), undo, "no edit");
        // Escape closes it.
        h.state_mut().ui.brush_picker = Some([c.x, c.y]);
        h.run_steps(1);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert_eq!(h.state().ui.brush_picker, None);
    }

    #[test]
    fn pen_side_button_opens_picker_without_painting() {
        let mut h = harness(None);
        let c = h.state().last_canvas_rect.center();
        let undo = h.state().session.active().unwrap().history.past_len();
        h.state_mut().stylus.feed.set(Some(crate::stylus::PenSample { pressure: 0.8, ..Default::default() }));
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        press(&mut h, c, PointerButton::Secondary, true);
        assert!(h.state().ui.brush_picker.is_some(), "barrel press opens before release");
        h.event(egui::Event::PointerMoved(c + vec2(48.0, 36.0)));
        h.run_steps(2);
        assert!(h.state().ui.brush_picker.is_some(), "pen motion beyond the click threshold does not cancel the picker");
        press(&mut h, c + vec2(48.0, 36.0), PointerButton::Secondary, false);
        assert!(h.state().ui.brush_picker.is_some());
        assert!(strokes(&h).is_empty());
        assert_eq!(h.state().session.active().unwrap().history.past_len(), undo);
    }

    #[test]
    fn alt_secondary_press_reserves_brush_resize_without_opening_picker() {
        let mut h = harness(None);
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::ModifiersChanged(Modifiers::ALT));
        h.event(egui::Event::PointerMoved(c));
        h.event(egui::Event::PointerButton { pos: c, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::ALT });
        h.run_steps(2);
        assert!(h.state().ui.brush_picker.is_none(), "resize must be reserved before a drag has started");
        assert!(strokes(&h).is_empty());
    }

    #[test]
    fn right_click_during_brush_free_transform_opens_transform_modes_not_the_brush_picker() {
        let mut h = harness(None);
        h.state_mut().run("paint.stroke", json!({"points":[[180, 130], [220, 170]], "size":20})).unwrap();
        let ctx = h.ctx.clone();
        crate::transform_tool::begin(h.state_mut(), &ctx).unwrap();
        h.run_steps(2);
        assert_eq!(h.state().ui.tool, Tool::Brush, "transform retains the selected painting tool");
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        press(&mut h, c, PointerButton::Secondary, true);
        assert!(h.state().ui.brush_picker.is_none(), "the press must not open or focus a competing picker");
        press(&mut h, c, PointerButton::Secondary, false);
        assert!(h.state().ui.brush_picker.is_none());
        assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|menu| menu.transform), "release retains the transform mode menu");
        assert!(h.state().ui.transform.is_some());
    }

    #[test]
    fn picker_enables_background_previews_without_any_docked_panel() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = true;
        show_picker(&mut app, &ctx);
        assert_eq!(ctx.data(|d| d.get_temp::<bool>(egui::Id::new("brush-preview-background"))), Some(true));
    }

    #[test]
    fn touch_hold_is_not_a_pen_side_button() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let pos = egui::pos2(100.0, 100.0);
        let mut saw_hold = false;
        for (step, time) in [0.0, 0.01, 0.1, 0.6, 1.0, 1.5].into_iter().enumerate() {
            let mut input =
                egui::RawInput { time: Some(time), screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0))), ..Default::default() };
            if step == 1 {
                input.events = vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase: egui::TouchPhase::Start, pos, force: Some(0.7) },
                    egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
                ];
            }
            let mut output = ctx.run_ui(input, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let (_, response) = ui.allocate_exact_size(vec2(400.0, 300.0), egui::Sense::click_and_drag());
                    saw_hold |= response.long_touched();
                    canvas_buttons(&mut app, &response, Tool::Brush);
                });
            });
            output.textures_delta.clear();
            assert!(app.ui.brush_picker.is_none());
        }
        assert!(saw_hold, "exercise a real egui long-touch, not an explicit right-button click");
    }

    #[test]
    fn click_outside_closes_the_picker() {
        let mut h = harness(None);
        let c = h.state().last_canvas_rect.center();
        h.state_mut().ui.brush_picker = Some([c.x, c.y]);
        h.run_steps(2);
        let far = c - vec2(300.0, 200.0);
        h.event(egui::Event::PointerMoved(far));
        press(&mut h, far, PointerButton::Primary, true);
        press(&mut h, far, PointerButton::Primary, false);
        assert_eq!(h.state().ui.brush_picker, None);
    }

    #[test]
    fn right_drag_erases_with_the_erase_preference() {
        let mut h = harness(Some("erase"));
        let (a, b) = drag(&mut h, PointerButton::Primary);
        let mid = a + (b - a) * 0.5;
        assert!(alpha_at(&h, mid) > 0.9);
        let undo = h.state().session.active().unwrap().history.past_len();
        drag(&mut h, PointerButton::Secondary);
        let s = strokes(&h);
        assert_eq!(s.len(), 2);
        assert_eq!(s[1]["erase"], json!(true), "the right stroke erases");
        assert_eq!(alpha_at(&h, mid), 0.0, "erased to transparency");
        assert_eq!(h.state().session.active().unwrap().history.past_len(), undo + 1, "one undo step");
        assert_eq!(h.state().ui.brush_picker, None, "no picker in erase mode");
        // The next left stroke paints again.
        drag(&mut h, PointerButton::Primary);
        assert_eq!(strokes(&h)[2]["erase"], json!(false));
        assert!(alpha_at(&h, mid) > 0.9);
    }

    #[test]
    fn right_stroke_erases_with_pen_pressure_through_the_control_channel_path() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
        app.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "erase"})).unwrap();
        app.ui.tool = Tool::Brush;
        let m = Modifiers::NONE;
        assert!(pointer_secondary(&mut app, true, egui::Modifiers::NONE, [0.0, 0.0]));
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: 0.4 }, m);
        assert!(!app.secondary_erase, "armed for one stroke only");
        tool_event(&mut app, ToolEvent::Move { x: 100.0, y: 40.0, pressure: 0.8 }, m);
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 40.0 }, m);
        let p = app.session.journal.iter().rev().find(|(id, _)| id == "paint.stroke").map(|(_, p)| p.clone()).unwrap();
        assert_eq!(p["erase"], json!(true));
        assert_eq!(p["points"][0], json!([10.0, 40.0, 0.4f32 as f64]), "pressure reaches the erasing stroke");
        // On the Background the erasing stroke paints the background colour (#76).
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.layers[0].surface().unwrap().rgba(50, 40), [1.0, 1.0, 1.0, 1.0]);
        // With the default preference the right button opens the picker and nothing paints.
        app.run("prefs.set", json!({"path": "tools.rightClickWithPaintingTools", "value": "brushPicker"})).unwrap();
        assert!(!pointer_secondary(&mut app, true, egui::Modifiers::NONE, [0.0, 0.0]));
        assert!(app.ui.brush_picker.is_some() && !app.secondary_erase);
    }

    #[test]
    fn smoothing_is_a_per_tool_option_that_presets_keep() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        let amount = |app: &PhotocraftApp| app.session.tools.brush.smoothing.amount;
        app.ui.tool = Tool::Brush;
        sync_tool_smoothing(&mut app);
        assert_eq!(amount(&app), 0.1, "Photoshop's default");
        // Picking a preset (whose own smoothing is 0) keeps the tool's smoothing.
        let preset = app.session.tools.presets.iter().find(|p| p.brush.smoothing.amount == 0.0).map(|p| p.name.clone()).unwrap();
        app.run("tools.setBrush", json!({"preset": preset})).unwrap();
        assert_eq!(amount(&app), 0.1);
        // A user value (the options-bar field) survives a preset too, mode included.
        app.session.tools.brush.smoothing.amount = 0.42;
        app.session.tools.brush.smoothing.pulled_string = true;
        app.run("tools.setBrush", json!({"preset": preset})).unwrap();
        assert_eq!((amount(&app), app.session.tools.brush.smoothing.pulled_string), (0.42, true));
        // The Eraser has its own (10 % at first); switching back restores the Brush's.
        app.ui.tool = Tool::Eraser;
        sync_tool_smoothing(&mut app);
        assert_eq!(amount(&app), 0.1);
        app.session.tools.brush.smoothing.amount = 0.0;
        app.ui.tool = Tool::Brush;
        sync_tool_smoothing(&mut app);
        assert_eq!((amount(&app), app.session.tools.brush.smoothing.pulled_string), (0.42, true));
        // Other tools leave it alone; the Eraser kept its 0 %.
        app.ui.tool = Tool::Move;
        sync_tool_smoothing(&mut app);
        assert_eq!(amount(&app), 0.42);
        app.ui.tool = Tool::Eraser;
        // A stroke picks up the tool's smoothing even without a frame in between.
        tool_event(&mut app, ToolEvent::Down { x: 5.0, y: 5.0, pressure: 1.0 }, Modifiers::NONE);
        assert_eq!(amount(&app), 0.0);
        tool_event(&mut app, ToolEvent::Up { x: 30.0, y: 5.0 }, Modifiers::NONE);
    }

    #[test]
    fn eraser_tool_on_the_background_paints_the_background_colour() {
        // #76: the Eraser on the Background (locked transparency) paints the background colour.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
        app.run("tools.setColors", json!({"background": "#ff0000"})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0}})).unwrap();
        app.ui.tool = Tool::Eraser;
        let m = Modifiers::NONE;
        tool_event(&mut app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Move { x: 100.0, y: 40.0, pressure: 1.0 }, m);
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 40.0 }, m);
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.layers[0].name, "Background");
        assert_eq!(st.doc.layers[0].surface().unwrap().rgba(50, 40), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(st.history.undo_label(), Some("Eraser"));
        assert!(!app.ui.status_error, "{}", app.ui.status);
    }

    /// Assert the one committed stroke contains a point at each queued screen position.
    fn assert_committed_points(h: &Harness<'static, PhotocraftApp>, queued: &[Pos2]) {
        let s = strokes(h);
        assert_eq!(s.len(), 1);
        let points = s[0]["points"].as_array().unwrap();
        let app = h.state();
        let v = app.ui.views[0].clone();
        let r = app.last_canvas_rect;
        for p in queued {
            let d = (*p - r.center()) / v.zoom;
            let want = [d.x as f64 + v.center[0] as f64, d.y as f64 + v.center[1] as f64];
            assert!(
                points.iter().any(|q| {
                    let q = q.as_array().unwrap();
                    (q[0].as_f64().unwrap() - want[0]).abs() < 1.0 && (q[1].as_f64().unwrap() - want[1]).abs() < 1.0
                }),
                "queued move {want:?} missing from {points:?}"
            );
        }
    }

    #[test]
    fn one_frame_feeds_every_pointer_move_not_just_the_latest() {
        // Fast strokes used to lose the moves between two frames: the canvas read only
        // `interact_pointer_pos()` once per frame, so the stroke was a coarse polyline. Every
        // `PointerMoved` egui-winit delivered in one frame must now reach the stroke.
        let mut h = harness(None);
        let c = h.state().last_canvas_rect.center();
        let a = c - vec2(90.0, 0.0);
        h.event(egui::Event::PointerMoved(a));
        h.run_steps(1);
        press(&mut h, a, PointerButton::Primary, true);
        // Cross egui's drag threshold on its own frame.
        h.event(egui::Event::PointerMoved(a + vec2(20.0, 0.0)));
        h.run_steps(1);
        // Several moves in ONE frame: `Harness::event` would make one frame per event, so push
        // them straight into the next frame's input. All of them must reach the committed stroke.
        let queued: Vec<Pos2> = (1..=6).map(|i| a + vec2(20.0 + i as f32 * 8.0, 0.0)).collect();
        for p in &queued {
            h.input_mut().events.push(egui::Event::PointerMoved(*p));
        }
        h.run_steps(1);
        let end = *queued.last().unwrap();
        press(&mut h, end, PointerButton::Primary, false);
        assert_committed_points(&h, &queued);
    }
}
