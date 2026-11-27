//! Alt-clicking a real sibling seam toggles clipping without selecting/reordering a row.
//! Cached geometry only captures the press. Current geometry and topology authorize release.
use std::{
    collections::{BTreeMap, BTreeSet},
    hash::{Hash, Hasher},
};

use egui::{Event, Id, PointerButton, Pos2, Rect, Stroke, Ui, pos2};
use photocraft_doc::{Document, Layer};
use serde_json::{Value, json};

use super::RowRects;

fn clipping_modifier(modifiers: egui::Modifiers) -> bool {
    modifiers.alt && !modifiers.ctrl && !modifiers.command && !modifiers.mac_cmd && !modifiers.shift
}

pub(crate) fn active_command(app: &crate::PhotocraftApp) -> &'static str {
    if app.session.active().and_then(|st| st.active_layer.and_then(|id| st.doc.layer(id))).is_some_and(|layer| layer.clipped) {
        "layer.releaseClippingMask"
    } else {
        "layer.createClippingMask"
    }
}

pub(crate) fn active_label(app: &crate::PhotocraftApp) -> &'static str {
    if active_command(app) == "layer.releaseClippingMask" { "Release Clipping Mask" } else { "Create Clipping Mask" }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pair {
    upper: u64,
    lower: u64,
}

#[derive(Clone, Debug)]
struct Seam {
    pair: Pair,
    rect: Rect,
    release: bool,
}

#[derive(Clone, Debug)]
struct Press {
    doc: u64,
    pair: Pair,
    origin: Pos2,
    cancelled: bool,
}

#[derive(Clone, Default)]
struct State {
    signature: u64,
    seams: Vec<Seam>,
    press: Option<Press>,
}

pub(crate) struct Frame {
    id: Id,
    signature: u64,
    pairs: Vec<Pair>,
    releases: BTreeMap<u64, bool>,
    state: State,
    released: Option<(Pos2, bool)>,
    pub(crate) owns_pointer: bool,
}

/// Visit each layer once, without recursive traversal or repeated document path searches.
fn siblings(doc: &Document) -> BTreeMap<u64, u64> {
    let mut result = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut stack = vec![doc.layers.as_slice()];
    while let Some(layers) = stack.pop() {
        for pair in layers.windows(2) {
            if let [lower, upper] = pair {
                result.insert(upper.id.0, lower.id.0);
            }
        }
        for layer in layers {
            if seen.insert(layer.id.0)
                && let Some(children) = layer.children()
            {
                stack.push(children);
            }
        }
    }
    result
}

fn pairs(doc: &Document, rows: &[(usize, &Layer)]) -> Vec<Pair> {
    let siblings = siblings(doc);
    rows.windows(2)
        .filter_map(|rows| {
            let [(_, upper), (_, lower)] = rows else { return None };
            (siblings.get(&upper.id.0) == Some(&lower.id.0)).then_some(Pair { upper: upper.id.0, lower: lower.id.0 })
        })
        .collect()
}

fn signature(ui: &Ui, doc: &Document, rows: &[(usize, &Layer)], expansions: &[(u64, bool, bool)]) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    doc.id.0.hash(&mut hash);
    ui.ctx().viewport_id().hash(&mut hash);
    for value in [
        ui.cursor().top(),
        ui.available_width(),
        ui.clip_rect().left(),
        ui.clip_rect().right(),
        ui.clip_rect().top(),
        ui.clip_rect().bottom(),
        ui.spacing().item_spacing.y,
    ] {
        value.to_bits().hash(&mut hash);
    }
    crate::theme::Tokens::get(ui.ctx()).pro.hash(&mut hash);
    for &(depth, layer) in rows {
        (depth, layer.id.0, layer.clipped, layer.is_group(), layer.mask.is_some(), layer.vector_mask.is_some(), layer.effects.items.len()).hash(&mut hash);
        if let photocraft_doc::LayerContent::Smart(smart) = &layer.content {
            smart.smart_filters.len().hash(&mut hash);
        }
        super::reserved_width(layer).to_bits().hash(&mut hash);
    }
    expansions.hash(&mut hash);
    hash.finish()
}

