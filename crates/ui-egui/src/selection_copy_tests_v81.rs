//! Pointer-path regression: Alt copies only selected pixels; Shift still constrains the move.
use super::*;
use photocraft_color::PixelFormat;
use photocraft_doc::{Document, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

const AREA: Rect = Rect::new(0, 0, 96, 64);
const SOURCE: Rect = Rect::new(8, 8, 16, 16);
const COPY: egui::Modifiers = egui::Modifiers::ALT;
const STRAIGHT_COPY: egui::Modifiers = egui::Modifiers { shift: true, alt: true, ..egui::Modifiers::NONE };

fn fixture(depth: u32, soft: bool) -> (PhotocraftApp, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width":96,"height":64,"depth":depth,"background":"transparent"})).unwrap();
    app.sync_views();
    let id = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("synthetic source", |doc, _| {
            let surface = doc.layer_mut(id).unwrap().surface_mut().unwrap();
            surface.fill_rect(SOURCE, &[1.0, 0.0, 0.0, if soft { 0.6 } else { 1.0 }]);
            // This island is outside the selection and must never be copied or moved.
            surface.fill_rect(Rect::new(8, 32, 16, 40), &[0.0, 1.0, 0.0, 1.0]);
            let mut selection = Surface::new(PixelFormat::GRAY8);
            selection.fill_rect(SOURCE, &[if soft { 0.5 } else { 1.0 }]);
            doc.selection = Some(selection);
            Ok(())
        })
        .unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_auto_select = true;
    app.ui.extras.snap = false;
    app.ui.view.show.smart_guides = false;
    (app, id)
}

fn pixel_bits(doc: &Document, id: LayerId, area: Rect) -> Vec<u32> {
    doc.layer(id).unwrap().surface().unwrap().read_region(area).iter().map(|v| v.to_bits()).collect()
}

fn selection_bits(doc: &Document) -> Vec<u32> {
    doc.selection.as_ref().unwrap().read_region(AREA).iter().map(|v| v.to_bits()).collect()
}

fn event(app: &mut PhotocraftApp, kind: &str, point: [f64; 2], mods: egui::Modifiers) {
    let [x, y] = point;
    let ev = match kind {
        "down" => ToolEvent::Down { x, y, pressure: 0.7 },
        "move" => ToolEvent::Move { x, y, pressure: 0.7 },
        _ => ToolEvent::Up { x, y },
    };
    tool_event(app, ev, mods);
}

#[test]
fn alt_selection_drag_copies_pixels_live_without_picking_or_duplicating_layers_v81() {
    for depth in [8, 16, 32] {
        for soft in [false, true] {
            for (mods, end, offset) in [(COPY, [34.0, 16.0], (24, 6)), (STRAIGHT_COPY, [34.0, 16.0], (24, 0))] {
                let (mut app, id) = fixture(depth, soft);
                app.session.execute("layer.new.layer", json!({"name":"Unselected blue layer above"})).unwrap();
                let other = app.session.active().unwrap().active_layer.unwrap();
                app.session
                    .edit("other layer", |doc, _| {
                        doc.layer_mut(other).unwrap().surface_mut().unwrap().fill_rect(AREA, &[0.0, 0.0, 1.0, 1.0]);
                        Ok(())
                    })
                    .unwrap();
                app.session.execute("layer.select", json!({"layer":id.0})).unwrap();
                let st = app.session.active().unwrap();
                let before = pixel_bits(&st.doc, id, AREA);
                let before_selection = selection_bits(&st.doc);
                let other_before = pixel_bits(&st.doc, other, AREA);
                let history = st.history.past_len();
                let layers = st.doc.layers.len();
                let format = st.doc.layer(id).unwrap().surface().unwrap().format();
                let revision = st.revision;
                let journal = app.session.journal.len();
                event(&mut app, "down", [10.0, 10.0], mods);
                let first = crate::move_ui::display_doc(&mut app, 0).unwrap().0;
                assert_eq!(pixel_bits(&first, id, AREA), before, "zero-position copy is an identity");
                event(&mut app, "move", end, mods);
                let preview = crate::move_ui::display_doc(&mut app, 0).unwrap().0;
                assert_eq!(crate::move_ui::floating_offset(&app), Some(offset));
                let st = app.session.active().unwrap();
                assert_eq!(st.active_layer, Some(id), "auto-select must not steal selected pixels");
                assert_eq!(st.doc.layers.len(), layers, "selected-pixel copying is not layer.duplicate");
                assert_eq!(st.history.past_len(), history);
                assert_eq!(st.revision, revision);
                assert_eq!(pixel_bits(&st.doc, id, AREA), before, "preview is not a document edit");
                assert_eq!(pixel_bits(&preview, id, SOURCE), pixel_bits(&st.doc, id, SOURCE), "source remains bit-exact");
                let surf = preview.layer(id).unwrap().surface().unwrap();
                let original = surf.rgba(10, 10);
                let destination = surf.rgba(10 + offset.0, 10 + offset.1);
                assert_eq!(destination[0], 1.0);
                assert_eq!(destination[1], 0.0);
                let coverage = if soft { 128.0 / 255.0 } else { 1.0 };
                assert!((destination[3] - original[3] * coverage).abs() <= 1.0 / 255.0, "{depth}/{soft}: actual selected coverage only");
                assert_eq!(surf.rgba(10, 34), [0.0, 1.0, 0.0, 1.0]);
                assert_eq!(surf.rgba(10 + offset.0, 34 + offset.1)[3], 0.0, "unselected island was not copied");
                event(&mut app, "up", end, mods);
                let st = app.session.active().unwrap();
                assert!(st.floating.is_none());
                assert_eq!(st.history.past_len(), history + 1);
                assert_eq!(st.doc.layers.len(), layers);
                assert_eq!(st.doc.layer(id).unwrap().surface().unwrap().format(), format);
                assert_eq!(pixel_bits(&st.doc, id, AREA), pixel_bits(&preview, id, AREA));
                assert_eq!(pixel_bits(&st.doc, other, AREA), other_before);
                assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), SOURCE.translate(offset.0, offset.1));
                assert!(app.session.journal[journal..].iter().all(|(cmd, _)| !matches!(cmd.as_str(), "layer.duplicate" | "layer.translate" | "layer.pickAt")));
                let after = pixel_bits(&st.doc, id, AREA);
                let after_selection = selection_bits(&st.doc);
                assert!(app.session.undo());
                assert_eq!(pixel_bits(&app.session.active().unwrap().doc, id, AREA), before);
                assert_eq!(selection_bits(&app.session.active().unwrap().doc), before_selection);
                assert!(app.session.redo());
                assert_eq!(pixel_bits(&app.session.active().unwrap().doc, id, AREA), after);
                assert_eq!(selection_bits(&app.session.active().unwrap().doc), after_selection);
            }
        }
    }
}

