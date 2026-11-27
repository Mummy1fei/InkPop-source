use super::*;
use crate::{DiagnosticEvent, DiagnosticOperation, DiagnosticStage, Services, state::MadeLayer};
use std::{cell::RefCell, rc::Rc};

fn call(app: &mut PhotocraftApp, ctx: &egui::Context, params: Value) -> Value {
    let (request, _) = ControlRequest::new("app.place", params);
    match handle(app, ctx, &request) {
        Outcome::Done(value) => value,
        _ => panic!("expected immediate response"),
    }
}

type EventLog = Rc<RefCell<Vec<DiagnosticEvent>>>;
type ReadCounter = Rc<RefCell<usize>>;

fn fixture(smart: bool) -> (PhotocraftApp, EventLog, ReadCounter) {
    let mut source = photocraft_engine::Session::new();
    source.execute("file.new", json!({"width":5,"height":4,"background":"white"})).unwrap();
    let bytes = photocraft_io::export(&source.active().unwrap().doc, "source.png", &Default::default()).unwrap().bytes;
    let events: Rc<RefCell<Vec<DiagnosticEvent>>> = Rc::default();
    let reads: Rc<RefCell<usize>> = Rc::default();
    let log = events.clone();
    let count = reads.clone();
    let services = Services {
        automation_read: Some(Box::new(move |path| {
            *count.borrow_mut() += 1;
            if path == "input.png" { Ok(("private image.png".into(), bytes.clone())) } else { Err("outside granted root".into()) }
        })),
        diagnostic_event: Some(Box::new(move |event| log.borrow_mut().push(event))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width":17,"height":13})).unwrap();
    app.session.prefs.edit(|p| p.general.always_create_smart_objects_when_placing = smart);
    (app, events, reads)
}

#[test]
fn scoped_place_commits_one_place_step_with_truthful_diagnostic_order_v90() {
    for smart in [false, true] {
        let (mut app, events, _) = fixture(smart);
        let ctx = egui::Context::default();
        let before = app.session.active().unwrap().doc.layers.len();
        let response = call(&mut app, &ctx, json!({"path":"input.png"}));
        assert_eq!(response["ok"], true, "{response}");
        assert_eq!(app.ui.transform.as_ref().unwrap().made, Some(MadeLayer::Place));
        for point in &mut app.ui.transform.as_mut().unwrap().quad {
            point[0] += 2.0;
            point[1] += 1.0;
        }
        crate::transform_tool::commit(&mut app);
        assert!(app.ui.transform.is_none());
        assert!(app.transform_preview.is_none());
        assert_eq!(app.session.active().unwrap().doc.layers.len(), before + 1);
        let recorded = events.borrow().clone();
        let confirm = recorded.iter().position(|e| e.stage == DiagnosticStage::TransformConfirmStart).unwrap();
        assert_eq!(recorded[confirm].operation, DiagnosticOperation::Place);
        assert_eq!(recorded[confirm].is_smart, Some(smart));
        assert_eq!(recorded[confirm].source_size, Some([5, 4]));
        assert_eq!(recorded[confirm + 1].stage, DiagnosticStage::PreviewReleased);
        assert_eq!(recorded[confirm + 2].stage, DiagnosticStage::TransformCommandFinished);
        assert_eq!(recorded[confirm + 2].success, Some(true));
        crate::diagnostics_ui::frame_built(&mut app, &ctx);
        assert_eq!(events.borrow().last().unwrap().stage, DiagnosticStage::UiFrameBuilt);
        let count = events.borrow().len();
        crate::diagnostics_ui::frame_built(&mut app, &ctx);
        assert_eq!(events.borrow().len(), count, "idle frames must not emit checkpoints");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.len(), before);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.len(), before + 1);
    }
}

#[test]
fn untouched_confirmation_does_not_invent_an_engine_finished_checkpoint_v90() {
    let (mut app, events, _) = fixture(false);
    let ctx = egui::Context::default();
    assert_eq!(call(&mut app, &ctx, json!({"path":"input.png"}))["ok"], true);
    events.borrow_mut().clear();
    crate::transform_tool::commit(&mut app);
    crate::diagnostics_ui::frame_built(&mut app, &ctx);
    assert_eq!(
        events.borrow().iter().map(|e| e.stage).collect::<Vec<_>>(),
        vec![DiagnosticStage::TransformConfirmStart, DiagnosticStage::PreviewReleased, DiagnosticStage::UiFrameBuilt]
    );
}

#[test]
fn scoped_place_rejects_bad_input_authority_and_busy_edit_before_mutation_v90() {
    let (mut app, _, reads) = fixture(false);
    let ctx = egui::Context::default();
    for params in [Value::Null, json!({}), json!({"path":7}), json!({"path":""}), json!({"path":"input.png","linked":true})] {
        assert_eq!(call(&mut app, &ctx, params)["ok"], false);
    }
    assert_eq!(*reads.borrow(), 0);
    assert_eq!(call(&mut app, &ctx, json!({"path":"../private.png"}))["ok"], false);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 1);
    let service = app.services.automation_read.take();
    assert_eq!(call(&mut app, &ctx, json!({"path":"input.png"}))["ok"], false);
    app.services.automation_read = service;
    assert_eq!(call(&mut app, &ctx, json!({"path":"input.png"}))["ok"], true);
    let count = app.session.active().unwrap().doc.layers.len();
    let read_count = *reads.borrow();
    assert_eq!(call(&mut app, &ctx, json!({"path":"input.png"}))["ok"], false);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), count);
    assert_eq!(*reads.borrow(), read_count);
    crate::transform_tool::cancel(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 1);
}

#[test]
fn rejected_transform_reports_failure_and_rolls_placement_back_v90() {
    let (mut app, events, _) = fixture(false);
    let ctx = egui::Context::default();
    assert_eq!(call(&mut app, &ctx, json!({"path":"input.png"}))["ok"], true);
    app.ui.transform.as_mut().unwrap().quad = [[0.0, 0.0]; 4];
    crate::transform_tool::commit(&mut app);
    assert_eq!(events.borrow().iter().find(|e| e.stage == DiagnosticStage::TransformCommandFinished).unwrap().success, Some(false));
    assert!(app.ui.transform.is_none());
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 1);
}