pub(crate) fn begin(ui: &Ui, doc: &Document, rows: &[(usize, &Layer)], expansions: &[(u64, bool, bool)]) -> Frame {
    let id = ui.id().with(("layer-clipping-seams-v89", ui.ctx().viewport_id()));
    let signature = signature(ui, doc, rows, expansions);
    let pairs = pairs(doc, rows);
    let mut state = ui.ctx().data(|data| data.get_temp::<State>(id)).unwrap_or_default();
    let valid_geometry = state.signature == signature && ui.is_enabled() && super::renaming(ui.ctx()).is_none();
    let mut released = None;
    let mut owns_pointer = state.press.is_some();
    // Context input access holds a lock. Read the snapshot before asking which
    // egui layer owns a point; foreground menus must consume their own clicks.
    let (events, modifiers, focused, primary_down) = ui.input(|input| (input.events.clone(), input.modifiers, input.focused, input.pointer.primary_down()));
    {
        for event in &events {
            // Mouse-up completes this owned gesture. Later Alt-up/motion/focus
            // events in the same input batch cannot cancel that completed click.
            if released.is_some() {
                break;
            }
            match event {
                Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers } if state.press.is_none() => {
                    if valid_geometry
                        && ui.ctx().layer_id_at(*pos) == Some(ui.layer_id())
                        && clipping_modifier(*modifiers)
                        && let Some(seam) = state.seams.iter().find(|seam| seam.rect.contains(*pos) && pairs.contains(&seam.pair))
                    {
                        state.press = Some(Press { doc: doc.id.0, pair: seam.pair, origin: *pos, cancelled: false });
                        owns_pointer = true;
                    }
                }
                Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers } if state.press.is_some() => {
                    released = Some((*pos, clipping_modifier(*modifiers)));
                }
                Event::PointerMoved(pos) => {
                    if let Some(press) = &mut state.press
                        && (pos.distance(press.origin) > 6.0
                            || !valid_geometry
                            || ui.ctx().layer_id_at(*pos) != Some(ui.layer_id())
                            || !state.seams.iter().any(|seam| seam.pair == press.pair && seam.rect.contains(*pos)))
                    {
                        press.cancelled = true;
                    }
                }
                Event::Key { key: egui::Key::Escape, pressed: true, .. } | Event::WindowFocused(false) | Event::PointerGone => {
                    if let Some(press) = &mut state.press {
                        press.cancelled = true;
                    }
                }
                Event::ModifiersChanged(modifiers) if !clipping_modifier(*modifiers) => {
                    if let Some(press) = &mut state.press {
                        press.cancelled = true;
                    }
                }
                _ => {}
            }
        }
        if let Some(press) = &mut state.press {
            // Topology, document identity and geometry are still revalidated for
            // a completed click; only live-input cancellation ends at mouse-up.
            if press.doc != doc.id.0 || !pairs.contains(&press.pair) || !valid_geometry {
                press.cancelled = true;
            }
            // A focus loss can end a native press without delivering PointerButton up.
            if released.is_none() && (!clipping_modifier(modifiers) || !focused || !primary_down) {
                press.cancelled = true;
            }
        }
    }
    if owns_pointer {
        ui.ctx().data_mut(|data| {
            let drag = Id::new("layer-drag");
            data.remove::<u64>(drag);
            data.remove::<Vec<u64>>(drag.with("selection"));
        });
        crate::layer_transfer::cancel_panel_drag(ui.ctx());
    }
    let releases = rows.iter().map(|(_, layer)| (layer.id.0, layer.clipped)).collect();
    Frame { id, signature, pairs, releases, state, released, owns_pointer }
}

fn seams(pairs: &[Pair], rows: &[RowRects], clip: Rect, spacing: f32, releases: &BTreeMap<u64, bool>) -> Vec<Seam> {
    let by_id: BTreeMap<_, _> = rows.iter().map(|row| (row.layer, row)).collect();
    pairs
        .iter()
        .filter_map(|pair| {
            let upper = by_id.get(&pair.upper)?;
            let lower = by_id.get(&pair.lower)?;
            let left = upper.thumbnail.left().max(lower.thumbnail.left());
            let right = upper.clipping_name_area.right().min(lower.clipping_name_area.right());
            let y = lower.row.top() - spacing * 0.5;
            // Only the narrow row boundary, including the thumbnail columns. Thumbnail
            // centers, visibility, disclosure, masks and indicators keep their own clicks.
            let rect = Rect::from_min_max(pos2(left, y - 4.0), pos2(right, y + 4.0)).intersect(clip);
            if rect.width() < 16.0 || rect.height() < 4.0 || upper.row.bottom() > lower.row.top() {
                return None;
            }
            let release = *releases.get(&pair.upper)?;
            Some(Seam { pair: *pair, rect, release })
        })
        .collect()
}

