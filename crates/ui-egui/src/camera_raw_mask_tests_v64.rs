use super::*;
use egui_kittest::{Harness, kittest::Queryable};

fn fixture_items() -> Vec<LocalAdjustment> {
    vec![
        LocalAdjustment {
            name: "径向渐变 1".into(),
            exposure: 0.4,
            mask: MaskShape::Radial { center: [0.5; 2], radius: [0.3, 0.2], feather: 0.5, rotation: 25.0 },
            components: vec![
                MaskComponent { name: "添加区域".into(), mask: MaskShape::Linear { start: [0.2; 2], end: [0.8; 2] }, ..Default::default() },
                MaskComponent { name: "减去区域".into(), subtract: true, ..Default::default() },
            ],
            ..Default::default()
        },
        LocalAdjustment { name: "第二蒙版：用于检查长名称与尾部按钮".into(), ..Default::default() },
    ]
}
struct Fixture {
    items: Vec<LocalAdjustment>,
    state: State,
    panel_rect: Rect,
}
impl Default for Fixture {
    fn default() -> Self {
        Self { items: fixture_items(), state: State { selected: Some(0), edit: true, ..Default::default() }, panel_rect: Rect::NOTHING }
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
            let image = Rect::from_min_max(bounds.min + vec2(16.0, 24.0), pos2(bounds.right() - 394.0, bounds.bottom() - 24.0));
            ui.painter().rect_filled(image, 0.0, t.canvas);
            ui.painter().text(image.center(), egui::Align2::CENTER_CENTER, "Camera Raw · 蒙版", crate::theme::medium(18.0), t.text_dim);
            self.panel_rect = Rect::from_min_max(pos2(bounds.right() - 378.0, bounds.top() + 12.0), bounds.max - vec2(12.0, 12.0));
            let mut props = ui.new_child(egui::UiBuilder::new().id_salt("v64-mask-panel").max_rect(self.panel_rect));
            props.set_clip_rect(self.panel_rect);
            egui::ScrollArea::vertical().auto_shrink([false, true]).show(&mut props, |ui| {
                panel(ui, &mut self.items, &mut self.state);
                if let Some(operation) = self.state.take_operation() {
                    let result = photocraft_engine::camera_raw_mask_cmds::apply(&self.items, &operation).unwrap();
                    self.state.install_result(&mut self.items, &operation, result);
                }
            });
        });
    }
}
fn harness(theme: crate::theme::ThemeKind) -> Harness<'static, Fixture> {
    let mut h = Harness::builder().with_size(vec2(1040.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        crate::PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        Fixture::default()
    });
    h.run_steps(8);
    h
}
fn click(h: &mut Harness<'static, Fixture>, at: Pos2) {
    h.hover_at(at);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(3);
}

fn text_shapes<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::epaint::TextShape>) {
    match shape {
        egui::Shape::Text(text) => out.push(text),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                text_shapes(shape, out);
            }
        }
        _ => {}
    }
}

#[test]
fn painted_group_and_primary_glyphs_start_at_the_name_column_in_five_themes() {
    for theme in crate::theme::ThemeKind::ALL {
        let h = harness(theme);
        let mut texts = Vec::new();
        for shape in &h.output().shapes {
            text_shapes(&shape.shape, &mut texts);
        }
        let mut lefts = Vec::new();
        for (primary, name) in [(false, "径向渐变 1"), (true, "径向渐变（基础）")] {
            let row = h.state().state.list_rows.iter().find(|r| r.group == 0 && r.primary == primary && r.component.is_none()).unwrap();
            let text = texts.iter().find(|text| text.galley.job.text == name && row.name_rect.contains(text.pos)).unwrap();
            let placed = text.galley.rows.first().unwrap();
            let glyph = placed.glyphs.first().unwrap();
            let left = text.pos.x + placed.pos.x + glyph.pos.x;
            assert!((left - row.name_rect.left()).abs() <= 1.0, "{theme:?} {name}: actual glyph x={left}, nameRect={:?}", row.name_rect);
            lefts.push(left);
        }
        assert!((lefts[0] - lefts[1]).abs() <= 1.0, "{theme:?}: painted names must align, not just their reserved rectangles");
    }
}

#[test]
fn manager_fills_sidebar_names_align_after_thumbnail_and_controls_stay_inside_in_five_themes() {
    for theme in crate::theme::ThemeKind::ALL {
        let h = harness(theme);
        let f = h.state();
        let manager = f.state.manager_rect.unwrap();
        assert!(manager.width() >= f.panel_rect.width() - 18.0, "{theme:?}: {manager:?}");
        let group = f.state.list_rows.iter().find(|r| r.group == 0 && !r.primary && r.component.is_none()).unwrap();
        let base = f.state.list_rows.iter().find(|r| r.group == 0 && r.primary).unwrap();
        assert!((base.name_rect.left() - group.name_rect.left()).abs() < 0.1);
        assert!(group.name_rect.left() > group.thumbnail_rect.unwrap().right());
        assert!(base.delete_rect.is_none(), "the required primary is not separately deletable");
        for row in &f.state.list_rows {
            assert!(manager.contains_rect(row.rect));
            assert!(row.name_rect.right() < row.checkbox_rect.left());
            assert!(row.name_rect.left() >= group.name_rect.left() - 0.1);
            assert!(row.delete_rect.is_none_or(|rect| manager.contains_rect(rect)));
        }
        let described = f.state.describe(&f.items, None);
        assert_eq!(described["rows"].as_array().unwrap().len(), f.state.list_rows.len());
    }
}

