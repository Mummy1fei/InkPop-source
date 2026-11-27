use super::*;

fn temp() -> PathBuf {
    let path = std::env::temp_dir().join(format!("inkpop-diagnostic-tests-{}-{}", std::process::id(), timestamp()));
    std::fs::create_dir(&path).unwrap();
    path
}

fn event(stage: DiagnosticStage) -> DiagnosticEvent {
    DiagnosticEvent {
        stage,
        operation: DiagnosticOperation::Place,
        tool: DiagnosticTool::Eraser,
        source_size: Some([768, 1024]),
        target_size: Some([1920, 1080]),
        is_smart: Some(false),
        success: None,
    }
}

#[test]
fn diagnostic_whitelist_rejects_private_fields_bad_stages_and_sizes_v90() {
    let secret = "private-path-preset-arguments-pixels-3245";
    let mut item = diagnostic_record(event(DiagnosticStage::PreviewReady), 1, 20, Some("vulkan"));
    item["path"] = json!(secret);
    item["arguments"] = json!({"pixels":secret});
    item["error"] = json!(secret);
    item["sourceSize"] = json!([1_000_000_000u64, 10]);
    item["targetSize"] = json!([0, 1]);
    let mut bad = item.clone();
    bad["stage"] = json!(secret);
    let source = json!({"diagnostics":{"sequence":1,"persistedSequence":99,"events":[bad,item],"private":secret},"path":secret});
    let safe = safe_context(&source);
    assert!(!safe.to_string().contains(secret));
    assert_eq!(safe["diagnostics"]["persistedSequence"], 1);
    let rows = safe["diagnostics"]["events"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].get("sourceSize").is_none());
    assert!(rows[0].get("targetSize").is_none());
    assert_eq!(rows[0]["tool"], "eraser");
    assert_eq!(rows[0]["backend"], "vulkan");
    assert_eq!(rows[0]["isSmart"], false);
    assert_eq!(diagnostic_record(event(DiagnosticStage::PreviewReady), 2, 25, Some(secret)).get("backend"), None);
    for value in [json!([-1, 2]), json!([1.5, 2]), json!([3]), json!([2, 3, 4]), json!([262_144, 262_144])] {
        assert!(valid_diagnostic_size(&value).is_none(), "{value}");
    }
    assert_eq!(valid_diagnostic_size(&json!([24_000, 1_000])), Some(json!([24_000, 1_000])));
}

