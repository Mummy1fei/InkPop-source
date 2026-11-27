//! Save / Don't Save / Cancel prompt for actions that would throw away unsaved changes: closing
//! documents, Revert, and leaving the app (File › Exit or the window's close button).
//!
//! egui is immediate-mode, so the action is parked in [`Prompt`] while the modal is up and re-run
//! once every affected document has been answered. Cancel at any point drops it. Documents are
//! tracked by id, not tab index, so closing one elsewhere while the prompt is up can't retarget it.

use egui::Key;
use photocraft_doc::DocId;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::palette::ScrollAreaPaletteExt;
use crate::widgets::{ButtonRole, DialogButton};

const EXIT: &str = "file.exit";
const PROMPT_PADDING: f32 = 24.0;
const MESSAGE_BUTTON_GAP: f32 = 16.0;
const BUTTON_GAP: f32 = 10.0;

/// Presentation only: keep long downloads readable without creating a wall of
/// filename text. Keep the start and extension-bearing tail, never rename a document.
fn compact_document_name(name: &str) -> String {
    if name.chars().count() <= 80 {
        return name.to_owned();
    }
    let start: String = name.chars().take(52).collect();
    let tail: String = name.chars().rev().take(27).collect::<Vec<_>>().into_iter().rev().collect();
    format!("{start}…{tail}")
}

#[derive(Clone, Copy)]
struct PromptLayout {
    content_width: f32,
    max_height: f32,
    initial_size: egui::Vec2,
}

