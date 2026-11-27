//! A transform may never undo or commit into a different document.
use super::*;
use photocraft_engine::{DocState, Session};
use photocraft_geom::Rect;

struct Snapshot {
    document: Arc<Document>,
    revision: u64,
    saved_revision: u64,
    history: Vec<String>,
    redo: Vec<String>,
    active: Option<LayerId>,
    selected: Vec<LayerId>,
    pixels: Vec<[f32; 4]>,
}

fn snapshot(state: &DocState) -> Snapshot {
    Snapshot {
        document: state.doc.clone(),
        revision: state.revision,
        saved_revision: state.saved_revision,
        history: state.history.entries(),
        redo: state.history.redo_labels().map(str::to_owned).collect(),
        active: state.active_layer,
        selected: state.selected_layers(),
        pixels: photocraft_compose::render(&state.doc, state.doc.bounds()).px,
    }
}

fn unchanged(state: &DocState, before: &Snapshot) {
    assert!(Arc::ptr_eq(&state.doc, &before.document), "document identity changed");
    assert_eq!(state.revision, before.revision);
    assert_eq!(state.saved_revision, before.saved_revision);
    assert_eq!(state.history.entries(), before.history);
    assert_eq!(state.history.redo_labels().collect::<Vec<_>>(), before.redo.iter().map(String::as_str).collect::<Vec<_>>());
    assert_eq!(state.active_layer, before.active);
    assert_eq!(state.selected_layers(), before.selected);
    assert_eq!(photocraft_compose::render(&state.doc, state.doc.bounds()).px, before.pixels);
}

fn pair() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
    for (name, color) in [("source A", [1.0, 0.0, 0.0, 1.0]), ("other B", [0.0, 0.0, 1.0, 1.0])] {
        app.session.execute("file.new", json!({"name":name,"width":64,"height":64,"background":"white"})).unwrap();
        app.session.execute("layer.new.layer", json!({"name":"paint"})).unwrap();
        app.session
            .edit("paint a visible square", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(8, 8, 32, 32), &color);
                Ok(())
            })
            .unwrap();
    }
    app.session.set_active(0);
    app.sync_views();
    (app, egui::Context::default())
}

fn place(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let mut source = Session::new();
    source.execute("file.new", json!({"width":5,"height":4,"background":"white"})).unwrap();
    let bytes = photocraft_io::export(&source.active().unwrap().doc, "source.png", &Default::default()).unwrap().bytes;
    app.session.prefs.edit(|prefs| prefs.general.always_create_smart_objects_when_placing = false);
    app.place_bytes("placed.png", bytes, None).unwrap();
    begin_placed(app, ctx).unwrap();
}

fn made(app: &mut PhotocraftApp, ctx: &egui::Context, kind: MadeLayer) {
    match kind {
        MadeLayer::Place => place(app, ctx),
        MadeLayer::Copy => begin_copy(app, ctx).unwrap(),
    }
}

fn translated(app: &mut PhotocraftApp) {
    let session = app.ui.transform.as_mut().unwrap();
    session.quad = session.quad.map(|[x, y]| [x + 7.0, y + 3.0]);
}

fn no_preview(app: &PhotocraftApp) {
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
}

fn group_active(app: &mut PhotocraftApp) -> LayerId {
    let child = app.session.active().unwrap().active_layer.unwrap();
    let group = app.session.execute("layer.groupLayers", json!({"name":"view-only disclosure"})).unwrap()["layer"].as_u64().unwrap();
    app.session.execute("layer.select", json!({"layer":child.0})).unwrap();
    LayerId(group)
}

fn view_only_cow(app: &mut PhotocraftApp, group: LayerId, rename: bool) {
    let state = app.session.active().unwrap();
    let previous = state.doc.clone();
    let revision = state.revision;
    let history = state.history.entries();
    app.session.execute("layer.setExpanded", json!({"layer":group.0,"expanded":false})).unwrap();
    let state = app.session.active_mut().unwrap();
    if rename {
        // The same name-only COW that adopting a saved path performs, without
        // touching a real path or changing the exact-Arc undo checks below.
        Arc::make_mut(&mut state.doc).name = "saved under a new name.psd".into();
    }
    assert!(!Arc::ptr_eq(&previous, &state.doc), "the regression requires a real view-only COW");
    assert_eq!(state.revision, revision, "disclosure and file-name changes are not pixel edits");
    assert_eq!(state.history.entries(), history);
}

