use super::*;
use crate::prefs::palette::{ColorPaletteFile, ColorPreset, StudioLightColors};
use std::collections::BTreeSet;

fn luminance(value: PaletteRgb) -> f64 {
    let channels = value.rgb().map(|v| {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    });
    channels[0] * 0.2126 + channels[1] * 0.7152 + channels[2] * 0.0722
}

fn contrast(a: PaletteRgb, b: PaletteRgb) -> f64 {
    let a = luminance(a);
    let b = luminance(b);
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[test]
fn four_builtin_palettes_are_complete_distinct_and_round_trip_as_single_files_v69() {
    let ids: BTreeSet<_> = BUILTIN_PALETTES.iter().map(|p| p.id).collect();
    let names: BTreeSet<_> = BUILTIN_PALETTES.iter().map(|p| p.name).collect();
    assert_eq!(ids.len(), 4);
    assert_eq!(names.len(), 4);
    let mut distinct = BTreeSet::new();
    for builtin in BUILTIN_PALETTES {
        let colors = builtin.colors();
        assert!(builtin.matches(&colors));
        for (&slot, &color) in &colors {
            assert_eq!(builtin.color(slot), Some(color));
        }
        assert_eq!(colors.keys().copied().collect::<BTreeSet<_>>(), ColorSlot::ALL.into_iter().collect());
        assert_eq!(colors[&ColorSlot::TitleBar], colors[&ColorSlot::MenuBar]);
        let file = ColorPaletteFile::new(builtin.name, colors.clone()).unwrap();
        let bytes = file.to_json().unwrap();
        let decoded = ColorPaletteFile::from_json(&bytes).unwrap().into_preset();
        assert_eq!(decoded, ColorPreset::new(builtin.name, colors).unwrap());
        assert!(distinct.insert(bytes));
    }
}

#[test]
fn builtin_menu_content_and_active_controls_have_readable_role_contrast_v69() {
    use ColorSlot::*;
    for builtin in BUILTIN_PALETTES {
        let c = builtin.colors();
        for background in [MenuBackground, MenuHover, MenuSelection] {
            let ratio = contrast(c[&MenuText], c[&background]);
            assert!(ratio >= 7.0, "{} menu {background:?}: {ratio}", builtin.id);
        }
        for background in [TitleBar, Toolbar, OptionsBar, StatusBar, Dock, Card, PanelTitle, FloatingTitle, Field, Hover, Pressed, Selection] {
            let ratio = contrast(c[&Text], c[&background]);
            assert!(ratio >= 4.5, "{} main text on {background:?}: {ratio}", builtin.id);
        }
        for (foreground, background, minimum) in [
            (AccentText, Accent, 4.5),
            (TextDim, Card, 4.5),
            (TextFaint, Card, 3.0),
            (ScrollbarHandle, ScrollbarTrack, 3.0),
            (SliderHandle, SliderTrack, 3.0),
            (FieldBorder, Field, 2.0),
        ] {
            let ratio = contrast(c[&foreground], c[&background]);
            assert!(ratio >= minimum, "{} {foreground:?}/{background:?}: {ratio}", builtin.id);
        }
    }
}

#[test]
fn builtin_drafts_are_fresh_and_do_not_seed_or_mutate_private_saved_libraries_v69() {
    let initial = StudioLightColors::default();
    assert!(initial.presets.is_empty() && initial.colors.is_empty());
    let mut state = StudioLightColors {
        colors: BTreeMap::new(),
        presets: vec![ColorPreset::new("Private saved palette", [(ColorSlot::Text, PaletteRgb::new([12, 34, 56]))].into()).unwrap()],
    };
    let saved = state.presets.clone();
    for builtin in BUILTIN_PALETTES {
        state.colors = builtin.colors();
        state.colors.insert(ColorSlot::Accent, PaletteRgb::new([1, 2, 3]));
        assert!(!builtin.matches(&state.colors));
        assert_ne!(state.colors, builtin.colors(), "changing a draft must not edit the starter");
        assert_eq!(state.presets, saved);
        state.validate().unwrap();
    }
}
