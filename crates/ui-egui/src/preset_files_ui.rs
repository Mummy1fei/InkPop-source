//! Preset files opened like documents (File › Open, drag-and-drop, the Brushes panel's
//! Import Brushes…, the Preset Manager's Load…): Photoshop brushes (`.abr`) go to the brush
//! library through `brush.presets.importAbr`, gradients (`.grd`) to the Gradients panel through
//! `gradient.presets.importGrd`, instead of opening as documents.

use serde_json::{Value, json};

use crate::PhotocraftApp;

#[derive(Clone, Copy)]
pub(crate) enum ToolsFileAction {
    Load,
    Save,
    Replace,
}

fn pending_tools_id() -> egui::Id {
    egui::Id::new("pending-tool-preset-file-action")
}

pub(crate) fn queue_tools(ctx: &egui::Context, action: ToolsFileAction, frame: u64) {
    ctx.data_mut(|data| data.insert_temp(pending_tools_id(), (frame, action)));
    ctx.request_repaint_of(egui::ViewportId::ROOT);
}

/// File services run from ROOT on a later application frame, after the panel popup closes.
/// An egui discard pass in the same frame has not applied the native hide commands yet.
/// A child host must not own the OS picker while it can be hidden or reordered.
pub(crate) fn process_pending(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.viewport_id() != egui::ViewportId::ROOT {
        return;
    }
    let action = ctx.data_mut(|data| {
        let (frame, action) = data.get_temp::<(u64, ToolsFileAction)>(pending_tools_id())?;
        if app.frame <= frame {
            return None;
        }
        data.remove::<(u64, ToolsFileAction)>(pending_tools_id());
        Some(action)
    });
    let Some(action) = action else { return };
    let result = match action {
        ToolsFileAction::Load => load_tools(app, false),
        ToolsFileAction::Save => save_tools(app),
        ToolsFileAction::Replace => load_tools(app, true),
    };
    let label = match action {
        ToolsFileAction::Load => tl!("Load Tool Presets…"),
        ToolsFileAction::Save => tl!("Save Tool Presets…"),
        ToolsFileAction::Replace => tl!("Replace Tool Presets…"),
    };
    crate::preset_panels::tool_file_result(app, result, label);
}

/// Extensions handled here rather than by the document importer.
pub const PRESET_EXTS: &[&str] = &["abr", "grd", "tpl", "pctools"];

fn ext(name: &str) -> String {
    std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Is `name` a preset file this module imports?
pub fn is_preset_file(name: &str) -> bool {
    PRESET_EXTS.contains(&ext(name).as_str())
}

/// Command params for the file: a real path keeps the journal small; browser files arrive as bytes.
fn source(name: &str, bytes: &[u8]) -> Value {
    #[cfg(not(target_arch = "wasm32"))]
    if std::path::Path::new(name).is_absolute() && std::path::Path::new(name).is_file() {
        return json!({"path": name});
    }
    let _ = name;
    json!({"data": photocraft_engine::paint::tile::b64_encode(bytes)})
}

/// Import a preset file; `None` when `name` isn't one.
pub fn open(app: &mut PhotocraftApp, name: &str, bytes: &[u8]) -> Option<Result<(), String>> {
    let (cmd, what) = match ext(name).as_str() {
        "abr" => ("brush.presets.importAbr", "brushes"),
        "grd" => ("gradient.presets.importGrd", "gradients"),
        "tpl" => ("tool.presets.importTpl", "tool presets"),
        "pctools" => ("tool.presets.import", "tool presets"),
        _ => return None,
    };
    let stem = std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| tl!("Imported").into());
    let mut p = if ext(name) == "pctools" {
        match native_tools_params(bytes, false) {
            Ok(p) => p,
            Err(e) => return Some(Err(e)),
        }
    } else {
        source(name, bytes)
    };
    p["group"] = json!(stem);
    if what == "brushes" {
        p["select"] = json!(true);
    }
    if what == "tool presets" {
        app.ui.panels.tool_presets = true;
        app.ui.presets_ui.tool_presets = true;
        crate::dock::reveal(app, crate::dock::Group::ToolPresets);
    }
    Some(app.run(cmd, p).map(|r| {
        // A background import (#210): `jobs_ui` reports the count when it lands.
        if r.get("pending").and_then(Value::as_bool) == Some(true) {
            app.ui.status_error = false;
            app.ui.status = format!("Importing {what} from {stem}…");
            return;
        }
        let n = r.get("count").and_then(Value::as_u64).unwrap_or(0);
        let warnings: Vec<String> = r.get("warnings").and_then(|w| serde_json::from_value(w.clone()).ok()).unwrap_or_default();
        app.ui.status_error = false;
        app.ui.status = match warnings.first() {
            Some(w) => format!("Imported {n} {what} from {stem} ({} notes: {w})", warnings.len()),
            None => format!("Imported {n} {what} from {stem}"),
        };
        if what == "brushes" {
            // Show them: Brush Settings › Brushes tab.
            app.ui.panels.brush_settings = true;
            app.ui.brush_tab = 1;
        }
    }))
}

