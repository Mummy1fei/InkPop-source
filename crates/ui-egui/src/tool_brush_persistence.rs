//! Independent last-used painting tools. Full tips stay outside ordinary preferences.
//! The snapshot is self-contained, so a late/deleted preset library cannot replace it.

use std::collections::HashSet;

use photocraft_engine::BrushSettings;
use photocraft_engine::paint::{GrayTile, Pattern, TipShape};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{PhotocraftApp, Tool, state::ToolOptions};

#[path = "tool_brush_persistence_bounded.rs"]
mod bounded;

pub const MAX_FILE_BYTES: usize = 128 * 1024 * 1024;
const MAX_SLOTS: usize = 20;
const SAVE_DELAY: f64 = 1.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PaintingOptions {
    sample_all_layers: bool,
    strength: f32,
    protect_detail: bool,
    finger_painting: bool,
    clone_aligned: bool,
    #[serde(deserialize_with = "bounded::short_string")]
    clone_sample: String,
    pencil_auto_erase: bool,
    #[serde(deserialize_with = "bounded::short_string")]
    bg_sampling: String,
    #[serde(deserialize_with = "bounded::short_string")]
    bg_limits: String,
    bg_tolerance: f32,
    bg_protect_fg: bool,
    #[serde(deserialize_with = "bounded::short_string")]
    tone_range: String,
    exposure: f32,
    protect_tones: bool,
    #[serde(deserialize_with = "bounded::short_string")]
    sponge_mode: String,
    vibrance: bool,
}

impl Default for PaintingOptions {
    fn default() -> Self {
        Self::capture(&ToolOptions::default())
    }
}

macro_rules! painting_options {
    ($target:expr, $source:expr, $($field:ident),+ $(,)?) => {
        $( $target.$field = $source.$field; )+
    };
}

impl PaintingOptions {
    fn capture(options: &ToolOptions) -> Self {
        Self {
            sample_all_layers: options.sample_all_layers,
            strength: options.strength,
            protect_detail: options.protect_detail,
            finger_painting: options.finger_painting,
            clone_aligned: options.clone_aligned,
            clone_sample: options.clone_sample.clone(),
            pencil_auto_erase: options.pencil_auto_erase,
            bg_sampling: options.bg_sampling.clone(),
            bg_limits: options.bg_limits.clone(),
            bg_tolerance: options.bg_tolerance,
            bg_protect_fg: options.bg_protect_fg,
            tone_range: options.tone_range.clone(),
            exposure: options.exposure,
            protect_tones: options.protect_tones,
            sponge_mode: options.sponge_mode.clone(),
            vibrance: options.vibrance,
        }
    }

    fn apply(&self, options: &mut ToolOptions) {
        let values = self.clone();
        painting_options!(
            options,
            values,
            sample_all_layers,
            strength,
            protect_detail,
            finger_painting,
            clone_aligned,
            clone_sample,
            pencil_auto_erase,
            bg_sampling,
            bg_limits,
            bg_tolerance,
            bg_protect_fg,
            tone_range,
            exposure,
            protect_tones,
            sponge_mode,
            vibrance
        );
    }

