//! Non-blocking notices: a small stack of cards in the lower-right corner of the window for things
//! the user should see but that must not interrupt work (import warnings such as "adjustment
//! flattened", files that couldn't be opened). Notices live in [`UiState`](crate::UiState) so the
//! control channel can read them (`ui.inspect`) and clear them (`ui.set`).

use std::{collections::HashMap, sync::Arc, time::Duration};

use egui::epaint::Galley;
use egui::{Id, Pos2, Rect, ViewportCommand, ViewportId, pos2, vec2};
use serde::{Deserialize, Serialize};

use crate::PhotocraftApp;

const WAYLAND_FILE_DROP_DISMISSED: &str = "ui.waylandFileDropGuidanceDismissed";
/// Number of notices shown at once.
pub const MAX_NOTICES: usize = 3;
const DISPLAY_SECS: f64 = 5.0;
const MIN_DISPLAY_SECS: f64 = 3.0;
/// Lines shown per notice before "…and N more".
const MAX_LINES: usize = 8;
const CARD_WIDTH: f32 = 362.0;
const CARD_GAP: f32 = 8.0;
const CARD_INSET: f32 = 11.0;
const CLOSE_WIDTH: f32 = 24.0;
const LINE_GAP: f32 = 3.0;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    pub id: u64,
    pub title: String,
    #[serde(default)]
    pub lines: Vec<String>,
    /// An error (warning colour) rather than an informational notice.
    #[serde(default)]
    pub error: bool,
    /// Preference key to set when this notice is dismissed.
    #[serde(default)]
    pub dismiss_pref: Option<String>,
    #[serde(skip)]
    shown_at: Option<f64>,
    #[serde(skip)]
    resolved: bool,
}

impl Notice {
    fn expires_at(&self) -> Option<f64> {
        self.shown_at.map(|time| time + if self.resolved { MIN_DISPLAY_SECS } else { DISPLAY_SECS })
    }
}

/// Show a notice (newest last); returns its id.
pub fn post(app: &mut PhotocraftApp, title: impl Into<String>, lines: Vec<String>, error: bool, dismiss_pref: Option<&str>) -> u64 {
    let id = app.ui.alloc_id();
    app.ui.notices.push(Notice { id, title: title.into(), lines, error, dismiss_pref: dismiss_pref.map(str::to_owned), ..Default::default() });
    cap_notices(&mut app.ui.notices);
    id
}

fn cap_notices(notices: &mut Vec<Notice>) {
    let excess = notices.len().saturating_sub(MAX_NOTICES);
    notices.drain(..excess);
}

/// Show the Wayland-specific fallback guidance unless the user dismissed it in preferences.
pub fn wayland_file_drop_guidance(app: &mut PhotocraftApp) {
    if !app.services.is_wayland
        || app.session.prefs().dialogs.get(WAYLAND_FILE_DROP_DISMISSED).and_then(serde_json::Value::as_bool) == Some(true)
        || app.ui.notices.iter().any(|notice| notice.dismiss_pref.as_deref() == Some(WAYLAND_FILE_DROP_DISMISSED))
    {
        return;
    }
    let guidance = if crate::i18n::current().code() == "zh-hans" || crate::i18n::system_lang().code() == "zh-hans" {
        "Wayland 暂不支持原生文件拖放。请使用“文件 › 打开”，或在 XWayland 下运行 InkPop。"
    } else {
        tl!("Native file drag-and-drop is not supported on Wayland yet. Use File › Open, or run InkPop under XWayland with `WAYLAND_DISPLAY= photocraft`.")
    };
    post(app, tl!("Native file drag-and-drop is unavailable"), vec![guidance.into()], false, Some(WAYLAND_FILE_DROP_DISMISSED));
}

fn dismiss(app: &mut PhotocraftApp, id: u64) {
    let dismiss_pref = app.ui.notices.iter().find(|notice| notice.id == id).and_then(|notice| notice.dismiss_pref.clone());
    app.ui.notices.retain(|notice| notice.id != id);
    if let Some(key) = dismiss_pref {
        app.session.prefs.edit(|prefs| {
            prefs.dialogs.insert(key, serde_json::Value::Bool(true));
        });
    }
}

/// Clear a recovered condition after the user has had time to read it.
pub(crate) fn resolve(app: &mut PhotocraftApp, id: u64) {
    app.ui.notices.retain_mut(|notice| {
        if notice.id == id {
            notice.resolved = true;
            return notice.shown_at.is_some();
        }
        true
    });
}

