//! Independent desktop dialog windows. An egui modal inside ROOT cannot cover a native canvas.
//! Keep application modality in the owned-window backend, never OS-wide AlwaysOnTop.

use crate::theme::palette::ScrollAreaPaletteExt;
use egui::{ViewportCommand, ViewportId, vec2};
use serde_json::Value;

use crate::{
    PhotocraftApp,
    state::{Dialog, DialogKind},
};

const MODAL: &str = "photocraft-native-dialog-modal-viewport";
const CANVAS_INPUT: &str = "photocraft-native-modal-canvas-input";
const PICKER_TITLE_HEIGHT: f32 = 30.0;
const MIN_FORM_SIZE: egui::Vec2 = vec2(360.0, 200.0);

/// Generic forms retain their existing read-only canvas navigation and the top
/// Color Picker's eyedropper. Document-locking auxiliary operations do not.
fn generic_canvas_input(app: &PhotocraftApp) -> bool {
    !app.ui.dialogs.is_empty()
        && app.discard.is_none()
        && app.session.active_job().is_none()
        && app.camera_raw.is_none()
        && app.distort.liquify.is_none()
        && app.distort.gallery.is_none()
        && app.wide_angle.is_none()
        && app.tiff_options.is_none()
        && app.jpeg_options.is_none()
}

pub(crate) fn canvas_input_allowed(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    generic_canvas_input(app)
        && (ctx.viewport_id() == ViewportId::ROOT
            || app.ui.windows.iter().any(|window| {
                window.open && Some(window.document) == app.session.active_index() && ViewportId::from_hash_of(("docwin", window.id)) == ctx.viewport_id()
            }))
}

fn viewport(id: u64) -> ViewportId {
    ViewportId::from_hash_of(("photocraft-dialog", id))
}

fn opened(id: u64) -> egui::Id {
    egui::Id::new(("photocraft-dialog-opened", id))
}

/// The last dialog is the active modal. Siblings remain alive, preserving size and form state.
pub(crate) fn native_dialog_viewport(app: &PhotocraftApp) -> Option<ViewportId> {
    if app.jpeg_options.is_some() {
        return Some(auxiliary_viewport(crate::jpeg_options_ui::DIALOG_KEY));
    }
    app.ui.dialogs.last().map(|dialog| viewport(dialog.id)).or_else(|| auxiliary_keys(app).last().map(|key| auxiliary_viewport(key)))
}

pub(crate) fn auxiliary_viewport(key: &str) -> ViewportId {
    ViewportId::from_hash_of(("photocraft-auxiliary-dialog", key))
}

fn auxiliary_keys(app: &PhotocraftApp) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some((kind, _)) = &app.ui.shell.dialog {
        keys.push(format!("shell-{kind}"));
    }
    if let Some((kind, _)) = &app.ui.analysis.dialog {
        keys.push(format!("analysis-{kind}"));
    }
    if app.ui.type_panels.spell.is_some() {
        keys.push("check-spelling".to_string());
    }
    for (key, active) in [
        (crate::camera_raw_ui::NATIVE_DIALOG_KEY, app.camera_raw.as_ref().is_some_and(|dialog| !dialog.native_closing())),
        ("liquify", app.distort.liquify.is_some()),
        ("filter-gallery", app.distort.gallery.is_some()),
        ("wide-angle", app.wide_angle.is_some()),
        ("tiff-options", app.tiff_options.is_some()),
        (crate::jpeg_options_ui::DIALOG_KEY, app.jpeg_options.is_some()),
        ("gpu-fallback", app.ui.gpu_fallback_notice.is_some()),
        ("job-progress", app.session.active_job().is_some_and(|job| job.elapsed_ms >= crate::jobs_ui::DIALOG_DELAY_MS)),
    ] {
        if active {
            keys.push(key.to_string());
        }
    }
    keys
}

/// Existing automation selector for the New Brush Preset naming window.
pub(crate) fn native_preset_viewport(app: &PhotocraftApp) -> Option<ViewportId> {
    app.ui.dialogs.iter().rev().find(|dialog| crate::brush_panel::owns_new_preset(&dialog.fields)).map(|dialog| viewport(dialog.id))
}

pub(super) fn cleanup(app: &PhotocraftApp, ctx: &egui::Context) {
    let active = (!ctx.embed_viewports() && (app.discard.is_none() || app.jpeg_options.is_some())).then(|| native_dialog_viewport(app)).flatten();
    let auxiliary = auxiliary_keys(app);
    let canvas_input = active.filter(|_| generic_canvas_input(app)).map(|modal| {
        let mut allowed = vec![ViewportId::ROOT];
        allowed.extend(
            app.ui
                .windows
                .iter()
                .filter(|window| window.open && Some(window.document) == app.session.active_index())
                .map(|window| ViewportId::from_hash_of(("docwin", window.id))),
        );
        (modal, allowed)
    });
    ctx.data_mut(|data| {
        if let Some(allowed) = canvas_input {
            data.insert_temp(egui::Id::new(CANVAS_INPUT), allowed);
        } else {
            data.remove::<(ViewportId, Vec<ViewportId>)>(egui::Id::new(CANVAS_INPUT));
        }
        let previous: Vec<String> = data.get_temp(egui::Id::new("native-auxiliary-dialogs")).unwrap_or_default();
        for key in previous.iter().filter(|key| !auxiliary.contains(key)) {
            data.remove::<bool>(egui::Id::new(("auxiliary-dialog-opened", key)));
        }
        data.insert_temp(egui::Id::new("native-auxiliary-dialogs"), auxiliary);
        if let Some(id) = active {
            data.insert_temp(egui::Id::new(MODAL), id);
        } else {
            data.remove::<ViewportId>(egui::Id::new(MODAL));
        }
    });
}

