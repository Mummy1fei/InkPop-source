//! Held Ctrl is view state: real canvas events must move/select without painting,
//! changing a saved tool/brush, or stealing an existing gesture or text focus.

use super::*;
use crate::canvas::{ToolEvent, ViewXform, tool_event};
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use photocraft_geom::Rect;
use serde_json::json;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":320,"height":240,"background":"transparent"})).unwrap();
    app.run("layer.new.layer", json!({"name":"Red subject"})).unwrap();
    app.session
        .edit("test subject", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(40, 40, 100, 100), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.run("layer.new.layer", json!({"name":"Blue selected layer"})).unwrap();
    app.session
        .edit("test second subject", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(200, 150, 250, 200), &[0.0, 0.3, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.ui.tool = tool;
    app.sync_tool_brush();
    app.run("tools.setBrush", json!({"brush":{"size":17,"hardness":0.37,"spacing":0.21,"smoothing":{"amount":0.0}}})).unwrap();
    app.ui.tool_options.move_auto_select = false;
    app.ui.tool_options.move_show_transform = false;
    app.ui.extras.snap = false;
    app.ui.extras.rulers = false;
    app.ui.view.show.smart_guides = false;
    app.sync_views();
    app
}

fn bounds(app: &PhotocraftApp, index: usize) -> Rect {
    app.session.active().unwrap().doc.layers[index].surface().unwrap().content_bounds()
}

fn brush(app: &PhotocraftApp) -> serde_json::Value {
    serde_json::to_value(&app.session.tools.brush).unwrap()
}

fn paint_count(app: &PhotocraftApp) -> usize {
    app.session.journal.iter().filter(|(id, _)| id.starts_with("paint.")).count()
}

fn paints_label(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    fn contains(shape: &egui::Shape, label: &str) -> bool {
        match shape {
            egui::Shape::Text(text) => text.galley.job.text == label,
            egui::Shape::Vec(items) => items.iter().any(|shape| contains(shape, label)),
            _ => false,
        }
    }
    h.output().shapes.iter().any(|shape| contains(&shape.shape, label))
}

fn frame(ctx: &egui::Context, input: egui::RawInput, body: impl FnMut(&mut egui::Ui)) {
    let mut output = ctx.run_ui(input, body);
    output.textures_delta.clear();
}

fn harness(theme: crate::theme::ThemeKind, gpu: bool) -> Harness<'static, PhotocraftApp> {
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
        app(Tool::Brush),
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    let ctx = h.ctx.clone();
    h.state_mut().set_theme(&ctx, theme);
    h.run_steps(4);
    let view = &mut h.state_mut().ui.views[0];
    view.zoom = 1.0;
    view.center = [160.0, 120.0];
    view.fit_pending = false;
    h.run_steps(3);
    h
}

fn screen(h: &Harness<'_, PhotocraftApp>, point: [f32; 2]) -> Pos2 {
    let app = h.state();
    let view = &app.ui.views[0];
    ViewXform {
        rotation: view.rotation,
        rect: crate::rulers::content_rect(app, app.last_canvas_rect),
        zoom: view.zoom,
        center: view.center,
        flip: app.ui.view.flip_horizontal,
    }
    .to_screen(point[0], point[1])
}

fn modifiers(h: &mut Harness<'_, PhotocraftApp>, value: Modifiers) {
    h.event(Event::ModifiersChanged(value));
    h.run_steps(2);
}

fn button(h: &mut Harness<'_, PhotocraftApp>, point: [f32; 2], pressed: bool, value: Modifiers) {
    let pos = screen(h, point);
    h.event(Event::ModifiersChanged(value));
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: value });
    h.run_steps(4);
}

fn motion(h: &mut Harness<'_, PhotocraftApp>, point: [f32; 2]) {
    let pos = screen(h, point);
    h.event(Event::PointerMoved(pos));
    h.run_steps(2);
}

