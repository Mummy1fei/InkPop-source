//! Programmatic control of the running app, for agents, tests and (soon) MCP.
//!
//! Transport-agnostic: a transport thread (TCP in the desktop app, a channel in tests) sends
//! [`ControlRequest`]s; the UI thread handles them between frames and replies with JSON.
//!
//! Methods:
//! - `engine.execute {command, params}`: run any engine or UI command by id
//! - `engine.commands`: list commands with enablement
//! - `ui.inspect`: full UI state (tool, panels, views, dialogs, windows, window size); the menu
//!   tree is `ui.menu.list`
//! - `ui.set {tool?, panels?, dock?, dockTabs?, dockWidth?, colorPanel?, maskTarget?, vectorMaskTarget?, selectionMode?, zoom?, center?, fit?, theme?, brushSection?, brushTab?, brushesView?, brushSize?}`:
//!   change UI state; any other field is an error ([`UI_SET_FIELDS`])
//! - `ui.menu.invoke {id, wait?}` / `ui.menu.list`: activate a menu item by id; list the menu tree
//! - `ui.dialog.open {kind, fields?}` (kinds: newDocument, about, layerStyle {effect?}, colorPicker {target: foreground|background}, command {command}) / `ui.dialog.set {dialog, field, value}` / `ui.dialog.confirm {dialog, wait?}` / `ui.dialog.cancel {dialog}`
//! - `ui.dialog.apply {dialog}`: commit Preferences changes without closing the dialog
//! - `ui.window.open {document?}` / `ui.window.close {window}` / `ui.window.merge {source, target}` / `ui.window.selectTab {window, document}`
//! - `ui.pointer {events: [{kind: down|move|up, x, y, pressure?, tiltX?, tiltY?, rotation?}], modifiers?, button?}`: drive the active tool in document coordinates (`button: "secondary"` opens the tool's canvas context menu or Brush Preset picker, or erases with Preferences › Tools › Right-click with painting tools = erase)
//! - `ui.click {x, y, button?, count?}` / `ui.move {x, y}`: synthetic pointer input in screen points
//! - `ui.key {key, command?, shift?, alt?, ctrl?}` / `ui.type {text}`: synthetic keyboard input
//! - `ui.resize {width, height}`: resize the main window
//! - `ui.gpu.simulateLoss {error?}`: act as if the wgpu device was lost (or, with `error: true`,
//!   reported an error): the app switches to the CPU renderer for the rest of the session, as on a
//!   real loss. For testing the fallback; returns whether a GPU canvas was active
//! - `ui.screenshot {path?, focus?, window?, dialog?}`: capture the main window (PNG), or an open
//!   native document `window` id, or `dialog: "discard"`. Raises the selected window first (default)
//!   because occluded macOS windows stop rendering
//! - `ui.focus`: bring the main window to the front
//! - `app.open {path}` / `app.save {path}`: relative file I/O under the automation roots; reply with `warnings`
//! - `app.place {path}`: capability-scoped placement followed by the production placement preview
//! - `app.quit`
//! - `jobs.list` / `jobs.cancel {job?}`: background jobs (#210) with progress; cancel one (or all).
//!   `engine.execute`, `ui.menu.invoke` and `ui.dialog.confirm` wait for a command that runs as a
//!   job unless `wait: false` (then the reply is `{job, pending: true}`)

use std::sync::mpsc::Sender;

use base64::Engine as _;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::{DialogKind, Tool, UiState};

#[cfg(test)]
#[path = "control_place_tests_v90.rs"]
mod place_tests_v90;

pub type ControlResponse = Value;

pub struct ControlRequest {
    pub method: String,
    pub params: Value,
    pub reply: Sender<ControlResponse>,
}

impl ControlRequest {
    pub fn new(method: impl Into<String>, params: Value) -> (Self, std::sync::mpsc::Receiver<ControlResponse>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (Self { method: method.into(), params, reply: tx }, rx)
    }
}

pub enum Outcome {
    Done(Value),
    Screenshot {
        token: u64,
        path: Option<String>,
        viewport: egui::ViewportId,
    },
    /// Synthetic input queued: reply once the app has processed all of it (so a following
    /// `ui.inspect`/`engine.execute` observes the effect).
    AfterInput,
    /// A command started a background job and the caller waits for it: reply with its result
    /// once it has been applied (or with its error / "cancelled").
    AfterJob(photocraft_engine::jobs::JobId),
}

/// The fields `ui.set` reads. Anything else is rejected before a field is applied, so a typo or
/// a field the method doesn't have can't reply with success while nothing changes (#412).
pub const UI_SET_FIELDS: [&str; 27] = [
    "sampling",
    "layerFilter",
    "presetManager",
    "tool",
    "panels",
    "dock",
    "dockTabs",
    "dockWidth",
    "colorPanel",
    "maskTarget",
    "vectorMaskTarget",
    "selectionMode",
    "zoom",
    "center",
    "fit",
    "theme",
    "brushSection",
    "brushTab",
    "brushesView",
    "brushSize",
    "gradientBlendMode",
    "gradientClassic",
    "moveAutoSelect",
    "moveTarget",
    "moveShowTransform",
    "objectSelectionShape",
    "smartObjectSelection",
];

/// Strict, bounded view-state patches. Runtime caches never enter the control protocol.
fn option_patch<T: serde::Serialize + serde::de::DeserializeOwned>(current: &T, patch: &Value, fields: &[&str]) -> Result<T, String> {
    let object = patch.as_object().ok_or("options must be an object")?;
    if let Some(key) = object.keys().find(|key| !fields.contains(&key.as_str())) {
        return Err(format!("unsupported option `{key}`"));
    }
    let mut value = serde_json::to_value(current).map_err(|e| e.to_string())?;
    let target = value.as_object_mut().ok_or("invalid options state")?;
    target.extend(object.clone());
    serde_json::from_value(value).map_err(|e| e.to_string())
}

fn ok(v: Value) -> Outcome {
    Outcome::Done(json!({"ok": true, "result": v}))
}
fn err(e: impl std::fmt::Display) -> Outcome {
    Outcome::Done(json!({"ok": false, "error": e.to_string()}))
}
fn wrap(r: Result<Value, String>) -> Outcome {
    match r {
        Ok(v) => ok(v),
        Err(e) => err(e),
    }
}

/// Run a command the way automation does (script events off). Long commands may run as
/// background jobs: by default the reply waits for the job's result (backward compatible); with
/// `wait` false it is `{job, pending: true}` at once.
fn run_waiting(app: &mut PhotocraftApp, wait: bool, run: impl FnOnce(&mut PhotocraftApp) -> Result<Value, String>) -> Outcome {
    let events_enabled = app.session.prefs().script_events.enabled;
    if events_enabled {
        app.session.edit_prefs(|prefs| prefs.script_events.enabled = false);
    }
    app.jobs.last_started = None;
    let result = run(app);
    if events_enabled {
        app.session.edit_prefs(|prefs| prefs.script_events.enabled = true);
    }
    match (result, app.jobs.last_started.take()) {
        (Ok(_), Some(job)) if wait => Outcome::AfterJob(job),
        (result, _) => wrap(result),
    }
}

/// Screen position of document point (x, y) on the main canvas, where a `ui.pointer` right-click
/// opens its menu (the mouse's opens at the pointer); the canvas centre when it can't be mapped.
fn screen_point(app: &PhotocraftApp, x: f64, y: f64) -> [f32; 2] {
    let p = crate::canvas::ViewXform::active(app)
        .map(|xf| xf.to_screen(x as f32, y as f32))
        .filter(|p| p.is_finite())
        .unwrap_or_else(|| app.last_canvas_rect.center());
    [p.x, p.y]
}

pub fn handle(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let saved = app.session.authorize;
    if let Some(gate) = app.services.automation_authorize {
        app.session.authorize = Some(gate);
    }
    let outcome = dispatch(app, ctx, req);
    app.session.authorize = saved;
    outcome
}

