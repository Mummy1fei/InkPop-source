//! ⌘K command palette: fuzzy search over every menu command and tool.

use crate::theme::palette::ScrollAreaPaletteExt;
use egui::{Align2, Color32, CornerRadius, RichText, Sense, Stroke, vec2};

use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, icons};

/// Case-insensitive subsequence match score (higher is better); None = no match.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut ti = 0;
    let mut last = None;
    for qc in query.to_lowercase().chars() {
        let mut found = false;
        while ti < t.len() {
            if t[ti] == qc {
                score += if last == Some(ti.wrapping_sub(1)) { 8 } else { 1 };
                if ti == 0 || t[ti - 1] == ' ' {
                    score += 5;
                }
                last = Some(ti);
                ti += 1;
                found = true;
                break;
            }
            ti += 1;
        }
        if !found {
            return None;
        }
    }
    Some(score - (t.len() as i32 / 8))
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.palette_open {
        clear_native(ctx);
        return;
    }
    if !ctx.embed_viewports() {
        show_native(app, ctx);
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    // Scrim.
    egui::Area::new(egui::Id::new("palette-scrim")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let r = ui.allocate_exact_size(screen.size(), Sense::click()).1;
        ui.painter().rect_filled(screen, 0.0, Color32::from_black_alpha(110));
        if r.clicked() {
            app.ui.palette_open = false;
        }
    });
    let width = 560.0;
    let mut run: Option<String> = None;
    egui::Area::new(egui::Id::new("palette"))
        .order(egui::Order::Tooltip)
        .pivot(Align2::CENTER_TOP)
        .fixed_pos(egui::pos2(screen.center().x, screen.top() + 110.0))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(t.card)
                .stroke(Stroke::new(1.0, t.card_border))
                .corner_radius(CornerRadius::same(t.radius_lg as u8))
                .shadow(egui::Shadow { offset: [0, 18], blur: 48, spread: 0, color: t.shadow })
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    run = contents(app, ui, width);
                });
        });
    run_command(app, ctx, run);
}

fn contents(app: &mut PhotocraftApp, ui: &mut egui::Ui, width: f32) -> Option<String> {
    let ctx = ui.ctx().clone();
    let t = Tokens::get(&ctx);
    let mut run = None;
    ui.set_width(width);
    let qid = egui::Id::new("palette-query");
    let mut q: String = ui.data_mut(|d| d.get_temp(qid).unwrap_or_default());
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        icons::paint(ui, r, "search", 16.0, t.text_dim);
        let te = egui::TextEdit::singleline(&mut q)
            .hint_text(tl!("Search commands, tools and panels…"))
            .frame(egui::Frame::NONE)
            .font(egui::FontId::proportional(15.0))
            .desired_width(width - 40.0);
        let resp = ui.add(te);
        // Only when focus is missing: every request_focus interrupts the IME, and
        // doing that each frame makes Wayland input methods drop key releases (#585).
        if !resp.has_focus() {
            resp.request_focus();
        }
    });
    ui.add_space(6.0);
    crate::widgets::hairline(ui);
    ui.add_space(6.0);
    let lang = crate::i18n::Lang::from_pref(&app.session.prefs().interface.language);
    let mut hits: Vec<(i32, String, String, Option<String>, bool)> = crate::menus::menu_items(app)
        .into_iter()
        .filter_map(|m| {
            let path_en = m.path.join(" › ");
            let path = m.path.iter().map(|p| crate::i18n::tr(lang, p)).collect::<Vec<_>>().join(" › ");
            let label = crate::i18n::tr_id(lang, &m.id, &m.label);
            // Match what is shown and the English name (commands are documented in English).
            let s = fuzzy_score(&q, &format!("{label} {path} {} {path_en}", m.label))?;
            Some((
                s,
                m.id,
                label.trim_end_matches('…').to_string(),
                Some(path + &m.shortcut.map(|s| format!("   {}", crate::shortcuts::pretty(&s))).unwrap_or_default()),
                m.enabled,
            ))
        })
        .collect();
    for tool in crate::state::Tool::ALL {
        if let Some(s) = fuzzy_score(&q, &format!("{} {}", tl!(tool.label()), tool.label())) {
            hits.push((s + 2, format!("tool:{tool:?}"), tl!(tool.label()).into(), Some(format!("Tool   {}", tool.key())), true));
        }
    }
    hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.2.cmp(&b.2)));
    hits.truncate(12);
    let sel_id = egui::Id::new("palette-sel");
    let mut sel: usize = ui.data_mut(|d| d.get_temp(sel_id).unwrap_or(0));
    let (down, up, enter, esc) = ui
        .input(|i| (i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
    if down {
        sel = (sel + 1).min(hits.len().saturating_sub(1));
    }
    if up {
        sel = sel.saturating_sub(1);
    }
    sel = sel.min(hits.len().saturating_sub(1));
    if hits.is_empty() {
        ui.label(RichText::new(tl!("No matching commands")).color(t.text_faint));
    }
    for (i, (_, id, label, detail, enabled)) in hits.iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
        if i == sel || resp.hovered() {
            ui.painter().rect_filled(rect, t.radius_sm, if i == sel { t.accent_soft } else { t.hover });
        }
        let color = if *enabled { t.text } else { t.text_faint };
        ui.painter().text(rect.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, label, theme::medium(13.0), color);
        if let Some(d) = detail {
            ui.painter().text(rect.right_center() - vec2(12.0, 0.0), Align2::RIGHT_CENTER, d, egui::FontId::proportional(11.5), t.text_faint);
        }
        if resp.clicked() && *enabled {
            run = Some(id.clone());
        }
    }
    if enter && let Some(h) = hits.get(sel).filter(|h| h.4) {
        run = Some(h.1.clone());
        // dialogs::show runs later in the same frame and would read this Enter as
        // its OK button, instantly closing the dialog we are about to open (#536).
        ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
    }
    if esc {
        app.ui.palette_open = false;
    }
    ui.add_space(4.0);
    ui.label(
        RichText::new(crate::i18n::fmt(tl!("↑↓ navigate   {key} run   esc close"), &[("key", &crate::shortcuts::pretty("Enter"))])).small().color(t.text_faint),
    );
    ui.data_mut(|d| {
        d.insert_temp(qid, q);
        d.insert_temp(sel_id, sel);
    });
    run
}

