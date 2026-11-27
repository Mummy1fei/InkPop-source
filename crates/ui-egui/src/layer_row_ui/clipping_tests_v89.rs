//! Real pointer input against the actual Layers panel (registered below panels.rs).
use super::*;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::Harness;

fn harness(theme: crate::theme::ThemeKind, gpu: bool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"name":"Clipping seams","width":64,"height":48,"background":"transparent"})).unwrap();
    for (name, color) in [("A", "#23bb53"), ("B", "#3377dd"), ("C", "#dd4433")] {
        app.run("layer.new.layer", json!({"name":name})).unwrap();
        app.run("edit.fill", json!({"color":color})).unwrap();
    }
    app.sync_views();
    let builder = Harness::builder().with_size(vec2(480.0, 760.0)).with_step_dt(1.0 / 60.0).with_max_steps(64);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    let mut h = builder.build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| layers(app, ui));
            let overlay = ui.ctx().data(|data| data.get_temp::<Rect>(egui::Id::new("clipping-popup-test-rect-v89")));
            if let Some(rect) = overlay {
                egui::Area::new(egui::Id::new("clipping-popup-test-area-v89"))
                    .order(egui::Order::Foreground)
                    .movable(false)
                    .fixed_pos(rect.min)
                    .fade_in(false)
                    .show(ui.ctx(), |ui| {
                        let (rect, response) = ui.allocate_exact_size(rect.size(), Sense::click());
                        ui.painter().rect_filled(rect, 4.0, Color32::from_rgb(64, 80, 96));
                        if response.clicked() {
                            ui.ctx().data_mut(|data| *data.get_temp_mut_or_default::<usize>(egui::Id::new("clipping-popup-test-clicks-v89")) += 1);
                        }
                    });
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    PhotocraftApp::enable_native_tool_cursors(&h.ctx);
    h.state_mut().run("prefs.set", json!({"path":"interface.theme","value":theme.id()})).unwrap();
    crate::prefs_ui::load(h.state_mut());
    let ctx = h.ctx.clone();
    h.state_mut().set_theme(&ctx, theme);
    h.run_steps(6);
    h
}

fn id(h: &Harness<'_, PhotocraftApp>, name: &str) -> LayerId {
    h.state().session.active().unwrap().doc.walk().iter().find(|(_, _, layer)| layer.name == name).unwrap().2.id
}

fn row(h: &Harness<'_, PhotocraftApp>, layer: LayerId) -> crate::layer_row_ui::RowRects {
    crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|row| row.layer == layer.0).unwrap()
}

fn seam(h: &Harness<'_, PhotocraftApp>, upper: LayerId, lower: LayerId) -> egui::Pos2 {
    let upper = row(h, upper);
    let lower = row(h, lower);
    let left = upper.clipping_name_area.left().max(lower.clipping_name_area.left());
    let right = upper.clipping_name_area.right().min(lower.clipping_name_area.right());
    pos2((left + right) * 0.5, (upper.row.bottom() + lower.row.top()) * 0.5)
}

fn alt() -> Modifiers {
    Modifiers::ALT
}

fn ctrl() -> Modifiers {
    Modifiers { ctrl: true, command: true, ..Modifiers::NONE }
}

fn event(h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2, pressed: Option<bool>, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(at));
    if let Some(pressed) = pressed {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers });
    }
    h.run_steps(1);
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2, modifiers: Modifiers) {
    event(h, at, Some(true), modifiers);
    event(h, at, Some(false), modifiers);
    h.run_steps(3);
}

fn clipped(h: &Harness<'_, PhotocraftApp>, layer: LayerId) -> bool {
    h.state().session.active().unwrap().doc.layer(layer).unwrap().clipped
}

