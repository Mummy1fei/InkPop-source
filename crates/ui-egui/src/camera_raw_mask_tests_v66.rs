use super::*;
use egui_kittest::{Harness, kittest::Queryable};

struct Fixture {
    items: Vec<LocalAdjustment>,
    state: State,
    rect: Rect,
    dirty_frames: usize,
    escape_seen_after_panel: bool,
}
impl Default for Fixture {
    fn default() -> Self {
        Self {
            items: vec![LocalAdjustment {
                name: "已有蒙版".into(),
                exposure: 0.7,
                mask: MaskShape::Linear { start: [0.2, 0.2], end: [0.8, 0.8] },
                ..Default::default()
            }],
            state: State { selected: Some(0), edit: true, ..Default::default() },
            rect: Rect::NOTHING,
            dirty_frames: 0,
            escape_seen_after_panel: false,
        }
    }
}
impl eframe::App for Fixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let _language = crate::i18n::language_scope(crate::i18n::Lang::from_code("zh-hans").unwrap());
        if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
            return;
        }
        egui::CentralPanel::default().show(ui, |ui| {
            let bounds = ui.max_rect();
            self.rect = Rect::from_min_max(bounds.min + vec2(20.0, 60.0), pos2(bounds.right() - 410.0, bounds.bottom() - 60.0));
            let t = crate::theme::Tokens::get(ui.ctx());
            for column in 0..24 {
                let x0 = self.rect.left() + self.rect.width() * column as f32 / 24.0;
                let x1 = self.rect.left() + self.rect.width() * (column + 1) as f32 / 24.0;
                ui.painter().rect_filled(
                    Rect::from_min_max(pos2(x0, self.rect.top()), pos2(x1, self.rect.bottom())),
                    0.0,
                    Color32::from_rgb(45 + column * 5, 100, 150),
                );
            }
            let response = ui.interact(self.rect, ui.id().with("creation-preview"), egui::Sense::click_and_drag());
            let can_edit = self.state.edit && self.state.selection_visible(&self.items);
            let changed = preview(
                ui,
                &mut self.state,
                &mut self.items,
                Preview { rect: self.rect, response: &response, source: &[[0.35, 0.5, 0.65, 1.0]], size: [1, 1], can_edit },
            );
            let props = Rect::from_min_max(pos2(bounds.right() - 388.0, bounds.top() + 12.0), bounds.max - vec2(12.0, 12.0));
            ui.painter().rect_filled(props, 0.0, t.dock);
            let mut child = ui.new_child(egui::UiBuilder::new().id_salt("creation-props").max_rect(props.shrink(8.0)));
            child.set_clip_rect(props);
            let panel_changed = egui::ScrollArea::vertical().show(&mut child, |ui| panel(ui, &mut self.items, &mut self.state)).inner;
            self.escape_seen_after_panel |= ui.input(|input| input.key_pressed(egui::Key::Escape));
            let operation_changed = if let Some(operation) = self.state.take_operation() {
                let result = photocraft_engine::camera_raw_mask_cmds::apply(&self.items, &operation).unwrap();
                self.state.install_result(&mut self.items, &operation, result)
            } else {
                false
            };
            self.dirty_frames += usize::from(changed || panel_changed || operation_changed);
        });
    }
}
fn harness(theme: crate::theme::ThemeKind) -> Harness<'static, Fixture> {
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        crate::PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        Fixture::default()
    });
    h.run_steps(8);
    h
}
fn choose(h: &mut Harness<'static, Fixture>, action: &str, shape: &str) {
    h.get_by_label(action).click();
    h.run_steps(3);
    h.get_by_label(shape).click();
    h.run_steps(3);
}
fn start_drag(h: &mut Harness<'static, Fixture>) -> (Pos2, Pos2) {
    let from = screen(h.state().rect, [0.25, 0.3]);
    let to = screen(h.state().rect, [0.70, 0.75]);
    h.hover_at(from);
    h.drag_at(from);
    h.run_steps(1);
    h.hover_at(to);
    h.run_steps(2);
    (from, to)
}
#[test]
fn gradient_creation_has_no_default_mask_or_handles_in_five_themes() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = harness(theme);
        let before = h.state().items.clone();
        for shape in ["线性渐变", "径向渐变"] {
            choose(&mut h, "＋ 创建新蒙版", shape);
            assert_eq!(h.state().items, before, "{theme:?}: arming {shape} must not write a default shape");
            let description = h.state().state.describe(&h.state().items, Some(h.state().rect));
            assert_eq!(description["creating"], shape);
            assert!(description["shape"].is_null());
            assert_eq!(description["handles"], serde_json::json!([]));
            assert_eq!(h.state().dirty_frames, 0);
            h.get_by_label("取消绘制").click();
            h.run_steps(3);
            assert_eq!(h.state().items, before);
            assert!(h.state().state.creation.is_none());
        }
    }
}
#[test]
fn actual_drag_commits_one_new_gradient_only_after_release_and_preserves_previous_mask() {
    for shape in ["线性渐变", "径向渐变"] {
        let mut h = harness(crate::theme::ThemeKind::StudioLight);
        let before = h.state().items.clone();
        choose(&mut h, "＋ 创建新蒙版", shape);
        let (_, to) = start_drag(&mut h);
        assert_eq!(h.state().items, before, "drag preview must stay outside render parameters");
        let description = h.state().state.describe(&h.state().items, Some(h.state().rect));
        assert_eq!(description["dragging"], true);
        assert!(!description["handles"].as_array().unwrap().is_empty());
        assert_eq!(h.state().dirty_frames, 0);
        h.drop_at(to);
        h.run_steps(4);
        assert_eq!(h.state().items.len(), 2);
        assert_eq!(h.state().items[0], before[0]);
        let added = &h.state().items[1].mask;
        match added {
            MaskShape::Linear { start, end } => {
                assert!((start[0] - 0.25).abs() < 0.002 && (start[1] - 0.3).abs() < 0.002);
                assert!((end[0] - 0.70).abs() < 0.002 && (end[1] - 0.75).abs() < 0.002);
            }
            MaskShape::Radial { center, radius, .. } => {
                assert!((center[0] - 0.25).abs() < 0.002 && (center[1] - 0.3).abs() < 0.002);
                assert!((radius[0] - 0.45).abs() < 0.002 && (radius[1] - 0.45).abs() < 0.002);
            }
            _ => panic!("must create the chosen gradient"),
        }
        assert_eq!(h.state().state.selected, Some(1));
        assert!(h.state().state.creation.is_none() && !h.state().state.placing);
        assert_eq!(h.state().dirty_frames, 1, "committing the drag schedules one parameter update");
    }
}
#[test]
fn no_drag_click_escape_and_cancelled_contact_never_modify_existing_mask() {
    let mut h = harness(crate::theme::ThemeKind::Pro);
    let before = h.state().items.clone();
    choose(&mut h, "＋ 创建新蒙版", "线性渐变");
    let at = screen(h.state().rect, [0.6, 0.3]);
    h.hover_at(at);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(h.state().items, before);
    assert!(h.state().state.creation.is_some());
    let (_, to) = start_drag(&mut h);
    h.event(egui::Event::PointerGone);
    h.run_steps(1);
    h.drop_at(to);
    h.run_steps(3);
    assert_eq!(h.state().items, before);
    assert!(h.state().state.creation.is_some());
    h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().state.creation.is_none());
    assert_eq!(h.state().items, before);
    assert_eq!(h.state().dirty_frames, 0);
}
#[test]
fn add_subtract_and_intersect_gradients_wait_for_their_own_drawn_range() {
    for action in ["添加", "减去", "与蒙版相交 ⏵"] {
        let mut h = harness(crate::theme::ThemeKind::ProMedium);
        let before = h.state().items.clone();
        if action == "与蒙版相交 ⏵" {
            h.get_by_label("蒙版菜单：已有蒙版").click();
            h.run_steps(3);
        }
        choose(&mut h, action, "径向渐变");
        assert_eq!(h.state().items, before);
        let (_, to) = start_drag(&mut h);
        assert_eq!(h.state().items, before);
        h.drop_at(to);
        h.run_steps(4);
        assert_eq!(h.state().items.len(), 1);
        assert_eq!(h.state().items[0].mask, before[0].mask);
        assert_eq!(h.state().items[0].exposure, before[0].exposure);
        let component = &h.state().items[0].components[0];
        assert_eq!(component.subtract, action == "减去");
        assert_eq!(component.intersect, action == "与蒙版相交 ⏵");
        assert_eq!(h.state().state.selected_component, Some(0));
        assert!(!h.state().state.placing, "the next drag must edit the committed handles");
        assert!(photocraft_algo::camera_raw_masks::validate(&h.state().items).is_ok());
    }
}
#[test]
fn first_gradient_can_be_drawn_when_there_is_no_previous_selection() {
    let mut h = harness(crate::theme::ThemeKind::Pro);
    h.state_mut().items.clear();
    h.state_mut().state = State::default();
    h.run_steps(3);
    choose(&mut h, "＋ 创建新蒙版", "线性渐变");
    assert!(h.state().items.is_empty());
    assert!(h.state().state.selection_visible(&h.state().items));
    let (_, to) = start_drag(&mut h);
    h.drop_at(to);
    h.run_steps(4);
    assert_eq!(h.state().items.len(), 1);
}

