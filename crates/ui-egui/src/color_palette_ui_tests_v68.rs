use super::*;
use crate::{Services, theme::ThemeKind};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

struct Fixture {
    app: PhotocraftApp,
    interface: Map<String, Value>,
    fields: Map<String, Value>,
}

fn fixture(theme: &str) -> Fixture {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    let mut interface = app.session.prefs().get("interface").unwrap().as_object().unwrap().clone();
    interface.insert("theme".into(), json!(theme));
    Fixture { app, interface, fields: Map::new() }
}

fn harness(state: Fixture) -> Harness<'static, Fixture> {
    let mut h = Harness::builder().with_size(egui::vec2(720.0, 620.0)).with_max_steps(64).build_ui_state(
        |ui, state: &mut Fixture| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let old = crate::i18n::current();
            crate::i18n::set_current(crate::i18n::Lang::EN);
            crate::color_picker_ui::service(&mut state.app, ui.ctx());
            editor(&mut state.app, ui, &mut state.interface, &mut state.fields);
            crate::color_picker_ui::service(&mut state.app, ui.ctx());
            crate::i18n::set_current(old);
        },
        state,
    );
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::StudioLight);
    h.run_steps(6);
    h
}

fn config(h: &Harness<'_, Fixture>) -> StudioLightColors {
    read_config(&h.state().interface).unwrap()
}
fn set_colors(h: &mut Harness<'_, Fixture>, colors: BTreeMap<ColorSlot, PaletteRgb>) {
    let mut value = config(h);
    value.colors = colors;
    store_config(&mut h.state_mut().interface, &value).unwrap();
}
fn sample() -> BTreeMap<ColorSlot, PaletteRgb> {
    [(ColorSlot::TitleBar, PaletteRgb::new([23, 65, 109]))].into()
}

#[test]
fn palette_module_groups_cover_each_existing_slot_once_v68() {
    let slots: Vec<_> = GROUPS.iter().flat_map(|group| group.colors.iter().map(|&(slot, _)| slot)).collect();
    assert_eq!(GROUPS.len(), 8);
    assert_eq!(slots.len(), ColorSlot::ALL.len() - 1);
    let independent: BTreeSet<_> = ColorSlot::ALL.into_iter().filter(|slot| *slot != ColorSlot::MenuBar).collect();
    assert_eq!(slots.into_iter().collect::<BTreeSet<_>>(), independent);
}

#[path = "color_palette_ui_tests_v69.rs"]
mod v69;

#[test]
fn palette_ui_save_load_replace_and_reset_use_the_draft_v68() {
    let mut state = fixture("studioLight");
    state.fields.insert(NAME.into(), json!("Ocean"));
    let initial = StudioLightColors { colors: sample(), ..Default::default() };
    store_config(&mut state.interface, &initial).unwrap();
    let original_preferences = state.app.session.prefs().to_json();
    let mut h = harness(state);
    h.get_by_label("Save New Palette").click();
    h.run_steps(3);
    assert_eq!(config(&h).presets.len(), 1);
    assert_eq!(config(&h).presets[0].colors, sample());
    let changed: BTreeMap<_, _> = [(ColorSlot::TitleBar, PaletteRgb::new([200, 180, 160]))].into();
    set_colors(&mut h, changed.clone());
    h.run_steps(2);
    h.get_by_label("Load Palette").click();
    h.run_steps(3);
    assert_eq!(config(&h).colors, sample());
    set_colors(&mut h, changed.clone());
    h.run_steps(2);
    h.get_by_label("Replace Selected").click();
    h.run_steps(3);
    assert_eq!(config(&h).presets[0].colors, changed);
    // Duplicate names never silently replace a saved palette.
    let before = config(&h);
    h.get_by_label("Save New Palette").click();
    h.run_steps(3);
    assert_eq!(config(&h), before);
    assert_eq!(h.state().fields[ERROR], true);
    h.get_by_label("Restore Default Colors").click();
    h.run_steps(3);
    assert!(config(&h).colors.is_empty());
    assert_eq!(config(&h).presets, before.presets);
    assert_eq!(h.state().app.session.prefs().to_json(), original_preferences, "all button actions still belong to the unapplied Preferences draft");
}

#[test]
fn palette_ui_other_themes_are_disabled_until_draft_switch_v68() {
    let mut state = fixture("studio");
    state.fields.insert(NAME.into(), json!("Ocean"));
    let original = state.app.session.prefs().to_json();
    let mut h = harness(state);
    assert!(h.get_by_label("Save New Palette").accesskit_node().is_disabled());
    assert!(h.get_by_label("Restore Default Colors").accesskit_node().is_disabled());
    h.get_by_label("Switch to Studio (Light)").click();
    h.run_steps(3);
    assert_eq!(h.state().interface["theme"], "studioLight");
    assert!(!h.get_by_label("Save New Palette").accesskit_node().is_disabled());
    assert_eq!(h.state().app.session.prefs().to_json(), original, "switching the editor theme is also a draft change");
}

