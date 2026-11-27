//! The window's own title bar on Windows and Linux, where the window has no OS decorations
//! ([`PhotocraftApp::custom_titlebar`]): the app's top bar (`panels::title_bar`) is the title bar,
//! as in Photoshop on Windows. The free gap between its menus and controls drags the window and a
//! double-click there maximizes it (`panels::title_bar`, on every platform); caption buttons at
//! the far right minimize, maximize/restore and close, and thin invisible zones along the window's
//! edges resize it.

use egui::{CursorIcon, Id, LayerId, Order, PointerButton, Rect, ResizeDirection, Sense, Stroke, Ui, Vec2, ViewportCommand, ViewportId, pos2, vec2};
use serde_json::json;

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// One caption button: as wide as the native Windows ones.
pub const BUTTON_WIDTH: f32 = 46.0;
/// The three caption buttons together.
pub const WIDTH: f32 = 3.0 * BUTTON_WIDTH;
/// Resize zone thickness along an edge, and the side of a corner zone.
const EDGE: f32 = 5.0;
const CORNER: f32 = 12.0;
// Detached documents need a slightly wider hit target than tool panels. These are input
// targets only: increasing them must not make the painted border or title bar any thicker.
const FRAME_EDGE: f32 = 7.0;
const FRAME_CORNER: f32 = 18.0;
/// The smallest detached canvas that still leaves room for its title bar and rounded frame.
/// Keep this in sync with `document_window_ui::detached_viewport_builder`; using a smaller
/// egui minimum than the native viewport minimum makes touch resizing appear to stop early.
pub const MIN_INNER_WIDTH: f32 = 160.0;
pub const MIN_INNER_HEIGHT: f32 = 96.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Caption {
    Minimize,
    Maximize,
    Close,
}

/// Left to right.
const CAPTIONS: [Caption; 3] = [Caption::Minimize, Caption::Maximize, Caption::Close];

/// Caption button ids (stable, so tests and agents can find them).
fn caption_id(c: Caption) -> Id {
    Id::new(("titlebar-caption", c as u8))
}

fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

fn toggle_maximize(ctx: &egui::Context) {
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized(ctx)));
}

/// Minimize, Maximize/Restore and Close at the right end of `bar` (the title bar's outer rect),
/// full bar height, flush with the window's edge (even if the bar's contents run wider) so a
/// maximized window's corner hits Close. Close runs File › Exit, which asks about unsaved
/// documents first.
pub fn caption_buttons(app: &mut PhotocraftApp, ui: &mut Ui, bar: Rect) {
    let t = Tokens::get(ui.ctx());
    let max = maximized(ui.ctx());
    let right = bar.right().min(ui.ctx().content_rect().right());
    let mut clicked = None;
    for (i, c) in CAPTIONS.into_iter().enumerate() {
        let r = Rect::from_min_size(pos2(right - WIDTH + i as f32 * BUTTON_WIDTH, bar.top()), vec2(BUTTON_WIDTH, bar.height()));
        let resp = ui.interact(r, caption_id(c), Sense::click());
        let close = c == Caption::Close;
        let fill = match (resp.is_pointer_button_down_on(), resp.hovered()) {
            (true, _) if close => t.caption_close.gamma_multiply(0.8),
            (_, true) if close => t.caption_close,
            (true, _) => t.pressed,
            (_, true) => t.hover,
            _ => egui::Color32::TRANSPARENT,
        };
        ui.painter().rect_filled(r, 0.0, fill);
        let ink = if close && resp.hovered() { t.caption_close_text } else { t.icon };
        paint_glyph(ui, c, max, r.center(), Stroke::new(1.0, ink));
        let tip = match c {
            Caption::Minimize => tl!("Minimize"),
            Caption::Maximize if max => tl!("Restore"),
            Caption::Maximize => tl!("Maximize"),
            Caption::Close => tl!("Close"),
        };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tip));
        if resp.on_hover_text(tip).clicked() {
            clicked = Some(c);
        }
    }
    match clicked {
        Some(Caption::Minimize) => ui.ctx().send_viewport_cmd(ViewportCommand::Minimized(true)),
        Some(Caption::Maximize) => toggle_maximize(ui.ctx()),
        Some(Caption::Close) => {
            let ctx = ui.ctx().clone();
            let _ = crate::menus::invoke(app, &ctx, "file.exit", json!({}));
        }
        None => {}
    }
}

/// The 10 pt line glyphs of the caption buttons.
fn paint_glyph(ui: &Ui, c: Caption, maximized: bool, at: egui::Pos2, s: Stroke) {
    let p = ui.painter();
    let g = Rect::from_center_size(at, vec2(10.0, 10.0));
    match c {
        Caption::Minimize => {
            p.line_segment([g.left_center(), g.right_center()], s);
        }
        // Restore: a front square with the corner of the one behind it.
        Caption::Maximize if maximized => {
            let front = Rect::from_min_max(g.min + vec2(0.0, 2.0), g.max - vec2(2.0, 0.0));
            p.rect_stroke(front, 0.0, s, egui::StrokeKind::Middle);
            p.line(vec![g.min + vec2(2.0, 2.0), g.min + vec2(2.0, 0.0), g.right_top(), g.max - vec2(0.0, 2.0), g.max - vec2(2.0, 2.0)], s);
        }
        Caption::Maximize => {
            p.rect_stroke(g, 0.0, s, egui::StrokeKind::Middle);
        }
        Caption::Close => {
            p.line_segment([g.left_top(), g.right_bottom()], s);
            p.line_segment([g.right_top(), g.left_bottom()], s);
        }
    }
}

/// The edge and corner zones of a window with content rect `w`: (zone, direction, cursor). Corners
/// come last so they win where they overlap the edges.
fn zones(w: Rect, inset: f32, document_frame: bool) -> [(Rect, ResizeDirection, CursorIcon); 8] {
    use ResizeDirection::*;
    // Transparent hosts reserve a shadow gutter. Include the visible frame contour,
    // rather than placing every straight hit target outside that contour.
    let (edge, corner) = if document_frame { (FRAME_EDGE, FRAME_CORNER) } else { (EDGE, CORNER) };
    let (e, c) = (edge + inset, corner + inset);
    [
        (Rect::from_min_max(w.left_top(), pos2(w.right(), w.top() + e)), North, CursorIcon::ResizeVertical),
        (Rect::from_min_max(pos2(w.left(), w.bottom() - e), w.right_bottom()), South, CursorIcon::ResizeVertical),
        (Rect::from_min_max(w.left_top(), pos2(w.left() + e, w.bottom())), West, CursorIcon::ResizeHorizontal),
        (Rect::from_min_max(pos2(w.right() - e, w.top()), w.right_bottom()), East, CursorIcon::ResizeHorizontal),
        (Rect::from_min_size(w.left_top(), vec2(c, c)), NorthWest, CursorIcon::ResizeNwSe),
        (Rect::from_min_size(w.right_top() - vec2(c, 0.0), vec2(c, c)), NorthEast, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.left_bottom() - vec2(0.0, c), vec2(c, c)), SouthWest, CursorIcon::ResizeNeSw),
        (Rect::from_min_size(w.right_bottom() - vec2(c, c), vec2(c, c)), SouthEast, CursorIcon::ResizeNwSe),
    ]
}

/// Raw title gestures must exclude the same edge targets as the resize widgets.
pub(crate) fn frame_resize_contains(ctx: &egui::Context, point: egui::Pos2, inset: f32) -> bool {
    resize_contains(ctx, point, inset, false)
}

pub(crate) fn document_frame_resize_contains(ctx: &egui::Context, point: egui::Pos2, inset: f32) -> bool {
    resize_contains(ctx, point, inset, true)
}