fn dispatch(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let p = &req.params;
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    let u = |k: &str| p.get(k).and_then(Value::as_u64);
    let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(true);
    match req.method.as_str() {
        "ui.context.choose" => {
            let Some(id) = s("id") else { return err("missing `id`") };
            let Some(menu) = app.ui.canvas_tool_menu.as_ref() else { return err("no canvas context menu is open") };
            if !crate::canvas_tool_menu::available(app, menu, id) {
                return err("context action is unavailable");
            }
            if let Some(authorize) = app.services.automation_command.as_ref()
                && let Err(error) = authorize(id, &json!({}))
            {
                return err(error);
            }
            crate::canvas_tool_menu::choose(app, ctx, id);
            ok(json!({"command": id, "dialog": app.ui.dialogs.last().map(|d| d.id)}))
        }
        "engine.execute" | "ui.menu.invoke" => {
            let Some(id) = s("command").or(s("id")) else { return err("missing `command`") };
            let params = p.get("params").cloned().unwrap_or(json!({}));
            if let Some(authorize) = app.services.automation_command.as_ref()
                && let Err(error) = authorize(id, &params)
            {
                return err(error);
            }
            // `engine.execute` is programmatic: engine commands run directly with their default
            // params and never open a dialog (an agent would otherwise get a modal instead of a
            // result). `ui.menu.invoke` behaves like a menu click, so it may open the dialog.
            if req.method == "engine.execute" && photocraft_engine::commands::find(id).is_some() {
                return run_waiting(app, wait, |app| app.run(id, params));
            }
            run_waiting(app, wait, |app| crate::menus::invoke(app, ctx, id, params))
        }
        "doc.exportPhotoshopCopy" => wrap(crate::ps_collaboration_ui::export_automation(app, p)),
        "engine.commands" => wrap(app.run("command.list", json!({}))),
        // Background jobs (#210): running ones with progress, then the last few that ended.
        "jobs.list" => wrap(app.session.execute("jobs.list", json!({})).map_err(|e| e.to_string())),
        "jobs.cancel" => {
            let all = p.get("job").is_none_or(Value::is_null);
            match p.get("job").and_then(Value::as_u64) {
                Some(j) if app.session.job(photocraft_engine::jobs::JobId(j)).is_some() => {
                    crate::jobs_ui::cancel(app, photocraft_engine::jobs::JobId(j));
                    ok(json!({"cancelled": 1}))
                }
                Some(j) => err(format!("no running job {j}")),
                None if all => {
                    let ids: Vec<_> = app.session.jobs().into_iter().map(|j| j.id).collect();
                    for j in &ids {
                        crate::jobs_ui::cancel(app, *j);
                    }
                    ok(json!({"cancelled": ids.len()}))
                }
                None => err("`job` must be a job id"),
            }
        }
        "ui.menu.list" => ok(serde_json::to_value(crate::menus::menu_items(app)).unwrap_or_default()),
        "ui.inspect" => ok(inspect(app, ctx)),
        "ui.set" => {
            if let Some(field) = p.as_object().and_then(|o| o.keys().find(|k| !UI_SET_FIELDS.contains(&k.as_str()))) {
                return err(format!("unknown field `{field}` (fields: {})", UI_SET_FIELDS.join(", ")));
            }
            let sampling = if let Some(value) = p.get("sampling") {
                match option_patch::<crate::sampling_ui::Options>(&app.ui.sampling, value, &["size", "source", "showRing", "mixerSize", "mixerSampleAllLayers"])
                {
                    Ok(options) if crate::sampling_ui::SIZES.contains(&options.size) && crate::sampling_ui::SIZES.contains(&options.mixer_size) => {
                        Some(options)
                    }
                    Ok(_) => return err("sampling size must be 1, 3, 5, 11, 31, 51 or 101"),
                    Err(error) => return err(format!("sampling: {error}")),
                }
            } else {
                None
            };
            let layer_filter = if let Some(value) = p.get("layerFilter") {
                let Some(object) = value.as_object() else { return err("layerFilter must be an object") };
                let mut options = object.clone();
                let kinds = options.remove("kinds").unwrap_or_else(|| json!(app.ui.layer_filter));
                let kinds: Vec<String> = match serde_json::from_value(kinds) {
                    Ok(kinds) => kinds,
                    Err(error) => return err(format!("layerFilter kinds: {error}")),
                };
                if kinds.len() > 5 || kinds.iter().any(|kind| !["pixel", "adjustment", "type", "shape", "smart"].contains(&kind.as_str())) {
                    return err("layerFilter kinds must be at most five valid content kinds");
                }
                match option_patch::<crate::layer_filter_ui::Options>(
                    &app.ui.layer_filter_options,
                    &Value::Object(options),
                    &["enabled", "mode", "query", "value"],
                ) {
                    Ok(options) if options.query.chars().count() <= 256 && crate::layer_filter_ui::valid_value(&options) => Some((options, kinds)),
                    Ok(_) => return err("invalid layerFilter value or query length"),
                    Err(error) => return err(format!("layerFilter: {error}")),
                }
            } else {
                None
            };
            let gradient_blend = if let Some(value) = p.get("gradientBlendMode") {
                let Some(name) = value.as_str() else { return err("gradientBlendMode must be a blend mode name") };
                let Some(mode) = photocraft_engine::commands::blend_from_str(name).filter(|m| photocraft_color::BlendMode::LAYER_MODES.contains(m)) else {
                    return err(format!("unknown gradient blend mode `{name}`"));
                };
                Some(mode)
            } else {
                None
            };
            if let Some(value) = p.get("gradientClassic")
                && !value.is_boolean()
            {
                return err("gradientClassic must be a boolean");
            }
            for field in ["moveAutoSelect", "moveShowTransform", "smartObjectSelection"] {
                if p.get(field).is_some_and(|value| !value.is_boolean()) {
                    return err(format!("{field} must be a boolean"));
                }
            }
            if let Some(value) = p.get("moveTarget")
                && !matches!(value.as_str(), Some("layer" | "group"))
            {
                return err("moveTarget must be layer or group");
            }
            let object_shape = if let Some(value) = p.get("objectSelectionShape") {
                match serde_json::from_value::<crate::state::ObjectSelectionShape>(value.clone()) {
                    Ok(shape) => Some(shape),
                    Err(_) => return err("objectSelectionShape must be rectangle, lasso or foregroundStroke"),
                }
            } else {
                None
            };
            if let Some(t) = s("tool") {
                match Tool::from_name(t) {
                    Some(t) => app.ui.tool = t,
                    None => return err(format!("unknown tool `{t}`")),
                }
            }
            let gradient_before = app.ui.tool_options.clone();
            if let Some(shape) = object_shape {
                app.ui.tool_options.object_selection_shape = shape;
            }
            if let Some(smart) = p.get("smartObjectSelection").and_then(Value::as_bool) {
                app.ui.tool_options.smart_object_selection = smart;
            }
            if let Some(auto_select) = p.get("moveAutoSelect").and_then(Value::as_bool) {
                app.ui.tool_options.move_auto_select = auto_select;
            }
            if let Some(target) = s("moveTarget") {
                app.ui.tool_options.move_target = target.to_owned();
            }
            if let Some(show) = p.get("moveShowTransform").and_then(Value::as_bool) {
                app.ui.tool_options.move_show_transform = show;
            }
            if let Some(mode) = gradient_blend {
                app.ui.tool_options.gradient_blend_mode = mode;
            }
            if let Some(classic) = p.get("gradientClassic").and_then(Value::as_bool) {
                app.ui.tool_options.gradient_classic = classic;
            }
            if gradient_blend.is_some() {
                crate::gradient_ui::options_changed(app, &gradient_before);
            }
            if let Some(panels) = p.get("panels") {
                let mut cur = serde_json::to_value(&app.ui.panels).unwrap_or_default();
                if let (Some(c), Some(n)) = (cur.as_object_mut(), panels.as_object()) {
                    for (k, v) in n {
                        c.insert(k.clone(), v.clone());
                    }
                }
                match serde_json::from_value(cur) {
                    Ok(v) => app.ui.panels = v,
                    Err(e) => return err(e),
                }
            }
            if let Some(m) = p.get("maskTarget").and_then(Value::as_bool) {
                app.ui.mask_target = m;
                app.ui.vector_mask_target &= !m;
            }
            if let Some(m) = p.get("vectorMaskTarget").and_then(Value::as_bool) {
                app.ui.vector_mask_target = m;
                app.ui.mask_target &= !m;
            }
            // Selection tools' options-bar mode: 0 New, 1 Add, 2 Subtract, 3 Intersect.
            if let Some(m) = p.get("selectionMode").and_then(Value::as_u64) {
                app.ui.selection_mode = m.min(3) as u8;
            }
            if let Some(tabs) = p.get("dockTabs") {
                let mut cur = serde_json::to_value(app.ui.dock_tabs).unwrap_or_default();
                if let (Some(c), Some(n)) = (cur.as_object_mut(), tabs.as_object()) {
                    for (k, v) in n {
                        c.insert(k.clone(), v.clone());
                    }
                }
                match serde_json::from_value(cur) {
                    Ok(v) => {
                        app.ui.dock_tabs = v;
                        app.ui.brush_tab = app.ui.dock_tabs.brush.min(1);
                    }
                    Err(e) => return err(e),
                }
            }
            // Dock group order, heights and collapsed groups (see `dock::DockLayout`).
            if let Some(d) = p.get("dock") {
                match serde_json::from_value(d.clone()) {
                    Ok(v) => app.ui.dock = v,
                    Err(e) => return err(e),
                }
            }
            // Which chip the Color panel edits.
            if let Some(c) = p.get("colorPanel") {
                match serde_json::from_value(c.clone()) {
                    Ok(v) => app.ui.color_panel = v,
                    Err(e) => return err(e),
                }
            }
            // Right dock width in points (clamped to the dock's 250..=520 range), applied next frame.
            if let Some(w) = p.get("dockWidth").and_then(Value::as_f64) {
                crate::panels::request_dock_width(ctx, w as f32);
            }
            if let Some(i) = app.session.active_index() {
                if let Some(z) = p.get("zoom").and_then(Value::as_f64) {
                    app.ui.views[i].zoom = (z as f32).clamp(0.01, 64.0);
                    app.ui.views[i].fit_pending = false;
                }
                if let Some(c) = p.get("center").and_then(Value::as_array)
                    && c.len() == 2
                {
                    app.ui.views[i].center = [c[0].as_f64().unwrap_or(0.0) as f32, c[1].as_f64().unwrap_or(0.0) as f32];
                    app.ui.views[i].fit_pending = false;
                }
                if p.get("fit").and_then(Value::as_bool) == Some(true) {
                    app.ui.views[i].fit_pending = true;
                }
            }
            if let Some(name) = s("theme") {
                match crate::theme::ThemeKind::from_name(name) {
                    Some(k) => app.set_theme(ctx, k),
                    None => {
                        let names: Vec<_> = crate::theme::ThemeKind::ALL.iter().map(|k| k.id()).collect();
                        return err(format!("unknown theme `{name}` ({})", names.join(", ")));
                    }
                }
            }
            if let Some(i) = u("brushSection") {
                app.ui.brush_section = (i as usize).min(crate::brush_panel::SECTIONS.len() - 1);
            }
            if let Some(manager) = p.get("presetManager").and_then(Value::as_bool) {
                app.ui.presets_ui.manager = manager;
            }
            if let Some(i) = u("brushTab") {
                app.ui.brush_tab = (i as usize).min(1);
                app.ui.dock_tabs.brush = app.ui.brush_tab;
                for c in &mut app.ui.dock.containers {
                    if let Some(index) = c.tabs.iter().position(|t| t.group == crate::dock::Group::Brush && t.index == app.ui.brush_tab) {
                        c.selected = index;
                    }
                }
            }
            if let Some(v) = p.get("brushesView").cloned() {
                match serde_json::from_value(v) {
                    Ok(v) => app.ui.brushes_panel.view = v,
                    Err(e) => return err(format!("brushesView: {e} (list, grid)")),
                }
            }
            if let Some(size) = p.get("brushSize").and_then(Value::as_f64)
                && let Err(e) = app.run("tools.setBrush", json!({"brush": {"size": size}}))
            {
                return err(e);
            }
            if let Some(options) = sampling {
                app.ui.sampling = options;
                crate::sampling_ui::end(app);
                crate::sampling_ui::save(app);
            }
            if let Some((options, kinds)) = layer_filter {
                app.ui.layer_filter_options = options;
                app.ui.layer_filter = kinds;
            }
            ok(Value::Null)
        }
        "ui.dialog.open" => {
            let kind = match s("kind").unwrap_or("") {
                "newDocument" | "NewDocument" => DialogKind::NewDocument,
                "about" | "About" => DialogKind::About,
                "layerStyle" | "LayerStyle" => {
                    return match crate::layer_style::open(app, s("effect")) {
                        Some(id) => ok(json!({"dialog": id})),
                        None => err("no active layer"),
                    };
                }
                "colorPicker" | "ColorPicker" => {
                    let target = if s("target") == Some("background") { "background" } else { "foreground" };
                    return ok(json!({"dialog": crate::color_picker_ui::open(app, target)}));
                }
                "command" | "Command" => {
                    let Some(cmd) = s("command") else { return err("command dialogs need `command`") };
                    let label = photocraft_engine::commands::find(cmd).map(|c| c.label).unwrap_or(cmd);
                    return ok(json!({"dialog": crate::dialogs::open_command_dialog(app, cmd, label)}));
                }
                other => return err(format!("unknown dialog kind `{other}`")),
            };
            let mut fields = if kind == DialogKind::NewDocument { UiState::new_document_fields() } else { Default::default() };
            if let Some(f) = p.get("fields").and_then(Value::as_object) {
                fields.extend(f.clone());
            }
            ok(json!({"dialog": app.ui.open_dialog(kind, fields)}))
        }
        "ui.dialog.set" => {
            let (Some(id), Some(field)) = (u("dialog"), s("field")) else { return err("need `dialog` and `field`") };
            let value = p.get("value").cloned().unwrap_or(Value::Null);
            match app.ui.dialog_mut(id) {
                Some(d) => {
                    let stop_color = field == "color" && crate::gradient_ui::is_stop_picker(&d.fields);
                    d.fields.insert(field.to_string(), value);
                    if stop_color {
                        crate::gradient_ui::repaint_stop_preview(app, ctx);
                    }
                    ok(Value::Null)
                }
                None => err(format!("no dialog {id}")),
            }
        }
        "ui.dialog.confirm" | "ui.dialog.apply" => match u("dialog") {
            Some(id) => {
                let command = app.ui.dialogs.iter().find(|dialog| dialog.id == id).and_then(|dialog| {
                    if req.method == "ui.dialog.apply" {
                        dialog.fields.get("values").map(|values| ("prefs.set".to_string(), json!({"path": "", "value": values})))
                    } else {
                        dialog.fields.get("__command").and_then(Value::as_str).map(|command| (command.to_string(), Value::Object(dialog.fields.clone())))
                    }
                });
                if let Some((command, params)) = command
                    && let Some(authorize) = app.services.automation_command.as_ref()
                    && let Err(error) = authorize(&command, &params)
                {
                    return err(error);
                }
                let apply = req.method == "ui.dialog.apply";
                run_waiting(app, wait, |app| if apply { crate::prefs_ui::apply(app, id) } else { crate::dialogs::confirm(app, id) })
            }
            None => err("missing `dialog`"),
        },
        "ui.dialog.cancel" => match u("dialog").and_then(|id| app.ui.close_dialog(id)) {
            Some(dialog) => {
                if crate::gradient_ui::is_stop_picker(&dialog.fields) {
                    crate::gradient_ui::repaint_stop_preview(app, ctx);
                }
                ok(Value::Null)
            }
            None => err("no such dialog"),
        },
        "ui.window.open" => {
            if let Some(d) = u("document") {
                app.session.set_active(d as usize);
            }
            wrap(crate::menus::invoke(app, ctx, "window.newWindowForDocument", json!({})))
        }
        "ui.window.close" => {
            let Some(id) = u("window") else { return err("missing `window`") };
            let before = app.ui.windows.len();
            app.ui.windows.retain(|w| w.id != id);
            if app.ui.windows.len() < before { ok(Value::Null) } else { err(format!("no window {id}")) }
        }
        "ui.window.merge" => {
            let (Some(source), Some(target)) = (u("source"), u("target")) else { return err("missing `source` or `target`") };
            if crate::document_window_ui::merge(app, ctx, source, target) { ok(Value::Null) } else { err("could not merge document windows") }
        }
        "ui.window.selectTab" => {
            let (Some(window), Some(document)) = (u("window"), u("document").and_then(|d| usize::try_from(d).ok())) else {
                return err("missing `window` or `document`");
            };
            if crate::document_window_ui::select_document_tab(app, ctx, window, document) { ok(Value::Null) } else { err("could not activate document tab") }
        }
        "ui.pointer" => {
            let Some(events) = p.get("events").and_then(Value::as_array) else { return err("missing `events`") };
            // Modifier flags may be top-level or grouped under "modifiers".
            let m = p.get("modifiers").unwrap_or(p);
            let flag = |k: &str| m.get(k).and_then(Value::as_bool).unwrap_or(false);
            let mods = egui::Modifiers {
                shift: flag("shift"),
                alt: flag("alt"),
                command: flag("command"),
                mac_cmd: cfg!(target_os = "macos") && flag("command"),
                ctrl: flag("ctrl"),
            };
            if let Some(t) = s("tool").and_then(Tool::from_name) {
                app.ui.tool = t;
            }
            // Space held: the Crop tool moves the frame being drawn, a marquee, lasso or shape
            // being drawn moves instead of growing (hold_keys.rs).
            let space = flag("space");
            crate::crop_ui::set_space(app, space);
            for e in events {
                let x = e.get("x").and_then(Value::as_f64).unwrap_or(0.0);
                let y = e.get("y").and_then(Value::as_f64).unwrap_or(0.0);
                let pr = e.get("pressure").and_then(Value::as_f64).unwrap_or(1.0) as f32;
                let ev = match e.get("kind").and_then(Value::as_str).unwrap_or("move") {
                    "down" => ToolEvent::Down { x, y, pressure: pr },
                    "up" => ToolEvent::Up { x, y },
                    _ => ToolEvent::Move { x, y, pressure: pr },
                };
                // With the Color Picker on top the image is its eyedropper, as for the mouse.
                if crate::color_picker_ui::top(app).is_some() {
                    if !matches!(ev, ToolEvent::Up { .. }) {
                        crate::color_picker_ui::sample_at(app, x, y);
                    }
                    continue;
                }
                if matches!(s("button"), Some("secondary" | "right")) {
                    let down = matches!(ev, ToolEvent::Down { .. });
                    // Right-click with the Move tool, or ⌘/Ctrl+right-click: list the layers there.
                    if crate::layer_pick_ui::is_gesture(app.ui.tool, mods) {
                        if down {
                            app.ui.canvas_tool_menu = None;
                            crate::layer_pick_ui::open(app, screen_point(app, x, y), x, y);
                        }
                        continue;
                    }
                    if crate::canvas_tool_menu::applies(app.ui.tool) {
                        if down {
                            let tool = app.ui.tool;
                            let pos = screen_point(app, x, y);
                            if tool == crate::state::Tool::Pen {
                                crate::canvas_tool_menu::open(app, tool, pos);
                            } else {
                                crate::canvas_tool_menu::open_selection_on_press(app, ctx, tool, egui::pos2(pos[0], pos[1]));
                            }
                        }
                        continue;
                    }
                    if !crate::paint_mouse::pointer_secondary(app, down, mods, screen_point(app, x, y)) {
                        continue;
                    }
                }
                // A simulated pen: tilt/rotation reach the stroke like a real stylus's (see `stylus`).
                let tilt = |k: &str| e.get(k).and_then(Value::as_f64).map(|v| v as f32);
                let pen = (tilt("tiltX"), tilt("tiltY"), tilt("rotation"));
                if pen != (None, None, None) {
                    let (tilt_x, tilt_y, rotation) = (pen.0.unwrap_or(0.0), pen.1.unwrap_or(0.0), pen.2.unwrap_or(0.0));
                    app.stylus.feed.set(Some(crate::stylus::PenSample { pressure: pr, tilt_x, tilt_y, rotation, eraser: false }));
                }
                if let Some(d) = app.drag.as_mut().filter(|d| crate::hold_keys::repositions(d.tool)) {
                    d.reposition = space;
                }
                if space {
                    crate::canvas::tool_event_with_base_tool(app, ev, mods);
                } else {
                    crate::canvas::tool_event_in_context(app, ctx, ev, mods);
                }
            }
            app.stylus.feed.set(None);
            ok(json!({"status": app.ui.status}))
        }
        "ui.brushPicker.open" => {
            if !crate::paint_mouse::has_brush_picker(app.ui.tool) || app.discard.is_some() || !app.ui.dialogs.is_empty() {
                return err("brush picker requires a painting tool without a modal dialog");
            }
            let (parent, document) = if let Some(window) = p.get("window") {
                let Some(id) = window.as_u64() else { return err("`window` must be a document window id") };
                let Some(w) = app.ui.windows.iter().find(|w| w.id == id && w.open) else { return err("document window is not open") };
                let Some(doc) = app.session.documents().get(w.document) else { return err("document is not open") };
                (egui::ViewportId::from_hash_of(("docwin", id)), doc.doc.id)
            } else {
                let Some(doc) = app.session.active() else { return err("no document") };
                (egui::ViewportId::ROOT, doc.doc.id)
            };
            let coordinate = |key| p.get(key).and_then(Value::as_f64).filter(|n| n.is_finite() && n.abs() <= 100_000.0).map(|n| n as f32);
            let (Some(x), Some(y)) = (coordinate("x"), coordinate("y")) else { return err("finite `x` and `y` client coordinates are required") };
            crate::brush_picker::open_at(app, ctx, parent, document, [x, y]);
            ok(Value::Null)
        }
        "ui.brushPicker.resize" => {
            let dimension = |key, lo, hi| p.get(key).and_then(Value::as_f64).filter(|n| (lo..=hi).contains(n)).map(|n| n as f32);
            let (Some(w), Some(h)) = (dimension("width", 260.0, 1600.0), dimension("height", 280.0, 1400.0)) else {
                return err("size must be width 260..1600 and height 280..1400");
            };
            if !crate::brush_picker::resize_native(ctx, [w, h]) {
                return err("native brush picker is not open");
            }
            ok(Value::Null)
        }
        "ui.brushPicker.close" => {
            crate::brush_picker::close(app, ctx);
            ok(Value::Null)
        }
        "ui.click" | "ui.move" => {
            // Screen coordinates in points (as reported by ui.inspect window size).
            let x = p.get("x").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let y = p.get("y").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let pos = egui::pos2(x, y);
            let button = match s("button").unwrap_or("left") {
                "right" | "secondary" => egui::PointerButton::Secondary,
                "middle" => egui::PointerButton::Middle,
                _ => egui::PointerButton::Primary,
            };
            app.synthetic.push(egui::Event::PointerMoved(pos));
            if req.method == "ui.click" {
                let clicks = p.get("count").and_then(Value::as_u64).unwrap_or(1);
                for _ in 0..clicks {
                    app.synthetic.push(egui::Event::PointerButton { pos, button, pressed: true, modifiers: Default::default() });
                    app.synthetic.push(egui::Event::PointerButton { pos, button, pressed: false, modifiers: Default::default() });
                }
            }
            ctx.request_repaint();
            Outcome::AfterInput
        }
        "ui.key" => {
            let Some(name) = s("key") else { return err("missing `key`") };
            let Some(key) = egui::Key::from_name(name) else { return err(format!("unknown key `{name}`")) };
            // Modifier flags may be top-level or grouped under "modifiers".
            let m = p.get("modifiers").unwrap_or(p);
            let flag = |k: &str| m.get(k).and_then(Value::as_bool).unwrap_or(false);
            let modifiers = egui::Modifiers {
                command: flag("command"),
                mac_cmd: cfg!(target_os = "macos") && flag("command"),
                shift: flag("shift"),
                alt: flag("alt"),
                ctrl: flag("ctrl"),
            };
            app.synthetic.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
            app.synthetic.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
            ctx.request_repaint();
            Outcome::AfterInput
        }
        "ui.type" => {
            let Some(text) = s("text") else { return err("missing `text`") };
            app.synthetic.push(egui::Event::Text(text.to_string()));
            ctx.request_repaint();
            Outcome::AfterInput
        }
        "ui.resize" => {
            let (w, h) = (p.get("width").and_then(Value::as_f64).unwrap_or(1280.0), p.get("height").and_then(Value::as_f64).unwrap_or(800.0));
            if !w.is_finite() || !h.is_finite() || !(1.0..=16384.0).contains(&w) || !(1.0..=16384.0).contains(&h) {
                return err("window dimensions must be finite and between 1 and 16384 points");
            }
            let viewport = if ["panel", "dialog", "window"].iter().any(|key| p.get(key).is_some()) {
                match screenshot_viewport(app, ctx, p) {
                    Ok(viewport) => viewport,
                    Err(error) => return err(error),
                }
            } else {
                ctx.viewport_id()
            };
            let size = egui::vec2(w as f32, h as f32);
            let size = if p.get("panel").is_some() { size.max(crate::panel_workspace::native::PANEL_MIN_SIZE) } else { size };
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::InnerSize(size));
            ctx.request_repaint_of(viewport);
            ok(Value::Null)
        }
        "ui.gpu.simulateLoss" => {
            let error = p.get("error").and_then(Value::as_bool).unwrap_or(false);
            let fault = if error {
                photocraft_gpu::Fault::Error("simulated error (ui.gpu.simulateLoss)".into())
            } else {
                photocraft_gpu::Fault::Lost("simulated (ui.gpu.simulateLoss)".into())
            };
            let active = match app.gpu_health() {
                Some(h) => {
                    h.mark(fault);
                    true
                }
                None => false,
            };
            app.check_gpu(ctx);
            ctx.request_repaint();
            ok(json!({"wasActive": active, "gpuInfo": app.perf.gpu_info}))
        }
        "ui.focus" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.request_repaint();
            ok(Value::Null)
        }
        "ui.screenshot" => {
            let viewport = match screenshot_viewport(app, ctx, p) {
                Ok(viewport) => viewport,
                Err(error) => return err(error),
            };
            // Occluded windows don't render on macOS, so the screenshot would never arrive: raise first.
            if p.get("focus").and_then(Value::as_bool).unwrap_or(true) {
                ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
            }
            // The capture itself is issued by `PhotocraftApp::issue_screenshots` once open/close
            // animations (modals, popups) have settled.
            let token = app.ui.alloc_id();
            ctx.request_repaint();
            Outcome::Screenshot { token, path: s("path").map(str::to_string), viewport }
        }
        "app.open" => match s("path") {
            Some(path) => {
                let opened = match app.services.automation_read.as_mut() {
                    Some(read) => read(path),
                    None => Err("automation read authority is not configured".into()),
                };
                wrap(opened.and_then(|(name, bytes)| {
                    let name = app.open_name(&name);
                    let warnings = app.open_automation_bytes(&name, &bytes)?;
                    // Brushes/gradients go to the preset libraries: no document, no Open Recent entry.
                    if !crate::preset_files_ui::is_preset_file(&name) {
                        app.opened_from(path);
                    }
                    Ok(json!({"path": path, "name": name, "warnings": warnings}))
                }))
            }
            None => err("missing `path`"),
        },
        "app.place" => wrap(scoped_place(app, ctx, p)),
        "app.save" => wrap(app.save_automation(s("path").map(str::to_string)).map(|(p, w)| json!({"path": p, "warnings": w}))),
        "app.quit" => {
            app.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ok(Value::Null)
        }
        other => err(format!("unknown method `{other}`")),
    }
}