#[test]
fn clipped_layer_row_arrow_points_down_to_its_base() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = harness(theme, false);
        let upper = id(&h, "C");
        let lower = id(&h, "B");
        h.state_mut().run("layer.createClippingMask", json!({"layer":upper.0})).unwrap();
        h.run_steps(3);
        let clipped_row = row(&h, upper);
        let base_row = row(&h, lower);
        assert!(base_row.row.top() >= clipped_row.row.bottom());

        let thumb_left = clipped_row.thumbnail.left();
        let gutter = Rect::from_min_max(pos2(thumb_left - 12.0, clipped_row.row.top()), pos2(thumb_left, clipped_row.row.bottom()));
        let paths: Vec<_> = h
            .output()
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Path(path) if path.points.iter().all(|point| gutter.contains(*point)) => Some(&path.points),
                _ => None,
            })
            .collect();
        let [shaft, head] = paths.as_slice() else { panic!("{theme:?}: expected a shaft and arrowhead in the layer gutter") };
        let [start, bend, tip] = shaft.as_slice() else { panic!("{theme:?}: expected a bent shaft") };
        let [left, head_tip, right] = head.as_slice() else { panic!("{theme:?}: expected an open arrowhead") };
        assert_eq!(tip, head_tip);
        assert!(start.x > bend.x && start.y == bend.y && bend.x == tip.x && bend.y < tip.y, "{theme:?}: shaft bends down");
        assert!(left.x < tip.x && tip.x < right.x && left.y < tip.y && right.y < tip.y, "{theme:?}: arrowhead points down");
    }
}

#[test]
fn clipping_alt_thumbnail_boundary_and_other_modifiers_v97() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = harness(theme, false);
        let a = id(&h, "A");
        let b = id(&h, "B");
        let c = id(&h, "C");
        h.state_mut().run("layer.select", json!({"layer":a.0})).unwrap();
        h.run_steps(3);
        let upper = row(&h, c);
        let lower = row(&h, b);
        let at = pos2(upper.thumbnail.center().x.max(lower.thumbnail.center().x), (upper.row.bottom() + lower.row.top()) * 0.5);
        let selected = h.state().session.active().unwrap().selected_layers();
        let history = h.state().session.active().unwrap().history.past_len();
        // Ctrl keeps selection semantics and cannot advertise/toggle clipping.
        event(&mut h, at, None, ctrl());
        assert!(h.output().platform_output.cursor_image.is_none());
        click(&mut h, at, ctrl());
        assert!(!clipped(&h, c));
        h.state_mut().run("layer.select", json!({"layer":a.0})).unwrap();
        h.run_steps(3);
        event(&mut h, at, None, alt());
        assert!(h.output().platform_output.cursor_image.is_some());
        click(&mut h, at, alt());
        assert!(clipped(&h, c), "{theme:?}: Alt-click also works between thumbnails");
        assert_eq!(h.state().session.active().unwrap().selected_layers(), selected);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
        assert_no_drag(&h);
        let upper = row(&h, c);
        let lower = row(&h, b);
        let at = pos2(upper.thumbnail.center().x.max(lower.thumbnail.center().x), (upper.row.bottom() + lower.row.top()) * 0.5);
        click(&mut h, at, alt());
        assert!(!clipped(&h, c));
        assert_no_drag(&h);
    }
}

