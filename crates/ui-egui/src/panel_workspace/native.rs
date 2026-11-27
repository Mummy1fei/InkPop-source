//! Native hosts for floating tool groups. Their ordinary application-local window role is above
//! document windows; no tool panel enters the system's always-on-top band.

use std::collections::HashMap;

use egui::{Id, Pos2, Rect, Sense, Vec2, ViewportCommand, ViewportId, pos2, vec2};

use super::{Container, Payload};
use crate::{PhotocraftApp, theme::Tokens};

const ROLES: &str = "photocraft-native-window-roles";
const HOSTS: &str = "photocraft-native-panel-hosts";
const AUX_HOSTS: &str = "photocraft-native-auxiliary-hosts";
const FOCUS_REQUESTS: &str = "photocraft-native-panel-focus-requests";
const SHORTCUTS: &str = "photocraft-native-panel-pending-shortcuts";
const FRAME_GUTTER: f32 = 6.0;
pub(crate) const PANEL_MIN_SIZE: Vec2 = vec2(250.0, 180.0);
const AUX_MIN_SIZE: Vec2 = vec2(160.0, 96.0);

fn viewport(container: u64, epoch: u64) -> ViewportId {
    ViewportId::from_hash_of(("native-panel-container", container, epoch))
}

/// Resolve a currently displayed panel stack for screenshots without opening or restoring it.
pub(crate) fn active_viewport(app: &PhotocraftApp, container: u64) -> Option<ViewportId> {
    let c = app.ui.dock.containers.iter().find(|c| c.id == container)?;
    let root = c.stack.unwrap_or(c.id);
    super::floating_host_visible(app, root).then(|| viewport(root, app.ui.shell.layout_epoch))
}

/// Explicit editor requests may raise a restored panel once; ordinary rendering must not.
pub(crate) fn request_group_focus(app: &PhotocraftApp, ctx: &egui::Context, group: super::Group) {
    let roots: Vec<u64> = app
        .ui
        .dock
        .containers
        .iter()
        .filter(|c| !c.closed && !c.collapsed && c.tabs.iter().any(|t| t.group == group))
        .map(|c| c.stack.unwrap_or(c.id))
        .filter(|root| super::floating_host_visible(app, *root))
        .collect();
    for root in roots {
        if ctx.embed_viewports() {
            let mut id = Id::new(("panel-container", root));
            if app.ui.shell.layout_epoch != 0 {
                id = id.with(app.ui.shell.layout_epoch);
            }
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, id));
        } else {
            let id = viewport(root, app.ui.shell.layout_epoch);
            ctx.data_mut(|d| d.get_temp_mut_or_default::<HashMap<ViewportId, ()>>(Id::new(FOCUS_REQUESTS)).insert(id, ()));
        }
    }
}

fn take_focus_request(ctx: &egui::Context, id: ViewportId) -> bool {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<HashMap<ViewportId, ()>>(Id::new(FOCUS_REQUESTS)).remove(&id).is_some())
}

fn scale(ctx: &egui::Context, viewport: ViewportId) -> f32 {
    ctx.input(|i| i.raw.viewports.get(&viewport).and_then(|v| v.native_pixels_per_point))
        .map(|native| native * ctx.zoom_factor())
        .filter(|ppp| ppp.is_finite() && *ppp > 0.0)
        .unwrap_or_else(|| ctx.pixels_per_point())
}

fn root_origin_px(ctx: &egui::Context) -> Pos2 {
    ctx.input(|i| i.raw.viewports.get(&ViewportId::ROOT).and_then(|v| v.inner_rect).map(|r| r.min)).unwrap_or(Pos2::ZERO) * scale(ctx, ViewportId::ROOT)
}

fn from_screen(ctx: &egui::Context, screen: Pos2) -> Pos2 {
    ((screen - root_origin_px(ctx)) / scale(ctx, ViewportId::ROOT)).to_pos2()
}

pub(super) fn root_point(ctx: &egui::Context, local: Pos2) -> Pos2 {
    if ctx.embed_viewports() || ctx.viewport_id() == ViewportId::ROOT {
        return local;
    }
    let origin = ctx.input(|i| i.viewport().inner_rect.map(|r| r.min)).unwrap_or(Pos2::ZERO);
    from_screen(ctx, (origin + local.to_vec2()) * ctx.pixels_per_point())
}

pub(super) fn root_rect(ctx: &egui::Context, rect: Rect) -> Rect {
    Rect::from_min_max(root_point(ctx, rect.min), root_point(ctx, rect.max))
}

fn set_role(ctx: &egui::Context, viewport: ViewportId, role: Option<u8>) {
    ctx.data_mut(|d| {
        let mut roles = d.get_temp::<HashMap<ViewportId, u8>>(Id::new(ROLES)).unwrap_or_default();
        if let Some(role) = role {
            roles.insert(viewport, role);
        } else {
            roles.remove(&viewport);
        }
        d.insert_temp(Id::new(ROLES), roles);
    });
}

