//! #589: layers dragged onto another document's tab, then dropped on its canvas, are copied there.

use egui::accesskit::Role;
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, ViewportId, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::LayerId;
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::{Tool, View};

/// "source-doc" (40×30) with a red 10×10 square at (4, 6) on layer "paint", and "dest-doc"
/// (60×50). The source is active.
fn documents() -> (photocraft_engine::Session, LayerId) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "name": "source-doc"})).unwrap();
    let paint = LayerId(s.execute("layer.new.layer", json!({"name": "paint"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("select.rect", json!({"x": 4, "y": 6, "width": 10, "height": 10})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s.execute("file.new", json!({"width": 60, "height": 50, "name": "dest-doc"})).unwrap();
    s.set_active(0);
    (s, paint)
}

fn harness() -> (Harness<'static, PhotocraftApp>, LayerId) {
    let (s, paint) = documents();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    h.run_steps(8);
    (h, paint)
}

/// A document tab, found by its accessible title ("name @ 100% (…)", see `pro_tabs`).
fn tab(h: &Harness<'_, PhotocraftApp>, name: &str) -> Pos2 {
    h.get_by_label_contains(&format!("{name} @")).rect().center()
}

/// Press at `path[0]`, move through the rest (a few frames at each) and release at the last
/// point with `release` held.
fn drag_along(h: &mut Harness<'_, PhotocraftApp>, path: &[Pos2], release: Modifiers) {
    drag_with_modifiers(h, path, Modifiers::NONE, release);
}

fn drag_with_modifiers(h: &mut Harness<'_, PhotocraftApp>, path: &[Pos2], press: Modifiers, release: Modifiers) {
    let first = path[0];
    h.event(egui::Event::PointerMoved(first));
    h.run_steps(1);
    h.event(egui::Event::ModifiersChanged(press));
    h.event(egui::Event::PointerButton { pos: first, button: PointerButton::Primary, pressed: true, modifiers: press });
    h.run_steps(1);
    for w in path.windows(2) {
        for k in 1..=4 {
            h.event(egui::Event::PointerMoved(w[0] + (w[1] - w[0]) * (k as f32 / 4.0)));
            h.run_steps(1);
        }
        h.run_steps(2);
    }
    let last = path[path.len() - 1];
    h.event(egui::Event::ModifiersChanged(release));
    h.event(egui::Event::PointerButton { pos: last, button: PointerButton::Primary, pressed: false, modifiers: release });
    h.run_steps(3);
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

/// The copy in "dest-doc" (its active layer) and its bounds.
fn copy(h: &Harness<'_, PhotocraftApp>) -> (String, Rect) {
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.name, "dest-doc", "the destination is shown");
    let l = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
    let b = photocraft_engine::layer_multi_cmds::layer_bounds(l).unwrap();
    (l.name.clone(), Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
}

fn source_square(h: &Harness<'_, PhotocraftApp>, paint: LayerId) -> photocraft_geom::Rect {
    let doc = &h.state().session.documents()[0].doc;
    photocraft_engine::layer_multi_cmds::layer_bounds(doc.layer(paint).unwrap()).unwrap()
}

#[test]
fn a_layers_panel_drag_onto_a_tab_drops_the_layer_where_it_is_released() {
    let (mut h, _) = harness();
    let row = h.get_by_role_and_label(Role::Button, "paint").rect().center();
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[row, over_tab, drop], Modifiers::NONE);
    let at = ViewXform::active(h.state()).unwrap().to_doc(drop);
    let (name, r) = copy(&h);
    assert_eq!(name, "paint");
    assert!((r.center().x - at[0] as f32).abs() <= 1.0 && (r.center().y - at[1] as f32).abs() <= 1.0, "centred on the drop point {at:?}: {r:?}");
    assert_eq!(h.state().session.documents()[0].doc.layers.len(), 2, "the source keeps its layers");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), 2);
}

#[test]
fn selected_panel_rows_reorder_together_and_stay_selected() {
    let (mut h, paint) = harness();
    let second = LayerId(h.state_mut().session.execute("layer.new.layer", json!({"name": "second"})).unwrap()["layer"].as_u64().unwrap());
    let target = LayerId(h.state_mut().session.execute("layer.new.layer", json!({"name": "target"})).unwrap()["layer"].as_u64().unwrap());
    photocraft_engine::layer_multi_cmds::set_selection(&mut h.state_mut().session, vec![paint, second], Some(second), Some(second)).unwrap();
    h.run_steps(3);
    let from = h.get_by_role_and_label(Role::Button, "paint").rect().center();
    let target_row = h.get_by_role_and_label(Role::Button, "target").rect();
    let to = egui::pos2(target_row.center().x, target_row.top() + 3.0);
    let past = h.state().session.active().unwrap().history.past_len();
    drag_along(&mut h, &[from, to], Modifiers::NONE);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.layers.iter().skip(1).map(|l| l.id).collect::<Vec<_>>(), vec![target, paint, second]);
    assert_eq!(st.selected_layers(), vec![paint, second]);
    assert_eq!(st.active_layer, Some(second));
    assert_eq!(st.history.past_len(), past + 1, "one undo for the complete block");
    h.state_mut().session.execute("edit.undo", json!({})).unwrap();
    assert_eq!(h.state().session.active().unwrap().doc.layers.iter().skip(1).map(|l| l.id).collect::<Vec<_>>(), vec![paint, second, target]);
}

#[test]
fn selected_panel_rows_drop_into_a_group_together() {
    let (mut h, paint) = harness();
    let second = LayerId(h.state_mut().session.execute("layer.new.layer", json!({"name": "second"})).unwrap()["layer"].as_u64().unwrap());
    let group = LayerId(h.state_mut().session.execute("layer.new.group", json!({"name": "destination", "empty": true})).unwrap()["layer"].as_u64().unwrap());
    photocraft_engine::layer_multi_cmds::set_selection(&mut h.state_mut().session, vec![paint, second], Some(paint), Some(paint)).unwrap();
    h.run_steps(3);
    let from = h.get_by_role_and_label(Role::Button, "paint").rect().center();
    let to = h.get_by_role_and_label(Role::Button, "destination").rect().center();
    drag_along(&mut h, &[from, to], Modifiers::NONE);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.doc.layer(group).unwrap().children().unwrap().iter().map(|l| l.id).collect::<Vec<_>>(), vec![paint, second]);
    assert_eq!(st.selected_layers(), vec![paint, second]);
}

