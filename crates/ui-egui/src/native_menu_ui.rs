//! Desktop menu levels are owned windows, so document HWNDs cannot cover them.
use crate::theme::palette::ScrollAreaPaletteExt;
use egui::{Id, Pos2, Rect, Vec2, ViewportCommand, ViewportId, pos2, vec2};

use crate::{PhotocraftApp, menus::MenuItem};

#[derive(Clone, Default)]
struct State {
    levels: Vec<Level>,
    titles: Vec<Rect>,
    search: String,
    drag_open: bool,
    release_screen: Option<Pos2>,
    keyboard: bool,
    pointer_screen: Option<Pos2>,
}

#[derive(Clone)]
struct Level {
    path: Vec<String>,
    position: Pos2,
    selected: Option<usize>,
    opened: bool,
}

#[derive(Clone)]
enum Row {
    Item(MenuItem),
    Submenu(String),
    Separator,
}

fn state_id() -> Id {
    Id::new("photocraft-native-menu-state")
}
fn viewport(path: &[String]) -> ViewportId {
    // A native host belongs to a cascade depth, not to a menu's contents. Hashing the
    // path rebuilt a HWND and its GPU surface for every File -> Edit -> Image hover.
    // Keep one host per visible depth and patch only its geometry/content instead.
    ViewportId::from_hash_of(("photocraft-native-menu-level", path.len()))
}
pub(crate) fn is_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<State>(state_id())).is_some_and(|s| !s.levels.is_empty())
}

pub(crate) fn active_viewport(ctx: &egui::Context) -> Option<ViewportId> {
    ctx.data(|d| d.get_temp::<State>(state_id())).and_then(|s| s.levels.last().map(|l| viewport(&l.path)))
}

fn sync_hosts(ctx: &egui::Context, state: &State) {
    let key = Id::new("photocraft-native-menu-hosts");
    let current: Vec<_> = state.levels.iter().map(|l| viewport(&l.path)).collect();
    let previous = ctx.data(|d| d.get_temp::<Vec<ViewportId>>(key)).unwrap_or_default();
    for id in previous.into_iter().filter(|id| !current.contains(id)) {
        ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(false));
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<std::collections::HashMap<ViewportId, u8>>(Id::new("photocraft-native-window-roles")).remove(&id);
        });
    }
    ctx.data_mut(|d| d.insert_temp(key, current));
}

fn rows(items: &[MenuItem], path: &[String]) -> Vec<Row> {
    let mut out = Vec::new();
    let mut submenus = std::collections::HashSet::new();
    for item in items.iter().filter(|item| item.path.starts_with(path)) {
        if let Some(name) = item.path.get(path.len()) {
            if submenus.insert(name) {
                out.push(Row::Submenu(name.clone()));
            }
        } else if item.label == "---" {
            if !out.is_empty() && !matches!(out.last(), Some(Row::Separator)) {
                out.push(Row::Separator);
            }
        } else {
            out.push(Row::Item(item.clone()));
        }
    }
    while matches!(out.last(), Some(Row::Separator)) {
        out.pop();
    }
    out
}

/// A click used to dismiss a native menu belongs to the menu, including the rest of that
/// gesture. Otherwise closing the HWND hands the same press to Brush/Move in the canvas below.
pub(crate) fn observe_input(ctx: &egui::Context, raw: &mut egui::RawInput) -> bool {
    let state = ctx.data(|d| d.get_temp::<State>(state_id())).unwrap_or_default();
    let blocked_id = Id::new("photocraft-native-menu-dismissed-gestures");
    let mut blocked = ctx.data(|d| d.get_temp::<std::collections::HashSet<ViewportId>>(blocked_id)).unwrap_or_default();
    let press = raw.events.iter().find_map(|event| match event {
        egui::Event::PointerButton { pressed: true, pos, .. } | egui::Event::Touch { phase: egui::TouchPhase::Start, pos, .. } => Some(*pos),
        _ => None,
    });
    let menu_viewport = state.levels.iter().any(|level| viewport(&level.path) == raw.viewport_id);
    let chrome_popup = crate::native_popup_ui::active_viewport(ctx);
    let on_title = raw.viewport_id == ViewportId::ROOT && press.is_some_and(|p| state.titles.iter().any(|rect| rect.contains(p)));
    if (!state.levels.is_empty() || chrome_popup.is_some())
        && !menu_viewport
        && !crate::native_popup_ui::owns_viewport(ctx, raw.viewport_id)
        && !on_title
        && press.is_some()
    {
        blocked.insert(raw.viewport_id);
        ctx.data_mut(|d| d.remove::<State>(state_id()));
        crate::native_popup_ui::close(ctx);
        ctx.request_repaint_of(ViewportId::ROOT);
    }
    let swallowed = blocked.contains(&raw.viewport_id);
    if swallowed {
        let ended = raw.events.iter().any(|event| {
            matches!(
                event,
                egui::Event::PointerButton { pressed: false, .. }
                    | egui::Event::Touch { phase: egui::TouchPhase::End | egui::TouchPhase::Cancel, .. }
                    | egui::Event::WindowFocused(false)
            )
        });
        raw.events.retain(|event| {
            !matches!(
                event,
                egui::Event::PointerButton { .. }
                    | egui::Event::PointerMoved(_)
                    | egui::Event::PointerGone
                    | egui::Event::MouseMoved(_)
                    | egui::Event::Touch { .. }
                    | egui::Event::MouseWheel { .. }
            )
        });
        if ended {
            blocked.remove(&raw.viewport_id);
        }
    }
    ctx.data_mut(|d| d.insert_temp(blocked_id, blocked));
    swallowed
}

fn pointer_moved(state: &mut State, ctx: &egui::Context) -> bool {
    let point = ctx.input(|i| {
        if !i.events.iter().any(|event| matches!(event, egui::Event::PointerMoved(_) | egui::Event::Touch { phase: egui::TouchPhase::Move, .. })) {
            return None;
        }
        let local = i.pointer.hover_pos()?;
        let origin = i.viewport().inner_rect.map(|r| r.min).unwrap_or(Pos2::ZERO);
        Some((origin + local.to_vec2()) * i.pixels_per_point)
    });
    let Some(point) = point else { return false };
    let moved = state.pointer_screen.is_some_and(|old| old.distance_sq(point) >= 1.0);
    state.pointer_screen = Some(point);
    if moved {
        state.keyboard = false;
    }
    moved
}

