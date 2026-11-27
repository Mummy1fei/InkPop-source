//! Layers-panel pieces for smart objects: the thumbnail badge and the Smart Filters sub-rows
//! (eye toggles; double-click a filter to re-open its dialog with the recorded parameters).

use egui::{Color32, Rect, Sense, Stroke, pos2, vec2};
use photocraft_doc::{Layer, LayerContent, SmartObject};
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{PhotocraftApp, icons};

/// Smart-object badge in the bottom-right corner of a layer thumbnail.
pub fn thumb_badge(ui: &egui::Ui, l: &Layer, thumb: Rect) {
    if !matches!(l.content, LayerContent::Smart(_)) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    // Keep the badge in the corner so the thumbnail still shows its image.
    let size = 11.0_f32.min(thumb.width().max(0.0)).min(thumb.height().max(0.0));
    let r = Rect::from_min_size(thumb.right_bottom() - vec2(size, size), vec2(size, size));
    ui.painter().rect_filled(r, 2.0, t.field);
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    icons::paint(ui, r, "app-window", (size - 2.0).max(0.0), t.icon);
}

/// The display name of a smart filter (its command's label without the ellipsis). A Photoshop
/// filter PhotoCraft doesn't implement shows its own name and is marked as kept as is.
fn filter_label(f: &photocraft_doc::SmartFilter) -> String {
    let command = f.command.as_str();
    if command == photocraft_engine::smart_cmds::UNSUPPORTED_FILTER {
        let name = f.params.get("name").and_then(Value::as_str).unwrap_or_else(|| tl!("Photoshop filter"));
        return crate::i18n::fmt(tl!("{name} (kept, not editable)"), &[("name", name.trim_end_matches("...").trim_end_matches('…'))]);
    }
    photocraft_engine::commands::find(command)
        .map_or_else(|| command.rsplit('.').next().unwrap_or(command).to_string(), |c| tl!(c.label).trim_end_matches('…').to_string())
}

pub const SET_EXPANDED: &str = "ui.smartFilters.setExpanded";

