//! Background jobs in the shell (#210): long commands and file opens run on worker threads (see
//! `photocraft_engine::jobs`) while the window keeps drawing at full frame rate.
//!
//! - [`run`] starts a job-capable command in the background (when [`PhotocraftApp::background_jobs`]
//!   is on: the desktop app; tests and the web run inline as before).
//! - [`tick`] (every frame) applies finished jobs, reports failures, answers control requests
//!   waiting on a job, finishes background opens and keeps frames coming while anything runs.
//! - [`status_progress`] draws the status-bar progress with a Cancel button; [`dialog`] the modal
//!   progress window for jobs that lock the active document (filters, Content-Aware Fill, …),
//!   shown after a short delay so quick jobs don't flash it. Esc cancels.
//! - Files opening in the background get a document tab at once ([`OpenTab`]); its canvas area
//!   shows the progress, and closing the tab cancels the open.
//!
//! The canvas keeps showing the pre-job state: workers never touch the session's documents.

use std::sync::Arc;
use std::sync::mpsc::Sender;

use egui::{Color32, Rect, RichText, Sense, Stroke, vec2};
use photocraft_engine::jobs::{JobEvent, JobId, JobInfo, JobOutcome, OPEN_JOB, OpenSource, Started};
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{ControlResponse, PhotocraftApp, notices};

/// Delay before the modal progress dialog appears (quick jobs finish without it).
pub const DIALOG_DELAY_MS: f64 = 250.0;
/// Native progress is the window body itself, not a modal card floating inside
/// a larger transparent viewport. Keep enough height for the same controls at
/// the auxiliary host's minimum size.
const NATIVE_DIALOG_SIZE: egui::Vec2 = vec2(420.0, 200.0);

/// A file opening in the background: shown as a document tab with progress.
#[derive(Clone, Debug)]
pub struct OpenTab {
    pub job: JobId,
    /// Display name (the file name).
    pub name: String,
    /// Where it was read from (File › Save writes back there; added to Open Recent).
    pub path: Option<String>,
    /// The tab position it goes to when it opens (a file dropped on the tabs); `None`: the end.
    pub slot: Option<usize>,
}

/// The shell's job bookkeeping.
#[derive(Default)]
pub struct JobsUi {
    /// Control replies waiting for a job to end.
    pub(crate) waiters: Vec<(JobId, Sender<ControlResponse>)>,
    /// Background opens, in tab order.
    pub opens: Vec<OpenTab>,
    /// The open tab shown in the canvas area instead of the active document.
    pub focus: Option<JobId>,
    /// The job the latest [`run`] started (the control channel waits on it).
    pub(crate) last_started: Option<JobId>,
}

/// Run command `id`: in the background when background jobs are on and the command is a job
/// (the result is then `{"job": id, "pending": true}` and the edit lands when [`tick`] applies
/// it), else synchronously like [`photocraft_engine::Session::execute`].
pub fn run(app: &mut PhotocraftApp, id: &str, params: Value) -> Result<Value, String> {
    if !app.background_jobs {
        return app.session.execute(id, params).map_err(|e| e.to_string());
    }
    match app.session.start(id, params).map_err(|e| e.to_string())? {
        Started::Done(v) => Ok(v),
        Started::Job(job) => {
            app.jobs.last_started = Some(job);
            // The status bar's progress readout names the job; clear any old message.
            app.ui.status.clear();
            app.ui.status_error = false;
            Ok(json!({"job": job.0, "pending": true}))
        }
    }
}

/// Open a file in the background: a tab appears at once and shows the progress; the document
/// replaces it when decoded. `path` is remembered for File › Save and Open Recent.
pub fn start_open(app: &mut PhotocraftApp, name: &str, path: Option<String>, source: OpenSource) -> Result<(), String> {
    let name = &app.open_name(name);
    match app.session.start_open(name, source).map_err(|e| e.to_string())? {
        // Inline (wasm): finish now, like a background open that ended at once.
        Started::Done(v) => finish_open(app, name, path.as_deref(), &v, None).map(|_| ()),
        Started::Job(job) => {
            app.jobs.opens.push(OpenTab { job, name: name.to_string(), path, slot: None });
            app.jobs.focus = Some(job);
            app.ui.chrome.home = None;
            app.ui.status = crate::i18n::fmt(tl!("Opening {name}…"), &[("name", name)]);
            app.ui.status_error = false;
            Ok(())
        }
    }
}

