use super::*;

fn open_builtin_choices(h: &mut Harness<'_, Fixture>) {
    h.query_all_by_role(egui::accesskit::Role::ComboBox).next().unwrap().click();
    h.run_steps(3);
    assert!(egui::Popup::is_any_open(&h.ctx));
}

#[test]
fn builtin_palette_dropdown_replaces_only_draft_colors_with_full_user_library_v69() {
    let mut state = fixture("studioLight");
    let presets = (0..photocraft_engine::prefs::palette::MAX_PRESETS)
        .map(|index| ColorPreset::new(format!("Personal palette {index}"), sample()).unwrap())
        .collect::<Vec<_>>();
    store_config(&mut state.interface, &StudioLightColors { colors: sample(), presets: presets.clone() }).unwrap();
    state.fields.insert(SELECTED.into(), json!("Personal palette 0"));
    let original = state.app.session.prefs().to_json();
    let mut h = harness(state);
    for palette in &BUILTIN_PALETTES {
        open_builtin_choices(&mut h);
        h.get_by_label(palette.name).click();
        h.run_steps(3);
        assert_eq!(config(&h).colors, palette.colors());
        assert_eq!(config(&h).presets, presets, "built-ins neither consume nor replace the 32 user-library slots");
        assert_eq!(h.state().fields[NAME], palette.name);
        assert_eq!(h.state().fields[SELECTED], "");
        assert_eq!(
            h.query_all_by_role(egui::accesskit::Role::ComboBox).next().unwrap().value().as_deref(),
            Some(palette.name),
            "the collapsed selector identifies the currently matching built-in"
        );
        assert_eq!(h.state().fields[MESSAGE], "", "the current name and colour strip supply feedback without a new message row");
        assert!(h.get_by_label("Delete Saved").accesskit_node().is_disabled(), "built-in choices cannot be deleted as user presets");
        assert_eq!(h.state().app.session.prefs().to_json(), original, "one-click choices are still unapplied drafts");
    }
    h.get_by_label("Restore Default Colors").click();
    h.run_steps(3);
    assert!(config(&h).colors.is_empty());
    assert_eq!(config(&h).presets, presets);
}

#[test]
fn builtin_palette_dropdown_is_disabled_outside_studio_light_v69() {
    let mut h = harness(fixture("pro"));
    assert!(h.query_all_by_role(egui::accesskit::Role::ComboBox).next().unwrap().accesskit_node().is_disabled());
    h.get_by_label("Switch to Studio (Light)").click();
    h.run_steps(3);
    assert!(!h.query_all_by_role(egui::accesskit::Role::ComboBox).next().unwrap().accesskit_node().is_disabled());
    open_builtin_choices(&mut h);
    for palette in &BUILTIN_PALETTES {
        assert!(!h.get_by_label(palette.name).accesskit_node().is_disabled());
    }
}

#[test]
fn merged_title_menu_hex_edits_and_default_clear_legacy_menu_override_v69() {
    use egui::accesskit::Role;
    let mut state = fixture("studioLight");
    let original = StudioLightColors {
        colors: [(ColorSlot::MenuBar, PaletteRgb::new([18, 52, 86])), (ColorSlot::Canvas, PaletteRgb::new([88, 99, 110]))].into(),
        presets: vec![ColorPreset::new("Personal", sample()).unwrap()],
    };
    store_config(&mut state.interface, &original).unwrap();
    let mut h = harness(state);
    h.get_by_label("Title and Menu Bar");
    assert!(h.query_by_label("Main Menu Bar").is_none());
    assert_eq!(editor_rgb(&config(&h), ColorSlot::TitleBar), [18, 52, 86]);
    h.query_all_by_role(Role::TextInput).find(|node| node.value().as_deref() == Some("#123456")).unwrap().click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.event(egui::Event::Text("#abcdef".into()));
    h.run_steps(3);
    assert_eq!(config(&h).colors.get(&ColorSlot::TitleBar), Some(&PaletteRgb::new([171, 205, 239])));
    assert!(!config(&h).colors.contains_key(&ColorSlot::MenuBar));
    assert_eq!(config(&h).presets, original.presets);
    h.query_all_by_label("Default").find(|node| !node.accesskit_node().is_disabled()).unwrap().click();
    h.run_steps(3);
    assert!(!config(&h).colors.contains_key(&ColorSlot::TitleBar));
    assert!(!config(&h).colors.contains_key(&ColorSlot::MenuBar));
    assert_eq!(config(&h).colors.get(&ColorSlot::Canvas), original.colors.get(&ColorSlot::Canvas));
    assert_eq!(config(&h).presets, original.presets);

    // Reset must also work when loading an old menu-only colour without editing.
    set_colors(&mut h, [(ColorSlot::MenuBar, PaletteRgb::new([18, 52, 86]))].into());
    h.run_steps(3);
    h.query_all_by_label("Default").find(|node| !node.accesskit_node().is_disabled()).unwrap().click();
    h.run_steps(3);
    assert!(config(&h).colors.is_empty());
}

