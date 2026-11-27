//! Explicit appearance-preserving Photoshop collaboration copies and Raw replay consent.
//!
//! Adobe's descriptor format is not a promise of identical processing. A collaboration
//! copy bakes the displayed smart-object cache, while an optional native companion keeps
//! InkPop's complete recipe. Neither operation adopts a path or edits the open document.

use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

use photocraft_doc::{Document, Layer, LayerContent, LayerId, SmartFilter, SmartObject, SmartSource};
use serde::Serialize;
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::{EngineError, Result, Session};

pub const RAW_RECOMPUTE_CONSENT_ERROR: &str = "InkPop Camera Raw recompute requires consent:";
pub const RAW_TEMPLATE_KEY: &str = "__cameraRawPsd";
pub const RAW_ACCEPTED_KEY: &str = "__inkpopRawRecomputeAccepted";
const MAX_DEPTH: usize = photocraft_doc::MAX_GROUP_DEPTH;
const MAX_LAYERS: usize = 100_000;

fn other(message: impl Into<String>) -> EngineError {
    EngineError::Other(message.into())
}

pub(crate) fn filter_needs_consent(filter: &SmartFilter) -> bool {
    filter.command == "filter.cameraRaw" && filter.params.get(RAW_TEMPLATE_KEY).is_some() && filter.params.get(RAW_ACCEPTED_KEY) != Some(&Value::Bool(true))
}

fn is_raw_filter(filter: &SmartFilter) -> bool {
    filter.command == "filter.cameraRaw"
        || (filter.command == photocraft_io::smart_map::UNSUPPORTED_FILTER
            && matches!(filter.params.get("filterID").and_then(Value::as_i64), Some(2621 | 2783)))
}

pub(crate) fn check_smart_recompute(sm: &SmartObject) -> Result<()> {
    // Even a hidden filter or disabled stack has a preserved cache. Toggling, deleting
    // or changing the source would replace that cache, so check both old and new stacks.
    if sm.smart_filters.iter().any(filter_needs_consent) {
        return Err(other(format!(
            "{RAW_RECOMPUTE_CONSENT_ERROR} Photoshop and InkPop may render these parameters differently. The cached appearance and settings were kept unchanged."
        )));
    }
    Ok(())
}

pub fn requires_raw_recompute_consent(doc: &Document, id: LayerId) -> bool {
    matches!(doc.layer(id).map(|l| &l.content), Some(LayerContent::Smart(sm)) if sm.smart_filters.iter().any(filter_needs_consent))
}

/// Call before scheduling an editor/render job, as well as at the final engine mutation.
pub fn check_raw_recompute(doc: &Document, id: LayerId) -> Result<()> {
    match doc.layer(id).map(|l| &l.content) {
        Some(LayerContent::Smart(sm)) => check_smart_recompute(sm),
        Some(_) => Err(other("the target layer is not a smart object")),
        None => Err(other("the target layer does not exist")),
    }
}

