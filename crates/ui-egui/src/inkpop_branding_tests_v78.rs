use super::*;

#[test]
fn system_information_and_print_dialog_identify_inkpop_v78() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let info = system_info_json(&app);
    let lines = info["lines"].as_array().unwrap();
    assert!(lines.first().unwrap().as_str().unwrap().starts_with("InkPop "));
    assert_eq!(info["version"], photocraft_engine::build_info::long_version());
    assert!(lines.iter().all(|line| !line.as_str().unwrap().contains("PhotoCraft")));

    app.run("file.new", json!({"width":32,"height":24})).unwrap();
    let dialog = crate::file_ui::open_print(&mut app);
    let fields = &app.ui.dialogs.iter().find(|d| d.id == dialog).unwrap().fields;
    assert_eq!(fields["__label"], "InkPop Print Settings");
    // The engine and saved print options retain the established machine value.
    assert_eq!(fields["colorHandling"], "printerManages");
}

#[test]
fn gpu_retry_notice_names_inkpop_without_restarting_v78() {
    let services = crate::Services { save_prefs: Some(Box::new(|_| Ok(()))), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    queue_fallback_notice(&mut app, "synthetic GPU failure");
    choose_recovery(&mut app, true).unwrap();
    assert!(app.ui.gpu_fallback_notice.is_none());
    let notice = app.ui.notices.last().unwrap();
    assert_eq!(notice.title, "GPU retry scheduled");
    assert!(notice.lines.iter().any(|line| line.contains("restart InkPop")));
    assert!(notice.lines.iter().all(|line| !line.contains("PhotoCraft")));
}