/// View-only state; transient document IDs must not enter saved workspaces.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FiltersUi {
    pub collapsed: std::collections::HashSet<(u64, u64)>,
    pub layers: Vec<LayerRecord>,
    pub rows: Vec<FilterRecord>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayerRecord {
    pub document: u64,
    pub layer: u64,
    pub expanded: bool,
    pub filter_count: usize,
    pub row: Rect,
    pub disclosure: Rect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FilterRecord {
    pub layer: u64,
    pub index: Option<usize>,
    pub row: Rect,
    pub eye: Rect,
    pub name: Option<Rect>,
}

pub fn begin(app: &mut PhotocraftApp) {
    app.ui.smart_filters.layers.clear();
    app.ui.smart_filters.rows.clear();
}

pub fn expanded(app: &PhotocraftApp, layer: photocraft_doc::LayerId) -> bool {
    app.session.active().is_none_or(|state| !app.ui.smart_filters.collapsed.contains(&(state.doc.id.0, layer.0)))
}

pub fn set_expanded(app: &mut PhotocraftApp, ctx: &egui::Context, params: &Value) -> Result<Value, String> {
    let object = params.as_object().ok_or("smart filter expansion requires an object")?;
    if object.keys().any(|key| !matches!(key.as_str(), "layer" | "expanded")) {
        return Err("unsupported smart filter expansion property".into());
    }
    let expanded = params.get("expanded").and_then(Value::as_bool).ok_or("expanded must be a boolean")?;
    let state = app.session.active().ok_or("no active document")?;
    let layer = match params.get("layer") {
        Some(value) => photocraft_doc::LayerId(value.as_u64().ok_or("invalid layer id")?),
        None => state.active_layer.ok_or("no active layer")?,
    };
    if !state.doc.layer(layer).is_some_and(has_filters) {
        return Err("layer has no smart filters".into());
    }
    let key = (state.doc.id.0, layer.0);
    let documents: std::collections::HashSet<_> = app.session.documents().iter().map(|state| state.doc.id.0).collect();
    app.ui.smart_filters.collapsed.retain(|(doc, _)| documents.contains(doc));
    if expanded {
        app.ui.smart_filters.collapsed.remove(&key);
    } else {
        app.ui.smart_filters.collapsed.insert(key);
    }
    ctx.request_repaint();
    Ok(json!({"layer":layer,"expanded":expanded}))
}

pub fn has_filters(layer: &Layer) -> bool {
    matches!(&layer.content, LayerContent::Smart(smart) if !smart.smart_filters.is_empty())
}

pub fn record_layer(app: &mut PhotocraftApp, document: photocraft_doc::DocId, layer: &Layer, row: Rect, disclosure: Rect) {
    let LayerContent::Smart(smart) = &layer.content else { return };
    let expanded = expanded(app, layer.id);
    app.ui.smart_filters.layers.push(LayerRecord {
        document: document.0,
        layer: layer.id.0,
        expanded,
        filter_count: smart.smart_filters.len(),
        row,
        disclosure,
    });
}

pub fn inspect(app: &PhotocraftApp) -> Value {
    let rect = |rect: Rect| [rect.left(), rect.top(), rect.right(), rect.bottom()];
    let layers: Vec<_> = app.ui.smart_filters.layers.iter().map(|row|json!({"document":row.document,"layer":row.layer,"expanded":row.expanded,"filter_count":row.filter_count,"row":rect(row.row),"disclosure":rect(row.disclosure)})).collect();
    let rows: Vec<_> = app
        .ui
        .smart_filters
        .rows
        .iter()
        .map(|row| json!({"layer":row.layer,"index":row.index,"row":rect(row.row),"eye":rect(row.eye),"name":row.name.map(rect)}))
        .collect();
    json!({"layers":layers,"rows":rows})
}

/// Collapse is checked before allocating rows or formatting filter names;
/// this is view-only and never asks the engine to recompute pixels.
pub fn filter_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, l: &Layer, depth: usize, actions: &mut Vec<(String, Value)>) {
    let LayerContent::Smart(sm) = &l.content else { return };
    if sm.smart_filters.is_empty() || !expanded(app, l.id) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let rows = std::iter::once(None).chain((0..sm.smart_filters.len()).rev().map(Some));
    for index in rows {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.35));
        }
        if t.pro {
            ui.painter().line_segment([pos2(rect.left() + 30.0, rect.top()), pos2(rect.left() + 30.0, rect.bottom())], Stroke::new(1.0, t.separator));
        }
        let filter = index.and_then(|index| sm.smart_filters.get(index));
        let (name, shown, on) = match filter {
            Some(filter) => (filter_label(filter), filter.visible, filter.visible && sm.filters_enabled),
            None => (tl!("Smart Filters").to_string(), sm.filters_enabled, sm.filters_enabled),
        };
        let base = (rect.left() + 36.0 + depth as f32 * 14.0).min((rect.right() - 100.0).max(rect.left() + 30.0));
        let eye_x = base + if index.is_some() { 18.0 } else { 0.0 };
        let eye = Rect::from_center_size(pos2(eye_x + 10.0, rect.center().y), vec2(20.0, 20.0));
        let eye_resp = ui.interact(eye, ui.id().with(("sf-eye", l.id.0, index)), Sense::click());
        icons::paint(ui, eye, if shown { "eye" } else { "eye-off" }, 14.0, if on { t.icon } else { t.text_faint });
        eye_resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, shown, format!("{} {}", tl!("Visibility"), name)));
        if eye_resp.clicked() {
            actions.push(match index {
                None => ("layer.smartFilter.disableSmartFilters".into(), json!({"layer":l.id.0})),
                Some(index) => ("layer.smartFilter.setVisible".into(), json!({"layer":l.id.0,"index":index})),
            });
        }
        let x = if index.is_none() {
            let sw = Rect::from_center_size(pos2(eye.right() + 11.0, rect.center().y), vec2(14.0, 12.0));
            ui.painter().rect_filled(sw, 1.0, if sm.filter_mask.as_ref().is_some_and(|mask| mask.enabled) { Color32::from_gray(200) } else { Color32::WHITE });
            ui.painter().rect_stroke(sw, 1.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
            sw.right() + 6.0
        } else {
            eye.right() + 6.0
        };
        let name_rect = crate::layer_row_ui::label(
            ui.painter(),
            x,
            rect.center().y,
            rect.right() - 8.0,
            &name,
            egui::FontId::proportional(11.5),
            if on { t.text_dim } else { t.text_faint },
        );
        app.ui.smart_filters.rows.push(FilterRecord { layer: l.id.0, index, row: rect, eye, name: name_rect });
        if let Some(index) = index
            && resp.double_clicked()
            && !eye_resp.contains_pointer()
        {
            open_editor(app, ui.ctx(), l.id.0, sm, index);
        }
    }
}

/// Re-opens a smart filter's dialog with its recorded parameters; OK runs
/// `layer.smartFilter.setParams` instead of adding another filter (see `dialogs::confirm`).
pub fn open_editor(app: &mut PhotocraftApp, ctx: &egui::Context, layer: u64, sm: &SmartObject, index: usize) {
    let Some(f) = sm.smart_filters.get(index) else { return };
    // Camera Raw has its own full-window dialog rather than a generic parameter form.
    if f.command == photocraft_engine::lens_cmds::RAW {
        if let Err(e) = crate::camera_raw_ui::open_smart_filter(app, ctx, photocraft_doc::LayerId(layer), index) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    if !crate::filter_dialog::has_dialog(&f.command) {
        return;
    }
    let Some(id) = crate::filter_dialog::open(app, &f.command) else { return };
    if let Some(d) = app.ui.dialog_mut(id) {
        // The live preview would stack a second copy of the filter on the smart object.
        d.fields.remove("__preview");
        d.fields.insert("__smartFilter".into(), json!({"layer": layer, "index": index}));
        if let Value::Object(p) = &f.params {
            for (k, v) in p {
                if d.fields.contains_key(k) {
                    d.fields.insert(k.clone(), v.clone());
                }
            }
        }
    }
}

/// The command a confirmed filter dialog runs: smart-filter edits update the recorded filter.
pub fn confirm_command(fields: &serde_json::Map<String, Value>, command: String, params: Value) -> (String, Value) {
    match fields.get("__smartFilter") {
        Some(sf) => ("layer.smartFilter.setParams".into(), json!({"layer": sf["layer"], "index": sf["index"], "params": params})),
        None => (command, params),
    }
}

#[cfg(test)]
#[path = "smart_ui_tests_v67.rs"]
mod tests_v67;

#[cfg(test)]
#[path = "smart_badge_tests_v75.rs"]
mod badge_tests_v75;

#[cfg(test)]
#[path = "smart_filter_panel_tests_v80.rs"]
mod panel_tests_v80;
