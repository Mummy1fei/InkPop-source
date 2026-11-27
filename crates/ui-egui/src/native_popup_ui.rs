//! Application-owned popups for chrome controls that must overlap floating document windows.
//! These are dismissible menus, not application-modal dialogs or OS-wide topmost windows.
use crate::theme::palette::ScrollAreaPaletteExt;
use std::collections::HashMap;

use egui::{Id, Pos2, Rect, Vec2, ViewportCommand, ViewportId};

use crate::PhotocraftApp;

#[derive(Clone)]
struct State {
    key: String,
    position: Pos2,
    room: Rect,
    opened: bool,
    seen: u64,
    child: Option<Child>,
}

#[derive(Clone)]
struct Child {
    key: String,
    position: Pos2,
    size: Vec2,
    opened: bool,
}

fn state_id() -> Id {
    Id::new("photocraft-chrome-popup")
}
fn viewport(key: &str) -> ViewportId {
    ViewportId::from_hash_of(("photocraft-chrome-popup", key))
}

/// Keep just the frequently-used Layers menu's native surface ready. A deferred,
/// hidden viewport does not paint alongside every root frame like an immediate one.
/// The caller stops declaring it while a modal, minimization or app switch owns UI.
pub(crate) fn warm_layer_context(ctx: &egui::Context) {
    if ctx.embed_viewports() || ctx.viewport_id() != ViewportId::ROOT || ctx.data(|data| data.get_temp::<State>(state_id())).is_some() {
        return;
    }
    let id = viewport("layer-context");
    let exists = ctx.input(|input| input.raw.viewports.contains_key(&id));
    ctx.show_viewport_deferred(
        id,
        egui::ViewportBuilder::default()
            .with_title("InkPop layer menu")
            .with_decorations(false)
            .with_transparent(true)
            .with_taskbar(false)
            .with_resizable(false)
            .with_window_level(egui::WindowLevel::Normal)
            .with_inner_size(egui::vec2(300.0, 600.0))
            .with_visible(false)
            .with_active(false),
        |_, _| {},
    );
    if !exists {
        ctx.request_repaint_of(id);
    }
}

pub(crate) fn is_open(ctx: &egui::Context, key: &str) -> bool {
    ctx.data(|data| data.get_temp::<State>(state_id())).is_some_and(|state| state.key == key)
}

/// Keep enabled hit geometry on the first reused frame, but reject stale
/// interactions calculated by egui before the child callback can reset input.
pub(crate) fn accepts_actions(ctx: &egui::Context) -> bool {
    ctx.viewport_id() != viewport("layer-context")
        || !ctx.data(|data| data.get_temp::<State>(state_id())).is_some_and(|state| state.key == "layer-context" && !state.opened)
}

pub(crate) fn active_viewport(ctx: &egui::Context) -> Option<ViewportId> {
    ctx.data(|data| data.get_temp::<State>(state_id())).map(|state| viewport(state.child.as_ref().map_or(&state.key, |child| &child.key)))
}

/// Both parent and flyout accept input; outside gestures dismiss the whole chain.
pub(crate) fn owns_viewport(ctx: &egui::Context, id: ViewportId) -> bool {
    ctx.data(|data| data.get_temp::<State>(state_id()))
        .is_some_and(|state| viewport(&state.key) == id || state.child.is_some_and(|child| viewport(&child.key) == id))
}

pub(crate) fn close(ctx: &egui::Context) {
    let state = ctx.data_mut(|data| {
        let state = data.get_temp::<State>(state_id());
        data.remove::<State>(state_id());
        state
    });
    if let Some(state) = state {
        for id in std::iter::once(viewport(&state.key)).chain(state.child.map(|child| viewport(&child.key))) {
            ctx.data_mut(|data| {
                data.get_temp_mut_or_default::<HashMap<ViewportId, u8>>(Id::new("photocraft-native-window-roles")).remove(&id);
            });
            ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(false));
        }
        ctx.request_repaint();
    }
}

