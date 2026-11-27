//! Native platform services: file dialogs (rfd), filesystem, codecs.

use photocraft_codecs::{ChannelLayout, EncodeOptions, Image, SampleType as CS};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Layer, Size};
use photocraft_format::RecoveryStore;
use photocraft_geom::Rect;
use photocraft_ui_egui::{ClipboardRead, Recovered, Services};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

#[path = "services_lut_v75.rs"]
mod lut_v75;

#[path = "services_clipboard_v75.rs"]
mod clipboard_v75;

/// Everything File › Open reads: PhotoCraft and Photoshop documents, flat images, and Photoshop
/// brushes (.abr) and gradients (.grd), which go to the preset libraries.
const OPEN_EXTS: &[&str] = &[
    "pcraft", "psd", "psb", "psdt", "png", "jpg", "jpeg", "tif", "tiff", "webp", "gif", "bmp", "tga", "ico", "qoi", "exr", "hdr", "pbm", "pgm", "ppm", "pam",
    "pfm", "heic", "heif", "hif", "dng", "cr2", "cr3", "nef", "nrw", "arw", "pef", "orf", "rw2", "raf", "abr", "grd", "tpl", "pctools",
];

/// File › Save As formats: (filter name, extensions). The filter matching the suggested name's
/// extension comes first. The document UI suggests PSD for added layers, while
/// an explicit flat export keeps its own filter rather than acquiring .psd.
const SAVE_FILTERS: &[(&str, &[&str])] = &[
    ("Photoshop", &["psd", "psb"]),
    ("InkPop", &["pcraft"]),
    ("PNG", &["png"]),
    ("JPEG", &["jpg"]),
    ("WebP", &["webp"]),
    ("TIFF", &["tif"]),
    ("Targa", &["tga"]),
    ("OpenEXR", &["exr"]),
];

/// [`SAVE_FILTERS`] with the one for `suggested`'s extension first.
fn save_filters(suggested: &str) -> Vec<(&'static str, &'static [&'static str])> {
    let ext = Path::new(suggested).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if ext == "json" {
        return vec![("InkPop Workspace", &["json"])];
    }
    if ext == "pctools" {
        return vec![("InkPop Tool Presets", &["pctools"])];
    }
    let mut v = SAVE_FILTERS.to_vec();
    if let Some(i) = v.iter().position(|(_, exts)| exts.contains(&ext.as_str())) {
        let f = v.remove(i);
        v.insert(0, f);
    }
    v
}

/// Per-user settings directory: `PHOTOCRAFT_CONFIG_DIR`, else `<exe dir>/PhotoCraftData` in
/// portable mode, else the platform convention. Everything the app persists lives under it; see
/// [`crate::app_dirs`].
pub fn config_dir() -> Option<PathBuf> {
    crate::app_dirs::config_dir()
}

pub fn prefs_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("preferences.json"))
}

fn tool_state_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("tool-state.json"))
}

fn load_tool_state(path: &Path) -> Result<Option<String>, String> {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let max = photocraft_ui_egui::tool_brush_persistence::MAX_FILE_BYTES;
    if file.metadata().map_err(|error| error.to_string())?.len() > max as u64 {
        return Err("last-used tool brush state exceeds 128 MiB; file kept".into());
    }
    let mut bytes = Vec::new();
    file.take(max as u64 + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() > max {
        return Err("last-used tool brush state exceeds 128 MiB; file kept".into());
    }
    String::from_utf8(bytes).map(Some).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tool_state_tests_v73 {
    use super::*;

    struct TempState {
        directory: PathBuf,
        path: PathBuf,
    }

    impl TempState {
        fn new() -> Self {
            static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let directory = std::env::temp_dir().join(format!("inkpop-tool-state-v73-{}-{stamp}-{n}", std::process::id()));
            std::fs::create_dir(&directory).unwrap();
            let path = directory.join("tool-state.json");
            Self { directory, path }
        }
    }

    impl Drop for TempState {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::remove_dir(&self.directory);
        }
    }

    #[test]
    fn atomic_tool_state_roundtrip_v73() {
        let tmp = TempState::new();
        assert_eq!(load_tool_state(&tmp.path).unwrap(), None);
        write_atomic(&tmp.path, b"{\"version\":1}").unwrap();
        assert_eq!(load_tool_state(&tmp.path).unwrap().as_deref(), Some("{\"version\":1}"));
        write_atomic(&tmp.path, b"{\"version\":2}").unwrap();
        assert_eq!(load_tool_state(&tmp.path).unwrap().as_deref(), Some("{\"version\":2}"));
        assert_eq!(std::fs::read_dir(&tmp.directory).unwrap().count(), 1);
    }

    #[test]
    fn tool_state_invalid_utf8_keeps_file_v73() {
        let tmp = TempState::new();
        std::fs::write(&tmp.path, [0xff, 0xfe, 0xfd]).unwrap();
        assert!(load_tool_state(&tmp.path).is_err());
        assert_eq!(std::fs::read(&tmp.path).unwrap(), [0xff, 0xfe, 0xfd]);
    }

    #[test]
    fn tool_state_oversized_file_rejected_by_metadata_v73() {
        let tmp = TempState::new();
        let file = std::fs::File::create(&tmp.path).unwrap();
        let length = photocraft_ui_egui::tool_brush_persistence::MAX_FILE_BYTES as u64 + 1;
        file.set_len(length).unwrap();
        drop(file);
        assert!(load_tool_state(&tmp.path).unwrap_err().contains("128 MiB"));
        assert_eq!(std::fs::metadata(&tmp.path).unwrap().len(), length);
    }
}

/// The brush preset store (one file per preset group plus tip bitmaps; see
/// `photocraft_engine::preset_store`).
pub fn presets_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("Presets"))
}