impl Frame {
    pub(crate) fn finish(mut self, ui: &Ui, doc: &Document, actions: &mut Vec<(String, Value)>) {
        let rows = super::recorded(ui.ctx());
        let seams = seams(&self.pairs, &rows, ui.clip_rect(), ui.spacing().item_spacing.y, &self.releases);
        if let Some((point, modifier_held)) = self.released {
            if let Some(press) = &self.state.press
                && modifier_held
                && !press.cancelled
                && press.doc == doc.id.0
                && ui.ctx().layer_id_at(point) == Some(ui.layer_id())
                && let Some(seam) = seams.iter().find(|seam| seam.pair == press.pair && seam.rect.contains(point))
            {
                actions.push((if seam.release { "layer.releaseClippingMask" } else { "layer.createClippingMask" }.into(), json!({"layer": seam.pair.upper})));
            }
            self.state.press = None;
        } else if !ui.input(|input| input.pointer.primary_down()) {
            self.state.press = None;
        }
        let can_hover = ui.is_enabled() && super::renaming(ui.ctx()).is_none();
        let hovered = ui
            .input(|input| (can_hover && clipping_modifier(input.modifiers) && input.focused).then(|| input.pointer.hover_pos()).flatten())
            .filter(|point| ui.ctx().layer_id_at(*point) == Some(ui.layer_id()))
            .and_then(|point| seams.iter().find(|seam| seam.rect.contains(point)));
        if let Some(seam) = hovered {
            // Register the seam as its own target as well: no normal row response should
            // claim a second click while Alt remains over this narrow boundary.
            let response = ui.interact(seam.rect, self.id.with((seam.pair.upper, seam.pair.lower)), egui::Sense::hover());
            response.on_hover_text(if seam.release { tl!("Release Clipping Mask") } else { tl!("Create Clipping Mask") });
            if !crate::native_cursor::clipping(ui.ctx())
                && let Some(point) = ui.input(|input| input.pointer.hover_pos())
            {
                ui.ctx().set_cursor_image(None);
                ui.ctx().set_cursor_icon(egui::CursorIcon::None);
                let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, self.id.with("cursor")));
                for path in super::clipping_arrow_paths() {
                    let points = path.map(|offset| point + offset);
                    painter.add(egui::Shape::line(points.into(), Stroke::new(3.0, egui::Color32::BLACK)));
                    painter.add(egui::Shape::line(points.into(), Stroke::new(1.0, egui::Color32::WHITE)));
                }
            }
        }
        self.state.signature = self.signature;
        self.state.seams = seams;
        ui.ctx().data_mut(|data| data.insert_temp(self.id, self.state));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sibling_pairs_reject_group_headers_and_filtered_gaps() {
        use photocraft_doc::{ColorMode, PixelFormat, SampleType, Size};
        let mut doc = Document::new("Seams", Size::new(32, 32), ColorMode::Rgb, SampleType::U8);
        let a = Layer::raster("A", PixelFormat::RGBA8);
        let b = Layer::raster("B", PixelFormat::RGBA8);
        let c = Layer::raster("C", PixelFormat::RGBA8);
        let [aid, bid, cid] = [a.id.0, b.id.0, c.id.0];
        doc.layers = vec![a, b, c];
        assert_eq!(
            pairs(&doc, &[(0, &doc.layers[2]), (0, &doc.layers[1]), (0, &doc.layers[0])]),
            vec![Pair { upper: cid, lower: bid }, Pair { upper: bid, lower: aid }]
        );
        assert!(pairs(&doc, &[(0, &doc.layers[2]), (0, &doc.layers[0])]).is_empty());
        let child = Layer::raster("Child", PixelFormat::RGBA8);
        let child_id = child.id.0;
        doc.layers.push(Layer::group("Folder", vec![child]));
        let folder = doc.layers.last().unwrap();
        let child = folder.children().unwrap().first().unwrap();
        assert_ne!(folder.id.0, child_id);
        assert!(pairs(&doc, &[(0, folder), (1, child)]).is_empty());
    }
}