pub(super) fn cleanup(ctx: &egui::Context, roots: &[Container], epoch: u64, frame: u64) {
    let old = ctx.data(|d| d.get_temp::<Vec<ViewportId>>(Id::new(HOSTS))).unwrap_or_default();
    let next: Vec<_> = if ctx.embed_viewports() { vec![] } else { roots.iter().map(|c| viewport(c.id, epoch)).collect() };
    for id in old.into_iter().filter(|id| !next.contains(id)) {
        set_role(ctx, id, None);
        ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(false));
        ctx.send_viewport_cmd_to(id, ViewportCommand::Close);
    }
    ctx.data_mut(|d| d.insert_temp(Id::new(HOSTS), next));
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<HashMap<ViewportId, ()>>(Id::new(FOCUS_REQUESTS)).retain(|id, _| roots.iter().any(|root| viewport(root.id, epoch) == *id));
    });
    // Auxiliary windows have independent open flags. A window omitted by its owner disappears
    // from eframe immediately; retire its bookkeeping on the next root pass as well.
    let mut auxiliary = ctx.data(|d| d.get_temp::<HashMap<ViewportId, u64>>(Id::new(AUX_HOSTS))).unwrap_or_default();
    auxiliary.retain(|id, seen| {
        let keep = !ctx.embed_viewports() && seen.saturating_add(1) >= frame;
        if !keep {
            set_role(ctx, *id, None);
        }
        keep
    });
    ctx.data_mut(|d| d.insert_temp(Id::new(AUX_HOSTS), auxiliary));
}

fn bounded(value: f32, fallback: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() { value.clamp(min, max) } else { fallback }
}

/// Fit a floating window to the OS work area in physical pixels, then let its contents scroll.
fn fit_panel_to_work_area(area: &crate::MonitorWorkAreaFn, panel: Rect, ppp: f32) -> Rect {
    if !panel.is_finite() || !panel.is_positive() || !ppp.is_finite() || ppp <= 0.0 {
        return panel;
    }
    let Some(area) = area(panel.center() * ppp).filter(|r| r.is_finite() && r.is_positive()) else { return panel };
    let area = Rect::from_min_max(area.min / ppp, area.max / ppp);
    let size = panel.size().min(area.size());
    let max = (area.max - size).max(area.min);
    Rect::from_min_size(panel.min.clamp(area.min, max), size)
}

fn builder(ctx: &egui::Context, root: &Container, title: &str, first: bool) -> egui::ViewportBuilder {
    let mut builder = egui::ViewportBuilder::default()
        .with_title(title)
        .with_decorations(false)
        .with_transparent(true)
        .with_has_shadow(false)
        .with_resizable(true)
        .with_min_inner_size(PANEL_MIN_SIZE)
        .with_taskbar(false)
        .with_minimize_button(false)
        .with_maximize_button(false)
        .with_window_level(egui::WindowLevel::Normal);
    if first {
        let relative = vec2(bounded(root.position[0], 100.0, -16000.0, 16000.0), bounded(root.position[1], 100.0, -16000.0, 16000.0));
        let size = vec2(bounded(root.size[0], 360.0, 250.0, 2000.0), bounded(root.size[1], 500.0, 180.0, 2000.0));
        builder = builder
            .with_position(root_origin_px(ctx) / scale(ctx, ViewportId::ROOT) + relative)
            .with_inner_size(size)
            // Opening a panel does not steal focus from an in-progress tool gesture.
            .with_active(false);
    }
    builder
}

#[derive(Clone, Copy)]
struct Drag {
    offset_px: Vec2,
    contact: Option<(egui::TouchDeviceId, egui::TouchId)>,
    native: bool,
    start_outer: Pos2,
    last: Pos2,
}

