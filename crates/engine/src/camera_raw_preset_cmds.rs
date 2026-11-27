//! Named Camera Raw recipes, persisted through the existing preferences store.
use crate::commands::CommandSpec;
use crate::presets::{always, bad, req_str};
use crate::{Result, Session};
use photocraft_algo::camera_raw::CameraRaw;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAX_PRESETS: usize = 256;
/// Decoded Raster coverage across the complete in-memory recipe library.
/// Separate from the compact on-disk/transport envelope limit below.
pub(crate) const MAX_COLLECTION_RASTER_PIXELS: usize = 256 * 1024 * 1024;
// Leave room inside the control transport's 1 MiB request limit for the RPC envelope.
pub const MAX_PRESET_BYTES: usize = 1_048_576 - 4096;
pub const PRESET_FORMAT: &str = "photocraft-camera-raw-v1";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawPreset {
    pub name: String,
    pub params: CameraRaw,
}
impl RawPreset {
    pub(crate) fn raster_pixels(&self) -> usize {
        use photocraft_algo::camera_raw_masks::MaskShape;
        let shape_bytes = |shape: &MaskShape| match shape {
            MaskShape::Raster { values, .. } => values.len(),
            _ => 0,
        };
        let context_bytes = self.params.color_spaces.source_profile.len().saturating_mul(3) / 4;
        self.params.local_adjustments.iter().fold(self.params.color_lookup.values.len().saturating_mul(4).saturating_add(context_bytes), |total, group| {
            group.components.iter().fold(total.saturating_add(shape_bytes(&group.mask)), |total, component| total.saturating_add(shape_bytes(&component.mask)))
        })
    }
    fn envelope(&self) -> Value {
        json!({"format": PRESET_FORMAT, "name": self.name, "params": self.params})
    }
    /// One canonical compact envelope is budgeted for save, export and import.
    pub fn export_value(&self) -> std::result::Result<Value, String> {
        self.validate()?;
        Ok(self.envelope())
    }
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.name.trim().is_empty() || self.name.len() > 256 || self.name.chars().any(char::is_control) {
            return Err("Preset name must contain 1–256 bytes without control characters".into());
        }
        self.params.validate()?;
        let value = serde_json::to_value(&self.params).map_err(|e| e.to_string())?;
        // Numeric NaN/Infinity serialize as null; reject rather than silently changing a recipe.
        fn has_null(v: &Value) -> bool {
            match v {
                Value::Null => true,
                Value::Array(a) => a.iter().any(has_null),
                Value::Object(o) => o.values().any(has_null),
                _ => false,
            }
        }
        if has_null(&value) {
            return Err("Preset contains invalid non-finite values".into());
        }
        if serde_json::to_vec(&self.envelope()).map_err(|e| e.to_string())?.len() > MAX_PRESET_BYTES {
            return Err("Preset exceeds 1 MiB".into());
        }
        Ok(())
    }
}

fn collection_values(value: Value) -> Option<Vec<Value>> {
    // Do not deserialize even the first compressed recipe if the count already
    // exceeds the library limit. The source Value only contains compact data.
    value.as_array().filter(|items| items.len() <= MAX_PRESETS)?;
    match value {
        Value::Array(items) => Some(items),
        _ => None,
    }
}

pub(crate) fn decode_collection(value: Value) -> Option<Vec<RawPreset>> {
    decode_collection_with_budget(value, MAX_COLLECTION_RASTER_PIXELS)
}

fn decode_collection_with_budget(value: Value, budget: usize) -> Option<Vec<RawPreset>> {
    let values = collection_values(value)?;
    let mut decoded = Vec::with_capacity(values.len());
    let mut raster_pixels = 0usize;
    let mut names = std::collections::HashSet::new();
    for value in values {
        let item: RawPreset = serde_json::from_value(value).ok()?;
        item.validate().ok()?;
        let expanded = item.raster_pixels();
        if expanded > budget.saturating_sub(raster_pixels) || !names.insert(item.name.clone()) {
            return None;
        }
        raster_pixels += expanded;
        decoded.push(item);
    }
    Some(decoded)
}

fn collection_fits(items: &[RawPreset], replacement: Option<usize>, incoming: &RawPreset, budget: usize) -> bool {
    let mut total = incoming.raster_pixels();
    if total > budget {
        return false;
    }
    for (index, item) in items.iter().enumerate() {
        if replacement == Some(index) {
            continue;
        }
        let expanded = item.raster_pixels();
        if expanded > budget.saturating_sub(total) {
            return false;
        }
        total += expanded;
    }
    true
}
fn list(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!(s.presets.camera_raw))
}
fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cameraRaw.presets.get";
    let name = req_str(p, "name", cmd)?;
    let v = s.presets.camera_raw.iter().find(|a| a.name == name).ok_or_else(|| bad(cmd, "Preset not found"))?;
    Ok(json!(v))
}
fn save(s: &mut Session, p: &Value) -> Result<Value> {
    save_with_budget(s, p, MAX_COLLECTION_RASTER_PIXELS)
}

