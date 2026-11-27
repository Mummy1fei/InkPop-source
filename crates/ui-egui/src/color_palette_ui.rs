//! Studio Light palette editor. All edits stay in the Preferences draft until Apply/OK.
use egui::RichText;
use photocraft_engine::prefs::palette::builtin::{BUILTIN_PALETTES, BuiltinPalette};
use photocraft_engine::prefs::palette::{ColorPaletteFile, ColorPreset, ColorSlot, PaletteRgb, StudioLightColors};
use serde_json::{Map, Value, json};

use crate::{PhotocraftApp, theme::Tokens};

const SELECTED: &str = "__paletteSelected";
const NAME: &str = "__paletteName";
const CATEGORY: &str = "__paletteCategory";
const MESSAGE: &str = "__paletteMessage";
const ERROR: &str = "__paletteError";

struct Group {
    title: &'static str,
    colors: &'static [(ColorSlot, &'static str)],
}

const GROUPS: &[Group] = &[
    Group {
        title: "Window and Toolbars",
        colors: &[
            (ColorSlot::TitleBar, "Title and Menu Bar"),
            (ColorSlot::Toolbar, "Tool Palette"),
            (ColorSlot::OptionsBar, "Tool Options Bar"),
            (ColorSlot::StatusBar, "Status Bar"),
        ],
    },
    Group {
        title: "Panels and Floating Windows",
        colors: &[
            (ColorSlot::Dock, "Dock Background"),
            (ColorSlot::Card, "Panel Background"),
            (ColorSlot::CardBorder, "Panel Border"),
            (ColorSlot::PanelTitle, "Panel Title Bar"),
            (ColorSlot::FloatingTitle, "Floating Window Title Bar"),
        ],
    },
    Group {
        title: "Menus",
        colors: &[
            (ColorSlot::MenuBackground, "Popup Menu Background"),
            (ColorSlot::MenuHover, "Menu Hover"),
            (ColorSlot::MenuSelection, "Menu Selected Item"),
            (ColorSlot::MenuText, "Menu Text"),
        ],
    },
    Group {
        title: "Input Controls",
        colors: &[
            (ColorSlot::Field, "Input Background"),
            (ColorSlot::FieldBorder, "Input Border"),
            (ColorSlot::Hover, "Control Hover"),
            (ColorSlot::Pressed, "Control Pressed"),
        ],
    },
    Group {
        title: "Text and Icons",
        colors: &[
            (ColorSlot::Text, "Primary Text"),
            (ColorSlot::TextDim, "Secondary Text"),
            (ColorSlot::TextFaint, "Muted Text"),
            (ColorSlot::Icon, "Tool Icons"),
        ],
    },
    Group {
        title: "Accents and Selection",
        colors: &[
            (ColorSlot::Accent, "Accent Color"),
            (ColorSlot::AccentText, "Accent Text"),
            (ColorSlot::Selection, "Selected Row"),
            (ColorSlot::Separator, "Separators"),
        ],
    },
    Group {
        title: "Progress, Sliders and Scrollbars",
        colors: &[
            (ColorSlot::ProgressTrack, "Progress Track"),
            (ColorSlot::ProgressFill, "Progress Fill"),
            (ColorSlot::SliderTrack, "Slider Track"),
            (ColorSlot::SliderFill, "Slider Fill"),
            (ColorSlot::SliderHandle, "Slider Handle"),
            (ColorSlot::ScrollbarTrack, "Scrollbar Track"),
            (ColorSlot::ScrollbarHandle, "Scrollbar Handle"),
        ],
    },
    Group { title: "Canvas", colors: &[(ColorSlot::Canvas, "Canvas Surround"), (ColorSlot::CanvasDot, "Canvas Grid Dots")] },
];

fn read_config(interface: &Map<String, Value>) -> Result<StudioLightColors, String> {
    match interface.get("studioLightColors") {
        Some(value) => serde_json::from_value(value.clone()).map_err(|error| error.to_string()),
        None => Ok(StudioLightColors::default()),
    }
}

fn store_config(interface: &mut Map<String, Value>, config: &StudioLightColors) -> Result<(), String> {
    config.validate()?;
    let value = serde_json::to_value(config).map_err(|error| error.to_string())?;
    interface.insert("studioLightColors".into(), value);
    Ok(())
}

fn selected_index(config: &StudioLightColors, selected: &str) -> Option<usize> {
    config.presets.iter().position(|preset| preset.name == selected)
}