fn resize_contains(ctx: &egui::Context, point: egui::Pos2, inset: f32, document_frame: bool) -> bool {
    !maximized(ctx)
        && !ctx.input(|i| i.viewport().fullscreen.unwrap_or(false))
        && zones(ctx.content_rect(), inset, document_frame).iter().any(|(rect, _, _)| rect.contains(point))
}

/// Claim a frame press before canvas tools read the same raw events. Native resize widgets are
/// drawn last so their cursor wins, but ownership must already be decided in that first frame.
/// Looking at the press (not the hover/move) lets an existing brush stroke cross the edge freely.
pub(crate) fn document_frame_resize_owns_input(ctx: &egui::Context, inset: f32) -> bool {
    if resize_active(ctx, ctx.viewport_id()) {
        return true;
    }
    if maximized(ctx) || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
        return false;
    }
    let targets = zones(ctx.content_rect(), inset, true);
    ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|event| match event {
                egui::Event::Touch { phase: egui::TouchPhase::Start, pos, .. }
                | egui::Event::PointerButton { button: PointerButton::Primary, pressed: true, pos, .. } => Some(*pos),
                _ => None,
            })
            .any(|pos| targets.iter().any(|(rect, _, _)| rect.contains(pos)))
    })
}

#[derive(Clone, Copy)]
struct PenResize {
    direction: ResizeDirection,
    /// Desktop pixels with native sampling, otherwise viewport-local points.
    start: egui::Pos2,
    /// Initial native geometry in physical pixels, or logical points for the fallback.
    outer: egui::Pos2,
    size: Vec2,
    last: egui::Pos2,
    contact: (egui::TouchDeviceId, egui::TouchId),
    native_screen: bool,
    min_size: Vec2,
}

#[derive(Clone, Copy)]
struct MouseResize {
    direction: ResizeDirection,
    /// Desktop pixels when the platform cursor service is available, otherwise local points.
    start: egui::Pos2,
    /// Initial outer position in physical pixels, or logical points for the fallback.
    outer: egui::Pos2,
    /// Initial inner size in physical pixels, or logical points for the fallback.
    size: Vec2,
    last: egui::Pos2,
    native_screen: bool,
    min_size: Vec2,
}

fn pen_resize_id(viewport: ViewportId) -> Id {
    Id::new(("titlebar-pen-resize", viewport))
}

fn mouse_resize_id(viewport: ViewportId) -> Id {
    Id::new(("titlebar-mouse-resize", viewport))
}

#[derive(Clone, Copy)]
struct ResizeSample {
    direction: ResizeDirection,
    start: egui::Pos2,
    outer: egui::Pos2,
    size: Vec2,
    last: egui::Pos2,
    native_screen: bool,
    min_size: Vec2,
}

pub(crate) fn resize_active(ctx: &egui::Context, viewport: ViewportId) -> bool {
    ctx.data(|d| d.get_temp::<PenResize>(pen_resize_id(viewport)).is_some() || d.get_temp::<MouseResize>(mouse_resize_id(viewport)).is_some())
}

pub(crate) fn cancel_resize(ctx: &egui::Context, viewport: ViewportId) {
    ctx.data_mut(|d| {
        d.remove::<PenResize>(pen_resize_id(viewport));
        d.remove::<MouseResize>(mouse_resize_id(viewport));
    });
}

fn resize_position(ctx: &egui::Context, contact: (egui::TouchDeviceId, egui::TouchId)) -> Option<egui::Pos2> {
    ctx.input(|i| {
        // Windows often emits a synthetic PointerMoved immediately after Touch::Move. Never use
        // that emulated sample for a pen resize: its coordinates belong to the pre-resize
        // viewport and would make the native window jump. A missing touch sample simply leaves
        // the current size in place until the next real Move arrives.
        i.events.iter().rev().find_map(|event| match event {
            egui::Event::Touch { device_id, id, phase: egui::TouchPhase::Move | egui::TouchPhase::End, pos, .. } if (*device_id, *id) == contact => Some(*pos),
            _ => None,
        })
    })
}

/// Mouse leave and synthesized releases can accompany a still-active pen. Only its own
/// terminal event or actual focus loss releases a contact-owned resize.
fn pen_resize_ended(ctx: &egui::Context, contact: (egui::TouchDeviceId, egui::TouchId)) -> bool {
    ctx.input(|i| {
        i.events.iter().any(|event| match event {
            egui::Event::Touch { device_id, id, phase: egui::TouchPhase::End | egui::TouchPhase::Cancel, .. } => (*device_id, *id) == contact,
            egui::Event::WindowFocused(false) => true,
            _ => false,
        })
    })
}

fn resize_geometry(ctx: &egui::Context, state: ResizeSample, pos: egui::Pos2) -> Option<(Vec2, egui::Pos2, f32)> {
    if !pos.is_finite() || state.last.distance_sq(pos) <= 0.25 {
        return None;
    }
    let scale = if state.native_screen { ctx.pixels_per_point().max(f32::EPSILON) } else { 1.0 };
    let delta = (pos - state.start) / scale;
    let initial_size = state.size / scale;
    let (mut size, mut outer) = (initial_size, state.outer / scale);
    let west = matches!(state.direction, ResizeDirection::West | ResizeDirection::NorthWest | ResizeDirection::SouthWest);
    let north = matches!(state.direction, ResizeDirection::North | ResizeDirection::NorthWest | ResizeDirection::NorthEast);
    let east = matches!(state.direction, ResizeDirection::East | ResizeDirection::NorthEast | ResizeDirection::SouthEast);
    let south = matches!(state.direction, ResizeDirection::South | ResizeDirection::SouthWest | ResizeDirection::SouthEast);
    if east {
        size.x += delta.x;
    }
    if south {
        size.y += delta.y;
    }
    if west {
        size.x -= delta.x;
    }
    if north {
        size.y -= delta.y;
    }
    size = size.max(state.min_size);
    // When the native minimum is reached, keep the opposite edge anchored. Applying the raw
    // pointer delta here moves the whole window while the size is clamped, so the next samples
    // appear to stop following the pen and can no longer recover when the pen reverses direction.
    if west {
        outer.x += initial_size.x - size.x;
    }
    if north {
        outer.y += initial_size.y - size.y;
    }
    Some((size, outer, scale))
}

fn resize_moves_origin(direction: ResizeDirection) -> bool {
    matches!(direction, ResizeDirection::West | ResizeDirection::NorthWest | ResizeDirection::SouthWest | ResizeDirection::North | ResizeDirection::NorthEast)
}

fn apply_pen_resize(ctx: &egui::Context, state: PenResize, pos: egui::Pos2) -> bool {
    let sample = ResizeSample {
        direction: state.direction,
        start: state.start,
        outer: state.outer,
        size: state.size,
        last: state.last,
        native_screen: state.native_screen,
        min_size: state.min_size,
    };
    let Some((size, outer, scale)) = resize_geometry(ctx, sample, pos) else {
        return false;
    };
    ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
    if resize_moves_origin(state.direction) {
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(outer));
    }
    crate::document_window_ui::record_window_gesture(
        ctx,
        json!({
            "gesture":"resize", "phase":"move", "direction":format!("{:?}",state.direction), "viewport":format!("{:?}",ctx.viewport_id()),
            "time":ctx.input(|i| i.time), "contact":state.contact.1.0,
            "coordinateSource":if state.native_screen {"nativeContact"} else {"localFallback"},
            "screenPx":[pos.x,pos.y], "target":[outer.x,outer.y], "size":[size.x,size.y], "ppp":scale
        }),
    );
    true
}

