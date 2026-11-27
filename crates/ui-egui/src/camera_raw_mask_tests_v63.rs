use super::*;
use egui_kittest::{Harness, kittest::Queryable};

fn radial() -> MaskShape {
    MaskShape::Radial { center: [0.48, 0.48], radius: [0.28, 0.22], feather: 0.5, rotation: 28.0 }
}
fn outside_point(mask: &MaskShape, rect: Rect, angle: f32, distance: f32) -> Pos2 {
    let MaskShape::Radial { center, radius, rotation, .. } = mask else { panic!() };
    let aspect = image_aspect(rect);
    let edge = screen(rect, radial_point(*center, [radius[0] * aspect[0] * angle.cos(), radius[1] * aspect[1] * angle.sin()], *rotation, aspect));
    let local_normal = vec2(angle.cos() / (radius[0] * rect.width()), angle.sin() / (radius[1] * rect.height())).normalized();
    let (sin, cos) = rotation.to_radians().sin_cos();
    edge + vec2(local_normal.x * cos - local_normal.y * sin, local_normal.x * sin + local_normal.y * cos) * distance
}

#[test]
fn rotation_target_is_outside_ellipse_with_constant_screen_point_width() {
    for size in [vec2(300.0, 200.0), vec2(900.0, 600.0), vec2(1800.0, 600.0)] {
        let rect = Rect::from_min_size(pos2(30.0, 40.0), size);
        for angle in [0.55, 0.9, 1.15] {
            let mask = radial();
            assert_eq!(hit_handle(&mask, rect, outside_point(&mask, rect, angle, -4.0)), Some(Handle::Move));
            assert_ne!(hit_handle(&mask, rect, outside_point(&mask, rect, angle, 1.0)), Some(Handle::Rotate));
            for distance in [5.0, 12.0, 22.0] {
                assert_eq!(
                    hit_handle(&mask, rect, outside_point(&mask, rect, angle, distance)),
                    Some(Handle::Rotate),
                    "size={size:?} angle={angle} distance={distance}"
                );
            }
            assert_eq!(hit_handle(&mask, rect, outside_point(&mask, rect, angle, 30.0)), None);
            for (handle, point) in handles(&mask, image_aspect(rect)) {
                assert_eq!(hit_handle(&mask, rect, screen(rect, point)), Some(handle));
            }
        }
    }
}

#[test]
fn perpendicular_distance_handles_flat_rotated_ellipses() {
    let mask = MaskShape::Radial { center: [0.5; 2], radius: [0.4, 0.04], feather: 0.4, rotation: -37.0 };
    let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0));
    for angle in [0.45, 0.8, 1.2] {
        assert_eq!(hit_handle(&mask, rect, outside_point(&mask, rect, angle, 18.0)), Some(Handle::Rotate));
        assert_eq!(hit_handle(&mask, rect, outside_point(&mask, rect, angle, 28.0)), None);
    }
}

