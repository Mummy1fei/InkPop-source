use super::*;
use egui::{Pos2, Rect, Vec2, vec2};

fn custom() -> StudioLightColors {
    let mut config = StudioLightColors::default();
    for (slot, color) in [
        (ColorSlot::Toolbar, [185, 214, 223]),
        (ColorSlot::MenuBackground, [247, 233, 203]),
        (ColorSlot::MenuHover, [201, 220, 181]),
        (ColorSlot::MenuSelection, [132, 174, 119]),
        (ColorSlot::MenuText, [25, 55, 32]),
        (ColorSlot::Field, [240, 247, 231]),
        (ColorSlot::Card, [250, 249, 238]),
        (ColorSlot::PanelTitle, [218, 226, 194]),
        (ColorSlot::Text, [32, 43, 35]),
        (ColorSlot::Accent, [58, 128, 96]),
        (ColorSlot::Canvas, [193, 211, 204]),
        (ColorSlot::SliderTrack, [113, 141, 165]),
        (ColorSlot::SliderFill, [42, 113, 93]),
        (ColorSlot::SliderHandle, [230, 179, 65]),
        (ColorSlot::ScrollbarTrack, [230, 224, 201]),
        (ColorSlot::ScrollbarHandle, [117, 98, 68]),
        (ColorSlot::ProgressTrack, [224, 218, 202]),
        (ColorSlot::ProgressFill, [85, 133, 95]),
    ] {
        config.colors.insert(slot, PaletteRgb::new(color));
    }
    config
}

fn paints(shapes: &[egui::epaint::ClippedShape], color: Color32) -> bool {
    fn contains(shape: &egui::Shape, color: Color32) -> bool {
        match shape {
            egui::Shape::Vec(children) => children.iter().any(|s| contains(s, color)),
            egui::Shape::Rect(rect) => rect.fill == color,
            egui::Shape::Circle(circle) => circle.fill == color,
            egui::Shape::Mesh(mesh) => mesh.vertices.iter().any(|v| v.color == color),
            _ => false,
        }
    }
    shapes.iter().any(|shape| contains(&shape.shape, color))
}

fn frame(ctx: &Context, mut body: impl FnMut(&mut Ui)) -> egui::FullOutput {
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(700.0, 500.0))), ..Default::default() };
    let mut output = ctx.run_ui(input, |ui| body(ui));
    output.textures_delta.clear();
    output
}

#[test]
fn empty_palette_is_exactly_the_original_style_in_every_theme_v68() {
    let ctx = Context::default();
    for kind in ThemeKind::ALL {
        super::super::apply(&ctx, kind);
        let original_tokens = Tokens::get(&ctx);
        let original_style = ctx.global_style();
        apply(&ctx, kind, &StudioLightColors::default());
        assert_eq!(Tokens::get(&ctx), original_tokens, "{kind:?} tokens");
        assert_eq!(ctx.global_style(), original_style, "{kind:?} all style fields, including spacing and alpha");
    }
}

#[test]
fn studio_light_palette_is_gated_and_restored_without_recolouring_other_themes_v68() {
    let ctx = Context::default();
    let config = custom();
    apply(&ctx, ThemeKind::StudioLight, &config);
    let customized = Tokens::get(&ctx);
    assert_eq!(customized.canvas, Color32::from_rgb(193, 211, 204));
    for kind in ThemeKind::ALL.into_iter().filter(|kind| *kind != ThemeKind::StudioLight) {
        apply(&ctx, kind, &config);
        assert_eq!(Tokens::get(&ctx), Tokens::for_kind(kind));
        assert!(ColorSlot::ALL.into_iter().all(|slot| override_color(&ctx, slot).is_none()));
        let actual_style = ctx.global_style();
        // NumberFormatter equality is Arc identity, so two fresh Contexts are not a
        // comparable Style baseline even when every debug-visible field matches. The
        // ordinary built-in apply in this SAME context retains that exact formatter.
        super::super::apply(&ctx, kind);
        assert_eq!(actual_style, ctx.global_style(), "{kind:?} exactly matches ordinary theme application in the same context");
        apply(&ctx, ThemeKind::StudioLight, &config);
        assert_eq!(Tokens::get(&ctx), customized);
        assert_eq!(color(&ctx, ColorSlot::Toolbar), Color32::from_rgb(185, 214, 223));
    }
    super::super::apply(&ctx, ThemeKind::StudioLight);
    assert!(override_color(&ctx, ColorSlot::Toolbar).is_none(), "ordinary theme apply invalidates old modules");
    apply(&ctx, ThemeKind::StudioLight, &config);
    assert_eq!(Tokens::get(&ctx), customized, "same palette restores after an external base apply");
}

