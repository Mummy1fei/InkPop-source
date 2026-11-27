//! Bounded, local diagnostic records. No screenshots, documents, presets,
//! command arguments, control tokens, or arbitrary panic payloads are recorded.
//! Rust panics have symbol backtraces; native exit reports have an exit code,
//! not an invented native stack. An orphan marker supplies evidence next start
//! when the independent Windows monitor could not complete.

use photocraft_ui_egui::{DiagnosticEvent, DiagnosticOperation, DiagnosticStage, DiagnosticTool};
use serde_json::{Value, json};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

const MAX_REPORT_BYTES: usize = 128 * 1024;
const MAX_REPORTS: usize = 20;
const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024;
const MAX_MARKER_BYTES: u64 = 16 * 1024;
const MONITOR_FLAG: &str = "--inkpop-crash-monitor";
static DIRECTORY: OnceLock<Option<PathBuf>> = OnceLock::new();
static STATE: Mutex<Option<Session>> = Mutex::new(None);
static REPORTING_PANIC: AtomicBool = AtomicBool::new(false);
static SERIAL: AtomicU64 = AtomicU64::new(0);
static DIAGNOSTIC_DIRTY: AtomicBool = AtomicBool::new(false);
const DIAGNOSTIC_EVENTS: usize = 16;
const DIAGNOSTIC_IDLE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
const DIAGNOSTIC_RETRY_LIMIT: u8 = 3;

fn timestamp() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()
}

fn bounded(text: &str, characters: usize) -> String {
    text.chars().filter(|character| !character.is_control() || *character == '\n' || *character == '\t').take(characters).collect()
}

fn linked(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else { return false };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    metadata.file_type().is_symlink()
}

fn safe_directory(path: &Path) -> bool {
    path.is_absolute() && !path.ancestors().any(linked) && path.is_dir()
}

fn choose_directory(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    for path in candidates {
        if path.ancestors().any(linked) || std::fs::create_dir_all(&path).is_err() || !safe_directory(&path) {
            continue;
        }
        let probe = path.join(format!(".inkpop-probe-{}-{}", std::process::id(), timestamp()));
        if OpenOptions::new().write(true).create_new(true).open(&probe).is_ok() {
            let _ = std::fs::remove_file(probe);
            return Some(path);
        }
    }
    None
}

/// The actual writable report folder, for the desktop UI's "Open crash reports" action.
pub fn directory() -> Option<PathBuf> {
    DIRECTORY.get_or_init(|| choose_directory(crate::app_dirs::crash_report_candidates())).clone()
}

pub fn open_directory() -> Result<(), String> {
    let path = directory().ok_or("No writable crash-report directory is available")?;
    open::that(path).map_err(|error| error.to_string())
}

fn run_id() -> String {
    format!("{:032x}-{:08x}-{:08x}", timestamp(), std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed))
}

fn valid_id(id: &str) -> bool {
    id.len() == 50 && id.bytes().enumerate().all(|(position, byte)| if matches!(position, 32 | 41) { byte == b'-' } else { byte.is_ascii_hexdigit() })
}

fn marker_path(dir: &Path, id: &str) -> Option<PathBuf> {
    valid_id(id).then(|| dir.join(format!("running-{id}.json")))
}

fn context(id: &str, pid: u32) -> Value {
    json!({"schema":1,"product":"InkPop","version":photocraft_engine::build_info::custom_version(),
        "baseVersion":photocraft_engine::build_info::long_version(),"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,
        "processId":pid,"session":id,"startedUnixNanos":timestamp().to_string(),
        "gpu":{"phase":"adapter-not-available"},"privacy":"No document pixels, presets, arguments, tokens, or arbitrary panic payloads."})
}