fn content_width(dialog: &Dialog) -> f32 {
    if let Some(width) = crate::file_ui::dialog_width(&dialog.fields).or_else(|| crate::prefs_ui::width(&dialog.fields)) {
        width
    } else if dialog.kind == DialogKind::NewDocument {
        800.0
    } else if super::has_contacts(dialog) {
        1040.0
    } else if dialog.kind == DialogKind::About {
        700.0
    } else if crate::color_picker_ui::owns(&dialog.fields) {
        528.0
    } else if dialog.kind == DialogKind::LayerStyle || dialog.fields.contains_key("__export") {
        600.0
    } else {
        440.0
    }
}

fn preferred_content_size(dialog: &Dialog) -> egui::Vec2 {
    if crate::prefs_ui::is_preferences(&dialog.fields) {
        // 18 navigation rows plus content, native frame margins and the fixed
        // footer fit without turning the entire form into a scroll canvas.
        return vec2(920.0, 740.0);
    }
    if super::has_contacts(dialog) {
        return vec2(1072.0, 860.0);
    }
    if dialog.fields.get("__prefsui").and_then(Value::as_str) == Some("shortcuts") {
        return vec2(920.0, 740.0);
    }
    if crate::color_picker_ui::owns(&dialog.fields) {
        let extras = usize::from(dialog.fields.get("__allowAlpha").and_then(Value::as_bool) == Some(true))
            + usize::from(dialog.fields.get("__allowNone").and_then(Value::as_bool) == Some(true));
        return vec2(560.0, 368.0 + extras as f32 * 28.0);
    }
    let height = if crate::brush_panel::owns_new_preset(&dialog.fields) {
        220.0
    } else if dialog.kind == DialogKind::NewDocument {
        680.0
    } else if crate::prefs_ui::owns(&dialog.fields) || dialog.kind == DialogKind::LayerStyle || dialog.kind == DialogKind::About {
        560.0
    } else {
        440.0
    };
    vec2(content_width(dialog) + 32.0, height)
}

/// Keep the form's established content room when scrollbars have a stable gutter.
/// About has only vertical scrolling; every other native form reserves both axes.
/// Derive this from the actual theme style, including user/UI-scale overrides,
/// rather than assuming a scrollbar thickness in pixels.
fn form_scrollbar_gutter(ctx: &egui::Context, dialog: &Dialog) -> egui::Vec2 {
    if cfg!(target_arch = "wasm32") {
        return egui::Vec2::ZERO;
    }
    let width = ctx.global_style().spacing.scroll.allocated_width();
    vec2(width, if dialog.kind == DialogKind::About { 0.0 } else { width })
}

fn initial_size(ctx: &egui::Context, dialog: &Dialog) -> egui::Vec2 {
    preferred_content_size(dialog) + form_scrollbar_gutter(ctx, dialog)
}

fn builder(ctx: &egui::Context, dialog: &Dialog, title: &str, first: bool) -> egui::ViewportBuilder {
    if let Some(size) = message_size(ctx, dialog) {
        // Measure before creating the HWND to avoid a large first-frame flash.
        return window_builder(ctx, title, size, first).with_min_inner_size(vec2(300.0, 112.0).min(size));
    }
    if crate::prefs_ui::owns(&dialog.fields) || super::has_contacts(dialog) {
        preferences_window_builder(ctx, title, initial_size(ctx, dialog), first)
    } else if picker_uses_client_title(dialog) {
        // The Windows non-client caption repaints on activation independently of egui.
        // Use the same frameless owned surface as other floating Windows UI and keep the
        // picker's existing room for fields and buttons below its in-client title.
        window_builder(ctx, title, initial_size(ctx, dialog) + vec2(0.0, PICKER_TITLE_HEIGHT), first)
            .with_min_inner_size(picker_min_size())
            .with_decorations(false)
            .with_transparent(true)
            .with_has_shadow(false)
    } else {
        window_builder(ctx, title, initial_size(ctx, dialog), first)
    }
}

fn picker_uses_client_title(dialog: &Dialog) -> bool {
    cfg!(target_os = "windows") && crate::color_picker_ui::owns(&dialog.fields)
}

fn picker_min_size() -> egui::Vec2 {
    MIN_FORM_SIZE + vec2(0.0, PICKER_TITLE_HEIGHT)
}

