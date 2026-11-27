//! Panel containers: individual tab tear-off, tab grouping, floating stacks and drag docking.
use crate::theme::palette::ScrollAreaPaletteExt;
use crate::{PhotocraftApp, dock::Group, theme::Tokens, widgets};
use egui::{Rect, Sense, Stroke, pos2, vec2};
use serde::{Deserialize, Serialize};

pub(crate) mod native;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tab {
    pub group: Group,
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Container {
    pub id: u64,
    pub tabs: Vec<Tab>,
    pub selected: usize,
    pub floating: bool,
    /// Floating groups in the same stack share one frame. Root has no stack reference.
    pub stack: Option<u64>,
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub height: f32,
    pub collapsed: bool,
    pub closed: bool,
    /// Studio's default Properties card is shown only for adjustment/fill layers.
    pub auto_properties: bool,
}
impl Default for Container {
    fn default() -> Self {
        Self {
            id: 0,
            tabs: vec![],
            selected: 0,
            floating: false,
            stack: None,
            position: [100.0, 100.0],
            size: [360.0, 500.0],
            height: 250.0,
            collapsed: false,
            closed: false,
            auto_properties: false,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Payload {
    pub container: u64,
    pub tab: Option<usize>,
}
#[derive(Clone)]
struct Zone {
    id: u64,
    rect: Rect,
    frame: u64,
    viewport: egui::ViewportId,
}

#[derive(Clone)]
struct DragPoint {
    container: u64,
    point: egui::Pos2,
    precise: bool,
}

fn drag_point_id() -> egui::Id {
    egui::Id::new("panel-workspace-current-drag-point")
}

/// Native source windows retain pointer capture. Share their root-space position
/// rather than trusting a target window's old, window-local hover event.
pub(super) fn set_drag_point(ctx: &egui::Context, container: u64, point: egui::Pos2, precise: bool) {
    if !point.is_finite() {
        return;
    }
    let previous = ctx.data(|d| d.get_temp::<DragPoint>(drag_point_id()));
    if previous.as_ref().is_some_and(|old| old.container == container && old.precise && !precise) {
        return;
    }
    let changed = previous.as_ref().is_none_or(|old| old.container != container || old.point != point);
    ctx.data_mut(|d| d.insert_temp(drag_point_id(), DragPoint { container, point, precise }));
    if changed {
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        let zones = ctx.data(|d| d.get_temp::<Vec<Zone>>(zones_id())).unwrap_or_default();
        for zone in zones {
            if zone.rect.contains(point) || previous.as_ref().is_some_and(|old| zone.rect.contains(old.point)) {
                ctx.request_repaint_of(zone.viewport);
            }
        }
    }
}

pub(super) fn clear_drag_point(ctx: &egui::Context) {
    let previous = ctx.data_mut(|d| {
        let previous = d.get_temp::<DragPoint>(drag_point_id());
        d.remove::<DragPoint>(drag_point_id());
        previous
    });
    if let Some(previous) = previous {
        ctx.request_repaint_of(egui::ViewportId::ROOT);
        for zone in ctx.data(|d| d.get_temp::<Vec<Zone>>(zones_id())).unwrap_or_default() {
            if zone.rect.contains(previous.point) {
                ctx.request_repaint_of(zone.viewport);
            }
        }
    }
}

fn current_drag_point(ctx: &egui::Context, payload: &Payload) -> Option<egui::Pos2> {
    if !ctx.embed_viewports()
        && let Some(point) = ctx.data(|d| d.get_temp::<DragPoint>(drag_point_id())).filter(|p| p.container == payload.container)
    {
        return Some(point.point);
    }
    ctx.pointer_interact_pos().map(|point| native::root_point(ctx, point))
}

/// Both the preview and the final drop accept only a visible target's actual tab strip.
fn merge_target(app: &PhotocraftApp, ctx: &egui::Context, payload: &Payload, point: egui::Pos2) -> Option<Zone> {
    if app.session.prefs().workspace_locked {
        return None;
    }
    let source = app.ui.dock.containers.iter().find(|c| c.id == payload.container && !c.closed)?;
    if payload.tab.is_some_and(|index| index >= source.tabs.len()) {
        return None;
    }
    let source_root = source.stack.unwrap_or(source.id);
    ctx.data(|d| d.get_temp::<Vec<Zone>>(zones_id())).unwrap_or_default().into_iter().rev().find(|zone| {
        if zone.frame != app.frame || !zone.rect.contains(point) {
            return false;
        }
        app.ui.dock.containers.iter().find(|c| c.id == zone.id).is_some_and(|target| {
            target.stack.unwrap_or(target.id) != source_root
                && !target.closed
                && !target.collapsed
                && !visible(app, target).is_empty()
                && (!target.floating || floating_host_visible(app, target.stack.unwrap_or(target.id)))
        })
    })
}

fn zones_id() -> egui::Id {
    egui::Id::new("panel-workspace-zones")
}

pub fn migrate(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !app.ui.dock.containers.is_empty() {
        return;
    }
    let pro = Tokens::get(ctx).pro;
    app.ui.dock.containers = app
        .ui
        .dock
        .order()
        .into_iter()
        .map(|group| {
            let floating = app.ui.dock.floating.contains(&group)
                || match group {
                    Group::Brush => app.ui.panels.brush_floating,
                    Group::ToolPresets => app.ui.panels.tool_presets_floating,
                    Group::Properties => !pro,
                    _ => false,
                };
            let selected = match group {
                Group::Brush => app.ui.brush_tab,
                Group::Color => app.ui.dock_tabs.color,
                Group::History => app.ui.dock_tabs.history,
                Group::Layers => app.ui.dock_tabs.layers,
                Group::Navigator => app.ui.dock_tabs.navigator,
                Group::Character => app.ui.dock_tabs.character,
                Group::Properties => app.ui.dock_tabs.properties,
                Group::ToolPresets => 0,
            };
            let legacy_id = match group {
                Group::Brush => egui::Id::new("brush-settings"),
                Group::ToolPresets => egui::Id::new(("preset-window", "toolPresets")),
                _ => egui::Id::new(("floating-dock", group)),
            };
            let old_rect = ctx.memory(|m| m.area_rect(legacy_id));
            Container {
                id: group as u64,
                tabs: (0..group.tabs(pro).len()).map(|index| Tab { group, index }).collect(),
                selected,
                floating,
                auto_properties: group == Group::Properties && !pro && !app.ui.dock.floating.contains(&group),
                height: app.ui.dock.heights.get(&group).copied().unwrap_or(group.default_height()),
                collapsed: app.ui.dock.is_collapsed(group),
                position: old_rect.map_or([100.0 + group as u64 as f32 * 30.0, 80.0], |r| [r.left(), r.top()]),
                size: old_rect.map_or(if group == Group::Brush { [500.0, 520.0] } else { [360.0, 500.0] }, |r| [r.width(), r.height()]),
                ..Default::default()
            }
        })
        .collect();
}
pub fn float_group(app: &mut PhotocraftApp, ctx: &egui::Context, group: Group) {
    migrate(app, ctx);
    if let Some(c) = app.ui.dock.containers.iter_mut().find(|c| c.tabs.iter().any(|t| t.group == group)) {
        c.floating = true;
        c.stack = None;
        c.auto_properties = false;
    }
}
pub(super) fn visible(app: &PhotocraftApp, c: &Container) -> Vec<Tab> {
    if c.closed {
        return vec![];
    }
    if c.auto_properties
        && c.floating
        && !app
            .session
            .active()
            .and_then(|st| st.active_layer.and_then(|id| st.doc.layer(id)))
            .is_some_and(|layer| matches!(layer.content, photocraft_doc::LayerContent::Adjustment(_) | photocraft_doc::LayerContent::Fill(_)))
    {
        return vec![];
    }
    c.tabs.iter().copied().filter(|t| t.group.shown(&app.ui.panels)).collect()
}
pub(crate) fn close_containers(app: &mut PhotocraftApp, ids: &[u64]) {
    for c in &mut app.ui.dock.containers {
        if ids.contains(&c.id) {
            c.closed = true;
        }
    }
    for group in Group::ALL {
        let containers: Vec<&Container> = app.ui.dock.containers.iter().filter(|c| c.tabs.iter().any(|t| t.group == group)).collect();
        if !containers.is_empty() && containers.iter().all(|c| c.closed) {
            *group.shown_mut(&mut app.ui.panels) = false;
        }
    }
}
pub fn docked_groups(app: &PhotocraftApp) -> Vec<Group> {
    let mut out = vec![];
    for c in app.ui.dock.containers.iter().filter(|c| !c.floating) {
        for t in visible(app, c) {
            if !out.contains(&t.group) {
                out.push(t.group);
            }
        }
    }
    out
}
fn register(app: &PhotocraftApp, ctx: &egui::Context, id: u64, rect: Rect) {
    let rect = native::root_rect(ctx, rect);
    let mut zones = ctx.data(|d| d.get_temp::<Vec<Zone>>(zones_id())).unwrap_or_default();
    zones.retain(|z| z.frame == app.frame && z.id != id);
    zones.push(Zone { id, rect, frame: app.frame, viewport: ctx.viewport_id() });
    ctx.data_mut(|d| d.insert_temp(zones_id(), zones));
}
fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, c: &Container) {
    crate::brush_preview::background(ui.ctx(), app.background_jobs);
    let tabs = visible(app, c);
    if tabs.is_empty() {
        return;
    }
    let names: Vec<&str> = tabs.iter().map(|t| t.group.tabs(Tokens::get(ui.ctx()).pro).get(t.index).copied().unwrap_or("Panel")).collect();
    let mut selected = c.selected.min(tabs.len().saturating_sub(1));
    let compact_body = tabs.get(selected).is_some_and(|tab| tab.group == Group::Properties && tab.index == 0);
    let resp = widgets::card_ex_with_compact_body(ui, &format!("container-{}", c.id), &names, &mut selected, c.collapsed, compact_body, |ui, index| {
        if let Some(tab) = tabs.get(index).copied() {
            let height = ui.available_height().max(40.0);
            if tab.group.scrolls_itself(tab.index) {
                crate::panels::dock_body(app, ui, tab.group, tab.index);
            } else {
                egui::ScrollArea::vertical()
                    .id_salt(("container-scroll", c.id, tab.group, tab.index))
                    .max_height(height)
                    .auto_shrink([false, false])
                    .show_palette(ui, |ui| crate::panels::dock_body(app, ui, tab.group, tab.index));
            }
        }
    });
    let locked = app.session.prefs().workspace_locked;
    if !locked {
        if !c.floating {
            resp.strip.dnd_set_drag_payload(Payload { container: c.id, tab: None });
        }
        for (index, rect) in &resp.tabs {
            let r = ui.interact(*rect, ui.id().with(("tear-off-tab", c.id, index)), Sense::click_and_drag());
            if let Some(name) = names.get(*index) {
                r.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected == *index, tl!(*name)));
            }
            if r.clicked() {
                selected = *index;
            }
            if r.double_clicked()
                && let Some(current) = app.ui.dock.containers.iter_mut().find(|v| v.id == c.id)
            {
                current.collapsed = !current.collapsed;
            }
            if let Some(tab) = tabs.get(*index)
                && let Some(original) = c.tabs.iter().position(|t| t == tab)
            {
                r.dnd_set_drag_payload(Payload { container: c.id, tab: Some(original) });
            }
        }
    }
    if let Some(current) = app.ui.dock.containers.iter_mut().find(|v| v.id == c.id) {
        // Content callbacks can explicitly reveal another tab in this same container
        // (Adjustments grid, or a Layers thumbnail grouped with Properties).
        // Do not overwrite that request with the pre-callback tab-strip snapshot.
        if current.selected != c.selected {
            selected = current.selected;
        }
        current.selected = selected;
        if resp.strip.double_clicked() || resp.tab_double_clicked {
            current.collapsed = !current.collapsed;
        }
    }
    // A callback may also have made a previously hidden group visible.
    let tabs = app.ui.dock.containers.iter().find(|v| v.id == c.id).map(|current| visible(app, current)).unwrap_or(tabs);
    if let Some(tab) = tabs.get(selected) {
        *tab.group.tab_mut(&mut app.ui.dock_tabs) = tab.index;
        if tab.group == Group::Brush {
            app.ui.brush_tab = tab.index;
        }
    }
    if !ui.ctx().embed_viewports() && tabs.get(selected).is_some_and(|tab| tab.group == Group::ToolPresets) {
        if resp.menu.clicked() {
            crate::preset_panels::open_tool_presets_native(app, ui.ctx(), crate::preset_panels::ToolMenuHost::Container(c.id), resp.menu.rect.left_bottom());
        }
    } else {
        egui::Popup::menu(&resp.menu).show(|ui| {
            if tabs.get(selected).is_some_and(|tab| tab.group == Group::ToolPresets) {
                crate::preset_panels::tool_presets_menu(app, ui);
                ui.separator();
            }
            if tabs.get(selected).is_some_and(|tab| tab.group == Group::Color && tab.group.tabs(Tokens::get(ui.ctx()).pro).get(tab.index) == Some(&"Color")) {
                crate::color_panel_ui::menu(app, ui);
                ui.separator();
            }
            if ui.button(if c.floating { tl!("Dock Panel") } else { tl!("Float Panel") }).clicked() {
                if let Some(current) = app.ui.dock.containers.iter_mut().find(|v| v.id == c.id) {
                    current.floating = !current.floating;
                    current.stack = None;
                    current.auto_properties = false;
                }
                ui.close();
            }
            if ui.button(tl!("Close Tab Group")).clicked() {
                close_containers(app, &[c.id]);
                ui.close();
            }
        });
    }
    // The menu/tab strip is the only UI merge target. A large content card
    // must never silently accept the floating panel dropped over it.
    let header = resp.strip.rect.intersect(ui.clip_rect());
    if header.is_positive() {
        register(app, ui.ctx(), c.id, header);
    }
    if let Some(payload) = egui::DragAndDrop::payload::<Payload>(ui.ctx()) {
        if payload.container == c.id
            && !ui.ctx().embed_viewports()
            && let Some(point) = ui.ctx().pointer_interact_pos()
        {
            set_drag_point(ui.ctx(), c.id, native::root_point(ui.ctx(), point), false);
        }
        if let Some(point) = current_drag_point(ui.ctx(), &payload)
            && merge_target(app, ui.ctx(), &payload, point).is_some_and(|target| target.id == c.id)
        {
            let t = Tokens::get(ui.ctx());
            // A translucent tint leaves the existing tab labels legible.
            ui.painter().rect_filled(header, t.radius_sm, t.accent.gamma_multiply(0.18));
            ui.painter().rect_stroke(header.shrink(1.0), t.radius_sm, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        }
    }
}

pub fn docked(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let shown: Vec<Container> = app.ui.dock.containers.iter().filter(|c| !c.floating && !visible(app, c).is_empty()).cloned().collect();
    let area = ui.available_rect_before_wrap();
    let mut legacy_layout = app.ui.dock.clone();
    let mut groups = Vec::new();
    let compatible = shown.iter().all(|c| c.tabs.first().is_some_and(|first| c.tabs.iter().all(|t| t.group == first.group)));
    let compatible = compatible
        && shown.iter().all(|c| {
            let Some(group) = c.tabs.first().map(|t| t.group) else { return false };
            if groups.contains(&group) {
                return false;
            }
            groups.push(group);
            legacy_layout.set_collapsed(group, c.collapsed);
            if (c.height - group.default_height()).abs() > 0.5 {
                legacy_layout.heights.insert(group, c.height);
            }
            true
        });
    let legacy_heights = if compatible { legacy_layout.heights_for(&groups, area.height(), if Tokens::get(ui.ctx()).pro { 28. } else { 40. }) } else { vec![] };
    let count = shown.len();
    let minimum: f32 = shown.iter().map(|c| if c.collapsed { 36.0 } else { 90.0 }).sum();
    let requested: f32 = shown.iter().map(|c| if c.collapsed { 36.0 } else { c.height.clamp(90.0, 4000.0) }).sum();
    let available = (area.height() - count.saturating_sub(1) as f32 * 6.0).max(minimum);
    let scale = ((available - minimum) / (requested - minimum).max(1.0)).min(1.0);
    let mut y = area.top();
    let mut rects = Vec::new();
    for (i, c) in shown.iter().enumerate() {
        let height = if let Some((_, height)) = legacy_heights.get(i) {
            *height
        } else if i + 1 == count {
            (area.bottom() - y).max(36.0)
        } else if c.collapsed {
            36.0
        } else {
            90.0 + (c.height.clamp(90.0, 4000.0) - 90.0) * scale
        };
        let rect = Rect::from_min_size(pos2(area.left(), y), vec2(area.width(), height));
        for tab in &c.tabs {
            if !rects.iter().any(|(g, _)| *g == tab.group) {
                rects.push((tab.group, rect));
            }
        }
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt(("container", c.id)).max_rect(rect));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        body(app, &mut child, c);
        if i + 1 < count {
            let split = Rect::from_min_size(pos2(area.left(), rect.bottom()), vec2(area.width(), 6.0));
            let r = ui.interact(split, ui.id().with(("container-split", c.id)), Sense::drag());
            if r.hovered() || r.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }
            if r.dragged()
                && let Some(current) = app.ui.dock.containers.iter_mut().find(|v| v.id == c.id)
            {
                current.height = (height + r.drag_delta().y).clamp(90.0, 4000.0);
            }
        }
        y = rect.bottom() + 6.0;
    }
    ui.advance_cursor_after_rect(area);
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("dock-group-rects"), rects));
}

