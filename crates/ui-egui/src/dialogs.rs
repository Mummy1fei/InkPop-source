//! Dialogs rendered from `UiState::dialogs`. Field values live in the dialog data, so automation can
//! set them (`ui.dialog.set`) and confirm (`ui.dialog.confirm`) exactly like a user.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::{Dialog, DialogKind};
use crate::widgets::{ButtonRole, DialogButton, dialog_buttons};

mod native;
pub(crate) use native::{auxiliary_viewport, finish_auxiliary, finish_shell, show_auxiliary, show_auxiliary_viewport, show_shell};
pub(crate) use native::{canvas_input_allowed, native_dialog_viewport, native_preset_viewport};

/// Where this frame's dialogs are on screen (the canvas reads last frame's: it draws first).
const RECTS: &str = "pc-dialog-rects";

fn rects(ctx: &egui::Context) -> Vec<egui::Rect> {
    ctx.data(|m| m.get_temp(egui::Id::new(RECTS))).unwrap_or_default()
}

/// With a dialog open the rest of the window is inert (egui's modal layer), but like Photoshop the
/// image can still be panned and zoomed under it. The pointer position when it is over `canvas`
/// and not over a dialog or one of its popups.
pub fn free_pointer_over(ctx: &egui::Context, canvas: egui::Rect) -> Option<egui::Pos2> {
    if egui::Popup::is_any_open(ctx) {
        return None;
    }
    let p = ctx.pointer_hover_pos()?;
    let rects = rects(ctx);
    (canvas.contains(p) && !rects.iter().any(|r| r.contains(p))).then_some(p)
}

/// Pan drag under an open dialog: Space-drag, middle-drag, or a drag with the Hand tool, started on
/// the free canvas. Returns this frame's pointer movement.
pub fn pan_delta(ctx: &egui::Context, canvas: egui::Rect, hand: bool) -> Option<egui::Vec2> {
    let rects = rects(ctx);
    let (origin, panning, delta) = ctx.input(|i| {
        let p = &i.pointer;
        (p.press_origin(), p.middle_down() || (p.primary_down() && (hand || i.key_down(egui::Key::Space))), p.delta())
    });
    let origin = origin?;
    (panning && canvas.contains(origin) && !rects.iter().any(|r| r.contains(origin))).then_some(delta)
}

