use super::*;

fn temp() -> PathBuf {
    let path = std::env::temp_dir().join(format!("inkpop-crash-tests-{}-{}", std::process::id(), timestamp()));
    std::fs::create_dir(&path).unwrap();
    path
}

fn reports(dir: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(report_name))
        .collect();
    paths.sort();
    paths
}

#[test]
fn crash_directory_defaults_to_exe_and_falls_back_without_overwriting_a_blocking_file_v79() {
    let root = temp();
    let blocked = root.join("CrashReports");
    std::fs::write(&blocked, b"not a folder").unwrap();
    let fallback = root.join("Data/CrashReports");
    assert_eq!(choose_directory([blocked.clone(), fallback.clone()]), Some(fallback));
    assert_eq!(std::fs::read(blocked).unwrap(), b"not a folder");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn reports_rotate_by_count_and_bytes_and_leave_foreign_files_untouched_v79() {
    let root = temp();
    let foreign = root.join("manual-note.json");
    std::fs::write(&foreign, b"keep manual notes").unwrap();
    for _ in 0..35 {
        let path = write_report(&root, json!({"event":"synthetic","backtrace":"a".repeat(110 * 1024)})).unwrap();
        assert!(std::fs::metadata(path).unwrap().len() <= MAX_REPORT_BYTES as u64);
    }
    let actual = reports(&root);
    assert!(actual.len() <= MAX_REPORTS && actual.len() >= 2);
    assert!(actual.iter().map(|path| std::fs::metadata(path).unwrap().len()).sum::<u64>() <= MAX_TOTAL_BYTES);
    assert_eq!(std::fs::read(&foreign).unwrap(), b"keep manual notes");
    assert!(write_report(&root, json!({"backtrace":"b".repeat(MAX_REPORT_BYTES)})).is_err());
    assert!(!report_name("InkPop-crash-deadbeef.json"));
    assert!(!report_name("../InkPop-crash-forged.json"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_sessions_are_not_recovered_but_orphans_produce_a_bounded_unknown_exit_record_v79() {
    let root = temp();
    let mut session = Session::begin(root.clone()).unwrap();
    session.details["gpu"] = json!({"backend":"dx12","adapter":"Synthetic persistent adapter"});
    session.persist().unwrap();
    let marker = marker_path(&root, &session.id).unwrap();
    assert!(marker.exists());
    recover_orphans(&root);
    assert!(reports(&root).is_empty(), "another live instance still holds the marker");
    drop(session); // Simulate an unclean stop: no explicit finish.
    recover_orphans(&root);
    assert!(!marker.exists());
    let paths = reports(&root);
    assert_eq!(paths.len(), 1);
    let report: Value = serde_json::from_slice(&std::fs::read(&paths[0]).unwrap()).unwrap();
    assert_eq!(report["event"], "previous-session-ended-without-clean-close");
    assert_eq!(report["exitCode"], Value::Null);
    assert_eq!(report["backtrace"], Value::Null);
    assert_eq!(report["gpu"]["adapter"], "Synthetic persistent adapter");
    let clean = Session::begin(root.clone()).unwrap();
    let clean_path = marker_path(&root, &clean.id).unwrap();
    clean.finish(0);
    assert!(!clean_path.exists());
    recover_orphans(&root);
    assert_eq!(reports(&root).len(), 1, "normal close produces no crash record");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn truncated_markers_still_leave_evidence_without_copying_their_contents_v79() {
    let root = temp();
    let id = run_id();
    let path = marker_path(&root, &id).unwrap();
    std::fs::write(&path, b"{incomplete-private-parameters").unwrap();
    recover_orphans(&root);
    let data = std::fs::read_to_string(reports(&root).first().unwrap()).unwrap();
    assert!(!data.contains("incomplete-private-parameters"));
    assert!(!path.exists());
    assert!(data.contains("previous-session-ended-without-clean-close"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn privacy_whitelist_excludes_document_arguments_tokens_pixels_and_panic_payloads_v79() {
    let secret = "secret-document-token-and-pixels-123";
    let value = json!({"version":"v79","processId":42,"document":secret,"arguments":secret,"controlToken":secret,
        "gpu":{"backend":"dx12","adapter":"Synthetic GPU","pixels":secret,"parameters":secret}});
    let report = safe_context(&value).to_string();
    assert!(!report.contains(secret));
    assert!(report.contains("Synthetic GPU"));
    assert_eq!(panic_category(Some(secret)), "rust-string-panic-details-redacted");
    assert_eq!(panic_category(Some("index out of bounds: secret pixels")), "index-out-of-bounds");
    assert_eq!(bounded("a\u{0}b\rc", 2), "ab");
    assert!(!valid_id("../../private-file"));
}

#[test]
fn a_real_child_rust_panic_writes_a_backtrace_without_its_private_payload_v79() {
    let root = temp();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--ignored", "--exact", "crash_reports::tests::rust_panic_fixture_v79", "--nocapture"])
        .env("INKPOP_TEST_CRASH_DIRECTORY", &root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let status = command.status().unwrap();
    assert!(!status.success());
    let paths = reports(&root);
    assert_eq!(paths.len(), 1);
    let text = std::fs::read_to_string(&paths[0]).unwrap();
    let report: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(report["event"], "rust-panic");
    assert_eq!(report["panicPayloadRedacted"], true);
    assert!(!text.contains("sensitive-test-document-preset-token-63821"));
    assert!(report["backtrace"].as_str().is_some_and(|trace| !trace.is_empty()));
    assert!(report["location"]["file"].as_str().is_some_and(|file| file.ends_with("crash_reports_tests_v79.rs")));
    assert_eq!(report["exitCode"], Value::Null);
    recover_orphans(&root);
    assert_eq!(reports(&root).len(), 2, "next-start evidence remains distinct from a panic hook");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[ignore = "Owned subprocess fixture; launched explicitly by the panic regression"]
fn rust_panic_fixture_v79() {
    let Some(directory) = std::env::var_os("INKPOP_TEST_CRASH_DIRECTORY").map(PathBuf::from) else { return };
    DIRECTORY.set(Some(directory.clone())).unwrap();
    *STATE.lock().unwrap() = Some(Session::begin(directory).unwrap());
    crate::crash_guard::install_hook();
    panic!("sensitive-test-document-preset-token-63821");
}

#[cfg(windows)]
#[test]
fn independent_monitor_rejects_its_own_pid_with_a_real_locked_session_without_waiting_v79() {
    use std::os::windows::process::CommandExt;
    let root = temp();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "crash_reports::tests::native_exit_fixture_v79", "--nocapture"])
        .env("INKPOP_TEST_CRASH_DIRECTORY", &root)
        .env("INKPOP_TEST_MONITOR_SELF_CHECK", "1")
        .creation_flags(0x08000000)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    // Run this regression in our own child so removing the production guard
    // cannot deadlock the whole test suite on WaitForSingleObject(self).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break Some(status);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert!(status.is_some_and(|status| status.success()), "the self-pid check must return without waiting for its own process");
    assert!(reports(&root).is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
fn independent_monitor_records_a_synthetic_windows_exception_exit_v79() {
    use std::os::windows::process::CommandExt;
    let root = temp();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "crash_reports::tests::native_exit_fixture_v79", "--nocapture"])
        .env("INKPOP_TEST_CRASH_DIRECTORY", &root)
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
    }
    assert!(ready.exists(), "owned test child must publish its locked marker");
    let id = std::fs::read_to_string(ready).unwrap();
    std::fs::write(root.join("go.txt"), b"ready for monitor").unwrap();
    monitor_process_in(&root, child.id(), &id).unwrap();
    let status = child.wait().unwrap();
    assert_eq!(status.code().map(|code| code as u32), Some(0xC0000005));
    let report: Value = serde_json::from_slice(&std::fs::read(reports(&root).first().unwrap()).unwrap()).unwrap();
    assert_eq!(report["event"], "windows-exception-exit");
    assert_eq!(report["exitCodeHex"], "0xC0000005");
    assert_eq!(report["backtrace"], Value::Null, "an exit code does not invent a native stack");
    assert!(std::fs::read_dir(&root).unwrap().flatten().all(|entry| !entry.file_name().to_string_lossy().starts_with("running-")));
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(windows)]
#[test]
#[ignore = "Owned subprocess fixture; launched explicitly by the monitor regression"]
fn native_exit_fixture_v79() {
    let Some(directory) = std::env::var_os("INKPOP_TEST_CRASH_DIRECTORY").map(PathBuf::from) else { return };
    let session = Session::begin(directory.clone()).unwrap();
    if std::env::var_os("INKPOP_TEST_MONITOR_SELF_CHECK").is_some() {
        let path = marker_path(&directory, &session.id).unwrap();
        let probe = OpenOptions::new().read(true).write(true).open(&path).unwrap();
        assert!(matches!(probe.try_lock(), Err(std::fs::TryLockError::WouldBlock)), "the current session really holds the Windows file lock");
        let started = std::time::Instant::now();
        let error = monitor_process_in(&directory, std::process::id(), &session.id).unwrap_err();
        assert!(error.contains("cannot wait on itself"));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert!(path.exists(), "rejected self-monitoring must not consume the live marker");
        drop(probe);
        session.finish(0);
        return;
    }
    std::fs::write(directory.join("ready.txt"), &session.id).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !directory.join("go.txt").exists() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    std::thread::sleep(std::time::Duration::from_secs(2));
    // Safe synthetic status only: this does not trigger an access violation,
    // collect a dump, or touch another application's process.
    std::process::exit(0xC0000005u32 as i32);
}
