//! Native panel keyboard events are resolved locally and executed by ROOT once.

use super::*;
use egui::{Event, Key, Modifiers};
use photocraft_doc::LayerContent;
use serde_json::json;

const SMART: &str = "layer.smartObjects.convertToSmartObject";

fn app(command: &str, shortcut: &str) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":32,"height":24})).unwrap();
    app.run("layer.new.layer", json!({"name":"Panel shortcut raster"})).unwrap();
    app.run("edit.fill", json!({"color":"#6b8fc9"})).unwrap();
    app.session.prefs.edit(|prefs| prefs.shortcuts.insert(command.into(), shortcut.into()));
    app
}

fn press(key: Key) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

fn input(id: ViewportId, events: Vec<Event>) -> egui::RawInput {
    let mut input = egui::RawInput { viewport_id: id, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 550.0))), events, ..Default::default() };
    input.viewports.entry(id).or_default().focused = Some(true);
    input
}

fn panel_frame(ctx: &egui::Context, app: &mut PhotocraftApp, events: Vec<Event>, mut body: impl FnMut(&mut PhotocraftApp, &mut egui::Ui)) -> egui::FullOutput {
    let mut output = ctx.run_ui(input(viewport(701, 0), events), |ui| {
        body(app, ui);
        collect_shortcuts(app, ui.ctx());
    });
    output.textures_delta.clear();
    output
}

fn root_frame(ctx: &egui::Context, app: &mut PhotocraftApp) -> egui::FullOutput {
    let mut output = ctx.run_ui(input(ViewportId::ROOT, Vec::new()), |ui| finish_pending(app, ui.ctx()));
    output.textures_delta.clear();
    output
}

fn smart(app: &PhotocraftApp) -> bool {
    let state = app.session.active().unwrap();
    matches!(state.doc.layer(state.active_layer.unwrap()).unwrap().content, LayerContent::Smart(_))
}

#[test]
fn floating_layers_custom_s_is_consumed_locally_and_runs_once_in_root_v70() {
    let ctx = egui::Context::default();
    let mut app = app(SMART, "S");
    let tool = app.ui.tool;
    panel_frame(&ctx, &mut app, vec![press(Key::S)], |app, ui| {
        let doc = app.session.active().unwrap().doc.id;
        crate::layer_row_ui::claim_keyboard_focus(ui.ctx(), doc, Id::new("selected-layer-row"));
    });
    assert!(!smart(&app), "a child viewport must not run application commands");
    assert_eq!(app.ui.tool, tool, "the custom command must not become a tool selection");
    root_frame(&ctx, &mut app);
    assert!(smart(&app));
    let log = crate::shortcut_dispatch::take_log(&ctx);
    assert_eq!(log, vec![(SMART.into(), crate::shortcut_dispatch::Outcome::Ran)]);
    root_frame(&ctx, &mut app);
    assert!(crate::shortcut_dispatch::take_log(&ctx).is_empty(), "a repeated root pass cannot replay the command");
}

#[test]
fn floating_panel_discarded_layout_pass_cannot_queue_the_same_press_twice_v70() {
    let ctx = egui::Context::default();
    let mut app = app(SMART, "S");
    let mut discarded = false;
    panel_frame(&ctx, &mut app, vec![press(Key::S)], |_, ui| {
        if !discarded {
            discarded = true;
            ui.ctx().request_discard("exercise native panel multipass event replay");
        }
    });
    assert_eq!(ctx.data(|data| data.get_temp::<Vec<PendingShortcut>>(Id::new(SHORTCUTS))).unwrap().len(), 1);
    root_frame(&ctx, &mut app);
    assert!(smart(&app));
    assert_eq!(crate::shortcut_dispatch::take_log(&ctx).len(), 1);
}

#[test]
fn floating_panel_text_fields_keep_bare_custom_shortcut_as_text_v70() {
    let ctx = egui::Context::default();
    let mut app = app(SMART, "S");
    let mut text = String::new();
    for events in [Vec::new(), vec![press(Key::S), Event::Text("s".into())]] {
        panel_frame(&ctx, &mut app, events, |_, ui| {
            ui.add(egui::TextEdit::singleline(&mut text).id(Id::new("panel-name-field"))).request_focus();
        });
    }
    assert_eq!(text, "s");
    root_frame(&ctx, &mut app);
    assert!(!smart(&app));
    assert!(crate::shortcut_dispatch::take_log(&ctx).is_empty());
}