/// `position` is in the initiating viewport's client coordinates.
pub(crate) fn open(ctx: &egui::Context, key: &str, position: Pos2, frame: u64) {
    close(ctx);
    let origin = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.min)).unwrap_or(Pos2::ZERO);
    let room = popup_room(ctx, origin);
    let state = State { key: key.to_string(), position: origin + position.to_vec2(), room, opened: false, seen: frame, child: None };
    ctx.data_mut(|data| data.insert_temp(state_id(), state));
    ctx.request_repaint_of(ViewportId::ROOT);
}

fn popup_room(ctx: &egui::Context, origin: Pos2) -> Rect {
    if ctx.viewport_id() == ViewportId::ROOT {
        return crate::work_area::visible_rect(ctx).translate(origin.to_vec2());
    }
    // Detached Layers/panel HWNDs can be very narrow. Their menus must use the display's
    // room rather than the panel's client rect; otherwise moving to a native host still clips
    // the content to precisely the same small dimensions as the old embedded menu.
    ctx.input(|input| {
        let source = input.viewport();
        if let (Some(monitor), Some(inner)) = (source.monitor_size, source.inner_rect)
            && monitor.is_finite()
            && monitor.x >= 160.0
            && monitor.y >= 160.0
            && inner.is_finite()
        {
            let center = inner.center();
            let screen_origin = egui::pos2((center.x / monitor.x).floor() * monitor.x, (center.y / monitor.y).floor() * monitor.y);
            return Rect::from_min_size(screen_origin, egui::vec2(monitor.x, (monitor.y - crate::work_area::TASKBAR).max(120.0)));
        }
        input
            .raw
            .viewports
            .get(&ViewportId::ROOT)
            .and_then(|root| root.inner_rect)
            .filter(|rect| rect.is_finite() && rect.is_positive())
            .unwrap_or_else(|| Rect::from_min_size(origin, egui::vec2(1000.0, 760.0)))
    })
}

pub(crate) fn toggle(ctx: &egui::Context, key: &str, position: Pos2, frame: u64) {
    if is_open(ctx, key) {
        close(ctx);
    } else {
        open(ctx, key, position, frame);
    }
}

/// Called after root chrome is drawn. An omitted source control must not resurrect its menu later.
pub(crate) fn cleanup(ctx: &egui::Context, frame: u64) {
    if ctx.data(|data| data.get_temp::<State>(state_id())).is_some_and(|state| state.seen < frame) {
        close(ctx);
    }
}

fn placement(room: Rect, position: Pos2, desired: Vec2) -> (Pos2, Vec2) {
    let room = if room.is_finite() && room.is_positive() { room } else { Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 760.0)) };
    let desired = if desired.is_finite() { desired.max(egui::vec2(160.0, 80.0)) } else { egui::vec2(300.0, 400.0) };
    let size = desired.min(room.size());
    let position = if position.is_finite() { position } else { room.min };
    (position.clamp(room.min, (room.max - size).max(room.min)), size)
}

fn builder(state: &State, title: &str, size: Vec2) -> egui::ViewportBuilder {
    let (position, size) = placement(state.room, state.position, size);
    let mut builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_decorations(false)
        .with_transparent(true)
        .with_taskbar(false)
        .with_resizable(false)
        .with_window_level(egui::WindowLevel::Normal);
    if !state.opened {
        // The immediate renderer presents the first menu frame before processing the
        // Visible/Focus commands below. Never expose a newly-created, unpainted HWND:
        // its system background otherwise flashes before our themed menu is ready.
        builder = builder.with_position(position).with_inner_size(size).with_visible(false).with_active(false);
    }
    builder
}

pub(super) fn menu_frame(style: &egui::Style) -> egui::Frame {
    // Unlike an embedded popup, a native popup has no room outside its client
    // rect for a blurred shadow. Keep the theme's rounded stroke, without the
    // clipped shadow pixels that appear rectangular over another floating panel.
    egui::Frame::menu(style).shadow(egui::Shadow::NONE)
}