#[test]
fn clipping_menu_and_real_shortcut_toggle_without_changing_engine_create_v97() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight, false);
    let c = id(&h, "C");
    let original = h.state().session.active().unwrap().doc.layer(c).unwrap().content.clone();
    let ctx = h.ctx.clone();
    let modifiers = Modifiers { ctrl: true, command: true, alt: true, ..Modifiers::NONE };
    for (expected, label) in [(true, "Release Clipping Mask"), (false, "Create Clipping Mask")] {
        let input = egui::RawInput {
            events: vec![Event::ModifiersChanged(modifiers), Event::Key { key: egui::Key::G, physical_key: None, pressed: true, repeat: false, modifiers }],
            ..Default::default()
        };
        ctx.run_ui(input, |ui| {
            assert!(crate::shortcut_dispatch::dispatch_pressed(h.state_mut(), ui.ctx(), crate::shortcut_dispatch::Focus::None, false));
        })
        .textures_delta
        .clear();
        assert_eq!(clipped(&h, c), expected);
        let items = crate::menus::menu_items(h.state());
        assert_eq!(
            items.iter().filter(|item| item.path.len() == 1 && item.path.first().is_some_and(|path| path == "Layer") && item.label == label).count(),
            1,
            "one stateful menu entry"
        );
        let item = items.into_iter().find(|item| item.id == "layer.createClippingMask").unwrap();
        assert_eq!(item.label, label);
        let shortcuts = crate::prefs_ui::shortcut_items(h.state());
        assert_eq!(shortcuts.iter().find(|item| item.0 == "layer.createClippingMask").unwrap().1, "Create Clipping Mask");
        assert_eq!(shortcuts.iter().find(|item| item.0 == "layer.releaseClippingMask").unwrap().1, "Release Clipping Mask");
        assert_eq!(h.state().session.active().unwrap().doc.layer(c).unwrap().content, original);
    }
    h.state_mut().run("layer.createClippingMask", json!({"layer":c.0})).unwrap();
    h.state_mut().run("layer.createClippingMask", json!({"layer":c.0})).unwrap();
    assert!(clipped(&h, c), "explicit engine create is still create, not toggle");
    crate::menus::invoke(h.state_mut(), &ctx, "layer.createClippingMask", json!({})).unwrap();
    assert!(!clipped(&h, c), "the displayed menu action releases the mask");
}

fn assert_no_drag(h: &Harness<'_, PhotocraftApp>) {
    assert!(h.ctx.data(|data| data.get_temp::<u64>(egui::Id::new("layer-drag"))).is_none());
    assert!(crate::layer_transfer::pointer_if_armed(h.state(), &h.ctx).is_none());
}

#[test]
fn clipping_seam_v89_all_themes_explicit_upper_preserves_selection_and_single_history() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = harness(theme, false);
        let a = id(&h, "A");
        let b = id(&h, "B");
        let c = id(&h, "C");
        h.state_mut().run("layer.select", json!({"layer":a.0})).unwrap();
        h.state_mut().run("layer.select", json!({"layer":b.0,"mode":"add"})).unwrap();
        h.run_steps(3);
        let before = h.state().session.active().unwrap().doc.clone();
        let selection = h.state().session.active().unwrap().selected_layers();
        let active = h.state().session.active().unwrap().active_layer;
        let history = h.state().session.active().unwrap().history.past_len();
        let at = seam(&h, c, b);
        // No dedicated hover frame: the press must capture before row interaction.
        click(&mut h, at, alt());
        let state = h.state().session.active().unwrap();
        assert!(state.doc.layer(c).unwrap().clipped, "{theme:?}");
        assert!(!state.doc.layer(b).unwrap().clipped);
        assert_eq!(state.selected_layers(), selection);
        assert_eq!(state.active_layer, active);
        assert_eq!(state.history.past_len(), history + 1);
        assert_eq!(state.doc.layer(c).unwrap().content, before.layer(c).unwrap().content);
        assert!(h.state().ui.dialogs.is_empty());
        assert!(h.state().ui.layer_menu.is_none());
        assert!(crate::layer_row_ui::renaming(&h.ctx).is_none());
        assert_no_drag(&h);
        assert!(h.state_mut().session.undo());
        assert!(!clipped(&h, c));
        assert!(h.state_mut().session.redo());
        assert!(clipped(&h, c));
        h.run_steps(3);
        let at = seam(&h, c, b);
        click(&mut h, at, alt());
        assert!(!clipped(&h, c), "repeat Alt-click releases");
        assert_eq!(h.state().session.active().unwrap().selected_layers(), selection);
        assert_eq!(h.state().session.active().unwrap().active_layer, active);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 2);
        assert!(h.state_mut().session.undo());
        assert!(clipped(&h, c));
        assert_no_drag(&h);
    }
}

