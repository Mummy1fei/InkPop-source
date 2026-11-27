//! Real Layers-panel icon size, responsive spacing and adjacent Fill control regressions.
use egui::{Rect, Shape, vec2};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::{PhotocraftApp, theme::ThemeKind};

const KINDS: [&str; 5] =
    ["Filter for pixel layers", "Filter for adjustment layers", "Filter for type layers", "Filter for shape layers", "Filter for smart objects"];
const LOCKS: [&str; 5] =
    ["Lock transparent pixels", "Lock image pixels", "Lock position", "Prevent auto-nesting in and out of Artboards and Frames", "Lock all"];

fn harness(theme: ThemeKind, width: f32, language: &str) -> Harness<'static, PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":32,"height":24,"background":"transparent"})).unwrap();
    session.execute("layer.new.layer", json!({"name":"Icon controls"})).unwrap();
    session.execute("prefs.set", json!({"path":"interface.language","value":language})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        PhotocraftApp::new(session, Default::default())
    });
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new(
        "ui.set",
        json!({"theme":theme.id(),"dockWidth":width,"dock":{"collapsed":["color","properties","history","navigator"]}}),
    );
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    h
}

fn large_glyph(shape: &Shape, button: Rect) -> bool {
    let icon = match shape {
        Shape::Rect(shape) if shape.brush.is_some() => Some(shape.rect),
        Shape::Mesh(mesh) if !mesh.vertices.is_empty() => Some(mesh.calc_bounds()),
        Shape::Vec(shapes) => return shapes.iter().any(|shape| large_glyph(shape, button)),
        _ => None,
    };
    icon.is_some_and(|rect| button.contains_rect(rect) && (16.0..=18.0).contains(&rect.width()) && (16.0..=18.0).contains(&rect.height()))
}

fn controls(h: &Harness<'_, PhotocraftApp>, labels: &[&str], lang: crate::i18n::Lang) -> Vec<Rect> {
    labels.iter().map(|label| h.get_by_label(crate::i18n::tr(lang, label)).rect()).collect()
}

fn same_row_inside(rects: &[Rect], bounds: Rect, context: &str) {
    for (index, rect) in rects.iter().enumerate() {
        assert!((rect.height() - 26.0).abs() < 0.1, "{context}: larger hit area: {rect:?}");
        assert!(rect.width() >= 22.0, "{context}: narrow panel still provides a usable target: {rect:?}");
        assert!(
            rect.left() >= bounds.left() - 1.0 && rect.right() <= bounds.right() + 1.0,
            "{context}: control must remain inside its actual available/clipped panel width: {rect:?}/{bounds:?}"
        );
        if index > 0 {
            let previous = rects[index - 1];
            assert!((previous.center().y - rect.center().y).abs() < 0.1, "{context}: no wrapping: {previous:?}/{rect:?}");
            assert!(previous.right() <= rect.left() + 0.01, "{context}: controls cannot overlap: {previous:?}/{rect:?}");
        }
    }
}

#[test]
fn layers_filter_and_lock_glyphs_are_large_and_leave_fill_visible_at_minimum_dock_width_v80() {
    for theme in ThemeKind::ALL {
        for language in ["en", "zh-hans"] {
            let lang = crate::i18n::Lang::from_code(language).unwrap();
            let mut narrow_kind_width = 0.0;
            for width in [250.0, 520.0] {
                let h = harness(theme, width, language);
                let rows = crate::layer_row_ui::recorded(&h.ctx);
                let rows_bounds = rows.first().unwrap().row;
                let bounds = super::controls_bounds(&h.ctx).expect("real header available/clip bounds recorded while drawing");
                let context = format!("{theme:?}/{language}/{width}: header={bounds:?}, scroll-row={rows_bounds:?}");
                assert!(rows_bounds.right() <= bounds.right(), "{context}: the layer list has its own reserved scrollbar gutter");
                let kinds = controls(&h, &KINDS, lang);
                let locks = controls(&h, &LOCKS, lang);
                same_row_inside(&kinds, bounds, &context);
                same_row_inside(&locks, bounds, &context);
                let enable = h.get_by_label(crate::i18n::tr(lang, "Enable layer filtering")).rect();
                assert!((enable.width() - 26.0).abs() < 0.1 && (enable.height() - 26.0).abs() < 0.1, "{context}: enable={enable:?}");
                assert!(
                    kinds.last().unwrap().right() <= enable.left() && enable.right() <= bounds.right() + 1.0,
                    "{context}: kinds={kinds:?}, enable={enable:?}"
                );
                assert!(
                    (enable.center().y - kinds[0].center().y).abs() < 0.1,
                    "{context}: enable must stay on the filter row: kinds={kinds:?}, enable={enable:?}"
                );
                for button in kinds.iter().chain(locks.iter()) {
                    assert!(
                        h.output().shapes.iter().any(|shape| large_glyph(&shape.shape, *button)),
                        "{theme:?}/{language}/{width}: the actual painted SVG must be 16–18pt, independent of its hit box"
                    );
                }
                let fill_label = if crate::theme::Tokens::for_kind(theme).pro { "Fill:" } else { "Fill" };
                let fill = h.get_by_label(crate::i18n::tr(lang, fill_label)).rect();
                assert!(
                    locks.last().unwrap().right() <= fill.left(),
                    "{context}: locks must leave the real Fill label visible: locks={locks:?}, fill={fill:?}"
                );
                assert!(
                    fill.right() + 66.0 <= bounds.right() + 2.0,
                    "{context}: the complete existing Fill value field remains in the panel: locks={locks:?}, fill={fill:?}"
                );
                if width == 250.0 {
                    narrow_kind_width = kinds[0].width();
                } else {
                    assert!(kinds[0].width() > narrow_kind_width + 15.0, "wide panels distribute additional row space across the five controls");
                }
            }
        }
    }
}

#[test]
fn larger_filter_and_lock_hit_boxes_keep_their_real_pointer_actions_v80() {
    for theme in ThemeKind::ALL {
        let mut h = harness(theme, 250.0, "en");
        h.get_by_label("Filter for type layers").click();
        h.run_steps(3);
        assert_eq!(h.state().ui.layer_filter, ["type"]);
        h.get_by_label("Enable layer filtering").click();
        h.run_steps(3);
        assert!(!h.state().ui.layer_filter_options.enabled);
        let revision = h.state().session.active().unwrap().revision;
        h.get_by_label("Lock position").click();
        h.run_steps(3);
        let state = h.state().session.active().unwrap();
        assert!(state.doc.layer(state.active_layer.unwrap()).unwrap().locks.position);
        assert!(state.revision > revision, "larger lock controls still dispatch the real layer command");
    }
}