#[test]
fn first_radial_mask_commits_on_contact_end_and_pointer_gone_in_the_same_frame() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight);
    h.state_mut().items.clear();
    h.state_mut().state = State::default();
    h.run_steps(3);
    choose(&mut h, "＋ 创建新蒙版", "径向渐变");
    let (_, to) = start_drag(&mut h);
    assert!(h.state().state.creation.as_ref().unwrap().preview.is_some());
    assert!(h.state().items.is_empty(), "preview geometry must stay transient");
    h.input_mut().events.extend([
        egui::Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId(1), phase: egui::TouchPhase::End, pos: to, force: None },
        egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerGone,
    ]);
    h.run_steps(4);
    let [first] = h.state().items.as_slice() else { panic!("expected exactly one radial mask") };
    let MaskShape::Radial { center, radius, .. } = &first.mask else { panic!("expected a radial mask") };
    assert!((center[0] - 0.25).abs() < 0.002 && (center[1] - 0.30).abs() < 0.002, "{center:?}");
    assert!((radius[0] - 0.45).abs() < 0.002 && (radius[1] - 0.45).abs() < 0.002, "{radius:?}");
    assert!(h.state().state.creation.is_none());
    assert_eq!(h.state().dirty_frames, 1);
}

#[test]
fn a_batched_gradient_drag_commits_without_waiting_for_widget_drag_start() {
    for kind in ["线性渐变", "径向渐变"] {
        let mut h = harness(crate::theme::ThemeKind::StudioLight);
        h.state_mut().items.clear();
        h.state_mut().state = State::default();
        h.run_steps(3);
        choose(&mut h, "＋ 创建新蒙版", kind);
        let from = screen(h.state().rect, [0.25, 0.30]);
        let to = screen(h.state().rect, [0.70, 0.75]);
        let contact = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        h.input_mut().events.extend([egui::Event::PointerMoved(from), contact(from, true), egui::Event::PointerMoved(to), contact(to, false)]);
        h.run_steps(4);
        assert_eq!(h.state().items.len(), 1, "{kind}");
        assert!(h.state().state.creation.is_none());
        assert_eq!(h.state().dirty_frames, 1);
    }
}