fn drag_title(app: &mut PhotocraftApp, ui: &mut egui::Ui, root: &Container, rect: Rect, dockable: bool) {
    let ctx = ui.ctx().clone();
    // Context accessors acquire the same lock as input/data closures. Sample once
    // outside those closures, including the pen fallback path below.
    let pixels_per_point = ctx.pixels_per_point();
    let id = Id::new(("native-panel-drag", ctx.viewport_id()));
    if app.session.prefs().workspace_locked {
        ctx.data_mut(|d| d.remove::<Drag>(id));
        super::clear_drag_point(&ctx);
        return;
    }
    let response = ui.interact(rect, id.with("header"), Sense::click_and_drag());
    let touch_start = ctx.input(|i| {
        i.events.iter().find_map(|e| match e {
            egui::Event::Touch { device_id, id, phase: egui::TouchPhase::Start, pos, .. } if rect.contains(*pos) => Some((*pos, (*device_id, *id))),
            _ => None,
        })
    });
    let pointer_start = ctx.input(|i| i.pointer.primary_pressed().then(|| i.pointer.press_origin()).flatten()).filter(|p| rect.contains(*p));
    if ctx.data(|d| d.get_temp::<Drag>(id)).is_none()
        && let Some(local) = touch_start.map(|(p, _)| p).or(pointer_start).filter(|p| !crate::titlebar::frame_resize_contains(&ctx, *p, FRAME_GUTTER))
    {
        let outer = ctx.input(|i| i.viewport().outer_rect.map(|r| r.min)).unwrap_or(Pos2::ZERO);
        let contact = touch_start.map(|(_, c)| c);
        let native = if let Some((_, contact)) = contact {
            app.services.contact_screen_pos.as_mut().and_then(|read| read(contact.0, true))
        } else {
            app.services.screen_cursor_pos.as_mut().and_then(|read| read())
        };
        let screen = native.unwrap_or((outer + local.to_vec2()) * pixels_per_point);
        ctx.data_mut(|d| {
            d.insert_temp(id, Drag { offset_px: screen - outer * pixels_per_point, contact, native: native.is_some(), start_outer: outer, last: outer });
        });
    }
    let Some(mut drag) = ctx.data(|d| d.get_temp::<Drag>(id)) else { return };
    let (ended, cancelled) = ctx.input(|i| {
        // DragAndDrop consumes Escape during begin_pass; the raw copy still records it.
        let cancelled = i.raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. }))
            || i.events.iter().any(|e| match e {
                egui::Event::WindowFocused(false) => true,
                egui::Event::Touch { device_id, id, phase: egui::TouchPhase::Cancel, .. } => drag.contact == Some((*device_id, *id)),
                _ => false,
            });
        let ended = drag.contact.map_or(!i.pointer.primary_down(), |contact| {
            i.events.iter().any(|e| matches!(e, egui::Event::Touch { device_id, id, phase: egui::TouchPhase::End, .. } if contact == (*device_id, *id)))
        });
        (ended, cancelled)
    });
    let screen = if drag.native {
        if let Some((_, contact)) = drag.contact {
            app.services.contact_screen_pos.as_mut().and_then(|read| read(contact.0, false))
        } else {
            app.services.screen_cursor_pos.as_mut().and_then(|read| read())
        }
    } else {
        ctx.input(|i| {
            i.events
                .iter()
                .rev()
                .find_map(|e| match e {
                    egui::Event::Touch { device_id, id, phase: egui::TouchPhase::Move | egui::TouchPhase::End, pos, .. }
                        if drag.contact == Some((*device_id, *id)) =>
                    {
                        Some(*pos)
                    }
                    egui::Event::PointerMoved(pos) if drag.contact.is_none() => Some(*pos),
                    _ => None,
                })
                .map(|local| {
                    let outer = i.viewport().outer_rect.map(|r| r.min).unwrap_or(drag.start_outer);
                    (outer + local.to_vec2()) * pixels_per_point
                })
        })
    };
    if !cancelled && let Some(screen) = screen {
        if dockable && egui::DragAndDrop::payload::<Payload>(&ctx).is_some_and(|payload| payload.container == root.id) {
            super::set_drag_point(&ctx, root.id, from_screen(&ctx, screen), true);
        }
        let target = (screen - drag.offset_px) / pixels_per_point;
        if target.is_finite() && target.distance_sq(drag.last) * pixels_per_point.powi(2) >= 1.0 {
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(target));
            drag.last = target;
            if dockable {
                egui::DragAndDrop::set_payload(&ctx, Payload { container: root.id, tab: None });
                ctx.data_mut(|d| d.insert_temp(Id::new("panel-drag-whole-stack"), root.id));
                super::set_drag_point(&ctx, root.id, from_screen(&ctx, screen), true);
            }
        }
        if ended && dockable {
            queue_drop(&ctx, from_screen(&ctx, screen));
        }
    }
    if ended || cancelled {
        ctx.data_mut(|d| d.remove::<Drag>(id));
        if cancelled {
            egui::DragAndDrop::clear_payload(&ctx);
            super::clear_drag_point(&ctx);
            ctx.data_mut(|data| {
                data.remove::<u64>(Id::new("panel-drag-whole-stack"));
                data.remove::<u64>(Id::new("panel-title-drag-origin"));
            });
        }
        ctx.request_repaint_of(ViewportId::ROOT);
    } else {
        ctx.data_mut(|d| d.insert_temp(id, drag));
        ctx.request_repaint_after(std::time::Duration::from_millis(8));
    }
    if response.hovered() || response.dragged() {
        ctx.set_cursor_icon(egui::CursorIcon::Default);
    }
}