/// Message-only dialogs size to translated text, including the warning icon,
/// native margins and fixed action row. Regular forms keep their own geometry.
fn message_size(ctx: &egui::Context, dialog: &Dialog) -> Option<egui::Vec2> {
    let warning = crate::rasterize_prompt::owns(&dialog.fields);
    if dialog.kind != DialogKind::Error && !warning {
        return None;
    }
    let text =
        dialog.fields.get("message").and_then(Value::as_str).unwrap_or(if warning { "This layer must be rasterized before proceeding." } else { "Error" });
    let text = tl!(text);
    let font = if warning { egui::FontId::proportional(13.0) } else { egui::TextStyle::Body.resolve(&ctx.style_of(ctx.theme())) };
    let icon_width = if warning { 44.0 } else { 0.0 };
    let owner = ctx
        .input(|input| input.raw.viewports.get(&ViewportId::ROOT).and_then(|root| root.inner_rect))
        .filter(|rect| rect.is_finite() && rect.is_positive())
        .or_else(|| Some(ctx.content_rect()).filter(|rect| rect.is_finite() && rect.is_positive()));
    let max_width = owner.map_or(560.0, |rect| (rect.width() - 40.0).clamp(300.0, 560.0));
    let (width, height) = ctx.fonts_mut(|fonts| {
        let natural = fonts.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE).size().x;
        let width = (natural + icon_width + 32.0).clamp(300.0, max_width);
        let wrapped = fonts.layout(text.to_owned(), font, egui::Color32::WHITE, width - icon_width - 32.0).size();
        (width, wrapped.y.max(if warning { 32.0 } else { 20.0 }))
    });
    let measured = vec2(width.ceil(), (height + 40.0 + super::footer_height(dialog, width)).ceil());
    Some(owner.map_or(measured, |rect| measured.min((rect.size() - vec2(40.0, 70.0)).max(vec2(300.0, 112.0)))))
}

fn preferences_window_builder(ctx: &egui::Context, title: &str, preferred: egui::Vec2, first: bool) -> egui::ViewportBuilder {
    let root = ctx.input(|input| input.raw.viewports.get(&ViewportId::ROOT).cloned()).unwrap_or_default();
    let inner = root.inner_rect.filter(|rect| rect.is_finite() && rect.is_positive());
    // ViewportInfo dimensions are already logical egui points, including the
    // user's display/UI scale. Do not multiply them by native_pixels_per_point.
    // egui exposes monitor size, not the true monitor work-area origin. Reserve
    // taskbar/caption room for size; retain the established parent-centred
    // placement instead of guessing the origin of differently sized monitors.
    let monitor = root.monitor_size.filter(|size| size.is_finite() && size.x > 0.0 && size.y > 0.0);
    let room = monitor.map_or_else(
        || inner.map_or(vec2(1000.0, 760.0), |rect| (rect.size() - vec2(40.0, 70.0)).max(vec2(360.0, 200.0))),
        |size| (size - vec2(32.0, crate::work_area::TASKBAR + 64.0)).max(vec2(1.0, 1.0)),
    );
    let size = preferred.min(room);
    let mut window = window_builder(ctx, title, preferred, false).with_min_inner_size(vec2(360.0, 200.0).min(room));
    if first {
        window = window.with_inner_size(size).with_active(true);
        if let Some(rect) = inner {
            let mut position = rect.center() - size * 0.5;
            // A maximized/fullscreen owner is already a known work-area rect.
            // For ordinary windows its rect is not the monitor's work area.
            if root.maximized == Some(true) || root.fullscreen == Some(true) {
                position = position.clamp(rect.min, (rect.max - size).max(rect.min));
            } else if let Some(monitor) = monitor
                && egui::Rect::from_min_size(egui::Pos2::ZERO, monitor).contains_rect(rect)
            {
                // Single-monitor fallback: this rectangle fits the assumed
                // zero-origin monitor. This is not an origin guarantee on
                // heterogeneous multi-monitor desktops. Reserve caption room.
                let min = egui::pos2(16.0, 32.0);
                let max = egui::pos2(monitor.x - 16.0 - size.x, monitor.y - crate::work_area::TASKBAR - 32.0 - size.y).max(min);
                position = position.clamp(min, max);
            }
            window = window.with_position(position);
        }
    }
    window
}

fn window_builder(ctx: &egui::Context, title: &str, initial_size: egui::Vec2, first: bool) -> egui::ViewportBuilder {
    let root = ctx.input(|i| i.raw.viewports.get(&ViewportId::ROOT).and_then(|v| v.inner_rect)).filter(|rect| rect.is_finite() && rect.is_positive());
    let size = initial_size.min(root.map_or(vec2(1000.0, 760.0), |rect| (rect.size() - vec2(40.0, 70.0)).max(MIN_FORM_SIZE)));
    let mut builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_min_inner_size(MIN_FORM_SIZE)
        .with_resizable(true)
        .with_minimize_button(false)
        .with_maximize_button(false)
        .with_taskbar(false)
        .with_window_level(egui::WindowLevel::Normal);
    if first {
        // Send initial geometry once. Resizing/restoring the main window must not reset
        // the user's dialog size or move it to another display.
        builder = builder.with_inner_size(size);
        if let Some(rect) = root {
            builder = builder.with_position(rect.center() - size * 0.5);
        }
        builder = builder.with_active(true);
    }
    builder
}