#[test]
fn made_cross_document_cleanup_never_undoes_the_new_active_document_v98() {
    for kind in [MadeLayer::Place, MadeLayer::Copy] {
        let (mut app, ctx) = pair();
        let original = app.session.documents()[0].doc.clone();
        let history = app.session.documents()[0].history.entries();
        let other = snapshot(&app.session.documents()[1]);
        made(&mut app, &ctx, kind);
        app.session.set_active(1);
        end_if_left(&mut app);
        no_preview(&app);
        assert_eq!(app.session.active_index(), Some(1));
        unchanged(&app.session.documents()[1], &other);
        let source = &app.session.documents()[0];
        assert!(Arc::ptr_eq(&source.doc, &original));
        assert_eq!(source.history.entries(), history);
        assert!(!source.history.can_redo(), "cancelled creation cannot be redone");
    }
    // A new child created inside a closed group is automatically revealed by
    // the actual Layers panel tracker, after its transform has captured ownership.
    for kind in [MadeLayer::Place, MadeLayer::Copy] {
        let (mut app, ctx) = pair();
        let group = group_active(&mut app);
        let child = app.session.active().unwrap().active_layer.unwrap();
        app.session.execute("layer.setExpanded", json!({"layer":group.0,"expanded":false})).unwrap();
        app.session.execute("layer.select", json!({"layer":group.0})).unwrap();
        assert_eq!(crate::layer_reveal::track(&mut app, &ctx), None);
        app.session.execute("layer.select", json!({"layer":child.0})).unwrap();
        let original = app.session.active().unwrap().doc.clone();
        let other = snapshot(&app.session.documents()[1]);
        made(&mut app, &ctx, kind);
        let revision = app.session.active().unwrap().revision;
        assert_eq!(crate::layer_reveal::track(&mut app, &ctx), app.session.active().unwrap().active_layer);
        assert!(matches!(&app.session.active().unwrap().doc.layer(group).unwrap().content, LayerContent::Group(group) if group.expanded));
        assert_eq!(app.session.active().unwrap().revision, revision);
        end_if_left(&mut app);
        assert!(app.ui.transform.is_some(), "automatic reveal must not cancel a new child transform");
        app.session.set_active(1);
        end_if_left(&mut app);
        no_preview(&app);
        unchanged(&app.session.documents()[1], &other);
        assert!(Arc::ptr_eq(&app.session.documents()[0].doc, &original));
    }
}

#[test]
fn direct_cancel_and_commit_use_the_transform_owner_v98() {
    for kind in [MadeLayer::Place, MadeLayer::Copy] {
        for confirm in [false, true] {
            let (mut app, ctx) = pair();
            let original = app.session.documents()[0].doc.clone();
            let other = snapshot(&app.session.documents()[1]);
            made(&mut app, &ctx, kind);
            translated(&mut app);
            app.session.set_active(1);
            if confirm {
                commit(&mut app)
            } else {
                cancel(&mut app)
            }
            no_preview(&app);
            assert_eq!(app.session.active_index(), Some(1));
            unchanged(&app.session.documents()[1], &other);
            assert!(Arc::ptr_eq(&app.session.documents()[0].doc, &original));
        }
    }
}

#[test]
fn closing_the_source_never_rolls_back_a_surviving_document_v98() {
    for kind in [MadeLayer::Place, MadeLayer::Copy] {
        let (mut app, ctx) = pair();
        made(&mut app, &ctx, kind);
        let other = snapshot(&app.session.documents()[1]);
        app.session.close(0).unwrap();
        end_if_left(&mut app);
        no_preview(&app);
        unchanged(app.session.active().unwrap(), &other);
    }
}