/// A click or drag with the primary button started on the free canvas under an open dialog, Space
/// not held: where the pointer is this frame. The press itself counts even when it is released in
/// the same frame (a quick click, `ui.click`); the drag only while it stays on the free canvas.
pub fn free_press(ctx: &egui::Context, canvas: egui::Rect) -> Option<egui::Pos2> {
    if egui::Popup::is_any_open(ctx) {
        return None;
    }
    let rects = rects(ctx);
    let free = |p: egui::Pos2| canvas.contains(p) && !rects.iter().any(|r| r.contains(p));
    ctx.input(|i| {
        if i.key_down(egui::Key::Space) {
            return None;
        }
        let pressed = i.events.iter().rev().find_map(|e| match e {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } => Some(*pos),
            _ => None,
        });
        if let Some(p) = pressed {
            return free(p).then_some(p);
        }
        let held = i.pointer.primary_down() && i.pointer.press_origin().is_some_and(free);
        i.pointer.latest_pos().filter(|p| held && free(*p))
    })
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    native::cleanup(app, ctx);
    let dialogs = app.ui.dialogs.clone();
    let mut shown = Vec::new();
    for d in dialogs {
        if !ctx.embed_viewports() {
            native::show(app, ctx, &d);
            continue;
        }
        let lang = if crate::prefs_ui::is_preferences(&d.fields) {
            crate::i18n::Lang::from_pref(d.fields.get("values").and_then(|v| v.pointer("/interface/language")).and_then(Value::as_str).unwrap_or("auto"))
        } else {
            crate::i18n::current()
        };
        let _language = crate::i18n::language_scope(lang);
        let mut fields = d.fields.clone();
        let mut outcome: Option<bool> = None; // Some(true)=OK, Some(false)=Cancel
        let mut apply_requested = false;
        let title = display_title(&d);
        let id = egui::Id::new(("dialog", d.id));
        // Opens centred, then its top-left stays put (offset from the window's top-left, moved by
        // dragging the title bar; view state only, so egui memory): a dialog whose body grows, like
        // Layer Style switching effects, extends down and right instead of re-centring.
        let pinned: Option<egui::Vec2> = ctx.data(|m| m.get_temp(id));
        let mut drag = egui::Vec2::ZERO;
        let mut sizing = false;
        let area = match pinned {
            Some(offset) => egui::Modal::default_area(id).anchor(egui::Align2::LEFT_TOP, offset),
            None => egui::Modal::default_area(id),
        };
        // Photoshop doesn't dim the window behind dialogs: previews must be judged at true contrast.
        let modal = egui::Modal::new(id).area(area).backdrop_color(egui::Color32::TRANSPARENT).show(ctx, |ui| {
            sizing = ui.is_sizing_pass();
            ui.set_min_width(380.0);
            let wide = crate::prefs_ui::width(&d.fields);
            if let Some(w) = wide {
                ui.set_min_width(w.min(460.0));
            }
            if d.kind == DialogKind::NewDocument {
                ui.set_min_width(800.0);
            }
            // The About window: room for the contributor table, the same width on every tab.
            let about_tabs = d.kind == DialogKind::About && d.fields.get("systemInfo").and_then(Value::as_bool) != Some(true);
            if about_tabs {
                ui.set_min_width((ctx.content_rect().width() - 64.0).clamp(380.0, 1040.0));
            }
            ui.set_max_width(wide.unwrap_or(if d.kind == DialogKind::NewDocument {
                800.0
            } else if about_tabs {
                1040.0
            } else if d.kind == DialogKind::LayerStyle || d.fields.contains_key("__export") || crate::color_picker_ui::owns(&d.fields) {
                600.0
            } else {
                440.0
            }));
            if let Some(w) = crate::file_ui::dialog_width(&d.fields) {
                ui.set_min_width(w);
                ui.set_max_width(w);
            }
            let t = ui
                .add(egui::Label::new(egui::RichText::new(&title).font(crate::theme::semibold(15.0))).selectable(false).truncate())
                .on_hover_text(&title)
                .rect;
            let bar = egui::Rect::from_min_max(t.min, egui::pos2(ui.max_rect().right(), t.bottom()));
            drag = ui.interact(bar, id.with("title"), egui::Sense::drag()).drag_delta();
            ui.add_space(4.0);
            crate::widgets::hairline(ui);
            ui.add_space(8.0);
            body(app, ui, &d, &mut fields);
            ui.add_space(8.0);
            (outcome, apply_requested) = buttons(app, ui, &d, &fields);
            // The frame around the content (Frame::popup's margin and stroke).
            ui.min_rect().expand(ui.spacing().menu_margin.sum().max_elem() + 2.0)
        });
        shown.push(modal.inner);
        // Pin once laid out at its real size (the first frame is an invisible sizing pass).
        if !sizing && (pinned.is_none() || drag != egui::Vec2::ZERO) {
            let screen = ctx.content_rect();
            // Keep the whole dialog (and so its title bar) on screen.
            let room = (screen.size() - modal.response.rect.size()).max(egui::Vec2::ZERO);
            let offset = pinned.unwrap_or(modal.response.rect.min - screen.min) + drag;
            ctx.data_mut(|m| m.insert_temp(id, offset.clamp(egui::Vec2::ZERO, room)));
        }
        // Esc cancels (topmost dialog, no popup open). A click outside does nothing: Photoshop keeps
        // the dialog, and the pointer may be panning or zooming the canvas under it.
        if outcome.is_none()
            && (modal.response.should_close()
                || (modal.is_top_modal && !modal.any_popup_open && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))))
        {
            outcome = Some(false);
        }
        finish(app, ctx, &d, fields, outcome, apply_requested);
    }
    ctx.data_mut(|m| m.insert_temp(egui::Id::new(RECTS), shown));
}