fn stack_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, members: &[Container]) {
    let ctx = ui.ctx().clone();
    ui.set_min_size(ui.available_size());
    let total = ui.available_height();
    let width = ui.available_width();
    let weights: f32 = members.iter().map(|c| if c.collapsed { 36. } else { c.height.max(80.) }).sum();
    for (i, c) in members.iter().enumerate() {
        let height = if members.len() == 1 {
            total
        } else if i + 1 == members.len() {
            ui.available_height().max(60.0)
        } else {
            ((total - members.len().saturating_sub(1) as f32 * 6.) * (if c.collapsed { 36. } else { c.height.max(80.) }) / weights.max(1.)).max(36.0)
        };
        let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
        let mut child = ui.new_child(egui::UiBuilder::new().id_salt(("floating-container-body", c.id)).max_rect(rect));
        child.set_clip_rect(rect.intersect(ui.clip_rect()));
        body(app, &mut child, c);
        if let Some(next) = members.get(i + 1) {
            let (_, r) = ui.allocate_exact_size(vec2(width, 6.), Sense::drag());
            if r.hovered() || r.dragged() {
                ctx.set_cursor_icon(egui::CursorIcon::ResizeVertical);
            }
            if r.dragged() {
                let delta = r.drag_delta().y;
                if let Some(current) = app.ui.dock.containers.iter_mut().find(|v| v.id == c.id) {
                    current.height = (height + delta).max(80.);
                }
                if let Some(current) = app.ui.dock.containers.iter_mut().find(|v| v.id == next.id) {
                    current.height = (next.height - delta).max(80.);
                }
            }
        }
    }
}