#[test]
fn alt_selection_click_cancel_and_return_to_origin_do_not_change_pixels_or_history_v81() {
    for mode in ["click", "cancel", "return"] {
        let (mut app, id) = fixture(16, true);
        let before = pixel_bits(&app.session.active().unwrap().doc, id, AREA);
        let selected = selection_bits(&app.session.active().unwrap().doc);
        let history = app.session.active().unwrap().history.past_len();
        event(&mut app, "down", [10.0, 10.0], STRAIGHT_COPY);
        if mode != "click" {
            event(&mut app, "move", [34.0, 16.0], STRAIGHT_COPY);
            assert!(crate::move_ui::display_doc(&mut app, 0).is_some());
        }
        if mode == "cancel" {
            assert!(cancel_selection_move(&mut app));
            event(&mut app, "up", [34.0, 16.0], STRAIGHT_COPY);
        } else {
            event(&mut app, "up", [10.0, 10.0], STRAIGHT_COPY);
        }
        let st = app.session.active().unwrap();
        assert!(st.floating.is_none() && app.drag.is_none(), "{mode}");
        assert_eq!(st.history.past_len(), history, "{mode}");
        assert_eq!(pixel_bits(&st.doc, id, AREA), before, "{mode}");
        assert_eq!(selection_bits(&st.doc), selected, "{mode}");
    }
}