fn run_command(app: &mut PhotocraftApp, ctx: &egui::Context, run: Option<String>) {
    if let Some(id) = run {
        app.ui.palette_open = false;
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("palette-query"), String::new()));
        if let Some(tool) = id.strip_prefix("tool:") {
            if let Some(t) = crate::state::Tool::from_name(tool) {
                app.ui.tool = t;
            }
        } else if let Err(e) = crate::menus::invoke(app, ctx, &id, serde_json::json!({})) {
            app.ui.status = e;
        }
    }
}

pub(crate) fn native_viewport() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("photocraft-command-palette")
}

fn native_builder(ctx: &egui::Context, first: bool) -> egui::ViewportBuilder {
    let mut builder = egui::ViewportBuilder::default()
        .with_title(tl!("Search commands, tools and panels…"))
        .with_decorations(false)
        .with_taskbar(false)
        .with_resizable(false)
        .with_minimize_button(false)
        .with_maximize_button(false)
        .with_window_level(egui::WindowLevel::Normal);
    if first {
        let root = ctx.input(|input| input.raw.viewports.get(&egui::ViewportId::ROOT).and_then(|info| info.inner_rect));
        let size = root.map_or(vec2(600.0, 520.0), |rect| (rect.size() - vec2(40.0, 80.0)).max(vec2(360.0, 220.0)).min(vec2(600.0, 520.0)));
        builder = builder.with_inner_size(size).with_active(true);
        if let Some(root) = root.filter(|rect| rect.is_finite() && rect.is_positive()) {
            builder = builder.with_position(egui::pos2(root.center().x - size.x * 0.5, root.top() + (root.height() - size.y).clamp(0.0, 110.0)));
        }
    }
    builder
}

fn clear_native(ctx: &egui::Context) {
    let opened = egui::Id::new("native-palette-opened");
    let exists = ctx.data(|data| data.get_temp::<bool>(opened)).unwrap_or(false);
    if exists {
        ctx.data_mut(|data| {
            data.remove::<bool>(opened);
            data.remove::<bool>(egui::Id::new("native-palette-focused"));
            data.get_temp_mut_or_default::<std::collections::HashMap<egui::ViewportId, u8>>(egui::Id::new("photocraft-native-window-roles"))
                .remove(&native_viewport());
        });
        ctx.send_viewport_cmd_to(native_viewport(), egui::ViewportCommand::Visible(false));
    }
}

