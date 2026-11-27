use super::*;
use egui_kittest::{Harness, kittest::Queryable};

fn fixture_items() -> Vec<LocalAdjustment> {
    vec![
        LocalAdjustment {
            name: "蒙版 1".into(),
            exposure: 0.5,
            mask: MaskShape::Radial { center: [0.5; 2], radius: [0.3, 0.2], feather: 0.5, rotation: 20.0 },
            components: vec![MaskComponent { name: "添加区域".into(), mask: MaskShape::Linear { start: [0.2; 2], end: [0.8; 2] }, ..Default::default() }],
            ..Default::default()
        },
        LocalAdjustment { name: "空蒙版".into(), mask: MaskShape::Brush { strokes: Vec::new() }, ..Default::default() },
    ]
}
struct Fixture {
    items: Vec<LocalAdjustment>,
    state: State,
    applied: Vec<MaskOperation>,
}
impl Default for Fixture {
    fn default() -> Self {
        Self { items: fixture_items(), state: State { selected: Some(0), edit: true, ..Default::default() }, applied: Vec::new() }
    }
}
impl eframe::App for Fixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let _language = crate::i18n::language_scope(crate::i18n::Lang::from_code("zh-hans").unwrap());
        if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
            return;
        }
        let t = crate::theme::Tokens::get(ui.ctx());
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.dock)).show(ui, |ui| {
            let bounds = ui.max_rect();
            let preview = Rect::from_min_max(bounds.min + vec2(16.0, 24.0), pos2(bounds.right() - 396.0, bounds.bottom() - 24.0));
            ui.painter().rect_filled(preview, 0.0, t.canvas);
            ui.painter().text(preview.center(), egui::Align2::CENTER_CENTER, "Camera Raw · 蒙版", crate::theme::medium(18.0), t.text_dim);
            let props = Rect::from_min_max(pos2(bounds.right() - 378.0, bounds.top() + 12.0), bounds.max - vec2(12.0, 12.0));
            let mut child = ui.new_child(egui::UiBuilder::new().id_salt("v65-manager").max_rect(props));
            child.set_clip_rect(props);
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(&mut child, |ui| {
                panel(ui, &mut self.items, &mut self.state);
            });
            if let Some(operation) = self.state.take_operation() {
                let result = photocraft_engine::camera_raw_mask_cmds::apply(&self.items, &operation).unwrap();
                self.state.install_result(&mut self.items, &operation, result);
                self.applied.push(operation);
            }
        });
    }
}
fn harness(theme: crate::theme::ThemeKind) -> Harness<'static, Fixture> {
    let mut h = Harness::builder().with_size(vec2(1120.0, 960.0)).with_max_steps(64).build_eframe(move |cc| {
        crate::PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        Fixture::default()
    });
    h.run_steps(8);
    h
}
fn menu(h: &mut Harness<'static, Fixture>, name: &str) {
    h.get_by_label(&format!("蒙版菜单：{name}")).click();
    h.run_steps(3);
}
fn press(h: &mut Harness<'static, Fixture>, key: egui::Key, modifiers: egui::Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.event(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
    h.run_steps(1);
    h.event(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(3);
}

#[test]
fn eyes_change_actual_group_primary_and_component_visibility_without_changing_overlay() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight);
    let overlay = h.state().state.overlay;
    h.get_by_label("可见性：蒙版 1").click();
    h.run_steps(3);
    assert!(!h.state().items[0].enabled);
    assert_eq!(h.state().state.overlay, overlay);
    h.hover_at(pos2(50.0, 50.0));
    h.run_steps(3);
    let group = h.state().state.list_rows.iter().find(|r| r.group == 0 && !r.primary && r.component.is_none()).unwrap();
    assert!(!group.eye_visible && !group.menu_visible, "hidden rows also hide their icons without hover");
    let at = group.name_rect.center();
    h.hover_at(at);
    h.run_steps(3);
    let group = h.state().state.list_rows.iter().find(|r| r.group == 0 && !r.primary && r.component.is_none()).unwrap();
    assert!(group.eye_visible && group.menu_visible, "hover reveals the hidden row's eye-off and menu");
    let thumbnail = group.thumbnail_rect.unwrap();
    h.hover_at(thumbnail.center());
    h.run_steps(3);
    let group = h.state().state.list_rows.iter().find(|r| r.group == 0 && !r.primary && r.component.is_none()).unwrap();
    assert!(group.eye_visible && group.menu_visible, "hovering the actual group thumbnail must reveal the same row controls");
    h.get_by_label("可见性：径向渐变（基础）").click();
    h.run_steps(3);
    assert!(!h.state().items[0].mask_enabled);
    h.get_by_label("可见性：添加区域").click();
    h.run_steps(3);
    assert!(!h.state().items[0].components[0].enabled);
    assert_eq!(h.state().state.overlay, overlay);
    assert_eq!(h.state().applied.len(), 3);
}