#[test]
fn a_reopened_source_id_is_not_the_original_transform_snapshot_v98() {
    for confirm in [false, true] {
        let (mut app, ctx) = pair();
        place(&mut app, &ctx);
        translated(&mut app);
        let replacement = (*app.session.active().unwrap().doc).clone();
        app.session.close(0).unwrap();
        let index = app.session.add_document(replacement, None);
        let reopened = snapshot(&app.session.documents()[index]);
        let other = snapshot(&app.session.documents()[0]);
        if confirm {
            commit(&mut app)
        } else {
            cancel(&mut app)
        }
        no_preview(&app);
        unchanged(&app.session.documents()[index], &reopened);
        unchanged(&app.session.documents()[0], &other);
    }
    // Imported ordinary documents start at revision 1 with empty history. Reopen
    // the same persisted ID at that identical revision: only the lifetime differs.
    let (mut app, ctx) = pair();
    let imported = (*app.session.active().unwrap().doc).clone();
    app.session.close(0).unwrap();
    let index = app.session.add_document(imported, None);
    let state = app.session.active().unwrap();
    assert_eq!(state.revision, 1);
    assert_eq!(state.history.past_len(), 0);
    let lifetime = state.lifetime_token();
    let cloned_state: DocState = state.clone();
    assert!(lifetime.ptr_eq(&cloned_state.lifetime_token()), "a cloned runtime state keeps the open lifetime");
    begin(&mut app, &ctx).unwrap();
    translated(&mut app);
    let reopened_doc = (*app.session.active().unwrap().doc).clone();
    app.session.close(index).unwrap();
    let index = app.session.add_document(reopened_doc, None);
    assert_eq!(app.session.active().unwrap().revision, 1);
    assert!(!lifetime.ptr_eq(&app.session.active().unwrap().lifetime_token()));
    let reopened = snapshot(&app.session.documents()[index]);
    let other = snapshot(&app.session.documents()[0]);
    commit(&mut app);
    no_preview(&app);
    unchanged(&app.session.documents()[index], &reopened);
    unchanged(&app.session.documents()[0], &other);
}

#[test]
fn later_source_edits_are_never_undone_by_stale_transform_cleanup_v98() {
    for kind in [MadeLayer::Place, MadeLayer::Copy] {
        for action in 0..3 {
            let (mut app, ctx) = pair();
            made(&mut app, &ctx, kind);
            translated(&mut app);
            app.session.execute("layer.new.layer", json!({"name":"later edit must remain"})).unwrap();
            let later = snapshot(app.session.active().unwrap());
            let other = snapshot(&app.session.documents()[1]);
            match action {
                0 => cancel(&mut app),
                1 => commit(&mut app),
                _ => end_if_left(&mut app),
            }
            no_preview(&app);
            unchanged(&app.session.documents()[0], &later);
            unchanged(&app.session.documents()[1], &other);
        }
    }
}

#[test]
fn undo_and_aba_redo_invalidate_a_transform_without_clearing_redo_v98() {
    for redo in [false, true] {
        let (mut app, ctx) = pair();
        place(&mut app, &ctx);
        let created = app.session.active().unwrap().doc.clone();
        assert!(app.session.undo());
        if redo {
            assert!(app.session.redo());
            assert!(Arc::ptr_eq(&created, &app.session.active().unwrap().doc), "ABA returns to the same Arc but a newer revision");
        }
        let state = snapshot(app.session.active().unwrap());
        end_if_left(&mut app);
        no_preview(&app);
        unchanged(app.session.active().unwrap(), &state);
    }
}

#[test]
fn changed_history_edges_cannot_rewind_an_unrelated_creation_v98() {
    let (mut app, ctx) = pair();
    place(&mut app, &ctx);
    app.session.active_mut().unwrap().history.purge_last();
    let state = snapshot(app.session.active().unwrap());
    cancel(&mut app);
    no_preview(&app);
    unchanged(app.session.active().unwrap(), &state);
    for relabel in [false, true] {
        let (mut app, ctx) = pair();
        place(&mut app, &ctx);
        if relabel {
            app.session.active_mut().unwrap().history.set_current_label("a different operation's history label");
        } else {
            app.session.active_mut().unwrap().history.purge_last();
        }
        let before_commit = snapshot(app.session.active().unwrap());
        let past = app.session.active().unwrap().history.past_len();
        translated(&mut app);
        commit(&mut app);
        no_preview(&app);
        assert_eq!(app.session.active().unwrap().history.past_len(), past + 1, "a changed creation edge must not fold an unrelated step");
        assert!(app.session.undo());
        assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &before_commit.document));
        assert_eq!(app.session.active().unwrap().history.entries(), before_commit.history);
    }
}