type PromptButton = (ButtonRole, &'static str, Option<Key>, f32, Answer);

fn prompt_buttons(ctx: &egui::Context, reverts: bool) -> Vec<PromptButton> {
    let mac = ctx.os() == egui::os::OperatingSystem::Mac;
    let cancel = (ButtonRole::Cancel, "Cancel", mac.then_some(Key::C), 84.0, Answer::Cancel);
    if reverts {
        vec![(ButtonRole::Default, "Revert", Some(Key::R), 84.0, Answer::Discard), cancel]
    } else if mac {
        vec![
            (ButtonRole::Default, "Save", Some(Key::S), 84.0, Answer::Save),
            (ButtonRole::Alternate, "Don't Save", Some(Key::D), 100.0, Answer::Discard),
            cancel,
        ]
    } else {
        vec![(ButtonRole::Default, "Yes", Some(Key::Y), 84.0, Answer::Save), (ButtonRole::Alternate, "No", Some(Key::N), 84.0, Answer::Discard), cancel]
    }
}

fn button_row_width(ctx: &egui::Context, buttons: &[PromptButton]) -> f32 {
    let color = crate::theme::Tokens::get(ctx).text;
    buttons
        .iter()
        .map(|button| {
            let label = button.2.map_or_else(|| tl!(button.1).to_string(), |key| mnemonic(button.1, key));
            (ctx.fonts_mut(|fonts| fonts.layout_no_wrap(label, crate::theme::medium(13.0), color)).size().x + 28.0).max(button.3)
        })
        .sum::<f32>()
        + BUTTON_GAP * buttons.len().saturating_sub(1) as f32
}

fn punctuation_only_last_row(galley: &egui::Galley) -> bool {
    galley.rows.len() > 1
        && galley.rows.last().is_some_and(|row| {
            let mut glyphs = row.glyphs.iter().map(|glyph| glyph.chr).filter(|character| !character.is_whitespace());
            glyphs.next().is_some_and(|character| matches!(character, '?' | '？' | '!' | '！' | '。' | '.' | '”' | '’' | '"' | '\''))
                && glyphs.all(|character| matches!(character, '?' | '？' | '!' | '！' | '。' | '.' | '”' | '’' | '"' | '\''))
        })
}

fn message_galley(ctx: &egui::Context, message: &str, width: f32) -> std::sync::Arc<egui::Galley> {
    let style = ctx.global_style();
    let font = egui::TextStyle::Body.resolve(&style);
    let color = style.visuals.text_color();
    let width = width.max(1.0);
    let layout = |width| ctx.fonts_mut(|fonts| fonts.layout(message.to_owned(), font.clone(), color, width));
    let mut galley = layout(width);
    // egui can put CJK closing punctuation alone on the final line. Rebalance
    // the preceding line without modifying the filename or inserting fake text.
    for inset in [8.0, 16.0, 24.0, 32.0, 48.0] {
        if !punctuation_only_last_row(&galley) {
            break;
        }
        galley = layout((width - inset).max(width * 0.5));
    }
    galley
}

fn prompt_layout(ctx: &egui::Context, message: &str, reverts: bool) -> PromptLayout {
    let visible = crate::work_area::visible_rect(ctx).size();
    let max_width = (visible.x - 24.0).max(2.0 * PROMPT_PADDING + 1.0);
    let max_height = (visible.y - 24.0).max(2.0 * PROMPT_PADDING + 1.0);
    let style = ctx.global_style();
    let natural = ctx.fonts_mut(|fonts| fonts.layout_no_wrap(message.to_owned(), egui::TextStyle::Body.resolve(&style), style.visuals.text_color())).size().x;
    let buttons = prompt_buttons(ctx, reverts);
    let row_width = button_row_width(ctx, &buttons);
    let content_width = natural.clamp(320.0, 520.0).max(row_width).min((max_width - 2.0 * PROMPT_PADDING).max(1.0));
    let button_height = if crate::theme::Tokens::get(ctx).pro { 28.0 } else { 30.0 };
    let rows_height =
        if row_width <= content_width { button_height } else { button_height * buttons.len() as f32 + 8.0 * buttons.len().saturating_sub(1) as f32 };
    let initial_height = message_galley(ctx, message, content_width).size().y + MESSAGE_BUTTON_GAP + rows_height + 2.0 * PROMPT_PADDING + 8.0;
    PromptLayout { content_width, max_height, initial_size: egui::vec2(content_width + 2.0 * PROMPT_PADDING, initial_height.min(max_height)) }
}

/// An action waiting on the user, and the dirty documents still to ask about.
pub struct Prompt {
    id: String,
    params: Value,
    /// The document the command was aimed at (`document` param, else the active one), if it has one.
    target: Option<DocId>,
    docs: Vec<DocId>,
    /// Native confirmation windows acquire focus once, rather than fighting other applications.
    native_opened: bool,
    awaiting_save: bool,
    saved_companion: Option<(DocId, u64)>,
    /// Apply native answers after a complete root frame without the confirmation, so the
    /// renderer has detached its native surface and restored the owner.
    pending_answer: Option<(Answer, u64)>,
}

fn index_of(app: &PhotocraftApp, id: DocId) -> Option<usize> {
    app.session.documents().iter().position(|d| d.doc.id == id)
}

/// Which documents a command discards, relative to its target (the `document` param, else the active one).
#[derive(Clone, Copy)]
enum Reach {
    Target,
    AllButTarget,
    All,
}

/// Every command that can throw away unsaved changes. A new one only needs a row here.
const DISCARDING: &[(&str, Reach)] = &[
    ("file.close", Reach::Target),
    ("file.revert", Reach::Target),
    ("file.closeOthers", Reach::AllButTarget),
    ("file.closeAll", Reach::All),
    (EXIT, Reach::All),
];

/// The command's target document and the unsaved documents it would discard (empty for commands
/// that discard nothing).
fn discarded(app: &PhotocraftApp, id: &str, params: &Value) -> (Option<DocId>, Vec<DocId>) {
    let docs = app.session.documents();
    let target = params.get("document").and_then(Value::as_u64).map(|i| i as usize).or(app.session.active_index());
    let affected: Vec<usize> = match DISCARDING.iter().find(|(c, _)| *c == id) {
        Some((_, Reach::Target)) => target.into_iter().collect(),
        Some((_, Reach::AllButTarget)) => (0..docs.len()).filter(|&i| Some(i) != target).collect(),
        Some((_, Reach::All)) => (0..docs.len()).collect(),
        None => Vec::new(),
    };
    let dirty = affected.into_iter().filter_map(|i| docs.get(i)).filter(|d| d.is_dirty()).map(|d| d.doc.id).collect();
    (target.and_then(|i| docs.get(i)).map(|d| d.doc.id), dirty)
}

/// Park `id` behind a prompt if it would discard unsaved work. Returns whether it did.
pub fn intercept(app: &mut PhotocraftApp, id: &str, params: &Value) -> bool {
    let (target, docs) = discarded(app, id, params);
    if docs.is_empty() {
        return false;
    }
    let prompt = Prompt {
        id: id.to_string(),
        params: params.clone(),
        target,
        docs,
        native_opened: false,
        awaiting_save: false,
        saved_companion: None,
        pending_answer: None,
    };
    match &app.discard {
        None => app.discard = Some(prompt),
        // Quitting overrides whatever is pending: it covers every document, so nothing is lost.
        Some(open) if id == EXIT && open.id != EXIT => app.discard = Some(prompt),
        // Repeated quit requests (the X pressed again) keep the prompt, and the answers so far.
        Some(open) if open.id == id => {}
        Some(_) => {
            app.ui.status = tl!("Answer the unsaved-changes prompt first").into();
            app.ui.status_error = true;
        }
    }
    true
}

/// Called once per frame: holds back a window close request while there is unsaved work.
pub fn guard_window_close(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.allow_close || !ctx.input(|i| i.viewport().close_requested()) {
        return;
    }
    if intercept(app, EXIT, &Value::Null) {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
}

/// Moves on to the next document, or runs the parked action once none are left.
fn advance(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = app.discard.as_mut() else { return };
    p.awaiting_save = false;
    p.saved_companion = None;
    if !p.docs.is_empty() {
        p.docs.remove(0);
    }
    if !p.docs.is_empty() {
        return;
    }
    let Some(Prompt { id, mut params, target, .. }) = app.discard.take() else { return };
    if id == EXIT {
        app.allow_close = true;
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
        return;
    }
    if let Some(target) = target {
        // The tab may have moved since the command was issued; aim it at the same document.
        let Some(i) = index_of(app, target) else { return };
        params = json!({"document": i});
    }
    if let Err(e) = crate::menus::invoke_unguarded(app, ctx, &id, params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Saves `doc` in place; false when it is gone, the save failed or its file dialog was cancelled.
fn save(app: &mut PhotocraftApp, ctx: &egui::Context, doc: DocId) -> bool {
    let Some(i) = index_of(app, doc) else { return false };
    app.session.set_active(i);
    match crate::menus::invoke_unguarded(app, ctx, "file.save", json!({})) {
        Ok(_) => {
            let saved = app.session.documents().iter().find(|st| st.doc.id == doc).is_some_and(|st| !st.is_dirty());
            if !saved && let Some(prompt) = &mut app.discard {
                prompt.awaiting_save = true;
            }
            saved
        }
        // Backing out of the file dialog is the user's choice, not an error.
        Err(e) if e == "cancelled" => false,
        Err(e) => {
            app.ui.status = format!("Couldn't save: {e}");
            app.ui.status_error = true;
            false
        }
    }
}

/// A successfully written native companion is a complete save of this exact snapshot.
/// It satisfies an already-pending close without adopting the copy path or saved state.
pub(crate) fn note_saved_companion(app: &mut PhotocraftApp, doc: DocId, revision: u64) {
    if let Some(prompt) = &mut app.discard
        && prompt.awaiting_save
        && prompt.docs.first() == Some(&doc)
    {
        prompt.saved_companion = Some((doc, revision));
    }
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<egui::ViewportId>(native_modal_id()));
    let root_frame = ctx.cumulative_frame_nr_for(egui::ViewportId::ROOT);
    if app
        .discard
        .as_ref()
        .and_then(|p| p.pending_answer)
        .is_some_and(|(_, queued_frame)| root_frame <= queued_frame || ctx.input(|i| i.raw.viewports.contains_key(&native_viewport_id())))
    {
        // Omit the answered confirmation from this frame. A hidden HWND still has a
        // swapchain until the backend prunes it. A Discard answer must let this finish
        // before tearing down the floating document's swapchain in a later frame.
        ctx.request_repaint();
        return;
    }
    if let Some((answer, _)) = app.discard.as_mut().and_then(|p| p.pending_answer.take()) {
        let doc = app.discard.as_ref().and_then(|p| p.docs.first()).copied();
        if let Some(doc) = doc {
            apply_answer(app, ctx, doc, answer);
        }
    }
    if app.discard.as_ref().is_some_and(|p| p.awaiting_save) {
        if app.tiff_options.is_some() || app.jpeg_options.is_some() || app.ui.dialogs.iter().any(|d| crate::ps_collaboration_ui::owns(&d.fields)) {
            return;
        }
        let saved = app.discard.as_ref().and_then(|p| p.docs.first()).and_then(|id| index_of(app, *id)).is_some_and(|i| {
            let st = &app.session.documents()[i];
            !st.is_dirty() || app.discard.as_ref().and_then(|p| p.saved_companion) == Some((st.doc.id, st.revision))
        });
        if saved {
            advance(app, ctx);
            return;
        }
        if let Some(p) = &mut app.discard {
            p.awaiting_save = false;
        }
    }
    let Some(p) = &app.discard else { return };
    let Some(&doc) = p.docs.first() else { return };
    let (exits, reverts) = (p.id == EXIT, p.id == "file.revert");
    let Some(name) = index_of(app, doc).map(|i| app.session.documents()[i].doc.name.clone()) else {
        // Closed by something else while the prompt was up: nothing left to ask about.
        advance(app, ctx);
        return;
    };
    let display_name = compact_document_name(&name);
    let (title, message) = if reverts {
        ("Revert", crate::i18n::fmt(tl!("Revert “{name}” to the last saved version? Your changes will be lost."), &[("name", &display_name)]))
    } else {
        let template = if exits {
            tl!("Do you want to save the changes you made to “{name}” before quitting?")
        } else {
            tl!("Do you want to save the changes you made to “{name}” before closing?")
        };
        (tl!("Unsaved changes"), crate::i18n::fmt(template, &[("name", &display_name)]))
    };
    let layout = prompt_layout(ctx, &message, reverts);
    let native = !ctx.embed_viewports() && app.ui.windows.iter().any(|w| w.open);
    let answer = if native {
        let first = app.discard.as_ref().is_some_and(|p| !p.native_opened);
        let viewport = native_viewport_id();
        ctx.data_mut(|d| d.insert_temp(native_modal_id(), viewport));
        // The root still needs an egui modal: its widgets have already been drawn this frame.
        // Native HWND disabling is performed by the backend using the marker above.
        confirmation_modal().frame(egui::Frame::NONE).show(ctx, |_| {});
        let answer = ctx.show_viewport_immediate(viewport, native_builder(ctx, title, first, layout.initial_size), |ui, _| {
            app.collect_screenshots(ui.ctx());
            if first {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            let mut answer = None;
            egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style()).inner_margin(PROMPT_PADDING)).show(ui, |ui| {
                // The operating system already presents the title in the caption.
                let content = ui.vertical(|ui| document_prompt_content(ui, None, &message, &name, reverts, &layout));
                answer = content.inner;
                // Fit the actual wrapped message and button row, including larger UI fonts
                // and long document names, instead of retaining a fixed empty lower half.
                let size = egui::vec2(layout.initial_size.x, (content.response.rect.height() + 2.0 * PROMPT_PADDING).ceil().min(layout.max_height));
                if ui.input(|i| i.viewport().inner_rect.is_some_and(|rect| (rect.size() - size).abs().max_elem() > 1.0)) {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                }
            });
            if ui.input(|i| i.viewport().close_requested()) {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
                answer = Some(Answer::Cancel);
            }
            if answer.is_some() {
                ui.ctx().data_mut(|d| {
                    d.remove::<egui::ViewportId>(native_modal_id());
                    // Avoid another GPU presentation immediately before the confirmation closes.
                    d.insert_temp(egui::Id::new(("photocraft-native-skip-present-once", viewport)), true);
                });
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Visible(false));
            }
            answer
        });
        if let Some(p) = app.discard.as_mut() {
            p.native_opened = answer.is_none();
            p.pending_answer = answer.map(|answer| (answer, root_frame));
        }
        if answer.is_some() {
            ctx.data_mut(|d| d.remove::<egui::ViewportId>(native_modal_id()));
            ctx.request_repaint();
        }
        // Never execute file.exit (or a blocking native Save As picker) in the child callback.
        return;
    } else {
        let frame = egui::Frame::popup(&ctx.global_style()).inner_margin(PROMPT_PADDING);
        let modal = confirmation_modal().frame(frame).show(ctx, |ui| document_prompt_content(ui, Some(title), &message, &name, reverts, &layout));
        if modal.should_close() { Some(Answer::Cancel) } else { modal.inner }
    };
    if let Some(answer) = answer {
        apply_answer(app, ctx, doc, answer);
    }
}

/// Block background interaction without changing the document's displayed contrast.
/// `Frame::NONE` alone does not remove a Modal's default full-viewport black backdrop.
fn confirmation_modal() -> egui::Modal {
    egui::Modal::new(egui::Id::new("discard-prompt")).backdrop_color(egui::Color32::TRANSPARENT)
}

fn native_modal_id() -> egui::Id {
    egui::Id::new("photocraft-native-modal-viewport")
}

pub(crate) fn native_viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("discard-native-prompt")
}

