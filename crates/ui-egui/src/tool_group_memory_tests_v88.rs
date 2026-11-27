//! User-facing group recall, through keyboard dispatch and real toolbar targets.
//! The hidden-toolbar cases distinguish selection memory from egui paint data.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, Vec2, vec2};
use egui_kittest::Harness;
use serde_json::json;

use crate::{PhotocraftApp, Services, Tool};

fn app(shift_switch: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.session.prefs.edit(|prefs| prefs.tools.use_shift_key_for_tool_switch = shift_switch);
    app
}

fn select(app: &mut PhotocraftApp, tool: Tool) {
    // This is also how control/set and existing preset paths publish a selected
    // tool. No toolbar paint is needed to remember the selection.
    app.ui.tool = tool;
    app.sync_tool_brush();
}

fn keys(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(700.0, 400.0)).with_max_steps(64).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            app.sync_tool_brush();
            crate::shortcuts::handle(app, ui.ctx());
            app.sync_tool_brush();
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::StudioLight);
    h.run_steps(3);
    h
}

fn tap(h: &mut Harness<'_, PhotocraftApp>, key: Key, modifiers: Modifiers) {
    h.key_press_modifiers(modifiers, key);
    h.run_steps(3);
}

#[test]
fn real_canvas_ctrl_move_does_not_replace_a_rebound_group_memory_v89() {
    use crate::canvas::{ToolEvent, tool_event};
    let mut app = app(true);
    app.run("file.new", json!({"width":64,"height":64})).unwrap();
    app.run("layer.new.layer", json!({"name":"move target"})).unwrap();
    app.session
        .edit("seed target", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(4, 4, 28, 28), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.prefs.edit(|prefs| {
        prefs.shortcuts.insert(Tool::Move.shortcut_id(), "G".into());
    });
    select(&mut app, Tool::PaintBucket);
    select(&mut app, Tool::Brush);
    let before = app.ui.tool_history.clone();
    let brush = serde_json::to_value(&app.session.tools.brush).unwrap();
    app.ui.tool_options.move_show_transform = false;
    app.ui.extras.snap = false;
    for event in [ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, ToolEvent::Move { x: 20.0, y: 20.0, pressure: 1.0 }, ToolEvent::Up { x: 20.0, y: 20.0 }]
    {
        tool_event(&mut app, event, Modifiers::CTRL);
        assert_eq!(app.ui.tool, Tool::Brush);
        assert_eq!(app.ui.tool_history, before);
        assert!(!app.tool_history_suspended);
    }
    assert_eq!(serde_json::to_value(&app.session.tools.brush).unwrap(), brush);
    assert!(app.session.journal.iter().any(|(command, _)| command.starts_with("layer.")));
    let mut h = keys(app);
    tap(&mut h, Key::G, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::PaintBucket);
}

#[test]
fn every_shared_default_tool_key_recalls_the_nonfirst_tool_without_a_toolbar_v88() {
    // Independent expected selections for every default multi-tool shortcut.
    // The old None => group[0] branch fails every one of these re-entry cases.
    let cases = [
        (Tool::EllipseMarquee, Key::M),
        (Tool::MagneticLasso, Key::L),
        (Tool::ObjectSelection, Key::W),
        (Tool::SliceSelect, Key::C),
        (Tool::Count, Key::I),
        (Tool::MixerBrush, Key::B),
        (Tool::MagicEraser, Key::E),
        (Tool::PaintBucket, Key::G),
        (Tool::VerticalType, Key::T),
        (Tool::ContentAwareMove, Key::J),
        (Tool::Sponge, Key::O),
        (Tool::DirectSelection, Key::A),
        (Tool::CustomShape, Key::U),
    ];
    for shift_switch in [false, true] {
        for (expected, key) in cases {
            let mut app = app(shift_switch);
            select(&mut app, expected);
            let mut h = keys(app);
            tap(&mut h, Key::V, Modifiers::NONE);
            assert_eq!(h.state().ui.tool, Tool::Move);
            tap(&mut h, key, Modifiers::NONE);
            assert_eq!(h.state().ui.tool, expected, "{expected:?}, shift switch={shift_switch}: re-entry must recall rather than select the first tool");
            assert_eq!(h.state().ui.tool_history.first(), Some(&expected));
        }
    }
}

#[test]
fn an_unvisited_group_still_uses_its_default_first_tool_v88() {
    for (key, expected) in [(Key::G, Tool::Gradient), (Key::M, Tool::RectMarquee), (Key::L, Tool::Lasso)] {
        let mut app = app(true);
        app.ui.tool_history.clear();
        select(&mut app, Tool::Move);
        let mut h = keys(app);
        tap(&mut h, key, Modifiers::NONE);
        assert_eq!(h.state().ui.tool, expected);
    }
}

#[test]
fn recall_preserves_plain_key_and_shift_cycle_preferences_v88() {
    for shift_switch in [false, true] {
        let mut app = app(shift_switch);
        select(&mut app, Tool::PaintBucket);
        let mut h = keys(app);
        tap(&mut h, Key::V, Modifiers::NONE);
        tap(&mut h, Key::G, Modifiers::NONE);
        assert_eq!(h.state().ui.tool, Tool::PaintBucket);
        if shift_switch {
            tap(&mut h, Key::G, Modifiers::NONE);
            assert_eq!(h.state().ui.tool, Tool::PaintBucket, "plain key keeps the current member with Shift switching enabled");
            tap(&mut h, Key::G, Modifiers::SHIFT);
            assert_eq!(h.state().ui.tool, Tool::Gradient);
            tap(&mut h, Key::G, Modifiers::SHIFT);
        } else {
            tap(&mut h, Key::G, Modifiers::NONE);
            assert_eq!(h.state().ui.tool, Tool::Gradient, "plain key still cycles while already in this group");
            tap(&mut h, Key::G, Modifiers::NONE);
        }
        assert_eq!(h.state().ui.tool, Tool::PaintBucket);
        tap(&mut h, Key::V, Modifiers::NONE);
        tap(&mut h, Key::G, Modifiers::NONE);
        assert_eq!(h.state().ui.tool, Tool::PaintBucket, "cycling also updates the group's recalled tool");
    }
}

#[test]
fn custom_bindings_recall_only_tools_still_in_the_effective_shortcut_group_v88() {
    let mut app = app(true);
    app.session.prefs.edit(|prefs| {
        prefs.shortcuts.insert(Tool::Gradient.shortcut_id(), "K".into());
        prefs.shortcuts.insert(Tool::PaintBucket.shortcut_id(), "K".into());
        prefs.shortcuts.insert(Tool::Brush.shortcut_id(), "G".into());
    });
    select(&mut app, Tool::PaintBucket);
    let mut h = keys(app);
    tap(&mut h, Key::V, Modifiers::NONE);
    tap(&mut h, Key::K, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::PaintBucket, "custom shared key recalls its last eligible tool");
    tap(&mut h, Key::V, Modifiers::NONE);
    tap(&mut h, Key::G, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Brush, "the default G name cannot select a bucket now bound to K");
    h.state_mut().session.prefs.edit(|prefs| {
        prefs.shortcuts.insert(Tool::PaintBucket.shortcut_id(), String::new());
    });
    tap(&mut h, Key::V, Modifiers::NONE);
    tap(&mut h, Key::K, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::Gradient, "an unbound remembered tool is not a shortcut candidate");
}

#[test]
fn changing_documents_does_not_reset_tool_group_choices_v88() {
    let mut app = app(true);
    app.run("file.new", json!({"name":"first","width":16,"height":16})).unwrap();
    app.run("file.new", json!({"name":"second","width":16,"height":16})).unwrap();
    select(&mut app, Tool::PaintBucket);
    select(&mut app, Tool::VerticalType);
    let mut h = keys(app);
    tap(&mut h, Key::V, Modifiers::NONE);
    h.state_mut().session.set_active(0);
    h.state_mut().sync_views();
    tap(&mut h, Key::G, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::PaintBucket);
    h.state_mut().session.set_active(1);
    h.state_mut().sync_views();
    tap(&mut h, Key::T, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::VerticalType);
}

fn toolbar(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1200.0, 1800.0)).with_max_steps(64).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            app.sync_tool_brush();
            crate::shortcuts::handle(app, ui.ctx());
            crate::panels::toolbar(app, ui);
            app.sync_tool_brush();
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::StudioLight);
    h.ctx.set_embed_viewports(true);
    h.run_steps(4);
    h
}