#[test]
fn unchanged_palette_and_library_edits_do_not_reset_local_runtime_style_v68() {
    let ctx = Context::default();
    let mut config = custom();
    apply(&ctx, ThemeKind::StudioLight, &config);
    ctx.global_style_mut(|s| {
        s.interaction.tooltip_delay = 7.25;
        s.spacing.slider_width = 277.0;
    });
    let original = ctx.global_style();
    apply(&ctx, ThemeKind::StudioLight, &config);
    assert_eq!(ctx.global_style(), original);
    config.presets.push(photocraft_engine::prefs::palette::ColorPreset::new("Saved only", BTreeMap::new()).unwrap());
    apply(&ctx, ThemeKind::StudioLight, &config);
    assert_eq!(ctx.global_style(), original, "a library name is not an active visual change");
    config.colors.insert(ColorSlot::Card, PaletteRgb::new([240, 248, 252]));
    apply(&ctx, ThemeKind::StudioLight, &config);
    assert_eq!(Tokens::get(&ctx).card, Color32::from_rgb(240, 248, 252));
    apply(&ctx, ThemeKind::StudioLight, &StudioLightColors::default());
    assert_eq!(Tokens::get(&ctx), Tokens::for_kind(ThemeKind::StudioLight));
}

#[test]
fn sparse_roles_preserve_semantic_image_colours_and_builtin_coverage_v68() {
    let ctx = Context::default();
    let original = Tokens::for_kind(ThemeKind::StudioLight);
    let mut config = StudioLightColors::default();
    config.colors.insert(ColorSlot::Accent, PaletteRgb::new([54, 126, 160]));
    apply(&ctx, ThemeKind::StudioLight, &config);
    let t = Tokens::get(&ctx);
    assert_eq!(t.accent, Color32::from_rgb(54, 126, 160));
    assert_eq!(t.accent_soft.a(), original.accent_soft.a());
    assert_eq!(t.accent_border.a(), original.accent_border.a());
    assert_eq!(
        (t.histogram_bg, t.histogram_level, t.danger, t.warning, t.caption_close),
        (original.histogram_bg, original.histogram_level, original.danger, original.warning, original.caption_close)
    );
    assert_eq!((t.radius, t.radius_lg, t.shadow), (original.radius, original.radius_lg, original.shadow));
    config.colors.insert(ColorSlot::Selection, PaletteRgb::new([209, 229, 241]));
    apply(&ctx, ThemeKind::StudioLight, &config);
    assert_eq!(Tokens::get(&ctx).accent_soft, Color32::from_rgb(209, 229, 241));
    assert_eq!(ctx.global_style().visuals.selection.bg_fill, Color32::from_rgb(209, 229, 241));
}

#[test]
fn custom_slider_paints_independent_track_fill_and_handle_but_preserves_colour_ramps_v68() {
    let ctx = Context::default();
    apply(&ctx, ThemeKind::StudioLight, &custom());
    let mut value = 35.0;
    let output = frame(&ctx, |ui| {
        crate::widgets::slider(ui, &mut value, 0.0..=100.0, None);
    });
    for color in [[113, 141, 165], [42, 113, 93], [230, 179, 65]] {
        assert!(paints(&output.shapes, Color32::from_rgb(color[0], color[1], color[2])), "independent slider colour {color:?}");
    }
    let ramp = [Color32::from_rgb(255, 32, 16), Color32::from_rgb(8, 96, 255)];
    let gradient = frame(&ctx, |ui| {
        crate::widgets::slider(ui, &mut value, 0.0..=100.0, Some(&ramp));
    });
    for color in ramp {
        assert!(paints(&gradient.shapes, color), "colour adjustment semantics stay in their own ramp");
    }
    assert!(paints(&gradient.shapes, Color32::from_rgb(230, 179, 65)));
    assert!(!paints(&gradient.shapes, Color32::from_rgb(42, 113, 93)), "do not paint a generic fill over a hue/exposure ramp");
}