    fn valid(&self) -> bool {
        [self.strength, self.bg_tolerance, self.exposure].into_iter().all(|v| v.is_finite() && (0.0..=100.0).contains(&v))
            && [self.clone_sample.as_str(), self.bg_sampling.as_str(), self.bg_limits.as_str(), self.tone_range.as_str(), self.sponge_mode.as_str()]
                .into_iter()
                .all(|v| v.len() <= 64 && !v.chars().any(char::is_control))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Selection {
    #[serde(deserialize_with = "bounded::preset_name")]
    pub brush: Option<String>,
    #[serde(deserialize_with = "bounded::preset_name")]
    pub tool: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Slot {
    tool: Tool,
    brush: BrushSettings,
    #[serde(default)]
    options: PaintingOptions,
    #[serde(default)]
    selection: Selection,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Snapshot {
    version: u32,
    selected_tool: Tool,
    owner: Option<Tool>,
    #[serde(default, deserialize_with = "crate::tool_group_memory::deserialize_history")]
    tool_history: Vec<Tool>,
    #[serde(deserialize_with = "bounded::slots")]
    slots: Vec<Slot>,
}

#[derive(Default)]
pub(crate) struct Runtime {
    pub(crate) options_owner: Option<Tool>,
    observed: Option<Snapshot>,
    saved: Option<Snapshot>,
    due: Option<f64>,
    error: Option<String>,
}

pub(crate) fn remember_options(app: &mut PhotocraftApp, tool: Tool) {
    if app.brush_persistence.options_owner != Some(tool) {
        return;
    }
    let options = PaintingOptions::capture(&app.ui.tool_options);
    if let Some((_, old)) = app.ui.tool_brush_options.iter_mut().find(|(t, _)| *t == tool) {
        *old = options;
    } else {
        app.ui.tool_brush_options.push((tool, options));
    }
}

pub(crate) fn restore_options(app: &mut PhotocraftApp, tool: Tool) {
    if let Some((_, options)) = app.ui.tool_brush_options.iter().find(|(t, _)| *t == tool) {
        options.apply(&mut app.ui.tool_options);
    }
    let name = app
        .ui
        .tool_brush_selections
        .iter()
        .find(|(t, _)| *t == tool)
        .and_then(|(_, s)| s.tool.as_ref())
        .filter(|name| app.session.presets.tool_presets.iter().any(|p| &p.name == *name))
        .cloned();
    if let Some(name) = name {
        app.ui.presets_ui.selected.insert("toolPresets".into(), name);
    } else {
        app.ui.presets_ui.selected.remove("toolPresets");
    }
    app.brush_persistence.options_owner = Some(tool);
}

pub(crate) fn leave_painting_options(app: &mut PhotocraftApp) {
    if let Some(tool) = app.brush_persistence.options_owner {
        remember_options(app, tool);
    }
    app.brush_persistence.options_owner = None;
}

pub(crate) fn selected(app: &mut PhotocraftApp, id: &str, preset: Option<&str>, reset: bool, result: &Value) {
    let tool = app.ui.tool;
    if !crate::paint_mouse::has_brush_picker(tool) || !matches!(id, "tools.setBrush" | "tool.presets.select") {
        return;
    }
    app.brush_persistence.options_owner = Some(tool);
    let mut selection = app.ui.tool_brush_selections.iter().find(|(t, _)| *t == tool).map(|(_, s)| s.clone()).unwrap_or_default();
    if id == "tool.presets.select" {
        selection.tool = result.get("name").and_then(Value::as_str).map(str::to_owned);
        if result.pointer("/options/brush").is_some() {
            selection.brush = None;
        }
    } else if let Some(name) = preset {
        selection.brush = Some(name.to_owned());
        selection.tool = None;
        app.ui.presets_ui.selected.remove("toolPresets");
    } else if reset {
        selection = Selection::default();
        app.ui.presets_ui.selected.remove("toolPresets");
    }
    if let Some((_, old)) = app.ui.tool_brush_selections.iter_mut().find(|(t, _)| *t == tool) {
        *old = selection;
    } else {
        app.ui.tool_brush_selections.push((tool, selection));
    }
}

fn capture(app: &mut PhotocraftApp) -> Snapshot {
    app.sync_tool_brush();
    app.remember_tool_brush();
    let mut seen = HashSet::new();
    let slots = app
        .ui
        .tool_brushes
        .iter()
        .filter(|(tool, _)| crate::paint_mouse::has_brush_picker(*tool) && seen.insert(*tool))
        .take(MAX_SLOTS)
        .map(|(tool, brush)| Slot {
            tool: *tool,
            brush: brush.clone(),
            options: app.ui.tool_brush_options.iter().find(|(t, _)| t == tool).map(|(_, o)| o.clone()).unwrap_or_default(),
            selection: app.ui.tool_brush_selections.iter().find(|(t, _)| t == tool).map(|(_, s)| s.clone()).unwrap_or_default(),
        })
        .collect();
    Snapshot { version: 1, selected_tool: app.ui.tool, owner: app.brush_tool, tool_history: app.ui.tool_history.clone(), slots }
}

fn tile_bytes(tile: &GrayTile) -> Result<usize, String> {
    let samples = (tile.width as usize).checked_mul(tile.height as usize).ok_or("tool brush bitmap dimensions overflow")?;
    if tile.width == 0 || tile.height == 0 || tile.width > 16384 || tile.height > 16384 || samples > (1 << 26) || samples != tile.data.len() {
        return Err("invalid or oversized tool brush bitmap".into());
    }
    samples.checked_mul(2).ok_or_else(|| "tool brush bitmap size overflow".into())
}

fn validate(snapshot: &Snapshot) -> Result<(), String> {
    if snapshot.version != 1 || snapshot.slots.len() > MAX_SLOTS {
        return Err("unsupported or oversized tool brush state".into());
    }
    let mut history = HashSet::new();
    if snapshot.tool_history.len() > Tool::ALL.len() || snapshot.tool_history.iter().any(|tool| !history.insert(*tool)) {
        return Err("invalid or duplicate tool history".into());
    }
    let mut seen = HashSet::new();
    let mut bytes = 0usize;
    for slot in &snapshot.slots {
        if !crate::paint_mouse::has_brush_picker(slot.tool) || !seen.insert(slot.tool) || !slot.options.valid() {
            return Err("invalid or duplicate tool brush slot".into());
        }
        for name in [&slot.selection.brush, &slot.selection.tool].into_iter().flatten() {
            if name.len() > 1024 || name.chars().any(char::is_control) {
                return Err("invalid tool brush preset name".into());
            }
        }
        let b = &slot.brush;
        let mut light = b.clone();
        light.tip = TipShape::Round;
        light.dual_brush.tip = TipShape::Round;
        light.texture.pattern = Pattern::default();
        fn has_null(value: &Value) -> bool {
            match value {
                Value::Null => true,
                Value::Array(values) => values.iter().any(has_null),
                Value::Object(values) => values.values().any(has_null),
                _ => false,
            }
        }
        if serde_json::to_value(light).map_err(|e| e.to_string()).as_ref().map_or(true, has_null) {
            return Err("non-finite tool brush parameter".into());
        }
        if !b.size.is_finite() || !(0.5..=5000.0).contains(&b.size) || !b.dual_brush.size.is_finite() || !(0.5..=5000.0).contains(&b.dual_brush.size) {
            return Err("invalid tool brush size".into());
        }
        for tip in [&b.tip, &b.dual_brush.tip] {
            if let TipShape::Sampled(tile) = tip {
                bytes = bytes.checked_add(tile_bytes(tile)?).ok_or("tool brush size overflow")?;
            }
        }
        if let Pattern::Tile(tile) = &b.texture.pattern {
            bytes = bytes.checked_add(tile_bytes(tile)?).ok_or("tool brush size overflow")?;
        }
    }
    // Base64 expands by 4/3; reserve one MiB for the small settings and names.
    if bytes > (MAX_FILE_BYTES - (1 << 20)) / 4 * 3 {
        return Err("last-used tool brushes exceed the 128 MiB state limit; previous saved state was kept".into());
    }
    if snapshot.owner.is_some_and(|owner| !seen.contains(&owner))
        || (crate::paint_mouse::has_brush_picker(snapshot.selected_tool) && snapshot.owner != Some(snapshot.selected_tool))
    {
        return Err("inconsistent tool brush owner".into());
    }
    Ok(())
}

fn decode(text: &str) -> Result<Snapshot, String> {
    if text.len() > MAX_FILE_BYTES {
        return Err("last-used tool brush state exceeds 128 MiB".into());
    }
    let snapshot: Snapshot = serde_json::from_str(text).map_err(|e| format!("last-used tool brush state: {e}"))?;
    validate(&snapshot)?;
    Ok(snapshot)
}

pub(crate) fn load(app: &mut PhotocraftApp) {
    if let Some(load) = app.services.load_tool_state.as_mut() {
        match load().and_then(|text| text.map(|t| decode(&t)).transpose()) {
            Ok(Some(snapshot)) => {
                app.ui.tool_brushes = snapshot.slots.iter().map(|s| (s.tool, s.brush.clone())).collect();
                app.ui.tool_brush_options = snapshot.slots.iter().map(|s| (s.tool, s.options.clone())).collect();
                app.ui.tool_brush_selections = snapshot.slots.iter().map(|s| (s.tool, s.selection.clone())).collect();
                app.ui.tool_history = snapshot.tool_history;
                app.ui.tool = snapshot.selected_tool;
                app.brush_tool = snapshot.owner;
                if let Some(owner) = snapshot.owner {
                    if let Some(slot) = snapshot.slots.iter().find(|s| s.tool == owner) {
                        app.session.tools.brush = slot.brush.clone();
                    }
                    restore_options(app, owner);
                }
            }
            Ok(None) => {}
            Err(error) => {
                app.ui.status = format!("Couldn't restore last-used brushes: {error}");
                app.ui.status_error = true;
            }
        }
    }
    let baseline = capture(app);
    app.brush_persistence.observed = Some(baseline.clone());
    app.brush_persistence.saved = Some(baseline);
}

/// Shallow Arc snapshots and cheap equality only; serialize after the edit settles.
pub(crate) fn persist(app: &mut PhotocraftApp, now: f64, force: bool) -> Option<f64> {
    app.services.save_tool_state.as_ref()?;
    let snapshot = capture(app);
    let changed = app.brush_persistence.observed.as_ref() != Some(&snapshot);
    if changed {
        app.brush_persistence.observed = Some(snapshot.clone());
        app.brush_persistence.due = Some(now + SAVE_DELAY);
        app.brush_persistence.error = None;
    }
    if app.brush_persistence.saved.as_ref() == Some(&snapshot) {
        app.brush_persistence.due = None;
        return None;
    }
    let due = app.brush_persistence.due.unwrap_or(now);
    if !force && now < due {
        return Some(due - now);
    }
    let result = validate(&snapshot).and_then(|()| serde_json::to_string(&snapshot).map_err(|e| e.to_string())).and_then(|text| {
        if text.len() > MAX_FILE_BYTES {
            return Err("last-used tool brushes exceed 128 MiB; previous saved state was kept".into());
        }
        app.services.save_tool_state.as_mut().map_or(Ok(()), |save| save(&text))
    });
    match result {
        Ok(()) => {
            app.brush_persistence.saved = Some(snapshot);
            app.brush_persistence.due = None;
            app.brush_persistence.error = None;
            None
        }
        Err(error) => {
            if app.brush_persistence.error.as_ref() != Some(&error) {
                app.ui.status = format!("Couldn't save last-used brushes: {error}");
                app.ui.status_error = true;
            }
            app.brush_persistence.error = Some(error);
            app.brush_persistence.due = Some(now + 5.0);
            Some(5.0)
        }
    }
}

#[cfg(test)]
#[path = "tool_brush_persistence_tests_v73.rs"]
mod tests;

#[cfg(test)]
#[path = "tool_group_persistence_tests_v88.rs"]
mod tool_group_tests_v88;
