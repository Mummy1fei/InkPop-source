use super::*;
use egui::{Event, Modifiers, PointerButton, Rect, Shape};
use egui_kittest::{Harness, kittest::Queryable};
use std::sync::Arc;

struct Fixture {
    app: PhotocraftApp,
    fields: Map<String, Value>,
}

fn harness(theme: crate::theme::ThemeKind, strokes: bool) -> Harness<'static, Fixture> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":64,"height":64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    if strokes {
        app.run("layer.layerStyle.stroke", json!({"size":3})).unwrap();
        app.run("layer.layerStyle.stroke", json!({"size":7,"add":true})).unwrap();
    }
    let st = app.session.active().unwrap();
    let fields = initial_fields(st.doc.layer(st.active_layer.unwrap()).unwrap(), Some(BLENDING), st.doc.global_light.angle);
    let mut h = Harness::builder().with_size(vec2(640., 600.)).with_max_steps(64).build_ui_state(
        |ui, f: &mut Fixture| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let previous = crate::i18n::current();
            crate::i18n::set_current(crate::i18n::Lang::from_code("en").unwrap());
            egui::CentralPanel::default().show(ui, |ui| body(&mut f.app, ui, &mut f.fields));
            crate::i18n::set_current(previous);
        },
        Fixture { app, fields },
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(6);
    h
}

fn checkboxes(h: &Harness<'_, Fixture>) -> Vec<Rect> {
    let mut rects: Vec<_> = h.query_all_by_role(egui::accesskit::Role::CheckBox).map(|node| node.rect()).filter(|r| r.right() < 210.).collect();
    rects.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
    rects
}

fn checkbox_shapes(shape: &Shape, clip: Rect, out: &mut Vec<(Rect, Color32, f32)>) {
    match shape {
        Shape::Rect(r) if (r.rect.width() - 14.).abs() < 0.01 && (r.rect.height() - 14.).abs() < 0.01 && r.rect.right() < 210. => {
            assert!(clip.contains_rect(r.rect), "checkbox must be visible, not clipped");
            out.push((r.rect, r.fill, r.stroke.width));
        }
        Shape::Vec(shapes) => {
            for shape in shapes {
                checkbox_shapes(shape, clip, out);
            }
        }
        _ => {}
    }
}

fn painted(h: &Harness<'_, Fixture>) -> Vec<(Rect, Color32, f32)> {
    let mut out = vec![];
    for s in &h.output().shapes {
        checkbox_shapes(&s.shape, s.clip_rect, &mut out);
    }
    out
}

fn click(h: &mut Harness<'_, Fixture>, index: usize) {
    let pos = checkboxes(h)[index].center();
    h.hover_at(pos);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.hover_at(egui::pos2(630., 590.));
    h.run_steps(2);
}

#[test]
fn every_unconfigured_effect_has_a_visible_idle_checkbox_in_all_themes() {
    for theme in crate::theme::ThemeKind::ALL {
        let h = harness(theme, false);
        assert!(effects_of(&h.state().fields).is_empty(), "opening cannot instantiate effects");
        let rects = checkboxes(&h);
        assert_eq!(rects.len(), KINDS.len(), "{theme:?}: all ten effects have switches");
        let shapes = painted(&h);
        for rect in rects {
            assert!(shapes.iter().any(|(r, _, width)| *r == rect && *width >= 1.), "{theme:?}: missing unchecked border at {rect:?}");
        }
    }
}

#[test]
fn idle_checkbox_adds_once_then_disables_and_reenables_without_editing_the_layer() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = harness(theme, false);
        let before = h.state().app.session.active().unwrap().doc.clone();
        let history = h.state().app.session.active().unwrap().history.past_len();
        for (index, (kind, _)) in KINDS.iter().enumerate() {
            click(&mut h, index);
            assert_eq!(effects_of(&h.state().fields).len(), index + 1, "one click adds one instance");
            let item = effects_of(&h.state().fields).iter().find(|entry| entry["kind"] == *kind).unwrap();
            let id = item["id"].as_str().unwrap().to_owned();
            assert_eq!(item["on"], true);
            assert_eq!(h.state().fields["selected"], id);
            let rect = checkboxes(&h)[index];
            assert!(painted(&h).iter().any(|(r, fill, _)| *r == rect && *fill == Tokens::for_kind(theme).accent));
            click(&mut h, index);
            assert_eq!(entry(&h.state().fields, &id).unwrap()["on"], false);
            assert!(painted(&h).iter().any(|(r, _, width)| *r == checkboxes(&h)[index] && *width >= 1.));
            click(&mut h, index);
            assert_eq!(entry(&h.state().fields, &id).unwrap()["on"], true);
            assert_eq!(effects_of(&h.state().fields).len(), index + 1, "reenabling must not duplicate instances");
            click(&mut h, index);
        }
        assert!(Arc::ptr_eq(&before, &h.state().app.session.active().unwrap().doc));
        assert_eq!(h.state().app.session.active().unwrap().history.past_len(), history, "checkbox changes remain a cancelable draft");
    }
}

#[test]
fn disabled_style_checkbox_and_duplicate_instances_keep_their_own_parameters() {
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = harness(theme, true);
        assert_eq!(checkboxes(&h).len(), KINDS.len() + 1);
        click(&mut h, 1);
        let items = effects_of(&h.state().fields);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["on"], false);
        assert_eq!(items[1]["on"], true);
        assert_eq!(items[0]["params"]["size"], 3.);
        assert_eq!(items[1]["params"]["size"], 7.);
        let f = h.state_mut();
        confirm(&mut f.app, &f.fields).unwrap();
        let st = f.app.session.active().unwrap();
        let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
        assert!(!layer.effects.items[0].enabled());
        assert!(layer.effects.items[1].enabled());
        f.fields = initial_fields(layer, Some(BLENDING), st.doc.global_light.angle);
        h.run_steps(3);
        assert_eq!(effects_of(&h.state().fields)[0]["on"], false, "reopen preserves disabled state");
        assert_eq!(checkboxes(&h).len(), KINDS.len() + 1);
    }
}
