//! Layers-panel metadata filters: panel rows only, never document visibility.
use egui::{Rect, Sense, Stroke, StrokeKind, vec2};
use photocraft_color::BlendMode;
use photocraft_doc::{LabelColor, Layer, LayerContent, LayerId, SmartSource};
use serde::{Deserialize, Serialize};

use crate::{icons, theme::Tokens};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    #[default]
    Kind,
    Name,
    Effect,
    Blend,
    Attribute,
    Color,
    SmartObject,
    Selected,
    Artboard,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Options {
    pub enabled: bool,
    pub mode: Mode,
    pub query: String,
    pub value: String,
}

impl Default for Options {
    fn default() -> Self {
        Self { enabled: true, mode: Mode::Kind, query: String::new(), value: "any".into() }
    }
}

impl Options {
    pub fn active(&self, kinds: &[String]) -> bool {
        self.enabled
            && match self.mode {
                Mode::Kind => !kinds.is_empty(),
                Mode::Name => !self.query.trim().is_empty(),
                _ => true,
            }
    }

    pub fn matches(&self, layer: &Layer, kinds: &[String], selected: &[LayerId]) -> bool {
        if !self.active(kinds) {
            return true;
        }
        match self.mode {
            Mode::Kind => kinds.iter().any(|kind| kind == kind_of(layer)),
            Mode::Name => layer.name.to_lowercase().contains(&self.query.trim().to_lowercase()),
            Mode::Effect => match self.value.as_str() {
                "none" => layer.effects.items.is_empty(),
                "any" => !layer.effects.items.is_empty(),
                label => layer.effects.items.iter().any(|effect| effect.label() == label),
            },
            Mode::Blend => self.value == "any" || layer.blend.label() == self.value,
            Mode::Attribute => match self.value.as_str() {
                "hidden" => !layer.visible,
                "locked" => locked(layer),
                "unlocked" => !locked(layer),
                "clipped" => layer.clipped,
                "mask" => layer.mask.is_some(),
                "vectorMask" => layer.vector_mask.is_some(),
                "linked" => layer.link_group.is_some(),
                _ => layer.visible,
            },
            Mode::Color => label_name(layer.label) == self.value,
            Mode::SmartObject => match &layer.content {
                LayerContent::Smart(smart) => match self.value.as_str() {
                    "embedded" => matches!(smart.source, SmartSource::Embedded { .. }),
                    "linked" => matches!(smart.source, SmartSource::Linked { .. }),
                    "filters" => !smart.smart_filters.is_empty(),
                    _ => true,
                },
                _ => false,
            },
            Mode::Selected => selected.contains(&layer.id),
            Mode::Artboard => layer.artboard().is_some(),
        }
    }
}

fn locked(layer: &Layer) -> bool {
    let locks = layer.locks;
    locks.all || locks.pixels || locks.position || locks.transparency || locks.artboard
}

pub fn kind_of(layer: &Layer) -> &'static str {
    match &layer.content {
        LayerContent::Raster(_) => "pixel",
        LayerContent::Adjustment(_) | LayerContent::Fill(_) => "adjustment",
        LayerContent::Text(_) => "type",
        LayerContent::Shape(_) => "shape",
        LayerContent::Smart(_) => "smart",
        LayerContent::Group(_) => "group",
    }
}

fn label_name(color: LabelColor) -> &'static str {
    match color {
        LabelColor::None => "none",
        LabelColor::Red => "red",
        LabelColor::Orange => "orange",
        LabelColor::Yellow => "yellow",
        LabelColor::Green => "green",
        LabelColor::Blue => "blue",
        LabelColor::Violet => "violet",
        LabelColor::Gray => "gray",
    }
}

fn choices(mode: Mode) -> Vec<(String, &'static str)> {
    let items: Vec<(&str, &str)> = match mode {
        Mode::Effect => vec![
            ("any", "Any Effect"),
            ("none", "No Effects"),
            ("Drop Shadow", "Drop Shadow"),
            ("Inner Shadow", "Inner Shadow"),
            ("Outer Glow", "Outer Glow"),
            ("Inner Glow", "Inner Glow"),
            ("Stroke", "Stroke"),
            ("Color Overlay", "Color Overlay"),
            ("Gradient Overlay", "Gradient Overlay"),
            ("Pattern Overlay", "Pattern Overlay"),
            ("Satin", "Satin"),
            ("Bevel & Emboss", "Bevel & Emboss"),
        ],
        Mode::Blend => std::iter::once(("any", "Any Mode"))
            .chain(std::iter::once(("Pass Through", "Pass Through")))
            .chain(BlendMode::LAYER_MODES.into_iter().map(|mode| (mode.label(), mode.label())))
            .collect(),
        Mode::Attribute => vec![
            ("visible", "Visible"),
            ("hidden", "Hidden"),
            ("locked", "Locked"),
            ("unlocked", "Unlocked"),
            ("clipped", "Clipped"),
            ("mask", "Layer Mask"),
            ("vectorMask", "Vector Mask"),
            ("linked", "Linked Layers"),
        ],
        Mode::Color => vec![
            ("none", "None"),
            ("red", "Red"),
            ("orange", "Orange"),
            ("yellow", "Yellow"),
            ("green", "Green"),
            ("blue", "Blue"),
            ("violet", "Violet"),
            ("gray", "Gray"),
        ],
        Mode::SmartObject => vec![("any", "All Smart Objects"), ("embedded", "Embedded"), ("linked", "Linked"), ("filters", "Has Smart Filters")],
        _ => Vec::new(),
    };
    items.into_iter().map(|(value, label)| (value.to_string(), label)).collect()
}