#[test]
fn shift_centres_the_copy_when_the_documents_differ_in_size() {
    let (mut h, _) = harness();
    let row = h.get_by_role_and_label(Role::Button, "paint").rect().center();
    let over_tab = tab(&h, "dest-doc");
    let drop = h.state().last_canvas_rect.center() + vec2(80.0, 60.0);
    drag_along(&mut h, &[row, over_tab, drop], Modifiers::SHIFT);
    assert_eq!(copy(&h).1, Rect::from_min_max(egui::pos2(25.0, 20.0), egui::pos2(35.0, 30.0)));
}

#[test]
fn a_move_tool_drag_onto_a_tab_copies_the_layer_under_the_pointer_and_leaves_the_source() {
    for press in [Modifiers::NONE, Modifiers::ALT] {
        let (mut h, paint) = harness();
        h.state_mut().ui.tool = Tool::Move;
        // Auto-Select intentionally advances the source revision on a layer pick.
        h.state_mut().ui.tool_options.move_auto_select = false;
        h.run_steps(2);
        let before = source_square(&h, paint);
        let source_revision = h.state().session.documents()[0].revision;
        let steps = h.state().session.documents()[0].history.entries().len();
        let grab = ViewXform::active(h.state()).unwrap().to_screen(9.0, 11.0);
        let grab_doc = ViewXform::active(h.state()).unwrap().to_doc(grab);
        let over_tab = tab(&h, "dest-doc");
        let drop = h.state().last_canvas_rect.center();
        drag_with_modifiers(&mut h, &[grab, over_tab, drop], press, Modifiers::NONE);
        let at = ViewXform::active(h.state()).unwrap().to_doc(drop);
        let (name, r) = copy(&h);
        assert_eq!(name, "paint");
        // The grabbed pixel lands under the pointer.
        let want = egui::pos2((9.0 + at[0] - grab_doc[0]) as f32, (11.0 + at[1] - grab_doc[1]) as f32);
        assert!((r.center() - want).length() <= 1.5, "{r:?} around {want:?}");
        assert_eq!(source_square(&h, paint), before, "the source layer did not move");
        assert_eq!(h.state().session.documents()[0].revision, source_revision);
        assert_eq!(h.state().session.documents()[0].history.entries().len(), steps, "no Move step in the source");
    }
}