fn host(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    root: &Container,
    title: &str,
    dockable: bool,
    mut body: impl FnMut(&mut PhotocraftApp, &mut egui::Ui),
) -> (bool, Option<Rect>) {
    let id = viewport(root.id, app.ui.shell.layout_epoch);
    let focus_requested = take_focus_request(ctx, id);
    let mut geometry_result = None;
    let first = ctx.input(|i| i.raw.viewports.get(&id).is_none_or(|v| v.inner_rect.is_none()));
    set_role(ctx, id, Some(1));
    let mut close = false;
    let mut viewport_builder = builder(ctx, root, tl!(title), first);
    if !dockable {
        viewport_builder = viewport_builder.with_min_inner_size(AUX_MIN_SIZE);
        if first {
            viewport_builder = viewport_builder.with_inner_size(vec2(bounded(root.size[0], 360.0, 160.0, 2000.0), bounded(root.size[1], 400.0, 96.0, 2000.0)));
        }
    }
    if let Some(area) = &app.services.monitor_work_area
        && let (Some(position), Some(size)) = (viewport_builder.position, viewport_builder.inner_size)
    {
        let panel = fit_panel_to_work_area(area, Rect::from_min_size(position, size), scale(ctx, ViewportId::ROOT));
        viewport_builder = viewport_builder.with_position(panel.min).with_inner_size(panel.size());
    }
    ctx.show_viewport_immediate(id, viewport_builder, |ui, _| {
        app.collect_screenshots(ui.ctx());
        let ctx = ui.ctx().clone();
        if focus_requested {
            ctx.send_viewport_cmd(ViewportCommand::Focus);
        }
        close = ctx.input(|i| i.viewport().close_requested());
        let t = Tokens::get(&ctx);
        let frame = egui::Frame::NONE
            .fill(t.card)
            .stroke(egui::Stroke::new(1.0, t.card_border))
            .corner_radius(egui::CornerRadius::same(t.radius_lg.clamp(0.0, 255.0) as u8))
            .shadow(egui::Shadow { offset: [0, 2], blur: 8, spread: 0, color: t.shadow })
            .inner_margin(6);
        let card = ui.max_rect().shrink(FRAME_GUTTER);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(card));
        frame.show(&mut child, |ui| {
            ui.set_min_size(ui.available_size());
            let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
            if let Some(fill) = crate::theme::palette::override_color(ui.ctx(), photocraft_engine::prefs::palette::ColorSlot::FloatingTitle) {
                ui.painter().rect_filled(bar, t.radius_sm, fill);
            }
            ui.painter().with_clip_rect(Rect::from_min_max(bar.min, bar.max - vec2(28.0, 0.0))).text(
                bar.left_center() + vec2(4.0, 0.0),
                egui::Align2::LEFT_CENTER,
                tl!(title),
                crate::theme::medium(12.0),
                t.text,
            );
            let close_rect = Rect::from_min_size(pos2(bar.right() - 24.0, bar.top()), vec2(24.0, bar.height()));
            let button = ui.interact(close_rect, Id::new(("native-panel-close", root.id)), Sense::click());
            crate::icons::button_chrome(ui, close_rect, false, button.hovered());
            crate::icons::paint(ui, close_rect, "x", 13.0, t.icon);
            button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Close")));
            close |= button.on_hover_text(tl!("Close")).clicked();
            drag_title(app, ui, root, Rect::from_min_max(bar.min, close_rect.left_bottom()), dockable);
            body(app, ui);
        });
        crate::hold_keys::publish_panel_alt(app, &ctx);
        collect_shortcuts(app, &ctx);
        resize_frame(app, ui, dockable);
        let geometry = ctx.input(|i| {
            let v = i.viewport();
            (v.minimized != Some(true) && v.visible() != Some(false)).then(|| v.outer_rect.zip(v.inner_rect)).flatten()
        });
        if let Some((outer, inner)) =
            geometry.filter(|(outer, inner)| outer.is_finite() && inner.is_finite() && inner.width() >= 160.0 && inner.height() >= 96.0)
        {
            let ppp = ctx.pixels_per_point();
            let old = Rect::from_min_size(outer.min, inner.size());
            let dragging = ctx.data(|data| data.get_temp::<Drag>(Id::new(("native-panel-drag", id))).is_some());
            let fitted = app
                .services
                .monitor_work_area
                .as_ref()
                .filter(|_| !dragging && !crate::titlebar::resize_active(&ctx, id))
                .map_or(old, |area| fit_panel_to_work_area(area, old, ppp));
            if (fitted.size() - old.size()).abs().max_elem() * ppp > 1.0 {
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(fitted.size()));
            }
            if fitted.min.distance(old.min) * ppp > 1.0 {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(fitted.min));
            }
            let position = from_screen(&ctx, fitted.min * ppp);
            geometry_result = Some(Rect::from_min_size(position, fitted.size()));
        }
        if dockable
            && ctx.input(|i| i.pointer.any_released())
            && let Some(point) = ctx.pointer_interact_pos()
        {
            queue_drop(&ctx, root_point(&ctx, point));
        }
        if close || (dockable && !super::floating_host_visible(app, root.id)) {
            set_role(&ctx, id, None);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            ctx.request_repaint_of(ViewportId::ROOT);
        }
    });
    (close, geometry_result)
}

fn resize_frame(app: &mut PhotocraftApp, ui: &mut egui::Ui, dockable: bool) {
    crate::titlebar::resize_zones_for_frame(
        ui,
        &mut app.services.contact_screen_pos,
        &mut app.services.screen_cursor_pos,
        FRAME_GUTTER,
        if dockable { PANEL_MIN_SIZE } else { AUX_MIN_SIZE },
    );
}

pub(super) fn show(app: &mut PhotocraftApp, ctx: &egui::Context, root: &Container, members: &[Container], title: &str) {
    let (close, geometry) = host(app, ctx, root, title, true, |app, ui| super::stack_body(app, ui, members));
    if let Some(rect) = geometry
        && let Some(current) = app.ui.dock.containers.iter_mut().find(|c| c.id == root.id)
    {
        current.position = [rect.left(), rect.top()];
        current.size = [rect.width(), rect.height()];
    }
    if close {
        super::close_containers(app, &members.iter().map(|c| c.id).collect::<Vec<_>>());
    }
}

pub(crate) struct Auxiliary<'a> {
    pub key: &'a str,
    pub title: &'a str,
    pub position: Pos2,
    pub size: Vec2,
}

/// Preserve the existing embedded host and widget body, but give desktop palettes their own
/// native surface. Geometry remains in egui's persisted UI state, as for the old egui windows.
pub(crate) fn auxiliary(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    spec: Auxiliary<'_>,
    embedded: egui::Window<'_>,
    mut body: impl FnMut(&mut PhotocraftApp, &mut egui::Ui),
) -> bool {
    if ctx.embed_viewports() {
        embedded.show(ctx, |ui| body(app, ui));
        return false;
    }
    let geometry_id = Id::new(("native-auxiliary-panel-geometry", spec.key));
    let geometry = ctx.data_mut(|d| d.get_persisted::<Rect>(geometry_id)).unwrap_or(Rect::from_min_size(spec.position, spec.size));
    let root = Container {
        id: Id::new(("native-auxiliary-panel", spec.key)).value(),
        position: [geometry.left(), geometry.top()],
        size: [geometry.width(), geometry.height()],
        ..Default::default()
    };
    let id = viewport(root.id, app.ui.shell.layout_epoch);
    ctx.data_mut(|d| {
        let mut shown = d.get_temp::<HashMap<ViewportId, u64>>(Id::new(AUX_HOSTS)).unwrap_or_default();
        shown.insert(id, app.frame);
        d.insert_temp(Id::new(AUX_HOSTS), shown);
    });
    let (closed, geometry) = host(app, ctx, &root, spec.title, false, body);
    if let Some(geometry) = geometry {
        ctx.data_mut(|d| d.insert_persisted(geometry_id, geometry));
    }
    closed
}

#[derive(Clone)]
struct PendingDrop {
    payload: Payload,
    point: Pos2,
}