#[test]
fn clipping_seam_v89_hover_is_nonmutating_and_ctrl_only_row_behaviors_survive() {
    let mut h = harness(crate::theme::ThemeKind::ProMedium, false);
    let a = id(&h, "A");
    let b = id(&h, "B");
    let c = id(&h, "C");
    let before = h.state().session.active().unwrap().doc.clone();
    let history = h.state().session.active().unwrap().history.past_len();
    let at = seam(&h, c, b);
    event(&mut h, at, None, alt());
    assert_eq!(h.state().session.active().unwrap().doc, before);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
    assert!(h.output().platform_output.cursor_image.is_some(), "native clipping icon on Alt hover");
    event(&mut h, at, None, Modifiers::NONE);
    assert!(h.output().platform_output.cursor_image.is_none(), "normal pointer outside Alt gesture");
    click(&mut h, at, Modifiers::NONE);
    assert!(!clipped(&h, c));
    h.state_mut().run("layer.select", json!({"layer":a.0})).unwrap();
    h.run_steps(3);
    let name = row(&h, c).clipping_name_area.center();
    click(&mut h, name, ctrl());
    assert!(h.state().session.active().unwrap().is_layer_selected(a));
    assert!(h.state().session.active().unwrap().is_layer_selected(c));
    assert!(!clipped(&h, c));
    // Ctrl thumbnail still loads transparency, not clipping or layer selection.
    let selection = h.state().session.active().unwrap().selected_layers();
    let r = row(&h, b).row;
    click(&mut h, pos2(r.left() + 46.0, r.center().y), ctrl());
    assert!(h.state().session.active().unwrap().doc.selection.is_some());
    assert_eq!(h.state().session.active().unwrap().selected_layers(), selection);
    assert!(!clipped(&h, b));
    let eye = row(&h, b).eye.center();
    click(&mut h, eye, Modifiers::ALT);
    let state = h.state().session.active().unwrap();
    assert!(state.doc.layer(b).unwrap().visible);
    assert!(!state.doc.layer(c).unwrap().visible);
    assert_no_drag(&h);
}

#[test]
fn clipping_seam_v89_cancelled_presses_never_reorder_select_or_toggle() {
    for cancel in ["outside", "control", "escape", "focus", "document"] {
        let mut h = harness(crate::theme::ThemeKind::StudioLight, false);
        let a = id(&h, "A");
        let b = id(&h, "B");
        let c = id(&h, "C");
        h.state_mut().run("layer.select", json!({"layer":a.0})).unwrap();
        h.run_steps(3);
        let before = h.state().session.active().unwrap().doc.clone();
        let selection = h.state().session.active().unwrap().selected_layers();
        let history = h.state().session.active().unwrap().history.past_len();
        let at = seam(&h, c, b);
        event(&mut h, at, Some(true), alt());
        assert_no_drag(&h);
        match cancel {
            "outside" => {
                event(&mut h, at + vec2(0.0, 35.0), None, alt());
            }
            "control" => {
                event(&mut h, at, None, Modifiers::NONE);
            }
            "escape" => {
                h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: alt() });
                h.run_steps(1);
            }
            "focus" => {
                h.event(Event::WindowFocused(false));
                h.run_steps(1);
                h.event(Event::WindowFocused(true));
                h.run_steps(1);
            }
            "document" => {
                h.state_mut().run("file.new", json!({"name":"Other","width":16,"height":16})).unwrap();
                h.run_steps(2);
            }
            other => panic!("unexpected cancellation case: {other}"),
        }
        event(&mut h, at, Some(false), alt());
        h.run_steps(3);
        let state = if cancel == "document" {
            h.state().session.documents().iter().find(|state| state.doc.id == before.id).unwrap()
        } else {
            h.state().session.active().unwrap()
        };
        assert_eq!(state.doc, before, "{cancel}");
        assert_eq!(state.selected_layers(), selection, "{cancel}");
        assert_eq!(state.history.past_len(), history, "{cancel}");
        assert_no_drag(&h);
    }
}