/// A collapsed floating group retracts to its rail icon, rather than leaving an empty
/// full-size native frame. Other expanded members of a stack can keep that host alive.
pub(super) fn floating_host_visible(app: &PhotocraftApp, root: u64) -> bool {
    app.ui.dock.containers.iter().any(|c| c.id == root && c.floating && c.stack.is_none())
        && app.ui.dock.containers.iter().any(|c| (c.id == root || c.stack == Some(root)) && !c.collapsed && !visible(app, c).is_empty())
}

pub fn floating(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if root_layout_suspended(ctx) {
        return;
    }
    let roots: Vec<Container> =
        app.ui.dock.containers.iter().filter(|c| c.floating && c.stack.is_none() && floating_host_visible(app, c.id)).cloned().collect();
    native::cleanup(ctx, &roots, app.ui.shell.layout_epoch, app.frame);
    for root in roots {
        let members: Vec<Container> = app
            .ui
            .dock
            .containers
            .iter()
            .filter(|c| c.id == root.id || c.stack == Some(root.id))
            .filter(|c| !c.collapsed && !visible(app, c).is_empty())
            .cloned()
            .collect();
        let title_member = members.iter().find(|c| c.id == root.id).or_else(|| members.first()).unwrap_or(&root);
        let title = visible(app, title_member)
            .get(title_member.selected)
            .or_else(|| title_member.tabs.first())
            .and_then(|t| t.group.tabs(Tokens::get(ctx).pro).get(t.index))
            .copied()
            .unwrap_or("Panel");
        if !ctx.embed_viewports() {
            native::show(app, ctx, &root, &members, title);
            continue;
        }
        let mut open = true;
        let window_id = egui::Id::new(("panel-container", root.id));
        let window_id = if app.ui.shell.layout_epoch == 0 { window_id } else { window_id.with(app.ui.shell.layout_epoch) };
        let response = egui::Window::new(tl!(title))
            .drag_area(egui::WindowDrag::TitleBar)
            .movable(!app.session.prefs().workspace_locked)
            .id(window_id)
            .open(&mut open)
            .resizable(true)
            .current_pos(pos2(root.position[0], root.position[1]))
            .default_size(vec2(root.size[0].clamp(250.0, 2000.0), root.size[1].clamp(220.0, 2000.0)))
            .min_size(vec2(250.0, 180.0))
            .show(ctx, |ui| {
                stack_body(app, ui, &members);
            });
        if let Some(response) = response {
            let rect = response.response.rect;
            if let Some(current) = app.ui.dock.containers.iter_mut().find(|v| v.id == root.id) {
                current.position = [rect.left(), rect.top()];
                current.size = [rect.width(), rect.height()];
            }
            // Floating stacks move only from the native title; docked tabs may tear off.
            let titlebar = ctx.read_response(window_id.with("__title_click")).map_or(Rect::NOTHING, |r| r.rect);
            let title_origin = egui::Id::new("panel-title-drag-origin");
            if !app.session.prefs().workspace_locked
                && ctx.input(|i| i.pointer.primary_pressed() && i.pointer.press_origin().is_some_and(|p| titlebar.contains(p)))
            {
                ctx.data_mut(|d| d.insert_temp(title_origin, root.id));
            }
            if !app.session.prefs().workspace_locked
                && ctx.data(|d| d.get_temp::<u64>(title_origin)) == Some(root.id)
                && ctx.input(|i| i.pointer.primary_down() && i.pointer.is_decidedly_dragging())
            {
                egui::DragAndDrop::set_payload(ctx, Payload { container: root.id, tab: None });
                ctx.data_mut(|d| d.insert_temp(egui::Id::new("panel-drag-whole-stack"), root.id));
            }
        }
        if !open {
            close_containers(app, &members.iter().map(|c| c.id).collect::<Vec<_>>());
        }
    }
    native::finish_pending(app, ctx);
    finish_drag(app, ctx, ctx.input(|i| i.pointer.any_released()), ctx.pointer_interact_pos());
}

