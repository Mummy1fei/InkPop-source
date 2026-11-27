//! Shell-only Window and View items: Window › Workspace › New / Delete / Lock Workspace (saved
//! in the preferences), Window › Modifier Keys (sticky Shift/⌘/⌥ for touch and pen users),
//! View › Pixel Aspect Ratio › Custom Pixel Aspect Ratio…, View › Show › Show Extras Options…
//! and the View › 32-bit Preview Options dialog.
//!
//! State is [`ShellUi`] (serialisable, part of `UiState`); saved workspaces and the lock live in
//! `Preferences::workspaces` / `workspace_locked` so they persist with the preferences.

use crate::theme::palette::ScrollAreaPaletteExt;
use egui::{Align2, RichText, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// A View › Pixel Aspect Ratio › Custom entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomPar {
    pub name: String,
    pub ratio: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShellUi {
    /// Window › Modifier Keys panel open, and its latched keys.
    pub modifier_keys: bool,
    pub sticky_shift: bool,
    pub sticky_command: bool,
    pub sticky_alt: bool,
    /// Custom pixel aspect ratios (the active one is `ViewOptions::pixel_aspect` = `custom:<ratio>:<name>`).
    pub custom_pars: Vec<CustomPar>,
    /// Open dialog: (kind, fields). Kinds: newWorkspace, deleteWorkspace, customPar, preview32,
    /// extrasOptions.
    pub dialog: Option<(String, Map<String, Value>)>,
    #[serde(skip)]
    pub layout_epoch: u64,
    #[serde(skip)]
    pub restore_dock_width: bool,
    /// Let the popup's native hide commands complete before entering an OS file dialog.
    #[serde(skip)]
    workspace_file_action: Option<(u64, String)>,
}

const IDS: [&str; 11] = [
    "window.panel.modifierKeys",
    "window.workspace.newWorkspace",
    "window.workspace.deleteWorkspace",
    "window.workspace.lockWorkspace",
    "window.workspace.select",
    "window.workspace.save",
    "window.workspace.rename",
    "window.workspace.import",
    "window.workspace.export",
    "view.pixelAspectRatio.custom",
    "view.show.showExtrasOptions",
];

/// Menu ids handled here (live menu items).
pub fn handles(id: &str) -> bool {
    IDS.contains(&id)
}

pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    match id {
        "window.panel.modifierKeys" => Some(app.ui.shell.modifier_keys),
        "window.workspace.lockWorkspace" => Some(app.session.prefs().workspace_locked),
        "view.pixelAspectRatio.custom" => Some(app.ui.view.pixel_aspect.starts_with("custom:")),
        _ => None,
    }
}

pub fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    Some(match id {
        "window.workspace.deleteWorkspace" => !app.session.prefs().workspaces.is_empty(),
        "window.workspace.rename" => app.session.prefs().workspaces.contains_key(&app.ui.workspace),
        "view.thirtyTwoBitPreviewOptions" => app.session.is_enabled(id),
        i if handles(i) => true,
        _ => return None,
    })
}

/// Latched Modifier Keys panel keys added to the real ones.
pub fn sticky_mods(app: &PhotocraftApp, mut m: egui::Modifiers) -> egui::Modifiers {
    let s = &app.ui.shell;
    m.shift |= s.sticky_shift;
    m.alt |= s.sticky_alt;
    if s.sticky_command {
        m.command = true;
        if cfg!(target_os = "macos") {
            m.mac_cmd = true;
        } else {
            m.ctrl = true;
        }
    }
    m
}

fn no_params(p: &Value) -> bool {
    p.as_object().is_none_or(|o| o.is_empty())
}

fn fields(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

fn open(app: &mut PhotocraftApp, kind: &str, f: Value) -> Result<Value, String> {
    app.ui.shell.dialog = Some((kind.to_string(), fields(f)));
    Ok(json!({"dialog": kind}))
}

/// Built-in workspace names (Window › Workspace presets).
const PRESETS: [&str; 6] = ["Essentials", "Photography", "Painting", "Pixel Art", "Graphic and Web", "Motion"];

/// Run a shell command, or open its dialog. `None` when `id` isn't ours.
pub fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id == "view.thirtyTwoBitPreviewOptions" && no_params(params) {
        let d = app.session.active()?;
        let h = app.session.color.hdr.get(&d.doc.id).copied().unwrap_or_default();
        return Some(open(
            app,
            "preview32",
            json!({"method": if h.highlight_compression { "highlightCompression" } else { "exposureGamma" }, "exposure": h.exposure, "gamma": h.gamma}),
        ));
    }
    if !handles(id) {
        return None;
    }
    // Capture legacy floating-window geometry without switching the live layout renderer.
    let original_dock = (matches!(id, "window.workspace.newWorkspace" | "window.workspace.save" | "window.workspace.export")
        && app.ui.dock.containers.is_empty())
    .then(|| app.ui.dock.clone());
    if original_dock.is_some() {
        crate::panel_workspace::migrate(app, ctx);
    }
    let s = &mut app.ui.shell;
    let result = match id {
        "window.panel.modifierKeys" => {
            s.modifier_keys = params.get("show").and_then(Value::as_bool).unwrap_or(!s.modifier_keys);
            for (k, slot) in [("shift", &mut s.sticky_shift), ("command", &mut s.sticky_command), ("alt", &mut s.sticky_alt)] {
                if let Some(v) = params.get(k).and_then(Value::as_bool) {
                    *slot = v;
                }
            }
            Ok(json!({"visible": s.modifier_keys, "shift": s.sticky_shift, "command": s.sticky_command, "alt": s.sticky_alt}))
        }
        "window.workspace.lockWorkspace" => {
            let on = params.get("on").and_then(Value::as_bool).unwrap_or(!app.session.prefs().workspace_locked);
            app.session.prefs.edit(|p| p.workspace_locked = on);
            Ok(json!({"locked": on}))
        }
        "window.workspace.newWorkspace" if no_params(params) => {
            open(app, "newWorkspace", json!({"name": "", "keyboardShortcuts": false, "menus": false, "toolbar": false}))
        }
        "window.workspace.newWorkspace" => new_workspace(app, params),
        "window.workspace.save" => save_workspace(app),
        "window.workspace.rename" if no_params(params) => {
            if !app.session.prefs().workspaces.contains_key(&app.ui.workspace) {
                Err("built-in workspaces cannot be renamed; create a new workspace first".into())
            } else {
                open(app, "renameWorkspace", json!({"name": app.ui.workspace}))
            }
        }
        "window.workspace.rename" => rename_workspace(app, params),
        "window.workspace.import" => import_workspace(app, params),
        "window.workspace.export" => export_workspace(app, params),
        "window.workspace.deleteWorkspace" if no_params(params) => {
            let first = app.session.prefs().workspaces.keys().next().cloned().unwrap_or_default();
            open(app, "deleteWorkspace", json!({"name": first}))
        }
        "window.workspace.deleteWorkspace" => delete_workspace(app, params),
        "window.workspace.select" => select_workspace(app, params),
        "view.pixelAspectRatio.custom" if no_params(params) => open(app, "customPar", json!({"name": "", "ratio": 1.0})),
        "view.pixelAspectRatio.custom" => custom_par(app, params),
        "view.show.showExtrasOptions" if no_params(params) => {
            let show = serde_json::to_value(app.ui.view.show).unwrap_or_default();
            open(app, "extrasOptions", show)
        }
        "view.show.showExtrasOptions" => {
            let mut show = serde_json::to_value(app.ui.view.show).unwrap_or_default();
            if let (Some(o), Some(p)) = (show.as_object_mut(), params.as_object()) {
                for (k, v) in p {
                    if !o.contains_key(k) || !v.is_boolean() {
                        return Some(Err(format!("unknown Show item `{k}` ({})", o.keys().cloned().collect::<Vec<_>>().join("|"))));
                    }
                    o.insert(k.clone(), v.clone());
                }
            }
            app.ui.view.show = serde_json::from_value(show.clone()).map_err(|e| e.to_string()).ok()?;
            Ok(show)
        }
        _ => return None,
    };
    let captured_legacy = original_dock.is_some();
    if let Some(dock) = original_dock {
        app.ui.dock = dock;
    }
    if captured_legacy
        && matches!(id, "window.workspace.newWorkspace" | "window.workspace.save")
        && result.as_ref().is_ok_and(|value| value.get("workspace").is_some())
    {
        // A successful save must make the active layout and its restart snapshot agree.
        // Opening/cancelling a dialog and exporting never switch the live renderer.
        apply_custom(app);
        crate::dock::remember(app);
    }
    Some(result)
}