#[test]
fn clipping_seam_v89_buffered_press_release_only_toggles_once() {
    let mut h = harness(crate::theme::ThemeKind::Studio, false);
    let b = id(&h, "B");
    let c = id(&h, "C");
    let at = seam(&h, c, b);
    let history = h.state().session.active().unwrap().history.past_len();
    let selected = h.state().session.active().unwrap().selected_layers();
    h.event(Event::ModifiersChanged(alt()));
    h.event(Event::PointerMoved(at));
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: alt() });
    }
    h.run_steps(3);
    assert!(clipped(&h, c));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
    assert_eq!(h.state().session.active().unwrap().selected_layers(), selected);
    assert_no_drag(&h);
}

#[test]
fn clipping_seam_v89_true_siblings_only_and_filtered_gap_is_rejected() {
    let mut h = harness(crate::theme::ThemeKind::Classic, false);
    let a = id(&h, "A");
    let b = id(&h, "B");
    let c = id(&h, "C");
    h.state_mut().run("layer.select", json!({"layer":a.0})).unwrap();
    h.state_mut().run("layer.select", json!({"layer":c.0,"mode":"add"})).unwrap();
    h.state_mut().ui.layer_filter_options.mode = crate::layer_filter_ui::Mode::Selected;
    h.run_steps(3);
    assert!(!crate::layer_row_ui::recorded(&h.ctx).iter().any(|row| row.layer == b.0));
    let at = seam(&h, c, a);
    event(&mut h, at, None, alt());
    assert!(h.output().platform_output.cursor_image.is_none());
    click(&mut h, at, alt());
    assert!(!clipped(&h, c));
    h.state_mut().ui.layer_filter_options.enabled = false;
    h.state_mut().run("layer.select", json!({"layer":b.0})).unwrap();
    h.state_mut().run("layer.select", json!({"layer":c.0,"mode":"add"})).unwrap();
    let folder = LayerId(h.state_mut().run("layer.groupLayers", json!({"name":"Folder"})).unwrap()["layer"].as_u64().unwrap());
    h.run_steps(4);
    let children = h.state().session.active().unwrap().doc.layer(folder).unwrap().children().unwrap();
    assert_eq!(children.len(), 2);
    let top = children.last().unwrap().id;
    let bottom = children.first().unwrap().id;
    let at = seam(&h, folder, top);
    event(&mut h, at, None, alt());
    assert!(h.output().platform_output.cursor_image.is_none(), "header and its child are not siblings");
    click(&mut h, at, alt());
    assert!(!clipped(&h, folder));
    let at = seam(&h, top, bottom);
    click(&mut h, at, alt());
    assert!(clipped(&h, top), "same-parent child layers clip normally");
    let at = seam(&h, bottom, a);
    event(&mut h, at, None, alt());
    assert!(h.output().platform_output.cursor_image.is_none(), "last child and next root layer cannot clip across parents");
    assert_no_drag(&h);
}

#[test]
fn clipping_seam_v89_paint_plus_two_adjustments_form_one_chain() {
    let mut h = harness(crate::theme::ThemeKind::Pro, false);
    let c = id(&h, "C");
    let b = id(&h, "B");
    h.state_mut().run("layer.select", json!({"layer":c.0})).unwrap();
    let first = LayerId(h.state_mut().run("layer.newAdjustmentLayer.invert", json!({})).unwrap()["layer"].as_u64().unwrap());
    let second = LayerId(h.state_mut().run("layer.newAdjustmentLayer.invert", json!({})).unwrap()["layer"].as_u64().unwrap());
    h.run_steps(4);
    let selection = h.state().session.active().unwrap().selected_layers();
    for (upper, lower) in [(c, b), (first, c), (second, first)] {
        let at = seam(&h, upper, lower);
        click(&mut h, at, alt());
        assert!(clipped(&h, upper));
    }
    assert_eq!(h.state().session.active().unwrap().selected_layers(), selection);
    for layer in [c, first, second] {
        assert!(clipped(&h, layer));
        assert!(row(&h, layer).clipping_name_area.left() > row(&h, b).clipping_name_area.left());
    }
    assert!(h.state().ui.dialogs.is_empty());
    assert_no_drag(&h);
}