/// Copy only known diagnostic fields out of a marker; never publish an arbitrary
/// marker object, even if another process modified it on disk.
fn safe_context(value: &Value) -> Value {
    let mut result = context("unknown", value.get("processId").and_then(Value::as_u64).and_then(|pid| u32::try_from(pid).ok()).unwrap_or(0));
    for key in ["version", "baseVersion", "os", "architecture", "session", "startedUnixNanos"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            result[key] = json!(bounded(text, 160));
        }
    }
    if let Some(gpu) = value.get("gpu") {
        result["gpu"] = gpu_fields(gpu);
    }
    if let Some(diagnostics) = value.get("diagnostics").and_then(safe_diagnostics) {
        result["diagnostics"] = diagnostics;
    }
    result
}

fn gpu_fields(gpu: &Value) -> Value {
    let mut result = json!({});
    for key in ["phase", "backend", "adapter", "driver", "driverInfo", "deviceType"] {
        if let Some(text) = gpu.get(key).and_then(Value::as_str) {
            result[key] = json!(bounded(text, 256));
        }
    }
    result
}

fn valid_diagnostic_size(value: &Value) -> Option<Value> {
    let items = value.as_array()?;
    if items.len() != 2 {
        return None;
    }
    let w = items.first()?.as_u64()?;
    let h = items.get(1)?.as_u64()?;
    ((1..=262_144).contains(&w) && (1..=262_144).contains(&h) && w.checked_mul(h)? <= (1 << 30)).then(|| json!([w, h]))
}

fn safe_diagnostic_event(event: &Value) -> Option<Value> {
    let stage = event.get("stage")?.as_str()?;
    if !matches!(
        stage,
        "place-source-ready" | "preview-ready" | "transform-confirm-start" | "transform-command-finished" | "preview-released" | "ui-frame-built"
    ) {
        return None;
    }
    let operation = event.get("operation")?.as_str()?;
    if !matches!(operation, "place" | "paste" | "transform") {
        return None;
    }
    let tool = event.get("tool")?.as_str()?;
    if !matches!(tool, "brush" | "eraser" | "mixer-brush" | "smudge" | "move" | "other") {
        return None;
    }
    let sequence = event.get("sequence")?.as_u64()?;
    let elapsed_ms = event.get("elapsedMs")?.as_u64()?;
    let mut out = json!({"stage":stage,"operation":operation,"tool":tool,"sequence":sequence,"elapsedMs":elapsed_ms});
    for key in ["sourceSize", "targetSize"] {
        if let Some(size) = event.get(key).and_then(valid_diagnostic_size) {
            out[key] = size;
        }
    }
    for key in ["isSmart", "success"] {
        if let Some(value) = event.get(key).and_then(Value::as_bool) {
            out[key] = json!(value);
        }
    }
    if let Some(backend) =
        event.get("backend").and_then(Value::as_str).filter(|v| matches!(*v, "auto" | "vulkan" | "dx12" | "metal" | "gl" | "cpu" | "browser-webgpu" | "noop"))
    {
        out["backend"] = json!(backend);
    }
    Some(out)
}

fn safe_diagnostics(value: &Value) -> Option<Value> {
    let sequence = value.get("sequence")?.as_u64()?;
    let persisted = value.get("persistedSequence").and_then(Value::as_u64).unwrap_or(0).min(sequence);
    let events: Vec<_> = value.get("events")?.as_array()?.iter().rev().take(DIAGNOSTIC_EVENTS).filter_map(safe_diagnostic_event).collect();
    let events: Vec<_> = events.into_iter().rev().collect();
    Some(json!({"schema":1,"sequence":sequence,"persistedSequence":persisted,"events":events}))
}

fn diagnostic_stage(stage: DiagnosticStage) -> &'static str {
    match stage {
        DiagnosticStage::PlaceSourceReady => "place-source-ready",
        DiagnosticStage::PreviewReady => "preview-ready",
        DiagnosticStage::TransformConfirmStart => "transform-confirm-start",
        DiagnosticStage::TransformCommandFinished => "transform-command-finished",
        DiagnosticStage::PreviewReleased => "preview-released",
        DiagnosticStage::UiFrameBuilt => "ui-frame-built",
    }
}