/// Per frame: apply finished jobs and react to them; keep frames coming while jobs run; Esc
/// cancels the job in view.
pub fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // Opening a new document in the engine makes it active. A pending floating tab
    // may finish while the previous tab still owns a placement or brush gesture.
    let held = (app.ui.transform.is_some() || app.drag.is_some() || app.ui.text_edit.is_some()).then(|| app.session.active().map(|s| s.doc.id)).flatten();
    for e in app.session.poll_jobs() {
        on_event(app, e);
    }
    if let Some(index) = held.and_then(|id| app.session.documents().iter().position(|state| state.doc.id == id)) {
        app.session.set_active(index);
    }
    // A focused open tab that ended (or was cancelled elsewhere) gives way to the documents.
    if app.jobs.focus.is_some_and(|j| !app.jobs.opens.iter().any(|o| o.job == j)) {
        app.jobs.focus = last_root_open(app);
    }
    if !app.session.has_jobs() {
        return;
    }
    // Animate progress and poll for results at display rate; the UI thread only draws.
    ctx.request_repaint_after(std::time::Duration::from_millis(16));
    if let Some(job) = job_in_view(app)
        && !ctx.text_edit_focused()
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        cancel(app, job.id);
    }
}

/// The job the user is looking at: the open tab in view, the active document's job, or (for
/// the status bar) the newest running job.
pub fn job_in_view(app: &PhotocraftApp) -> Option<JobInfo> {
    if let Some(j) = app.jobs.focus.and_then(|j| app.session.job(j)) {
        return Some(j);
    }
    app.session.active_job().or_else(|| app.session.jobs().pop())
}

/// Cancel job `id` (the status bar's ×, the dialog's Cancel, Esc, closing an opening tab).
pub fn cancel(app: &mut PhotocraftApp, id: JobId) {
    let label = app.session.job(id).map(|j| display_job_label(&j.command, &j.label));
    if app.session.cancel_job(id) {
        // The event (and its status text) arrives with the next poll; say so now as well.
        if let Some(l) = label {
            app.ui.status = crate::i18n::fmt(tl!("Cancelled {label}"), &[("label", &l)]);
            app.ui.status_error = false;
        }
    }
}

