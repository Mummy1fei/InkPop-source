//! Lossless native tool-preset exchange. `.pctools` is versioned JSON, not Photoshop TPL.

use std::collections::BTreeSet;

use photocraft_paint::{BrushPreset, BrushSettings};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{ToolPreset, always, bad};
use crate::commands::CommandSpec;
use crate::{Result, Session};

const MAX_TEXT: usize = 128 << 20;
const MAX_PRESETS: usize = 10_000;
const FORMAT: &str = "photocraft-tool-presets";

fn unique_name(taken: &mut BTreeSet<String>, base: &str) -> String {
    let mut name = base.to_string();
    let mut suffix = 2usize;
    while !taken.insert(name.to_lowercase()) {
        let end = format!(" {suffix}");
        let stem: String = base.chars().take(255usize.saturating_sub(end.chars().count())).collect();
        name = stem + &end;
        suffix = suffix.saturating_add(1);
    }
    name
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Library {
    format: String,
    version: u32,
    presets: Vec<ToolPreset>,
}

fn chosen<'a>(s: &'a Session, p: &Value, cmd: &str) -> Result<Vec<&'a ToolPreset>> {
    let Some(names) = p.get("presets") else { return Ok(s.presets.tool_presets.iter().collect()) };
    let names = names.as_array().filter(|n| !n.is_empty() && n.len() <= MAX_PRESETS).ok_or_else(|| bad(cmd, "`presets` must be a non-empty names array"))?;
    let mut seen = BTreeSet::new();
    names
        .iter()
        .map(|n| {
            let name = n.as_str().ok_or_else(|| bad(cmd, "preset names must be strings"))?;
            if !seen.insert(name) {
                return Err(bad(cmd, "duplicate preset name"));
            }
            s.presets.tool_presets.iter().find(|t| t.name == name).ok_or_else(|| bad(cmd, format!("no tool preset `{name}`")))
        })
        .collect()
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "tool.presets.export";
    let presets: Vec<_> = chosen(s, p, CMD)?.into_iter().cloned().collect();
    if presets.len() > MAX_PRESETS {
        return Err(bad(CMD, "too many presets"));
    }
    let count = presets.len();
    let text = serde_json::to_string(&Library { format: FORMAT.into(), version: 1, presets }).map_err(|e| bad(CMD, e.to_string()))?;
    if text.len() > MAX_TEXT {
        return Err(bad(CMD, "tool preset library exceeds 128 MiB"));
    }
    Ok(json!({"text":text,"count":count,"extension":"pctools"}))
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "tool.presets.import";
    let text =
        p.get("text").and_then(Value::as_str).filter(|t| t.len() <= MAX_TEXT).ok_or_else(|| bad(CMD, "give `text` (native .pctools JSON, at most 128 MiB)"))?;
    let replace = match p.get("replace") {
        None => false,
        Some(v) => v.as_bool().ok_or_else(|| bad(CMD, "`replace` must be a boolean"))?,
    };
    let library: Library = serde_json::from_str(text).map_err(|e| bad(CMD, format!("invalid .pctools library: {e}")))?;
    if library.format != FORMAT || library.version != 1 {
        return Err(bad(CMD, "unsupported tool preset library format/version"));
    }
    let kept = if replace { 0 } else { s.presets.tool_presets.len() };
    if library.presets.len().saturating_add(kept) > MAX_PRESETS {
        return Err(bad(CMD, "too many presets (maximum 10,000 including existing presets)"));
    }
    // Validate the complete set before touching the user's current library, especially Replace.
    for t in &library.presets {
        if t.name.trim().is_empty() || t.name.chars().count() > 255 || t.tool.trim().is_empty() || t.tool.chars().count() > 128 || !t.options.is_object() {
            return Err(bad(CMD, "invalid preset name, tool or options"));
        }
        if let Some(b) = t.options.get("brush") {
            if !b.is_object() {
                return Err(bad(CMD, "`brush` must be an object"));
            }
            let brush = crate::brush_cmds::merge_brush(&BrushSettings::default(), b, CMD)?;
            crate::brush_cmds::validate_brush_size(&brush, CMD)?;
        }
    }
    let mut taken: BTreeSet<String> = if replace { BTreeSet::new() } else { s.presets.tool_presets.iter().map(|t| t.name.to_lowercase()).collect() };
    let mut incoming = library.presets;
    for t in &mut incoming {
        t.name = unique_name(&mut taken, &t.name);
    }
    let names: Vec<_> = incoming.iter().map(|t| t.name.clone()).collect();
    if replace {
        s.presets.tool_presets = incoming;
    } else {
        s.presets.tool_presets.extend(incoming);
    }
    s.presets_changed();
    Ok(json!({"count":names.len(),"imported":names,"replaced":replace}))
}

