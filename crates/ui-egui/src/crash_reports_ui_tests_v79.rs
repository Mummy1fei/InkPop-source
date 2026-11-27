use crate::{PhotocraftApp, Services, menus};
use photocraft_engine::Session;
use serde_json::json;
use std::{cell::Cell, rc::Rc};

#[test]
fn crash_report_folder_command_uses_the_platform_service_without_a_document_v79() {
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let mut app = PhotocraftApp::new(
        Session::new(),
        Services {
            open_crash_reports: Some(Box::new(move || {
                count.set(count.get() + 1);
                Ok(())
            })),
            ..Default::default()
        },
    );
    assert!(menus::is_enabled(&app, "help.openCrashReports"));
    assert_eq!(menus::invoke(&mut app, &egui::Context::default(), "help.openCrashReports", json!({})).unwrap(), json!({"opened":true}));
    assert_eq!(calls.get(), 1);
    assert!(app.session.documents().is_empty());
}

#[test]
fn crash_report_folder_unavailable_or_open_failure_is_reported_truthfully_v79() {
    let mut app = PhotocraftApp::new(Session::new(), Services::default());
    assert!(!menus::is_enabled(&app, "help.openCrashReports"));
    assert!(menus::invoke(&mut app, &egui::Context::default(), "help.openCrashReports", json!({})).is_err());
    app.services.open_crash_reports = Some(Box::new(|| Err("folder open failed".into())));
    assert!(menus::is_enabled(&app, "help.openCrashReports"));
    assert!(menus::invoke(&mut app, &egui::Context::default(), "help.openCrashReports", json!({})).unwrap_err().contains("folder open failed"));
}