fn buttons(h: &Harness<'_, PhotocraftApp>) -> Vec<Rect> {
    let mut buttons: Vec<_> = h.ctx.viewport(|viewport| {
        viewport
            .prev_pass
            .widgets
            .layers()
            .flat_map(|(_, widgets)| widgets.iter())
            .filter(|widget| widget.rect.size() == Vec2::splat(36.0) && widget.sense.senses_click())
            .map(|widget| widget.rect)
            .collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())));
    assert_eq!(buttons.len(), 20, "the tall one-column toolbar has twenty tool slots");
    buttons
}

fn click(h: &mut Harness<'_, PhotocraftApp>, pos: Pos2, button: PointerButton) {
    h.hover_at(pos);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
}

#[test]
fn a_toolbar_rendered_after_tool_changes_uses_the_same_selection_memory_v88() {
    let mut app = app(true);
    select(&mut app, Tool::PaintBucket);
    select(&mut app, Tool::Move);
    // No toolbar has seen PaintBucket: egui's old per-slot temp data is empty.
    let mut h = toolbar(app);
    let at = buttons(&h)[11].center();
    click(&mut h, at, PointerButton::Primary);
    assert_eq!(h.state().ui.tool, Tool::PaintBucket, "toolbar re-entry must agree with G even after no toolbar paints");
    let brush = buttons(&h)[7].center();
    click(&mut h, brush, PointerButton::Primary);
    assert_eq!(h.state().ui.tool, Tool::Brush);
    h.set_size(vec2(1200.0, 600.0));
    h.run_steps(4);
    let bucket = buttons(&h)[11].center();
    click(&mut h, bucket, PointerButton::Primary);
    assert_eq!(h.state().ui.tool, Tool::PaintBucket);
}

