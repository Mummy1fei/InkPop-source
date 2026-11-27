//! Detached documents share the editor, history and view state with their docked tab.
use egui::{Id, Pos2, Rect, Sense, Vec2, ViewportCommand, ViewportId, pos2, vec2};

use crate::{
    PhotocraftApp, canvas,
    state::{DetachedTab, DocWindow},
};
use photocraft_doc::DocId;
use photocraft_engine::jobs::JobId;

// Use the same gutter for painting and input. A zero-inset resize target sits entirely in the
// transparent shadow outside the visible frame, making pen presses on that frame miss it.
const FRAME_GUTTER: f32 = 6.0;
const HEADER_HEIGHT: f32 = 28.0;

#[path = "document_window_ui/focus_v75.rs"]
mod focus_v75;

/// Run before ROOT panels so they read the visible document's layers in this frame.
pub(crate) fn reconcile_visible_focus(app: &mut PhotocraftApp, ctx: &egui::Context) {
    focus_v75::reconcile(app, ctx);
}

fn target_id() -> Id {
    Id::new("document-window-dock-target")
}
fn hint_id() -> Id {
    Id::new("document-window-dock-hint")
}

fn group_hint_id() -> Id {
    Id::new("document-window-group-hint")
}

fn header_rect_id(window: u64) -> Id {
    Id::new(("document-window-header-rect", window))
}

fn initial_position_id(document: usize) -> Id {
    Id::new(("document-window-initial-position", document))
}

fn manual_minimize_id(window: u64) -> Id {
    Id::new(("document-window-manual-minimize", window))
}

fn minimize_pending_id(window: u64) -> Id {
    Id::new(("document-window-minimize-pending", window))
}

fn native_restore_request_id(viewport: ViewportId) -> Id {
    Id::new(("photocraft-native-restore-request", viewport))
}

fn native_restore_supported(ctx: &egui::Context, viewport: ViewportId) -> bool {
    ctx.data(|d| d.get_temp::<bool>(Id::new(("photocraft-native-restore-supported", viewport)))) == Some(true)
}

fn manually_minimized(ctx: &egui::Context, window: u64) -> bool {
    ctx.data(|d| d.get_temp::<bool>(manual_minimize_id(window))).unwrap_or(false)
}

/// The root's early-return path must preserve intent before eframe drops child viewports.
pub(crate) fn root_hidden(app: &PhotocraftApp, ctx: &egui::Context) {
    for window in app.ui.windows.iter().filter(|w| w.open) {
        ctx.data_mut(|d| {
            d.insert_temp(root_hidden_id(window.id), true);
            if d.get_temp::<bool>(manual_minimize_id(window.id)) == Some(true) {
                d.insert_temp(minimize_pending_id(window.id), true);
            }
        });
    }
}

fn request_manual_minimize(ctx: &egui::Context, viewport: ViewportId, window: u64) {
    ctx.data_mut(|d| {
        d.insert_temp(manual_minimize_id(window), true);
        d.insert_temp(minimize_pending_id(window), true);
        d.remove::<u8>(native_restore_request_id(viewport));
    });
    ctx.send_viewport_cmd_to(viewport, ViewportCommand::Minimized(true));
    ctx.request_repaint_of(viewport);
}

fn restore_pending_id(window: u64) -> Id {
    Id::new(("document-window-restore-pending", window))
}

fn restore_cooldown_id(window: u64) -> Id {
    Id::new(("document-window-restore-cooldown", window))
}

fn geometry_id(window: u64) -> Id {
    Id::new(("document-window-last-geometry", window))
}

fn geometry_restore_pending_id(window: u64) -> Id {
    Id::new(("document-window-geometry-restore-pending", window))
}

fn geometry_restore_cooldown_id(window: u64) -> Id {
    Id::new(("document-window-geometry-restore-cooldown", window))
}

fn root_hidden_id(window: u64) -> Id {
    Id::new(("document-window-root-hidden", window))
}

fn pending_close_id() -> Id {
    Id::new("document-window-pending-document-closes")
}

fn queue_document_close(ctx: &egui::Context, window: u64, document: u64) {
    ctx.data_mut(|d| {
        let mut pending = d.get_temp::<Vec<(u64, u64)>>(pending_close_id()).unwrap_or_default();
        if !pending.iter().any(|(w, doc)| *w == window && *doc == document) {
            pending.push((window, document));
        }
        d.insert_temp(pending_close_id(), pending);
    });
}

fn flush_document_closes(app: &PhotocraftApp, ctx: &egui::Context) {
    let pending = ctx.data(|d| d.get_temp::<Vec<(u64, u64)>>(pending_close_id())).unwrap_or_default();
    if pending.is_empty() {
        return;
    }
    let mut keep = Vec::with_capacity(pending.len());
    for (window, document) in pending {
        if app.session.documents().iter().any(|state| state.doc.id.0 == document) {
            // A clean close removes the document immediately.  If it is still present without
            // a discard prompt, the user cancelled the pending close; drop the stale request.
            if app.discard.is_some() {
                keep.push((window, document));
            }
        } else {
            if !app.ui.windows.iter().any(|candidate| candidate.id == window && candidate.open) {
                ctx.send_viewport_cmd_to(ViewportId::from_hash_of(("docwin", window)), ViewportCommand::Close);
            }
        }
    }
    ctx.data_mut(|d| {
        if keep.is_empty() {
            d.remove::<Vec<(u64, u64)>>(pending_close_id());
        } else {
            d.insert_temp(pending_close_id(), keep);
        }
    });
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct DetachedGeometry {
    /// Native physical pixels. `ViewportInfo` stores monitor coordinates in points, but the
    /// point scale can change when Windows recreates a child on another monitor during a root
    /// minimize. Keeping pixels here lets restore convert once using the child's current DPI.
    position_px: Pos2,
    size_px: Vec2,
    point_scale: f32,
}

fn reported_viewport_pixels_per_point(ctx: &egui::Context, viewport: ViewportId) -> Option<f32> {
    let zoom = ctx.zoom_factor().max(f32::EPSILON);
    ctx.input(|i| {
        i.raw
            .viewports
            .get(&viewport)
            .and_then(|v| v.native_pixels_per_point)
            .or_else(|| i.viewport().native_pixels_per_point)
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .map(|scale| scale * zoom)
    })
}

fn viewport_pixels_per_point(ctx: &egui::Context, viewport: ViewportId) -> f32 {
    reported_viewport_pixels_per_point(ctx, viewport).unwrap_or_else(|| ctx.pixels_per_point().max(f32::EPSILON))
}

fn detached_geometry_matches_saved(ctx: &egui::Context, viewport: ViewportId, saved: DetachedGeometry) -> bool {
    let scale = reported_viewport_pixels_per_point(ctx, viewport).unwrap_or(saved.point_scale);
    ctx.input(|i| {
        let Some(info) = i.raw.viewports.get(&viewport) else { return false };
        let (Some(outer), Some(inner)) = (info.outer_rect, info.inner_rect) else { return false };
        let position_px = outer.min * scale;
        let size_px = inner.size() * scale;
        (position_px - saved.position_px).length_sq() <= 4.0 && (size_px - saved.size_px).length_sq() <= 4.0
    })
}

fn remember_detached_geometry(ctx: &egui::Context, viewport: ViewportId, window: u64) {
    if detached_drag_active(ctx, window)
        || manually_minimized(ctx, window)
        || ctx.data(|d| d.get_temp::<u8>(native_restore_request_id(viewport))) == Some(3)
        || ctx.input(|i| i.raw.viewports.get(&viewport).is_some_and(|v| v.minimized == Some(true)))
    {
        return;
    }
    // After a root minimize, Windows can briefly report a valid-looking minimum-size child at
    // (0, 0). Never replace the user's saved placement with that fallback while a geometry
    // restore is pending or while the root is transitioning back to visible.
    let restoring = ctx.data(|d| d.get_temp::<bool>(geometry_restore_pending_id(window))).unwrap_or(false);
    let root_hidden = ctx.data(|d| d.get_temp::<bool>(root_hidden_id(window))).unwrap_or(false);
    if restoring || root_hidden {
        let saved = ctx.data(|d| d.get_temp::<DetachedGeometry>(geometry_id(window)));
        let matches_saved = saved.is_some_and(|saved| detached_geometry_matches_saved(ctx, viewport, saved));
        if matches_saved {
            ctx.data_mut(|d| {
                d.remove::<bool>(geometry_restore_pending_id(window));
                d.remove::<u8>(geometry_restore_cooldown_id(window));
                d.remove::<bool>(root_hidden_id(window));
            });
        }
        return;
    }
    let scale = viewport_pixels_per_point(ctx, viewport);
    let geometry = ctx.input(|i| {
        let info = i.raw.viewports.get(&viewport)?;
        let outer = info.outer_rect?;
        let inner = info.inner_rect?;
        let size = inner.size();
        (outer.min.x.is_finite() && outer.min.y.is_finite() && size.x.is_finite() && size.y.is_finite() && size.x >= 16.0 && size.y >= 16.0)
            .then_some(DetachedGeometry { position_px: outer.min * scale, size_px: size * scale, point_scale: scale })
    });
    if let Some(geometry) = geometry {
        ctx.data_mut(|d| {
            d.insert_temp(geometry_id(window), geometry);
            d.remove::<bool>(geometry_restore_pending_id(window));
        });
    }
}

fn detached_geometry_needs_restore(ctx: &egui::Context, viewport: ViewportId) -> bool {
    ctx.input(|i| {
        let Some(info) = i.raw.viewports.get(&viewport) else { return true };
        let Some(rect) = info.inner_rect else { return true };
        let size = rect.size();
        !size.x.is_finite() || !size.y.is_finite() || size.x < 16.0 || size.y < 16.0
    })
}

fn restore_detached_geometry(ctx: &egui::Context, viewport: ViewportId, window: u64, force: bool) {
    if detached_drag_active(ctx, window) || manually_minimized(ctx, window) || ctx.data(|d| d.get_temp::<u8>(native_restore_request_id(viewport))) == Some(3) {
        return;
    }
    let Some(geometry) = ctx.data(|d| d.get_temp::<DetachedGeometry>(geometry_id(window))) else { return };
    let pending = ctx.data(|d| d.get_temp::<bool>(geometry_restore_pending_id(window))).unwrap_or(false);
    if pending {
        // A Windows restore can be deferred until the child receives its first visible frame.
        // Retry after a few frames if the native command did not apply; otherwise one lost
        // command leaves the canvas permanently at the fallback origin.
        let retry_ready = ctx.data_mut(|d| {
            let Some(remaining) = d.get_temp::<u8>(geometry_restore_cooldown_id(window)) else { return true };
            if remaining == 0 {
                d.remove::<u8>(geometry_restore_cooldown_id(window));
                true
            } else {
                d.insert_temp(geometry_restore_cooldown_id(window), remaining - 1);
                false
            }
        });
        if !retry_ready {
            return;
        }
    }
    let geometry_matches = detached_geometry_matches_saved(ctx, viewport, geometry);
    if !force && geometry_matches && !detached_geometry_needs_restore(ctx, viewport) {
        return;
    }
    // If the child is between native frames, egui may not report its monitor DPI. Keep using
    // the scale captured with the saved geometry until the child reports a new one.
    let scale = reported_viewport_pixels_per_point(ctx, viewport).unwrap_or(geometry.point_scale);
    ctx.send_viewport_cmd_to(viewport, ViewportCommand::InnerSize(geometry.size_px / scale));
    ctx.send_viewport_cmd_to(viewport, ViewportCommand::OuterPosition(geometry.position_px / scale));
    ctx.data_mut(|d| d.insert_temp(geometry_restore_pending_id(window), true));
    ctx.data_mut(|d| d.insert_temp(geometry_restore_cooldown_id(window), 3_u8));
    ctx.request_repaint_of(viewport);
}

/// Queue one native restore and permit a short retry window. Windows may report a transparent
/// borderless child as `minimized = None` while focus moves to a root menu or brush picker. A
/// single command can be lost in that handoff, while issuing it every frame causes visible flash.
fn request_detached_restore(ctx: &egui::Context, viewport: ViewportId, window: u64) {
    ctx.send_viewport_cmd_to(viewport, ViewportCommand::Visible(true));
    ctx.send_viewport_cmd_to(viewport, ViewportCommand::Minimized(false));
    ctx.data_mut(|d| {
        d.insert_temp(restore_pending_id(window), true);
        d.insert_temp(restore_cooldown_id(window), 2_u8);
    });
    ctx.request_repaint_of(viewport);
}

/// Explicitly reopen one minimized document from Window > Arrange > Float in Window.
pub(crate) fn restore_manual_from_menu(ctx: &egui::Context, window: u64) {
    if !manually_minimized(ctx, window) {
        return;
    }
    let viewport = ViewportId::from_hash_of(("docwin", window));
    ctx.data_mut(|d| {
        d.remove::<bool>(manual_minimize_id(window));
        d.remove::<bool>(minimize_pending_id(window));
        d.remove::<u8>(native_restore_request_id(viewport));
        d.remove::<bool>(geometry_restore_pending_id(window));
        d.remove::<u8>(geometry_restore_cooldown_id(window));
    });
    // End the data lock before helpers read geometry or queue native commands.
    request_detached_restore(ctx, viewport, window);
    restore_detached_geometry(ctx, viewport, window, true);
    ctx.send_viewport_cmd_to(viewport, ViewportCommand::Focus);
}

fn restore_cooldown_elapsed(ctx: &egui::Context, window: u64) -> bool {
    let Some(remaining) = ctx.data(|d| d.get_temp::<u8>(restore_cooldown_id(window))) else {
        return true;
    };
    if remaining == 0 {
        ctx.data_mut(|d| d.remove::<u8>(restore_cooldown_id(window)));
        true
    } else {
        ctx.data_mut(|d| d.insert_temp(restore_cooldown_id(window), remaining - 1));
        false
    }
}

fn drag_last_position_id(window: u64) -> Id {
    Id::new(("document-window-drag-last-position", window))
}

fn detached_window_level_id(window: u64) -> Id {
    Id::new(("document-window-window-level-on-top", window))
}

fn title_drag_id(window: u64) -> Id {
    Id::new(("document-window-drag", window))
}

fn detached_drag_active(ctx: &egui::Context, window: u64) -> bool {
    ctx.data(|d| d.get_temp::<TitleDrag>(title_drag_id(window))).is_some() || crate::titlebar::resize_active(ctx, ViewportId::from_hash_of(("docwin", window)))
}

fn discard_detached_recovery(ctx: &egui::Context, window: u64) {
    // A new user placement supersedes every pending placement from an earlier restore.
    ctx.data_mut(|d| {
        d.remove::<bool>(root_hidden_id(window));
        d.remove::<bool>(geometry_restore_pending_id(window));
        d.remove::<u8>(geometry_restore_cooldown_id(window));
        d.remove::<bool>(restore_pending_id(window));
        d.remove::<u8>(restore_cooldown_id(window));
    });
}

/// All detached windows use ordinary z-order. Windows ownership is established by the
/// native backend; focus must never promote a document to the system topmost band.
fn sync_detached_window_level(ctx: &egui::Context, viewport: ViewportId, window: u64) {
    ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<std::collections::HashMap<ViewportId, u8>>(Id::new("photocraft-native-window-roles")).insert(viewport, 0);
    });
    if detached_drag_active(ctx, window) || ctx.data(|d| d.get_temp::<bool>(detached_window_level_id(window))) == Some(true) {
        return;
    }
    ctx.send_viewport_cmd_to(viewport, ViewportCommand::WindowLevel(egui::WindowLevel::Normal));
    ctx.data_mut(|d| d.insert_temp(detached_window_level_id(window), true));
}

