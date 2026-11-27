use super::*;
use serde_json::json;

fn fixture() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    for name in ["Destination.png", "Dragged.png", "Docked.png"] {
        app.run("file.new", json!({"name":name,"width":96,"height":76})).unwrap();
    }
    (app, egui::Context::default())
}

#[test]
fn merged_window_keeps_both_documents_views_and_history_when_one_closes() {
    let (mut app, ctx) = fixture();
    let target = crate::view_cmds::float_window(&mut app, 0, 0);
    let source = crate::view_cmds::float_window(&mut app, 1, 0);
    app.ui.views[0].zoom = 0.4;
    app.ui.views[1].zoom = 0.7;
    let ids: Vec<_> = app.session.documents().iter().map(|s| s.doc.id).collect();

    assert!(merge(&mut app, &ctx, source, target));
    assert_eq!(app.ui.windows.len(), 1);
    assert_eq!(app.ui.windows[0].tabs, [DetachedTab::Document(0), DetachedTab::Document(1)]);
    assert_eq!(app.session.active_index(), Some(1));
    assert_eq!(app.ui.windows[0].view.zoom, 0.7);
    assert_eq!(crate::canvas::root_document_v74::index(&app, &ctx), Some(2));
    assert!(!select_document_tab(&mut app, &ctx, target, 2));
    assert!(select_document_tab(&mut app, &ctx, target, 0));
    assert_eq!(app.ui.windows[0].view.zoom, 0.4);

    app.run("document.move", json!({"document":0,"to":2})).unwrap();
    assert_eq!(app.ui.windows[0].tabs, [DetachedTab::Document(2), DetachedTab::Document(0)]);
    assert_eq!(app.session.active().unwrap().doc.id, ids[0]);
    queue_document_close(&ctx, target, ids[0].0);
    app.run("file.close", json!({"document":2})).unwrap();
    let mut out = ctx.run_ui(egui::RawInput::default(), |_| flush_document_closes(&app, &ctx));
    out.textures_delta.clear();
    assert!(out.viewport_output.values().all(|view| !view.commands.iter().any(|c| matches!(c, ViewportCommand::Close))));
    let window = &app.ui.windows[0];
    assert_eq!(window.id, target);
    assert_eq!(window.tabs, [DetachedTab::Document(0)]);
    assert_eq!(app.session.documents()[window.document].doc.id, ids[1]);
    assert_eq!(window.view.zoom, 0.7);
    assert_eq!(app.session.documents().len(), 2);
}

#[test]
fn physical_title_target_includes_canvas_below_dragged_window_and_excludes_unrelated_window() {
    let (mut app, ctx) = fixture();
    let source = crate::view_cmds::float_window(&mut app, 1, 0);
    let target = crate::view_cmds::float_window(&mut app, 0, 0);
    let source_viewport = ViewportId::from_hash_of(("docwin", source));
    let target_viewport = ViewportId::from_hash_of(("docwin", target));
    let mut input = egui::RawInput::default();
    let info = input.viewports.entry(target_viewport).or_default();
    info.inner_rect = Some(Rect::from_min_size(pos2(300.0, 120.0), vec2(500.0, 300.0)));
    info.native_pixels_per_point = Some(1.5);
    ctx.data_mut(|d| d.insert_temp(header_rect_id(target), Rect::from_min_size(pos2(10.0, 10.0), vec2(420.0, HEADER_HEIGHT))));
    let physical = pos2(340.0, 145.0) * 1.5;
    app.services.screen_viewport = Some(Box::new(move |_, _| Some(source_viewport)));
    let mut out = ctx.run_ui(input, |_| {
        assert_eq!(group_drop_target(&mut app, &ctx, source, physical), Some(target));
        assert_eq!(group_drop_target(&mut app, &ctx, source, physical + vec2(0.0, 70.0)), None);
        app.services.screen_viewport = Some(Box::new(|_, _| Some(ViewportId::ROOT)));
        assert_eq!(group_drop_target(&mut app, &ctx, source, physical), None);
        app.services.screen_viewport = Some(Box::new(move |_, _| Some(target_viewport)));
        ctx.data_mut(|d| d.insert_temp(manual_minimize_id(target), true));
        assert_eq!(group_drop_target(&mut app, &ctx, source, physical), None);
    });
    out.textures_delta.clear();
}

#[test]
fn pending_files_keep_their_order_when_a_floating_group_merges_during_decode() {
    let (mut app, ctx) = fixture();
    let target = crate::view_cmds::float_window(&mut app, 0, 0);
    let source = crate::view_cmds::float_window(&mut app, 1, 0);
    queue_open_tab(&mut app, source, JobId(13));
    queue_open_tab(&mut app, source, JobId(14));
    assert!(merge(&mut app, &ctx, source, target));
    app.run("file.new", json!({"name":"Second.png","width":96,"height":76})).unwrap();
    let second = app_doc_id(&app);
    finish_open_tab(&mut app, JobId(14), second);
    app.run("file.new", json!({"name":"First.png","width":96,"height":76})).unwrap();
    let first = app_doc_id(&app);
    finish_open_tab(&mut app, JobId(13), first);
    assert_eq!(app.ui.windows[0].tabs, [DetachedTab::Document(0), DetachedTab::Document(1), DetachedTab::Document(4), DetachedTab::Document(3)]);
}