#[test]
fn ordinary_selection_mask_and_warp_sessions_keep_document_ownership_v98() {
    for kind in 0..4 {
        let (mut app, ctx) = pair();
        match kind {
            0 => begin(&mut app, &ctx).unwrap(),
            1 => {
                app.session.execute("select.rect", json!({"x":8,"y":8,"width":8,"height":16})).unwrap();
                begin_selection(&mut app, &ctx).unwrap();
            }
            2 => {
                app.session.execute("layer.layerMask.revealAll", json!({})).unwrap();
                app.session
                    .edit("unlink mask", |doc, active| {
                        doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap().linked = false;
                        Ok(())
                    })
                    .unwrap();
                app.ui.mask_target = true;
                begin(&mut app, &ctx).unwrap();
                assert_eq!(app.ui.transform.as_ref().unwrap().target, Some(json!("mask")));
            }
            _ => begin_warp(&mut app, &ctx).unwrap(),
        }
        translated(&mut app);
        let source = snapshot(&app.session.documents()[0]);
        let other = snapshot(&app.session.documents()[1]);
        app.session.set_active(1);
        commit(&mut app);
        no_preview(&app);
        unchanged(&app.session.documents()[0], &source);
        unchanged(&app.session.documents()[1], &other);
    }
    for kind in 0..2 {
        let (mut app, ctx) = pair();
        let group = group_active(&mut app);
        if kind == 0 {
            begin(&mut app, &ctx).unwrap()
        } else {
            begin_warp(&mut app, &ctx).unwrap()
        }
        let session = app.ui.transform.as_ref().unwrap().session;
        view_only_cow(&mut app, group, true);
        end_if_left(&mut app);
        assert_eq!(app.ui.transform.as_ref().unwrap().session, session, "view-only COW preserves the open preview");
        cancel(&mut app);
        no_preview(&app);
    }
}

#[test]
fn same_layer_id_in_another_document_does_not_transfer_a_transform_v98() {
    let (mut app, ctx) = pair();
    begin(&mut app, &ctx).unwrap();
    translated(&mut app);
    let reused_id = app.ui.transform.as_ref().unwrap().layer;
    app.session.set_active(1);
    app.session
        .edit("preserved layer id from a different document", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().id = LayerId(reused_id);
            *active = Some(LayerId(reused_id));
            Ok(())
        })
        .unwrap();
    let other = snapshot(app.session.active().unwrap());
    commit(&mut app);
    no_preview(&app);
    unchanged(app.session.active().unwrap(), &other);
}

#[test]
fn normal_made_commit_cancel_and_rejected_commit_preserve_history_v98() {
    for kind in [MadeLayer::Place, MadeLayer::Copy] {
        for action in 0..4 {
            let (mut app, ctx) = pair();
            let before = app.session.active().unwrap().doc.clone();
            let past = app.session.active().unwrap().history.past_len();
            made(&mut app, &ctx, kind);
            match action {
                0 => cancel(&mut app),
                1 => commit(&mut app),
                2 => {
                    translated(&mut app);
                    commit(&mut app);
                }
                _ => {
                    app.ui.transform.as_mut().unwrap().quad = [[0.0, 0.0]; 4];
                    commit(&mut app);
                }
            }
            no_preview(&app);
            let state = app.session.active().unwrap();
            if action == 0 || action == 3 {
                assert!(Arc::ptr_eq(&before, &state.doc));
                assert_eq!(state.history.past_len(), past);
                assert!(!state.history.can_redo());
            } else {
                assert_eq!(state.doc.layers.len(), before.layers.len() + 1);
                assert_eq!(state.history.past_len(), past + 1, "creation and transform remain one undo step");
                assert!(app.session.undo());
                assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
            }
        }
    }
    for kind in [MadeLayer::Place, MadeLayer::Copy] {
        for confirm in [false, true] {
            let (mut app, ctx) = pair();
            let group = group_active(&mut app);
            let before = app.session.active().unwrap().doc.clone();
            let past = app.session.active().unwrap().history.past_len();
            made(&mut app, &ctx, kind);
            translated(&mut app);
            view_only_cow(&mut app, group, true);
            end_if_left(&mut app);
            assert!(app.ui.transform.is_some(), "normal Made transform survives disclosure/name COW");
            if confirm {
                commit(&mut app);
                assert_eq!(app.session.active().unwrap().history.past_len(), past + 1, "fold uses the current source after view COW");
                assert!(app.session.undo(), "one undo removes both creation and transform");
            } else {
                cancel(&mut app);
                assert!(!app.session.active().unwrap().history.can_redo());
            }
            no_preview(&app);
            assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc));
            assert_eq!(app.session.active().unwrap().history.past_len(), past);
        }
    }
}

