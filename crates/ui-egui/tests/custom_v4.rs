use egui::{Event, Id, Modifiers, PointerButton, pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_ui_egui::{PhotocraftApp, theme::ThemeKind};
use serde_json::json;
fn app() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1400., 900.)).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, ThemeKind::Studio);
        let mut a = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        a.session.prefs.edit(|p| p.interface.language = "en".into());
        a.run("file.new", json!({"width":80,"height":60})).unwrap();
        a
    });
    h.run_steps(6);
    h
}
fn drag(h: &mut Harness<'static, PhotocraftApp>, start: egui::Pos2, end: egui::Pos2) {
    h.event(Event::PointerMoved(start));
    h.run_steps(2);
    h.event(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    for i in 1..=14 {
        h.event(Event::PointerMoved(start + (end - start) * (i as f32 / 14.)));
        h.run_steps(2);
    }
    h.event(Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(5);
}
#[test]
fn floating_tabs_tear_out_and_rejoin_without_moving_the_source() {
    let mut h = app();
    let ctx = h.ctx.clone();
    h.state_mut().set_theme(&ctx, ThemeKind::Studio);
    h.state_mut().ui.panels.color = true;
    h.state_mut().ui.panels.brush_settings = true;
    h.state_mut().ui.dock = serde_json::from_value(json!({"containers":[{
        "id":91,"tabs":[{"group":"brush","index":0},{"group":"color","index":1}],
        "floating":true,"position":[100,100],"size":[500,550]
    }]}))
    .unwrap();
    h.run_steps(6);
    let window = h.ctx.memory(|m| m.area_rect(Id::new(("panel-container", 91u64)))).unwrap();
    let start = h
        .query_all_by_label("Color")
        .map(|n| n.rect().center())
        .find(|p| window.contains(*p) && p.y > window.top() + 28. && p.y < window.top() + 100.)
        .unwrap();
    drag(&mut h, start, pos2(800., 200.));
    assert_eq!(h.state().ui.dock.containers.len(), 2);
    let source = h.state().ui.dock.containers.iter().find(|c| c.id == 91).unwrap();
    assert_eq!(source.tabs.len(), 1);
    let source_rect = h.ctx.memory(|m| m.area_rect(Id::new(("panel-container", 91u64)))).unwrap();
    assert!((source_rect.min - window.min).length() < 1.);
    let detached = h.state().ui.dock.containers.iter().find(|c| c.id != 91).unwrap();
    assert!(detached.floating && detached.stack.is_none());
    let detached_id = detached.id;
    let detached_rect = h.ctx.memory(|m| m.area_rect(Id::new(("panel-container", detached_id)))).unwrap();
    let start = h
        .query_all_by_label("Color")
        .map(|n| n.rect().center())
        .find(|p| detached_rect.contains(*p) && p.y > detached_rect.top() + 28. && p.y < detached_rect.top() + 100.)
        .unwrap();
    drag(&mut h, start, source_rect.center());
    assert_eq!(h.state().ui.dock.containers.len(), 2, "dropping over panel content must keep both floating panels");
    let detached_id = h.state().ui.dock.containers.iter().find(|c| c.id != 91).unwrap().id;
    let detached_rect = h.ctx.memory(|m| m.area_rect(Id::new(("panel-container", detached_id)))).unwrap();
    let start = h
        .query_all_by_label("Color")
        .map(|n| n.rect().center())
        .find(|p| detached_rect.contains(*p) && p.y > detached_rect.top() + 28. && p.y < detached_rect.top() + 100.)
        .unwrap();
    let header = h
        .query_all_by_label("Brush Settings")
        .map(|n| n.rect().center())
        .find(|p| source_rect.contains(*p) && p.y > source_rect.top() + 28. && p.y < source_rect.top() + 100.)
        .unwrap();
    drag(&mut h, start, header);
    assert_eq!(h.state().ui.dock.containers.len(), 1, "start={start:?}, header={header:?}, containers={:?}", h.state().ui.dock.containers);
    assert_eq!(h.state().ui.dock.containers[0].tabs.len(), 2);
    let final_rect = h.ctx.memory(|m| m.area_rect(Id::new(("panel-container", 91u64)))).unwrap();
    assert!((final_rect.min - source_rect.min).length() < 1., "merging into its header keeps the target position");
}
#[test]
fn rail_docks_a_window_and_dragging_its_icon_floats_it() {
    let mut h = app();
    h.state_mut().ui.panels.tool_presets = true;
    h.state_mut().ui.dock = serde_json::from_value(
        json!({"containers":[{"id":91,"tabs":[{"group":"toolPresets","index":0}],"floating":true,"position":[300,100],"size":[330,500]}]}),
    )
    .unwrap();
    h.run_steps(6);
    let r = h.ctx.memory(|m| m.area_rect(Id::new(("panel-container", 91u64)))).unwrap();
    let rail = h.ctx.data(|d| d.get_temp::<egui::Rect>(Id::new("panel-icon-rail-rect"))).unwrap();
    drag(&mut h, r.min + vec2(120., 15.), pos2(rail.center().x, 250.));
    assert!(!h.state().ui.dock.containers[0].floating);
    let rail = h.ctx.data(|d| d.get_temp::<egui::Rect>(Id::new("panel-icon-rail-rect"))).unwrap();
    let points: Vec<_> = h.query_all_by_label("Tool Presets").map(|n| n.rect().center()).collect();
    let icon = points.iter().copied().find(|p| rail.contains(*p)).unwrap_or_else(|| panic!("rail {rail:?}, labels {points:?}"));
    drag(&mut h, icon, pos2(400., 350.));
    assert!(h.state().ui.dock.containers.iter().any(|c| c.floating && c.tabs.iter().any(|t| format!("{:?}", t.group) == "ToolPresets")));
}
#[test]
fn manager_multiselect_deletes_only_checked_brushes() {
    let mut h = app();
    let before = h.state().session.tools.presets.len();
    let names: Vec<_> = h.state().session.tools.presets.iter().take(2).map(|p| p.name.clone()).collect();
    h.state_mut().ui.presets_ui.manager = true;
    h.run_steps(6);
    for n in &names {
        h.get_by_label(n).click();
        h.run_steps(2);
    }
    h.get_by_label("删除选中的 2 项").click();
    h.run_steps(3);
    assert_eq!(h.state().session.tools.presets.len(), before - 2);
    assert!(names.iter().all(|n| !h.state().session.tools.presets.iter().any(|p| &p.name == n)));
}

#[test]
fn color_rail_restores_a_missing_container_and_manager_uses_multiple_columns() {
    let mut h = app();
    h.state_mut().ui.dock = serde_json::from_value(json!({"containers":[{"id":91,"tabs":[{"group":"layers","index":0}]}]})).unwrap();
    h.state_mut().ui.panels.color = false;
    h.run_steps(4);
    h.get_by_label("Color & Swatches").click();
    h.run_steps(5);
    assert!(h.state().ui.panels.color);
    assert!(h.state().ui.dock.containers.iter().any(|c| c.tabs.iter().any(|t| format!("{:?}", t.group) == "Color")));
    let names: Vec<_> = h.state().session.tools.presets.iter().take(2).map(|p| p.name.clone()).collect();
    h.state_mut().ui.presets_ui.manager = true;
    h.run_steps(5);
    let a = h.get_by_label(&names[0]).rect();
    let b = h.get_by_label(&names[1]).rect();
    assert!((a.center().y - b.center().y).abs() < 2. && b.left() > a.left(), "{a:?} {b:?}");
}

#[test]
fn manager_shift_click_selects_a_range_across_columns() {
    let mut h = app();
    let names: Vec<_> = h.state().session.tools.presets.iter().take(5).map(|p| p.name.clone()).collect();
    h.state_mut().ui.presets_ui.manager = true;
    h.run_steps(5);
    h.get_by_label(&names[1]).click();
    h.run_steps(2);
    h.event(Event::ModifiersChanged(Modifiers::SHIFT));
    h.run_steps(1);
    h.get_by_label(&names[4]).click();
    h.run_steps(2);
    assert_eq!(h.state().ui.presets_ui.manager_selected.len(), 4);
    assert!(names[1..].iter().all(|n| h.state().ui.presets_ui.manager_selected.contains(n)));
}

#[test]
fn tool_preset_secondary_click_renames_and_deletes_in_floating_panel() {
    let mut h = app();
    h.state_mut().ui.panels.tool_presets = true;
    h.state_mut().ui.dock = serde_json::from_value(
        json!({"containers":[{"id":91,"tabs":[{"group":"toolPresets","index":0}],"floating":true,"position":[300,100],"size":[330,500]}]}),
    )
    .unwrap();
    h.run_steps(6);
    let name = h.state().session.presets.tool_presets[0].name.clone();
    let right = |h: &mut Harness<'static, PhotocraftApp>, name: &str| {
        let p = h.get_by_label(name).rect().center();
        h.event(Event::PointerMoved(p));
        h.run_steps(2);
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed, modifiers: Modifiers::NONE });
            h.run_steps(2);
        }
    };
    right(&mut h, &name);
    h.get_by_label("Rename Tool Preset…").click();
    h.run_steps(2);
    h.event(Event::Text("Renamed preset".into()));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(h.state().session.presets.tool_presets.iter().any(|p| p.name == "Renamed preset"));
    right(&mut h, "Renamed preset");
    h.get_by_label("Delete Tool Preset").click();
    h.run_steps(3);
    assert!(!h.state().session.presets.tool_presets.iter().any(|p| p.name == "Renamed preset"));
}
#[test]
fn color_panel_keeps_semantics_and_hsb_controls_across_all_themes() {
    let mut h = app();
    h.state_mut().ui.theme = ThemeKind::Studio;
    h.state_mut().ui.dock_tabs.color = 1;
    h.state_mut().ui.dock =
        serde_json::from_value(json!({"containers":[{"id":91,"tabs":[{"group":"color","index":1}],"floating":true,"position":[300,100],"size":[350,600]}]}))
            .unwrap();
    for theme in ThemeKind::ALL {
        let ctx = h.ctx.clone();
        h.state_mut().set_theme(&ctx, theme);
        h.run_steps(4);
        let pro = matches!(theme, ThemeKind::Pro | ThemeKind::ProMedium);
        assert_eq!(h.state().ui.dock.containers[0].tabs[0].index, usize::from(!pro));
        assert!(h.query_all_by_label("Hue").count() > 0);
        assert!(h.query_all_by_label("Saturation").count() > 0);
        assert!(h.query_all_by_label("Brightness").count() > 0);
    }
}