fn app_doc_id(app: &PhotocraftApp) -> Option<DocId> {
    app.session.active().map(|s| s.doc.id)
}

#[test]
fn merged_tabs_share_the_caption_row_and_select_without_starting_a_drag() {
    let (mut app, ctx) = fixture();
    let target = crate::view_cmds::float_window(&mut app, 0, 0);
    let source = crate::view_cmds::float_window(&mut app, 1, 0);
    assert!(merge(&mut app, &ctx, source, target));
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(620.0, 400.0)));
    let mut tab_center = Pos2::ZERO;
    let mut events = Vec::new();
    for frame in 0..3 {
        let input = egui::RawInput { screen_rect, events: std::mem::take(&mut events), ..Default::default() };
        let window = app.ui.windows[0].clone();
        let out = ctx.run_ui(input, |ui| {
            assert!(!header(&mut app, ui, &window, "Dragged.png"));
            let bar = ctx.data(|data| data.get_temp::<Rect>(header_rect_id(target))).unwrap();
            assert!(ui.available_rect_before_wrap().top() - bar.bottom() < 8.0);
            assert!(!detached_drag_active(&ctx, target));
        });
        if frame == 0 {
            let bar = ctx.data(|data| data.get_temp::<Rect>(header_rect_id(target))).unwrap();
            let titles: Vec<_> = out
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if ["Destination.png", "Dragged.png"].contains(&text.galley.job.text.trim_end_matches(" *")) => Some(text),
                    _ => None,
                })
                .collect();
            assert_eq!(titles.len(), 2);
            assert!(titles.iter().all(|text| bar.contains_rect(Rect::from_min_size(text.pos, text.galley.size()))));
            assert_eq!(titles[0].pos.y, titles[1].pos.y);
            tab_center = titles[0].pos + titles[0].galley.size() / 2.0;
            events.push(egui::Event::PointerMoved(tab_center));
        }
        if frame < 2 {
            events.push(egui::Event::PointerButton {
                pos: tab_center,
                button: egui::PointerButton::Primary,
                pressed: frame == 0,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }
    assert_eq!(app.ui.windows[0].document, 0);
    assert_eq!(app.session.active_index(), Some(0));
}

#[derive(Debug)]
struct FileDrop(std::path::PathBuf, Vec<u8>);

impl egui::DroppedFile for FileDrop {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.1.clone())
    }
}

#[test]
fn native_child_drop_routes_title_to_tabs_and_body_to_original_document() {
    let (mut app, ctx) = fixture();
    let window = crate::view_cmds::float_window(&mut app, 0, 0);
    let viewport = ViewportId::from_hash_of(("docwin", window));
    app.session.set_active(0);
    app.run("edit.fill", json!({"color":"#3b7e9c"})).unwrap();
    let png = photocraft_io::export(&app.session.documents()[0].doc, "File.png", &Default::default()).unwrap().bytes;
    let drop = |name| std::sync::Arc::new(FileDrop(std::path::PathBuf::from(name), png.clone())) as egui::DroppedFileHandle;
    app.services.import =
        Some(Box::new(|name, bytes| photocraft_io::import(name, bytes).map(|result| (result.document, result.warnings)).map_err(|e| e.to_string())));
    let header = Rect::from_min_max(pos2(10.0, 10.0), pos2(490.0, 38.0));
    let body = Rect::from_min_max(pos2(10.0, 38.0), pos2(490.0, 390.0));
    ctx.data_mut(|d| d.insert_temp(header_rect_id(window), header));
    let deliver = |app: &mut PhotocraftApp, name, point| {
        app.services.cursor_pos = Some(Box::new(move |_| Some(point)));
        let mut input = egui::RawInput { viewport_id: viewport, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0))), ..Default::default() };
        input.viewports.entry(viewport).or_default();
        input.dropped_files.push(drop(name));
        let document = app.ui.windows[0].document;
        let mut out = ctx.run_ui(input, |ui| {
            drop_in_window(app, ui, window, document, body);
            // It must not be ingested again if the child is laid out a second time.
            drop_in_window(app, ui, window, document, body);
        });
        out.textures_delta.clear();
    };

    deliver(&mut app, "Opened.png", header.center());
    assert_eq!(app.session.documents().len(), 4);
    assert_eq!(app.ui.windows[0].tabs, [DetachedTab::Document(0), DetachedTab::Document(3)]);
    assert_eq!(app.ui.windows[0].document, 3);
    assert_eq!(app.session.documents()[3].doc.name, "Opened.png");

    let original = app.session.documents()[3].doc.id;
    let baseline = app.session.documents()[3].doc.layers.len();
    deliver(&mut app, "Placed.png", body.center());
    assert_eq!(app.drop_places.len(), 1);
    assert_eq!(app.drop_places[0].document, original);
    assert!(select_document_tab(&mut app, &ctx, window, 0));
    ctx.run_ui(egui::RawInput::default(), |_| app.place_next_dropped(&ctx)).textures_delta.clear();
    assert_eq!(app.session.documents().len(), 4);
    let document = app.session.active().unwrap();
    assert_eq!(document.doc.id, original);
    assert_eq!(document.doc.layers.len(), baseline + 1);
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    crate::transform_tool::cancel(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), baseline);
}
