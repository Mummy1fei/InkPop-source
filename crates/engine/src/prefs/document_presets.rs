//! Bounded named blank-document settings. No image data or filesystem access.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, de};

pub const MAX_PRESETS: usize = 128;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", try_from = "StoredPreset")]
pub struct DocumentPreset {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub resolution: f32,
    pub unit: String,
    pub resolution_unit: String,
    pub mode: String,
    pub depth: u32,
    pub background: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredPreset {
    name: String,
    width: u32,
    height: u32,
    resolution: f32,
    unit: String,
    resolution_unit: String,
    mode: String,
    depth: u32,
    background: String,
}

impl TryFrom<StoredPreset> for DocumentPreset {
    type Error = String;

    fn try_from(raw: StoredPreset) -> Result<Self, Self::Error> {
        let preset = Self {
            name: raw.name.trim().to_string(),
            width: raw.width,
            height: raw.height,
            resolution: raw.resolution,
            unit: raw.unit,
            resolution_unit: raw.resolution_unit,
            mode: raw.mode,
            depth: raw.depth,
            background: raw.background,
        };
        preset.validate()?;
        Ok(preset)
    }
}

impl DocumentPreset {
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.name.chars().count() > 80 || self.name.chars().any(char::is_control) {
            return Err("preset name must contain 1–80 visible characters".into());
        }
        if !(1..=300_000).contains(&self.width) || !(1..=300_000).contains(&self.height) || !(1.0..=76_200.0).contains(&self.resolution) {
            return Err("invalid document preset dimensions or resolution".into());
        }
        if !matches!(self.unit.as_str(), "px" | "in" | "cm" | "mm" | "pt" | "pica")
            || !matches!(self.resolution_unit.as_str(), "in" | "cm")
            || !matches!(self.mode.as_str(), "rgb" | "gray" | "cmyk" | "lab")
            || !matches!(self.depth, 8 | 16 | 32)
            || !matches!(self.background.as_str(), "white" | "black" | "backgroundColor" | "transparent")
        {
            return Err("invalid document preset options".into());
        }
        Ok(())
    }
}

pub(super) fn deserialize_presets<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<DocumentPreset>, D::Error> {
    struct PresetsVisitor;
    impl<'de> de::Visitor<'de> for PresetsVisitor {
        type Value = Vec<DocumentPreset>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("at most 128 uniquely named document presets")
        }
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            let mut presets = Vec::new();
            let mut names = BTreeSet::new();
            loop {
                if presets.len() == MAX_PRESETS {
                    struct RejectOverflow;
                    impl<'de> de::DeserializeSeed<'de> for RejectOverflow {
                        type Value = ();
                        fn deserialize<T: Deserializer<'de>>(self, _: T) -> Result<(), T::Error> {
                            Err(de::Error::custom("at most 128 document presets are allowed"))
                        }
                    }
                    let _ = sequence.next_element_seed(RejectOverflow)?;
                    break;
                }
                let Some(preset) = sequence.next_element::<DocumentPreset>()? else { break };
                if !names.insert(preset.name.to_lowercase()) {
                    return Err(de::Error::custom("document preset names must be unique, ignoring case"));
                }
                presets.push(preset);
            }
            Ok(presets)
        }
    }
    deserializer.deserialize_seq(PresetsVisitor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn preset() -> serde_json::Value {
        json!({"name":"Print study", "width":600,"height":450,"resolution":300.0,"unit":"in",
            "resolutionUnit":"cm","mode":"lab","depth":16,"background":"transparent"})
    }

    #[test]
    fn document_presets_round_trip_and_old_preferences_default_v71() {
        let mut session = crate::Session::new();
        session.execute("prefs.set", json!({"path":"documentPresets", "value":[preset()]})).unwrap();
        let saved = session.prefs_to_json();
        let mut restored = crate::Session::new();
        restored.load_prefs_json(&saved).unwrap();
        assert_eq!(restored.prefs().document_presets, session.prefs().document_presets);
        assert!(serde_json::from_value::<super::super::Preferences>(json!({})).unwrap().document_presets.is_empty());
    }

    #[test]
    fn document_presets_reject_invalid_duplicate_and_oversized_inputs_atomically_v71() {
        let mut session = crate::Session::new();
        session.execute("prefs.set", json!({"path":"documentPresets", "value":[preset()]})).unwrap();
        let before = session.prefs().clone();
        for (key, value) in [
            ("width", json!(0)),
            ("height", json!(300_001)),
            ("resolution", json!(-1.0)),
            ("unit", json!("unsupported")),
            ("mode", json!("unsupported")),
            ("depth", json!(24)),
            ("background", json!("unsupported")),
            ("name", json!("\n")),
        ] {
            let mut invalid = preset();
            invalid[key] = value;
            assert!(session.execute("prefs.set", json!({"path":"documentPresets","value":[invalid]})).is_err());
            assert_eq!(*session.prefs(), before);
        }
        let mut duplicate = preset();
        duplicate["name"] = json!(" print STUDY ");
        assert!(session.execute("prefs.set", json!({"path":"documentPresets","value":[preset(), duplicate]})).is_err());
        let too_many: Vec<_> = (0..=MAX_PRESETS)
            .map(|i| {
                let mut item = preset();
                item["name"] = json!(format!("Preset {i}"));
                item
            })
            .collect();
        assert!(session.execute("prefs.set", json!({"path":"documentPresets","value":too_many})).is_err());
        assert_eq!(*session.prefs(), before);
    }
}