/// Window › Workspace › New Workspace…: save the panel layout (and optionally keyboard
/// shortcuts, menus and toolbar) under a name, and make it current.
fn new_workspace(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let name = workspace_name(p.get("name").and_then(Value::as_str).unwrap_or(""))?;
    if app.session.prefs().workspaces.contains_key(&name) {
        return Err(format!("a workspace named \"{name}\" already exists"));
    }
    if app.session.prefs().workspaces.len() >= MAX_WORKSPACES {
        return Err("too many saved workspaces (maximum 128)".into());
    }
    let prefs = app.session.prefs().clone();
    let mut ws = crate::dock::snapshot(app);
    if p.get("keyboardShortcuts").and_then(Value::as_bool) == Some(true) {
        ws["shortcuts"] = json!(prefs.shortcuts);
    }
    if p.get("menus").and_then(Value::as_bool) == Some(true) {
        ws["menus"] = serde_json::to_value(&prefs.menus).unwrap_or_default();
    }
    if p.get("toolbar").and_then(Value::as_bool) == Some(true) {
        ws["toolbar"] = serde_json::to_value(&prefs.toolbar).unwrap_or_default();
    }
    let replaced = prefs.workspaces.contains_key(&name);
    app.session.prefs.edit(|pr| pr.workspaces.insert(name.clone(), ws));
    app.ui.workspace = name.clone();
    Ok(json!({"workspace": name, "replaced": replaced}))
}

fn delete_workspace(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let name = p.get("name").and_then(Value::as_str).ok_or("pass `name`")?.to_string();
    if !app.session.prefs().workspaces.contains_key(&name) {
        return Err(format!("no saved workspace \"{name}\""));
    }
    // Like Photoshop, the current workspace can't be deleted.
    if app.ui.workspace == name {
        return Err(format!("\"{name}\" is the current workspace; switch to another first"));
    }
    app.session.prefs.edit(|pr| pr.workspaces.remove(&name));
    Ok(json!({"deleted": name}))
}

fn select_workspace(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let name = p.get("name").and_then(Value::as_str).ok_or("pass `name`")?.to_string();
    if PRESETS.contains(&name.as_str()) {
        app.ui.workspace = name.clone();
        crate::menus::apply_workspace(app);
        return Ok(json!({"workspace": name}));
    }
    if !app.session.prefs().workspaces.contains_key(&name) {
        return Err(format!("no workspace \"{name}\""));
    }
    app.ui.workspace = name.clone();
    apply_custom(app);
    Ok(json!({"workspace": name}))
}

/// Apply the current workspace when it is a saved one. Returns false for the built-in presets.
pub fn apply_custom(app: &mut PhotocraftApp) -> bool {
    let Some(ws) = app.session.prefs().workspaces.get(&app.ui.workspace).cloned() else { return false };
    // Workspaces saved before the dock layout existed get the default heights.
    app.ui.dock = Default::default();
    crate::dock::apply(app, &ws);
    let (sc, menus, toolbar) = (ws.get("shortcuts").cloned(), ws.get("menus").cloned(), ws.get("toolbar").cloned());
    if sc.is_some() || menus.is_some() || toolbar.is_some() {
        app.session.prefs.edit(|p| {
            if let Some(v) = sc.and_then(|v| serde_json::from_value(v).ok()) {
                p.shortcuts = v;
            }
            if let Some(v) = menus.and_then(|v| serde_json::from_value(v).ok()) {
                p.menus = v;
            }
            if let Some(v) = toolbar.and_then(|v| serde_json::from_value(v).ok()) {
                p.toolbar = v;
            }
        });
    }
    true
}

const MAX_WORKSPACES: usize = 128;
const MAX_NAME: usize = 80;
const MAX_FILE_BYTES: usize = 1_048_576;

fn workspace_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME || name.chars().any(char::is_control) {
        return Err("workspace name must contain 1–80 characters without control characters".into());
    }
    if PRESETS.iter().any(|p| p.eq_ignore_ascii_case(name)) {
        return Err(format!("\"{name}\" is a built-in workspace; choose another name"));
    }
    Ok(name.to_string())
}

fn save_workspace(app: &mut PhotocraftApp) -> Result<Value, String> {
    let name = app.ui.workspace.clone();
    let Some(old) = app.session.prefs().workspaces.get(&name).cloned() else {
        return open(app, "newWorkspace", json!({"name": ""}));
    };
    let mut layout = crate::dock::snapshot(app);
    let prefs = app.session.prefs();
    for (key, value) in [("shortcuts", json!(prefs.shortcuts)), ("menus", json!(prefs.menus)), ("toolbar", json!(prefs.toolbar))] {
        if old.get(key).is_some() {
            layout[key] = value;
        }
    }
    app.session.prefs.edit(|p| p.workspaces.insert(name.clone(), layout));
    Ok(json!({"workspace": name, "saved": true}))
}