#[test]
fn diagnostic_ring_is_bounded_without_writing_noncritical_events_v90() {
    let root = temp();
    let mut session = Session::begin(root.clone()).unwrap();
    let before = read_marker_file(&mut session.file).unwrap();
    for _ in 0..100 {
        session.record_diagnostic(event(DiagnosticStage::PreviewReady));
    }
    assert_eq!(read_marker_file(&mut session.file).unwrap(), before, "noncritical input must not write disk immediately");
    let diagnostics = safe_context(&session.details)["diagnostics"].clone();
    assert_eq!(diagnostics["sequence"], 100);
    assert_eq!(diagnostics["persistedSequence"], 0);
    let rows = diagnostics["events"].as_array().unwrap();
    assert_eq!(rows.len(), DIAGNOSTIC_EVENTS);
    assert_eq!(rows.first().unwrap()["sequence"], 85);
    assert_eq!(rows.last().unwrap()["sequence"], 100);
    assert!(serde_json::to_vec(&session.details).unwrap().len() < MAX_MARKER_BYTES as usize);
    session.finish(0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn critical_confirm_stages_are_in_the_marker_before_return_v90() {
    let root = temp();
    let mut session = Session::begin(root.clone()).unwrap();
    for (i, stage) in
        [DiagnosticStage::TransformConfirmStart, DiagnosticStage::PreviewReleased, DiagnosticStage::TransformCommandFinished].into_iter().enumerate()
    {
        session.record_diagnostic(event(stage));
        let marker = read_marker_file(&mut session.file).unwrap();
        assert_eq!(marker["diagnostics"]["sequence"], (i + 1) as u64);
        assert_eq!(marker["diagnostics"]["persistedSequence"], (i + 1) as u64);
        assert_eq!(marker["diagnostics"]["events"].as_array().unwrap().last().unwrap()["stage"], diagnostic_stage(stage));
        assert!(!session.diagnostic_pending);
    }
    session.finish(0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn diagnostic_idle_is_throttled_and_stops_after_bounded_write_failures_v90() {
    let root = temp();
    let mut session = Session::begin(root.clone()).unwrap();
    session.record_diagnostic(event(DiagnosticStage::UiFrameBuilt));
    let start = session.diagnostic_last_attempt;
    assert!(session.persist_diagnostics_idle(start));
    assert!(read_marker_file(&mut session.file).unwrap().get("diagnostics").is_none());
    assert!(!session.persist_diagnostics_idle(start + DIAGNOSTIC_IDLE_INTERVAL));
    assert_eq!(read_marker_file(&mut session.file).unwrap()["diagnostics"]["persistedSequence"], 1);
    assert!(!session.persist_diagnostics_idle(start + DIAGNOSTIC_IDLE_INTERVAL * 2));
    let id = session.id.clone();
    drop(session.file);
    let path = marker_path(&root, &id).unwrap();
    let file = OpenOptions::new().read(true).open(&path).unwrap();
    let now = std::time::Instant::now();
    let mut failed = Session {
        dir: root.clone(),
        id,
        file,
        details: context("synthetic-write-failure", 1),
        diagnostic_started: now,
        diagnostic_last_attempt: now,
        diagnostic_pending: false,
        diagnostic_failures: 0,
    };
    failed.record_diagnostic(event(DiagnosticStage::UiFrameBuilt));
    assert!(failed.persist_diagnostics_idle(now + DIAGNOSTIC_IDLE_INTERVAL));
    assert!(failed.persist_diagnostics_idle(now + DIAGNOSTIC_IDLE_INTERVAL * 2));
    assert!(!failed.persist_diagnostics_idle(now + DIAGNOSTIC_IDLE_INTERVAL * 3));
    assert_eq!(failed.diagnostic_failures, DIAGNOSTIC_RETRY_LIMIT);
    assert_eq!(failed.details["diagnostics"]["sequence"], 1, "memory evidence remains available to panic reports");
    failed.record_diagnostic(event(DiagnosticStage::TransformConfirmStart));
    assert!(failed.diagnostic_pending, "a later critical transition retries independently without panicking");
    drop(failed);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn orphan_recovery_keeps_only_persisted_safe_breadcrumbs_v90() {
    let root = temp();
    let mut session = Session::begin(root.clone()).unwrap();
    session.record_diagnostic(event(DiagnosticStage::TransformConfirmStart));
    session.record_diagnostic(event(DiagnosticStage::UiFrameBuilt));
    assert_eq!(safe_context(&session.details)["diagnostics"]["sequence"], 2, "panic snapshot has latest memory evidence");
    assert_eq!(safe_context(&session.details)["diagnostics"]["persistedSequence"], 1);
    drop(session);
    recover_orphans(&root);
    let paths: Vec<_> =
        std::fs::read_dir(&root).unwrap().flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|v| v.to_str()).is_some_and(report_name)).collect();
    assert_eq!(paths.len(), 1);
    let report: Value = serde_json::from_slice(&std::fs::read(paths.first().unwrap()).unwrap()).unwrap();
    assert_eq!(report["event"], "previous-session-ended-without-clean-close");
    assert_eq!(report["diagnostics"]["sequence"], 1);
    assert_eq!(report["diagnostics"]["persistedSequence"], 1);
    assert_eq!(report["diagnostics"]["events"][0]["stage"], "transform-confirm-start");
    assert_eq!(report["backtrace"], Value::Null);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn monitor_preserves_real_child_critical_breadcrumbs_on_synthetic_exception_exit_v90() {
    use std::os::windows::process::CommandExt;
    for stage in ["confirm", "command", "release"] {
        let root = temp();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "crash_reports::diagnostic_tests_v90::native_breadcrumb_fixture_v90", "--nocapture"])
            .env("INKPOP_TEST_CRASH_DIRECTORY", &root)
            .env("INKPOP_TEST_DIAGNOSTIC_STAGE", stage)
            .creation_flags(0x08000000)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let ready = root.join("ready.txt");
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !ready.exists() && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if !ready.exists() {
            let _ = child.kill();
            let _ = child.wait();
        }
        assert!(ready.exists(), "owned diagnostic child must publish its marker");
        let id = std::fs::read_to_string(ready).unwrap();
        std::fs::write(root.join("go.txt"), b"ready").unwrap();
        monitor_process_in(&root, child.id(), &id).unwrap();
        let status = child.wait().unwrap();
        assert_eq!(status.code().map(|v| v as u32), Some(0xC0000005));
        let path =
            std::fs::read_dir(&root).unwrap().flatten().map(|e| e.path()).find(|p| p.file_name().and_then(|v| v.to_str()).is_some_and(report_name)).unwrap();
        let report: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let expected = match stage {
            "confirm" => "transform-confirm-start",
            "command" => "transform-command-finished",
            _ => "preview-released",
        };
        assert_eq!(report["processId"], child.id());
        assert_eq!(report["session"], id);
        assert_eq!(report["event"], "windows-exception-exit");
        assert_eq!(report["exitCodeHex"], "0xC0000005");
        assert_eq!(report["diagnostics"]["events"][0]["stage"], expected);
        assert_eq!(report["diagnostics"]["persistedSequence"], 1);
        assert_eq!(report["backtrace"], Value::Null);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(windows)]
#[test]
#[ignore = "Owned subprocess fixture, launched by diagnostic monitor test"]
fn native_breadcrumb_fixture_v90() {
    let Some(root) = std::env::var_os("INKPOP_TEST_CRASH_DIRECTORY").map(PathBuf::from) else { return };
    let stage = match std::env::var("INKPOP_TEST_DIAGNOSTIC_STAGE").ok().as_deref() {
        Some("confirm") => DiagnosticStage::TransformConfirmStart,
        Some("command") => DiagnosticStage::TransformCommandFinished,
        Some("release") => DiagnosticStage::PreviewReleased,
        _ => return,
    };
    DIRECTORY.set(Some(root.clone())).unwrap();
    let session = Session::begin(root.clone()).unwrap();
    let id = session.id.clone();
    *STATE.lock().unwrap() = Some(session);
    diagnostic(event(stage));
    std::fs::write(root.join("ready.txt"), id).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !root.join("go.txt").exists() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::thread::sleep(std::time::Duration::from_secs(2));
    // Synthetic exit status only: does not cause an access violation or collect a dump.
    std::process::exit(0xC0000005u32 as i32);
}