fn scoped_place(app: &mut PhotocraftApp, ctx: &egui::Context, params: &Value) -> Result<Value, String> {
    let object = params.as_object().ok_or("placement params must be an object")?;
    if object.keys().any(|key| key != "path") {
        return Err("placement accepts only `path`".into());
    }
    let path = object.get("path").and_then(Value::as_str).filter(|path| !path.is_empty()).ok_or("missing `path`")?;
    if app.session.active().is_none() {
        return Err("placement requires an open document".into());
    }
    if app.ui.transform.is_some() || app.ui.text_edit.is_some() || !app.ui.dialogs.is_empty() || app.discard.is_some() || app.camera_raw.is_some() {
        return Err("finish the current editing operation before placement".into());
    }
    // The platform capability performs canonical path/root/size checks. Never
    // dispatch an ambient file command or read a path directly from this crate.
    let (name, bytes) = app.services.automation_read.as_mut().ok_or("automation read authority is not configured")?(path)?;
    let placed = app.place_bytes(&name, bytes, None)?;
    crate::transform_tool::begin_placed(app, ctx)?;
    Ok(json!({"placed":placed,"transform":app.ui.transform}))
}

/// The selectors name existing app surfaces only; they never grant filesystem authority.
fn screenshot_viewport(app: &PhotocraftApp, ctx: &egui::Context, params: &Value) -> Result<egui::ViewportId, String> {
    let window = params.get("window");
    let dialog = params.get("dialog");
    let panel = params.get("panel");
    if [window, dialog, panel].iter().filter(|p| p.is_some()).count() > 1 {
        return Err("choose only one of `window`, `dialog` or `panel`".into());
    }
    if let Some(panel) = panel {
        let id = panel.as_u64().ok_or("`panel` must be an unsigned panel-container id")?;
        if ctx.embed_viewports() {
            return Err("native panel screenshots are unavailable with embedded viewports".into());
        }
        return crate::panel_workspace::native::active_viewport(app, id).ok_or_else(|| format!("floating panel {id} is not open"));
    }
    if let Some(window) = window {
        let id = window.as_u64().ok_or("`window` must be an unsigned document-window id")?;
        if !app.ui.windows.iter().any(|w| w.id == id && w.open && app.session.documents().get(w.document).is_some()) {
            return Err(format!("document window {id} is not open"));
        }
        if ctx.embed_viewports() {
            return Err("native document-window screenshots are unavailable with embedded viewports".into());
        }
        return Ok(egui::ViewportId::from_hash_of(("docwin", id)));
    }
    if let Some(dialog) = dialog {
        if dialog.as_str() == Some("menu") {
            return crate::native_menu_ui::active_viewport(ctx).ok_or_else(|| "native menu is not open".into());
        }
        if dialog.as_str() == Some("chromePopup") {
            return crate::native_popup_ui::active_viewport(ctx).ok_or_else(|| "native chrome popup is not open".into());
        }
        if dialog.as_str() == Some("active") {
            if ctx.embed_viewports() {
                return Err("native dialog is unavailable with embedded viewports".into());
            }
            return crate::dialogs::native_dialog_viewport(app).ok_or_else(|| "native dialog is not open".into());
        }
        if dialog.as_str() == Some("newBrushPreset") {
            if ctx.embed_viewports() {
                return Err("native brush preset dialog is unavailable with embedded viewports".into());
            }
            return crate::dialogs::native_preset_viewport(app).ok_or_else(|| "native brush preset dialog is not open".into());
        }
        if dialog.as_str() == Some("brushPicker") {
            if ctx.embed_viewports() {
                return Err("native brush picker is unavailable with embedded viewports".into());
            }
            return crate::brush_picker::native_viewport(ctx).ok_or_else(|| "native brush picker is not open".into());
        }
        if dialog.as_str() != Some("discard") {
            return Err("`dialog` must be \"active\", \"discard\", \"brushPicker\" or \"newBrushPreset\"".into());
        }
        if app.discard.is_none() || ctx.embed_viewports() || !app.ui.windows.iter().any(|w| w.open) {
            return Err("native unsaved-changes dialog is not open".into());
        }
        return Ok(crate::discard_ui::native_viewport_id());
    }
    Ok(egui::ViewportId::ROOT)
}