fn recovery_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("Recovery"))
}

/// Write `bytes` crash-safely (temp file beside the target, fsync, rename, directory fsync; see
/// [`photocraft_format::atomic`]). Every document write (Save, Save As, Export, Save for Web) and
/// the preferences go through here, so a failed or interrupted save never destroys the old file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    photocraft_format::atomic_write(path, bytes).map_err(|e| e.to_string())
}

type SharedRecovery = Rc<RefCell<Option<RecoveryStore>>>;

/// Run `f` on the recovery store (`Err` without a config directory). The service closures never
/// call each other, so the store is never borrowed twice.
fn with_store<R>(store: &SharedRecovery, f: impl FnOnce(&mut RecoveryStore) -> R) -> Result<R, String> {
    let mut slot = store.try_borrow_mut().map_err(|_| "crash recovery is busy".to_string())?;
    Ok(f(slot.as_mut().ok_or("no config directory")?))
}

/// Crash recovery: background incremental .pcraft autosaves into `dir` (`None`: no config
/// directory, so autosaves fail and nothing is recovered). Recovered documents keep their entries
/// until a newer autosave replaces them or they're saved or closed (see [`RecoveryStore`]).
fn recovery_services(dir: Option<PathBuf>) -> Services {
    let store: SharedRecovery = Rc::new(RefCell::new(dir.map(RecoveryStore::new)));
    let (s1, s2, s3) = (store.clone(), store.clone(), store.clone());
    Services {
        autosave: Some(Box::new(move |doc: &Arc<Document>, revision: u64, path: Option<&str>| {
            with_store(&s1, |s| s.autosave(doc, revision, path.map(str::to_string)))
        })),
        discard_autosave: Some(Box::new(move |id: u64| {
            let _ = with_store(&s2, |s| s.discard(id));
        })),
        recover: Some(Box::new(move || {
            let found = with_store(&s3, |s| s.recover()).unwrap_or_default();
            found.into_iter().map(|(e, doc)| Recovered { key: e.info.key, path: e.info.original_path, doc }).collect()
        })),
        adopt_autosave: Some(Box::new(move |id: u64, key: &str| {
            let _ = with_store(&store, |s| s.adopt(id, key));
        })),
        ..Default::default()
    }
}

/// The pixels to paste when the clipboard holds copied files rather than an image (Copy in
/// Files, Finder or Explorer puts paths on the clipboard, #338): the first file that decodes,
/// upright, as RGBA8. Files that aren't images are skipped after reading only their header.
#[cfg(test)]
fn image_from_files(paths: &[PathBuf]) -> Option<(u32, u32, Vec<u8>)> {
    checked_image_from_files(paths).ok().flatten()
}