/// Resolve in the focused menu's event stream, but dispatch later using ROOT's context. Window
/// commands such as Full Screen and dialog creation must not target the tiny menu viewport.
fn take_shortcut(app: &PhotocraftApp, ctx: &egui::Context) -> Option<String> {
    if !ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Key { pressed: true, .. }))) {
        return None;
    }
    let bindings = crate::shortcut_dispatch::bindings(app);
    ctx.input_mut(|i| {
        let (at, command) = i.events.iter().enumerate().find_map(|(at, event)| {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = event else { return None };
            bindings
                .iter()
                .find(|(_, binding)| {
                    // Text focus's policy preserves text-edit keys and bare typing while allowing
                    // command combinations and function keys in both ordinary and Help menus.
                    crate::shortcut_dispatch::Focus::Text.allows(binding) && crate::shortcuts::key_matches(binding, *key, *modifiers)
                })
                .map(|(id, _)| (at, id.clone()))
        })?;
        i.events.remove(at);
        Some(command)
    })
}

fn navigation(ctx: &egui::Context, text_editing: bool) -> (bool, bool, bool, bool, bool, bool, bool) {
    ctx.input_mut(|i| {
        // consume_key uses logical modifier matching, which can accept extra modifiers. Menu
        // navigation must match the actual event exactly, especially Alt+Space and Ctrl+Enter.
        let mut plain = |wanted| {
            let mut consumed = false;
            i.events.retain(|event| {
                let matches = matches!(event, egui::Event::Key { key, pressed: true, modifiers, .. } if *key == wanted && *modifiers == egui::Modifiers::NONE);
                consumed |= matches;
                !matches
            });
            consumed
        };
        let down = plain(egui::Key::ArrowDown);
        let up = plain(egui::Key::ArrowUp);
        let activate = plain(egui::Key::Enter) || (!text_editing && plain(egui::Key::Space));
        let right = !text_editing && plain(egui::Key::ArrowRight);
        let left = !text_editing && plain(egui::Key::ArrowLeft);
        let escape = plain(egui::Key::Escape);
        (down, up, activate, right, left, escape, i.viewport().close_requested())
    })
}

pub(crate) fn bar(_app: &mut PhotocraftApp, ui: &mut egui::Ui) -> f32 {
    let ctx = ui.ctx().clone();
    let hook = Id::new("photocraft-native-menu-input-hook");
    let installed = ctx.data(|d| d.get_temp::<bool>(hook)).unwrap_or(false);
    if !installed {
        ctx.on_begin_pass(
            "photocraft-native-menu-dismiss",
            std::sync::Arc::new(|ui| {
                let current_viewport = ui.ctx().viewport_id();
                let pending_gesture = ui.ctx().data(|d| {
                    d.get_temp::<std::collections::HashSet<ViewportId>>(Id::new("photocraft-native-menu-dismissed-gestures"))
                        .is_some_and(|blocked| blocked.contains(&current_viewport))
                });
                if !is_open(ui.ctx()) && !pending_gesture && crate::native_popup_ui::active_viewport(ui.ctx()).is_none() {
                    return;
                }
                let mut raw = ui.input(|i| i.raw.clone());
                if observe_input(ui.ctx(), &mut raw) {
                    ui.input_mut(|i| {
                        i.events = raw.events;
                        i.raw.events = i.events.clone();
                        // Immediate viewports bypass App::raw_input_hook. Their PointerState
                        // has already consumed this press, so clear it before widgets/canvas run.
                        i.pointer = egui::PointerState::default();
                    });
                }
            }),
        );
        ctx.data_mut(|d| d.insert_temp(hook, true));
    }
    let mut state = ctx.data(|d| d.get_temp::<State>(state_id())).unwrap_or_default();
    let origin = ctx.input(|i| i.viewport().inner_rect.map(|r| r.min)).unwrap_or(Pos2::ZERO);
    let moved = pointer_moved(&mut state, &ctx);
    let mut right = ui.cursor().left();
    let mut titles = Vec::new();
    state.release_screen =
        if state.drag_open && ui.input(|i| i.pointer.primary_released()) { ctx.pointer_interact_pos().map(|p| origin + p.to_vec2()) } else { None };
    egui::MenuBar::new().ui(ui, |ui| {
        crate::theme::palette::menu_scope(ui);
        ui.spacing_mut().item_spacing.x = crate::menus::TOP_MENU_GAP;
        ui.spacing_mut().button_padding = vec2(6.0, 3.0);
        for top in crate::menus::TOP_MENUS {
            let selected = state.levels.first().is_some_and(|l| l.path.first().is_some_and(|p| p == top));
            // `frame(false)` removes the content margins as well as the visual frame.
            // MenuBar supplies a transparent idle style; always keep its padding so
            // Chinese titles remain distinct and opening a menu cannot move its neighbours.
            let response = ui.add(egui::Button::new(tl!(top)).selected(selected).frame(true));
            titles.push(response.rect);
            right = right.max(response.rect.right());
            let press = ui.input(|i| i.pointer.primary_pressed()) && (response.is_pointer_button_down_on() || response.clicked());
            if press || (!state.levels.is_empty() && !selected && response.hovered() && moved) {
                crate::native_popup_ui::close(&ctx);
                if selected && press {
                    state.levels.clear();
                } else {
                    state.drag_open = press;
                    state.keyboard = false;
                    state.search.clear();
                    state.levels =
                        vec![Level { path: vec![top.to_string()], position: origin + response.rect.left_bottom().to_vec2(), selected: None, opened: false }];
                }
            }
        }
    });
    if ctx.input(|i| i.pointer.any_pressed()) && ctx.pointer_interact_pos().is_some_and(|p| !titles.iter().any(|r| r.contains(p))) {
        state.levels.clear();
    }
    state.titles = titles;
    ctx.data_mut(|d| d.insert_temp(state_id(), state));
    right
}