/// Shared by embedded dialogs and independent desktop dialog windows.
fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, d: &Dialog, fields: &mut serde_json::Map<String, Value>) {
    match d.kind {
        DialogKind::NewDocument => crate::new_doc_ui::body(app, ui, fields),
        DialogKind::About if fields.get("systemInfo").and_then(Value::as_bool) == Some(true) => {
            let lines = crate::gpu_status::system_info(app);
            for l in &lines {
                ui.add(egui::Label::new(egui::RichText::new(l).font(crate::theme::mono(12.0))).selectable(true));
            }
            ui.add_space(8.0);
            if crate::widgets::secondary_button(ui, tl!("Copy"), 84.0).clicked() {
                ui.ctx().copy_text(lines.join("\n"));
            }
        }
        DialogKind::About => {
            // Tabs About · Contributors and models (craftrules standards/contributors.md). The
            // tab is a dialog field, so automation can switch it with `ui.dialog.set`.
            let tab = about_tab(fields);
            let mut chosen = tab;
            ui.horizontal(|ui| {
                for (key, label) in [("about", tl!("About")), ("credits", "photocraft0.5贡献者与模型")] {
                    let response = crate::widgets::pill_tab(ui, label, tab == key);
                    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), tab == key, label));
                    if response.clicked() {
                        chosen = key;
                    }
                }
            });
            if chosen != tab {
                fields.insert("tab".into(), json!(chosen));
            }
            ui.add_space(8.0);
            match chosen {
                "credits" => {
                    ui.vertical_centered(|ui| {
                        crate::links::discord_button(app, ui, 220.0);
                        ui.add_space(8.0);
                        crate::links::link_row(app, ui);
                    });
                    ui.add_space(10.0);
                    ui.weak("egui · wgpu · photocraft-engine");
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                    crate::credits::credits_ui(ui);
                }
                _ => {
                    ui.spacing_mut().item_spacing.y = 3.0;
                    ui.heading(photocraft_engine::build_info::CUSTOM_PRODUCT_TITLE);
                    ui.label("基于开源软件PhotoCraft 版本0.5开发");
                    ui.label("开发者：Mummy-fei");
                    ui.label(format!("自有版本 {}", photocraft_engine::build_info::custom_version()));
                    crate::about_licenses_ui::show(app, ui);
                }
            }
        }
        DialogKind::Command if crate::ps_collaboration_ui::owns(fields) => crate::ps_collaboration_ui::body(ui, fields),
        DialogKind::Command if crate::fill_ui::owns(fields) => crate::fill_ui::body(app, ui, fields),
        DialogKind::Command if crate::rasterize_prompt::owns(fields) => crate::rasterize_prompt::body(ui, fields),
        DialogKind::Command if crate::variables_ui::owns(fields) => crate::variables_ui::body(app, ui, fields),
        DialogKind::Command if crate::file_ui::owns(fields) => crate::file_ui::body(app, ui, fields),
        DialogKind::Command if crate::color_picker_ui::owns(fields) => crate::color_picker_ui::body(ui, fields),
        DialogKind::Command if crate::color_range_ui::owns(fields) => crate::color_range_ui::body(app, ui, fields),
        DialogKind::Command if crate::prefs_ui::owns(fields) => crate::prefs_ui::body(app, ui, fields),
        DialogKind::Command if fields.contains_key("__export") => crate::export_dialog::body(app, ui, fields),
        DialogKind::Command if fields.contains_key("__sizing") => crate::sizing::body(ui, fields),
        DialogKind::Command if crate::adjust_dialog::owns(fields) => crate::adjust_dialog::body(app, ui, fields),
        DialogKind::Command if fields.contains_key("__filter") => {
            // Color Settings: the monitor profile in use can change while it is open.
            if fields.get("__command").and_then(Value::as_str) == Some("edit.colorSettings") {
                fields.insert("__note".into(), Value::String(crate::monitor_status::note(app)));
            }
            crate::filter_dialog::body(ui, fields)
        }
        DialogKind::Command if crate::brush_panel::owns_new_preset(fields) => crate::brush_panel::new_preset_body(ui, fields),
        DialogKind::Command if fields.contains_key("__form") => crate::view_cmds::form_body(ui, fields),
        DialogKind::Command => {}
        DialogKind::LayerStyle => crate::layer_style::body(app, ui, fields),
        DialogKind::Error => {
            let message = fields.get("message").and_then(Value::as_str).unwrap_or("Error");
            ui.label(tl!(message));
        }
    }
}

fn has_contacts(d: &Dialog) -> bool {
    d.kind == DialogKind::About && d.fields.get("systemInfo").and_then(Value::as_bool) != Some(true)
}

fn footer_height(d: &Dialog, width: f32) -> f32 {
    if has_contacts(d) && width < 480.0 { 96.0 } else { 64.0 }
}