fn checked_image_from_files(paths: &[PathBuf]) -> Result<Option<(u32, u32, Vec<u8>)>, String> {
    use std::io::Read;
    for path in paths {
        // text/uri-list lines end in CRLF (RFC 2483, and GTK writes them so), but arboard splits
        // on LF only, so a path copied in GNOME Files arrives with a trailing '\r'.
        let path = path.to_str().map_or_else(|| path.clone(), |s| PathBuf::from(s.trim_end_matches('\r')));
        let mut head = Vec::with_capacity(256);
        let Ok(mut file) = std::fs::File::open(&path) else { continue };
        if (&mut file).take(256).read_to_end(&mut head).is_err() {
            continue;
        }
        // TGA has no magic number and is only guessed from the header, so trust the extension
        // too before reading what may be a large non-image file.
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match photocraft_codecs::detect(&head) {
            Some(photocraft_codecs::Format::Tga) if photocraft_codecs::from_extension(ext) != Some(photocraft_codecs::Format::Tga) => continue,
            None => continue,
            _ => {}
        }
        let length = file.metadata().map_err(|error| error.to_string())?.len();
        let max = clipboard_v75::MAX_IMAGE_BYTES as u64;
        if length > max {
            return Err("clipboard image file exceeds the 512 MiB decode limit".into());
        }
        let mut bytes = head;
        bytes.try_reserve_exact((length as usize).saturating_sub(bytes.len())).map_err(|_| "not enough memory to read clipboard image file")?;
        file.take(max.saturating_add(1).saturating_sub(bytes.len() as u64)).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
        if bytes.len() as u64 > max {
            return Err("clipboard image file exceeds the 512 MiB decode limit".into());
        }
        // A malformed candidate may precede an actual image in a copied file list.
        if let Some(image) = clipboard_v75::decode_image_candidate(&bytes)? {
            return Ok(Some(image));
        }
    }
    Ok(None)
}

/// Stop reading palette files at the format budget before parsing or allocating the rest.
fn color_palette_file_bytes(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let max = photocraft_engine::prefs::palette::MAX_FILE_BYTES;
    if file.metadata().map_err(|error| error.to_string())?.len() > max as u64 {
        return Err("interface palette files must not exceed 128 KiB".into());
    }
    let mut bytes = Vec::new();
    file.take(max as u64 + 1).read_to_end(&mut bytes).map_err(|error| error.to_string())?;
    if bytes.len() > max {
        return Err("interface palette files must not exceed 128 KiB".into());
    }
    Ok(bytes)
}