#[test]
fn switching_creation_type_or_selected_row_cancels_the_previous_gradient_draft() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight);
    let before = h.state().items.clone();
    choose(&mut h, "＋ 创建新蒙版", "径向渐变");
    h.get_by_label("已有蒙版").click();
    h.run_steps(3);
    assert!(h.state().state.creation.is_none());
    assert_eq!(h.state().items, before);
    choose(&mut h, "＋ 创建新蒙版", "线性渐变");
    choose(&mut h, "＋ 创建新蒙版", "画笔");
    assert!(h.state().state.creation.is_none());
    assert_eq!(h.state().items.len(), 2);
    assert_eq!(h.state().items[0], before[0]);
    assert!(matches!(&h.state().items[1].mask, MaskShape::Brush { strokes } if strokes.is_empty()));
}

#[test]
fn returning_to_the_press_origin_does_not_commit_a_stale_drag_preview() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight);
    let before = h.state().items.clone();
    choose(&mut h, "＋ 创建新蒙版", "线性渐变");
    let (from, _) = start_drag(&mut h);
    h.hover_at(from);
    h.run_steps(2);
    h.drop_at(from);
    h.run_steps(3);
    assert_eq!(h.state().items, before);
    assert!(h.state().state.creation.is_some());
    assert_eq!(h.state().dirty_frames, 0);
}