fn native_tools_params(bytes: &[u8], replace: bool) -> Result<Value, String> {
    if bytes.len() > 128 << 20 {
        return Err("Tool preset library exceeds 128 MiB".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|e| format!("Invalid .pctools text: {e}"))?;
    Ok(json!({"text":text,"replace":replace}))
}

/// Load/Replace chooses a preset file directly, without opening unrelated document files.
pub(crate) fn load_tools(app: &mut PhotocraftApp, replace: bool) -> Result<Value, String> {
    let Some(picked) = app.pick_file_bytes() else { return Ok(json!({"cancelled":true})) };
    let (name, bytes) = picked?;
    match ext(&name).as_str() {
        "pctools" => app.run("tool.presets.import", native_tools_params(&bytes, replace)?),
        "tpl" => {
            let mut p = source(&name, &bytes);
            p["replaceAll"] = json!(replace);
            // Load always appends; Replace deliberately replaces the complete library.
            p["replace"] = json!(false);
            app.run("tool.presets.importTpl", p)
        }
        _ => Err(tl!("Choose a .pctools or Photoshop .tpl tool preset file.").into()),
    }
}

/// Save in the native format so sampled tips, colours and every tool option round-trip.
pub(crate) fn save_tools(app: &mut PhotocraftApp) -> Result<Value, String> {
    let Some(mut path) = app.services.pick_save.as_mut().and_then(|f| f("ToolPresets.pctools")) else { return Ok(json!({"cancelled":true})) };
    if ext(&path).is_empty() {
        path.push_str(".pctools");
    }
    if ext(&path) != "pctools" {
        return Err(tl!("Save tool presets with the .pctools extension.").into());
    }
    let out = app.run("tool.presets.export", json!({}))?;
    let text = out.get("text").and_then(Value::as_str).ok_or("No exported tool presets")?;
    let write = app.services.write.as_mut().ok_or("No writer configured")?;
    write(&path, text.as_bytes())?;
    Ok(json!({"path":path,"count":out["count"]}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_preset_files() {
        assert!(is_preset_file("/a/b/Set.ABR"));
        assert!(is_preset_file("sunsets.grd"));
        assert!(is_preset_file("M工具预设.TPL"));
        assert!(is_preset_file("M工具预设.PCTOOLS"));
        assert!(!is_preset_file("photo.psd"));
        assert!(!is_preset_file("abr"));
    }

    #[test]
    fn native_tools_save_load_and_cancel_preserve_library() {
        use std::sync::{Arc, Mutex};
        let saved = Arc::new(Mutex::new(Vec::new()));
        let dest = saved.clone();
        let mut app = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services {
                pick_save: Some(Box::new(|name| Some(name.into()))),
                write: Some(Box::new(move |path, bytes| {
                    assert!(path.ends_with(".pctools"));
                    *dest.lock().unwrap() = bytes.to_vec();
                    Ok(())
                })),
                ..Default::default()
            },
        );
        let before = app.session.presets.tool_presets.clone();
        save_tools(&mut app).unwrap();
        let bytes = saved.lock().unwrap().clone();
        app.session.presets.tool_presets.clear();
        app.services.pick_open = Some(Box::new(move || Some(("Library.pctools".into(), Ok(bytes.clone())))));
        load_tools(&mut app, true).unwrap();
        assert_eq!(app.session.presets.tool_presets, before);
        app.services.pick_open = Some(Box::new(|| None));
        assert_eq!(load_tools(&mut app, true).unwrap()["cancelled"], true);
        assert_eq!(app.session.presets.tool_presets, before);
        app.services.pick_open = Some(Box::new(|| Some(("bad.pctools".into(), Ok(b"broken".to_vec())))));
        assert!(load_tools(&mut app, true).is_err());
        assert_eq!(app.session.presets.tool_presets, before);
    }

    #[test]
    fn tools_file_picker_is_queued_until_root_consumes_it_once() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let mut app = PhotocraftApp::new(
            photocraft_engine::Session::new(),
            crate::Services {
                pick_open: Some(Box::new(move || {
                    counter.fetch_add(1, Ordering::SeqCst);
                    None
                })),
                ..Default::default()
            },
        );
        let ctx = egui::Context::default();
        queue_tools(&ctx, ToolsFileAction::Load, app.frame);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        process_pending(&mut app, &ctx);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "same-frame discard pass must not show the picker");
        process_pending(&mut app, &ctx);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a second discard pass must leave the request queued");
        app.frame += 1;
        process_pending(&mut app, &ctx);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        process_pending(&mut app, &ctx);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!app.ui.status_error);
    }
}
