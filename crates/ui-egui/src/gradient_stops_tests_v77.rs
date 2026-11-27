//! Real pointer input for the selected opacity stop, preview, sorting and undo.
use super::*;
use crate::theme::ThemeKind;
use egui::{Event, Modifiers, PointerButton};
use egui_kittest::{Harness, kittest::Queryable};

struct Fixture {
    app: PhotocraftApp,
    outer: Rect,
    full_properties: bool,
    hidden: bool,
}

impl eframe::App for Fixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            if self.hidden {
                return;
            }
            let layer = self.app.session.active().unwrap().doc.layer(self.app.session.active().unwrap().active_layer.unwrap()).unwrap().clone();
            if self.full_properties {
                ui.label("Gradient Fill — selected opacity stop");
                properties(&mut self.app, ui, &layer);
            } else {
                self.outer = Rect::from_min_size(ui.next_widget_position(), vec2(ui.available_width(), 58.0));
                stops_editor(&mut self.app, ui, &layer);
            }
        });
    }
}

fn fixture(theme: ThemeKind, depth: u32, gpu: bool, full_properties: bool) -> Harness<'static, Fixture> {
    let builder = Harness::builder().with_size(vec2(340.0, 440.0)).with_max_steps(64).with_step_dt(1.0 / 60.0);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":200,"height":120,"depth":depth})).unwrap();
        app.run(cmds::CREATE, json!({"from":[0,60],"to":[200,60],"stops":[[0,"#446688"],[1,"#e8bc68"]],"transparency":[[0,100],[1,70]]})).unwrap();
        Fixture { app, outer: Rect::NOTHING, full_properties, hidden: false }
    })
}

fn id(h: &Harness<'_, Fixture>) -> LayerId {
    h.state().app.session.active().unwrap().active_layer.unwrap()
}

fn selected_key(h: &Harness<'_, Fixture>) -> egui::Id {
    egui::Id::new(("gradient-stops-selected", h.state().app.session.active().unwrap().doc.id, id(h).0))
}

fn opacity_stops(h: &Harness<'_, Fixture>) -> Vec<(f32, f32)> {
    let Fill::Gradient { opacity_stops, .. } = fill_of(h.state().app.session.active().unwrap().doc.layer(id(h)).unwrap()).unwrap() else { panic!("gradient") };
    opacity_stops.clone()
}

fn marker(h: &Harness<'_, Fixture>, location: f32) -> Pos2 {
    let outer = h.state().outer;
    pos2(outer.left() + 8.0 + location * (outer.width() - 16.0), outer.top() + 6.0)
}

fn press(h: &mut Harness<'_, Fixture>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

fn click(h: &mut Harness<'_, Fixture>, pos: Pos2) {
    h.hover_at(pos);
    h.run_steps(1);
    press(h, pos, true);
    press(h, pos, false);
    h.run_steps(2);
}

fn slider_track(h: &Harness<'_, Fixture>) -> Rect {
    h.output()
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Rect(r) if (r.rect.height() - 3.0).abs() < 0.01 && r.rect.width() > 200.0 => Some(r.rect),
            _ => None,
        })
        .expect("actual opacity slider track")
}

fn drag_opacity(h: &mut Harness<'_, Fixture>, percent: f32) {
    let track = slider_track(h);
    let from = pos2(track.left() + track.width() * 0.8, track.center().y);
    let to = pos2(track.left() + track.width() * percent / 100.0, track.center().y);
    h.hover_at(from);
    h.run_steps(1);
    press(h, from, true);
    h.hover_at(pos2(to.x - 12.0, to.y));
    h.run_steps(2);
    press(h, to, false);
    h.run_steps(2);
}