fn finish_drag(app: &mut PhotocraftApp, ctx: &egui::Context, released: bool, root_point: Option<egui::Pos2>) {
    if app.session.prefs().workspace_locked {
        if egui::DragAndDrop::payload::<Payload>(ctx).is_some() {
            egui::DragAndDrop::clear_payload(ctx);
        }
        clear_drag_point(ctx);
        ctx.data_mut(|data| {
            data.remove::<u64>(egui::Id::new("panel-drag-whole-stack"));
            data.remove::<u64>(egui::Id::new("panel-title-drag-origin"));
        });
        return;
    }
    if released
        && let Some(payload) = egui::DragAndDrop::payload::<Payload>(ctx)
        && let Some(p) = root_point
    {
        let on_rail = ctx.data(|d| d.get_temp::<Rect>(egui::Id::new("panel-icon-rail-rect"))).is_some_and(|r| r.contains(p));
        let destination = if on_rail {
            None
        } else {
            merge_target(app, ctx, &payload, p).map(|zone| {
                // A whole group dragged within the dock column keeps the established
                // reorder interaction. Floating groups and individual tabs still merge.
                let reorder = payload.tab.is_none()
                    && app.ui.dock.containers.iter().any(|c| c.id == payload.container && !c.floating)
                    && app.ui.dock.containers.iter().any(|c| c.id == zone.id && !c.floating);
                (zone.id, if reorder { -1 } else { 0 })
            })
        };
        // Moving a native floating window without a drop target already updates its position.
        let free_root = destination.is_none()
            && !on_rail
            && payload.tab.is_none()
            && app.ui.dock.containers.iter().any(|c| c.id == payload.container && c.floating && c.stack.is_none());
        if !free_root {
            let whole_stack = ctx.data(|d| d.get_temp::<u64>(egui::Id::new("panel-drag-whole-stack"))) == Some(payload.container);
            let children: Vec<u64> =
                if whole_stack { app.ui.dock.containers.iter().filter(|c| c.stack == Some(payload.container)).map(|c| c.id).collect() } else { vec![] };
            drop_panel(&mut app.ui.dock.containers, &payload, destination, [p.x, p.y]);
            for id in children {
                drop_panel(&mut app.ui.dock.containers, &Payload { container: id, tab: None }, destination, [p.x, p.y]);
            }
        }
        if destination.is_none() && on_rail {
            let id = if payload.tab.is_some() { app.ui.dock.containers.iter().map(|c| c.id).max().unwrap_or(payload.container) } else { payload.container };
            for c in &mut app.ui.dock.containers {
                if c.id == id || c.stack == Some(id) {
                    c.floating = false;
                    c.stack = None;
                }
            }
        }
        egui::DragAndDrop::clear_payload(ctx);
        clear_drag_point(ctx);
        ctx.data_mut(|d| {
            d.remove::<u64>(egui::Id::new("panel-drag-whole-stack"));
        });
        let mut order = Vec::new();
        for tab in app.ui.dock.containers.iter().flat_map(|c| c.tabs.iter()) {
            if !order.contains(&tab.group) {
                order.push(tab.group);
            }
        }
        app.ui.dock.order = order;
        // The new containers are created after this frame's windows were drawn.
        // Show the tear-off/merge immediately, even when no further input arrives.
        ctx.request_repaint();
    }
    if released {
        clear_drag_point(ctx);
        ctx.data_mut(|d| d.remove::<u64>(egui::Id::new("panel-title-drag-origin")));
    }
}