fn diagnostic_is_critical(stage: DiagnosticStage) -> bool {
    matches!(stage, DiagnosticStage::TransformConfirmStart | DiagnosticStage::TransformCommandFinished | DiagnosticStage::PreviewReleased)
}

fn diagnostic_record(event: DiagnosticEvent, sequence: u64, elapsed_ms: u64, backend: Option<&str>) -> Value {
    let operation = match event.operation {
        DiagnosticOperation::Place => "place",
        DiagnosticOperation::Paste => "paste",
        DiagnosticOperation::Transform => "transform",
    };
    let tool = match event.tool {
        DiagnosticTool::Brush => "brush",
        DiagnosticTool::Eraser => "eraser",
        DiagnosticTool::MixerBrush => "mixer-brush",
        DiagnosticTool::Smudge => "smudge",
        DiagnosticTool::Move => "move",
        DiagnosticTool::Other => "other",
    };
    let mut value = json!({"stage":diagnostic_stage(event.stage),"operation":operation,"tool":tool,"sequence":sequence,"elapsedMs":elapsed_ms});
    if let Some(size) = event.source_size {
        value["sourceSize"] = json!(size);
    }
    if let Some(size) = event.target_size {
        value["targetSize"] = json!(size);
    }
    if let Some(smart) = event.is_smart {
        value["isSmart"] = json!(smart);
    }
    if let Some(success) = event.success {
        value["success"] = json!(success);
    }
    if let Some(backend) = backend {
        value["backend"] = json!(backend);
    }
    safe_diagnostic_event(&value).unwrap_or_else(|| json!({}))
}

fn read_marker(path: &Path) -> Option<Value> {
    if linked(path) || std::fs::metadata(path).ok()?.len() > MAX_MARKER_BYTES {
        return None;
    }
    let mut file = File::open(path).ok()?;
    read_marker_file(&mut file)
}

fn read_marker_file(file: &mut File) -> Option<Value> {
    if file.metadata().ok()?.len() > MAX_MARKER_BYTES {
        return None;
    }
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_MARKER_BYTES).read_to_end(&mut bytes).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn report_name(name: &str) -> bool {
    name.strip_prefix("InkPop-crash-").and_then(|name| name.strip_suffix(".json")).is_some_and(valid_id)
}

fn rotate(dir: &Path, incoming: u64) -> std::io::Result<()> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else { continue };
        if !report_name(name) || linked(&path) || !entry.file_type()?.is_file() {
            continue;
        }
        if files.len() >= 4096 {
            // The directory contains only self-owned reports here; bound the
            // inventory too rather than allocating one entry per foreign file.
            std::fs::remove_file(path)?;
        } else {
            files.push((name.to_owned(), path.clone(), entry.metadata()?.len()));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut total = files.iter().map(|row| row.2).fold(0u64, u64::saturating_add);
    let mut count = files.len();
    for (_, path, bytes) in files {
        if count < MAX_REPORTS && total.saturating_add(incoming) <= MAX_TOTAL_BYTES {
            break;
        }
        std::fs::remove_file(path)?;
        count = count.saturating_sub(1);
        total = total.saturating_sub(bytes);
    }
    Ok(())
}

fn write_report(dir: &Path, mut report: Value) -> std::io::Result<PathBuf> {
    if !safe_directory(dir) {
        return Err(std::io::Error::other("unsafe report directory"));
    }
    report["recordedUnixNanos"] = json!(timestamp().to_string());
    let bytes = serde_json::to_vec_pretty(&report)?;
    if bytes.len() > MAX_REPORT_BYTES {
        return Err(std::io::Error::other("crash report exceeds 128 KiB"));
    }
    let lock_path = dir.join(".inkpop-report-lock");
    if linked(&lock_path) {
        return Err(std::io::Error::other("linked report lock"));
    }
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock_path)?;
    lock.try_lock().map_err(|_| std::io::Error::other("another report writer is active"))?;
    rotate(dir, bytes.len() as u64)?;
    let path = dir.join(format!("InkPop-crash-{}.json", run_id()));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&path)?;
    file.write_all(&bytes)?;
    file.sync_data()?;
    Ok(path)
}