fn sync_native_chrome(ui: &egui::Ui, first: bool) {
    let ctx = ui.ctx();
    let dark = ui.visuals().dark_mode;
    let key = egui::Id::new(("dialog-native-chrome-theme", ctx.viewport_id()));
    if first || ctx.data(|data| data.get_temp::<bool>(key)) != Some(dark) {
        ctx.send_viewport_cmd(ViewportCommand::SetTheme(if dark { egui::SystemTheme::Dark } else { egui::SystemTheme::Light }));
        ctx.data_mut(|data| data.insert_temp(key, dark));
    }
}

fn picker_title(ui: &mut egui::Ui, title: &str, top: bool) -> bool {
    let t = crate::theme::Tokens::get(ui.ctx());
    let fill = crate::theme::palette::override_color(ui.ctx(), photocraft_engine::prefs::palette::ColorSlot::FloatingTitle).unwrap_or(t.card);
    egui::Panel::top("native-picker-title")
        .exact_size(PICKER_TITLE_HEIGHT)
        .frame(egui::Frame::NONE.fill(fill))
        .show(ui, |ui| {
            let bar = ui.max_rect();
            let close = egui::Rect::from_center_size(bar.right_center() - vec2(26.0, 0.0), vec2(28.0, 20.0));
            let drag = egui::Rect::from_min_max(bar.min + vec2(12.0, 5.0), egui::pos2(close.left(), bar.bottom()));
            let text = egui::Rect::from_min_max(bar.min + vec2(10.0, 0.0), drag.right_bottom());
            ui.painter().hline(bar.x_range(), bar.bottom(), egui::Stroke::new(1.0, t.separator));
            ui.painter().with_clip_rect(text).text(text.left_center(), egui::Align2::LEFT_CENTER, title, crate::theme::medium(12.0), t.text);
            if !top {
                crate::icons::paint(ui, close, "x", 13.0, t.icon);
                return false;
            }
            let title = ui.interact(drag, ui.id().with("drag"), egui::Sense::click_and_drag());
            if title.drag_started_by(egui::PointerButton::Primary) {
                ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
            }
            let button = ui.interact(close, ui.id().with("close"), egui::Sense::click());
            crate::icons::button_chrome(ui, close, false, button.hovered());
            crate::icons::paint(ui, close, "x", 13.0, t.icon);
            button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Close")));
            button.on_hover_text(tl!("Close")).clicked()
        })
        .inner
}

fn form_body(ui: &mut egui::Ui, app: &mut PhotocraftApp, dialog: &Dialog, fields: &mut serde_json::Map<String, Value>) -> (f32, f32, f32) {
    let output = form_body_scroll(ui, app, dialog, fields);
    (output.content_size.x, output.inner_rect.width(), output.state.offset.x)
}

fn form_body_scroll(
    ui: &mut egui::Ui,
    app: &mut PhotocraftApp,
    dialog: &Dialog,
    fields: &mut serde_json::Map<String, Value>,
) -> egui::scroll_area::ScrollAreaOutput<()> {
    let about = dialog.kind == DialogKind::About;
    let preferences = crate::prefs_ui::is_preferences(fields);
    let width = content_width(dialog);
    let scroll = if about { egui::ScrollArea::vertical() } else { egui::ScrollArea::both() };
    scroll.id_salt(("native-dialog-body", dialog.id)).auto_shrink([false, false]).show_palette(ui, |ui| {
        // About uses wrapping columns and should fit after reserving the
        // vertical scrollbar. Preferences uses its finite content canvas rather
        // than forcing the pre-scroll viewport width back into the post-scroll
        // content, which creates a horizontal bar when the vertical bar appears.
        // A genuinely narrow resized Preferences still scrolls to its controls.
        // Use the post-scrollbar width for responsive forms. Reusing the outer
        // viewport width creates a horizontal bar as soon as a vertical one appears.
        // Preferences keeps its genuine two-column minimum on small windows.
        ui.set_width(if preferences { width } else { ui.available_width() });
        super::body(app, ui, dialog, fields);
    })
}

