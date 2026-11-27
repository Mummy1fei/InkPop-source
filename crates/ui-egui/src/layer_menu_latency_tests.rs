use super::*;

#[test]
fn layer_context_prewarm_requires_an_idle_visible_native_document_root() {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    assert!(!can_warm_layer_context(&app, &ctx), "no empty-startup window allocation");
    app.run("file.new", json!({"width":16,"height":16})).unwrap();
    assert!(can_warm_layer_context(&app, &ctx));
    ctx.set_embed_viewports(true);
    assert!(!can_warm_layer_context(&app, &ctx));
    ctx.set_embed_viewports(false);
    app.ui.open_dialog(crate::state::DialogKind::About, Default::default());
    assert!(!can_warm_layer_context(&app, &ctx), "a settings/modal UI owns the focus");
    app.ui.dialogs.clear();
    let active = egui::Id::new("photocraft-native-app-active");
    ctx.data_mut(|data| data.insert_temp(active, false));
    assert!(!can_warm_layer_context(&app, &ctx), "prewarm cannot activate another application's hidden owner");
    ctx.data_mut(|data| data.insert_temp(active, true));
    assert!(can_warm_layer_context(&app, &ctx));
    let mut input = egui::RawInput::default();
    input.viewports.get_mut(&egui::ViewportId::ROOT).unwrap().minimized = Some(true);
    let mut output = ctx.run_ui(input, |ui| assert!(!can_warm_layer_context(&app, ui.ctx()), "minimizing must not restore a popup"));
    output.textures_delta.clear();
    let source = egui::ViewportId::from_hash_of("detached-layers-warm-guard");
    let mut input = egui::RawInput { viewport_id: source, ..Default::default() };
    input.viewports.entry(source).or_default();
    let mut output = ctx.run_ui(input, |ui| assert!(!can_warm_layer_context(&app, ui.ctx()), "only ROOT declares the cached host"));
    output.textures_delta.clear();
    let mut output = ctx.run_ui(Default::default(), |ui| {
        assert!(can_warm_layer_context(&app, ui.ctx()));
        app.run("layer.new.layer", json!({})).unwrap();
        assert!(crate::discard_ui::intercept(&mut app, "file.close", &json!({})));
        assert!(!can_warm_layer_context(&app, ui.ctx()), "a pending close confirmation must not prewarm");
    });
    output.textures_delta.clear();
}

#[test]
fn native_layout_reuses_measurements_but_invalidates_for_rows_font_and_density() {
    let ctx = egui::Context::default();
    let mut larger_entries = None;
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let layer = Layer::group("Group", vec![]);
        let entries = visible_entries(&layer, false, false);
        let first = native_layout(ui.ctx(), entries.clone());
        let cached = native_layout(ui.ctx(), entries.clone());
        assert!(std::sync::Arc::ptr_eq(&first.entries, &cached.entries));
        let changed = native_layout(ui.ctx(), visible_entries(&layer, true, false));
        assert!(!std::sync::Arc::ptr_eq(&first.entries, &changed.entries));
        let mut style = (*ui.ctx().style_of(ui.ctx().theme())).clone();
        style.text_styles.insert(egui::TextStyle::Button, egui::FontId::proportional(22.0));
        ui.ctx().set_style_of(ui.ctx().theme(), style);
        let larger = native_layout(ui.ctx(), entries.clone());
        assert_eq!(larger.font.size, 22.0);
        assert!(!std::sync::Arc::ptr_eq(&changed.entries, &larger.entries));
        assert!(larger.size.x > first.size.x);
        larger_entries = Some(larger.entries.clone());
        ui.ctx().set_pixels_per_point(2.0);
    });
    output.textures_delta.clear();
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let entries = larger_entries.as_ref().unwrap();
        let denser = native_layout(ui.ctx(), entries.to_vec());
        assert_eq!(denser.pixels_per_point, 2.0);
        assert!(!std::sync::Arc::ptr_eq(entries, &denser.entries));
    });
    output.textures_delta.clear();
}