struct Session {
    dir: PathBuf,
    id: String,
    file: File,
    details: Value,
    diagnostic_started: std::time::Instant,
    diagnostic_last_attempt: std::time::Instant,
    diagnostic_pending: bool,
    diagnostic_failures: u8,
}

impl Session {
    fn begin(dir: PathBuf) -> std::io::Result<Self> {
        recover_orphans(&dir);
        let id = run_id();
        let path = marker_path(&dir, &id).ok_or_else(|| std::io::Error::other("invalid crash session"))?;
        let file = OpenOptions::new().read(true).write(true).create_new(true).open(path)?;
        file.try_lock().map_err(|_| std::io::Error::other("could not lock crash marker"))?;
        let now = std::time::Instant::now();
        let mut session = Self {
            dir,
            id: id.clone(),
            file,
            details: context(&id, std::process::id()),
            diagnostic_started: now,
            diagnostic_last_attempt: now,
            diagnostic_pending: false,
            diagnostic_failures: 0,
        };
        session.persist()?;
        Ok(session)
    }

    fn persist(&mut self) -> std::io::Result<()> {
        let mut snapshot = self.details.clone();
        if let Some(diagnostics) = snapshot.get_mut("diagnostics").and_then(Value::as_object_mut) {
            let sequence = diagnostics.get("sequence").and_then(Value::as_u64).unwrap_or(0);
            diagnostics.insert("persistedSequence".into(), json!(sequence));
        }
        let bytes = serde_json::to_vec(&snapshot)?;
        if bytes.len() as u64 > MAX_MARKER_BYTES {
            return Err(std::io::Error::other("crash marker exceeds 16 KiB"));
        }
        self.file.seek(SeekFrom::Start(0))?;
        self.file.set_len(0)?;
        self.file.write_all(&bytes)?;
        self.file.flush()?;
        self.details = snapshot;
        self.diagnostic_pending = false;
        self.diagnostic_failures = 0;
        Ok(())
    }