fn apply_mouse_resize(ctx: &egui::Context, state: MouseResize, pos: egui::Pos2) -> bool {
    let sample = ResizeSample {
        direction: state.direction,
        start: state.start,
        outer: state.outer,
        size: state.size,
        last: state.last,
        native_screen: state.native_screen,
        min_size: state.min_size,
    };
    let Some((size, outer, scale)) = resize_geometry(ctx, sample, pos) else {
        return false;
    };
    ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
    if resize_moves_origin(state.direction) {
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(outer));
    }
    crate::document_window_ui::record_window_gesture(
        ctx,
        json!({
            "gesture":"resize", "phase":"move", "direction":format!("{:?}",state.direction), "viewport":format!("{:?}",ctx.viewport_id()),
            "time":ctx.input(|i| i.time), "coordinateSource":if state.native_screen {"nativeMouse"} else {"localFallback"},
            "screenPx":[pos.x,pos.y], "target":[outer.x,outer.y], "size":[size.x,size.y], "ppp":scale
        }),
    );
    true
}

/// Invisible resize zones along the window's edges, above everything else; a press in one starts
/// an OS resize in that direction. None while the window is maximized or full screen. Root and
/// decorated windows keep the platform resize path; detached transparent windows call
/// [`resize_zones_for_frame`] so their client-side resize does not need a native frame.
pub fn resize_zones(ui: &mut Ui, screen_cursor_pos: &mut Option<crate::ContactScreenPosFn>) {
    let mut no_mouse = None;
    resize_zones_impl(ui, screen_cursor_pos, &mut no_mouse, 0.0, vec2(MIN_INNER_WIDTH, MIN_INNER_HEIGHT), false);
}

/// Client resizing for an inset floating frame. Match the host's minimum so a native
/// clamp cannot move its opposite edge when dragging the top or left border.
pub(crate) fn resize_zones_for_frame(
    ui: &mut Ui,
    screen_cursor_pos: &mut Option<crate::ContactScreenPosFn>,
    screen_mouse_pos: &mut Option<crate::ScreenCursorPosFn>,
    inset: f32,
    min_size: Vec2,
) {
    resize_zones_impl(ui, screen_cursor_pos, screen_mouse_pos, inset, min_size, false);
}

/// Detached canvases use the same state machine as tools, with their own larger hit targets.
pub(crate) fn resize_zones_for_document(
    ui: &mut Ui,
    screen_cursor_pos: &mut Option<crate::ContactScreenPosFn>,
    screen_mouse_pos: &mut Option<crate::ScreenCursorPosFn>,
    inset: f32,
    min_size: Vec2,
) {
    resize_zones_impl(ui, screen_cursor_pos, screen_mouse_pos, inset, min_size, true);
}

