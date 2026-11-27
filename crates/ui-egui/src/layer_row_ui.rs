//! Layers panel row: the right-hand indicators and the name between them and the thumbnails
//! (#144).
//!
//! The indicators (lock, fx badge with its effects triangle, link, the blend-mode label) are
//! laid out first, from the row's right edge inward; the name gets the width that is left and
//! is cut with an ellipsis, so a long name never runs under an indicator at any panel width or
//! UI scale. The blend-mode label is optional: it is dropped before the name gets too narrow.
//! The rects drawn each frame are recorded ([`recorded`]) for tests and automation.

use egui::{Align2, FontId, Galley, Painter, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};
use photocraft_color::BlendMode;
use photocraft_doc::Layer;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::icons;
use crate::theme::{self, Tokens};

/// Space kept free at a row's right edge: the overlay scrollbar sits there.
pub const RIGHT_PAD: f32 = 10.0;
/// Gap between indicators, and between the name and the first indicator.
const GAP: f32 = 4.0;
/// Width of an icon indicator (lock, link).
const ICON_W: f32 = 14.0;
/// Width of the effects triangle beside the fx badge.
const TRIANGLE_W: f32 = 10.0;
/// The blend-mode label is shown only while the name keeps at least this much room.
pub const MIN_NAME_W: f32 = 64.0;

/// A right-hand row indicator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indicator {
    Lock,
    FxTriangle,
    SmartFiltersTriangle,
    Fx,
    Link,
    Blend,
}

#[path = "layer_row_ui/clipping_v89.rs"]
pub(crate) mod clipping;

/// Where one row's name and indicators went this frame (screen points).
#[derive(Clone, Debug)]
pub struct RowRects {
    pub layer: u64,
    pub row: Rect,
    pub thumbnail: Rect,
    /// The name as painted (after truncation); `None` when there was no room for it.
    pub name: Option<Rect>,
    /// Full safe text span for layer-seam clipping; excludes thumbnails/indicators.
    pub clipping_name_area: Rect,
    pub indicators: Vec<(Indicator, Rect)>,
    pub eye: Rect,
    pub visibility_icon: &'static str,
    /// The colored visibility/indentation gutter, clear of the thumbnail and name.
    pub label: Option<Rect>,
}

/// Lay out `items` (right to left, with their widths; `Blend` is optional) inside `row`,
/// leaving the name the span from `name_left`. Returns the indicator rects and the name's
/// right limit.
pub fn layout(row: Rect, name_left: f32, items: &[(Indicator, f32)]) -> (Vec<(Indicator, Rect)>, f32) {
    let mut right = row.right() - RIGHT_PAD;
    let mut out = Vec::with_capacity(items.len());
    for &(kind, w) in items {
        let w = w.max(0.0);
        // The fx badge hugs its triangle; everything else keeps a gap.
        let gap = if kind == Indicator::Fx && out.last().is_some_and(|(k, _)| *k == Indicator::FxTriangle) { 1.0 } else { GAP };
        let left = right - w;
        if kind == Indicator::Blend && left - GAP - name_left < MIN_NAME_W {
            continue;
        }
        out.push((kind, Rect::from_min_max(pos2(left, row.top()), pos2(right, row.bottom()))));
        right = left - gap;
    }
    let name_right = out.last().map_or(right, |(_, r)| r.left() - GAP);
    (out, name_right)
}

/// Width the always-shown indicators of `l` take at the row's right (blend label excluded).
pub fn reserved_width(l: &Layer) -> f32 {
    let mut w = RIGHT_PAD;
    if crate::smart_ui::has_filters(l) {
        w += 18.0 + GAP;
    }
    if l.locks.transparency || l.locks.position || l.locks.all {
        w += ICON_W + GAP;
    }
    if !l.effects.items.is_empty() {
        w += TRIANGLE_W + 1.0 + 13.0 + GAP;
    }
    if l.link_group.is_some() {
        w += ICON_W + GAP;
    }
    w
}

/// Group indentation for a row at `depth`: 14 pt a level, squeezed when the panel is so narrow
/// that it would leave the name less than [`MIN_NAME_W`] (`fixed` = everything else in the row).
pub fn indent(depth: usize, row_width: f32, fixed: f32) -> f32 {
    (depth as f32 * 14.0).min((row_width - fixed - MIN_NAME_W).max(0.0))
}

