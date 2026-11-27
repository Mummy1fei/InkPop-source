//! Move overlays resolve the current pickup/selection, never a preceding layer's frame.
use super::*;
use photocraft_doc::LayerId;

/// `Some(None)` means Ctrl pickup over empty canvas: no previous-layer overlay belongs here.
/// Pixel selections and existing transforms/gestures retain their original owner.
fn ctrl_pick(app: &PhotocraftApp, ctx: &egui::Context, xf: &ViewXform) -> Option<Option<LayerId>> {
    ctrl_pick_at(app, ctx, xf, ctx.input(|input| input.pointer.hover_pos()))
}

fn ctrl_pick_at(app: &PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, point: Option<Pos2>) -> Option<Option<LayerId>> {
    if app.drag.is_some()
        || app.ui.transform.is_some()
        || app.session.active()?.doc.selection.is_some()
        || crate::hold_keys::effective_tool(app, ctx) != Tool::Move
        || !ctx.input(|input| crate::hold_keys::ctrl_move_requested(app, input.modifiers))
        || crate::hold_keys::keyboard_protected(app, ctx)
    {
        return None;
    }
    let point = point.filter(|point| xf.rect.contains(*point));
    let Some(point) = point else { return Some(None) };
    let [x, y] = xf.to_doc(point);
    if !x.is_finite() || !y.is_finite() {
        return Some(None);
    }
    let doc = &app.session.active()?.doc;
    let hit = photocraft_engine::pick_cmds::layers_at(doc, x.floor() as i32, y.floor() as i32).first().copied();
    Some(hit.map(|id| {
        if app.ui.tool_options.move_target == "group" {
            doc.path_of(id).and_then(|path| path.first().and_then(|index| doc.layer_at(std::slice::from_ref(index)))).map_or(id, |layer| layer.id)
        } else {
            id
        }
    }))
}

/// A read-only hover candidate may have an edge, but its predecessor's handles must neither
/// appear nor intercept this press before Auto-Select chooses the new layer.
pub(super) fn controls_match_pick(app: &PhotocraftApp, ctx: &egui::Context, xf: &ViewXform) -> bool {
    controls_match(app, ctrl_pick(app, ctx, xf))
}

pub(super) fn controls_match_pick_at(app: &PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, point: Pos2) -> bool {
    controls_match(app, ctrl_pick_at(app, ctx, xf, Some(point)))
}

fn controls_match(app: &PhotocraftApp, pick: Option<Option<LayerId>>) -> bool {
    match pick {
        Some(Some(id)) => app.session.active().is_some_and(|state| state.is_layer_selected(id)),
        Some(None) => false,
        None => true,
    }
}

pub(super) fn layer_edge_bounds(app: &PhotocraftApp, ctx: &egui::Context, xf: &ViewXform, idx: usize, primary: bool) -> Option<DRect> {
    let state = app.session.documents().get(idx)?;
    if primary && app.ui.transform.is_some() {
        return None; // The real transform overlay already owns the current frame/handles.
    }
    let moving = primary && app.drag.as_ref().is_some_and(|drag| drag.tool == Tool::Move);
    if moving && app.drag.as_ref().is_some_and(|drag| drag.sel_move == Some(true)) {
        let bounds = state.doc.selection.as_ref()?.content_bounds();
        let (dx, dy) = crate::canvas::selection_shown_offset(app).unwrap_or((0, 0));
        return (!bounds.is_empty()).then(|| bounds.translate(dx, dy));
    }
    let selected = state.selected_layers();
    let targets = if primary {
        match ctrl_pick(app, ctx, xf) {
            Some(Some(id)) if state.is_layer_selected(id) && selected.len() > 1 => selected,
            Some(Some(id)) => vec![id],
            Some(None) => return None,
            None if moving || crate::hold_keys::effective_tool(app, ctx) == Tool::Move => selected,
            None => state.active_layer.into_iter().collect(),
        }
    } else {
        state.active_layer.into_iter().collect()
    };
    let targets = photocraft_engine::layer_multi_cmds::move_targets(&state.doc, &targets);
    let bounds = targets
        .into_iter()
        .filter_map(|id| state.doc.layer(id))
        .fold(DRect::EMPTY, |bounds, layer| bounds.union(&photocraft_engine::transform_cmds::transform_bounds(&state.doc, layer)));
    if bounds.is_empty() {
        return None;
    }
    let offset = app.drag.as_ref().filter(|_| moving).map(|drag| {
        let end = drag.points.last().map_or(drag.start, |point| [point[0], point[1]]);
        ((end[0] - drag.start[0]).round().clamp(-1e7, 1e7) as i32, (end[1] - drag.start[1]).round().clamp(-1e7, 1e7) as i32)
    });
    Some(offset.map_or(bounds, |(dx, dy)| bounds.translate(dx, dy)))
}

pub(super) fn draw_layer_edges(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, idx: usize, primary: bool) {
    if app.ui.view.shows(app.ui.view.show.layer_edges)
        && let Some(bounds) = layer_edge_bounds(app, painter.ctx(), xf, idx, primary)
    {
        painter.add(egui::Shape::closed_line(xf.doc_corners(bounds).to_vec(), Stroke::new(1.0, Color32::from_rgb(0x2d, 0x8c, 0xeb))));
    }
}

#[cfg(test)]
#[path = "ctrl_move_outline_tests_v75.rs"]
mod tests;