pub fn native(automation: Option<photocraft_automation::AuthorizedWorkspace>, cc: &eframe::CreationContext<'_>) -> Services {
    let clip: Rc<RefCell<Option<arboard::Clipboard>>> = Rc::default();
    // A workspace export is an interactive Save As operation. Parent it explicitly
    // to the editor so it cannot open behind a floating document/tool window.
    let workspace_save = rfd::FileDialog::new().set_parent(cc).set_title("Export Workspace").add_filter("InkPop Workspace", &["json"]);
    let colors_open = rfd::FileDialog::new().set_parent(cc).add_filter("InkPop Color Palette", &["json"]);
    let colors_save = colors_open.clone();
    let lut_open = rfd::FileDialog::new().set_parent(cc).add_filter("LUT", lut_v75::EXTENSIONS);
    let automation_read = automation.clone().map(|workspace| {
        Box::new(move |path: &str| {
            let bytes = workspace.read(path).map_err(|error| error.to_string())?;
            let name = Path::new(path).file_name().and_then(|name| name.to_str()).unwrap_or(path).to_string();
            Ok((name, bytes))
        }) as photocraft_ui_egui::AutomationReadFn
    });
    let automation_write = automation.clone().map(|workspace| {
        Box::new(move |path: &str, bytes: &[u8]| workspace.write(path, bytes).map_err(|error| error.to_string())) as photocraft_ui_egui::AutomationWriteFn
    });
    let automation_write_new_files = automation.clone().map(|workspace| {
        Box::new(move |files: &[(String, Vec<u8>)]| workspace.write_new_files(files).map_err(|error| error.to_string())) as photocraft_ui_egui::WriteNewFilesFn
    });
    let step: fn(&str, &serde_json::Value) -> photocraft_engine::Result<()> = photocraft_automation::workspace::authorize_desktop_engine_step;
    let automation_authorize = automation.is_some().then_some(step);
    let automation_command = automation.map(|_| {
        Box::new(|id: &str, params: &serde_json::Value| {
            photocraft_automation::workspace::authorize_desktop_engine_command(id, params).map_err(|error| error.to_string())
        }) as photocraft_ui_egui::AutomationCommandFn
    });
    Services {
        request_recent_thumbnail: Some(crate::recent_preview::service()),
        diagnostic_event: Some(Box::new(crate::crash_reports::diagnostic)),
        diagnostic_idle: Some(Box::new(crate::crash_reports::diagnostic_idle)),
        warning_event: Some(Box::new(crate::crash_reports::warning)),
        import: Some(Box::new(|name: &str, bytes: &[u8]| {
            crate::crash_guard::guard("Open", || photocraft_io::import(name, bytes).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string()))
        })),
        export: Some(Box::new(|doc: &Document, path: &str, settings: &photocraft_ui_egui::ExportSettings| {
            let mut opts = photocraft_io::ExportOptions::default();
            if let Some(q) = settings.jpeg_quality {
                opts.encode.jpeg_quality = q;
            }
            opts.encode.webp_lossless = settings.webp_lossless;
            if let Some(q) = settings.webp_quality {
                opts.encode.webp_quality = q;
            }
            opts.tiff_layers = settings.tiff_layers;
            opts.xmp = if settings.xmp_all { photocraft_io::XmpEmbed::All } else { photocraft_io::XmpEmbed::None };
            crate::crash_guard::guard("Export", || photocraft_io::export(doc, path, &opts).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string()))
        })),
        pick_open: Some(Box::new(|| {
            let path = rfd::FileDialog::new().add_filter("All Formats", OPEN_EXTS).add_filter("InkPop", &["pcraft"]).pick_file()?;
            // A read failure goes back to the app, which reports it like any other open failure.
            let bytes = photocraft_format::read_file(&path).map_err(|e| e.to_string());
            Some((path.to_string_lossy().to_string(), bytes))
        })),
        pick_lut: Some(Box::new(move || {
            let title = photocraft_ui_egui::i18n::tr(photocraft_ui_egui::i18n::current(), "Import LUT…");
            let path = lut_open.clone().set_title(title).pick_file()?;
            Some((path.to_string_lossy().into_owned(), lut_v75::read(&path)))
        })),
        pick_open_workspace: Some(Box::new(|| {
            use std::io::Read;
            let path = rfd::FileDialog::new().add_filter("InkPop Workspace", &["json"]).pick_file()?;
            let bytes = (|| -> Result<Vec<u8>, String> {
                let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                let mut bytes = Vec::new();
                file.take(1_048_577).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                if bytes.len() > 1_048_576 {
                    return Err("workspace file exceeds 1 MiB".into());
                }
                Ok(bytes)
            })();
            Some((path.to_string_lossy().into_owned(), bytes))
        })),
        pick_open_colors: Some(Box::new(move || {
            let title = photocraft_ui_egui::i18n::tr(photocraft_ui_egui::i18n::current(), "Import Colors…");
            let path = colors_open.clone().set_title(title).pick_file()?;
            Some((path.to_string_lossy().into_owned(), color_palette_file_bytes(&path)))
        })),
        pick_open_paths: Some(Box::new(|| {
            rfd::FileDialog::new()
                .add_filter("All Formats", OPEN_EXTS)
                .add_filter("InkPop", &["pcraft"])
                .pick_files()
                .map(|paths| paths.into_iter().map(|path| path.to_string_lossy().into_owned()).collect())
        })),
        pick_save: Some(Box::new(|suggested: &str| {
            let p = std::path::Path::new(suggested);
            let mut d = rfd::FileDialog::new();
            for (name, exts) in save_filters(suggested) {
                d = d.add_filter(name, exts);
            }
            if let Some(name) = p.file_name() {
                d = d.set_file_name(name.to_string_lossy());
            }
            Some(d.save_file()?.to_string_lossy().to_string())
        })),
        pick_save_workspace: Some(Box::new(move |suggested: &str| {
            Some(workspace_save.clone().set_file_name(suggested).save_file()?.to_string_lossy().into_owned())
        })),
        pick_save_colors: Some(Box::new(move |suggested: &str| {
            let title = photocraft_ui_egui::i18n::tr(photocraft_ui_egui::i18n::current(), "Export Colors…");
            Some(colors_save.clone().set_title(title).set_file_name(suggested).save_file()?.to_string_lossy().into_owned())
        })),
        write: Some(Box::new(|path: &str, bytes: &[u8]| write_atomic(Path::new(path), bytes))),
        write_new_files: Some(Box::new(|files: &[(String, Vec<u8>)]| {
            let borrowed: Vec<_> = files.iter().map(|(path, bytes)| (path.as_str(), bytes.as_slice())).collect();
            photocraft_engine::ps_collaboration_cmds::write_new_files(&borrowed).map_err(|error| error.to_string())
        })),
        automation_write_new_files,
        automation_read,
        automation_write,
        automation_command,
        automation_authorize,
        encode_png: Some(Box::new(|w, h, rgba| {
            let img = Image::from_u8(w, h, ChannelLayout::Rgba, rgba.to_vec()).map_err(|e| e.to_string())?;
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).map_err(|e| e.to_string())
        })),
        inbox: None,
        open_url: Some(Box::new(|url: &str| open::that(url).map_err(|e| e.to_string()))),
        clipboard_set_image: Some({
            let clip = clip.clone();
            Box::new(move |w: u32, h: u32, px: &[u8]| {
                let mut slot = clip.try_borrow_mut().map_err(|_| "clipboard is busy".to_string())?;
                let cb = match slot.as_mut() {
                    Some(c) => c,
                    None => slot.insert(arboard::Clipboard::new().map_err(|e| e.to_string())?),
                };
                cb.set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: std::borrow::Cow::Borrowed(px) }).map_err(|e| e.to_string())
            })
        }),
        clipboard_get_image_checked: Some({
            let clip = clip.clone();
            #[cfg(target_os = "windows")]
            let mut revision_cache = clipboard_v75::RevisionCache::default();
            Box::new(move || {
                #[cfg(target_os = "windows")]
                let sequence = if clipboard_win::is_format_avail(clipboard_win::formats::CF_HDROP) {
                    // The revision freezes a path list, not the file bytes. Even an icon
                    // fallback must not hide a later file rewrite at the same path.
                    None
                } else {
                    clipboard_win::seq_num().map(|value| value.get())
                };
                #[cfg(target_os = "windows")]
                if revision_cache.unchanged(sequence) {
                    // The session already owns this exact clipboard revision. A second
                    // paste shares that surface instead of re-decoding another full image.
                    return Ok(ClipboardRead::Unchanged);
                }
                let mut slot = clip.try_borrow_mut().map_err(|_| "clipboard is busy")?;
                let cb = match slot.as_mut() {
                    Some(c) => c,
                    None => slot.insert(arboard::Clipboard::new().map_err(|error| error.to_string())?),
                };
                // Copied files first: Finder also puts the file's icon on the clipboard as an
                // image, which would otherwise paste instead of the file.
                if let Ok(paths) = cb.get().file_list()
                    && let Some(image) = checked_image_from_files(&paths)?
                {
                    #[cfg(target_os = "windows")]
                    {
                        // A path can be rewritten without changing the OS clipboard.
                        return revision_cache.imported(None, image);
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        let (width, height, pixels) = image;
                        return Ok(ClipboardRead::Image { width, height, pixels });
                    }
                }
                #[cfg(target_os = "windows")]
                {
                    match clipboard_v75::read_windows_image()? {
                        Some(image) => revision_cache.imported(sequence, image),
                        None => Ok(ClipboardRead::Empty),
                    }
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let img = match cb.get_image() {
                        Ok(image) => image,
                        Err(arboard::Error::ContentNotAvailable) => return Ok(ClipboardRead::Empty),
                        Err(error) => return Err(error.to_string()),
                    };
                    let width = u32::try_from(img.width).map_err(|_| "invalid clipboard image width")?;
                    let height = u32::try_from(img.height).map_err(|_| "invalid clipboard image height")?;
                    let expected = clipboard_v75::dimensions(width, height)?;
                    if img.bytes.len() != expected {
                        return Err("clipboard image pixel data has an invalid length".into());
                    }
                    Ok(ClipboardRead::Image { width, height, pixels: img.bytes.into_owned() })
                }
            })
        }),
        load_workspace: Some(Box::new(|| std::fs::read_to_string(config_dir()?.join("workspace-layout.json")).ok())),
        save_workspace: Some(Box::new(|text: &str| write_atomic(&config_dir().ok_or("no config directory")?.join("workspace-layout.json"), text.as_bytes()))),
        load_prefs: Some(Box::new(|| std::fs::read_to_string(prefs_file()?).ok())),
        save_prefs: Some(Box::new(|text: &str| write_atomic(&prefs_file().ok_or("no config directory")?, text.as_bytes()))),
        load_tool_state: Some(Box::new(|| tool_state_file().map_or(Ok(None), |path| load_tool_state(&path)))),
        save_tool_state: Some(Box::new(|text| {
            if text.len() > photocraft_ui_egui::tool_brush_persistence::MAX_FILE_BYTES {
                return Err("last-used tool brush state exceeds 128 MiB; previous file kept".into());
            }
            write_atomic(&tool_state_file().ok_or("no config directory")?, text.as_bytes())
        })),
        append_text: Some(Box::new(|path: &str, text: &str| {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).map_err(|e| e.to_string())?;
            f.write_all(text.as_bytes()).map_err(|e| e.to_string())
        })),
        // Set by main once the Apple-event handlers are connected (macOS).
        os_events: None,
        // Set by main, which starts loading the store before the window opens.
        preset_store: None,
        is_wayland: false,
        ..recovery_services(recovery_dir())
    }
}