fn save_with_budget(s: &mut Session, p: &Value, raster_budget: usize) -> Result<Value> {
    let cmd = "cameraRaw.presets.save";
    let name = req_str(p, "name", cmd)?.to_string();
    let value = p.get("params").filter(|v| v.is_object()).ok_or_else(|| bad(cmd, "params must be an object"))?;
    let allowed = serde_json::to_value(CameraRaw::default()).map_err(|e| bad(cmd, e.to_string()))?;
    if value.as_object().is_some_and(|o| o.keys().any(|k| allowed.get(k).is_none())) {
        return Err(bad(cmd, "Unknown Camera Raw parameter"));
    }
    let mut params: CameraRaw = serde_json::from_value(value.clone()).map_err(|e| bad(cmd, e.to_string()))?;
    params.pixel_scale = 1.0;
    let item = RawPreset { name, params };
    item.validate().map_err(|e| bad(cmd, e))?;
    let index = s.presets.camera_raw.iter().position(|v| v.name == item.name);
    if index.is_some() && p.get("replace").and_then(Value::as_bool) != Some(true) {
        return Err(bad(cmd, "A preset with this name already exists; choose another name or explicitly replace it"));
    }
    if index.is_none() && s.presets.camera_raw.len() >= MAX_PRESETS {
        return Err(bad(cmd, "Preset limit reached (256)"));
    }
    if !collection_fits(&s.presets.camera_raw, index, &item, raster_budget) {
        return Err(bad(cmd, "Camera Raw preset collection exceeds its decoded mask storage budget (256 MiB)"));
    }
    if let Some(i) = index {
        if let Some(slot) = s.presets.camera_raw.get_mut(i) {
            *slot = item.clone();
        }
    } else {
        s.presets.camera_raw.push(item.clone());
    }
    s.presets_changed();
    Ok(json!(item))
}