/// Snapshot of everything on screen, addressable by id.
pub fn inspect(app: &PhotocraftApp, ctx: &egui::Context) -> Value {
    let screen = ctx.content_rect();
    let dialogs: Vec<Value> =
        app.ui.dialogs.iter().map(|d| json!({"id": d.id, "kind": d.kind, "title": crate::dialogs::title(d), "fields": d.fields})).collect();
    json!({
        "window": {"width": screen.width(), "height": screen.height(), "pixelsPerPoint": ctx.pixels_per_point()},
        "tool": app.ui.tool,
        "toolHistory": app.ui.tool_history,
        "toolOptions": app.ui.tool_options,
        "smartFilters": crate::smart_ui::inspect(app),
        "sampling": app.ui.sampling,
        "layerFilter": {"options": app.ui.layer_filter_options, "kinds": app.ui.layer_filter,
            "rows": app.session.active().map(|state| crate::layer_filter_ui::rows(&state.doc, &app.ui.layer_filter_options, &app.ui.layer_filter, &state.selected_layers()).into_iter().map(|(depth, layer)| json!({"depth":depth,"id":layer.id})).collect::<Vec<_>>())},
        "magnetic": app.ui.magnetic,
        "textEdit": app.ui.text_edit,
        "layerMenu": app.ui.layer_menu,
        "brushPicker": crate::brush_picker::inspect(ctx),
        "canvasToolMenu": app.ui.canvas_tool_menu.as_ref().map(|menu| {
            json!({
                "pos": menu.pos,
                "tool": menu.tool,
                "entries": crate::canvas_tool_menu::rows(menu).iter().map(|row| match row {
                    Some((label, id)) => json!({"label": label, "id": id, "enabled": crate::canvas_tool_menu::entry_enabled(app, menu, id)}),
                    None => json!({"separator": true}),
                }).collect::<Vec<_>>()
            })
        }),
        "panels": app.ui.panels,
        "workspace": app.ui.workspace,
        "dock": app.ui.dock,
        "dockTabs": app.ui.dock_tabs,
        "views": app.ui.views,
        "dialogs": dialogs,
        "windows": app.ui.windows,
        "windowDragTrace": crate::document_window_ui::drag_diagnostics(ctx),
        "theme": app.ui.theme,
        "status": app.ui.status,
        "statusError": app.ui.status_error,
        "notices": app.ui.notices,
        "gpuFallbackNotice": app.ui.gpu_fallback_notice,
        "frame": app.frame,
        "session": photocraft_engine::inspect::session(&app.session),
        "document": app.session.active().map(photocraft_engine::inspect::document),
        "perf": {"fps": app.fps, "timings": app.perf},
        "brush": {"size": app.session.tools.brush.size, "hardness": app.session.tools.brush.hardness, "opacity": app.session.tools.brush.opacity},
        "distort": app.distort.describe(),
        "jobs": crate::jobs_ui::inspect(app),
        "cameraRaw": app.camera_raw.as_ref().map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)),
    })
}