#[test]
fn palette_real_preferences_apply_then_cancel_keep_only_applied_colors_v68() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    let original = app.session.prefs().to_json();
    let id = crate::prefs_ui::open_preferences(&mut app, "interface");
    {
        let dialog = app.ui.dialog_mut(id).unwrap();
        let interface = dialog.fields.get_mut("values").unwrap().get_mut("interface").unwrap().as_object_mut().unwrap();
        interface.insert("theme".into(), json!("studioLight"));
        store_config(interface, &StudioLightColors { colors: sample(), presets: vec![ColorPreset::new("Ocean", sample()).unwrap()] }).unwrap();
    }
    assert_eq!(app.session.prefs().to_json(), original);
    crate::prefs_ui::apply(&mut app, id).unwrap();
    assert_eq!(app.session.prefs().interface.studio_light_colors.colors, sample());
    app.ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap()["interface"]["studioLightColors"]["colors"] = json!({"titleBar":"#ff0000"});
    app.ui.close_dialog(id);
    assert_eq!(app.session.prefs().interface.studio_light_colors.colors, sample());
    assert_eq!(app.session.prefs().interface.studio_light_colors.presets.len(), 1);
    let encoded = app.session.prefs().to_json();
    let parsed: photocraft_engine::prefs::Preferences = serde_json::from_value(encoded).unwrap();
    assert_eq!(parsed.interface.studio_light_colors, app.session.prefs().interface.studio_light_colors);
}

#[test]
fn palette_file_services_use_chosen_path_and_only_one_palette_v68() {
    let mut state = fixture("studioLight");
    let original_preferences = state.app.session.prefs().to_json();
    let mut config = StudioLightColors { colors: sample(), presets: vec![ColorPreset::new("Private local library", BTreeMap::new()).unwrap()] };
    let saved = Rc::new(RefCell::new(None));
    let captured = saved.clone();
    state.app.services.pick_save_colors = Some(Box::new(|suggested| {
        assert_eq!(suggested, "InkPop-Ocean-colors.json");
        Some("chosen-folder/chosen-name.json".into())
    }));
    state.app.services.write = Some(Box::new(move |path, bytes| {
        *captured.borrow_mut() = Some((path.to_string(), bytes.to_vec()));
        Ok(())
    }));
    assert!(export_file(&mut state.app, &config, "Ocean").unwrap());
    let (path, bytes) = saved.borrow().clone().unwrap();
    assert_eq!(path, "chosen-folder/chosen-name.json");
    let file = ColorPaletteFile::from_json(&bytes).unwrap();
    assert_eq!(file.colors, sample());
    let data: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(data.get("presets").is_none() && data.get("preferences").is_none());
    config.colors.clear();
    state.app.services.pick_open_colors = Some(Box::new(move || Some(("any-location.json".into(), Ok(bytes.clone())))));
    assert_eq!(import_file(&mut state.app, &mut config).unwrap().as_deref(), Some("Ocean"));
    assert_eq!(config.colors, sample());
    assert_eq!(config.presets.len(), 1, "an imported file never replaces the local preset library");
    assert_eq!(state.app.session.prefs().to_json(), original_preferences);
}