#[test]
fn menus_dispatch_real_duplicate_invert_intersect_reset_and_bulk_operations() {
    let mut h = harness(crate::theme::ThemeKind::Pro);
    menu(&mut h, "蒙版 1");
    for name in [
        "重命名…",
        "反转蒙版    Alt+X",
        "复制并反转蒙版",
        "与蒙版相交 ⏵",
        "复制蒙版",
        "隐藏蒙版",
        "删除蒙版",
        "重置调整",
        "删除当前空蒙版",
        "删除所有空蒙版",
        "删除所有蒙版",
    ] {
        h.get_by_label(name);
    }
    h.get_by_label("复制并反转蒙版").click();
    h.run_steps(3);
    assert_eq!(h.state().items.len(), 3);
    assert!(h.state().items[1].invert);
    assert_eq!(h.state().state.selected, Some(1));
    let duplicate = h.state().items[1].name.clone();
    menu(&mut h, &duplicate);
    h.get_by_label("重置调整").click();
    h.run_steps(3);
    assert_eq!(h.state().items[1].exposure, 0.0);
    menu(&mut h, &duplicate);
    h.get_by_label("与蒙版相交 ⏵").click();
    h.run_steps(3);
    h.get_by_label("明亮度范围").click();
    h.run_steps(3);
    let intersect = h.state().items[1].components.last().unwrap();
    assert!(intersect.intersect && !intersect.subtract);
    assert_eq!(h.state().state.selected_component, Some(1));
    assert!(!h.state().state.placing && h.state().state.edit);
    menu(&mut h, &duplicate);
    h.get_by_label("删除所有空蒙版").click();
    h.run_steps(3);
    assert_eq!(h.state().items.len(), 2);
    menu(&mut h, &duplicate);
    h.get_by_label("删除所有蒙版").click();
    h.run_steps(3);
    assert!(h.state().items.is_empty() && h.state().state.selected.is_none());
}

#[test]
fn rename_is_temporary_and_component_menu_deletion_preserves_selection_mapping() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight);
    assert!(h.state().state.rename.is_none());
    menu(&mut h, "蒙版 1");
    h.get_by_label("重命名…").click();
    h.run_steps(3);
    assert!(h.state().state.rename.is_some());
    h.state_mut().state.rename.as_mut().unwrap().name = "新的蒙版名称".into();
    h.run_steps(2);
    h.get_by_label("确认重命名").click();
    h.run_steps(3);
    assert_eq!(h.state().items[0].name, "新的蒙版名称");
    assert!(h.state().state.rename.is_none());
    h.get_by_label("添加区域").click();
    h.run_steps(3);
    menu(&mut h, "添加区域");
    h.get_by_label("重命名组件…").click();
    h.run_steps(3);
    h.state_mut().state.rename.as_mut().unwrap().name = "重命名组件".into();
    h.run_steps(2);
    h.get_by_label("确认重命名").click();
    h.run_steps(3);
    assert_eq!(h.state().items[0].components[0].name, "重命名组件");
    menu(&mut h, "重命名组件");
    h.get_by_label("删除组件").click();
    h.run_steps(3);
    assert!(h.state().items[0].components.is_empty());
    assert_eq!(h.state().state.selected_component, None);
}