#[test]
fn dropping_back_on_the_source_copies_nothing() {
    let (mut h, _) = harness();
    let row = h.get_by_role_and_label(Role::Button, "paint").rect().center();
    let (dest_tab, source_tab) = (tab(&h, "dest-doc"), tab(&h, "source-doc"));
    let drop = h.state().last_canvas_rect.center();
    drag_along(&mut h, &[row, dest_tab, source_tab, drop], Modifiers::NONE);
    let docs = h.state().session.documents();
    assert_eq!((docs[0].doc.layers.len(), docs[1].doc.layers.len()), (2, 1));
    assert_eq!(h.state().session.active_index(), Some(0));
}

#[test]
fn tiled_alt_move_keeps_the_source_when_press_and_crossing_share_a_frame() {
    let (mut h, _) = harness();
    h.state_mut().ui.view.arrange = "twoUpVertical".into();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.tool_options.move_auto_select = false;
    h.state_mut().ui.extras.rulers = false;
    h.run_steps(3);
    let cells = crate::canvas::arranged_cells(h.state(), h.state().last_canvas_rect).unwrap();
    let grab = cells[0].1.center();
    let target = cells[1].1.center();
    let source_revision = h.state().session.documents()[0].revision;

    h.event(Event::PointerMoved(grab));
    h.run_steps(1);
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.event(Event::PointerButton { pos: grab, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT });
    h.event(Event::PointerMoved(target));
    h.run_steps(1);
    assert_eq!(h.state().session.active_index(), Some(0), "crossing a tile with the button held must not select it");

    h.event(Event::PointerButton { pos: target, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::ALT });
    h.run_steps(2);
    assert_eq!(copy(&h).0, "paint");
    assert_eq!(h.state().session.documents()[0].revision, source_revision);
}

struct CapturedDrag {
    app: PhotocraftApp,
    ctx: egui::Context,
    viewports: [ViewportId; 2],
    paint: LayerId,
}

impl CapturedDrag {
    const CLIENT: Rect = Rect::from_min_max(Pos2::ZERO, pos2(400.0, 320.0));
    const CANVAS: Rect = Rect::from_min_max(pos2(24.0, 48.0), pos2(376.0, 296.0));

    fn new(floating: usize) -> Self {
        let (session, paint) = documents();
        let mut app = PhotocraftApp::new(session, Default::default());
        let floating_viewport = ViewportId::from_hash_of(("docwin", crate::view_cmds::float_window(&mut app, floating, 0)));
        let mut viewports = [ViewportId::ROOT; 2];
        viewports[floating] = floating_viewport;
        app.session.set_active(0);
        app.sync_views();
        app.ui.tool = Tool::Move;
        app.ui.tool_options.move_auto_select = false;
        app.ui.tool_options.move_show_transform = false;
        app.ui.extras.rulers = false;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        app.services.screen_viewport = Some(Box::new(move |_, screen| {
            viewports.into_iter().find(|&viewport| {
                let (origin, scale) = Self::geometry(viewport);
                Rect::from_min_size(origin * scale, Self::CLIENT.size() * scale).contains(screen)
            })
        }));
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        PhotocraftApp::setup_context(&ctx, Default::default());
        Self { app, ctx, viewports, paint }
    }

    fn geometry(viewport: ViewportId) -> (Pos2, f32) {
        if viewport == ViewportId::ROOT { (pos2(-500.0, 80.0), 1.25) } else { (pos2(180.0, 130.0), 2.0) }
    }

    fn view(document: usize) -> View {
        if document == 0 {
            View { rotation: 0.0, doc_size: [40, 30], center: [20.0, 15.0], zoom: 2.0, fit_pending: false }
        } else {
            View { rotation: 0.0, doc_size: [60, 50], center: [30.0, 25.0], zoom: 3.0, fit_pending: false }
        }
    }

    fn document_point(document: usize, x: f32, y: f32) -> Pos2 {
        let view = Self::view(document);
        ViewXform { rect: Self::CANVAS, center: view.center, zoom: view.zoom, rotation: view.rotation, flip: false }.to_screen(x, y)
    }

    fn frame(&mut self, document: usize, mut events: Vec<Event>, mods: Modifiers) {
        let viewport_id = self.viewports[document];
        let focused = document == 0;
        events.insert(0, Event::ModifiersChanged(mods));
        let mut input = egui::RawInput { viewport_id, screen_rect: Some(Self::CLIENT), focused, events, ..Default::default() };
        for (index, viewport) in self.viewports.into_iter().enumerate() {
            let (origin, scale) = Self::geometry(viewport);
            let info = input.viewports.entry(viewport).or_default();
            info.native_pixels_per_point = Some(scale);
            info.inner_rect = Some(Self::CLIENT.translate(origin.to_vec2()));
            info.focused = Some(index == 0);
            if viewport != ViewportId::ROOT {
                info.parent = Some(ViewportId::ROOT);
            }
        }
        self.app.ui.views[document] = Self::view(document);
        self.app.last_canvas_rect = Self::CANVAS;
        let primary = self.app.session.active_index() == Some(document);
        let mut output = self.ctx.run_ui(input, |ui| {
            let _ = crate::canvas::canvas_view(&mut self.app, ui, document, Self::CANVAS, Self::view(document), primary);
        });
        output.textures_delta.clear();
    }

    fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
        Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }
    }

    fn start_with_alt(&mut self) {
        self.frame(1, vec![], Modifiers::NONE);
        let grab = Self::document_point(0, 8.0, 10.0);
        self.frame(0, vec![Event::PointerMoved(grab)], Modifiers::NONE);
        self.frame(0, vec![Self::button(grab, true, Modifiers::ALT)], Modifiers::ALT);
        // egui only starts a drag after the threshold. Alt belongs to the press even
        // when the key was released before the motion that crosses that threshold.
        self.frame(0, vec![Event::PointerMoved(grab + vec2(18.0, 0.0))], Modifiers::NONE);
        assert!(crate::move_mods::duplicate_deferred(&self.app));
    }

    fn release_on(&mut self, screen: Pos2) {
        let (source_origin, source_scale) = Self::geometry(self.viewports[0]);
        let reported_by_source = (screen / source_scale - source_origin).to_pos2();
        self.frame(0, vec![Event::PointerMoved(reported_by_source)], Modifiers::NONE);
        self.frame(0, vec![Self::button(reported_by_source, false, Modifiers::NONE)], Modifiers::NONE);
        assert!(self.app.drag.is_none());
    }

    fn document_bounds(&self, document: usize, layer: LayerId) -> photocraft_geom::Rect {
        let layer = self.app.session.documents()[document].doc.layer(layer).unwrap();
        photocraft_engine::layer_multi_cmds::layer_bounds(layer).unwrap()
    }
}

#[test]
fn cross_viewport_alt_move_copies_in_both_directions_at_mixed_dpi() {
    for floating in [0, 1] {
        let mut h = CapturedDrag::new(floating);
        let docs = h.app.session.documents();
        let source_before = docs[0].doc.clone();
        let source_revision = docs[0].revision;
        let source_steps = docs[0].history.past_len();
        let dest_steps = docs[1].history.past_len();
        h.start_with_alt();
        assert_eq!(h.app.session.documents()[0].revision, source_revision);
        let (origin, scale) = CapturedDrag::geometry(h.viewports[1]);
        let target_point = CapturedDrag::document_point(1, 27.0, 23.0);
        h.release_on((origin + target_point.to_vec2()) * scale);
        assert_eq!(h.app.session.active_index(), Some(1), "floating document {floating}");
        let docs = h.app.session.documents();
        let copied = docs[1].active_layer.unwrap();
        assert!(std::sync::Arc::ptr_eq(&docs[0].doc, &source_before));
        assert_eq!(docs[0].revision, source_revision);
        assert_eq!(docs[0].history.past_len(), source_steps);
        assert_eq!(docs[1].history.past_len(), dest_steps + 1);
        assert_eq!(docs[1].doc.layer(copied).unwrap().name, "paint");
        assert_eq!(h.document_bounds(1, copied), photocraft_geom::Rect::new(23, 19, 33, 29));
        h.app.run("edit.undo", json!({})).unwrap();
        assert!(h.app.session.documents()[1].doc.layer(copied).is_none());
        assert!(std::sync::Arc::ptr_eq(&h.app.session.documents()[0].doc, &source_before));
    }
}

#[test]
fn cross_viewport_alt_move_cancels_outside_but_keeps_same_document_duplicate() {
    let mut h = CapturedDrag::new(1);
    let original = h.app.session.documents()[0].doc.clone();
    let start_steps = h.app.session.documents()[0].history.past_len();
    let other_steps = h.app.session.documents()[1].history.past_len();
    h.start_with_alt();
    h.release_on(pos2(20.0, 20.0)); // A point in neither window.
    assert!(std::sync::Arc::ptr_eq(&h.app.session.documents()[0].doc, &original));
    assert_eq!(h.app.session.documents()[0].history.past_len(), start_steps);
    assert_eq!(h.app.session.documents()[1].history.past_len(), other_steps);

    h.start_with_alt();
    let (origin, scale) = CapturedDrag::geometry(h.viewports[0]);
    h.release_on((origin + CapturedDrag::document_point(0, 18.0, 12.0).to_vec2()) * scale);
    let source = h.app.session.active().unwrap();
    let copy = source.active_layer.unwrap();
    assert_ne!(copy, h.paint);
    assert_eq!(source.history.past_len(), start_steps + 1);
    assert_eq!(h.document_bounds(0, h.paint), photocraft_geom::Rect::new(4, 6, 14, 16));
    assert_eq!(h.document_bounds(0, copy), photocraft_geom::Rect::new(14, 8, 24, 18));
    assert_eq!(h.app.session.documents()[1].history.past_len(), other_steps);
}

