//! Photocraft's first UI shell, built on egui/eframe.
//!
//! This crate is deliberately thin. Every action goes through
//! [`photocraft_engine::Session::execute`], and all UI state lives in [`state::UiState`] (plain
//! data). The [`control`] module exposes both to automation, so agents can drive and inspect every
//! part of the interface.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// Translate a string literal into the current UI language: `tl!("Image Size…")`.
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}

mod about_licenses_ui;
pub mod actions;
pub mod adjust_dialog;
pub mod adjust_editors;
pub mod adjust_preview;
pub mod adjust_ui;
pub mod analysis_ui;
pub mod artboard_ui;
mod brand;
mod brush_cursor_ui;
pub mod brush_panel;
pub mod brush_picker;
pub mod brush_preview;
pub mod brush_resize;
pub mod brush_sections;
pub mod brushes_tab;
mod camera_raw_compare_ui;
mod camera_raw_detail_ui;
mod camera_raw_preview_ui;
mod camera_raw_scope_ui;
pub mod camera_raw_ui;
pub mod canvas;
pub mod canvas_tool_menu;
pub mod channel_view;
pub mod channels_panel;
pub mod chrome_ui;
pub mod cjk_fonts;
mod color_palette_ui;
mod color_panel_ui;
pub mod color_picker_ui;
pub mod color_range_ui;
mod color_wheel_ui;
pub mod comps_ui;
pub mod control;
#[cfg(test)]
mod crash_reports_ui_tests_v79;
pub mod credits;
pub mod crop_ui;
mod diagnostics_ui;
pub mod dialogs;
pub mod direct_select;
pub mod discard_ui;
pub mod distort_ui;
pub mod doc_props_ui;
pub mod dock;
mod document_title_v75;
mod document_window_ui;
pub mod enable_rules;
pub mod eraser_ui;
pub mod export_dialog;
pub mod file_open;
pub mod file_ui;
pub mod fill_ui;
pub mod filter_dialog;
pub mod gallery_ui;
pub mod gpu_canvas;
pub mod gpu_status;
pub mod gradient_ui;
pub mod hold_keys;
mod home_ui;
pub mod i18n;
mod icon_data;
pub mod icons;
pub mod jobs_ui;
mod jpeg_options_ui;
pub mod lasso_ui;
mod layer_filter_ui;
pub mod layer_menu_ui;
pub mod layer_pick_ui;
pub mod layer_props_ui;
mod layer_reveal;
pub mod layer_row_ui;
pub mod layer_style;
mod layer_transfer;
pub mod layer_tree_ui;
pub mod links;
pub mod liquify_ui;
pub mod magnetic_lasso_ui;
pub mod mask_thumbs_ui;
pub mod menu_catalog;
pub mod menu_nav;
pub mod menus;
mod mixer_preview;
pub mod monitor_status;
pub mod move_mods;
pub mod move_ui;
mod native_cursor;
pub mod native_menu;
mod native_menu_ui;
mod native_popup_ui;
pub mod new_doc_ui;
pub mod notices;
mod opacity_keys;
pub mod outline;
pub mod paint_mouse;
pub mod palette;
pub mod panel_workspace;
pub mod panels;
pub mod parity;
pub mod patch_preview;
pub mod perspective_ui;
pub mod plugin_ui;
pub mod point_curve;
pub mod prefs_ui;
pub mod preset_files_ui;
pub mod preset_panels;
pub mod props_layout;
pub mod proxy;
mod ps_collaboration_ui;
pub mod puppet_ui;
pub mod quick_pick;
pub mod rasterize_prompt;
mod recognition_quality_ui;
pub mod retouch_ui;
mod rgb_histogram;
pub mod rotate_view;
pub mod rulers;
mod sampling_ui;
pub mod scrollbars;
pub mod shortcut_dispatch;
pub mod shortcuts;
mod sizing;
pub mod slice_ui;
pub mod smart_ui;
pub mod snap_ui;
pub mod state;
pub mod stroke_constraint;
pub mod stroke_trail;
pub mod stylus;
mod tab_strip;
pub mod theme;
mod thumb_fingerprint_cache;
pub mod tiff_options_ui;
mod timeline_ui;
mod titlebar;
pub mod tone;
pub mod tool_brush_persistence;
mod tool_cursor_ui;
pub mod tool_feedback;
mod tool_group_memory;
#[cfg(test)]
mod tool_group_memory_tests_v88;
mod transform_damage;
pub mod transform_tex;
pub mod transform_tool;
pub mod type_panels_ui;
pub mod type_tool;
mod variables_ui;
pub mod vector_ui;
pub mod view_cmds;
mod welcome_links;
pub mod wheel_nav;
pub mod wide_angle_ui;
pub mod widgets;
pub mod work_area;
pub mod workspace_ui;
pub mod zoom_tool;

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

use photocraft_doc::{DocId, Document};
use photocraft_engine::Session;
use serde_json::Value;

pub use control::{ControlRequest, ControlResponse};
pub use file_open::OsEvent;
pub use state::{Tool, UiState};

/// Decode a file: (document, warnings about anything approximated or dropped).
pub type ImportFn = Box<dyn Fn(&str, &[u8]) -> Result<(Document, Vec<String>), String>>;
/// Encoder settings chosen in Export As (the file format comes from the name's extension).
#[derive(Clone, Debug, PartialEq)]
pub struct ExportSettings {
    /// JPEG quality 1–100 (None = codec default).
    pub jpeg_quality: Option<u8>,
    /// WebP: lossless (VP8L) rather than lossy (VP8 at [`Self::webp_quality`]).
    pub webp_lossless: bool,
    /// Lossy WebP quality 1–100 (None = codec default).
    pub webp_quality: Option<u8>,
    /// TIFF: keep the layers (Photoshop layer data); `false` is "Discard Layers and Save a Copy".
    pub tiff_layers: bool,
    /// Embed the document's whole XMP packet. `true` by default (Save As keeps the metadata);
    /// Export As starts at Metadata: None, since the packet can carry the text of every type
    /// layer and one id per placed document (#647).
    pub xmp_all: bool,
}

impl Default for ExportSettings {
    fn default() -> Self {
        ExportSettings { jpeg_quality: None, webp_lossless: true, webp_quality: None, tiff_layers: true, xmp_all: true }
    }
}

/// Encode a document: (file bytes, warnings about anything approximated or dropped).
pub type ExportFn = Box<dyn Fn(&Document, &str, &ExportSettings) -> Result<(Vec<u8>, Vec<String>), String>>;
/// The picked file's name and its bytes, or why it could not be read (shown like any other open
/// failure); `None` when the dialog was cancelled.
pub type PickOpenFn = Box<dyn FnMut() -> Option<(String, Result<Vec<u8>, String>)>>;
/// File › Open's multi-file picker: the selected paths, `None` when cancelled.
pub type PickOpenPathsFn = Box<dyn FnMut() -> Option<Vec<String>>>;
pub type PickSaveFn = Box<dyn FnMut(&str) -> Option<String>>;
pub type WriteFn = Box<dyn FnMut(&str, &[u8]) -> Result<(), String>>;
/// Read bytes through the desktop control session's authorized read root.
pub type AutomationReadFn = Box<dyn FnMut(&str) -> Result<(String, Vec<u8>), String>>;
/// Write bytes through the desktop control session's authorized write root.
pub type AutomationWriteFn = Box<dyn FnMut(&str, &[u8]) -> Result<(), String>>;
/// Reject engine commands which still perform ambient filesystem I/O.
pub type AutomationCommandFn = Box<dyn Fn(&str, &Value) -> Result<(), String>>;
/// Open a URL in the system browser (native) — reliable cross-platform, unlike `ctx.open_url`.
pub type OpenUrlFn = Box<dyn Fn(&str) -> Result<(), String>>;
pub type EncodePngFn = Box<dyn Fn(u32, u32, &[u8]) -> Result<Vec<u8>, String>>;
/// Put an RGBA8 image (width, height, pixels) on the OS clipboard.
pub type ClipboardSetFn = Box<dyn FnMut(u32, u32, &[u8]) -> Result<(), String>>;
/// Read an RGBA8 image from the OS clipboard.
pub type ClipboardGetFn = Box<dyn FnMut() -> Option<(u32, u32, Vec<u8>)>>;
/// The native clipboard revision either has pixels, is empty/non-image, or is already imported.
#[derive(Debug)]
pub enum ClipboardRead {
    Image { width: u32, height: u32, pixels: Vec<u8> },
    Empty,
    Unchanged,
}
/// Read clipboard pixels with explicit size/decode errors. A failure must not paste stale pixels.
pub type ClipboardGetCheckedFn = Box<dyn FnMut() -> Result<ClipboardRead, String>>;
/// Shared queue of (file name, bytes) delivered asynchronously.
pub type Inbox = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;
/// Read the saved preferences text (`None` when there is none yet).
pub type LoadTextFn = Box<dyn FnMut() -> Option<String>>;
pub type LoadToolStateFn = Box<dyn FnMut() -> Result<Option<String>, String>>;
/// Persist the preferences text.
pub type SaveTextFn = Box<dyn FnMut(&str) -> Result<(), String>>;
/// Autosave a document snapshot for crash recovery: (snapshot, revision, original path).
pub type AutosaveFn = Box<dyn FnMut(&std::sync::Arc<Document>, u64, Option<&str>) -> Result<(), String>>;
/// Drop the recovery data of a document (by `DocId` value) once it is saved or closed.
pub type DiscardAutosaveFn = Box<dyn FnMut(u64)>;
/// Load recoverable documents left by a previous session. Their recovery data stays until the
/// documents are saved or closed.
pub type RecoverFn = Box<dyn FnMut() -> Vec<Recovered>>;
/// A recovered document (by `DocId` value, once open) takes over its recovery entry (by key):
/// its autosaves replace the entry, and saving or closing it drops the entry.
pub type AdoptAutosaveFn = Box<dyn FnMut(u64, &str)>;

/// A document [`RecoverFn`] found.
pub struct Recovered {
    /// The recovery entry it was loaded from (see [`AdoptAutosaveFn`]).
    pub key: String,
    /// Where the user last saved it, if anywhere.
    pub path: Option<String>,
    pub doc: Document,
}
/// Create a new collaboration copy and optional companion through platform services.
pub type WriteNewFilesFn = Box<dyn FnMut(&[(String, Vec<u8>)]) -> Result<(), String>>;
/// Append text to a file.
pub type AppendTextFn = Box<dyn FnMut(&str, &str) -> Result<(), String>>;
/// Requests from the operating system since the last call (see [`OsEvent`]).
pub type OsEventsFn = Box<dyn FnMut() -> Vec<OsEvent>>;
/// Where the OS pointer is now, in egui points within the window; `None` when unknown.
pub type CursorPosFn = Box<dyn FnMut(&egui::Context) -> Option<egui::Pos2>>;
/// Current desktop pointer in physical screen pixels, sampled independently of viewport events.
pub type ScreenCursorPosFn = Box<dyn FnMut() -> Option<egui::Pos2>>;
/// Map a physical screen point to the application's foremost native viewport there.
pub type ScreenViewportFn = Box<dyn FnMut(&egui::Context, egui::Pos2) -> Option<egui::ViewportId>>;
/// The monitor's usable desktop area in physical pixels, at a screen point.
pub type MonitorWorkAreaFn = Box<dyn Fn(egui::Pos2) -> Option<egui::Rect>>;
/// Physical Alt and Ctrl state for the temporary eyedropper cursor on Windows.
pub type CursorModifiersFn = Box<dyn Fn() -> egui::Modifiers>;
/// Physical desktop coordinates captured with the native contact event. `true` reads the
/// contact's down sample; `false` reads its latest sample, independently of the mouse cursor.
pub type ContactScreenPosFn = Box<dyn FnMut(u64, bool) -> Option<egui::Pos2>>;
/// Request a bounded display thumbnail without blocking the UI. Unavailable or
/// unsupported previews return None; document opening remains a separate action.
pub type RecentThumbnailFn = Box<dyn Fn(&str) -> std::sync::mpsc::Receiver<Option<egui::ColorImage>>>;

