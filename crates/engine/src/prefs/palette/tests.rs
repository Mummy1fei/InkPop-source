use super::*;
use crate::Session;
use crate::prefs::{Preferences, Theme};
use serde_json::json;

fn colors() -> BTreeMap<ColorSlot, PaletteRgb> {
    [(ColorSlot::Toolbar, PaletteRgb::new([48, 88, 128])), (ColorSlot::Canvas, PaletteRgb::new([220, 228, 236]))].into()
}

fn configuration() -> StudioLightColors {
    StudioLightColors { colors: colors(), presets: vec![ColorPreset::new(" Ocean ", colors()).unwrap()] }
}

#[test]
fn sparse_palette_and_all_slot_ids_round_trip_v68() {
    let empty: StudioLightColors = serde_json::from_str("{}").unwrap();
    assert_eq!(empty, StudioLightColors::default());
    assert!(empty.colors.is_empty());
    let mut all = BTreeMap::new();
    let mut ids = BTreeSet::new();
    for slot in ColorSlot::ALL {
        assert!(ids.insert(slot.id()));
        assert_eq!(serde_json::to_value(slot).unwrap(), json!(slot.id()));
        all.insert(slot, PaletteRgb::new([12, 34, 56]));
    }
    assert_eq!(all.len(), 35);
    let value = StudioLightColors { colors: all, presets: vec![] };
    assert_eq!(serde_json::from_str::<StudioLightColors>(&serde_json::to_string(&value).unwrap()).unwrap(), value);
    let rgb: PaletteRgb = serde_json::from_str("\"#A1b2C3\"").unwrap();
    assert_eq!(rgb.rgb(), [0xa1, 0xb2, 0xc3]);
    assert_eq!(serde_json::to_string(&rgb).unwrap(), "\"#a1b2c3\"");
}

#[test]
fn color_payload_rejects_wrong_types_and_unknown_or_repeated_slots_v68() {
    for input in ["\"ffffff\"", "\"#123\"", "\"#00112233\"", "\"#gg0011\"", "42", "[1,2,3]", "null"] {
        assert!(serde_json::from_str::<PaletteRgb>(input).is_err(), "{input}");
    }
    for input in [
        r##"{"colors":{"unknown":"#123456"}}"##,
        r##"{"colors":{"toolbar":"#123456","toolbar":"#654321"}}"##,
        r##"{"colors":{"toolbar":"white"}}"##,
        r##"{"colors":[]}"##,
        r##"{"presets":null}"##,
        r##"{"other":true}"##,
    ] {
        assert!(serde_json::from_str::<StudioLightColors>(input).is_err(), "{input}");
    }
}

#[test]
fn names_and_saved_library_are_bounded_and_unique_v68() {
    let valid = "字".repeat(MAX_NAME_CHARS);
    assert_eq!(ColorPreset::new(&valid, colors()).unwrap().name, valid);
    assert_eq!(ColorPreset::new("  Ocean  ", colors()).unwrap().name, "Ocean");
    for invalid in [String::new(), " \t ".into(), "a\nb".into(), "字".repeat(MAX_NAME_CHARS + 1), "x".repeat(1024)] {
        assert!(ColorPreset::new(&invalid, colors()).is_err());
        assert!(serde_json::from_value::<ColorPreset>(json!({"name": invalid, "colors": {}})).is_err());
    }
    let valid = StudioLightColors {
        colors: BTreeMap::new(),
        presets: (0..MAX_PRESETS).map(|index| ColorPreset::new(format!("Palette {index}"), colors()).unwrap()).collect(),
    };
    valid.validate().unwrap();
    let decoded: StudioLightColors = serde_json::from_str(&serde_json::to_string(&valid).unwrap()).unwrap();
    assert_eq!(decoded, valid);
    let mut oversized = valid.clone();
    oversized.presets.push(ColorPreset::new("Overflow", colors()).unwrap());
    assert!(oversized.validate().is_err());
    assert!(serde_json::from_str::<StudioLightColors>(&serde_json::to_string(&oversized).unwrap()).is_err());
    for pair in [["Ocean", "ocean"], ["Ocean", " OCEAN "], ["Ähre", "ähre"]] {
        let value = json!({"presets": [{"name": pair[0], "colors": {}}, {"name": pair[1], "colors": {}}]});
        assert!(serde_json::from_value::<StudioLightColors>(value).is_err());
    }
}

#[test]
fn streaming_limit_rejects_unparsed_overflow_item_before_generic_json_tree_v68() {
    let presets: Vec<_> = (0..MAX_PRESETS).map(|index| json!({"name":format!("P{index}"),"colors":{}})).collect();
    let mut input = serde_json::to_string(&presets).unwrap();
    input.pop();
    // Invalid JSON in item 33 must not be decoded; the bounded seed rejects it first.
    input.push_str(",this must not be decoded]");
    let input = format!("{{\"presets\":{input}}}");
    let error = serde_json::from_str::<StudioLightColors>(&input).unwrap_err().to_string();
    assert!(error.contains("at most 32"), "{error}");
    let preferences = format!("{{\"interface\":{{\"studioLightColors\":{input}}}}}");
    let error = validate_preferences_json(&preferences).unwrap_err();
    assert!(error.contains("at most 32"), "{error}");
}

