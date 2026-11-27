use super::*;
use egui_kittest::{Harness, kittest::Queryable};

fn ellipse(center: [f32; 2], radius: [f32; 2], rotation: f32) -> MaskShape {
    MaskShape::Radial { center, radius, rotation, feather: 0.45 }
}
fn fixture_items() -> Vec<LocalAdjustment> {
    vec![LocalAdjustment {
        name: "蒙版测试".into(),
        exposure: 0.6,
        mask: ellipse([0.46, 0.48], [0.29, 0.23], 0.0),
        components: vec![
            MaskComponent { name: "补充范围".into(), mask: ellipse([0.72, 0.3], [0.12, 0.1], 15.0), ..Default::default() },
            MaskComponent { name: "排除区域".into(), subtract: true, mask: ellipse([0.40, 0.55], [0.10, 0.13], -20.0), ..Default::default() },
        ],
        ..Default::default()
    }]
}

struct Fixture {
    items: Vec<LocalAdjustment>,
    state: State,
    source: Vec<[f32; 4]>,
    image_rect: Rect,
    dirty_frames: usize,
}
impl Fixture {
    fn new() -> Self {
        Self {
            items: fixture_items(),
            state: State { selected: Some(0), edit: true, ..Default::default() },
            source: vec![[0.35, 0.5, 0.65, 1.0]; 180 * 120],
            image_rect: Rect::NOTHING,
            dirty_frames: 0,
        }
    }
}
impl eframe::App for Fixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let _language = crate::i18n::language_scope(crate::i18n::Lang::from_code("zh-hans").unwrap());
        if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
            return;
        }
        let t = crate::theme::Tokens::get(ui.ctx());
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.canvas)).show(ui, |ui| {
            let bounds = ui.max_rect();
            let pane = Rect::from_min_max(bounds.min + vec2(12.0, 38.0), pos2(bounds.right() - 388.0, bounds.bottom() - 35.0));
            let width = pane.width().min(pane.height() * 1.5) - 24.0;
            self.image_rect = Rect::from_center_size(pane.center(), vec2(width, width / 1.5));
            let mut preview_ui = ui.new_child(egui::UiBuilder::new().id_salt("test-preview").max_rect(pane));
            preview_ui.set_clip_rect(pane);
            for column in 0..24 {
                let x0 = self.image_rect.left() + self.image_rect.width() * column as f32 / 24.0;
                let x1 = self.image_rect.left() + self.image_rect.width() * (column + 1) as f32 / 24.0;
                preview_ui.painter().rect_filled(
                    Rect::from_min_max(pos2(x0, self.image_rect.top()), pos2(x1, self.image_rect.bottom())),
                    0.0,
                    Color32::from_rgb(55 + column * 5, 105, 155),
                );
            }
            let response = preview_ui.interact(pane, preview_ui.id().with("mask-input"), egui::Sense::click_and_drag());
            let changed = preview(
                &preview_ui,
                &mut self.state,
                &mut self.items,
                Preview { rect: self.image_rect, response: &response, source: &self.source, size: [180, 120], can_edit: true },
            );
            let panel_rect = Rect::from_min_max(pos2(bounds.right() - 372.0, bounds.top() + 12.0), bounds.max - vec2(12.0, 12.0));
            let mut panel_ui = ui.new_child(egui::UiBuilder::new().id_salt("test-mask-panel").max_rect(panel_rect));
            panel_ui.set_clip_rect(panel_rect);
            let panel_changed =
                egui::ScrollArea::vertical().id_salt("test-mask-outer").show(&mut panel_ui, |ui| panel(ui, &mut self.items, &mut self.state)).inner;
            if let Some(operation) = self.state.take_operation() {
                let result = photocraft_engine::camera_raw_mask_cmds::apply(&self.items, &operation).unwrap();
                self.state.install_result(&mut self.items, &operation, result);
            }
            self.dirty_frames += usize::from(changed || panel_changed);
        });
    }
}
fn harness(theme: crate::theme::ThemeKind) -> Harness<'static, Fixture> {
    let mut h = Harness::builder().with_size(vec2(1280.0, 960.0)).with_max_steps(64).build_eframe(move |cc| {
        // This fixture calls the production mask UI directly; no App::logic can override
        // its explicit Context theme with a separate default preference.
        crate::PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        Fixture::new()
    });
    h.run_steps(8);
    h
}