fn resize_zones_impl(
    ui: &mut Ui,
    screen_cursor_pos: &mut Option<crate::ContactScreenPosFn>,
    screen_mouse_pos: &mut Option<crate::ScreenCursorPosFn>,
    inset: f32,
    min_size: Vec2,
    document_frame: bool,
) {
    let ctx = ui.ctx().clone();
    let viewport = ctx.viewport_id();
    let state_id = pen_resize_id(viewport);
    if maximized(&ctx) || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
        if resize_active(&ctx, viewport) {
            ctx.data_mut(|d| {
                d.remove::<PenResize>(state_id);
                d.remove::<MouseResize>(mouse_resize_id(viewport));
            });
            ctx.request_repaint_of(ViewportId::ROOT);
        }
        return;
    }
    let w = ctx.content_rect();
    if let Some(mut state) = ctx.data(|d| d.get_temp::<PenResize>(state_id)) {
        let ended = pen_resize_ended(&ctx, state.contact);
        let cancelled = ctx.input(|i| {
            i.events.iter().any(|event| match event {
                egui::Event::Touch { device_id, id, phase: egui::TouchPhase::Cancel, .. } => (*device_id, *id) == state.contact,
                egui::Event::WindowFocused(false) => true,
                _ => false,
            })
        });
        let pos =
            if state.native_screen { screen_cursor_pos.as_mut().and_then(|read| read(state.contact.1.0, false)) } else { resize_position(&ctx, state.contact) };
        if let Some(pos) = pos.filter(|_| !cancelled)
            && apply_pen_resize(&ctx, state, pos)
        {
            state.last = pos;
            ctx.data_mut(|d| d.insert_temp(state_id, state));
            ctx.request_repaint_of(ViewportId::ROOT);
        }
        if ended {
            crate::document_window_ui::record_window_gesture(
                &ctx,
                json!({
                    "gesture":"resize", "phase":if cancelled {"cancel"} else {"end"},
                    "direction":format!("{:?}",state.direction), "viewport":format!("{viewport:?}"),
                    "contact":state.contact.1.0, "nativeScreen":state.native_screen,
                }),
            );
            ctx.data_mut(|d| d.remove::<PenResize>(state_id));
            ctx.request_repaint_of(ViewportId::ROOT);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            ctx.set_cursor_image(None);
            ctx.set_cursor_icon(match state.direction {
                ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
                ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
                ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
                ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
            });
        }
        // The synthesized pointer crosses egui's drag threshold after the Touch Start. It
        // must not start an OS resize or replace the fixed anchor of this gesture.
        return;
    }
    let mouse_state_id = mouse_resize_id(viewport);
    if let Some(mut state) = ctx.data(|d| d.get_temp::<MouseResize>(mouse_state_id)) {
        let (primary_down, focused, local_pos) = ctx.input(|i| (i.pointer.primary_down(), i.viewport().focused.unwrap_or(i.focused), i.pointer.interact_pos()));
        let cancelled = !focused;
        let ended = cancelled || !primary_down;
        let pos = if state.native_screen { screen_mouse_pos.as_mut().and_then(|read| read()) } else { local_pos };
        if let Some(pos) = pos.filter(|_| !cancelled)
            && apply_mouse_resize(&ctx, state, pos)
        {
            state.last = pos;
            ctx.data_mut(|d| d.insert_temp(mouse_state_id, state));
            ctx.request_repaint_of(ViewportId::ROOT);
        }
        if ended {
            crate::document_window_ui::record_window_gesture(
                &ctx,
                json!({
                    "gesture":"resize", "phase":if cancelled {"cancel"} else {"end"},
                    "direction":format!("{:?}",state.direction), "viewport":format!("{viewport:?}"),
                    "nativeScreen":state.native_screen, "source":"pointer",
                }),
            );
            ctx.data_mut(|d| d.remove::<MouseResize>(mouse_state_id));
            ctx.request_repaint_of(ViewportId::ROOT);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
            ctx.set_cursor_image(None);
            ctx.set_cursor_icon(match state.direction {
                ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
                ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
                ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
                ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
            });
        }
        return;
    }
    let touch_start = ctx.input(|i| {
        i.events.iter().rev().find_map(|event| match event {
            egui::Event::Touch { device_id, id, phase: egui::TouchPhase::Start, pos, .. }
                if zones(w, inset, document_frame).iter().any(|(rect, _, _)| rect.contains(*pos)) =>
            {
                Some((*pos, (*device_id, *id)))
            }
            _ => None,
        })
    });
    if let Some((local_start, contact)) = touch_start
        && let Some((_, direction, _)) = zones(w, inset, document_frame).into_iter().rev().find(|(r, _, _)| r.contains(local_start))
    {
        ctx.set_cursor_image(None);
        // A busy UI frame can receive the entire pen gesture in one batch. Apply its final
        // displacement, but never retain a contact whose matching terminal event is present.
        let ended = pen_resize_ended(&ctx, contact);
        let cancelled = ctx.input(|i| {
            i.events.iter().any(|event| match event {
                egui::Event::Touch { device_id, id, phase: egui::TouchPhase::Cancel, .. } => (*device_id, *id) == contact,
                egui::Event::WindowFocused(false) => true,
                _ => false,
            })
        });
        let (inner, outer) = ctx.input(|i| (i.viewport().inner_rect.map_or(w.size(), |r| r.size()), i.viewport().outer_rect.map_or(w.min, |r| r.min)));
        let screen = screen_cursor_pos.as_mut().and_then(|read| read(contact.1.0, true)).filter(|p| p.is_finite());
        let scale = if screen.is_some() { ctx.pixels_per_point().max(f32::EPSILON) } else { 1.0 };
        let start = screen.unwrap_or(local_start);
        let mut state =
            PenResize { direction, start, outer: outer * scale, size: inner * scale, last: start, contact, native_screen: screen.is_some(), min_size };
        let current = if state.native_screen { screen_cursor_pos.as_mut().and_then(|read| read(contact.1.0, false)) } else { resize_position(&ctx, contact) };
        if let Some(current) = current.filter(|_| !cancelled)
            && apply_pen_resize(&ctx, state, current)
        {
            state.last = current;
        }
        if !ended {
            ctx.data_mut(|d| d.insert_temp(state_id, state));
        }
        crate::document_window_ui::record_window_gesture(
            &ctx,
            json!({
                "gesture":"resize", "phase":if cancelled {"cancel"} else if ended {"complete"} else {"start"},
                "direction":format!("{direction:?}"), "viewport":format!("{viewport:?}"),
                "contact":contact.1.0, "nativeScreen":screen.is_some(), "local":[local_start.x,local_start.y], "inset":inset,
            }),
        );
        // The root frame owns geometry recovery and layering. Wake it so both are suspended
        // for this viewport while the child owns its native resize commands.
        ctx.request_repaint_of(ViewportId::ROOT);
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
        return;
    }
    let touch_input = ctx.input(|i| i.any_touches() || i.events.iter().any(|event| matches!(event, egui::Event::Touch { .. })));
    // Wacom's mouse-compatible path may have no Touch event at all. Start on the raw press,
    // using the same first-frame hit test as the pen, rather than depending on egui's previous
    // frame hover map and a later drag threshold. The native cursor supplies movement only.
    let pointer_start = ctx.input(|i| {
        i.events.iter().find_map(|event| match event {
            egui::Event::PointerButton { button: PointerButton::Primary, pressed: true, pos, .. } => Some(*pos),
            _ => None,
        })
    });
    if !touch_input
        && screen_mouse_pos.is_some()
        && let Some(local_start) = pointer_start
        && let Some((_, direction, _)) = zones(w, inset, document_frame).into_iter().rev().find(|(r, _, _)| r.contains(local_start))
    {
        let (inner, outer, inner_min) = ctx.input(|i| {
            (i.viewport().inner_rect.map_or(w.size(), |r| r.size()), i.viewport().outer_rect.map_or(w.min, |r| r.min), i.viewport().inner_rect.map(|r| r.min))
        });
        let screen = screen_mouse_pos.as_mut().and_then(|read| read()).filter(|p| p.is_finite());
        let native_screen = screen.is_some();
        let scale = if native_screen { ctx.pixels_per_point().max(f32::EPSILON) } else { 1.0 };
        let start =
            if native_screen { inner_min.map(|origin| (origin + local_start.to_vec2()) * scale).or(screen).unwrap_or(local_start) } else { local_start };
        let mut state = MouseResize { direction, start, outer: outer * scale, size: inner * scale, last: start, native_screen, min_size };
        let current = if native_screen { screen } else { ctx.input(|i| i.pointer.interact_pos()) };
        if let Some(current) = current
            && apply_mouse_resize(&ctx, state, current)
        {
            state.last = current;
        }
        if ctx.input(|i| i.pointer.primary_down()) {
            ctx.data_mut(|data| data.insert_temp(mouse_state_id, state));
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
        crate::document_window_ui::record_window_gesture(
            &ctx,
            json!({
                "gesture":"resize", "phase":"start", "direction":format!("{direction:?}"), "viewport":format!("{viewport:?}"),
                "nativeScreen":native_screen, "source":"pointer", "local":[local_start.x,local_start.y], "inset":inset,
            }),
        );
        ctx.set_cursor_image(None);
        ctx.request_repaint_of(ViewportId::ROOT);
        return;
    }
    let layer = LayerId::new(Order::Foreground, Id::new(("titlebar-resize", viewport)));
    let zui = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(w));
    for (i, (r, dir, cursor)) in zones(w, inset, document_frame).into_iter().enumerate() {
        let resp = zui.interact(r, Id::new(("titlebar-resize", viewport, i)), Sense::drag()).on_hover_cursor(cursor);
        if resp.hovered() || resp.dragged() {
            ctx.set_cursor_image(None);
        }
        if !touch_input && screen_mouse_pos.is_none() && resp.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Pos2, RawInput, ViewportId};
    use photocraft_engine::Session;

    #[test]
    fn resize_hover_clears_same_frame_tool_bitmap_only_over_edge() {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 300.0));
        for (pos, edge) in [(egui::pos2(200.0, 150.0), false), (egui::pos2(399.0, 150.0), true), (egui::pos2(200.0, 150.0), false)] {
            let mut out = ctx.run_ui(RawInput { screen_rect: Some(screen), events: vec![Event::PointerMoved(pos)], ..Default::default() }, |ui| {
                ui.ctx().set_cursor_image(Some(egui::CustomCursorImage { rgba: std::sync::Arc::from([255_u8; 4]), size: [1, 1], hotspot: [0, 0] }));
                resize_zones(ui, &mut None);
            });
            out.textures_delta.clear();
            assert_eq!(out.platform_output.cursor_image.is_none(), edge, "at {pos:?}");
            if edge {
                assert_eq!(out.platform_output.cursor_icon, CursorIcon::ResizeHorizontal);
            }
        }
    }

    /// A headless window `width` pt wide showing the title bar and the resize zones.
    struct Win {
        ctx: egui::Context,
        app: PhotocraftApp,
        width: f32,
        time: f64,
        maximized: bool,
    }

    impl Win {
        fn new(width: f32) -> Self {
            let ctx = egui::Context::default();
            PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
            let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
            app.custom_titlebar = true;
            Self { ctx, app, width, time: 0.0, maximized: false }
        }

        fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
            self.time += 0.05;
            let mut input =
                RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(self.width, 600.0))), time: Some(self.time), events, ..Default::default() };
            if let Some(v) = input.viewports.get_mut(&ViewportId::ROOT) {
                v.maximized = Some(self.maximized);
            }
            let app = &mut self.app;
            let mut out = self.ctx.run_ui(input, |ui| {
                crate::panels::title_bar(app, ui);
                resize_zones(ui, &mut app.services.contact_screen_pos);
            });
            out.textures_delta.clear();
            out
        }

        fn frame_with_mouse_resize(&mut self, events: Vec<Event>) -> egui::FullOutput {
            self.time += 0.05;
            let mut input =
                RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(self.width, 600.0))), time: Some(self.time), events, ..Default::default() };
            input.viewports.entry(ViewportId::ROOT).or_default().maximized = Some(self.maximized);
            let app = &mut self.app;
            let mut out = self.ctx.run_ui(input, |ui| {
                crate::panels::title_bar(app, ui);
                resize_zones_for_frame(
                    ui,
                    &mut app.services.contact_screen_pos,
                    &mut app.services.screen_cursor_pos,
                    0.0,
                    vec2(MIN_INNER_WIDTH, MIN_INNER_HEIGHT),
                );
            });
            out.textures_delta.clear();
            out
        }

        /// Frames for a pointer gesture: move to `path[0]`, press, move along `path`, release. The
        /// viewport commands the frames sent.
        fn gesture(&mut self, path: &[Pos2]) -> Vec<ViewportCommand> {
            let mut events = vec![vec![Event::PointerMoved(path[0])], vec![button(path[0], true)]];
            events.extend(path[1..].iter().map(|p| vec![Event::PointerMoved(*p)]));
            events.push(vec![button(*path.last().unwrap_or(&path[0]), false)]);
            events.into_iter().flat_map(|e| self.frame(e).viewport_output.remove(&ViewportId::ROOT).map(|v| v.commands).unwrap_or_default()).collect()
        }

        fn click(&mut self, p: Pos2) -> Vec<ViewportCommand> {
            self.gesture(&[p])
        }

        fn caption(&self, c: Caption) -> Rect {
            self.ctx.read_response(caption_id(c)).map(|r| r.rect).unwrap_or(Rect::NOTHING)
        }

        /// The bar's clickable widgets other than its drag gap and the caption buttons.
        fn bar_widgets(&self) -> Vec<Rect> {
            let skip = CAPTIONS.map(caption_id);
            self.ctx.viewport(|v| {
                v.prev_pass
                    .widgets
                    .layers()
                    .filter(|(l, _)| l.order != Order::Foreground)
                    .flat_map(|(_, ws)| ws.iter())
                    .filter(|w| w.sense.senses_click() && !w.sense.senses_drag() && w.rect.top() < 30.0 && !skip.contains(&w.id))
                    .map(|w| w.rect)
                    .collect()
            })
        }
    }

    fn texts(out: &egui::FullOutput) -> Vec<(String, Rect)> {
        fn walk(s: &egui::Shape, v: &mut Vec<(String, Rect)>) {
            match s {
                egui::Shape::Text(t) => v.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
                egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
                _ => {}
            }
        }
        let mut v = vec![];
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
        v
    }

    fn button(p: Pos2, pressed: bool) -> Event {
        Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    #[test]
    fn caption_buttons_take_the_right_end_and_nothing_overlaps_them() {
        for width in [760.0, 1100.0, 1440.0] {
            let mut w = Win::new(width);
            w.frame(vec![]);
            let out = w.frame(vec![]);
            let [min, max, close] = CAPTIONS.map(|c| w.caption(c));
            assert_eq!((close.right(), close.top()), (width, 0.0), "Close sits in the window's corner at {width}");
            assert!(close.height() >= 32.0 && [min, max, close].iter().all(|r| r.width() == BUTTON_WIDTH));
            assert_eq!((min.right(), max.right()), (max.left(), close.left()));
            let widgets = w.bar_widgets();
            assert!(widgets.len() >= 10, "menus, search, theme, workspace: {widgets:?}");
            for a in &widgets {
                assert!(a.right() <= min.left(), "{a:?} runs into the caption buttons at {width}");
            }
            assert!(!texts(&out).iter().any(|(t, _)| t == "Discord"), "removed community shortcut must stay absent at {width}");
        }
    }

    #[test]
    fn the_brand_mark_leads_the_bar() {
        let mut w = Win::new(1440.0);
        w.frame(vec![]);
        let out = w.frame(vec![]);
        let mark = crate::brand::mark_rect(&w.ctx).expect("the brand mark is painted");
        let file = texts(&out).into_iter().find(|(t, _)| t == "File").map(|(_, r)| r).expect("the File menu title");
        assert!(mark.right() < file.left(), "the mark sits left of the menus: {mark:?} {file:?}");
        assert!(mark.left() < 20.0 && mark.width() >= 16.0, "{mark:?}");
    }

    #[test]
    fn close_runs_file_exit_which_asks_about_unsaved_work() {
        let mut w = Win::new(1100.0);
        w.frame(vec![]);
        let close = w.caption(Caption::Close).center();
        let cmds = w.click(close);
        // File › Exit asks the window to close; the unsaved-changes guard (`discard_ui`) then
        // decides, so the caption button and the menu item behave the same.
        assert!(cmds.iter().any(|c| matches!(c, ViewportCommand::Close)), "{cmds:?}");
    }

    #[test]
    fn minimize_and_maximize_follow_the_window_state() {
        let mut w = Win::new(1100.0);
        w.frame(vec![]);
        let (min, max) = (w.caption(Caption::Minimize).center(), w.caption(Caption::Maximize).center());
        assert!(w.click(min).contains(&ViewportCommand::Minimized(true)));
        assert!(w.click(max).contains(&ViewportCommand::Maximized(true)));
        w.maximized = true;
        assert!(w.click(max).contains(&ViewportCommand::Maximized(false)), "Restore");
    }

    #[test]
    fn empty_bar_space_drags_and_double_click_maximizes() {
        let mut w = Win::new(1440.0);
        w.frame(vec![]);
        let empty = pos2(980.0, 16.0);
        assert!(w.bar_widgets().iter().all(|r| !r.contains(empty)), "the probe point must be empty bar space");
        let cmds = w.gesture(&[empty, empty + vec2(12.0, 4.0), empty + vec2(30.0, 8.0)]);
        assert_eq!(cmds.iter().filter(|c| matches!(c, ViewportCommand::StartDrag)).count(), 1, "{cmds:?}");
        let mut cmds = w.click(empty);
        cmds.extend(w.click(empty));
        assert!(cmds.contains(&ViewportCommand::Maximized(true)), "{cmds:?}");
        w.maximized = true;
        w.time += 1.0;
        let mut cmds = w.click(empty);
        cmds.extend(w.click(empty));
        assert!(cmds.contains(&ViewportCommand::Maximized(false)), "{cmds:?}");
        // Widgets keep their clicks: dragging from a menu title doesn't move the window.
        let file = texts(&w.frame(vec![])).into_iter().find(|(t, _)| t == "File").map(|(_, r)| r.center());
        let file = file.expect("the File menu title");
        let cmds = w.gesture(&[file, file + vec2(12.0, 4.0), file + vec2(30.0, 8.0)]);
        assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::StartDrag)), "{cmds:?}");
    }

    #[test]
    fn edge_zones_resize_unless_maximized() {
        use ResizeDirection::*;
        let mut w = Win::new(1000.0);
        w.frame(vec![]);
        for (p, dir) in [
            (pos2(1.0, 300.0), West),
            (pos2(998.0, 300.0), East),
            (pos2(500.0, 1.0), North),
            (pos2(500.0, 598.0), South),
            (pos2(2.0, 2.0), NorthWest),
            (pos2(997.0, 3.0), NorthEast),
            (pos2(3.0, 597.0), SouthWest),
            (pos2(998.0, 598.0), SouthEast),
        ] {
            let cmds = w.gesture(&[p, p + vec2(6.0, 6.0)]);
            assert_eq!(cmds, vec![ViewportCommand::BeginResize(dir)], "at {p:?}");
        }
        let out = w.frame(vec![Event::PointerMoved(pos2(1.0, 300.0))]);
        assert_eq!(out.platform_output.cursor_icon, CursorIcon::ResizeHorizontal);
        // Maximized: no zones, and the window's top-right corner is Close.
        w.maximized = true;
        let cmds = w.gesture(&[pos2(1.0, 300.0), pos2(8.0, 300.0)]);
        assert!(!cmds.iter().any(|c| matches!(c, ViewportCommand::BeginResize(_))), "{cmds:?}");
        assert!(w.click(pos2(999.5, 0.5)).iter().any(|c| matches!(c, ViewportCommand::Close)));
    }

    #[test]
    fn detached_mouse_resize_stays_in_the_client_area_without_native_begin_resize() {
        use std::cell::Cell;
        use std::rc::Rc;

        let mut w = Win::new(1000.0);
        let desktop = Rc::new(Cell::new(pos2(301.0, 500.0)));
        let sample = desktop.clone();
        w.app.services.screen_cursor_pos = Some(Box::new(move || Some(sample.get())));
        let edge = pos2(1.0, 300.0);
        let _ = w.frame_with_mouse_resize(vec![Event::PointerMoved(edge)]);
        let _ = w.frame_with_mouse_resize(vec![button(edge, true)]);
        desktop.set(pos2(341.0, 500.0));
        let out = w.frame_with_mouse_resize(vec![Event::PointerMoved(pos2(41.0, 300.0))]);
        let commands = out.viewport_output.get(&ViewportId::ROOT).map_or(&[][..], |v| v.commands.as_slice());
        assert!(commands.iter().any(|c| matches!(c, ViewportCommand::InnerSize(_))), "{commands:?}");
        assert!(commands.iter().any(|c| matches!(c, ViewportCommand::OuterPosition(_))), "{commands:?}");
        assert!(!commands.iter().any(|c| matches!(c, ViewportCommand::BeginResize(_))), "{commands:?}");
    }

    #[test]
    fn pen_resize_ignores_synthesized_pointer_up_and_releases_on_its_contact_end() {
        use egui::{Event, TouchDeviceId, TouchId, TouchPhase, ViewportId};

        let mut w = Win::new(1000.0);
        w.frame(vec![]);
        let start = pos2(998.0, 598.0);
        let touch = |phase, pos| Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase, pos, force: Some(0.6) };

        // A synthesized release does not own the pen's contact. Its own End must stop later
        // movement, even when the driver continues reporting pen hover or cached Move samples.
        let _ = w.frame(vec![touch(TouchPhase::Start, start)]);
        let moving = w.frame(vec![touch(TouchPhase::Move, start - vec2(30.0, 20.0))]);
        let moving_commands = moving.viewport_output.get(&ViewportId::ROOT).map_or(&[][..], |v| v.commands.as_slice());
        assert!(moving_commands.iter().any(|c| matches!(c, ViewportCommand::InnerSize(_))), "{moving_commands:?}");

        let _ = w.frame(vec![Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: false, modifiers: Default::default() }]);
        assert!(resize_active(&w.ctx, ViewportId::ROOT));
        let _ = w.frame(vec![touch(TouchPhase::End, start - vec2(30.0, 20.0))]);
        let after_release = w.frame(vec![touch(TouchPhase::Move, start - vec2(60.0, 40.0))]);
        let after_commands = after_release.viewport_output.get(&ViewportId::ROOT).map_or(&[][..], |v| v.commands.as_slice());
        assert!(!after_commands.iter().any(|c| matches!(c, ViewportCommand::InnerSize(_))), "stale pen resize continued: {after_commands:?}");
    }

    #[test]
    fn pen_resize_keeps_state_when_a_touch_move_sample_is_missing() {
        let ctx = egui::Context::default();
        let mut input = RawInput { focused: true, ..Default::default() };
        input.viewports.entry(ViewportId::ROOT).or_default().focused = Some(true);
        let mut out = ctx.run_ui(input, |_| {});
        out.textures_delta.clear();
        let contact = (egui::TouchDeviceId(1), egui::TouchId(1));
        assert!(!pen_resize_ended(&ctx, contact), "a focused pen gesture must survive a dropped Move sample");

        let mut input = RawInput { focused: false, events: vec![Event::WindowFocused(false)], ..Default::default() };
        input.viewports.entry(ViewportId::ROOT).or_default().focused = Some(false);
        let mut out = ctx.run_ui(input, |_| {});
        out.textures_delta.clear();
        assert!(pen_resize_ended(&ctx, contact), "focus loss must cancel the gesture");
    }

    #[test]
    fn pen_resize_uses_touch_position_instead_of_synthetic_pointer_position() {
        use egui::{TouchDeviceId, TouchId, TouchPhase};
        let ctx = egui::Context::default();
        let touch = pos2(200.0, 180.0);
        let synthetic = pos2(170.0, 150.0);
        let mut out = ctx.run_ui(
            RawInput {
                events: vec![
                    Event::Touch { device_id: TouchDeviceId(1), id: TouchId(1), phase: TouchPhase::Move, pos: touch, force: Some(0.6) },
                    Event::PointerMoved(synthetic),
                ],
                ..Default::default()
            },
            |_| {},
        );
        out.textures_delta.clear();
        assert_eq!(resize_position(&ctx, (TouchDeviceId(1), TouchId(1))), Some(touch));
    }

    fn resize_frame(
        ctx: &egui::Context,
        viewport: ViewportId,
        outer: Pos2,
        size: Vec2,
        events: Vec<Event>,
        screen_cursor: &mut Option<crate::ContactScreenPosFn>,
    ) -> Vec<ViewportCommand> {
        let mut input = RawInput { viewport_id: viewport, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() };
        let info = input.viewports.entry(viewport).or_default();
        info.inner_rect = Some(Rect::from_min_size(outer, size));
        info.outer_rect = info.inner_rect;
        info.focused = Some(true);
        info.native_pixels_per_point = Some(1.0);
        let mut out = ctx.run_ui(input, |ui| resize_zones(ui, screen_cursor));
        out.textures_delta.clear();
        out.viewport_output.remove(&viewport).map_or_else(Vec::new, |v| v.commands)
    }

    fn touch(id: u64, phase: egui::TouchPhase, pos: Pos2) -> Event {
        Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(id), phase, pos, force: Some(0.6) }
    }

    #[test]
    fn child_pen_resize_does_not_resize_or_cancel_the_root_viewport() {
        let ctx = egui::Context::default();
        let child = ViewportId::from_hash_of("resize-child");
        let mut screen_cursor = None;
        let outer = pos2(300.0, 200.0);
        let size = vec2(500.0, 400.0);
        resize_frame(&ctx, child, outer, size, vec![touch(1, egui::TouchPhase::Start, pos2(498.0, 398.0))], &mut screen_cursor);
        assert!(resize_active(&ctx, child));
        assert!(!resize_active(&ctx, ViewportId::ROOT));

        // The same Context serves both native windows. A global PenResize would consume this
        // root Move using the child's 500x400 geometry, then root PointerGone would delete it.
        let root_commands = resize_frame(
            &ctx,
            ViewportId::ROOT,
            Pos2::ZERO,
            vec2(1200.0, 900.0),
            vec![touch(1, egui::TouchPhase::Move, pos2(600.0, 450.0))],
            &mut screen_cursor,
        );
        assert!(!root_commands.iter().any(|c| matches!(c, ViewportCommand::InnerSize(_) | ViewportCommand::OuterPosition(_))), "{root_commands:?}");
        resize_frame(&ctx, ViewportId::ROOT, Pos2::ZERO, vec2(1200.0, 900.0), vec![Event::PointerGone], &mut screen_cursor);
        assert!(resize_active(&ctx, child), "a root frame must not cancel the child's contact");

        let child_commands = resize_frame(&ctx, child, outer, size, vec![touch(1, egui::TouchPhase::Move, pos2(478.0, 368.0))], &mut screen_cursor);
        assert!(child_commands.contains(&ViewportCommand::InnerSize(vec2(480.0, 370.0))), "{child_commands:?}");
        resize_frame(&ctx, child, outer, size, vec![touch(1, egui::TouchPhase::End, pos2(478.0, 368.0))], &mut screen_cursor);
        assert!(!resize_active(&ctx, child));
    }

    #[test]
    fn resize_tap_that_ends_in_the_start_frame_cannot_latch_the_gesture() {
        let ctx = egui::Context::default();
        let child = ViewportId::from_hash_of("resize-completed-contact");
        let mut screen_cursor = None;
        let pos = pos2(498.0, 398.0);
        let commands = resize_frame(
            &ctx,
            child,
            pos2(300.0, 200.0),
            vec2(500.0, 400.0),
            vec![touch(1, egui::TouchPhase::Start, pos), touch(1, egui::TouchPhase::End, pos)],
            &mut screen_cursor,
        );
        assert!(!resize_active(&ctx, child));
        assert!(!commands.iter().any(|c| matches!(c, ViewportCommand::InnerSize(_) | ViewportCommand::OuterPosition(_) | ViewportCommand::BeginResize(_))));
    }

    #[test]
    fn native_resize_uses_fixed_desktop_anchor_and_keeps_polling_without_touch_moves() {
        let ctx = egui::Context::default();
        let child = ViewportId::from_hash_of("native-resize-child");
        let desktop = std::rc::Rc::new(std::cell::Cell::new(pos2(302.0, 202.0)));
        let sample = desktop.clone();
        let mut screen_cursor: Option<crate::ContactScreenPosFn> =
            Some(Box::new(move |_, at_start| Some(if at_start { pos2(302.0, 202.0) } else { sample.get() })));
        let size = vec2(500.0, 400.0);
        resize_frame(&ctx, child, pos2(300.0, 200.0), size, vec![touch(1, egui::TouchPhase::Start, pos2(2.0, 2.0))], &mut screen_cursor);
        desktop.set(pos2(342.0, 222.0));
        let first = resize_frame(
            &ctx,
            child,
            pos2(300.0, 200.0),
            size,
            vec![touch(1, egui::TouchPhase::Move, pos2(42.0, 22.0)), Event::PointerGone],
            &mut screen_cursor,
        );
        assert!(first.contains(&ViewportCommand::InnerSize(vec2(460.0, 380.0))), "{first:?}");
        assert!(first.contains(&ViewportCommand::OuterPosition(pos2(340.0, 220.0))), "{first:?}");
        assert!(resize_active(&ctx, child));
        let stationary = resize_frame(
            &ctx,
            child,
            pos2(340.0, 220.0),
            vec2(460.0, 380.0),
            vec![touch(1, egui::TouchPhase::Move, pos2(42.0, 22.0)), touch(2, egui::TouchPhase::End, pos2(10.0, 10.0))],
            &mut screen_cursor,
        );
        assert!(!stationary.iter().any(|c| matches!(c, ViewportCommand::InnerSize(_) | ViewportCommand::OuterPosition(_) | ViewportCommand::BeginResize(_))));
        assert!(resize_active(&ctx, child), "another finger cannot end the pen gesture");
        desktop.set(pos2(352.0, 232.0));
        let polled = resize_frame(&ctx, child, pos2(340.0, 220.0), vec2(460.0, 380.0), vec![], &mut screen_cursor);
        assert!(polled.contains(&ViewportCommand::InnerSize(vec2(450.0, 370.0))), "{polled:?}");
        assert!(polled.contains(&ViewportCommand::OuterPosition(pos2(350.0, 230.0))), "{polled:?}");
        desktop.set(pos2(999.0, 999.0));
        let cancelled = resize_frame(&ctx, child, pos2(350.0, 230.0), vec2(450.0, 370.0), vec![Event::WindowFocused(false)], &mut screen_cursor);
        assert!(!cancelled.iter().any(|c| matches!(c, ViewportCommand::InnerSize(_) | ViewportCommand::OuterPosition(_))));
        assert!(!resize_active(&ctx, child));
    }

    fn floating_resize_frame(
        ctx: &egui::Context,
        viewport: ViewportId,
        events: Vec<Event>,
        contact: &mut Option<crate::ContactScreenPosFn>,
        mouse: &mut Option<crate::ScreenCursorPosFn>,
    ) -> Vec<ViewportCommand> {
        let size = vec2(500.0, 400.0);
        let mut input = RawInput { viewport_id: viewport, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() };
        let info = input.viewports.entry(viewport).or_default();
        info.inner_rect = Some(Rect::from_min_size(pos2(300.0, 200.0), size));
        info.outer_rect = info.inner_rect;
        info.focused = Some(true);
        info.native_pixels_per_point = Some(1.0);
        let mut out = ctx.run_ui(input, |ui| resize_zones_for_document(ui, contact, mouse, 6.0, vec2(MIN_INNER_WIDTH, MIN_INNER_HEIGHT)));
        out.textures_delta.clear();
        out.viewport_output.remove(&viewport).map_or_else(Vec::new, |v| v.commands)
    }

    fn floating_edges() -> [(Pos2, ResizeDirection, Vec2, Pos2); 8] {
        use ResizeDirection::*;
        [
            (pos2(250.0, 10.0), North, vec2(500.0, 380.0), pos2(300.0, 220.0)),
            (pos2(250.0, 390.0), South, vec2(500.0, 420.0), pos2(300.0, 200.0)),
            (pos2(10.0, 200.0), West, vec2(470.0, 400.0), pos2(330.0, 200.0)),
            (pos2(490.0, 200.0), East, vec2(530.0, 400.0), pos2(300.0, 200.0)),
            (pos2(20.0, 20.0), NorthWest, vec2(470.0, 380.0), pos2(330.0, 220.0)),
            (pos2(480.0, 20.0), NorthEast, vec2(530.0, 380.0), pos2(300.0, 220.0)),
            (pos2(20.0, 380.0), SouthWest, vec2(470.0, 420.0), pos2(330.0, 200.0)),
            (pos2(480.0, 380.0), SouthEast, vec2(530.0, 420.0), pos2(300.0, 200.0)),
        ]
    }

    #[test]
    fn floating_frame_pen_resize_starts_on_visible_edges_and_all_eight_directions() {
        use std::{cell::Cell, rc::Rc};
        for (press, direction, expected_size, expected_origin) in floating_edges() {
            let ctx = egui::Context::default();
            let viewport = ViewportId::from_hash_of(("v51-pen-resize", format!("{direction:?}")));
            let anchor = pos2(300.0, 200.0) + press.to_vec2();
            let desktop = Rc::new(Cell::new(anchor));
            let sample = desktop.clone();
            let mut contact: Option<crate::ContactScreenPosFn> = Some(Box::new(move |_, at_start| Some(if at_start { anchor } else { sample.get() })));
            let mut mouse = None;
            floating_resize_frame(&ctx, viewport, vec![touch(1, egui::TouchPhase::Start, press), button(press, true)], &mut contact, &mut mouse);
            assert!(resize_active(&ctx, viewport), "first press missed {direction:?}");
            desktop.set(anchor + vec2(30.0, 20.0));
            let moved = floating_resize_frame(
                &ctx,
                viewport,
                vec![touch(1, egui::TouchPhase::Move, pos2(800.0, -400.0)), Event::PointerMoved(pos2(5.0, 5.0)), button(press, false), Event::PointerGone],
                &mut contact,
                &mut mouse,
            );
            assert!(moved.contains(&ViewportCommand::InnerSize(expected_size)), "{direction:?}: {moved:?}");
            if resize_moves_origin(direction) {
                assert!(moved.contains(&ViewportCommand::OuterPosition(expected_origin)), "{direction:?}: {moved:?}");
            }
            assert!(!moved.iter().any(|c| matches!(c, ViewportCommand::BeginResize(_))));
            assert!(resize_active(&ctx, viewport), "synthetic mouse release must not release a pen");
            // Windows keeps pointer capture across neighbouring windows. A matching terminal
            // contact delivered to its owner must release; an unrelated contact must not.
            floating_resize_frame(&ctx, viewport, vec![touch(2, egui::TouchPhase::End, press)], &mut contact, &mut mouse);
            assert!(resize_active(&ctx, viewport));
            floating_resize_frame(&ctx, viewport, vec![touch(1, egui::TouchPhase::End, press)], &mut contact, &mut mouse);
            assert!(!resize_active(&ctx, viewport));
            desktop.set(anchor + vec2(100.0, 100.0));
            let after = floating_resize_frame(&ctx, viewport, vec![touch(1, egui::TouchPhase::Move, press)], &mut contact, &mut mouse);
            assert!(!after.iter().any(|c| matches!(c, ViewportCommand::InnerSize(_))), "{direction:?}: {after:?}");
        }
    }

    #[test]
    fn floating_frame_mouse_compatible_pen_resize_starts_without_a_hover_or_drag_threshold() {
        use std::{cell::Cell, rc::Rc};
        for (press, direction, expected_size, expected_origin) in floating_edges() {
            let ctx = egui::Context::default();
            let viewport = ViewportId::from_hash_of(("v51-mouse-resize", format!("{direction:?}")));
            let anchor = pos2(300.0, 200.0) + press.to_vec2();
            let desktop = Rc::new(Cell::new(anchor));
            let sample = desktop.clone();
            let mut mouse: Option<crate::ScreenCursorPosFn> = Some(Box::new(move || Some(sample.get())));
            let mut contact = None;
            floating_resize_frame(&ctx, viewport, vec![button(press, true)], &mut contact, &mut mouse);
            assert!(resize_active(&ctx, viewport), "no previous hover, {direction:?}");
            desktop.set(anchor + vec2(30.0, 20.0));
            let moved = floating_resize_frame(&ctx, viewport, vec![Event::PointerMoved(press + vec2(30.0, 20.0))], &mut contact, &mut mouse);
            assert!(moved.contains(&ViewportCommand::InnerSize(expected_size)), "{direction:?}: {moved:?}");
            if resize_moves_origin(direction) {
                assert!(moved.contains(&ViewportCommand::OuterPosition(expected_origin)), "{direction:?}: {moved:?}");
            }
            assert!(!moved.iter().any(|c| matches!(c, ViewportCommand::BeginResize(_))));
            // A final release at a new coordinate must not lose the last movement sample.
            desktop.set(anchor + vec2(60.0, 40.0));
            let end = floating_resize_frame(&ctx, viewport, vec![button(press + vec2(60.0, 40.0), false)], &mut contact, &mut mouse);
            assert!(end.iter().any(|c| matches!(c, ViewportCommand::InnerSize(_))), "{direction:?}: {end:?}");
            assert!(!resize_active(&ctx, viewport));
        }
    }

    #[test]
    fn floating_resize_targets_claim_only_new_edge_presses_not_an_interior_stroke_crossing_the_edge() {
        let ctx = egui::Context::default();
        let w = Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0));
        let run = |events, expected| {
            let mut out = ctx.run_ui(RawInput { screen_rect: Some(w), events, ..Default::default() }, |_| {
                assert_eq!(document_frame_resize_owns_input(&ctx, 6.0), expected);
            });
            out.textures_delta.clear();
        };
        run(vec![touch(1, egui::TouchPhase::Start, pos2(20.0, 380.0)), touch(1, egui::TouchPhase::End, pos2(20.0, 380.0))], true);
        run(vec![button(pos2(10.0, 200.0), true)], true);
        run(vec![button(pos2(10.0, 200.0), false)], false);
        run(vec![button(pos2(250.0, 200.0), true)], false);
        run(vec![Event::PointerMoved(pos2(10.0, 200.0))], false);
        // The two lower corner targets now reach 24pt from the outer viewport, without
        // expanding the painted 1pt outline or swallowing the middle of the image.
        let targets = zones(w, 6.0, true);
        assert!(targets.iter().rev().any(|(rect, dir, _)| *dir == ResizeDirection::SouthWest && rect.contains(pos2(23.0, 377.0))));
        assert!(targets.iter().rev().any(|(rect, dir, _)| *dir == ResizeDirection::SouthEast && rect.contains(pos2(477.0, 377.0))));
        assert!(!targets.iter().any(|(rect, _, _)| rect.contains(pos2(250.0, 200.0))));
        let panel_targets = zones(w, 6.0, false);
        assert!(!panel_targets.iter().any(|(rect, _, _)| rect.contains(pos2(20.0, 380.0))), "tool panel hit targets remain 11/18pt");
        assert!(panel_targets.iter().any(|(rect, dir, _)| *dir == ResizeDirection::SouthWest && rect.contains(pos2(17.0, 383.0))));
    }

    #[test]
    fn clamped_resize_keeps_opposite_edges_fixed_and_recovers_when_the_pen_reverses() {
        let ctx = egui::Context::default();
        let sample = ResizeSample {
            direction: ResizeDirection::NorthWest,
            start: pos2(300.0, 200.0),
            outer: pos2(300.0, 200.0),
            size: vec2(500.0, 400.0),
            last: pos2(300.0, 200.0),
            native_screen: false,
            min_size: vec2(MIN_INNER_WIDTH, MIN_INNER_HEIGHT),
        };
        let (size, origin, _) = resize_geometry(&ctx, sample, pos2(900.0, 800.0)).unwrap();
        assert_eq!(size, vec2(160.0, 96.0));
        assert_eq!(origin + size, pos2(800.0, 600.0));
        let (size, origin, _) = resize_geometry(&ctx, sample, pos2(400.0, 250.0)).unwrap();
        assert_eq!(size, vec2(400.0, 350.0));
        assert_eq!(origin + size, pos2(800.0, 600.0));
    }

    #[test]
    fn fallback_pen_resize_uses_its_final_end_position_before_releasing() {
        let ctx = egui::Context::default();
        let child = ViewportId::from_hash_of("v51-pen-final-position");
        let mut contact = None;
        let mut mouse = None;
        let press = pos2(480.0, 380.0);
        floating_resize_frame(&ctx, child, vec![touch(1, egui::TouchPhase::Start, press)], &mut contact, &mut mouse);
        let end = floating_resize_frame(&ctx, child, vec![touch(1, egui::TouchPhase::End, press + vec2(60.0, 40.0))], &mut contact, &mut mouse);
        assert!(end.contains(&ViewportCommand::InnerSize(vec2(560.0, 440.0))), "{end:?}");
        assert!(!resize_active(&ctx, child));
    }

    #[test]
    fn a_complete_buffered_pen_drag_applies_displacement_without_latching_or_starting_os_resize() {
        for native in [false, true] {
            let ctx = egui::Context::default();
            let child = ViewportId::from_hash_of(("v51-buffered-pen", native));
            let press = pos2(480.0, 380.0);
            let end = press + vec2(60.0, 40.0);
            let mut contact: Option<crate::ContactScreenPosFn> = native.then(|| {
                Box::new(move |_, at_start| Some(pos2(300.0, 200.0) + if at_start { press.to_vec2() } else { end.to_vec2() })) as crate::ContactScreenPosFn
            });
            let mut mouse = None;
            let commands = floating_resize_frame(
                &ctx,
                child,
                vec![
                    touch(1, egui::TouchPhase::Start, press),
                    touch(1, egui::TouchPhase::Move, press + vec2(30.0, 20.0)),
                    touch(1, egui::TouchPhase::End, end),
                ],
                &mut contact,
                &mut mouse,
            );
            assert!(commands.contains(&ViewportCommand::InnerSize(vec2(560.0, 440.0))), "native={native}: {commands:?}");
            assert!(!resize_active(&ctx, child));
            assert!(!commands.iter().any(|c| matches!(c, ViewportCommand::BeginResize(_))));
        }
    }
}