#[test]
fn same_group_row_click_reclaims_curve_focus_and_enter_confirms_only_active_rename() {
    let mut h = harness(crate::theme::ThemeKind::Pro);
    h.state_mut().state.curve_state.selected = Some(1);
    h.get_by_label("蒙版 1").click();
    h.run_steps(3);
    assert_eq!(h.state().state.curve_state, Default::default());
    press(&mut h, egui::Key::X, egui::Modifiers::ALT);
    assert!(h.state().items[0].invert);
    menu(&mut h, "蒙版 1");
    h.get_by_label("重命名…").click();
    h.run_steps(3);
    h.state_mut().state.rename.as_mut().unwrap().name = "Enter确认的蒙版".into();
    h.run_steps(2);
    press(&mut h, egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(h.state().items[0].name, "Enter确认的蒙版");
    assert!(h.state().state.rename.is_none());
    let count = h.state().applied.len();
    press(&mut h, egui::Key::Enter, egui::Modifiers::NONE);
    assert_eq!(h.state().applied.len(), count, "Enter outside the active temporary field must not enqueue rename");
}

#[test]
fn exact_delete_and_alt_x_respect_text_curve_popup_and_generation_guards() {
    for blocked in ["shift", "control", "curve", "generation", "popup", "rename"] {
        let mut h = harness(crate::theme::ThemeKind::Pro);
        let modifiers = match blocked {
            "shift" => egui::Modifiers::SHIFT,
            "control" => egui::Modifiers::CTRL,
            "curve" => {
                h.state_mut().state.curve_state.selected = Some(1);
                egui::Modifiers::NONE
            }
            "generation" => {
                h.state_mut().state.request = Some("sky");
                egui::Modifiers::NONE
            }
            "popup" => {
                menu(&mut h, "蒙版 1");
                egui::Modifiers::NONE
            }
            _ => {
                menu(&mut h, "蒙版 1");
                h.get_by_label("重命名…").click();
                h.run_steps(3);
                egui::Modifiers::NONE
            }
        };
        press(&mut h, egui::Key::Delete, modifiers);
        assert_eq!(h.state().items.len(), 2, "{blocked}");
        assert!(h.state().applied.is_empty(), "{blocked}");
    }
    let mut h = harness(crate::theme::ThemeKind::Pro);
    press(&mut h, egui::Key::X, egui::Modifiers::ALT);
    assert!(h.state().items[0].invert);
    press(&mut h, egui::Key::Delete, egui::Modifiers::NONE);
    assert_eq!(h.state().items.len(), 1);
    assert_eq!(h.state().items[0].name, "空蒙版");
}

#[test]
fn operation_queue_generation_guard_and_result_mapping_handle_prefix_removal_and_component_removal() {
    let mut items = fixture_items();
    let mut state = State { selected: Some(1), edit: true, ..Default::default() };
    state.generate_for("sky", None).unwrap();
    assert!(state.queue_operation(MaskOperation::DeleteAll).is_err());
    state.cancel_generation();
    let operation = MaskOperation::Delete { index: 0 };
    let result = photocraft_engine::camera_raw_mask_cmds::apply(&items, &operation).unwrap();
    state.install_result(&mut items, &operation, result);
    assert_eq!(state.selected, Some(0));
    assert_eq!(items[0].name, "空蒙版");
    state.queue_operation(MaskOperation::SetEnabled { index: 0, enabled: false }).unwrap();
    assert!(state.queue_operation(MaskOperation::DeleteAll).is_err());
    assert!(matches!(state.take_operation(), Some(MaskOperation::SetEnabled { enabled: false, .. })));
}

#[test]
fn whitespace_selects_the_row_and_another_rows_eye_keeps_that_selection() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight);
    let row = h.state().state.list_rows.iter().find(|row| row.group == 1).copied().unwrap();
    let at = pos2(row.name_rect.right() - 3.0, row.name_rect.center().y);
    h.hover_at(at);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(3);
    assert_eq!(h.state().state.selected, Some(1));
    h.get_by_label("可见性：蒙版 1").click();
    h.run_steps(3);
    assert!(!h.state().items[0].enabled);
    assert_eq!(h.state().state.selected, Some(1), "visibility must not steal layer-like selection");
}