#[test]
fn cross_viewport_alt_move_owns_the_press_before_egui_recognizes_a_drag() {
    let mut h = CapturedDrag::new(1);
    h.frame(1, vec![], Modifiers::NONE);
    let grab = CapturedDrag::document_point(0, 8.0, 10.0);
    h.frame(0, vec![Event::PointerMoved(grab)], Modifiers::NONE);
    h.frame(0, vec![CapturedDrag::button(grab, true, Modifiers::ALT)], Modifiers::ALT);
    assert!(crate::move_mods::duplicate_deferred(&h.app), "the captured press survives leaving the widget before the drag threshold");

    let (origin, scale) = CapturedDrag::geometry(h.viewports[1]);
    let target = (origin + CapturedDrag::document_point(1, 27.0, 23.0).to_vec2()) * scale;
    let (source_origin, source_scale) = CapturedDrag::geometry(h.viewports[0]);
    let release = (target / source_scale - source_origin).to_pos2();
    h.frame(0, vec![Event::PointerGone, CapturedDrag::button(release, false, Modifiers::NONE)], Modifiers::NONE);
    let dest = &h.app.session.documents()[1];
    assert_eq!(h.document_bounds(1, dest.active_layer.unwrap()), photocraft_geom::Rect::new(23, 19, 33, 29));
    assert!(h.app.drag.is_none());
}

#[test]
fn cross_viewport_alt_move_transfers_only_the_selection_and_preserves_the_source() {
    for floating in [0, 1] {
        let mut h = CapturedDrag::new(floating);
        h.app.run("select.rect", json!({"x": 7, "y": 9, "width": 3, "height": 4})).unwrap();
        let source = h.app.session.documents()[0].doc.clone();
        let source_steps = h.app.session.documents()[0].history.past_len();
        h.start_with_alt();
        let (origin, scale) = CapturedDrag::geometry(h.viewports[1]);
        h.release_on((origin + CapturedDrag::document_point(1, 27.0, 23.0).to_vec2()) * scale);
        let states = h.app.session.documents();
        assert_eq!(h.app.session.active_index(), Some(1));
        assert!(std::sync::Arc::ptr_eq(&states[0].doc, &source));
        assert_eq!(states[0].history.past_len(), source_steps);
        assert!(states[0].floating.is_none());
        assert_eq!(h.document_bounds(1, states[1].active_layer.unwrap()), photocraft_geom::Rect::new(26, 22, 29, 26));
    }
}

#[test]
fn selected_alt_move_drops_on_a_tab_but_cancels_outside_a_canvas() {
    let (mut h, _) = harness();
    h.state_mut().ui.tool = Tool::Move;
    h.state_mut().ui.tool_options.move_auto_select = false;
    h.state_mut().ui.tool_options.move_show_transform = false;
    h.state_mut().ui.extras.rulers = false;
    h.state_mut().run("select.rect", json!({"x": 7, "y": 9, "width": 3, "height": 4})).unwrap();
    h.run_steps(3);
    let grab = ViewXform::active(h.state()).unwrap().to_screen(8.0, 10.0);
    let target = tab(&h, "dest-doc");
    let source = h.state().session.documents()[0].doc.clone();
    drag_with_modifiers(&mut h, &[grab, target], Modifiers::ALT, Modifiers::NONE);
    assert_eq!(copy(&h).1.size(), vec2(3.0, 4.0));
    assert!(std::sync::Arc::ptr_eq(&h.state().session.documents()[0].doc, &source));

    let mut h = CapturedDrag::new(1);
    h.app.run("select.rect", json!({"x": 7, "y": 9, "width": 3, "height": 4})).unwrap();
    let source = h.app.session.documents()[0].doc.clone();
    h.start_with_alt();
    h.release_on(pos2(20.0, 20.0));
    assert!(std::sync::Arc::ptr_eq(&h.app.session.documents()[0].doc, &source));
    assert!(h.app.session.documents()[0].floating.is_none());
    assert_eq!(h.app.session.documents()[1].doc.layers.len(), 1);
}