/// Flat-image import via photocraft-codecs (kept for reference/tests; the app uses photocraft-io).
#[allow(dead_code)]
pub fn import_flat(name: &str, bytes: &[u8]) -> Result<Document, String> {
    let img = photocraft_codecs::decode(bytes).map_err(|e| e.to_string())?;
    let (w, h) = (img.width(), img.height());
    let depth = match img.sample_type() {
        CS::U8 => SampleType::U8,
        CS::U16 => SampleType::U16,
        _ => SampleType::F32,
    };
    let gray = matches!(img.layout(), ChannelLayout::Gray | ChannelLayout::GrayA);
    let cmyk = matches!(img.layout(), ChannelLayout::Cmyk | ChannelLayout::CmykA);
    let mode = if gray {
        ColorMode::Grayscale
    } else if cmyk {
        ColorMode::Cmyk
    } else {
        ColorMode::Rgb
    };
    let target = match mode {
        ColorMode::Grayscale => ChannelLayout::GrayA,
        ColorMode::Cmyk => ChannelLayout::CmykA,
        _ => ChannelLayout::Rgba,
    };
    let sample = match depth {
        SampleType::U8 => CS::U8,
        SampleType::U16 => CS::U16,
        SampleType::F32 => CS::F32,
    };
    let conv = img.convert(target, sample);
    let stem = std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or(name.to_string());
    let mut doc = Document::new(stem, Size::new(w, h), mode, depth);
    doc.icc_profile = img.icc.clone().map(std::sync::Arc::new);
    if let Some((x, _)) = img.meta.dpi {
        doc.resolution_dpi = x;
    }
    let mut layer = Layer::raster("Background", doc.pixel_format());
    let data = conv.to_normalized();
    layer.surface_mut().ok_or("new raster layer has no pixels")?.write_region(Rect::from_xywh(0, 0, w, h), &data);
    doc.layers.push(layer);
    Ok(doc)
}

