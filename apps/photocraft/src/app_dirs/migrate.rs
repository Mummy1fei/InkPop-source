//! One-time import of the old Windows settings profile.

use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use photocraft_engine::preset_store::{self, ACTIONS_FILE, GROUP_EXT, INDEX_FILE, TIP_EXT, TIPS_DIR};

use super::{PORTABLE_DATA_DIR, portable_marker};

const SETTINGS: &[(&str, u64)] = &[
    ("preferences.json", 128 << 20),
    ("ui.ron", 16 << 20),
    ("workspace-layout.json", 16 << 20),
    ("tool-state.json", photocraft_ui_egui::tool_brush_persistence::MAX_FILE_BYTES as u64),
];
const PRESETS: &str = "Presets";
const LEGACY_RELEASES: u32 = 32;
const MAX_PRESET_ENTRIES: usize = 32_768;

/// Stage on the destination volume and publish the profile only after copying it completely.
/// Keep the lock file itself so simultaneous launches always coordinate on the same handle.
pub(super) fn migrate(target: &Path, exe: Option<&Path>) -> io::Result<()> {
    let Some(parent) = target.parent() else { return Ok(()) };
    if !target.is_absolute() || target.ancestors().any(linked) || present(target)? {
        return Ok(());
    }
    let source = exe.and_then(packaged_profile).or_else(|| Some(parent.join("Photocraft")).filter(|p| profile_exists(p)));
    let Some(source) = source else { return Ok(()) };

    fs::create_dir_all(parent)?;
    let lock_path = parent.join(".inkpop-profile-import.lock");
    if linked(&lock_path) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "linked profile migration lock"));
    }
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock_path)?;
    lock.lock()?;
    if present(target)? {
        return Ok(());
    }

    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let stage = parent.join(format!(".inkpop-profile-import-{}-{stamp}", std::process::id()));
    fs::create_dir(&stage)?;
    let stage = StagingDirectory(stage);
    let mut copied = false;
    for &(name, max) in SETTINGS {
        copied |= copy_regular(&source.join(name), &stage.0.join(name), max)?.is_some();
    }
    copied |= copy_presets(&source.join(PRESETS), &stage.0.join(PRESETS))?;
    if copied && !present(target)? {
        fs::rename(&stage.0, target)?;
    }
    Ok(())
}

struct StagingDirectory(PathBuf);

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        // A successful publish already moved this path; it cannot remove the published profile.
        if let Err(error) = fs::remove_dir_all(&self.0)
            && error.kind() != io::ErrorKind::NotFound
        {
            log::warn!("discarding staged legacy settings failed: {error}");
        }
    }
}

fn present(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

fn is_link(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    metadata.file_type().is_symlink()
}

fn linked(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|m| is_link(&m))
}

fn real_directory(path: &Path) -> bool {
    path.is_absolute() && path.is_dir() && !path.ancestors().any(linked)
}

fn regular(path: &Path) -> Option<Metadata> {
    fs::symlink_metadata(path).ok().filter(|m| m.is_file() && !is_link(m))
}

fn profile_exists(path: &Path) -> bool {
    real_directory(path) && (SETTINGS.iter().any(|(name, _)| regular(&path.join(name)).is_some()) || real_directory(&path.join(PRESETS)))
}

fn release_number(name: &str) -> Option<u32> {
    let (product, number) = name.rsplit_once("-V2.0-v")?;
    let version = number.parse::<u32>().ok()?;
    if number != version.to_string() {
        return None;
    }
    // Older InkPop portable folders included a localized label before the version.
    let localized =
        version < 120 && product.strip_prefix("InkPop-").is_some_and(|label| label.chars().count() == 3 && label.chars().all(|c| ('一'..='鿿').contains(&c)));
    (matches!(product, "InkPop" | "PhotoCraft_M") || localized).then_some(version)
}

fn packaged_profile(exe: &Path) -> Option<PathBuf> {
    let adjacent = exe.join(PORTABLE_DATA_DIR);
    if profile_exists(&adjacent) {
        return Some(adjacent);
    }
    if !real_directory(exe) {
        return None;
    }
    let name = exe.file_name()?.to_str()?;
    let latest = release_number(name)?;
    let parent = exe.parent()?;
    // Check only recognized release names, not unrelated siblings, and pick the most recently
    // saved preferences among those releases. Ties favor the newer package.
    let previous = latest.saturating_sub(LEGACY_RELEASES).max(1)..latest;
    fs::read_dir(parent)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let version = release_number(entry.file_name().to_str()?)?;
            previous.contains(&version).then(|| (version, entry.path()))
        })
        .filter(|(_, dir)| {
            real_directory(dir)
                && (regular(&dir.join("InkPop.exe")).is_some() || regular(&dir.join("photocraft.exe")).is_some())
                && portable_marker(dir).is_some_and(|marker| regular(&marker).is_some())
        })
        .filter_map(|(version, dir)| {
            let source = dir.join(PORTABLE_DATA_DIR);
            profile_exists(&source).then(|| {
                let saved = regular(&source.join("preferences.json")).and_then(|m| m.modified().ok()).unwrap_or(UNIX_EPOCH);
                (saved, version, source)
            })
        })
        .max_by_key(|(saved, version, _)| (*saved, *version))
        .map(|(_, _, source)| source)
}

fn copy_regular(from: &Path, to: &Path, max: u64) -> io::Result<Option<u64>> {
    let Some(meta) = regular(from) else { return Ok(None) };
    if meta.len() > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "legacy settings exceed the migration size limit"));
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut source = File::open(from)?.take(max.saturating_add(1));
    let mut dest = OpenOptions::new().write(true).create_new(true).open(to)?;
    let bytes = io::copy(&mut source, &mut dest)?;
    if bytes > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "legacy settings grew beyond the migration size limit"));
    }
    dest.sync_all()?;
    Ok(Some(bytes))
}

fn copy_presets(from: &Path, to: &Path) -> io::Result<bool> {
    if !real_directory(from) {
        return Ok(false);
    }
    let mut copied = false;
    let mut remaining = preset_store::MAX_STORE_BYTES;
    for (dir, tips) in [(PathBuf::new(), false), (PathBuf::from(TIPS_DIR), true)] {
        let source = from.join(&dir);
        if !real_directory(&source) {
            continue;
        }
        for (i, entry) in fs::read_dir(&source)?.enumerate() {
            if i == MAX_PRESET_ENTRIES {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "too many legacy preset entries"));
            }
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if name.len() > 128 || name.starts_with('.') || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b" _.-".contains(&b)) {
                continue;
            }
            let ext = Path::new(name).extension().and_then(|s| s.to_str());
            let maximum = match (tips, name, ext) {
                (true, _, Some(TIP_EXT)) => preset_store::MAX_TIP_BYTES,
                (false, INDEX_FILE, _) => preset_store::MAX_INDEX_BYTES,
                (false, ACTIONS_FILE, _) => preset_store::MAX_ACTIONS_BYTES,
                (false, _, Some(GROUP_EXT)) => preset_store::MAX_GROUP_BYTES,
                _ => continue,
            };
            if let Some(bytes) = copy_regular(&entry.path(), &to.join(&dir).join(name), maximum.min(remaining))? {
                remaining -= bytes;
                copied = true;
            }
        }
    }
    Ok(copied)
}

#[cfg(test)]
#[path = "migrate_tests.rs"]
mod tests;