/// A single-line galley cut with "…" to `max_width` (nothing when there's no room).
pub fn truncated(painter: &Painter, text: &str, font: FontId, color: egui::Color32, italics: bool, max_width: f32) -> Option<Arc<Galley>> {
    if max_width < 8.0 {
        return None;
    }
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0.0, egui::TextFormat { font_id: font, color, italics, ..Default::default() });
    job.wrap = egui::text::TextWrapping { max_width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    Some(painter.layout_job(job))
}

/// Document label colors stay recognizable across themes.
pub fn label_color(label: photocraft_doc::LabelColor) -> Option<egui::Color32> {
    use photocraft_doc::LabelColor;
    match label {
        LabelColor::None => None,
        LabelColor::Red => Some(egui::Color32::from_rgb(230, 85, 85)),
        LabelColor::Orange => Some(egui::Color32::from_rgb(235, 155, 65)),
        LabelColor::Yellow => Some(egui::Color32::from_rgb(220, 195, 65)),
        LabelColor::Green => Some(egui::Color32::from_rgb(85, 180, 105)),
        LabelColor::Blue => Some(egui::Color32::from_rgb(80, 145, 225)),
        LabelColor::Violet => Some(egui::Color32::from_rgb(165, 105, 210)),
        LabelColor::Gray => Some(egui::Color32::from_gray(145)),
    }
}

/// Paint before the visibility icon. Fill its whole gutter and the group's indentation,
/// preserving a one-point inset for the selected row outline and space before the thumbnail.
pub fn paint_label(painter: &Painter, row: Rect, indent: f32, label: photocraft_doc::LabelColor, t: &Tokens) -> Option<Rect> {
    let color = label_color(label)?;
    let rect = Rect::from_min_max(row.min + vec2(1.0, 1.0), pos2((row.left() + 30.0 + indent).min(row.right() - 1.0), row.bottom() - 1.0));
    let radius = if t.pro { 0 } else { t.radius.round().clamp(0.0, 255.0) as u8 };
    painter.rect_filled(rect, egui::CornerRadius { nw: radius, sw: radius, ne: 0, se: 0 }, color);
    Some(rect)
}

/// The label palette is medium-bright, so use the darker theme surface/text token on it.
/// Unlabelled hidden layers retain the same readable icon contrast as visible layers.
pub fn visibility_tint(label: photocraft_doc::LabelColor, t: &Tokens) -> egui::Color32 {
    if label == photocraft_doc::LabelColor::None {
        return t.icon;
    }
    let brightness = |c: egui::Color32| u32::from(c.r()) * 2126 + u32::from(c.g()) * 7152 + u32::from(c.b()) * 722;
    if brightness(t.text) < brightness(t.canvas) { t.text } else { t.canvas }
}

/// Paint `l`'s right-hand indicators in `row` and handle the effects triangle. Returns the
/// name's right limit, the indicator rects, and whether the triangle was clicked this frame (so
/// the row doesn't also treat the click as a selection).
#[derive(Clone, Copy)]
pub struct Expansion {
    pub effects: bool,
    pub smart_filters: bool,
}