pub fn drop_panel(containers: &mut Vec<Container>, payload: &Payload, target: Option<(u64, i8)>, position: [f32; 2]) {
    let Some(source) = containers.iter().position(|c| c.id == payload.container) else { return };
    if target.is_some_and(|(id, _)| id == payload.container) {
        return;
    }
    let original = containers.get(source).cloned().unwrap_or_default();
    let tabs = if let Some(index) = payload.tab {
        let Some(tab) = containers.get_mut(source).and_then(|c| (index < c.tabs.len()).then(|| c.tabs.remove(index))) else { return };
        vec![tab]
    } else {
        containers.get_mut(source).map(|c| std::mem::take(&mut c.tabs)).unwrap_or_default()
    };
    if tabs.is_empty() {
        return;
    }
    let new_id = containers.iter().map(|c| c.id).max().unwrap_or(7).saturating_add(1);
    let mut moved = Container { id: new_id, tabs, floating: true, position, stack: None, selected: 0, closed: false, ..original };
    let target_index = target.and_then(|(id, _)| containers.iter().position(|c| c.id == id));
    if let (Some(index), Some((_, edge))) = (target_index, target) {
        if edge == 0 {
            if let Some(c) = containers.get_mut(index) {
                c.selected = c.tabs.len();
                c.tabs.append(&mut moved.tabs);
            }
        } else {
            if let Some(c) = containers.get(index) {
                moved.floating = c.floating;
                if c.floating {
                    moved.stack = Some(c.stack.unwrap_or(c.id));
                }
            }
            let at = if edge < 0 { index } else { index.saturating_add(1) };
            containers.insert(at.min(containers.len()), moved);
        }
    } else if payload.tab.is_none() && containers.get(source).is_some_and(|c| c.floating) {
        if let Some(c) = containers.get_mut(source) {
            c.tabs = moved.tabs;
            c.stack = None;
            c.position = position;
        }
    } else {
        containers.push(moved);
    }
    // A removed root promotes one child so floating stacks can never disappear.
    let empty_ids: Vec<u64> = containers.iter().filter(|c| c.tabs.is_empty()).map(|c| c.id).collect();
    for root in empty_ids {
        if let Some(child) = containers.iter().find(|c| c.stack == Some(root)).map(|c| c.id) {
            for c in containers.iter_mut() {
                if c.id == child {
                    c.stack = None;
                } else if c.stack == Some(root) {
                    c.stack = Some(child);
                }
            }
        }
    }
    containers.retain(|c| !c.tabs.is_empty());
}