fn save_named(config: &mut StudioLightColors, name: &str) -> Result<String, String> {
    let name = photocraft_engine::prefs::palette::validate_name(name).map_err(|_| tl!("Enter a palette name with 1–64 visible characters.").to_string())?;
    if config.presets.iter().any(|preset| preset.name.to_lowercase() == name.to_lowercase()) {
        return Err(tl!("This name is already saved. Use Replace Selected or choose another name.").to_string());
    }
    if config.presets.len() >= photocraft_engine::prefs::palette::MAX_PRESETS {
        return Err(tl!("You can save up to 32 palettes. Delete one before adding another.").to_string());
    }
    let preset = ColorPreset::new(name, config.colors.clone())?;
    let mut candidate = config.clone();
    candidate.presets.push(preset.clone());
    candidate.validate()?;
    *config = candidate;
    Ok(preset.name)
}

fn replace_named(config: &mut StudioLightColors, selected: &str) -> Result<(), String> {
    let index = selected_index(config, selected).ok_or_else(|| tl!("Choose a saved palette first.").to_string())?;
    let mut candidate = config.clone();
    let palette = candidate.presets.get_mut(index).ok_or_else(|| tl!("Choose a saved palette first.").to_string())?;
    palette.colors = config.colors.clone();
    candidate.validate()?;
    *config = candidate;
    Ok(())
}

fn load_named(config: &mut StudioLightColors, selected: &str) -> Result<(), String> {
    let palette = config.presets.iter().find(|preset| preset.name == selected).ok_or_else(|| tl!("Choose a saved palette first.").to_string())?;
    config.colors = palette.colors.clone();
    Ok(())
}

fn remove_named(config: &mut StudioLightColors, selected: &str) -> Result<(), String> {
    let index = selected_index(config, selected).ok_or_else(|| tl!("Choose a saved palette first.").to_string())?;
    config.presets.remove(index);
    Ok(())
}

fn import_file(app: &mut PhotocraftApp, config: &mut StudioLightColors) -> Result<Option<String>, String> {
    let pick = app.services.pick_open_colors.as_mut().ok_or_else(|| tl!("Color palette file dialogs are unavailable on this platform.").to_string())?;
    let Some((_, bytes)) = pick() else { return Ok(None) };
    let bytes = bytes.map_err(|error| format!("{} {error}", tl!("Could not import colors:")))?;
    let file = ColorPaletteFile::from_json(&bytes).map_err(|error| format!("{} {error}", tl!("Could not import colors:")))?;
    // Parse and validate the entire file before touching the draft; do not import preferences or its library.
    let mut candidate = config.clone();
    candidate.colors = file.colors;
    candidate.validate()?;
    *config = candidate;
    Ok(Some(file.name))
}

fn export_filename(name: &str) -> String {
    let stem: String = name.chars().take(64).map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let stem = stem.trim_matches([' ', '.']);
    if stem.is_empty() { "InkPop-colors.json".into() } else { format!("InkPop-{stem}-colors.json") }
}

fn export_file(app: &mut PhotocraftApp, config: &StudioLightColors, name: &str) -> Result<bool, String> {
    let name = if name.trim().is_empty() { tl!("My Palette") } else { name };
    let file = ColorPaletteFile::new(name, config.colors.clone())?;
    let bytes = file.to_json()?;
    let suggested = export_filename(&file.name);
    let pick = app.services.pick_save_colors.as_mut().ok_or_else(|| tl!("Color palette file dialogs are unavailable on this platform.").to_string())?;
    let Some(path) = pick(&suggested) else { return Ok(false) };
    app.services.write.as_mut().ok_or_else(|| tl!("Color palette files cannot be written on this platform.").to_string())?(&path, &bytes)?;
    Ok(true)
}

fn set_message(fields: &mut Map<String, Value>, message: &str, error: bool) {
    fields.insert(MESSAGE.into(), json!(message));
    fields.insert(ERROR.into(), json!(error));
}

fn action_result<T>(fields: &mut Map<String, Value>, result: Result<T, String>, success: &str) -> Option<T> {
    match result {
        Ok(value) => {
            set_message(fields, success, false);
            Some(value)
        }
        Err(error) => {
            set_message(fields, &error, true);
            None
        }
    }
}

/// Color32 stores premultiplied RGB. The opaque swatch/hex editor represents
/// the source hue, while an absent override must keep the theme's original alpha.
fn editor_rgb(config: &StudioLightColors, slot: ColorSlot) -> [u8; 3] {
    let effective = crate::theme::palette::resolved_color(crate::theme::ThemeKind::StudioLight, config, slot);
    let [r, g, b, _] = effective.to_srgba_unmultiplied();
    [r, g, b]
}

fn builtin_name(palette: &BuiltinPalette) -> &'static str {
    // Literal keys let translation catalog checks find each built-in label.
    match palette.name {
        "Warm Gray Orange" => tl!("Warm Gray Orange"),
        "Mist Gray Lime" => tl!("Mist Gray Lime"),
        "Bright Blue Yellow" => tl!("Bright Blue Yellow"),
        "Porcelain Cobalt" => tl!("Porcelain Cobalt"),
        name => name,
    }
}