#[test]
fn palette_file_cancel_and_errors_preserve_config_v68() {
    let mut state = fixture("studioLight");
    let mut config = StudioLightColors { colors: sample(), presets: vec![] };
    let before = config.clone();
    state.app.services.pick_open_colors = Some(Box::new(|| None));
    assert_eq!(import_file(&mut state.app, &mut config).unwrap(), None);
    assert_eq!(config, before);
    for bytes in
        [b"{bad}".to_vec(), br##"{"format":"InkPopStudioLightColors","version":1,"name":"Oops","colors":{"canvas":"#ffffff"},"preferences":{}}"##.to_vec()]
    {
        state.app.services.pick_open_colors = Some(Box::new(move || Some(("bad.json".into(), Ok(bytes.clone())))));
        assert!(import_file(&mut state.app, &mut config).is_err());
        assert_eq!(config, before);
    }
    state.app.services.pick_open_colors = Some(Box::new(|| Some(("failed.json".into(), Err("read failure".into())))));
    assert!(import_file(&mut state.app, &mut config).is_err());
    assert_eq!(config, before);
    state.app.services.pick_save_colors = Some(Box::new(|_| None));
    assert!(!export_file(&mut state.app, &config, "Ocean").unwrap());
    state.app.services.pick_save_colors = Some(Box::new(|_| Some("somewhere.json".into())));
    state.app.services.write = Some(Box::new(|_, _| Err("disk full".into())));
    assert_eq!(export_file(&mut state.app, &config, "Ocean").unwrap_err(), "disk full");
    assert_eq!(config, before);
}

#[test]
fn palette_single_default_removes_only_that_override_and_preserves_library_v68() {
    let mut state = fixture("studioLight");
    let mut value = StudioLightColors { colors: sample(), presets: vec![ColorPreset::new("Ocean", sample()).unwrap()] };
    value.colors.insert(ColorSlot::Canvas, PaletteRgb::new([40, 50, 60]));
    store_config(&mut state.interface, &value).unwrap();
    let mut h = harness(state);
    let first_enabled = h.query_all_by_label("Default").find(|node| !node.accesskit_node().is_disabled()).unwrap();
    first_enabled.click();
    h.run_steps(3);
    let edited = config(&h);
    assert!(!edited.colors.contains_key(&ColorSlot::TitleBar));
    assert_eq!(edited.colors.get(&ColorSlot::Canvas), value.colors.get(&ColorSlot::Canvas));
    assert_eq!(edited.presets, value.presets);
}

#[test]
#[ignore = "writes actual Preferences Module Colors pages; root must inspect the PNGs"]
fn palette_editor_gpu_visual_v68() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v68/palette-editor-gpu");
    assert!(!out.exists(), "preserve prior visual evidence");
    std::fs::create_dir_all(&out).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 900.0)).with_max_steps(64).wgpu().build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::StudioLight);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        app.run("prefs.set", json!({"values":{"interface.theme":"studioLight","interface.language":"zh-hans"}})).unwrap();
        let id = crate::prefs_ui::open_preferences(&mut app, "interface");
        let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
        fields.insert("__interfacePaletteTab".into(), json!(true));
        fields.insert(NAME.into(), json!("海盐配色"));
        app
    });
    h.run_steps(8);
    let id = h.state().ui.dialogs.last().unwrap().id;
    for category in 0..GROUPS.len() {
        h.state_mut().ui.dialog_mut(id).unwrap().fields.insert(CATEGORY.into(), json!(category));
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("studioLight-module-{category}.png"))).unwrap();
    }
    h.state_mut().ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap()["interface"]["theme"] = json!("studio");
    h.run_steps(4);
    h.render().unwrap().save(out.join("other-theme-disabled.png")).unwrap();
}

#[test]
fn palette_editor_shows_unmultiplied_default_hues_without_creating_overrides_v68() {
    use egui::accesskit::Role;
    for (slot, category) in [(ColorSlot::Selection, 5), (ColorSlot::MenuSelection, 2)] {
        let mut state = fixture("studioLight");
        state.fields.insert(CATEGORY.into(), json!(category));
        let before = read_config(&state.interface).unwrap();
        let resolved = crate::theme::palette::resolved_color(ThemeKind::StudioLight, &before, slot);
        assert!(resolved.a() > 0 && resolved.a() < 255, "fixture must exercise a semi-transparent default");
        let [r, g, b, _] = resolved.to_srgba_unmultiplied();
        let expected = format!("#{r:02x}{g:02x}{b:02x}");
        let premultiplied = format!("#{:02x}{:02x}{:02x}", resolved.r(), resolved.g(), resolved.b());
        assert_ne!(expected, premultiplied, "test must detect the former dark premultiplied value");
        assert_eq!(editor_rgb(&before, slot), [r, g, b]);
        let mut h = harness(state);
        assert!(
            h.query_all_by_role(Role::TextInput).any(|node| node.value().as_deref() == Some(expected.as_str())),
            "the real Hex editor must show {expected}"
        );
        assert!(
            !h.query_all_by_role(Role::TextInput).any(|node| node.value().as_deref() == Some(premultiplied.as_str())),
            "the premultiplied dark value must not leak into the Hex editor"
        );
        assert_eq!(config(&h), before, "viewing the swatch must not turn a default into an opaque override");
        assert_eq!(
            crate::theme::palette::resolved_color(ThemeKind::StudioLight, &config(&h), slot),
            resolved,
            "the default alpha and original color remain unchanged"
        );
        let explicit = PaletteRgb::new([51, 128, 203]);
        set_colors(&mut h, [(slot, explicit)].into());
        h.run_steps(3);
        assert!(h.query_all_by_role(Role::TextInput).any(|node| node.value().as_deref() == Some("#3380cb")), "explicit RGB stays byte exact");
        assert_eq!(editor_rgb(&config(&h), slot), explicit.rgb());
        assert_eq!(config(&h).colors.get(&slot), Some(&explicit));
    }
}

fn clipped_editor_text<'a>(shape: &'a egui::Shape, clip: egui::Rect, output: &mut Vec<(egui::Rect, &'a egui::epaint::TextShape)>) {
    match shape {
        egui::Shape::Text(text) => output.push((clip, text)),
        egui::Shape::Vec(shapes) => {
            for child in shapes {
                clipped_editor_text(child, clip, output);
            }
        }
        _ => {}
    }
}