#[test]
fn selected_opacity_stop_is_independent_updates_live_and_coalesces_once_v77() {
    for theme in ThemeKind::ALL {
        for depth in [8, 16, 32] {
            let mut h = fixture(theme, depth, false, false);
            h.run_steps(4);
            let right = marker(&h, 1.0);
            click(&mut h, right);
            assert_eq!(h.ctx.data(|d| d.get_temp::<Marker>(selected_key(&h))), Some(Marker::Opacity(1)));
            let before = h.state().app.session.active().unwrap().history.past_len();
            let initial = opacity_stops(&h);
            let track = slider_track(&h);
            let start = pos2(track.left() + 0.8 * track.width(), track.center().y);
            let middle = pos2(track.left() + 0.5 * track.width(), track.center().y);
            h.hover_at(start);
            h.run_steps(1);
            press(&mut h, start, true);
            h.hover_at(middle);
            h.run_steps(2);
            assert_eq!(opacity_stops(&h)[0], initial[0]);
            assert!((opacity_stops(&h)[1].1 - 0.5).abs() < 1e-5, "document updates live during the gesture");
            assert_eq!(h.state().app.session.active().unwrap().history.past_len(), before + 1);
            let active_index = h.state().app.session.active_index().unwrap();
            assert!(display_doc(&mut h.state_mut().app, active_index).is_none(), "no detached numeric preview survives outside the document");
            let end = pos2(track.left() + 0.3 * track.width(), track.center().y);
            press(&mut h, end, false);
            h.run_steps(3);
            let stops = opacity_stops(&h);
            assert_eq!(stops[0], initial[0], "other opacity stop stays unchanged");
            assert!((stops[1].1 - 0.3).abs() < 1e-5, "final release coordinate is applied: {stops:?}");
            let st = h.state().app.session.active().unwrap();
            assert_eq!(st.history.past_len(), before + 1);
            assert_eq!(st.doc.layer(id(&h)).unwrap().opacity, 1.0);
            assert_eq!(st.doc.layer(id(&h)).unwrap().fill_opacity, 1.0);
            h.state_mut().app.run("edit.undo", json!({})).unwrap();
            assert_eq!(opacity_stops(&h), initial);
            h.state_mut().app.run("edit.redo", json!({})).unwrap();
            assert_eq!(opacity_stops(&h), stops);
        }
    }
}

#[test]
fn opacity_marker_fast_drag_crosses_stops_and_keeps_its_selection_v77() {
    let mut h = fixture(ThemeKind::StudioLight, 8, false, false);
    h.run_steps(4);
    let layer = id(&h);
    h.state_mut().app.run(cmds::STOP, json!({"layer":layer.0,"kind":"opacity","action":"add","location":0.4,"opacity":55})).unwrap();
    h.run_steps(2);
    let from = marker(&h, 0.0);
    let middle = marker(&h, 0.6);
    let to = marker(&h, 0.75);
    h.hover_at(from);
    h.run_steps(1);
    let before = h.state().app.session.active().unwrap().history.past_len();
    press(&mut h, from, true);
    h.hover_at(middle);
    h.run_steps(2);
    assert!(h.state().app.gradient.panel.is_some(), "large first movement must hit the original pressed marker");
    assert_eq!(opacity_stops(&h)[0].0, 0.0, "preview only before release");
    press(&mut h, to, false);
    h.run_steps(3);
    assert_eq!(h.state().app.session.active().unwrap().history.past_len(), before + 1);
    assert!((opacity_stops(&h)[1].0 - 0.75).abs() < 1e-5);
    assert_eq!(h.ctx.data(|d| d.get_temp::<Marker>(selected_key(&h))), Some(Marker::Opacity(1)));
    drag_opacity(&mut h, 30.0);
    assert_eq!(opacity_stops(&h)[0], (0.4, 0.55));
    assert!((opacity_stops(&h)[1].1 - 0.3).abs() < 1e-5);
    assert_eq!(opacity_stops(&h)[2], (1.0, 0.7));
}

#[test]
fn selected_opacity_stop_accepts_typed_value_and_undo_v77() {
    let mut h = fixture(ThemeKind::Pro, 16, false, false);
    h.run_steps(4);
    let right = marker(&h, 1.0);
    click(&mut h, right);
    let before = h.state().app.session.active().unwrap().history.past_len();
    h.query_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap().click();
    h.run_steps(2);
    h.event(Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::COMMAND });
    h.event(Event::Text("25".into()));
    h.run_steps(2);
    assert!((opacity_stops(&h)[1].1 - 0.25).abs() < 1e-5, "typing updates the document immediately");
    h.event(Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert!((opacity_stops(&h)[1].1 - 0.25).abs() < 1e-5);
    assert_eq!(h.state().app.session.active().unwrap().history.past_len(), before + 1);
    h.state_mut().app.run("edit.undo", json!({})).unwrap();
    assert_eq!(opacity_stops(&h)[1].1, 0.7);
}