fn builtin_preview(ui: &mut egui::Ui, config: &StudioLightColors) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(egui::vec2(64.0, 14.0), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let palette = BUILTIN_PALETTES.iter().find(|palette| palette.matches(&config.colors));
        for (index, slot) in [ColorSlot::TitleBar, ColorSlot::Card, ColorSlot::Accent, ColorSlot::ProgressFill].into_iter().enumerate() {
            let [r, g, b] = palette.and_then(|palette| palette.color(slot)).map_or_else(|| editor_rgb(config, slot), PaletteRgb::rgb);
            let cell = egui::Rect::from_min_max(
                egui::pos2(rect.left() + rect.width() * index as f32 / 4.0, rect.top()),
                egui::pos2(rect.left() + rect.width() * (index + 1) as f32 / 4.0, rect.bottom()),
            );
            ui.painter().rect_filled(cell, 0.0, egui::Color32::from_rgb(r, g, b));
        }
        ui.painter().rect_stroke(rect, 0.0, egui::Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    }
}

fn color_swatch(ui: &mut egui::Ui, rgb: &mut [u8; 3]) -> egui::Response {
    let response = crate::color_picker_ui::edit_srgb(ui, rgb);
    // Keep the shared color well aligned with the
    // neighboring TextEdit's actual border instead, including interaction state.
    if ui.is_rect_visible(response.rect) {
        let visuals = ui.style().interact(&response);
        ui.painter().rect_stroke(response.rect, visuals.corner_radius.at_most(2), visuals.bg_stroke, egui::StrokeKind::Inside);
    }
    response
}

fn set_edited_color(config: &mut StudioLightColors, slot: ColorSlot, rgb: [u8; 3]) {
    config.colors.insert(slot, PaletteRgb::new(rgb));
    if slot == ColorSlot::TitleBar {
        config.colors.remove(&ColorSlot::MenuBar);
    }
}

fn reset_color(config: &mut StudioLightColors, slot: ColorSlot) {
    config.colors.remove(&slot);
    if slot == ColorSlot::TitleBar {
        config.colors.remove(&ColorSlot::MenuBar);
    }
}