pub(super) fn show(app: &mut PhotocraftApp, ctx: &egui::Context, dialog: &Dialog) {
    if app.discard.is_some() {
        ctx.data_mut(|data| data.remove::<bool>(opened(dialog.id)));
        return;
    }
    let id = viewport(dialog.id);
    let top = native_dialog_viewport(app) == Some(id);
    let first = !ctx.data(|data| data.get_temp::<bool>(opened(dialog.id))).unwrap_or(false);
    // Block already-drawn root widgets too. Native siblings are disabled by owned_window.rs.
    egui::Modal::new(egui::Id::new(("native-dialog-root-blocker", dialog.id)))
        .frame(egui::Frame::NONE)
        .backdrop_color(egui::Color32::TRANSPARENT)
        .show(ctx, |_| {});
    let lang = if crate::prefs_ui::is_preferences(&dialog.fields) {
        crate::i18n::Lang::from_pref(
            dialog.fields.get("values").and_then(|value| value.pointer("/interface/language")).and_then(Value::as_str).unwrap_or("auto"),
        )
    } else {
        crate::i18n::current()
    };
    let _language = crate::i18n::language_scope(lang);
    let title = super::display_title(dialog);
    let client_title = picker_uses_client_title(dialog);
    let mut fields = dialog.fields.clone();
    let mut window = builder(ctx, dialog, &title, first);
    if first && !top {
        window = window.with_active(false);
    }
    let closed = ctx.show_viewport_immediate(id, window, |ui, _| {
        if !client_title {
            sync_native_chrome(ui, first);
        }
        app.collect_screenshots(ui.ctx());
        if first && top {
            ui.ctx().send_viewport_cmd(ViewportCommand::Visible(true));
            ui.ctx().send_viewport_cmd(ViewportCommand::Focus);
        }
        let title_close = client_title && picker_title(ui, &title, top);
        // Fixed footer stays reachable even when resizing or on a small / high-DPI display.
        // Reserve it first, but process its widgets after the form: Enter must commit the
        // text edit's latest value, and Apply must see changes from this same frame.
        let mut footer = egui::Panel::bottom("dialog-actions")
            .exact_size(super::footer_height(dialog, ui.available_width()))
            .frame(egui::Frame::central_panel(ui.style()).inner_margin(16.0))
            .show(ui, |ui| ui.new_child(egui::UiBuilder::new().id_salt("dialog-actions-content").max_rect(ui.max_rect())))
            .inner;
        egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style()).inner_margin(16.0)).show(ui, |ui| {
            ui.add_enabled_ui(top, |ui| {
                form_body(ui, app, dialog, &mut fields);
            });
        });
        let (mut outcome, apply_requested) = footer.add_enabled_ui(top, |ui| super::buttons(app, ui, dialog, &fields)).inner;
        if client_title && top {
            crate::titlebar::resize_zones_for_frame(ui, &mut app.services.contact_screen_pos, &mut app.services.screen_cursor_pos, 0.0, picker_min_size());
        }
        if top
            && (title_close
                || ui.input(|i| i.viewport().close_requested())
                || (!egui::Popup::is_any_open(ui.ctx()) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))))
        {
            outcome = Some(false);
        }
        if top {
            if generic_canvas_input(app) && !ui.ctx().text_edit_focused() {
                crate::shortcuts::nav_keys(app, ui.ctx());
            }
            super::finish(app, ui.ctx(), dialog, fields.clone(), outcome, apply_requested);
        }
        let closed = !app.ui.dialogs.iter().any(|current| current.id == dialog.id);
        if closed {
            // The backend re-enables the owner before hiding this focused HWND.
            cleanup(app, ui.ctx());
            ui.ctx().send_viewport_cmd(ViewportCommand::Visible(false));
        }
        closed
    });
    ctx.data_mut(|data| {
        if closed {
            data.remove::<bool>(opened(dialog.id));
        } else {
            data.insert_temp(opened(dialog.id), true);
        }
    });
    if closed {
        ctx.request_repaint();
    }
}

/// Workspace / view forms retain their existing ShellUi automation and command semantics.
pub(crate) fn show_shell(app: &mut PhotocraftApp, ctx: &egui::Context, kind: &str, title: &str, contents: impl FnMut(&mut egui::Ui)) -> bool {
    show_auxiliary(app, ctx, &format!("shell-{kind}"), title, vec2(480.0, if kind == "extrasOptions" { 520.0 } else { 360.0 }), contents)
}

pub(crate) fn show_auxiliary(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    key: &str,
    title: &str,
    size: egui::Vec2,
    mut contents: impl FnMut(&mut egui::Ui),
) -> bool {
    show_auxiliary_viewport(app, ctx, key, title, size, |_, ui| {
        egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style()).inner_margin(16.0)).show(ui, |ui| {
            egui::ScrollArea::both().auto_shrink([false, false]).show_palette(ui, |ui| {
                ui.set_width(ui.available_width());
                contents(ui);
            });
        });
    })
}