#[test]
#[ignore = "exports actual five-theme Layers UI and software clipping pointer for visual review"]
fn clipping_seam_gpu_visual_v89() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v97/clipping-ui-gpu");
    assert!(!out.exists(), "preserve earlier visual candidates");
    std::fs::create_dir_all(&out).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = harness(theme, true);
        h.ctx.data_mut(|data| data.insert_temp(egui::Id::new("photocraft-native-tool-cursors"), false));
        let b = id(&h, "B");
        let c = id(&h, "C");
        let at = seam(&h, c, b);
        event(&mut h, at, None, alt());
        h.render().unwrap().save(out.join(format!("{}-create-hover.png", theme.id()))).unwrap();
        click(&mut h, at, alt());
        let at = seam(&h, c, b);
        event(&mut h, at, None, alt());
        assert!(clipped(&h, c));
        h.render().unwrap().save(out.join(format!("{}-release-hover.png", theme.id()))).unwrap();
    }
}

#[test]
fn clipping_seam_v89_foreground_popup_consumes_its_click_without_clipping_underneath() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight, false);
    let b = id(&h, "B");
    let c = id(&h, "C");
    let at = seam(&h, c, b);
    let before = h.state().session.active().unwrap().doc.clone();
    let selected = h.state().session.active().unwrap().selected_layers();
    let history = h.state().session.active().unwrap().history.past_len();
    let area_id = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("clipping-popup-test-area-v89"));
    let rect_id = egui::Id::new("clipping-popup-test-rect-v89");
    let click_id = egui::Id::new("clipping-popup-test-clicks-v89");
    h.ctx.data_mut(|data| data.insert_temp(rect_id, Rect::from_center_size(at, vec2(120.0, 50.0))));
    h.run_steps(4);
    assert_eq!(h.ctx.layer_id_at(at), Some(area_id), "test really overlays the seam with a higher layer");
    event(&mut h, at, None, alt());
    assert!(h.output().platform_output.cursor_image.is_none(), "covered seam cannot advertise its cursor");
    click(&mut h, at, alt());
    assert_eq!(h.ctx.data(|data| data.get_temp::<usize>(click_id)), Some(1), "the foreground control receives the click");
    let state = h.state().session.active().unwrap();
    assert_eq!(state.doc, before);
    assert_eq!(state.selected_layers(), selected);
    assert_eq!(state.history.past_len(), history);
    assert_no_drag(&h);
    // Removing the overlay restores the exact same seam's normal interaction.
    h.ctx.data_mut(|data| data.remove::<Rect>(rect_id));
    h.run_steps(4);
    let at = seam(&h, c, b);
    click(&mut h, at, alt());
    assert!(clipped(&h, c));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
}

/// Build an independent expected native cursor at the same scale. No frame has
/// consumed this Context output yet, unlike the Harness FullOutput below.
fn expected_clipping_cursor(ppp: f32) -> egui::CustomCursorImage {
    let ctx = egui::Context::default();
    PhotocraftApp::enable_native_tool_cursors(&ctx);
    ctx.set_pixels_per_point(ppp);
    let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
    output.textures_delta.clear();
    assert!(crate::native_cursor::clipping(&ctx));
    ctx.output(|output| output.cursor_image.clone()).expect("independent native clipping image")
}