/// Report import `warnings` for the file operation `what` (e.g. "Opened a.psd"): the status
/// bar says how many there were, and a notice lists them. Nothing happens when there are none.
pub fn io_warnings(app: &mut PhotocraftApp, what: &str, warnings: &[String]) {
    report_io_warnings(app, what, warnings, true);
}

/// A completed save or export reports its warnings in the status bar and warning log.
pub(crate) fn output_warnings(app: &mut PhotocraftApp, what: &str, warnings: &[String]) {
    report_io_warnings(app, what, warnings, false);
}

fn report_io_warnings(app: &mut PhotocraftApp, what: &str, warnings: &[String], show_notice: bool) {
    let Some(first) = warnings.first() else { return };
    let n = warnings.len();
    let title = localized_operation(what);
    let first = localized_warning(first);
    if let Some(record) = app.services.warning_event.as_ref() {
        record(what, warnings);
    }
    let chinese = crate::i18n::current().code() == "zh-hans";
    app.ui.status = if n == 1 {
        if chinese { format!("{title}：{first}") } else { format!("{title}: {first}") }
    } else if chinese {
        format!("{title}：发现 {n} 条警告：{first} …")
    } else {
        format!("{title} with {n} warnings: {first} …")
    };
    app.ui.status_error = true;
    if show_notice {
        let notice_title = if chinese { format!("{title}（{n} 条警告）") } else { format!("{title} with {n} warning{}", if n == 1 { "" } else { "s" }) };
        let lines = warnings.iter().map(|warning| localized_warning(warning)).collect();
        post(app, notice_title, lines, false, None);
    }
}

fn localized_operation(what: &str) -> String {
    if crate::i18n::current().code() != "zh-hans" {
        return what.to_owned();
    }
    for (prefix, label) in [("Opened ", "打开"), ("Saved a copy as ", "已保存副本"), ("Saved ", "已保存"), ("Exported ", "已导出")] {
        if let Some(name) = what.strip_prefix(prefix) {
            return format!("{label} {name}");
        }
    }
    what.to_owned()
}

fn localized_warning(warning: &str) -> String {
    if crate::i18n::current().code() != "zh-hans" {
        return warning.to_owned();
    }
    let lower = warning.to_ascii_lowercase();
    if lower.contains("flattened") && lower.contains("blend") {
        return "图层已合并；图层、蒙版和混合模式未保留".into();
    }
    if lower.contains("flattened") && lower.contains("adjustment") {
        return warning.replace("Adjustment layer", "调整图层").replace(" was flattened", "已合并");
    }
    if lower.contains("flattened") {
        return "图层已合并".into();
    }
    if lower.contains("lossy compression") {
        return "使用了有损压缩".into();
    }
    warning.to_owned()
}

/// Report a failed file operation in the status bar and as a notice.
pub fn error(app: &mut PhotocraftApp, message: String) {
    app.ui.status = message.clone();
    app.ui.status_error = true;
    post(app, message, Vec::new(), true, None);
}

/// Draw the notices; each has a close button.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let native = !ctx.embed_viewports() && ctx.viewport_id() == ViewportId::ROOT;
    let now = ctx.input(|input| input.time);
    app.ui.notices.retain(|notice| notice.expires_at().is_none_or(|expires| expires > now));
    cap_notices(&mut app.ui.notices);
    if native {
        sync_native_windows(ctx, &app.ui.notices);
    }
    if app.ui.notices.is_empty() {
        return;
    }
    let dismiss_id = show_cards(app, ctx, native);
    if let Some(id) = dismiss_id {
        dismiss(app, id);
        ctx.request_repaint();
    }
    let expiry = app.ui.notices.iter().filter_map(Notice::expires_at).reduce(f64::min);
    if let Some(expiry) = expiry
        && let Ok(delay) = Duration::try_from_secs_f64(expiry - now)
    {
        ctx.request_repaint_after(delay);
    }
}

/// Measure the same text galleys we paint, before allocating any native windows.
struct Card {
    title: Arc<Galley>,
    body: Vec<Arc<Galley>>,
    header_height: f32,
    height: f32,
}