/// Restore only an observed minimize, never a root click or an occlusion. Owned windows
/// already follow their root's z-order; speculative Focus/ShowWindow calls interrupt pen input.
fn keep_detached_viewport_visible(ctx: &egui::Context, viewport: ViewportId, window: u64) {
    if detached_drag_active(ctx, window) {
        return;
    }
    let (minimized, root_visible) = ctx.input(|i| {
        let root = i.raw.viewports.get(&ViewportId::ROOT);
        (i.raw.viewports.get(&viewport).and_then(|v| v.minimized), root.is_none_or(|v| v.minimized != Some(true) && v.visible() != Some(false)))
    });
    let manual = manually_minimized(ctx, window);
    let native_restore = ctx.data_mut(|d| d.remove_temp::<u8>(native_restore_request_id(viewport)));
    if native_restore == Some(3) {
        // A taskbar/system-menu minimize is just as intentional as our custom caption.
        // Windows already executes the command; retain intent without another ShowWindow.
        ctx.data_mut(|d| {
            d.insert_temp(manual_minimize_id(window), true);
            d.insert_temp(minimize_pending_id(window), true);
        });
        ctx.request_repaint_of(viewport);
        return;
    }
    if manual {
        if let Some(kind @ (1 | 2)) = native_restore {
            // The taskbar/system menu explicitly targeted this child. Root SC_RESTORE and
            // owner visibility changes never produce this marker. Do not infer intent from
            // a late minimized=false after the root has already become visible.
            ctx.data_mut(|d| {
                d.remove::<bool>(manual_minimize_id(window));
                d.remove::<bool>(minimize_pending_id(window));
            });
            if kind == 2 {
                // An explicit maximize must keep the OS maximize placement.
                ctx.data_mut(|d| {
                    d.remove::<bool>(root_hidden_id(window));
                    d.remove::<bool>(geometry_restore_pending_id(window));
                    d.remove::<u8>(geometry_restore_cooldown_id(window));
                });
            } else {
                restore_detached_geometry(ctx, viewport, window, true);
            }
            ctx.request_repaint_of(viewport);
            return;
        }
        if !root_visible {
            ctx.data_mut(|d| d.insert_temp(minimize_pending_id(window), true));
            return;
        }
        if minimized == Some(true) {
            // Only a native acknowledgement completes the request. A recreated HWND initially
            // reporting false must not be mistaken for a taskbar restore by the user.
            ctx.data_mut(|d| d.remove::<bool>(minimize_pending_id(window)));
            return;
        }
        if native_restore_supported(ctx, viewport) || minimized.is_none() || ctx.data(|d| d.get_temp::<bool>(minimize_pending_id(window))) == Some(true) {
            request_manual_minimize(ctx, viewport, window);
            return;
        }
    }
    if minimized == Some(false) {
        ctx.data_mut(|d| {
            d.remove::<bool>(restore_pending_id(window));
            d.remove::<u8>(restore_cooldown_id(window));
            d.remove::<bool>(manual_minimize_id(window));
        });
    }
    if root_visible && !manual && minimized == Some(true) && restore_cooldown_elapsed(ctx, window) {
        request_detached_restore(ctx, viewport, window);
    }
}

/// Build the borderless native viewport used for detached documents.
///
/// Native viewport transparency is required for the rounded frame below to be visible at the
/// actual window boundary. Without it the renderer's opaque clear colour fills the four corners,
/// making the window look square even though the egui frame has a corner radius.
fn detached_viewport_builder(title: &str) -> egui::ViewportBuilder {
    egui::ViewportBuilder::default()
        .with_title(title)
        .with_inner_size([800.0, 600.0])
        .with_min_inner_size([crate::titlebar::MIN_INNER_WIDTH, crate::titlebar::MIN_INNER_HEIGHT])
        .with_decorations(false)
        .with_transparent(true)
        // A document needs a taskbar entry so an explicit minimize has a normal restore target.
        // Win32 ownership still provides application-local stacking and root minimize behavior.
        .with_taskbar(true)
        // The native backend establishes ownership; this never enters the system topmost band.
        .with_window_level(egui::WindowLevel::Normal)
        // The theme frame owns the silhouette. Windows needs the corresponding native override
        // in eframe's owned_window helper: egui-winit 0.36.2 ignores this flag on Windows.
        .with_has_shadow(false)
}

/// The detached viewport has a transparent, rectangular native surface. Keep the frame itself
/// aligned to the viewport bounds: adding an egui `outer_margin` here enlarges the native child
/// around the card and makes the apparent floating-canvas surround grow on every iteration.
///
/// Keep the shadow inside the transparent native surface. The floating panels use a broad window
/// shadow, but a detached canvas has no room outside its viewport; reserving a small pasteboard
/// band and using a narrow blur gives the same separation without creating a clipped square halo.
fn detached_frame(_style: &egui::Style, tokens: crate::theme::Tokens) -> egui::Frame {
    // Read the current tokens so all four corners follow the theme even during a theme switch.
    let radius = tokens.radius_lg.clamp(0.0, u8::MAX as f32).round() as u8;
    let corner_radius = egui::CornerRadius::same(radius);
    // Keep the visible window chrome compact. Four points are enough for the themed stroke and
    // title row while avoiding the old radius-sized pasteboard band (16px in Studio/StudioLight).
    let corner_inset = 4;
    // Detached documents are a canvas surface, unlike floating tool panels.  Panels use the
    // raised `card` surface so their padding reads as a card around controls; using that same
    // fill here creates the thick light/dark "package" around an image (especially in
    // StudioLight, where `card` is 252 and `canvas` is 226).  Keep the safe inner inset that
    // protects the transparent viewport's rounded corners, but paint the inset with the canvas
    // surface so it visually joins the pasteboard.
    // Classic intentionally uses the same gray for the canvas and the detached frame.  Its
    // normal card border is also that gray, so reusing it makes the floating document disappear
    // into the pasteboard.  Use the Classic text-dim token as a one-pixel outline; it is the
    // existing dark bevel color for that theme and keeps the other themes token-driven.
    let border = if tokens.kind == crate::theme::ThemeKind::Classic { tokens.text_dim } else { tokens.card_border };
    let shadow = if tokens.shadow.a() == 0 { egui::Shadow::NONE } else { egui::Shadow { offset: [0, 3], blur: 10, spread: 0, color: tokens.shadow } };
    egui::Frame::NONE
        .fill(tokens.canvas)
        .stroke(egui::Stroke::new(1.0, border))
        .corner_radius(corner_radius)
        // The content is rectangular (the canvas texture and title row), so keep it clear of the
        // rounded silhouette. This is deliberately much narrower than a floating panel's shadow:
        // the viewport reserves only a few transparent points around the card for the blur.
        .shadow(shadow)
        .inner_margin(egui::Margin::same(corner_inset))
}

/// Cross-window hit testing uses physical screen coordinates, including per-window DPI.
fn screen_point(ctx: &egui::Context, local: Pos2) -> Option<Pos2> {
    let ppp = ctx.pixels_per_point();
    ctx.input(|i| i.viewport().inner_rect.map(|r| (r.min + local.to_vec2()) * ppp))
}

/// Remember where a tab drag was released so the first frame of its native viewport opens under
/// the pointer. `ViewportBuilder` takes logical screen points while egui's viewport rectangles
/// are reported in physical pixels, so convert once here and avoid a one-frame jump after spawn.
pub(crate) fn set_initial_position(ctx: &egui::Context, document: usize, local: Pos2) {
    let Some(screen) = screen_point(ctx, local) else { return };
    let ppp = ctx.pixels_per_point().max(f32::EPSILON);
    ctx.data_mut(|d| d.insert_temp(initial_position_id(document), screen / ppp));
}

pub(crate) fn dock_target(app: &PhotocraftApp, ui: &egui::Ui, workspace: Rect) {
    let top = app.tab_strip.as_ref().map_or(workspace.top(), |s| s.rect.top());
    let band = Rect::from_min_size(pos2(workspace.left(), top), vec2(workspace.width(), 36.0));
    let physical = screen_point(ui.ctx(), band.min).zip(screen_point(ui.ctx(), band.max)).map(|(a, b)| Rect::from_min_max(a, b));
    ui.ctx().data_mut(|d| d.insert_temp(target_id(), physical));
    if ui.ctx().data(|d| d.get_temp::<bool>(hint_id())).unwrap_or(false) {
        let t = crate::theme::Tokens::get(ui.ctx());
        ui.painter().rect_filled(band, 3.0, t.accent.gamma_multiply(0.35));
        ui.painter().rect_stroke(band, 3.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    }
}

fn can_dock(ctx: &egui::Context, point: Option<Pos2>, enabled: bool) -> bool {
    enabled && point.is_some_and(|p| ctx.data(|d| d.get_temp::<Option<Rect>>(target_id())).flatten().is_some_and(|r| r.contains(p)))
}

pub(crate) fn dock(app: &mut PhotocraftApp, id: u64, ctx: &egui::Context) {
    let Some(window) = app.ui.windows.iter().find(|w| w.id == id).cloned() else { return };
    app.ui.windows.retain(|w| w.id != id);
    if window.document < app.session.documents().len() {
        app.session.set_active(window.document);
        app.ui.view.arrange = "tabs".into();
    }
    ctx.data_mut(|d| d.insert_temp(hint_id(), false));
    ctx.send_viewport_cmd_to(ViewportId::from_hash_of(("docwin", id)), ViewportCommand::Close);
    ctx.request_repaint_of(ViewportId::ROOT);
}

pub(crate) fn select_document_tab(app: &mut PhotocraftApp, ctx: &egui::Context, id: u64, document: usize) -> bool {
    if input_owned(app) {
        return false;
    }
    if !app.ui.windows.iter().any(|w| w.open && w.id == id && w.contains_document(document)) || app.session.documents().get(document).is_none() {
        return false;
    }
    crate::type_tool::commit(app);
    if app.ui.text_edit.is_some() {
        return false;
    }
    let Some(window) = app.ui.windows.iter_mut().find(|w| w.id == id && w.open && w.contains_document(document)) else { return false };
    window.document = document;
    window.view = app.ui.views.get(document).cloned().unwrap_or_default();
    app.session.set_active(document);
    app.jobs.focus = None;
    focus_v75::remember(app, ctx, document);
    ctx.request_repaint_of(ViewportId::ROOT);
    ctx.request_repaint_of(ViewportId::from_hash_of(("docwin", id)));
    true
}

fn input_owned(app: &PhotocraftApp) -> bool {
    app.ui.transform.is_some()
        || app.drag.is_some()
        || !app.ui.dialogs.is_empty()
        || app.discard.is_some()
        || app.camera_raw.is_some()
        || app.jpeg_options.is_some()
        || app.distort.liquify.is_some()
}

pub(crate) fn merge(app: &mut PhotocraftApp, ctx: &egui::Context, source: u64, target: u64) -> bool {
    if source == target || input_owned(app) {
        return false;
    }
    let Some(from) = app.ui.windows.iter().find(|w| w.open && w.id == source).cloned() else { return false };
    if !app.ui.windows.iter().any(|w| w.open && w.id == target) {
        return false;
    }
    crate::type_tool::commit(app);
    if app.ui.text_edit.is_some() {
        return false;
    }
    let Some(into) = app.ui.windows.iter_mut().find(|w| w.open && w.id == target) else { return false };
    let tabs = into.group_tabs_mut();
    let transferred = if from.tabs.is_empty() { vec![DetachedTab::Document(from.document)] } else { from.tabs };
    for tab in transferred {
        if !tabs.contains(&tab) {
            tabs.push(tab);
        }
    }
    app.ui.windows.retain(|w| w.id != source);
    let viewport = ViewportId::from_hash_of(("docwin", target));
    if select_document_tab(app, ctx, target, from.document) {
        ctx.send_viewport_cmd_to(viewport, ViewportCommand::Focus);
    }
    ctx.send_viewport_cmd_to(ViewportId::from_hash_of(("docwin", source)), ViewportCommand::Close);
    ctx.request_repaint_of(ViewportId::ROOT);
    true
}

pub(crate) fn owns_open_tab(app: &PhotocraftApp, job: JobId) -> bool {
    app.ui.windows.iter().any(|window| window.open && window.tabs.contains(&DetachedTab::Opening(job)))
}

pub(crate) fn queue_open_tab(app: &mut PhotocraftApp, id: u64, job: JobId) {
    if let Some(window) = app.ui.windows.iter_mut().find(|w| w.open && w.id == id) {
        window.group_tabs_mut().push(DetachedTab::Opening(job));
        app.jobs.focus = crate::jobs_ui::last_root_open(app);
    }
}

pub(crate) fn append_document_tab(app: &mut PhotocraftApp, id: u64, document: usize) {
    if let Some(window) = app.ui.windows.iter_mut().find(|w| w.open && w.id == id && !w.contains_document(document)) {
        window.group_tabs_mut().push(DetachedTab::Document(document));
        window.document = document;
        window.view = app.ui.views.get(document).cloned().unwrap_or_default();
    }
}

pub(crate) fn finish_open_tab(app: &mut PhotocraftApp, job: JobId, opened: Option<DocId>) {
    let select = opened.and_then(|doc| app.session.documents().iter().position(|state| state.doc.id == doc));
    for window in &mut app.ui.windows {
        let Some(position) = window.tabs.iter().position(|tab| *tab == DetachedTab::Opening(job)) else { continue };
        if let Some(document) = select {
            window.tabs[position] = DetachedTab::Document(document);
            if app.ui.transform.is_none() && app.drag.is_none() {
                window.document = document;
                window.view = app.ui.views.get(document).cloned().unwrap_or_default();
            }
        } else {
            window.tabs.remove(position);
        }
        break;
    }
}

fn group_drop_target(app: &mut PhotocraftApp, ctx: &egui::Context, source: u64, point: Pos2) -> Option<u64> {
    let native_hit = app.services.screen_viewport.as_mut().map(|hit| hit(ctx, point));
    let source_viewport = ViewportId::from_hash_of(("docwin", source));
    app.ui.windows.iter().rev().find_map(|window| {
        if !window.open || window.id == source || manually_minimized(ctx, window.id) {
            return None;
        }
        let viewport = ViewportId::from_hash_of(("docwin", window.id));
        // The moving window itself can cover the target title. The hit-test then sees the
        // source, so examine the remaining live titles by their native viewport geometry.
        if native_hit.is_some_and(|hit| !matches!(hit, Some(id) if id == source_viewport || id == viewport)) {
            return None;
        }
        let (origin, size) = ctx.input(|input| {
            let info = input.raw.viewports.get(&viewport)?;
            if info.visible() == Some(false) || info.minimized == Some(true) || info.close_requested() {
                return None;
            }
            let rect = info.inner_rect?;
            Some((rect.min, rect.size()))
        })?;
        let ppp = viewport_pixels_per_point(ctx, viewport);
        let rect = ctx.data(|d| d.get_temp::<Rect>(header_rect_id(window.id)))?;
        let hit = Rect::from_min_max(rect.min, pos2((size.x - rect.left()).max(rect.left()), rect.bottom()));
        hit.contains((point / ppp - origin).to_pos2()).then_some(window.id)
    })
}

#[derive(Clone, Copy)]
struct TitleDrag {
    /// Physical offset from the window origin to the press position.
    offset: Vec2,
    /// Touch input is accompanied by an emulated pointer event on Windows.  Keep the source so a
    /// stale synthetic pointer sample cannot move the native window for one frame.
    touch: bool,
    /// Logical outer position at press time.  A detached viewport may briefly omit `inner_rect`
    /// while focus moves to a tool/menu, so a local delta fallback must not depend on
    /// `screen_point` being available on every frame.
    start_outer: Option<Pos2>,
    /// Physical screen coordinate of the press.  Keep this separate from the local coordinate:
    /// after the native window moves, egui reports the pen at a new local point even though the
    /// screen delta is the user's actual drag distance.
    start_screen: Option<Pos2>,
    start_local: Pos2,
    /// The contact that owns this gesture; unrelated fingers must not release the pen.
    contact: Option<(egui::TouchDeviceId, egui::TouchId)>,
    /// Once native screen sampling is selected, never mix it with buffered client samples.
    native_screen: bool,
}

fn latest_drag_position(ctx: &egui::Context, touch: bool) -> Option<Pos2> {
    ctx.input(|i| {
        // egui-winit sends Touch immediately before a synthetic PointerMoved.  During a touch
        // drag only consume Touch positions; accepting the emulated pointer reintroduces the
        // one-frame jump that is visible as a flash while selecting the floating canvas.
        let from_events = i.events.iter().rev().find_map(|event| match event {
            egui::Event::Touch { phase: egui::TouchPhase::Start | egui::TouchPhase::Move, pos, .. } if touch => Some(*pos),
            egui::Event::PointerMoved(p) if !touch => Some(*p),
            _ => None,
        });
        from_events.or_else(|| (!touch).then(|| i.pointer.interact_pos()).flatten())
    })
}

fn drag_position_changed(ctx: &egui::Context, touch: bool) -> bool {
    ctx.input(|i| {
        i.events.iter().any(|event| match event {
            egui::Event::Touch { phase: egui::TouchPhase::Start | egui::TouchPhase::Move, .. } => touch,
            egui::Event::PointerMoved(_) => !touch,
            _ => false,
        })
    })
}

fn title_drag_ended(ctx: &egui::Context, drag: TitleDrag) -> bool {
    if let Some((device, contact)) = drag.contact {
        return ctx.input(|i| {
            i.events.iter().any(|event| match event {
                egui::Event::Touch { device_id, id, phase: egui::TouchPhase::End | egui::TouchPhase::Cancel, .. } => *device_id == device && *id == contact,
                egui::Event::WindowFocused(false) => true,
                _ => false,
            })
        });
    }
    ctx.input(|i| i.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))) || !i.pointer.primary_down())
}