fn desktop_room(ctx: &egui::Context) -> Rect {
    let mut local = crate::work_area::visible_rect(ctx);
    let title_bottom = ctx.data(|d| {
        d.get_temp::<State>(state_id())
            .map(|state| state.titles.iter().filter(|rect| rect.is_finite()).map(Rect::bottom).fold(local.top(), f32::max))
            .unwrap_or(local.top())
    });
    // Long menus scroll below the main menu titles. Clamping a tall native HWND to y=0 covered
    // those titles and prevented switching menus with the pointer.
    local.min.y = title_bottom.clamp(local.top(), (local.bottom() - 1.0).max(local.top()));
    let origin = ctx.input(|i| i.viewport().inner_rect.map(|r| r.min)).unwrap_or(Pos2::ZERO);
    local.translate(origin.to_vec2())
}

fn place_menu(room: Rect, desired_position: Pos2, desired_size: Vec2) -> (Pos2, Vec2) {
    let size = vec2(desired_size.x.clamp(1.0, room.width().max(1.0)), desired_size.y.clamp(1.0, room.height().max(1.0)));
    let position = pos2(
        desired_position.x.clamp(room.left(), (room.right() - size.x).max(room.left())),
        desired_position.y.clamp(room.top(), (room.bottom() - size.y).max(room.top())),
    );
    (position, size)
}

fn builder(position: Pos2, size: Vec2, first: bool) -> egui::ViewportBuilder {
    let mut b = egui::ViewportBuilder::default()
        .with_title("InkPop menu")
        .with_decorations(false)
        .with_transparent(true)
        .with_taskbar(false)
        .with_resizable(false)
        .with_inner_size(size)
        .with_position(position)
        .with_window_level(egui::WindowLevel::Normal);
    if first {
        // The backend paints the first frame before processing Visible(true). Never
        // expose the platform's empty client area while the menu is being prepared.
        b = b.with_active(false).with_visible(false);
    }
    b
}

pub(super) fn menu_frame(style: &egui::Style) -> egui::Frame {
    // This frame fills the native client rect. Its popup shadow would extend
    // outside that rect and get cut into a rectangular halo at the rounded corners.
    egui::Frame::popup(style).inner_margin(6.0).shadow(egui::Shadow::NONE)
}

pub(super) fn row_button(ui: &egui::Ui, text: String, shortcut: String, selected: bool, checked: bool) -> egui::Button<'static> {
    // frame(false) suppresses every fill, including hover and keyboard selection.
    // Idle rows stay flat; hovered and selected rows use the current theme.
    let button = crate::menus::row_button(ui, text, checked).selected(selected).frame(true).frame_when_inactive(selected);
    if shortcut.is_empty() { button } else { button.shortcut_text(shortcut) }
}

fn menu_width(ctx: &egui::Context, list: &[Row], lang: crate::i18n::Lang, room_width: f32) -> f32 {
    let style = ctx.style_of(ctx.theme());
    let font = egui::TextStyle::Button.resolve(&style);
    let check_column = ctx.fonts_mut(|fonts| fonts.row_height(&font)).max(16.0) + 2.0 * style.spacing.icon_spacing;
    list.iter()
        .map(|row| {
            let (label, shortcut) = match row {
                Row::Item(i) => {
                    (crate::i18n::tr_id(lang, &i.id, &i.label).to_string(), i.shortcut.as_deref().map(crate::shortcuts::pretty).unwrap_or_default())
                }
                Row::Submenu(name) => (crate::i18n::tr(lang, name).to_string(), "▶".into()),
                Row::Separator => return 0.0,
            };
            ctx.fonts_mut(|fonts| {
                fonts.layout_no_wrap(label, font.clone(), egui::Color32::WHITE).size().x
                    + fonts.layout_no_wrap(shortcut, font.clone(), egui::Color32::WHITE).size().x
                    + 65.0
                    + check_column
                    + style.spacing.scroll.allocated_width()
            })
        })
        .fold(230.0, f32::max)
        .min(room_width.max(230.0))
}

fn rows_height(list: &[Row], help: bool, style: &egui::Style) -> f32 {
    // The native client must include the frame stroke as well as its padding. Omitting
    // the stroke left a 2 pt overflow even when every row fitted on the monitor.
    list.iter().map(|r| if matches!(r, Row::Separator) { 9.0 } else { 27.0 }).sum::<f32>()
        + if help { 34.0 } else { 0.0 }
        + menu_frame(style).total_margin().sum().y
}

pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.embed_viewports() {
        return;
    }
    let mut state = ctx.data(|d| d.get_temp::<State>(state_id())).unwrap_or_default();
    if app.discard.is_some() || !app.ui.dialogs.is_empty() || app.camera_raw.is_some() {
        state.levels.clear();
    }
    if state.levels.first().is_some_and(|l| l.opened) && ctx.data(|d| d.get_temp::<bool>(Id::new("photocraft-native-app-active"))) == Some(false) {
        state.levels.clear();
    }
    if state.levels.is_empty() {
        sync_hosts(ctx, &state);
        ctx.data_mut(|d| d.insert_temp(state_id(), state));
        return;
    }
    let items = crate::menus::menu_items(app);
    let room = desktop_room(ctx);
    let lang = crate::i18n::current();
    let mut clicked = None;
    let mut close = false;
    let mut switch_top = None;
    let mut depth = 0;
    // Menu paths are bounded by the command registry; cap defensive recursion anyway.
    while depth < state.levels.len().min(12) && !close && clicked.is_none() {
        let Some(level) = state.levels.get(depth).cloned() else { break };
        let help = level.path == ["Help"];
        let list = if help && !state.search.trim().is_empty() {
            crate::menus::search_items(&items, &state.search, lang).into_iter().cloned().map(Row::Item).collect()
        } else {
            rows(&items, &level.path)
        };
        let id = viewport(&level.path);
        let width = menu_width(ctx, &list, lang, room.width());
        let height = rows_height(&list, help, &ctx.style_of(ctx.theme()));
        let (position, size) = place_menu(room, level.position, vec2(width, height));
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<std::collections::HashMap<ViewportId, u8>>(Id::new("photocraft-native-window-roles")).insert(id, 2);
        });
        let mut selected = level.selected;
        let mut child = None;
        let mut leaf_hovered = false;
        let release_screen = state.release_screen;
        let hosted = ctx.data(|d| d.get_temp::<Vec<ViewportId>>(Id::new("photocraft-native-menu-hosts"))).is_some_and(|hosts| hosts.contains(&id));
        ctx.show_viewport_immediate(id, builder(position, size, !hosted), |ui, _| {
            app.collect_screenshots(ui.ctx());
            if !level.opened {
                ui.ctx().send_viewport_cmd(ViewportCommand::Visible(true));
                // Pointer switching reuses the already focused HWND. Keyboard
                // navigation back from a child still needs focus on its parent.
                if !hosted || state.keyboard {
                    ui.ctx().send_viewport_cmd(ViewportCommand::Focus);
                }
            }
            let enabled: Vec<usize> = list
                .iter()
                .enumerate()
                .filter_map(|(n, r)| match r {
                    Row::Item(i) => i.enabled.then_some(n),
                    Row::Submenu(_) => Some(n),
                    Row::Separator => None,
                })
                .collect();
            let moved = pointer_moved(&mut state, ui.ctx());
            if let Some(command) = take_shortcut(app, ui.ctx()) {
                clicked = Some((command, false));
                return;
            }
            let text_editing = help && (!level.opened || ui.ctx().text_edit_focused());
            let keys = navigation(ui.ctx(), text_editing);
            if keys.0 || keys.1 || keys.2 || keys.3 || keys.4 || keys.5 {
                state.keyboard = true;
            }
            close |= keys.5 || keys.6;
            if depth == 0 && (keys.4 || (keys.3 && selected.and_then(|n| list.get(n)).is_none_or(|r| !matches!(r, Row::Submenu(_))))) {
                switch_top = Some(if keys.4 { -1 } else { 1 });
            }
            if keys.4 && depth > 0 {
                state.levels.truncate(depth);
                if let Some(parent) = state.levels.last() {
                    ui.ctx().send_viewport_cmd_to(viewport(&parent.path), ViewportCommand::Focus);
                }
                close = false;
            }
            if (keys.0 || keys.1) && !enabled.is_empty() {
                let current = selected.and_then(|s| enabled.iter().position(|n| *n == s));
                let next = match current {
                    Some(n) if keys.0 => (n + 1) % enabled.len(),
                    Some(n) => (n + enabled.len() - 1) % enabled.len(),
                    None if keys.0 => 0,
                    None => enabled.len() - 1,
                };
                selected = enabled.get(next).copied();
                state.levels.truncate(depth + 1);
            }
            crate::theme::palette::menu_scope(ui);
            egui::CentralPanel::default().frame(menu_frame(ui.style())).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                if help {
                    let response = ui.add(egui::TextEdit::singleline(&mut state.search).hint_text(tl!("Search…")).desired_width(ui.available_width()));
                    if !level.opened {
                        response.request_focus();
                    }
                    ui.separator();
                }
                egui::ScrollArea::vertical()
                    .id_salt(&level.path)
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                    .auto_shrink([false, false])
                    .show_palette(ui, |ui| {
                        for (n, row) in list.iter().enumerate() {
                            if matches!(row, Row::Separator) {
                                ui.add(egui::Separator::default().spacing(9.0));
                                continue;
                            }
                            let (text, shortcut, enabled, checked) = match row {
                                Row::Item(i) => (
                                    crate::i18n::tr_id(lang, &i.id, &i.label).to_string(),
                                    i.shortcut.as_deref().map(crate::shortcuts::pretty).unwrap_or_default(),
                                    i.enabled,
                                    i.checked == Some(true),
                                ),
                                Row::Submenu(name) => (crate::i18n::tr(lang, name).to_string(), "▶".into(), true, false),
                                Row::Separator => continue,
                            };
                            let mut b = row_button(ui, text, shortcut, selected == Some(n), checked);
                            if let Row::Item(item) = row
                                && let Some(tint) = item.color.as_deref().and_then(crate::menus::menu_tint)
                            {
                                b = b.fill(tint.gamma_multiply(0.55));
                            }
                            let r = ui.add_enabled_ui(enabled, |ui| ui.add_sized(vec2(ui.available_width(), 27.0), b)).inner;
                            if r.hovered() && moved && !state.keyboard {
                                selected = Some(n);
                            }
                            if selected == Some(n) && (keys.0 || keys.1) {
                                r.scroll_to_me(None);
                            }
                            let released_here = release_screen.is_some_and(|p| r.rect.translate(position.to_vec2()).contains(p));
                            let hit = r.clicked() || released_here;
                            let activate = hit || (selected == Some(n) && (keys.2 || keys.3));
                            match row {
                                Row::Item(i) if enabled && (hit || (selected == Some(n) && keys.2)) => {
                                    clicked = Some((i.id.clone(), ui.input(|input| input.modifiers.alt)));
                                }
                                Row::Item(_) if r.hovered() && moved && !state.keyboard => leaf_hovered = true,
                                Row::Submenu(name) if activate || (r.hovered() && moved && !state.keyboard) => {
                                    let mut path = level.path.clone();
                                    path.push(name.clone());
                                    let x = if position.x + size.x + width > room.right() { position.x - width } else { position.x + size.x };
                                    child = Some(Level { path, position: pos2(x, position.y + r.rect.top()), selected: None, opened: false });
                                }
                                _ => {}
                            }
                        }
                    });
            });
        });
        if let Some(current) = state.levels.get_mut(depth) {
            current.selected = selected;
            current.opened = true;
        }
        if let Some(child) = child {
            if state.levels.get(depth + 1).is_none_or(|l| l.path != child.path) {
                state.levels.truncate(depth + 1);
                state.levels.push(child);
            }
        } else if leaf_hovered {
            state.levels.truncate(depth + 1);
        }
        depth += 1;
    }
    if close || clicked.is_some() {
        state.levels.clear();
    } else if let Some(direction) = switch_top {
        let index = state.levels.first().and_then(|l| l.path.first()).and_then(|top| crate::menus::TOP_MENUS.iter().position(|name| *name == top));
        if let Some(index) = index {
            let count = crate::menus::TOP_MENUS.len();
            let next = if direction < 0 { (index + count - 1) % count } else { (index + 1) % count };
            if let (Some(top), Some(rect)) = (crate::menus::TOP_MENUS.get(next), state.titles.get(next)) {
                let origin = ctx.input(|i| i.viewport().inner_rect.map(|r| r.min)).unwrap_or(Pos2::ZERO);
                state.levels = vec![Level { path: vec![top.to_string()], position: origin + rect.left_bottom().to_vec2(), selected: None, opened: false }];
            }
        }
    }
    if ctx.input(|i| !i.pointer.primary_down() && !i.pointer.primary_released()) {
        state.drag_open = false;
    }
    sync_hosts(ctx, &state);
    ctx.data_mut(|d| d.insert_temp(state_id(), state));
    if let Some((command, alt)) = clicked {
        let command = crate::menus::alt_click(command, alt);
        if let Err(error) = crate::menus::invoke(app, ctx, &command, serde_json::json!({})) {
            app.ui.status = error;
        }
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_menu_check_column_keeps_cjk_long_shortcuts_and_submenus_inside_the_measured_client() {
        for theme in crate::theme::ThemeKind::ALL {
            let ctx = egui::Context::default();
            PhotocraftApp::setup_context(&ctx, theme);
            ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
            let list = vec![
                Row::Item(MenuItem {
                    id: "test.cjk".into(),
                    label: "显示选区边缘和当前图层边界".into(),
                    path: vec!["View".into()],
                    shortcut: Some("Ctrl+Alt+Shift+F11".into()),
                    enabled: true,
                    checked: Some(true),
                    color: None,
                }),
                Row::Submenu("画布显示选项".into()),
            ];
            let width = menu_width(&ctx, &list, crate::i18n::Lang::EN, 1200.0);
            let height = rows_height(&list, false, &ctx.style_of(ctx.theme()));
            let mut metrics = (0.0, 0.0);
            for _ in 0..5 {
                let mut output =
                    ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, height))), ..Default::default() }, |ui| {
                        crate::theme::palette::menu_scope(ui);
                        egui::CentralPanel::default().frame(menu_frame(ui.style())).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let available = ui.available_width();
                            let scroll = egui::ScrollArea::vertical()
                                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                                .auto_shrink([false, false])
                                .show_palette(ui, |ui| {
                                    for row in &list {
                                        let (label, shortcut, checked) = match row {
                                            Row::Item(item) => (
                                                item.label.clone(),
                                                item.shortcut.as_deref().map(crate::shortcuts::pretty).unwrap_or_default(),
                                                item.checked == Some(true),
                                            ),
                                            Row::Submenu(name) => (name.clone(), "▶".into(), false),
                                            Row::Separator => continue,
                                        };
                                        let response = ui.add_sized(vec2(ui.available_width(), 27.0), row_button(ui, label, shortcut, false, checked));
                                        assert!((response.rect.height() - 27.0).abs() < 0.01, "normal row height must stay unchanged");
                                    }
                                });
                            metrics = (scroll.content_size.x - scroll.inner_rect.width(), available - scroll.inner_rect.width());
                        });
                    });
                output.textures_delta.clear();
            }
            assert!(metrics.0 <= 0.5, "{theme:?}: measured check column must not create horizontal overflow: {metrics:?}");
            let reserved = ctx.global_style().spacing.scroll.allocated_width();
            assert!((metrics.1 - reserved).abs() <= 0.5, "{theme:?}: fitting rows reserve a stable gutter without overflowing: {metrics:?}");
        }
    }

    #[test]
    fn native_menu_scrolls_only_when_rows_exceed_the_client_in_all_themes() {
        for theme in crate::theme::ThemeKind::ALL {
            for scale in [1.0, 1.25, 1.5, 2.0] {
                for clipped in [false, true] {
                    let ctx = egui::Context::default();
                    PhotocraftApp::setup_context(&ctx, theme);
                    ctx.set_pixels_per_point(scale);
                    let mut list = vec![Row::Submenu("Item".into()); 8];
                    list.insert(4, Row::Separator);
                    let height = rows_height(&list, false, &ctx.style_of(ctx.theme()));
                    let size = vec2(300.0, if clipped { height - 27.0 } else { height });
                    let mut metrics = (0.0, 0.0, 0.0);
                    for _ in 0..8 {
                        let mut output = ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() }, |ui| {
                            crate::theme::palette::menu_scope(ui);
                            egui::CentralPanel::default().frame(menu_frame(ui.style())).show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                let width = ui.available_width();
                                let out = egui::ScrollArea::vertical()
                                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                                    .auto_shrink([false, false])
                                    .show_palette(ui, |ui| {
                                        for row in &list {
                                            if matches!(row, Row::Separator) {
                                                ui.add(egui::Separator::default().spacing(9.0));
                                            } else {
                                                ui.add_sized(vec2(ui.available_width(), 27.0), egui::Button::new("Item"));
                                            }
                                        }
                                    });
                                metrics = (out.content_size.y - out.inner_rect.height(), width - out.inner_rect.width(), out.content_size.y);
                            });
                        });
                        output.textures_delta.clear();
                    }
                    let (overflow, gutter, content) = metrics;
                    assert!((content - 225.0).abs() < 0.1, "{theme:?}/{scale}: row measurement {content}");
                    if clipped {
                        assert!(overflow > 25.0 && gutter > 2.0, "{theme:?}/{scale}: long menu needs its scroll thumb {metrics:?}");
                    } else {
                        let reserved = ctx.global_style().spacing.scroll.allocated_width();
                        assert!(
                            overflow <= 0.5 && (gutter - reserved).abs() < 0.5,
                            "{theme:?}/{scale}: fitting menu reserves the same gutter without overflow {metrics:?}"
                        );
                    }
                }
            }
        }
    }

    fn open_file_menu(ctx: &egui::Context) {
        ctx.data_mut(|d| {
            d.insert_temp(
                state_id(),
                State {
                    levels: vec![Level { path: vec!["File".into()], position: pos2(5.0, 24.0), selected: None, opened: true }],
                    titles: vec![Rect::from_min_size(Pos2::ZERO, vec2(50.0, 24.0))],
                    ..Default::default()
                },
            )
        });
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    fn button(position: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
    }
    #[test]
    fn native_frames_have_no_rectangular_shadow_outside_the_theme_rounding() {
        let ctx = egui::Context::default();
        for theme in crate::theme::ThemeKind::ALL {
            crate::theme::apply(&ctx, theme);
            let style = ctx.style_of(ctx.theme());
            let frame = menu_frame(&style);
            assert_eq!(frame.shadow, egui::Shadow::NONE, "{theme:?}");
            assert_eq!(frame.corner_radius, style.visuals.menu_corner_radius);
            assert_eq!(frame.total_margin(), egui::Frame::popup(&style).inner_margin(6.0).total_margin());
        }
    }

    #[test]
    fn menu_viewports_never_use_global_topmost_or_taskbar() {
        let b = builder(pos2(1., 2.), vec2(250., 300.), true);
        assert_eq!(b.transparent, Some(true), "native menu alpha must use the same border-free owned-window policy as floating panels");
        assert_eq!(b.window_level, Some(egui::WindowLevel::Normal));
        assert_eq!(b.taskbar, Some(false));
        assert_eq!(b.active, Some(false));
        assert_eq!(b.visible, Some(false), "first native frame is prepared while hidden");
        assert_eq!(builder(Pos2::ZERO, vec2(250., 300.), false).active, None);
        assert_eq!(builder(Pos2::ZERO, vec2(250., 300.), false).visible, None);
    }

    #[test]
    fn horizontal_switching_reuses_one_native_host_without_hiding_or_recreating_it() {
        let ctx = egui::Context::default();
        let mut native_builder = builder(pos2(20.0, 30.0), vec2(250.0, 300.0), true);
        let mut hosts = std::collections::HashSet::new();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for (n, top) in crate::menus::TOP_MENUS.iter().cycle().take(100).enumerate() {
                let path = vec![top.to_string()];
                hosts.insert(viewport(&path));
                let state = State { levels: vec![Level { path, position: Pos2::ZERO, selected: None, opened: false }], ..Default::default() };
                sync_hosts(ui.ctx(), &state);
                let (_, recreate) = native_builder.patch(builder(pos2(20.0 + n as f32, 30.0), vec2(250.0, 100.0 + n as f32), false));
                assert!(!recreate, "switching only patches position and size");
            }
        });
        output.textures_delta.clear();
        assert_eq!(hosts.len(), 1, "100 menu switches must not create 100 native windows/surfaces");
        assert!(!output.viewport_output.values().any(|v| v.commands.iter().any(|c| matches!(c, ViewportCommand::Visible(false)))));
        let parent = viewport(&["File".into()]);
        let child = viewport(&["File".into(), "Open Recent".into()]);
        assert_ne!(parent, child, "cascade levels still require separate windows");
    }

    #[test]
    fn native_menu_rows_draw_hover_and_keyboard_selection_in_every_theme() {
        fn has_fill(shapes: &[egui::epaint::ClippedShape], fill: egui::Color32) -> bool {
            fn contains(shape: &egui::Shape, fill: egui::Color32) -> bool {
                match shape {
                    egui::Shape::Rect(rect) => rect.fill == fill,
                    egui::Shape::Vec(shapes) => shapes.iter().any(|s| contains(s, fill)),
                    _ => false,
                }
            }
            shapes.iter().any(|s| contains(&s.shape, fill))
        }
        for theme in crate::theme::ThemeKind::ALL {
            let ctx = egui::Context::default();
            PhotocraftApp::setup_context(&ctx, theme);
            let mut row_rect = Rect::NOTHING;
            for (selected, hover) in [(false, false), (false, true), (true, false)] {
                let mut painted = false;
                // egui resolves interaction from the registered previous-frame rect.
                for _ in 0..4 {
                    let events = if hover { vec![egui::Event::PointerMoved(row_rect.center())] } else { vec![egui::Event::PointerGone] };
                    let mut output = ctx.run_ui(
                        egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 100.0))), events, ..Default::default() },
                        |ui| {
                            row_rect = ui.add_sized(vec2(300.0, 27.0), row_button(ui, "Vanishing Point…".into(), "".into(), selected, false)).rect;
                        },
                    );
                    output.textures_delta.clear();
                    let style = ctx.style_of(ctx.theme());
                    let expected = if selected { style.visuals.selection.bg_fill } else { style.visuals.widgets.hovered.weak_bg_fill };
                    painted = has_fill(&output.shapes, expected);
                }
                assert_eq!(painted, selected || hover, "{theme:?}: selected={selected}, hover={hover}");
            }
        }
    }
    #[test]
    #[ignore = "GPU visual capture; run with PHOTOCRAFT_MENU_VISUAL_DIR"]
    fn native_menu_hover_visual_capture() {
        let destination = std::path::PathBuf::from(std::env::var("PHOTOCRAFT_MENU_VISUAL_DIR").expect("capture output directory"));
        std::fs::create_dir_all(&destination).unwrap();
        for theme in crate::theme::ThemeKind::ALL {
            let mut h = egui_kittest::Harness::builder().with_size(vec2(340.0, 180.0)).wgpu().build_ui_state(
                |ui, rects: &mut Vec<Rect>| {
                    rects.clear();
                    crate::theme::palette::menu_scope(ui);
                    egui::CentralPanel::default().frame(menu_frame(ui.style())).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for text in ["Camera Raw Filter…", "Vanishing Point…", "Blur", "Sharpen"] {
                            rects.push(ui.add_sized(vec2(ui.available_width(), 27.0), row_button(ui, text.into(), String::new(), false, false)).rect);
                        }
                    });
                },
                Vec::new(),
            );
            PhotocraftApp::setup_context(&h.ctx, theme);
            h.run_steps(4);
            for (name, position) in [("idle", pos2(330.0, 170.0)), ("hover", h.state()[1].center())] {
                h.hover_at(position);
                h.run_steps(4);
                h.render().unwrap().save(destination.join(format!("menu-{name}-{}.png", theme.id()))).unwrap();
            }
        }
    }

    #[test]
    fn click_on_document_dismisses_native_menu() {
        let ctx = egui::Context::default();
        ctx.data_mut(|d| {
            d.insert_temp(
                state_id(),
                State { levels: vec![Level { path: vec!["File".into()], position: Pos2::ZERO, selected: None, opened: true }], ..Default::default() },
            )
        });
        let mut raw = egui::RawInput { viewport_id: ViewportId::from_hash_of("document"), ..Default::default() };
        raw.events.push(egui::Event::PointerButton { pos: Pos2::ZERO, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        observe_input(&ctx, &mut raw);
        assert!(!is_open(&ctx));
        assert!(raw.events.is_empty(), "the dismissing press must not reach the canvas");
    }

    #[test]
    fn dismissal_consumes_pen_and_synthetic_pointer_until_release_but_keeps_next_click() {
        let ctx = egui::Context::default();
        open_file_menu(&ctx);
        let pos = pos2(200.0, 150.0);
        let device = egui::TouchDeviceId(2);
        let id = egui::TouchId(9);
        let touch = |phase| egui::Event::Touch { device_id: device, id, phase, pos, force: Some(0.5) };
        let mut raw = egui::RawInput { events: vec![touch(egui::TouchPhase::Start), button(pos, true)], ..Default::default() };
        assert!(observe_input(&ctx, &mut raw));
        assert!(raw.events.is_empty());
        raw.events = vec![touch(egui::TouchPhase::Move), egui::Event::PointerMoved(pos)];
        assert!(observe_input(&ctx, &mut raw));
        assert!(raw.events.is_empty());
        raw.events = vec![touch(egui::TouchPhase::End), button(pos, false)];
        assert!(observe_input(&ctx, &mut raw));
        assert!(raw.events.is_empty());
        raw.events = vec![button(pos, true)];
        assert!(!observe_input(&ctx, &mut raw));
        assert_eq!(raw.events.len(), 1);
    }

    #[test]
    fn clicking_a_menu_title_switches_without_starting_dismissal() {
        let ctx = egui::Context::default();
        open_file_menu(&ctx);
        let mut raw = egui::RawInput { events: vec![button(pos2(20.0, 10.0), true)], ..Default::default() };
        assert!(!observe_input(&ctx, &mut raw));
        assert!(is_open(&ctx));
        assert_eq!(raw.events.len(), 1);
    }

    #[test]
    fn help_editing_keeps_caret_keys_and_spaces_and_modified_keys_do_not_activate_rows() {
        let ctx = egui::Context::default();
        let events = vec![
            key(egui::Key::ArrowLeft, egui::Modifiers::NONE),
            key(egui::Key::ArrowRight, egui::Modifiers::NONE),
            key(egui::Key::Space, egui::Modifiers::NONE),
            key(egui::Key::Enter, egui::Modifiers::CTRL),
        ];
        let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            assert_eq!(navigation(ui.ctx(), true), (false, false, false, false, false, false, false));
            assert_eq!(ui.input(|i| i.events.len()), 4);
        });
        output.textures_delta.clear();
        let events = vec![key(egui::Key::ArrowRight, egui::Modifiers::CTRL), key(egui::Key::Space, egui::Modifiers::ALT)];
        let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            assert_eq!(navigation(ui.ctx(), false), (false, false, false, false, false, false, false));
            assert_eq!(ui.input(|i| i.events.len()), 2, "modified navigation keys remain available to their proper owner");
        });
        output.textures_delta.clear();
    }

    #[test]
    fn every_modified_menu_navigation_key_is_preserved_and_plain_keys_still_navigate() {
        let ctx = egui::Context::default();
        let keys =
            [egui::Key::ArrowDown, egui::Key::ArrowUp, egui::Key::Enter, egui::Key::Space, egui::Key::ArrowRight, egui::Key::ArrowLeft, egui::Key::Escape];
        let modifiers = [egui::Modifiers::SHIFT, egui::Modifiers::ALT, egui::Modifiers::CTRL, egui::Modifiers::COMMAND];
        let events: Vec<_> = modifiers.iter().flat_map(|mods| keys.iter().map(move |k| key(*k, *mods))).collect();
        let count = events.len();
        let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            assert_eq!(navigation(ui.ctx(), false), (false, false, false, false, false, false, false));
            assert_eq!(ui.input(|i| i.events.len()), count);
        });
        output.textures_delta.clear();
        let events = vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE), key(egui::Key::Enter, egui::Modifiers::NONE)];
        let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            assert_eq!(navigation(ui.ctx(), false), (true, false, true, false, false, false, false));
            assert!(ui.input(|i| i.events.is_empty()));
        });
        output.textures_delta.clear();
    }

    #[test]
    fn native_titles_keep_padding_and_geometry_when_open_in_every_theme() {
        for theme in crate::theme::ThemeKind::ALL {
            for language in ["en", "zh-hans"] {
                let ctx = egui::Context::default();
                PhotocraftApp::setup_context(&ctx, theme);
                let _language = crate::i18n::language_scope(crate::i18n::Lang::from_code(language).unwrap());
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                let mut render = || {
                    let mut output =
                        ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 100.0))), ..Default::default() }, |ui| {
                            bar(&mut app, ui);
                        });
                    output.textures_delta.clear();
                    ctx.data(|d| d.get_temp::<State>(state_id())).unwrap().titles
                };
                // CJK fallback loading deliberately replaces the initial placeholder font
                // over several passes. Compare selected/idle geometry after it settles.
                for _ in 0..6 {
                    render();
                }
                let idle = render();
                assert_eq!(idle.len(), crate::menus::TOP_MENUS.len());
                let font = egui::TextStyle::Button.resolve(&ctx.global_style());
                for (name, rect) in crate::menus::TOP_MENUS.iter().zip(&idle) {
                    let width = ctx.fonts_mut(|f| f.layout_no_wrap(tl!(name).into(), font.clone(), egui::Color32::WHITE).size().x);
                    assert!(rect.width() >= width + 11.5, "{theme:?}/{language}/{name}: lost inner padding");
                    assert!(rect.right() <= 640.0);
                }
                for pair in idle.windows(2) {
                    assert!(pair[1].left() - pair[0].right() >= crate::menus::TOP_MENU_GAP - 0.5);
                }
                for top in crate::menus::TOP_MENUS {
                    ctx.data_mut(|d| {
                        let mut state = d.get_temp::<State>(state_id()).unwrap();
                        state.levels = vec![Level { path: vec![top.into()], position: Pos2::ZERO, selected: None, opened: false }];
                        d.insert_temp(state_id(), state);
                    });
                    assert_eq!(render(), idle, "{theme:?}/{language}/{top}: opening must not shift neighbouring titles");
                }
            }
        }
    }

    #[test]
    fn long_native_menus_and_submenus_leave_the_root_titles_exposed() {
        let ctx = egui::Context::default();
        let origin = pos2(350.0, 200.0);
        let size = vec2(1000.0, 900.0);
        let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), ..Default::default() };
        input.viewports.entry(ViewportId::ROOT).or_default().inner_rect = Some(Rect::from_min_size(origin, size));
        let mut output = ctx.run_ui(input, |ui| {
            // Theme/font metrics can produce different actual title rectangles.
            for title_height in [24.0, 30.0, 36.0, 40.0, 44.0] {
                ui.ctx().data_mut(|d| {
                    d.insert_temp(state_id(), State { titles: vec![Rect::from_min_size(Pos2::ZERO, vec2(500.0, title_height))], ..Default::default() })
                });
                let room = desktop_room(ui.ctx());
                assert_eq!(room.top(), origin.y + title_height);
                for desired in [origin, origin + vec2(400.0, 850.0)] {
                    let (position, menu_size) = place_menu(room, desired, vec2(350.0, 2000.0));
                    assert_eq!(position.y, origin.y + title_height);
                    assert!(position.y + menu_size.y <= origin.y + size.y);
                }
            }
        });
        output.textures_delta.clear();
        let tiny_room = Rect::from_min_size(pos2(0.0, 40.0), vec2(100.0, 20.0));
        let (position, size) = place_menu(tiny_room, Pos2::ZERO, vec2(300.0, 900.0));
        assert_eq!(position, tiny_room.min);
        assert_eq!(size, tiny_room.size());
    }

    #[test]
    fn focused_native_menu_resolves_new_document_shortcut_without_executing_in_its_viewport() {
        let ctx = egui::Context::default();
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let id = viewport(&["File".to_string()]);
        let mut input = egui::RawInput { viewport_id: id, events: vec![key(egui::Key::N, egui::Modifiers::COMMAND)], ..Default::default() };
        input.viewports.entry(id).or_default();
        let mut output = ctx.run_ui(input, |ui| {
            assert_eq!(take_shortcut(&app, ui.ctx()).as_deref(), Some("file.new"));
            assert!(app.ui.dialogs.is_empty());
            assert!(take_shortcut(&app, ui.ctx()).is_none(), "the shortcut event is consumed once");
        });
        output.textures_delta.clear();
    }

    #[test]
    fn stationary_pointer_on_new_viewport_does_not_override_keyboard_navigation() {
        let ctx = egui::Context::default();
        let mut state = State { keyboard: true, pointer_screen: Some(pos2(200.0, 100.0)), ..Default::default() };
        let child = viewport(&["File".to_string(), "Open Recent".to_string()]);
        for (local, expected_moved) in [(Pos2::ZERO, false), (pos2(10.0, 0.0), true)] {
            let mut raw = egui::RawInput { viewport_id: child, events: vec![egui::Event::PointerMoved(local)], ..Default::default() };
            let info = raw.viewports.entry(child).or_default();
            info.native_pixels_per_point = Some(2.0);
            info.inner_rect = Some(Rect::from_min_size(pos2(100.0, 50.0), vec2(300.0, 200.0)));
            let mut output = ctx.run_ui(raw, |ui| {
                assert_eq!(pointer_moved(&mut state, ui.ctx()), expected_moved);
                assert_eq!(state.keyboard, !expected_moved);
            });
            output.textures_delta.clear();
        }
    }

    #[test]
    fn dismissing_menu_on_canvas_does_not_paint_but_next_gesture_does() {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", serde_json::json!({"width":32,"height":32,"background":"transparent"})).unwrap();
        app.ui.tool = crate::state::Tool::Brush;
        app.sync_views();
        let mut frame = |events: Vec<egui::Event>| {
            let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0))), events, ..Default::default() };
            observe_input(&ctx, &mut input);
            let mut output = ctx.run_ui(input, |ui| {
                crate::canvas::document_area(&mut app, ui);
            });
            output.textures_delta.clear();
            app.session.active().unwrap().revision
        };
        frame(vec![]);
        let revision = frame(vec![egui::Event::PointerMoved(pos2(180.0, 150.0))]);
        open_file_menu(&ctx);
        for events in [vec![button(pos2(180.0, 150.0), true)], vec![egui::Event::PointerMoved(pos2(220.0, 150.0))], vec![button(pos2(220.0, 150.0), false)]] {
            assert_eq!(frame(events), revision);
        }
        frame(vec![egui::Event::PointerMoved(pos2(180.0, 150.0)), button(pos2(180.0, 150.0), true)]);
        frame(vec![egui::Event::PointerMoved(pos2(220.0, 150.0))]);
        assert!(frame(vec![button(pos2(220.0, 150.0), false)]) > revision);
    }
}