/// One page inside Preferences > Interface. `interface` belongs to the dialog's cloned values.
pub(crate) fn editor(app: &mut PhotocraftApp, ui: &mut egui::Ui, interface: &mut Map<String, Value>, fields: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(tl!("Module Colors")).font(crate::theme::semibold(13.0)).color(t.text));
    ui.label(RichText::new(tl!("Custom colors apply only to Studio (Light).")).color(t.text_dim));
    ui.label(RichText::new(tl!("Click Apply to preview your colors. Cancel discards unapplied changes.")).small().color(t.text_faint));
    let active = interface.get("theme").and_then(Value::as_str) == Some("studioLight");
    if !active {
        if ui.button(tl!("Switch to Studio (Light)")).clicked() {
            interface.insert("theme".into(), json!("studioLight"));
        }
        ui.add_space(4.0);
    }
    let mut config = match read_config(interface) {
        Ok(config) => config,
        Err(error) => {
            ui.colored_label(t.warning, error);
            return;
        }
    };
    let before = config.clone();
    ui.add_enabled_ui(active, |ui| {
        let mut selected = fields.get(SELECTED).and_then(Value::as_str).unwrap_or("").to_string();
        let mut name = fields.get(NAME).and_then(Value::as_str).unwrap_or("").to_string();
        if !selected.is_empty() && selected_index(&config, &selected).is_none() {
            selected.clear();
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Built-in Palettes")).color(t.text_dim));
            let mut builtin = BUILTIN_PALETTES.iter().find(|palette| palette.matches(&config.colors)).map_or("", |palette| palette.id);
            let choices: Vec<(&str, &str)> =
                std::iter::once(("", tl!("Choose a palette…"))).chain(BUILTIN_PALETTES.iter().map(|palette| (palette.id, builtin_name(palette)))).collect();
            if crate::widgets::dropdown(ui, "studio-light-builtin-palettes", &mut builtin, &choices, 240.0)
                && let Some(palette) = BUILTIN_PALETTES.iter().find(|palette| palette.id == builtin)
            {
                config.colors = palette.colors();
                name = builtin_name(palette).to_string();
                selected.clear();
                // The selected name and preview strip already give feedback without
                // adding a message row that hides the lower colour controls.
                set_message(fields, "", false);
            }
            builtin_preview(ui, &config);
        });
        ui.horizontal(|ui| {
            ui.label(tl!("Saved Palettes"));
            let choices: Vec<(String, &str)> = std::iter::once((String::new(), tl!("Choose a palette…")))
                .chain(config.presets.iter().map(|preset| (preset.name.clone(), preset.name.as_str())))
                .collect();
            crate::widgets::dropdown(ui, "studio-light-saved-palettes", &mut selected, &choices, 180.0);
            let has_selected = selected_index(&config, &selected).is_some();
            if ui.add_enabled(has_selected, egui::Button::new(tl!("Load Palette"))).clicked()
                && action_result(fields, load_named(&mut config, &selected), tl!("Palette loaded into the draft.")).is_some()
            {
                name = selected.clone();
            }
            if ui.add_enabled(has_selected, egui::Button::new(tl!("Delete Saved"))).clicked()
                && action_result(fields, remove_named(&mut config, &selected), tl!("Saved palette deleted from the draft.")).is_some()
            {
                selected.clear();
            }
        });
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut name).hint_text(tl!("Palette name")).desired_width(180.0).char_limit(64));
            if ui.button(tl!("Save New Palette")).clicked()
                && let Some(saved) = action_result(fields, save_named(&mut config, &name), tl!("Palette saved in the draft."))
            {
                selected = saved;
            }
            if ui
                .add_enabled(selected_index(&config, &selected).is_some(), egui::Button::new(tl!("Replace Selected")))
                .on_hover_text(tl!("Replace the selected saved palette with the current colors."))
                .clicked()
            {
                let _ = action_result(fields, replace_named(&mut config, &selected), tl!("Selected palette replaced in the draft."));
            }
        });
        ui.horizontal(|ui| {
            if ui.button(tl!("Import Colors…")).clicked() {
                match import_file(app, &mut config) {
                    Ok(Some(imported)) => {
                        name = imported;
                        selected.clear();
                        set_message(fields, tl!("Colors imported into the draft."), false);
                    }
                    Ok(None) => {}
                    Err(error) => set_message(fields, &error, true),
                }
            }
            if ui.button(tl!("Export Colors…")).clicked() {
                match export_file(app, &config, &name) {
                    Ok(true) => set_message(fields, tl!("Colors exported."), false),
                    Ok(false) => {}
                    Err(error) => set_message(fields, &error, true),
                }
            }
            if ui.button(tl!("Restore Default Colors")).clicked() {
                config.colors.clear();
                set_message(fields, tl!("Default colors restored in the draft. Saved palettes are retained."), false);
            }
        });
        fields.insert(SELECTED.into(), json!(selected));
        fields.insert(NAME.into(), json!(name));
        if let Some(message) = fields.get(MESSAGE).and_then(Value::as_str).filter(|message| !message.is_empty()) {
            let error = fields.get(ERROR).and_then(Value::as_bool).unwrap_or(false);
            ui.label(RichText::new(message).small().color(if error { t.warning } else { t.text_dim }));
        }
        ui.separator();
        let mut category = fields.get(CATEGORY).and_then(Value::as_u64).unwrap_or(0).min((GROUPS.len() - 1) as u64).to_string();
        ui.horizontal(|ui| {
            ui.label(tl!("Module"));
            let choices: Vec<(String, &str)> = GROUPS.iter().enumerate().map(|(index, group)| (index.to_string(), tl!(group.title))).collect();
            crate::widgets::dropdown(ui, "studio-light-color-module", &mut category, &choices, 300.0);
        });
        let index = category.parse::<usize>().unwrap_or(0);
        fields.insert(CATEGORY.into(), json!(index));
        if let Some(group) = GROUPS.get(index) {
            egui::Grid::new("studio-light-module-colors").num_columns(4).spacing([12.0, 6.0]).show(ui, |ui| {
                for &(slot, label) in group.colors {
                    ui.label(tl!(label));
                    let rgb = editor_rgb(&config, slot);
                    let mut edited = rgb;
                    ui.push_id(slot.id(), |ui| {
                        color_swatch(ui, &mut edited);
                    });
                    // Grid cells initially expose only their previous minimum width.
                    // Give the single-line editor its own horizontal allocation so its
                    // 72pt text width plus frame padding never collapses to 24pt.
                    ui.allocate_ui_with_layout(egui::vec2(86.0, 24.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.set_min_width(86.0);
                        crate::prefs_ui::hex_field(ui, slot.id(), &mut edited);
                    });
                    if edited != rgb {
                        set_edited_color(&mut config, slot, edited);
                    }
                    if ui
                        .add_enabled(
                            config.colors.contains_key(&slot) || (slot == ColorSlot::TitleBar && config.colors.contains_key(&ColorSlot::MenuBar)),
                            egui::Button::new(tl!("Default")),
                        )
                        .on_hover_text(tl!("Restore this color to its theme default."))
                        .clicked()
                    {
                        reset_color(&mut config, slot);
                    }
                    ui.end_row();
                }
            });
        }
    });
    if config != before
        && let Err(error) = store_config(interface, &config)
    {
        set_message(fields, &error, true);
    }
}

#[cfg(test)]
#[path = "color_palette_ui_tests_v68.rs"]
mod tests;