fn rename_workspace(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let name = workspace_name(p.get("name").and_then(Value::as_str).ok_or("pass `name`")?)?;
    let old = app.ui.workspace.clone();
    let mut layout = app.session.prefs().workspaces.get(&old).cloned().ok_or("only saved workspaces can be renamed")?;
    if name != old && app.session.prefs().workspaces.contains_key(&name) {
        return Err(format!("a workspace named \"{name}\" already exists"));
    }
    layout["workspace"] = json!(name);
    app.session.prefs.edit(|prefs| {
        prefs.workspaces.remove(&old);
        prefs.workspaces.insert(name.clone(), layout);
    });
    app.ui.workspace = name.clone();
    crate::dock::remember(app);
    Ok(json!({"workspace": name, "renamed": old}))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkspaceFile {
    format: String,
    version: u32,
    name: String,
    layout: Value,
}

/// Imports are checked before any live UI or preferences are changed.
fn validate_layout(layout: &Value) -> Result<(), String> {
    let object = layout.as_object().ok_or("workspace layout must be an object")?;
    const KEYS: [&str; 12] = [
        "workspace",
        "panels",
        "dockTabs",
        "dock",
        "colorPanel",
        "currentToolOnly",
        "shortcuts",
        "menus",
        "toolbar",
        "colorTabOrder",
        "toolPresetView",
        "toolPresetSortByTool",
    ];
    if let Some(key) = object.keys().find(|key| !KEYS.contains(&key.as_str())) {
        return Err(format!("unknown workspace layout field `{key}`"));
    }
    if layout.get("colorTabOrder").is_some_and(|value| !matches!(value.as_str(), Some("colorFirst" | "swatchesFirst"))) {
        return Err("unknown workspace color tab order".into());
    }
    let read = |key: &str| layout.get(key).cloned().ok_or_else(|| format!("workspace layout is missing `{key}`"));
    let _: crate::state::Panels = serde_json::from_value(read("panels")?).map_err(|e| e.to_string())?;
    let tabs: crate::state::DockTabs = serde_json::from_value(read("dockTabs")?).map_err(|e| e.to_string())?;
    let dock: crate::dock::DockLayout = serde_json::from_value(read("dock")?).map_err(|e| e.to_string())?;
    if dock.width.is_some_and(|w| !w.is_finite() || !(250.0..=520.0).contains(&w)) {
        return Err("workspace dock width must be 250–520".into());
    }
    if dock.order.len() > 8
        || dock.floating.len() > 8
        || dock.collapsed.len() > 8
        || dock.containers.len() > 32
        || dock.heights.values().any(|h| !h.is_finite() || !(36.0..=4000.0).contains(h))
    {
        return Err("workspace contains invalid panel counts or heights".into());
    }
    let mut tab_keys = std::collections::BTreeSet::new();
    let mut ids = std::collections::BTreeSet::new();
    for c in &dock.containers {
        if !ids.insert(c.id)
            || c.tabs.is_empty()
            || c.tabs.len() > 32
            || c.selected >= c.tabs.len()
            || c.position.iter().any(|v| !v.is_finite() || v.abs() > 100_000.0)
            || c.size.iter().any(|v| !v.is_finite() || !(36.0..=8192.0).contains(v))
            || !c.height.is_finite()
            || !(36.0..=4000.0).contains(&c.height)
        {
            return Err("workspace contains invalid panel geometry or identifiers".into());
        }
        for tab in &c.tabs {
            if tab.index >= tab.group.tabs(false).len() || !tab_keys.insert((tab.group, tab.index)) {
                return Err("workspace contains an invalid or duplicate panel tab".into());
            }
        }
        if let Some(parent) = c.stack
            && (parent == c.id || !dock.containers.iter().any(|root| root.id == parent && root.stack.is_none() && root.floating))
        {
            return Err("workspace contains an invalid panel stack".into());
        }
    }
    let mut tabs = tabs;
    for group in crate::dock::Group::ALL {
        if *group.tab_mut(&mut tabs) >= group.tabs(false).len() {
            return Err("workspace contains an invalid selected tab".into());
        }
    }
    // Validate optional customization payloads using the same types as preferences/UI.
    let mut prefs = photocraft_engine::prefs::Preferences::default();
    if let Some(v) = layout.get("shortcuts") {
        prefs.shortcuts = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    }
    if let Some(v) = layout.get("menus") {
        prefs.menus = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    }
    if let Some(v) = layout.get("toolbar") {
        prefs.toolbar = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    }
    if let Some(v) = layout.get("colorPanel") {
        let _: crate::state::ColorPanelState = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    }
    if layout.get("currentToolOnly").is_some_and(|v| !v.is_boolean()) {
        return Err("currentToolOnly must be boolean".into());
    }
    if let Some(view) = layout.get("toolPresetView") {
        let _: crate::preset_panels::ToolPresetView = serde_json::from_value(view.clone()).map_err(|e| format!("invalid tool preset view: {e}"))?;
    }
    if layout.get("toolPresetSortByTool").is_some_and(|v| !v.is_boolean()) {
        return Err("toolPresetSortByTool must be boolean".into());
    }
    Ok(())
}

fn import_workspace(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let bytes = if let Some(value) = p.get("data") {
        serde_json::to_vec(value).map_err(|e| e.to_string())?
    } else if let Some(path) = p.get("path").and_then(Value::as_str) {
        app.services.automation_read.as_mut().ok_or("workspace file access is unavailable")?(path)?.1
    } else {
        let Some((_, bytes)) = app.services.pick_open_workspace.as_mut().ok_or("workspace import is unavailable on this platform")?() else {
            return Ok(json!({"cancelled": true}));
        };
        bytes?
    };
    if bytes.len() > MAX_FILE_BYTES {
        return Err("workspace file exceeds 1 MiB".into());
    }
    let file: WorkspaceFile = serde_json::from_slice(&bytes).map_err(|e| format!("invalid workspace file: {e}"))?;
    if file.format != "PhotoCraft Workspace" || file.version != 1 {
        return Err("unsupported workspace file format or version".into());
    }
    let base = workspace_name(p.get("name").and_then(Value::as_str).unwrap_or(&file.name))?;
    validate_layout(&file.layout)?;
    if app.session.prefs().workspaces.len() >= MAX_WORKSPACES {
        return Err("too many saved workspaces (maximum 128)".into());
    }
    let mut name = base.clone();
    for suffix in 2..=MAX_WORKSPACES + 2 {
        if !app.session.prefs().workspaces.contains_key(&name) {
            break;
        }
        let stem: String = base.chars().take(MAX_NAME.saturating_sub(6)).collect();
        name = format!("{stem} ({suffix})");
    }
    let mut layout = file.layout;
    layout["workspace"] = json!(name);
    app.session.prefs.edit(|prefs| prefs.workspaces.insert(name.clone(), layout));
    app.ui.workspace = name.clone();
    apply_custom(app);
    crate::dock::remember(app);
    Ok(json!({"workspace": name, "imported": true}))
}

fn export_workspace(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let name = if PRESETS.contains(&app.ui.workspace.as_str()) { format!("{} Layout", app.ui.workspace) } else { app.ui.workspace.clone() };
    let suggested = export_filename(&name);
    let file = WorkspaceFile { format: "PhotoCraft Workspace".into(), version: 1, name, layout: crate::dock::snapshot(app) };
    validate_layout(&file.layout)?;
    let data = serde_json::to_value(file).map_err(|e| e.to_string())?;
    if p.get("dataOnly").and_then(Value::as_bool) == Some(true) {
        return Ok(data);
    }
    let bytes = serde_json::to_vec_pretty(&data).map_err(|e| e.to_string())?;
    let path = if let Some(path) = p.get("path").and_then(Value::as_str) {
        app.services.automation_write.as_mut().ok_or("workspace file access is unavailable")?(path, &bytes)?;
        path.to_string()
    } else {
        let pick = app.services.pick_save_workspace.as_mut().or(app.services.pick_save.as_mut()).ok_or("workspace export is unavailable on this platform")?;
        let Some(path) = pick(&suggested) else {
            return Ok(json!({"cancelled": true}));
        };
        app.services.write.as_mut().ok_or("workspace export is unavailable on this platform")?(&path, &bytes)?;
        path
    };
    Ok(json!({"exported": path}))
}

/// Workspace names are display text, not paths. Keep Unicode while avoiding Windows
/// separators/reserved characters; the prefix also makes names such as CON safe.
fn export_filename(name: &str) -> String {
    let stem: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let stem = stem.trim_matches([' ', '.']);
    if stem.is_empty() { "InkPop-workspace.json".into() } else { format!("InkPop-{stem}.json") }
}

/// Top-right selector includes built-in and saved layouts plus management actions.
pub fn selector(app: &mut PhotocraftApp, ui: &mut egui::Ui, width: f32) {
    let current = app.ui.workspace.clone();
    let custom: Vec<String> = app.session.prefs().workspaces.keys().cloned().collect();
    let mut action: Option<(&str, Value)> = None;
    let label = if PRESETS.contains(&current.as_str()) { tl!(&current) } else { &current };
    let mut contents = |app: &mut PhotocraftApp, ui: &mut egui::Ui| {
        for name in PRESETS {
            if ui.selectable_label(current == name, tl!(name)).clicked() {
                action = Some(("window.workspace.select", json!({"name": name})));
            }
        }
        if !custom.is_empty() {
            ui.separator();
        }
        egui::ScrollArea::vertical().id_salt("workspace-custom-list").max_height(120.0).show_palette(ui, |ui| {
            for name in &custom {
                if ui.selectable_label(&current == name, name).clicked() {
                    action = Some(("window.workspace.select", json!({"name": name})));
                }
            }
        });
        ui.separator();
        for (id, label) in [
            ("window.workspace.newWorkspace", tl!("New Workspace")),
            ("window.workspace.save", tl!("Save Workspace")),
            ("window.workspace.rename", tl!("Rename Workspace")),
            ("window.workspace.import", tl!("Import Workspace...")),
            ("window.workspace.export", tl!("Export Workspace...")),
        ] {
            if ui.add_enabled(is_enabled(app, id).unwrap_or(true), egui::Button::new(label).frame(true).frame_when_inactive(false)).clicked() {
                action = Some((id, json!({})));
            }
        }
        if action.is_some() {
            ui.close();
        }
        action.is_some()
    };
    if ui.ctx().embed_viewports() {
        egui::ComboBox::from_id_salt("workspace").selected_text(label).width(width).height(640.0).icon(crate::widgets::chevron_icon).show_ui(ui, |ui| {
            contents(app, ui);
        });
    } else {
        let ctx = ui.ctx().clone();
        let response = ui.add_sized(vec2(width, ui.spacing().interact_size.y), egui::Button::new(format!("{label}  ▾")));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, tl!("Workspace")));
        if response.clicked() {
            crate::native_popup_ui::toggle(&ctx, "workspace-selector", response.rect.left_bottom(), app.frame);
        }
        crate::native_popup_ui::show(app, &ctx, "workspace-selector", tl!("Workspace"), vec2(280.0, 510.0), contents);
    }
    if let Some((id, _)) = action.as_ref().filter(|(id, _)| matches!(*id, "window.workspace.import" | "window.workspace.export")) {
        app.ui.shell.workspace_file_action = Some((app.frame, (*id).into()));
        ui.ctx().request_repaint();
        action = None;
    }
    if let Some((id, params)) = action {
        if let Err(error) = crate::menus::invoke(app, ui.ctx(), id, params) {
            app.ui.status = error;
            app.ui.status_error = true;
        } else {
            app.ui.status_error = false;
        }
    }
}