fn card_layout(ctx: &egui::Context, notice: &Notice, width: f32, height_limit: f32) -> Card {
    let style = ctx.global_style();
    let tokens = crate::theme::Tokens::get(ctx);
    let font = egui::TextStyle::Body.resolve(&style);
    let title_color = if notice.error { tokens.warning } else { tokens.text };
    let inner_width = (width - CARD_INSET * 2.0).max(1.0);
    ctx.fonts_mut(|fonts| {
        let medium = crate::theme::medium(font.size);
        let title_font = if fonts.families().contains(&medium.family) { medium } else { font.clone() };
        let mut title = egui::text::LayoutJob::simple(notice.title.clone(), title_font, title_color, (inner_width - CLOSE_WIDTH - LINE_GAP).max(1.0));
        title.wrap.max_rows = 1;
        title.wrap.break_anywhere = true;
        let title = fonts.layout_job(title);
        let header_height = style.spacing.interact_size.y.max(title.size().y);
        let mut height = CARD_INSET * 2.0 + header_height;
        let mut body = Vec::new();
        let row_height = fonts.row_height(&font).max(1.0);
        let layout_line = |fonts: &mut egui::epaint::text::FontsView<'_>, text, color, max_rows| {
            let mut job = egui::text::LayoutJob::simple(text, font.clone(), color, inner_width);
            job.wrap.max_rows = max_rows;
            fonts.layout_job(job)
        };
        for line in notice.lines.iter().take(MAX_LINES) {
            let available = ((height_limit - height - LINE_GAP) / row_height).floor() as usize;
            if available == 0 {
                break;
            }
            let line = layout_line(fonts, format!("• {line}"), tokens.text_dim, available);
            height += LINE_GAP + line.size().y;
            body.push(line);
        }
        if body.len() < notice.lines.len() && height + LINE_GAP + row_height <= height_limit {
            let more = layout_line(fonts, format!("…and {} more", notice.lines.len() - body.len()), tokens.text_faint, 1);
            height += LINE_GAP + more.size().y;
            body.push(more);
        }
        Card { title, body, header_height, height: height.ceil() }
    })
}

fn paint_card(ui: &mut egui::Ui, rect: Rect, card: &Card, notice: &mut Notice) -> bool {
    let tokens = crate::theme::Tokens::get(ui.ctx());
    let border = if notice.error { tokens.warning } else { tokens.card_border };
    ui.painter().rect(rect, tokens.radius, tokens.card, egui::Stroke::new(1.0, border), egui::StrokeKind::Inside);
    let inside = rect.shrink(CARD_INSET);
    let title_size = card.title.size();
    let title_pos = pos2(inside.left(), inside.top() + (card.header_height - title_size.y) / 2.0);
    if ui.is_visible() && ui.clip_rect().contains(title_pos + title_size / 2.0) {
        notice.shown_at.get_or_insert_with(|| ui.input(|input| input.time));
    }
    let full_text = || std::iter::once(notice.title.as_str()).chain(notice.lines.iter().map(String::as_str)).collect::<Vec<_>>().join("\n");
    let mut text_bounds = Rect::from_min_size(title_pos, title_size);
    ui.painter().galley(title_pos, card.title.clone(), tokens.text);
    let mut y = inside.top() + card.header_height;
    for line in &card.body {
        y += LINE_GAP;
        let pos = pos2(inside.left(), y);
        ui.painter().galley(pos, line.clone(), tokens.text_dim);
        text_bounds = text_bounds.union(Rect::from_min_size(pos, line.size()));
        y += line.size().y;
    }
    let text = ui.interact(text_bounds, ui.id().with(("notice-text", notice.id)), egui::Sense::hover());
    text.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), &notice.title));
    text.on_hover_ui(|ui| {
        ui.label(full_text());
    });
    let close = Rect::from_min_size(pos2(inside.right() - CLOSE_WIDTH, inside.top()), vec2(CLOSE_WIDTH, card.header_height));
    ui.push_id(notice.id, |ui| {
        ui.place(close, egui::Button::new(egui::RichText::new("×").color(tokens.text_dim)).frame(false)).on_hover_text(tl!("Dismiss")).clicked()
    })
    .inner
}

fn native_viewport(notice: u64) -> ViewportId {
    ViewportId::from_hash_of(("inkpop-file-notices", notice))
}

fn sync_native_windows(ctx: &egui::Context, notices: &[Notice]) {
    let stack = Id::new("inkpop-notice-viewports");
    let active: Vec<_> = notices.iter().map(|notice| native_viewport(notice.id)).collect();
    let retired = ctx.data_mut(|data| {
        let previous = data.get_temp::<Vec<ViewportId>>(stack).unwrap_or_default();
        data.insert_temp(stack, active.clone());
        let roles = data.get_temp_mut_or_default::<HashMap<ViewportId, u8>>(Id::new("photocraft-native-window-roles"));
        let retired: Vec<_> = previous.into_iter().filter(|id| !active.contains(id)).collect();
        for id in &retired {
            roles.remove(id);
        }
        // Application-local popup priority keeps each card above floating canvases and panels.
        roles.extend(active.into_iter().map(|id| (id, 2)));
        retired
    });
    for id in retired {
        ctx.data_mut(|data| data.remove::<bool>(Id::new(("inkpop-notice-opened", id))));
        ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(false));
    }
}