pub fn save_screenshot(app: &mut PhotocraftApp, image: &egui::ColorImage, path: Option<&str>) -> Value {
    let [w, h] = image.size;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    let png = match app.services.encode_png.as_ref() {
        Some(enc) => enc(w as u32, h as u32, &rgba),
        None => Err("no PNG encoder configured".into()),
    };
    let Some(path) = path else {
        return match png {
            Ok(bytes) => json!({
                "ok": true,
                "result": {
                    "base64": base64::engine::general_purpose::STANDARD.encode(bytes),
                    "mime": "image/png",
                    "width": w,
                    "height": h,
                }
            }),
            Err(error) => json!({"ok": false, "error": error}),
        };
    };
    let result = png.and_then(|bytes| match app.services.automation_write.as_mut() {
        Some(write) => write(path, &bytes),
        None => Err("automation write authority is not configured".into()),
    });
    match result {
        Ok(()) => json!({"ok": true, "result": {"path": path, "width": w, "height": h}}),
        Err(e) => json!({"ok": false, "error": e}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampling_and_layer_filter_control_patches_are_typed_bounded_and_atomic_v79() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width":32,"height":24})).unwrap();
        app.run("layer.new.layer", json!({"name":"Needle"})).unwrap();
        assert_eq!(
            call(
                &mut app,
                &ctx,
                "ui.set",
                json!({"sampling":{"size":31,"source":"currentBelowNoAdjustments","showRing":false,"mixerSize":5,"mixerSampleAllLayers":false},"layerFilter":{"mode":"name","query":"Needle"}})
            )["ok"],
            true
        );
        let before = inspect(&app, &ctx);
        assert_eq!(before["sampling"]["mixerSampleAllLayers"], false);
        assert_eq!(before["layerFilter"]["rows"].as_array().unwrap().len(), 1);
        assert_eq!(app.session.prefs().dialogs["tools.sampling"]["size"], 31);
        for patch in [
            json!({"sampling":{"size":2}}),
            json!({"sampling":{"source":"invalid"}}),
            json!({"sampling":{"showRing":1}}),
            json!({"sampling":{"cache":{}}}),
            json!({"layerFilter":{"kinds":["unknown"]}}),
            json!({"layerFilter":{"mode":"color","value":"invalid"}}),
            json!({"layerFilter":{"query":"x".repeat(257)}}),
            json!({"layerFilter":{"enabled":1}}),
        ] {
            let mut patch = patch;
            patch.as_object_mut().unwrap().insert("tool".into(), json!("move"));
            assert_eq!(call(&mut app, &ctx, "ui.set", patch)["ok"], false);
            assert_eq!(inspect(&app, &ctx), before, "invalid patch does not partially change tool, sampling or filters");
        }
        let invalid = json!({"sampling":{"size":101},"layerFilter":{"mode":"unknown"}});
        assert_eq!(call(&mut app, &ctx, "ui.set", invalid)["ok"], false);
        assert_eq!(inspect(&app, &ctx), before);
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"layerFilter":{"enabled":false}}))["ok"], true);
        assert_eq!(inspect(&app, &ctx)["layerFilter"]["rows"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn resize_targets_the_active_raw_dialog_without_resizing_the_main_window() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":32,"height":32})).unwrap();
        crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
        let child = screenshot_viewport(&app, &ctx, &json!({"dialog":"active"})).unwrap();
        assert_ne!(child, egui::ViewportId::ROOT);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.ctx().show_viewport_deferred(child, egui::ViewportBuilder::default(), |_, _| {});
            assert_eq!(call(&mut app, ui.ctx(), "ui.resize", json!({"dialog":"active","width":740,"height":480}))["ok"], true);
        });
        output.textures_delta.clear();
        assert!(output.viewport_output[&child].commands.contains(&egui::ViewportCommand::InnerSize(egui::vec2(740.0, 480.0))));
        assert!(!output.viewport_output[&egui::ViewportId::ROOT].commands.iter().any(|c| matches!(c, egui::ViewportCommand::InnerSize(_))));
        app.camera_raw = None;
        let mut output = ctx.run_ui(Default::default(), |ui| {
            assert_eq!(call(&mut app, ui.ctx(), "ui.resize", json!({"dialog":"active","width":740,"height":480}))["ok"], false)
        });
        output.textures_delta.clear();
        assert!(!output.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::InnerSize(_)))));
    }

    #[test]
    fn resize_targets_only_open_panels_and_rejects_invalid_sizes() {
        use crate::{
            dock::Group,
            panel_workspace::{Container, Tab},
        };
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.ui.panels.color = true;
        app.ui.dock.containers = vec![Container { id: 91, tabs: vec![Tab { group: Group::Color, index: 0 }], floating: true, ..Default::default() }];
        let child = crate::panel_workspace::native::active_viewport(&app, 91).unwrap();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.ctx().show_viewport_deferred(child, egui::ViewportBuilder::default(), |_, _| {});
            assert_eq!(call(&mut app, ui.ctx(), "ui.resize", json!({"panel":91,"width":520,"height":610}))["ok"], true);
        });
        output.textures_delta.clear();
        assert!(output.viewport_output.get(&child).is_some_and(|v| v.commands.contains(&egui::ViewportCommand::InnerSize(egui::vec2(520.0, 610.0)))));
        assert!(!output.viewport_output[&egui::ViewportId::ROOT].commands.iter().any(|c| matches!(c, egui::ViewportCommand::InnerSize(_))));
        let mut output = ctx.run_ui(Default::default(), |ui| {
            ui.ctx().show_viewport_deferred(child, egui::ViewportBuilder::default(), |_, _| {});
            assert_eq!(call(&mut app, ui.ctx(), "ui.resize", json!({"panel":91,"width":1,"height":1}))["ok"], true);
        });
        output.textures_delta.clear();
        assert!(output.viewport_output[&child].commands.contains(&egui::ViewportCommand::InnerSize(crate::panel_workspace::native::PANEL_MIN_SIZE)));
        crate::dock::rail_click(&mut app, Group::Color, false);
        for params in [json!({"panel":91,"width":520,"height":610}), json!({"width":1e100}), json!({"height":0}), json!({"width":-20})] {
            let mut output = ctx.run_ui(Default::default(), |ui| assert_eq!(call(&mut app, ui.ctx(), "ui.resize", params.clone())["ok"], false));
            output.textures_delta.clear();
            assert!(!output.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::InnerSize(_)))));
            assert!(app.ui.dock.containers[0].collapsed);
        }
    }

    #[test]
    fn panel_screenshots_reject_retracted_hosts_without_restoring_them() {
        use crate::{
            dock::Group,
            panel_workspace::{Container, Tab},
        };
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.ui.panels.brush_settings = true;
        app.ui.dock.containers = vec![Container { id: 91, tabs: vec![Tab { group: Group::Brush, index: 0 }], floating: true, ..Default::default() }];
        assert!(screenshot_viewport(&app, &ctx, &json!({"panel":91})).is_ok());
        crate::dock::rail_click(&mut app, Group::Brush, false);
        assert!(screenshot_viewport(&app, &ctx, &json!({"panel":91})).is_err());
        assert!(app.ui.dock.containers[0].collapsed);
        for params in [json!({"panel":"91"}), json!({"panel":-1}), json!({"panel":999}), json!({"panel":91,"dialog":"menu"}), json!({"panel":91,"window":7})] {
            assert!(screenshot_viewport(&app, &ctx, &params).is_err());
        }
        crate::dock::rail_click(&mut app, Group::Brush, false);
        assert!(screenshot_viewport(&app, &ctx, &json!({"panel":91})).is_ok());
        ctx.set_embed_viewports(true);
        assert!(screenshot_viewport(&app, &ctx, &json!({"panel":91})).is_err());
    }

    #[test]
    fn screenshots_select_only_open_native_surfaces() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        assert_eq!(screenshot_viewport(&app, &ctx, &json!({})).unwrap(), egui::ViewportId::ROOT);
        assert!(screenshot_viewport(&app, &ctx, &json!({"window": 7})).is_err());
        assert!(screenshot_viewport(&app, &ctx, &json!({"dialog": "discard"})).is_err());
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.ui.windows.push(crate::state::DocWindow::new(7, 0, Default::default()));
        assert_eq!(screenshot_viewport(&app, &ctx, &json!({"window": 7})).unwrap(), egui::ViewportId::from_hash_of(("docwin", 7_u64)));
        assert!(crate::discard_ui::intercept(&mut app, "file.close", &Value::Null));
        assert_eq!(screenshot_viewport(&app, &ctx, &json!({"dialog": "discard"})).unwrap(), crate::discard_ui::native_viewport_id());
        for params in
            [json!({"window": 7, "dialog": "discard"}), json!({"window": "7"}), json!({"window": -1}), json!({"dialog": 7}), json!({"dialog": "unknown"})]
        {
            assert!(screenshot_viewport(&app, &ctx, &params).is_err(), "{params}");
        }
        ctx.set_embed_viewports(true);
        assert!(screenshot_viewport(&app, &ctx, &json!({"window": 7})).is_err());
        assert!(screenshot_viewport(&app, &ctx, &json!({"dialog": "discard"})).is_err());
        ctx.set_embed_viewports(false);
        app.ui.windows[0].open = false;
        assert!(screenshot_viewport(&app, &ctx, &json!({"window": 7})).is_err());
    }

    #[test]
    fn brush_picker_control_validates_targets_and_exposes_its_own_surface() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":32,"height":32})).unwrap();
        app.ui.tool = crate::state::Tool::Brush;
        for params in [json!({"x":1}), json!({"x":0,"y":"bad"}), json!({"x":0,"y":0,"window":999})] {
            assert_eq!(call(&mut app, &ctx, "ui.brushPicker.open", params)["ok"], false);
            assert!(crate::brush_picker::inspect(&ctx).is_null());
        }
        assert_eq!(call(&mut app, &ctx, "ui.brushPicker.open", json!({"x":350,"y":220}))["ok"], true);
        let viewport = screenshot_viewport(&app, &ctx, &json!({"dialog":"brushPicker"})).unwrap();
        assert_ne!(viewport, egui::ViewportId::ROOT);
        assert_eq!(call(&mut app, &ctx, "ui.brushPicker.resize", json!({"width":0,"height":10}))["ok"], false);
        assert_eq!(call(&mut app, &ctx, "ui.brushPicker.resize", json!({"width":700,"height":600}))["ok"], true);
        assert_eq!(call(&mut app, &ctx, "ui.brushPicker.close", json!({}))["ok"], true);
        assert!(screenshot_viewport(&app, &ctx, &json!({"dialog":"brushPicker"})).is_err());
    }

    #[test]
    fn new_brush_preset_screenshot_requires_an_open_native_dialog() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let params = json!({"dialog":"newBrushPreset"});
        assert!(screenshot_viewport(&app, &ctx, &params).is_err());
        let brush = app.session.tools.brush.clone();
        let id = crate::brush_panel::open_new_preset(&mut app, &brush);
        assert_eq!(screenshot_viewport(&app, &ctx, &params).unwrap(), crate::dialogs::native_preset_viewport(&app).unwrap());
        ctx.set_embed_viewports(true);
        assert!(screenshot_viewport(&app, &ctx, &params).is_err());
        app.ui.close_dialog(id);
        ctx.set_embed_viewports(false);
        assert!(screenshot_viewport(&app, &ctx, &params).is_err());
    }

    #[test]
    fn screenshot_paths_still_require_the_existing_automation_authority() {
        let services = crate::Services {
            encode_png: Some(Box::new(|_, _, _| Ok(vec![1, 2, 3]))),
            write: Some(Box::new(|_, _| panic!("screenshots must never use the ambient writer"))),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let image = egui::ColorImage::filled([1, 1], egui::Color32::WHITE);
        assert_eq!(save_screenshot(&mut app, &image, Some("capture.png"))["error"], "automation write authority is not configured");
        app.services.automation_write = Some(Box::new(|path, _| Err(format!("authority denied {path}"))));
        for path in ["../escape.png", "C:/outside.png", "capture.png"] {
            assert_eq!(save_screenshot(&mut app, &image, Some(path))["error"], format!("authority denied {path}"));
        }
        assert_eq!(save_screenshot(&mut app, &image, None)["ok"], true, "inline captures need no filesystem authority");
    }

    fn call(app: &mut PhotocraftApp, ctx: &egui::Context, method: &str, params: Value) -> Value {
        let (req, _rx) = ControlRequest::new(method, params);
        match handle(app, ctx, &req) {
            Outcome::Done(v) => v,
            _ => panic!("{method}: expected an immediate reply"),
        }
    }

    #[test]
    fn right_pointer_opens_agent_visible_selection_menu() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("select.rect", json!({"x": 1, "y": 1, "width": 8, "height": 8})).unwrap();
        let result = call(
            &mut app,
            &ctx,
            "ui.pointer",
            json!({
                "tool": "RectMarquee", "button": "secondary",
                "events": [{"kind": "down", "x": 4, "y": 4}, {"kind": "up", "x": 4, "y": 4}]
            }),
        );
        assert_eq!(result.get("ok"), Some(&Value::Bool(true)));
        let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
        let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap();
        assert!(entries.iter().any(|entry| entry.get("id") == Some(&json!("select.inverse")) && entry.get("enabled") == Some(&json!(true))));
        assert!(app.session.active().unwrap().doc.selection.is_some(), "right-click must not edit selection");
    }

    #[test]
    fn agent_can_inspect_and_choose_pen_make_selection_from_full_context_menu() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("path.set", json!({"name":"work","path":{"subpaths":[{"closed":true,"knots":[[2,2],[20,2],[20,20]]}]}})).unwrap();
        let expected_pos = screen_point(&app, 8.0, 8.0);
        let result = call(
            &mut app,
            &ctx,
            "ui.pointer",
            json!({
                "tool": "Pen", "button": "secondary", "events": [{"kind":"down","x":8,"y":8},{"kind":"up","x":8,"y":8}]
            }),
        );
        assert_eq!(result["ok"], true);
        let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(app.ui.canvas_tool_menu.as_ref().unwrap().pos, expected_pos);
        let entries = inspected.pointer("/result/canvasToolMenu/entries").and_then(Value::as_array).unwrap();
        assert_eq!(entries.iter().filter(|row| row.get("separator") == Some(&json!(true))).count(), 9);
        assert!(entries.iter().any(|row| row.get("id") == Some(&json!("path.toSelection")) && row.get("enabled") == Some(&json!(true))));
        assert_eq!(call(&mut app, &ctx, "ui.context.choose", json!({"id":"file.new"}))["ok"], false);
        let chosen = call(&mut app, &ctx, "ui.context.choose", json!({"id":"path.toSelection"}));
        assert_eq!(chosen["ok"], true);
        let dialog = chosen["result"]["dialog"].as_u64().unwrap();
        assert_eq!(call(&mut app, &ctx, "ui.dialog.confirm", json!({"dialog":dialog}))["ok"], true);
        assert!(app.session.active().unwrap().doc.selection.is_some());
    }

    #[test]
    fn engine_execute_runs_with_defaults_but_menu_invoke_opens_the_dialog() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("edit.fill", json!({"color": "#808080"})).unwrap();
        let rev = app.session.active().unwrap().revision;
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "filter.blur.gaussianBlur"}));
        assert!(!r.to_string().contains("\"dialog\""), "engine.execute opened a dialog: {r}");
        assert!(app.session.active().unwrap().revision > rev, "engine.execute didn't run the filter");
        let rev = app.session.active().unwrap().revision;
        let r = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "filter.blur.gaussianBlur"}));
        assert!(r.to_string().contains("dialog"), "ui.menu.invoke should open the dialog: {r}");
        assert_eq!(app.session.active().unwrap().revision, rev, "opening a dialog must not edit the document");
    }

    #[test]
    fn ui_set_rejects_unknown_fields_before_changing_anything() {
        use crate::theme::ThemeKind;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        app.set_theme(&ctx, ThemeKind::StudioLight);
        // `dark` was advertised to MCP clients but never read; a typo looked like success too.
        for params in [json!({"dark": true}), json!({"thme": "classic"}), json!({"tool": "move", "dark": true})] {
            let r = call(&mut app, &ctx, "ui.set", params.clone());
            assert_eq!(r["ok"], false, "{params}: {r}");
            assert!(r["error"].as_str().unwrap().contains("unknown field"), "{r}");
        }
        assert_eq!(app.ui.theme, ThemeKind::StudioLight);
        assert_eq!(app.ui.tool, Tool::Brush, "a rejected call applies none of its fields");
        // Every field the method reads gets past the check (a bad value is its own error).
        for field in UI_SET_FIELDS {
            let r = call(&mut app, &ctx, "ui.set", json!({ field: null }));
            assert!(!r.to_string().contains("unknown field"), "{field}: {r}");
        }
        assert_eq!(call(&mut app, &ctx, "ui.set", Value::Null)["ok"], true);
    }

    #[test]
    fn ui_set_gradient_blend_mode_validates_and_updates_options() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let good = call(&mut app, &ctx, "ui.set", json!({"tool": "gradient", "gradientBlendMode": "Difference", "gradientClassic": true}));
        assert_eq!(good["ok"], true, "{good}");
        assert_eq!(app.ui.tool_options.gradient_blend_mode, photocraft_color::BlendMode::Difference);
        assert!(app.ui.tool_options.gradient_classic);
        let bad = call(&mut app, &ctx, "ui.set", json!({"gradientBlendMode": "nonsense", "gradientClassic": false}));
        assert_eq!(bad["ok"], false, "{bad}");
        assert!(app.ui.tool_options.gradient_classic, "invalid mode must not change options");
    }

    #[test]
    fn ui_set_color_panel_picks_the_edited_chip() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"colorPanel": {"background": true}}))["ok"], true);
        assert!(app.ui.color_panel.background);
        assert_eq!(call(&mut app, &ctx, "ui.set", json!({"colorPanel": {"background": "yes"}}))["ok"], false);
        assert!(app.ui.color_panel.background, "a bad value changes nothing");
    }

    #[test]
    fn ui_set_unknown_theme_error_names_every_theme_and_each_name_works() {
        use crate::theme::ThemeKind;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let r = call(&mut app, &ctx, "ui.set", json!({"theme": "nope"}));
        assert_eq!(r["ok"], false);
        let error = r["error"].as_str().unwrap();
        for kind in ThemeKind::ALL {
            assert!(error.contains(kind.id()), "{error} lacks {}", kind.id());
            assert_eq!(call(&mut app, &ctx, "ui.set", json!({"theme": kind.id()}))["ok"], true);
            assert_eq!(app.ui.theme, kind);
            assert_eq!(ThemeKind::from_name(kind.id()), Some(kind));
        }
    }

    #[test]
    fn ui_set_brush_size_dispatches_a_journaled_brush_command() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();

        let r = call(&mut app, &ctx, "ui.set", json!({"brushSize": 42.5}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(app.session.tools.brush.size, 42.5);
        let (id, params) = app.session.journal.last().cloned().expect("brush change is journaled");
        assert_eq!(id, "tools.setBrush");
        assert_eq!(params, json!({"brush": {"size": 42.5}}));

        // The control API has historically ignored non-numeric optional values.
        for params in [json!({}), json!({"brushSize": null}), json!({"brushSize": "large"}), json!({"brushSize": true})] {
            let journal_len = app.session.journal.len();
            let r = call(&mut app, &ctx, "ui.set", params);
            assert_eq!(r["ok"], true, "{r}");
            assert_eq!(app.session.tools.brush.size, 42.5);
            assert_eq!(app.session.journal.len(), journal_len);
        }

        // Values that cannot be represented by BrushSettings must report the command error and
        // leave both the brush and journal unchanged instead of mutating tool state directly.
        let journal_len = app.session.journal.len();
        let r = call(&mut app, &ctx, "ui.set", json!({"brushSize": f64::MAX}));
        assert_eq!(r["ok"], false, "{r}");
        assert!(r["error"].as_str().is_some(), "{r}");
        assert_eq!(app.session.tools.brush.size, 42.5);
        assert_eq!(app.session.journal.len(), journal_len);
    }

    #[test]
    fn move_options_are_drivable_and_reject_invalid_values_before_mutation() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let r = call(&mut app, &ctx, "ui.set", json!({"tool": "move", "moveAutoSelect": false, "moveTarget": "group", "moveShowTransform": true}));
        assert_eq!(r["ok"], true, "{r}");
        let inspected = call(&mut app, &ctx, "ui.inspect", json!({}));
        assert_eq!(inspected["result"]["toolOptions"]["move_auto_select"], false);
        assert_eq!(inspected["result"]["toolOptions"]["move_target"], "group");
        assert_eq!(inspected["result"]["toolOptions"]["move_show_transform"], true);
        let before = app.ui.tool_options.clone();
        for invalid in [
            json!({"tool": "brush", "moveAutoSelect": "false"}),
            json!({"moveAutoSelect": true, "moveTarget": "pixels"}),
            json!({"moveTarget": null}),
            json!({"moveAutoSelect": true, "moveShowTransform": 1}),
        ] {
            let r = call(&mut app, &ctx, "ui.set", invalid);
            assert_eq!(r["ok"], false, "{r}");
            assert_eq!(app.ui.tool, Tool::Move);
            assert_eq!(app.ui.tool_options, before);
        }
    }

    #[test]
    fn object_gesture_options_validate_before_mutating_tools_v93() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        for (name, shape) in [
            ("rectangle", crate::state::ObjectSelectionShape::Rectangle),
            ("lasso", crate::state::ObjectSelectionShape::Lasso),
            ("foregroundStroke", crate::state::ObjectSelectionShape::ForegroundStroke),
        ] {
            let result = call(&mut app, &ctx, "ui.set", json!({"tool":"objectSelection","objectSelectionShape":name,"smartObjectSelection":true}));
            assert_eq!(result["ok"], true, "{result}");
            assert_eq!(app.ui.tool_options.object_selection_shape, shape);
            assert!(app.ui.tool_options.smart_object_selection);
        }
        let before = app.ui.tool_options.clone();
        for mut invalid in [json!({"objectSelectionShape":"bad"}), json!({"objectSelectionShape":1}), json!({"smartObjectSelection":"true"})] {
            invalid["tool"] = json!("brush");
            let result = call(&mut app, &ctx, "ui.set", invalid);
            assert_eq!(result["ok"], false, "{result}");
            assert_eq!(app.ui.tool, Tool::ObjectSelection);
            assert_eq!(app.ui.tool_options, before);
        }
    }

    #[test]
    fn preferences_apply_is_available_over_control_without_closing() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let r = call(&mut app, &ctx, "ui.menu.invoke", json!({"id": "edit.preferences.interface"}));
        let id = r["result"]["dialog"].as_u64().unwrap();
        let mut values = app.ui.dialog_mut(id).unwrap().fields["values"].clone();
        values["interface"]["uiScale"] = json!("200");
        assert_eq!(call(&mut app, &ctx, "ui.dialog.set", json!({"dialog": id, "field": "values", "value": values}))["ok"], true);
        assert_eq!(call(&mut app, &ctx, "ui.dialog.apply", json!({"dialog": id}))["ok"], true);
        assert_eq!(app.session.prefs().interface.ui_scale, photocraft_engine::prefs::UiScale::P200);
        assert!(app.ui.dialog_mut(id).is_some());
        assert_eq!(call(&mut app, &ctx, "ui.dialog.cancel", json!({"dialog": id}))["ok"], true);
        assert_eq!(app.session.prefs().interface.ui_scale, photocraft_engine::prefs::UiScale::P200);
        for params in [json!({}), json!({"dialog": "invalid"}), json!({"dialog": u64::MAX})] {
            assert_eq!(call(&mut app, &ctx, "ui.dialog.apply", params)["ok"], false);
        }
    }

    #[test]
    fn preferences_apply_respects_the_automation_command_policy() {
        let services = crate::Services {
            automation_command: Some(Box::new(|id, _| if id == "prefs.set" { Err("preference changes denied".into()) } else { Ok(()) })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = egui::Context::default();
        let before = app.session.prefs().to_json();
        let id = crate::prefs_ui::open_preferences(&mut app, "interface");
        app.ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap()["interface"]["uiScale"] = json!("200");
        let r = call(&mut app, &ctx, "ui.dialog.apply", json!({"dialog": id}));
        assert_eq!(r["ok"], false);
        assert_eq!(r["error"], "preference changes denied");
        assert_eq!(app.session.prefs().to_json(), before);
        assert!(app.ui.dialog_mut(id).is_some());
    }

    #[test]
    fn synthetic_shortcuts_use_the_automation_command_policy() {
        let services = crate::Services {
            automation_command: Some(Box::new(|id, _| if id.starts_with("file.") { Err("filesystem command denied".into()) } else { Ok(()) })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = egui::Context::default();
        app.automation_input = true;
        let error = crate::menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap_err();
        assert_eq!(error, "filesystem command denied");
    }

    #[test]
    fn automation_open_and_save_use_the_roots_and_reply_with_warnings() {
        use photocraft_color::{ColorMode, SampleType};
        use photocraft_doc::Document;
        use photocraft_geom::Size;
        use std::cell::RefCell;
        use std::rc::Rc;

        // Without granted roots both fail closed, even with an exporter and an ambient writer.
        let services = crate::Services {
            export: Some(Box::new(|_d: &Document, _p: &str, _s: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
            write: Some(Box::new(|_p: &str, _b: &[u8]| Err("ambient writer used".into()))),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let ctx = egui::Context::default();
        let r = call(&mut app, &ctx, "app.open", json!({"path": "in/warn.psd"}));
        assert!(r.to_string().contains("automation read authority is not configured"), "{r}");
        app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        let r = call(&mut app, &ctx, "app.save", json!({"path": "out.png"}));
        assert!(r.to_string().contains("automation write authority is not configured"), "{r}");

        let written: Rc<RefCell<Vec<String>>> = Rc::default();
        let w = written.clone();
        let services = crate::Services {
            import: Some(Box::new(|name: &str, _b: &[u8]| {
                Ok((Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), vec!["Adjustment layer flattened".to_string()]))
            })),
            export: Some(Box::new(|_d: &Document, _p: &str, _s: &crate::ExportSettings| Ok((b"out".to_vec(), vec!["Layers were flattened".to_string()])))),
            automation_read: Some(Box::new(|path: &str| Ok((path.rsplit('/').next().unwrap_or(path).to_string(), b"x".to_vec())))),
            automation_write: Some(Box::new(move |path: &str, _b: &[u8]| {
                w.borrow_mut().push(path.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let r = call(&mut app, &ctx, "app.open", json!({"path": "in/warn.psd"}));
        assert_eq!(r["result"]["warnings"], json!(["Adjustment layer flattened"]), "{r}");
        assert_eq!(r["result"]["name"], "warn.psd");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("in/warn.psd"));
        assert_eq!(app.ui.recent_files, vec!["in/warn.psd".to_string()]);
        let r = call(&mut app, &ctx, "app.save", json!({"path": "out.png"}));
        assert_eq!(r["result"], json!({"path": "out.png", "warnings": ["Layers were flattened"]}), "{r}");
        assert_eq!(*written.borrow(), vec!["out.png".to_string()]);
        // Without `path`, only a layered file is written back, like File › Save (#416).
        call(&mut app, &ctx, "app.open", json!({"path": "in/flat.jpg"}));
        let r = call(&mut app, &ctx, "app.save", json!({}));
        assert!(r["error"].as_str().unwrap().contains("pass `path`"), "{r}");
        assert_eq!(written.borrow().len(), 1, "nothing written over the JPEG");
        call(&mut app, &ctx, "app.open", json!({"path": "in/layered.psd"}));
        let r = call(&mut app, &ctx, "app.save", json!({}));
        assert_eq!(r["result"]["path"], "in/layered.psd", "{r}");
        assert_eq!(written.borrow().last().map(String::as_str), Some("in/layered.psd"));
        // A template opens untitled, so a save without `path` never writes over it.
        let r = call(&mut app, &ctx, "app.open", json!({"path": "in/card.psdt"}));
        assert_eq!(r["result"]["name"], "Untitled-1", "{r}");
        assert_eq!(app.session.active().unwrap().path, None);
        let r = call(&mut app, &ctx, "app.save", json!({}));
        assert!(r["error"].as_str().unwrap().contains("pass `path`"), "{r}");
    }

    fn deny_ambient_file(id: &str, _: &serde_json::Value) -> photocraft_engine::Result<()> {
        if id.starts_with("file.") && id != "file.new" {
            Err(photocraft_engine::EngineError::Other(format!("automation command `{id}` is disabled")))
        } else {
            Ok(())
        }
    }

    #[test]
    fn actions_play_checks_each_step_over_control() {
        let services = crate::Services { automation_authorize: Some(deny_ambient_file), ..Default::default() };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.session.actions.list.push(photocraft_engine::actions_cmds::Action {
            name: "Open".into(),
            steps: vec![("file.open".into(), json!({"path": "/etc/passwd"})), ("layer.new.layer".into(), json!({}))],
        });
        let ctx = egui::Context::default();
        let r = call(&mut app, &ctx, "engine.execute", json!({"command": "actions.play", "params": {"action": "Open"}}));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["result"]["ran"], 0, "{r}");
        assert_eq!(r["result"]["failed"]["id"], "file.open", "{r}");
        assert!(r["result"]["failed"]["error"].as_str().unwrap_or("").contains("disabled"), "{r}");
        assert!(app.session.documents().is_empty());
        assert!(app.session.authorize.is_none(), "the per-step gate is only installed for the request");
    }

    #[test]
    fn actions_play_checks_each_step_for_synthetic_input() {
        let services = crate::Services { automation_authorize: Some(deny_ambient_file), ..Default::default() };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.session.actions.list.push(photocraft_engine::actions_cmds::Action {
            name: "Open".into(),
            steps: vec![("file.open".into(), json!({"path": "/etc/passwd"})), ("layer.new.layer".into(), json!({}))],
        });
        app.automation_input = true;
        let r = app.run("actions.play", json!({"action": "Open"})).unwrap();
        assert_eq!(r["ran"], 0, "{r}");
        assert_eq!(r["failed"]["id"], "file.open", "{r}");
        assert!(app.session.authorize.is_none(), "the per-step gate is only installed for the command");

        // A local play (no automation input) still runs recorded steps.
        app.automation_input = false;
        app.session.actions.list[0].steps.remove(0);
        app.session.execute("file.new", json!({"width": 4, "height": 4})).unwrap();
        let r = app.run("actions.play", json!({"action": "Open"})).unwrap();
        assert_eq!(r["ran"], 1, "{r}");
    }
}