fn screen_drag_target(drag: TitleDrag, screen_px: Pos2, ppp: f32) -> Pos2 {
    (screen_px - drag.offset) / ppp.max(f32::EPSILON)
}

/// Bounded, in-memory input diagnostics. No image data, document names or file paths are stored.
fn record_title_drag(ctx: &egui::Context, window: u64, phase: &str, drag: TitleDrag, local: Option<Pos2>, screen: Option<Pos2>, target: Option<Pos2>) {
    let (outer, ppp, time, events) = ctx.input(|i| {
        (
            i.viewport().outer_rect.map(|r| r.min),
            i.pixels_per_point,
            i.time,
            i.events
                .iter()
                .map(|event| match event {
                    egui::Event::Touch { id, phase, .. } => format!("Touch({id:?},{phase:?})"),
                    egui::Event::PointerMoved(_) => "PointerMoved".into(),
                    egui::Event::PointerGone => "PointerGone".into(),
                    egui::Event::PointerButton { pressed, .. } => format!("PointerButton({pressed})"),
                    egui::Event::WindowFocused(focused) => format!("Focused({focused})"),
                    _ => "Other".into(),
                })
                .take(64)
                .collect::<Vec<_>>(),
        )
    });
    let point = |p: Option<Pos2>| p.map(|p| [p.x, p.y]);
    let entry = serde_json::json!({"window":window,"gesture":"move","phase":phase,"nativeScreen":drag.native_screen,"time":time,
        "coordinateSource":if drag.native_screen && drag.touch {"nativeContact"} else if drag.native_screen {"mouseCursor"} else {"localFallback"},
        "source":if drag.touch {"touchOrPen"} else {"pointer"}, "contact":drag.contact.map(|c| format!("{c:?}")),
        "local":point(local),"screenPx":point(screen),"outer":point(outer),"target":point(target),"ppp":ppp,"events":events});
    record_window_gesture(ctx, entry);
}

pub(crate) fn record_window_gesture(ctx: &egui::Context, entry: serde_json::Value) {
    ctx.data_mut(|d| {
        let id = Id::new("document-window-drag-trace");
        let entries = d.get_temp_mut_or_default::<std::collections::VecDeque<serde_json::Value>>(id);
        if entries.len() >= 512 {
            entries.pop_front();
        }
        entries.push_back(entry);
    });
}

pub(crate) fn drag_diagnostics(ctx: &egui::Context) -> std::collections::VecDeque<serde_json::Value> {
    ctx.data(|d| d.get_temp(Id::new("document-window-drag-trace"))).unwrap_or_default()
}

fn pointer_press_origin(ctx: &egui::Context) -> Option<Pos2> {
    ctx.input(|i| {
        i.events.iter().rev().find_map(|event| match event {
            egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: true, pos, .. } => Some(*pos),
            _ => None,
        })
    })
}

fn local_drag_target(ctx: &egui::Context, drag: TitleDrag, local: Pos2) -> Option<Pos2> {
    // The pointer position delivered by egui is local to the viewport.  Once the detached
    // window moves, that local coordinate moves with the window too (the pen stays over the same
    // screen pixel), so subtracting `start_local` from it makes the drag stop after its first
    // sample.  Convert the current sample back to screen coordinates before applying the press
    // offset. This is only the fallback on platforms without an independent desktop sample.
    let ppp = ctx.pixels_per_point().max(f32::EPSILON);
    if let Some(outer) = drag.start_outer {
        if let (Some(start_screen), Some(current_screen)) = (drag.start_screen, screen_point(ctx, local)) {
            return Some(outer + (current_screen - start_screen) / ppp);
        }
        return Some(outer + (local - drag.start_local));
    }
    screen_point(ctx, local).map(|p| (p - drag.offset) / ppp)
}

fn paint_header_background(ui: &egui::Ui, bar: Rect, t: crate::theme::Tokens) {
    // The frame adds a 4pt inner margin and 1pt stroke. Extend only through the margin,
    // leaving the original rounded outline and transparent native corners untouched.
    let band = Rect::from_min_max(bar.min - vec2(4.0, 4.0), pos2(bar.right() + 4.0, bar.bottom()));
    let radius = (t.radius_lg - 1.0).max(0.0).round() as u8;
    ui.painter().rect_filled(
        band,
        egui::CornerRadius { nw: radius, ne: radius, sw: 0, se: 0 },
        crate::theme::palette::override_color(ui.ctx(), photocraft_engine::prefs::palette::ColorSlot::FloatingTitle).unwrap_or(t.chrome),
    );
    ui.painter().hline(band.x_range(), band.bottom(), egui::Stroke::new(1.0, t.separator));
}

fn header(app: &mut PhotocraftApp, ui: &mut egui::Ui, window: &DocWindow, title: &str) -> bool {
    let ctx = ui.ctx().clone();
    let blocked = app.discard.is_some() || !ui.is_enabled();
    let resizing = crate::titlebar::document_frame_resize_owns_input(&ctx, FRAME_GUTTER);
    let t = crate::theme::Tokens::get(&ctx);
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_HEIGHT), Sense::hover());
    ctx.data_mut(|d| d.insert_temp(header_rect_id(window.id), bar));
    paint_header_background(ui, bar, t);
    if ctx.data(|d| d.get_temp::<Option<u64>>(group_hint_id())).flatten() == Some(window.id) {
        ui.painter().rect_filled(bar, t.radius_sm, t.accent.gamma_multiply(0.3));
        ui.painter().rect_stroke(bar, t.radius_sm, egui::Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
    }
    let mut dock_now = false;
    let caption_width = ((bar.width() - 78.0) / 3.0).clamp(20.0, 32.0);
    for (i, (icon, tooltip)) in [("x", tl!("Close")), ("maximize-2", tl!("Maximize Window")), ("minus", tl!("Minimize"))].into_iter().enumerate() {
        let rect = Rect::from_min_size(pos2(bar.right() - caption_width * (i + 1) as f32, bar.top()), vec2(caption_width, HEADER_HEIGHT));
        let response = ui.interact(rect, Id::new(("document-window-caption", window.id, i)), if resizing || blocked { Sense::hover() } else { Sense::click() });
        let tint = if response.hovered() { t.text } else { t.text_dim };
        crate::icons::button_chrome(ui, rect, false, response.hovered());
        crate::icons::paint(ui, rect, icon, 14.0, tint);
        let response = response.on_hover_text(tooltip);
        if !resizing && !blocked && response.clicked() {
            match i {
                0 => {
                    // Photoshop's document-window X closes the document. Returning it to the
                    // tab group is a docking gesture, so do not turn a close click into a full
                    // sized tabbed canvas. Keep the child alive while an unsaved-changes prompt
                    // is open; the root frame closes it after the document is actually removed.
                    if let Some(document) = app.session.documents().get(window.document).map(|state| state.doc.id.0) {
                        queue_document_close(&ctx, window.id, document);
                        let _ = crate::menus::invoke(app, &ctx, "file.close", serde_json::json!({ "document": window.document }));
                    }
                }
                1 => ctx.send_viewport_cmd(ViewportCommand::Maximized(!ctx.input(|i| i.viewport().maximized.unwrap_or(false)))),
                _ => {
                    // Remember an explicit caption click so the focus handoff caused by a
                    // top-level menu cannot be mistaken for a request to minimize this child.
                    request_manual_minimize(&ctx, ctx.viewport_id(), window.id);
                    focus_v75::reconcile(app, &ctx);
                }
            }
        }
    }
    let mut title_rect = Rect::from_min_max(bar.min + vec2(8.0, 0.0), bar.max - vec2(caption_width * 3.0 + 4.0, 0.0));
    if window.tabs.len() >= 2 {
        let drag_space = (title_rect.width() - 76.0).clamp(24.0, 56.0);
        let tab_area = Rect::from_min_max(title_rect.min, pos2((title_rect.right() - drag_space).max(title_rect.left()), title_rect.bottom()));
        title_rect.min.x = floating_title_tabs(app, ui, window, tab_area, resizing || blocked);
    } else {
        ui.painter().with_clip_rect(title_rect).text(title_rect.left_center(), egui::Align2::LEFT_CENTER, title, egui::FontId::proportional(12.0), t.text);
    }
    if blocked {
        return false;
    }
    let response = ui.interact(title_rect, Id::new(("document-window-title", window.id)), Sense::click_and_drag());
    if !resizing && response.double_clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Maximized(!ctx.input(|i| i.viewport().maximized.unwrap_or(false))));
    }
    let drag_id = title_drag_id(window.id);
    let touch_start = ctx.input(|i| {
        i.events.iter().rev().find_map(|event| match event {
            egui::Event::Touch { device_id, id, phase: egui::TouchPhase::Start, pos, .. } if title_rect.contains(*pos) => Some((*pos, (*device_id, *id))),
            _ => None,
        })
    });
    let pointer_start = pointer_press_origin(&ctx).filter(|p| title_rect.contains(*p));
    // Start once on contact, never again when egui's drag threshold is crossed. Reinitializing
    // on response.drag_started() used the old press position with the newly moved window origin.
    if !detached_drag_active(&ctx, window.id)
        && let Some(origin) =
            touch_start.map(|(pos, _)| pos).or(pointer_start).filter(|pos| !crate::titlebar::document_frame_resize_contains(&ctx, *pos, FRAME_GUTTER))
    {
        let start_outer = ctx.input(|i| i.viewport().outer_rect.map(|r| r.min));
        let native = if let Some((_, (_, contact))) = touch_start {
            app.services.contact_screen_pos.as_mut().and_then(|read| read(contact.0, true))
        } else {
            app.services.screen_cursor_pos.as_mut().and_then(|read| read())
        };
        let start_screen = native.or_else(|| screen_point(&ctx, origin));
        let offset = match native.zip(start_outer) {
            Some((screen, outer)) => screen - outer * ctx.pixels_per_point(),
            None => origin.to_vec2() * ctx.pixels_per_point(),
        };
        let state = TitleDrag {
            offset,
            touch: touch_start.is_some(),
            start_outer,
            start_screen,
            start_local: origin,
            contact: touch_start.map(|(_, contact)| contact),
            native_screen: native.is_some(),
        };
        ctx.data_mut(|d| {
            d.insert_temp(drag_id, state);
            d.insert_temp(drag_last_position_id(window.id), start_outer);
        });
        discard_detached_recovery(&ctx, window.id);
        record_title_drag(&ctx, window.id, "start", state, Some(origin), native, None);
    }
    if let Some(drag) = ctx.data(|d| d.get_temp::<TitleDrag>(drag_id)) {
        let local = latest_drag_position(&ctx, drag.touch);
        // A contact uses coordinates captured with WM_POINTER, before ScreenToClient and before
        // event buffering. The mouse cursor may remain stationary for an entire Ink contact.
        let screen = if drag.native_screen {
            if let Some((_, contact)) = drag.contact {
                app.services.contact_screen_pos.as_mut().and_then(|read| read(contact.0, false))
            } else {
                app.services.screen_cursor_pos.as_mut().and_then(|read| read())
            }
        } else {
            local.and_then(|p| screen_point(&ctx, p))
        };
        let enabled = app.session.prefs().workspace.enable_floating_document_window_docking;
        let group = enabled
            .then_some(screen)
            .flatten()
            .filter(|point| drag.start_screen.is_some_and(|start| start.distance(*point) >= 4.0 * ctx.pixels_per_point()))
            .and_then(|point| group_drop_target(app, &ctx, window.id, point));
        let over = group.is_none() && can_dock(&ctx, screen, enabled);
        let previous = ctx.data(|d| d.get_temp::<Option<u64>>(group_hint_id())).flatten();
        ctx.data_mut(|d| {
            d.insert_temp(hint_id(), over);
            d.insert_temp(group_hint_id(), group);
        });
        if previous != group {
            ctx.request_repaint_of(ViewportId::ROOT);
        }
        if group.is_none() && previous.is_some() {
            // The release moved beyond any candidate. Do not leave an old highlighted target.
            ctx.request_repaint();
        }
        let ended = title_drag_ended(&ctx, drag);
        let target = if drag.native_screen {
            screen.map(|screen| screen_drag_target(drag, screen, ctx.pixels_per_point()))
        } else if drag_position_changed(&ctx, drag.touch) {
            local.and_then(|local| local_drag_target(&ctx, drag, local))
        } else {
            None
        };
        // No unbounded native ACK gate: Windows may round/constrain the requested position, or
        // omit outer_rect. A stationary desktop pointer produces a stationary target regardless.
        let cancelled = ctx.input(|i| {
            i.events.iter().any(|event| match event {
                egui::Event::WindowFocused(false) => true,
                egui::Event::Touch { device_id, id, phase: egui::TouchPhase::Cancel, .. } => drag.contact == Some((*device_id, *id)),
                _ => false,
            })
        });
        if let Some(target) = target.filter(|p| p.is_finite() && !cancelled) {
            let last = ctx.data(|d| d.get_temp::<Option<Pos2>>(drag_last_position_id(window.id))).flatten();
            if last.is_none_or(|p| p.distance_sq(target) * ctx.pixels_per_point().powi(2) >= 1.0) {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(target));
                ctx.data_mut(|d| d.insert_temp(drag_last_position_id(window.id), Some(target)));
                ctx.request_repaint_of(ViewportId::ROOT);
            }
        }
        record_title_drag(
            &ctx,
            window.id,
            if cancelled {
                "cancel"
            } else if ended {
                "end"
            } else {
                "move"
            },
            drag,
            local,
            screen,
            target.filter(|_| !cancelled),
        );
        if ended {
            if !cancelled && let Some(target) = group {
                merge(app, &ctx, window.id, target);
            }
            dock_now |= over && !cancelled;
            ctx.data_mut(|d| {
                d.remove::<TitleDrag>(drag_id);
                d.remove::<Option<Pos2>>(drag_last_position_id(window.id));
                d.insert_temp(hint_id(), false);
                d.remove::<Option<u64>>(group_hint_id());
            });
            ctx.request_repaint_of(ViewportId::ROOT);
        } else {
            // Native movement can consume a frame without delivering a new client event. Poll
            // only while the gesture is alive; movement does not depend on another Touch Move.
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }
    dock_now
}