#[test]
fn opacity_marker_tear_off_previews_stably_then_deletes_once_v77() {
    let mut h = fixture(ThemeKind::StudioLight, 8, false, false);
    h.run_steps(4);
    let layer = id(&h);
    h.state_mut().app.run(cmds::STOP, json!({"layer":layer.0,"kind":"opacity","action":"add","location":0.4,"opacity":55})).unwrap();
    h.run_steps(2);
    let original = opacity_stops(&h);
    let from = marker(&h, 0.4);
    let off = from - vec2(0.0, 44.0);
    h.hover_at(from);
    h.run_steps(1);
    let before = h.state().app.session.active().unwrap().history.past_len();
    press(&mut h, from, true);
    h.event(Event::PointerMoved(off));
    h.run_steps(4);
    assert_eq!(h.state().app.gradient.panel.as_ref().unwrap().2["action"], "delete");
    assert_eq!(opacity_stops(&h), original);
    press(&mut h, off, false);
    h.run_steps(3);
    assert_eq!(opacity_stops(&h), vec![(0.0, 1.0), (1.0, 0.7)]);
    assert_eq!(h.state().app.session.active().unwrap().history.past_len(), before + 1);
    h.state_mut().app.run("edit.undo", json!({})).unwrap();
    assert_eq!(opacity_stops(&h), original);
}

#[test]
fn opacity_preview_does_not_follow_another_document_with_same_layer_id_v77() {
    let mut h = fixture(ThemeKind::StudioLight, 8, false, false);
    h.run_steps(4);
    let layer = id(&h);
    let original_doc = h.state().app.session.active().unwrap().doc.clone();
    let original_index = h.state().app.session.active_index().unwrap();
    let track = slider_track(&h);
    let from = pos2(track.right(), track.center().y);
    let to = pos2(track.left() + 0.3 * track.width(), track.center().y);
    h.hover_at(from);
    h.run_steps(1);
    press(&mut h, from, true);
    h.hover_at(to);
    h.run_steps(2);
    assert!(h.state().app.gradient.panel.is_none());
    assert!((opacity_stops(&h)[0].1 - 0.3).abs() < 1e-5);
    h.state_mut().app.session.add_document((*original_doc).clone(), None);
    h.state_mut().app.session.select_layer(layer).unwrap();
    let next_doc = h.state().app.session.active().unwrap().doc.id;
    assert_ne!(next_doc, original_doc.id);
    let next_index = h.state().app.session.active_index().unwrap();
    assert!(display_doc(&mut h.state_mut().app, next_index).is_none(), "old preview must not render into the new document");
    h.run_steps(2);
    press(&mut h, to, false);
    h.run_steps(2);
    assert_eq!(opacity_stops(&h), vec![(0.0, 1.0), (1.0, 0.7)]);
    assert_eq!(h.state().app.session.active().unwrap().history.past_len(), 0);
    h.state_mut().app.session.set_active(original_index);
    h.run_steps(3);
    assert!((opacity_stops(&h)[0].1 - 0.3).abs() < 1e-5, "original document retains its own edit");
    assert!(h.state().app.gradient.panel.is_none());
    assert_eq!(
        h.query_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap().value().as_deref(),
        Some("30"),
        "number reads the document value rather than a stale preview cache"
    );
}

fn type_opacity_without_enter(h: &mut Harness<'_, Fixture>, text: &str) {
    h.query_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap().click();
    h.run_steps(2);
    h.event(Event::Text(text.into()));
    h.run_steps(2);
}