#[test]
fn radial_outer_ring_rotation_matches_pixel_space_and_keeps_rotated_axes() {
    let rect = Rect::from_min_size(pos2(30.0, 40.0), vec2(900.0, 600.0));
    let aspect = image_aspect(rect);
    let center = [0.5, 0.5];
    let original = ellipse(center, [0.3, 0.15], 0.0);
    let angle = 0.7f32;
    let edge = radial_point(center, [0.3 * aspect[0] * angle.cos(), 0.15 * aspect[1] * angle.sin()], 0.0, aspect);
    let edge_px = screen(rect, edge);
    let outside = edge_px + (edge_px - screen(rect, center)).normalized() * 6.0;
    assert_eq!(hit_handle(&original, rect, outside), Some(Handle::Rotate));
    assert_eq!(hit_handle(&original, rect, outside + (outside - screen(rect, center)).normalized() * 48.0), None);
    let origin = normalized(rect, outside);
    let q = [(origin[0] - center[0]) * aspect[0], (origin[1] - center[1]) * aspect[1]];
    let target = radial_point(center, q, 90.0, aspect);
    let mut mask = original.clone();
    let drag = GeometryDrag { handle: Handle::Rotate, origin, original: original.clone(), aspect };
    assert!(update_geometry(&mut mask, &drag, target));
    let MaskShape::Radial { rotation, radius, center: after, feather } = mask else { panic!() };
    assert!((rotation - 90.0).abs() < 1e-4);
    assert_eq!(radius, [0.3, 0.15]);
    assert_eq!(after, center);
    assert_eq!(feather, 0.45);
    let mut rotated = ellipse(center, radius, rotation);
    let edge = handles(&rotated, aspect).iter().find(|(h, _)| *h == Handle::RadiusX).unwrap().1;
    assert_eq!(hit_handle(&rotated, rect, screen(rect, edge)), Some(Handle::RadiusX));
    let grow = radial_point(center, [-0.4 * aspect[0], 0.0], rotation, aspect);
    let drag = GeometryDrag { handle: Handle::RadiusX, origin: edge, original: rotated.clone(), aspect };
    assert!(update_geometry(&mut rotated, &drag, grow));
    let MaskShape::Radial { rotation: next, radius, .. } = rotated else { panic!() };
    assert!((radius[0] - 0.4).abs() < 1e-5);
    assert_eq!(radius[1], 0.15);
    assert!((next - 90.0).abs() < 1e-4);
}

#[test]
fn rotation_drag_in_production_preview_moves_only_selected_component() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight);
    h.get_by_label("排除区域").click();
    h.run_steps(3);
    assert_eq!(h.state().state.selected_component, Some(1));
    let before = h.state().items[0].clone();
    let rect = h.state().image_rect;
    let aspect = image_aspect(rect);
    let MaskShape::Radial { center, radius, rotation, .. } = shape(&before, Some(1)) else { panic!() };
    let angle = 0.65f32;
    let edge = screen(rect, radial_point(*center, [radius[0] * aspect[0] * angle.cos(), radius[1] * aspect[1] * angle.sin()], *rotation, aspect));
    let from = edge + (edge - screen(rect, *center)).normalized() * 6.0;
    assert_eq!(hit_handle(shape(&before, Some(1)), rect, from), Some(Handle::Rotate));
    let origin = normalized(rect, from);
    let vector = [(origin[0] - center[0]) * aspect[0], (origin[1] - center[1]) * aspect[1]];
    let to = screen(rect, radial_point(*center, vector, 60.0, aspect));
    h.hover_at(from);
    h.drag_at(from);
    h.run_steps(2);
    for i in 1..=8 {
        h.hover_at(from + (to - from) * i as f32 / 8.0);
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(3);
    let after = &h.state().items[0];
    assert_eq!(after.mask, before.mask);
    assert_eq!(after.components[0], before.components[0]);
    assert_ne!(after.components[1].mask, before.components[1].mask);
    assert_eq!(after.exposure, before.exposure);
    assert!(!h.state().state.placing);
    assert!(h.state().state.geometry_drag.is_none());
}

#[test]
fn add_subtract_component_menus_and_delete_keep_one_shared_adjustment_group() {
    let mut h = harness(crate::theme::ThemeKind::ProMedium);
    let primary = h.state().items[0].mask.clone();
    h.get_by_label("添加").click();
    h.run_steps(2);
    h.get_by_label("线性渐变").click();
    h.run_steps(3);
    assert_eq!(h.state().items[0].components.len(), 2, "choosing a gradient only arms the creation tool");
    let rect = h.state().image_rect;
    let from = screen(rect, [0.2, 0.3]);
    let to = screen(rect, [0.7, 0.7]);
    h.hover_at(from);
    h.drag_at(from);
    h.run_steps(1);
    h.hover_at(to);
    h.run_steps(2);
    h.drop_at(to);
    h.run_steps(3);
    assert_eq!(h.state().items.len(), 1);
    assert_eq!(h.state().items[0].components.len(), 3);
    assert_eq!(h.state().state.selected_component, Some(2));
    assert!(!h.state().items[0].components[2].subtract);
    h.get_by_label("减去").click();
    h.run_steps(2);
    h.get_by_label("画笔").click();
    h.run_steps(3);
    assert_eq!(h.state().items.len(), 1);
    assert_eq!(h.state().items[0].components.len(), 4);
    assert!(h.state().items[0].components[3].subtract);
    assert_eq!(h.state().items[0].mask, primary);
    assert_eq!(h.state().items[0].exposure, 0.6);
    let delete_label = format!("蒙版菜单：{}", h.state().items[0].components[3].name);
    h.get_by_label(&delete_label).click();
    h.run_steps(2);
    h.get_by_label("删除组件").click();
    h.run_steps(3);
    assert_eq!(h.state().items[0].components.len(), 3);
    assert_eq!(h.state().state.selected_component, None);
    assert!(photocraft_algo::camera_raw_masks::validate(&h.state().items).is_ok());
}