fn show_cards(app: &mut PhotocraftApp, ctx: &egui::Context, native: bool) -> Option<u64> {
    let room = crate::work_area::visible_rect(ctx);
    let width = CARD_WIDTH.min((room.width() - 32.0).max(1.0));
    // Reserve the status bar and use available height for readable cards with no scroll container.
    let bottom_right = room.right_bottom() - vec2(16.0, 36.0);
    let max_height = (bottom_right.y - room.top() - 8.0) / app.ui.notices.len() as f32 - CARD_GAP;
    let origin = ctx.input(|input| input.viewport().inner_rect.map_or(Pos2::ZERO, |rect| rect.min)) - ctx.content_rect().min;
    let mut bottom = bottom_right.y;
    let mut dismiss_id = None;
    for notice in app.ui.notices.iter_mut().rev() {
        let card = card_layout(ctx, notice, width, max_height);
        let rect = Rect::from_min_size(pos2(bottom_right.x - width, bottom - card.height), vec2(width, card.height));
        bottom = rect.top() - CARD_GAP;
        let clicked = if native {
            show_native(ctx, rect.translate(origin), &card, notice)
        } else {
            egui::Area::new(Id::new(("photocraft-notice-card", notice.id)))
                .order(egui::Order::Foreground)
                .fixed_pos(rect.min)
                .default_size(rect.size())
                .constrain(false)
                .show(ctx, |ui| {
                    ui.set_min_size(rect.size());
                    paint_card(ui, rect, &card, notice)
                })
                .inner
        };
        if clicked {
            dismiss_id = Some(notice.id);
        }
    }
    dismiss_id
}