#[test]
fn palette_hex_columns_paint_all_seven_characters_inside_their_clip_v68() {
    use egui::accesskit::Role;
    for code in ["en", "zh-hans"] {
        let language = crate::i18n::Lang::from_code(code).unwrap();
        for (category, group) in GROUPS.iter().enumerate() {
            let mut state = fixture("studioLight");
            state.fields.insert(CATEGORY.into(), json!(category));
            let before = read_config(&state.interface).unwrap();
            let mut h = Harness::builder().with_size(egui::vec2(580.0, 700.0)).with_max_steps(64).build_ui_state(
                move |ui, state: &mut Fixture| {
                    if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                        return;
                    }
                    let old = crate::i18n::current();
                    crate::i18n::set_current(language);
                    // Match the actual Preferences right-hand content width, rather
                    // than a roomy stand-alone fixture that can hide grid clipping.
                    ui.set_width(540.0);
                    state.fields.insert("__testContentRight".into(), json!(ui.max_rect().right()));
                    crate::color_picker_ui::service(&mut state.app, ui.ctx());
                    editor(&mut state.app, ui, &mut state.interface, &mut state.fields);
                    crate::color_picker_ui::service(&mut state.app, ui.ctx());
                    crate::i18n::set_current(old);
                },
                state,
            );
            PhotocraftApp::setup_context(&h.ctx, ThemeKind::StudioLight);
            h.run_steps(8);
            let right = h.state().fields["__testContentRight"].as_f64().unwrap() as f32;
            let inputs: Vec<_> = h
                .query_all_by_role(Role::TextInput)
                .filter_map(|node| node.value().filter(|value| value.len() == 7 && value.starts_with('#')).map(|value| (value, node.rect())))
                .collect();
            assert_eq!(inputs.len(), group.colors.len(), "{code} group {category}: every color row has a real Hex editor");
            let mut texts = Vec::new();
            for shape in &h.output().shapes {
                clipped_editor_text(&shape.shape, shape.clip_rect, &mut texts);
            }
            for (value, rect) in inputs {
                assert!(rect.width() >= 72.0, "{code} group {category}: {value} input collapsed to {}pt", rect.width());
                assert!(rect.right() <= right + 1.0, "Hex column overflowed the 540pt Preferences content");
                let (clip, text) = texts
                    .iter()
                    .copied()
                    .find(|(_, text)| text.galley.job.text == value && rect.contains(text.pos + text.galley.size() / 2.0))
                    .unwrap_or_else(|| panic!("missing painted Hex value {value} in {rect:?}"));
                assert!(!text.galley.elided, "Hex text must never be elided");
                let glyphs: String = text.galley.rows.iter().flat_map(|row| row.glyphs.iter().map(|glyph| glyph.chr)).collect();
                assert_eq!(glyphs, value, "the actual Galley must contain all #rrggbb glyphs");
                let logical_bounds = egui::Rect::from_min_size(text.pos, text.galley.size());
                let ink_bounds = text.galley.mesh_bounds.translate(text.pos.to_vec2());
                assert!(
                    clip.expand(0.51).contains_rect(logical_bounds),
                    "{code} group {category}: {value} logical text {logical_bounds:?} clipped by {clip:?}"
                );
                assert!(clip.expand(0.51).contains_rect(ink_bounds), "{code} group {category}: actual Hex glyph ink must remain visible");
            }
            let defaults: Vec<_> = h.query_all_by_label(crate::i18n::tr(language, "Default")).map(|node| node.rect()).collect();
            assert_eq!(defaults.len(), group.colors.len());
            assert!(defaults.iter().all(|rect| rect.right() <= right + 1.0), "Default buttons must fit beside the wider Hex column");
            let wells: Vec<_> = h.query_all_by_role(Role::ColorWell).map(|node| node.rect()).collect();
            assert_eq!(wells.len(), group.colors.len());
            assert!(wells.iter().all(|rect| rect.right() <= right + 1.0));
            assert_eq!(config(&h), before, "geometry and paint must not create palette overrides");
            // The local allocation must not consume or disable the neighboring
            // color button: its real production independent picker still opens normally.
            if category == 0 {
                h.query_all_by_role(Role::ColorWell).next().unwrap().click();
                h.run_steps(3);
                assert_eq!(h.state().app.ui.dialogs.len(), 1, "the swatch opens one independent picker");
                assert!(crate::color_picker_ui::owns(&h.state().app.ui.dialogs[0].fields));
                assert!(!egui::Popup::is_any_open(&h.ctx), "the new picker is not an embedded popup");
                assert_eq!(config(&h), before);
            }
        }
    }
}
