//! Move must route pixel selections through the existing float/drop engine path.
use super::*;
use photocraft_color::PixelFormat;
use photocraft_doc::{Document, LayerId};
use photocraft_raster::Surface;

const AREA: photocraft_geom::Rect = photocraft_geom::Rect::new(0, 0, 64, 48);

fn app(depth: u32, soft: bool) -> (PhotocraftApp, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.session.execute("file.new", json!({"width":64,"height":48,"depth":depth,"background":"transparent"})).unwrap();
    app.sync_views();
    let id = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("fixture", |doc, _| {
            let surface = doc.layer_mut(id).unwrap().surface_mut().unwrap();
            surface.fill_rect(photocraft_geom::Rect::new(8, 8, 16, 16), &[1.0, 0.0, 0.0, if soft { 0.6 } else { 1.0 }]);
            surface.fill_rect(photocraft_geom::Rect::new(32, 8, 40, 16), &[0.0, 1.0, 0.0, 1.0]);
            let mut selection = Surface::new(PixelFormat::GRAY8);
            selection.fill_rect(photocraft_geom::Rect::new(8, 8, 16, 16), &[if soft { 0.5 } else { 1.0 }]);
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

fn pixels(doc: &Document, id: LayerId) -> Vec<u32> {
    doc.layer(id).unwrap().surface().unwrap().read_region(AREA).iter().map(|v| v.to_bits()).collect()
}
fn selection(doc: &Document) -> Vec<u32> {
    doc.selection.as_ref().unwrap().read_region(AREA).iter().map(|v| v.to_bits()).collect()
}
fn down(app: &mut PhotocraftApp, mods: egui::Modifiers) {
    tool_event(app, ToolEvent::Down { x: 10.0, y: 10.0, pressure: 1.0 }, mods);
}
fn movement(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Move { x: 22.0, y: 14.0, pressure: 1.0 }, egui::Modifiers::NONE);
}
fn up(app: &mut PhotocraftApp) {
    tool_event(app, ToolEvent::Up { x: 22.0, y: 14.0 }, egui::Modifiers::NONE);
}

#[test]
fn selected_pixels_move_preview_commit_undo_redo_match_at_all_rgb_depths() {
    for depth in [8, 16, 32] {
        for soft in [false, true] {
            let (mut app, id) = app(depth, soft);
            // Another layer under the pointer must not steal this selection.
            app.session.execute("layer.new.layer", json!({})).unwrap();
            let other = app.session.active().unwrap().active_layer.unwrap();
            app.session
                .edit("other fixture", |doc, _| {
                    doc.layer_mut(other).unwrap().surface_mut().unwrap().fill_rect(AREA, &[0.0, 0.0, 1.0, 1.0]);
                    Ok(())
                })
                .unwrap();
            app.session.execute("layer.select", json!({"layer":id.0})).unwrap();
            let st = app.session.active().unwrap();
            let before = pixels(&st.doc, id);
            let before_selection = selection(&st.doc);
            let other_before = pixels(&st.doc, other);
            let history = st.history.past_len();
            let format = st.doc.layer(id).unwrap().surface().unwrap().format();
            let journal = app.session.journal.len();
            down(&mut app, egui::Modifiers::NONE);
            assert_eq!(app.session.active().unwrap().active_layer, Some(id));
            assert_eq!(pixels(&app.session.active().unwrap().doc, id), before);
            let zero = crate::move_ui::display_doc(&mut app, 0).unwrap().0;
            assert_eq!(pixels(&zero, id), before, "{depth}/{soft}: first preview is exact");
            movement(&mut app);
            let preview = crate::move_ui::display_doc(&mut app, 0).unwrap().0;
            assert_eq!(crate::move_ui::floating_offset(&app), Some((12, 4)));
            assert_eq!(pixels(&app.session.active().unwrap().doc, id), before, "preview never edits document");
            up(&mut app);
            let st = app.session.active().unwrap();
            assert!(st.floating.is_none());
            assert_eq!(st.history.past_len(), history + 1);
            assert_eq!(pixels(&st.doc, id), pixels(&preview, id), "{depth}/{soft}: release matches shown pixels");
            assert_eq!(pixels(&st.doc, other), other_before);
            assert_eq!(st.doc.layer(id).unwrap().surface().unwrap().format(), format);
            assert_eq!(st.doc.layer(id).unwrap().surface().unwrap().rgba(34, 10), [0.0, 1.0, 0.0, 1.0], "unselected island stays fixed");
            assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), photocraft_geom::Rect::new(20, 12, 28, 20));
            assert!(app.session.journal[journal..].iter().all(|(id, _)| id != "layer.translate" && id != "layer.pickAt"));
            let after = pixels(&st.doc, id);
            let after_selection = selection(&st.doc);
            app.session.undo();
            assert_eq!(pixels(&app.session.active().unwrap().doc, id), before);
            assert_eq!(selection(&app.session.active().unwrap().doc), before_selection);
            app.session.redo();
            assert_eq!(pixels(&app.session.active().unwrap().doc, id), after);
            assert_eq!(selection(&app.session.active().unwrap().doc), after_selection);
        }
    }
}

