//! Dragging layers to another document (#589), as in Photoshop: drag layers from the Layers panel,
//! or with the Move tool from the canvas, onto another document's tab. The tab shows its document
//! while the pointer is over it (spring-loaded), and releasing over that document's canvas copies
//! the layers there with `layer.copyToDocument` (one history step in the destination).
//!
//! Where the copies land: a Move-tool drag puts the pixel that was grabbed under the pointer, and
//! a Layers panel drag centres the layers on it. With ⇧, or when released on the tab itself, they
//! keep their place when both documents have the same pixel size and are centred otherwise
//! (Photoshop's ⇧-drag). With Window › Arrange tiling several documents, releasing over another
//! document's tile drops there. An Alt-drag can also drop directly onto another open canvas,
//! including a separate native document window.
//!
//! An Alt-drag with an active selection carries only the selected pixels of the source layer.
//! The floating piece previews a local duplicate and is discarded when the drag leaves the source.

use photocraft_doc::{DocId, LayerId};
use serde_json::json;

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;

/// The layers being dragged and where from.
#[derive(Clone, Debug)]
struct Transfer {
    source: DocId,
    layers: Vec<u64>,
    selected_pixels: bool,
    /// The document point a Move-tool drag grabbed (`None` for a Layers panel drag).
    grab: Option<[f64; 2]>,
    /// What the ghost by the pointer says.
    label: String,
}

fn key() -> egui::Id {
    egui::Id::new("pc-layer-transfer")
}

fn get(ctx: &egui::Context) -> Option<Transfer> {
    ctx.data(|d| d.get_temp::<Transfer>(key()))
}

fn clear(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove::<Transfer>(key()));
}

/// Escape/lost focus in the Layers panel cancels its transfer payload too. A
/// canvas Move gesture retains its separate owner and cancellation semantics.
pub(crate) fn cancel_panel_drag(ctx: &egui::Context) {
    if get(ctx).is_some_and(|transfer| transfer.grab.is_none()) {
        clear(ctx);
    }
}

/// The layers `id` stands for: the selection when it is one of the selected layers, else itself.
fn begin(app: &PhotocraftApp, ctx: &egui::Context, id: LayerId, grab: Option<[f64; 2]>) -> bool {
    let Some(st) = app.session.active() else { return false };
    let layers = if st.is_layer_selected(id) { st.selected_layers() } else { vec![id] };
    let label = match layers.as_slice() {
        [one] => st.doc.layer(*one).map_or_else(String::new, |l| l.name.clone()),
        many => crate::i18n::trn(crate::i18n::current(), many.len() as u64, "{n} layer", "{n} layers"),
    };
    let t = Transfer { source: st.doc.id, layers: layers.iter().map(|l| l.0).collect(), selected_pixels: false, grab, label };
    ctx.data_mut(|d| d.insert_temp(key(), t));
    true
}

/// A Layers panel row drag began on `layer`: it can be dropped on another document.
pub fn begin_from_panel(app: &PhotocraftApp, ctx: &egui::Context, layer: LayerId) {
    begin(app, ctx, layer, None);
}

/// A Move-tool drag of whole layers (not a selection or a floating piece) under way.
fn move_drag(app: &PhotocraftApp) -> Option<[f64; 2]> {
    let d = app.drag.as_ref().filter(|d| d.tool == Tool::Move && d.sel_move.is_none())?;
    let st = app.session.active()?;
    let plain = app.ui.transform.is_none() && app.guide_drag.is_none() && st.doc.selection.is_none() && st.floating.is_none();
    plain.then_some(d.start)
}

fn selected_drag(app: &PhotocraftApp) -> Option<[f64; 2]> {
    let drag = app.drag.as_ref().filter(|drag| drag.tool == Tool::Move && drag.sel_move == Some(true) && drag.modifiers.alt)?;
    let state = app.session.active()?;
    photocraft_engine::float_cmds::floating(state).filter(|piece| piece.parts.is_copy() && piece.offset == (0, 0))?;
    Some(drag.start)
}

fn begin_from_canvas(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    let Some(state) = app.session.active() else { return false };
    if crate::move_mods::duplicate_deferred(app)
        && let Some(grab) = selected_drag(app)
        && let Some(piece) = photocraft_engine::float_cmds::floating(state)
    {
        let label = state.doc.layer(piece.layer).map_or_else(String::new, |layer| layer.name.clone());
        let transfer = Transfer { source: state.doc.id, layers: vec![piece.layer.0], selected_pixels: true, grab: Some(grab), label };
        ctx.data_mut(|data| data.insert_temp(key(), transfer));
        return true;
    }
    move_drag(app).zip(state.active_layer).is_some_and(|(grab, layer)| begin(app, ctx, layer, Some(grab)))
}

