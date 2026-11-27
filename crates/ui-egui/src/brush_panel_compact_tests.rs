//! Compact Brush Settings keeps persistent lock controls and a visible stroke preview.

use egui::{Color32, Event, Modifiers, PointerButton, Rect, Shape, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use super::*;
use crate::theme::ThemeKind;

fn harness(kind: ThemeKind, size: egui::Vec2, lang: crate::i18n::Lang) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.brush_section = 1;
    app.session.tools.brush.shape_dynamics.enabled = true;
    app.session.tools.brush.shape_dynamics.size.control = paint::Control::Fade;
    let mut h = Harness::builder().with_size(size).with_max_steps(64).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let previous = crate::i18n::current();
            crate::i18n::set_current(lang);
            egui::CentralPanel::default().show(ui, |ui| settings_tab(app, ui));
            crate::i18n::set_current(previous);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, kind);
    h.run_steps(8);
    h
}

fn textures(shape: &Shape, out: &mut Vec<(Rect, Color32)>) {
    match shape {
        Shape::Rect(r) if r.brush.is_some() => out.push((r.rect, r.fill)),
        Shape::Mesh(m) if !m.vertices.is_empty() => out.push((m.calc_bounds(), m.vertices[0].color)),
        Shape::Vec(shapes) => shapes.iter().for_each(|s| textures(s, out)),
        _ => {}
    }
}

fn drawn_textures(h: &Harness<'_, PhotocraftApp>) -> Vec<(Rect, Color32)> {
    let mut out = Vec::new();
    for s in &h.output().shapes {
        textures(&s.shape, &mut out);
    }
    out
}

fn check_preview(h: &Harness<'_, PhotocraftApp>, size: egui::Vec2) {
    let images = drawn_textures(h);
    let preview = images.iter().find(|(r, _)| r.width() > 150.0 && (36.0..=64.1).contains(&r.height())).expect("stroke preview drawn").0;
    assert!(preview.bottom() <= size.y && preview.right() <= size.x, "preview clipped: {preview:?}, panel {size:?}");
}

#[test]
fn section_locks_are_visible_at_rest_and_toggle_without_selecting_in_every_theme() {
    for kind in ThemeKind::ALL {
        let size = vec2(500.0, 520.0);
        let mut h = harness(kind, size, crate::i18n::Lang::from_code("en").unwrap());
        let images = drawn_textures(&h);
        let mut first = None;
        let mut last = None;
        for (name, _) in SECTIONS.iter().skip(1) {
            let lock = h.get_by_label(&format!("Lock {name}")).rect();
            assert!(images.iter().any(|(r, _)| lock.contains_rect(*r) && r.width() == 12.0), "{kind:?}: idle lock missing for {name}");
            assert!(lock.right() <= 140.0, "{kind:?}: category column is compact");
            first.get_or_insert(lock);
            last = Some(lock);
        }
        assert!(last.unwrap().center().y - first.unwrap().center().y <= 244.0, "category rows remain compact");
        check_preview(&h, size);
        let p = h.get_by_label("Lock Scattering").rect().center();
        h.hover_at(p);
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.run_steps(1);
        }
        h.hover_at(egui::pos2(490.0, 510.0));
        h.run_steps(4);
        assert!(h.state().session.tools.brush.locks.scattering);
        assert_eq!(h.state().ui.brush_section, 1, "lock click does not switch to Scattering");
        assert!(!h.state().session.tools.brush.scattering.enabled, "lock click does not enable a category");
        let lock = h.get_by_label("Lock Scattering").rect();
        assert!(drawn_textures(&h).iter().any(|(r, tint)| lock.contains_rect(*r) && *tint == Tokens::for_kind(kind).text));
        assert_eq!(h.state().session.journal.last().unwrap().0, "tools.setBrush");
    }
}

fn text_bounds(shape: &Shape, out: &mut Vec<Rect>) {
    match shape {
        Shape::Text(t) => out.push(Rect::from_min_size(t.pos, t.galley.size())),
        Shape::Vec(shapes) => shapes.iter().for_each(|s| text_bounds(s, out)),
        _ => {}
    }
}

#[test]
fn translated_categories_keep_a_clear_lock_column_and_small_panels_keep_preview() {
    for kind in ThemeKind::ALL {
        for code in ["zh-hans", "de"] {
            let lang = crate::i18n::Lang::from_code(code).unwrap();
            let h = harness(kind, vec2(500.0, 520.0), lang);
            let mut text = Vec::new();
            for shape in &h.output().shapes {
                text_bounds(&shape.shape, &mut text);
            }
            for (name, _) in SECTIONS.iter().skip(1) {
                let label = format!("{} {}", crate::i18n::tr(lang, "Lock"), crate::i18n::tr(lang, name));
                let lock = h.get_by_label(&label).rect();
                for t in text.iter().filter(|r| r.left() < lock.left() && (r.center().y - lock.center().y).abs() < 2.0) {
                    assert!(t.right() <= lock.left(), "{kind:?}/{code}: category text {t:?} overlaps lock {lock:?}");
                }
            }
            let small = vec2(280.0, 250.0);
            let h = harness(kind, small, lang);
            check_preview(&h, small);
            let label = format!("{} {}", crate::i18n::tr(lang, "Lock"), crate::i18n::tr(lang, "Shape Dynamics"));
            let lock = h.get_by_label(&label).rect();
            assert!(drawn_textures(&h).iter().any(|(r, _)| lock.contains_rect(*r)), "compact mode still paints its lock at rest");
        }
    }
}
