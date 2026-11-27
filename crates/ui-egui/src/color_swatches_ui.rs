//! The same editable, persistent colour library below the colour wheel and in the Swatches tab.
use crate::theme::palette::ScrollAreaPaletteExt;
use crate::{PhotocraftApp, theme::Tokens};
use egui::{Color32, Sense, Stroke, StrokeKind, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SwatchUi {
    pub editor: Option<SwatchEditor>,
    pub import_open: bool,
    pub import_text: String,
    pub error: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SwatchEditor {
    /// add, update, newGroup, renameGroup.
    pub action: String,
    pub group: String,
    pub original: String,
    pub name: String,
    pub color: [f32; 4],
}

fn dispatch(app: &mut PhotocraftApp, id: &str, params: Value) -> bool {
    match app.run(id, params) {
        Ok(_) => {
            app.ui.color_swatches.error.clear();
            true
        }
        Err(error) => {
            app.ui.status = error.clone();
            app.ui.color_swatches.error = error;
            false
        }
    }
}

fn unique_name(app: &PhotocraftApp, groups: bool) -> String {
    let library = &app.session.presets.swatches;
    let base = if groups { tl!("Group") } else { tl!("Color") };
    for i in 1..=4097 {
        let name = format!("{base} {i}");
        let taken = if groups {
            library.groups.iter().any(|g| g.name == name)
        } else {
            library.groups.iter().filter(|g| g.name == library.current).any(|g| g.items.iter().any(|s| s.name == name))
        };
        if !taken {
            return name;
        }
    }
    base.into()
}

fn start_editor(app: &mut PhotocraftApp, action: &str, original: &str, color: [f32; 4]) {
    let name = if original.is_empty() { unique_name(app, action == "newGroup") } else { original.into() };
    app.ui.color_swatches.error.clear();
    app.ui.color_swatches.editor =
        Some(SwatchEditor { action: action.into(), group: app.session.presets.swatches.current.clone(), original: original.into(), name, color });
}

pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.push_id("editable-color-swatches", |ui| {
        let library = &app.session.presets.swatches;
        let current = library.current.clone();
        let names: Vec<_> = library.groups.iter().map(|g| g.name.clone()).collect();
        let items = library.groups.iter().find(|g| g.name == current).map(|g| g.items.clone()).unwrap_or_default();
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let mut selected = current.clone();
            egui::ComboBox::from_id_salt("swatch-group").width((ui.available_width() - 68.0).max(80.0)).selected_text(&current).show_ui(ui, |ui| {
                for name in &names {
                    ui.selectable_value(&mut selected, name.clone(), name);
                }
            });
            if selected != current {
                dispatch(app, "swatch.presets.edit", json!({"action":"selectGroup","group":selected}));
            }
            if ui.small_button("+").on_hover_text(tl!("Save Current Color")).clicked() {
                let c = if app.ui.color_panel.background { app.session.tools.background } else { app.session.tools.foreground };
                start_editor(app, "add", "", c);
            }
            ui.menu_button("...", |ui| {
                if ui.button(tl!("New Group")).clicked() {
                    start_editor(app, "newGroup", "", app.session.tools.foreground);
                    ui.close();
                }
                if ui.button(tl!("Rename Group")).clicked() {
                    start_editor(app, "renameGroup", &current, app.session.tools.foreground);
                    ui.close();
                }
                // Delete only empty groups directly; populated groups remain safe from an accidental click.
                if ui.add_enabled(names.len() > 1 && items.is_empty(), egui::Button::new(tl!("Delete Group"))).clicked() {
                    dispatch(app, "swatch.presets.edit", json!({"action":"deleteGroup","group":current}));
                    ui.close();
                }
                ui.separator();
                if ui.button(tl!("Copy Swatch Group")).clicked() {
                    if let Some(group) = app.session.presets.swatches.groups.iter().find(|g| g.name == current) {
                        ui.ctx().copy_text(json!({"groups":[group],"current":current}).to_string());
                    }
                    ui.close();
                }
                if ui.button(tl!("Import Swatch Group")).clicked() {
                    app.ui.color_swatches.import_open = true;
                    app.ui.color_swatches.error.clear();
                    ui.close();
                }
            });
        });
        let tokens = Tokens::get(ui.ctx());
        egui::ScrollArea::vertical().id_salt("swatch-colors").max_height(112.0).show_palette(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(5.0, 5.0);
            ui.horizontal_wrapped(|ui| {
                for swatch in &items {
                    let [r, g, b] = crate::panels::srgb_bytes(swatch.color);
                    let label = format!("{}  #{r:02X}{g:02X}{b:02X}", swatch.name);
                    let (rect, response) = ui.allocate_exact_size(vec2(25.0, 25.0), Sense::click());
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
                    ui.painter().rect_filled(rect, tokens.radius_sm, Color32::from_rgb(r, g, b));
                    ui.painter().rect_stroke(
                        rect,
                        tokens.radius_sm,
                        Stroke::new(if response.hovered() { 2.0 } else { 1.0 }, if response.hovered() { tokens.text } else { tokens.card_border }),
                        StrokeKind::Inside,
                    );
                    if response.clicked()
                        && dispatch(
                            app,
                            "swatch.presets.select",
                            json!({"group":current,"preset":swatch.name,
                        "target":if app.ui.color_panel.background { "background" } else { "foreground" }}),
                        )
                        && !app.ui.color_panel.background
                    {
                        crate::type_tool::foreground_changed(app);
                    }
                    response.context_menu(|ui| {
                        if ui.button(tl!("Edit Color")).clicked() {
                            start_editor(app, "update", &swatch.name, swatch.color);
                            ui.close();
                        }
                        if ui.button(tl!("Set background color")).clicked() {
                            dispatch(app, "swatch.presets.select", json!({"group":current,"preset":swatch.name,"target":"background"}));
                            ui.close();
                        }
                        ui.menu_button(tl!("Move to Group"), |ui| {
                            for name in names.iter().filter(|name| **name != current) {
                                if ui.button(name).clicked() {
                                    dispatch(app, "swatch.presets.edit", json!({"action":"move","group":current,"preset":swatch.name,"to":name}));
                                    ui.close();
                                }
                            }
                        });
                        if ui.button(tl!("Delete")).clicked() {
                            dispatch(app, "swatch.presets.edit", json!({"action":"delete","group":current,"preset":swatch.name}));
                            ui.close();
                        }
                    });
                    response.on_hover_text(label);
                }
                if items.is_empty() {
                    ui.label(egui::RichText::new(tl!("Save colors with +")).small().color(tokens.text_dim));
                }
            });
        });
        editor(app, ui);
        if !app.ui.color_swatches.error.is_empty() {
            ui.label(egui::RichText::new(&app.ui.color_swatches.error).small().color(ui.visuals().error_fg_color));
        }
    });
}