/// Fixed local crash breadcrumbs. No paths, pixels, names or command parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticStage {
    PlaceSourceReady,
    PreviewReady,
    TransformConfirmStart,
    TransformCommandFinished,
    PreviewReleased,
    UiFrameBuilt,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticOperation {
    Place,
    Paste,
    Transform,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticTool {
    Brush,
    Eraser,
    MixerBrush,
    Smudge,
    Move,
    Other,
}

impl From<Tool> for DiagnosticTool {
    fn from(tool: Tool) -> Self {
        match tool {
            Tool::Brush | Tool::Pencil => Self::Brush,
            Tool::Eraser | Tool::BackgroundEraser | Tool::MagicEraser => Self::Eraser,
            Tool::MixerBrush => Self::MixerBrush,
            Tool::Smudge => Self::Smudge,
            Tool::Move => Self::Move,
            _ => Self::Other,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiagnosticEvent {
    pub stage: DiagnosticStage,
    pub operation: DiagnosticOperation,
    pub tool: DiagnosticTool,
    pub source_size: Option<[u32; 2]>,
    pub target_size: Option<[u32; 2]>,
    pub is_smart: Option<bool>,
    pub success: Option<bool>,
}

impl DiagnosticEvent {
    pub fn new(stage: DiagnosticStage, tool: Tool) -> Self {
        Self { stage, operation: DiagnosticOperation::Transform, tool: tool.into(), source_size: None, target_size: None, is_smart: None, success: None }
    }
}

/// Key transitions only. Desktop persists critical confirm stages before returning.
pub type DiagnosticFn = Box<dyn Fn(DiagnosticEvent)>;
/// Called at safe idle. Returns true only while a bounded persistence retry is pending.
pub type DiagnosticIdleFn = Box<dyn Fn() -> bool>;
/// Persist a bounded, privacy-limited file warning for later review.
pub type WarningFn = Box<dyn Fn(&str, &[String])>;

/// Platform services injected by the app binary (file dialogs, codecs), keeping this crate free of
/// I/O dependencies.
#[derive(Default)]
pub struct Services {
    /// Background, bounded Home-screen previews. Unset on platforms without paths.
    pub request_recent_thumbnail: Option<RecentThumbnailFn>,
    /// Decode a file's bytes into a document (PSD, PNG, JPEG, …).
    pub import: Option<ImportFn>,
    /// Encode a document for a file name (format chosen by extension).
    pub export: Option<ExportFn>,
    /// Show a single-file picker for commands that import one file (Open As, presets, scripts).
    pub pick_open: Option<PickOpenFn>,
    /// Creative LUT picker with .cube/.3dl/.look filters and a bounded native read.
    pub pick_lut: Option<PickOpenFn>,
    /// Bounded workspace JSON picker, separate from document and preset imports.
    pub pick_open_workspace: Option<PickOpenFn>,
    /// Bounded Studio Light color palette picker, separate from document imports.
    pub pick_open_colors: Option<PickOpenFn>,
    /// Show File › Open's multi-file picker; returns the selected paths.
    pub pick_open_paths: Option<PickOpenPathsFn>,
    /// Show a "save file" dialog; returns a path/name to write.
    pub pick_save: Option<PickSaveFn>,
    /// Workspace Save As dialog with its own title/filter and the root window as owner.
    /// Falls back to `pick_save` on platforms without a dedicated picker.
    pub pick_save_workspace: Option<PickSaveFn>,
    /// Studio Light color palette Save As dialog; lets the user choose name and location.
    pub pick_save_colors: Option<PickSaveFn>,
    /// Write bytes to a path (native) or trigger a download (web).
    pub write: Option<WriteFn>,
    /// Create a new collaboration pair; never replaces existing files.
    pub write_new_files: Option<WriteNewFilesFn>,
    /// The same transaction beneath the automation write capability.
    pub automation_write_new_files: Option<WriteNewFilesFn>,
    /// File access used only by control/MCP requests. Interactive dialogs keep
    /// using `pick_open`, `pick_save` and `write` with the user's authority.
    pub automation_read: Option<AutomationReadFn>,
    pub automation_write: Option<AutomationWriteFn>,
    pub automation_command: Option<AutomationCommandFn>,
    /// Same policy as [`Self::automation_command`], as a function pointer the engine calls for
    /// each step of `actions.play`. Installed on the session only while a control request or
    /// an automation-driven [`PhotocraftApp::run`] runs, so a local play of a recorded `file.*` step still works.
    pub automation_authorize: Option<fn(&str, &serde_json::Value) -> photocraft_engine::Result<()>>,
    /// Encode an RGBA8 image as PNG (used for screenshots and `ui.render`).
    pub encode_png: Option<EncodePngFn>,
    /// Open a URL in the system browser (native). Falls back to `ctx.open_url` (web) when unset.
    pub open_url: Option<OpenUrlFn>,
    /// Open the local folder selected by the desktop crash-report service.
    pub open_crash_reports: Option<Box<dyn Fn() -> Result<(), String>>>,
    /// Optional local diagnostics; no document data or arbitrary payloads are accepted.
    pub diagnostic_event: Option<DiagnosticFn>,
    /// Low-frequency persistence of noncritical breadcrumbs, called only at safe idle.
    pub diagnostic_idle: Option<DiagnosticIdleFn>,
    /// Persist import/export warnings in the local crash-report folder for later review.
    pub warning_event: Option<WarningFn>,
    /// Files delivered asynchronously (web file pickers, drag-and-drop): drained every frame.
    pub inbox: Option<Inbox>,
    /// OS clipboard images: copies go out, screenshots and images from other apps come in.
    pub clipboard_set_image: Option<ClipboardSetFn>,
    pub clipboard_get_image: Option<ClipboardGetFn>,
    pub clipboard_get_image_checked: Option<ClipboardGetCheckedFn>,
    /// Preferences persistence: the platform config directory on native, browser storage on
    /// the web (see `prefs_ui`).
    pub load_prefs: Option<LoadTextFn>,
    pub save_prefs: Option<SaveTextFn>,
    /// Independent, bounded last-used brush snapshots; bitmap payloads never enter preferences.json.
    pub load_tool_state: Option<LoadToolStateFn>,
    pub save_tool_state: Option<SaveTextFn>,
    /// Optional small, independently persisted panel layout.
    pub save_workspace: Option<SaveTextFn>,
    pub load_workspace: Option<LoadTextFn>,
    /// The native window is connected directly to a Wayland compositor.
    pub is_wayland: bool,
    /// Crash-recovery autosave (Preferences › File Handling) and recovery at launch.
    pub autosave: Option<AutosaveFn>,
    pub discard_autosave: Option<DiscardAutosaveFn>,
    pub recover: Option<RecoverFn>,
    pub adopt_autosave: Option<AdoptAutosaveFn>,
    /// History Log text file output.
    pub append_text: Option<AppendTextFn>,
    /// OS requests (macOS open-documents / quit Apple events), polled every frame.
    pub os_events: Option<OsEventsFn>,
    /// The pointer position read from the OS (desktop): winit 0.30's file drops carry none, and
    /// the window gets no pointer events during an OS drag (see `file_open::DropTarget`).
    pub cursor_pos: Option<CursorPosFn>,
    /// Native window dragging must not combine old client events with a newer window origin.
    pub screen_cursor_pos: Option<ScreenCursorPosFn>,
    pub screen_viewport: Option<ScreenViewportFn>,
    pub monitor_work_area: Option<MonitorWorkAreaFn>,
    pub eyedropper_cursor_modifiers: Option<CursorModifiersFn>,
    pub contact_screen_pos: Option<ContactScreenPosFn>,
    /// The persistent brush preset store, loading in the background (desktop; see
    /// `photocraft_engine::preset_store`). Attached to the session once it arrives; without
    /// one, brush presets are session-only (web, tests).
    pub preset_store: Option<std::sync::mpsc::Receiver<photocraft_engine::preset_store::Opened>>,
    /// Reads the displays and their ICC profiles in the background (desktop macOS; see
    /// `monitor_status`). Without one, the canvas uses the profile chosen in Color Settings, or sRGB.
    pub read_displays: Option<monitor_status::ReadDisplaysFn>,
    /// The macOS menu bar, when the desktop app installed one; the in-window menus are hidden then.
    pub native_menu: Option<native_menu::NativeMenu>,
}

pub struct PhotocraftApp {
    pub session: Session,
    pub ui: UiState,
    pub services: Services,
    pending_raw_editor: Option<(photocraft_doc::DocId, u64, photocraft_doc::LayerId, usize)>,
    /// Canvas caches per (document, display): CPU textures hold monitor values; the GPU
    /// canvas state is shared (`canvas::GPU_OUTPUT`).
    canvases: HashMap<(DocId, u32), canvas::CanvasCache>,
    /// Display profile readings (#569).
    monitors: monitor_status::State,
    checker: Option<egui::TextureHandle>,
    drag: Option<canvas::Drag>,
    /// Brush/Eraser stroke being drawn, rendered by the engine (see `canvas::LiveStroke`).
    live_stroke: Option<canvas::LiveStroke>,
    /// Footprint trail of a retouching drag (see `stroke_trail`).
    trail: Option<stroke_trail::Trail>,
    /// Move tool drag shown live (`move_ui`).
    pub(crate) move_preview: Option<move_ui::MovePreview>,
    /// Patch Tool drag: the healed document at the pointer (`patch_preview`).
    pub(crate) patch_preview: Option<patch_preview::PatchPreview>,
    pub(crate) mixer_preview: Option<mixer_preview::MixerPreview>,
    /// Tool whose brush is currently loaded into `session.tools.brush`.
    brush_tool: Option<Tool>,
    brush_persistence: tool_brush_persistence::Runtime,
    /// A canvas gesture temporarily borrows another tool without selecting it.
    tool_history_suspended: bool,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) mixer_job: Option<mixer_preview::PreviewJob>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) mixer_repaint: Option<egui::Context>,
    /// The pixels a Magnetic Lasso border follows (`magnetic_lasso_ui`).
    pub(crate) magnetic: magnetic_lasso_ui::Runtime,
    /// The next tool `Down` is a right-button drag that erases (see `paint_mouse`).
    secondary_erase: bool,
    /// While a batch of recovered pointer samples is replayed, defer the live-stroke update to one
    /// call for the whole frame (see `canvas::canvas_view`).
    defer_live_stroke: bool,
    /// End of the last painting stroke: Shift-click draws a straight line from it.
    last_stroke_end: Option<(DocId, [f64; 2])>,
    /// Control+Alt-drag brush resize in progress (`brush_resize`, #231).
    pub(crate) brush_resize: Option<brush_resize::Resize>,
    /// A ⌘⌥⌃-click layer pick is in progress; its drag and release are swallowed (`quick_pick`).
    pub(crate) quick_pick: bool,
    /// The next tool `Down` is an Alt+right-drag that resizes the brush (#297). `tool_event`
    /// takes it on every event, so a press another handler consumes can't leave it set.
    pub(crate) brush_resize_armed: bool,
    /// This press began with ⌥ (Alt) held on a painting tool, so it samples colours instead of
    /// painting until it is released (`canvas::alt_eyedropper`, #417).
    pub(crate) alt_sampling: bool,
    /// The first digit of a two-digit opacity typed on the number keys (`opacity_keys`, #352).
    pub(crate) opacity_keys: opacity_keys::Pending,
    control_rx: Option<Receiver<ControlRequest>>,
    pending_screenshots: Vec<(u64, Option<String>, Sender<ControlResponse>)>,
    /// Screenshots not yet requested: (token, viewport, earliest time in ms, frames seen).
    queued_screenshots: Vec<(u64, egui::ViewportId, f64, u32)>,
    /// Control replies waiting for queued synthetic input to be processed.
    input_waiters: Vec<Sender<ControlResponse>>,
    /// Live (uncommitted) adjustment edit shown on canvas while a slider is dragged.
    pub live_adjust: Option<(photocraft_doc::LayerId, Value)>,
    /// Frames rendered (for tests and the status bar).
    pub frame: u64,
    /// Apply theme on first frame.
    styled: bool,
    /// Whether the window uses an integrated (transparent) macOS title bar.
    pub integrated_titlebar: bool,
    /// Windows and Linux: the window has no OS decorations and the app's top bar is the title bar
    /// (caption buttons, window dragging and edge resizing, `titlebar`).
    pub custom_titlebar: bool,
    fonts_ready: bool,
    /// Screen rect of the main canvas last frame (for overlays and the navigator).
    pub last_canvas_rect: egui::Rect,
    /// The document area showing the active document's canvas last frame (not the tabs, the
    /// start screen or an opening file's card): files dropped here are placed as layers.
    pub(crate) drop_canvas_rect: Option<egui::Rect>,
    /// The document tab strip last frame: a drop there opens the file at the slot under it.
    pub(crate) tab_strip: Option<canvas::TabStrip>,
    /// Document tab currently being dragged; releasing outside the strip opens a floating canvas.
    pub(crate) tab_drag: Option<usize>,
    /// Files dropped on the canvas still to place, one Free Transform at a time.
    pub(crate) drop_places: std::collections::VecDeque<file_open::DroppedPlacement>,
    /// The document each of `ui.views` belongs to, as of the last [`Self::sync_views`].
    view_docs: Vec<DocId>,
    pub fps: f32,
    last_frame_time: f64,
    thumb_fingerprints: thumb_fingerprint_cache::ThumbFingerprintCache,
    thumbs: HashMap<(photocraft_doc::LayerId, u8), (u64, egui::TextureHandle)>,
    /// Snapshots whose live layer/mask keys were last used to prune thumbnail handles.
    thumb_documents: Vec<(DocId, std::sync::Weak<Document>)>,
    /// Content bounds cached per (key, revision): scanning a 36 MP layer every frame cost ~77 ms.
    bounds_cache: HashMap<u64, (u64, photocraft_geom::Rect)>,
    /// Downsampled proxy of the active document for live previews: (doc, revision, k, proxy).
    pub(crate) proxy: Option<(DocId, u64, u32, std::sync::Arc<Document>)>,
    /// Key of the preview currently uploaded to the GPU (doc, params hash).
    pub(crate) proxy_uploaded: Option<(DocId, u64)>,
    /// Selection outline cache: (doc, revision, segments).
    /// Live filter preview (proxy document with the filter applied).
    pub(crate) filter_preview: Option<filter_dialog::FilterPreview>,
    /// Select › Color Range dialog preview (proxy document + mask / image textures).
    pub(crate) color_range: Option<color_range_ui::Preview>,
    /// Image › Adjustments dialog preview through a temporary clipped adjustment layer.
    pub(crate) adjust_preview: Option<adjust_preview::AdjustPreview>,
    /// Synthetic input events queued by automation (`ui.click`, `ui.key`, …), injected next frame.
    pub(crate) synthetic: Vec<egui::Event>,
    /// True only while a frame is processing synthetic automation input. It
    /// makes shortcut- and dialog-triggered commands pass the same policy as
    /// direct control calls.
    pub(crate) automation_input: bool,
    /// Levels/Curves histogram cache: (document, adjustment layer, revision it is valid for).
    pub(crate) tone_hist: Option<(DocId, photocraft_doc::LayerId, u64, std::sync::Arc<tone::Histograms>)>,
    /// Histogram panel cache: (document, revision, computed at ms, histograms).
    pub(crate) doc_hist: Option<(DocId, u64, f64, std::sync::Arc<tone::Histograms>)>,
    /// Free Transform preview (document without the moving pixels + their texture).
    pub(crate) transform_preview: Option<transform_tool::TransformPreview>,
    /// Exact preview-to-committed ROI transition, never a cache relabel.
    pub(crate) transform_damage: Option<transform_damage::Transition>,
    /// Move-tool ⇧/⌥ drag state (move_mods).
    pub(crate) move_mods: move_mods::MoveDrag,
    /// Live Layer Style dialog preview: (key over revision + style fields, preview or validation error).
    pub(crate) style_preview: Option<(u64, Result<std::sync::Arc<Document>, String>)>,
    /// Liquify dialog, Puppet Warp and Perspective Warp sessions (distort_ui).
    pub(crate) distort: distort_ui::Distort,
    /// Gradient tool live-mode drags and previews (gradient_ui).
    pub(crate) gradient: gradient_ui::LiveGradient,
    /// Filter › Camera Raw Filter dialog (camera_raw_ui).
    pub(crate) camera_raw: Option<camera_raw_ui::CameraRawDialog>,
    /// Filter › Adaptive Wide Angle dialog (wide_angle_ui).
    pub(crate) wide_angle: Option<wide_angle_ui::WideAngleDialog>,
    /// Signature of the image we last put on the OS clipboard (to tell ours from other apps').
    os_clip_sig: Option<u64>,
    /// The session clipboard currently holds an image imported from the OS clipboard: it has no
    /// original position, so Paste centres it (see `menus` "edit.paste").
    pub(crate) clip_external: bool,
    /// The OS clipboard was already read for the paste in flight (Edit › Paste positions it first),
    /// so `run` doesn't read it a second time.
    pub(crate) clip_read_for_paste: bool,
    /// Pointer position over the canvas (document px), for the Info panel and status bar.
    pub(crate) hover_doc: Option<[f64; 2]>,
    /// Info panel sample cache: ((x, y, revision), composite RGBA).
    info_sample: Option<((i32, i32, u64), [f32; 4])>,
    /// Guide being dragged (from a ruler or with the Move tool).
    pub(crate) guide_drag: Option<rulers::GuideDrag>,
    /// Crop tool gesture in progress (see `crop_ui`).
    pub(crate) crop: crop_ui::CropState,
    /// Type tool layout cache: ((doc, revision, layer), layout).
    pub(crate) type_layout: Option<((u64, u64, u64), std::sync::Arc<photocraft_text::TextLayout>)>,
    /// Channel thumbnails for one document snapshot; view-only revisions reuse their pixels.
    channel_thumbs: Option<(DocId, std::sync::Weak<Document>, Vec<egui::TextureHandle>)>,
    /// Home recent-preview handles and its single asynchronous request.
    pub(crate) home: home_ui::HomeState,
    /// Channels panel overlays / channel views drawn over the canvas, per document id.
    pub(crate) channel_views: HashMap<u64, channel_view::Cache>,
    /// Selection outline keyed by (document, mask identity × step × visible region).
    pub(crate) outline_cache: Option<(DocId, u64, std::sync::Arc<Vec<outline::Segment>>)>,
    /// GPU canvas renderer, when running on the wgpu backend (see [`Self::set_wgpu`]). Dropped
    /// for the rest of the session when the device is lost (see `gpu_status`).
    gpu: Option<gpu_canvas::GpuCanvas>,
    /// Run once the first frames have rendered (see [`Self::on_started`]).
    started: Option<gpu_status::StartedHook>,
    diagnostic_frame_pending: Option<DiagnosticEvent>,
    /// Frame and canvas-upload timings (exposed via `ui.inspect`).
    pub perf: gpu_canvas::Perf,
    /// Preferences, autosave and snapping runtime state (see `prefs_ui`, `snap_ui`).
    pub(crate) prefs_rt: prefs_ui::Runtime,
    /// Close, Revert or Exit parked behind the unsaved-changes prompt (see `discard_ui`).
    pub(crate) discard: Option<discard_ui::Prompt>,
    /// A Save As to a layered TIFF parked behind the TIFF Options prompt (see `tiff_options_ui`).
    pub(crate) tiff_options: Option<tiff_options_ui::Prompt>,
    pub(crate) jpeg_options: Option<jpeg_options_ui::Prompt>,
    /// Compression selected for a main JPEG save during this editing session.
    jpeg_save_quality: HashMap<DocId, u8>,
    /// Set once the user has agreed to quit, so the resulting close request goes through.
    pub(crate) allow_close: bool,
    /// Pen pressure/tilt from the platform (see `stylus`).
    pub stylus: stylus::Stylus,
    /// Run long commands and file opens as background jobs with progress and Cancel (#210; see
    /// `jobs_ui`). The desktop app turns it on; off (the default), everything runs inline as
    /// before, which tests and scripts rely on.
    pub background_jobs: bool,
    /// Background job bookkeeping: opening tabs, control replies waiting on a job.
    pub jobs: jobs_ui::JobsUi,
    #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
    live_tokens: theme::live::LiveTokens,
}

impl PhotocraftApp {
    /// Enable OS-managed tool pointers for a native desktop integration.
    /// Clear the sticky image at every viewport pass so dialogs and controls
    /// cannot inherit a canvas pointer from a previous pass or another window.
    pub fn enable_native_tool_cursors(ctx: &egui::Context) {
        if cfg!(target_arch = "wasm32") {
            return;
        }
        let register = ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new("photocraft-native-tool-cursors"), true);
            let id = egui::Id::new("photocraft-native-tool-cursor-reset-registered");
            let registered = data.get_temp::<bool>(id).unwrap_or(false);
            data.insert_temp(id, true);
            !registered
        });
        if register {
            ctx.on_begin_pass("photocraft-tool-cursor-reset", std::sync::Arc::new(|ui| ui.ctx().set_cursor_image(None)));
        }
    }

    pub fn new(session: Session, services: Services) -> Self {
        let mut app = Self {
            session,
            ui: UiState::default(),
            services,
            pending_raw_editor: None,
            canvases: HashMap::new(),
            monitors: Default::default(),
            checker: None,
            drag: None,
            live_stroke: None,
            trail: None,
            move_preview: None,
            patch_preview: None,
            mixer_preview: None,
            brush_tool: None,
            brush_persistence: Default::default(),
            tool_history_suspended: false,
            #[cfg(not(target_arch = "wasm32"))]
            mixer_job: None,
            #[cfg(not(target_arch = "wasm32"))]
            mixer_repaint: None,
            magnetic: Default::default(),
            secondary_erase: false,
            defer_live_stroke: false,
            last_stroke_end: None,
            brush_resize: None,
            quick_pick: false,
            brush_resize_armed: false,
            alt_sampling: false,
            opacity_keys: None,
            control_rx: None,
            pending_screenshots: Vec::new(),
            queued_screenshots: Vec::new(),
            input_waiters: Vec::new(),
            live_adjust: None,
            frame: 0,
            styled: false,
            integrated_titlebar: false,
            custom_titlebar: false,
            fonts_ready: false,
            last_canvas_rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)),
            drop_canvas_rect: None,
            tab_strip: None,
            tab_drag: None,
            drop_places: Default::default(),
            view_docs: Vec::new(),
            fps: 0.0,
            last_frame_time: 0.0,
            thumbs: HashMap::new(),
            thumb_fingerprints: Default::default(),
            thumb_documents: Vec::new(),
            bounds_cache: HashMap::new(),
            proxy: None,
            proxy_uploaded: None,
            outline_cache: None,
            filter_preview: None,
            color_range: None,
            adjust_preview: None,
            synthetic: Vec::new(),
            automation_input: false,
            channel_thumbs: None,
            home: Default::default(),
            channel_views: HashMap::new(),
            type_layout: None,
            guide_drag: None,
            crop: Default::default(),
            hover_doc: None,
            info_sample: None,
            os_clip_sig: None,
            clip_external: false,
            clip_read_for_paste: false,
            transform_preview: None,
            transform_damage: None,
            move_mods: Default::default(),
            style_preview: None,
            distort: Default::default(),
            gradient: Default::default(),
            camera_raw: None,
            wide_angle: None,
            tone_hist: None,
            doc_hist: None,
            gpu: None,
            started: None,
            diagnostic_frame_pending: None,
            perf: Default::default(),
            prefs_rt: Default::default(),
            discard: None,
            tiff_options: None,
            jpeg_options: None,
            jpeg_save_quality: HashMap::new(),
            allow_close: false,
            stylus: Default::default(),
            background_jobs: false,
            jobs: Default::default(),
            #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
            live_tokens: theme::live::LiveTokens::from_env(),
        };
        // Saved preferences (and recovered documents) are in place before the first frame.
        prefs_ui::load(&mut app);
        notices::wayland_file_drop_guidance(&mut app);
        // File › Scripts › Script Events Manager: "Start Application".
        photocraft_engine::automate_cmds::fire_event(&mut app.session, "startApplication");
        app.sync_tool_brush();
        app
    }

    fn remember_tool_brush(&mut self) {
        let Some(previous) = self.brush_tool else { return };
        tool_brush_persistence::remember_options(self, previous);
        if let Some(slot) = self.ui.tool_brushes.iter_mut().find(|(t, _)| *t == previous) {
            slot.1 = self.session.tools.brush.clone();
        } else {
            self.ui.tool_brushes.push((previous, self.session.tools.brush.clone()));
        }
        if matches!(previous, Tool::Brush | Tool::Pencil | Tool::Eraser) {
            let smoothing = self.session.tools.brush.smoothing.clone();
            if let Some(slot) = self.ui.tool_smoothing.iter_mut().find(|(t, _)| *t == previous) {
                slot.1 = smoothing;
            } else {
                self.ui.tool_smoothing.push((previous, smoothing));
            }
        }
    }

    /// The session brush has one owner, even while a nonpainting tool is selected.
    /// Synchronize before a brush is edited or used, not only on the following frame.
    pub(crate) fn sync_tool_brush(&mut self) {
        if !self.tool_history_suspended {
            tool_group_memory::remember(&mut self.ui);
        }
        let tool = self.ui.tool;
        if !paint_mouse::has_brush_picker(tool) {
            tool_brush_persistence::leave_painting_options(self);
            return;
        }
        if self.brush_tool == Some(tool) {
            // Return from a nonpainting tool whose option widgets share storage.
            if self.brush_persistence.options_owner != Some(tool) {
                tool_brush_persistence::restore_options(self, tool);
            }
            return;
        }
        self.remember_tool_brush();
        if let Some((_, brush)) = self.ui.tool_brushes.iter().find(|(t, _)| *t == tool) {
            self.session.tools.brush = brush.clone();
        } else if matches!(tool, Tool::Brush | Tool::Pencil | Tool::Eraser) && self.ui.smoothing_tool.is_some() {
            // Legacy smoothing-only slots remain usable; complete brush slots
            // already contain smoothing and must not be overwritten afterward.
            self.session.tools.brush.smoothing = self
                .ui
                .tool_smoothing
                .iter()
                .find(|(t, _)| *t == tool)
                .map(|(_, smoothing)| smoothing.clone())
                .unwrap_or_else(|| photocraft_engine::paint::brush::Smoothing { amount: 0.1, ..Default::default() });
        }
        tool_brush_persistence::restore_options(self, tool);
        if matches!(tool, Tool::Brush | Tool::Pencil | Tool::Eraser) {
            self.ui.smoothing_tool = Some(tool);
        }
        self.brush_tool = Some(tool);
    }

    /// Draw the document canvas on the GPU (custom WGSL shader) instead of via egui textures.
    /// Call from the app creator with `cc.wgpu_render_state`; without it the CPU path is used.
    pub fn set_wgpu(&mut self, rs: eframe::egui_wgpu::RenderState) {
        // Preferences › Performance › cache tile size (PHOTOCRAFT_GPU_TILE still overrides).
        let tile = self.session.prefs().performance.cache_tile_size;
        // Escaped driver/setup panics must leave the session and CPU canvas alive.
        self.perf.gpu_info.set_adapter(&rs.adapter.get_info());
        let gpu = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gpu_canvas::GpuCanvas::with_tile(&rs, Some(tile)))) {
            Ok(gpu) => gpu,
            Err(payload) => {
                let detail = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_string()))
                    .unwrap_or_else(|| "GPU canvas initialization failed".into());
                self.perf.gpu_info.canvas = "cpu".into();
                self.perf.gpu_info.fallback = Some(detail.clone());
                gpu_status::queue_fallback_notice(self, detail);
                return;
            }
        };
        self.perf.gpu_info.canvas = "gpu".into();
        self.gpu = Some(gpu);
        self.prefs_rt.gpu_style = None;
    }

    /// Whether the document canvas currently draws on the GPU (false on the CPU path, and after
    /// the device was lost).
    pub fn gpu_active(&self) -> bool {
        self.gpu.is_some()
    }

    /// The GPU canvas's device health flag (tests inject faults through it).
    pub fn gpu_health(&self) -> Option<photocraft_gpu::DeviceHealth> {
        self.gpu.as_ref().map(|g| g.health().clone())
    }

    /// Run `hook` once the app has rendered its first frames (and a document opened at launch
    /// has drawn): the desktop app clears its crash-safe GPU startup marker there.
    pub fn on_started(&mut self, hook: impl FnOnce(&mut PhotocraftApp) + 'static) {
        self.started = Some(Box::new(hook));
    }

    /// Per-frame GPU health check (called from `logic`; tests call it directly): falls back to
    /// the CPU canvas after a device loss and runs the started hook.
    pub fn check_gpu(&mut self, ctx: &egui::Context) {
        gpu_status::check(self, ctx);
    }

    /// Attach a control channel (requests arrive from a transport thread: TCP, stdin, tests).
    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Run an engine command, reporting errors in the status bar.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        // Automation input also gates every step a command runs on its behalf (`actions.play`).
        let gate = if self.automation_input && self.session.authorize.is_none() { self.services.automation_authorize } else { None };
        if gate.is_some() {
            self.session.authorize = gate;
        }
        let result = self.run_command(id, params);
        if gate.is_some() {
            self.session.authorize = None;
        }
        result
    }

    fn run_command(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let clip_read = std::mem::take(&mut self.clip_read_for_paste);
        if self.automation_input
            && let Some(authorize) = self.services.automation_command.as_ref()
        {
            authorize(id, &params)?;
        }
        // A Layers-row click commits inline type, including a click on the current row.
        // Validate first: a rejected automation selection must not end an editing session.
        if id == "layer.select" && self.ui.text_edit.is_some() {
            let target = params.get("layer").and_then(Value::as_u64).map(photocraft_doc::LayerId);
            let mode = params.get("mode").and_then(Value::as_str).unwrap_or("replace");
            let valid = matches!(mode, "replace" | "add" | "range" | "toggle")
                && target.is_some_and(|layer| self.session.active().is_some_and(|st| st.doc.layer(layer).is_some()));
            if valid {
                let edited = self.ui.text_edit.as_ref().map(|edit| edit.layer);
                type_tool::commit(self);
                if self.ui.text_edit.is_some() {
                    return Err(self.ui.status.clone());
                }
                // Confirming an untouched empty text layer removes that transient layer.
                // Its own row cannot then be selected again; keep the restored selection.
                if target.is_some_and(|layer| Some(layer.0) == edited && self.session.active().is_some_and(|st| st.doc.layer(layer).is_none())) {
                    let selected = self.session.active().map(|st| st.selected_layers()).unwrap_or_default();
                    return Ok(serde_json::json!({"selected": selected}));
                }
            }
        }
        if let Some(r) = transform_tool::intercept(self, id) {
            return r;
        }
        if let Some(result) = ps_collaboration_ui::intercept_command(self, id, &params) {
            return result;
        }
        self.sync_tool_brush();
        if id == "tool.presets.select" {
            // The engine replaces the session brush before returning the new
            // tool. Save the outgoing owner before that replacement occurs.
            self.remember_tool_brush();
        }
        let t0 = gpu_canvas::now_ms();
        // The OS clipboard is read only on an explicit paste, never in the background (privacy, CPU).
        if matches!(id, "edit.paste" | "edit.pasteSpecial.pasteInPlace" | "file.newFromClipboard") {
            if !clip_read {
                self.import_os_clipboard()?;
            }
            if self.session.clipboard.is_none() && (self.services.clipboard_get_image.is_some() || self.services.clipboard_get_image_checked.is_some()) {
                // Enabled on the strength of the OS clipboard, which held no image: a quiet no-op.
                self.ui.status = "Nothing to paste: the clipboard holds no image".into();
                self.ui.status_error = false;
                return Ok(serde_json::json!({"pasted": false}));
            }
        }
        // Clipboard errors and an empty clipboard return before this temporary preference
        // change, so a rejected paste cannot leave script events disabled.
        let suppress_events = self.automation_input && self.session.prefs().script_events.enabled;
        if suppress_events {
            self.session.edit_prefs(|prefs| prefs.script_events.enabled = false);
        }
        // Long commands become background jobs when enabled (`jobs_ui`); the rest run inline.
        let params = self.with_mask_target(id, params);
        let brush_preset = if id == "tools.setBrush" { params.get("preset").and_then(Value::as_str).map(str::to_owned) } else { None };
        let brush_reset = id == "tools.setBrush" && params.get("reset").and_then(Value::as_bool) == Some(true);
        let r = jobs_ui::run(self, id, params);
        if r.is_ok() && matches!(id, "edit.copy" | "edit.cut" | "edit.copyMerged") {
            self.clip_external = false;
            self.export_os_clipboard();
        }
        self.perf.last_command = id.to_string();
        if r.is_ok() {
            match id {
                "file.placeEmbedded" | "file.placeLinked" => diagnostics_ui::source_ready(self, DiagnosticOperation::Place),
                "edit.paste" | "edit.pasteSpecial.pasteInPlace" => diagnostics_ui::source_ready(self, DiagnosticOperation::Paste),
                _ => {}
            }
        }
        self.perf.command_ms = gpu_canvas::now_ms() - t0;
        match &r {
            Ok(v) => {
                if id == "tool.presets.select"
                    && let Some(name) = v.get("name").and_then(Value::as_str)
                {
                    self.ui.presets_ui.selected.insert("toolPresets".into(), name.into());
                    if let Some(tool) = v.get("tool").and_then(Value::as_str).and_then(Tool::from_name) {
                        self.ui.tool = tool;
                        if v.get("options").and_then(|o| o.get("brush")).is_some_and(Value::is_object) {
                            // This brush is the explicit incoming preset. Adopt
                            // it without restoring an older per-tool snapshot.
                            self.brush_tool = paint_mouse::has_brush_picker(tool).then_some(tool);
                            if matches!(tool, Tool::Brush | Tool::Pencil | Tool::Eraser) {
                                self.ui.smoothing_tool = Some(tool);
                            }
                        } else {
                            self.sync_tool_brush();
                        }
                    }
                    if let Some(options) = v.get("options").and_then(|o| o.get("toolOptions")).and_then(Value::as_object) {
                        let mut current = serde_json::to_value(&self.ui.tool_options).unwrap_or_default();
                        if let Some(current) = current.as_object_mut() {
                            for (key, value) in options {
                                current.insert(key.clone(), value.clone());
                            }
                        }
                        if let Ok(options) = serde_json::from_value(current) {
                            self.ui.tool_options = options;
                        }
                    }
                }
                tool_brush_persistence::selected(self, id, brush_preset.as_deref(), brush_reset, v);
                self.sync_views();
                if self.ui.status_error {
                    self.ui.status.clear();
                    self.ui.status_error = false;
                }
                if id == "layer.smartObjects.editContents" && v.get("independentCopy").and_then(Value::as_bool) == Some(true) {
                    notices::post(
                        self,
                        tl!("Smart object source opened as a copy"),
                        vec![tl!("Save this copy separately to preserve the original smart filters and appearance.").to_string()],
                        false,
                        None,
                    );
                }
            }
            Err(e) => {
                self.ui.status = e.clone();
                self.ui.status_error = true;
            }
        }
        if suppress_events {
            self.session.edit_prefs(|prefs| prefs.script_events.enabled = true);
        }
        r
    }

    /// Keep one view per document, in tab order: a view and its windows stay with their document
    /// when tabs move (`document.move`) or close.
    pub fn sync_views(&mut self) {
        crate::lasso_ui::cancel_stale(self);
        let ids: Vec<DocId> = self.session.documents().iter().map(|d| d.doc.id).collect();
        // Where the document of view `i` is now. Views not tracked yet keep their index.
        let now = |i: usize| match self.view_docs.get(i) {
            Some(id) => ids.iter().position(|d| d == id),
            None => (i < ids.len()).then_some(i),
        };
        let mut views: Vec<Option<state::View>> = ids.iter().map(|_| None).collect();
        for (i, v) in std::mem::take(&mut self.ui.views).into_iter().enumerate() {
            if let Some(slot) = now(i).and_then(|to| views.get_mut(to)) {
                *slot = Some(v);
            }
        }
        self.ui.views = views.into_iter().map(Option::unwrap_or_default).collect();
        self.ui.windows.retain_mut(|w| {
            w.tabs.retain_mut(|tab| match tab {
                state::DetachedTab::Document(document) => now(*document).map(|index| *document = index).is_some(),
                state::DetachedTab::Opening(_) => true,
            });
            let previous = now(w.document);
            let selected = previous.or_else(|| {
                w.tabs.iter().find_map(|tab| match tab {
                    state::DetachedTab::Document(index) => Some(*index),
                    state::DetachedTab::Opening(_) => None,
                })
            });
            let Some(selected) = selected else { return false };
            if previous.is_none()
                && let Some(view) = self.ui.views.get(selected)
            {
                w.view = view.clone();
            }
            w.document = selected;
            true
        });
        if !self.ui.smart_filters.collapsed.is_empty() {
            self.ui.smart_filters.collapsed.retain(|(doc, _)| ids.iter().any(|id| id.0 == *doc));
        }
        if !self.jpeg_save_quality.is_empty() {
            self.jpeg_save_quality.retain(|doc, _| ids.contains(doc));
        }
        self.view_docs = ids;
        self.prune_thumbs();
        self.sync_mask_targets();
        // Channel-view textures outlive a hidden view (cheap re-show), not their document.
        if !self.channel_views.is_empty() {
            let docs = self.session.documents();
            self.channel_views.retain(|id, _| docs.iter().any(|d| d.doc.id.0 == *id));
        }
        // Commands may close and reopen a preserved-ID document before the next repaint.
        canvas::retain_gpu_documents(self);
    }

    /// Record `path` as the most-recently-opened file (File › Open Recent): de-duplicated, newest
    /// first, capped at Preferences › File Handling › Recent File List Contains. The list lives in
    /// the preferences (`fileHandling.recentFiles`), so it survives a restart; `ui.recent_files`
    /// mirrors it for the menus and the Home screen.
    pub fn push_recent(&mut self, path: &str) {
        self.home.invalidate(path);
        let cap = self.recent_cap();
        let mut r = self.session.prefs().file_handling.recent_files.clone();
        r.retain(|p| p != path);
        r.insert(0, path.to_string());
        r.truncate(cap);
        self.ui.recent_files = r.clone();
        self.session.prefs.edit(|p| p.file_handling.recent_files = r);
    }

    /// File › Open Recent › Clear Recent File List.
    pub fn clear_recent(&mut self) {
        self.ui.recent_files.clear();
        self.session.prefs.edit(|p| p.file_handling.recent_files.clear());
    }

    /// How many recent files to remember ("Recent File List Contains", 0–100).
    pub fn recent_cap(&self) -> usize {
        self.session.prefs().file_handling.recent_file_count.min(100) as usize
    }

    /// Mirror the stored recent-file list into the UI state (after loading the preferences, or
    /// when the Preferences dialog or an agent changes the list or its length). Cheap enough to
    /// run every frame: a slice comparison of at most 100 paths, no allocation unless it changed.
    pub fn sync_recent(&mut self) {
        let cap = self.recent_cap();
        let stored = &self.session.prefs().file_handling.recent_files;
        let want = stored.get(..cap.min(stored.len())).unwrap_or_default();
        if self.ui.recent_files.as_slice() != want {
            self.ui.recent_files = want.to_vec();
        }
    }

    /// Open a file's bytes as a new document named `name`; returns the import warnings (also
    /// shown to the user). Files from disk go through [`open_file`](Self::open_file), which also
    /// remembers the path. Brush and gradient files go to the preset libraries instead.
    pub fn open_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<Vec<String>, String> {
        if let Some(r) = preset_files_ui::open(self, name, bytes) {
            return r.map(|()| Vec::new());
        }
        let name = &self.open_name(name);
        // Decoded on a worker: a tab with progress appears now, the document when it's ready
        // (warnings are shown then).
        if self.background_jobs {
            jobs_ui::start_open(self, name, None, jobs_ui::bytes(bytes))?;
            return Ok(Vec::new());
        }
        let import = self.services.import.as_ref().ok_or("no importer configured")?;
        let (doc, warnings) = import(name, bytes)?;
        // Edit › Color Settings policies apply on open; mismatches can ask what to do.
        // No path yet: a bare name isn't a location to save back to (`open_file` sets the path).
        let (_, color) = self.session.open_document(doc, None);
        self.sync_views();
        self.ui.status = format!("Opened {name}");
        self.ui.status_error = false;
        notices::io_warnings(self, &format!("Opened {name}"), &warnings);
        // Script events bound to "Open Document".
        photocraft_engine::automate_cmds::document_opened(&mut self.session);
        self.sync_views();
        let ask = color.get("ask").and_then(Value::as_bool) == Some(true);
        if ask && (color.get("mismatch").and_then(Value::as_bool) == Some(true) || color.get("missing").is_some()) {
            prefs_ui::open_mismatch(self, &color);
        }
        Ok(warnings)
    }

    /// Open bytes supplied by an authenticated automation client without
    /// invoking user-configured script-event paths outside the granted root.
    /// Returns the import warnings (also shown to the user). Brush and gradient
    /// files go to the preset libraries, as for interactive opens.
    pub fn open_automation_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<Vec<String>, String> {
        let events_enabled = self.session.prefs().script_events.enabled;
        if events_enabled {
            self.session.edit_prefs(|prefs| prefs.script_events.enabled = false);
        }
        let result = match preset_files_ui::open(self, name, bytes) {
            Some(r) => r.map(|()| Vec::new()),
            None => self.import_automation_document(name, bytes),
        };
        if events_enabled {
            self.session.edit_prefs(|prefs| prefs.script_events.enabled = true);
        }
        result
    }

    /// The document half of [`open_automation_bytes`](Self::open_automation_bytes): no script
    /// events and no Color Settings policy (which may read user-configured profile paths).
    fn import_automation_document(&mut self, name: &str, bytes: &[u8]) -> Result<Vec<String>, String> {
        let import = self.services.import.as_ref().ok_or("no importer configured")?;
        let (doc, warnings) = import(name, bytes)?;
        // The caller records the path it read from.
        self.session.add_document(doc, None);
        self.sync_views();
        self.ui.status = format!("Opened {name}");
        self.ui.status_error = false;
        notices::io_warnings(self, &format!("Opened {name}"), &warnings);
        Ok(warnings)
    }

    /// File › Open: native platforms return all selected paths; the web delivers its pick through
    /// the single-file service/inbox instead.
    pub fn open_dialog_file(&mut self) {
        if let Some(pick_paths) = self.services.pick_open_paths.as_mut() {
            if let Some(paths) = pick_paths() {
                self.open_paths(&paths);
            }
            return;
        }
        let Some((path, bytes)) = self.services.pick_open.as_mut().and_then(|f| f()) else { return };
        if let Err(e) = bytes.and_then(|bytes| self.open_file(&path, &bytes)) {
            self.open_failed(&file_open::display_name(&path), &e);
        }
    }

    /// Show the open dialog for a file a command reads (a script, notes, a placed image, presets):
    /// `None` when cancelled, else its name and bytes or the read error.
    pub(crate) fn pick_file_bytes(&mut self) -> Option<Result<(String, Vec<u8>), String>> {
        let (name, bytes) = self.services.pick_open.as_mut().and_then(|f| f())?;
        Some(bytes.map(|b| (name.clone(), b)).map_err(|e| format!("{}: {e}", file_open::display_name(&name))))
    }

    /// Save the active document to `path` (or a path chosen in the save dialog); returns the path
    /// and the export warnings (also shown to the user).
    pub fn save_as(&mut self, path: Option<String>) -> Result<(String, Vec<String>), String> {
        self.save_target(path, true)
    }

    pub(crate) fn save(&mut self, path: Option<String>) -> Result<(String, Vec<String>), String> {
        self.save_target(path, false)
    }

    fn save_target(&mut self, path: Option<String>, save_as: bool) -> Result<(String, Vec<String>), String> {
        if self.jpeg_options.is_some() {
            return Err(tl!("Finish the current dialog first.").into());
        }
        // Edit Contents documents save back into their smart object.
        if path.is_none() && self.session.is_enabled("layer.smartObjects.saveContents") {
            self.run("layer.smartObjects.saveContents", serde_json::json!({}))?;
            return Ok(("smart object".into(), Vec::new()));
        }
        let st = self.session.active().ok_or("no document")?;
        // A flat image with added layers needs a layered default, even though it
        // still has its original .png/.jpg path. Explicitly picked formats win.
        let suggested = file_open::suggested_save_path(&st.doc, st.path.as_deref());
        let choose_jpeg_quality = save_as || path.is_none();
        let path = match path {
            Some(p) => p,
            None => self.services.pick_save.as_mut().and_then(|f| f(&suggested)).ok_or("cancelled")?,
        };
        let settings = if jpeg_options_ui::is_jpeg_path(&path) {
            let state = self.session.active().ok_or("no document")?;
            let quality = self.jpeg_save_quality.get(&state.doc.id).copied();
            if choose_jpeg_quality || quality.is_none() {
                let as_copy = file_open::saves_as_flat_copy(&state.doc, state.path.as_deref(), &path, false);
                self.park_jpeg_save(path.clone(), as_copy)?;
                return Ok((path, Vec::new()));
            }
            ExportSettings { jpeg_quality: quality, ..Default::default() }
        } else {
            ExportSettings::default()
        };
        // Ordinary Save writes the editable smart-filter parameters to the working file.
        // Delivering a baked Photoshop copy is an explicit Save As / export choice.
        if save_as && ps_collaboration_ui::preflight_save(self, &path)? {
            return Ok((path, Vec::new()));
        }
        // A layered TIFF asks about its layers first (Preferences › File Handling); the save
        // continues from the prompt.
        if tiff_options_ui::wants_prompt(self, &path) {
            tiff_options_ui::park(self, path.clone());
            return Ok((path, Vec::new()));
        }
        self.write_document(path, &settings)
    }

    pub(crate) fn park_jpeg_save(&mut self, path: String, as_copy: bool) -> Result<(), String> {
        if self.jpeg_options.is_some() {
            return Err(tl!("Finish the current dialog first.").into());
        }
        let state = self.session.active().ok_or("no document")?;
        let default = self.session.prefs().export.jpeg_quality.clamp(1, 100) as u8;
        let quality = self.jpeg_save_quality.get(&state.doc.id).copied().unwrap_or(default);
        self.jpeg_options = Some(jpeg_options_ui::Prompt::new(state.doc.id, path, as_copy, quality));
        Ok(())
    }

    pub(crate) fn complete_jpeg_save(&mut self, prompt: jpeg_options_ui::Prompt) -> Result<(String, Vec<String>), String> {
        if self.session.active().is_none_or(|state| state.doc.id != prompt.document) {
            return Err("the document changed before JPEG save; choose its destination again".into());
        }
        let settings = ExportSettings { jpeg_quality: Some(prompt.quality), ..Default::default() };
        let saved = self.write_saved_image(prompt.path, &settings, prompt.as_copy)?;
        if !prompt.as_copy {
            self.jpeg_save_quality.insert(prompt.document, prompt.quality);
        }
        Ok(saved)
    }

    /// Encodes to the requested path. A flat derivative keeps the document's working path.
    pub(crate) fn write_document(&mut self, path: String, settings: &ExportSettings) -> Result<(String, Vec<String>), String> {
        let st = self.session.active().ok_or("no document")?;
        let as_copy = file_open::saves_as_flat_copy(&st.doc, st.path.as_deref(), &path, settings.tiff_layers);
        self.write_saved_image(path, settings, as_copy)
    }

    pub(crate) fn write_saved_image(&mut self, path: String, settings: &ExportSettings, as_copy: bool) -> Result<(String, Vec<String>), String> {
        let st = self.session.active().ok_or("no document")?;
        let (document, revision) = (st.doc.id, st.revision);
        let snapshot = if as_copy { st.doc.clone() } else { file_open::export_named_snapshot(&st.doc, &path) };
        let export = self.services.export.as_ref().ok_or("no exporter configured")?;
        let (bytes, warnings) = export(&snapshot, &path, settings)?;
        let write = self.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        if !as_copy {
            self.adopt_saved_path(document, revision, &path)?;
        }
        let action = if as_copy { "Saved a copy as" } else { "Saved" };
        self.ui.status = format!("{action} {path}");
        // "Save Document" script events and File › Generate › Image Assets.
        if !as_copy
            && let Some(i) = self.session.active_index()
            && let Some(r) = photocraft_engine::automate_cmds::document_saved(&mut self.session, i)
        {
            self.ui.status = format!("Saved {path}; {} image assets in {}", r["files"].as_array().map_or(0, Vec::len), r["dir"].as_str().unwrap_or(""));
        }
        self.ui.status_error = false;
        notices::output_warnings(self, &format!("{action} {}", file_open::display_name(&path)), &warnings);
        self.sync_views();
        Ok((path, warnings))
    }

    /// Save through the control session's capability-scoped writer. No file
    /// picker or ambient writer is reachable from this path. Returns the path
    /// written and the export warnings (also shown to the user).
    pub fn save_automation(&mut self, path: Option<String>) -> Result<(String, Vec<String>), String> {
        let state = self.session.active().ok_or("no document")?;
        // As File › Save: without `path` only a layered file is written back (#416).
        let target = path
            .or_else(|| state.path.clone().filter(|p| photocraft_engine::file_cmds::saves_in_place(p)))
            .ok_or("pass `path`: a save without one writes back only to the document's own PSD, PSB or .pcraft file")?;
        let (document, revision) = (state.doc.id, state.revision);
        let as_copy = file_open::saves_as_flat_copy(&state.doc, state.path.as_deref(), &target, true);
        let snapshot = if as_copy { state.doc.clone() } else { file_open::export_named_snapshot(&state.doc, &target) };
        let export = self.services.export.as_ref().ok_or("no exporter configured")?;
        let (bytes, warnings) = export(&snapshot, &target, &ExportSettings::default())?;
        let write = self.services.automation_write.as_mut().ok_or("automation write authority is not configured")?;
        write(&target, &bytes)?;
        if !as_copy {
            self.adopt_saved_path(document, revision, &target)?;
        }
        let action = if as_copy { "Saved a copy as" } else { "Saved" };
        self.ui.status = format!("{action} {target}");
        self.ui.status_error = false;
        notices::output_warnings(self, &format!("{action} {}", file_open::display_name(&target)), &warnings);
        self.sync_views();
        Ok((target, warnings))
    }

    fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else { return };
        while let Ok(req) = rx.try_recv() {
            let reply = req.reply.clone();
            let outcome = control::handle(self, ctx, &req);
            self.sync_tool_brush();
            match outcome {
                control::Outcome::Done(v) => {
                    let _ = reply.send(v);
                }
                control::Outcome::AfterInput => self.input_waiters.push(reply),
                control::Outcome::AfterJob(job) => self.jobs.waiters.push((job, reply)),
                control::Outcome::Screenshot { token, path, viewport } => {
                    // Wait out egui's fade animations (~83 ms) and a few rendered frames first.
                    let settle = ctx.global_style().animation_time as f64 * 2000.0 + 60.0;
                    self.queued_screenshots.push((token, viewport, gpu_canvas::now_ms() + settle, 0));
                    self.pending_screenshots.push((token, path, reply));
                }
            }
        }
        self.control_rx = Some(rx);
    }

    fn issue_screenshots(&mut self, ctx: &egui::Context) {
        let now = gpu_canvas::now_ms();
        self.queued_screenshots.retain_mut(|(token, viewport, at, frames)| {
            *frames += 1;
            if now >= *at && *frames >= 3 {
                ctx.send_viewport_cmd_to(*viewport, egui::ViewportCommand::Screenshot(egui::UserData::new(*token)));
                ctx.request_repaint_of(*viewport);
                false
            } else {
                true
            }
        });
    }

    pub(crate) fn collect_screenshots(&mut self, ctx: &egui::Context) {
        if self.pending_screenshots.is_empty() {
            return;
        }
        let events: Vec<_> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { user_data, image, .. } => {
                        let token = user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>()).copied()?;
                        Some((token, image.clone()))
                    }
                    _ => None,
                })
                .collect()
        });
        for (token, image) in events {
            if let Some(i) = self.pending_screenshots.iter().position(|(t, _, _)| *t == token) {
                let (_, path, reply) = self.pending_screenshots.remove(i);
                let r = control::save_screenshot(self, &image, path.as_deref());
                let _ = reply.send(r);
            }
        }
    }
}

