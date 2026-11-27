//! Bounded, display-only colour overrides for the Studio Light theme.
//!
//! An absent slot means the built-in theme colour. No document colour, profile or
//! image sample is stored here; these are sRGB interface colours only.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub mod builtin;

pub const MAX_PRESETS: usize = 32;
pub const MAX_NAME_CHARS: usize = 64;
pub const MAX_FILE_BYTES: usize = 128 * 1024;
const FILE_FORMAT: &str = "InkPopStudioLightColors";
const FILE_VERSION: u32 = 1;

macro_rules! color_slots {
    ($($variant:ident => $id:literal),+ $(,)?) => {
        /// Stable module/role identifiers. Unknown slots are rejected on import.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub enum ColorSlot { $(#[serde(rename = $id)] $variant),+ }

        impl ColorSlot {
            pub const ALL: [Self; 35] = [$(Self::$variant),+];

            pub const fn id(self) -> &'static str {
                match self { $(Self::$variant => $id),+ }
            }
        }
    };
}

color_slots! {
    TitleBar => "titleBar",
    MenuBar => "menuBar",
    Toolbar => "toolbar",
    OptionsBar => "optionsBar",
    StatusBar => "statusBar",
    Dock => "dock",
    Card => "card",
    CardBorder => "cardBorder",
    PanelTitle => "panelTitle",
    FloatingTitle => "floatingTitle",
    MenuBackground => "menuBackground",
    MenuHover => "menuHover",
    MenuSelection => "menuSelection",
    MenuText => "menuText",
    Field => "field",
    FieldBorder => "fieldBorder",
    Hover => "hover",
    Pressed => "pressed",
    Text => "text",
    TextDim => "textDim",
    TextFaint => "textFaint",
    Icon => "icon",
    Accent => "accent",
    AccentText => "accentText",
    Selection => "selection",
    Separator => "separator",
    ProgressTrack => "progressTrack",
    ProgressFill => "progressFill",
    SliderTrack => "sliderTrack",
    SliderFill => "sliderFill",
    SliderHandle => "sliderHandle",
    ScrollbarTrack => "scrollbarTrack",
    ScrollbarHandle => "scrollbarHandle",
    Canvas => "canvas",
    CanvasDot => "canvasDot",
}

/// One opaque sRGB interface colour, serialized canonically as `#rrggbb`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteRgb([u8; 3]);

impl PaletteRgb {
    pub const fn new(rgb: [u8; 3]) -> Self {
        Self(rgb)
    }

    pub const fn rgb(self) -> [u8; 3] {
        self.0
    }
}

impl Serialize for PaletteRgb {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("#{:02x}{:02x}{:02x}", self.0[0], self.0[1], self.0[2]))
    }
}

impl<'de> Deserialize<'de> for PaletteRgb {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct RgbVisitor;
        impl Visitor<'_> for RgbVisitor {
            type Value = PaletteRgb;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an sRGB colour in #rrggbb notation")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if value.len() != 7 || !value.starts_with('#') {
                    return Err(E::custom("palette colours must use #rrggbb notation"));
                }
                super::parse_hex(value).map(PaletteRgb).ok_or_else(|| E::custom("invalid palette colour"))
            }
        }
        deserializer.deserialize_str(RgbVisitor)
    }
}

pub fn validate_name(name: &str) -> Result<String, String> {
    // Bound before allocating or case-folding; four bytes per Unicode scalar.
    if name.len() > MAX_NAME_CHARS * 4 {
        return Err(format!("palette names must contain 1..{MAX_NAME_CHARS} characters"));
    }
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(format!("palette names must contain 1..{MAX_NAME_CHARS} visible characters"));
    }
    Ok(name.to_string())
}

fn deserialize_name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    struct NameVisitor;
    impl Visitor<'_> for NameVisitor {
        type Value = String;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a nonempty palette name of at most 64 characters")
        }
        fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
            validate_name(value).map_err(E::custom)
        }
        fn visit_string<E: de::Error>(self, value: String) -> Result<String, E> {
            self.visit_str(&value)
        }
    }
    deserializer.deserialize_str(NameVisitor)
}

fn deserialize_colors<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BTreeMap<ColorSlot, PaletteRgb>, D::Error> {
    struct ColorsVisitor;
    impl<'de> Visitor<'de> for ColorsVisitor {
        type Value = BTreeMap<ColorSlot, PaletteRgb>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a map of at most 35 distinct interface colour slots")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut colors = BTreeMap::new();
            while let Some(slot) = map.next_key::<ColorSlot>()? {
                if colors.len() >= ColorSlot::ALL.len() {
                    return Err(de::Error::custom("too many palette colour slots"));
                }
                if colors.contains_key(&slot) {
                    return Err(de::Error::custom("duplicate palette colour slot"));
                }
                colors.insert(slot, map.next_value::<PaletteRgb>()?);
            }
            Ok(colors)
        }
    }
    deserializer.deserialize_map(ColorsVisitor)
}

/// A named saved palette. Names are trimmed and compared case-insensitively.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorPreset {
    #[serde(deserialize_with = "deserialize_name")]
    pub name: String,
    #[serde(deserialize_with = "deserialize_colors")]
    pub colors: BTreeMap<ColorSlot, PaletteRgb>,
}

impl ColorPreset {
    pub fn new(name: impl Into<String>, colors: BTreeMap<ColorSlot, PaletteRgb>) -> Result<Self, String> {
        let name = validate_name(&name.into())?;
        Ok(Self { name, colors })
    }