pub fn indicators(
    ui: &egui::Ui,
    painter: &Painter,
    row: Rect,
    name_left: f32,
    l: &Layer,
    expansion: Expansion,
    actions: &mut Vec<(String, Value)>,
) -> (f32, Vec<(Indicator, Rect)>, bool) {
    let t = Tokens::get(ui.ctx());
    let locked = l.locks.transparency || l.locks.position || l.locks.all;
    let fx_font = theme::semibold(11.0);
    let blend_font = FontId::proportional(10.5);
    let has_fx = !l.effects.items.is_empty();
    let show_blend = l.blend != BlendMode::Normal && l.blend != BlendMode::PassThrough;
    let fx_galley = has_fx.then(|| painter.layout_no_wrap("fx".into(), fx_font, t.text_dim));
    let blend_galley = show_blend.then(|| painter.layout_no_wrap(tl!(l.blend.label()).into(), blend_font, t.text_faint));
    let mut items = Vec::new();
    if crate::smart_ui::has_filters(l) {
        items.push((Indicator::SmartFiltersTriangle, 18.0));
    }
    if locked {
        items.push((Indicator::Lock, ICON_W));
    }
    if let Some(g) = &fx_galley {
        items.push((Indicator::FxTriangle, TRIANGLE_W));
        items.push((Indicator::Fx, g.size().x));
    }
    if l.link_group.is_some() {
        items.push((Indicator::Link, ICON_W));
    }
    if let Some(g) = &blend_galley {
        items.push((Indicator::Blend, g.size().x));
    }
    let (rects, name_right) = layout(row, name_left, &items);
    let mut clicked = false;
    let cy = row.center().y;
    for &(kind, r) in &rects {
        match kind {
            Indicator::Lock => icons::paint(ui, Rect::from_center_size(r.center(), vec2(ICON_W, ICON_W)), "lock", 12.0, t.text_faint),
            Indicator::Link => icons::paint(ui, Rect::from_center_size(r.center(), vec2(ICON_W, ICON_W)), "link", 12.0, t.text_faint),
            Indicator::Fx => {
                if let Some(g) = fx_galley.clone() {
                    painter.galley(pos2(r.left(), cy - g.size().y / 2.0), g, t.text_dim);
                }
            }
            Indicator::Blend => {
                if let Some(g) = blend_galley.clone() {
                    painter.galley(pos2(r.left(), cy - g.size().y / 2.0), g, t.text_faint);
                }
            }
            Indicator::SmartFiltersTriangle => {
                let response = ui.interact(r, ui.id().with(("smart-filter-disclosure", l.id.0)), Sense::click());
                paint_triangle(painter, r.center(), expansion.smart_filters, if response.hovered() { t.text } else { t.icon });
                if response.clicked() {
                    clicked = true;
                    actions.push((crate::smart_ui::SET_EXPANDED.into(), json!({"layer":l.id.0,"expanded":!expansion.smart_filters})));
                }
                let verb = if expansion.smart_filters { tl!("Collapse") } else { tl!("Expand") };
                response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{} {} {}", verb, tl!("Smart Filters"), l.name)));
                response.on_hover_text(format!("{} {}", verb, tl!("Smart Filters")));
            }
            Indicator::FxTriangle => {
                let resp = ui.interact(r, ui.id().with(("fx-disclosure", l.id.0)), Sense::click());
                paint_triangle(painter, r.center(), expansion.effects, if resp.hovered() { t.text } else { t.icon });
                if resp.clicked() {
                    clicked = true;
                    let all = ui.input(|i| i.modifiers.alt);
                    actions.push(("layer.setEffectsExpanded".into(), json!({"layer": l.id.0, "expanded": !expansion.effects, "all": all})));
                }
                let (verb, name) = (if expansion.effects { tl!("Collapse") } else { tl!("Expand") }, l.name.clone());
                let tip = if expansion.effects {
                    tl!("Hide the layer's effects  ({key}-click: all layers)")
                } else {
                    tl!("Show the layer's effects  ({key}-click: all layers)")
                };
                let resp = resp.on_hover_text(crate::i18n::fmt(tip, &[("key", &crate::shortcuts::pretty("Alt"))]));
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, format!("{verb} effects {name}")));
            }
        }
    }
    (name_right, rects, clicked)
}

/// A small disclosure triangle: pointing down when open, right when closed.
pub fn paint_triangle(painter: &Painter, c: Pos2, open: bool, color: egui::Color32) {
    let s = 3.0;
    let pts = if open {
        vec![pos2(c.x - s, c.y - s * 0.6), pos2(c.x + s, c.y - s * 0.6), pos2(c.x, c.y + s * 0.6)]
    } else {
        vec![pos2(c.x - s * 0.6, c.y - s), pos2(c.x + s * 0.6, c.y), pos2(c.x - s * 0.6, c.y + s)]
    };
    painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
}

/// Shared by the layer indicator and the cursor. The arrow tip is the origin.
pub(crate) fn clipping_arrow_paths() -> [[egui::Vec2; 3]; 2] {
    [[vec2(6.0, -10.0), vec2(0.0, -10.0), egui::Vec2::ZERO], [vec2(-3.0, -3.0), egui::Vec2::ZERO, vec2(3.0, -3.0)]]
}

/// A clipping arrow points down at the base layer and stays readable without a fallback glyph.
pub(crate) fn paint_clipping_arrow(painter: &Painter, c: Pos2, color: egui::Color32) {
    let stroke = Stroke::new(1.5, color);
    let tip = c + vec2(-2.0, 5.0);
    for path in clipping_arrow_paths() {
        painter.add(Shape::line(path.map(|offset| tip + offset).into(), stroke));
    }
}