fn on_event(app: &mut PhotocraftApp, e: JobEvent) {
    // Control requests waiting on this job.
    let reply = match &e.outcome {
        JobOutcome::Done(v) => json!({"ok": true, "result": v}),
        JobOutcome::Failed(err) => json!({"ok": false, "error": err}),
        JobOutcome::Cancelled => json!({"ok": false, "error": "cancelled"}),
    };
    app.jobs.waiters.retain(|(job, tx)| {
        if *job == e.id {
            let _ = tx.send(reply.clone());
            false
        } else {
            true
        }
    });
    if e.command == OPEN_JOB {
        let Some(i) = app.jobs.opens.iter().position(|o| o.job == e.id) else { return };
        let tab = app.jobs.opens.remove(i);
        let mut opened = None;
        match e.outcome {
            JobOutcome::Done(v) => match finish_open(app, &tab.name, tab.path.as_deref(), &v, tab.slot) {
                Ok(document) => opened = document,
                Err(err) => app.open_failed(&tab.name, &err),
            },
            JobOutcome::Failed(err) => app.open_failed(&tab.name, &err),
            JobOutcome::Cancelled => {
                app.ui.status = crate::i18n::fmt(tl!("Cancelled opening {name}"), &[("name", &tab.name)]);
                app.ui.status_error = false;
            }
        }
        crate::document_window_ui::finish_open_tab(app, tab.job, opened);
        return;
    }
    match e.outcome {
        JobOutcome::Done(v) if matches!(e.command.as_str(), "brush.presets.importAbr" | "tool.presets.importTpl") => {
            let n = v.get("count").and_then(Value::as_u64).unwrap_or(0);
            let group = v.get("group").and_then(Value::as_str).unwrap_or_default();
            let what = if e.command == "tool.presets.importTpl" { tl!("Tool Presets") } else { tl!("Brushes") };
            app.ui.status = crate::i18n::fmt(tl!("Imported {n} {what} from {group}"), &[("n", &n.to_string()), ("what", what), ("group", group)]);
            if let Some(warnings) = v.get("warnings").and_then(Value::as_array) {
                let warnings: Vec<String> = warnings.iter().filter_map(Value::as_str).map(str::to_string).collect();
                let title = if e.command == "tool.presets.importTpl" { tl!("Load Tool Presets…") } else { tl!("Load Brushes…") };
                notices::io_warnings(app, title, &warnings);
            }
            app.ui.status_error = false;
            if e.command == "tool.presets.importTpl" {
                app.ui.panels.tool_presets = true;
                crate::dock::reveal(app, crate::dock::Group::ToolPresets);
            } else {
                app.ui.panels.brush_settings = true;
                app.ui.brush_tab = 1;
                crate::dock::reveal(app, crate::dock::Group::Brush);
            }
        }
        JobOutcome::Done(_) => {
            app.sync_views();
            app.ui.status = display_job_label(&e.command, &e.label);
            app.ui.status_error = false;
        }
        JobOutcome::Failed(err) => {
            app.sync_views();
            notices::error(app, format!("{}: {err}", display_job_label(&e.command, &e.label)));
        }
        JobOutcome::Cancelled => {
            app.ui.status = crate::i18n::fmt(tl!("Cancelled {label}"), &[("label", &display_job_label(&e.command, &e.label))]);
            app.ui.status_error = false;
        }
    }
}

/// What [`crate::PhotocraftApp::open_bytes`] does after decoding, for a background open's result
/// (`{document, warnings, color}`).
fn finish_open(app: &mut PhotocraftApp, name: &str, path: Option<&str>, v: &Value, slot: Option<usize>) -> Result<Option<photocraft_doc::DocId>, String> {
    let mut index = v.get("document").and_then(Value::as_u64).ok_or("the open job returned no document")? as usize;
    if let Some(slot) = slot
        && let Ok(moved) = app.run("document.move", json!({"document": index, "to": slot}))
    {
        index = moved.get("document").and_then(Value::as_u64).map_or(index, |i| i as usize);
    }
    let warnings: Vec<String> =
        v.get("warnings").and_then(Value::as_array).map(|a| a.iter().filter_map(|w| w.as_str().map(str::to_string)).collect()).unwrap_or_default();
    app.session.set_active(index);
    let document = app.session.active().map(|state| state.doc.id);
    if let Some(p) = path {
        app.opened_from(p);
    }
    app.sync_views();
    app.ui.status = crate::i18n::fmt(tl!("Opened {name}"), &[("name", name)]);
    app.ui.status_error = false;
    notices::io_warnings(app, &crate::i18n::fmt(tl!("Opened {name}"), &[("name", name)]), &warnings);
    // Script events bound to "Open Document".
    photocraft_engine::automate_cmds::document_opened(&mut app.session);
    app.sync_views();
    let color = v.get("color").cloned().unwrap_or(Value::Null);
    let ask = color.get("ask").and_then(Value::as_bool) == Some(true);
    if ask && (color.get("mismatch").and_then(Value::as_bool) == Some(true) || color.get("missing").is_some()) {
        crate::prefs_ui::open_mismatch(app, &color);
    }
    Ok(document)
}

// ------------------------------------------------------------------------------------- drawing