#[test]
fn local_exposure_slider_reuses_mask_overlay_and_thumbnail_textures() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight);
    let overlay = h.state().state.overlay_texture.as_ref().unwrap().id();
    let thumb = h.state().state.thumbnails[0].as_ref().unwrap().id();
    let before = h.state().items[0].exposure;
    let exposure = h.get_by_label("曝光").rect();
    let at = pos2(exposure.left() + 80.0, exposure.bottom() + 15.0);
    h.hover_at(at);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(4);
    assert_ne!(h.state().items[0].exposure, before, "click the real production exposure slider");
    assert_eq!(h.state().state.overlay_texture.as_ref().unwrap().id(), overlay);
    assert_eq!(h.state().state.thumbnails[0].as_ref().unwrap().id(), thumb);
}

#[test]
fn generated_background_component_does_not_invert_siblings_and_cancel_drops_result() {
    let ctx = egui::Context::default();
    let mut items = fixture_items();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut state = State { selected: Some(0), pending: Some(rx), component_target: Some((0, true)), ..Default::default() };
    tx.send(Ok((MaskShape::Raster { width: 2, height: 1, values: vec![0, 255] }, "背景", true, serde_json::Value::Null))).unwrap();
    let source = std::sync::Arc::from(vec![[0.0; 4]; 4]);
    assert!(poll_and_start(
        &ctx,
        &mut state,
        &mut items,
        &source,
        2,
        2,
        RecognitionOptions { transform: None, quality: photocraft_engine::prefs::ImageRecognitionQuality::Fast }
    ));
    assert!(!items[0].invert);
    let component = items[0].components.last().unwrap();
    assert!(component.subtract);
    assert_eq!(component.mask, MaskShape::Raster { width: 2, height: 1, values: vec![255, 0] });
    let count = items[0].components.len();
    let (tx, rx) = std::sync::mpsc::channel();
    state.pending = Some(rx);
    state.component_target = Some((0, false));
    state.reset_geometry();
    tx.send(Ok((ellipse([0.5; 2], [0.2; 2], 0.0), "径向渐变", false, serde_json::Value::Null))).unwrap();
    assert!(!poll_and_start(
        &ctx,
        &mut state,
        &mut items,
        &source,
        2,
        2,
        RecognitionOptions { transform: None, quality: photocraft_engine::prefs::ImageRecognitionQuality::Fast }
    ));
    assert_eq!(items[0].components.len(), count);
}

fn square_rects(shape: &egui::Shape, out: &mut Vec<egui::CornerRadius>) {
    match shape {
        egui::Shape::Rect(r) if (r.rect.width() - 13.0).abs() < 0.01 && (r.rect.height() - 13.0).abs() < 0.01 => out.push(r.corner_radius),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                square_rects(shape, out);
            }
        }
        _ => {}
    }
}
#[test]
fn raw_checkboxes_remain_square_and_clickable_in_every_theme() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = Harness::builder().with_size(vec2(280.0, 120.0)).build_ui_state(
            |ui, on| {
                if ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    square_checkbox(ui, on, "启用测试");
                }
            },
            false,
        );
        crate::PhotocraftApp::setup_context(&h.ctx, theme);
        h.run_steps(6);
        let mut radii = Vec::new();
        for shape in &h.output().shapes {
            square_rects(&shape.shape, &mut radii);
        }
        assert!(radii.len() >= 2);
        assert!(radii.iter().all(|r| *r == egui::CornerRadius::ZERO), "{theme:?}");
        h.get_by_label("启用测试").click();
        h.run_steps(2);
        assert!(*h.state());
    }
}

#[test]
#[ignore = "exports v62 five-theme grouped/rotated mask controls and checkbox GPU screenshots"]
fn grouped_radial_mask_gpu_visual_v62() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v62/mask-ui-gpu");
    assert!(!out.exists(), "preserve previous evidence");
    std::fs::create_dir_all(&out).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = Harness::builder().with_size(vec2(1280.0, 960.0)).wgpu().with_max_steps(64).build_eframe(move |cc| {
            crate::PhotocraftApp::setup_context(&cc.egui_ctx, theme);
            Fixture::new()
        });
        if let MaskShape::Radial { rotation, .. } = &mut h.state_mut().items[0].mask {
            *rotation = 28.0;
        }
        h.run_steps(8);
        h.render().unwrap().save(out.join(format!("{}-group-primary.png", theme.id()))).unwrap();
        h.get_by_label("排除区域").click();
        h.run_steps(3);
        h.render().unwrap().save(out.join(format!("{}-subtract-component.png", theme.id()))).unwrap();
    }
}