#[test]
fn clipping_seam_v89_native_cursor_is_published_to_integration_each_hover_frame() {
    let mut h = harness(crate::theme::ThemeKind::ProMedium, false);
    let b = id(&h, "B");
    let c = id(&h, "C");
    let at = seam(&h, c, b);
    event(&mut h, at, None, alt());
    // egui drains Context output at end_pass; the returned FullOutput is exactly
    // what a desktop integration receives, and must carry the native bitmap.
    let create = h.output().platform_output.cursor_image.clone().expect("Alt seam hover publishes a native create pointer");
    assert_eq!(create, expected_clipping_cursor(h.ctx.pixels_per_point()));
    h.run_steps(3);
    let stationary = h.output().platform_output.cursor_image.as_ref().expect("stationary Alt hover republishes after begin-pass reset");
    assert_eq!(stationary, &create);
    assert!(std::sync::Arc::ptr_eq(&stationary.rgba, &create.rgba), "stationary hover uses the bounded cursor cache");
    click(&mut h, at, alt());
    assert!(clipped(&h, c));
    let at = seam(&h, c, b);
    event(&mut h, at, None, alt());
    let release = h.output().platform_output.cursor_image.as_ref().expect("clipped seam publishes a native release pointer");
    assert_eq!(release, &create, "create and release share the layer-row arrow");
    assert!(std::sync::Arc::ptr_eq(&release.rgba, &create.rgba));
    click(&mut h, at, alt());
    assert!(!clipped(&h, c));
    let at = seam(&h, c, b);
    event(&mut h, at, None, Modifiers::NONE);
    assert!(h.output().platform_output.cursor_image.is_none(), "releasing Alt removes the seam pointer from actual integration output");

    h.ctx.data_mut(|data| data.insert_temp(egui::Id::new("photocraft-native-tool-cursors"), false));
    event(&mut h, at, None, alt());
    for path in crate::layer_row_ui::clipping_arrow_paths() {
        let points = path.map(|offset| at + offset);
        assert!(h.output().shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Path(path) if path.points == points)));
    }
}

#[test]
fn clipping_seam_v89_same_frame_alt_release_order_respects_completed_click() {
    for initially_clipped in [false, true] {
        for alt_released_before_mouse_up in [false, true] {
            let mut h = harness(crate::theme::ThemeKind::ProMedium, false);
            let a = id(&h, "A");
            let b = id(&h, "B");
            let c = id(&h, "C");
            h.state_mut().run("layer.select", json!({"layer":a.0})).unwrap();
            if initially_clipped {
                h.state_mut().run("layer.createClippingMask", json!({"layer":c.0})).unwrap();
            }
            h.run_steps(3);
            let at = seam(&h, c, b);
            let before = h.state().session.active().unwrap().doc.clone();
            let selected = h.state().session.active().unwrap().selected_layers();
            let active = h.state().session.active().unwrap().active_layer;
            let history = h.state().session.active().unwrap().history.past_len();
            let mut events = vec![
                Event::ModifiersChanged(alt()),
                Event::PointerMoved(at),
                Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: alt() },
            ];
            if alt_released_before_mouse_up {
                events.push(Event::ModifiersChanged(Modifiers::NONE));
                events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            } else {
                events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: alt() });
                events.push(Event::ModifiersChanged(Modifiers::NONE));
            }
            // Harness::event processes each queued event in its own frame. Put
            // all events directly into RawInput to exercise the real batch order.
            h.input_mut().events.extend(events);
            h.run_steps(1);
            let state = h.state().session.active().unwrap();
            let should_toggle = !alt_released_before_mouse_up;
            let mut expected = before.clone();
            std::sync::Arc::make_mut(&mut expected).layer_mut(c).unwrap().clipped = initially_clipped ^ should_toggle;
            assert_eq!(state.doc, expected, "initially_clipped={initially_clipped}, Alt up before mouse up={alt_released_before_mouse_up}");
            assert_eq!(state.history.past_len(), history + usize::from(should_toggle), "exactly one completed click or no cancelled click");
            assert_eq!(state.selected_layers(), selected);
            assert_eq!(state.active_layer, active);
            assert_no_drag(&h);
            h.run_steps(3);
            assert_eq!(h.state().session.active().unwrap().history.past_len(), history + usize::from(should_toggle), "no deferred second action");
            if should_toggle {
                assert!(h.state_mut().session.undo());
                assert_eq!(h.state().session.active().unwrap().doc, before);
                assert!(h.state_mut().session.redo());
                assert_eq!(h.state().session.active().unwrap().doc, expected);
            }
        }
    }
}