/// Do not run floating `egui::Window` layout while the native root is minimized or has entered
/// the transient zero-sized phase of a Windows minimize. A minimized frame can report a 0×0
/// content rect; feeding that through `current_pos`/`response.rect` overwrites every persisted
/// panel position with the primary-monitor origin. The next visible frame can safely lay out the
/// windows again from their last saved geometry.
fn root_layout_suspended(ctx: &egui::Context) -> bool {
    ctx.input(|i| {
        let Some(root) = i.raw.viewports.get(&egui::ViewportId::ROOT) else { return false };
        root.minimized == Some(true) || root.visible() == Some(false) || root.inner_rect.is_some_and(|rect| rect.width() < 16.0 || rect.height() < 16.0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adjustment_grid_and_mixed_layers_thumbnail_keep_the_properties_tab_requested_by_the_body() {
        use egui_kittest::kittest::Queryable;
        for theme in crate::theme::ThemeKind::ALL {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", serde_json::json!({"width":32,"height":32})).unwrap();
            app.run("layer.newAdjustmentLayer.curves", serde_json::json!({})).unwrap();
            app.ui.panels.layers = true;
            app.ui.panels.properties = true;
            app.ui.dock.containers = vec![Container {
                id: 93,
                tabs: vec![Tab { group: Group::Layers, index: 0 }, Tab { group: Group::Properties, index: 0 }, Tab { group: Group::Properties, index: 1 }],
                selected: 2,
                ..Default::default()
            }];
            let mut h = egui_kittest::Harness::builder().with_size(vec2(600.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                        let c = app.ui.dock.containers[0].clone();
                        body(app, ui, &c);
                    }
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, theme);
            h.run_steps(3);
            let label = crate::i18n::tr(crate::i18n::current(), "Vibrance");
            h.get_by_label(label).click();
            h.run_steps(3);
            assert_eq!(h.state().ui.dock.containers[0].selected, 1, "{theme:?}: grid callback selection was overwritten");
            assert_eq!(h.state().ui.dock_tabs.properties, 0);
            let layer = h.state().session.active().unwrap().active_layer.unwrap();
            h.state_mut().ui.dock.containers[0].selected = 0;
            h.run_steps(3);
            let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == layer.0).unwrap();
            let p = pos2(row.eye.right() + 17.0, row.row.center().y);
            for _ in 0..2 {
                h.event(egui::Event::PointerMoved(p));
                h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
                h.step();
                h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
                h.step();
            }
            h.run_steps(3);
            assert_eq!(h.state().ui.dock.containers[0].selected, 1, "{theme:?}: mixed Layers thumbnail reveal was overwritten");
            assert_eq!(h.state().ui.dock_tabs.properties, 0);
        }
    }
    #[test]
    fn migrating_default_studio_layout_does_not_open_an_empty_properties_window() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        migrate(&mut app, &ctx);
        let properties = app.ui.dock.containers.iter().find(|c| c.tabs.iter().any(|t| t.group == Group::Properties)).unwrap();
        assert!(properties.auto_properties);
        assert!(visible(&app, properties).is_empty());
        crate::dock::reveal(&mut app, Group::Properties);
        let properties = app.ui.dock.containers.iter().find(|c| c.tabs.iter().any(|t| t.group == Group::Properties)).unwrap();
        assert!(!properties.auto_properties);
        assert!(!visible(&app, properties).is_empty());
    }

    #[test]
    fn floating_panel_body_stays_put_after_a_touch_device_is_detected() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.ui.panels.layers = true;
        app.ui.dock.containers = vec![Container {
            id: 71,
            tabs: vec![Tab { group: Group::Layers, index: 0 }],
            floating: true,
            position: [100., 100.],
            size: [320., 400.],
            ..Default::default()
        }];
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1000., 800.)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                app.frame += 1;
                egui::CentralPanel::default().show(ui, |_| {});
                floating(app, &ctx);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(6);
        for phase in [egui::TouchPhase::Start, egui::TouchPhase::End] {
            h.event(egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase, pos: pos2(900., 700.), force: Some(1.) });
            h.run_steps(1);
        }
        assert!(h.ctx.input(|i| i.has_touch_screen()));
        let id = egui::Id::new(("panel-container", 71u64));
        let before = h.ctx.memory(|m| m.area_rect(id)).unwrap();
        for title in [false, true] {
            let rect = h.ctx.memory(|m| m.area_rect(id)).unwrap();
            let start = if title { pos2(rect.center().x, rect.top() + 14.) } else { rect.center() };
            let end = start + vec2(70., 40.);
            h.event(egui::Event::PointerMoved(start));
            h.run_steps(1);
            h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
            for step in 1..=8 {
                h.event(egui::Event::PointerMoved(start + (end - start) * (step as f32 / 8.)));
                h.run_steps(2);
            }
            h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            h.run_steps(4);
            let after = h.ctx.memory(|m| m.area_rect(id)).unwrap();
            if title {
                assert!((after.min - before.min).length() > 30., "{before:?} -> {after:?}");
            } else {
                assert!((after.min - before.min).length() < 1., "{before:?} -> {after:?}");
            }
        }
    }

    #[test]
    fn minimized_or_zero_sized_root_suspends_floating_panel_layout() {
        let ctx = egui::Context::default();
        let viewport = egui::ViewportId::ROOT;
        let mut input = egui::RawInput::default();
        input.viewports.entry(viewport).or_default().minimized = Some(true);
        let mut out = ctx.run_ui(input, |_| {});
        out.textures_delta.clear();
        assert!(root_layout_suspended(&ctx));

        let mut input = egui::RawInput::default();
        let root = input.viewports.entry(viewport).or_default();
        root.minimized = Some(false);
        root.inner_rect = Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1.0, 1.0)));
        let mut out = ctx.run_ui(input, |_| {});
        out.textures_delta.clear();
        assert!(root_layout_suspended(&ctx));

        let mut input = egui::RawInput::default();
        let root = input.viewports.entry(viewport).or_default();
        root.minimized = Some(false);
        root.inner_rect = Some(Rect::from_min_size(pos2(40.0, 30.0), vec2(900.0, 700.0)));
        let mut out = ctx.run_ui(input, |_| {});
        out.textures_delta.clear();
        assert!(!root_layout_suspended(&ctx));
    }
    #[test]
    fn dragging_a_real_floating_title_joins_the_target_and_windows_stay_stable() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", serde_json::json!({"width":80,"height":60})).unwrap();
        app.ui.panels.brush_settings = true;
        app.ui.panels.color = true;
        app.ui.dock.containers = vec![
            Container {
                id: 10,
                tabs: vec![Tab { group: Group::Brush, index: 0 }, Tab { group: Group::Brush, index: 1 }],
                floating: true,
                position: [10., 80.],
                size: [430., 620.],
                ..Default::default()
            },
            Container {
                id: 11,
                tabs: vec![Tab { group: Group::Color, index: 1 }],
                floating: true,
                position: [540., 80.],
                size: [320., 500.],
                ..Default::default()
            },
        ];
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1300., 900.)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                app.frame += 1;
                egui::CentralPanel::default().show(ui, |_| {});
                floating(app, &ctx);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(6);
        let source = h.ctx.memory(|m| m.area_rect(egui::Id::new(("panel-container", 10u64)))).unwrap();
        let start = pos2(source.center().x, source.top() + 14.);
        let end = h.ctx.data(|data| data.get_temp::<Vec<Zone>>(zones_id())).unwrap().iter().find(|zone| zone.id == 11).unwrap().rect.center();
        h.event(egui::Event::PointerMoved(start));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        for i in 1..=12 {
            h.event(egui::Event::PointerMoved(start + (end - start) * (i as f32 / 12.)));
            h.run_steps(2);
        }
        h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(4);
        let target = h.state().ui.dock.containers.iter().find(|c| c.id == 11).unwrap();
        assert_eq!(target.tabs.len(), 3, "{:?}", h.state().ui.dock.containers);
        assert!(target.tabs.contains(&Tab { group: Group::Brush, index: 1 }));
        let first = h.ctx.memory(|m| m.area_rect(egui::Id::new(("panel-container", 11u64)))).unwrap();
        h.run_steps(20);
        let last = h.ctx.memory(|m| m.area_rect(egui::Id::new(("panel-container", 11u64)))).unwrap();
        assert!((last.width() - first.width()).abs() < 1. && (last.height() - first.height()).abs() < 1., "{first:?}->{last:?}");
    }
    #[test]
    fn tabs_tear_off_join_and_stack_without_loss() {
        let mut c = vec![
            Container { id: 1, tabs: vec![Tab { group: Group::Layers, index: 0 }, Tab { group: Group::Layers, index: 1 }], ..Default::default() },
            Container { id: 2, tabs: vec![Tab { group: Group::Color, index: 1 }], floating: true, ..Default::default() },
        ];
        drop_panel(&mut c, &Payload { container: 1, tab: Some(1) }, None, [50.0, 60.0]);
        assert_eq!(c.len(), 3);
        let detached = c.last().unwrap().id;
        assert_eq!(c.last().unwrap().position, [50.0, 60.0]);
        drop_panel(&mut c, &Payload { container: detached, tab: None }, Some((2, 1)), [0.0; 2]);
        assert_eq!(c.iter().filter(|v| v.stack == Some(2)).count(), 1);
        let child = c.iter().find(|v| v.stack == Some(2)).unwrap().id;
        drop_panel(&mut c, &Payload { container: child, tab: None }, Some((1, 0)), [0.0; 2]);
        assert_eq!(c.iter().map(|v| v.tabs.len()).sum::<usize>(), 3);
        assert_eq!(c.iter().find(|v| v.id == 1).unwrap().tabs.len(), 2);
        let saved = serde_json::to_value(&c).unwrap();
        assert_eq!(serde_json::from_value::<Vec<Container>>(saved).unwrap(), c);
    }
}