    pub fn validate(&self) -> Result<(), String> {
        validate_name(&self.name)?;
        Ok(())
    }
}

fn deserialize_presets<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<ColorPreset>, D::Error> {
    struct PresetsVisitor;
    impl<'de> Visitor<'de> for PresetsVisitor {
        type Value = Vec<ColorPreset>;
        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("at most 32 uniquely named interface palettes")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            let mut presets = Vec::new();
            let mut names = BTreeSet::new();
            loop {
                if presets.len() == MAX_PRESETS {
                    // Reject the next element without first decoding a potentially
                    // huge colour map or name belonging to an out-of-budget item.
                    struct RejectOverflow;
                    impl<'de> de::DeserializeSeed<'de> for RejectOverflow {
                        type Value = ();
                        fn deserialize<T: Deserializer<'de>>(self, _: T) -> Result<(), T::Error> {
                            Err(de::Error::custom("at most 32 saved palettes are allowed"))
                        }
                    }
                    let _ = sequence.next_element_seed(RejectOverflow)?;
                    break;
                }
                let Some(preset) = sequence.next_element::<ColorPreset>()? else { break };
                if !names.insert(preset.name.to_lowercase()) {
                    return Err(de::Error::custom("saved palette names must be unique, ignoring case"));
                }
                presets.push(preset);
            }
            Ok(presets)
        }
    }
    deserializer.deserialize_seq(PresetsVisitor)
}

/// Preferences for the Studio Light theme. Empty overrides preserve its defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StudioLightColors {
    #[serde(deserialize_with = "deserialize_colors")]
    pub colors: BTreeMap<ColorSlot, PaletteRgb>,
    #[serde(deserialize_with = "deserialize_presets")]
    pub presets: Vec<ColorPreset>,
}

impl StudioLightColors {
    pub fn validate(&self) -> Result<(), String> {
        if self.presets.len() > MAX_PRESETS {
            return Err("at most 32 saved palettes are allowed".into());
        }
        let mut names = BTreeSet::new();
        for preset in &self.presets {
            let normalized = validate_name(&preset.name)?;
            if !names.insert(normalized.to_lowercase()) {
                return Err("saved palette names must be unique, ignoring case".into());
            }
        }
        Ok(())
    }
}

/// One portable palette. The file schema deliberately excludes the user's entire
/// preferences and preset library, and cannot target another theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorPaletteFile {
    pub name: String,
    pub colors: BTreeMap<ColorSlot, PaletteRgb>,
}

impl ColorPaletteFile {
    pub fn new(name: impl Into<String>, colors: BTreeMap<ColorSlot, PaletteRgb>) -> Result<Self, String> {
        let preset = ColorPreset::new(name, colors)?;
        Ok(Self { name: preset.name, colors: preset.colors })
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_FILE_BYTES {
            return Err("interface palette files must not exceed 128 KiB".into());
        }
        serde_json::from_slice(bytes).map_err(|error| format!("interface palette: {error}"))
    }

    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        validate_name(&self.name)?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|error| format!("interface palette: {error}"))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err("interface palette files must not exceed 128 KiB".into());
        }
        Ok(bytes)
    }

    pub fn into_preset(self) -> ColorPreset {
        ColorPreset { name: self.name, colors: self.colors }
    }
}

impl Serialize for ColorPaletteFile {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let name = validate_name(&self.name).map_err(serde::ser::Error::custom)?;
        let mut state = serializer.serialize_struct("ColorPaletteFile", 4)?;
        state.serialize_field("format", FILE_FORMAT)?;
        state.serialize_field("version", &FILE_VERSION)?;
        state.serialize_field("name", &name)?;
        state.serialize_field("colors", &self.colors)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for ColorPaletteFile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct FileData {
            #[serde(deserialize_with = "deserialize_format")]
            format: (),
            version: u32,
            #[serde(deserialize_with = "deserialize_name")]
            name: String,
            #[serde(deserialize_with = "deserialize_colors")]
            colors: BTreeMap<ColorSlot, PaletteRgb>,
        }
        fn deserialize_format<'de, D: Deserializer<'de>>(deserializer: D) -> Result<(), D::Error> {
            struct FormatVisitor;
            impl Visitor<'_> for FormatVisitor {
                type Value = ();
                fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                    formatter.write_str(FILE_FORMAT)
                }
                fn visit_str<E: de::Error>(self, value: &str) -> Result<(), E> {
                    if value == FILE_FORMAT { Ok(()) } else { Err(E::custom("unsupported interface palette format")) }
                }
            }
            deserializer.deserialize_str(FormatVisitor)
        }
        let FileData { format: (), version, name, colors } = FileData::deserialize(deserializer)?;
        if version != FILE_VERSION {
            return Err(de::Error::custom("unsupported interface palette version"));
        }
        Ok(Self { name, colors })
    }
}

/// Check this newly added payload before the legacy preferences reader builds a
/// generic JSON tree. Unrelated preference fields remain forwards-compatible.
pub(super) fn validate_preferences_json(json: &str) -> Result<(), String> {
    #[derive(Default, Deserialize)]
    #[serde(default, rename_all = "camelCase")]
    struct InterfaceProbe {
        studio_light_colors: StudioLightColors,
    }
    #[derive(Default, Deserialize)]
    #[serde(default)]
    struct PreferencesProbe {
        interface: InterfaceProbe,
    }
    let probe: PreferencesProbe = serde_json::from_str(json).map_err(|error| format!("preferences: {error}"))?;
    probe.interface.studio_light_colors.validate()
}

#[cfg(test)]
mod tests;