/// A progress bar: `frac` of `rect` filled with the accent; `None` = indeterminate (a sliding
/// segment), for jobs that haven't reported progress yet.
pub fn bar(ui: &egui::Ui, rect: Rect, frac: Option<f32>, t: &Tokens) {
    let p = ui.painter();
    let r = rect.height() / 2.0;
    p.rect_filled(rect, r, crate::theme::palette::color(ui.ctx(), photocraft_engine::prefs::palette::ColorSlot::ProgressTrack));
    p.rect_stroke(rect, r, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    let fill = match frac {
        Some(f) => Rect::from_min_size(rect.min, vec2((rect.width() * f.clamp(0.0, 1.0)).max(rect.height()), rect.height())),
        None => {
            let time = ui.input(|i| i.time) as f32;
            let w = rect.width() * 0.3;
            let x = rect.left() + ((time * 0.8).fract() * (rect.width() + w)) - w;
            Rect::from_min_max(egui::pos2(x.max(rect.left()), rect.top()), egui::pos2((x + w).min(rect.right()), rect.bottom()))
        }
    };
    if fill.width() > 0.0 {
        p.rect_filled(fill, r, crate::theme::palette::color(ui.ctx(), photocraft_engine::prefs::palette::ColorSlot::ProgressFill));
    }
}

/// Progress that has been reported, or `None` (indeterminate) while it's still 0.
fn shown_fraction(j: &JobInfo) -> Option<f32> {
    (j.progress > 0.0).then_some(j.progress)
}

fn percent(j: &JobInfo) -> String {
    // Nothing yet reported: the bar is indeterminate, so no number.
    if j.progress > 0.0 { format!("{:.0}%", j.progress * 100.0) } else { String::new() }
}

/// Engine/control payloads retain their English labels. Only the known file
/// open template is formatted here; filenames and arbitrary plug-in labels
/// must not be treated as translation keys of their own.
fn display_job_label(command: &str, label: &str) -> String {
    if command == OPEN_JOB
        && let Some(name) = label.strip_prefix("Opening ")
    {
        crate::i18n::fmt(tl!("Opening {name}"), &[("name", name)])
    } else {
        tl!(label).to_string()
    }
}

/// The status bar's job readout: "Gaussian Blur ▬▬▬▭▭ 45 %  ×". Draws nothing when no job runs.
pub fn status_progress(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(j) = job_in_view(app) else { return };
    let t = Tokens::get(ui.ctx());
    let (bar_w, bar_h) = if t.pro { (120.0, 6.0) } else { (140.0, 6.0) };
    let more = app.session.jobs().len().saturating_sub(1);
    // Right to left: ×, percentage, bar, label.
    let (xr, xresp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
    if xresp.hovered() {
        ui.painter().rect_filled(xr, t.radius_sm, t.hover);
    }
    crate::icons::paint(ui, xr, "x", 11.0, if xresp.hovered() { t.text } else { t.text_dim });
    let xresp = xresp.on_hover_text(tl!("Cancel (Esc)"));
    ui.label(RichText::new(percent(&j)).color(t.text_dim).size(11.5).monospace());
    let (br, _) = ui.allocate_exact_size(vec2(bar_w, bar_h), Sense::hover());
    bar(ui, br, shown_fraction(&j), &t);
    let label = display_job_label(&j.command, &j.label);
    let label = if more > 0 { format!("{label} (+{more})") } else { label };
    let label_width = ui.available_width().clamp(1.0, 200.0);
    ui.add_sized(vec2(label_width, 18.0), egui::Label::new(RichText::new(&label).color(t.text_dim).size(12.0)).truncate()).on_hover_text(&label);
    if xresp.clicked() {
        cancel(app, j.id);
    }
}

/// The modal progress window for a job locking the active document (Photoshop shows one for
/// long filters): title, status, bar, percentage and elapsed time, Cancel. Appears after
/// [`DIALOG_DELAY_MS`]; Esc cancels (see [`tick`]).
pub fn dialog(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(j) = app.session.active_job() else { return };
    if j.elapsed_ms < DIALOG_DELAY_MS {
        return;
    }
    if !ctx.embed_viewports() && ctx.viewport_id() == egui::ViewportId::ROOT {
        let mut cancel_it = false;
        let title = display_job_label(&j.command, &j.label);
        let close = crate::dialogs::show_auxiliary_viewport(app, ctx, "job-progress", &title, NATIVE_DIALOG_SIZE, |_, ui| {
            cancel_it = native_dialog_body(ui, &j).inner;
        });
        if close || cancel_it {
            cancel(app, j.id);
        }
        crate::dialogs::finish_auxiliary(app, ctx, "job-progress");
        return;
    }
    let modal = egui::Modal::new(egui::Id::new("job-progress")).backdrop_color(Color32::from_black_alpha(36)).show(ctx, |ui| {
        ui.set_width(360.0);
        dialog_contents(ui, &j)
    });
    // A click on the backdrop doesn't cancel the work; Esc is handled in `tick`.
    if modal.inner {
        cancel(app, j.id);
    }
}

/// Paint the entire native client area opaquely. In particular, do not recurse
/// into `dialog`: that would nest an egui modal (and its transparent margin)
/// inside an already-modal native window.
fn native_dialog_body(ui: &mut egui::Ui, job: &JobInfo) -> egui::InnerResponse<bool> {
    let t = Tokens::get(ui.ctx());
    let client = ui.available_rect_before_wrap();
    let mut needed_height = 0.0;
    let response = egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style()).fill(t.card).inner_margin(16.0)).show(ui, |ui| {
        let start = ui.cursor().top();
        let cancel = dialog_contents(ui, job);
        needed_height = ui.cursor().top() - start + 32.0;
        cancel
    });
    // Larger UI fonts or a wrapped status may need more room than the compact
    // initial client height. Grow to the actual laid-out contents; never reset
    // an already-larger user-resized window on each progress tick.
    if needed_height > client.height() + 0.5 {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(client.width(), needed_height)));
    }
    response
}