#[test]
fn ctrl_move_auto_select_is_inclusive_and_never_changes_the_saved_checkbox_v74() {
    for saved in [false, true] {
        for value in [Modifiers::CTRL, Modifiers::CTRL | Modifiers::COMMAND] {
            let mut app = app(Tool::Move);
            app.ui.tool_options.move_auto_select = saved;
            let red = app.session.active().unwrap().doc.layers[1].id;
            let before = app.session.active().unwrap().history.past_len();
            tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, value);
            tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 60.0 }, value);
            assert_eq!(app.session.active().unwrap().active_layer, Some(red), "Ctrl enables pickup even when the base was already enabled");
            assert_eq!(app.ui.tool_options.move_auto_select, saved);
            assert_eq!(app.session.active().unwrap().history.past_len(), before);
            assert_eq!(auto_select(&app, Modifiers::NONE), saved);
            assert_eq!(app.ui.tool, Tool::Move);
        }
    }
}

#[test]
fn idle_tools_use_temporary_move_without_mutating_brushes_or_selection_tools_v74() {
    for tool in [Tool::Brush, Tool::Pencil, Tool::MixerBrush, Tool::Smudge, Tool::RectMarquee, Tool::Type, Tool::Lasso, Tool::Pen] {
        let mut app = app(tool);
        let before = brush(&app);
        let history = app.session.active().unwrap().history.past_len();
        let red = app.session.active().unwrap().doc.layers[1].id;
        tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 0.4 }, Modifiers::CTRL);
        assert_eq!(app.drag.as_ref().unwrap().tool, Tool::Move);
        tool_event(&mut app, ToolEvent::Move { x: 90.0, y: 75.0, pressure: 0.8 }, Modifiers::CTRL);
        tool_event(&mut app, ToolEvent::Up { x: 90.0, y: 75.0 }, Modifiers::NONE);
        assert_eq!(app.session.active().unwrap().active_layer, Some(red));
        assert_eq!(bounds(&app, 1), Rect::new(70, 55, 130, 115));
        assert_eq!(bounds(&app, 2), Rect::new(200, 150, 250, 200));
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
        assert_eq!(paint_count(&app), 0);
        assert_eq!(app.ui.tool, tool);
        assert_eq!(brush(&app), before, "{tool:?} restores its complete tip/settings");
        assert!(app.drag.is_none() && app.ui.text_edit.is_none() && app.ui.pen.is_none());
        assert!(app.session.active().unwrap().doc.selection.is_none());
    }
}

#[test]
fn real_mouse_and_pen_keep_move_until_release_then_restore_the_brush_v74() {
    for pen in [false, true] {
        let mut h = harness(Default::default(), false);
        let before = brush(h.state());
        let history = h.state().session.active().unwrap().history.past_len();
        if pen {
            h.state_mut().stylus.feed.set(Some(crate::stylus::PenSample { pressure: 0.63, tilt_x: 15.0, ..Default::default() }));
        }
        button(&mut h, [60.0, 60.0], true, Modifiers::CTRL);
        motion(&mut h, [80.0, 70.0]);
        assert_eq!(h.state().drag.as_ref().unwrap().tool, Tool::Move);
        modifiers(&mut h, Modifiers::NONE);
        assert_eq!(effective_tool(h.state(), &h.ctx), Tool::Move, "a started drag keeps its owner after Ctrl goes up");
        motion(&mut h, [110.0, 90.0]);
        button(&mut h, [110.0, 90.0], false, Modifiers::NONE);
        let app = h.state();
        assert_eq!(bounds(app, 1), Rect::new(90, 70, 150, 130));
        assert_eq!(app.session.active().unwrap().history.past_len(), history + 1);
        assert_eq!(paint_count(app), 0);
        assert_eq!(app.ui.tool, Tool::Brush);
        assert_eq!(effective_tool(app, &h.ctx), Tool::Brush);
        assert_eq!(brush(app), before);
        assert!(!app.ui.tool_options.move_auto_select);
    }
}

#[test]
fn ctrl_short_click_is_latched_when_ctrl_is_released_before_the_pointer_v74() {
    let mut h = harness(Default::default(), false);
    let red = h.state().session.active().unwrap().doc.layers[1].id;
    let before = h.state().session.active().unwrap().history.past_len();
    button(&mut h, [60.0, 60.0], true, Modifiers::CTRL);
    modifiers(&mut h, Modifiers::NONE);
    button(&mut h, [60.0, 60.0], false, Modifiers::NONE);
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(red));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before);
    assert_eq!(paint_count(h.state()), 0, "egui's delayed click cannot reach the saved Brush");
    assert_eq!(effective_tool(h.state(), &h.ctx), Tool::Brush);
}