pub fn valid_value(options: &Options) -> bool {
    let values = choices(options.mode);
    if values.is_empty() { options.value == "any" } else { values.iter().any(|(value, _)| value == &options.value) }
}

/// None releases Artboard filtering when selection leaves all artboards.
pub fn artboard_match_ids(doc: &photocraft_doc::Document, selected: &[LayerId]) -> Option<std::collections::HashSet<LayerId>> {
    let selected: std::collections::HashSet<_> = selected.iter().copied().collect();
    let mut wanted_boards = std::collections::HashSet::new();
    let mut entries = Vec::new();
    let mut board_edges = Vec::new();
    let mut found = 0;
    let mut pending: Vec<_> = doc.layers.iter().map(|layer| (layer, None, 0)).collect();
    while let Some((layer, parent_board, depth)) = pending.pop() {
        let board = if layer.artboard().is_some() {
            board_edges.push((layer.id, parent_board));
            Some(layer.id)
        } else {
            parent_board
        };
        if selected.contains(&layer.id) {
            found += 1;
            let board = board?;
            wanted_boards.insert(board);
        }
        entries.push((layer.id, board));
        if depth < photocraft_doc::MAX_GROUP_DEPTH
            && let Some(children) = layer.children()
        {
            pending.extend(children.iter().map(|child| (child, board, depth + 1)));
        }
    }
    if wanted_boards.is_empty() || found != selected.len() {
        return None;
    }
    for (board, parent) in board_edges {
        if parent.is_some_and(|parent| wanted_boards.contains(&parent)) {
            wanted_boards.insert(board);
        }
    }
    let ids: std::collections::HashSet<_> =
        entries.into_iter().filter_map(|(id, board)| board.filter(|board| wanted_boards.contains(board)).map(|_| id)).collect();
    Some(ids)
}

pub fn is_filtering(doc: &photocraft_doc::Document, options: &Options, kinds: &[String], selected: &[LayerId]) -> bool {
    options.active(kinds) && (options.mode != Mode::Artboard || artboard_match_ids(doc, selected).is_some())
}

/// Shared by the rendered panel and automation inspection.
pub fn rows<'a>(doc: &'a photocraft_doc::Document, options: &Options, kinds: &[String], selected: &[LayerId]) -> Vec<(usize, &'a Layer)> {
    if !options.active(kinds) {
        return crate::layer_tree_ui::display_rows(doc, false);
    }
    if options.mode == Mode::Artboard {
        return match artboard_match_ids(doc, selected) {
            Some(ids) => crate::layer_tree_ui::filtered_rows(doc, |layer| ids.contains(&layer.id)),
            None => crate::layer_tree_ui::display_rows(doc, false),
        };
    }
    if options.mode == Mode::Selected {
        let selected: std::collections::HashSet<_> = selected.iter().copied().collect();
        return crate::layer_tree_ui::filtered_rows(doc, |layer| selected.contains(&layer.id));
    }
    crate::layer_tree_ui::filtered_rows(doc, |layer| options.matches(layer, kinds, selected))
}

fn dropdown<T: PartialEq + Clone>(ui: &mut egui::Ui, id: &str, current: &mut T, options: &[(T, &str)], width: f32) -> bool {
    let label = options.iter().find(|(value, _)| value == current).map_or("—", |(_, label)| tl!(*label));
    let mut changed = false;
    // ComboBox::width is a minimum: bound its parent as well so translated selected text
    // cannot consume the neighbouring enlarged controls' reserved area.
    ui.allocate_ui_with_layout(vec2(width, CONTROL_HEIGHT), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        egui::ComboBox::from_id_salt(id).selected_text(label).width(width).height(420.0).truncate().show_ui(ui, |ui| {
            for (value, label) in options {
                if ui.selectable_label(value == current, tl!(*label)).clicked() {
                    *current = value.clone();
                    changed = true;
                }
            }
        });
    });
    changed
}

