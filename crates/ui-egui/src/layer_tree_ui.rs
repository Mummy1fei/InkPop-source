//! Layers panel tree: the rows in display order (top of the stack first, each group directly
//! above its contents) and the groups' disclosure triangles (#126).
//!
//! Clicking a triangle opens or closes the group; ⌥-click sets every group in the document to
//! the new state (Photoshop). The state is the group's `expanded` flag, document data saved in
//! `.pcraft` and as the PSD section divider's open/closed folder type, toggled through
//! `layer.setExpanded` (a view change: no history step).

use egui::{Rect, Sense, Shape, Stroke, pos2, vec2};
use photocraft_doc::{Document, Layer, LayerContent};
use serde_json::{Value, json};

use crate::theme::Tokens;

/// Width the disclosure triangle takes in a group row.
pub const TRIANGLE_W: f32 = 14.0;

/// Rows as the Layers panel lists them: topmost layer first, a group directly above its
/// children, and the contents of closed groups left out (unless `show_all`, used while the
/// panel is filtered by kind so matches inside closed groups still show).
pub fn display_rows(doc: &Document, show_all: bool) -> Vec<(usize, &Layer)> {
    fn rec<'a>(layers: &'a [Layer], depth: usize, show_all: bool, out: &mut Vec<(usize, &'a Layer)>) {
        for l in layers.iter().rev() {
            out.push((depth, l));
            if let LayerContent::Group(g) = &l.content
                && (g.expanded || show_all)
            {
                rec(&g.children, depth + 1, show_all, out);
            }
        }
    }
    let mut out = Vec::new();
    rec(&doc.layers, 0, show_all, &mut out);
    out
}

/// Filter matches plus their group ancestors. Traversal ignores disclosure only
/// for this view and never writes expanded/visible/selection state to the document.
/// Rows retain real LayerIds, not indices in the shortened list.
pub fn filtered_rows(doc: &Document, matches: impl Fn(&Layer) -> bool) -> Vec<(usize, &Layer)> {
    let mut pending: Vec<_> = doc.layers.iter().map(|layer| (0, layer, None::<usize>)).collect();
    let mut rows = Vec::new();
    let mut parents = Vec::new();
    let mut keep = Vec::new();
    while let Some((depth, layer, parent)) = pending.pop() {
        let index = rows.len();
        rows.push((depth, layer));
        parents.push(parent);
        keep.push(matches(layer));
        if depth < photocraft_doc::MAX_GROUP_DEPTH
            && let Some(children) = layer.children()
        {
            pending.extend(children.iter().map(|child| (depth + 1, child, Some(index))));
        }
    }
    for index in (0..rows.len()).rev() {
        if keep.get(index).copied().unwrap_or(false)
            && let Some(parent) = parents.get(index).copied().flatten()
            && let Some(show) = keep.get_mut(parent)
        {
            *show = true;
        }
    }
    rows.into_iter().zip(keep).filter_map(|(row, show)| show.then_some(row)).collect()
}

/// The hit area of a group row's triangle starting at `x`.
pub fn triangle_rect(row: Rect, x: f32) -> Rect {
    Rect::from_min_size(pos2(x - 2.0, row.top()), vec2(TRIANGLE_W + 2.0, row.height()))
}

/// Paint and handle a group row's disclosure triangle at `*x` (advancing it). Returns true when
/// it was clicked this frame, so the row doesn't also treat the click as a selection.
pub fn disclosure(ui: &mut egui::Ui, row: Rect, x: &mut f32, l: &Layer, actions: &mut Vec<(String, Value)>) -> bool {
    disclosure_filtered(ui, row, x, l, false, actions)
}

/// During filtering ancestor folders appear expanded only in this panel. Their
/// disclosure cannot silently alter the stored group state while matches show.
pub fn disclosure_filtered(ui: &mut egui::Ui, row: Rect, x: &mut f32, l: &Layer, filtered: bool, actions: &mut Vec<(String, Value)>) -> bool {
    let LayerContent::Group(g) = &l.content else { return false };
    let hit = triangle_rect(row, *x);
    let resp = ui.interact(hit, ui.id().with(("disclosure", l.id.0)), if filtered { Sense::hover() } else { Sense::click() });
    let t = Tokens::get(ui.ctx());
    let c = pos2(*x + TRIANGLE_W / 2.0 - 1.0, row.center().y);
    let s = 3.5;
    let pts = if g.expanded || filtered {
        vec![pos2(c.x - s, c.y - s * 0.6), pos2(c.x + s, c.y - s * 0.6), pos2(c.x, c.y + s * 0.6)]
    } else {
        vec![pos2(c.x - s * 0.6, c.y - s), pos2(c.x + s * 0.6, c.y), pos2(c.x - s * 0.6, c.y + s)]
    };
    let color = if resp.hovered() { t.text } else { t.icon };
    ui.painter().add(Shape::convex_polygon(pts, color, Stroke::NONE));
    *x += TRIANGLE_W;
    if filtered {
        resp.on_hover_text(tl!("Matching layers are shown inside collapsed groups"));
        return false;
    }
    let clicked = resp.clicked();
    if clicked {
        let all = ui.input(|i| i.modifiers.alt);
        actions.push(("layer.setExpanded".into(), json!({"layer": l.id.0, "expanded": !g.expanded, "all": all})));
    }
    let (verb, name) = (if g.expanded { tl!("Collapse") } else { tl!("Expand") }, l.name.clone());
    let tip = if g.expanded { tl!("Collapse group  ({key}-click: all groups)") } else { tl!("Expand group  ({key}-click: all groups)") };
    let resp = resp.on_hover_text(crate::i18n::fmt(tip, &[("key", &crate::shortcuts::pretty("Alt"))]));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{verb} group {name}")));
    clicked
}

#[cfg(test)]
mod tests;