fn convert(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "tool.presets.convertToBrushes";
    let chosen = chosen(s, p, CMD)?;
    let mut names: BTreeSet<String> = s.tools.presets.iter().map(|p| p.name.to_lowercase()).collect();
    let mut added = Vec::new();
    let mut skipped = Vec::new();
    for tool in chosen {
        let Some(patch) = tool.options.get("brush").filter(|b| b.is_object()) else {
            skipped.push(tool.name.clone());
            continue;
        };
        let brush = crate::brush_cmds::merge_brush(&BrushSettings::default(), patch, CMD)?;
        crate::brush_cmds::validate_brush_size(&brush, CMD)?;
        let name = unique_name(&mut names, &tool.name);
        added.push(BrushPreset { name, brush, builtin: false, group: "Converted Tool Presets".into() });
    }
    let imported: Vec<_> = added.iter().map(|b| b.name.clone()).collect();
    if !added.is_empty() {
        s.tools.presets.extend(added);
        s.brush_presets_changed();
    }
    Ok(json!({"count":imported.len(),"imported":imported,"skipped":skipped}))
}

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "tool.presets.export",
            label: "Save Tool Presets…",
            menu: &[],
            shortcut: None,
            params: r#"{"presets":[names]?} → {text,count,extension}; lossless native .pctools JSON (not Photoshop TPL)"#,
            enabled: always,
            run: export,
            journal: false,
        },
        CommandSpec {
            id: "tool.presets.import",
            label: "Load Tool Presets…",
            menu: &[],
            shortcut: None,
            params: r#"{"text":native .pctools JSON,"replace":bool=false} → {count,imported,replaced}; validates before replacing"#,
            enabled: always,
            run: import,
            journal: true,
        },
        CommandSpec {
            id: "tool.presets.convertToBrushes",
            label: "Convert Tool Presets to Brushes",
            menu: &[],
            shortcut: None,
            params: r#"{"presets":[names]?} → {count,imported,skipped}; preserves tool presets, skips non-brush tools"#,
            enabled: always,
            run: convert,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_tool_library_roundtrips_bitmap_color_and_opaque_tool_options() {
        let mut s = Session::new();
        let brush = BrushSettings {
            tip: photocraft_paint::TipShape::Sampled(photocraft_paint::GrayTile::from_f32(2, 2, &[0.0, 0.25, 0.5, 1.0])),
            ..Default::default()
        };
        s.presets.tool_presets = vec![ToolPreset {
            name: "墨笔".into(),
            tool: "mixerBrush".into(),
            options: json!({"brush":brush,"foreground":[0.1,0.2,0.3,1.0],"toolOptions":{"strength":73.0},"extension":{"future":true}}),
        }];
        let before = s.presets.tool_presets.clone();
        let out = export(&mut s, &json!({})).unwrap();
        let mut target = Session::new();
        import(&mut target, &json!({"text":out["text"],"replace":true})).unwrap();
        assert_eq!(target.presets.tool_presets, before);
        let old_brush = target.tools.brush.clone();
        convert(&mut target, &json!({"presets":["墨笔"]})).unwrap();
        assert_eq!(target.tools.presets.last().unwrap().brush, brush);
        assert_eq!(target.tools.brush, old_brush);
        assert_eq!(target.presets.tool_presets, before);
    }

    #[test]
    fn replace_is_atomic_for_bad_file_version_options_and_brush() {
        let mut s = Session::new();
        let before = s.presets.tool_presets.clone();
        for text in [
            "{".into(),
            json!({"format":FORMAT,"version":99,"presets":[]}).to_string(),
            json!({"format":FORMAT,"version":1,"presets":[{"name":"bad","tool":"brush","options":null}]}).to_string(),
            json!({"format":FORMAT,"version":1,"presets":[{"name":"bad","tool":"brush","options":{"brush":{"size":1e30}}}]}).to_string(),
        ] {
            assert!(import(&mut s, &json!({"text":text,"replace":true})).is_err());
            assert_eq!(s.presets.tool_presets, before);
        }
        assert!(export(&mut s, &json!({"presets":["missing"]})).is_err());
        assert!(convert(&mut s, &json!({"presets":[null]})).is_err());
    }

    #[test]
    fn append_unique_names_and_convert_skip_nonbrush() {
        let mut s = Session::new();
        let out = export(&mut s, &json!({})).unwrap();
        let count = s.presets.tool_presets.len();
        import(&mut s, &json!({"text":out["text"]})).unwrap();
        assert_eq!(s.presets.tool_presets.len(), count * 2);
        let converted = convert(&mut s, &json!({})).unwrap();
        assert_eq!(converted["count"], 8);
        assert_eq!(converted["skipped"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn append_count_and_collision_names_stay_bounded() {
        let mut s = Session::new();
        let entry = ToolPreset { name: "字".repeat(255), tool: "brush".into(), options: json!({}) };
        s.presets.tool_presets = vec![entry.clone(); MAX_PRESETS];
        let text = serde_json::to_string(&Library { format: FORMAT.into(), version: 1, presets: vec![entry.clone()] }).unwrap();
        assert!(import(&mut s, &json!({"text":text})).is_err());
        assert_eq!(s.presets.tool_presets.len(), MAX_PRESETS);
        s.presets.tool_presets = vec![entry];
        import(&mut s, &json!({"text":text})).unwrap();
        let renamed = &s.presets.tool_presets[1].name;
        assert_eq!(renamed.chars().count(), 255);
        assert!(renamed.ends_with(" 2"));
        let exported = export(&mut s, &json!({})).unwrap();
        import(&mut s, &json!({"text":exported["text"],"replace":true})).unwrap();
    }
}
