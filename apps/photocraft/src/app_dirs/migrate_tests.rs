use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("inkpop-profile-{}-{stamp}-{seq}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, name: &str, contents: &str) {
        let path = self.0.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.0.join(name)).unwrap()
    }

    fn release(&self, version: u32) -> PathBuf {
        let dir = format!("output/InkPop-V2.0-v{version}");
        self.write(&format!("{dir}/InkPop.exe"), "test");
        self.0.join(dir)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn extracts_recent_files_and_supported_settings_from_the_previous_portable_release() {
    let f = Fixture::new();
    let old_dir = "output/InkPop-旧名称-V2.0-v112";
    f.write(&format!("{old_dir}/InkPop.exe"), "test");
    let old_app = f.0.join(old_dir);
    let new_app = f.release(120);
    fs::write(old_app.join("portable.txt"), "").unwrap();
    let old = "output/InkPop-旧名称-V2.0-v112/PhotoCraftData";
    let prefs = r#"{"fileHandling":{"recentFiles":["sample.psd"]},"interface":{"uiScale":125}}"#;
    let known = [
        ("preferences.json", prefs),
        ("ui.ron", "window layout"),
        ("workspace-layout.json", "workspace"),
        ("tool-state.json", "last tools"),
        ("Presets/index.json", "preset index"),
        ("Presets/actions.json", "actions"),
        ("Presets/sample.pcbrushes", "brush"),
        ("Presets/tips/sample.pctip", "tip"),
    ];
    for (path, value) in known {
        f.write(&format!("{old}/{path}"), value);
    }
    let excluded = ["CrashReports/snapshot.json", "Recovery/work.psd", "control-token", "gpu-starting.json", "Presets/tool.pctools", "Presets/tips/temp.tmp"];
    for path in excluded {
        f.write(&format!("{old}/{path}"), "should not copy");
    }
    // Neither an unrelated sibling nor a newer package qualifies as an earlier release.
    f.write("output/Other/PhotoCraftData/preferences.json", "other");
    let future = f.release(121);
    fs::write(future.join("portable.txt"), "").unwrap();
    f.write("output/InkPop-V2.0-v121/PhotoCraftData/preferences.json", "future");

    let target = f.0.join("appdata/InkPop");
    migrate(&target, Some(&new_app)).unwrap();
    for (path, value) in known {
        assert_eq!(f.read(&format!("appdata/InkPop/{path}")), value);
        assert_eq!(f.read(&format!("{old}/{path}")), value);
    }
    for path in excluded {
        assert!(!target.join(path).exists());
        assert!(f.0.join(old).join(path).is_file());
    }
    assert!(!new_app.join(PORTABLE_DATA_DIR).exists());
}

#[test]
fn migrates_legacy_platform_data_once_and_preserves_existing_target_profile() {
    let f = Fixture::new();
    f.write("appdata/Photocraft/preferences.json", "legacy");
    let target = f.0.join("appdata/InkPop");
    migrate(&target, None).unwrap();
    assert_eq!(f.read("appdata/InkPop/preferences.json"), "legacy");

    f.write("appdata/InkPop/preferences.json", "new version saved");
    f.write("appdata/Photocraft/preferences.json", "old version saved later");
    f.write("appdata/Photocraft/tool-state.json", "old tool state");
    migrate(&target, None).unwrap();
    assert_eq!(f.read("appdata/InkPop/preferences.json"), "new version saved");
    assert!(!target.join("tool-state.json").exists());
}

#[test]
fn prefers_adjacent_local_profile_when_switching_off_portable_mode() {
    let f = Fixture::new();
    f.write("arbitrary-folder/PhotoCraftData/preferences.json", "portable");
    f.write("appdata/Photocraft/preferences.json", "old normal");
    let target = f.0.join("appdata/InkPop");
    let app = f.0.join("arbitrary-folder");
    assert!(portable_marker(&app).is_none());
    migrate(&target, Some(&app)).unwrap();
    assert_eq!(f.read("appdata/InkPop/preferences.json"), "portable");
}

#[test]
fn failed_migration_leaves_no_published_partial_profile() {
    let f = Fixture::new();
    f.write("appdata/Photocraft/preferences.json", "valid bytes");
    let too_large = f.0.join("appdata/Photocraft/ui.ron");
    File::create(&too_large).unwrap().set_len((16 << 20) + 1).unwrap();
    let target = f.0.join("appdata/InkPop");
    assert_eq!(migrate(&target, None).unwrap_err().kind(), io::ErrorKind::InvalidData);
    assert!(!target.exists());
    assert_eq!(f.read("appdata/Photocraft/preferences.json"), "valid bytes");
    fs::remove_file(too_large).unwrap();
    migrate(&target, None).unwrap();
    assert_eq!(f.read("appdata/InkPop/preferences.json"), "valid bytes");
    assert!(!target.join("ui.ron").exists());
}

#[test]
fn concurrent_initialization_publishes_a_complete_profile() {
    let f = Fixture::new();
    f.write("appdata/Photocraft/preferences.json", "settings");
    f.write("appdata/Photocraft/tool-state.json", "tools");
    let target = f.0.join("appdata/InkPop");
    std::thread::scope(|scope| {
        let first = scope.spawn(|| migrate(&target, None));
        let second = scope.spawn(|| migrate(&target, None));
        first.join().unwrap().unwrap();
        second.join().unwrap().unwrap();
    });
    assert_eq!(f.read("appdata/InkPop/preferences.json"), "settings");
    assert_eq!(f.read("appdata/InkPop/tool-state.json"), "tools");
    assert_eq!(fs::read_dir(target.parent().unwrap()).unwrap().count(), 3, "only source, destination and lock remain");
}

#[cfg(any(unix, windows))]
#[test]
fn linked_sources_are_never_followed() {
    let f = Fixture::new();
    f.write("other/preferences.json", "unrelated");
    f.write("appdata/Photocraft/workspace-layout.json", "workspace");
    let link = f.0.join("appdata/Photocraft/Presets");
    #[cfg(unix)]
    std::os::unix::fs::symlink(f.0.join("other"), &link).unwrap();
    #[cfg(windows)]
    {
        // Junctions, like links, redirect to directories outside the migration scope.
        let Ok(output) = std::process::Command::new("cmd").args(["/c", "mklink", "/J"]).arg(&link).arg(f.0.join("other")).output() else { return };
        if !output.status.success() {
            return;
        }
    }
    f.write("other/sample.pcbrushes", "unrelated brush");
    let target = f.0.join("appdata/InkPop");
    migrate(&target, None).unwrap();
    assert!(!target.join("Presets").exists());
    assert_eq!(f.read("appdata/InkPop/workspace-layout.json"), "workspace");
    fs::remove_dir(&link).or_else(|_| fs::remove_file(&link)).unwrap();
}