fn queue_drop(ctx: &egui::Context, point: Pos2) {
    if let Some(payload) = egui::DragAndDrop::payload::<Payload>(ctx) {
        ctx.data_mut(|d| {
            let id = Id::new("native-panel-pending-drop");
            // A title gesture has the precise desktop position. Do not overwrite it with the
            // buffered synthetic mouse release generated from the same pen contact.
            if d.get_temp::<PendingDrop>(id).is_none() {
                d.insert_temp(id, PendingDrop { payload: (*payload).clone(), point });
            }
        });
    }
}

pub(super) fn finish_pending(app: &mut PhotocraftApp, ctx: &egui::Context) {
    finish_shortcuts(app, ctx);
    let id = Id::new("native-panel-pending-drop");
    let pending = ctx.data_mut(|d| {
        let pending = d.get_temp::<PendingDrop>(id);
        d.remove::<PendingDrop>(id);
        pending
    });
    if let Some(pending) = pending {
        egui::DragAndDrop::set_payload(ctx, pending.payload);
        super::finish_drag(app, ctx, true, Some(pending.point));
    }
}

fn application_keys_allowed(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    app.camera_raw.is_none()
        && app.ui.dialogs.is_empty()
        && crate::dialogs::native_dialog_viewport(app).is_none()
        && app.discard.is_none()
        && !app.distort.active()
        && app.distort.gallery.is_none()
        && !crate::menu_nav::is_open(ctx)
}

#[derive(Clone)]
struct PendingShortcut {
    command: String,
    document: Option<photocraft_doc::DocId>,
}

/// Native panel key events never enter ROOT's input stream. Resolve only
/// application bindings after panel widgets have had first refusal, then let
/// ROOT run the commands so dialogs and window commands get the correct host.
fn collect_shortcuts(app: &PhotocraftApp, ctx: &egui::Context) {
    if ctx.viewport_id() == ViewportId::ROOT || !application_keys_allowed(app, ctx) {
        return;
    }
    let frame_id = Id::new(("native-panel-shortcuts-collected-frame", ctx.viewport_id()));
    let frame = ctx.cumulative_frame_nr();
    if ctx.data(|data| data.get_temp::<u64>(frame_id)) == Some(frame) {
        return;
    }
    let focus = if app.ui.text_edit.is_some() { crate::shortcut_dispatch::Focus::Text } else { crate::shortcut_dispatch::Focus::of(ctx) };
    let mut commands = Vec::new();
    while let Some(command) = crate::shortcut_dispatch::take_pressed_command(app, ctx, focus, app.ui.text_edit.is_some()) {
        commands.push(command);
    }
    if !commands.is_empty() {
        let document = app.session.active().map(|state| state.doc.id);
        ctx.data_mut(|data| {
            data.insert_temp(frame_id, frame);
            data.get_temp_mut_or_default::<Vec<PendingShortcut>>(Id::new(SHORTCUTS))
                .extend(commands.into_iter().map(|command| PendingShortcut { command, document }));
        });
        ctx.request_repaint_of(ViewportId::ROOT);
    }
}

fn finish_shortcuts(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.viewport_id() != ViewportId::ROOT {
        return;
    }
    let commands = ctx.data_mut(|data| data.remove_temp::<Vec<PendingShortcut>>(Id::new(SHORTCUTS))).unwrap_or_default();
    for pending in commands {
        if !application_keys_allowed(app, ctx) {
            break;
        }
        // Conservatively drop even view/file commands if their originating
        // document changed while crossing native viewports.
        if app.session.active().map(|state| state.doc.id) != pending.document {
            continue;
        }
        crate::shortcut_dispatch::dispatch(app, ctx, &pending.command);
    }
}

#[cfg(test)]
#[path = "native_tests_v70.rs"]
mod tests_v70;