fn show_native(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let id = native_viewport();
    let opened = egui::Id::new("native-palette-opened");
    let first = !ctx.data(|data| data.get_temp::<bool>(opened)).unwrap_or(false);
    // Search is a dismissible popup, not an application-modal setting form.
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<std::collections::HashMap<egui::ViewportId, u8>>(egui::Id::new("photocraft-native-window-roles")).insert(id, 2);
    });
    let mut run = None;
    ctx.show_viewport_immediate(id, native_builder(ctx, first), |ui, _| {
        app.collect_screenshots(ui.ctx());
        if first {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        let focused = ui.input(|input| input.viewport().focused.unwrap_or(input.focused));
        let focused_key = egui::Id::new("native-palette-focused");
        let was_focused = ui.data(|data| data.get_temp::<bool>(focused_key)).unwrap_or(false);
        if focused {
            ui.data_mut(|data| data.insert_temp(focused_key, true));
        }
        if ui.input(|input| input.viewport().close_requested()) || (!first && was_focused && !focused) {
            // Clicking another owned window or another application dismisses search.
            app.ui.palette_open = false;
            return;
        }
        egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style()).inner_margin(10.0)).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show_palette(ui, |ui| {
                run = contents(app, ui, ui.available_width());
            });
        });
    });
    ctx.data_mut(|data| data.insert_temp(opened, true));
    if run.is_some() {
        app.ui.palette_open = false;
    }
    if !app.ui.palette_open {
        clear_native(ctx);
    }
    // Invoke after returning to ROOT so the new dialog belongs to the application,
    // and this popup's Enter cannot immediately confirm the newly opened form.
    run_command(app, ctx, run);
}

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers, vec2};
    use egui_kittest::Harness;
    use serde_json::json;

    use super::fuzzy_score;
    use crate::PhotocraftApp;

    #[test]
    fn native_search_is_normal_popup_and_only_activates_when_opened() {
        let ctx = egui::Context::default();
        let first = super::native_builder(&ctx, true);
        assert_eq!(first.window_level, Some(egui::WindowLevel::Normal));
        assert_eq!(first.active, Some(true));
        assert_eq!(first.taskbar, Some(false));
        assert_eq!(first.decorations, Some(false));
        let later = super::native_builder(&ctx, false);
        assert!(later.active.is_none() && later.position.is_none() && later.inner_size.is_none());
    }

    #[test]
    fn closing_native_search_only_removes_its_popup_role() {
        let ctx = egui::Context::default();
        let roles = egui::Id::new("photocraft-native-window-roles");
        let panel = egui::ViewportId::from_hash_of("another-tool-panel");
        ctx.data_mut(|data| {
            data.insert_temp(egui::Id::new("native-palette-opened"), true);
            data.insert_temp(roles, std::collections::HashMap::from([(super::native_viewport(), 2u8), (panel, 1u8)]));
        });
        super::clear_native(&ctx);
        let retained = ctx.data(|data| data.get_temp::<std::collections::HashMap<egui::ViewportId, u8>>(roles)).unwrap();
        assert_eq!(retained.get(&panel), Some(&1));
        assert!(!retained.contains_key(&super::native_viewport()));
        assert!(ctx.data(|data| data.get_temp::<bool>(egui::Id::new("native-palette-opened"))).is_none());
    }

    #[test]
    fn open_palette_interrupts_the_ime_only_when_it_takes_focus() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        // Fonts bind on the next pass; this one opens nothing.
        ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
        app.ui.palette_open = true;
        // The field takes focus on the first pass and owns the IME from the second on.
        let interrupts: Vec<Option<bool>> = (0..4)
            .map(|_| {
                let mut out = ctx.run_ui(Default::default(), |ui| super::show(&mut app, ui.ctx()));
                out.textures_delta.clear();
                out.platform_output.ime.map(|ime| ime.should_interrupt_composition)
            })
            .collect();
        assert_eq!(interrupts, [None, Some(false), Some(false), Some(false)]);
    }

    #[test]
    fn fuzzy_ranks_prefix_and_contiguous_higher() {
        assert!(fuzzy_score("hue", "Hue/Saturation…").unwrap() > fuzzy_score("hue", "Channel Mixer Hue").unwrap_or(-99));
        assert!(fuzzy_score("gblur", "Gaussian Blur").is_some());
        assert!(fuzzy_score("xyz", "Gaussian Blur").is_none());
        assert_eq!(fuzzy_score("", "anything"), Some(0));
    }

    /// #536: Edit › Search → "Keyboard Shortcuts" must open the dialog, with and without a document.
    #[test]
    fn palette_opens_the_keyboard_shortcuts_dialog() {
        for with_doc in [false, true] {
            let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
            });
            if with_doc {
                h.state_mut().run("file.new", json!({"width": 64, "height": 64})).unwrap();
                h.state_mut().sync_views();
            }
            let ctx = h.ctx.clone();
            crate::menus::invoke(h.state_mut(), &ctx, "edit.search", json!({})).unwrap();
            h.run_steps(2);
            assert!(h.state().ui.palette_open, "Edit > Search opens the palette (with_doc={with_doc})");
            h.event(Event::Text("keyboard shortcuts".into()));
            h.run_steps(2);
            h.event(Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
            h.run_steps(3);
            let app = h.state();
            assert!(!app.ui.palette_open, "Enter closes the palette (with_doc={with_doc})");
            let dialog = app
                .ui
                .dialogs
                .iter()
                .find(|d| d.fields.get("__prefsui").and_then(|v| v.as_str()) == Some("shortcuts"))
                .map(|d| d.id)
                .unwrap_or_else(|| panic!("#536: the Keyboard Shortcuts dialog did not open (with_doc={with_doc})"));
            // The dialog keeps its own Enter: a second press is its OK and closes it.
            h.event(Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
            h.run_steps(3);
            assert!(h.state().ui.dialogs.iter().all(|d| d.id != dialog), "a second Enter closes the dialog as its OK (with_doc={with_doc})");
        }
    }
}