struct Fixture {
    items: Vec<LocalAdjustment>,
    state: State,
    image_rect: Rect,
    source: Vec<[f32; 4]>,
    focus_section: Option<&'static str>,
}
impl Default for Fixture {
    fn default() -> Self {
        Self {
            items: vec![LocalAdjustment {
                name: "蒙版 1".into(),
                exposure: 0.3,
                temperature: 8.0,
                mask: radial(),
                components: vec![MaskComponent {
                    name: "减去区域".into(),
                    subtract: true,
                    mask: MaskShape::Radial { center: [0.43, 0.52], radius: [0.09, 0.12], feather: 0.5, rotation: -20.0 },
                    ..Default::default()
                }],
                ..Default::default()
            }],
            state: State { selected: Some(0), edit: true, ..Default::default() },
            image_rect: Rect::NOTHING,
            source: vec![[0.3, 0.45, 0.65, 1.0]; 180 * 120],
            focus_section: None,
        }
    }
}
impl eframe::App for Fixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let _language = crate::i18n::language_scope(crate::i18n::Lang::from_code("zh-hans").unwrap());
        if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
            return;
        }
        let tokens = crate::theme::Tokens::get(ui.ctx());
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(tokens.dock)).show(ui, |ui| {
            let bounds = ui.max_rect();
            let pane = Rect::from_min_max(bounds.min + vec2(12.0, 12.0), pos2(bounds.right() - 380.0, bounds.bottom() - 12.0));
            self.image_rect = Rect::from_center_size(pane.center(), vec2(pane.width() - 24.0, (pane.width() - 24.0) / 1.5));
            let mut preview_ui = ui.new_child(egui::UiBuilder::new().id_salt("v63-preview").max_rect(pane));
            preview_ui.set_clip_rect(pane);
            for i in 0..32 {
                let left = self.image_rect.left() + self.image_rect.width() * i as f32 / 32.0;
                let right = self.image_rect.left() + self.image_rect.width() * (i + 1) as f32 / 32.0;
                preview_ui.painter().rect_filled(
                    Rect::from_min_max(pos2(left, self.image_rect.top()), pos2(right, self.image_rect.bottom())),
                    0.0,
                    Color32::from_rgb(30 + i * 5, 110, 155),
                );
            }
            let response = preview_ui.interact(pane, preview_ui.id().with("input"), egui::Sense::click_and_drag());
            preview(
                &preview_ui,
                &mut self.state,
                &mut self.items,
                Preview { rect: self.image_rect, response: &response, source: &self.source, size: [180, 120], can_edit: true },
            );
            let props = Rect::from_min_max(pos2(bounds.right() - 368.0, bounds.top() + 12.0), bounds.max - vec2(12.0, 12.0));
            let mut props_ui = ui.new_child(egui::UiBuilder::new().id_salt("v63-panel").max_rect(props));
            props_ui.set_clip_rect(props);
            egui::ScrollArea::vertical().id_salt("v63-outer-scroll").show(&mut props_ui, |ui| {
                panel(ui, &mut self.items, &mut self.state);
                if let Some(operation) = self.state.take_operation() {
                    let result = photocraft_engine::camera_raw_mask_cmds::apply(&self.items, &operation).unwrap();
                    self.state.install_result(&mut self.items, &operation, result);
                }
                if let Some(target) =
                    self.focus_section.and_then(|target| self.state.section_rects.iter().find(|(name, _)| *name == target).map(|(_, rect)| *rect))
                {
                    ui.scroll_to_rect(target, Some(egui::Align::Min));
                }
            });
        });
    }
}
fn harness(theme: crate::theme::ThemeKind) -> Harness<'static, Fixture> {
    let mut h = Harness::builder().with_size(vec2(1280.0, 960.0)).with_max_steps(64).build_eframe(move |cc| {
        crate::PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        Fixture::default()
    });
    h.run_steps(8);
    h
}
fn translated(source: &str) -> &str {
    crate::i18n::tr_ctx(crate::i18n::Lang::from_code("zh-hans").unwrap(), "cameraRaw", source)
}
fn focus(h: &mut Harness<'static, Fixture>, section: &'static str) {
    h.state_mut().focus_section = Some(section);
    h.run_steps(8);
}

#[test]
fn exterior_pointer_drag_rotates_without_moving_or_resizing() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight);
    let rect = h.state().image_rect;
    let before = h.state().items[0].clone();
    let from = outside_point(&before.mask, rect, 0.8, 18.0);
    assert_eq!(hit_handle(&before.mask, rect, from), Some(Handle::Rotate));
    h.hover_at(from);
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::None);
    let MaskShape::Radial { center, .. } = before.mask else { panic!() };
    let q = from - screen(rect, center);
    let angle = 0.7f32;
    let to = screen(rect, center) + vec2(q.x * angle.cos() - q.y * angle.sin(), q.x * angle.sin() + q.y * angle.cos());
    h.drag_at(from);
    h.run_steps(2);
    for i in 1..=8 {
        h.hover_at(from + (to - from) * i as f32 / 8.0);
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(3);
    let after = &h.state().items[0];
    let MaskShape::Radial { center: next_center, radius, rotation, feather } = after.mask else { panic!() };
    assert_eq!(next_center, center);
    assert_eq!(radius, [0.28, 0.22]);
    assert_eq!(feather, 0.5);
    assert!((rotation - (28.0 + angle.to_degrees())).abs() < 0.001);
    assert_eq!(after.components, before.components);
    assert_eq!(after.exposure, before.exposure);
}

#[test]
fn mask_panel_has_matching_sections_color_bars_and_live_effect_controls() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight);
    assert_eq!(
        h.state().state.section_rects.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        ["Light", "Color", "Effects", "Curve", "Color Mixer", "Color Grading", "Detail"]
    );
    assert!(h.state().state.manager_rect.unwrap().is_positive());
    focus(&mut h, "Color");
    h.get_by_label(translated("Temperature"));
    h.get_by_label(translated("Tint"));
    h.get_by_label(translated("Hue"));
    h.get_by_label(translated("Vibrance"));
    focus(&mut h, "Effects");
    h.get_by_label(translated("Effects")).click();
    h.run_steps(8);
    let before = h.state().items[0].texture;
    let row = h.get_by_label(translated("Texture")).rect();
    let at = pos2(row.left() + 80.0, row.bottom() + 15.0);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
    assert_ne!(h.state().items[0].texture, before);
    assert!(photocraft_algo::camera_raw_masks::validate(&h.state().items).is_ok());
}