#[test]
fn real_row_clicks_select_toggle_and_delete_components_and_groups() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight);
    h.get_by_label("减去区域").click();
    h.run_steps(3);
    assert_eq!(h.state().state.selected_component, Some(1));
    let check = h.state().state.list_rows.iter().find(|r| r.component == Some(1)).unwrap().checkbox_rect.center();
    click(&mut h, check);
    assert!(!h.state().items[0].components[1].enabled);
    h.get_by_label("蒙版菜单：添加区域").click();
    h.run_steps(2);
    h.get_by_label("删除组件").click();
    h.run_steps(3);
    assert_eq!(h.state().items[0].components.len(), 1);
    assert_eq!(h.state().state.selected_component, Some(0));
    assert_eq!(h.state().items[0].components[0].name, "减去区域");
    h.get_by_label("蒙版菜单：减去区域").click();
    h.run_steps(2);
    h.get_by_label("删除组件").click();
    h.run_steps(3);
    assert!(h.state().items[0].components.is_empty());
    assert_eq!(h.state().state.selected_component, None);
    h.get_by_label("第二蒙版：用于检查长名称与尾部按钮").click();
    h.run_steps(3);
    assert_eq!(h.state().state.selected, Some(1));
    h.get_by_label("蒙版菜单：径向渐变 1").click();
    h.run_steps(2);
    h.get_by_label("删除蒙版").click();
    h.run_steps(3);
    assert_eq!(h.state().items.len(), 1);
    assert_eq!(h.state().state.selected, Some(0));
    h.get_by_label("蒙版菜单：第二蒙版：用于检查长名称与尾部按钮").click();
    h.run_steps(2);
    h.get_by_label("删除蒙版").click();
    h.run_steps(3);
    assert!(h.state().items.is_empty());
    assert_eq!(h.state().state.selected, None);
    assert!(!h.state().state.edit);
}

#[test]
fn deletion_clears_gestures_and_caches_and_rejects_generation_or_invalid_indices() {
    let mut items = fixture_items();
    let mut state = State { selected: Some(0), selected_component: Some(1), stroke: Some(0), start: Some([0.5; 2]), placing: true, ..Default::default() };
    state.curve_state.selected = Some(2);
    state.generate_for("sky", Some((0, false))).unwrap();
    let before = items.clone();
    assert!(!remove_group(&mut items, &mut state, 0));
    assert!(!remove_component(&mut items, &mut state, 0, 0));
    assert_eq!(items, before);
    state.cancel_generation();
    assert!(!remove_group(&mut items, &mut state, usize::MAX));
    assert!(!remove_component(&mut items, &mut state, 0, usize::MAX));
    assert!(remove_component(&mut items, &mut state, 0, 0));
    assert_eq!(state.selected_component, Some(0));
    assert_eq!(state.curve_state, Default::default());
    assert!(state.stroke.is_none() && state.start.is_none() && !state.placing);
    assert!(remove_group(&mut items, &mut state, 0));
    assert!(state.thumbnails.is_empty());
}

fn square_shapes<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::epaint::RectShape>) {
    match shape {
        egui::Shape::Rect(r) if (r.rect.width() - 13.0).abs() < 0.01 && (r.rect.height() - 13.0).abs() < 0.01 => out.push(r),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                square_shapes(shape, out);
            }
        }
        _ => {}
    }
}
#[test]
fn checked_controls_are_square_transparent_and_theme_neutral() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = Harness::builder().with_size(vec2(280.0, 120.0)).build_ui_state(
            |ui, on| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    square_checkbox(ui, on, "中性勾选");
                }
            },
            true,
        );
        crate::PhotocraftApp::setup_context(&h.ctx, theme);
        h.run_steps(6);
        let mut squares = Vec::new();
        for shape in &h.output().shapes {
            square_shapes(&shape.shape, &mut squares);
        }
        assert!(squares.len() >= 2);
        let tokens = crate::theme::Tokens::get(&h.ctx);
        assert!(squares.iter().all(|r| r.corner_radius == egui::CornerRadius::ZERO && r.fill == Color32::TRANSPARENT));
        assert!(squares.iter().any(|r| r.stroke.color == tokens.text_dim));
        h.get_by_label("中性勾选").click();
        h.run_steps(2);
        assert!(!*h.state());
    }
}

#[test]
#[ignore = "exports v64 full-width neutral mask manager in five themes for root visual review"]
fn full_width_mask_manager_gpu_visual_v64() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v64/mask-ui-gpu");
    assert!(!out.exists(), "preserve previous evidence");
    std::fs::create_dir_all(&out).unwrap();
    let mut captures = Vec::new();
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = Harness::builder().with_size(vec2(1040.0, 900.0)).with_max_steps(64).wgpu().build_eframe(move |cc| {
            crate::PhotocraftApp::setup_context(&cc.egui_ctx, theme);
            Fixture::default()
        });
        h.run_steps(8);
        for (name, component) in [("primary", None), ("subtract", Some(1))] {
            h.state_mut().state.selected_component = component;
            h.run_steps(4);
            let filename = format!("{}-{name}.png", theme.id());
            h.render().unwrap().save(out.join(&filename)).unwrap();
            captures.push(serde_json::json!({"file": filename, "manager": h.state().state.describe(&h.state().items, None)}));
        }
    }
    std::fs::write(out.join("captures.json"), serde_json::to_vec_pretty(&captures).unwrap()).unwrap();
}