fn buttons(app: &mut PhotocraftApp, ui: &mut egui::Ui, d: &Dialog, fields: &serde_json::Map<String, Value>) -> (Option<bool>, bool) {
    let mut outcome = None;
    let mut apply_requested = false;
    if has_contacts(d) {
        let inline = ui.available_width() >= 448.0;
        if !inline {
            crate::welcome_links::contacts(app, ui);
            ui.add_space(8.0);
        }
        ui.horizontal(|ui| {
            let room = (ui.available_width() - 84.0 - ui.spacing().item_spacing.x).max(0.0);
            if inline {
                ui.allocate_ui_with_layout(egui::vec2(room, 30.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_min_width(room);
                    crate::welcome_links::contacts(app, ui);
                });
            } else {
                // add_space advances the cursor without adding item spacing.
                ui.add_space((ui.available_width() - 84.0).max(0.0));
            }
            if dialog_buttons(ui, &[DialogButton::new(ButtonRole::Default, tl!("OK"), 84.0)]).is_some() {
                outcome = Some(false);
            }
        });
        return (outcome, false);
    }
    // Align::Min, not Center: a centred row fills the height left over from last frame's
    // (larger) size, so a dialog whose body gets shorter would never shrink back.
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        if matches!(d.kind, DialogKind::About | DialogKind::Error) {
            if dialog_buttons(ui, &[DialogButton::new(ButtonRole::Default, tl!("OK"), 84.0)]).is_some() {
                outcome = Some(false);
            }
        } else {
            let ok_label = if d.kind == DialogKind::NewDocument {
                tl!("Create")
            } else if d.fields.contains_key("__export") {
                tl!("Export")
            } else {
                crate::file_ui::ok_label(&d.fields).unwrap_or(tl!("OK"))
            };
            let ok = DialogButton::new(ButtonRole::Default, ok_label, 84.0);
            let cancel = DialogButton::new(ButtonRole::Cancel, if d.kind == DialogKind::NewDocument { tl!("Close") } else { tl!("Cancel") }, 84.0);
            let clicked = if d.kind == DialogKind::Command && crate::prefs_ui::is_preferences(fields) {
                let changed = crate::prefs_ui::preferences_changed(app, fields);
                dialog_buttons(ui, &[ok, cancel, DialogButton::new(ButtonRole::Apply, tl!("Apply"), 84.0).enabled(changed)])
            } else {
                dialog_buttons(ui, &[ok, cancel])
            };
            match clicked {
                Some(ButtonRole::Cancel) => outcome = Some(false),
                Some(ButtonRole::Apply) => apply_requested = true,
                Some(_) => outcome = Some(true),
                None if ui.input(|i| i.key_pressed(egui::Key::Enter)) => outcome = Some(true),
                None => {}
            }
        }
    });
    (outcome, apply_requested)
}

fn finish(app: &mut PhotocraftApp, ctx: &egui::Context, d: &Dialog, fields: serde_json::Map<String, Value>, outcome: Option<bool>, apply_requested: bool) {
    if crate::gradient_ui::is_stop_picker(&d.fields) && (d.fields.get("color") != fields.get("color") || outcome.is_some()) {
        crate::gradient_ui::repaint_stop_preview(app, ctx);
    }
    if let Some(dm) = app.ui.dialog_mut(d.id) {
        dm.fields = fields;
    }
    if apply_requested && outcome.is_none() {
        let _ = crate::prefs_ui::apply(app, d.id);
    }
    match outcome {
        Some(true) => {
            let _ = confirm(app, d.id);
        }
        Some(false) => {
            app.ui.close_dialog(d.id);
            // A color child returns to its still-open parent, including that parent's preview.
            if !crate::color_picker_ui::owns(&d.fields) {
                app.filter_preview = None;
                app.color_range = None;
            }
        }
        None => {}
    }
}

/// The About window's tabs, as stored in its `tab` field.
pub const ABOUT_TABS: [&str; 2] = ["about", "credits"];

/// The About tab to show: the `tab` field when it names one, otherwise "about".
fn about_tab(fields: &serde_json::Map<String, Value>) -> &'static str {
    let want = fields.get("tab").and_then(Value::as_str).unwrap_or("");
    ABOUT_TABS.iter().copied().find(|t| *t == want).unwrap_or("about")
}

/// The dialog title as shown: [`title`] in the UI language.
fn display_title(d: &Dialog) -> String {
    match d.kind {
        DialogKind::About if d.fields.get("systemInfo").and_then(Value::as_bool) != Some(true) => {
            crate::i18n::fmt(tl!("About {product}"), &[("product", photocraft_engine::build_info::CUSTOM_PRODUCT_TITLE)])
        }
        DialogKind::Command => {
            let label = d.fields.get("__label").and_then(Value::as_str).unwrap_or("Command");
            // Dialog labels are catalogued with their "…" ("Export As…"); some commands omit it.
            let with_dots = format!("{}…", label.trim_end_matches('…'));
            let shown = if crate::i18n::has(crate::i18n::current(), label) { tl!(label) } else { tl!(&with_dots) };
            shown.trim_end_matches('…').to_string()
        }
        _ => tl!(&title(d)).to_string(),
    }
}