#[test]
fn scrollbar_override_paints_the_bar_without_recolouring_body_or_parent_style_v68() {
    let ctx = Context::default();
    let config = custom();
    apply(&ctx, ThemeKind::StudioLight, &config);
    // The first pass initializes the ScrollArea's content extent. The second paints its thumb.
    let mut render = |ui: &mut Ui| {
        let before = ui.style().clone();
        let output = egui::ScrollArea::vertical().id_salt("v68-independent-scrollbar").max_height(90.0).show_palette(ui, |body| {
            assert_eq!(body.style().visuals.extreme_bg_color, before.visuals.extreme_bg_color);
            assert_eq!(body.style().visuals.widgets.inactive.fg_stroke.color, before.visuals.widgets.inactive.fg_stroke.color);
            body.allocate_space(vec2(140.0, 600.0));
        });
        assert!(output.content_size.y >= 600.0);
        assert_eq!(ui.style(), &before, "parent style restored after the native ScrollArea");
    };
    frame(&ctx, &mut render);
    let output = frame(&ctx, render);
    assert!(paints(&output.shapes, Color32::from_rgb(230, 224, 201)));
    assert!(paints(&output.shapes, Color32::from_rgb(117, 98, 68)));
}

#[test]
fn scrollbar_palette_keeps_virtual_row_ranges_and_ids_v68() {
    fn draw(config: &StudioLightColors) -> (std::ops::Range<usize>, Id, Vec2) {
        let ctx = Context::default();
        apply(&ctx, ThemeKind::StudioLight, config);
        let mut rows = 0..0;
        let mut id = Id::NULL;
        let mut size = Vec2::ZERO;
        for _ in 0..3 {
            frame(&ctx, |ui| {
                let out = egui::ScrollArea::vertical().id_salt("v68-virtual").max_height(90.0).show_palette_rows(ui, 20.0, 300, |body, range| {
                    rows = range;
                    id = body.id();
                    body.allocate_space(vec2(150.0, 20.0));
                });
                size = out.content_size;
            });
        }
        (rows, id, size)
    }
    assert_eq!(draw(&custom()), draw(&StudioLightColors::default()));
}

#[test]
fn menu_and_progress_scopes_are_local_and_use_their_own_visual_channels_v68() {
    let ctx = Context::default();
    apply(&ctx, ThemeKind::StudioLight, &custom());
    frame(&ctx, |ui| {
        let original = ui.style().clone();
        ui.scope(|ui| {
            menu_scope(ui);
            assert_eq!(ui.visuals().window_fill, Color32::from_rgb(247, 233, 203));
            assert_eq!(ui.visuals().widgets.hovered.bg_fill, Color32::from_rgb(201, 220, 181));
            assert_eq!(ui.visuals().selection.bg_fill, Color32::from_rgb(132, 174, 119));
            assert_eq!(ui.visuals().text_color(), Color32::from_rgb(25, 55, 32));
        });
        assert_eq!(ui.style(), &original);
        ui.scope(|ui| {
            progress_scope(ui);
            ui.add(egui::ProgressBar::new(0.5).desired_width(180.0));
        });
        assert_eq!(ui.style(), &original);
    });
    let output = frame(&ctx, |ui| {
        ui.scope(|ui| {
            progress_scope(ui);
            ui.add(egui::ProgressBar::new(0.5).desired_width(180.0));
        });
    });
    assert!(paints(&output.shapes, Color32::from_rgb(224, 218, 202)));
    assert!(paints(&output.shapes, Color32::from_rgb(85, 133, 95)));
}