#[test]
fn typed_stop_opacity_survives_section_collapse_and_reopen_as_a_new_gesture_v77() {
    let mut h = fixture(ThemeKind::StudioLight, 8, false, true);
    h.run_steps(4);
    let before = h.state().app.session.active().unwrap().history.past_len();
    type_opacity_without_enter(&mut h, "25");
    assert!((opacity_stops(&h)[0].1 - 0.25).abs() < 1e-5);
    h.get_by_label("Gradient").click();
    h.run_steps(3);
    assert_eq!(h.query_all_by_role(egui::accesskit::Role::SpinButton).count(), 2, "collapsed stop editor is actually removed");
    assert!(h.state().app.gradient.panel.is_none());
    assert!((opacity_stops(&h)[0].1 - 0.25).abs() < 1e-5);
    assert_eq!(h.state().app.session.active().unwrap().history.past_len(), before + 1);
    h.get_by_label("Gradient").click();
    h.run_steps(3);
    type_opacity_without_enter(&mut h, "55");
    assert!((opacity_stops(&h)[0].1 - 0.55).abs() < 1e-5);
    assert_eq!(h.state().app.session.active().unwrap().history.past_len(), before + 2);
    h.state_mut().app.run("edit.undo", json!({})).unwrap();
    assert!((opacity_stops(&h)[0].1 - 0.25).abs() < 1e-5);
    h.state_mut().app.run("edit.undo", json!({})).unwrap();
    assert_eq!(opacity_stops(&h)[0].1, 1.0);
}

#[test]
fn typed_stop_opacity_survives_hidden_panel_and_same_document_layer_switch_v77() {
    let mut h = fixture(ThemeKind::Pro, 16, false, false);
    h.run_steps(4);
    let gradient = id(&h);
    let background = h.state().app.session.active().unwrap().doc.layers[0].id;
    let before = h.state().app.session.active().unwrap().history.past_len();
    type_opacity_without_enter(&mut h, "25");
    h.state_mut().hidden = true;
    h.run_steps(3);
    assert!(h.query_all_by_role(egui::accesskit::Role::SpinButton).next().is_none());
    assert!(h.state().app.gradient.panel.is_none());
    assert!((opacity_stops(&h)[0].1 - 0.25).abs() < 1e-5);
    h.state_mut().app.session.select_layer(background).unwrap();
    h.state_mut().hidden = false;
    h.run_steps(3);
    assert!(h.query_all_by_role(egui::accesskit::Role::SpinButton).next().is_none());
    let active_index = h.state().app.session.active_index().unwrap();
    assert!(display_doc(&mut h.state_mut().app, active_index).is_none());
    let saved_layer = h.state().app.session.active().unwrap().doc.layer(gradient).unwrap();
    let Fill::Gradient { opacity_stops: saved, .. } = fill_of(saved_layer).unwrap() else { panic!("gradient") };
    assert!((saved[0].1 - 0.25).abs() < 1e-5);
    assert_eq!(h.state().app.session.active().unwrap().history.past_len(), before + 1);
    h.state_mut().app.session.select_layer(gradient).unwrap();
    h.run_steps(3);
    assert_eq!(h.query_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap().value().as_deref(), Some("25"));
    type_opacity_without_enter(&mut h, "55");
    assert!((opacity_stops(&h)[0].1 - 0.55).abs() < 1e-5);
    assert_eq!(h.state().app.session.active().unwrap().history.past_len(), before + 2);
    h.state_mut().app.run("edit.undo", json!({})).unwrap();
    assert!((opacity_stops(&h)[0].1 - 0.25).abs() < 1e-5);
    h.state_mut().app.run("edit.undo", json!({})).unwrap();
    assert_eq!(opacity_stops(&h)[0].1, 1.0);
}

#[test]
#[ignore = "writes five-theme actual GPU properties screenshots for root visual review"]
fn gradient_opacity_properties_gpu_visual_v77() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v77/gradient-opacity-gpu");
    assert!(!out.exists(), "preserve previous evidence");
    std::fs::create_dir_all(&out).unwrap();
    let mut manifest = Vec::new();
    for theme in ThemeKind::ALL {
        let mut h = fixture(theme, 8, true, true);
        let layer = id(&h);
        h.state_mut().app.run(cmds::STOP, json!({"layer":layer.0,"action":"opacity","kind":"opacity","index":0,"opacity":30})).unwrap();
        h.run_steps(6);
        h.get_by_label("Location");
        let file = format!("{}-gradient.png", theme.id());
        h.render().unwrap().save(out.join(&file)).unwrap();
        manifest.push(json!({"file":file,"theme":theme.id(),"actualTokenTheme":Tokens::get(&h.ctx).kind.id(),"selectedStop":0,"opacityPercent":30,"locationPercent":0,"scope":"production properties body; not native window or Photoshop parity"}));
    }
    std::fs::write(out.join("manifest.json"), serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
}