/// `contents` returns true after completing an action. Commands changing viewport state should
/// be collected here and invoked by the caller after this returns to the source viewport.
pub(crate) fn show(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    key: &str,
    title: &str,
    size: Vec2,
    mut contents: impl FnMut(&mut PhotocraftApp, &mut egui::Ui) -> bool,
) {
    let Some(state) = ctx.data(|data| data.get_temp::<State>(state_id())).filter(|state| state.key == key) else { return };
    // An internal focus transfer precedes the document's pointer press. Keep the popup
    // registered until observe_input can dismiss it and swallow the entire gesture.
    let app_deactivated = state.opened && ctx.data(|data| data.get_temp::<bool>(Id::new("photocraft-native-app-active"))) == Some(false);
    if app.discard.is_some() || crate::dialogs::native_dialog_viewport(app).is_some() || app_deactivated {
        close(ctx);
        return;
    }
    let id = viewport(key);
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<HashMap<ViewportId, u8>>(Id::new("photocraft-native-window-roles")).insert(id, 2);
    });
    let mut dismissed = false;
    ctx.show_viewport_immediate(id, builder(&state, title, size), |ui, _| {
        if key == "layer-context" && !state.opened && !ctx.embed_viewports() {
            // A parked HWND can retain a press/release from its last invocation.
            // Clear only this child; never consume the document's opening gesture.
            ui.input_mut(|input| input.pointer = Default::default());
            ui.ctx().stop_dragging();
            ui.ctx().request_repaint();
        }
        app.collect_screenshots(ui.ctx());
        if !state.opened {
            ui.ctx().send_viewport_cmd(ViewportCommand::Visible(true));
            ui.ctx().send_viewport_cmd(ViewportCommand::Focus);
        }
        dismissed = ui.input(|input| input.viewport().close_requested() || input.key_pressed(egui::Key::Escape));
        if !dismissed {
            crate::theme::palette::menu_scope(ui);
            egui::CentralPanel::default().frame(menu_frame(ui.style())).show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                    .auto_shrink([false, false])
                    .show_palette(ui, |ui| {
                        dismissed |= contents(app, ui);
                    });
            });
        }
    });
    // Contents may have opened a child. Do not overwrite it with the pre-draw snapshot.
    ctx.data_mut(|data| {
        if let Some(mut current) = data.get_temp::<State>(state_id()).filter(|current| current.key == key) {
            current.opened = true;
            current.seen = app.frame;
            data.insert_temp(state_id(), current);
        }
    });
    if dismissed {
        close(ctx);
    }
}

/// Open next to a row in the current popup, flipping left at the work area's edge.
pub(crate) fn open_child(ctx: &egui::Context, parent: &str, child: &str, anchor: Rect, desired: Vec2) {
    let Some(mut state) = ctx.data(|data| data.get_temp::<State>(state_id())).filter(|state| state.key == parent) else { return };
    let key = format!("{parent}/{child}");
    if state.child.as_ref().is_some_and(|current| current.key == key) {
        return;
    }
    close_child(ctx);
    let origin = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.min)).unwrap_or(Pos2::ZERO);
    let anchor = anchor.translate(origin.to_vec2());
    let x = if anchor.right() + desired.x > state.room.right() { anchor.left() - desired.x } else { anchor.right() };
    let (position, size) = placement(state.room, egui::pos2(x, anchor.top()), desired);
    state.child = Some(Child { key, position, size, opened: false });
    ctx.data_mut(|data| data.insert_temp(state_id(), state));
    ctx.request_repaint();
}

pub(crate) fn close_child(ctx: &egui::Context) {
    let Some(mut state) = ctx.data(|data| data.get_temp::<State>(state_id())) else { return };
    if let Some(child) = state.child.take() {
        let id = viewport(&child.key);
        ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(false));
        ctx.data_mut(|data| {
            data.get_temp_mut_or_default::<HashMap<ViewportId, u8>>(Id::new("photocraft-native-window-roles")).remove(&id);
            data.insert_temp(state_id(), state);
        });
    }
}

