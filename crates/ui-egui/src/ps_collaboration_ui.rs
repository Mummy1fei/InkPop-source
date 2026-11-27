//! Explicit Raw recompute decisions and appearance-preserving collaboration exports.
use photocraft_doc::LayerId;
use photocraft_engine::ps_collaboration_cmds as engine;
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, state::DialogKind};

const MARK: &str = "__photoshopCollaboration";

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

fn fields(app: &PhotocraftApp, mode: &str) -> Result<Map<String, Value>, String> {
    let st = app.session.active().ok_or("no document")?;
    let mut f = Map::new();
    f.insert(MARK.into(), json!(mode));
    f.insert("__label".into(), json!(tl!("Photoshop Collaboration")));
    f.insert("document".into(), json!(st.doc.id));
    f.insert("revision".into(), json!(st.revision));
    Ok(f)
}

fn check_document(app: &PhotocraftApp, f: &Map<String, Value>) -> Result<(), String> {
    let st = app.session.active().ok_or("no document")?;
    if f.get("document") != Some(&json!(st.doc.id)) || f.get("revision").and_then(Value::as_u64) != Some(st.revision) {
        return Err(tl!("The document changed. Open the collaboration dialog again.").into());
    }
    Ok(())
}

fn automation(app: &PhotocraftApp) -> bool {
    app.automation_input || app.session.authorize.is_some()
}

/// Intercept before jobs are scheduled: never wait for a worker to discover lost consent.
pub fn intercept_command(app: &mut PhotocraftApp, command: &str, params: &Value) -> Option<Result<Value, String>> {
    let replay = (command.starts_with("layer.smartFilter.") && command != "layer.smartFilter.acceptRawRecompute")
        || command.starts_with("filter.")
        || matches!(command, "layer.smartObjects.replaceContents" | "layer.smartObjects.relinkToFile" | "layer.smartObjects.updateModifiedContent");
    if !replay {
        return None;
    }
    let st = app.session.active()?;
    let layer = params.get("layer").and_then(Value::as_u64).map(LayerId).or(st.active_layer)?;
    if !engine::requires_raw_recompute_consent(&st.doc, layer) {
        return None;
    }
    if automation(app) {
        return Some(Err(format!("{} Run layer.smartFilter.acceptRawRecompute explicitly before retrying.", engine::RAW_RECOMPUTE_CONSENT_ERROR)));
    }
    Some((|| {
        let mut f = fields(app, "recompute")?;
        f.insert("layer".into(), json!(layer.0));
        f.insert("command".into(), json!(command));
        f.insert("params".into(), params.clone());
        Ok(json!({"dialog":app.ui.open_dialog(DialogKind::Command, f),"pendingConsent":true}))
    })())
}

pub fn intercept_editor(app: &mut PhotocraftApp, layer: LayerId, index: usize) -> Result<bool, String> {
    let st = app.session.active().ok_or("no document")?;
    if !engine::requires_raw_recompute_consent(&st.doc, layer) {
        return Ok(false);
    }
    if automation(app) {
        return Err(format!("{} Run layer.smartFilter.acceptRawRecompute explicitly before opening the editor.", engine::RAW_RECOMPUTE_CONSENT_ERROR));
    }
    let mut f = fields(app, "recompute")?;
    f.insert("layer".into(), json!(layer.0));
    f.insert("editorIndex".into(), json!(index));
    app.ui.open_dialog(DialogKind::Command, f);
    Ok(true)
}

fn open_report(app: &mut PhotocraftApp, path: Option<&str>, normal_save: bool) -> Result<Value, String> {
    let report = app.run("file.photoshopCompatibilityReport", json!({}))?;
    let mut f = fields(app, if normal_save { "save" } else { "copy" })?;
    f.insert("report".into(), report);
    f.insert("includeMaster".into(), json!(true));
    f.insert("saveMode".into(), json!("copy"));
    if let Some(path) = path {
        f.insert("path".into(), json!(path));
    }
    Ok(json!({"dialog": app.ui.open_dialog(DialogKind::Command, f)}))
}

pub fn invoke(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    if params.get("path").is_some() {
        return export(app, params);
    }
    open_report(app, None, false)
}

pub fn preflight_save(app: &mut PhotocraftApp, path: &str) -> Result<bool, String> {
    let ext = path.rsplit('.').next().unwrap_or("");
    if !matches!(ext.to_ascii_lowercase().as_str(), "psd" | "psb") {
        return Ok(false);
    }
    let st = app.session.active().ok_or("no document")?;
    if engine::analyze(&st.doc).map_err(|e| e.to_string())?.raw_filter_count == 0 {
        return Ok(false);
    }
    open_report(app, Some(path), true)?;
    Ok(true)
}