#[cfg(test)]
#[path = "native_merge_tests_v74.rs"]
mod merge_tests_v74;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_properties_focus_targets_the_live_stack_once_without_global_topmost() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.ui.panels.properties = true;
        app.ui.panels.color = true;
        let root = Container { id: 71, floating: true, tabs: vec![super::super::Tab { group: crate::dock::Group::Color, index: 0 }], ..Default::default() };
        app.ui.dock.containers = vec![
            root.clone(),
            Container {
                id: 72,
                stack: Some(71),
                floating: true,
                tabs: vec![super::super::Tab { group: crate::dock::Group::Properties, index: 0 }],
                ..Default::default()
            },
        ];
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        request_group_focus(&app, &ctx, crate::dock::Group::Properties);
        assert!(!take_focus_request(&ctx, viewport(72, 0)));
        assert!(take_focus_request(&ctx, viewport(71, 0)));
        assert!(!take_focus_request(&ctx, viewport(71, 0)), "steady frames cannot continually raise a tool window");
        let b = builder(&ctx, &root, "Properties", true);
        assert_eq!(b.window_level, Some(egui::WindowLevel::Normal));
        request_group_focus(&app, &ctx, crate::dock::Group::Properties);
        cleanup(&ctx, &[], 0, 1);
        assert!(!take_focus_request(&ctx, viewport(71, 0)), "closing a host retires its pending focus");
    }

    fn resize_input(viewport: ViewportId, size: Vec2, events: Vec<egui::Event>) -> egui::RawInput {
        let mut input = egui::RawInput { viewport_id: viewport, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() };
        let info = input.viewports.entry(viewport).or_default();
        info.inner_rect = Some(Rect::from_min_size(pos2(100.0, 100.0), size));
        info.outer_rect = info.inner_rect;
        info.focused = Some(true);
        input
    }

    fn mouse_button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
    }

    #[test]
    fn visible_panel_borders_and_corners_resize_with_mouse_in_every_theme() {
        // Straight points are on the opaque 6pt-inset contour, outside the old 5pt zones.
        let points = [
            pos2(6.0, 260.0),
            pos2(394.0, 260.0),
            pos2(200.0, 6.0),
            pos2(200.0, 544.0),
            pos2(9.0, 9.0),
            pos2(391.0, 9.0),
            pos2(9.0, 541.0),
            pos2(391.0, 541.0),
        ];
        for theme in crate::theme::ThemeKind::ALL {
            for point in points {
                let ctx = egui::Context::default();
                PhotocraftApp::setup_context(&ctx, theme);
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                app.services.screen_cursor_pos = Some(Box::new(|| None));
                let child = viewport(91, 0);
                let moved = point + vec2(35.0, 25.0);
                let mut commands = Vec::new();
                for events in [vec![egui::Event::PointerMoved(point)], vec![mouse_button(point, true)], vec![egui::Event::PointerMoved(moved)], vec![]] {
                    let mut output = ctx.run_ui(resize_input(child, vec2(400.0, 550.0), events), |ui| resize_frame(&mut app, ui, true));
                    commands.extend(output.viewport_output.get(&child).unwrap().commands.clone());
                    output.textures_delta.clear();
                }
                assert!(
                    commands.iter().any(|c| matches!(c, ViewportCommand::InnerSize(size) if *size != vec2(400.0,550.0))),
                    "{theme:?} contour {point:?}: {commands:?}"
                );
                assert!(!commands.iter().any(|c| matches!(c, ViewportCommand::BeginResize(_))));
                let mut output = ctx.run_ui(resize_input(child, vec2(400.0, 550.0), vec![mouse_button(moved, false)]), |ui| resize_frame(&mut app, ui, true));
                output.textures_delta.clear();
                assert!(!crate::titlebar::resize_active(&ctx, child));
                assert!(!crate::titlebar::resize_active(&ctx, ViewportId::ROOT));
            }
        }
    }

    #[test]
    fn pen_panel_resize_keeps_opposite_corner_at_native_minimum_and_recovers() {
        for (dockable, minimum) in [(true, PANEL_MIN_SIZE), (false, AUX_MIN_SIZE)] {
            let ctx = egui::Context::default();
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            let child = viewport(91, 0);
            let size = vec2(400.0, 550.0);
            let touch = |phase, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(2), phase, pos, force: Some(0.5) };
            for (phase, point) in [
                (egui::TouchPhase::Start, pos2(9.0, 9.0)),
                (egui::TouchPhase::Move, pos2(609.0, 709.0)),
                (egui::TouchPhase::Move, pos2(29.0, 39.0)),
                (egui::TouchPhase::End, pos2(29.0, 39.0)),
            ] {
                let mut output = ctx.run_ui(resize_input(child, size, vec![touch(phase, point)]), |ui| resize_frame(&mut app, ui, dockable));
                let commands = &output.viewport_output[&child].commands;
                if phase == egui::TouchPhase::Move && point.x > 600.0 {
                    assert!(commands.contains(&ViewportCommand::InnerSize(minimum)), "{commands:?}");
                    assert!(
                        commands.contains(&ViewportCommand::OuterPosition(pos2(500.0 - minimum.x, 650.0 - minimum.y))),
                        "opposite corner must remain fixed: {commands:?}"
                    );
                } else if phase == egui::TouchPhase::Move {
                    assert!(commands.contains(&ViewportCommand::InnerSize(vec2(380.0, 520.0))), "reversing must recover from the minimum: {commands:?}");
                }
                output.textures_delta.clear();
            }
            assert!(!crate::titlebar::resize_active(&ctx, child));
        }
    }

    #[test]
    fn panel_header_close_and_body_stay_outside_resize_targets() {
        for point in [pos2(376.0, 24.0), pos2(180.0, 24.0), pos2(200.0, 260.0)] {
            let ctx = egui::Context::default();
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.services.screen_cursor_pos = Some(Box::new(|| None));
            let child = viewport(91, 0);
            for events in [vec![egui::Event::PointerMoved(point)], vec![mouse_button(point, true)], vec![egui::Event::PointerMoved(point + vec2(20.0, 20.0))]] {
                let mut output = ctx.run_ui(resize_input(child, vec2(400.0, 550.0), events), |ui| resize_frame(&mut app, ui, true));
                assert!(!crate::titlebar::resize_active(&ctx, child), "{point:?}");
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn title_corner_cannot_start_docking_while_resizing() {
        for pen in [false, true] {
            let ctx = egui::Context::default();
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.services.screen_cursor_pos = Some(Box::new(|| None));
            let root = Container { id: 91, floating: true, ..Default::default() };
            let child = viewport(91, 0);
            let touch = |phase, pos| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(2), phase, pos, force: Some(0.5) };
            let start = pos2(14.0, 14.0);
            let moved = pos2(44.0, 54.0);
            let events = if pen {
                vec![
                    vec![],
                    vec![touch(egui::TouchPhase::Start, start)],
                    vec![touch(egui::TouchPhase::Move, moved)],
                    vec![],
                    vec![touch(egui::TouchPhase::End, moved)],
                ]
            } else {
                vec![
                    vec![egui::Event::PointerMoved(start)],
                    vec![mouse_button(start, true)],
                    vec![egui::Event::PointerMoved(moved)],
                    vec![],
                    vec![mouse_button(moved, false)],
                ]
            };
            let mut resized = false;
            for events in events {
                let mut output = ctx.run_ui(resize_input(child, vec2(400.0, 550.0), events), |ui| {
                    drag_title(&mut app, ui, &root, Rect::from_min_max(pos2(12.0, 12.0), pos2(364.0, 36.0)), true);
                    resize_frame(&mut app, ui, true);
                    assert!(ui.ctx().data(|d| d.get_temp::<Drag>(Id::new(("native-panel-drag", child)))).is_none());
                    assert!(egui::DragAndDrop::payload::<Payload>(ui.ctx()).is_none());
                });
                resized |= output.viewport_output[&child].commands.iter().any(|c| matches!(c, ViewportCommand::InnerSize(_)));
                output.textures_delta.clear();
            }
            assert!(resized, "pen={pen}: corner must still resize");
            assert!(!crate::titlebar::resize_active(&ctx, child));
        }
    }

    #[test]
    fn hidden_stack_root_does_not_hide_an_expanded_member_and_last_member_retracts_host() {
        use crate::{dock::Group, panel_workspace::Tab};
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.ui.panels.brush_settings = true;
        app.ui.panels.color = true;
        app.ui.dock.containers = vec![
            Container { id: 71, tabs: vec![Tab { group: Group::Brush, index: 0 }], floating: true, ..Default::default() },
            Container { id: 72, tabs: vec![Tab { group: Group::Color, index: 0 }], floating: true, stack: Some(71), ..Default::default() },
        ];
        let id = viewport(71, 0);
        assert_eq!(active_viewport(&app, 71), Some(id));
        crate::dock::rail_click(&mut app, Group::Brush, false);
        assert!(app.ui.dock.containers[0].collapsed);
        assert_eq!(active_viewport(&app, 72), Some(id), "Color must keep the shared host alive");
        app.ui.dock.containers[0].closed = true;
        assert!(super::super::floating_host_visible(&app, 71), "closed root membership is not closed host state");
        crate::dock::rail_click(&mut app, Group::Color, false);
        assert_eq!(active_viewport(&app, 71), None, "all retracted members mean no host");
        crate::dock::rail_click(&mut app, Group::Color, false);
        assert_eq!(active_viewport(&app, 72), Some(id));
        app.ui.dock.containers[0].floating = false;
        assert_eq!(active_viewport(&app, 72), None, "a docked root cannot retain a floating HWND");
    }

    #[test]
    fn native_panel_is_owned_normal_level_without_taskbar_or_os_frame() {
        let ctx = egui::Context::default();
        let b = builder(&ctx, &Container::default(), "Panel", true);
        assert_eq!(b.window_level, Some(egui::WindowLevel::Normal));
        assert_eq!(b.taskbar, Some(false));
        assert_eq!(b.decorations, Some(false));
        assert_eq!(b.transparent, Some(true));
        assert_eq!(b.active, Some(false));
        let retained = builder(&ctx, &Container::default(), "Panel", false);
        assert!(retained.position.is_none());
        assert!(retained.inner_size.is_none());
        assert!(retained.active.is_none());
    }

    #[test]
    fn panel_close_removes_only_its_own_role() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let panel = viewport(7, 0);
        let popup = ViewportId::from_hash_of("popup");
        set_role(&ctx, panel, Some(1));
        set_role(&ctx, popup, Some(2));
        ctx.data_mut(|d| d.insert_temp(Id::new(HOSTS), vec![panel]));
        cleanup(&ctx, &[], 0, 0);
        let roles = ctx.data(|d| d.get_temp::<HashMap<ViewportId, u8>>(Id::new(ROLES))).unwrap();
        assert!(!roles.contains_key(&panel));
        assert_eq!(roles.get(&popup), Some(&2));
    }

    #[test]
    fn layout_reload_gets_new_host_and_invalid_geometry_is_bounded() {
        assert_ne!(viewport(7, 0), viewport(7, 1));
        let root = Container { position: [f32::NAN, f32::INFINITY], size: [f32::NAN, -10.0], ..Default::default() };
        let b = builder(&egui::Context::default(), &root, "Panel", true);
        assert!(b.position.unwrap().is_finite());
        assert_eq!(b.inner_size, Some(vec2(360.0, 180.0)));
    }

    #[test]
    fn floating_panel_fits_an_offset_monitor_work_area_without_changing_a_panel_that_fits() {
        let area: crate::MonitorWorkAreaFn = Box::new(|point| {
            assert!(point.x >= 2560.0);
            Some(Rect::from_min_max(pos2(2560.0, -80.0), pos2(5120.0, 1320.0)))
        });
        let visible = Rect::from_min_size(pos2(2260.0, 125.0), vec2(300.0, 750.0));
        assert_eq!(fit_panel_to_work_area(&area, visible, 1.25), visible);
        let oversized = Rect::from_min_size(pos2(4000.0, 140.0), vec2(320.0, 1250.0));
        let fitted = fit_panel_to_work_area(&area, oversized, 1.25);
        assert_eq!(fitted, Rect::from_min_max(pos2(3776.0, -64.0), pos2(4096.0, 1056.0)));
    }

    #[test]
    fn native_drop_coordinates_use_both_window_origins_and_dpi() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let child = viewport(7, 0);
        let mut input = egui::RawInput { viewport_id: child, ..Default::default() };
        let root = input.viewports.entry(ViewportId::ROOT).or_default();
        root.native_pixels_per_point = Some(2.0);
        root.inner_rect = Some(Rect::from_min_size(pos2(100.0, 50.0), vec2(1000.0, 800.0)));
        let child_info = input.viewports.entry(child).or_default();
        child_info.native_pixels_per_point = Some(1.0);
        child_info.inner_rect = Some(Rect::from_min_size(pos2(350.0, 200.0), vec2(400.0, 500.0)));
        let mut output = ctx.run_ui(input, |ui| {
            assert_eq!(root_point(ui.ctx(), pos2(10.0, 20.0)), pos2(80.0, 60.0));
            assert_eq!(root_rect(ui.ctx(), Rect::from_min_size(pos2(10.0, 20.0), vec2(100.0, 80.0))).size(), vec2(50.0, 40.0));
        });
        output.textures_delta.clear();
    }

    #[test]
    fn drop_is_resolved_after_later_floating_targets_are_registered() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.frame = 12;
        app.ui.dock.containers = vec![
            Container { id: 1, tabs: vec![super::super::Tab { group: crate::dock::Group::Layers, index: 0 }], floating: true, ..Default::default() },
            Container { id: 2, tabs: vec![super::super::Tab { group: crate::dock::Group::Color, index: 0 }], floating: true, ..Default::default() },
        ];
        egui::DragAndDrop::set_payload(&ctx, Payload { container: 1, tab: Some(0) });
        queue_drop(&ctx, pos2(150.0, 114.0));
        // The target viewport can occur after the source in the render list.
        super::super::register(&app, &ctx, 2, Rect::from_min_size(pos2(100.0, 100.0), vec2(200.0, 28.0)));
        egui::DragAndDrop::clear_payload(&ctx);
        finish_pending(&mut app, &ctx);
        assert_eq!(app.ui.dock.containers.len(), 1);
        assert_eq!(app.ui.dock.containers[0].id, 2);
        assert_eq!(app.ui.dock.containers[0].tabs.len(), 2);
    }

    #[test]
    fn pen_title_drag_uses_live_desktop_samples_even_without_new_client_motion() {
        use std::{cell::Cell, rc::Rc};
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let sample = Rc::new(Cell::new(pos2(120.0, 120.0)));
        let read_sample = Rc::clone(&sample);
        app.services.contact_screen_pos = Some(Box::new(move |_, _| Some(read_sample.get())));
        let root = Container { id: 77, ..Default::default() };
        let device = egui::TouchDeviceId(1);
        let contact = egui::TouchId(4);
        let input = |events, outer: Pos2| {
            let mut input = egui::RawInput { events, ..Default::default() };
            let viewport = input.viewports.entry(ViewportId::ROOT).or_default();
            viewport.native_pixels_per_point = Some(1.0);
            viewport.outer_rect = Some(Rect::from_min_size(outer, vec2(400.0, 400.0)));
            viewport.inner_rect = viewport.outer_rect;
            input
        };
        let mut started = ctx.run_ui(
            input(
                vec![egui::Event::Touch { device_id: device, id: contact, phase: egui::TouchPhase::Start, pos: pos2(20.0, 20.0), force: Some(0.5) }],
                pos2(100.0, 100.0),
            ),
            |ui| drag_title(&mut app, ui, &root, Rect::from_min_size(pos2(12.0, 12.0), vec2(300.0, 24.0)), false),
        );
        started.textures_delta.clear();
        sample.set(pos2(180.0, 150.0));
        let mut moved = ctx.run_ui(input(vec![], pos2(100.0, 100.0)), |ui| {
            drag_title(&mut app, ui, &root, Rect::from_min_size(pos2(12.0, 12.0), vec2(300.0, 24.0)), false);
        });
        let commands = &moved.viewport_output[&ViewportId::ROOT].commands;
        assert!(commands.iter().any(|command| matches!(command, ViewportCommand::OuterPosition(p) if *p == pos2(160.0, 130.0))));
        assert!(!commands.iter().any(|command| matches!(command, ViewportCommand::StartDrag | ViewportCommand::Focus)));
        moved.textures_delta.clear();
        let mut settled = ctx.run_ui(input(vec![], pos2(160.0, 130.0)), |ui| {
            drag_title(&mut app, ui, &root, Rect::from_min_size(pos2(12.0, 12.0), vec2(300.0, 24.0)), false);
        });
        assert!(!settled.viewport_output[&ViewportId::ROOT].commands.iter().any(|command| matches!(command, ViewportCommand::OuterPosition(_))));
        assert!(egui::DragAndDrop::payload::<Payload>(&ctx).is_none(), "an auxiliary palette must not become a dock-container payload");
        settled.textures_delta.clear();
    }

    #[test]
    fn pen_title_drag_fallback_moves_without_reentering_context_input() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let root = Container { id: 78, ..Default::default() };
        let mut frame = |phase, position| {
            let mut input = egui::RawInput {
                events: vec![egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(5), phase, pos: position, force: Some(0.5) }],
                ..Default::default()
            };
            let viewport = input.viewports.entry(ViewportId::ROOT).or_default();
            viewport.native_pixels_per_point = Some(2.0);
            viewport.outer_rect = Some(Rect::from_min_size(pos2(100.0, 100.0), vec2(400.0, 400.0)));
            viewport.inner_rect = viewport.outer_rect;
            let mut output = ctx.run_ui(input, |ui| {
                drag_title(&mut app, ui, &root, Rect::from_min_size(pos2(12.0, 12.0), vec2(300.0, 24.0)), false);
            });
            output.textures_delta.clear();
            output
        };
        let _ = frame(egui::TouchPhase::Start, pos2(30.0, 24.0));
        let output = frame(egui::TouchPhase::Move, pos2(60.0, 44.0));
        assert!(
            output.viewport_output[&ViewportId::ROOT]
                .commands
                .iter()
                .any(|command| { matches!(command, ViewportCommand::OuterPosition(p) if *p == pos2(130.0, 120.0)) })
        );
    }
}