const CONTROL_HEIGHT: f32 = 26.0;
const CONTROL_GLYPH: f32 = 17.0;
// Include the four two-point gaps used by the Layers controls row in its minimum budget.
pub const LOCKS_MIN_WIDTH: f32 = 5.0 * 22.0 + 4.0 * 2.0;

#[cfg(test)]
fn controls_bounds(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|data| data.get_temp(egui::Id::new("layers-controls-available-bounds")))
}

/// Hit boxes may share extra row width, while each glyph keeps a readable fixed size.
fn control_button(ui: &mut egui::Ui, icon: &str, width: f32, selected: bool, tooltip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(width, CONTROL_HEIGHT), Sense::click());
    let tint = icons::button_chrome(ui, rect, selected, response.hovered());
    icons::paint(ui, rect, icon, CONTROL_GLYPH.min((width - 4.0).max(0.0)), tint);
    response.on_hover_text(tooltip)
}

/// Width the five lock controls can use after reserving the Fill label/value and optional Lock label.
/// Normal panels provide at least this minimum; unusually small embedded layouts remain bounded.
pub fn locks_width(available: f32) -> f32 {
    if available.is_finite() { available.max(0.0) } else { LOCKS_MIN_WIDTH }
}

/// Reuses the original UiState.layer_filter kinds for saved-workspace compatibility.
pub fn show(ui: &mut egui::Ui, options: &mut Options, kinds: &mut Vec<String>) {
    #[cfg(test)]
    {
        // Header controls live outside the scrolling layer list, whose fixed scrollbar
        // gutter makes its row rect narrower. Record their actual available/clipped region.
        let bounds = ui.available_rect_before_wrap().intersect(ui.clip_rect());
        ui.ctx().data_mut(|data| data.insert_temp(egui::Id::new("layers-controls-available-bounds"), bounds));
    }
    let t = Tokens::get(ui.ctx());
    ui.push_id("layers-filter", |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let modes = [
                (Mode::Kind, "Kind"),
                (Mode::Name, "Name"),
                (Mode::Effect, "Effect"),
                (Mode::Blend, "Mode"),
                (Mode::Attribute, "Attribute"),
                (Mode::Color, "Color"),
                (Mode::SmartObject, "Smart Object"),
                (Mode::Selected, "Selected"),
                (Mode::Artboard, "Artboard"),
            ];
            let gap = ui.spacing().item_spacing.x;
            let mode_width = (ui.available_width() - 6.0 * CONTROL_HEIGHT - 6.0 * gap).clamp(58.0, 104.0);
            if dropdown(ui, "layer-filter-mode", &mut options.mode, &modes, mode_width) {
                options.value = match options.mode {
                    Mode::Attribute => "visible",
                    Mode::Color => "none",
                    _ => "any",
                }
                .into();
                options.enabled = true;
            }
            let remaining = (ui.available_width() - CONTROL_HEIGHT - gap).max(0.0);
            let kind_width = ((remaining - 4.0 * gap) / 5.0).max(0.0);
            ui.add_enabled_ui(options.enabled, |ui| match options.mode {
                Mode::Kind => {
                    for (kind, icon, tip) in [
                        ("pixel", "image", "Filter for pixel layers"),
                        ("adjustment", "contrast", "Filter for adjustment layers"),
                        ("type", "type", "Filter for type layers"),
                        ("shape", "square", "Filter for shape layers"),
                        ("smart", "app-window", "Filter for smart objects"),
                    ] {
                        let on = kinds.iter().any(|value| value == kind);
                        let response = control_button(ui, icon, kind_width, on, tl!(tip));
                        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), on, tl!(tip)));
                        if response.clicked() {
                            if on {
                                kinds.retain(|value| value != kind);
                            } else {
                                kinds.push(kind.into());
                            }
                        }
                    }
                }
                Mode::Name => {
                    ui.add(
                        egui::TextEdit::singleline(&mut options.query)
                            .hint_text(tl!("Search layers"))
                            .desired_width((remaining - 8.0).max(32.0))
                            .char_limit(256),
                    );
                }
                Mode::Selected | Mode::Artboard => {
                    let text = if options.mode == Mode::Selected { tl!("Selected layers") } else { tl!("Artboards") };
                    ui.add_sized(vec2(remaining, CONTROL_HEIGHT), egui::Label::new(text).truncate());
                }
                mode => {
                    let choices = choices(mode);
                    dropdown(ui, "layer-filter-value", &mut options.value, &choices, remaining);
                }
            });
            let (rect, mut response) = ui.allocate_exact_size(vec2(CONTROL_HEIGHT, CONTROL_HEIGHT), Sense::click());
            let tint = if options.enabled { t.accent } else { t.text_faint };
            let track = Rect::from_center_size(rect.center(), vec2(14.0, 21.0));
            ui.painter().rect_filled(track, 5.0, if options.enabled { t.accent_soft } else { t.field });
            ui.painter().rect_stroke(track, 5.0, Stroke::new(1.0, tint), StrokeKind::Inside);
            ui.painter().circle_filled(track.center() + vec2(0.0, if options.enabled { -4.0 } else { 4.0 }), 3.5, tint);
            if response.clicked() {
                options.enabled = !options.enabled;
                response.mark_changed();
            }
            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, options.enabled, tl!("Enable layer filtering")));
            response.on_hover_text(tl!("Enable layer filtering"));
        });
    });
    ui.add_space(3.0);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LockState {
    pub applicable: usize,
    pub all_on: bool,
    pub mixed: bool,
    pub inherited: bool,
}