fn fixture(ui: &mut Ui, kind: ThemeKind, config: &StudioLightColors) {
    apply(ui.ctx(), kind, config);
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_filled(ui.max_rect(), 0.0, t.canvas);
    egui::Frame::NONE.fill(color(ui.ctx(), ColorSlot::Toolbar)).inner_margin(12.0).show(ui, |ui| {
        ui.label(egui::RichText::new("Toolbar / module palette").color(t.text));
    });
    ui.add_space(12.0);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(340.0);
            let mut selected = 0;
            crate::widgets::card(ui, "v68-fixture-panel", &["Layers", "Channels"], &mut selected, |ui, _| {
                ui.label("Body text keeps its own palette");
                let mut value = 40.0;
                crate::widgets::slider_row(ui, "Independent slider", &mut value, 0.0..=100.0, "%", None);
                crate::widgets::slider(ui, &mut value, 0.0..=100.0, Some(&[Color32::RED, Color32::GREEN, Color32::BLUE]));
                ui.scope(|ui| {
                    progress_scope(ui);
                    ui.add(egui::ProgressBar::new(0.55).desired_width(240.0));
                });
                egui::ScrollArea::vertical().id_salt("v68-fixture-scroll").max_height(90.0).show_palette(ui, |ui| {
                    for index in 0..15 {
                        ui.label(format!("Scroll content {index}"));
                    }
                });
            });
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width(280.0);
            menu_scope(ui);
            egui::Frame::menu(ui.style()).show(ui, |ui| {
                ui.label("Menu roles");
                ui.add(egui::Button::new("Plain item"));
                ui.add(egui::Button::new("Selected item").selected(true));
                ui.add_enabled(false, egui::Button::new("Disabled item"));
            });
        });
    });
}

#[test]
#[ignore = "writes five original themes and two actual custom control fixtures; root must inspect PNGs"]
fn studio_light_palette_gpu_visual_v68() {
    use egui_kittest::kittest::Queryable;

    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v68/palette-gpu");
    assert!(!out.exists(), "preserve prior visual evidence");
    std::fs::create_dir_all(&out).unwrap();
    let mut variants: Vec<_> = ThemeKind::ALL.into_iter().map(|kind| (format!("{}-default", kind.id()), kind, StudioLightColors::default())).collect();
    variants.push(("studioLight-custom-green".into(), ThemeKind::StudioLight, custom()));
    let mut blue = custom();
    for (slot, value) in [
        (ColorSlot::Toolbar, [199, 219, 240]),
        (ColorSlot::Card, [240, 246, 253]),
        (ColorSlot::PanelTitle, [198, 214, 239]),
        (ColorSlot::MenuBackground, [232, 241, 252]),
        (ColorSlot::MenuSelection, [159, 192, 226]),
        (ColorSlot::Canvas, [180, 198, 222]),
        (ColorSlot::ProgressFill, [66, 107, 170]),
        (ColorSlot::SliderFill, [76, 124, 194]),
        (ColorSlot::SliderHandle, [215, 229, 250]),
        (ColorSlot::ScrollbarHandle, [91, 121, 161]),
    ] {
        blue.colors.insert(slot, PaletteRgb::new(value));
    }
    variants.push(("studioLight-custom-blue".into(), ThemeKind::StudioLight, blue));
    for (name, kind, config) in variants {
        let mut harness = egui_kittest::Harness::builder().with_size(vec2(760.0, 490.0)).wgpu_setup(crate::gpu_canvas::wgpu_setup()).build_ui(move |ui| {
            // build_ui eagerly renders before setup_context can install the project's
            // named fonts. Do not enter card/tab/slider code until that setup is active.
            if ui.fonts(|fonts| fonts.definitions().families.contains_key(&egui::FontFamily::Name("medium".into()))) {
                fixture(ui, kind, &config);
            }
        });
        crate::PhotocraftApp::setup_context(&harness.ctx, kind);
        harness.run_steps(5);
        let _ = harness.get_by_label("Toolbar / module palette"); // Reject an empty guarded fixture.
        harness.render().unwrap().save(out.join(format!("{name}.png"))).unwrap();
    }
}

#[test]
fn explicit_panel_background_also_paints_native_dialog_style_clients_v68() {
    let ctx = Context::default();
    let config = custom();
    apply(&ctx, ThemeKind::StudioLight, &config);
    let expected = Color32::from_rgb(250, 249, 238);
    let output = frame(&ctx, |ui| {
        egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style())).show(ui, |_| {});
    });
    assert!(paints(&output.shapes, expected), "native settings clients must use the configured panel background");
    assert_eq!(Tokens::get(&ctx).chrome, Tokens::for_kind(ThemeKind::StudioLight).chrome);
    apply(&ctx, ThemeKind::StudioLight, &StudioLightColors::default());
    assert_eq!(
        ctx.global_style().visuals.panel_fill,
        Tokens::for_kind(ThemeKind::StudioLight).chrome,
        "clearing the override restores the original distinct dialog/chrome default"
    );
}