fn painted_rects<'a>(shape: &'a egui::Shape, result: &mut Vec<&'a egui::epaint::RectShape>) {
    match shape {
        egui::Shape::Rect(rect) => result.push(rect),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                painted_rects(shape, result);
            }
        }
        _ => {}
    }
}

#[test]
fn color_wells_paint_the_same_visible_border_as_neighboring_hex_inputs_v69() {
    use egui::accesskit::Role;
    let mut h = harness(fixture("studioLight"));
    let wells: Vec<_> = h.query_all_by_role(Role::ColorWell).map(|node| node.rect()).collect();
    let hexes: Vec<_> = h
        .query_all_by_role(Role::TextInput)
        .filter(|node| node.value().is_some_and(|value| value.len() == 7 && value.starts_with('#')))
        .map(|node| node.rect())
        .collect();
    assert_eq!(wells.len(), 4, "title and menu occupy one independent row");
    assert_eq!(wells.len(), hexes.len());
    let mut rects = Vec::new();
    for shape in &h.output().shapes {
        painted_rects(&shape.shape, &mut rects);
    }
    let expected = h.ctx.global_style().visuals.widgets.inactive.bg_stroke;
    assert!(expected.width >= 1.0 && expected.color.a() > 0);
    for (well, hex) in wells.iter().zip(hexes) {
        let well_border = rects.iter().rev().find(|shape| shape.rect == *well && shape.stroke == expected);
        assert!(well_border.is_some(), "the actual ColorWell must have the field outline, not its old background-coloured stroke");
        let hex_border = rects.iter().find(|shape| shape.rect == hex && shape.stroke == expected);
        assert!(hex_border.is_some(), "the adjacent real Hex input must use the same stroke");
    }
    h.query_all_by_role(Role::ColorWell).next().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().app.ui.dialogs.len(), 1);
    let picker = h.state().app.ui.dialogs[0].id;
    assert!(crate::color_picker_ui::owns(&h.state().app.ui.dialogs[0].fields));
    assert!(!egui::Popup::is_any_open(&h.ctx), "the swatch uses the independent dialog");
    assert!(config(&h).colors.is_empty(), "opening the real picker does not commit a new colour");
    h.state_mut().app.ui.dialog_mut(picker).unwrap().fields.insert("color".into(), json!("#123456"));
    h.state_mut().app.ui.close_dialog(picker);
    h.run_steps(3);
    assert!(config(&h).colors.is_empty(), "Cancel preserves every palette override");
    h.query_all_by_role(Role::ColorWell).next().unwrap().click();
    h.run_steps(3);
    let picker = h.state().app.ui.dialogs.last().unwrap().id;
    h.state_mut().app.ui.dialog_mut(picker).unwrap().fields.insert("color".into(), json!("#123456"));
    crate::dialogs::confirm(&mut h.state_mut().app, picker).unwrap();
    h.run_steps(3);
    assert!(h.state().app.ui.dialogs.is_empty());
    assert_eq!(config(&h).colors, [(ColorSlot::TitleBar, PaletteRgb::new([18, 52, 86]))].into(), "OK changes only the clicked module");
}

struct PreferencesFixture {
    app: PhotocraftApp,
    fields: Map<String, Value>,
}

fn assert_all_first_group_wells_visible(h: &Harness<'_, PreferencesFixture>) -> Vec<egui::Rect> {
    let wells: Vec<_> = h.query_all_by_role(egui::accesskit::Role::ColorWell).map(|node| node.rect()).collect();
    assert_eq!(wells.len(), 4, "all four independent top-bar/tool rows must be present");
    for well in &wells {
        let painted_inside_clip = h.output().shapes.iter().any(|shape| {
            let mut rects = Vec::new();
            painted_rects(&shape.shape, &mut rects);
            shape.clip_rect.expand(0.51).contains_rect(*well) && rects.iter().any(|rect| rect.rect == *well && rect.stroke.width >= 1.0)
        });
        assert!(painted_inside_clip, "the complete colour control {well:?} must actually paint inside the Preferences scroll clip");
    }
    wells
}