fn dialog_contents(ui: &mut egui::Ui, job: &JobInfo) -> bool {
    let t = Tokens::get(ui.ctx());
    let title = display_job_label(&job.command, &job.label);
    ui.add(egui::Label::new(RichText::new(&title).font(crate::theme::semibold(15.0))).truncate()).on_hover_text(&title);
    ui.add_space(4.0);
    crate::widgets::hairline(ui);
    ui.add_space(10.0);
    let msg = if job.message.is_empty() || job.message == job.label { tl!("Working…") } else { tl!(&job.message) };
    // A long phase remains readable without allowing arbitrary status text to
    // push the Cancel button off-screen. Hover exposes the complete message.
    let mut message = egui::text::LayoutJob::simple(msg.to_owned(), egui::TextStyle::Body.resolve(ui.style()), t.text_dim, ui.available_width());
    message.wrap.max_rows = 3;
    message.wrap.break_anywhere = true;
    ui.add(egui::Label::new(message)).on_hover_text(msg);
    ui.add_space(8.0);
    let (br, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
    bar(ui, br, shown_fraction(job), &t);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(percent(job)).color(t.text_faint).size(11.5));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new(elapsed(job.elapsed_ms)).color(t.text_faint).size(11.5));
        });
    });
    ui.add_space(12.0);
    // Bound the right-aligned footer to a natural horizontal row. A bare
    // `with_layout` here claims all remaining vertical space, making the
    // measured cursor depend on client height and requesting growth forever.
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
            ui.label(RichText::new(tl!("Esc to cancel")).color(t.text_faint).size(11.0));
            cancel
        })
        .inner
    })
    .inner
}

fn elapsed(ms: f64) -> String {
    let s = (ms / 1000.0).max(0.0);
    if s < 60.0 { format!("{s:.1} s") } else { format!("{}:{:02}", (s / 60.0).floor() as u64, (s % 60.0).floor() as u64) }
}