#[test]
fn native_layout_invalidates_when_interface_font_tweak_changes_without_font_id_or_dpi_change() {
    use photocraft_engine::prefs::UiFontSize;

    fn measure(ctx: &egui::Context) -> NativeLayout {
        let mut measured = None;
        // Interface sizing applies through a font plugin and may need the next pass.
        for _ in 0..3 {
            let mut output = ctx.run_ui(Default::default(), |ui| {
                measured =
                    Some(native_layout(ui.ctx(), vec![Some(("This deliberately wide layer menu label exercises interface font sizing", "layer.delete"))]));
            });
            output.textures_delta.clear();
        }
        measured.unwrap()
    }

    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, crate::theme::ThemeKind::Pro);
    crate::theme::set_ui_font_size(&ctx, UiFontSize::Tiny);
    let tiny = measure(&ctx);
    crate::theme::set_ui_font_size(&ctx, UiFontSize::Large);
    let large = measure(&ctx);
    assert_eq!(tiny.font, large.font, "interface font tweaks must not depend on FontId changes");
    assert_eq!(tiny.pixels_per_point, large.pixels_per_point);
    assert!(large.font_row_height > tiny.font_row_height);
    assert!(!std::sync::Arc::ptr_eq(&tiny.entries, &large.entries), "old font widths must leave the cache");
    assert!(large.size.x > tiny.size.x, "the host width must grow with the shaped label");
    assert!(std::sync::Arc::ptr_eq(&large.entries, &measure(&ctx).entries), "unchanged metrics keep the cache warm");
}

#[test]
fn releasing_the_button_that_opened_a_menu_cannot_activate_its_first_item() {
    use egui_kittest::{Harness, kittest::Queryable};
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":16,"height":16})).unwrap();
    session.execute("layer.new.layer", json!({"name":"Enabled context target"})).unwrap();
    let app = crate::PhotocraftApp::new(session, Default::default());
    let document = app.session.active().unwrap();
    let layer = document.doc.layer(document.active_layer.unwrap()).unwrap().clone();
    assert!(crate::menus::is_enabled(&app, "layer.layerStyle.blendingOptions"), "the first item must be actionable for the release regression");
    let mut harness = Harness::builder().with_size(egui::vec2(400.0, 1200.0)).build_ui_state(
        |ui, actions| {
            contents(&app, ui, &layer, false, actions, true);
        },
        Vec::<(String, Value)>::new(),
    );
    let first = harness.get_by_label("Blending Options…").rect().center();
    harness.event(egui::Event::PointerMoved(first));
    harness.event(egui::Event::PointerButton { pos: first, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
    harness.step();
    assert!(harness.state().is_empty());
    harness.event(egui::Event::PointerButton { pos: first, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    harness.step();
    assert!(harness.state().is_empty(), "a stray release in the new viewport never invokes a command");
    harness.get_by_label("Blending Options…").click();
    harness.run_steps(3);
    assert_eq!(harness.state().last(), Some(&("layer.layerStyle.blendingOptions".into(), Value::Null)));
}

/// Reproduces the replaced CPU preparation without opening a window. This measures metadata
/// extraction, filtering and sizing, not OS scheduling, presentation or physical pen latency.
#[test]
#[ignore = "release-only CPU preparation benchmark; run serially with --nocapture"]
fn layer_menu_preparation_cpu_benchmark() {
    use std::{hint::black_box, time::Instant};
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(Default::default(), |ui| {
        for child_count in [0, 1200] {
            let layer = Layer::group(
                "Large group",
                (0..child_count).map(|index| Layer::raster(format!("Child {index}"), photocraft_color::PixelFormat::RGBA8)).collect(),
            );
            let mut old = Vec::new();
            let mut current = Vec::new();
            for iteration in 0..102 {
                let start = Instant::now();
                let cloned = black_box(&layer).clone();
                let list = visible_entries(&cloned, false, false);
                let font = egui::TextStyle::Button.resolve(ui.style());
                let width = list
                    .iter()
                    .flatten()
                    .map(|(label, _)| {
                        ui.ctx().fonts_mut(|fonts| fonts.layout_no_wrap(tl!(label).to_owned(), font.clone(), egui::Color32::WHITE).size().x + 44.0)
                    })
                    .fold(240.0f32, f32::max);
                let duplicate = visible_entries(&cloned, false, false);
                black_box((width, duplicate));
                let old_us = start.elapsed().as_secs_f64() * 1_000_000.0;
                let start = Instant::now();
                let target = Target::from(black_box(&layer));
                let layout = native_layout(ui.ctx(), visible_target_entries(&target, false, false));
                black_box(layout);
                let current_us = start.elapsed().as_secs_f64() * 1_000_000.0;
                if iteration > 0 {
                    old.push(old_us);
                    current.push(current_us);
                }
            }
            old.sort_by(f64::total_cmp);
            current.sort_by(f64::total_cmp);
            println!(
                "LAYER_MENU_V52_PREPARATION {}",
                json!({"children":child_count,"samples":old.len(),"oldMedianUs":old[50],"newMedianUs":current[50],"oldP95Us":old[95],"newP95Us":current[95]})
            );
        }
    });
    output.textures_delta.clear();
}
