use super::*;
use crate::theme::ThemeKind;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn scene(kind: ThemeKind, depth: u32) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":64,"height":48,"depth":depth})).unwrap();
    app.run("gradient.fill.create", json!({"from":[0,0],"to":[64,0]})).unwrap();
    let mut h = Harness::builder().with_size(vec2(320.0, 220.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let st = app.session.active().unwrap();
            let layer = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
            for (field, label, value) in [("opacity", "Opacity", layer.opacity), ("fill", "Fill", layer.fill_opacity)] {
                let r = layer_opacity_control(app, ui, layer.id, field, label, value);
                ui.data_mut(|d| d.insert_temp(egui::Id::new(("track-v77", field)), r.rect));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, kind);
    h.run_steps(3);
    h
}

fn value(h: &Harness<'_, PhotocraftApp>, field: &str) -> f32 {
    let st = h.state().session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    if field == "opacity" { layer.opacity } else { layer.fill_opacity }
}

fn track(h: &Harness<'_, PhotocraftApp>, field: &str) -> Rect {
    h.ctx.data(|d| d.get_temp(egui::Id::new(("track-v77", field)))).unwrap()
}

fn at(rect: Rect, fraction: f32) -> egui::Pos2 {
    pos2(rect.left() + 7.0 + fraction * (rect.width() - 14.0), rect.center().y)
}

#[test]
fn properties_opacity_fill_tracks_update_while_held_and_undo_once_per_drag() {
    for kind in ThemeKind::ALL {
        for depth in [8, 16, 32] {
            let mut h = scene(kind, depth);
            for field in ["opacity", "fill"] {
                let rect = track(&h, field);
                let history = h.state().session.active().unwrap().history.past_len();
                h.hover_at(at(rect, 1.0));
                h.run_steps(1);
                h.drag_at(at(rect, 1.0));
                h.run_steps(1);
                for fraction in [0.75, 0.45, 0.2] {
                    h.hover_at(at(rect, fraction));
                    h.run_steps(1);
                    assert!((value(&h, field) - fraction).abs() < 0.015, "{kind:?}/{depth}/{field}: live value {}", value(&h, field));
                    h.run_steps(2);
                    assert!((value(&h, field) - fraction).abs() < 0.015, "held pointer must not reset value");
                }
                h.drop_at(at(rect, 0.27));
                h.run_steps(2);
                assert!((value(&h, field) - 0.27).abs() < 0.015, "release uses the latest pointer coordinate");
                assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 1);
                h.state_mut().run("edit.undo", json!({})).unwrap();
                h.run_steps(2);
                assert_eq!(value(&h, field), 1.0);
                h.state_mut().run("edit.redo", json!({})).unwrap();
                h.run_steps(2);
                assert!((value(&h, field) - 0.27).abs() < 0.015);
                // A later gesture must not merge into the previous undo step.
                h.run_steps(32);
                h.drag_at(at(rect, 0.27));
                h.run_steps(1);
                h.hover_at(at(rect, 0.6));
                h.run_steps(1);
                h.drop_at(at(rect, 0.6));
                h.run_steps(2);
                assert!((value(&h, field) - 0.6).abs() < 0.015);
                assert_eq!(h.state().session.active().unwrap().history.past_len(), history + 2);
                h.state_mut().run("edit.undo", json!({})).unwrap();
                h.run_steps(2);
                assert!((value(&h, field) - 0.27).abs() < 0.015);
            }
        }
    }
}

#[test]
fn properties_opacity_fill_track_clicks_and_numeric_edits_still_work() {
    let mut h = scene(ThemeKind::StudioLight, 16);
    for (index, field) in ["opacity", "fill"].into_iter().enumerate() {
        let rect = track(&h, field);
        let p = at(rect, 0.35);
        h.drag_at(p);
        h.run_steps(1);
        h.drop_at(p);
        h.run_steps(2);
        assert!((value(&h, field) - 0.35).abs() < 0.015);
        h.run_steps(32);
        h.query_all_by_role(egui::accesskit::Role::SpinButton).nth(index).unwrap().click();
        h.run_steps(1);
        h.event(egui::Event::Text("82".into()));
        h.run_steps(1);
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        assert!((value(&h, field) - 0.82).abs() < 0.015);
    }
}

#[test]
fn properties_drag_does_not_follow_a_different_document_with_the_same_layer_id() {
    let mut h = scene(ThemeKind::StudioLight, 16);
    let rect = track(&h, "opacity");
    h.drag_at(at(rect, 1.0));
    h.run_steps(1);
    h.hover_at(at(rect, 0.65));
    h.run_steps(1);
    assert!((value(&h, "opacity") - 0.65).abs() < 0.015);
    let original = h.state().session.active().unwrap().doc.clone();
    let id = h.state().session.active().unwrap().active_layer.unwrap();
    h.state_mut().session.add_document((*original).clone(), None);
    h.state_mut().session.select_layer(id).unwrap();
    h.run_steps(2);
    h.hover_at(at(rect, 0.15));
    h.run_steps(1);
    h.drop_at(at(rect, 0.15));
    h.run_steps(2);
    assert!((value(&h, "opacity") - 0.65).abs() < 0.015, "old drag must not affect another document");
    assert_eq!(h.state().session.active().unwrap().history.past_len(), 0);
}