#[test]
fn ctrl_pressed_during_a_brush_or_marquee_drag_keeps_the_existing_owner_v74() {
    for tool in [Tool::Brush, Tool::RectMarquee, Tool::Lasso] {
        let mut app = app(tool);
        tool_event(&mut app, ToolEvent::Down { x: 210.0, y: 160.0, pressure: 0.7 }, Modifiers::NONE);
        assert!(!ctrl_move_requested(&app, Modifiers::CTRL));
        tool_event(&mut app, ToolEvent::Move { x: 240.0, y: 160.0, pressure: 0.6 }, Modifiers::CTRL);
        assert_eq!(app.drag.as_ref().unwrap().tool, tool);
        tool_event(&mut app, ToolEvent::Up { x: 240.0, y: 180.0 }, Modifiers::NONE);
        assert_eq!(bounds(&app, 1), Rect::new(40, 40, 100, 100));
        assert_eq!(app.ui.tool, tool);
        if tool == Tool::Brush {
            assert_eq!(paint_count(&app), 1);
        } else {
            assert_eq!(paint_count(&app), 0);
            assert!(app.session.active().unwrap().doc.selection.is_some());
        }
    }
}

#[test]
fn ctrl_shift_is_an_axis_locked_move_and_never_a_second_brush_stroke_v74() {
    let mut app = app(Tool::Brush);
    let before = app.session.active().unwrap().history.past_len();
    let modifiers = Modifiers::CTRL | Modifiers::SHIFT;
    tool_event(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 0.7 }, modifiers);
    tool_event(&mut app, ToolEvent::Move { x: 95.0, y: 65.0, pressure: 0.6 }, modifiers);
    tool_event(&mut app, ToolEvent::Up { x: 110.0, y: 70.0 }, modifiers);
    assert_eq!(bounds(&app, 1), Rect::new(90, 40, 150, 100));
    assert_eq!(paint_count(&app), 0);
    assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
    assert_eq!(app.ui.tool, Tool::Brush);
}

#[test]
fn ctrl_alt_resize_and_ctrl_space_zoom_keep_priority_over_temporary_move_v74() {
    let mut app = app(Tool::Brush);
    let before = app.session.tools.brush.size;
    let history = app.session.active().unwrap().history.past_len();
    let modifiers = Modifiers::CTRL | Modifiers::ALT;
    tool_event(&mut app, ToolEvent::Down { x: 130.0, y: 100.0, pressure: 0.7 }, modifiers);
    tool_event(&mut app, ToolEvent::Move { x: 145.0, y: 100.0, pressure: 0.7 }, modifiers);
    tool_event(&mut app, ToolEvent::Up { x: 145.0, y: 100.0 }, modifiers);
    assert!(app.session.tools.brush.size > before);
    assert_eq!(app.session.active().unwrap().history.past_len(), history);
    assert_eq!(paint_count(&app), 0);
    let mut h = harness(Default::default(), false);
    h.event(Event::ModifiersChanged(Modifiers::CTRL | Modifiers::COMMAND));
    h.event(Event::Key { key: egui::Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::CTRL | Modifiers::COMMAND });
    h.run_steps(3);
    assert_eq!(held_tool(h.state(), &h.ctx), Some(Temporary::ZoomIn));
    assert_eq!(effective_tool(h.state(), &h.ctx), Tool::Zoom);
    assert_eq!(h.state().ui.tool, Tool::Brush);
}