#[test]
fn explicitly_drawing_the_template_dimensions_is_still_a_valid_new_gradient() {
    for (label, template) in new_shapes().into_iter().filter(|(_, shape)| matches!(shape, MaskShape::Linear { .. } | MaskShape::Radial { .. })) {
        let mut h = harness(crate::theme::ThemeKind::Pro);
        choose(&mut h, "＋ 创建新蒙版", label);
        let (from, to) = match &template {
            MaskShape::Linear { start, end } => (screen(h.state().rect, *start), screen(h.state().rect, *end)),
            MaskShape::Radial { center, radius, .. } => {
                (screen(h.state().rect, *center), screen(h.state().rect, [center[0] + radius[0], center[1] + radius[1]]))
            }
            _ => panic!(),
        };
        h.hover_at(from);
        h.drag_at(from);
        h.run_steps(1);
        h.hover_at(to);
        h.run_steps(2);
        h.drop_at(to);
        h.run_steps(3);
        assert_eq!(h.state().items.len(), 2, "{label} must be created even when its shape equals the default template");
        let actual = &h.state().items[1].mask;
        match (actual, &template) {
            (MaskShape::Linear { start: a, end: b }, MaskShape::Linear { start: c, end: d }) => {
                assert!((a[0] - c[0]).abs() < 1e-5 && (a[1] - c[1]).abs() < 1e-5);
                assert!((b[0] - d[0]).abs() < 1e-5 && (b[1] - d[1]).abs() < 1e-5);
            }
            (MaskShape::Radial { center: a, radius: b, .. }, MaskShape::Radial { center: c, radius: d, .. }) => {
                assert!((a[0] - c[0]).abs() < 1e-5 && (a[1] - c[1]).abs() < 1e-5);
                assert!((b[0] - d[0]).abs() < 1e-5 && (b[1] - d[1]).abs() < 1e-5);
            }
            _ => panic!(),
        }
    }
}