impl eframe::App for PhotocraftApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // The root and detached viewports share the renderer. A transparent clear colour lets
        // detached documents expose the rounded corners of their egui frame. The root viewport
        // is fully covered by its CentralPanel, so its visible workspace background is unchanged.
        egui::Color32::TRANSPARENT.to_normalized_gamma_f32()
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        tool_brush_persistence::persist(self, 0.0, true);
        dock::remember(self);
        prefs_ui::persist(self, f64::MAX);
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        dock::persist(self, ctx);
        i18n::set_current(i18n::Lang::from_pref(&self.session.prefs().interface.language));
        if !self.styled {
            Self::setup_context(ctx, self.ui.theme);
            self.styled = true;
        } else {
            self.fonts_ready = true;
        }
        self.frame += 1;
        let now = ctx.input(|i| i.time);
        let dt = (now - self.last_frame_time) as f32;
        if dt > 0.0 {
            self.fps = self.fps * 0.9 + (1.0 / dt).min(240.0) * 0.1;
        }
        self.last_frame_time = now;
        self.sync_tool_brush();
        self.sync_views();
        self.check_gpu(ctx);
        #[cfg(all(debug_assertions, not(target_arch = "wasm32")))]
        if self.live_tokens.poll(ctx, self.ui.theme) {
            self.checker = None;
        }
        self.drain_control(ctx);
        native_menu::run(self, ctx);
        color_picker_ui::service(self, ctx);
        if self.ui.text_edit.is_some() && !self.ui.tool.is_type() {
            type_tool::commit(self);
        }
        if self.ui.pen.is_some() && self.ui.tool != state::Tool::Pen {
            vector_ui::pen_commit(self, false);
        }
        transform_tool::end_if_left(self);
        self.collect_screenshots(ctx);
        self.issue_screenshots(ctx);
        ps_collaboration_ui::tick(self, ctx);
        prefs_ui::tick(self, ctx);
        monitor_status::poll(self, ctx);
        // Control requests and persisted preferences can change the language in this frame.
        i18n::sync_context(ctx, &self.session.prefs().interface.language);
        // A window bigger than its display (1440 × 900 on 1366 × 768) runs under the taskbar:
        // maximize it into the work area once (#315).
        work_area::fit_window(ctx);
        discard_ui::guard_window_close(self, ctx);
        brush_picker::maintain(self, ctx);
        // Background jobs: apply finished ones, keep frames coming, Esc cancels (before the
        // shortcuts see Esc).
        jobs_ui::tick(self, ctx);
        crate::mixer_preview::tick(self, ctx);
        shortcuts::handle_before_document_pointer(self, ctx);
        self.sync_tool_brush();
        let arrived: Vec<(String, Vec<u8>)> =
            self.services.inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
        for (name, bytes) in arrived {
            if let Err(e) = self.open_bytes(&name, &bytes) {
                self.open_failed(&name, &e);
            }
        }
        // Finder double-click / Open With / Dock drops (macOS open-documents events).
        self.drain_os_events(ctx);
        // The control transport wakes the UI on arrival (ctx.request_repaint); only poll while a
        // screenshot is pending. (Polling every 50 ms here made idle apps render at 20 fps.)
        if !self.pending_screenshots.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        prefs_ui::observe_display_scale_input(ctx, raw_input);
        native_menu_ui::observe_input(ctx, raw_input);
        // Native menu key equivalents become the key presses they were (see `native_menu`).
        if let Some(menu) = self.services.native_menu.as_mut() {
            menu.raw_input(raw_input);
        }
        shortcuts::clipboard_keys(ctx, ctx.text_edit_focused() || self.ui.text_edit.is_some(), raw_input);
        // Control pointer/key coordinates target ROOT. Deferred children (including
        // a hidden prewarmed menu) also pass this hook and must not drain that queue.
        if raw_input.viewport_id == egui::ViewportId::ROOT {
            raw_input.events.extend(self.take_synthetic_step());
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // A minimized Windows root viewport can still deliver one egui frame with a 0×0
        // content rect. Rendering Window widgets in that frame makes egui clamp their saved
        // positions and sizes to the primary-monitor origin; the next restore then shows every
        // floating panel collapsed in the top-left corner. Let the native window manager finish
        // the minimize/restore transition before laying out any root or detached UI.
        let root_usable = ctx.input(|i| {
            let Some(viewport) = i.raw.viewports.get(&egui::ViewportId::ROOT) else { return true };
            if viewport.visible() == Some(false) || viewport.minimized == Some(true) {
                return false;
            }
            viewport.inner_rect.is_none_or(|rect| rect.width() >= 2.0 && rect.height() >= 2.0)
        });
        if !root_usable {
            crate::document_window_ui::root_hidden(self, &ctx);
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            return;
        }
        i18n::set_current(i18n::Lang::from_pref(&self.session.prefs().interface.language));
        // Fonts registered via set_fonts only take effect next frame; named families would panic now.
        if !self.fonts_ready {
            ctx.request_repaint();
            self.automation_input = false;
            return;
        }
        camera_raw_ui::finish_native_close(self, &ctx);
        // Resolve a manually minimized document before sidebar widgets read the
        // active document. The child viewport may still report stale focus.
        document_window_ui::reconcile_visible_focus(self, &ctx);
        // The raw drop belongs to this UI pass. App::logic may still see the previous pass.
        if ctx.current_pass_index() == 0 {
            if ctx.input(|i| !i.raw.hovered_files.is_empty()) {
                ctx.request_repaint();
            }
            let dropped = ctx.input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
            if !dropped.is_empty() {
                let at = self.services.cursor_pos.as_mut().and_then(|read| read(&ctx));
                self.open_dropped(&ctx, dropped, at);
            }
            self.place_next_dropped(&ctx);
        }
        workspace_ui::process_pending(self, &ctx);
        preset_files_ui::process_pending(self, &ctx);
        let t0 = gpu_canvas::now_ms();
        // View › Screen Mode › Full Screen Mode: only the image, on black (F or Esc returns).
        let chrome = !self.ui.view.hides_chrome();
        if !ctx.embed_viewports() {
            panel_workspace::migrate(self, &ctx);
        }
        if !chrome && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            let _ = menus::invoke(self, &ctx, "view.screenMode.standard", serde_json::json!({}));
        }
        if chrome {
            panels::title_bar(self, ui);
        }
        if chrome && self.ui.panels.options_bar {
            panels::options_bar(self, ui);
        }
        if chrome && self.ui.panels.status_bar {
            panels::status_bar(self, ui);
        }
        if chrome && self.ui.panels.toolbar {
            panels::toolbar(self, ui);
            self.sync_tool_brush();
        }
        if chrome {
            panels::right_dock(self, ui);
        }
        let t = theme::Tokens::get(&ctx);
        let backdrop = if chrome { prefs_ui::pasteboard_color(self).unwrap_or(t.canvas) } else { egui::Color32::BLACK };
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(backdrop)).show(ui, |ui| {
            ui.add_enabled_ui(self.camera_raw.is_none() && self.jpeg_options.is_none(), |ui| canvas::document_area(self, ui));
        });
        shortcuts::handle_after_document_pointer(self, &ctx);
        panels::properties_window(self, &ctx);
        brush_panel::window(self, &ctx);
        dock::floating(self, &ctx);
        preset_panels::windows(self, &ctx);
        preset_panels::manager(self, &ctx);
        type_panels_ui::windows(self, &ctx);
        analysis_ui::windows(self, &ctx);
        timeline_ui::windows(self, &ctx);
        workspace_ui::windows(self, &ctx);
        palette::show(self, &ctx);
        dialogs::show(self, &ctx);
        jobs_ui::dialog(self, &ctx);
        if self.discard.is_some() {
            brush_picker::close(self, &ctx);
        }
        discard_ui::show(self, &ctx);
        tiff_options_ui::show(self, &ctx);
        match jpeg_options_ui::show(self, &ctx) {
            Some(jpeg_options_ui::Answer::Save(prompt)) => {
                if let Err(error) = self.complete_jpeg_save(prompt) {
                    notices::error(self, i18n::fmt(tl!("Save failed: {error}"), &[("error", &error)]));
                }
                ctx.request_repaint();
            }
            Some(jpeg_options_ui::Answer::Cancel) => {
                self.ui.status = tl!("Save cancelled").into();
                self.ui.status_error = false;
                ctx.request_repaint();
            }
            None => {}
        }
        distort_ui::show(self, &ctx);
        camera_raw_ui::show(self, &ctx);
        wide_angle_ui::show(self, &ctx);
        native_menu_ui::show(self, &ctx);
        layer_menu_ui::show_native(self, &ctx);
        preset_panels::show_tool_presets_native(self, &ctx);
        if !ctx.embed_viewports() {
            brush_picker::show_native(self, &ctx);
        }
        canvas::extra_windows(self, &ctx);
        canvas_tool_menu::show_native(self, &ctx);
        color_picker_ui::service(self, &ctx);
        native_popup_ui::cleanup(&ctx, self.frame);
        crate::mixer_preview::tick(self, &ctx);
        notices::show(self, &ctx);
        gpu_status::show_fallback(self, &ctx);
        if self.custom_titlebar {
            titlebar::resize_zones(ui, &mut self.services.contact_screen_pos);
        }
        // A device lost while drawing this frame: switch to the CPU canvas before the next one.
        gpu_status::check(self, &ctx);
        self.automation_input = false;
        native_menu::sync(self, &ctx);
        document_title_v75::sync(self, &ctx);
        self.perf.frame(gpu_canvas::now_ms() - t0);
        diagnostics_ui::frame_built(self, &ctx);
        // Synthetic input is injected one press/release step per frame: keep frames coming until
        // the queue is empty, then release control replies waiting on it.
        if !self.synthetic.is_empty() {
            ctx.request_repaint();
        } else if !self.input_waiters.is_empty() {
            for w in self.input_waiters.drain(..) {
                let _ = w.send(serde_json::json!({"ok": true, "result": null}));
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_dropped(f: &(dyn egui::DroppedFile + Send + Sync)) -> Result<Vec<u8>, String> {
    f.bytes()
}

/// On the web, dropped-file bytes arrive asynchronously; the web shell handles drops itself.
#[cfg(target_arch = "wasm32")]
fn read_dropped(_f: &dyn egui::DroppedFile) -> Result<Vec<u8>, String> {
    Err("drag-and-drop on the web is handled by the page; use File → Open".into())
}

impl PhotocraftApp {
    /// `params` aimed at the active layer's mask (`"target":"mask"`) when the Layers panel targets
    /// it and command `id` edits the target (adjustments, filters, fills) without naming one:
    /// ⌘I then inverts the mask, as in Photoshop (#780). A targeted alpha channel or Quick Mask
    /// mode wins, as the engine routes those itself.
    pub fn with_mask_target(&self, id: &str, params: Value) -> Value {
        if !self.ui.mask_target || !photocraft_engine::channel_cmds::follows_target(id) || params.get("target").is_some() {
            return params;
        }
        let Some(st) = self.session.active() else { return params };
        let composite = st.channel_view.target == photocraft_engine::channel_cmds::ChannelTarget::Composite && st.doc.quick_mask.is_none();
        if !composite || st.active_layer.and_then(|id| st.doc.layer(id)).is_none_or(|l| l.mask.is_none()) {
            return params;
        }
        match params {
            Value::Object(mut m) => {
                m.insert("target".into(), Value::from("mask"));
                Value::Object(m)
            }
            _ => serde_json::json!({ "target": "mask" }),
        }
    }

    /// Viewing a layer mask (#196) targets it; a vector-mask target needs a vector mask on the
    /// active layer (a shape layer's path is its content, not a mask).
    fn sync_mask_targets(&mut self) {
        let Some(st) = self.session.active() else { return };
        if photocraft_engine::mask_view_cmds::current(st).is_some() {
            self.ui.mask_target = true;
            self.ui.vector_mask_target = false;
        }
        if self.ui.vector_mask_target && !mask_thumbs_ui::has_vector_mask(st) {
            self.ui.vector_mask_target = false;
        }
    }

    fn prune_thumbs(&mut self) {
        let documents = self.session.documents();
        if self.thumb_documents.len() == documents.len()
            && self.thumb_documents.iter().zip(documents).all(|((id, snapshot), st)| *id == st.doc.id && snapshot.ptr_eq(&std::sync::Arc::downgrade(&st.doc)))
        {
            return;
        }
        self.thumb_fingerprints.prune(documents.iter().map(|state| state.doc.id));
        // Check snapshots every sync, but walk the layer tree only after it changes. A Weak
        // tracks replacement (including undo and preserved-ID reopen) without retaining pixels.
        if !self.thumbs.is_empty() {
            let mut live = std::collections::HashSet::new();
            let mut pending: Vec<_> = documents.iter().flat_map(|st| &st.doc.layers).collect();
            while let Some(layer) = pending.pop() {
                live.insert((layer.id, mask_thumbs_ui::THUMB_LAYER));
                if layer.mask.is_some() {
                    live.insert((layer.id, mask_thumbs_ui::THUMB_MASK));
                }
                if layer.vector_mask.is_some() {
                    live.insert((layer.id, mask_thumbs_ui::THUMB_VECTOR));
                }
                if let Some(children) = layer.children() {
                    pending.extend(children);
                }
            }
            // Native files can restore IDs: retain the union across all open documents, rather
            // than deleting an ID just because one document containing it was closed.
            self.thumb_fingerprints.retain_keys(&live);
            self.thumbs.retain(|key, _| live.contains(key));
        }
        self.thumb_documents = documents.iter().map(|st| (st.doc.id, std::sync::Arc::downgrade(&st.doc))).collect();
    }

    pub fn set_theme(&mut self, ctx: &egui::Context, kind: theme::ThemeKind) {
        let pro = |k| matches!(k, theme::ThemeKind::Pro | theme::ThemeKind::ProMedium);
        if pro(self.ui.theme) != pro(kind) {
            // The first two tabs have different presentation order in the two theme families.
            // Keep their meaning, including detached individual tabs, when the theme changes.
            let remap = |i: &mut usize| {
                if *i < 2 {
                    *i = 1 - *i;
                }
            };
            remap(&mut self.ui.dock_tabs.color);
            for container in &mut self.ui.dock.containers {
                for tab in &mut container.tabs {
                    if tab.group == dock::Group::Color {
                        remap(&mut tab.index);
                    }
                }
            }
        }
        self.ui.theme = kind;
        theme::palette::apply(ctx, kind, &self.session.prefs().interface.studio_light_colors);
        self.checker = None;
    }

    /// Cached 64px thumbnail of a pixel-ish layer, laid out in document space.
    pub fn layer_thumb(&mut self, ctx: &egui::Context, doc: &Document, layer: &photocraft_doc::Layer) -> egui::TextureId {
        // Key by content, not document revision: COW tiles change pointer only when their pixels
        // change, so unrelated edits (e.g. painting another layer) don't rebuild this thumbnail.
        let rev = layer.surface().map_or(0, |surface| self.cached_surface_fingerprint(doc, layer.id, mask_thumbs_ui::THUMB_LAYER, surface))
            ^ (doc.size.width as u64) << 40
            ^ (doc.size.height as u64).rotate_left(23);
        let key = (layer.id, mask_thumbs_ui::THUMB_LAYER);
        if let Some((r, tex)) = self.thumbs.get(&key)
            && *r == rev
        {
            return tex.id();
        }
        let mut px = [0.0f32; 8];
        let img = thumb_image(doc, 64, |x, y| {
            let Some(s) = layer.surface() else { return [0.0; 4] };
            let n = s.channels();
            s.read_pixel(x, y, &mut px[..n]);
            photocraft_raster::to_rgba(&s.format(), &px[..n])
        });
        self.store_thumb(ctx, key, rev, img)
    }

    pub fn mask_thumb(&mut self, ctx: &egui::Context, doc: &Document, id: photocraft_doc::LayerId, mask: &photocraft_doc::LayerMask) -> egui::TextureId {
        let rev = self.cached_surface_fingerprint(doc, id, mask_thumbs_ui::THUMB_MASK, &mask.surface)
            ^ (doc.size.width as u64) << 40
            ^ (doc.size.height as u64).rotate_left(23);
        let key = (id, mask_thumbs_ui::THUMB_MASK);
        if let Some((r, tex)) = self.thumbs.get(&key)
            && *r == rev
        {
            return tex.id();
        }
        let mut v = [0.0f32; 1];
        let img = thumb_image(doc, 64, |x, y| {
            mask.surface.read_pixel(x, y, &mut v);
            [v[0], v[0], v[0], 1.0]
        });
        self.store_thumb(ctx, key, rev, img)
    }

    fn cached_surface_fingerprint(&mut self, doc: &Document, layer: photocraft_doc::LayerId, kind: u8, surface: &photocraft_raster::Surface) -> u64 {
        // Match the actual immutable snapshot; live previews use the safe fallback.
        if let Some(source) = self.session.documents().iter().map(|state| &state.doc).find(|source| std::ptr::eq(source.as_ref(), doc)) {
            self.thumb_fingerprints.fingerprint(source, layer, kind, surface)
        } else {
            surface_fingerprint(surface)
        }
    }

    fn store_thumb(&mut self, ctx: &egui::Context, key: (photocraft_doc::LayerId, u8), rev: u64, img: egui::ColorImage) -> egui::TextureId {
        match self.thumbs.get_mut(&key) {
            Some((r, tex)) => {
                tex.set(img, egui::TextureOptions::LINEAR);
                *r = rev;
                tex.id()
            }
            None => {
                let tex = ctx.load_texture(format!("thumb-{}-{}", key.0.0, key.1), img, egui::TextureOptions::LINEAR);
                let id = tex.id();
                self.thumbs.insert(key, (rev, tex));
                id
            }
        }
    }
}

/// Cheap identity of a surface's pixels: tile coordinates and `Arc` pointers.
pub fn surface_fingerprint(s: &photocraft_raster::Surface) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut format_hash = std::collections::hash_map::DefaultHasher::new();
    s.format().hash(&mut format_hash);
    s.default_bytes().hash(&mut format_hash);
    let mut h: u64 = 0xcbf2_9ce4_8422_2325 ^ s.tile_count() as u64 ^ format_hash.finish();
    for (c, t) in s.tiles() {
        let p = std::sync::Arc::as_ptr(t) as usize as u64;
        h = (h ^ p ^ ((c.tx as u64) << 32 | c.ty as u32 as u64)).wrapping_mul(0x100_0000_01b3);
    }
    h
}

/// Square thumbnail of the canvas area, letterboxed, sampling `f(x, y)` in document space.
fn thumb_image(doc: &Document, side: usize, mut f: impl FnMut(i32, i32) -> [f32; 4]) -> egui::ColorImage {
    let (w, h) = (doc.size.width.max(1) as f32, doc.size.height.max(1) as f32);
    let scale = w.max(h) / side as f32;
    let (ox, oy) = ((side as f32 - w / scale) / 2.0, (side as f32 - h / scale) / 2.0);
    let mut px = vec![egui::Color32::TRANSPARENT; side * side];
    for ty in 0..side {
        for tx in 0..side {
            let dx = (tx as f32 - ox + 0.5) * scale;
            let dy = (ty as f32 - oy + 0.5) * scale;
            if dx < 0.0 || dy < 0.0 || dx >= w || dy >= h {
                continue;
            }
            let c = f(dx as i32, dy as i32);
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            px[ty * side + tx] = egui::Color32::from_rgba_unmultiplied(q(c[0]), q(c[1]), q(c[2]), q(c[3]));
        }
    }
    egui::ColorImage::new([side, side], px)
}

impl PhotocraftApp {
    /// Next step of queued synthetic input (automation): events up to and including the first
    /// release, preserving a complete click/key pulse in one frame. Hosts that don't call
    /// `raw_input_hook` (offscreen harnesses) feed these to their input themselves.
    pub fn take_synthetic_step(&mut self) -> Vec<egui::Event> {
        let n = self
            .synthetic
            .iter()
            .position(|e| matches!(e, egui::Event::PointerButton { pressed: false, .. } | egui::Event::Key { pressed: false, .. }))
            .map_or(self.synthetic.len(), |i| i + 1);
        let events: Vec<_> = self.synthetic.drain(..n).collect();
        if !events.is_empty() {
            self.automation_input = true;
        }
        events
    }

    /// Install fonts, image loaders and the theme. Call from the app creator when possible so the
    /// very first frame renders; otherwise `logic` does it and the first frame is skipped.
    pub fn setup_context(ctx: &egui::Context, kind: theme::ThemeKind) {
        theme::install_fonts(ctx);
        egui_extras::install_image_loaders(ctx);
        theme::apply(ctx, kind);
        // egui's own ⌘+ / ⌘- / ⌘0 scale the whole interface; PhotoCraft zooms the canvas instead
        // (shortcuts.rs), like Photoshop.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
    }
}

impl PhotocraftApp {
    /// Cached `Surface::content_bounds` keyed by an id and the surface's tile identity.
    pub fn cached_bounds(&mut self, key: u64, surface: &photocraft_raster::Surface) -> photocraft_geom::Rect {
        let rev = surface_fingerprint(surface);
        if let Some((r, b)) = self.bounds_cache.get(&key)
            && *r == rev
        {
            return *b;
        }
        let b = surface.content_bounds();
        if self.bounds_cache.len() > 256 {
            self.bounds_cache.clear();
        }
        self.bounds_cache.insert(key, (rev, b));
        b
    }
}

impl PhotocraftApp {
    /// Zoom of the active document's main view (screen points per document pixel).
    pub fn current_zoom(&self) -> f32 {
        self.session.active_index().and_then(|i| self.ui.views.get(i)).map_or(1.0, |v| v.zoom)
    }
}

impl PhotocraftApp {
    /// Channels panel thumbnails of the active document snapshot: the composite,
    /// each colour channel (when there is more than one), each alpha channel, then the Quick Mask.
    pub fn channel_thumbs(&mut self, ctx: &egui::Context) -> Vec<egui::TextureId> {
        let Some(st) = self.session.active() else {
            self.channel_thumbs = None;
            return Vec::new();
        };
        let (id, doc) = (st.doc.id, st.doc.clone());
        // View-only commands bump revision without changing pixels. Keeping a Weak pins allocation
        // identity against address reuse without retaining the document's pixel data.
        let snapshot = std::sync::Arc::downgrade(&doc);
        if !matches!(&self.channel_thumbs, Some((d, old, _)) if *d == id && old.ptr_eq(&snapshot)) {
            let comp = photocraft_compose::thumbnail(&doc, 56);
            let (w, h) = (comp.width as usize, comp.height as usize);
            let side = w.max(h);
            let make = |f: &dyn Fn(&[u8]) -> egui::Color32, name: &str| {
                let mut px = vec![egui::Color32::TRANSPARENT; side * side];
                let (ox, oy) = ((side - w) / 2, (side - h) / 2);
                for y in 0..h {
                    for x in 0..w {
                        let o = (y * w + x) * 4;
                        px[(y + oy) * side + x + ox] = f(&comp.pixels[o..o + 4]);
                    }
                }
                ctx.load_texture(format!("chan-{name}"), egui::ColorImage::new([side, side], px), egui::TextureOptions::LINEAR)
            };
            let fmt = doc.pixel_format();
            let colors = fmt.mode.color_channels();
            let mut texs = vec![make(&|p| egui::Color32::from_rgb(p[0], p[1], p[2]), "composite")];
            if colors > 1 {
                for k in 0..colors {
                    let cmyk = fmt.mode == photocraft_doc::ColorMode::Cmyk;
                    texs.push(make(
                        &|p| {
                            let rgba = [p[0], p[1], p[2], 255].map(|v| f32::from(v) / 255.0);
                            let x = photocraft_raster::from_rgba(&fmt, rgba)[k];
                            let g = if cmyk { 1.0 - x } else { x };
                            egui::Color32::from_gray((g.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
                        },
                        &format!("c{k}"),
                    ));
                }
            }
            // Alpha channels and the Quick Mask, sampled at thumbnail resolution.
            let (dw, dh) = (doc.size.width.max(1) as f32, doc.size.height.max(1) as f32);
            for (i, ch) in doc.channels.iter().chain(doc.quick_mask.as_ref()).enumerate() {
                let mut px = vec![egui::Color32::TRANSPARENT; side * side];
                let (ox, oy) = ((side - w) / 2, (side - h) / 2);
                for y in 0..h {
                    for x in 0..w {
                        let sx = ((x as f32 + 0.5) * dw / w as f32) as i32;
                        let sy = ((y as f32 + 0.5) * dh / h as f32) as i32;
                        let v = ch.surface.sample_channel(sx, sy, 0);
                        px[(y + oy) * side + x + ox] = egui::Color32::from_gray((v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                    }
                }
                texs.push(ctx.load_texture(format!("chan-a{i}"), egui::ColorImage::new([side, side], px), egui::TextureOptions::LINEAR));
            }
            self.channel_thumbs = Some((id, snapshot, texs));
        }
        self.channel_thumbs.as_ref().map(|(_, _, t)| t.iter().map(|t| t.id()).collect()).unwrap_or_default()
    }
}

fn clip_signature(w: u32, h: u32, px: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut signature = std::collections::hash_map::DefaultHasher::new();
    w.hash(&mut signature);
    h.hash(&mut signature);
    px.hash(&mut signature);
    signature.finish()
}

impl PhotocraftApp {
    /// Mirror the session clipboard onto the OS clipboard (RGBA8).
    fn export_os_clipboard(&mut self) {
        let (Some(set), Some(clip)) = (self.services.clipboard_set_image.as_mut(), self.session.clipboard.as_ref()) else { return };
        let b = clip.bounds;
        if b.is_empty() {
            return;
        }
        let mut px = vec![[0u8; 4]; b.width() as usize * b.height() as usize];
        clip.surface.read_rgba8_into(b, &mut px);
        let bytes: Vec<u8> = px.into_iter().flatten().collect();
        if set(b.width(), b.height(), &bytes).is_ok() {
            self.os_clip_sig = Some(clip_signature(b.width(), b.height(), &bytes));
        }
    }

    /// If the OS clipboard holds an image that isn't the one we put there, make it the session
    /// clipboard (so ⌘V pastes screenshots and images copied in other apps, like Photoshop).
    /// Returns true when a new external image was imported.
    pub(crate) fn import_os_clipboard(&mut self) -> Result<bool, String> {
        let image = if let Some(get) = self.services.clipboard_get_image_checked.as_mut() {
            get()?
        } else if let Some(get) = self.services.clipboard_get_image.as_mut() {
            match get() {
                Some((width, height, pixels)) => ClipboardRead::Image { width, height, pixels },
                None => ClipboardRead::Empty,
            }
        } else {
            return Ok(false);
        };
        let (w, h, bytes) = match image {
            ClipboardRead::Unchanged => {
                return if self.session.clipboard.is_some() { Ok(false) } else { Err("clipboard revision has no imported image; copy the image again".into()) };
            }
            ClipboardRead::Empty => {
                // Internal copies still paste on platforms without image export. An old
                // external image must not reappear after the user copies text or clears it.
                if self.clip_external {
                    self.session.clipboard = None;
                    self.os_clip_sig = None;
                    self.clip_external = false;
                }
                return Ok(false);
            }
            ClipboardRead::Image { width, height, pixels } => (width, height, pixels),
        };
        let expected = u64::from(w).checked_mul(u64::from(h)).and_then(|pixels| pixels.checked_mul(4));
        if w == 0 || h == 0 || w > 262_144 || h > 262_144 || u64::from(w) * u64::from(h) > 100_000_000 {
            return Err("clipboard image exceeds the 100 megapixel paste limit (or has invalid dimensions)".into());
        }
        if expected != Some(bytes.len() as u64) {
            return Err("clipboard image pixel data has an invalid length".into());
        }
        let sig = clip_signature(w, h, &bytes);
        if self.os_clip_sig == Some(sig) && self.session.clipboard.is_some() {
            return Ok(false);
        }
        let r = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
        let mut surface = photocraft_raster::Surface::from_interleaved(photocraft_color::PixelFormat::RGBA8, r, &bytes);
        surface.prune();
        self.session.clipboard = Some(photocraft_engine::edit_cmds::Clip { surface, bounds: r });
        self.os_clip_sig = Some(sig);
        self.clip_external = true;
        Ok(true)
    }
}

#[cfg(test)]
mod input_tests;

#[cfg(test)]
mod tool_brush_switch_tests_v59;

#[cfg(test)]
mod native_corner_pixel_tests;

#[cfg(test)]
mod pencil_tests;

#[cfg(test)]
mod transform_undo_tests;

#[cfg(test)]
mod move_auto_select_tests;

#[cfg(test)]
mod marquee_tests;

#[cfg(test)]
mod stamp_tests;

#[cfg(test)]
mod polygon_lasso_tests;

#[cfg(test)]
mod clipboard_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn checked_empty_clears_external_but_preserves_internal_clipboard_v75() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.run("file.new", serde_json::json!({"width": 8, "height": 8})).unwrap();
        app.run("edit.copy", serde_json::json!({})).unwrap();
        app.services.clipboard_get_image_checked = Some(Box::new(|| Ok(ClipboardRead::Empty)));
        app.run("edit.paste", serde_json::json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 2, "an internal copy still pastes when OS images are unavailable");
        app.services.clipboard_get_image_checked = Some(Box::new(|| Ok(ClipboardRead::Image { width: 1, height: 1, pixels: vec![255, 0, 0, 255] })));
        app.run("edit.paste", serde_json::json!({})).unwrap();
        assert!(app.clip_external);
        let before = app.session.active().unwrap().doc.layers.len();
        app.services.clipboard_get_image_checked = Some(Box::new(|| Ok(ClipboardRead::Empty)));
        let result = app.run("edit.paste", serde_json::json!({})).unwrap();
        assert_eq!(result["pasted"], serde_json::json!(false));
        assert_eq!(app.session.active().unwrap().doc.layers.len(), before, "external old pixels must not paste after text/empty clipboard");
        assert!(app.session.clipboard.is_none() && app.os_clip_sig.is_none() && !app.clip_external);
    }

    #[test]
    fn unchanged_clipboard_requires_an_imported_surface_v75() {
        let mut app =
            PhotocraftApp::new(Session::new(), Services { clipboard_get_image_checked: Some(Box::new(|| Ok(ClipboardRead::Unchanged))), ..Default::default() });
        assert!(app.import_os_clipboard().is_err());
        app.services.clipboard_get_image_checked = Some(Box::new(|| Ok(ClipboardRead::Image { width: 1, height: 1, pixels: vec![255, 0, 0, 255] })));
        assert!(app.import_os_clipboard().unwrap());
        let original = app.session.clipboard.as_ref().unwrap().surface.clone();
        app.services.clipboard_get_image_checked = Some(Box::new(|| Ok(ClipboardRead::Unchanged)));
        assert!(!app.import_os_clipboard().unwrap());
        assert_eq!(app.session.clipboard.as_ref().unwrap().surface, original);
    }

    #[test]
    fn external_change_outside_old_signature_samples_is_imported_v75() {
        let mut first = [0, 0, 0, 255].repeat(300);
        let image = first.clone();
        let mut app = PhotocraftApp::new(
            Session::new(),
            Services {
                clipboard_get_image_checked: Some(Box::new(move || Ok(ClipboardRead::Image { width: 300, height: 1, pixels: image.clone() }))),
                ..Default::default()
            },
        );
        assert!(app.import_os_clipboard().unwrap());
        let previous = clip_signature(300, 1, &first);
        first[4] = 255; // Byte 4 was never sampled by the old step_by(997) hash.
        assert_ne!(previous, clip_signature(300, 1, &first));
        app.services.clipboard_get_image_checked = Some(Box::new(move || Ok(ClipboardRead::Image { width: 300, height: 1, pixels: first.clone() })));
        assert!(app.import_os_clipboard().unwrap());
        assert_eq!(app.session.clipboard.as_ref().unwrap().surface.rgba(1, 0), [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn clipboard_read_failure_does_not_paste_old_pixels_v75() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.run("file.new", serde_json::json!({"width": 8, "height": 8})).unwrap();
        app.run("edit.copy", serde_json::json!({})).unwrap();
        let old_clipboard = app.session.clipboard.as_ref().unwrap().surface.clone();
        let old_count = app.session.active().unwrap().doc.layers.len();
        app.services.clipboard_get_image_checked = Some(Box::new(|| Err("clipboard decode rejected".into())));
        let error = crate::menus::invoke(&mut app, &egui::Context::default(), "edit.paste", serde_json::json!({})).unwrap_err();
        assert_eq!(error, "clipboard decode rejected");
        assert_eq!(app.session.active().unwrap().doc.layers.len(), old_count);
        app.automation_input = true;
        app.session.edit_prefs(|preferences| preferences.script_events.enabled = true);
        assert!(app.run("edit.paste", serde_json::json!({})).is_err());
        assert!(app.session.prefs().script_events.enabled, "a rejected paste restores automation preference state");
        assert_eq!(app.session.clipboard.as_ref().unwrap().surface, old_clipboard);
        assert!(app.run("edit.pasteSpecial.pasteInPlace", serde_json::json!({})).is_err());
        assert_eq!(app.session.active().unwrap().doc.layers.len(), old_count);
    }

    #[test]
    fn clipboard_oversized_or_malformed_payload_rejected_without_surface_alloc_v75() {
        for (width, height, pixels) in [(u32::MAX, u32::MAX, vec![]), (10_001, 10_000, vec![]), (2, 3, vec![0; 4])] {
            let mut app = PhotocraftApp::new(
                Session::new(),
                Services {
                    clipboard_get_image_checked: Some(Box::new(move || Ok(ClipboardRead::Image { width, height, pixels: pixels.clone() }))),
                    ..Default::default()
                },
            );
            assert!(crate::menus::is_enabled(&app, "edit.paste"));
            assert!(app.run("edit.paste", serde_json::json!({})).is_err());
            assert!(app.session.clipboard.is_none());
            assert!(app.session.active().is_none());
        }
    }

    #[test]
    fn checked_clipboard_pastes_exact_alpha_once_at_target_depths_v75() {
        for depth in [8, 16, 32] {
            let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let observed = reads.clone();
            let mut app = PhotocraftApp::new(
                Session::new(),
                Services {
                    clipboard_get_image_checked: Some(Box::new(move || {
                        observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(ClipboardRead::Image { width: 2, height: 1, pixels: vec![255, 0, 0, 128, 0, 0, 255, 255] })
                    })),
                    ..Default::default()
                },
            );
            app.run("file.new", serde_json::json!({"width": 8, "height": 8, "depth": depth})).unwrap();
            crate::menus::invoke(&mut app, &egui::Context::default(), "edit.paste", serde_json::json!({})).unwrap();
            assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
            let d = app.session.active().unwrap();
            assert_eq!(d.doc.layers.len(), 2);
            let pixels = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap();
            let bounds = pixels.content_bounds();
            let first = pixels.rgba(bounds.x0, bounds.y0);
            let second = pixels.rgba(bounds.x0 + 1, bounds.y0);
            assert!(first[0] > 0.99 && (first[3] - 128.0 / 255.0).abs() < 0.01, "{depth}: {first:?}");
            assert!(second[2] > 0.99 && second[3] > 0.99, "{depth}: {second:?}");
        }
    }

    #[test]
    fn os_clipboard_bridge() {
        let os: OsClip = Arc::default();
        let (a, b) = (os.clone(), os.clone());
        let services = Services {
            clipboard_set_image: Some(Box::new(move |w: u32, h: u32, px: &[u8]| {
                *a.lock().unwrap() = Some((w, h, px.to_vec()));
                Ok(())
            })),
            clipboard_get_image: Some(Box::new(move || b.lock().unwrap().clone())),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(Session::new(), services);
        app.session.execute("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
        app.sync_views();
        app.run("select.rect", serde_json::json!({"x": 0, "y": 0, "width": 8, "height": 4})).unwrap();
        app.run("edit.copy", serde_json::json!({})).unwrap();
        let (w, h, px) = os.lock().unwrap().clone().unwrap();
        assert_eq!((w, h, px.len()), (8, 4, 8 * 4 * 4));
        // Our own image comes back unchanged (no re-import, keeps the original position).
        assert!(!app.import_os_clipboard().unwrap());
        // Another app puts a 3×2 red image on the clipboard: ⌘V pastes it.
        *os.lock().unwrap() = Some((3, 2, [255u8, 0, 0, 255].repeat(6)));
        app.run("edit.paste", serde_json::json!({})).unwrap();
        let st = app.session.active().unwrap();
        let surf = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
        assert_eq!(surf.content_bounds().width(), 3);
    }

    type OsClip = Arc<Mutex<Option<(u32, u32, Vec<u8>)>>>;

    /// An app whose OS clipboard is `os`, counting every read in `reads`.
    fn app_with_os_clipboard(os: &OsClip, reads: &Arc<std::sync::atomic::AtomicUsize>) -> PhotocraftApp {
        let (b, n) = (os.clone(), reads.clone());
        let get = move || {
            n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            b.lock().unwrap().clone()
        };
        let mut app = PhotocraftApp::new(Session::new(), Services { clipboard_get_image: Some(Box::new(get)), ..Default::default() });
        app.session.execute("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
        app.sync_views();
        app
    }

    #[test]
    fn external_clipboard_enables_and_centres_paste() {
        // Paste is enabled from the clipboard service alone (no read); the explicit paste imports
        // the OS image, once, and centres it like Photoshop does for foreign clipboard content.
        let (os, reads) = (OsClip::default(), Arc::default());
        let mut app = app_with_os_clipboard(&os, &reads);
        *os.lock().unwrap() = Some((4, 4, [255u8, 0, 0, 255].repeat(16)));
        assert!(app.session.clipboard.is_none());
        assert!(crate::menus::is_enabled(&app, "edit.paste"));
        assert!(crate::menus::is_enabled(&app, "edit.pasteSpecial.pasteInPlace"));
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 0, "enablement never reads the clipboard");
        let r = crate::menus::invoke(&mut app, &egui::Context::default(), "edit.paste", serde_json::json!({})).unwrap();
        assert_ne!(r["offset"], serde_json::json!([0, 0]), "external image is centred: {r}");
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1, "one read per explicit paste");
        // Without a clipboard service Paste greys until something is copied in the app.
        let mut plain = PhotocraftApp::new(Session::new(), Services::default());
        plain.session.execute("file.new", serde_json::json!({"width": 8, "height": 8})).unwrap();
        assert!(!crate::menus::is_enabled(&plain, "edit.paste"));
    }

    /// #368: an image copied in another app opens as a document of its own, from File › New
    /// from Clipboard or from Paste with nothing open.
    #[test]
    fn os_clipboard_image_becomes_a_new_document() {
        let (os, reads) = (OsClip::default(), Arc::new(std::sync::atomic::AtomicUsize::new(0)));
        let (b, n) = (os.clone(), Arc::clone(&reads));
        let get = move || {
            n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            b.lock().unwrap().clone()
        };
        let mut app = PhotocraftApp::new(Session::new(), Services { clipboard_get_image: Some(Box::new(get)), ..Default::default() });
        let ctx = egui::Context::default();
        // Listed right after File › New…, enabled without reading the clipboard.
        let items = crate::menus::menu_items(&app);
        let at = items.iter().position(|i| i.id == "file.new").unwrap();
        assert_eq!(items[at + 1].id, "file.newFromClipboard");
        assert!(items[at + 1].enabled && crate::menus::is_enabled(&app, "edit.paste"));
        assert!(!crate::menus::is_enabled(&app, "edit.pasteSpecial.pasteInPlace"), "Paste in Place needs a document");
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 0);
        // An empty clipboard is a quiet no-op.
        let r = crate::menus::invoke(&mut app, &ctx, "file.newFromClipboard", serde_json::json!({})).unwrap();
        assert_eq!(r["pasted"], serde_json::json!(false));
        assert!(app.session.documents().is_empty() && !app.ui.status_error);
        // Paste with nothing open makes the document.
        *os.lock().unwrap() = Some((5, 3, [0u8, 0, 255, 255].repeat(15)));
        crate::menus::invoke(&mut app, &ctx, "edit.paste", serde_json::json!({})).unwrap();
        let d = &app.session.active().unwrap().doc;
        assert_eq!((d.size.width, d.size.height, d.layers.len()), (5, 3, 1));
        assert_eq!(app.ui.views.len(), 1, "the new document has a view");
        // New from Clipboard with a document open adds another.
        crate::menus::invoke(&mut app, &ctx, "file.newFromClipboard", serde_json::json!({})).unwrap();
        assert_eq!(app.session.documents().len(), 2);
    }

    #[test]
    fn pasting_an_empty_or_non_image_os_clipboard_is_a_quiet_no_op() {
        let (os, reads) = (OsClip::default(), Arc::default());
        let mut app = app_with_os_clipboard(&os, &reads);
        let layers = app.session.active().unwrap().doc.layers.len();
        let r = crate::menus::invoke(&mut app, &egui::Context::default(), "edit.paste", serde_json::json!({})).unwrap();
        assert_eq!(r["pasted"], serde_json::json!(false));
        assert_eq!(app.run("edit.pasteSpecial.pasteInPlace", serde_json::json!({})).unwrap()["pasted"], serde_json::json!(false));
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers);
        assert!(!app.ui.status_error && !app.ui.status.is_empty());
    }

    #[test]
    fn idle_frames_never_read_the_os_clipboard() {
        let (os, reads) = (OsClip::default(), Arc::<std::sync::atomic::AtomicUsize>::default());
        *os.lock().unwrap() = Some((4, 4, [255u8, 0, 0, 255].repeat(16)));
        let (b, n) = (os.clone(), reads.clone());
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let get = move || {
                n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                b.lock().unwrap().clone()
            };
            let mut app = PhotocraftApp::new(Session::new(), Services { clipboard_get_image: Some(Box::new(get)), ..Default::default() });
            app.session.execute("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
            app
        });
        for _ in 0..20 {
            h.step();
        }
        assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 0, "the OS clipboard is read only on an explicit paste");
        assert!(h.state().session.clipboard.is_none());
    }
}

#[cfg(test)]
mod palette_integration_tests_v68;