#[test]
fn portable_file_schema_is_strict_bounded_and_separate_from_preferences_v68() {
    let file = ColorPaletteFile::new(" Ocean ", colors()).unwrap();
    let bytes = file.to_json().unwrap();
    let envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(envelope["format"], FILE_FORMAT);
    assert_eq!(envelope["version"], 1);
    assert_eq!(envelope["name"], "Ocean");
    assert_eq!(envelope.as_object().unwrap().len(), 4);
    assert_eq!(ColorPaletteFile::from_json(&bytes).unwrap(), file);
    assert_eq!(file.clone().into_preset(), ColorPreset::new("Ocean", colors()).unwrap());
    for (key, value) in [
        ("format", json!("OtherTheme")),
        ("version", json!(2)),
        ("version", json!("1")),
        ("presets", json!([])),
        ("theme", json!("pro")),
        ("name", json!("")),
        ("colors", json!({"unknown":"#123456"})),
    ] {
        let mut bad = envelope.clone();
        bad[key] = value;
        assert!(ColorPaletteFile::from_json(&serde_json::to_vec(&bad).unwrap()).is_err(), "{bad}");
    }
    let mut oversized = bytes.clone();
    oversized.resize(MAX_FILE_BYTES + 1, b' ');
    assert!(ColorPaletteFile::from_json(&oversized).unwrap_err().contains("128 KiB"));
    let mut trailing = bytes;
    trailing.extend_from_slice(b" {}");
    assert!(ColorPaletteFile::from_json(&trailing).is_err());
    assert!(ColorPaletteFile { name: " ".into(), colors: colors() }.to_json().is_err());
}

#[test]
fn palette_preferences_preserve_legacy_defaults_and_round_trip_v68() {
    let mut session = Session::new();
    session.load_prefs_json(r#"{"interface":{"theme":"studioLight","futureKey":true},"futureSection":{"x":1}}"#).unwrap();
    assert_eq!(session.prefs().interface.studio_light_colors, StudioLightColors::default());
    assert_eq!(session.prefs().interface.theme, Theme::StudioLight);
    session.execute("prefs.set", json!({"path":"interface.studioLightColors","value":configuration()})).unwrap();
    assert_eq!(session.prefs().interface.studio_light_colors.presets[0].name, "Ocean");
    let mut restored = Session::new();
    restored.load_prefs_json(&session.prefs_to_json()).unwrap();
    assert_eq!(restored.prefs(), session.prefs());
}

#[test]
fn invalid_palette_sets_and_loads_are_atomic_at_each_entry_point_v68() {
    let mut session = Session::new();
    session.execute("prefs.set", json!({"path":"interface.studioLightColors","value":configuration()})).unwrap();
    let original = session.prefs_value();
    for request in [
        json!({"path":"interface.studioLightColors.colors.toolbar","value":"#invalid"}),
        json!({"path":"interface.studioLightColors.colors.unknown","value":"#123456"}),
        json!({"path":"interface.studioLightColors","value":{"colors":{"toolbar":"#invalid"}}}),
        json!({"path":"interface","value":{"theme":"classic","studioLightColors":{"presets":[{"name":" ","colors":{}}]}}}),
        json!({"path":"","value":{"interface":{"studioLightColors":{"colors":{"unknown":"#123456"}}}}}),
        json!({"values":{"interface.theme":"classic","interface.studioLightColors.colors.toolbar":"no"}}),
    ] {
        assert!(session.execute("prefs.set", request.clone()).is_err(), "{request}");
        assert_eq!(session.prefs_value(), original, "{request}");
    }
    let mut corrupted = original.clone();
    corrupted["interface"]["studioLightColors"]["colors"]["toolbar"] = json!("#invalid");
    assert!(session.load_prefs_json(&serde_json::to_string(&corrupted).unwrap()).is_err());
    assert_eq!(session.prefs_value(), original);
}

#[test]
fn sparse_slot_set_and_active_reset_keep_library_and_other_preferences_v68() {
    let mut prefs = Preferences::default();
    prefs.interface.studio_light_colors = configuration();
    prefs.interface.theme = Theme::Classic;
    prefs.interface.canvas_custom_color = "#876543".into();
    prefs.set("interface.studioLightColors.colors.menuBar", json!("#ABCDEF")).unwrap();
    assert_eq!(prefs.get("interface.studioLightColors.colors.menuBar"), Some(json!("#abcdef")));
    assert_eq!(prefs.interface.studio_light_colors.colors[&ColorSlot::MenuBar].rgb(), [0xab, 0xcd, 0xef]);
    let library = prefs.interface.studio_light_colors.presets.clone();
    prefs.reset(Some("interface.studioLightColors.colors")).unwrap();
    assert!(prefs.interface.studio_light_colors.colors.is_empty());
    assert_eq!(prefs.interface.studio_light_colors.presets, library);
    assert_eq!(prefs.interface.theme, Theme::Classic);
    assert_eq!(prefs.interface.canvas_custom_color, "#876543");
}