pub fn title(d: &Dialog) -> String {
    match d.kind {
        DialogKind::NewDocument => "New Document".into(),
        DialogKind::About if d.fields.get("systemInfo").and_then(Value::as_bool) == Some(true) => "System Info".into(),
        DialogKind::About => format!("About {}", photocraft_engine::build_info::CUSTOM_PRODUCT_TITLE),
        DialogKind::LayerStyle => "Layer Style".into(),
        DialogKind::Command => d.fields.get("__label").and_then(Value::as_str).unwrap_or("Command").trim_end_matches('…').to_string(),
        DialogKind::Error => "Error".into(),
    }
}

/// Confirm a dialog: run its action and close it. Used by the OK button and by automation.
pub fn confirm(app: &mut PhotocraftApp, id: u64) -> Result<Value, String> {
    if let Some(fields) = app.ui.dialogs.iter().find(|d| d.id == id && crate::brush_panel::owns_new_preset(&d.fields)).map(|d| d.fields.clone()) {
        let result = crate::brush_panel::confirm_new_preset(app, &fields);
        match &result {
            Ok(_) => {
                app.ui.close_dialog(id);
            }
            Err(error) => {
                if let Some(dialog) = app.ui.dialog_mut(id) {
                    dialog.fields.insert("__preset_error".into(), json!(error));
                }
            }
        }
        return result;
    }
    if let Some(fields) = app.ui.dialogs.iter().find(|d| d.id == id && crate::color_picker_ui::owns(&d.fields)).map(|d| d.fields.clone()) {
        let result = crate::color_picker_ui::confirm(app, &fields);
        match &result {
            Ok(_) => {
                app.ui.close_dialog(id);
            }
            Err(error) => {
                if let Some(dialog) = app.ui.dialog_mut(id) {
                    dialog.fields.insert("__picker_error".into(), json!(error));
                }
            }
        }
        return result;
    }
    let d = app.ui.close_dialog(id).ok_or_else(|| format!("no dialog {id}"))?;
    match d.kind {
        DialogKind::NewDocument => {
            let r = app.run("file.new", crate::new_doc_ui::command_params(&d.fields));
            if let Some(i) = app.session.active_index() {
                app.ui.views[i].fit_pending = true;
            }
            r
        }
        DialogKind::Command if crate::ps_collaboration_ui::owns(&d.fields) => crate::ps_collaboration_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::fill_ui::owns(&d.fields) => crate::fill_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::rasterize_prompt::owns(&d.fields) => crate::rasterize_prompt::confirm(app, &d.fields),
        DialogKind::Command if crate::variables_ui::owns(&d.fields) => crate::variables_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::file_ui::owns(&d.fields) => crate::file_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::color_picker_ui::owns(&d.fields) => crate::color_picker_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::color_range_ui::owns(&d.fields) => crate::color_range_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::prefs_ui::owns(&d.fields) => crate::prefs_ui::confirm(app, &d.fields),
        DialogKind::Command if d.fields.contains_key("__export") => crate::export_dialog::confirm(app, &d.fields),
        DialogKind::Command => {
            app.filter_preview = None;
            let cmd = d.fields.get("__command").and_then(|v| v.as_str().map(str::to_string)).ok_or("dialog has no command")?;
            let (cmd, params) = crate::smart_ui::confirm_command(&d.fields, cmd, crate::filter_dialog::params_of(&d.fields));
            app.run(&cmd, params)
        }
        DialogKind::LayerStyle => crate::layer_style::confirm(app, &d.fields),
        DialogKind::About | DialogKind::Error => Ok(Value::Null),
    }
}