#[derive(Clone, Copy)]
struct CanvasTarget {
    viewport: egui::ViewportId,
    layer: egui::LayerId,
    document: DocId,
    xf: ViewXform,
    frame: u64,
}

fn targets_key() -> egui::Id {
    key().with("visible-canvases")
}

fn source_key() -> egui::Id {
    key().with("canvas-source")
}

fn trace(message: std::fmt::Arguments<'_>) {
    if std::env::var_os("PHOTOCRAFT_TRACE_LAYER_TRANSFER").is_some() {
        eprintln!("LAYER_TRANSFER {message}");
    }
}

/// Record the actual press before the pointer can leave an owned HWND. egui's
/// drag threshold can be crossed after the source widget has lost hover.
pub(crate) fn canvas_move_press(app: &PhotocraftApp, response: &egui::Response) -> Option<(egui::Pos2, egui::Modifiers)> {
    if app.drag.is_some() || app.session.documents().len() < 2 || !response.enabled() || app.ui.transform.is_some() || app.distort.active() {
        return None;
    }
    let state = app.session.active()?;
    if state.active_layer.is_none() || state.floating.is_some() {
        return None;
    }
    let (pos, modifiers) = response.ctx.input(|input| {
        input.events.iter().find_map(|event| match event {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers } => Some((*pos, *modifiers)),
            _ => None,
        })
    })?;
    let held = crate::workspace_ui::sticky_mods(app, modifiers);
    let layer = response.ctx.layer_id_at(pos);
    trace(format_args!("press {pos:?} alt={} layer {layer:?} canvas {:?} {:?}", held.alt, response.layer_id, response.interact_rect));
    (held.alt && !held.ctrl && !held.command && response.interact_rect.contains(pos) && layer == Some(response.layer_id)).then_some((pos, modifiers))
}

/// Use each visible document's own view, including the inactive canvas behind a floating one.
pub(crate) fn record_canvas(app: &PhotocraftApp, ui: &egui::Ui, document: DocId, xf: ViewXform) {
    if app.session.documents().len() < 2 || !ui.is_enabled() || !xf.rect.is_positive() {
        return;
    }
    let ctx = ui.ctx();
    let target = CanvasTarget { viewport: ctx.viewport_id(), layer: ui.layer_id(), document, xf, frame: app.frame };
    ctx.data_mut(|data| {
        let targets = data.get_temp_mut_or_default::<Vec<CanvasTarget>>(targets_key());
        targets.retain(|old| app.frame.saturating_sub(old.frame) <= 1 && (old.viewport, old.document) != (target.viewport, document));
        targets.push(target);
    });
}

pub(crate) fn arm_canvas_move(app: &mut PhotocraftApp, ctx: &egui::Context, source: DocId) {
    if app.session.documents().len() >= 2
        && (move_drag(app).is_some() || selected_drag(app).is_some())
        && app.drag.as_ref().is_some_and(|drag| drag.modifiers.alt && !drag.modifiers.ctrl && !drag.modifiers.command)
    {
        crate::move_mods::defer_duplicate(app);
        if crate::move_mods::duplicate_deferred(app) {
            let viewport = ctx.viewport_id();
            ctx.data_mut(|data| data.insert_temp(source_key(), (viewport, source)));
            trace(format_args!("armed {viewport:?} {source:?} frame {}", app.frame));
        }
    }
}

fn owns_canvas_move(app: &PhotocraftApp, ctx: &egui::Context, source: DocId) -> bool {
    let viewport = ctx.viewport_id();
    crate::move_mods::duplicate_deferred(app) && ctx.data(|data| data.get_temp::<(egui::ViewportId, DocId)>(source_key())) == Some((viewport, source))
}

fn end_canvas_move(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let source = ctx.data_mut(|data| {
        let source = data.get_temp::<(egui::ViewportId, DocId)>(source_key());
        data.remove::<(egui::ViewportId, DocId)>(source_key());
        source
    });
    if selected_drag(app).is_some()
        && let Some((_, source)) = source
        && let Some(state) = app.session.active_mut().filter(|state| state.doc.id == source)
    {
        state.floating = None;
    }
    app.drag = None;
    app.move_preview = None;
    crate::move_mods::abandon(app);
}