fn editor(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if let Some(mut edit) = app.ui.color_swatches.editor.clone() {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(tl!("Name"));
            ui.add(egui::TextEdit::singleline(&mut edit.name).char_limit(128).desired_width((ui.available_width() - 8.0).max(60.0)));
        });
        if matches!(edit.action.as_str(), "add" | "update") {
            ui.horizontal(|ui| {
                ui.label(tl!("Color"));
                // egui's floating-point editor expects linear RGB; tool colours are encoded sRGB.
                let [r, g, b, a] = edit.color;
                let mut linear = [r, g, b].map(photocraft_color::convert::srgb_to_linear);
                let mut rgba = [linear[0], linear[1], linear[2], a];
                if crate::color_picker_ui::edit_rgba_unmultiplied(ui, &mut rgba).changed() {
                    linear = [rgba[0], rgba[1], rgba[2]].map(photocraft_color::convert::linear_to_srgb);
                    edit.color = [linear[0].clamp(0.0, 1.0), linear[1].clamp(0.0, 1.0), linear[2].clamp(0.0, 1.0), rgba[3].clamp(0.0, 1.0)];
                }
            });
        }
        let mut close = false;
        ui.horizontal(|ui| {
            if ui.button(tl!("Save")).clicked() {
                close = dispatch(
                    app,
                    "swatch.presets.edit",
                    json!({"action":edit.action,"group":edit.group,"preset":edit.original,"name":edit.name,"color":edit.color}),
                );
            }
            if ui.button(tl!("Cancel")).clicked() {
                close = true;
                app.ui.color_swatches.error.clear();
            }
        });
        app.ui.color_swatches.editor = if close { None } else { Some(edit) };
    }
    if app.ui.color_swatches.import_open {
        ui.separator();
        ui.label(tl!("Paste Swatch Group JSON"));
        ui.add(egui::TextEdit::multiline(&mut app.ui.color_swatches.import_text).char_limit(1_048_576).desired_rows(3).desired_width(ui.available_width()));
        ui.horizontal(|ui| {
            if ui.button(tl!("Import")).clicked() {
                match serde_json::from_str::<Value>(&app.ui.color_swatches.import_text) {
                    Ok(library) => {
                        if dispatch(app, "swatch.presets.edit", json!({"action":"import","library":library})) {
                            app.ui.color_swatches.import_open = false;
                            app.ui.color_swatches.import_text.clear();
                        }
                    }
                    Err(error) => app.ui.color_swatches.error = error.to_string(),
                }
            }
            if ui.button(tl!("Cancel")).clicked() {
                app.ui.color_swatches.import_open = false;
                app.ui.color_swatches.error.clear();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    #[test]
    fn saved_colors_are_visible_selectable_and_editable_under_the_wheel() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("swatch.presets.edit", json!({"action":"newGroup","name":"Studio"})).unwrap();
        app.run("swatch.presets.edit", json!({"action":"add","name":"Accent","color":"#cc8844"})).unwrap();
        let mut h = Harness::builder().with_size(vec2(320., 600.)).build_ui_state(|ui, app: &mut PhotocraftApp| crate::color_panel_ui::show(app, ui), app);
        h.run_steps(2);
        let chip = h.get_by_label("Accent  #CC8844");
        let wheel = h.ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new("color-wheel-rect"))).unwrap();
        assert!(chip.rect().top() > wheel.bottom());
        chip.click();
        h.run_steps(2);
        assert_eq!(h.state().session.tools.foreground, [204.0 / 255.0, 136.0 / 255.0, 68.0 / 255.0, 1.0]);
        h.get_by_label("+").click();
        h.run_steps(2);
        assert!(h.state().ui.color_swatches.editor.is_some());
        h.get_by_label(tl!("Save")).click();
        h.run_steps(2);
        assert!(h.state().ui.color_swatches.editor.is_none());
        assert_eq!(h.state().session.presets.swatches.groups.last().unwrap().items.len(), 2);
    }
}