#[test]
fn floating_panel_widget_navigation_and_default_tool_keys_are_not_forwarded_v70() {
    let ctx = egui::Context::default();
    let mut app = app(SMART, Key::ArrowDown.name());
    panel_frame(&ctx, &mut app, Vec::new(), |_, ui| {
        ui.button("Panel control").request_focus();
    });
    panel_frame(&ctx, &mut app, vec![press(Key::ArrowDown)], |_, ui| {
        ui.button("Panel control").request_focus();
    });
    root_frame(&ctx, &mut app);
    assert!(!smart(&app));
    assert!(crate::shortcut_dispatch::take_log(&ctx).is_empty());
    app.session.prefs.edit(|prefs| prefs.shortcuts.clear());
    let original_tool = app.ui.tool;
    panel_frame(&ctx, &mut app, vec![press(Key::B)], |_, ui| {
        if let Some(id) = ui.ctx().memory(|memory| memory.focused()) {
            ui.ctx().memory_mut(|memory| memory.surrender_focus(id));
        }
    });
    root_frame(&ctx, &mut app);
    assert_eq!(app.ui.tool, original_tool, "forwarding adds no single-key tool dispatch in floating panels");
    assert!(crate::shortcut_dispatch::take_log(&ctx).is_empty());
}

#[test]
fn floating_panel_shortcut_drops_if_active_document_changes_before_root_v70() {
    let ctx = egui::Context::default();
    let mut app = app(SMART, "S");
    let source = app.session.active().unwrap().doc.id;
    panel_frame(&ctx, &mut app, vec![press(Key::S)], |_, _| {});
    app.run("file.new", json!({"width":32,"height":24})).unwrap();
    assert_ne!(source, app.session.active().unwrap().doc.id);
    root_frame(&ctx, &mut app);
    assert!(!smart(&app));
    assert!(crate::shortcut_dispatch::take_log(&ctx).is_empty(), "do not convert a different document");
}

#[test]
fn floating_panel_shortcuts_stop_when_root_opens_a_dialog_and_respect_current_modal_v70() {
    let ctx = egui::Context::default();
    let mut app = app(SMART, "S");
    app.session.prefs.edit(|prefs| prefs.shortcuts.insert("edit.preferences.general".into(), "P".into()));
    panel_frame(&ctx, &mut app, vec![press(Key::P), press(Key::S)], |_, _| {});
    assert!(app.ui.dialogs.is_empty(), "panel child cannot open the preferences dialog");
    root_frame(&ctx, &mut app);
    assert_eq!(app.ui.dialogs.len(), 1);
    assert!(!smart(&app), "a command after dialog creation must not run behind the dialog");
    assert_eq!(crate::shortcut_dispatch::take_log(&ctx), vec![("edit.preferences.general".into(), crate::shortcut_dispatch::Outcome::Ran)]);
    let dialog = app.ui.dialogs.pop().unwrap();
    panel_frame(&ctx, &mut app, vec![press(Key::S)], |_, _| {});
    app.ui.dialogs.push(dialog);
    root_frame(&ctx, &mut app);
    assert!(!smart(&app), "a modal opened before root draining blocks an already queued command");
    app.ui.dialogs.clear();
    root_frame(&ctx, &mut app);
    assert!(!smart(&app), "blocked commands are dropped instead of replaying after a modal closes");
}

#[test]
fn auxiliary_modals_block_new_and_queued_panel_shortcuts_without_later_replay_v70() {
    for analysis in [false, true] {
        let ctx = egui::Context::default();
        let mut app = app(SMART, "S");
        panel_frame(&ctx, &mut app, vec![press(Key::S)], |_, _| {});
        if analysis {
            app.ui.analysis.dialog = Some(("scale".into(), serde_json::Map::new()));
        } else {
            app.ui.shell.dialog = Some(("newWorkspace".into(), serde_json::Map::new()));
        }
        assert!(crate::dialogs::native_dialog_viewport(&app).is_some());
        root_frame(&ctx, &mut app);
        panel_frame(&ctx, &mut app, vec![press(Key::S)], |_, _| {});
        root_frame(&ctx, &mut app);
        assert!(!smart(&app));
        assert!(crate::shortcut_dispatch::take_log(&ctx).is_empty());
        app.ui.analysis.dialog = None;
        app.ui.shell.dialog = None;
        root_frame(&ctx, &mut app);
        assert!(!smart(&app), "auxiliary-modal-blocked commands must not replay after it closes");
        assert!(crate::shortcut_dispatch::take_log(&ctx).is_empty());
    }
}