#[test]
fn an_empty_copy_preview_failure_only_removes_the_copy_v98() {
    let (mut app, ctx) = pair();
    app.session.execute("layer.new.layer", json!({"name":"empty source"})).unwrap();
    let source = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.entries();
    let other = snapshot(&app.session.documents()[1]);
    assert!(begin_copy(&mut app, &ctx).is_err());
    no_preview(&app);
    assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &source));
    assert_eq!(app.session.active().unwrap().history.entries(), history);
    assert!(!app.session.active().unwrap().history.can_redo());
    unchanged(&app.session.documents()[1], &other);
}

#[test]
fn box_undo_redo_steps_do_not_change_the_source_stamp_v98() {
    let (mut app, ctx) = pair();
    begin_copy(&mut app, &ctx).unwrap();
    translated(&mut app);
    let settled = Step::of(app.ui.transform.as_ref().unwrap());
    app.transform_preview.as_mut().unwrap().steps.settle(settled);
    assert!(owns_active(&app));
    intercept(&mut app, "edit.undo").unwrap().unwrap();
    assert!(owns_active(&app), "box-only undo leaves the document source unchanged");
    intercept(&mut app, "edit.redo").unwrap().unwrap();
    assert!(owns_active(&app));
    commit(&mut app);
    no_preview(&app);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 3);
}

#[test]
fn clicking_a_real_document_tab_cancels_in_the_source_document_v98() {
    let (app, _) = pair();
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1000.0, 700.0)).build_ui_state(
        |ui, (app, ready): &mut (PhotocraftApp, bool)| {
            if *ready {
                end_if_left(app);
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            }
        },
        (app, false),
    );
    PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::StudioLight);
    harness.state_mut().1 = true;
    harness.run_steps(3);
    let ctx = harness.ctx.clone();
    let original = harness.state().0.session.documents()[0].doc.clone();
    let other = snapshot(&harness.state().0.session.documents()[1]);
    place(&mut harness.state_mut().0, &ctx);
    harness.run_steps(2);
    let point = harness.state().0.tab_strip.as_ref().unwrap().tabs[1].center();
    harness.event(egui::Event::PointerMoved(point));
    harness.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    harness.run_steps(1);
    harness.event(egui::Event::PointerButton { pos: point, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    harness.run_steps(3);
    let app = &harness.state().0;
    no_preview(app);
    assert_eq!(app.session.active_index(), Some(1));
    unchanged(&app.session.documents()[1], &other);
    assert!(Arc::ptr_eq(&app.session.documents()[0].doc, &original));
}

#[test]
fn a_new_begin_cannot_replace_an_unconfirmed_creation_v98() {
    let (mut app, ctx) = pair();
    place(&mut app, &ctx);
    let original_session = app.ui.transform.as_ref().unwrap().session;
    let state = snapshot(app.session.active().unwrap());
    assert!(begin(&mut app, &ctx).is_err());
    assert!(begin_copy(&mut app, &ctx).is_err());
    assert!(begin_selection(&mut app, &ctx).is_err());
    assert_eq!(app.ui.transform.as_ref().unwrap().session, original_session);
    unchanged(app.session.active().unwrap(), &state);
    assert!(owns_active(&app));
    cancel(&mut app);
    no_preview(&app);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 2);
}