pub fn preflight_copy(app: &mut PhotocraftApp, path: &str) -> Result<bool, String> {
    if !preflight_save(app, path)? {
        return Ok(false);
    }
    if let Some(dialog) = app.ui.dialogs.last_mut() {
        dialog.fields.insert("saveACopy".into(), json!(true));
    }
    Ok(true)
}

pub fn save_result(app: &PhotocraftApp, path: String, warnings: Vec<String>) -> Value {
    if let Some(dialog) = app.ui.dialogs.iter().rev().find(|d| owns(&d.fields) && d.fields.get(MARK).and_then(Value::as_str) == Some("save")) {
        json!({"pending":true,"dialog":dialog.id,"targetPath":path})
    } else if app.tiff_options.is_some() || app.jpeg_options.is_some() {
        json!({"pending":true,"targetPath":path})
    } else {
        json!({"path":path,"warnings":warnings})
    }
}

pub fn body(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    if f.get(MARK).and_then(Value::as_str) == Some("recompute") {
        ui.label(tl!("Photoshop and InkPop may render Camera Raw settings differently."));
        ui.add_space(8.0);
        ui.label(tl!(
            "Continuing permits InkPop to recompute this smart-filter stack. Colors may change. Cancel keeps the cached appearance and settings unchanged."
        ));
        return;
    }
    ui.label(tl!("The Photoshop copy turns Camera Raw smart-filter stacks into pixel layers using the current displayed result. Keep the editable InkPop master to continue adjusting the complete recipe."));
    ui.add_space(8.0);
    if let Some(layers) = f.get("report").and_then(|r| r.get("layers")).and_then(Value::as_array) {
        for layer in layers {
            ui.strong(layer.get("name").and_then(Value::as_str).unwrap_or(""));
            if let Some(filters) = layer.get("filters").and_then(Value::as_array) {
                for filter in filters {
                    let command = filter.get("command").and_then(Value::as_str).unwrap_or("");
                    let mapped = filter.get("standardParameters").and_then(Value::as_bool) == Some(true);
                    let label = if command == "psd.unsupportedFilter" {
                        tl!("Camera Raw (preserved)")
                    } else {
                        photocraft_engine::commands::find(command).map_or(tl!("Smart Filter"), |spec| crate::i18n::t(spec.label))
                    };
                    ui.label(format!(
                        "  {label}: {}",
                        if command == "psd.unsupportedFilter" {
                            tl!("Preserved Photoshop filter; settings cannot be edited in InkPop")
                        } else if mapped {
                            tl!("Standard parameters mapped; equal rendering is not guaranteed")
                        } else {
                            tl!("InkPop-only parameters; editable in the native master")
                        }
                    ));
                }
            }
        }
    }
    ui.add_space(8.0);
    if f.get(MARK).and_then(Value::as_str) == Some("save") {
        let mut mode = f.get("saveMode").and_then(Value::as_str).unwrap_or("copy").to_owned();
        ui.radio_value(&mut mode, "copy".into(), tl!("Save an appearance-preserving Photoshop copy"));
        ui.radio_value(&mut mode, "parameters".into(), tl!("Save original smart-filter parameters (rendering may differ in Photoshop)"));
        f.insert("saveMode".into(), json!(mode));
    }
    if f.get("saveMode").and_then(Value::as_str) != Some("parameters") {
        let mut include = f.get("includeMaster").and_then(Value::as_bool).unwrap_or(true);
        crate::widgets::checkbox(ui, &mut include, tl!("Also save editable InkPop master (.pcraft)"));
        f.insert("includeMaster".into(), json!(include));
        ui.weak(tl!("Choose a new PSD/PSB filename next. The companion uses the same name with .inkpop.pcraft. Existing files are never replaced."));
    }
}

pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    check_document(app, f)?;
    if f.get(MARK).and_then(Value::as_str) == Some("recompute") {
        let layer = f.get("layer").and_then(Value::as_u64).ok_or("missing layer")?;
        let command = f.get("command").and_then(Value::as_str);
        if command.is_some() {
            let requested = f.get("params").and_then(|params| params.get("layer")).and_then(Value::as_u64);
            let selected = app.session.active().and_then(|st| st.active_layer).map(|id| id.0);
            if requested.is_some_and(|id| id != layer) || (requested.is_none() && selected != Some(layer)) {
                return Err(tl!("The document changed. Open the collaboration dialog again.").into());
            }
        }
        if command.is_some_and(|c| {
            c == "layer.smartFilter.acceptRawRecompute"
                || !(c.starts_with("layer.smartFilter.") || c.starts_with("filter.") || c.starts_with("layer.smartObjects."))
        }) {
            return Err("invalid pending recompute command".into());
        }
        app.run("layer.smartFilter.acceptRawRecompute", json!({"layer":layer}))?;
        if let Some(index) = f.get("editorIndex").and_then(Value::as_u64) {
            let index = usize::try_from(index).map_err(|_| "invalid filter index")?;
            let st = app.session.active().ok_or("no document")?;
            app.pending_raw_editor = Some((st.doc.id, st.revision, LayerId(layer), index));
            return Ok(json!({"pendingEditor":true}));
        }
        return app.run(command.ok_or("missing pending command")?, f.get("params").cloned().unwrap_or(json!({})));
    }
    if f.get(MARK).and_then(Value::as_str) == Some("save") && f.get("saveMode").and_then(Value::as_str) == Some("parameters") {
        if automation(app) {
            return Err("use doc.save for a capability-scoped save".into());
        }
        let path = f.get("path").and_then(Value::as_str).ok_or("missing save path")?;
        if f.get("saveACopy").and_then(Value::as_bool) == Some(true) {
            return crate::view_cmds::write_copy(app, path.to_owned());
        }
        return app.write_document(path.to_owned(), &Default::default()).map(|(path, warnings)| json!({"path":path,"warnings":warnings}));
    }
    if automation(app) {
        return Err("use doc.exportPhotoshopCopy with a new relative output path".into());
    }
    let st = app.session.active().ok_or("no document")?;
    let name = st.doc.name.rsplit_once('.').map_or(st.doc.name.as_str(), |(stem, _)| stem);
    let suggested = format!("{name}_PS.psd");
    let path = app.services.pick_save.as_mut().and_then(|pick| pick(&suggested)).ok_or("cancelled")?;
    // A native picker yields the UI thread; ensure it still refers to the same snapshot.
    check_document(app, f)?;
    let include = f.get("includeMaster").and_then(Value::as_bool).unwrap_or(true);
    let mut params = json!({"path":path});
    if !include {
        params["masterPath"] = Value::Null;
    }
    export(app, &params)
}

/// File effects use the actual invocation's service authority, never a dialog field.
pub fn export(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    let scoped = automation(app);
    export_with_authority(app, params, scoped)
}

pub fn export_automation(app: &mut PhotocraftApp, params: &Value) -> Result<Value, String> {
    export_with_authority(app, params, true)
}

fn export_with_authority(app: &mut PhotocraftApp, params: &Value, scoped: bool) -> Result<Value, String> {
    let (path, master) = engine::export_paths(params).map_err(|e| e.to_string())?;
    let st = app.session.active().ok_or("no document")?;
    #[cfg(not(target_arch = "wasm32"))]
    if !scoped {
        engine::validate_export_targets(&path, master.as_deref(), st.path.as_deref()).map_err(|e| e.to_string())?;
    }
    let saved_snapshot = (st.doc.id, st.revision);
    let encoded = engine::encode_copy(&st.doc, &path, master.is_some()).map_err(|e| e.to_string())?;
    let mut files = vec![(path.clone(), encoded.psd)];
    if let (Some(path), Some(bytes)) = (&master, encoded.master) {
        files.push((path.clone(), bytes));
    }
    let write = if scoped { &mut app.services.automation_write_new_files } else { &mut app.services.write_new_files };
    write.as_mut().ok_or("collaboration export authority is not configured")?(&files)?;
    // A pending close is satisfied by the successfully written PSD itself.  The
    // optional .inkpop.pcraft companion is useful for editability, but choosing
    // “save PSD only” must not leave the close prompt parked forever.
    crate::discard_ui::note_saved_companion(app, saved_snapshot.0, saved_snapshot.1);
    app.ui.status = match &master {
        Some(master) => format!("{}: {path} + {master}", tl!("Photoshop copy saved")),
        None => format!("{}: {path}", tl!("Photoshop copy saved")),
    };
    app.ui.status_error = false;
    crate::notices::output_warnings(app, tl!("Photoshop copy saved"), &encoded.warnings);
    Ok(json!({"path":path,"masterPath":master,"report":encoded.report,"warnings":encoded.warnings}))
}

pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.viewport_id() != egui::ViewportId::ROOT {
        return;
    }
    let Some((doc, revision, layer, index)) = app.pending_raw_editor.take() else { return };
    let valid = app.session.active().is_some_and(|st| st.doc.id == doc && st.revision == revision);
    let result = if valid {
        crate::camera_raw_ui::open_smart_filter(app, ctx, layer, index)
    } else {
        Err(tl!("The document changed. Open the collaboration dialog again.").into())
    };
    if let Err(error) = result {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{LayerContent, SmartFilter};
    use std::sync::{Arc, Mutex};

    fn app(imported: bool) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":16,"height":12})).unwrap();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        app.session
            .edit("Raw fixture", |doc, _| {
                let LayerContent::Smart(sm) = &mut doc.layer_mut(id).unwrap().content else { panic!() };
                let filter = SmartFilter {
                    command: "filter.cameraRaw".into(),
                    params: json!({"exposure":0.5}),
                    blend: photocraft_color::BlendMode::Normal,
                    opacity: 1.,
                    visible: true,
                };
                sm.smart_filters = vec![if imported {
                    photocraft_io::smart_map::filter_from_item(&photocraft_io::smart_map::item_for_filter(&filter).unwrap())
                } else {
                    filter
                }];
                Ok(())
            })
            .unwrap();
        app
    }

    #[test]
    fn imported_raw_preflight_blocks_jobs_cancel_and_stale_confirmation() {
        let mut app = app(true);
        let before = app.session.active().unwrap().doc.clone();
        let result = app.run("layer.smartFilter.setVisible", json!({"index":0,"visible":false})).unwrap();
        assert_eq!(result["pendingConsent"], true);
        assert_eq!(*app.session.active().unwrap().doc, *before);
        assert!(!app.session.has_jobs());
        let dialog = app.ui.dialogs.last().unwrap().clone();
        app.ui.close_dialog(dialog.id);
        assert_eq!(*app.session.active().unwrap().doc, *before);
        app.run("layer.renameLayer", json!({"name":"Changed"})).unwrap();
        assert!(confirm(&mut app, &dialog.fields).is_err());
        assert!(engine::requires_raw_recompute_consent(&app.session.active().unwrap().doc, app.session.active().unwrap().active_layer.unwrap()));
    }

    #[test]
    fn automation_requires_explicit_accept_without_opening_dialog() {
        let mut app = app(true);
        app.automation_input = true;
        assert!(app.run("layer.smartFilter.setVisible", json!({"index":0,"visible":false})).unwrap_err().contains(engine::RAW_RECOMPUTE_CONSENT_ERROR));
        assert!(app.ui.dialogs.is_empty());
        let id = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.smartFilter.acceptRawRecompute", json!({"layer":id.0})).unwrap();
        assert!(!engine::requires_raw_recompute_consent(&app.session.active().unwrap().doc, id));
    }

    #[test]
    fn scoped_export_cannot_fall_back_to_ambient_writer_or_adopt_path() {
        let mut app = app(false);
        app.services.write_new_files = Some(Box::new(|_| panic!("ambient service must not be reached")));
        let before = app.session.active().unwrap().doc.clone();
        let revision = app.session.active().unwrap().revision;
        assert!(export_automation(&mut app, &json!({"path":"copy.psd"})).is_err());
        let written = Arc::new(Mutex::new(Vec::new()));
        let output = written.clone();
        app.services.automation_write_new_files = Some(Box::new(move |files| {
            *output.lock().unwrap() = files.to_vec();
            Ok(())
        }));
        let result = export_automation(&mut app, &json!({"path":"copy.psd"})).unwrap();
        assert_eq!(result["masterPath"], "copy.inkpop.pcraft");
        let output = written.lock().unwrap();
        assert_eq!(output.len(), 2);
        let imported = photocraft_io::import("copy.psd", &output[0].1).unwrap().document;
        assert!(!matches!(imported.layers.last().unwrap().content, LayerContent::Smart(_)));
        let master = photocraft_io::import("copy.pcraft", &output[1].1).unwrap().document;
        assert!(matches!(master.layers.last().unwrap().content, LayerContent::Smart(_)));
        let st = app.session.active().unwrap();
        assert_eq!(*st.doc, *before);
        assert_eq!(st.revision, revision);
        assert!(st.path.is_none());
    }

    #[test]
    fn ordinary_save_adopts_the_first_path_and_round_trips_raw_on_each_save_v117() {
        for path in ["working.PSD", "working.psb"] {
            let mut app = app(false);
            let mut first_pick = Some(path.to_owned());
            app.services.pick_save = Some(Box::new(move |_| Some(first_pick.take().expect("only the first save asks for a path"))));
            app.services.export = Some(Box::new(|doc, path, _| {
                let result = photocraft_io::export(doc, path, &Default::default()).map_err(|error| error.to_string())?;
                Ok((result.bytes, result.warnings))
            }));
            let saved = Arc::new(Mutex::new(Vec::new()));
            let writes = saved.clone();
            app.services.write = Some(Box::new(move |target, bytes| {
                assert_eq!(target, path);
                writes.lock().unwrap().push(bytes.to_vec());
                Ok(())
            }));
            let ctx = egui::Context::default();
            for (i, name) in ["First edit", "Second edit"].into_iter().enumerate() {
                app.run("layer.renameLayer", json!({"name":name})).unwrap();
                let answer = crate::menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
                assert_eq!(answer["path"], path);
                assert!(app.ui.dialogs.is_empty());
                let st = app.session.active().unwrap();
                assert!(!st.is_dirty());
                assert_eq!(st.path.as_deref(), Some(path));
                let writes = saved.lock().unwrap();
                assert_eq!(writes.len(), i + 1);
                let reopened = photocraft_io::import(path, &writes[i]).unwrap().document;
                let layer = reopened.layers.iter().find(|layer| layer.name == name).unwrap();
                let LayerContent::Smart(smart) = &layer.content else { panic!("working file lost its smart object") };
                assert_eq!(smart.smart_filters[0].command, "filter.cameraRaw");
                assert_eq!(smart.smart_filters[0].params["exposure"], 0.5);
            }
            let answer = crate::menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path":"other.psd"})).unwrap();
            assert_eq!(answer["pending"], true);
            assert_eq!(app.ui.dialogs.last().unwrap().fields["saveMode"], "copy");
            assert_eq!(saved.lock().unwrap().len(), 2);
        }
    }

    #[test]
    fn save_preflight_only_for_raw_psd_and_defaults_to_companion_copy() {
        let mut app = app(false);
        assert!(!preflight_save(&mut app, "master.pcraft").unwrap());
        assert!(preflight_save(&mut app, "mother.psd").unwrap());
        let f = &app.ui.dialogs.last().unwrap().fields;
        assert_eq!(f["saveMode"], "copy");
        assert_eq!(f["includeMaster"], true);
        assert_eq!(f["report"]["rawFilterCount"], 1);
    }

    #[test]
    fn fingerprint_includes_untiled_default_mask_and_format() {
        let a = photocraft_doc::LayerMask::reveal_all();
        let b = photocraft_doc::LayerMask::hide_all();
        assert_eq!(a.surface.tile_count(), 0);
        assert_eq!(b.surface.tile_count(), 0);
        assert_ne!(crate::surface_fingerprint(&a.surface), crate::surface_fingerprint(&b.surface));
    }
    #[test]
    fn thumbnail_dimensions_change_identity_even_when_tiles_stay_identical_v84() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":64,"height":64})).unwrap();
        let ctx = egui::Context::default();
        let doc = app.session.active().unwrap().doc.clone();
        let layer = doc.layers.last().unwrap();
        app.layer_thumb(&ctx, &doc, layer);
        let key = (layer.id, crate::mask_thumbs_ui::THUMB_LAYER);
        let old = app.thumbs.get(&key).unwrap().0;
        app.session
            .edit("Canvas height", |doc, _| {
                doc.size.height = 128;
                Ok(())
            })
            .unwrap();
        let doc = app.session.active().unwrap().doc.clone();
        app.layer_thumb(&ctx, &doc, doc.layer(key.0).unwrap());
        assert_ne!(app.thumbs.get(&key).unwrap().0, old);
    }
    #[test]
    fn implicit_raw_confirmation_is_bound_to_its_original_layer_v84() {
        let mut app = app(true);
        let original = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        let other = app.session.active().unwrap().active_layer.unwrap();
        app.run("layer.select", json!({"layer":original.0})).unwrap();
        app.run("layer.smartFilter.setVisible", json!({"index":0,"visible":false})).unwrap();
        let pending = app.ui.dialogs.last().unwrap().fields.clone();
        app.run("layer.select", json!({"layer":other.0})).unwrap();
        assert!(confirm(&mut app, &pending).is_err());
        assert!(engine::requires_raw_recompute_consent(&app.session.active().unwrap().doc, original));
    }
}