/// Paint `text` left-aligned at `x`, vertically centred on `cy`, cut to end before `right`.
/// Returns the painted rect.
pub fn label(painter: &Painter, x: f32, cy: f32, right: f32, text: &str, font: FontId, color: egui::Color32) -> Option<Rect> {
    let g = truncated(painter, text, font, color, false, right - x)?;
    let r = Align2::LEFT_CENTER.anchor_size(pos2(x, cy), g.size());
    painter.galley(r.min, g, color);
    Some(r)
}

/// The Layers panel's own items at the top of its panel menu (Photoshop's flyout).
pub fn panel_menu(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui) {
    let can = app.session.is_enabled("layer.setExpanded");
    if ui.add_enabled(can, egui::Button::new(tl!("Collapse All Groups"))).clicked() {
        if let Err(e) = app.run("layer.setExpanded", json!({"all": true, "expanded": false})) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        ui.close();
    }
}

fn rects_id() -> egui::Id {
    egui::Id::new("layer-row-rects")
}

/// Start a frame's record (the Layers panel calls it before drawing its rows).
pub fn begin(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(rects_id(), Vec::<RowRects>::new()));
}

pub fn record(ctx: &egui::Context, r: RowRects) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Vec<RowRects>>(rects_id()).push(r));
}

/// The rows the Layers panel drew last frame.
pub fn recorded(ctx: &egui::Context) -> Vec<RowRects> {
    ctx.data(|d| d.get_temp::<Vec<RowRects>>(rects_id())).unwrap_or_default()
}

/// Native context menus open on the button press, including a pen barrel-button pulse.
/// Waiting for `secondary_clicked` adds the user's hold time and rejects a moving pen.
/// The release belongs to this opening gesture and must not open a second popup.
pub(crate) fn context_menu_press(response: &egui::Response) -> Option<Pos2> {
    let owns_pointer = response.contains_pointer()
        || response.is_pointer_button_down_on()
        || response.dragged_by(egui::PointerButton::Secondary)
        || response.drag_stopped_by(egui::PointerButton::Secondary);
    if !response.enabled() || !owns_pointer || response.dragged_by(egui::PointerButton::Primary) || response.drag_stopped_by(egui::PointerButton::Primary) {
        return None;
    }
    response.ctx.input(|input| {
        if input.pointer.primary_down() {
            return None;
        }
        input.events.iter().find_map(|event| match event {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Secondary, pressed: true, .. } if response.interact_rect.contains(*pos) => {
                Some(*pos)
            }
            _ => None,
        })
    })
}

#[derive(Clone, Copy)]
struct KeyboardFocus {
    viewport: egui::ViewportId,
    document: photocraft_doc::DocId,
    row: egui::Id,
}

fn keyboard_focus_id() -> egui::Id {
    egui::Id::new("layer-list-keyboard-focus")
}

/// A selection click makes Delete act on the layer list without turning the row into a text
/// field or changing the ordinary tool shortcuts. The scope is local to this document/window.
pub fn claim_keyboard_focus(ctx: &egui::Context, document: photocraft_doc::DocId, row: egui::Id) {
    let viewport = ctx.viewport_id();
    ctx.data_mut(|d| d.insert_temp(keyboard_focus_id(), KeyboardFocus { viewport, document, row }));
}

/// Clear on the next pointer press, keyboard focus traversal or native focus loss. A Layers
/// selection click will claim the scope again on release; clicks in canvas/fields will not.
pub fn update_keyboard_focus(ctx: &egui::Context) {
    let changed = ctx.input(|i| {
        i.pointer.any_pressed()
            || i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false) | egui::Event::Key { key: egui::Key::Tab, pressed: true, .. }))
    });
    if changed {
        ctx.data_mut(|d| d.remove::<KeyboardFocus>(keyboard_focus_id()));
    }
}

pub fn has_keyboard_focus(ctx: &egui::Context, document: photocraft_doc::DocId) -> bool {
    ctx.data(|d| d.get_temp::<KeyboardFocus>(keyboard_focus_id()))
        .is_some_and(|scope| scope.viewport == ctx.viewport_id() && scope.document == document && ctx.memory(|m| m.focused()).is_none_or(|id| id == scope.row))
}