#[allow(dead_code)]
pub fn export_flat(doc: &Document, path: &str) -> Result<Vec<u8>, String> {
    let format = photocraft_codecs::from_extension(path).ok_or_else(|| format!("unknown file type for {path}"))?;
    let buf = photocraft_compose::flatten(doc);
    let (w, h) = (buf.rect.width(), buf.rect.height());
    let data: Vec<f32> = buf.px.iter().flat_map(|p| *p).collect();
    let img = match doc.depth {
        SampleType::U8 => Image::from_u8(w, h, ChannelLayout::Rgba, buf.to_rgba8().pixels),
        SampleType::U16 => Image::from_u16(w, h, ChannelLayout::Rgba, &data.iter().map(|v| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16).collect::<Vec<_>>()),
        SampleType::F32 => Image::from_f32(w, h, ChannelLayout::Rgba, &data),
    }
    .map_err(|e| e.to_string())?;
    let img = match &doc.icc_profile {
        Some(icc) if doc.mode == ColorMode::Rgb => img.with_icc(Some((**icc).clone())),
        _ => img,
    };
    photocraft_codecs::encode(&img, format, &EncodeOptions::default()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::Session;
    use photocraft_format::list_recovery;
    use photocraft_ui_egui::{PhotocraftApp, prefs_ui};
    use serde_json::json;

    #[test]
    fn color_palette_native_reader_bounds_the_file_before_parsing_v68() {
        let path = std::env::temp_dir().join(format!("inkpop-palette-budget-{}.json", std::process::id()));
        let max = photocraft_engine::prefs::palette::MAX_FILE_BYTES;
        std::fs::write(&path, vec![b' '; max]).unwrap();
        assert_eq!(color_palette_file_bytes(&path).unwrap().len(), max);
        std::fs::write(&path, vec![b' '; max + 1]).unwrap();
        assert!(color_palette_file_bytes(&path).unwrap_err().contains("128 KiB"));
        std::fs::remove_file(&path).unwrap();
        assert!(color_palette_file_bytes(&path).is_err());
    }

    /// "Export As" formats must lead with their own filter, or the save panel appends the first one's extension (`photo.webp.psd`).
    #[test]
    fn save_filters_lead_with_every_export_format() {
        for ext in ["psd", "psb", "pcraft", "png", "jpg", "webp", "tif", "tga", "json", "pctools"] {
            assert!(save_filters(&format!("photo.{ext}"))[0].1.contains(&ext), "{ext}");
        }
        assert_eq!(save_filters("地图.psd")[0].0, "Photoshop");
    }

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
    const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];

    /// One app launch with crash recovery in `dir` (dropping it is the crash: background writes
    /// already queued finish, nothing else runs).
    fn launch(dir: &Path) -> PhotocraftApp {
        PhotocraftApp::new(Session::new(), recovery_services(Some(dir.to_path_buf())))
    }

    /// The first pixel of each open document, and whether it's unsaved.
    fn open_docs(app: &PhotocraftApp) -> Vec<([f32; 4], bool)> {
        app.session.documents().iter().map(|d| (photocraft_compose::flatten(&d.doc).px.first().copied().unwrap_or_default(), d.is_dirty())).collect()
    }

    fn index_of(app: &PhotocraftApp, color: [f32; 4]) -> usize {
        open_docs(app).iter().position(|(c, _)| *c == color).unwrap()
    }

    fn new_doc(app: &mut PhotocraftApp, color: &str) {
        app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
        app.run("edit.fill", json!({"color": color})).unwrap();
    }

    fn autosave(app: &mut PhotocraftApp, ctx: &egui::Context) {
        prefs_ui::autosave_now(app);
        prefs_ui::tick(app, ctx);
    }

    #[test]
    fn recovered_documents_survive_a_second_crash_until_saved_or_closed() {
        let dir = std::env::temp_dir().join(format!("photocraft-recovery-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let ctx = egui::Context::default();
        // Launch 1: two unsaved documents are autosaved, then the app crashes.
        {
            let mut app = launch(&dir);
            new_doc(&mut app, "#ff0000");
            new_doc(&mut app, "#0000ff");
            autosave(&mut app, &ctx);
        }
        assert_eq!(list_recovery(&dir).len(), 2);
        // Launch 2 recovers both and crashes again before its first autosave (#444).
        {
            let mut app = launch(&dir);
            let mut docs = open_docs(&app);
            docs.sort_by(|a, b| a.0[0].total_cmp(&b.0[0]));
            assert_eq!(docs, [(BLUE, true), (RED, true)], "recovered and still unsaved");
            prefs_ui::tick(&mut app, &ctx);
        }
        assert_eq!(list_recovery(&dir).len(), 2, "recovering keeps the entries on disk");
        // Launch 3 has both again; autosaving unchanged documents adds nothing, a new document
        // (whose id may repeat one from an earlier launch) gets an entry of its own.
        {
            let mut app = launch(&dir);
            assert_eq!(app.session.documents().len(), 2);
            autosave(&mut app, &ctx);
            new_doc(&mut app, "#00ff00");
            autosave(&mut app, &ctx);
        }
        assert_eq!(list_recovery(&dir).len(), 3);
        // Launch 4: closing and saving recovered documents removes their entries, no duplicates.
        let mut app = launch(&dir);
        assert_eq!(app.session.documents().len(), 3);
        let red = index_of(&app, RED);
        app.run("file.close", json!({"document": red})).unwrap();
        // Saving marks the revision saved (the file write itself is the save service's job).
        assert!(app.session.set_active(index_of(&app, BLUE)));
        let st = app.session.active_mut().unwrap();
        st.saved_revision = st.revision;
        prefs_ui::tick(&mut app, &ctx);
        let left = list_recovery(&dir);
        assert_eq!(left.len(), 1);
        assert_eq!(photocraft_compose::flatten(&photocraft_format::recover(&left[0]).unwrap()).px.first().copied(), Some(GREEN));
        let green = index_of(&app, GREEN);
        app.run("file.close", json!({"document": green})).unwrap();
        prefs_ui::tick(&mut app, &ctx);
        assert!(list_recovery(&dir).is_empty());
        drop(app);
        let _ = std::fs::remove_dir_all(&dir);
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    fn temp(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("photocraft-services-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn red_png(w: u32, h: u32) -> Vec<u8> {
        let img = Image::from_u8(w, h, ChannelLayout::Rgba, [255u8, 0, 0, 255].repeat((w * h) as usize)).unwrap();
        photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &EncodeOptions::default()).unwrap()
    }

    /// A 1×1 uncompressed 32-bit TGA (TGA has no magic number).
    fn tga_1x1() -> Vec<u8> {
        let mut b = vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 32, 8];
        b.extend_from_slice(&[0, 0, 255, 255]); // BGRA red
        b
    }

    #[test]
    fn copied_image_file_pastes_its_pixels() {
        let dir = temp("png");
        let png = dir.join("red.png");
        std::fs::write(&png, red_png(3, 2)).unwrap();
        let (w, h, px) = image_from_files(&[png]).unwrap();
        assert_eq!((w, h), (3, 2));
        assert_eq!(px, [255u8, 0, 0, 255].repeat(6));
    }

    #[test]
    fn copied_image_file_rewrite_is_not_hidden_by_clipboard_revision_v75() {
        let dir = temp("clipboard-file-rewrite-v75");
        let path = dir.join("mutable.png");
        std::fs::write(&path, red_png(2, 1)).unwrap();
        let mut cache = clipboard_v75::RevisionCache::default();
        cache.imported(Some(77), (1, 1, vec![0, 255, 0, 255])).unwrap();
        assert!(cache.unchanged(Some(77)));
        let first = checked_image_from_files(std::slice::from_ref(&path)).unwrap().unwrap();
        let first = cache.imported(None, first).unwrap();
        assert!(matches!(first, ClipboardRead::Image { pixels, .. } if pixels == [255, 0, 0, 255].repeat(2)));
        assert!(!cache.unchanged(Some(77)), "copied files invalidate a previous immutable-bitmap revision");
        let blue = Image::from_u8(2, 1, ChannelLayout::Rgba, [0, 0, 255, 255].repeat(2)).unwrap();
        let encoded = photocraft_codecs::encode(&blue, photocraft_codecs::Format::Png, &EncodeOptions::default()).unwrap();
        std::fs::write(&path, encoded).unwrap();
        let next = checked_image_from_files(std::slice::from_ref(&path)).unwrap().unwrap();
        let next = cache.imported(None, next).unwrap();
        assert!(matches!(next, ClipboardRead::Image { pixels, .. } if pixels == [0, 0, 255, 255].repeat(2)));
        assert!(!cache.unchanged(Some(77)), "the same path list cannot cache external file bytes");
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn crlf_from_a_uri_list_is_ignored() {
        // arboard splits text/uri-list on LF, so GNOME Files' CRLF list leaves a '\r' behind.
        let dir = temp("crlf");
        let png = dir.join("red image.png");
        std::fs::write(&png, red_png(2, 2)).unwrap();
        let with_cr = PathBuf::from(format!("{}\r", png.display()));
        assert_eq!(image_from_files(&[with_cr]).map(|(w, h, _)| (w, h)), Some((2, 2)));
    }

    #[test]
    fn non_images_are_skipped_for_the_first_image() {
        let dir = temp("mixed");
        let (txt, png) = (dir.join("notes.txt"), dir.join("red.png"));
        std::fs::write(&txt, b"not an image").unwrap();
        std::fs::write(&png, red_png(4, 1)).unwrap();
        let missing = dir.join("gone.png");
        assert_eq!(image_from_files(&[missing.clone(), txt.clone(), dir.clone(), png]).map(|(w, h, _)| (w, h)), Some((4, 1)));
        assert!(image_from_files(&[missing, txt, dir]).is_none());
        assert!(image_from_files(&[]).is_none());
    }

    #[test]
    fn a_damaged_image_is_skipped_not_a_panic() {
        let dir = temp("damaged");
        let bad = dir.join("cut.png");
        std::fs::write(&bad, &red_png(8, 8)[..20]).unwrap();
        assert!(image_from_files(&[bad]).is_none());
    }

    #[test]
    fn copied_image_file_over_budget_is_rejected_before_read_v75() {
        let dir = temp("clipboard-limit-v75");
        let path = dir.join("large.png");
        std::fs::write(&path, red_png(1, 1)).unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(clipboard_v75::MAX_IMAGE_BYTES as u64 + 1).unwrap();
        drop(file);
        let result = checked_image_from_files(std::slice::from_ref(&path));
        assert!(result.unwrap_err().contains("512 MiB"));
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn tga_is_trusted_only_with_its_extension() {
        let dir = temp("tga");
        let (tga, bin) = (dir.join("red.tga"), dir.join("red.bin"));
        std::fs::write(&tga, tga_1x1()).unwrap();
        std::fs::write(&bin, tga_1x1()).unwrap();
        assert_eq!(image_from_files(&[tga]), Some((1, 1, vec![255, 0, 0, 255])));
        assert!(image_from_files(&[bin]).is_none());
    }
}