/// A flyout is a separate normal owned viewport, not clipped to its parent's HWND.
/// Escape/Left return to the parent; a completed action dismisses the whole chain.
pub(crate) fn show_child(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    parent: &str,
    title: &str,
    mut contents: impl FnMut(&mut PhotocraftApp, &mut egui::Ui) -> bool,
) {
    let Some(state) = ctx.data(|data| data.get_temp::<State>(state_id())).filter(|state| state.key == parent) else { return };
    let Some(child) = state.child.clone() else { return };
    let id = viewport(&child.key);
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<HashMap<ViewportId, u8>>(Id::new("photocraft-native-window-roles")).insert(id, 2);
    });
    let child_state = State { key: child.key.clone(), position: child.position, room: state.room, opened: child.opened, seen: app.frame, child: None };
    let mut completed = false;
    let mut back = false;
    ctx.show_viewport_immediate(id, builder(&child_state, title, child.size), |ui, _| {
        app.collect_screenshots(ui.ctx());
        if !child.opened {
            ui.ctx().send_viewport_cmd(ViewportCommand::Visible(true));
            ui.ctx().send_viewport_cmd(ViewportCommand::Focus);
        }
        back = ui.input(|input| input.viewport().close_requested() || input.key_pressed(egui::Key::Escape) || input.key_pressed(egui::Key::ArrowLeft));
        if !back {
            crate::theme::palette::menu_scope(ui);
            egui::CentralPanel::default().frame(menu_frame(ui.style())).show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
                    .show_palette(ui, |ui| completed = contents(app, ui));
            });
        }
    });
    if completed {
        close(ctx);
    } else if back {
        close_child(ctx);
        ctx.send_viewport_cmd_to(viewport(parent), ViewportCommand::Focus);
    } else {
        ctx.data_mut(|data| {
            if let Some(mut current) = data.get_temp::<State>(state_id()).filter(|current| current.key == parent) {
                if let Some(child) = &mut current.child {
                    child.opened = true;
                }
                data.insert_temp(state_id(), current);
            }
        });
    }
}