#[test]
fn click_and_cancel_selected_pixel_drag_are_exact_and_history_free() {
    for depth in [8, 16, 32] {
        let (mut app, id) = app(depth, true);
        let before = pixels(&app.session.active().unwrap().doc, id);
        let selected = selection(&app.session.active().unwrap().doc);
        let history = app.session.active().unwrap().history.past_len();
        down(&mut app, egui::Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Up { x: 10.0, y: 10.0 }, egui::Modifiers::NONE);
        assert!(app.session.active().unwrap().floating.is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
        down(&mut app, egui::Modifiers::NONE);
        movement(&mut app);
        assert!(crate::move_ui::display_doc(&mut app, 0).is_some());
        assert!(cancel_selection_move(&mut app));
        up(&mut app); // A stale release after Escape cannot commit anything.
        let st = app.session.active().unwrap();
        assert!(st.floating.is_none() && app.drag.is_none());
        assert_eq!(st.history.past_len(), history);
        assert_eq!(pixels(&st.doc, id), before);
        assert_eq!(selection(&st.doc), selected);
    }
}

#[test]
fn invalid_selection_targets_never_fall_back_to_whole_layer_movement() {
    for kind in ["empty-selection", "empty-pixels", "all-lock", "pixel-lock", "position-lock", "alpha-lock", "multi", "group"] {
        let (mut app, id) = app(8, false);
        match kind {
            "multi" => {
                app.session.execute("layer.new.layer", json!({})).unwrap();
                app.session.execute("layer.select", json!({"layer":id.0,"mode":"add"})).unwrap();
            }
            "group" => {
                app.session.execute("layer.new.groupFromLayers", json!({"name":"G"})).unwrap();
            }
            _ => {
                app.session
                    .edit("invalid fixture", |doc, _| {
                        match kind {
                            "empty-selection" => doc.selection = Some(Surface::new(PixelFormat::GRAY8)),
                            "empty-pixels" => {
                                let mut selection = Surface::new(PixelFormat::GRAY8);
                                selection.fill_rect(photocraft_geom::Rect::new(50, 30, 54, 34), &[1.0]);
                                doc.selection = Some(selection);
                            }
                            "all-lock" => doc.layer_mut(id).unwrap().locks.all = true,
                            "pixel-lock" => doc.layer_mut(id).unwrap().locks.pixels = true,
                            "position-lock" => doc.layer_mut(id).unwrap().locks.position = true,
                            "alpha-lock" => doc.layer_mut(id).unwrap().locks.transparency = true,
                            _ => {}
                        }
                        Ok(())
                    })
                    .unwrap();
            }
        }
        let st = app.session.active().unwrap();
        let before = pixels(&st.doc, id);
        let selected = selection(&st.doc);
        let history = st.history.past_len();
        let active = st.active_layer;
        let count = st.doc.layers.len();
        down(&mut app, egui::Modifiers::NONE);
        movement(&mut app);
        up(&mut app);
        let st = app.session.active().unwrap();
        assert!(app.ui.status_error, "{kind}: unsupported target should explain rejection");
        assert!(app.drag.is_none() && st.floating.is_none(), "{kind}");
        assert_eq!(st.history.past_len(), history, "{kind}");
        assert_eq!(st.active_layer, active, "{kind}");
        assert_eq!(st.doc.layers.len(), count, "{kind}");
        assert_eq!(pixels(&st.doc, id), before, "{kind}");
        assert_eq!(selection(&st.doc), selected, "{kind}");
    }
}

#[test]
fn move_nudge_uses_selected_pixels_and_never_translates_the_whole_layer() {
    let (mut app, id) = app(16, false);
    crate::move_mods::nudge(&mut app, 1.0, 0.0, false);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), photocraft_geom::Rect::new(9, 8, 17, 16));
    let surf = st.doc.layer(id).unwrap().surface().unwrap();
    assert_eq!(surf.rgba(8, 10)[3], 0.0);
    assert_eq!(surf.rgba(16, 10), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(surf.rgba(34, 10), [0.0, 1.0, 0.0, 1.0]);
}

#[test]
fn escape_key_cancels_selected_pixel_preview_without_consuming_history() {
    let (mut app, id) = app(16, true);
    let before = pixels(&app.session.active().unwrap().doc, id);
    let original_selection = selection(&app.session.active().unwrap().doc);
    let history = app.session.active().unwrap().history.past_len();
    down(&mut app, egui::Modifiers::NONE);
    movement(&mut app);
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            events: vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
            ..Default::default()
        },
        |ui| crate::shortcuts::handle(&mut app, ui.ctx()),
    );
    output.textures_delta.clear();
    up(&mut app);
    let state = app.session.active().unwrap();
    assert!(app.drag.is_none() && state.floating.is_none());
    assert_eq!(state.history.past_len(), history);
    assert_eq!(pixels(&state.doc, id), before);
    assert_eq!(selection(&state.doc), original_selection);
}