#[test]
fn text_ime_dialog_and_transform_guards_do_not_become_temporary_move_v74() {
    let mut app = app(Tool::Type);
    crate::type_tool::pointer_up(&mut app, [130.0, 100.0], [130.0, 100.0]);
    app.ui.text_edit.as_mut().unwrap().preedit = Some((0, 0));
    assert!(!ctrl_move_requested(&app, Modifiers::CTRL), "inline canvas IME owns Ctrl shortcuts");
    app.ui.text_edit = None;
    app.ui.tool = Tool::Brush;
    app.ui.open_dialog(crate::state::DialogKind::About, serde_json::Map::new());
    assert!(!ctrl_move_requested(&app, Modifiers::CTRL));
    app.ui.dialogs.clear();
    let mut app = self::app(Tool::Brush);
    let ctx = egui::Context::default();
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    assert!(!ctrl_move_requested(&app, Modifiers::CTRL));
    let original = app.ui.transform.clone();
    tool_event(&mut app, ToolEvent::Move { x: 140.0, y: 120.0, pressure: 0.5 }, Modifiers::CTRL);
    assert!(app.ui.transform.is_some(), "temporary switching must not commit an existing transform");
    assert_eq!(app.ui.transform.as_ref().unwrap().quad, original.unwrap().quad);
    assert_eq!(app.ui.tool, Tool::Brush);
}

#[test]
fn focused_egui_text_or_numeric_fields_keep_ctrl_in_the_field_v74() {
    for numeric in [false, true] {
        let mut app = app(Tool::Brush);
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        let mut value = if numeric { "17".to_string() } else { "name".to_string() };
        for _ in 0..2 {
            frame(&ctx, egui::RawInput::default(), |ui| {
                let response = ui.add(egui::TextEdit::singleline(&mut value).id_salt("focused-tool-field"));
                response.request_focus();
            });
        }
        frame(&ctx, egui::RawInput { events: vec![Event::ModifiersChanged(Modifiers::CTRL)], ..Default::default() }, |ui| {
            ui.add(egui::TextEdit::singleline(&mut value).id_salt("focused-tool-field"));
            assert!(ctx.text_edit_focused());
            assert_eq!(held_tool(&app, &ctx), None);
            assert_eq!(effective_tool(&app, &ctx), Tool::Brush);
            assert!(!auto_select_held(&app, &ctx));
            // The control route with a context must use the same protection.
            crate::canvas::tool_event_in_context(&mut app, &ctx, ToolEvent::Move { x: 140.0, y: 100.0, pressure: 1.0 }, Modifiers::CTRL);
            assert!(app.drag.is_none());
        });
        assert_eq!(paint_count(&app), 0);
    }
}

#[test]
fn options_show_the_held_tool_and_reveal_the_saved_checkbox_after_release_v74() {
    let mut h = harness(Default::default(), false);
    assert!(!paints_label(&h, tl!("Auto-Select:")));
    modifiers(&mut h, Modifiers::CTRL);
    assert_eq!(effective_tool(h.state(), &h.ctx), Tool::Move);
    assert!(paints_label(&h, tl!("Auto-Select:")), "the real options bar paints the temporary Move controls");
    assert!(auto_select_for_ui(h.state(), &h.ctx));
    assert!(!h.state().ui.tool_options.move_auto_select);
    modifiers(&mut h, Modifiers::NONE);
    assert_eq!(effective_tool(h.state(), &h.ctx), Tool::Brush);
    assert!(!paints_label(&h, tl!("Auto-Select:")));
    assert!(!h.state().ui.tool_options.move_auto_select);
}