#[cfg(test)]
mod performance_tests {
    use super::*;
    #[test]
    #[ignore = "release floating-panel drag benchmark with 20 MB brush assets"]
    fn floating_drag_merge_perf() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.ui.panels.brush_settings = true;
        app.ui.panels.color = true;
        app.session.tools.brush.tip =
            photocraft_engine::paint::TipShape::Sampled(photocraft_engine::paint::GrayTile::from_fn(2500, 2500, |x, y| ((x + y) % 17) as f32 / 16.));
        app.session.tools.brush.texture.pattern =
            photocraft_engine::paint::Pattern::Tile(photocraft_engine::paint::GrayTile::from_fn(2048, 2048, |x, y| ((x + y) % 31) as f32 / 30.));
        app.ui.dock.containers = vec![
            Container {
                id: 10,
                tabs: vec![Tab { group: Group::Brush, index: 0 }],
                floating: true,
                position: [10., 80.],
                size: [430., 620.],
                ..Default::default()
            },
            Container {
                id: 11,
                tabs: vec![Tab { group: Group::Color, index: 1 }],
                floating: true,
                position: [700., 80.],
                size: [320., 500.],
                ..Default::default()
            },
        ];
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1400., 900.)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                app.frame += 1;
                egui::CentralPanel::default().show(ui, |_| {});
                floating(app, &ctx);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(8);
        let start = h.ctx.memory(|m| m.area_rect(egui::Id::new(("panel-container", 10u64)))).unwrap();
        let start = pos2(start.center().x, start.top() + 14.);
        let end = h.ctx.data(|data| data.get_temp::<Vec<Zone>>(zones_id())).unwrap().iter().find(|zone| zone.id == 11).unwrap().rect.center();
        h.event(egui::Event::PointerMoved(start));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: start, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        let mut frames = vec![];
        for i in 1..=60 {
            h.event(egui::Event::PointerMoved(start + (end - start) * (i as f32 / 60.)));
            let time = std::time::Instant::now();
            h.run_steps(1);
            frames.push(time.elapsed().as_secs_f64() * 1000.);
        }
        h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        let time = std::time::Instant::now();
        h.run_steps(1);
        let merge = time.elapsed().as_secs_f64() * 1000.;
        h.run_steps(4);
        assert_eq!(h.state().ui.dock.containers.iter().find(|c| c.id == 11).unwrap().tabs.len(), 2);
        frames.sort_by(f64::total_cmp);
        println!("floating drag 20 MB assets: median_ms={:.3}, p95_ms={:.3}, merge_frame_ms={merge:.3}", frames[30], frames[57]);
    }
}

#[cfg(test)]
#[path = "panel_workspace/merge_tests_v74.rs"]
mod merge_tests_v74;