/// Host for full-window editors that draw their own content and need mutable app state.
pub(crate) fn show_auxiliary_viewport(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    key: &str,
    title: &str,
    size: egui::Vec2,
    mut contents: impl FnMut(&mut PhotocraftApp, &mut egui::Ui),
) -> bool {
    if app.discard.is_some() && key != crate::jpeg_options_ui::DIALOG_KEY {
        ctx.data_mut(|data| data.remove::<bool>(egui::Id::new(("auxiliary-dialog-opened", key))));
        return false;
    }
    cleanup(app, ctx);
    let id = auxiliary_viewport(key);
    let opened = egui::Id::new(("auxiliary-dialog-opened", key));
    let first = !ctx.data(|data| data.get_temp::<bool>(opened)).unwrap_or(false);
    let top = native_dialog_viewport(app) == Some(id);
    egui::Modal::new(egui::Id::new(("native-auxiliary-dialog-root-blocker", key)))
        .frame(egui::Frame::NONE)
        .backdrop_color(egui::Color32::TRANSPARENT)
        .show(ctx, |_| {});
    let mut window = window_builder(ctx, title, size, first);
    if first && !top {
        window = window.with_active(false);
    }
    let closed = ctx.show_viewport_immediate(id, window, |ui, _| {
        sync_native_chrome(ui, first);
        app.collect_screenshots(ui.ctx());
        if first && top {
            ui.ctx().send_viewport_cmd(ViewportCommand::Focus);
        }
        ui.add_enabled_ui(top, |ui| contents(app, ui));
        finish_auxiliary(app, ui.ctx(), key);
        top && (ui.input(|input| input.viewport().close_requested())
            || (!egui::Popup::is_any_open(ui.ctx()) && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))))
    });
    ctx.data_mut(|data| {
        if auxiliary_keys(app).iter().any(|open| open == key) {
            data.insert_temp(opened, true);
        } else {
            data.remove::<bool>(opened);
        }
    });
    closed
}

pub(crate) fn finish_shell(app: &PhotocraftApp, ctx: &egui::Context, kind: &str) {
    finish_auxiliary(app, ctx, &format!("shell-{kind}"));
}

pub(crate) fn finish_auxiliary(app: &PhotocraftApp, ctx: &egui::Context, key: &str) {
    if !ctx.embed_viewports() && !auxiliary_keys(app).iter().any(|open| open == key) {
        let viewport = auxiliary_viewport(key);
        let skip_present = key == crate::camera_raw_ui::NATIVE_DIALOG_KEY && ctx.viewport_id() == viewport;
        cleanup(app, ctx);
        ctx.data_mut(|data| {
            data.remove::<bool>(egui::Id::new(("auxiliary-dialog-opened", key)));
            if skip_present {
                data.insert_temp(egui::Id::new(("photocraft-native-skip-present-once", viewport)), true);
            }
        });
        ctx.send_viewport_cmd_to(viewport, ViewportCommand::Visible(false));
        ctx.request_repaint();
    }
}

#[cfg(test)]
#[path = "native_tests_v70.rs"]
mod preferences_geometry_tests;

#[cfg(test)]
#[path = "contacts_tests_v72.rs"]
mod contacts_tests_v72;

#[cfg(test)]
#[path = "scroll_fit_tests_v73.rs"]
mod scroll_fit_tests_v73;

#[cfg(test)]
#[path = "message_layout_tests_v75.rs"]
mod message_layout_tests_v75;

#[cfg(test)]
mod tests {
    use super::*;