/// Called from ROOT before drawing child windows, after the previous frame's
/// popup hide commands have returned to the native event loop.
pub(crate) fn process_pending(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.viewport_id() != egui::ViewportId::ROOT || !app.ui.shell.workspace_file_action.as_ref().is_some_and(|(frame, _)| app.frame > *frame) {
        return;
    }
    if let Some((_, id)) = app.ui.shell.workspace_file_action.take() {
        if let Err(error) = crate::menus::invoke(app, ctx, &id, json!({})) {
            app.ui.status = error;
            app.ui.status_error = true;
        } else {
            app.ui.status_error = false;
        }
    }
}

/// Custom Pixel Aspect Ratio: `{name, ratio}` adds (or updates) and selects; `{delete: name}`
/// removes; `{select: name}` selects a saved one.
fn custom_par(app: &mut PhotocraftApp, p: &Value) -> Result<Value, String> {
    let s = &mut app.ui.shell;
    if let Some(n) = p.get("delete").and_then(Value::as_str) {
        let before = s.custom_pars.len();
        s.custom_pars.retain(|c| c.name != n);
        if s.custom_pars.len() == before {
            return Err(format!("no custom pixel aspect ratio \"{n}\""));
        }
        if app.ui.view.pixel_aspect.ends_with(&format!(":{n}")) {
            app.ui.view.pixel_aspect = "square".into();
        }
        return Ok(json!({"deleted": n}));
    }
    let c = if let Some(n) = p.get("select").and_then(Value::as_str) {
        s.custom_pars.iter().find(|c| c.name == n).cloned().ok_or_else(|| format!("no custom pixel aspect ratio \"{n}\""))?
    } else {
        let name = p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or("pass `name`")?.to_string();
        let ratio = p.get("ratio").and_then(Value::as_f64).ok_or("pass `ratio`")? as f32;
        if !(0.1..=10.0).contains(&ratio) {
            return Err("the ratio must be between 0.100 and 10.000".into());
        }
        let c = CustomPar { name, ratio };
        match s.custom_pars.iter_mut().find(|x| x.name == c.name) {
            Some(x) => *x = c.clone(),
            None => s.custom_pars.push(c.clone()),
        }
        c
    };
    app.ui.view.pixel_aspect = format!("custom:{}:{}", c.ratio, c.name);
    Ok(json!({"pixelAspectRatio": c.ratio, "name": c.name}))
}

/// The open dialog's OK as (command id, params).
pub fn dialog_command(kind: &str, f: &Map<String, Value>) -> Option<(&'static str, Value)> {
    Some(match kind {
        "newWorkspace" => ("window.workspace.newWorkspace", Value::Object(f.clone())),
        "renameWorkspace" => ("window.workspace.rename", Value::Object(f.clone())),
        "deleteWorkspace" => ("window.workspace.deleteWorkspace", json!({"name": f.get("name")})),
        "customPar" => ("view.pixelAspectRatio.custom", json!({"name": f.get("name"), "ratio": f.get("ratio")})),
        "preview32" => ("view.thirtyTwoBitPreviewOptions", Value::Object(f.clone())),
        "extrasOptions" => ("view.show.showExtrasOptions", Value::Object(f.clone())),
        _ => return None,
    })
}

/// Draws the Modifier Keys panel and the open dialog.
pub fn windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.ui.shell.modifier_keys {
        let mut close = false;
        let mut s = app.ui.shell.clone();
        let cmd = crate::shortcuts::pretty("Cmd");
        let shift = crate::shortcuts::pretty("Shift");
        let alt = crate::shortcuts::pretty("Alt");
        let native_close = crate::analysis_ui::panel_window(app, ctx, "modifier-keys", tl!("Modifier Keys"), vec2(-420.0, 80.0), 150.0, |ui| {
            close = crate::analysis_ui::title_row(ui, tl!("Modifier Keys"));
            ui.horizontal(|ui| {
                for (label, on) in [(&*shift, &mut s.sticky_shift), (&*cmd, &mut s.sticky_command), (&*alt, &mut s.sticky_alt)] {
                    if ui.add(egui::Button::new(RichText::new(label).size(14.0)).selected(*on).min_size(vec2(40.0, 30.0))).clicked() {
                        *on = !*on;
                    }
                }
            });
        });
        s.modifier_keys = !(close || native_close);
        app.ui.shell = s;
    }
    if app.ui.shell.dialog.is_some() {
        dialog(app, ctx);
    }
}

