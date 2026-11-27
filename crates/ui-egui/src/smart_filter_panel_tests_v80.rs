use super::*;
use crate::{PhotocraftApp, theme::ThemeKind};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

fn scene(theme: ThemeKind, width: f32) -> (Harness<'static, PhotocraftApp>, u64) {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":32,"height":24,"background":"#2468b4"})).unwrap();
    session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let layer = session.active().unwrap().active_layer.unwrap().0;
    session.execute("layer.setProps", json!({"name":"Smart Object / long filter name"})).unwrap();
    for radius in [0.3, 0.4, 0.5] {
        session.execute("filter.blur.gaussianBlur", json!({"radius":radius})).unwrap();
    }
    session.execute("layer.layerStyle.dropShadow", json!({"distance":1,"size":1})).unwrap();
    session.execute("layer.new.layer", json!({"name":"Keep selected"})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        PhotocraftApp::new(session, Default::default())
    });
    let ctx = h.ctx.clone();
    let (req, _) = crate::control::ControlRequest::new(
        "ui.set",
        json!({"theme":theme.id(),"dockWidth":width,"dock":{"collapsed":["color","properties","history","navigator"]}}),
    );
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(6);
    (h, layer)
}

fn pointer_click(h: &mut Harness<'_, PhotocraftApp>, at: egui::Pos2) {
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
}

#[test]
fn real_smart_filter_disclosure_hides_only_subrows_without_render_commands_or_selection_changes_v80() {
    for theme in ThemeKind::ALL {
        for width in [250.0, 520.0] {
            let (mut h, layer) = scene(theme, width);
            let before_doc = h.state().session.active().unwrap().doc.clone();
            let before_revision = h.state().session.active().unwrap().revision;
            let before_active = h.state().session.active().unwrap().active_layer;
            let before_history = h.state().session.active().unwrap().history.past_len();
            let journal = h.state().session.journal.len();
            let display = h.state().ui.smart_filters.layers.iter().find(|row| row.layer == layer).unwrap();
            assert!(display.expanded && display.filter_count == 3);
            assert!(display.disclosure.width() >= 18.0 && display.row.contains_rect(display.disclosure));
            let disclosure = display.disclosure;
            let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|row| row.layer == layer).unwrap();
            assert!(row.name.unwrap().right() < disclosure.left());
            for (kind, rect) in row.indicators {
                if kind != crate::layer_row_ui::Indicator::SmartFiltersTriangle {
                    assert!(!rect.intersects(disclosure));
                }
            }
            let rows: Vec<_> = h.state().ui.smart_filters.rows.iter().filter(|row| row.layer == layer).collect();
            assert_eq!(rows.iter().map(|row| row.index).collect::<Vec<_>>(), vec![None, Some(2), Some(1), Some(0)]);
            let header_eye = rows[0].eye;
            for row in rows {
                let name = row.name.unwrap();
                assert!(row.eye.left() > row.row.left() + 30.0);
                assert!(row.eye.right() <= name.left() && name.right() <= row.row.right());
                assert!(name.left() - row.eye.right() <= 30.0);
                if row.index.is_some() {
                    assert!(row.eye.left() > header_eye.left());
                }
            }
            pointer_click(&mut h, disclosure.center());
            assert!(!h.state().ui.smart_filters.layers.iter().find(|row| row.layer == layer).unwrap().expanded);
            assert!(!h.state().ui.smart_filters.rows.iter().any(|row| row.layer == layer));
            let after = h.state().session.active().unwrap();
            assert_eq!(after.doc, before_doc);
            assert_eq!(after.revision, before_revision);
            assert_eq!(after.active_layer, before_active);
            assert_eq!(after.history.past_len(), before_history);
            assert_eq!(h.state().session.journal.len(), journal);
            h.get_by_label(&format!("{} {} {}", tl!("Expand"), tl!("Smart Filters"), "Smart Object / long filter name")).click();
            h.run_steps(3);
            assert_eq!(h.state().ui.smart_filters.rows.iter().filter(|row| row.layer == layer).count(), 4);
            assert_eq!(h.state().session.active().unwrap().doc, before_doc);
        }
    }
}

#[test]
fn rapid_eye_clicks_keep_filter_editor_closed_and_toggle_only_the_requested_visibility_v80() {
    let (mut h, layer) = scene(ThemeKind::StudioLight, 330.0);
    let original = h.state().session.active().unwrap().doc.clone();
    for index in [None, Some(2), Some(1), Some(0)] {
        let eye = h.state().ui.smart_filters.rows.iter().find(|row| row.layer == layer && row.index == index).unwrap().eye;
        pointer_click(&mut h, eye.center());
        assert!(h.state().ui.dialogs.is_empty() && h.state().camera_raw.is_none());
        let state = h.state().session.active().unwrap();
        let LayerContent::Smart(smart) = &state.doc.layer(photocraft_doc::LayerId(layer)).unwrap().content else { panic!("smart layer") };
        if let Some(index) = index {
            assert!(!smart.smart_filters[index].visible && smart.filters_enabled);
        } else {
            assert!(!smart.filters_enabled);
        }
        pointer_click(&mut h, eye.center());
        assert!(h.state().ui.dialogs.is_empty() && h.state().camera_raw.is_none());
        assert_eq!(h.state().session.active().unwrap().doc, original);
    }
}

#[test]
fn expansion_command_validates_without_partial_changes_and_is_scoped_by_document_v80() {
    let (mut h, layer) = scene(ThemeKind::Pro, 330.0);
    let ctx = h.ctx.clone();
    let before = h.state().session.active().unwrap().doc.clone();
    let state_before = h.state().ui.smart_filters.clone();
    for params in [
        json!({}),
        json!({"expanded":1}),
        json!({"expanded":false,"layer":-1}),
        json!({"expanded":false,"unknown":true}),
        json!({"expanded":false,"layer":999999}),
        json!([]),
    ] {
        assert!(crate::menus::invoke(h.state_mut(), &ctx, SET_EXPANDED, params).is_err());
        assert_eq!(h.state().ui.smart_filters, state_before);
        assert_eq!(h.state().session.active().unwrap().doc, before);
    }
    crate::menus::invoke(h.state_mut(), &ctx, SET_EXPANDED, json!({"layer":layer,"expanded":false})).unwrap();
    let old_doc = h.state().session.active().unwrap().doc.id;
    h.state_mut().run("file.new", json!({"width":32,"height":24})).unwrap();
    assert!(expanded(h.state(), photocraft_doc::LayerId(layer)), "another document cannot inherit the fold for the same layer ID");
    h.state_mut().session.set_active(0);
    assert_eq!(h.state().session.active().unwrap().doc.id, old_doc);
    assert!(!expanded(h.state(), photocraft_doc::LayerId(layer)));
    let stored = serde_json::to_value(&h.state().ui).unwrap();
    assert!(stored.get("smart_filters").is_none(), "do not persist temporary DocIds as workspace identity");
}