/// Open the parameter dialog of `command`: the adjustment editor for `image.adjustments.*`, the
/// schema dialog for filters, the Color Range dialog for `select.colorRange`, otherwise a bare
/// confirm dialog.
pub fn open_command_dialog(app: &mut PhotocraftApp, command: &str, label: &str) -> u64 {
    if command == crate::color_range_ui::COMMAND {
        return crate::color_range_ui::open(app);
    }
    if let Some(id) = crate::adjust_dialog::open(app, command) {
        return id;
    }
    if crate::filter_dialog::has_dialog(command)
        && let Some(id) = crate::filter_dialog::open(app, command)
    {
        return id;
    }
    let mut fields = serde_json::Map::new();
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(label));
    app.ui.open_dialog(DialogKind::Command, fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dragging_the_title_bar_moves_the_dialog() {
        use egui_kittest::{Harness, kittest::Queryable};

        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
        harness.state_mut().ui.open_dialog(DialogKind::LayerStyle, serde_json::Map::new());
        harness.run_steps(3);
        let before = harness.get_by_label("Layer Style").rect();

        // Grab the title text itself: it must move the dialog, not select the text.
        let from = before.center();
        harness.hover_at(from);
        harness.drag_at(from);
        harness.run_steps(2);
        for i in 1..=10 {
            harness.hover_at(from + egui::vec2(-12.0, 8.0) * i as f32);
            harness.run_steps(1);
        }
        harness.drop_at(from + egui::vec2(-120.0, 80.0));
        harness.run_steps(3);

        let moved = harness.get_by_label("Layer Style").rect().min - before.min;
        assert!((moved - egui::vec2(-120.0, 80.0)).length() < 1.0, "dialog moved by {moved:?}");
        assert_eq!(harness.state().ui.dialogs.len(), 1, "dragging must not close the dialog");
    }

    #[test]
    fn custom_about_identity_and_merged_credits_are_stable() {
        assert_eq!(photocraft_engine::build_info::CUSTOM_PRODUCT_NAME, "InkPop");
        assert_eq!(photocraft_engine::build_info::CUSTOM_PRODUCT_NAME_ZH, "InkPop");
        assert_eq!(photocraft_engine::build_info::CUSTOM_PRODUCT_TITLE, "InkPop");
        assert!(!photocraft_engine::build_info::custom_version().is_empty());
        assert_eq!(ABOUT_TABS, ["about", "credits"]);
        let mut fields = serde_json::Map::new();
        fields.insert("tab".into(), json!("credits"));
        assert_eq!(about_tab(&fields), "credits");
        assert_eq!(about_tab(&serde_json::Map::new()), "about");
    }

    #[test]
    fn about_home_keeps_identity_and_credits_keep_links_and_generated_records() {
        use egui_kittest::{Harness, kittest::Queryable};

        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(1400.0, 1400.0)).build_ui_state(
            |ui, app| {
                let _language = crate::i18n::language_scope(crate::i18n::Lang::EN);
                show(app, ui.ctx());
            },
            app,
        );
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
        harness.state_mut().ui.open_dialog(DialogKind::About, serde_json::Map::new());
        harness.run_steps(3);
        assert!(harness.query_by_label("InkPop").is_some());
        assert!(harness.query_by_label("基于开源软件PhotoCraft 版本0.5开发").is_some());
        assert!(harness.query_by_label("开发者：Mummy-fei").is_some());
        assert!(harness.query_by_label("photocraft0.5贡献者与模型").is_some());
        assert!(harness.query_by_label(&format!("自有版本 {}", photocraft_engine::build_info::custom_version())).is_some());
        assert!(harness.query_by_label_contains("基础版本").is_none());
        assert!(harness.query_by_label("Join us on Discord").is_none());
        assert!(harness.query_by_label("egui · wgpu · photocraft-engine").is_none());

        harness.get_by_label("photocraft0.5贡献者与模型").click();
        harness.run_steps(3);
        for label in ["Join us on Discord", "PhotoCraft website", "GitHub", "ArtCraft", "egui · wgpu · photocraft-engine", "Models"] {
            assert!(harness.query_all_by_label(label).next().is_some(), "credits page retains {label}");
        }
        assert!(harness.query_by_label_contains("Contributors:").is_some(), "generated contributor list remains available");
        assert!(harness.query_by_label_contains("自有版本").is_none());
    }

    #[test]
    fn about_native_title_translates_without_changing_protocol_identity() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.open_dialog(DialogKind::About, serde_json::Map::new());
        let dialog = app.ui.dialogs.last().unwrap();
        let english = format!("About {}", photocraft_engine::build_info::CUSTOM_PRODUCT_TITLE);
        assert_eq!(title(dialog), english);
        for code in ["en", "zh-hans", "zh-hant", "ja"] {
            let lang = crate::i18n::Lang::from_code(code).unwrap();
            crate::i18n::with_language(lang, || {
                let displayed = display_title(dialog);
                assert!(displayed.contains(photocraft_engine::build_info::CUSTOM_PRODUCT_TITLE));
                if code == "en" {
                    assert_eq!(displayed, english);
                } else {
                    assert_ne!(displayed, english);
                    assert!(!displayed.contains("{product}"));
                }
            });
        }
    }
}
