//! Persistent user blank-document presets, using the shared preferences command.

use super::*;
use crate::theme::palette::ScrollAreaPaletteExt;
use photocraft_engine::prefs::document_presets::{DocumentPreset, MAX_PRESETS};

pub(super) fn apply(f: &mut Map<String, Value>, p: &DocumentPreset) {
    for (key, value) in [
        ("width", json!(p.width)),
        ("height", json!(p.height)),
        ("resolution", json!(p.resolution)),
        ("__unit", json!(p.unit)),
        ("__resUnit", json!(p.resolution_unit)),
        ("mode", json!(p.mode)),
        ("depth", json!(p.depth)),
        ("background", json!(p.background)),
        ("__customPreset", json!(p.name)),
        ("__savePresetName", json!(p.name)),
    ] {
        f.insert(key.into(), value);
    }
    f.remove("__preset");
    f.remove("__presetError");
}

pub(super) fn snapshot(f: &Map<String, Value>, name: &str) -> Result<DocumentPreset, String> {
    let data = json!({"name": name.trim(), "width":f.get("width").cloned().unwrap_or(json!(1920)),
        "height":f.get("height").cloned().unwrap_or(json!(1080)), "resolution":get_f(f,"resolution",72.0),
        "unit":get_s(f,"__unit","px"), "resolutionUnit":get_s(f,"__resUnit","in"),
        "mode":get_s(f,"mode","rgb"), "depth":f.get("depth").cloned().unwrap_or(json!(8)),
        "background":get_s(f,"background","white")});
    serde_json::from_value(data).map_err(|error| error.to_string())
}

pub(super) fn store(app: &mut crate::PhotocraftApp, f: &mut Map<String, Value>, preset: DocumentPreset) -> bool {
    let mut presets = app.session.prefs().document_presets.clone();
    if let Some(existing) = presets.iter_mut().find(|p| p.name.to_lowercase() == preset.name.to_lowercase()) {
        *existing = preset.clone();
    } else {
        if presets.len() >= MAX_PRESETS {
            f.insert("__presetError".into(), json!(tl!("You can save up to 128 document presets.")));
            return false;
        }
        presets.push(preset.clone());
    }
    if app.run("prefs.set", json!({"path":"documentPresets","value":presets})).is_err() {
        f.insert("__presetError".into(), json!(tl!("Could not save document preset.")));
        return false;
    }
    apply(f, &preset);
    f.insert("__category".into(), json!("Custom"));
    true
}

pub(super) fn remove(app: &mut crate::PhotocraftApp, f: &mut Map<String, Value>, name: &str) {
    let presets: Vec<_> = app.session.prefs().document_presets.iter().filter(|p| p.name != name).cloned().collect();
    if app.run("prefs.set", json!({"path":"documentPresets","value":presets})).is_ok() {
        f.remove("__customPreset");
        f.remove("__savePresetName");
        f.remove("__presetError");
    } else {
        f.insert("__presetError".into(), json!(tl!("Could not save document preset.")));
    }
}

pub(super) fn grid(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let presets = app.session.prefs().document_presets.clone();
    let chosen = get_s(f, "__customPreset", "");
    ui.label(RichText::new(crate::i18n::fmt(tl!("SAVED DOCUMENT PRESETS ({n})"), &[("n", &presets.len().to_string())])).size(11.0).color(t.text_faint));
    ui.add_space(6.0);
    if presets.is_empty() {
        ui.label(RichText::new(tl!("Save the current settings to create a custom preset.")).color(t.text_dim));
        return;
    }
    egui::ScrollArea::vertical().id_salt("new-document-custom-presets").max_height(410.0).show_palette(ui, |ui| {
        for row in presets.chunks(3) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for preset in row {
                    let (rect, response) = ui.allocate_exact_size(vec2(164.0, 112.0), Sense::click());
                    let selected = chosen == preset.name;
                    let foreground = if selected && t.bevel { t.accent_text } else { t.text };
                    let secondary = if selected && t.bevel { t.accent_text } else { t.text_faint };
                    let mut icon_tokens = t;
                    if selected && t.bevel {
                        icon_tokens.text_dim = t.accent_text;
                    }
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(
                        rect,
                        t.radius,
                        if selected {
                            t.row_selected
                        } else if response.hovered() {
                            t.hover
                        } else {
                            t.field
                        },
                    );
                    if selected {
                        painter.rect_stroke(rect, t.radius, Stroke::new(1.5, t.accent), StrokeKind::Inside);
                    }
                    page_icon(
                        ui,
                        Rect::from_center_size(pos2(rect.center().x, rect.top() + 30.0), vec2(40.0, 40.0)),
                        preset.width,
                        preset.height,
                        &icon_tokens,
                    );
                    let title = crate::tab_strip::elided(ui, &preset.name, egui::FontId::proportional(12.0), foreground, 144.0);
                    painter.galley(pos2(rect.center().x - title.size().x / 2.0, rect.top() + 64.0), title, foreground);
                    let size = format!("{} × {} px · {} ppi", preset.width, preset.height, widgets::fmt_num(preset.resolution as f64));
                    painter.text(pos2(rect.center().x, rect.top() + 90.0), Align2::CENTER_CENTER, size, egui::FontId::proportional(10.0), secondary);
                    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &preset.name));
                    if response.on_hover_text(&preset.name).clicked() {
                        apply(f, preset);
                    }
                }
            });
            ui.add_space(8.0);
        }
    });
}

pub(super) fn save_controls(app: &mut crate::PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    ui.add_space(14.0);
    widgets::hairline(ui);
    ui.add_space(8.0);
    small_label(ui, tl!("Document preset name"));
    let mut name = get_s(f, "__savePresetName", "");
    if ui.add(egui::TextEdit::singleline(&mut name).desired_width(240.0).char_limit(80)).changed() {
        f.insert("__savePresetName".into(), json!(name));
        f.remove("__presetError");
    }
    let name = name.trim();
    let replace = app.session.prefs().document_presets.iter().any(|p| p.name.to_lowercase() == name.to_lowercase());
    let label = if replace { tl!("Replace document preset") } else { tl!("Save document preset") };
    let preset = snapshot(f, name);
    ui.add_enabled_ui(preset.is_ok(), |ui| {
        if widgets::secondary_button(ui, label, 240.0).clicked()
            && let Ok(preset) = preset
        {
            store(app, f, preset);
        }
    });
    let selected = get_s(f, "__customPreset", "");
    if !selected.is_empty()
        && app.session.prefs().document_presets.iter().any(|p| p.name == selected)
        && widgets::secondary_button(ui, tl!("Delete document preset"), 240.0).clicked()
    {
        remove(app, f, &selected);
    }
    if let Some(error) = f.get("__presetError").and_then(Value::as_str) {
        ui.label(RichText::new(error).size(11.5).color(Tokens::get(ui.ctx()).text_dim));
    }
}