pub(crate) fn cancel_canvas_move(app: &mut PhotocraftApp, ctx: &egui::Context, source: DocId) {
    if owns_canvas_move(app, ctx, source)
        && ctx.input(|input| {
            input.key_pressed(egui::Key::Escape)
                || (!input.pointer.primary_released()
                    && (!input.pointer.primary_down() || input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false)))))
        })
    {
        trace(format_args!(
            "cancelled {source:?} frame {} {:?}",
            app.frame,
            ctx.input(|i| (i.pointer.primary_down(), i.pointer.primary_released(), i.focused))
        ));
        end_canvas_move(app, ctx);
    }
}

fn viewport_point(app: &mut PhotocraftApp, ctx: &egui::Context, local: egui::Pos2) -> Option<(egui::ViewportId, egui::Pos2)> {
    let source = ctx.viewport_id();
    let Some(hit) = &mut app.services.screen_viewport else { return Some((source, local)) };
    let origin = ctx.input(|input| input.viewport().inner_rect.map(|rect| rect.min))?;
    let screen = (origin + local.to_vec2()) * ctx.pixels_per_point();
    let viewport = hit(ctx, screen);
    trace(format_args!("source {source:?} local {local:?} origin {origin:?} screen {screen:?} found {viewport:?}"));
    let viewport = viewport?;
    if viewport == source {
        return Some((source, local));
    }
    let zoom = ctx.zoom_factor();
    let result = ctx.input(|input| {
        let info = input.raw.viewports.get(&viewport)?;
        if info.minimized == Some(true) || info.visible() == Some(false) || info.close_requested() {
            return None;
        }
        let scale = info.native_pixels_per_point? * zoom;
        let origin = info.inner_rect?.min;
        (scale.is_finite() && scale > 0.0 && origin.is_finite()).then(|| (viewport, (screen / scale - origin).to_pos2()))
    });
    trace(format_args!("target point {result:?}"));
    result
}

fn live_target(app: &PhotocraftApp, ctx: &egui::Context, canvas: CanvasTarget) -> Option<usize> {
    let index = app.session.documents().iter().position(|state| state.doc.id == canvas.document)?;
    let window = app.ui.windows.iter().find(|window| window.open && window.document == index);
    let known = if canvas.viewport == egui::ViewportId::ROOT {
        ctx.embed_viewports() || window.is_none()
    } else {
        window.is_some_and(|window| egui::ViewportId::from_hash_of(("docwin", window.id)) == canvas.viewport)
    };
    (known && app.frame.saturating_sub(canvas.frame) <= 1).then_some(index)
}

/// The source still owns the captured button release. Resolve the destination in screen
/// coordinates, then perform one destination edit before the normal Move tool can commit.
pub(crate) fn release_canvas_move(app: &mut PhotocraftApp, ctx: &egui::Context, source: DocId, release: Option<egui::Pos2>) -> bool {
    if !owns_canvas_move(app, ctx, source) {
        return false;
    }
    let current = ctx.viewport_id();
    let point = release.and_then(|local| viewport_point(app, ctx, local));
    trace(format_args!("release {release:?} point {point:?} frame {}", app.frame));
    let hit = point.and_then(|(viewport, local)| {
        let canvases = ctx.data(|data| data.get_temp::<Vec<CanvasTarget>>(targets_key())).unwrap_or_default();
        trace(format_args!("canvases {:?}", canvases.iter().map(|c| (c.viewport, c.document, c.xf.rect, c.frame)).collect::<Vec<_>>()));
        let layer = (viewport == current).then(|| ctx.layer_id_at(local)).flatten();
        canvases.into_iter().rev().find_map(|canvas| {
            if canvas.viewport != viewport || !canvas.xf.rect.contains(local) || (viewport == current && layer != Some(canvas.layer)) {
                return None;
            }
            let index = live_target(app, ctx, canvas)?;
            Some((index, canvas.document, viewport, canvas.xf.to_doc(local)))
        })
    });
    if hit.is_some_and(|(_, document, _, _)| document == source) {
        ctx.data_mut(|data| data.remove::<(egui::ViewportId, DocId)>(source_key()));
        return false;
    }
    if let Some((index, _, viewport, at)) = hit
        && app.session.active().is_some_and(|state| state.doc.id == source)
        && begin_from_canvas(app, ctx)
    {
        end_canvas_move(app, ctx);
        if drop(app, ctx, index, Some(at)) {
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    } else {
        end_canvas_move(app, ctx);
    }
    true
}

/// The pointer, while layers are dragged (or a Move drag could become a transfer): tabs only
/// hit-test it then.
pub fn pointer_if_armed(app: &PhotocraftApp, ctx: &egui::Context) -> Option<egui::Pos2> {
    if get(ctx).is_none() && move_drag(app).is_none() && !(crate::move_mods::duplicate_deferred(app) && selected_drag(app).is_some()) {
        return None;
    }
    ctx.input(|i| i.pointer.latest_pos())
}

/// The pointer is over document tab `index` while layers are dragged: a Move drag becomes a
/// transfer (the source layers stay where they were), the tab's document is shown, and a release
/// right here drops the layers into it.
pub fn over_tab(app: &mut PhotocraftApp, ctx: &egui::Context, index: usize) {
    if get(ctx).is_none() {
        if app.session.active_index() == Some(index) || !begin_from_canvas(app, ctx) {
            return;
        }
        // The Move drag ends without moving anything; an ⌥-drag's copy is taken back.
        end_canvas_move(app, ctx);
    }
    if app.session.active_index() != Some(index) {
        app.session.set_active(index);
        app.jobs.focus = None;
        // The Layers panel now lists the other document: its rows don't take the drag.
        ctx.data_mut(|d| d.remove::<u64>(egui::Id::new("layer-drag")));
    }
    if ctx.input(|i| i.pointer.any_released()) {
        drop(app, ctx, index, None);
    }
}

/// Every frame after the document area: a release over another document's canvas drops the
/// layers there; while the button is held a ghost follows the pointer.
pub fn finish(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(t) = get(ctx) else { return };
    let (pos, released, down) = ctx.input(|i| (i.pointer.latest_pos(), i.pointer.any_released(), i.pointer.primary_down()));
    let target = pos.and_then(|p| target_at(app, p));
    if !released {
        if !down {
            clear(ctx);
        } else if let Some(p) = pos
            && app.session.active().is_some_and(|st| st.doc.id != t.source)
        {
            ghost(ctx, p, &t.label);
            if target.is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::Copy);
            }
        }
        return;
    }
    if let Some((dest, at)) = target {
        drop(app, ctx, dest, at);
    }
    clear(ctx);
}

