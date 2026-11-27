//! Named, grouped tool colours. Encoded sRGB floats, matching `tools.setColors`.
//! Library changes are preferences, never document pixels/history.
use super::{Group, always, bad, call, req_str};
use crate::{Result, Session, commands::CommandSpec};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const CMD: &str = "swatch.presets.edit";
const MAX_GROUPS: usize = 128;
const MAX_COLORS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Swatch {
    pub name: String,
    pub color: [f32; 4],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SwatchLibrary {
    pub groups: Vec<Group<Swatch>>,
    pub current: String,
}

impl Default for SwatchLibrary {
    fn default() -> Self {
        let colors = [
            ("Black", [0, 0, 0]),
            ("White", [255, 255, 255]),
            ("Gray", [128, 128, 128]),
            ("Red", [230, 40, 40]),
            ("Orange", [245, 120, 30]),
            ("Yellow", [250, 210, 30]),
            ("Green", [40, 200, 80]),
            ("Teal", [30, 200, 170]),
            ("Cyan", [30, 170, 230]),
            ("Blue", [40, 100, 230]),
            ("Violet", [120, 70, 220]),
            ("Magenta", [210, 50, 180]),
        ];
        Self {
            groups: vec![Group::new(
                "Basic",
                colors
                    .into_iter()
                    .map(|(name, [r, g, b])| Swatch { name: name.into(), color: [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0] })
                    .collect(),
            )],
            current: "Basic".into(),
        }
    }
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
}
fn name_param<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    let name = req_str(p, key, CMD)?;
    if !valid_name(name) {
        return Err(bad(CMD, "names must contain 1..128 bytes and no control characters"));
    }
    Ok(name)
}
fn valid_color(color: &[f32; 4]) -> bool {
    color.iter().all(|c| c.is_finite() && (0.0..=1.0).contains(c))
}
fn color_param(p: &Value, fallback: [f32; 4]) -> Result<[f32; 4]> {
    let Some(value) = p.get("color") else {
        return if valid_color(&fallback) { Ok(fallback) } else { Err(bad(CMD, "invalid current tool color")) };
    };
    let color = if let Some(hex) = value.as_str() {
        let hex = hex.strip_prefix('#').unwrap_or(hex);
        if !matches!(hex.len(), 6 | 8) {
            return Err(bad(CMD, "color must be #rrggbb or #rrggbbaa"));
        }
        let mut color = [1.0; 4];
        for (i, channel) in color.iter_mut().enumerate().take(hex.len() / 2) {
            let part = hex.get(i * 2..i * 2 + 2).ok_or_else(|| bad(CMD, "invalid hex color"))?;
            *channel = u8::from_str_radix(part, 16).map_err(|_| bad(CMD, "invalid hex color"))? as f32 / 255.0;
        }
        color
    } else {
        serde_json::from_value::<[f32; 4]>(value.clone()).map_err(|_| bad(CMD, "color must be four normalized RGBA numbers"))?
    };
    if !valid_color(&color) {
        return Err(bad(CMD, "RGBA components must be finite and within 0..1"));
    }
    Ok(color)
}

impl SwatchLibrary {
    pub fn validate(&self) -> Result<()> {
        if self.groups.is_empty() || self.groups.len() > MAX_GROUPS {
            return Err(bad(CMD, "library must contain 1..128 groups"));
        }
        let mut groups = BTreeSet::new();
        let mut count = 0usize;
        for group in &self.groups {
            if !valid_name(&group.name) || !groups.insert(&group.name) {
                return Err(bad(CMD, "group names must be valid and unique"));
            }
            let mut names = BTreeSet::new();
            count = count.saturating_add(group.items.len());
            if count > MAX_COLORS {
                return Err(bad(CMD, "library supports at most 4096 colors"));
            }
            for swatch in &group.items {
                if !valid_name(&swatch.name) || !names.insert(&swatch.name) || !valid_color(&swatch.color) {
                    return Err(bad(CMD, "invalid or duplicate color in group"));
                }
            }
        }
        if !groups.contains(&self.current) {
            return Err(bad(CMD, "current group does not exist"));
        }
        Ok(())
    }
}

fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    let action = req_str(p, "action", CMD)?;
    // Work on a bounded copy; any validation error leaves the previous library intact.
    let mut library = s.presets.swatches.clone();
    let group_name = p.get("group").map(|_| name_param(p, "group")).transpose()?.unwrap_or(&library.current).to_owned();
    match action {
        "newGroup" => {
            let name = name_param(p, "name")?.to_owned();
            if library.groups.len() >= MAX_GROUPS {
                return Err(bad(CMD, "maximum group count reached"));
            }
            library.groups.push(Group::new(&name, vec![]));
            library.current = name;
        }
        "import" => {
            let value = p.get("library").ok_or_else(|| bad(CMD, "missing library"))?;
            // The JSON transport is already bounded; reject counts before cloning/deserializing.
            let groups = value.get("groups").and_then(Value::as_array).ok_or_else(|| bad(CMD, "missing groups"))?;
            if groups.len() > MAX_GROUPS || groups.iter().map(|g| g.get("items").and_then(Value::as_array).map_or(0, Vec::len)).sum::<usize>() > MAX_COLORS {
                return Err(bad(CMD, "import exceeds library limits"));
            }
            let imported: SwatchLibrary = serde_json::from_value(value.clone()).map_err(|e| bad(CMD, e.to_string()))?;
            imported.validate()?;
            // Append groups atomically; duplicate names are errors, never silent replacement.
            library.current = imported.current;
            library.groups.extend(imported.groups);
        }
        "move" => {
            let name = name_param(p, "preset")?;
            let target = name_param(p, "to")?;
            let source = library.groups.iter().position(|g| g.name == group_name).ok_or_else(|| bad(CMD, "group not found"))?;
            let destination = library.groups.iter().position(|g| g.name == target).ok_or_else(|| bad(CMD, "destination group not found"))?;
            let from = library.groups.get_mut(source).ok_or_else(|| bad(CMD, "group not found"))?;
            let index = from.items.iter().position(|item| item.name == name).ok_or_else(|| bad(CMD, "color not found"))?;
            let swatch = from.items.remove(index);
            library.groups.get_mut(destination).ok_or_else(|| bad(CMD, "destination group not found"))?.items.push(swatch);
        }
        _ => {
            let gi = library.groups.iter().position(|g| g.name == group_name).ok_or_else(|| bad(CMD, "group not found"))?;
            if action == "deleteGroup" {
                if library.groups.len() == 1 {
                    return Err(bad(CMD, "keep at least one group"));
                }
                library.groups.remove(gi);
                if library.current == group_name {
                    library.current = library.groups.first().map(|g| g.name.clone()).unwrap_or_default();
                }
            } else {
                let group = library.groups.get_mut(gi).ok_or_else(|| bad(CMD, "group not found"))?;
                match action {
                    "selectGroup" => library.current = group.name.clone(),
                    "renameGroup" => {
                        group.name = name_param(p, "name")?.to_owned();
                        if library.current == group_name {
                            library.current = group.name.clone();
                        }
                    }
                    "add" => {
                        group.items.push(Swatch { name: name_param(p, "name")?.into(), color: color_param(p, s.tools.foreground)? });
                    }
                    "update" | "delete" => {
                        let name = name_param(p, "preset")?;
                        let index = group.items.iter().position(|item| item.name == name).ok_or_else(|| bad(CMD, "color not found"))?;
                        if action == "delete" {
                            group.items.remove(index);
                        } else {
                            let swatch = group.items.get_mut(index).ok_or_else(|| bad(CMD, "color not found"))?;
                            if p.get("name").is_some() {
                                swatch.name = name_param(p, "name")?.into();
                            }
                            swatch.color = color_param(p, swatch.color)?;
                        }
                    }
                    _ => return Err(bad(CMD, "unknown swatch action")),
                }
            }
        }
    }
    library.validate()?;
    s.presets.swatches = library;
    s.presets_changed();
    Ok(json!({"current":s.presets.swatches.current}))
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let group = p.get("group").map(|_| name_param(p, "group")).transpose()?.unwrap_or(&s.presets.swatches.current);
    let name = name_param(p, "preset")?;
    let color = s
        .presets
        .swatches
        .groups
        .iter()
        .find(|g| g.name == group)
        .and_then(|g| g.items.iter().find(|v| v.name == name))
        .ok_or_else(|| bad("swatch.presets.select", "color not found"))?
        .color;
    let target =
        match p.get("target").map(|v| v.as_str().ok_or_else(|| bad("swatch.presets.select", "target must be a string"))).transpose()?.unwrap_or("foreground") {
            "foreground" => "foreground",
            "background" => "background",
            _ => return Err(bad("swatch.presets.select", "target must be foreground or background")),
        };
    call(s, "tools.setColors", json!({target:color}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "swatch.presets.list",
            label: "List Swatch Groups",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: |s, _| Ok(json!(s.presets.swatches)),
            journal: false,
        },
        CommandSpec {
            id: CMD,
            label: "Edit Swatches",
            menu: &[],
            shortcut: None,
            params: r##"{"action":"add|update|delete|move|newGroup|renameGroup|deleteGroup|selectGroup|import","group":name?,"preset":name?,"name":name?,"to":group?,"color":"#rrggbb"|[r,g,b,a]?,"library":{"groups":[{"name":str,"items":[{"name":str,"color":[r,g,b,a]}]}],"current":str}?} import appends groups; duplicate names rejected; changes persist in preferences"##,
            enabled: always,
            run: edit,
            journal: true,
        },
        CommandSpec {
            id: "swatch.presets.select",
            label: "Select Swatch",
            menu: &[],
            shortcut: None,
            params: r#"{"group":name?,"preset":name,"target":"foreground|background"?}"#,
            enabled: always,
            run: select,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn groups_edit_colors_select_and_round_trip_preferences() {
        let mut s = Session::new();
        s.execute(CMD, json!({"action":"newGroup","name":"My palette"})).unwrap();
        s.execute(CMD, json!({"action":"add","name":"Sky","color":[0.2,0.4,0.8,1.0]})).unwrap();
        s.execute(CMD, json!({"action":"update","preset":"Sky","name":"Sea","color":"#0080ff"})).unwrap();
        s.execute("swatch.presets.select", json!({"preset":"Sea","target":"background"})).unwrap();
        assert_eq!(s.tools.background, [0.0, 128.0 / 255.0, 1.0, 1.0]);
        s.execute(CMD, json!({"action":"renameGroup","name":"Ocean"})).unwrap();
        let mut restored = Session::new();
        restored.load_prefs_json(&s.prefs_to_json()).unwrap();
        assert_eq!(s.presets.swatches, restored.presets.swatches);
        s.execute(CMD, json!({"action":"delete","preset":"Sea"})).unwrap();
        s.execute(CMD, json!({"action":"deleteGroup"})).unwrap();
        assert_eq!(s.presets.swatches.current, "Basic");
    }
    #[test]
    fn invalid_edits_are_atomic_and_bad_saved_library_keeps_defaults() {
        let mut s = Session::new();
        let before = s.presets.swatches.clone();
        for p in [
            json!({}),
            json!({"action":"newGroup","name":"Basic"}),
            json!({"action":"deleteGroup"}),
            json!({"action":"add","name":"Bad","color":[-1,0,0,1]}),
            json!({"action":"add","name":"Bad","color":"#你好"}),
            json!({"action":"add","name":"Bad","color":[0,0,0]}),
            json!({"action":"update","preset":"missing"}),
            json!({"action":"renameGroup","name":""}),
            json!({"action":"selectGroup","group":"missing"}),
            json!({"action":"import","library":before}),
        ] {
            assert!(s.execute(CMD, p).is_err());
            assert_eq!(s.presets.swatches, before);
        }
        s.load_presets_json(json!({"swatches":{"groups":[],"current":"missing"}}));
        assert_eq!(s.presets.swatches, before);
    }
    #[test]
    fn import_appends_groups_and_retains_precision() {
        let mut s = Session::new();
        let color = [0.1234567, 0.2345678, 0.3456789, 0.456789];
        s.execute(CMD, json!({"action":"import","library":{"groups":[{"name":"Imported","items":[{"name":"Float","color":color}]}],"current":"Imported"}}))
            .unwrap();
        assert_eq!(s.presets.swatches.groups.len(), 2);
        s.execute("swatch.presets.select", json!({"preset":"Float"})).unwrap();
        assert_eq!(s.tools.foreground, color);
        assert!(s.execute("swatch.presets.select", json!({"preset":"Float","target":"invalid"})).is_err());
        s.execute(CMD, json!({"action":"move","preset":"Float","to":"Basic"})).unwrap();
        assert!(s.presets.swatches.groups.last().unwrap().items.is_empty());
        assert_eq!(s.presets.swatches.groups.first().unwrap().items.last().unwrap().color, color);
    }
}