#[test]
fn selecting_bucket_from_the_real_flyout_is_remembered_after_clicking_move_v88() {
    let mut h = toolbar(app(true));
    let slot = buttons(&h)[11].center();
    click(&mut h, slot, PointerButton::Secondary);
    let mut rows: Vec<_> = h.ctx.viewport(|viewport| {
        viewport
            .prev_pass
            .widgets
            .layers()
            .flat_map(|(_, widgets)| widgets.iter())
            .filter(|widget| widget.rect.height() == 26.0 && widget.rect.width() >= 180.0 && widget.sense.senses_click())
            .map(|widget| widget.rect)
            .collect()
    });
    rows.sort_by(|a, b| a.top().total_cmp(&b.top()));
    assert_eq!(rows.len(), 2, "the real gradient/bucket flyout has two clickable rows");
    click(&mut h, rows[1].center(), PointerButton::Primary);
    assert_eq!(h.state().ui.tool, Tool::PaintBucket);
    let move_button = buttons(&h)[0].center();
    click(&mut h, move_button, PointerButton::Primary);
    assert_eq!(h.state().ui.tool, Tool::Move);
    tap(&mut h, Key::G, Modifiers::NONE);
    assert_eq!(h.state().ui.tool, Tool::PaintBucket, "the flyout choice also reaches the keyboard shortcut memory");
    let move_button = buttons(&h)[0].center();
    click(&mut h, move_button, PointerButton::Primary);
    let returned_slot = buttons(&h)[11].center();
    click(&mut h, returned_slot, PointerButton::Primary);
    assert_eq!(h.state().ui.tool, Tool::PaintBucket);
}

#[test]
fn held_navigation_and_alt_do_not_replace_the_saved_toolbar_group_member_v88() {
    let mut app = app(true);
    select(&mut app, Tool::RotateView);
    select(&mut app, Tool::PaintBucket);
    let mut h = toolbar(app);
    let history = h.state().ui.tool_history.clone();
    h.event(Event::Key { key: Key::Space, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(crate::hold_keys::effective_tool(h.state(), &h.ctx), Tool::Hand);
    assert_eq!(h.state().ui.tool, Tool::PaintBucket);
    assert_eq!(h.state().ui.tool_history, history);
    h.event(Event::Key { key: Key::Space, physical_key: None, pressed: false, repeat: false, modifiers: Modifiers::NONE });
    h.event(Event::ModifiersChanged(Modifiers::CTRL));
    h.run_steps(2);
    assert_eq!(crate::hold_keys::effective_tool(h.state(), &h.ctx), Tool::Move);
    assert_eq!(h.state().ui.tool_history, history);
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, Tool::PaintBucket);
    assert_eq!(h.state().ui.tool_history, history);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    let hand_group = buttons(&h)[18].center();
    click(&mut h, hand_group, PointerButton::Primary);
    assert_eq!(h.state().ui.tool, Tool::RotateView, "temporary Space did not overwrite the Hand/Rotate group with Hand");
}