#[test]
fn cancelling_a_new_gradient_restores_the_previously_disabled_edit_checkbox() {
    for exit in ["button", "escape", "row", "replace-then-cancel"] {
        let mut h = harness(crate::theme::ThemeKind::StudioLight);
        h.get_by_label("编辑蒙版").click();
        h.run_steps(3);
        assert!(!h.state().state.edit, "fixture disables the production edit checkbox");
        let before = h.state().items.clone();
        choose(&mut h, "＋ 创建新蒙版", "线性渐变");
        assert!(h.state().state.edit && h.state().state.creation.is_some());
        match exit {
            "escape" => {
                h.event(egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
            }
            "row" => h.get_by_label("已有蒙版").click(),
            "replace-then-cancel" => {
                choose(&mut h, "＋ 创建新蒙版", "径向渐变");
                h.get_by_label("取消绘制").click();
            }
            _ => h.get_by_label("取消绘制").click(),
        }
        h.run_steps(3);
        assert!(h.state().state.creation.is_none(), "{exit}");
        assert!(!h.state().state.edit, "{exit} must restore the user's disabled edit mode");
        assert_eq!(h.state().items, before);
        assert_eq!(h.state().dirty_frames, 0);
    }
}

#[test]
fn cancelling_the_first_gradient_restores_default_edit_mode_without_a_selection() {
    let mut h = harness(crate::theme::ThemeKind::Pro);
    h.state_mut().items.clear();
    h.state_mut().state = State::default();
    h.run_steps(3);
    choose(&mut h, "＋ 创建新蒙版", "径向渐变");
    assert!(h.state().state.edit);
    h.get_by_label("取消绘制").click();
    h.run_steps(3);
    assert!(h.state().items.is_empty() && h.state().state.selected.is_none());
    assert!(!h.state().state.edit && h.state().state.creation.is_none());
}

#[test]
fn successfully_drawing_a_new_gradient_enters_edit_mode_even_when_previously_disabled() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight);
    h.get_by_label("编辑蒙版").click();
    h.run_steps(3);
    assert!(!h.state().state.edit);
    choose(&mut h, "＋ 创建新蒙版", "径向渐变");
    let (_, to) = start_drag(&mut h);
    h.drop_at(to);
    h.run_steps(3);
    assert_eq!(h.state().items.len(), 2);
    assert!(h.state().state.creation.is_none() && h.state().state.edit);
}

#[test]
fn escape_and_drag_release_in_the_same_frame_cancel_without_committing_or_closing_raw() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight);
    h.get_by_label("编辑蒙版").click();
    h.run_steps(3);
    let before = h.state().items.clone();
    choose(&mut h, "＋ 创建新蒙版", "径向渐变");
    let (_, to) = start_drag(&mut h);
    assert!(h.state().state.creation.as_ref().unwrap().preview.is_some());
    // Harness::event/drop_at deliberately run one frame per queued event.
    // Inject the actual batch into RawInput to exercise one native frame.
    h.input_mut().events.extend([
        egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
        egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE },
        egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::NONE },
    ]);
    assert_eq!(h.input().events.len(), 3);
    h.run_steps(3);
    assert_eq!(h.state().items, before, "Escape must take precedence over the drag's mouseup");
    assert!(!h.state().state.edit && h.state().state.creation.is_none());
    assert!(!h.state().escape_seen_after_panel, "the outer Raw key handler must not receive the cancelled draft's Escape");
    assert_eq!(h.state().dirty_frames, 0);
}

#[test]
#[ignore = "exports v66 five-theme armed/drawing/committed states for visual review"]
fn gradient_creation_gpu_visual_v66() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v66/mask-creation-gpu");
    assert!(!out.exists(), "preserve previous screenshots");
    std::fs::create_dir_all(&out).unwrap();
    let mut captures = Vec::new();
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).with_max_steps(64).wgpu().build_eframe(move |cc| {
            crate::PhotocraftApp::setup_context(&cc.egui_ctx, theme);
            Fixture::default()
        });
        h.run_steps(8);
        choose(&mut h, "＋ 创建新蒙版", "径向渐变");
        let mut drop_at = Pos2::ZERO;
        for state in ["armed", "drawing", "committed"] {
            match state {
                "drawing" => drop_at = start_drag(&mut h).1,
                "committed" => {
                    h.drop_at(drop_at);
                    h.run_steps(4);
                }
                _ => {
                    h.hover_at(pos2(20.0, 20.0));
                    h.run_steps(3);
                }
            }
            let filename = format!("{}-{state}.png", theme.id());
            h.render().unwrap().save(out.join(&filename)).unwrap();
            captures.push(serde_json::json!({"file":filename,"count":h.state().items.len(),"maskEditor":h.state().state.describe(&h.state().items, Some(h.state().rect))}));
        }
    }
    std::fs::write(out.join("captures.json"), serde_json::to_vec_pretty(&captures).unwrap()).unwrap();
}