/// Parameter updates may not forge consent or erase the imported Adobe template.
pub(crate) fn validate_raw_params_update(filter: &SmartFilter, new: &Value) -> Result<()> {
    if filter.command != "filter.cameraRaw" {
        return Ok(());
    }
    if filter.params.get(RAW_TEMPLATE_KEY).is_some() && !new.is_object() {
        return Err(other("imported Camera Raw parameters must be updated as an object; the source descriptor must be retained"));
    }
    for key in [RAW_TEMPLATE_KEY, RAW_ACCEPTED_KEY] {
        if new.get(key).is_some() && new.get(key) != filter.params.get(key) {
            return Err(other("Camera Raw origin and recompute consent can only be changed by the dedicated consent command"));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterCompatibility {
    pub index: usize,
    pub command: String,
    pub visible: bool,
    pub standard_parameters: bool,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerCompatibility {
    pub layer: u64,
    pub name: String,
    pub requires_recompute_consent: bool,
    pub filters: Vec<FilterCompatibility>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompatibilityReport {
    pub raw_filter_count: usize,
    pub baked_layer_count: usize,
    pub layers: Vec<LayerCompatibility>,
    pub warnings: Vec<String>,
}

fn visit(layers: &[Layer], depth: usize, count: &mut usize, f: &mut impl FnMut(&Layer) -> Result<()>) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(other("Photoshop collaboration copy: layer nesting exceeds the safe limit"));
    }
    for layer in layers {
        *count = count.checked_add(1).ok_or_else(|| other("layer count overflow"))?;
        if *count > MAX_LAYERS {
            return Err(other("Photoshop collaboration copy: too many layers"));
        }
        f(layer)?;
        if let LayerContent::Group(group) = &layer.content {
            visit(&group.children, depth + 1, count, f)?;
        }
    }
    Ok(())
}

pub fn analyze(doc: &Document) -> Result<CompatibilityReport> {
    let mut report = CompatibilityReport::default();
    visit(&doc.layers, 0, &mut 0, &mut |layer| {
        let LayerContent::Smart(sm) = &layer.content else { return Ok(()) };
        let raw_count = sm.smart_filters.iter().filter(|f| is_raw_filter(f)).count();
        if raw_count == 0 {
            return Ok(());
        }
        report.raw_filter_count = report.raw_filter_count.saturating_add(raw_count);
        report.baked_layer_count = report.baked_layer_count.saturating_add(1);
        let filters = sm.smart_filters.iter().enumerate().map(|(index, filter)| {
            let mapped = photocraft_io::smart_map::item_for_filter(filter);
            FilterCompatibility {
                index,
                command: filter.command.clone(),
                visible: filter.visible,
                standard_parameters: mapped.is_ok() && filter.command != photocraft_io::smart_map::UNSUPPORTED_FILTER,
                detail: match mapped {
                    Ok(_) if filter.command == "filter.cameraRaw" => "Standard parameters are mapped; equal rendering in Photoshop is not guaranteed. The complete smart-filter stack is baked in the collaboration copy.".into(),
                    Ok(_) if is_raw_filter(filter) => "This Photoshop Camera Raw descriptor is preserved, but cannot be edited or recomputed in InkPop. Its displayed cache is baked in the collaboration copy.".into(),
                    Ok(_) => "The complete smart-filter stack is baked with Camera Raw in the collaboration copy.".into(),
                    Err(error) => error,
                },
            }
        }).collect();
        report.layers.push(LayerCompatibility {
            layer: layer.id.0,
            name: layer.name.clone(),
            requires_recompute_consent: sm.smart_filters.iter().any(filter_needs_consent),
            filters,
        });
        if sm.cache.is_none() {
            report.warnings.push(format!("{}: no rendered cache; an appearance copy cannot be created safely.", layer.name));
        }
        if matches!(sm.source, SmartSource::Linked { .. }) {
            report.warnings.push(format!("{}: linked source is not read or refreshed; the copy uses the currently displayed cache. The native companion retains the link and cache and is not fully self-contained.", layer.name));
        }
        Ok(())
    })?;
    if report.raw_filter_count > 0 {
        report.warnings.push("Camera Raw layers become ordinary pixel layers in the Photoshop copy. Keep the InkPop native companion to continue editing the complete recipe. Other layers, masks and effects remain layered; overall cross-application rendering is not guaranteed.".into());
    }
    Ok(report)
}

/// Clone the exact cache used by the compositor; never refresh from a possibly changed
/// linked source and never run a different application's Raw recipe implicitly.
pub fn prepare(doc: &Document) -> Result<(Document, CompatibilityReport)> {
    let report = analyze(doc)?; // validates depth/count before cloning or recursive mutation
    let mut copy = doc.clone();
    fn bake(layers: &mut [Layer], format: photocraft_color::PixelFormat) -> Result<()> {
        for layer in layers {
            match &mut layer.content {
                LayerContent::Group(group) => bake(&mut group.children, format)?,
                LayerContent::Smart(sm) if sm.smart_filters.iter().any(is_raw_filter) => {
                    let cache = sm.cache.as_ref().ok_or_else(|| other(format!("{}: missing displayed Camera Raw cache; export was stopped", layer.name)))?;
                    if cache.format() != format {
                        return Err(other(format!("{}: cached pixel format differs from the document; export was stopped", layer.name)));
                    }
                    layer.content = LayerContent::Raster(cache.clone());
                    layer.psd_blocks.retain(|(key, _)| !matches!(key, b"SoLd" | b"SoLE" | b"PlLd" | b"InSF" | b"FEid" | b"FXid" | b"PxSD"));
                    layer.fill_cache = None;
                }
                _ => {}
            }
        }
        Ok(())
    }
    bake(&mut copy.layers, doc.pixel_format())?;
    // Preserved global linked data may still belong to non-Raw smart objects. The
    // PSD writer owns source reconciliation; do not indiscriminately remove lnk2.
    Ok((copy, report))
}

#[derive(Debug)]
pub struct EncodedCopy {
    pub psd: Vec<u8>,
    pub master: Option<Vec<u8>>,
    pub report: CompatibilityReport,
    pub warnings: Vec<String>,
}

/// The ordinary PSD exporter can read external linked files. This pure entry point
/// must reject those dependencies (including native embedded-document dependencies)
/// before invoking it. Preserved PSD in-memory links never need ambient file access.
fn check_no_external_reads(doc: &Document, nesting: usize, checked: &mut std::collections::HashSet<[u8; 32]>) -> Result<()> {
    if nesting > 8 {
        return Err(other("embedded smart-object nesting exceeds the collaboration limit"));
    }
    visit(&doc.layers, 0, &mut 0, &mut |layer| {
        let LayerContent::Smart(sm) = &layer.content else { return Ok(()) };
        match &sm.source {
            SmartSource::Linked { path } => {
                let known = doc
                    .metadata
                    .psd_global_blocks
                    .iter()
                    .filter(|(_, key, _)| matches!(key, b"lnk2" | b"lnk3" | b"lnkD" | b"lnkE"))
                    .any(|(_, _, bytes)| photocraft_io::linked::block_uuids(bytes).iter().any(|uuid| uuid == path));
                let template = sm.psd_raw.as_ref().and_then(|bytes| photocraft_io::smart_map::parse_sold(bytes));
                if !known && !template.is_some_and(|placed| placed.idnt == *path) {
                    return Err(other(format!(
                        "{}: an external linked source would need to be read. Embed it explicitly before exporting a collaboration copy.",
                        layer.name
                    )));
                }
            }
            SmartSource::Embedded { bytes, .. } if photocraft_format::is_pcraft(bytes) => {
                let key = *blake3::hash(bytes).as_bytes();
                if checked.insert(key) {
                    if checked.len() > 1024 {
                        return Err(other("too many embedded source documents"));
                    }
                    let nested = photocraft_format::load_from_bytes(bytes).map_err(|e| other(format!("cannot inspect embedded source: {e}")))?;
                    check_no_external_reads(&nested, nesting + 1, checked)?;
                }
            }
            _ => {}
        }
        Ok(())
    })
}

pub fn encode_copy(doc: &Document, path: &str, include_master: bool) -> Result<EncodedCopy> {
    validate_extension(path, &["psd", "psb"])?;
    let (copy, report) = prepare(doc)?;
    check_no_external_reads(&copy, 0, &mut Default::default())?;
    let (psd, mut warnings) = crate::file_cmds::encode(&copy, path, crate::file_cmds::SaveOpts::default())?;
    warnings.extend(report.warnings.iter().cloned());
    let master = if include_master {
        // Native encoding preserves links verbatim. Resolving them here would bypass
        // capability-scoped file services and could silently read unrelated files.
        Some(crate::smart_cmds::encode_source(doc)?)
    } else {
        None
    };
    Ok(EncodedCopy { psd, master, report, warnings })
}

fn validate_path(path: &str) -> Result<()> {
    if path.is_empty() || path.trim() != path || path.len() > 32_000 || path.chars().any(char::is_control) || Path::new(path).file_name().is_none() {
        return Err(other("provide a nonempty file path without control characters"));
    }
    Ok(())
}

fn validate_extension(path: &str, extensions: &[&str]) -> Result<()> {
    validate_path(path)?;
    if !Path::new(path).extension().and_then(|s| s.to_str()).is_some_and(|ext| extensions.iter().any(|want| ext.eq_ignore_ascii_case(want))) {
        return Err(other(format!("expected file extension: {}", extensions.join(" / "))));
    }
    Ok(())
}

pub fn default_master_path(path: &str) -> Result<String> {
    validate_extension(path, &["psd", "psb"])?;
    Ok(Path::new(path).with_extension("inkpop.pcraft").to_string_lossy().into_owned())
}

/// Parse strictly; omitted companion means enabled, explicit null/false disables it.
pub fn export_paths(params: &Value) -> Result<(String, Option<String>)> {
    let path = params.get("path").and_then(Value::as_str).ok_or_else(|| other("`path` must be a PSD/PSB file path"))?.to_string();
    validate_extension(&path, &["psd", "psb"])?;
    let master = match params.get("masterPath") {
        None => Some(default_master_path(&path)?),
        Some(Value::Null | Value::Bool(false)) => None,
        Some(Value::String(master)) => {
            validate_extension(master, &["pcraft"])?;
            Some(master.clone())
        }
        _ => return Err(other("`masterPath` must be a .pcraft path, null or false")),
    };
    Ok((path, master))
}

#[cfg(not(target_arch = "wasm32"))]
fn destination_identity(path: &str) -> Result<PathBuf> {
    let path = Path::new(path);
    // symlink_metadata rejects dangling destination links as well as ordinary files.
    match std::fs::symlink_metadata(path) {
        Ok(_) => return Err(other(format!("export only creates new files; destination already exists: {}", path.display()))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(other(format!("cannot inspect destination {}: {error}", path.display()))),
    }
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let parent = std::fs::canonicalize(parent).map_err(|error| other(format!("cannot resolve output folder: {error}")))?;
    Ok(parent.join(path.file_name().ok_or_else(|| other("output needs a file name"))?))
}

/// Check before presenting a final export and again just before a capability-scoped
/// writer commits. Writers must also use create-new semantics to avoid a TOCTOU overwrite.
#[cfg(not(target_arch = "wasm32"))]
pub fn validate_export_targets(path: &str, master: Option<&str>, source: Option<&str>) -> Result<()> {
    validate_extension(path, &["psd", "psb"])?;
    let output = destination_identity(path)?;
    let identity = |p: &Path| p.to_string_lossy().replace('\\', "/").to_lowercase();
    let output_id = identity(&output);
    if let Some(master) = master {
        validate_extension(master, &["pcraft"])?;
        if identity(&destination_identity(master)?) == output_id {
            return Err(other("Photoshop copy and native companion must have different paths"));
        }
    }
    if let Some(source) = source.filter(|s| !s.is_empty()) {
        let source = std::fs::canonicalize(source).unwrap_or_else(|_| PathBuf::from(source));
        let source_id = identity(&source);
        if source_id == output_id || master.is_some_and(|p| identity(&PathBuf::from(p)) == source_id) {
            return Err(other("a collaboration copy cannot replace the open document"));
        }
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
pub fn validate_export_targets(path: &str, master: Option<&str>, _source: Option<&str>) -> Result<()> {
    validate_extension(path, &["psd", "psb"])?;
    if let Some(master) = master {
        validate_extension(master, &["pcraft"])?;
    }
    Ok(())
}

fn has_doc(session: &Session) -> std::result::Result<(), String> {
    session.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn accept(session: &mut Session, params: &Value) -> Result<Value> {
    if params.get("layer").is_some_and(|v| v.as_u64().is_none()) {
        return Err(other("`layer` must be a layer id"));
    }
    let id = layer_param(session, params)?;
    session.edit("Accept InkPop Camera Raw Recompute", |doc, _| {
        let locks = doc.effective_locks(id);
        if locks.all || locks.pixels {
            return Err(other("the target layer is locked"));
        }
        let layer = doc.layer_mut(id).ok_or_else(|| other("the target layer does not exist"))?;
        let LayerContent::Smart(sm) = &mut layer.content else { return Err(other("the target layer is not a smart object")) };
        let mut accepted = 0usize;
        for filter in &mut sm.smart_filters {
            if filter_needs_consent(filter) {
                let params = filter.params.as_object_mut().ok_or_else(|| other("invalid Camera Raw parameter object"))?;
                params.insert(RAW_ACCEPTED_KEY.into(), Value::Bool(true));
                accepted = accepted.saturating_add(1);
            }
        }
        if accepted == 0 {
            return Err(other("this layer has no Camera Raw recompute decision pending"));
        }
        Ok(json!({"layer":id.0,"accepted":accepted}))
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn export_native(session: &mut Session, params: &Value) -> Result<Value> {
    let (path, master_path) = export_paths(params)?;
    let state = session.active().ok_or(EngineError::NoDocument)?;
    validate_export_targets(&path, master_path.as_deref(), state.path.as_deref())?;
    let encoded = encode_copy(&state.doc, &path, master_path.is_some())?;
    let mut files = vec![(path.as_str(), encoded.psd.as_slice())];
    if let (Some(path), Some(bytes)) = (master_path.as_deref(), encoded.master.as_deref()) {
        files.push((path, bytes));
    }
    write_new_files(&files)?;
    Ok(json!({"path":path,"masterPath":master_path,"report":encoded.report,"warnings":encoded.warnings}))
}

/// Native service implementation; capability-scoped services provide equivalent
/// create-new and rollback behavior without calling ambient IO.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_new_files(files: &[(&str, &[u8])]) -> Result<()> {
    use std::io::Write;
    if files.is_empty() || files.len() > 2 {
        return Err(other("a collaboration export needs one or two output files"));
    }
    for (path, _) in files {
        validate_path(path)?;
    }
    // Open every destination exclusively before writing any content. Never remove
    // somebody else's file if the second create races or a write fails.
    let mut opened = Vec::new();
    for (path, bytes) in files {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => opened.push((*path, *bytes, file)),
            Err(error) => {
                let mut remaining = Vec::new();
                for (created, _, file) in opened {
                    drop(file);
                    if std::fs::remove_file(created).is_err() {
                        remaining.push(created);
                    }
                }
                return Err(other(format!(
                    "collaboration export failed; cannot create {path}: {error}; incomplete files that could not be removed: {remaining:?}"
                )));
            }
        }
    }
    let result: std::io::Result<()> = (|| {
        for (_, bytes, file) in &mut opened {
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let mut remaining = Vec::new();
        for (created, _, file) in opened {
            drop(file);
            if std::fs::remove_file(created).is_err() {
                remaining.push(created);
            }
        }
        return Err(other(format!("collaboration export failed: {error}; incomplete files that could not be removed: {remaining:?}")));
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn export_native(_session: &mut Session, _params: &Value) -> Result<Value> {
    Err(other("Photoshop copy export requires the platform file service"))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "file.photoshopCompatibilityReport",
            label: "Photoshop Compatibility…",
            menu: &["File"],
            shortcut: None,
            params: "{}",
            enabled: has_doc,
            journal: false,
            run: |s, _| serde_json::to_value(analyze(&s.active().ok_or(EngineError::NoDocument)?.doc)?).map_err(|e| other(e.to_string())),
        },
        CommandSpec {
            id: "file.export.photoshopCopy",
            label: "Photoshop Collaboration Copy…",
            menu: &["File", "Export"],
            shortcut: None,
            params: r#"{"path":"new.psd","masterPath":"new.inkpop.pcraft"? | null | false}"#,
            enabled: crate::file_cmds::native_doc,
            journal: true,
            run: export_native,
        },
        CommandSpec {
            id: "layer.smartFilter.acceptRawRecompute",
            label: "Accept InkPop Camera Raw Recompute",
            menu: &[],
            shortcut: None,
            params: r#"{"layer":id?}"#,
            enabled: has_doc,
            journal: true,
            run: accept,
        },
    ]
}

#[cfg(test)]
#[path = "ps_collaboration_cmds/tests_v84.rs"]
mod tests_v84;