#[test]
fn copy_mode_is_latched_at_press_and_shift_can_be_released_while_dragging_v81() {
    let (mut app, id) = fixture(32, false);
    event(&mut app, "down", [10.0, 10.0], STRAIGHT_COPY);
    event(&mut app, "move", [34.0, 16.0], STRAIGHT_COPY);
    assert_eq!(crate::move_ui::floating_offset(&app), Some((24, 0)));
    event(&mut app, "up", [34.0, 16.0], egui::Modifiers::NONE);
    let st = app.session.active().unwrap();
    let surf = st.doc.layer(id).unwrap().surface().unwrap();
    assert_eq!(surf.rgba(10, 10), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(surf.rgba(34, 16), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), SOURCE.translate(24, 6));
}

#[test]
fn alt_selected_pixel_nudge_uses_copy_mode_and_locked_drag_does_not_duplicate_layers_v81() {
    let (mut app, id) = fixture(16, false);
    let history = app.session.active().unwrap().history.past_len();
    crate::move_mods::nudge(&mut app, 24.0, 0.0, true);
    let st = app.session.active().unwrap();
    let surf = st.doc.layer(id).unwrap().surface().unwrap();
    assert_eq!(surf.rgba(10, 10), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(surf.rgba(34, 10), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(st.history.past_len(), history + 1);
    assert_eq!(st.doc.layers.len(), 1);
    for lock in ["all", "pixels", "position", "transparency"] {
        let (mut app, id) = fixture(8, false);
        app.session.execute("layer.lockLayers", json!({(lock):true})).unwrap();
        let before = pixel_bits(&app.session.active().unwrap().doc, id, AREA);
        let history = app.session.active().unwrap().history.past_len();
        event(&mut app, "down", [10.0, 10.0], STRAIGHT_COPY);
        event(&mut app, "move", [34.0, 16.0], STRAIGHT_COPY);
        event(&mut app, "up", [34.0, 16.0], STRAIGHT_COPY);
        let st = app.session.active().unwrap();
        assert!(app.ui.status_error && app.drag.is_none() && st.floating.is_none(), "{lock}");
        assert_eq!(pixel_bits(&st.doc, id, AREA), before, "{lock}");
        assert_eq!(st.history.past_len(), history, "{lock}");
        assert_eq!(st.doc.layers.len(), 1, "{lock}");
    }
}

#[test]
fn actual_canvas_primary_drag_with_shift_alt_copies_the_selected_region_in_all_themes_v81() {
    use egui::{Event, PointerButton};
    use egui_kittest::Harness;
    for theme in crate::theme::ThemeKind::ALL {
        let (app, id) = fixture(8, false);
        let mut h = Harness::builder().with_size(egui::vec2(900.0, 650.0)).with_max_steps(64).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| document_area(app, ui));
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, theme);
        h.run_steps(4);
        h.state_mut().ui.views[0].zoom = 4.0;
        h.state_mut().ui.views[0].center = [48.0, 32.0];
        h.state_mut().ui.views[0].fit_pending = false;
        h.run_steps(3);
        let start = ViewXform::active(h.state()).unwrap().to_screen(10.0, 10.0);
        let end = ViewXform::active(h.state()).unwrap().to_screen(34.0, 16.0);
        let history = h.state().session.active().unwrap().history.past_len();
        h.event(Event::ModifiersChanged(STRAIGHT_COPY));
        h.event(Event::PointerMoved(start));
        h.event(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: STRAIGHT_COPY });
        h.run_steps(4);
        h.event(Event::PointerMoved(end));
        h.run_steps(4);
        assert_eq!(crate::move_ui::floating_offset(h.state()), Some((24, 0)), "{theme:?}: real widget dragging uses the same selected-pixel path");
        h.event(Event::PointerButton { pos: end, button: PointerButton::Primary, pressed: false, modifiers: STRAIGHT_COPY });
        h.run_steps(4);
        let st = h.state().session.active().unwrap();
        assert!(st.floating.is_none() && h.state().drag.is_none(), "{theme:?}");
        let surf = st.doc.layer(id).unwrap().surface().unwrap();
        assert_eq!(surf.rgba(10, 10), [1.0, 0.0, 0.0, 1.0], "{theme:?}");
        assert_eq!(surf.rgba(34, 10), [1.0, 0.0, 0.0, 1.0], "{theme:?}");
        assert_eq!(surf.rgba(34, 34)[3], 0.0, "{theme:?}: unselected pixels are not copied");
        assert_eq!(st.history.past_len(), history + 1, "{theme:?}");
        assert_eq!(st.doc.layers.len(), 1, "{theme:?}");
    }
}