    struct AboutBodyFixture {
        app: PhotocraftApp,
        dialog: Dialog,
        fields: serde_json::Map<String, Value>,
        geometry: (f32, f32, f32),
    }
    impl eframe::App for AboutBodyFixture {
        fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
            egui::Panel::bottom("native-about-qa-footer").exact_size(64.0).show(ui, |_| {});
            egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style()).inner_margin(16.0)).show(ui, |ui| {
                self.geometry = form_body(ui, &mut self.app, &self.dialog, &mut self.fields);
            });
        }
    }

    #[test]
    fn about_native_body_fits_its_viewport_without_horizontal_scroll_in_all_themes() {
        for theme in crate::theme::ThemeKind::ALL {
            for width in [360.0, 732.0] {
                let mut h = egui_kittest::Harness::builder().with_size(vec2(width, 560.0)).with_max_steps(64).build_eframe(move |cc| {
                    PhotocraftApp::setup_context(&cc.egui_ctx, theme);
                    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                    app.ui.open_dialog(DialogKind::About, serde_json::Map::new());
                    let dialog = app.ui.dialogs.last().unwrap().clone();
                    AboutBodyFixture { app, dialog, fields: serde_json::Map::new(), geometry: (0.0, 0.0, 0.0) }
                });
                h.run_steps(8);
                let (content, viewport, horizontal_offset) = h.state().geometry;
                assert!(content <= viewport + 1.0, "{theme:?} {width}: source columns overflow by {}", content - viewport);
                assert_eq!(horizontal_offset, 0.0);
            }
        }
    }

    #[test]
    fn native_canvas_allowlist_is_bound_to_generic_modal_and_active_document() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for _ in 0..2 {
            app.run("file.new", serde_json::json!({"width": 20, "height": 20})).unwrap();
        }
        let active = crate::view_cmds::float_window(&mut app, 1, 1);
        let inactive = crate::view_cmds::float_window(&mut app, 0, 0);
        app.session.set_active(1);
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let dialog = crate::color_picker_ui::open(&mut app, "foreground");
        cleanup(&app, &ctx);
        let grant = ctx.data(|data| data.get_temp::<(ViewportId, Vec<ViewportId>)>(egui::Id::new(CANVAS_INPUT))).unwrap();
        assert_eq!(grant.0, viewport(dialog));
        assert!(grant.1.contains(&ViewportId::ROOT));
        assert!(grant.1.contains(&ViewportId::from_hash_of(("docwin", active))));
        assert!(!grant.1.contains(&ViewportId::from_hash_of(("docwin", inactive))));
        app.run("edit.fill", serde_json::json!({"color":"#ff0000"})).unwrap();
        assert!(crate::discard_ui::intercept(&mut app, "file.close", &serde_json::json!({})));
        cleanup(&app, &ctx);
        assert!(!canvas_input_allowed(&app, &ctx));
        assert!(ctx.data(|data| data.get_temp::<(ViewportId, Vec<ViewportId>)>(egui::Id::new(CANVAS_INPUT))).is_none());
        app.discard = None;
        app.ui.close_dialog(dialog);
        app.ui.shell.dialog = Some(("newWorkspace".into(), serde_json::Map::new()));
        cleanup(&app, &ctx);
        assert!(native_dialog_viewport(&app).is_some());
        assert!(ctx.data(|data| data.get_temp::<(ViewportId, Vec<ViewportId>)>(egui::Id::new(CANVAS_INPUT))).is_none());
    }

    #[test]
    fn settings_window_is_normal_resizable_and_only_activates_when_opened() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let id = crate::prefs_ui::open_preferences(&mut app, "general");
        let dialog = app.ui.dialog_mut(id).unwrap().clone();
        let first = builder(&ctx, &dialog, "Preferences", true);
        assert_eq!(first.window_level, Some(egui::WindowLevel::Normal));
        assert_eq!(first.active, Some(true));
        assert_eq!(first.taskbar, Some(false));
        assert_eq!(first.resizable, Some(true));
        assert!(first.inner_size.unwrap().x >= 780.0);
        let later = builder(&ctx, &dialog, "Preferences", false);
        assert!(later.active.is_none() && later.position.is_none() && later.inner_size.is_none());
    }

    #[test]
    fn last_dialog_has_native_priority_and_closing_restores_previous_marker() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let settings = crate::prefs_ui::open_preferences(&mut app, "general");
        let error = app.ui.open_dialog(DialogKind::Error, serde_json::Map::new());
        cleanup(&app, &ctx);
        assert_eq!(native_dialog_viewport(&app), Some(viewport(error)));
        assert_eq!(ctx.data(|data| data.get_temp::<ViewportId>(egui::Id::new(MODAL))), Some(viewport(error)));
        app.ui.close_dialog(error);
        cleanup(&app, &ctx);
        assert_eq!(ctx.data(|data| data.get_temp::<ViewportId>(egui::Id::new(MODAL))), Some(viewport(settings)));
        app.ui.close_dialog(settings);
        cleanup(&app, &ctx);
        assert!(ctx.data(|data| data.get_temp::<ViewportId>(egui::Id::new(MODAL))).is_none());
    }

    #[test]
    fn cancelling_clears_the_preset_modal_marker_without_saving() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let brush = app.session.tools.brush.clone();
        let id = crate::brush_panel::open_new_preset(&mut app, &brush);
        let modal = native_preset_viewport(&app).unwrap();
        cleanup(&app, &ctx);
        assert_eq!(ctx.data(|data| data.get_temp::<ViewportId>(egui::Id::new(MODAL))), Some(modal));
        app.ui.close_dialog(id);
        cleanup(&app, &ctx);
        assert!(ctx.data(|data| data.get_temp::<ViewportId>(egui::Id::new(MODAL))).is_none());
        assert!(!app.session.journal.iter().any(|(id, _)| id == "brush.presets.save"));
    }

    #[test]
    fn embedded_dialogs_never_set_a_native_modal_marker() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(true);
        crate::prefs_ui::open_preferences(&mut app, "general");
        cleanup(&app, &ctx);
        assert!(ctx.data(|data| data.get_temp::<ViewportId>(egui::Id::new(MODAL))).is_none());
    }

    #[test]
    fn general_dialogs_override_auxiliary_forms_and_automation_close_resets_activation() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        app.ui.shell.dialog = Some(("newWorkspace".into(), serde_json::Map::new()));
        let auxiliary = auxiliary_viewport("shell-newWorkspace");
        cleanup(&app, &ctx);
        assert_eq!(native_dialog_viewport(&app), Some(auxiliary));
        let opened = egui::Id::new(("auxiliary-dialog-opened", "shell-newWorkspace"));
        ctx.data_mut(|data| data.insert_temp(opened, true));

        let error = app.ui.open_dialog(DialogKind::Error, serde_json::Map::new());
        cleanup(&app, &ctx);
        assert_eq!(native_dialog_viewport(&app), Some(viewport(error)));
        app.ui.close_dialog(error);
        cleanup(&app, &ctx);
        assert_eq!(native_dialog_viewport(&app), Some(auxiliary));

        // The control API can close a form between UI frames, bypassing its button path.
        app.ui.shell.dialog = None;
        cleanup(&app, &ctx);
        assert!(ctx.data(|data| data.get_temp::<bool>(opened)).is_none());
        assert!(native_dialog_viewport(&app).is_none());
        app.ui.shell.dialog = Some(("newWorkspace".into(), serde_json::Map::new()));
        cleanup(&app, &ctx);
        assert_eq!(native_dialog_viewport(&app), Some(auxiliary));
        assert!(ctx.data(|data| data.get_temp::<bool>(opened)).is_none(), "reopening must activate once again");
    }

    #[test]
    fn native_settings_apply_keeps_window_and_cancel_preserves_last_applied_values() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let id = crate::prefs_ui::open_preferences(&mut app, "interface");
        let dialog = app.ui.dialog_mut(id).unwrap().clone();
        let mut applied = dialog.fields.clone();
        *applied.get_mut("values").unwrap().pointer_mut("/interface/language").unwrap() = serde_json::json!("en");
        super::super::finish(&mut app, &ctx, &dialog, applied, None, true);
        assert!(app.ui.dialog_mut(id).is_some(), "Apply must keep the existing dialog and native HWND alive");
        assert_eq!(app.session.prefs_value().pointer("/interface/language"), Some(&serde_json::json!("en")));

        let mut cancelled = app.ui.dialog_mut(id).unwrap().fields.clone();
        *cancelled.get_mut("values").unwrap().pointer_mut("/interface/language").unwrap() = serde_json::json!("ja");
        super::super::finish(&mut app, &ctx, &dialog, cancelled, Some(false), false);
        assert!(app.ui.dialog_mut(id).is_none());
        assert_eq!(app.session.prefs_value().pointer("/interface/language"), Some(&serde_json::json!("en")));
    }
}