fn body_size_id(viewport: ViewportId, document: photocraft_doc::DocId) -> Id {
    Id::new(("floating-document-canvas-size-v79", viewport, document.0))
}

/// Ignore native minimize/recovery placeholders: they are not a user resize.
fn body_geometry_ready(app: &PhotocraftApp, ctx: &egui::Context, document: usize, rect: Rect) -> bool {
    if !rect.min.is_finite() || !rect.max.is_finite() || rect.width() < 16.0 || rect.height() < 16.0 {
        return false;
    }
    if ctx.input(|input| {
        input.viewport().inner_rect.is_some_and(|inner| !inner.min.is_finite() || !inner.max.is_finite() || inner.width() < 16.0 || inner.height() < 16.0)
    }) {
        return false;
    }
    let window =
        app.ui.windows.iter().find(|window| window.open && window.document == document && ViewportId::from_hash_of(("docwin", window.id)) == ctx.viewport_id());
    window.is_none_or(|window| {
        ctx.data(|data| {
            data.get_temp::<bool>(root_hidden_id(window.id)) != Some(true) && data.get_temp::<bool>(geometry_restore_pending_id(window.id)) != Some(true)
        })
    })
}

fn recenter_resized_body(app: &PhotocraftApp, ctx: &egui::Context, document: usize, size: Vec2, view: &mut crate::state::View) {
    let Some(state) = app.session.documents().get(document) else { return };
    let key = body_size_id(ctx.viewport_id(), state.doc.id);
    let previous = ctx.data(|data| data.get_temp::<Vec2>(key));
    match previous {
        None => {
            ctx.data_mut(|data| data.insert_temp(key, size));
        }
        Some(previous) if (size.x - previous.x).abs() > 0.5 || (size.y - previous.y).abs() > 0.5 => {
            view.center = [state.doc.size.width as f32 * 0.5, state.doc.size.height as f32 * 0.5];
            // Zoom, rotation and an explicit pending Fit request remain the user's choices.
            ctx.data_mut(|data| data.insert_temp(key, size));
            ctx.request_repaint_of(ViewportId::ROOT);
        }
        _ => {}
    }
}

/// A focused floating document owns the same tool pipeline as a docked document.
fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, document: usize, shortcuts: bool) {
    if document >= app.session.documents().len() {
        return;
    }
    let ctx = ui.ctx().clone();
    focus_v75::reconcile(app, &ctx);
    let available = focus_v75::available(app, &ctx, document);
    let rect = ui.available_rect_before_wrap();
    // Windows may paint a zero/minimum-size fallback during minimize/restore.
    // Neither that rect nor canvas_view's fit/navigation may overwrite the last valid view.
    if !available || !body_geometry_ready(app, &ctx, document, rect) {
        return;
    }
    let focused = shortcuts && ctx.input(|i| i.viewport().focused.unwrap_or(i.focused));
    // Cursor motion can end over another window in the same frame as the press.
    // Activate the canvas that actually received the press before routing tools.
    let pressed = ctx.input(|i| i.events.iter().any(|event| matches!(event, egui::Event::PointerButton { pos, pressed: true, .. } if rect.contains(*pos))));
    let modal = app.camera_raw.is_some() || app.discard.is_some() || app.jpeg_options.is_some() || !app.ui.dialogs.is_empty() || app.distort.liquify.is_some();
    if available && !modal && (focused || pressed) && app.session.active_index() != Some(document) && app.drag.is_none() && app.ui.transform.is_none() {
        app.session.set_active(document);
        app.sync_views();
        ctx.request_repaint_of(ViewportId::ROOT);
    }
    let active = available && app.session.active_index() == Some(document);
    if active && (focused || pressed) {
        focus_v75::remember(app, &ctx, document);
    }
    let document_id = app.session.documents()[document].doc.id;
    if active {
        app.last_canvas_rect = rect;
        if shortcuts && focused {
            crate::shortcuts::handle(app, &ctx);
            app.sync_tool_brush();
        }
    }
    // A shortcut may close/reindex documents. Never use the old index after that.
    if app.session.documents().get(document).is_none_or(|st| st.doc.id != document_id) || app.session.active_index() != Some(document) && active {
        return;
    }
    let Some(mut view) = app.ui.views.get(document).cloned() else { return };
    recenter_resized_body(app, &ctx, document, rect.size(), &mut view);
    crate::monitor_status::check_window(app, &ctx);
    let dialog_canvas = active && crate::dialogs::canvas_input_allowed(app, &ctx);
    let resizing = shortcuts && crate::titlebar::document_frame_resize_owns_input(&ctx, FRAME_GUTTER);
    ui.add_enabled_ui(available && !modal && !resizing, |ui| {
        // Keep normal widgets/tools disabled; canvas_view's separate dialog path
        // reads only navigation and top Color Picker sampling from raw events.
        let view = canvas::canvas_view(app, ui, document, rect, view, active && (!modal || dialog_canvas) && !resizing);
        if let Some(slot) = app.ui.views.get_mut(document) {
            *slot = view;
        }
    });
    if active {
        crate::mixer_preview::tick(app, &ctx);
    }
}

fn floating_title_tabs(app: &mut PhotocraftApp, ui: &mut egui::Ui, window: &DocWindow, area: Rect, blocked: bool) -> f32 {
    if area.width() < 22.0 {
        return area.left();
    }
    let t = crate::theme::Tokens::get(ui.ctx());
    let titles: Vec<_> = window
        .tabs
        .iter()
        .map(|tab| match tab {
            DetachedTab::Document(document) => app
                .session
                .documents()
                .get(*document)
                .map(|state| format!("{}{}", state.doc.name, if state.is_dirty() { " *" } else { "" }))
                .unwrap_or_default(),
            DetachedTab::Opening(job) => app
                .jobs
                .opens
                .iter()
                .find(|tab| tab.job == *job)
                .map(|tab| crate::i18n::fmt(tl!("Opening {name}…"), &[("name", &tab.name)]))
                .unwrap_or_default(),
        })
        .collect();
    let active = window.tabs.iter().position(|tab| *tab == DetachedTab::Document(window.document)).unwrap_or(0);
    let font = egui::FontId::proportional(11.5);
    let widths: Vec<_> =
        titles.iter().map(|label| (ui.painter().layout_no_wrap(label.clone(), font.clone(), t.text).size().x + 24.0).clamp(64.0, 220.0)).collect();
    let fit = crate::tab_strip::fit(&widths, active, area.width(), 54.0, 22.0);
    let mut selected = None;
    let mut left = area.left();
    for (i, width) in fit.shown {
        let (Some(tab), Some(label)) = (window.tabs.get(i), titles.get(i)) else { continue };
        let rect = Rect::from_min_size(pos2(left, area.top()), vec2(width, area.height()));
        left = rect.right();
        let ready = matches!(tab, DetachedTab::Document(_));
        let response = ui.interact(rect, Id::new(("floating-document-tab", window.id, i)), if ready && !blocked { Sense::click() } else { Sense::hover() });
        if i == active || response.hovered() {
            ui.painter().rect_filled(rect, 0.0, if i == active { t.card } else { t.hover });
        }
        if i == active {
            ui.painter().rect_filled(Rect::from_min_max(pos2(rect.left(), rect.bottom() - 2.0), rect.max), 0.0, t.accent);
        }
        let galley = crate::tab_strip::elided(ui, label, font.clone(), if ready { t.text } else { t.text_dim }, width - 12.0);
        ui.painter().with_clip_rect(rect).galley(rect.center() - galley.size() / 2.0, galley, t.text);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ready, label));
        if !blocked && response.on_hover_text(label).clicked() {
            selected = Some(*tab);
        }
    }
    if !fit.overflow.is_empty() {
        let rect = Rect::from_min_size(pos2(left, area.top()), vec2(22.0, area.height()));
        left = rect.right();
        let button = ui.interact(rect, Id::new(("floating-tab-overflow", window.id)), if blocked { Sense::hover() } else { Sense::click() });
        crate::icons::paint(ui, rect, "chevrons-right", 12.0, t.text_dim);
        if !blocked {
            egui::Popup::menu(&button).show(|ui| {
                for i in fit.overflow {
                    if let (Some(tab), Some(label)) = (window.tabs.get(i), titles.get(i))
                        && ui.add_enabled(matches!(tab, DetachedTab::Document(_)), egui::Button::new(label)).clicked()
                    {
                        selected = Some(*tab);
                        ui.close();
                    }
                }
            });
        }
    }
    if let Some(DetachedTab::Document(document)) = selected {
        select_document_tab(app, ui.ctx(), window.id, document);
    }
    left.min(area.right())
}

fn floating_drop_target(window: u64, document: DocId, at: Option<Pos2>, header: Option<Rect>, body: Rect) -> crate::file_open::DropTarget {
    use crate::file_open::DropTarget;
    match at {
        Some(p) if header.is_some_and(|r| r.contains(p)) => DropTarget::FloatingTabs(window),
        Some(p) if body.contains(p) => DropTarget::FloatingCanvas(document),
        _ => DropTarget::Elsewhere,
    }
}

fn drop_in_window(app: &mut PhotocraftApp, ui: &egui::Ui, window: u64, document: usize, body: Rect) {
    let ctx = ui.ctx();
    if ctx.current_pass_index() != 0 {
        return;
    }
    let files = ui.input_mut(|input| std::mem::take(&mut input.raw.dropped_files));
    if files.is_empty() {
        return;
    }
    let Some(document) = app.session.documents().get(document).map(|state| state.doc.id) else { return };
    let at = app.services.cursor_pos.as_mut().and_then(|read| read(ctx));
    let header = ctx.data(|d| d.get_temp::<Rect>(header_rect_id(window))).map(|r| r.union(Rect::from_min_max(r.min, pos2(r.right(), body.top()))));
    let target = floating_drop_target(window, document, at, header, body);
    app.route_dropped(files, target);
    if matches!(target, crate::file_open::DropTarget::FloatingCanvas(_)) && app.ui.transform.is_none() && !input_owned(app) {
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }
    ctx.request_repaint_of(ViewportId::ROOT);
}

#[cfg(test)]
#[path = "document_window_ui/resize_center_tests_v79.rs"]
mod resize_center_tests_v79;

#[cfg(test)]
#[path = "document_window_ui/floating_tabs_tests.rs"]
mod floating_tabs_tests;

pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    flush_document_closes(app, ctx);
    focus_v75::reconcile(app, ctx);
    let ids: Vec<_> = app.ui.windows.iter().filter(|w| w.open).map(|w| w.id).collect();
    for id in ids {
        let Some(window) = app.ui.windows.iter().find(|w| w.id == id && w.open).cloned() else { continue };
        let Some(st) = app.session.documents().get(window.document) else { continue };
        let document_id = st.doc.id;
        let title = format!("{}{}", st.doc.name, if st.is_dirty() { " *" } else { "" });
        let vid = ViewportId::from_hash_of(("docwin", window.id));
        let recreating = ctx.input(|i| !i.raw.viewports.contains_key(&vid));
        if recreating && manually_minimized(ctx, window.id) {
            ctx.data_mut(|d| d.insert_temp(minimize_pending_id(window.id), true));
        }
        sync_detached_window_level(ctx, vid, window.id);
        // Treat the root's minimized and transient 0×0 phases as hidden.  Windows can report
        // `visible = true` for one frame while the root is being minimized; accepting that frame
        // lets the detached child publish its fallback (0,0)/(minimum-size) geometry and that
        // fallback then becomes the next restore position.
        let root_visible = ctx.input(|i| {
            let Some(root) = i.raw.viewports.get(&ViewportId::ROOT) else { return true };
            root.visible() != Some(false) && root.minimized != Some(true) && !root.inner_rect.is_some_and(|rect| rect.width() < 16.0 || rect.height() < 16.0)
        });
        let root_was_hidden = ctx.data(|d| d.get_temp::<bool>(root_hidden_id(window.id))).unwrap_or(false);
        if !root_visible {
            ctx.data_mut(|d| d.insert_temp(root_hidden_id(window.id), true));
        } else if root_was_hidden {
            // Reapply the last good geometry before reading the child's first post-minimize
            // frame. This prevents a transient 160×96/(0,0) fallback from becoming the new
            // persisted position.
            restore_detached_geometry(ctx, vid, window.id, true);
        }
        // Keep the last valid native geometry. Windows can report a transient 0x0 child after
        // the root application is minimized; restoring the size and position prevents all
        // detached canvases from returning as tiny windows at the primary monitor origin.
        remember_detached_geometry(ctx, vid, window.id);
        // Root menus/popups can make a transparent borderless child look minimized for one native
        // frame. Restore it from the parent before `show_viewport_immediate`; the child callback
        // may be skipped while winit considers the viewport minimized.
        if app.discard.is_none() {
            keep_detached_viewport_visible(ctx, vid, window.id);
        }
        focus_v75::reconcile(app, ctx);
        let initial_position = ctx.data(|d| d.get_temp::<Pos2>(initial_position_id(window.document)));
        ctx.data_mut(|d| d.remove::<Pos2>(initial_position_id(window.document)));
        let mut builder = detached_viewport_builder(&title);
        if let Some(position) = initial_position {
            builder = builder.with_position(position);
        }
        if recreating && manually_minimized(ctx, window.id) {
            // Recreate without flashing or taking focus, then minimize the hidden HWND in its
            // first callback. Seed the restore rectangle from the last normal placement.
            builder = builder.with_visible(false).with_active(false);
            if let Some(saved) = ctx.data(|d| d.get_temp::<DetachedGeometry>(geometry_id(window.id))) {
                let scale = reported_viewport_pixels_per_point(ctx, vid).unwrap_or(saved.point_scale);
                builder = builder.with_position(saved.position_px / scale).with_inner_size(saved.size_px / scale);
            }
        }
        let mut return_to_tab = false;
        ctx.show_viewport_immediate(vid, builder, |ui, class| {
            app.collect_screenshots(ui.ctx());
            keep_detached_viewport_visible(ui.ctx(), vid, window.id);
            return_to_tab = ui.input(|i| i.viewport().close_requested());
            let tokens = crate::theme::Tokens::get(ui.ctx());
            // Use the same radius and border tokens as floating tool panels. The native backend
            // must separately disable system chrome; only this frame should draw the outline.
            let frame = detached_frame(ui.style(), tokens);
            // Keep the background transparent outside the card and reserve a narrow pasteboard
            // band for its soft shadow. This is renderer geometry, not native non-client chrome.
            let card_rect = ui.max_rect().shrink(FRAME_GUTTER);
            let mut card_ui = ui.new_child(egui::UiBuilder::new().max_rect(card_rect));
            frame.show(&mut card_ui, |ui| {
                if app.discard.is_some() {
                    ui.disable();
                    ui.ctx().data_mut(|d| {
                        d.remove::<TitleDrag>(title_drag_id(window.id));
                    });
                    crate::titlebar::cancel_resize(ui.ctx(), vid);
                }
                return_to_tab |= header(app, ui, &window, &title);
                if !app.ui.windows.iter().any(|w| w.id == window.id && w.open) {
                    // The following root frame retires the merged or closed viewport.
                    ui.ctx().data_mut(|d| d.insert_temp(Id::new(("photocraft-native-skip-present-once", vid)), true));
                    return;
                }
                if let Some(document) = app.ui.windows.iter().find(|w| w.open && w.id == window.id).map(|w| w.document) {
                    if class != egui::ViewportClass::EmbeddedWindow && app.discard.is_none() {
                        drop_in_window(app, ui, window.id, document, ui.available_rect_before_wrap());
                    }
                    // A synchronous title drop may select a newly opened tab.
                    let document = app.ui.windows.iter().find(|w| w.open && w.id == window.id).map_or(document, |w| w.document);
                    body(app, ui, document, class != egui::ViewportClass::EmbeddedWindow);
                }
            });
            if class != egui::ViewportClass::EmbeddedWindow
                && app.discard.is_none()
                && ui.ctx().data(|d| d.get_temp::<TitleDrag>(title_drag_id(window.id))).is_none()
            {
                let started_resize =
                    !crate::titlebar::resize_active(ui.ctx(), vid) && crate::titlebar::document_frame_resize_owns_input(ui.ctx(), FRAME_GUTTER);
                crate::titlebar::resize_zones_for_document(
                    ui,
                    &mut app.services.contact_screen_pos,
                    &mut app.services.screen_cursor_pos,
                    FRAME_GUTTER,
                    vec2(crate::titlebar::MIN_INNER_WIDTH, crate::titlebar::MIN_INNER_HEIGHT),
                );
                if started_resize {
                    // A complete Start/Move/End can finish in this callback without retaining
                    // an active state. That new user placement still supersedes old recovery.
                    discard_detached_recovery(ui.ctx(), window.id);
                }
            }
        });
        // Closing a document via a shortcut may have reindexed every remaining window.
        if app.session.documents().get(window.document).is_some_and(|st| st.doc.id == document_id) {
            if let Some(view) = app.ui.views.get(window.document).cloned()
                && let Some(w) = app.ui.windows.iter_mut().find(|w| w.id == window.id && w.document == window.document)
            {
                w.view = view;
            }
            if return_to_tab {
                dock(app, window.id, ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Modifiers, PointerButton};
    use serde_json::json;

    fn fixture() -> (PhotocraftApp, egui::Context) {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        for _ in 0..2 {
            app.run("file.new", json!({"width":100,"height":80,"background":"transparent"})).unwrap();
        }
        app.ui.tool = crate::state::Tool::Brush;
        app.sync_tool_brush();
        app.run("tools.setBrush", json!({"brush":{"size":10,"hardness":1.0,"spacing":0.1}})).unwrap();
        app.session.set_active(0);
        (app, ctx)
    }

    fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, events: Vec<Event>, focused: bool) {
        let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0))), events, focused, ..Default::default() };
        input.viewports.entry(ViewportId::ROOT).or_default().focused = Some(focused);
        let mut out = ctx.run_ui(input, |ui| body(app, ui, 1, true));
        out.textures_delta.clear();
    }

    fn pointer(app: &mut PhotocraftApp, ctx: &egui::Context, at: Pos2, pressed: Option<bool>) {
        let mut events = vec![Event::PointerMoved(at)];
        if let Some(pressed) = pressed {
            events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        }
        frame(app, ctx, events, true);
    }

    #[test]
    fn native_generic_dialog_canvas_samples_without_painting_or_switching_documents() {
        let (mut app, ctx) = fixture();
        ctx.set_embed_viewports(false);
        let window = crate::view_cmds::float_window(&mut app, 1, 1);
        let viewport = ViewportId::from_hash_of(("docwin", window));
        app.session.set_active(1);
        app.run("edit.fill", json!({"color":"#22aa44"})).unwrap();
        app.sync_views();
        let picker = crate::color_picker_ui::open(&mut app, "foreground");
        let revision = app.session.active().unwrap().revision;
        let at = pos2(250.0, 200.0);
        for events in [
            vec![],
            vec![],
            vec![Event::PointerMoved(at), Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }],
            vec![Event::PointerMoved(at + vec2(2.0, 0.0))],
            vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }],
        ] {
            let mut input =
                egui::RawInput { viewport_id: viewport, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0))), events, ..Default::default() };
            input.viewports.entry(viewport).or_default().focused = Some(true);
            let mut out = ctx.run_ui(input, |ui| body(&mut app, ui, 1, true));
            out.textures_delta.clear();
        }
        assert_eq!(app.ui.dialog_mut(picker).unwrap().fields["color"], json!("#22aa44"));
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert_eq!(app.session.active_index(), Some(1));
        assert!(app.drag.is_none() && app.live_stroke.is_none());
        // A non-picker generic dialog still grants navigation but a click cannot paint.
        app.ui.close_dialog(picker);
        app.ui.open_dialog(crate::state::DialogKind::About, Default::default());
        let mut input = egui::RawInput {
            viewport_id: viewport,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0))),
            events: vec![Event::PointerMoved(at), Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }],
            ..Default::default()
        };
        input.viewports.entry(viewport).or_default().focused = Some(true);
        let mut out = ctx.run_ui(input, |ui| body(&mut app, ui, 1, true));
        out.textures_delta.clear();
        assert_eq!(app.session.active().unwrap().revision, revision);
        assert!(app.drag.is_none() && app.live_stroke.is_none());
    }

    #[test]
    fn detached_header_paints_menu_chrome_inside_rounded_frame_in_all_themes() {
        let ctx = egui::Context::default();
        let bar = Rect::from_min_size(pos2(11.0, 11.0), vec2(778.0, 28.0));
        for kind in crate::theme::ThemeKind::ALL {
            let tokens = crate::theme::Tokens::for_kind(kind);
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| paint_header_background(ui, bar, tokens));
            let shape = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Rect(r) if r.fill == tokens.chrome => Some(r),
                    _ => None,
                })
                .expect("title background must be painted");
            assert_eq!(shape.rect, Rect::from_min_max(pos2(7.0, 7.0), pos2(793.0, 39.0)), "{kind:?}: preserve 1pt outer stroke");
            assert_eq!(shape.corner_radius.nw, (tokens.radius_lg - 1.0).max(0.0).round() as u8);
            assert_eq!(shape.corner_radius.ne, shape.corner_radius.nw);
            assert_eq!(shape.corner_radius.sw, 0);
            assert_eq!(shape.corner_radius.se, 0);
            assert_ne!(tokens.chrome, tokens.canvas, "{kind:?}: title must contrast with pasteboard");
            output.textures_delta.clear();
        }
    }

    #[test]
    fn detached_viewport_is_transparent_for_theme_rounded_corners() {
        let builder = detached_viewport_builder("Test document");
        assert_eq!(builder.decorations, Some(false));
        assert_eq!(builder.transparent, Some(true));
        assert_eq!(builder.taskbar, Some(true), "minimized documents must remain accessible from the taskbar");
        assert_eq!(builder.window_level, Some(egui::WindowLevel::Normal), "detached windows must never be topmost");
        assert_eq!(builder.has_shadow, Some(false));
    }

    #[test]
    fn detached_frame_matches_window_radius_without_expanding_the_native_surface() {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Studio);
        let style = ctx.style_of(egui::Theme::Dark);
        let tokens = crate::theme::Tokens::get(&ctx);
        let frame = detached_frame(&style, tokens);
        let expected_radius = egui::CornerRadius::same(tokens.radius_lg as u8);
        assert_eq!(frame.corner_radius, expected_radius);
        let radius = expected_radius;
        assert!(radius.nw > 0 && radius.ne > 0 && radius.sw > 0 && radius.se > 0, "all four detached corners must use the theme radius");
        let expected_inset = 4;
        assert_eq!(frame.inner_margin, egui::Margin::same(expected_inset), "rectangular canvas content must stay inside every rounded corner");
        assert_eq!(frame.fill, tokens.canvas, "detached documents use the canvas surface, not a raised panel card");

        assert_eq!(frame.outer_margin, egui::Margin::ZERO, "the card must stay tight to the native viewport");
        assert_eq!(frame.shadow.offset, [0, 3]);
        assert_eq!(frame.shadow.blur, 10);
        assert_eq!(frame.shadow.spread, 0);
        assert_eq!(frame.shadow.color, tokens.shadow);
    }

    #[test]
    fn detached_frame_uses_each_theme_radius_even_when_visuals_are_stale() {
        let ctx = egui::Context::default();
        for kind in crate::theme::ThemeKind::ALL {
            crate::theme::apply(&ctx, kind);
            // Simulate a child viewport retaining the default visuals while the theme token has
            // already switched. The frame must still follow the active theme's radius.
            let stale = egui::Style::default();
            let tokens = crate::theme::Tokens::get(&ctx);
            let frame = detached_frame(&stale, tokens);
            assert_eq!(frame.corner_radius, egui::CornerRadius::same(tokens.radius_lg as u8), "{kind:?}");
            assert_eq!(frame.fill, tokens.canvas, "{kind:?}: detached canvas should use the canvas surface");
        }
    }

    #[test]
    fn classic_detached_frame_has_contrast_against_the_canvas() {
        let tokens = crate::theme::Tokens::for_kind(crate::theme::ThemeKind::Classic);
        let frame = detached_frame(&egui::Style::default(), tokens);
        assert_eq!(frame.fill, tokens.canvas);
        assert_eq!(frame.stroke.color, tokens.text_dim);
        assert_ne!(frame.stroke.color, frame.fill, "Classic's floating frame must not blend into the canvas surround");
    }

    #[test]
    fn root_click_and_occlusion_never_raise_or_restore_a_document() {
        let ctx = egui::Context::default();
        let vid = ViewportId::ROOT;
        let mut input = egui::RawInput::default();
        input.events.push(Event::PointerButton { pos: Pos2::ZERO, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        input.viewports.entry(vid).or_default().occluded = Some(true);
        input.viewports.entry(ViewportId::ROOT).or_default().focused = Some(true);
        let mut out = ctx.run_ui(input, |_| {
            sync_detached_window_level(&ctx, vid, 7);
            keep_detached_viewport_visible(&ctx, vid, 7);
        });
        out.textures_delta.clear();
        let commands = &out.viewport_output.get(&vid).unwrap().commands;
        assert!(commands.contains(&ViewportCommand::WindowLevel(egui::WindowLevel::Normal)));
        assert!(
            !commands.iter().any(|cmd| matches!(
                cmd,
                ViewportCommand::Focus
                    | ViewportCommand::Visible(_)
                    | ViewportCommand::Minimized(_)
                    | ViewportCommand::OuterPosition(_)
                    | ViewportCommand::InnerSize(_)
                    | ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop)
            )),
            "root input must not change document placement or steal focus: {commands:?}"
        );
    }

    #[test]
    fn manual_minimize_preserves_geometry_until_native_taskbar_restore() {
        let ctx = egui::Context::default();
        let viewport = ViewportId::from_hash_of("taskbar-restore-test");
        let window = 71;
        let normal = Rect::from_min_size(pos2(240.0, 180.0), vec2(800.0, 600.0));
        let frame = |minimized, rect| {
            let mut input = egui::RawInput::default();
            input.viewports.entry(ViewportId::ROOT).or_default().minimized = Some(false);
            let info = input.viewports.entry(viewport).or_default();
            info.minimized = Some(minimized);
            info.native_pixels_per_point = Some(1.0);
            info.outer_rect = Some(rect);
            info.inner_rect = Some(rect);
            let mut out = ctx.run_ui(input, |_| {
                remember_detached_geometry(&ctx, viewport, window);
                keep_detached_viewport_visible(&ctx, viewport, window);
            });
            out.textures_delta.clear();
            out
        };
        frame(false, normal);
        let saved = ctx.data(|d| d.get_temp::<DetachedGeometry>(geometry_id(window))).unwrap();
        ctx.data_mut(|d| d.insert_temp(manual_minimize_id(window), true));
        // Windows may publish an iconic position/size while the root stays visible.
        let out = frame(true, Rect::from_min_size(pos2(-32000.0, -32000.0), vec2(160.0, 28.0)));
        let during = ctx.data(|d| d.get_temp::<DetachedGeometry>(geometry_id(window))).unwrap();
        assert_eq!(during.position_px, saved.position_px);
        assert_eq!(during.size_px, saved.size_px);
        assert!(ctx.data(|d| d.get_temp::<bool>(manual_minimize_id(window))).unwrap());
        assert!(
            !out.viewport_output.values().flat_map(|v| &v.commands).any(|cmd| matches!(
                cmd,
                ViewportCommand::Minimized(false) | ViewportCommand::Visible(true) | ViewportCommand::OuterPosition(_) | ViewportCommand::InnerSize(_)
            )),
            "an intentional minimize must not be reversed or repositioned"
        );
        // The taskbar performs the native restore; the app accepts the original placement.
        frame(false, normal);
        assert!(ctx.data(|d| d.get_temp::<bool>(manual_minimize_id(window))).is_none());
        let restored = ctx.data(|d| d.get_temp::<DetachedGeometry>(geometry_id(window))).unwrap();
        assert_eq!(restored.position_px, saved.position_px);
        assert_eq!(restored.size_px, saved.size_px);
    }

    #[test]
    fn manual_minimize_survives_root_minimize_restore_and_viewport_recreation() {
        for recreate in [false, true] {
            let (mut app, ctx) = fixture();
            let window = crate::view_cmds::float_window(&mut app, 1, 0);
            let viewport = ViewportId::from_hash_of(("docwin", window));
            ctx.set_embed_viewports(false);
            let saved = DetachedGeometry { position_px: pos2(340.0, 220.0), size_px: vec2(720.0, 510.0), point_scale: 1.0 };
            ctx.data_mut(|d| d.insert_temp(geometry_id(window), saved));
            let step = |root_minimized: bool, child_minimized: Option<bool>, caption: bool| {
                let mut input = egui::RawInput::default();
                let root = input.viewports.entry(ViewportId::ROOT).or_default();
                root.minimized = Some(root_minimized);
                root.maximized = Some(!root_minimized);
                if let Some(minimized) = child_minimized {
                    let child = input.viewports.entry(viewport).or_default();
                    child.minimized = Some(minimized);
                    child.native_pixels_per_point = Some(1.0);
                    // Includes the misleading normal-looking rect of a freshly recreated HWND.
                    child.outer_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(160.0, 96.0)));
                    child.inner_rect = child.outer_rect;
                }
                let mut out = ctx.run_ui(input, |_| {
                    // Keep a real viewport output so assertions inspect actual commands.
                    ctx.show_viewport_deferred(viewport, detached_viewport_builder("Minimize regression"), |_, _| {});
                    if caption {
                        request_manual_minimize(&ctx, viewport, window);
                    } else if root_minimized {
                        root_hidden(&app, &ctx);
                    } else {
                        restore_detached_geometry(&ctx, viewport, window, true);
                        remember_detached_geometry(&ctx, viewport, window);
                        keep_detached_viewport_visible(&ctx, viewport, window);
                    }
                });
                out.textures_delta.clear();
                out.viewport_output.remove(&viewport).unwrap().commands
            };
            assert!(step(false, Some(false), true).contains(&ViewportCommand::Minimized(true)));
            // Before the first native ack, a delayed false must not cancel the caption request.
            assert!(step(false, Some(false), false).contains(&ViewportCommand::Minimized(true)));
            step(false, Some(true), false);
            step(true, Some(true), false);
            if recreate {
                step(true, None, false);
                let commands = step(false, None, false);
                assert!(commands.contains(&ViewportCommand::Minimized(true)));
            }
            // Root restores/maximizes; child may be newly created or transiently shown by OS.
            let commands = step(false, Some(false), false);
            assert!(commands.contains(&ViewportCommand::Minimized(true)), "recreate={recreate}");
            assert!(!commands.iter().any(|cmd| matches!(
                cmd,
                ViewportCommand::Minimized(false)
                    | ViewportCommand::Visible(true)
                    | ViewportCommand::Focus
                    | ViewportCommand::InnerSize(_)
                    | ViewportCommand::OuterPosition(_)
            )));
            assert!(manually_minimized(&ctx, window));
            assert_eq!(ctx.data(|d| d.get_temp::<DetachedGeometry>(geometry_id(window))), Some(saved));
            step(false, Some(true), false);
            assert!(manually_minimized(&ctx, window));
            // After ack, a later taskbar restore is allowed and releases the retained intent.
            let commands = step(false, Some(false), false);
            assert!(!commands.contains(&ViewportCommand::Minimized(true)));
            assert!(!manually_minimized(&ctx, window));
            assert_eq!(ctx.data(|d| d.get_temp::<DetachedGeometry>(geometry_id(window))), Some(saved));
        }
    }

    #[test]
    fn float_in_window_menu_restores_only_existing_active_document() {
        let (mut app, ctx) = fixture();
        let target = crate::view_cmds::float_window(&mut app, 0, 0);
        let other = crate::view_cmds::float_window(&mut app, 1, 0);
        let target_viewport = ViewportId::from_hash_of(("docwin", target));
        let other_viewport = ViewportId::from_hash_of(("docwin", other));
        let saved = DetachedGeometry { position_px: pos2(310.0, 190.0), size_px: vec2(740.0, 520.0), point_scale: 1.0 };
        ctx.set_embed_viewports(false);
        ctx.data_mut(|d| {
            for window in [target, other] {
                d.insert_temp(manual_minimize_id(window), true);
                d.insert_temp(minimize_pending_id(window), true);
                d.insert_temp(geometry_id(window), saved);
                d.insert_temp(root_hidden_id(window), true);
            }
            // A previous failed placement must not throttle this explicit user request.
            d.insert_temp(geometry_restore_pending_id(target), true);
            d.insert_temp(geometry_restore_cooldown_id(target), 3_u8);
        });
        let mut input = egui::RawInput::default();
        for viewport in [target_viewport, other_viewport] {
            input.viewports.entry(viewport).or_default().minimized = Some(true);
        }
        let mut out = ctx.run_ui(input, |_| {
            for viewport in [target_viewport, other_viewport] {
                ctx.show_viewport_deferred(viewport, detached_viewport_builder("Menu restore regression"), |_, _| {});
            }
            let result = crate::menus::invoke(&mut app, &ctx, "window.arrange.floatInWindow", json!({})).unwrap();
            assert_eq!(result["window"], json!(target));
        });
        out.textures_delta.clear();
        assert_eq!(app.ui.windows.iter().map(|w| w.id).collect::<Vec<_>>(), vec![target, other]);
        assert!(!manually_minimized(&ctx, target));
        assert!(manually_minimized(&ctx, other));
        for window in [target, other] {
            assert_eq!(ctx.data(|d| d.get_temp::<DetachedGeometry>(geometry_id(window))), Some(saved));
        }
        let commands = &out.viewport_output[&target_viewport].commands;
        for expected in [
            ViewportCommand::Visible(true),
            ViewportCommand::Minimized(false),
            ViewportCommand::OuterPosition(saved.position_px),
            ViewportCommand::InnerSize(saved.size_px),
            ViewportCommand::Focus,
        ] {
            assert!(commands.contains(&expected), "missing {expected:?}: {commands:?}");
        }
        assert_eq!(commands.last(), Some(&ViewportCommand::Focus));
        assert!(out.viewport_output[&other_viewport].commands.is_empty());
        let root_commands = &out.viewport_output[&ViewportId::ROOT].commands;
        eprintln!("menu restore root commands: {root_commands:?}");
        // egui's first pass synchronizes native theme even when the app queues no
        // root window action. Permit that framework command, but no restore/focus/
        // geometry action (or any other command) may leak to the root viewport.
        assert!(root_commands.iter().all(|command| matches!(command, ViewportCommand::SetTheme(_))), "unexpected root commands: {root_commands:?}");
    }

    #[test]
    fn native_restore_intent_rejects_delayed_owner_restore_after_minimize_ack() {
        let (mut app, ctx) = fixture();
        let window = crate::view_cmds::float_window(&mut app, 1, 0);
        let viewport = ViewportId::from_hash_of(("docwin", window));
        ctx.set_embed_viewports(false);
        let saved = DetachedGeometry { position_px: pos2(340.0, 220.0), size_px: vec2(720.0, 510.0), point_scale: 1.0 };
        ctx.data_mut(|d| {
            d.insert_temp(geometry_id(window), saved);
            d.insert_temp(Id::new(("photocraft-native-restore-supported", viewport)), true);
        });
        let step = |root_minimized, child_minimized: Option<bool>, caption| {
            let mut input = egui::RawInput::default();
            input.viewports.entry(ViewportId::ROOT).or_default().minimized = Some(root_minimized);
            if let Some(minimized) = child_minimized {
                let child = input.viewports.entry(viewport).or_default();
                child.minimized = Some(minimized);
                child.native_pixels_per_point = Some(1.0);
                child.outer_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)));
                child.inner_rect = child.outer_rect;
            }
            let mut out = ctx.run_ui(input, |_| {
                ctx.show_viewport_deferred(viewport, detached_viewport_builder("Native restore regression"), |_, _| {});
                if caption {
                    request_manual_minimize(&ctx, viewport, window);
                } else {
                    if root_minimized {
                        root_hidden(&app, &ctx);
                    }
                    remember_detached_geometry(&ctx, viewport, window);
                    keep_detached_viewport_visible(&ctx, viewport, window);
                }
            });
            out.textures_delta.clear();
            out.viewport_output.remove(&viewport).unwrap().commands
        };
        step(false, Some(false), true);
        step(false, Some(true), false);
        step(true, Some(true), false);
        // The native failure kept the same HWND: root restored while child was still
        // iconic, then Windows showed it at (0, 0) in a later message cycle.
        ctx.data_mut(|d| d.insert_temp(native_restore_request_id(ViewportId::ROOT), 1_u8));
        step(false, Some(true), false);
        for minimized in [Some(false), Some(true), Some(false), None, Some(false)] {
            let commands = step(false, minimized, false);
            if minimized != Some(true) {
                assert!(commands.contains(&ViewportCommand::Minimized(true)));
            }
            assert!(manually_minimized(&ctx, window));
            assert_eq!(ctx.data(|d| d.get_temp::<DetachedGeometry>(geometry_id(window))), Some(saved));
            assert!(!commands.iter().any(|cmd| matches!(cmd, ViewportCommand::Minimized(false) | ViewportCommand::Focus | ViewportCommand::Visible(true))));
        }
        // Only the child's own native system command releases its retained intent.
        ctx.data_mut(|d| d.insert_temp(native_restore_request_id(viewport), 1_u8));
        let commands = step(false, Some(false), false);
        assert!(!manually_minimized(&ctx, window));
        assert!(!commands.contains(&ViewportCommand::Minimized(true)));
        assert!(commands.contains(&ViewportCommand::OuterPosition(saved.position_px)));
        assert!(commands.contains(&ViewportCommand::InnerSize(saved.size_px)));
        assert_eq!(ctx.data(|d| d.get_temp::<DetachedGeometry>(geometry_id(window))), Some(saved));
        step(false, Some(false), true);
        step(false, Some(true), false);
        ctx.data_mut(|d| d.insert_temp(native_restore_request_id(viewport), 2_u8));
        let commands = step(false, Some(false), false);
        assert!(!manually_minimized(&ctx, window));
        assert!(
            !commands.iter().any(|cmd| matches!(cmd, ViewportCommand::Minimized(true) | ViewportCommand::InnerSize(_) | ViewportCommand::OuterPosition(_)))
        );
        // Native taskbar/system-menu minimize also owns a durable manual intent.
        ctx.data_mut(|d| d.insert_temp(native_restore_request_id(viewport), 3_u8));
        let commands = step(false, Some(false), false);
        assert!(manually_minimized(&ctx, window));
        assert!(!commands.contains(&ViewportCommand::Minimized(false)));
        let commands = step(false, Some(true), false);
        assert!(manually_minimized(&ctx, window));
        assert!(!commands.contains(&ViewportCommand::Minimized(false)));
    }

    #[test]
    fn visibility_recovery_is_suspended_during_detached_title_drag() {
        let ctx = egui::Context::default();
        assert!(!detached_drag_active(&ctx, 42));
        ctx.data_mut(|d| {
            d.insert_temp(
                title_drag_id(42),
                TitleDrag {
                    offset: Vec2::ZERO,
                    touch: true,
                    start_outer: None,
                    start_screen: None,
                    start_local: Pos2::ZERO,
                    contact: None,
                    native_screen: false,
                },
            );
        });
        assert!(detached_drag_active(&ctx, 42));
        assert!(!detached_drag_active(&ctx, 43));
    }

    #[test]
    fn detached_geometry_recovery_only_triggers_for_invalid_native_size() {
        let ctx = egui::Context::default();
        let viewport = ViewportId::from_hash_of(("docwin", 7_u64));
        let valid = Rect::from_min_size(pos2(240.0, 160.0), vec2(640.0, 480.0));
        let mut input = egui::RawInput::default();
        input.viewports.entry(viewport).or_default().inner_rect = Some(valid);
        let mut out = ctx.run_ui(input, |_| {});
        out.textures_delta.clear();
        assert!(!detached_geometry_needs_restore(&ctx, viewport));

        let mut input = egui::RawInput::default();
        input.viewports.entry(viewport).or_default().inner_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1.0, 1.0)));
        let mut out = ctx.run_ui(input, |_| {});
        out.textures_delta.clear();
        assert!(detached_geometry_needs_restore(&ctx, viewport));
    }

    #[test]
    fn detached_geometry_cache_uses_native_pixels_across_dpi_changes() {
        let ctx = egui::Context::default();
        let viewport = ViewportId::from_hash_of(("docwin", 9_u64));
        let mut input = egui::RawInput::default();
        let info = input.viewports.entry(viewport).or_default();
        info.native_pixels_per_point = Some(2.0);
        info.outer_rect = Some(Rect::from_min_size(pos2(100.0, 50.0), vec2(420.0, 320.0)));
        info.inner_rect = Some(Rect::from_min_size(pos2(110.0, 60.0), vec2(400.0, 300.0)));
        let mut out = ctx.run_ui(input, |_| {});
        out.textures_delta.clear();

        remember_detached_geometry(&ctx, viewport, 9);
        let saved = ctx.data(|d| d.get_temp::<DetachedGeometry>(geometry_id(9))).expect("geometry is valid");
        assert_eq!(saved.position_px, pos2(200.0, 100.0));
        assert_eq!(saved.size_px, vec2(800.0, 600.0));

        // Windows may recreate the child on a 1x monitor after restoring the root. The same
        // physical geometry should still match after egui reports coordinates in 1x points.
        let mut input = egui::RawInput::default();
        let info = input.viewports.entry(viewport).or_default();
        info.native_pixels_per_point = Some(1.0);
        info.outer_rect = Some(Rect::from_min_size(pos2(200.0, 100.0), vec2(420.0, 320.0)));
        info.inner_rect = Some(Rect::from_min_size(pos2(210.0, 110.0), vec2(800.0, 600.0)));
        let mut out = ctx.run_ui(input, |_| {});
        out.textures_delta.clear();
        assert!(detached_geometry_matches_saved(&ctx, viewport, saved));
    }

    #[test]
    fn touch_drag_ignores_the_emulated_pointer_sample() {
        let ctx = egui::Context::default();
        let touch = pos2(120.0, 64.0);
        let synthetic = pos2(118.0, 61.0);
        let input = egui::RawInput {
            events: vec![
                egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase: egui::TouchPhase::Move, pos: touch, force: Some(0.7) },
                egui::Event::PointerMoved(synthetic),
            ],
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |_| {});
        out.textures_delta.clear();
        assert_eq!(latest_drag_position(&ctx, true), Some(touch));
        assert_eq!(latest_drag_position(&ctx, false), Some(synthetic));
        assert!(drag_position_changed(&ctx, true));
    }

    #[test]
    fn pen_contact_survives_mouse_leave_and_an_empty_frame() {
        let ctx = egui::Context::default();
        let drag = TitleDrag {
            offset: Vec2::ZERO,
            touch: true,
            start_outer: None,
            start_screen: None,
            start_local: Pos2::ZERO,
            contact: Some((egui::TouchDeviceId(1), egui::TouchId(1))),
            native_screen: false,
        };
        let touch = |phase| Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase, pos: pos2(80.0, 20.0), force: Some(0.7) };
        for events in [vec![touch(egui::TouchPhase::Start)], vec![touch(egui::TouchPhase::Move), Event::PointerGone], vec![]] {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |_| {});
            out.textures_delta.clear();
            assert!(!title_drag_ended(&ctx, drag), "mouse leave / no sample is not pen up");
        }
        let mut out = ctx.run_ui(egui::RawInput { events: vec![touch(egui::TouchPhase::End)], ..Default::default() }, |_| {});
        out.textures_delta.clear();
        assert!(title_drag_ended(&ctx, drag));
    }

    fn title_frame(app: &mut PhotocraftApp, ctx: &egui::Context, window: &DocWindow, outer: Option<Pos2>, events: Vec<Event>) -> Vec<ViewportCommand> {
        title_frame_result(app, ctx, window, outer, events).0
    }

    fn title_frame_result(
        app: &mut PhotocraftApp,
        ctx: &egui::Context,
        window: &DocWindow,
        outer: Option<Pos2>,
        events: Vec<Event>,
    ) -> (Vec<ViewportCommand>, bool) {
        let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0))), events, ..Default::default() };
        let info = input.viewports.entry(ViewportId::ROOT).or_default();
        info.inner_rect = outer.map(|p| Rect::from_min_size(p, vec2(500.0, 400.0)));
        info.outer_rect = info.inner_rect;
        info.focused = Some(true);
        let mut docked = false;
        let mut out = ctx.run_ui(input, |ui| {
            docked = header(app, ui, window, "Test document");
        });
        out.textures_delta.clear();
        (out.viewport_output.remove(&ViewportId::ROOT).map(|v| v.commands).unwrap_or_default(), docked)
    }

    fn contact(id: u64, phase: egui::TouchPhase, pos: Pos2) -> Event {
        Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(id), phase, pos, force: Some(0.7) }
    }

    #[test]
    fn native_pen_drag_does_not_wait_for_ack_or_feed_window_motion_back_into_pointer_motion() {
        let (mut app, ctx) = fixture();
        let id = crate::view_cmds::float_window(&mut app, 1, 0);
        let window = app.ui.windows.iter().find(|w| w.id == id).unwrap().clone();
        let desktop = std::rc::Rc::new(std::cell::Cell::new(pos2(380.0, 220.0)));
        let sample = desktop.clone();
        app.services.screen_cursor_pos = Some(Box::new(|| Some(pos2(380.0, 220.0))));
        app.services.contact_screen_pos = Some(Box::new(move |_, at_start| Some(if at_start { pos2(380.0, 220.0) } else { sample.get() })));
        let press = pos2(80.0, 20.0);
        ctx.data_mut(|d| {
            d.insert_temp(root_hidden_id(id), true);
            d.insert_temp(geometry_restore_pending_id(id), true);
        });
        title_frame(
            &mut app,
            &ctx,
            &window,
            Some(pos2(300.0, 200.0)),
            vec![
                contact(1, egui::TouchPhase::Start, press),
                Event::PointerButton { pos: press, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
            ],
        );
        assert!(ctx.data(|d| d.get_temp::<bool>(root_hidden_id(id))).is_none(), "a new user placement supersedes a stale restore");
        desktop.set(pos2(430.0, 220.0));
        let moved =
            title_frame(&mut app, &ctx, &window, Some(pos2(300.0, 200.0)), vec![contact(1, egui::TouchPhase::Move, pos2(130.0, 20.0)), Event::PointerGone]);
        assert!(moved.iter().any(|cmd| matches!(cmd, ViewportCommand::OuterPosition(p) if *p == pos2(350.0, 200.0))));
        assert!(detached_drag_active(&ctx, id), "PointerGone must not end a contact");
        // The cached local event is still 130, but the native origin has changed. The desktop
        // pointer is stationary, so adding that new origin must not produce an extra 50px move.
        let stationary = title_frame(&mut app, &ctx, &window, Some(pos2(348.0, 200.0)), vec![contact(1, egui::TouchPhase::Move, pos2(130.0, 20.0))]);
        assert!(!stationary.iter().any(|cmd| matches!(cmd, ViewportCommand::OuterPosition(_))));
        desktop.set(pos2(480.0, 220.0));
        let next = title_frame(&mut app, &ctx, &window, None, vec![]);
        assert!(
            next.iter().any(|cmd| matches!(cmd, ViewportCommand::OuterPosition(p) if *p == pos2(400.0, 200.0))),
            "a missing/mismatched ACK and no client event must not freeze native sampling"
        );
        title_frame(&mut app, &ctx, &window, Some(pos2(400.0, 200.0)), vec![contact(2, egui::TouchPhase::End, press)]);
        assert!(detached_drag_active(&ctx, id), "another finger cannot release the pen");
        title_frame(&mut app, &ctx, &window, Some(pos2(400.0, 200.0)), vec![contact(1, egui::TouchPhase::End, press)]);
        assert!(!detached_drag_active(&ctx, id));
        let trace = drag_diagnostics(&ctx);
        assert_eq!(trace.iter().filter(|entry| entry["phase"] == "start").count(), 1, "drag threshold must not reset the anchor");
    }

    #[test]
    fn active_title_drag_blocks_geometry_and_layer_commands_and_focus_loss_cleans_it_up() {
        let (mut app, ctx) = fixture();
        let id = crate::view_cmds::float_window(&mut app, 1, 0);
        let window = app.ui.windows.iter().find(|w| w.id == id).unwrap().clone();
        let sample = std::rc::Rc::new(std::cell::Cell::new(pos2(380.0, 220.0)));
        let desktop = sample.clone();
        app.services.contact_screen_pos = Some(Box::new(move |_, at_start| Some(if at_start { pos2(380.0, 220.0) } else { desktop.get() })));
        title_frame(&mut app, &ctx, &window, Some(pos2(300.0, 200.0)), vec![contact(1, egui::TouchPhase::Start, pos2(80.0, 20.0))]);
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {
            let vid = ViewportId::from_hash_of(("docwin", id));
            remember_detached_geometry(&ctx, vid, id);
            restore_detached_geometry(&ctx, vid, id, true);
            sync_detached_window_level(&ctx, vid, id);
            keep_detached_viewport_visible(&ctx, vid, id);
        });
        out.textures_delta.clear();
        assert!(out.viewport_output.values().all(|viewport| viewport.commands.is_empty()));
        sample.set(pos2(900.0, 900.0));
        let lost = title_frame(&mut app, &ctx, &window, Some(pos2(300.0, 200.0)), vec![Event::WindowFocused(false)]);
        assert!(!lost.iter().any(|cmd| matches!(cmd, ViewportCommand::OuterPosition(_))), "switching apps must not move the canvas to the external cursor");
        assert!(!detached_drag_active(&ctx, id));
    }

    #[test]
    fn pen_resize_suspends_detached_geometry_and_layer_recovery() {
        let (mut app, ctx) = fixture();
        let window = 91;
        let viewport = ViewportId::from_hash_of(("docwin", window));
        ctx.data_mut(|d| {
            d.insert_temp(geometry_id(window), DetachedGeometry { position_px: pos2(100.0, 100.0), size_px: vec2(500.0, 400.0), point_scale: 1.0 })
        });
        let mut input = egui::RawInput {
            viewport_id: viewport,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0))),
            events: vec![contact(1, egui::TouchPhase::Start, pos2(498.0, 398.0))],
            ..Default::default()
        };
        input.viewports.entry(viewport).or_default().focused = Some(true);
        let mut out = ctx.run_ui(input, |ui| {
            crate::titlebar::resize_zones(ui, &mut app.services.contact_screen_pos);
            assert!(detached_drag_active(&ctx, window));
            remember_detached_geometry(&ctx, viewport, window);
            restore_detached_geometry(&ctx, viewport, window, true);
            sync_detached_window_level(&ctx, viewport, window);
            keep_detached_viewport_visible(&ctx, viewport, window);
        });
        out.textures_delta.clear();
        assert!(
            out.viewport_output.values().all(|v| v.commands.iter().all(|cmd| !matches!(
                cmd,
                ViewportCommand::InnerSize(_)
                    | ViewportCommand::OuterPosition(_)
                    | ViewportCommand::WindowLevel(_)
                    | ViewportCommand::Focus
                    | ViewportCommand::Visible(_)
                    | ViewportCommand::Minimized(_)
            ))),
            "recovery must not fight a live pen resize: {:?}",
            out.viewport_output.values().flat_map(|v| &v.commands).collect::<Vec<_>>()
        );
    }

    #[test]
    fn pointer_release_ends_drag_even_when_an_unrelated_touch_is_down() {
        let (mut app, ctx) = fixture();
        let id = crate::view_cmds::float_window(&mut app, 1, 0);
        let window = app.ui.windows.iter().find(|w| w.id == id).unwrap().clone();
        let press = pos2(80.0, 20.0);
        let button = |pressed| Event::PointerButton { pos: press, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        title_frame(&mut app, &ctx, &window, Some(pos2(300.0, 200.0)), vec![button(true)]);
        assert!(detached_drag_active(&ctx, id));
        title_frame(&mut app, &ctx, &window, Some(pos2(300.0, 200.0)), vec![contact(2, egui::TouchPhase::Start, pos2(200.0, 150.0)), button(false)]);
        assert!(!detached_drag_active(&ctx, id), "an unrelated finger must not keep the released pointer dragging");
    }

    #[test]
    fn cancelling_over_the_dock_target_does_not_dock_the_document() {
        for end in [Event::WindowFocused(false), contact(1, egui::TouchPhase::Cancel, pos2(80.0, 20.0))] {
            let (mut app, ctx) = fixture();
            let id = crate::view_cmds::float_window(&mut app, 1, 0);
            let window = app.ui.windows.iter().find(|w| w.id == id).unwrap().clone();
            app.services.screen_cursor_pos = Some(Box::new(|| Some(pos2(380.0, 220.0))));
            ctx.data_mut(|d| d.insert_temp(target_id(), Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 1000.0)))));
            title_frame(&mut app, &ctx, &window, Some(pos2(300.0, 200.0)), vec![contact(1, egui::TouchPhase::Start, pos2(80.0, 20.0))]);
            let (_, docked) = title_frame_result(&mut app, &ctx, &window, Some(pos2(300.0, 200.0)), vec![end]);
            assert!(!docked, "cancel/focus loss is not a docking release");
            assert!(!detached_drag_active(&ctx, id));
        }
    }

    #[test]
    fn pointer_press_origin_accepts_pen_drivers_without_touch_events() {
        let ctx = egui::Context::default();
        let origin = pos2(96.0, 18.0);
        let mut out = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::PointerButton { pos: origin, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }],
                ..Default::default()
            },
            |_| {},
        );
        out.textures_delta.clear();
        assert_eq!(pointer_press_origin(&ctx), Some(origin));
    }

    #[test]
    fn pen_drag_target_uses_saved_outer_position_when_inner_rect_is_temporarily_missing() {
        let ctx = egui::Context::default();
        let start = pos2(120.0, 24.0);
        let state = TitleDrag {
            offset: start.to_vec2(),
            touch: true,
            start_outer: Some(pos2(760.0, 420.0)),
            start_screen: None,
            start_local: start,
            contact: None,
            native_screen: false,
        };
        let moved = pos2(168.0, 57.0);
        assert_eq!(local_drag_target(&ctx, state, moved), Some(pos2(808.0, 453.0)));
    }

    #[test]
    fn pen_drag_target_tracks_screen_motion_when_local_position_stays_fixed() {
        let ctx = egui::Context::default();
        let local = pos2(120.0, 24.0);
        let state = TitleDrag {
            offset: local.to_vec2(),
            touch: true,

            start_outer: Some(pos2(760.0, 420.0)),
            start_screen: Some(pos2(220.0, 224.0)),
            start_local: local,
            contact: None,
            native_screen: false,
        };

        // The pen remains over the same local title-bar point while the native viewport moves.
        // Its local coordinate is unchanged, but the viewport's screen origin changes.  A delta
        // based only on `local - start_local` would incorrectly keep returning the old position.
        let mut input = egui::RawInput::default();
        input.viewports.entry(ViewportId::ROOT).or_default().inner_rect = Some(Rect::from_min_size(pos2(100.0, 200.0), vec2(800.0, 600.0)));
        let mut out = ctx.run_ui(input, |_| {});
        out.textures_delta.clear();
        assert_eq!(local_drag_target(&ctx, state, local), Some(pos2(760.0, 420.0)));

        let mut input = egui::RawInput::default();
        input.viewports.entry(ViewportId::ROOT).or_default().inner_rect = Some(Rect::from_min_size(pos2(148.0, 235.0), vec2(800.0, 600.0)));
        let mut out = ctx.run_ui(input, |_| {});
        out.textures_delta.clear();
        assert_eq!(local_drag_target(&ctx, state, local), Some(pos2(808.0, 455.0)));
    }

    #[test]
    fn floating_selection_clipboard_native_events_v61() {
        let (mut app, ctx) = fixture();
        let untouched = app.session.documents()[0].revision;
        app.session.set_active(1);
        app.run("select.rect", json!({"x":10,"y":10,"width":20,"height":16})).unwrap();
        app.run("edit.fill", json!({"color":"#ee2244"})).unwrap();
        app.sync_views();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let count = app.session.active().unwrap().doc.layers.len();
        let viewport_id = ViewportId::from_hash_of("clipboard-native-child-v61");
        let mods = Modifiers { command: true, ctrl: true, ..Default::default() };
        let step = |app: &mut PhotocraftApp, events: Vec<Event>| {
            // Match eframe's immediate child route: no raw_input_hook, and winit's native
            // clipboard event replaces C/X/V Down. Image-only Paste has only V Up.
            let mut input = egui::RawInput {
                viewport_id,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0))),
                events: std::iter::once(Event::ModifiersChanged(mods)).chain(events).collect(),
                ..Default::default()
            };
            input.viewports.entry(viewport_id).or_default().focused = Some(true);
            let mut out = ctx.run_ui(input, |ui| body(app, ui, 1, true));
            out.textures_delta.clear();
        };
        step(&mut app, vec![Event::Copy]);
        let clip = app.session.clipboard.as_ref().expect("native Copy must reach edit.copy");
        assert_eq!(clip.bounds, photocraft_geom::Rect::new(10, 10, 30, 26));
        assert!(clip.surface.rgba(15, 15)[3] > 0.99);
        let release = || Event::Key { key: egui::Key::V, physical_key: None, pressed: false, repeat: false, modifiers: mods };
        // Text clipboard events paste once; the paired Up must never paste a second layer.
        step(&mut app, vec![Event::Paste("image-format-also-has-text".into()), release()]);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), count + 1);
        app.run("edit.undo", json!({})).unwrap();
        step(&mut app, vec![release()]);
        assert_eq!(app.session.active().unwrap().doc.layers.len(), count + 1, "image-only V Up must paste");
        app.run("edit.undo", json!({})).unwrap();
        app.run("layer.select", json!({"layer":layer})).unwrap();
        app.run("select.rect", json!({"x":10,"y":10,"width":20,"height":16})).unwrap();
        step(&mut app, vec![Event::Cut]);
        assert_eq!(app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().rgba(15, 15)[3], 0.0);
        assert!(app.session.clipboard.as_ref().unwrap().surface.rgba(15, 15)[3] > 0.99);
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().rgba(15, 15)[3] > 0.99);
        assert_eq!(app.session.documents()[0].revision, untouched, "another document must not be changed");
    }

    #[test]
    fn selection_clipboard_crosses_focused_detached_documents() {
        let (mut app, ctx) = fixture();
        ctx.set_embed_viewports(false);
        app.run("edit.fill", json!({"color":"#00ff00"})).unwrap();
        app.run("select.rect", json!({"x":12,"y":10,"width":16,"height":14})).unwrap();
        let source_revision = app.session.documents()[0].revision;
        let target_layers = app.session.documents()[1].doc.layers.len();
        let windows = [crate::view_cmds::float_window(&mut app, 0, 0), crate::view_cmds::float_window(&mut app, 1, 1)];
        let viewports = windows.map(|window| ViewportId::from_hash_of(("docwin", window)));
        let mods = Modifiers::COMMAND | Modifiers::CTRL;
        let frame = |app: &mut PhotocraftApp, document: usize, events| {
            let viewport_id = viewports[document];
            let mut input =
                egui::RawInput { viewport_id, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0))), events, ..Default::default() };
            for vid in viewports {
                input.viewports.entry(vid).or_default().focused = Some(vid == viewport_id);
            }
            // Immediate native viewports bypass the root raw input hook.
            let mut out = ctx.run_ui(input, |ui| body(app, ui, document, true));
            out.textures_delta.clear();
        };
        app.session.set_active(1);
        frame(&mut app, 0, vec![Event::Copy]);
        assert_eq!(app.session.active_index(), Some(0));
        let key = |pressed| Event::Key { key: egui::Key::V, physical_key: None, pressed, repeat: false, modifiers: mods };
        frame(&mut app, 1, vec![key(true)]);
        frame(&mut app, 1, vec![key(false)]);

        assert_eq!(app.session.active_index(), Some(1));
        assert_eq!(app.session.documents()[0].revision, source_revision);
        let target = app.session.active().unwrap();
        assert_eq!(target.doc.layers.len(), target_layers + 1);
        let surface = target.doc.layer(target.active_layer.unwrap()).unwrap().surface().unwrap();
        assert_eq!(surface.content_bounds(), photocraft_geom::Rect::new(12, 10, 28, 24));
        assert_eq!(surface.rgba(16, 15), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(surface.rgba(11, 15)[3], 0.0);
    }

    #[test]
    fn alt_drag_between_two_detached_canvases_uses_the_original_press_and_autoselected_layer() {
        let (mut app, ctx) = fixture();
        ctx.set_embed_viewports(false);
        let source = app.session.documents()[0].doc.id;
        app.session.set_active(0);
        app.run("layer.new.layer", json!({"name":"Red square"})).unwrap();
        app.run("select.rect", json!({"x":44,"y":34,"width":12,"height":12})).unwrap();
        app.run("edit.fill", json!({"color":"#e64343"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.run("layer.new.layer", json!({"name":"Blank layer"})).unwrap();
        let windows = [crate::view_cmds::float_window(&mut app, 0, 0), crate::view_cmds::float_window(&mut app, 1, 1)];
        let viewports = windows.map(|window| ViewportId::from_hash_of(("docwin", window)));
        let area = Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0));
        let origin = |index| vec2(if index == 0 { 0.0 } else { 600.0 }, 100.0);
        app.services.screen_viewport =
            Some(Box::new(move |_, point| (0..2).find(|&index| area.translate(origin(index)).contains(point)).map(|index| viewports[index])));
        app.ui.tool = crate::state::Tool::Move;
        app.ui.tool_options.move_auto_select = true;
        app.ui.tool_options.move_show_transform = false;
        app.ui.extras.rulers = false;
        app.sync_views();
        app.session.set_active(1);
        let frame = |app: &mut PhotocraftApp, document: usize, events| {
            let mut input = egui::RawInput { viewport_id: viewports[document], screen_rect: Some(area), focused: false, events, ..Default::default() };
            for (index, viewport) in viewports.into_iter().enumerate() {
                let info = input.viewports.entry(viewport).or_default();
                info.parent = Some(ViewportId::ROOT);
                info.inner_rect = Some(area.translate(origin(index)));
                info.native_pixels_per_point = Some(1.0);
                info.focused = Some(index == 1);
            }
            let mut out = ctx.run_ui(input, |ui| body(app, ui, document, true));
            out.textures_delta.clear();
        };
        for document in [1, 0, 1, 0] {
            frame(&mut app, document, vec![]);
        }
        let grab = area.center();
        let release = grab + origin(1) - origin(0);
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::ALT };
        let before = app.session.documents()[0].doc.clone();
        let history = app.session.documents()[0].history.past_len();
        let target_count = app.session.documents()[1].doc.layers.len();
        frame(&mut app, 0, vec![Event::PointerMoved(grab), button(grab, true), Event::PointerMoved(release), button(release, false)]);
        assert_eq!(app.session.active_index(), Some(1));
        let documents = app.session.documents();
        assert_eq!(documents[0].doc.id, source);
        assert!(std::sync::Arc::ptr_eq(&documents[0].doc, &before));
        assert_eq!(documents[0].history.past_len(), history);
        let target = &documents[1];
        assert_eq!(target.doc.layers.len(), target_count + 1);
        assert_eq!(target.doc.layer(target.active_layer.unwrap()).unwrap().name, "Red square");
    }

    #[test]
    fn floating_canvas_paints_its_document_and_keyboard_undo_uses_the_same_history() {
        let (mut app, ctx) = fixture();
        let untouched = app.session.documents()[0].revision;
        frame(&mut app, &ctx, vec![], true);
        assert_eq!(app.session.active_index(), Some(1));
        pointer(&mut app, &ctx, pos2(220.0, 200.0), None);
        pointer(&mut app, &ctx, pos2(220.0, 200.0), Some(true));
        pointer(&mut app, &ctx, pos2(280.0, 200.0), None);
        pointer(&mut app, &ctx, pos2(280.0, 200.0), Some(false));
        let pixel = |app: &PhotocraftApp| app.session.documents()[1].doc.layers[0].surface().unwrap().rgba(50, 40)[3];
        assert!(pixel(&app) > 0.5, "the floating view must actually paint the layer");
        assert_eq!(app.session.documents()[0].revision, untouched);
        frame(
            &mut app,
            &ctx,
            vec![Event::Key {
                key: egui::Key::Z,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers { command: true, ctrl: true, ..Default::default() },
            }],
            true,
        );
        assert_eq!(pixel(&app), 0.0, "Undo in the child targets the same document history");
        assert_eq!(app.session.documents()[0].revision, untouched);
    }

    #[test]
    fn floating_selection_and_docking_preserve_document_view_and_history() {
        let (mut app, ctx) = fixture();
        app.ui.tool = crate::state::Tool::RectMarquee;
        frame(&mut app, &ctx, vec![], true);
        pointer(&mut app, &ctx, pos2(200.0, 150.0), None);
        pointer(&mut app, &ctx, pos2(200.0, 150.0), Some(true));
        pointer(&mut app, &ctx, pos2(290.0, 250.0), None);
        pointer(&mut app, &ctx, pos2(290.0, 250.0), Some(false));
        assert!(app.session.documents()[1].doc.selection.is_some());
        assert!(app.session.documents()[0].doc.selection.is_none());
        let id = crate::view_cmds::float_window(&mut app, 1, 0);
        app.ui.views[1].zoom = 2.5;
        let doc = app.session.documents()[1].doc.id;
        let revision = app.session.documents()[1].revision;
        dock(&mut app, id, &ctx);
        assert!(app.ui.windows.is_empty());
        assert_eq!(app.session.active_index(), Some(1));
        assert_eq!(app.ui.views[1].zoom, 2.5);
        assert_eq!(app.session.documents()[1].doc.id, doc);
        assert_eq!(app.session.documents()[1].revision, revision);
        app.run("edit.undo", json!({})).unwrap();
        assert!(app.session.documents()[1].doc.selection.is_none());
    }

    #[test]
    fn unfocused_or_modal_floating_canvas_does_not_change_the_active_document() {
        let (mut app, ctx) = fixture();
        frame(&mut app, &ctx, vec![], false);
        assert_eq!(app.session.active_index(), Some(0));
        crate::camera_raw_ui::open(&mut app, &ctx).unwrap();
        frame(&mut app, &ctx, vec![], true);
        assert_eq!(app.session.active_index(), Some(0));
    }

    #[test]
    fn title_drag_docks_only_on_release_inside_the_target_and_obeys_preference() {
        let (mut app, ctx) = fixture();
        let id = crate::view_cmds::float_window(&mut app, 1, 0);
        let w = app.ui.windows[0].clone();
        let target = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 36.0));
        ctx.data_mut(|d| d.insert_temp(target_id(), Some(target)));
        assert!(!can_dock(&ctx, Some(pos2(100.0, 20.0)), false));
        assert!(!can_dock(&ctx, Some(pos2(100.0, 100.0)), true));
        let mut run = |events: Vec<Event>, origin: Pos2| {
            let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0))), events, ..Default::default() };
            input.viewports.entry(ViewportId::ROOT).or_default().inner_rect = Some(Rect::from_min_size(origin, vec2(500.0, 400.0)));
            let mut result = false;
            let mut out = ctx.run_ui(input, |ui| {
                result = header(&mut app, ui, &w, "Test document");
            });
            out.textures_delta.clear();
            result
        };
        let button = |p, pressed| Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        run(vec![Event::PointerMoved(pos2(100.0, 15.0))], pos2(0.0, 100.0));
        assert!(!run(vec![button(pos2(100.0, 15.0), true)], pos2(0.0, 100.0)));
        assert!(!run(vec![Event::PointerMoved(pos2(100.0, -80.0))], pos2(0.0, 100.0)));
        assert!(run(vec![button(pos2(100.0, -80.0), false)], pos2(0.0, 100.0)));
        dock(&mut app, id, &ctx);
        assert!(app.ui.windows.is_empty());
    }

    #[test]
    fn floating_close_button_closes_the_document_instead_of_docking_it() {
        let (mut app, ctx) = fixture();
        let id = crate::view_cmds::float_window(&mut app, 1, 0);
        let w = app.ui.windows[0].clone();
        let close = pos2(480.0, 14.0);
        let mut frame = |events: Vec<Event>| {
            let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0))), events, ..Default::default() };
            input.viewports.entry(ViewportId::ROOT).or_default().inner_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(500.0, 400.0)));
            let mut out = ctx.run_ui(input, |ui| {
                let _ = header(&mut app, ui, &w, "Test document");
            });
            out.textures_delta.clear();
        };
        frame(vec![Event::PointerMoved(close)]);
        frame(vec![Event::PointerButton { pos: close, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        frame(vec![Event::PointerButton { pos: close, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
        assert_eq!(app.session.documents().len(), 1, "the caption X closes the document");
        flush_document_closes(&app, &ctx);
        assert!(!app.ui.windows.iter().any(|window| window.id == id));
        assert!(ctx.data(|d| d.get_temp::<Vec<(u64, u64)>>(pending_close_id())).is_none());
    }

    #[test]
    fn tab_release_position_is_consumed_once_as_the_viewport_origin() {
        let ctx = egui::Context::default();
        let origin = pos2(240.0, 120.0);
        let local = pos2(185.0, 75.0);
        let mut input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(900.0, 700.0))), ..Default::default() };
        input.viewports.entry(ViewportId::ROOT).or_default().inner_rect = Some(Rect::from_min_size(origin, vec2(900.0, 700.0)));
        let mut out = ctx.run_ui(input, |_ui| {
            set_initial_position(&ctx, 17, local);
            assert_eq!(ctx.data(|d| d.get_temp::<Pos2>(initial_position_id(17))), Some(origin + local.to_vec2()));
        });
        out.textures_delta.clear();
        let consumed = ctx.data(|d| d.get_temp::<Pos2>(initial_position_id(17)));
        ctx.data_mut(|d| d.remove::<Pos2>(initial_position_id(17)));
        assert_eq!(consumed, Some(origin + local.to_vec2()));
        assert_eq!(ctx.data(|d| d.get_temp::<Pos2>(initial_position_id(17))), None, "a stale release point must not reposition a later window");
    }

    // Reproduce the native callback's ordering: header/body reserve a press, then the resize
    // helper establishes its contact before the next frame's release. A header/body-only
    // fixture cannot test release ownership because it omits the actual gesture owner.
    fn detached_resize_frame(app: &mut PhotocraftApp, ctx: &egui::Context, window: &DocWindow, events: Vec<Event>) -> Vec<ViewportCommand> {
        let viewport = ViewportId::from_hash_of(("docwin", window.id));
        let size = vec2(500.0, 400.0);
        let mut input = egui::RawInput { viewport_id: viewport, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() };
        let info = input.viewports.entry(viewport).or_default();
        info.inner_rect = Some(Rect::from_min_size(pos2(300.0, 200.0), size));
        info.outer_rect = info.inner_rect;
        info.focused = Some(true);
        info.native_pixels_per_point = Some(1.0);
        let mut out = ctx.run_ui(input, |ui| {
            let frame = detached_frame(ui.style(), crate::theme::Tokens::get(ui.ctx()));
            let card_rect = ui.max_rect().shrink(FRAME_GUTTER);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(card_rect));
            frame.show(&mut child, |ui| {
                header(app, ui, window, "Test document");
                body(app, ui, window.document, true);
            });
            crate::titlebar::resize_zones_for_document(
                ui,
                &mut app.services.contact_screen_pos,
                &mut app.services.screen_cursor_pos,
                FRAME_GUTTER,
                vec2(crate::titlebar::MIN_INNER_WIDTH, crate::titlebar::MIN_INNER_HEIGHT),
            );
        });
        out.textures_delta.clear();
        out.viewport_output.remove(&viewport).map_or_else(Vec::new, |v| v.commands)
    }

    #[test]
    fn visible_upper_corner_press_resizes_without_starting_a_title_drag_or_caption_click() {
        let (mut app, ctx) = fixture();
        let id = crate::view_cmds::float_window(&mut app, 1, 0);
        let window = app.ui.windows.iter().find(|w| w.id == id).unwrap().clone();
        let viewport = ViewportId::from_hash_of(("docwin", id));
        detached_resize_frame(&mut app, &ctx, &window, vec![]);
        // In the real compact frame the title content starts at 11pt. The 24pt corner target
        // overlaps the title/caption, so raw title and close/maximize handlers must defer.
        for press in [pos2(20.0, 20.0), pos2(480.0, 20.0)] {
            detached_resize_frame(
                &mut app,
                &ctx,
                &window,
                vec![
                    contact(1, egui::TouchPhase::Start, press),
                    Event::PointerButton { pos: press, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
                ],
            );
            assert!(ctx.data(|d| d.get_temp::<TitleDrag>(title_drag_id(id))).is_none(), "{press:?}");
            assert!(crate::titlebar::resize_active(&ctx, viewport), "the native callback establishes resize ownership");
            let out = detached_resize_frame(
                &mut app,
                &ctx,
                &window,
                vec![
                    contact(1, egui::TouchPhase::End, press),
                    Event::PointerButton { pos: press, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
                ],
            );
            assert!(!out.iter().any(|c| matches!(c, ViewportCommand::OuterPosition(_))), "{out:?}");
            assert_eq!(app.session.documents().len(), 2, "a corner release must not click Close");
            assert!(!crate::titlebar::resize_active(&ctx, viewport));
        }
        // A normal interior title press retains the established direct pen drag path.
        title_frame(&mut app, &ctx, &window, Some(pos2(300.0, 200.0)), vec![contact(1, egui::TouchPhase::Start, pos2(80.0, 20.0))]);
        assert!(ctx.data(|d| d.get_temp::<TitleDrag>(title_drag_id(id))).is_some());
    }

    #[test]
    fn edge_press_is_reserved_before_the_resize_widget_runs_and_cannot_start_a_brush_stroke() {
        let (mut app, ctx) = fixture();
        let id = crate::view_cmds::float_window(&mut app, 1, 0);
        let window = app.ui.windows.iter().find(|w| w.id == id).unwrap().clone();
        let viewport = ViewportId::from_hash_of(("docwin", id));
        detached_resize_frame(&mut app, &ctx, &window, vec![]);
        let revision = app.session.documents()[1].revision;
        for tool in [crate::state::Tool::Brush, crate::state::Tool::Hand, crate::state::Tool::RotateView] {
            app.ui.tool = tool;
            let before = app.ui.views[1].clone();
            let edge = pos2(480.0, 380.0);
            detached_resize_frame(
                &mut app,
                &ctx,
                &window,
                vec![
                    contact(1, egui::TouchPhase::Start, edge),
                    Event::PointerMoved(edge),
                    Event::PointerButton { pos: edge, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE },
                ],
            );
            assert!(app.drag.is_none(), "{tool:?} must not own an edge press");
            assert!(crate::titlebar::resize_active(&ctx, viewport), "the frame helper owns the pen until release");
            detached_resize_frame(
                &mut app,
                &ctx,
                &window,
                vec![
                    contact(1, egui::TouchPhase::End, edge),
                    Event::PointerButton { pos: edge, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE },
                ],
            );
            assert!(!crate::titlebar::resize_active(&ctx, viewport));
            assert_eq!(app.session.documents()[1].revision, revision, "{tool:?} must not edit the document");
            assert_eq!(app.ui.views[1].center, before.center, "{tool:?} must not pan");
            assert_eq!(app.ui.views[1].rotation, before.rotation, "{tool:?} must not rotate");
        }
    }
}