fn lock_value(locks: photocraft_doc::Locks, key: &str) -> bool {
    match key {
        "transparency" => locks.transparency,
        "pixels" => locks.pixels,
        "position" => locks.position,
        "artboard" => locks.artboard,
        "all" => locks.all,
        _ => false,
    }
}

/// Own locks determine the checkbox. A parent's lock is explained separately so
/// unlocking a child never pretends to remove its inherited restriction.
pub fn lock_state(doc: &photocraft_doc::Document, selected: &[LayerId], key: &str) -> LockState {
    let mut applicable = 0;
    let mut displayed = 0;
    let mut on = 0;
    let mut inherited = false;
    for layer in selected.iter().filter_map(|id| doc.layer(*id)) {
        if !photocraft_engine::layer_multi_cmds::supports_lock(layer, key) {
            if photocraft_engine::layer_multi_cmds::fixed_lock_selected(layer, key) {
                displayed += 1;
                on += 1;
            }
            continue;
        }
        applicable += 1;
        displayed += 1;
        on += usize::from(lock_value(layer.locks, key));
        let effective = doc.effective_locks(layer.id);
        inherited |= (lock_value(effective, key) || effective.all) && !(lock_value(layer.locks, key) || layer.locks.all);
    }
    LockState { applicable, all_on: displayed > 0 && on == displayed, mixed: on > 0 && on < displayed, inherited }
}

/// Explicit row budget prevents larger lock icons from taking the adjacent Fill control's space.
pub fn locks(ui: &mut egui::Ui, doc: &photocraft_doc::Document, selected: &[LayerId], actions: &mut Vec<(String, serde_json::Value)>, width: f32) {
    let gap = ui.spacing().item_spacing.x;
    let button_width = ((locks_width(width) - 4.0 * gap) / 5.0).max(0.0);
    for (key, icon, tip) in [
        ("transparency", "grid-3x3", "Lock transparent pixels"),
        ("pixels", "brush", "Lock image pixels"),
        ("position", "move", "Lock position"),
        ("artboard", "scan", "Prevent auto-nesting in and out of Artboards and Frames"),
        ("all", "lock", "Lock all"),
    ] {
        let state = lock_state(doc, selected, key);
        let mut tip = tl!(tip).to_string();
        if state.mixed {
            tip.push_str(&format!("\n{}", tl!("Mixed selection")));
        }
        if state.inherited {
            tip.push_str(&format!("\n{}", tl!("Parent group lock remains active")));
        }
        if state.applicable == 0 && state.all_on {
            tip.push_str(&format!("\n{}", tl!("Always locked for this layer type")));
        }
        let clicked = ui
            .add_enabled_ui(state.applicable > 0, |ui| {
                let response = control_button(ui, icon, button_width, state.all_on || state.mixed, &tip);
                if state.mixed {
                    let p = response.rect.center_bottom() - vec2(0.0, 3.0);
                    ui.painter().line_segment([p - vec2(3.0, 0.0), p + vec2(3.0, 0.0)], Stroke::new(2.0, Tokens::get(ui.ctx()).accent_text));
                }
                response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), state.all_on, tl!(tip_for_lock(key))));
                response.on_disabled_hover_text(&tip).clicked()
            })
            .inner;
        if clicked {
            actions.push(("layer.lockLayers".into(), serde_json::json!({"toggle":key})));
        }
    }
}

fn tip_for_lock(key: &str) -> &'static str {
    match key {
        "transparency" => "Lock transparent pixels",
        "pixels" => "Lock image pixels",
        "position" => "Lock position",
        "artboard" => "Prevent auto-nesting in and out of Artboards and Frames",
        _ => "Lock all",
    }
}

#[cfg(test)]
#[path = "layer_filter_tests_v79.rs"]
mod tests;

#[cfg(test)]
#[path = "layer_filter_icon_tests_v80.rs"]
mod icon_tests_v80;