fn dialog(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some((kind, mut f)) = app.ui.shell.dialog.clone() else { return };
    let t = Tokens::get(ctx);
    let title = match kind.as_str() {
        "newWorkspace" => "New Workspace",
        "renameWorkspace" => "Rename Workspace",
        "deleteWorkspace" => "Delete Workspace",
        "customPar" => "Save Pixel Aspect Ratio",
        "preview32" => "32-bit Preview Options",
        _ => "Show Extras Options",
    };
    let names: Vec<String> = app.session.prefs().workspaces.keys().cloned().collect();
    let mut result: Option<bool> = None;
    let contents = |ui: &mut egui::Ui| {
        ui.label(RichText::new(tl!(&title)).color(t.text).size(13.0).strong());
        crate::widgets::hairline(ui);
        ui.add_space(6.0);
        let text = |ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, label: &str| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!(&label)).color(t.text_dim).size(11.0));
                let mut v = f.get(key).and_then(Value::as_str).unwrap_or("").to_string();
                if ui.add(egui::TextEdit::singleline(&mut v).desired_width(180.0)).changed() {
                    f.insert(key.into(), json!(v));
                }
            });
        };
        let check = |ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, label: &str| {
            let mut on = f.get(key).and_then(Value::as_bool).unwrap_or(false);
            if crate::widgets::checkbox(ui, &mut on, label).changed() {
                f.insert(key.into(), json!(on));
            }
        };
        let number = |ui: &mut egui::Ui, f: &mut Map<String, Value>, key: &str, label: &str, range: std::ops::RangeInclusive<f32>| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(tl!(&label)).color(t.text_dim).size(11.0));
                let mut v = f.get(key).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                if crate::widgets::slider(ui, &mut v, range.clone(), None).changed() | crate::widgets::value_field(ui, &mut v, range, "", 60.0).changed() {
                    f.insert(key.into(), json!(v));
                }
            });
        };
        match kind.as_str() {
            "newWorkspace" => {
                text(ui, &mut f, "name", tl!("Name:"));
                ui.label(RichText::new(tl!("Capture")).color(t.text_dim).size(11.0));
                ui.label(
                    RichText::new(tl!("Panel locations are saved in the workspace. Keyboard shortcuts, menus and toolbar are optional."))
                        .color(t.text_faint)
                        .size(10.5),
                );
                check(ui, &mut f, "keyboardShortcuts", tl!("Keyboard Shortcuts"));
                check(ui, &mut f, "menus", tl!("Menus"));
                check(ui, &mut f, "toolbar", tl!("Toolbar"));
            }
            "renameWorkspace" => text(ui, &mut f, "name", tl!("Name:")),
            "deleteWorkspace" => {
                let mut cur = f.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let opts: Vec<(String, &str)> = names.iter().map(|n| (n.clone(), n.as_str())).collect();
                if !opts.is_empty() && crate::widgets::dropdown(ui, "ws-delete", &mut cur, &opts, 180.0) {
                    f.insert("name".into(), json!(cur));
                }
            }
            "customPar" => {
                text(ui, &mut f, "name", tl!("Name:"));
                number(ui, &mut f, "ratio", tl!("Factor:"), 0.1..=10.0);
            }
            "preview32" => {
                let mut m = f.get("method").and_then(Value::as_str).unwrap_or("exposureGamma").to_string();
                if crate::widgets::dropdown(
                    ui,
                    "p32-method",
                    &mut m,
                    &[("exposureGamma".to_string(), tl!("Exposure and Gamma")), ("highlightCompression".to_string(), tl!("Highlight Compression"))],
                    180.0,
                ) {
                    f.insert("method".into(), json!(m));
                }
                ui.add_enabled_ui(m == "exposureGamma", |ui| {
                    number(ui, &mut f, "exposure", tl!("Exposure:"), -20.0..=20.0);
                    number(ui, &mut f, "gamma", tl!("Gamma:"), 0.1..=9.99);
                });
            }
            _ => {
                for (k, label) in [
                    ("layerEdges", tl!("Layer Edges")),
                    ("selectionEdges", tl!("Selection Edges")),
                    ("targetPath", tl!("Target Path")),
                    ("notes", tl!("Notes")),
                    ("pixelGrid", tl!("Pixel Grid")),
                    ("slices", tl!("Slices")),
                    ("count", tl!("Count")),
                    ("smartGuides", tl!("Smart Guides")),
                    ("brushPreview", tl!("Brush Preview")),
                    ("mesh", tl!("Mesh")),
                    ("editPins", tl!("Edit Pins")),
                    ("canvasGuides", tl!("Canvas Guides")),
                    ("artboardGuides", tl!("Artboard Guides")),
                ] {
                    check(ui, &mut f, k, label);
                }
            }
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = if kind == "deleteWorkspace" { tl!("Delete") } else { tl!("OK") };
                if let Some(role) = crate::widgets::dialog_buttons(
                    ui,
                    &[
                        crate::widgets::DialogButton::new(crate::widgets::ButtonRole::Default, ok, 70.0),
                        crate::widgets::DialogButton::new(crate::widgets::ButtonRole::Cancel, tl!("Cancel"), 70.0),
                    ],
                ) {
                    result = Some(role == crate::widgets::ButtonRole::Default);
                }
            });
        });
    };
    if ctx.embed_viewports() {
        egui::Window::new(tl!(title))
            .id(egui::Id::new("shell-dialog"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ctx, contents);
    } else if crate::dialogs::show_shell(app, ctx, &kind, tl!(title), contents) {
        result = Some(false);
    }
    let closed_kind = kind.clone();
    match result {
        Some(true) => {
            if let Some((id, p)) = dialog_command(&kind, &f) {
                match crate::menus::invoke(app, ctx, id, p) {
                    Ok(_) => app.ui.shell.dialog = None,
                    Err(e) => {
                        app.ui.status = e;
                        app.ui.status_error = true;
                        app.ui.shell.dialog = Some((kind, f));
                    }
                }
            }
        }
        Some(false) => app.ui.shell.dialog = None,
        None => app.ui.shell.dialog = Some((kind, f)),
    }
    crate::dialogs::finish_shell(app, ctx, &closed_kind);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Tool;

    fn app() -> (PhotocraftApp, egui::Context) {
        (PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default()), egui::Context::default())
    }

    #[test]
    fn new_select_delete_workspaces() {
        let (mut app, ctx) = app();
        let inv = |app: &mut PhotocraftApp, id: &str, p: Value| crate::menus::invoke(app, &ctx, id, p);
        assert!(!crate::menus::is_enabled(&app, "window.workspace.deleteWorkspace"));
        assert_eq!(inv(&mut app, "window.workspace.newWorkspace", json!({})).unwrap()["dialog"], "newWorkspace");
        app.ui.panels.history = true;
        app.ui.panels.navigator = true;
        app.session.prefs.edit(|p| {
            p.shortcuts.insert("filter.blur.gaussian".into(), "Cmd+Alt+G".into());
        });
        inv(&mut app, "window.workspace.newWorkspace", json!({"name": "Retouch", "keyboardShortcuts": true})).unwrap();
        assert_eq!(app.ui.workspace, "Retouch");
        assert!(inv(&mut app, "window.workspace.newWorkspace", json!({"name": "Painting"})).is_err());
        assert!(inv(&mut app, "window.workspace.newWorkspace", json!({"name": " "})).is_err());
        // The current workspace can't be deleted.
        assert!(inv(&mut app, "window.workspace.deleteWorkspace", json!({"name": "Retouch"})).is_err());
        inv(&mut app, "window.workspace.painting", json!({})).unwrap();
        assert!(!app.ui.panels.history);
        app.session.prefs.edit(|p| p.shortcuts.clear());
        // Switching back restores the panels and the captured shortcuts.
        inv(&mut app, "window.workspace.select", json!({"name": "Retouch"})).unwrap();
        assert!(app.ui.panels.history && app.ui.panels.navigator);
        assert_eq!(app.session.prefs().shortcuts.get("filter.blur.gaussian").map(String::as_str), Some("Cmd+Alt+G"));
        // Reset Workspace re-applies the saved layout.
        app.ui.panels.history = false;
        inv(&mut app, "window.workspace.resetWorkspace", json!({})).unwrap();
        assert!(app.ui.panels.history);
        inv(&mut app, "window.workspace.essentials", json!({})).unwrap();
        assert!(crate::menus::is_enabled(&app, "window.workspace.deleteWorkspace"));
        inv(&mut app, "window.workspace.deleteWorkspace", json!({"name": "Retouch"})).unwrap();
        assert!(app.session.prefs().workspaces.is_empty());
        // Saved workspaces persist with the preferences.
        inv(&mut app, "window.workspace.newWorkspace", json!({"name": "Mine"})).unwrap();
        let json = app.session.prefs_to_json();
        let mut s2 = photocraft_engine::Session::new();
        s2.load_prefs_json(&json).unwrap();
        assert!(s2.prefs().workspaces.contains_key("Mine"));
    }

    #[test]
    fn workspace_save_rename_roundtrip_keeps_complete_panel_layout() {
        let (mut app, ctx) = app();
        app.ui.dock.width = Some(420.0);
        app.ui.dock.containers = vec![crate::panel_workspace::Container {
            id: 91,
            floating: true,
            position: [500.0, 120.0],
            size: [360.0, 550.0],
            tabs: vec![crate::panel_workspace::Tab { group: crate::dock::Group::Color, index: 0 }],
            ..Default::default()
        }];
        new_workspace(&mut app, &json!({"name":"My Layout"})).unwrap();
        let old_epoch = app.ui.shell.layout_epoch;
        app.ui.dock.width = Some(300.0);
        app.ui.dock.containers.clear();
        select_workspace(&mut app, &json!({"name":"My Layout"})).unwrap();
        assert_eq!(app.ui.dock.width, Some(420.0));
        assert_eq!(app.ui.dock.containers[0].size, [360.0, 550.0]);
        assert!(app.ui.shell.layout_epoch > old_epoch);
        app.ui.dock.width = Some(390.0);
        crate::menus::invoke(&mut app, &ctx, "window.workspace.save", json!({})).unwrap();
        crate::menus::invoke(&mut app, &ctx, "window.workspace.rename", json!({"name":"Retouch Layout"})).unwrap();
        assert!(!app.session.prefs().workspaces.contains_key("My Layout"));
        let data = export_workspace(&mut app, &json!({"dataOnly":true})).unwrap();
        let result = import_workspace(&mut app, &json!({"data":data})).unwrap();
        assert_eq!(result["workspace"], "Retouch Layout (2)");
        assert_eq!(app.ui.dock.width, Some(390.0));
        assert_eq!(app.ui.dock.containers[0].position, [500.0, 120.0]);
        let prefs = app.session.prefs_to_json();
        let services = crate::Services { load_prefs: Some(Box::new(move || Some(prefs.clone()))), ..Default::default() };
        let restored = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        assert!(restored.session.prefs().workspaces.contains_key("Retouch Layout (2)"));
        assert_eq!(restored.ui.workspace, "Retouch Layout (2)");
        assert_eq!(restored.ui.dock.width, Some(390.0));
    }

    #[test]
    fn repeated_workspace_apply_restores_floating_outer_size_without_growth() {
        let (mut app, ctx) = app();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Studio);
        app.ui.panels.color = true;
        app.ui.dock.containers = vec![crate::panel_workspace::Container {
            id: 91,
            floating: true,
            position: [120.0, 80.0],
            size: [360.0, 500.0],
            tabs: vec![crate::panel_workspace::Tab { group: crate::dock::Group::Color, index: 1 }],
            ..Default::default()
        }];
        let render = |app: &mut PhotocraftApp| {
            for _ in 0..3 {
                let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 900.0))), ..Default::default() };
                let mut output = ctx.run_ui(raw, |ui| {
                    egui::CentralPanel::default().show(ui, |_| {});
                    crate::panel_workspace::floating(app, ui.ctx());
                });
                output.textures_delta.clear();
            }
        };
        render(&mut app);
        let expected = app.ui.dock.containers[0].size;
        new_workspace(&mut app, &json!({"name":"Geometry"})).unwrap();
        for _ in 0..4 {
            app.ui.dock.containers[0].size = [600.0, 650.0];
            app.ui.dock.containers[0].position = [450.0, 200.0];
            select_workspace(&mut app, &json!({"name":"Geometry"})).unwrap();
            render(&mut app);
            let actual = app.ui.dock.containers[0].size;
            assert!((actual[0] - expected[0]).abs() <= 1.0 && (actual[1] - expected[1]).abs() <= 1.0, "{actual:?} != {expected:?}");
            assert_eq!(app.ui.dock.containers[0].position, [120.0, 80.0]);
            save_workspace(&mut app).unwrap();
        }
    }

    #[test]
    fn exporting_legacy_workspace_does_not_migrate_the_live_ui() {
        let (mut app, ctx) = app();
        let before = app.ui.dock.clone();
        let result = menu(&mut app, &ctx, "window.workspace.export", &json!({"dataOnly":true})).unwrap().unwrap();
        assert_eq!(app.ui.dock, before);
        assert_eq!(result["layout"]["dock"]["containers"].as_array().unwrap().len(), 8);
    }

    #[test]
    fn saving_legacy_workspace_also_restores_active_floating_geometry_after_restart() {
        let (mut app, ctx) = app();
        app.ui.dock.floating.push(crate::dock::Group::Color);
        assert!(app.ui.dock.containers.is_empty());
        menu(&mut app, &ctx, "window.workspace.newWorkspace", &json!({"name":"Legacy saved"})).unwrap().unwrap();
        assert!(!app.ui.dock.containers.is_empty());
        let expected = crate::dock::snapshot(&app);
        crate::dock::remember(&mut app);
        let prefs = app.session.prefs_to_json();
        let services = crate::Services { load_prefs: Some(Box::new(move || Some(prefs.clone()))), ..Default::default() };
        let restored = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        assert_eq!(crate::dock::snapshot(&restored), expected);
        assert!(restored.ui.dock.containers.iter().any(|c| c.floating && c.tabs.iter().any(|t| t.group == crate::dock::Group::Color)));
    }

    #[test]
    fn startup_theme_does_not_remap_already_restored_color_tabs() {
        use crate::theme::ThemeKind;
        use photocraft_engine::prefs::Theme;
        for (kind, pref, color_index) in [
            (ThemeKind::Studio, Theme::Studio, 1),
            (ThemeKind::StudioLight, Theme::StudioLight, 1),
            (ThemeKind::Classic, Theme::Classic, 1),
            (ThemeKind::Pro, Theme::Pro, 0),
            (ThemeKind::ProMedium, Theme::ProMedium, 0),
        ] {
            let (mut original, ctx) = app();
            original.set_theme(&ctx, kind);
            original.session.prefs.edit(|p| p.interface.theme = pref);
            original.ui.workspace = "Saved Color".into();
            original.ui.dock_tabs.color = color_index;
            original.ui.dock.containers = vec![crate::panel_workspace::Container {
                id: 91,
                selected: color_index,
                floating: true,
                tabs: (0..4).map(|index| crate::panel_workspace::Tab { group: crate::dock::Group::Color, index }).collect(),
                ..Default::default()
            }];
            crate::dock::remember(&mut original);
            let expected = crate::dock::snapshot(&original);
            let prefs = original.session.prefs_to_json();
            let services = crate::Services { load_prefs: Some(Box::new(move || Some(prefs.clone()))), ..Default::default() };
            let mut restored = PhotocraftApp::new(photocraft_engine::Session::new(), services);
            assert_eq!(restored.ui.theme, kind);
            let restart_ctx = egui::Context::default();
            // Same initialization ordering as the native first frame: setup then preference tick.
            PhotocraftApp::setup_context(&restart_ctx, restored.ui.theme);
            crate::prefs_ui::tick(&mut restored, &restart_ctx);
            assert_eq!(crate::dock::snapshot(&restored), expected, "{kind:?}");
            assert_eq!(restored.ui.dock_tabs.color, color_index);
            let c = &restored.ui.dock.containers[0];
            assert_eq!(c.tabs[c.selected].index, color_index);
            assert_eq!(c.tabs.iter().map(|t| t.index).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
        }
    }

    #[test]
    fn workspace_color_tab_meaning_survives_cross_theme_select_and_import() {
        use crate::theme::ThemeKind;
        for (source, target, source_color, target_color) in [(ThemeKind::StudioLight, ThemeKind::Pro, 1, 0), (ThemeKind::ProMedium, ThemeKind::Studio, 0, 1)] {
            let (mut app, ctx) = app();
            app.set_theme(&ctx, source);
            app.ui.dock_tabs.color = source_color;
            app.ui.dock.containers = vec![crate::panel_workspace::Container {
                id: 91,
                selected: source_color,
                floating: true,
                tabs: (0..4).map(|index| crate::panel_workspace::Tab { group: crate::dock::Group::Color, index }).collect(),
                ..Default::default()
            }];
            new_workspace(&mut app, &json!({"name":"Cross theme"})).unwrap();
            let exported = export_workspace(&mut app, &json!({"dataOnly":true})).unwrap();
            app.set_theme(&ctx, target);
            select_workspace(&mut app, &json!({"name":"Cross theme"})).unwrap();
            assert_eq!(app.ui.theme, target);
            assert_eq!(app.ui.dock_tabs.color, target_color);
            let c = &app.ui.dock.containers[0];
            assert_eq!(c.tabs[c.selected].index, target_color);
            import_workspace(&mut app, &json!({"data":exported})).unwrap();
            assert_eq!(app.ui.theme, target);
            assert_eq!(app.ui.dock_tabs.color, target_color);
            let c = &app.ui.dock.containers[0];
            assert_eq!(c.tabs[c.selected].index, target_color);
            let mut legacy = export_workspace(&mut app, &json!({"dataOnly":true})).unwrap();
            legacy["layout"].as_object_mut().unwrap().remove("colorTabOrder");
            import_workspace(&mut app, &json!({"data":legacy})).unwrap();
            assert_eq!(app.ui.dock_tabs.color, target_color);
        }
    }

    #[test]
    fn workspace_file_roundtrip_preserves_tool_preset_view_and_sort() {
        use crate::preset_panels::ToolPresetView;
        for view in [ToolPresetView::Text, ToolPresetView::Small, ToolPresetView::Large] {
            for sort in [false, true] {
                let (mut source, _) = app();
                source.ui.presets_ui.tool_list_view = view;
                source.ui.presets_ui.tool_sort_by_tool = sort;
                new_workspace(&mut source, &json!({"name":"Preset layout"})).unwrap();
                let file = export_workspace(&mut source, &json!({"dataOnly":true})).unwrap();
                assert_eq!(file["layout"]["toolPresetView"], serde_json::to_value(view).unwrap());
                assert_eq!(file["layout"]["toolPresetSortByTool"], sort);
                let (mut target, _) = app();
                import_workspace(&mut target, &json!({"data":file})).unwrap();
                assert_eq!(target.ui.presets_ui.tool_list_view, view);
                assert_eq!(target.ui.presets_ui.tool_sort_by_tool, sort);
            }
        }
    }

    #[test]
    fn legacy_workspace_without_tool_preset_options_imports_with_default_view() {
        use crate::preset_panels::ToolPresetView;
        let (mut source, _) = app();
        new_workspace(&mut source, &json!({"name":"Legacy layout"})).unwrap();
        let mut file = export_workspace(&mut source, &json!({"dataOnly":true})).unwrap();
        let layout = file["layout"].as_object_mut().unwrap();
        layout.remove("toolPresetView");
        layout.remove("toolPresetSortByTool");
        let (mut target, _) = app();
        import_workspace(&mut target, &json!({"data":file})).unwrap();
        assert_eq!(target.ui.presets_ui.tool_list_view, ToolPresetView::Small);
        assert!(!target.ui.presets_ui.tool_sort_by_tool);
    }

    #[test]
    fn workspace_import_rejects_bad_input_without_mutating_layout_or_library() {
        let (mut app, _) = app();
        new_workspace(&mut app, &json!({"name":"Original"})).unwrap();
        let valid = export_workspace(&mut app, &json!({"dataOnly":true})).unwrap();
        let before = crate::dock::snapshot(&app);
        let library = app.session.prefs().workspaces.clone();
        for (path, value) in [
            ("/version", json!(2)),
            ("/format", json!("Other app")),
            ("/name", json!(" ")),
            ("/layout/dock/width", json!(999999)),
            ("/layout/dock/order", json!(["notAPanel"])),
            ("/layout/panels", json!(false)),
            ("/layout/dockTabs/color", json!(999)),
            ("/layout/toolPresetView", json!("unknown")),
            ("/layout/toolPresetView", json!(false)),
            ("/layout/toolPresetSortByTool", json!("yes")),
        ] {
            let mut data = valid.clone();
            *data.pointer_mut(path).unwrap() = value;
            assert!(import_workspace(&mut app, &json!({"data":data})).is_err(), "{path}");
            assert_eq!(crate::dock::snapshot(&app), before);
            assert_eq!(app.session.prefs().workspaces, library);
        }
        let mut data = valid;
        data["unknown"] = json!(true);
        assert!(import_workspace(&mut app, &json!({"data":data})).is_err());
        assert!(new_workspace(&mut app, &json!({"name":"Original"})).is_err());
        assert!(new_workspace(&mut app, &json!({"name":"x".repeat(81)})).is_err());
        assert!(new_workspace(&mut app, &json!({"name":"bad\nname"})).is_err());
        assert_eq!(app.session.prefs().workspaces, library);
    }

    #[test]
    fn workspace_native_file_services_export_and_import_json() {
        use std::sync::{Arc, Mutex};
        let written = Arc::new(Mutex::new(Vec::new()));
        let (mut app, _) = app();
        new_workspace(&mut app, &json!({"name":"Files"})).unwrap();
        app.services.pick_save = Some(Box::new(|suggested| {
            assert!(suggested.ends_with(".json"));
            Some("layout.json".into())
        }));
        let bytes = written.clone();
        app.services.write = Some(Box::new(move |path, data| {
            assert_eq!(path, "layout.json");
            *bytes.lock().unwrap() = data.to_vec();
            Ok(())
        }));
        assert_eq!(export_workspace(&mut app, &json!({})).unwrap()["exported"], "layout.json");
        let bytes = written.clone();
        app.services.pick_open_workspace = Some(Box::new(move || Some(("layout.json".into(), Ok(bytes.lock().unwrap().clone())))));
        assert_eq!(import_workspace(&mut app, &json!({})).unwrap()["workspace"], "Files (2)");
        app.services.pick_open_workspace = Some(Box::new(|| None));
        assert_eq!(import_workspace(&mut app, &json!({})).unwrap()["cancelled"], true);
    }

    #[test]
    fn workspace_export_save_as_uses_selected_name_and_path_without_changing_layout() {
        use std::{cell::RefCell, rc::Rc};
        let (mut app, ctx) = app();
        new_workspace(&mut app, &json!({"name":"绘画 / 参考:01"})).unwrap();
        let before = crate::dock::snapshot(&app);
        let prefs = app.session.prefs_to_json();
        let written = Rc::new(RefCell::new(None));
        app.services.pick_save = Some(Box::new(|_| panic!("dedicated workspace Save As must take precedence")));
        app.services.pick_save_workspace = Some(Box::new(|suggested| {
            assert_eq!(suggested, "InkPop-绘画 _ 参考_01.json");
            Some("D:/chosen folder/用户指定名称.json".into())
        }));
        let output = written.clone();
        app.services.write = Some(Box::new(move |path, bytes| {
            *output.borrow_mut() = Some((path.to_string(), serde_json::from_slice::<Value>(bytes).unwrap()));
            Ok(())
        }));
        let result = crate::menus::invoke(&mut app, &ctx, "window.workspace.export", json!({})).unwrap();
        assert_eq!(result["exported"], "D:/chosen folder/用户指定名称.json");
        let output = written.borrow();
        let (path, file) = output.as_ref().unwrap();
        assert_eq!(path, "D:/chosen folder/用户指定名称.json");
        assert_eq!(file["name"], "绘画 / 参考:01");
        assert_eq!(crate::dock::snapshot(&app), before);
        assert_eq!(app.session.prefs_to_json(), prefs);
    }

    #[test]
    fn cancelling_workspace_save_as_never_writes_or_migrates_live_legacy_layout() {
        let (mut app, ctx) = app();
        let before = crate::dock::snapshot(&app);
        let dock = app.ui.dock.clone();
        let prefs = app.session.prefs_to_json();
        app.services.pick_save_workspace = Some(Box::new(|_| None));
        app.services.pick_save = Some(Box::new(|_| panic!("cancel must not retry with another picker")));
        app.services.write = Some(Box::new(|_, _| panic!("cancel must not write a file")));
        let result = crate::menus::invoke(&mut app, &ctx, "window.workspace.export", json!({})).unwrap();
        assert_eq!(result["cancelled"], true);
        assert_eq!(app.ui.dock, dock);
        assert_eq!(crate::dock::snapshot(&app), before);
        assert_eq!(app.session.prefs_to_json(), prefs);
    }

    #[test]
    fn workspace_export_write_error_keeps_layout_and_saved_library() {
        let (mut app, ctx) = app();
        new_workspace(&mut app, &json!({"name":"Keep me"})).unwrap();
        let before = crate::dock::snapshot(&app);
        let prefs = app.session.prefs_to_json();
        app.services.pick_save_workspace = Some(Box::new(|_| Some("layout.json".into())));
        app.services.write = Some(Box::new(|_, _| Err("disk full".into())));
        assert_eq!(crate::menus::invoke(&mut app, &ctx, "window.workspace.export", json!({})).unwrap_err(), "disk full");
        assert_eq!(crate::dock::snapshot(&app), before);
        assert_eq!(app.session.prefs_to_json(), prefs);
    }

    #[test]
    fn workspace_selector_waits_for_popup_close_frame_before_file_dialog() {
        use std::{cell::Cell, rc::Rc};
        let (mut app, ctx) = app();
        PhotocraftApp::setup_context(&ctx, app.ui.theme);
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        app.services.pick_save_workspace = Some(Box::new(move |_| {
            observed.set(observed.get() + 1);
            None
        }));
        app.ui.shell.workspace_file_action = Some((app.frame, "window.workspace.export".into()));
        process_pending(&mut app, &ctx);
        assert_eq!(calls.get(), 0);
        app.frame += 1;
        process_pending(&mut app, &ctx);
        assert_eq!(calls.get(), 1);
        app.frame += 1;
        process_pending(&mut app, &ctx);
        assert_eq!(calls.get(), 1, "one queued action opens only one dialog");
    }

    #[test]
    fn workspace_automation_paths_use_authorized_services_only() {
        let (mut app, _) = app();
        assert!(export_workspace(&mut app, &json!({"path":"outside.json"})).is_err());
        assert!(import_workspace(&mut app, &json!({"path":"outside.json"})).is_err());
        app.services.automation_write = Some(Box::new(|_, _| Err("outside authorized root".into())));
        assert_eq!(export_workspace(&mut app, &json!({"path":"outside.json"})).unwrap_err(), "outside authorized root");
        app.services.automation_read = Some(Box::new(|_| Err("outside authorized root".into())));
        assert_eq!(import_workspace(&mut app, &json!({"path":"outside.json"})).unwrap_err(), "outside authorized root");
    }

    #[test]
    fn lock_workspace_toggles_and_persists() {
        let (mut app, ctx) = app();
        crate::menus::invoke(&mut app, &ctx, "window.workspace.lockWorkspace", json!({})).unwrap();
        assert!(app.session.prefs().workspace_locked);
        assert_eq!(checked(&app, "window.workspace.lockWorkspace"), Some(true));
        let items = crate::menus::menu_items(&app);
        assert!(items.iter().any(|i| i.id == "window.workspace.lockWorkspace" && i.checked == Some(true)));
        crate::menus::invoke(&mut app, &ctx, "window.workspace.lockWorkspace", json!({"on": false})).unwrap();
        assert!(!app.session.prefs().workspace_locked);
    }

    #[test]
    fn modifier_keys_latch_into_tool_events() {
        let (mut app, ctx) = app();
        crate::menus::invoke(&mut app, &ctx, "window.panel.modifierKeys", json!({"alt": true})).unwrap();
        assert!(app.ui.shell.modifier_keys);
        let m = sticky_mods(&app, egui::Modifiers::NONE);
        assert!(m.alt && !m.shift);
        // ⌥ latched: a Count-tool click on a marker removes it.
        app.run("file.new", json!({"width": 50, "height": 50})).unwrap();
        app.run("count.add", json!({"x": 10, "y": 10})).unwrap();
        app.ui.tool = Tool::Count;
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, egui::Modifiers::NONE);
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Up { x: 10.0, y: 10.0 }, egui::Modifiers::NONE);
        assert_eq!(app.session.active().unwrap().doc.measurement.count_total(), 0);
        crate::analysis_ui::tests::render(&mut app, &ctx, windows);
    }

    #[test]
    fn custom_pixel_aspect_ratio_and_extras_options() {
        let (mut app, ctx) = app();
        let inv = |app: &mut PhotocraftApp, id: &str, p: Value| crate::menus::invoke(app, &ctx, id, p);
        assert_eq!(inv(&mut app, "view.pixelAspectRatio.custom", json!({})).unwrap()["dialog"], "customPar");
        inv(&mut app, "view.pixelAspectRatio.custom", json!({"name": "Cinema", "ratio": 1.25})).unwrap();
        assert_eq!(crate::view_cmds::pixel_aspect_ratio(&app.ui.view.pixel_aspect), 1.25);
        assert_eq!(checked(&app, "view.pixelAspectRatio.custom"), Some(true));
        assert!(inv(&mut app, "view.pixelAspectRatio.custom", json!({"name": "x", "ratio": 50})).is_err());
        inv(&mut app, "view.pixelAspectRatio.square", json!({})).unwrap();
        inv(&mut app, "view.pixelAspectRatio.custom", json!({"select": "Cinema"})).unwrap();
        assert_eq!(crate::view_cmds::pixel_aspect_ratio(&app.ui.view.pixel_aspect), 1.25);
        inv(&mut app, "view.pixelAspectRatio.custom", json!({"delete": "Cinema"})).unwrap();
        assert_eq!(app.ui.view.pixel_aspect, "square");
        // Show Extras Options.
        assert_eq!(inv(&mut app, "view.show.showExtrasOptions", json!({})).unwrap()["dialog"], "extrasOptions");
        let (kind, mut f) = app.ui.shell.dialog.clone().unwrap();
        f.insert("count".into(), json!(false));
        let (id, p) = dialog_command(&kind, &f).unwrap();
        inv(&mut app, id, p).unwrap();
        assert!(!app.ui.view.show.count);
        assert!(inv(&mut app, "view.show.showExtrasOptions", json!({"bogus": true})).is_err());
        crate::analysis_ui::tests::render(&mut app, &ctx, windows);
    }

    #[test]
    fn thirty_two_bit_preview_dialog() {
        let (mut app, ctx) = app();
        app.run("file.new", json!({"width": 20, "height": 20, "depth": 32})).unwrap();
        let r = crate::menus::invoke(&mut app, &ctx, "view.thirtyTwoBitPreviewOptions", json!({})).unwrap();
        assert_eq!(r["dialog"], "preview32");
        let (kind, mut f) = app.ui.shell.dialog.clone().unwrap();
        f.insert("exposure".into(), json!(1.5));
        let (id, p) = dialog_command(&kind, &f).unwrap();
        crate::menus::invoke(&mut app, &ctx, id, p).unwrap();
        let d = app.session.active().unwrap().doc.id;
        assert_eq!(app.session.color.hdr[&d].exposure, 1.5);
    }
}