/// The document under screen point `p` that isn't the source, and the document point there
/// (`None` in a tile: tiles keep their own views).
fn target_at(app: &PhotocraftApp, p: egui::Pos2) -> Option<(usize, Option<[f64; 2]>)> {
    if !app.last_canvas_rect.contains(p) {
        return None;
    }
    match crate::canvas::arranged_cells(app, app.last_canvas_rect) {
        Some(cells) => cells.into_iter().find(|(_, r)| r.contains(p)).map(|(d, _)| (d, None)),
        None => Some((app.session.active_index()?, crate::canvas::ViewXform::active(app).map(|xf| xf.to_doc(p)))),
    }
}

/// Copy the dragged layers into document `dest`, at document point `at` (see the module docs).
fn drop(app: &mut PhotocraftApp, ctx: &egui::Context, dest: usize, at: Option<[f64; 2]>) -> bool {
    let Some(t) = get(ctx) else { return false };
    clear(ctx);
    let docs = app.session.documents();
    let Some(src) = docs.iter().position(|d| d.doc.id == t.source) else { return false };
    let Some(to) = docs.get(dest).filter(|_| dest != src) else { return false };
    let same_size = to.doc.size == docs[src].doc.size;
    let mut p = json!({"document": dest, "source": src, "layers": t.layers});
    if t.selected_pixels {
        p["selection"] = json!(true);
    }
    match at.filter(|_| !ctx.input(|i| i.modifiers.shift)) {
        Some(at) => match t.grab {
            Some(g) => p["offset"] = json!([(at[0] - g[0]).round(), (at[1] - g[1]).round()]),
            None => p["at"] = json!(at),
        },
        None => p["center"] = json!(!same_size),
    }
    if let Err(e) = app.run("layer.copyToDocument", p) {
        app.ui.status = e;
        app.ui.status_error = true;
        return false;
    }
    true
}

/// The dragged layers' name next to the pointer (also the Layers panel's reorder drag).
pub fn ghost(ctx: &egui::Context, p: egui::Pos2, label: &str) {
    let t = crate::theme::Tokens::get(ctx);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("layer-transfer-ghost")));
    let g = painter.layout_no_wrap(label.to_string(), egui::FontId::proportional(12.0), t.text);
    let r = egui::Rect::from_min_size(p + egui::vec2(12.0, -10.0), g.size() + egui::vec2(16.0, 8.0));
    painter.rect_filled(r, t.radius_sm, t.card.gamma_multiply(0.95));
    painter.rect_stroke(r, t.radius_sm, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    painter.galley(r.min + egui::vec2(8.0, 4.0), g, t.text);
}

#[cfg(test)]
mod tests;
