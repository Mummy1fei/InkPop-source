//! Delete follows the real Layers/canvas focus and never leaks into editors.

use egui::accesskit::Role;
use egui::{Event, Key, Modifiers, PointerButton, Pos2, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LayerId};
use serde_json::json;

use crate::{PhotocraftApp, layer_row_ui};

fn harness() -> (Harness<'static, PhotocraftApp>, [u64; 2]) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width":160,"height":120})).unwrap();
    let ids = ["Alpha", "Beta"].map(|name| {
        let id = s.execute("layer.new.layer", json!({"name":name})).unwrap()["layer"].as_u64().unwrap();
        s.execute("edit.fill", json!({"contents":"color","color":"#e06020"})).unwrap();
        id
    });
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({"tool":"hand","dock":{"collapsed":["color","properties","history","navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    (h, ids)
}

fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, modifiers: Modifiers) {
    h.hover_at(at);
    h.run_steps(1);
    h.event(Event::ModifiersChanged(modifiers));
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers });
        h.run_steps(1);
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(3);
}

fn row_point(h: &Harness<'_, PhotocraftApp>, id: u64) -> Pos2 {
    let r = layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == id).expect("row drawn");
    // Empty row space, outside the thumbnail/name double-click actions.
    pos2(r.row.right() - 25.0, r.row.center().y)
}

fn double_click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2) {
    h.hover_at(at);
    // Match the established rename tests: expire the earlier selection click,
    // then deliver two clicks two 60 Hz frames apart inside the double-click window.
    h.run_steps(40);
    for _ in 0..2 {
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.step();
        }
    }
    h.run_steps(3);
}

fn select(h: &mut Harness<'_, PhotocraftApp>, id: u64, add: bool) {
    h.run_steps(40); // Distinct selection gestures, never a name/Layer Style double-click.
    let modifiers = if add {
        Modifiers { command: true, ctrl: !cfg!(target_os = "macos"), mac_cmd: cfg!(target_os = "macos"), ..Modifiers::NONE }
    } else {
        Modifiers::NONE
    };
    click(h, row_point(h, id), modifiers);
    let doc = h.state().session.active().unwrap().doc.id;
    assert!(layer_row_ui::has_keyboard_focus(&h.ctx, doc), "selection click owns the layer-list Delete scope");
}

fn key(h: &mut Harness<'_, PhotocraftApp>, key: Key) {
    for pressed in [true, false] {
        h.event(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
    }
    h.run_steps(3);
}

fn exists(h: &Harness<'_, PhotocraftApp>, id: u64) -> bool {
    h.state().session.active().unwrap().doc.layer(LayerId(id)).is_some()
}

#[test]
fn layer_row_delete_removes_single_multiple_and_group_layers_with_undo() {
    for delete in [Key::Delete, Key::Backspace] {
        let (mut h, [a, b]) = harness();
        select(&mut h, a, false);
        key(&mut h, delete);
        assert!(!exists(&h, a) && exists(&h, b));
        assert!(h.state_mut().session.undo());
        h.run_steps(3);
        select(&mut h, a, false);
        select(&mut h, b, true);
        assert_eq!(h.state().session.active().unwrap().selected_layers().len(), 2);
        key(&mut h, delete);
        assert!(!exists(&h, a) && !exists(&h, b));
        assert!(h.state_mut().session.undo());
        assert!(exists(&h, a) && exists(&h, b), "multi-delete is one undo");
        h.run_steps(3);
        select(&mut h, a, false);
        select(&mut h, b, true);
        let group = h.state_mut().run("layer.groupLayers", json!({"name":"Folder"})).unwrap()["layer"].as_u64().unwrap();
        h.run_steps(4);
        select(&mut h, group, false);
        key(&mut h, delete);
        assert!(!exists(&h, group) && !exists(&h, a) && !exists(&h, b));
        assert!(h.state_mut().session.undo());
        assert!(exists(&h, group) && exists(&h, a) && exists(&h, b));
    }
}

#[test]
fn clicking_canvas_returns_delete_to_pixel_selection_clear() {
    let (mut h, [a, _]) = harness();
    select(&mut h, a, false);
    h.state_mut().run("select.rect", json!({"x":5,"y":5,"width":20,"height":20})).unwrap();
    let canvas = h.state().last_canvas_rect.center();
    click(&mut h, canvas, Modifiers::NONE);
    let doc = h.state().session.active().unwrap().doc.id;
    assert!(!layer_row_ui::has_keyboard_focus(&h.ctx, doc));
    key(&mut h, Key::Delete);
    let layer = h.state().session.active().unwrap().doc.layer(LayerId(a)).expect("Clear keeps the layer");
    let pixels = layer.surface().unwrap();
    assert_eq!(pixels.pixel(10, 10)[3], 0.0, "selected pixels cleared");
    assert_eq!(pixels.pixel(50, 50)[3], 1.0, "pixels outside the selection unchanged");
}

#[test]
fn delete_belongs_to_layer_rename_numeric_fields_and_inline_text() {
    let (mut h, [a, _]) = harness();
    select(&mut h, a, false);
    let name = layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == a).unwrap().name.unwrap().center();
    double_click(&mut h, name);
    assert_eq!(layer_row_ui::renaming(&h.ctx), Some(a));
    key(&mut h, Key::Delete);
    assert!(exists(&h, a));
    assert_eq!(layer_row_ui::rename(&h.ctx).unwrap().text, "", "Delete edits the selected layer name");
    key(&mut h, Key::Escape);

    select(&mut h, a, false);
    let panel = crate::dock::last_rects(&h.ctx).into_iter().find(|(g, _)| *g == crate::dock::Group::Layers).unwrap().1;
    let field = h
        .query_all_by_role(Role::SpinButton)
        .filter(|n| !n.accesskit_node().is_disabled() && !n.accesskit_node().is_hidden())
        .map(|n| n.rect())
        .filter(|r| panel.contains(r.center()))
        .min_by(|a, b| a.center().y.total_cmp(&b.center().y))
        .expect("opacity field")
        .center();
    click(&mut h, field, Modifiers::NONE);
    assert!(h.ctx.text_edit_focused());
    key(&mut h, Key::Delete);
    assert!(exists(&h, a), "numeric editing never deletes a layer");
    key(&mut h, Key::Escape);

    let text = h.state_mut().run("type.create", json!({"text":"Hello","x":20,"y":50,"size":18})).unwrap()["layer"].as_u64().unwrap();
    h.run_steps(3);
    select(&mut h, text, false);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "type.editText", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.state().ui.text_edit.is_some());
    key(&mut h, Key::Delete);
    let layer = h.state().session.active().unwrap().doc.layer(LayerId(text)).expect("inline editing retains text layer");
    let LayerContent::Text(content) = &layer.content else { panic!("text layer") };
    assert_eq!(content.text, "", "Delete edits selected inline text");
}

#[test]
fn layer_scope_does_not_override_user_delete_binding() {
    let (mut h, [a, _]) = harness();
    h.state_mut().run("edit.keyboardShortcuts", json!({"set":{"edit.clear":"Alt+K","tools.defaultColors":"Delete"}})).unwrap();
    select(&mut h, a, false);
    super::take_log(&h.ctx);
    key(&mut h, Key::Delete);
    assert!(exists(&h, a));
    assert_eq!(super::take_log(&h.ctx), vec![("tools.defaultColors".into(), super::Outcome::Ran)]);
}