    fn record_diagnostic(&mut self, event: DiagnosticEvent) {
        let previous =
            self.details.get("diagnostics").and_then(safe_diagnostics).unwrap_or_else(|| json!({"schema":1,"sequence":0,"persistedSequence":0,"events":[]}));
        let sequence = previous.get("sequence").and_then(Value::as_u64).unwrap_or(0).saturating_add(1);
        let persisted = previous.get("persistedSequence").and_then(Value::as_u64).unwrap_or(0);
        let elapsed = u64::try_from(self.diagnostic_started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let backend = self.details.get("gpu").and_then(|v| v.get("backend")).and_then(Value::as_str);
        let value = diagnostic_record(event, sequence, elapsed, backend);
        let mut events = previous.get("events").and_then(Value::as_array).cloned().unwrap_or_default();
        if events.len() >= DIAGNOSTIC_EVENTS {
            events.remove(0);
        }
        events.push(value);
        self.details["diagnostics"] = json!({"schema":1,"sequence":sequence,"persistedSequence":persisted,"events":events});
        self.diagnostic_pending = true;
        self.diagnostic_failures = 0;
        if diagnostic_is_critical(event.stage) {
            // Only explicit confirm/commit/release transitions write synchronously.
            // flush reaches the OS cache; no per-event sync_data or GPU wait.
            self.diagnostic_last_attempt = std::time::Instant::now();
            if self.persist().is_err() {
                self.diagnostic_failures = 1;
            }
        }
    }

    fn persist_diagnostics_idle(&mut self, now: std::time::Instant) -> bool {
        if !self.diagnostic_pending {
            return false;
        }
        if now.saturating_duration_since(self.diagnostic_last_attempt) < DIAGNOSTIC_IDLE_INTERVAL {
            return true;
        }
        self.diagnostic_last_attempt = now;
        if self.persist().is_err() {
            self.diagnostic_failures = self.diagnostic_failures.saturating_add(1);
            if self.diagnostic_failures >= DIAGNOSTIC_RETRY_LIMIT {
                // Keep the in-memory evidence for panic/clean error reports, but stop
                // scheduling idle disk retries until another key transition occurs.
                self.diagnostic_pending = false;
            }
        }
        self.diagnostic_pending
    }

    fn finish(mut self, exit_code: u32) {
        if exit_code != 0 {
            let mut report = safe_context(&self.details);
            report["event"] = json!("application-error-exit");
            report["exitCode"] = json!(exit_code);
            report["exitCodeHex"] = json!(format!("0x{exit_code:08X}"));
            let _ = write_report(&self.dir, report);
        }
        // An explicit clean flag distinguishes failed deletion from a process
        // dying between truncating and rewriting its marker (including zero bytes).
        self.details["cleanExit"] = json!(true);
        let _ = self.persist();
        let path = marker_path(&self.dir, &self.id);
        drop(self.file);
        if let Some(path) = path {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn recover_orphans(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten().take(4096) {
        let path = entry.path();
        let Some(id) =
            path.file_name().and_then(|name| name.to_str()).and_then(|name| name.strip_prefix("running-")).and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        if !valid_id(id) || linked(&path) {
            continue;
        }
        let Ok(mut file) = OpenOptions::new().read(true).write(true).open(&path) else { continue };
        if file.try_lock().is_err() {
            continue; // A live instance still owns this marker.
        }
        // Windows byte-range locks require reading through the same handle
        // that acquired the lock, rather than reopening a second file handle.
        let details = match read_marker_file(&mut file) {
            Some(details) => details,
            // A process can die between truncating and finishing its marker.
            // Preserve this as unknown-exit evidence without publishing bytes.
            None => context(id, id.split('-').nth(1).and_then(|part| u32::from_str_radix(part, 16).ok()).unwrap_or(0)),
        };
        if details.get("cleanExit").and_then(Value::as_bool) == Some(true) {
            drop(file);
            let _ = std::fs::remove_file(path);
            continue;
        }
        let mut report = safe_context(&details);
        report["event"] = json!("previous-session-ended-without-clean-close");
        report["exitCode"] = Value::Null;
        report["backtrace"] = Value::Null;
        report["note"] = json!("No live marker lock remained. Exit code and native stack were unavailable; a forced stop, power loss or crash is possible.");
        if write_report(dir, report).is_ok() {
            drop(file);
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Call after argument validation, before initializing graphics or loading data.
pub fn begin_session() {
    let Some(dir) = directory() else { return };
    match Session::begin(dir) {
        Ok(session) => {
            let id = session.id.clone();
            *STATE.lock().unwrap_or_else(PoisonError::into_inner) = Some(session);
            #[cfg(windows)]
            if let Err(error) = spawn_monitor(&id) {
                log::warn!("InkPop crash monitor could not start: {error}");
            }
            #[cfg(not(windows))]
            let _ = id;
        }
        Err(error) => log::warn!("InkPop crash marker could not be written: {error}"),
    }
}

pub fn finish_session(exit_code: u32) {
    if let Some(session) = STATE.lock().unwrap_or_else(PoisonError::into_inner).take() {
        session.finish(exit_code);
    }
}

pub fn gpu(details: Value) {
    if let Some(session) = STATE.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
        session.details["gpu"] = gpu_fields(&details);
        let _ = session.persist();
    }
}

/// Typed key transition only. Critical confirm stages are flushed before returning.
/// Callers must not invoke this from pointer-motion or paint-dab loops.
pub fn diagnostic(event: DiagnosticEvent) {
    if let Some(session) = STATE.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
        session.record_diagnostic(event);
        DIAGNOSTIC_DIRTY.store(session.diagnostic_pending, Ordering::Relaxed);
    }
}

/// Called by the shell only at safe idle. No disk I/O occurs without dirty data,
/// and failed/noncritical writes are bounded to one attempt per 250 ms, at most three.
pub fn diagnostic_idle() -> bool {
    if !DIAGNOSTIC_DIRTY.load(Ordering::Relaxed) {
        return false;
    }
    let mut state = STATE.lock().unwrap_or_else(PoisonError::into_inner);
    let pending = state.as_mut().is_some_and(|session| session.persist_diagnostics_idle(std::time::Instant::now()));
    DIAGNOSTIC_DIRTY.store(pending, Ordering::Relaxed);
    pending
}

/// Persist an import/export warning beside crash reports. The record deliberately contains only
/// the operation label and bounded warning text, never document pixels or full paths.
pub fn warning(operation: &str, warnings: &[String]) {
    let Some(dir) = directory() else { return };
    let mut report = context("file-warning", std::process::id());
    report["event"] = json!("file-warning");
    report["operation"] = json!(bounded(operation, 256));
    report["warnings"] = json!(warnings.iter().take(16).map(|warning| bounded(warning, 512)).collect::<Vec<_>>());
    let _ = write_report(&dir, report);
}

fn panic_category(text: Option<&str>) -> &'static str {
    match text {
        Some(text) if text.contains("index out of bounds") => "index-out-of-bounds",
        Some(text) if text.contains("unwrap") || text.contains("expect") => "unexpected-missing-value-or-error",
        Some(text) if text.contains("assertion") => "assertion-failed",
        Some(text) if text.contains("out of memory") || text.contains("allocation") => "allocation-failed",
        Some(_) => "rust-string-panic-details-redacted",
        None => "rust-nonstring-panic-payload-redacted",
    }
}

pub fn record_panic(info: &std::panic::PanicHookInfo<'_>) {
    if REPORTING_PANIC.swap(true, Ordering::Relaxed) {
        return;
    }
    let details = STATE
        .try_lock()
        .ok()
        .and_then(|state| state.as_ref().map(|session| safe_context(&session.details)))
        .unwrap_or_else(|| context("early-start-or-lock-unavailable", std::process::id()));
    let mut report = details;
    let text = info.payload().downcast_ref::<&str>().copied().or_else(|| info.payload().downcast_ref::<String>().map(String::as_str));
    report["event"] = json!("rust-panic");
    report["panicCategory"] = json!(panic_category(text));
    report["panicPayloadRedacted"] = json!(true);
    report["exitCode"] = Value::Null;
    report["note"] = json!("The hook cannot determine whether this panic will be caught. Native faults require the monitor exit report.");
    if let Some(location) = info.location() {
        let file = Path::new(location.file()).file_name().and_then(|name| name.to_str()).unwrap_or("unknown-source");
        report["location"] = json!({"file":bounded(file,128),"line":location.line(),"column":location.column()});
    }
    report["backtrace"] = json!(bounded(&std::backtrace::Backtrace::force_capture().to_string(), 24000));
    if let Some(dir) = directory() {
        let _ = write_report(&dir, report);
    }
    REPORTING_PANIC.store(false, Ordering::Relaxed);
}

#[cfg(windows)]
fn spawn_monitor(id: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new(std::env::current_exe()?)
        .args([MONITOR_FLAG, &std::process::id().to_string(), id])
        .creation_flags(0x08000000) // CREATE_NO_WINDOW; never opens a console.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}

/// Recognizes only the internal monitor invocation. It never starts the UI or
/// parses ordinary application/file/control arguments in the monitor process.
pub fn monitor_entrypoint() -> Option<i32> {
    let args: Vec<_> = std::env::args_os().skip(1).take(4).collect();
    if args.first().is_none_or(|argument| argument != MONITOR_FLAG) {
        return None;
    }
    let parsed = (args.len() == 3)
        .then(|| {
            let pid = args.get(1)?.to_str()?.parse::<u32>().ok()?;
            let id = args.get(2)?.to_str()?;
            (pid != 0 && valid_id(id)).then_some((pid, id))
        })
        .flatten();
    let Some((pid, id)) = parsed else { return Some(2) };
    #[cfg(windows)]
    {
        Some(if monitor_process(pid, id).is_ok() { 0 } else { 1 })
    }
    #[cfg(not(windows))]
    {
        let _ = (pid, id);
        Some(2)
    }
}

#[cfg(windows)]
fn monitor_process(pid: u32, id: &str) -> Result<(), String> {
    let dir = directory().ok_or("report directory unavailable")?;
    monitor_process_in(&dir, pid, id)
}

#[cfg(windows)]
fn monitor_process_in(dir: &Path, pid: u32, id: &str) -> Result<(), String> {
    use winsafe::{HPROCESS, co};
    if pid == std::process::id() {
        return Err("crash monitor cannot wait on itself".into());
    }
    if !safe_directory(dir) {
        return Err("unsafe monitor directory".into());
    }
    let path = marker_path(dir, id).ok_or("invalid monitor session")?;
    // Do not read the byte-range-locked marker while the GUI is alive. Its
    // generated name encodes the pid; read its final diagnostic data after wait.
    if id.split('-').nth(1).and_then(|part| u32::from_str_radix(part, 16).ok()) != Some(pid) || !path.is_file() || linked(&path) {
        return Err("monitor session mismatch".into());
    }
    let process = HPROCESS::OpenProcess(co::PROCESS::QUERY_LIMITED_INFORMATION | co::PROCESS::SYNCHRONIZE, false, pid).map_err(|error| error.to_string())?;
    let image = process.QueryFullProcessImageName(co::PROCESS_NAME::WIN32).map_err(|error| error.to_string())?;
    let target = Path::new(&image).canonicalize().map_err(|error| error.to_string())?;
    let executable = std::env::current_exe().and_then(|exe| exe.canonicalize()).map_err(|error| error.to_string())?;
    if target != executable {
        return Err("monitor target is not this application".into());
    }
    // An obsolete marker must not attach to a different instance after PID reuse.
    let marker = OpenOptions::new().read(true).write(true).open(&path).map_err(|error| error.to_string())?;
    if !matches!(marker.try_lock(), Err(std::fs::TryLockError::WouldBlock)) {
        return Err("monitor session is no longer held by the application".into());
    }
    drop(marker);
    process.WaitForSingleObject(None).map_err(|error| error.to_string())?;
    let code = process.GetExitCodeProcess().map_err(|error| error.to_string())?;
    // Normal close removed or explicitly completed the marker. A truncated
    // marker is still an unexpected exit and must not be mistaken for clean.
    // A new launch may already have recovered and removed the orphan marker.
    // Still preserve a native status observed through our original process handle.
    if !path.exists() && code < 0x40000000 {
        return Ok(());
    }
    let details = read_marker(&path).unwrap_or_else(|| context(id, pid));
    if details.get("cleanExit").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let mut report = safe_context(&details);
    report["event"] = json!(if code & 0x80000000 != 0 { "windows-exception-exit" } else { "unexpected-process-exit" });
    report["exitCode"] = json!(code);
    report["exitCodeHex"] = json!(format!("0x{code:08X}"));
    report["backtrace"] = Value::Null;
    report["note"] = json!("Independent monitor observed process termination; no native stack or memory dump is collected.");
    write_report(dir, report).map_err(|error| error.to_string())?;
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[cfg(test)]
#[path = "crash_reports_tests_v79.rs"]
mod tests;

#[cfg(test)]
#[path = "crash_reports_tests_v90.rs"]
mod diagnostic_tests_v90;