#[cfg(test)]
#[path = "camera_raw_preset_collection_tests_v66.rs"]
mod collection_tests_v66;
fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "cameraRaw.presets.delete";
    let name = req_str(p, "name", cmd)?;
    let i = s.presets.camera_raw.iter().position(|a| a.name == name).ok_or_else(|| bad(cmd, "Preset not found"))?;
    s.presets.camera_raw.remove(i);
    s.presets_changed();
    Ok(json!({"deleted":true}))
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "cameraRaw.presets.list",
            label: "Camera Raw Presets",
            menu: &[],
            shortcut: None,
            params: "{}",
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "cameraRaw.presets.get",
            label: "Load Camera Raw Preset",
            menu: &[],
            shortcut: None,
            params: r#"{"name":string}"#,
            enabled: always,
            run: get,
            journal: false,
        },
        CommandSpec {
            id: "cameraRaw.presets.save",
            label: "Save Camera Raw Preset",
            menu: &[],
            shortcut: None,
            params: r#"{"name":string,"params":CameraRaw,"replace":false} normalized masks are included"#,
            enabled: always,
            run: save,
            journal: true,
        },
        CommandSpec {
            id: "cameraRaw.presets.delete",
            label: "Delete Camera Raw Preset",
            menu: &[],
            shortcut: None,
            params: r#"{"name":string}"#,
            enabled: always,
            run: delete,
            journal: true,
        },
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reference_resolution_subject_sky_and_background_survive_presets_and_preferences() {
        use photocraft_algo::camera_raw_masks::{LocalAdjustment, MaskShape};
        let mut params = CameraRaw::default();
        for index in 0..3 {
            params.local_adjustments.push(LocalAdjustment {
                name: format!("Automatic {index}"),
                exposure: index as f32 * 0.2,
                mask: MaskShape::Raster {
                    width: 1024,
                    height: 683,
                    values: (0..1024 * 683).map(|i| ((i % 1024 + i / 1024 + index * 31) % 256) as u8).collect(),
                },
                ..Default::default()
            });
        }
        let preset = RawPreset { name: "Three automatic masks".into(), params };
        let export = preset.export_value().unwrap();
        assert!(serde_json::to_vec(&export).unwrap().len() < MAX_PRESET_BYTES);
        assert!(export["params"]["localAdjustments"][0]["mask"]["values"].is_string());
        let mut session = Session::new();
        session.execute("cameraRaw.presets.save", json!({"name":preset.name,"params":preset.params})).unwrap();
        assert_eq!(session.presets.camera_raw[0], preset);
        let mut reopened = Session::new();
        reopened.load_prefs_json(&session.prefs_to_json()).unwrap();
        assert_eq!(reopened.presets.camera_raw, session.presets.camera_raw);
        assert_eq!(reopened.presets.camera_raw[0].export_value().unwrap(), export);
    }
    #[test]
    fn maximum_envelope_roundtrips_and_over_budget_save_is_atomic() {
        use photocraft_algo::camera_raw_masks::{LocalAdjustment, MaskShape};
        let mut item = RawPreset { name: "edge".into(), params: CameraRaw::default() };
        item.params
            .local_adjustments
            .push(LocalAdjustment { mask: MaskShape::Raster { width: 1024, height: 1024, values: vec![0; 1024 * 1024] }, ..Default::default() });
        // Tune deterministic high-entropy content near the transport boundary.
        // A plain repeated raster now correctly compresses to a few kilobytes.
        let mut seed = 173u32;
        let noise = (0..1024 * 1024)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 24) as u8
            })
            .collect::<Vec<_>>();
        let (mut low, mut high) = (0, noise.len());
        while high - low > 1 {
            let middle = (low + high) / 2;
            let MaskShape::Raster { values, .. } = &mut item.params.local_adjustments[0].mask else { panic!("raster fixture") };
            values[..middle].copy_from_slice(&noise[..middle]);
            values[middle..].fill(0);
            if serde_json::to_vec(&item.envelope()).unwrap().len() <= MAX_PRESET_BYTES - 128 {
                low = middle;
            } else {
                high = middle;
            }
        }
        let MaskShape::Raster { values, .. } = &mut item.params.local_adjustments[0].mask else { panic!("raster fixture") };
        values[..low].copy_from_slice(&noise[..low]);
        values[low..].fill(0);
        let base = serde_json::to_vec(&item.envelope()).unwrap().len();
        let padding = MAX_PRESET_BYTES.checked_sub(base).unwrap();
        assert!(padding <= 252, "reserve enough legal name bytes to reach the exact envelope edge");
        item.name.extend(std::iter::repeat_n('x', padding));
        let bytes = serde_json::to_vec(&item.export_value().unwrap()).unwrap();
        assert_eq!(bytes.len(), MAX_PRESET_BYTES, "exercise the exact entire-envelope boundary");
        let rpc = json!({"id":1,"method":"ui.menu.invoke","params":{"id":"filter.cameraRaw","params":{"ui":{"preset":{"action":"import","data":item.export_value().unwrap()}}}}});
        assert!(serde_json::to_vec(&rpc).unwrap().len() < 1_048_576, "legal exports must fit the public import request wrapper");
        let mut session = Session::new();
        session.execute("cameraRaw.presets.save", json!({"name": item.name, "params": item.params})).unwrap();
        let exported = session.presets.camera_raw[0].export_value().unwrap();
        assert_eq!(serde_json::to_vec(&exported).unwrap(), bytes);
        let decoded: Value = serde_json::from_slice(&bytes).unwrap();
        let mut fresh = Session::new();
        fresh.execute("cameraRaw.presets.save", json!({"name": decoded["name"], "params": decoded["params"]})).unwrap();
        assert_eq!(fresh.presets.camera_raw, session.presets.camera_raw);
        item.name.push('x');
        assert!(item.export_value().is_err());
        let before = session.presets.camera_raw.clone();
        assert!(session.execute("cameraRaw.presets.save", json!({"name": item.name, "params": item.params, "replace": true})).is_err());
        assert_eq!(session.presets.camera_raw, before);
    }

    #[test]
    fn recipes_roundtrip_and_invalid_save_is_atomic() {
        let mut s = Session::new();
        s.execute("cameraRaw.presets.save", json!({"name":"暖色","params":{"exposure":1.0,"temperature":20.0}})).unwrap();
        let saved = s.presets.camera_raw.clone();
        for p in [
            json!({"name":"暖色","params":{}}),
            json!({"name":" ","params":{}}),
            json!({"name":"bad","params":{"unknown":1}}),
            json!({"name":"bad","params":{"localAdjustments":[{"exposure":999}]}}),
        ] {
            assert!(s.execute("cameraRaw.presets.save", p).is_err());
            assert_eq!(s.presets.camera_raw, saved);
        }
        let data = s.prefs_to_json();
        let mut fresh = Session::new();
        fresh.load_prefs_json(&data).unwrap();
        assert_eq!(fresh.presets.camera_raw, saved);
        fresh.execute("cameraRaw.presets.delete", json!({"name":"暖色"})).unwrap();
        assert!(fresh.presets.camera_raw.is_empty());
        assert_eq!(s.presets.camera_raw, saved);
    }
}