#[test]
fn local_color_mixer_keeps_hsl_edits_on_the_selected_mask_and_band() {
    let capture = std::env::var_os("INKPOP_MASK_MIXER_SNAPSHOT");
    let mut h = if capture.is_some() {
        Harness::builder().with_size(vec2(1280.0, 960.0)).with_max_steps(64).wgpu().build_eframe(|cc| {
            crate::PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::StudioLight);
            Fixture::default()
        })
    } else {
        harness(crate::theme::ThemeKind::StudioLight)
    };
    let second = LocalAdjustment { name: "蒙版 2".into(), ..h.state().items[0].clone() };
    h.state_mut().items.push(second.clone());
    focus(&mut h, "Color Mixer");
    h.get_by_label(translated("Color Mixer")).click();
    h.run_steps(8);
    let band = h.get_by_label(translated("Purple")).rect();
    let click = |h: &mut Harness<'_, Fixture>, pos| {
        h.hover_at(pos);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.run_steps(3);
    };
    click(&mut h, band.center());
    assert_eq!(h.state().state.mixer_band, 6);
    // Other sections may be open in the mask panel (with their own Hue/Saturation labels).
    for name in ["Hue", "Saturation", "Luminance"] {
        let label = h.query_all_by_label(translated(name)).map(|node| node.rect()).find(|r| r.top() > band.bottom()).unwrap();
        click(&mut h, pos2(label.left() + 75.0, label.bottom() + 15.0));
    }
    let first = &h.state().items[0];
    for values in [&first.hsl_hue, &first.hsl_sat, &first.hsl_lum] {
        assert_ne!(values[6], 0.0);
        assert!(values.iter().enumerate().all(|(i, v)| i == 6 || *v == 0.0));
    }
    assert_eq!(&h.state().items[1], &second);
    if let Some(path) = capture {
        h.hover_at(pos2(15.0, 40.0));
        h.run_steps(3);
        h.render().unwrap().save(path).unwrap();
    }
}

#[test]
fn local_sharpen_radius_double_click_keeps_parameters_valid() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight);
    h.state_mut().items[0].sharpen_amount = 75.0;
    h.state_mut().items[0].sharpen_radius = 2.2;
    let before = h.state().items[0].clone();
    focus(&mut h, "Detail");
    h.get_by_label(translated("Detail")).click();
    h.run_steps(8);
    let radius = h.get_by_label(translated("Radius")).rect();
    let at = pos2(radius.left() + 80.0, radius.bottom() + 15.0);
    h.hover_at(at);
    // Expire the category-header click so these two real pointer clicks form
    // their own double-click on the production slider track.
    h.run_steps(40);
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
    }
    h.run_steps(3);
    let after = &h.state().items[0];
    assert!((0.5..=3.0).contains(&after.sharpen_radius));
    assert_ne!(after.sharpen_radius, before.sharpen_radius);
    assert_eq!(after.sharpen_amount, before.sharpen_amount);
    assert_eq!(after.exposure, before.exposure);
    assert_eq!(after.temperature, before.temperature);
    assert_eq!(after.mask, before.mask);
    assert_eq!(after.components, before.components);
    assert!(photocraft_algo::camera_raw_masks::validate(&h.state().items).is_ok(), "double-click must not reset a positive-only parameter to zero");
}

#[test]
#[ignore = "exports v63 seven local sections in five themes; root must inspect saved PNGs"]
fn extended_mask_controls_gpu_visual_v63() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v63/mask-ui-gpu");
    assert!(!out.exists(), "preserve previous evidence");
    std::fs::create_dir_all(&out).unwrap();
    let mut paths = Vec::new();
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = Harness::builder().with_size(vec2(1280.0, 960.0)).with_max_steps(64).wgpu().build_eframe(move |cc| {
            crate::PhotocraftApp::setup_context(&cc.egui_ctx, theme);
            Fixture::default()
        });
        h.run_steps(8);
        let name = format!("{}-manager.png", theme.id());
        h.render().unwrap().save(out.join(&name)).unwrap();
        paths.push(name);
        for section in ["Color", "Effects", "Curve", "Color Mixer", "Color Grading", "Detail"] {
            focus(&mut h, section);
            if section != "Color" {
                h.get_by_label(translated(section)).click();
                h.run_steps(8);
            }
            let name = format!("{}-{}.png", theme.id(), section.replace(' ', "-"));
            h.render().unwrap().save(out.join(&name)).unwrap();
            paths.push(name);
        }
    }
    std::fs::write(out.join("captures.json"), serde_json::to_vec_pretty(&paths).unwrap()).unwrap();
}