fn native_builder(ctx: &egui::Context, title: &str, first: bool, size: egui::Vec2) -> egui::ViewportBuilder {
    let mut builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_resizable(false)
        .with_minimize_button(false)
        .with_maximize_button(false)
        .with_taskbar(false)
        .with_window_level(egui::WindowLevel::Normal)
        .with_min_inner_size(egui::vec2(size.x, size.y.min(120.0)));
    if first {
        builder = builder.with_inner_size(size);
        let root_rect = ctx.input(|i| i.raw.viewports.get(&egui::ViewportId::ROOT).and_then(|v| v.inner_rect));
        if let Some(rect) = root_rect.filter(|rect| rect.is_finite() && rect.is_positive()) {
            builder = builder.with_position(rect.center() - size * 0.5);
        }
        builder = builder.with_active(true);
    }
    builder
}

fn apply_answer(app: &mut PhotocraftApp, ctx: &egui::Context, doc: DocId, answer: Answer) {
    match answer {
        Answer::Cancel => app.discard = None,
        Answer::Discard => advance(app, ctx),
        Answer::Save if save(app, ctx, doc) => advance(app, ctx),
        Answer::Save => {}
    }
}

fn document_prompt_content(ui: &mut egui::Ui, title: Option<&str>, message: &str, full_name: &str, reverts: bool, layout: &PromptLayout) -> Option<Answer> {
    let content = ui.scope(|ui| prompt_content(ui, title, message, reverts, layout));
    content.response.on_hover_text(full_name);
    content.inner
}