#[cfg(test)]
mod shared_picker_geometry_v74 {
    use super::*;
    #[test]
    fn color_picker_is_a_separate_resizable_normal_window_and_does_not_reset_moved_geometry_v74() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", serde_json::json!({"width":32,"height":32})).unwrap();
        let parent = crate::fill_ui::open(&mut app);
        let id = crate::color_picker_ui::open(&mut app, "foreground");
        let dialog = app.ui.dialogs.last().unwrap().clone();
        assert_eq!(native_dialog_viewport(&app), Some(viewport(id)));
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.viewports.get_mut(&ViewportId::ROOT).unwrap().inner_rect = Some(egui::Rect::from_min_size(egui::pos2(50.0, 50.0), vec2(1400.0, 900.0)));
        let mut output = ctx.run_ui(input, |ui| {
            let first = builder(ui.ctx(), &dialog, "Color Picker", true);
            let title_height = if cfg!(target_os = "windows") { PICKER_TITLE_HEIGHT } else { 0.0 };
            assert_eq!(first.inner_size, Some(vec2(560.0, 368.0 + title_height) + form_scrollbar_gutter(ui.ctx(), &dialog)));
            assert_eq!(first.resizable, Some(true));
            assert_eq!(first.window_level, Some(egui::WindowLevel::Normal));
            assert_eq!(first.decorations, cfg!(target_os = "windows").then_some(false));
            assert_eq!(first.transparent, cfg!(target_os = "windows").then_some(true));
            let moved = builder(ui.ctx(), &dialog, "Color Picker", false);
            assert!(moved.inner_size.is_none() && moved.position.is_none(), "follow-up paints must not reset the OS-owned size/position");
        });
        output.textures_delta.clear();
        app.ui.close_dialog(id);
        assert_eq!(native_dialog_viewport(&app), Some(viewport(parent)), "parent remains alive and regains input priority");
    }

    #[test]
    fn picker_title_paints_the_same_on_press_and_routes_drag_and_close_separately() {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::default());
        let title = "Color Picker";
        let title_press = egui::pos2(75.0, 15.0);
        let close_press = egui::pos2(474.0, 15.0);
        let pointer = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let frame = |events, top| {
            let mut closed = false;
            let out = ctx.run_ui(
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(500.0, 300.0))), events, ..Default::default() },
                |ui| closed = picker_title(ui, title, top),
            );
            (out, closed)
        };
        let (idle, _) = frame(vec![egui::Event::PointerMoved(title_press)], true);
        let (held, closed) = frame(vec![pointer(title_press, true)], true);
        assert!(!closed);
        assert_eq!(idle.shapes, held.shapes);
        let (dragged, _) = frame(vec![egui::Event::PointerMoved(title_press + vec2(20.0, 0.0))], true);
        assert!(dragged.viewport_output[&ViewportId::ROOT].commands.contains(&ViewportCommand::StartDrag));
        frame(vec![pointer(title_press + vec2(20.0, 0.0), false)], true);
        frame(vec![egui::Event::PointerMoved(close_press)], true);
        frame(vec![pointer(close_press, true)], true);
        let (released, closed) = frame(vec![pointer(close_press, false)], true);
        assert!(closed);
        assert!(!released.viewport_output[&ViewportId::ROOT].commands.contains(&ViewportCommand::StartDrag));
        frame(vec![pointer(close_press, true)], false);
        assert!(!frame(vec![pointer(close_press, false)], false).1);
    }
}