#[test]
fn generated_background_intersection_queues_a_model_transaction_and_complements_only_its_shape() {
    let ctx = egui::Context::default();
    let mut items = fixture_items();
    let before = items.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut state = State { selected: Some(0), pending: Some(rx), intersection_target: Some(0), ..Default::default() };
    tx.send(Ok((MaskShape::Raster { width: 2, height: 1, values: vec![0, 255] }, "背景", true, serde_json::Value::Null))).unwrap();
    let source: std::sync::Arc<[[f32; 4]]> = std::sync::Arc::from(vec![[0.0; 4]; 4]);
    assert!(!poll_and_start(
        &ctx,
        &mut state,
        &mut items,
        &source,
        2,
        2,
        RecognitionOptions { transform: None, quality: photocraft_engine::prefs::ImageRecognitionQuality::Fast }
    ));
    assert_eq!(items, before, "generation completion does not bypass the registered model edit");
    let operation = state.take_operation().unwrap();
    assert!(matches!(&operation, MaskOperation::Intersect { shape: MaskShape::Raster { values, .. }, .. } if values == &[255, 0]));
    let result = photocraft_engine::camera_raw_mask_cmds::apply(&items, &operation).unwrap();
    state.install_result(&mut items, &operation, result);
    assert!(!items[0].invert);
    assert!(items[0].components.last().unwrap().intersect);
    assert_eq!(state.selected_component, Some(1));
}

fn collect_shapes<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
    out.push(shape);
    if let egui::Shape::Vec(shapes) = shape {
        for shape in shapes {
            collect_shapes(shape, out);
        }
    }
}