/// The canvas area of an opening tab: the file name, a bar and Cancel, centred on the
/// workspace backdrop.
pub fn open_card(app: &mut PhotocraftApp, ui: &mut egui::Ui, job: JobId) {
    let Some(tab) = app.jobs.opens.iter().find(|o| o.job == job).cloned() else { return };
    let Some(j) = app.session.job(job) else { return };
    let t = Tokens::get(ui.ctx());
    let area = ui.available_rect_before_wrap();
    ui.painter().rect_filled(area, 0.0, t.canvas);
    let card = Rect::from_center_size(area.center(), vec2(340.0, 132.0));
    ui.painter().rect_filled(card, t.radius_lg, t.card);
    ui.painter().rect_stroke(card, t.radius_lg, Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(card.shrink(18.0)));
    let title = crate::i18n::fmt(tl!("Opening {name}"), &[("name", &tab.name)]);
    child.add(egui::Label::new(RichText::new(&title).font(crate::theme::semibold(14.0))).truncate()).on_hover_text(&title);
    child.add_space(4.0);
    let msg = if j.message.is_empty() { tl!("Reading…").to_string() } else { format!("{}…", tl!(&j.message)) };
    child.add(egui::Label::new(RichText::new(&msg).color(t.text_dim).size(12.0)).truncate()).on_hover_text(&msg);
    child.add_space(10.0);
    let (br, _) = child.allocate_exact_size(vec2(child.available_width(), 8.0), Sense::hover());
    bar(&child, br, shown_fraction(&j), &t);
    child.add_space(10.0);
    let mut cancel_it = false;
    child.horizontal(|ui| {
        ui.label(RichText::new(percent(&j)).color(t.text_faint).size(11.5));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            cancel_it = crate::widgets::secondary_button(ui, tl!("Cancel"), 76.0).clicked();
        });
    });
    if cancel_it {
        cancel(app, job);
    }
}

/// Open tabs for the tab strips: (job, title, progress).
pub fn open_tabs(app: &PhotocraftApp) -> Vec<(JobId, String, f32)> {
    app.jobs
        .opens
        .iter()
        .filter(|o| !crate::document_window_ui::owns_open_tab(app, o.job))
        .map(|o| (o.job, o.name.clone(), app.session.job(o.job).map_or(0.0, |j| j.progress)))
        .collect()
}

pub(crate) fn last_root_open(app: &PhotocraftApp) -> Option<JobId> {
    app.jobs.opens.iter().rev().find(|o| !crate::document_window_ui::owns_open_tab(app, o.job)).map(|o| o.job)
}

/// Draw one opening tab's progress underline inside tab rect `r`.
pub fn tab_underline(ui: &egui::Ui, r: Rect, frac: f32, t: &Tokens) {
    let line = Rect::from_min_max(egui::pos2(r.left() + 4.0, r.bottom() - 3.0), egui::pos2(r.right() - 4.0, r.bottom() - 1.0));
    ui.painter().rect_filled(
        line,
        1.0,
        crate::theme::palette::override_color(ui.ctx(), photocraft_engine::prefs::palette::ColorSlot::ProgressTrack).unwrap_or(t.field_border),
    );
    let f = Rect::from_min_size(line.min, vec2(line.width() * frac.clamp(0.0, 1.0), line.height()));
    ui.painter().rect_filled(f, 1.0, crate::theme::palette::color(ui.ctx(), photocraft_engine::prefs::palette::ColorSlot::ProgressFill));
}

/// Jobs for `ui.inspect`.
pub fn inspect(app: &PhotocraftApp) -> Value {
    json!({
        "running": app.session.jobs(),
        "opening": app.jobs.opens.iter().map(|o| json!({"job": o.job.0, "name": o.name, "path": o.path})).collect::<Vec<_>>(),
        "focus": app.jobs.focus.map(|j| j.0),
        "background": app.background_jobs,
    })
}

/// Bytes shared with a background open without copying them again.
pub fn bytes(b: &[u8]) -> OpenSource {
    OpenSource::Bytes(Arc::new(b.to_vec()))
}

#[cfg(test)]
#[path = "jobs_ui_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "jobs_ui_tests_v67.rs"]
mod tests_v67;