/// A native window covers one card only. The spaces between cards remain clickable.
fn show_native(ctx: &egui::Context, rect: Rect, card: &Card, notice: &mut Notice) -> bool {
    let id = native_viewport(notice.id);
    let opened = Id::new(("inkpop-notice-opened", id));
    let first = !ctx.data(|data| data.get_temp::<bool>(opened).unwrap_or(false));
    let mut builder = egui::ViewportBuilder::default()
        .with_title("InkPop notices")
        .with_decorations(false)
        .with_transparent(true)
        .with_taskbar(false)
        .with_active(false)
        .with_resizable(false)
        .with_window_level(egui::WindowLevel::Normal)
        .with_position(rect.min)
        .with_inner_size(rect.size());
    if first {
        builder = builder.with_visible(false);
    }
    ctx.show_viewport_immediate(id, builder, |ui, _| {
        if first {
            // The renderer paints before revealing a new window.
            ui.ctx().data_mut(|data| data.insert_temp(opened, true));
            ui.ctx().send_viewport_cmd(ViewportCommand::Visible(true));
        }
        paint_card(ui, Rect::from_min_size(ui.min_rect().min, rect.size()), card, notice)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PhotocraftApp, Services};
    use photocraft_engine::Session;
    use std::sync::{Arc, Mutex};

    fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, time: f64, size: egui::Vec2) -> egui::FullOutput {
        let mut output = ctx
            .run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), time: Some(time), ..Default::default() }, |ui| {
                show(app, ui.ctx())
            });
        output.textures_delta.clear();
        output
    }

    #[test]
    fn fourth_notice_replaces_the_oldest_even_during_its_lifetime() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        let ctx = egui::Context::default();
        let size = egui::vec2(1000.0, 800.0);
        let first = post(&mut app, "First warning", Vec::new(), true, None);
        frame(&mut app, &ctx, 10.0, size);
        frame(&mut app, &ctx, 10.0, size);
        assert_eq!(app.ui.notices[0].shown_at, Some(10.0));

        let second = post(&mut app, "Second", Vec::new(), false, None);
        let third = post(&mut app, "Third", Vec::new(), false, None);
        frame(&mut app, &ctx, 10.5, size);
        frame(&mut app, &ctx, 10.5, size);
        assert_eq!(app.ui.notices.iter().map(|notice| notice.id).collect::<Vec<_>>(), [first, second, third]);
        let fourth = post(&mut app, "Fourth", Vec::new(), false, None);
        assert_eq!(app.ui.notices.iter().map(|notice| notice.id).collect::<Vec<_>>(), [second, third, fourth]);
        frame(&mut app, &ctx, 11.0, size);
        frame(&mut app, &ctx, 11.0, size);
        assert_eq!(app.ui.notices[0].expires_at(), Some(15.5));
        assert_eq!(app.ui.notices[2].expires_at(), Some(16.0));
        frame(&mut app, &ctx, 15.5, size);
        assert_eq!(app.ui.notices[0].id, fourth);
        frame(&mut app, &ctx, 16.0, size);
        assert!(app.ui.notices.is_empty());
    }

    #[test]
    fn embedded_cards_stack_up_from_the_bottom() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        let ctx = egui::Context::default();
        let size = vec2(900.0, 640.0);
        post(&mut app, "Old", vec!["• Longer explanation ".repeat(7)], false, None);
        let card_rects = |output: &egui::FullOutput| {
            let mut rects: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|entry| match &entry.shape {
                    egui::Shape::Rect(r) if (r.rect.width() - CARD_WIDTH).abs() < 0.1 => Some(r.rect),
                    _ => None,
                })
                .collect();
            rects.sort_by(|a, b| a.bottom().total_cmp(&b.bottom()));
            rects
        };
        frame(&mut app, &ctx, 10.0, size);
        let first = card_rects(&frame(&mut app, &ctx, 10.0, size));
        assert_eq!(first.len(), 1, "{first:?}");
        post(&mut app, "New", Vec::new(), false, None);
        frame(&mut app, &ctx, 11.0, size);
        let cards = card_rects(&frame(&mut app, &ctx, 11.0, size));
        let [oldest, newest] = cards.as_slice() else { panic!("expected two card shapes, got {cards:?}") };
        assert_eq!(first[0].bottom(), size.y - 36.0);
        assert_eq!(newest.bottom(), first[0].bottom());
        assert_eq!(oldest.bottom(), newest.top() - CARD_GAP);
        assert!(oldest.height() > newest.height());
    }

    #[test]
    fn native_notice_uses_popup_priority_without_focus_requests() {
        std::thread::spawn(|| {
            let mut app = PhotocraftApp::new(Session::new(), Services::default());
            let ctx = egui::Context::default();
            ctx.set_embed_viewports(false);
            let id = native_viewport(post(&mut app, "Native warning", Vec::new(), true, None));
            egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
                assert_eq!(viewport.ids.this, id);
                assert_eq!(viewport.ids.parent, ViewportId::ROOT);
                assert_eq!(viewport.builder.active, Some(false));
                assert_eq!(viewport.builder.transparent, Some(true));
                assert_eq!(viewport.builder.window_level, Some(egui::WindowLevel::Normal));
                let size = viewport.builder.inner_size.unwrap();
                assert_eq!(size.x, CARD_WIDTH);
                assert!(size.y < 70.0);
                let time = ctx.input(|input| input.time);
                let mut input = egui::RawInput {
                    viewport_id: id,
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    time: Some(time),
                    focused: false,
                    ..Default::default()
                };
                input.viewports.entry(id).or_default().focused = Some(false);
                let mut output = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
                assert!(output.viewport_output.values().flat_map(|output| &output.commands).all(|cmd| !matches!(cmd, ViewportCommand::Focus)));
                assert!(output.shapes.iter().all(|entry| !matches!(&entry.shape, egui::Shape::Rect(r) if r.fill == crate::theme::Tokens::get(ctx).canvas)));
                output.textures_delta.clear();
            });
            let size = egui::vec2(1000.0, 800.0);
            frame(&mut app, &ctx, 10.0, size);
            let roles = Id::new("photocraft-native-window-roles");
            assert_eq!(ctx.data(|data| data.get_temp::<HashMap<ViewportId, u8>>(roles).unwrap().get(&id).copied()), Some(2));
            assert_eq!(app.ui.notices[0].expires_at(), Some(15.0));
            frame(&mut app, &ctx, 14.99, size);
            assert_eq!(app.ui.notices.len(), 1);
            frame(&mut app, &ctx, 15.0, size);
            assert!(app.ui.notices.is_empty());
            assert!(ctx.data(|data| data.get_temp::<HashMap<ViewportId, u8>>(roles).unwrap()).is_empty());
        })
        .join()
        .unwrap();
    }

    #[test]
    fn long_filename_notice_keeps_close_button_inside_header_v95() {
        for theme in crate::theme::ThemeKind::ALL {
            let ctx = egui::Context::default();
            PhotocraftApp::setup_context(&ctx, theme);
            let mut notice = Notice { title: format!("Saved {}.psd", "LongFilename中文".repeat(60)), ..Default::default() };
            let mut output = ctx.run_ui(
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(380.0, 100.0))), ..Default::default() },
                |ui| {
                    let card = card_layout(ui.ctx(), &notice, CARD_WIDTH, 100.0);
                    paint_card(ui, Rect::from_min_size(Pos2::ZERO, vec2(CARD_WIDTH, card.height)), &card, &mut notice);
                },
            );
            output.textures_delta.clear();
            let mut elided = false;
            let mut close_visible = false;
            for shape in &output.shapes {
                if let egui::Shape::Text(text) = &shape.shape {
                    elided |= text.galley.elided;
                    if text.galley.text() == "×" {
                        close_visible = text.pos.x >= 300.0 && text.pos.x + text.galley.size().x <= CARD_WIDTH;
                    }
                }
            }
            assert!(elided && close_visible, "{theme:?}: title is elided and the close glyph stays on screen");
        }
    }

    #[test]
    fn wayland_guidance_is_only_shown_in_wayland_sessions() {
        let mut app = PhotocraftApp::new(Session::new(), Services { is_wayland: true, ..Default::default() });
        assert_eq!(app.ui.notices.len(), 1);
        assert_eq!(app.ui.notices[0].title, "Native file drag-and-drop is unavailable");
        let guidance = app.ui.notices[0].lines.join(" ");
        assert!(guidance.contains("not supported on Wayland yet"));
        assert!(guidance.contains("File › Open"));
        assert!(!guidance.contains("Ctrl+V"));
        assert!(guidance.contains("XWayland"));
        assert!(guidance.contains("WAYLAND_DISPLAY= photocraft"));
        assert_eq!(app.ui.notices[0].dismiss_pref.as_deref(), Some(WAYLAND_FILE_DROP_DISMISSED));
        for i in 0..MAX_NOTICES {
            post(&mut app, format!("Transient {i}"), Vec::new(), false, None);
        }
        assert_eq!(app.ui.notices.len(), MAX_NOTICES);
        assert!(app.ui.notices.iter().all(|notice| notice.dismiss_pref.is_none()));

        let app = PhotocraftApp::new(Session::new(), Services::default());
        assert!(app.ui.notices.is_empty());
    }

    #[test]
    fn dismissing_wayland_guidance_persists_across_launches() {
        let saved = Arc::new(Mutex::new(None::<String>));
        let load_store = saved.clone();
        let save_store = saved.clone();
        let services = Services {
            is_wayland: true,
            load_prefs: Some(Box::new(move || load_store.lock().unwrap_or_else(|e| e.into_inner()).clone())),
            save_prefs: Some(Box::new(move |text| {
                *save_store.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_owned());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(Session::new(), services);
        let id = app.ui.notices[0].id;
        dismiss(&mut app, id);
        crate::prefs_ui::tick(&mut app, &egui::Context::default());

        let services =
            Services { is_wayland: true, load_prefs: Some(Box::new(move || saved.lock().unwrap_or_else(|e| e.into_inner()).clone())), ..Default::default() };
        let app = PhotocraftApp::new(Session::new(), services);
        assert!(app.ui.notices.is_empty());
    }

    #[test]
    fn dismissing_a_notice_writes_its_own_preference_key() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        post(&mut app, "Dismissible", Vec::new(), false, Some("ui.testNoticeDismissed"));
        let id = app.ui.notices[0].id;
        dismiss(&mut app, id);
        assert_eq!(app.session.prefs().dialogs.get("ui.testNoticeDismissed").and_then(serde_json::Value::as_bool), Some(true));
    }

    #[test]
    fn chinese_file_warnings_are_localized_without_changing_the_recorded_source() {
        let previous = crate::i18n::current();
        crate::i18n::set_current(crate::i18n::Lang::from_code("zh-hans").expect("zh-hans"));
        assert_eq!(localized_operation("Saved sample.psd"), "已保存 sample.psd");
        assert_eq!(localized_warning("Layer(s) flattened; layers, masks and blend modes not kept"), "图层已合并；图层、蒙版和混合模式未保留");
        assert_eq!(localized_warning("Lossy compression"), "使用了有损压缩");
        crate::i18n::set_current(previous);
    }
}
