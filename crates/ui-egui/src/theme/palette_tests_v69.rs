use super::*;

fn config(colors: &[(ColorSlot, [u8; 3])]) -> StudioLightColors {
    StudioLightColors { colors: colors.iter().map(|&(slot, value)| (slot, PaletteRgb::new(value))).collect(), ..Default::default() }
}

#[test]
fn title_and_menu_share_current_or_legacy_override_in_draft_and_active_theme_v69() {
    let ctx = Context::default();
    for (config, expected) in [
        (config(&[(ColorSlot::TitleBar, [61, 87, 113])]), Color32::from_rgb(61, 87, 113)),
        (config(&[(ColorSlot::MenuBar, [117, 141, 83])]), Color32::from_rgb(117, 141, 83)),
        (config(&[(ColorSlot::TitleBar, [61, 87, 113]), (ColorSlot::MenuBar, [117, 141, 83])]), Color32::from_rgb(61, 87, 113)),
    ] {
        let original = config.clone();
        apply(&ctx, ThemeKind::StudioLight, &config);
        for slot in [ColorSlot::TitleBar, ColorSlot::MenuBar] {
            assert_eq!(resolved_color(ThemeKind::StudioLight, &config, slot), expected, "draft {slot:?}");
            assert_eq!(override_color(&ctx, slot), Some(expected), "active override {slot:?}");
            assert_eq!(color(&ctx, slot), expected, "active surface {slot:?}");
            assert!(has_override(&ctx, slot));
        }
        assert_eq!(config, original, "reading aliases must not migrate the user's map");
        assert_eq!(Tokens::get(&ctx).chrome, Tokens::for_kind(ThemeKind::StudioLight).chrome, "other chrome roles remain independent");
    }
}

#[test]
fn unified_topbar_override_stays_out_of_other_themes_and_clears_on_reset_v69() {
    let ctx = Context::default();
    let config = config(&[(ColorSlot::TitleBar, [61, 87, 113]), (ColorSlot::MenuBar, [117, 141, 83])]);
    for kind in ThemeKind::ALL {
        super::super::apply(&ctx, kind);
        let baseline = (Tokens::get(&ctx), ctx.global_style());
        apply(&ctx, kind, &config);
        if kind != ThemeKind::StudioLight {
            assert_eq!((Tokens::get(&ctx), ctx.global_style()), baseline);
            for slot in [ColorSlot::TitleBar, ColorSlot::MenuBar] {
                assert!(override_color(&ctx, slot).is_none());
                assert_eq!(color(&ctx, slot), baseline.0.chrome);
                assert_eq!(resolved_color(kind, &config, slot), baseline.0.chrome);
            }
        }
        apply(&ctx, kind, &StudioLightColors::default());
        assert_eq!((Tokens::get(&ctx), ctx.global_style()), baseline, "{kind:?} exact empty-map baseline");
        for slot in [ColorSlot::TitleBar, ColorSlot::MenuBar] {
            assert!(override_color(&ctx, slot).is_none());
            assert_eq!(color(&ctx, slot), baseline.0.chrome);
        }
    }
}

fn filled_rectangles(shapes: &[egui::epaint::ClippedShape], color: Color32) -> Vec<egui::Rect> {
    fn collect(shape: &egui::Shape, color: Color32, rectangles: &mut Vec<egui::Rect>) {
        match shape {
            egui::Shape::Vec(children) => children.iter().for_each(|shape| collect(shape, color, rectangles)),
            egui::Shape::Rect(rect) if rect.fill == color => rectangles.push(rect.rect),
            _ => {}
        }
    }
    let mut rectangles = Vec::new();
    for shape in shapes {
        collect(&shape.shape, color, &mut rectangles);
    }
    rectangles
}

#[test]
fn production_title_and_both_menu_backends_paint_one_background_with_conflicting_legacy_colors_v69() {
    for embedded in [true, false] {
        let ctx = Context::default();
        ctx.set_embed_viewports(embedded);
        crate::PhotocraftApp::setup_context(&ctx, ThemeKind::StudioLight);
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.theme = ThemeKind::StudioLight;
        let config = config(&[(ColorSlot::TitleBar, [61, 87, 113]), (ColorSlot::MenuBar, [117, 141, 83])]);
        let expected = Color32::from_rgb(61, 87, 113);
        let obsolete = Color32::from_rgb(117, 141, 83);
        apply(&ctx, ThemeKind::StudioLight, &config);
        let mut output = None;
        for _ in 0..3 {
            let mut frame = ctx.run_ui(
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 500.0))), ..Default::default() },
                |ui| crate::panels::title_bar(&mut app, ui),
            );
            // This paint-only fixture deliberately does not upload GPU textures.
            frame.textures_delta.clear();
            output = Some(frame);
        }
        let output = output.unwrap();
        let backgrounds = filled_rectangles(&output.shapes, expected);
        assert!(backgrounds.iter().any(|rect| rect.width() >= 1090.0), "the production title frame uses the unified colour (embedded={embedded})");
        assert!(
            backgrounds.iter().filter(|rect| rect.width() >= 200.0).count() >= 2,
            "the production menu region also paints the same colour (embedded={embedded})"
        );
        assert!(
            filled_rectangles(&output.shapes, obsolete).is_empty(),
            "the legacy menu background cannot remain as a contrasting patch (embedded={embedded})"
        );
    }
}