// ------------------------------------------------------------------ in-place rename (#314)

/// The Layers panel's in-place rename. One at a time, like Photoshop: Enter, Tab or a click
/// anywhere else commits it, Esc cancels it, and starting another rename commits this one.
#[derive(Clone, Debug, PartialEq)]
pub struct Rename {
    pub layer: u64,
    /// The name when the rename started (unchanged text renames nothing).
    pub original: String,
    pub text: String,
    /// Focus was given to the field (only once: asking every frame kept it from ever losing focus).
    pub focused: bool,
    /// The pass its field was last drawn in.
    pub pass: u64,
}

impl Rename {
    /// The command that applies it, if it changes the name.
    pub fn commit_action(&self) -> Option<(String, Value)> {
        let name = self.text.trim();
        (!name.is_empty() && name != self.original).then(|| ("layer.setProps".to_string(), json!({"layer": self.layer, "name": name})))
    }
}

fn rename_id() -> egui::Id {
    egui::Id::new("layer-rename")
}

/// The rename in progress, if any.
pub fn rename(ctx: &egui::Context) -> Option<Rename> {
    ctx.data(|d| d.get_temp::<Rename>(rename_id()))
}

/// The layer being renamed, if any.
pub fn renaming(ctx: &egui::Context) -> Option<u64> {
    rename(ctx).map(|r| r.layer)
}

/// Is a rename field on screen (drawn this pass or the last)? It owns the keyboard meanwhile.
pub fn rename_active(ctx: &egui::Context) -> bool {
    rename(ctx).is_some_and(|r| r.pass + 1 >= ctx.cumulative_pass_nr())
}

fn store(ctx: &egui::Context, r: Option<Rename>) {
    ctx.data_mut(|d| match r {
        Some(r) => {
            d.insert_temp(rename_id(), r);
        }
        None => {
            d.remove::<Rename>(rename_id());
        }
    });
}

/// Start renaming `layer` (now called `name`). A rename open on another layer is committed: its
/// command is returned for the caller to run.
pub fn start_rename(ctx: &egui::Context, layer: u64, name: &str) -> Option<(String, Value)> {
    let previous = rename(ctx);
    if previous.as_ref().is_some_and(|r| r.layer == layer) {
        return None;
    }
    store(ctx, Some(Rename { layer, original: name.to_string(), text: name.to_string(), focused: false, pass: ctx.cumulative_pass_nr() }));
    previous.and_then(|r| r.commit_action())
}

/// End the rename in progress: commit it (its command is returned) or cancel it.
pub fn end_rename(ctx: &egui::Context, commit: bool) -> Option<(String, Value)> {
    let r = rename(ctx)?;
    store(ctx, None);
    if commit { r.commit_action() } else { None }
}

/// Draw the rename field of `layer` in `rect` when it is the one being renamed, and handle its
/// keys and focus. Returns the rename command when it was committed this frame.
pub fn rename_field(ui: &mut egui::Ui, layer: u64, rect: Rect) -> Option<(String, Value)> {
    let ctx = ui.ctx().clone();
    let mut r = rename(&ctx).filter(|r| r.layer == layer)?;
    let id = egui::Id::new(("layer-rename-field", layer));
    let te = ui.put(rect, egui::TextEdit::singleline(&mut r.text).id(id).font(FontId::proportional(12.5)));
    r.pass = ctx.cumulative_pass_nr();
    if !r.focused {
        te.request_focus();
        // Photoshop selects the whole name, so typing replaces it.
        if let Some(mut state) = egui::TextEdit::load_state(&ctx, id) {
            let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(r.text.chars().count()));
            state.cursor.set_char_range(Some(all));
            state.store(&ctx, id);
        }
        r.focused = true;
        store(&ctx, Some(r));
        return None;
    }
    let (enter, tab, esc) = ctx.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Tab), i.key_pressed(egui::Key::Escape)));
    if esc {
        store(&ctx, None);
        None
    } else if enter || tab || te.lost_focus() || !te.has_focus() {
        store(&ctx, None);
        r.commit_action()
    } else {
        store(&ctx, Some(r));
        None
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod rename_tests;

#[cfg(test)]
mod edit_tests;

#[cfg(test)]
mod appearance_tests;

#[cfg(test)]
mod context_menu_tests;