fn prompt_content(ui: &mut egui::Ui, title: Option<&str>, message: &str, reverts: bool, layout: &PromptLayout) -> Option<Answer> {
    let ctx = ui.ctx().clone();
    let mac = ctx.os() == egui::os::OperatingSystem::Mac;
    // The answers in the platform's words; `dialog_buttons` puts them in its order. Windows (and
    // Linux) ask Yes / No / Cancel with Y, N and Esc, as Photoshop does there; macOS asks
    // Don't Save / Cancel / Save. Esc always cancels (the modal closes on it).
    let buttons = prompt_buttons(&ctx, reverts);
    let mut answer = ctx.input_mut(|i| buttons.iter().find(|b| b.2.is_some_and(|k| i.consume_key(egui::Modifiers::NONE, k))).map(|b| b.4));
    let labels: Vec<String> = buttons.iter().map(|b| b.2.map_or_else(|| tl!(b.1).to_string(), |k| mnemonic(b.1, k))).collect();
    let row: Vec<DialogButton> = buttons.iter().zip(&labels).map(|(b, label)| DialogButton::new(b.0, label, b.3)).collect();
    {
        // The modal Area can start with a narrow previous/default sizing rect.
        // `prompt_layout` already caps this width to the visible work area; using
        // the Area's old available width here makes the narrow column permanent.
        let width = layout.content_width.max(1.0);
        ui.set_width(width);
        if let Some(title) = title {
            ui.label(egui::RichText::new(tl!(title)).font(crate::theme::semibold(15.0)));
            ui.add_space(8.0);
        }
        let galley = message_galley(&ctx, message, width);
        let button_height = if crate::theme::Tokens::get(&ctx).pro { 28.0 } else { 30.0 };
        let stacked = button_row_width(&ctx, &buttons) > width;
        let actions_height = if stacked { button_height * buttons.len() as f32 + 8.0 * buttons.len().saturating_sub(1) as f32 } else { button_height };
        let title_height = if title.is_some() { 32.0 } else { 0.0 };
        let message_height = (layout.max_height - 2.0 * PROMPT_PADDING - actions_height - MESSAGE_BUTTON_GAP - title_height - 16.0).max(1.0);
        if galley.size().y > message_height {
            egui::ScrollArea::vertical().id_salt("discard-message").max_height(message_height).auto_shrink([false, true]).show_palette(ui, |body| {
                let galley = message_galley(body.ctx(), message, body.available_width());
                body.add(egui::Label::new(galley));
            });
        } else {
            ui.add(egui::Label::new(galley));
        }
        ui.add_space(MESSAGE_BUTTON_GAP);
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = BUTTON_GAP;
            if stacked {
                let order = if mac {
                    [ButtonRole::Alternate, ButtonRole::Cancel, ButtonRole::Default]
                } else {
                    [ButtonRole::Default, ButtonRole::Alternate, ButtonRole::Cancel]
                };
                for role in order {
                    for button in row.iter().filter(|button| button.role == role) {
                        if crate::widgets::dialog_buttons(ui, std::slice::from_ref(button)).is_some() {
                            answer = buttons.iter().find(|b| b.0 == role).map(|b| b.4);
                        }
                    }
                }
            } else if let Some(role) = crate::widgets::dialog_buttons(ui, &row) {
                answer = buttons.iter().find(|b| b.0 == role).map(|b| b.4);
            }
        });
    }
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape)) {
        answer = Some(Answer::Cancel);
    }
    // Enter takes the default answer, unless Tab focused a button: that one took the Enter as a click.
    if answer.is_none() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
        answer = buttons.iter().find(|b| b.0 == ButtonRole::Default).map(|b| b.4);
    }
    answer
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Save,
    Discard,
    Cancel,
}