#[cfg(test)]
#[path = "native_popup_warm_tests.rs"]
mod warm_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_popup_frames_keep_theme_rounding_without_a_clipped_shadow() {
        let ctx = egui::Context::default();
        for theme in crate::theme::ThemeKind::ALL {
            crate::theme::apply(&ctx, theme);
            let style = ctx.style_of(ctx.theme());
            let original = egui::Frame::menu(&style);
            let frame = menu_frame(&style);
            assert_eq!(frame.shadow, egui::Shadow::NONE, "{theme:?}");
            assert_eq!(frame.corner_radius, original.corner_radius);
            assert_eq!(frame.stroke, original.stroke);
            assert_eq!(frame.total_margin(), original.total_margin());
        }
    }

    #[test]
    fn detached_panel_context_menu_uses_monitor_room_not_the_small_panel_client() {
        let ctx = egui::Context::default();
        let source = ViewportId::from_hash_of("detached-layers");
        let mut input =
            egui::RawInput { viewport_id: source, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(250.0, 180.0))), ..Default::default() };
        input.viewports.insert(
            source,
            egui::ViewportInfo {
                monitor_size: Some(egui::vec2(1600.0, 900.0)),
                inner_rect: Some(Rect::from_min_size(egui::pos2(-1400.0, 200.0), egui::vec2(250.0, 180.0))),
                ..Default::default()
            },
        );
        let mut output = ctx.run_ui(input, |ui| open(ui.ctx(), "layer-context", egui::pos2(200.0, 160.0), 0));
        output.textures_delta.clear();
        let state = ctx.data(|data| data.get_temp::<State>(state_id())).unwrap();
        assert_eq!(state.position, egui::pos2(-1200.0, 360.0));
        assert_eq!(state.room, Rect::from_min_size(egui::pos2(-1600.0, 0.0), egui::vec2(1600.0, 852.0)));
        let (_, size) = placement(state.room, state.position, egui::vec2(300.0, 700.0));
        assert_eq!(size, egui::vec2(300.0, 700.0));
    }

    #[test]
    fn flyout_keeps_parent_input_and_outside_dismissal_swallows_the_complete_gesture() {
        let ctx = egui::Context::default();
        open(&ctx, "layer-context", Pos2::ZERO, 0);
        open_child(&ctx, "layer-context", "color", Rect::from_min_size(Pos2::ZERO, egui::vec2(240.0, 27.0)), egui::vec2(180.0, 232.0));
        let parent = viewport("layer-context");
        let child = viewport("layer-context/color");
        assert_eq!(active_viewport(&ctx), Some(child));
        for id in [parent, child] {
            let mut input = egui::RawInput {
                viewport_id: id,
                events: vec![egui::Event::PointerButton {
                    pos: Pos2::ZERO,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            };
            assert!(owns_viewport(&ctx, id));
            assert!(!crate::native_menu_ui::observe_input(&ctx, &mut input));
            assert_eq!(input.events.len(), 1);
            assert!(is_open(&ctx, "layer-context"));
        }
        let panel = ViewportId::from_hash_of("floating-tool-panel");
        for pressed in [true, false] {
            let mut input = egui::RawInput {
                viewport_id: panel,
                events: vec![egui::Event::PointerButton { pos: Pos2::ZERO, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }],
                ..Default::default()
            };
            assert!(crate::native_menu_ui::observe_input(&ctx, &mut input));
            assert!(input.events.is_empty());
            assert!(active_viewport(&ctx).is_none());
        }
    }

    #[test]
    fn child_opened_while_rendering_survives_parent_update_and_closes_with_its_roles() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let panel = ViewportId::from_hash_of("unrelated-panel");
        ctx.data_mut(|data| data.insert_temp(Id::new("photocraft-native-window-roles"), HashMap::from([(panel, 1u8)])));
        open(&ctx, "layer-context", Pos2::ZERO, 0);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            show(&mut app, ui.ctx(), "layer-context", "Layers", egui::vec2(240.0, 400.0), |_, ui| {
                let response = ui.button("Layer Color");
                open_child(ui.ctx(), "layer-context", "color", response.rect, egui::vec2(180.0, 232.0));
                false
            });
            show_child(&mut app, ui.ctx(), "layer-context", "Layer Color", |_, ui| ui.button("Red").clicked());
        });
        output.textures_delta.clear();
        assert_eq!(active_viewport(&ctx), Some(viewport("layer-context/color")));
        let roles = ctx.data(|data| data.get_temp::<HashMap<ViewportId, u8>>(Id::new("photocraft-native-window-roles"))).unwrap();
        assert_eq!(roles.get(&viewport("layer-context")), Some(&2));
        assert_eq!(roles.get(&viewport("layer-context/color")), Some(&2));
        close_child(&ctx);
        assert_eq!(active_viewport(&ctx), Some(viewport("layer-context")));
        open_child(&ctx, "layer-context", "color", Rect::ZERO, egui::vec2(180.0, 232.0));
        cleanup(&ctx, 1);
        assert!(active_viewport(&ctx).is_none());
        assert_eq!(
            ctx.data(|data| data.get_temp::<HashMap<ViewportId, u8>>(Id::new("photocraft-native-window-roles"))).unwrap(),
            HashMap::from([(panel, 1u8)])
        );
    }

    fn popup_frame(app: &mut PhotocraftApp, ctx: &egui::Context, focused: bool, events: Vec<egui::Event>) {
        let mut input = egui::RawInput { focused, events, ..Default::default() };
        input.viewports.get_mut(&ViewportId::ROOT).unwrap().focused = Some(focused);
        let mut output = ctx.run_ui(input, |ui| {
            show(app, ui.ctx(), "workspace", "Workspace", egui::vec2(280.0, 400.0), |_, ui| {
                ui.label("Layouts");
                false
            });
        });
        output.textures_delta.clear();
        app.frame += 1;
    }

    #[test]
    fn internal_focus_loss_preserves_popup_until_outside_gesture_is_swallowed() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        ctx.data_mut(|data| data.insert_temp(Id::new("photocraft-native-app-active"), true));
        open(&ctx, "workspace", Pos2::ZERO, app.frame);
        popup_frame(&mut app, &ctx, true, vec![]);
        popup_frame(&mut app, &ctx, false, vec![egui::Event::WindowFocused(false)]);
        assert_eq!(active_viewport(&ctx), Some(viewport("workspace")));

        let document = ViewportId::from_hash_of("floating-document");
        for pressed in [true, false] {
            let mut input = egui::RawInput {
                viewport_id: document,
                events: vec![egui::Event::PointerButton {
                    pos: egui::pos2(50.0, 50.0),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            };
            assert!(crate::native_menu_ui::observe_input(&ctx, &mut input));
            assert!(input.events.is_empty());
            assert!(active_viewport(&ctx).is_none());
        }
        let mut next = egui::RawInput {
            viewport_id: document,
            events: vec![egui::Event::PointerButton {
                pos: egui::pos2(50.0, 50.0),
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            }],
            ..Default::default()
        };
        assert!(!crate::native_menu_ui::observe_input(&ctx, &mut next));
        assert_eq!(next.events.len(), 1);
    }

    #[test]
    fn external_app_switch_and_escape_still_close_popup() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        open(&ctx, "workspace", Pos2::ZERO, app.frame);
        popup_frame(&mut app, &ctx, true, vec![]);
        ctx.data_mut(|data| data.insert_temp(Id::new("photocraft-native-app-active"), false));
        popup_frame(&mut app, &ctx, false, vec![]);
        assert!(active_viewport(&ctx).is_none());

        ctx.data_mut(|data| data.insert_temp(Id::new("photocraft-native-app-active"), true));
        open(&ctx, "workspace", Pos2::ZERO, app.frame);
        popup_frame(&mut app, &ctx, true, vec![]);
        popup_frame(
            &mut app,
            &ctx,
            true,
            vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
        );
        assert!(active_viewport(&ctx).is_none());
    }

    #[test]
    fn popup_stays_inside_available_room_including_negative_monitor_coordinates() {
        let room = Rect::from_min_size(egui::pos2(-1600.0, 40.0), egui::vec2(1000.0, 700.0));
        let (position, size) = placement(room, egui::pos2(-610.0, 700.0), egui::vec2(400.0, 500.0));
        assert!(room.contains_rect(Rect::from_min_size(position, size)));
        assert_eq!(position, egui::pos2(-1000.0, 240.0));
        let (position, size) = placement(room, Pos2::ZERO, egui::vec2(4000.0, 3000.0));
        assert_eq!(Rect::from_min_size(position, size), room);
    }
    #[test]
    fn popup_uses_normal_owned_band_and_only_activates_once() {
        let state = State {
            key: "test".into(),
            position: Pos2::ZERO,
            room: Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0)),
            opened: false,
            seen: 0,
            child: None,
        };
        let first = builder(&state, "Menu", egui::vec2(300.0, 400.0));
        assert_eq!(first.window_level, Some(egui::WindowLevel::Normal));
        assert_eq!(first.taskbar, Some(false));
        assert_eq!(first.transparent, Some(true), "rounded client corners must expose alpha instead of an opaque native window edge");
        assert_eq!(first.active, Some(false));
        assert_eq!(first.visible, Some(false), "the HWND must stay hidden until the renderer has painted the menu");
        let later = builder(&State { opened: true, ..state }, "Menu", egui::vec2(300.0, 400.0));
        assert!(later.active.is_none() && later.visible.is_none() && later.position.is_none() && later.inner_size.is_none());
    }
    #[test]
    fn omitted_source_closes_popup_without_removing_other_roles() {
        let ctx = egui::Context::default();
        open(&ctx, "tools", Pos2::ZERO, 3);
        let other = ViewportId::from_hash_of("panel");
        ctx.data_mut(|data| data.insert_temp(Id::new("photocraft-native-window-roles"), HashMap::from([(viewport("tools"), 2u8), (other, 1u8)])));
        cleanup(&ctx, 4);
        assert!(!is_open(&ctx, "tools"));
        assert_eq!(
            ctx.data(|data| data.get_temp::<HashMap<ViewportId, u8>>(Id::new("photocraft-native-window-roles"))).unwrap(),
            HashMap::from([(other, 1u8)])
        );
    }
}