#[test]
fn compact_builtin_row_keeps_all_four_colors_visible_in_real_preferences_620_height_v69() {
    for code in ["en", "zh-hans"] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        app.run("prefs.set", json!({"values":{"interface.theme":"studioLight","interface.language":code}})).unwrap();
        let id = crate::prefs_ui::open_preferences(&mut app, "interface");
        let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
        fields.insert("__interfacePaletteTab".into(), json!(true));
        let mut h = Harness::builder().with_size(egui::vec2(812.0, 620.0)).with_max_steps(64).build_ui_state(
            |ui, state: &mut PreferencesFixture| {
                if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                // Keep the original 620pt small-client regression while the
                // production right column stays 540pt and its scroll budget adapts.
                let previous = crate::i18n::current();
                let language = crate::i18n::Lang::from_pref(
                    state.fields.get("values").and_then(|value| value.pointer("/interface/language")).and_then(Value::as_str).unwrap_or("en"),
                );
                crate::i18n::set_current(language);
                crate::color_picker_ui::service(&mut state.app, ui.ctx());
                crate::prefs_ui::body(&mut state.app, ui, &mut state.fields);
                crate::color_picker_ui::service(&mut state.app, ui.ctx());
                crate::i18n::set_current(previous);
            },
            PreferencesFixture { app, fields },
        );
        PhotocraftApp::setup_context(&h.ctx, ThemeKind::StudioLight);
        h.run_steps(8);
        let before = assert_all_first_group_wells_visible(&h);
        let language = crate::i18n::Lang::from_code(code).unwrap();
        h.get_by_label(crate::i18n::tr(language, "Built-in Palettes"));
        h.get_by_label(crate::i18n::tr(language, "Title and Menu Bar"));
        for palette in &BUILTIN_PALETTES {
            h.query_all_by_role(egui::accesskit::Role::ComboBox).next().unwrap().click();
            h.run_steps(3);
            h.get_by_label(crate::i18n::tr(language, palette.name)).click();
            h.run_steps(3);
            assert_eq!(h.state().fields[MESSAGE], "");
            let after = assert_all_first_group_wells_visible(&h);
            assert_eq!(after, before, "choosing {code} {} must not insert a success-message row", palette.id);
        }
    }
}

#[test]
#[ignore = "writes final v69 built-in palette editor pages for actual root visual review"]
fn palette_editor_gpu_visual_v69() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v69/palette-editor-gpu");
    assert!(!out.exists(), "preserve previous visual evidence");
    std::fs::create_dir_all(&out).unwrap();
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 1000.0)).with_max_steps(64).wgpu().build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::StudioLight);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        app.run("prefs.set", json!({"values":{"interface.theme":"studioLight","interface.language":"zh-hans"}})).unwrap();
        let id = crate::prefs_ui::open_preferences(&mut app, "interface");
        app.ui.dialog_mut(id).unwrap().fields.insert("__interfacePaletteTab".into(), json!(true));
        app
    });
    h.run_steps(8);
    let id = h.state().ui.dialogs.last().unwrap().id;
    h.render().unwrap().save(out.join("default-merged-top-and-well-borders.png")).unwrap();
    let language = crate::i18n::Lang::from_code("zh-hans").unwrap();
    for palette in &BUILTIN_PALETTES {
        let label = h.get_by_label(crate::i18n::tr(language, "Built-in Palettes")).rect();
        h.query_all_by_role(egui::accesskit::Role::ComboBox)
            .find(|node| node.rect().left() >= label.right() - 1.0 && (node.rect().center().y - label.center().y).abs() <= 2.0)
            .unwrap()
            .click();
        h.run_steps(3);
        h.get_by_label(crate::i18n::tr(language, palette.name)).click();
        h.run_steps(3);
        crate::prefs_ui::apply(h.state_mut(), id).unwrap();
        h.run_steps(4);
        h.render().unwrap().save(out.join(format!("{}.png", palette.id))).unwrap();
    }
    h.state_mut().ui.dialog_mut(id).unwrap().fields.get_mut("values").unwrap()["interface"]["theme"] = json!("pro");
    h.run_steps(3);
    h.render().unwrap().save(out.join("other-theme-builtins-disabled.png")).unwrap();
}