/// "(S)ave": the key in parentheses, or appended ("Guardar (S)") when the translation doesn't start with it.
fn mnemonic(label: &str, key: Key) -> String {
    let (label, k) = (tl!(label), key.name());
    match label.split_at_checked(1) {
        Some((first, rest)) if first.eq_ignore_ascii_case(k) => format!("({first}){rest}"),
        _ => format!("{label} ({k})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_with_docs(n: usize) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for _ in 0..n {
            app.run("file.new", json!({"width": 8, "height": 8})).unwrap();
        }
        app.sync_views();
        app
    }

    fn make_dirty(app: &mut PhotocraftApp, doc: usize) {
        app.session.set_active(doc);
        app.run("layer.new.layer", json!({})).unwrap();
    }

    fn doc_id(app: &PhotocraftApp, i: usize) -> DocId {
        app.session.documents()[i].doc.id
    }

    #[test]
    fn clean_documents_close_without_asking() {
        let mut app = app_with_docs(2);
        let ctx = egui::Context::default();
        crate::menus::invoke(&mut app, &ctx, "file.close", json!({})).unwrap();
        assert!(app.discard.is_none());
        assert_eq!(app.session.documents().len(), 1);
    }

    #[test]
    fn closing_a_dirty_document_waits_for_an_answer() {
        let mut app = app_with_docs(2);
        let ctx = egui::Context::default();
        make_dirty(&mut app, 1);
        crate::menus::invoke(&mut app, &ctx, "file.close", json!({"document": 1})).unwrap();
        assert_eq!(app.session.documents().len(), 2, "nothing closes before the user answers");
        assert!(app.discard.is_some());
        advance(&mut app, &ctx);
        assert!(app.discard.is_none());
        assert_eq!(app.session.documents().len(), 1);
    }

    #[test]
    fn only_documents_that_would_be_discarded_are_asked_about() {
        let mut app = app_with_docs(3);
        make_dirty(&mut app, 0);
        make_dirty(&mut app, 2);
        let ids = [doc_id(&app, 0), doc_id(&app, 2)];
        assert_eq!(discarded(&app, "file.closeOthers", &json!({"document": 0})), (Some(ids[0]), vec![ids[1]]));
        assert_eq!(discarded(&app, "file.closeAll", &json!({})).1, ids);
        assert_eq!(discarded(&app, EXIT, &json!({})).1, ids);
        assert!(discarded(&app, "view.zoomIn", &json!({})).1.is_empty());
    }

    #[test]
    fn close_all_asks_per_dirty_document_then_runs() {
        let mut app = app_with_docs(3);
        let ctx = egui::Context::default();
        make_dirty(&mut app, 0);
        make_dirty(&mut app, 2);
        crate::menus::invoke(&mut app, &ctx, "file.closeAll", json!({})).unwrap();
        advance(&mut app, &ctx);
        assert_eq!(app.session.documents().len(), 3, "still waiting on the second document");
        advance(&mut app, &ctx);
        assert!(app.session.documents().is_empty());
    }

    #[test]
    fn the_parked_close_still_hits_its_document_after_tabs_move() {
        let mut app = app_with_docs(3);
        let ctx = egui::Context::default();
        make_dirty(&mut app, 2);
        let doomed = doc_id(&app, 2);
        crate::menus::invoke(&mut app, &ctx, "file.close", json!({"document": 2})).unwrap();
        // Another document closes while the prompt is up, shifting the tab to index 1.
        app.run("file.close", json!({"document": 0})).unwrap();
        advance(&mut app, &ctx);
        assert_eq!(app.session.documents().len(), 1);
        assert!(index_of(&app, doomed).is_none());
    }

    #[test]
    fn quitting_replaces_a_pending_close_but_other_requests_are_refused() {
        let mut app = app_with_docs(2);
        make_dirty(&mut app, 0);
        make_dirty(&mut app, 1);
        assert!(intercept(&mut app, "file.close", &json!({"document": 0})));
        assert!(intercept(&mut app, "file.closeAll", &Value::Null));
        assert_eq!(app.discard.as_ref().map(|p| (p.id.as_str(), p.docs.len())), Some(("file.close", 1)));
        assert!(app.ui.status_error);
        assert!(intercept(&mut app, EXIT, &Value::Null));
        assert_eq!(app.discard.as_ref().map(|p| (p.id.as_str(), p.docs.len())), Some((EXIT, 2)));
        // Pressing the window's X again must not restart the questions.
        app.discard.as_mut().unwrap().docs.remove(0);
        assert!(intercept(&mut app, EXIT, &Value::Null));
        assert_eq!(app.discard.as_ref().unwrap().docs.len(), 1);
    }

    #[test]
    fn exit_is_held_back_until_confirmed() {
        let mut app = app_with_docs(1);
        let ctx = egui::Context::default();
        make_dirty(&mut app, 0);
        assert!(intercept(&mut app, EXIT, &Value::Null));
        assert!(!app.allow_close);
        advance(&mut app, &ctx);
        assert!(app.allow_close);
    }

    #[test]
    fn native_confirmation_is_normal_level_and_does_not_reactivate_each_frame() {
        let ctx = egui::Context::default();
        let first = native_builder(&ctx, "Unsaved changes", true, egui::vec2(452.0, 124.0));
        assert_eq!(first.window_level, Some(egui::WindowLevel::Normal));
        assert_eq!(first.active, Some(true));
        assert_eq!(first.resizable, Some(false));
        assert_eq!(first.taskbar, Some(false));
        let subsequent = native_builder(&ctx, "Unsaved changes", false, egui::vec2(452.0, 124.0));
        assert_eq!(subsequent.active, None, "background applications must not lose focus on repaint");
        assert_eq!(subsequent.position, None, "repainting must not undo a user's dialog move");
        assert_eq!(subsequent.inner_size, None, "repainting must preserve the height fitted to the wrapped message");
    }

    #[test]
    fn confirmation_backdrop_blocks_input_without_painting_over_the_document() {
        let ctx = egui::Context::default();
        let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut backdrop = None;
        // Modal Area starts with a sizing pass; its response references the previous
        // interaction geometry until a subsequent frame. Inspect the settled result.
        for _ in 0..4 {
            let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(viewport), ..Default::default() }, |ui| {
                let modal = confirmation_modal().frame(egui::Frame::NONE).show(ui.ctx(), |_| {});
                backdrop = Some(modal.backdrop_response);
            });
            fn has_visible_rect(shape: &egui::Shape) -> bool {
                match shape {
                    egui::Shape::Rect(rect) => rect.fill.a() != 0 || rect.stroke.color.a() != 0 && rect.stroke.width > 0.0,
                    egui::Shape::Vec(shapes) => shapes.iter().any(has_visible_rect),
                    _ => false,
                }
            }
            assert!(!output.shapes.iter().any(|shape| has_visible_rect(&shape.shape)), "the native root input guard must not darken any pixels");
            output.textures_delta.clear();
        }
        let backdrop = backdrop.unwrap();
        assert_eq!(backdrop.rect, viewport);
        assert!(backdrop.sense.senses_click());
        assert!(backdrop.sense.senses_drag());
    }

    #[test]
    fn compact_native_content_has_no_repeated_title_and_centers_the_button_group() {
        use egui_kittest::kittest::Queryable;
        for theme in crate::theme::ThemeKind::ALL {
            for os in [egui::os::OperatingSystem::Windows, egui::os::OperatingSystem::Mac] {
                let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(452.0, 124.0)).with_os(os).build_ui_state(
                    |ui, ready| {
                        // Harness construction draws one frame. Register the application's
                        // named font families before allowing that first content frame.
                        if !*ready {
                            return;
                        }
                        // build_ui_state wraps its contents in an 8 pt outer margin. A
                        // native viewport callback has no such wrapper: use the full
                        // viewport here so the 420 pt content width starts at x=16,
                        // rather than x=24 (which would move its centre 8 pt right).
                        let mut viewport_ui =
                            ui.new_child(egui::UiBuilder::new().max_rect(ui.ctx().content_rect()).layout(egui::Layout::top_down(egui::Align::Min)));
                        egui::CentralPanel::default().frame(egui::Frame::NONE.inner_margin(16.0)).show(&mut viewport_ui, |ui| {
                            let layout = PromptLayout { content_width: ui.available_width(), max_height: 124.0, initial_size: egui::vec2(452.0, 124.0) };
                            prompt_content(ui, None, "Save changes before closing?", false, &layout);
                        });
                    },
                    false,
                );
                PhotocraftApp::setup_context(&h.ctx, theme);
                *h.state_mut() = true;
                h.run_steps(4);
                assert!(h.query_by_label("Unsaved changes").is_none());
                let labels = if os == egui::os::OperatingSystem::Mac { ["(D)on't Save", "(C)ancel", "(S)ave"] } else { ["(Y)es", "(N)o", "Cancel"] };
                let left = h.get_by_label(labels[0]).rect();
                let right = h.get_by_label(labels[2]).rect();
                let center = (left.left() + right.right()) * 0.5;
                assert!(
                    (center - 226.0).abs() <= 1.0,
                    "{theme:?} / {os:?}: button group must be centered; left={left:?}, right={right:?}, center={center}, expected=226"
                );
                assert!(right.bottom() <= 108.0, "{theme:?}: compact native height must contain the action row");
            }
        }
    }

    #[test]
    fn pending_native_cancel_clears_modal_marker_without_closing_a_document() {
        let mut app = app_with_docs(1);
        make_dirty(&mut app, 0);
        assert!(intercept(&mut app, "file.close", &Value::Null));
        app.discard.as_mut().unwrap().pending_answer = Some((Answer::Cancel, 0));
        let ctx = egui::Context::default();
        ctx.data_mut(|d| d.insert_temp(native_modal_id(), egui::ViewportId::from_hash_of("discard-native-prompt")));
        ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui.ctx())).textures_delta.clear();
        ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui.ctx())).textures_delta.clear();
        assert!(app.discard.is_none());
        assert_eq!(app.session.documents().len(), 1);
        assert!(ctx.data(|d| d.get_temp::<egui::ViewportId>(native_modal_id())).is_none());
    }

    #[test]
    fn pending_native_answer_waits_for_a_new_frame_not_a_layout_pass() {
        let mut app = app_with_docs(1);
        make_dirty(&mut app, 0);
        assert!(intercept(&mut app, EXIT, &Value::Null));
        let ctx = egui::Context::default();
        app.discard.as_mut().unwrap().pending_answer = Some((Answer::Discard, 0));
        let mut passes = 0;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            passes += 1;
            show(&mut app, ui.ctx());
            if passes == 1 {
                ui.ctx().request_discard("exercise a second layout pass before native output processing");
            }
        });
        out.textures_delta.clear();
        assert!(passes >= 2);
        assert!(!app.allow_close, "layout passes cannot execute an answer before the owner is re-enabled");
        assert!(out.viewport_output.values().all(|v| !v.commands.iter().any(|c| matches!(c, egui::ViewportCommand::Close))));
        ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui.ctx())).textures_delta.clear();
        assert!(app.allow_close);
    }

    #[test]
    fn pending_native_discard_closes_only_after_the_confirmation_viewport_is_gone() {
        let mut app = app_with_docs(2);
        make_dirty(&mut app, 0);
        let discarded = doc_id(&app, 0);
        let kept = doc_id(&app, 1);
        assert!(intercept(&mut app, "file.close", &json!({"document": 0})));
        app.discard.as_mut().unwrap().pending_answer = Some((Answer::Discard, 0));

        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.viewports.insert(native_viewport_id(), egui::ViewportInfo::default());
        for _ in 0..2 {
            let mut out = ctx.run_ui(input.clone(), |ui| show(&mut app, ui.ctx()));
            out.textures_delta.clear();
            assert!(!out.viewport_output.contains_key(&native_viewport_id()));
            assert!(index_of(&app, discarded).is_some());
        }
        ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui.ctx())).textures_delta.clear();
        assert!(app.discard.is_none());
        assert!(index_of(&app, discarded).is_none());
        assert_eq!(index_of(&app, kept), Some(0));
    }

    #[test]
    fn confirmed_quit_targets_root_even_if_answer_originated_in_a_child() {
        let mut app = app_with_docs(1);
        make_dirty(&mut app, 0);
        assert!(intercept(&mut app, EXIT, &Value::Null));
        let child = egui::ViewportId::from_hash_of("discard-native-prompt");
        let mut input = egui::RawInput { viewport_id: child, ..Default::default() };
        input.viewports.insert(child, egui::ViewportInfo::default());
        let mut out = egui::Context::default().run_ui(input, |ui| advance(&mut app, ui.ctx()));
        out.textures_delta.clear();
        assert!(out.viewport_output[&egui::ViewportId::ROOT].commands.iter().any(|c| matches!(c, egui::ViewportCommand::Close)));
        assert!(!out.viewport_output[&child].commands.iter().any(|c| matches!(c, egui::ViewportCommand::Close)));
    }

    #[test]
    fn mnemonic_labels_bracket_the_key() {
        assert_eq!(mnemonic("Don't Save", Key::D), "(D)on't Save");
        assert_eq!(mnemonic("Guardar", Key::S), "Guardar (S)");
        assert_eq!(mnemonic("保存", Key::S), "保存 (S)");
    }

    type Prompted = egui_kittest::Harness<'static, PhotocraftApp>;

    /// The unsaved-changes prompt for Close All over two dirty documents, as `os` draws it.
    fn prompt_on(os: egui::os::OperatingSystem) -> Prompted {
        let mut app = app_with_docs(2);
        make_dirty(&mut app, 0);
        make_dirty(&mut app, 1);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.ctx.set_os(os);
        assert!(intercept(h.state_mut(), "file.closeAll", &Value::Null));
        h.run_steps(2);
        h
    }

    /// The buttons named `labels`, sorted by where they are drawn, left to right.
    fn drawn_order<'a>(h: &Prompted, labels: [&'a str; 3]) -> Vec<&'a str> {
        use egui_kittest::kittest::Queryable;
        let mut v = labels.map(|l| (h.get_by_label(l).rect().left(), l)).to_vec();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v.into_iter().map(|(_, l)| l).collect()
    }

    /// Tab walks `labels` left to right and wraps; Shift+Tab steps back.
    fn tab_walks(h: &mut Prompted, labels: [&str; 3]) {
        use egui_kittest::kittest::Queryable;
        let focused = |h: &Prompted| labels.into_iter().find(|l| h.get_by_label(l).is_focused());
        for want in [labels[0], labels[1], labels[2], labels[0]] {
            h.key_press(Key::Tab);
            h.run_steps(2);
            assert_eq!(focused(h), Some(want));
        }
        h.key_press_modifiers(egui::Modifiers::SHIFT, Key::Tab);
        h.run_steps(2);
        assert_eq!(focused(h), Some(labels[2]));
    }

    fn docs_left(h: &Prompted) -> Option<usize> {
        h.state().discard.as_ref().map(|p| p.docs.len())
    }

    #[test]
    fn windows_and_linux_ask_yes_no_cancel_with_the_default_first() {
        for os in [egui::os::OperatingSystem::Windows, egui::os::OperatingSystem::Nix] {
            let mut h = prompt_on(os);
            let labels = ["(Y)es", "(N)o", "Cancel"];
            assert_eq!(drawn_order(&h, labels), labels, "{os:?}");
            tab_walks(&mut h, labels);
            h.key_press(Key::N);
            h.run_steps(2);
            assert_eq!(docs_left(&h), Some(1), "N answered the first document");
            // Cancel has no letter here: C does nothing, Esc cancels.
            h.key_press(Key::C);
            h.run_steps(2);
            assert_eq!(docs_left(&h), Some(1));
            h.key_press(Key::Escape);
            h.run_steps(2);
            assert!(h.state().discard.is_none());
            assert_eq!(h.state().session.documents().len(), 2, "Cancel closed nothing");
        }
    }

    #[test]
    fn macos_asks_dont_save_cancel_save_with_the_default_last() {
        let mut h = prompt_on(egui::os::OperatingSystem::Mac);
        let labels = ["(D)on't Save", "(C)ancel", "(S)ave"];
        assert_eq!(drawn_order(&h, labels), labels);
        tab_walks(&mut h, labels);
        h.key_press(Key::D);
        h.run_steps(2);
        assert_eq!(docs_left(&h), Some(1), "D answered the first document");
        h.key_press(Key::C);
        h.run_steps(2);
        assert!(h.state().discard.is_none());
        assert_eq!(h.state().session.documents().len(), 2, "Cancel closed nothing");
    }

    #[test]
    fn enter_takes_the_default_answer_or_the_focused_button() {
        use std::{cell::Cell, rc::Rc};
        let mut h = prompt_on(egui::os::OperatingSystem::Windows);
        let asked = Rc::new(Cell::new(false));
        let seen = asked.clone();
        // Backing out of the save dialog keeps the prompt.
        h.state_mut().services.pick_save = Some(Box::new(move |_| {
            seen.set(true);
            None
        }));
        h.key_press(Key::Enter);
        h.run_steps(2);
        assert!(asked.get(), "Enter answered Yes, which saves");
        assert_eq!(docs_left(&h), Some(2));
        // Tab to No: Enter now presses it.
        for _ in 0..2 {
            h.key_press(Key::Tab);
            h.run_steps(2);
        }
        asked.set(false);
        h.key_press(Key::Enter);
        h.run_steps(2);
        assert!(!asked.get());
        assert_eq!(docs_left(&h), Some(1), "Enter pressed the focused No");
    }

    /// One frame with the window's close button pressed; whether the guard cancelled the close.
    fn press_window_close(app: &mut PhotocraftApp) -> bool {
        let mut info = egui::ViewportInfo::default();
        info.events.push(egui::ViewportEvent::Close);
        let mut input = egui::RawInput::default();
        input.viewports.insert(egui::ViewportId::ROOT, info);
        let mut out = egui::Context::default().run_ui(input, |ui| guard_window_close(app, ui.ctx()));
        out.textures_delta.clear();
        let commands = &out.viewport_output[&egui::ViewportId::ROOT].commands;
        let cancelled = commands.iter().any(|c| matches!(c, egui::ViewportCommand::CancelClose));
        assert_eq!(commands.iter().any(|c| matches!(c, egui::ViewportCommand::Focus)), cancelled);
        cancelled
    }

    #[test]
    fn window_close_is_cancelled_while_work_is_unsaved() {
        let mut app = app_with_docs(1);
        make_dirty(&mut app, 0);
        assert!(press_window_close(&mut app));
        assert!(app.discard.is_some());
    }

    #[test]
    fn window_close_goes_through_when_nothing_is_unsaved_or_it_was_confirmed() {
        let mut app = app_with_docs(1);
        assert!(!press_window_close(&mut app));
        make_dirty(&mut app, 0);
        app.allow_close = true;
        assert!(!press_window_close(&mut app));
        assert!(app.discard.is_none());
    }
    #[test]
    fn raw_working_file_close_waits_for_a_successful_save_v117() {
        let mut app = app_with_docs(1);
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        app.session
            .edit("Synthetic Raw", |doc, _| {
                let photocraft_doc::LayerContent::Smart(sm) = &mut doc.layer_mut(layer).unwrap().content else { panic!() };
                sm.smart_filters.push(photocraft_doc::SmartFilter {
                    command: "filter.cameraRaw".into(),
                    params: json!({"exposure":0.5}),
                    blend: photocraft_color::BlendMode::Normal,
                    opacity: 1.,
                    visible: true,
                });
                Ok(())
            })
            .unwrap();
        app.session.active_mut().unwrap().path = Some("synthetic.psd".into());
        app.services.pick_save = Some(Box::new(|_| panic!("unexpected picker")));
        app.services.export = Some(Box::new(|_, _, _| Ok((vec![], vec![]))));
        app.services.write = Some(Box::new(|_, _| Err("write failed".into())));
        assert!(intercept(&mut app, "file.close", &Value::Null));
        let id = app.session.active().unwrap().doc.id;
        let ctx = egui::Context::default();
        apply_answer(&mut app, &ctx, id, Answer::Save);
        assert_eq!(app.session.documents().len(), 1);
        assert!(app.session.active().unwrap().is_dirty());
        assert!(app.discard.is_some() && app.ui.dialogs.is_empty());
        assert!(app.ui.status.contains("write failed"));
        app.services.write = Some(Box::new(|path, _| {
            assert_eq!(path, "synthetic.psd");
            Ok(())
        }));
        apply_answer(&mut app, &ctx, id, Answer::Save);
        assert!(app.discard.is_none() && app.ui.dialogs.is_empty());
        assert!(app.session.documents().is_empty());
    }

    #[test]
    fn saved_companion_receipt_only_satisfies_exact_pending_close_snapshot_v84() {
        for stale in [false, true] {
            let mut app = app_with_docs(1);
            make_dirty(&mut app, 0);
            assert!(intercept(&mut app, "file.close", &Value::Null));
            app.discard.as_mut().unwrap().awaiting_save = true;
            let st = app.session.active().unwrap();
            let snapshot = (st.doc.id, st.revision);
            note_saved_companion(&mut app, snapshot.0, snapshot.1);
            if stale {
                app.run("layer.renameLayer", json!({"name":"Edited after snapshot"})).unwrap();
            }
            let ctx = egui::Context::default();
            PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::StudioLight);
            ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui.ctx())).textures_delta.clear();
            assert_eq!(app.session.documents().len(), usize::from(stale));
        }
    }
}

#[cfg(test)]
#[path = "discard_layout_tests_v80.rs"]
mod layout_tests_v80;

#[cfg(test)]
#[path = "discard_layout_tests_v95.rs"]
mod layout_tests_v95;