#[test]
fn native_child_publishes_ctrl_to_root_and_keeps_its_own_press_latch_v74() {
    let app = app(Tool::Brush);
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    ctx.set_embed_viewports(false);
    let child = egui::ViewportId::from_hash_of("ctrl-move-native-child-v74");
    let raw = |viewport, focused, modifiers, mut events: Vec<Event>| {
        events.insert(0, Event::ModifiersChanged(modifiers));
        let mut input = egui::RawInput { viewport_id: viewport, focused: if viewport == child { focused } else { !focused }, events, ..Default::default() };
        input.viewports.entry(child).or_default().focused = Some(focused);
        input.viewports.entry(child).or_default().parent = Some(egui::ViewportId::ROOT);
        input.viewports.entry(egui::ViewportId::ROOT).or_default().focused = Some(!focused);
        input
    };
    let pos = egui::pos2(50.0, 50.0);
    frame(
        &ctx,
        raw(child, true, Modifiers::CTRL, vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::CTRL }]),
        |_| {
            assert_eq!(for_frame(&app, &ctx, false), Some(Temporary::Move));
        },
    );
    frame(&ctx, raw(egui::ViewportId::ROOT, true, Modifiers::NONE, Vec::new()), |_| {
        // Match the real parent frame: it must register its live child or egui
        // legitimately removes that viewport (including its held-pointer input).
        ctx.show_viewport_deferred(child, egui::ViewportBuilder::default(), |_, _| {});
        assert!(ctx.input_for(child, |input| input.pointer.primary_down()), "child input survives into the ROOT pass");
        assert_eq!(effective_tool(&app, &ctx), Tool::Move, "ROOT sees the focused child rather than stale ROOT modifiers");
        assert!(auto_select_for_ui(&app, &ctx));
        assert_eq!(for_frame(&app, &ctx, false), None, "ROOT input is independent");
    });
    frame(&ctx, raw(child, true, Modifiers::NONE, vec![Event::ModifiersChanged(Modifiers::NONE)]), |_| {
        assert!(ctx.input(|input| input.pointer.primary_down()), "registered child preserved its press across the parent frame");
        assert_eq!(for_frame(&app, &ctx, false), Some(Temporary::Move), "unfocused ROOT frame did not clear the child's press latch");
    });
    frame(
        &ctx,
        raw(child, true, Modifiers::NONE, vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]),
        |_| {
            assert_eq!(for_frame(&app, &ctx, false), Some(Temporary::Move), "latched choice reaches the delayed release");
        },
    );
    frame(&ctx, raw(child, true, Modifiers::NONE, Vec::new()), |_| {
        assert_eq!(for_frame(&app, &ctx, false), None);
    });
    frame(&ctx, raw(egui::ViewportId::ROOT, true, Modifiers::CTRL, Vec::new()), |_| {
        ctx.show_viewport_deferred(child, egui::ViewportBuilder::default(), |_, _| {});
        assert_eq!(effective_tool(&app, &ctx), Tool::Brush, "release in the child overrides stale ROOT Ctrl");
        assert!(!auto_select_held(&app, &ctx));
    });
    frame(&ctx, raw(egui::ViewportId::ROOT, false, Modifiers::NONE, Vec::new()), |_| {
        ctx.show_viewport_deferred(child, egui::ViewportBuilder::default(), |_, _| {});
        assert_eq!(effective_tool(&app, &ctx), Tool::Brush, "unfocused child state never sticks after focus returns");
    });
    assert_eq!(app.ui.tool, Tool::Brush);
    assert!(!app.ui.tool_options.move_auto_select);
}

#[test]
#[ignore = "writes five themes with Ctrl held/released for root visual review"]
fn ctrl_move_gpu_visual_v74() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v74/move-gpu");
    assert!(!out.exists(), "preserve previous screenshots");
    std::fs::create_dir_all(&out).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = harness(theme, true);
        let saved_brush = brush(h.state());
        motion(&mut h, [70.0, 65.0]);
        modifiers(&mut h, Modifiers::CTRL);
        h.run_steps(4);
        assert_eq!(effective_tool(h.state(), &h.ctx), Tool::Move);
        assert!(auto_select_for_ui(h.state(), &h.ctx));
        assert_eq!(h.state().ui.theme, theme);
        assert_eq!(crate::theme::Tokens::get(&h.ctx).kind, theme);
        assert!(paints_label(&h, tl!("Auto-Select:")), "the Ctrl-held capture must show actual Move controls");
        assert_eq!(h.state().ui.tool, Tool::Brush);
        assert!(!h.state().ui.tool_options.move_auto_select);
        assert_eq!(brush(h.state()), saved_brush);
        h.render().unwrap().save(out.join(format!("{}-ctrl-held.png", theme.id()))).unwrap();
        modifiers(&mut h, Modifiers::NONE);
        h.run_steps(4);
        assert_eq!(effective_tool(h.state(), &h.ctx), Tool::Brush);
        assert!(!h.state().ui.tool_options.move_auto_select);
        assert_eq!(h.state().ui.theme, theme);
        assert_eq!(crate::theme::Tokens::get(&h.ctx).kind, theme);
        assert!(!paints_label(&h, tl!("Auto-Select:")), "release restores actual Brush controls");
        assert_eq!(h.state().ui.tool, Tool::Brush);
        assert_eq!(brush(h.state()), saved_brush);
        h.render().unwrap().save(out.join(format!("{}-released.png", theme.id()))).unwrap();
    }
}