#[test]
fn hidden_selection_skips_overlay_and_handles_and_ends_geometry_without_changing_overlay_preference() {
    for hidden in ["group", "primary", "component", "missing"] {
        let mut items = fixture_items();
        let mut state = State { selected: Some(0), overlay: true, edit: true, ..Default::default() };
        match hidden {
            "group" => items[0].enabled = false,
            "primary" => items[0].mask_enabled = false,
            "component" => {
                state.selected_component = Some(0);
                items[0].components[0].enabled = false;
            }
            _ => state.selected = Some(usize::MAX),
        }
        assert!(!state.selection_visible(&items));
        state.geometry_drag = Some(GeometryDrag { handle: Handle::Move, origin: [0.5; 2], original: items[0].mask.clone(), aspect: [1.0; 2] });
        let before = items.clone();
        let context = egui::Context::default();
        let mut output = context.run_ui(egui::RawInput::default(), |root_ui| {
            egui::CentralPanel::default().show(root_ui, |ui| {
                let rect = Rect::from_min_size(pos2(12.0, 12.0), vec2(300.0, 200.0));
                let response = ui.interact(rect, ui.id().with("hidden-mask-preview"), egui::Sense::click_and_drag());
                let first_shape = ui.painter().add(egui::Shape::Noop);
                assert!(!preview(ui, &mut state, &mut items, Preview { rect, response: &response, source: &[[0.4; 4]; 4], size: [2, 2], can_edit: true }));
                let last_shape = ui.painter().add(egui::Shape::Noop);
                assert_eq!(last_shape.0, first_shape.0 + 1, "hidden {hidden} must draw neither overlay nor handle shapes");
            });
        });
        output.textures_delta.clear();
        assert!(!output.shapes.is_empty());
        assert_eq!(items, before);
        assert!(state.geometry_drag.is_none());
        assert!(state.overlay, "visibility must not clear the user's overlay preference");
        assert!(state.overlay_texture.is_none());
    }
}
#[test]
fn five_theme_selected_rows_have_actual_fill_readable_text_and_correct_trailing_hit_order() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = harness(theme);
        h.hover_at(pos2(40.0, 40.0));
        h.run_steps(3);
        let row = h.state().state.list_rows.iter().find(|r| r.group == 0 && !r.primary && r.component.is_none()).unwrap();
        let t = crate::theme::Tokens::get(&h.ctx);
        let fill = if t.row_selected == Color32::TRANSPARENT { t.accent_soft } else { t.row_selected };
        let mut shapes = Vec::new();
        for shape in &h.output().shapes {
            collect_shapes(&shape.shape, &mut shapes);
        }
        assert!(shapes.iter().any(|shape| matches!(shape, egui::Shape::Rect(rect) if rect.rect == row.rect && rect.fill == fill)), "{theme:?}");
        let text = shapes
            .iter()
            .find_map(|shape| match shape {
                egui::Shape::Text(text) if text.galley.job.text == "蒙版 1" && row.name_rect.contains(text.pos) => Some(text),
                _ => None,
            })
            .unwrap();
        let expected = if t.bevel { t.accent_text } else { t.text };
        assert!(text.galley.job.sections.iter().all(|section| section.format.color == expected));
        let placed = text.galley.rows.first().unwrap();
        assert!((text.pos.x + placed.pos.x + placed.glyphs[0].pos.x - row.name_rect.left()).abs() <= 1.0);
        assert!(row.name_rect.right() < row.menu_rect.left() && row.menu_rect.right() <= row.checkbox_rect.left());
        assert!(!row.eye_visible && !row.menu_visible && row.delete_rect.is_none());
    }
}

#[test]
#[ignore = "exports v65 five-theme idle/hover/hidden/menu/component states; root must visually review PNGs"]
fn mask_row_actions_gpu_visual_v65() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v65/mask-ui-gpu");
    assert!(!out.exists(), "preserve previous screenshots");
    std::fs::create_dir_all(&out).unwrap();
    let mut captures = Vec::new();
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = Harness::builder().with_size(vec2(1120.0, 960.0)).with_max_steps(64).wgpu().build_eframe(move |cc| {
            crate::PhotocraftApp::setup_context(&cc.egui_ctx, theme);
            Fixture::default()
        });
        h.run_steps(8);
        for state in ["idle", "hover", "hidden", "component", "menu"] {
            match state {
                "hover" => {
                    let row = h.state().state.list_rows[0];
                    h.hover_at(row.name_rect.center());
                }
                "hidden" => {
                    h.get_by_label("可见性：蒙版 1").click();
                    h.run_steps(3);
                    h.hover_at(pos2(40.0, 40.0));
                }
                "component" => {
                    h.get_by_label("添加区域").click();
                    h.run_steps(3);
                    h.hover_at(h.get_by_label("添加区域").rect().center());
                }
                "menu" => menu(&mut h, "蒙版 1"),
                _ => h.hover_at(pos2(40.0, 40.0)),
            }
            h.run_steps(4);
            let filename = format!("{}-{state}.png", theme.id());
            h.render().unwrap().save(out.join(&filename)).unwrap();
            captures.push(serde_json::json!({"file": filename, "maskEditor": h.state().state.describe(&h.state().items, None)}));
        }
    }
    std::fs::write(out.join("captures.json"), serde_json::to_vec_pretty(&captures).unwrap()).unwrap();
}
