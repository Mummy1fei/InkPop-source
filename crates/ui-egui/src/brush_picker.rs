//! Photoshop's Brush Preset picker: the options-bar brush chip's popup and the picker a
//! right-click on the canvas opens (`paint_mouse`). It is the quick picker (Size, Hardness and the
//! preset library in its groups); the full editor is the Brush Settings panel (F5), which the
//! picker and the options bar open with one click (#258).
//!
//! The picker edits a copy of the brush; the caller sends the size and hardness edits through
//! `tools.setBrush` ([`crate::brush_panel::commit_gesture`]) and applies the returned [`Pick`]
//! with [`apply`], so every change is a journaled command (Rule 1).

use crate::theme::palette::ScrollAreaPaletteExt;
use std::collections::BTreeSet;

use egui::{Color32, RichText, Sense, Stroke, pos2, vec2};
use photocraft_engine::BrushSettings;
use photocraft_engine::paint::{BrushPreset, MAX_BRUSH_SIZE};
use serde_json::json;

use crate::brush_panel::{full_uv, grouped_presets, is_current, run_or_status};
use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, brush_preview, icons, widgets};

mod native;
pub(crate) use native::{close, inspect, maintain, native_viewport, open_at, resize_native, show as show_native};

/// Width of the picker's contents.
pub const WIDTH: f32 = 300.0;
/// A preset cell in the picker's grid.
const CELL: egui::Vec2 = vec2(44.0, 50.0);

/// What a click in the picker asks for beyond the size and hardness edits.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// Pick a preset (`tools.setBrush {"preset": name}`).
    Preset(String),
    /// Open the Brush Settings panel (Window › Brush Settings, F5).
    OpenSettings,
    /// Save the current brush as a new preset.
    NewPreset,
}

/// Picker view state kept in egui memory. The folder state lives in preferences.
#[derive(Clone, Debug)]
struct View {
    filter: String,
    list: bool,
}

impl Default for View {
    fn default() -> Self {
        Self { filter: String::new(), list: true }
    }
}

fn view_id() -> egui::Id {
    egui::Id::new("brush-picker-view")
}

/// Run what the picker asked for.
pub fn apply(app: &mut PhotocraftApp, ctx: &egui::Context, pick: Option<Pick>) {
    match pick {
        None => {}
        Some(Pick::Preset(name)) => run_or_status(app, "tools.setBrush", json!({ "preset": name, "summary": true })),
        Some(Pick::OpenSettings) => open_settings(app, ctx),
        Some(Pick::NewPreset) => {
            let brush = app.session.tools.brush.clone();
            crate::brush_panel::open_new_preset(app, &brush);
        }
    }
}

/// Show the Brush Settings panel on its settings tab (what F5 does when it is hidden).
pub fn open_settings(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.ui.panels.brush_settings {
        app.ui.brush_tab = 0;
        crate::dock::reveal(app, crate::dock::Group::Brush);
        return;
    }
    if let Err(e) = crate::menus::invoke(app, ctx, "window.panel.brushSettings", json!({})) {
        app.ui.status = e;
    }
}

/// The options-bar button beside the brush chip that shows and hides the Brush Settings panel
/// (Photoshop's "Toggle the Brush Settings panel"). It runs Window › Brush Settings (F5).
pub fn settings_toggle(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let on = app.ui.panels.brush_settings;
    if named(icons::button(ui, "sliders-horizontal", 24.0, on, "Toggle the Brush Settings panel  (F5)"), "Toggle the Brush Settings panel").clicked()
        && let Err(e) = crate::menus::invoke(app, ui.ctx(), "window.panel.brushSettings", json!({}))
    {
        app.ui.status = e;
    }
}

/// An icon button's accessible name (icon buttons have only a tooltip).
fn named(resp: egui::Response, label: &str) -> egui::Response {
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    resp
}

/// The picker's contents: Size, Hardness, a search field with the Brush Settings and New Preset
/// buttons, and the presets in their groups as tip thumbnails.
pub(crate) fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, b: &mut BrushSettings) -> Option<Pick> {
    body_sized(app, ui, b, WIDTH, 300.0)
}

pub(crate) fn body_sized(app: &mut PhotocraftApp, ui: &mut egui::Ui, b: &mut BrushSettings, width: f32, list_height: f32) -> Option<Pick> {
    let saved = &app.session.prefs().tools.brush_picker_collapsed_groups;
    let mut collapsed = saved.clone();
    let pick = draw_body(ui, b, &app.session.tools.presets, &mut collapsed, width, list_height);
    if collapsed != *saved {
        app.session.prefs.edit(|prefs| prefs.tools.brush_picker_collapsed_groups = collapsed);
        // In a detached picker, the root's preference writer still needs a frame.
        ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
    }
    pick
}

fn draw_body(
    ui: &mut egui::Ui,
    b: &mut BrushSettings,
    presets: &[BrushPreset],
    collapsed: &mut BTreeSet<String>,
    width: f32,
    list_height: f32,
) -> Option<Pick> {
    let t = Tokens::get(ui.ctx());
    let mut pick = None;
    ui.set_width(width.max(240.0));
    // Size: value field plus a logarithmic slider (small sizes get most of the travel).
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Size")).color(t.text_dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut size = b.size;
            if widgets::value_field(ui, &mut size, 1.0..=MAX_BRUSH_SIZE, "px", 72.0).changed() {
                b.size = size.round().clamp(1.0, MAX_BRUSH_SIZE);
            }
        });
    });
    let mut lv = b.size.max(1.0).ln();
    if widgets::slider(ui, &mut lv, 0.0..=MAX_BRUSH_SIZE.ln(), None).changed() {
        b.size = lv.exp().round().clamp(1.0, MAX_BRUSH_SIZE);
    }
    let mut hard = b.hardness * 100.0;
    if widgets::slider_row(ui, "Hardness", &mut hard, 0.0..=100.0, "%", None).changed() {
        b.hardness = (hard / 100.0).clamp(0.0, 1.0);
    }
    ui.add_space(6.0);
    widgets::hairline(ui);
    ui.add_space(6.0);
    let mut view = ui.data(|d| d.get_temp::<View>(view_id())).unwrap_or_default();
    ui.horizontal(|ui| {
        icons::paint(ui, egui::Rect::from_min_size(ui.cursor().min + vec2(0.0, 3.0), vec2(16.0, 16.0)), "search", 14.0, t.text_faint);
        ui.add_space(20.0);
        ui.add(
            egui::TextEdit::singleline(&mut view.filter)
                .hint_text(tl!("Search Brushes"))
                .desired_width((width - 146.0).max(80.0))
                .id_salt("brush-picker-search"),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if named(icons::button(ui, "grid-2x2", 24.0, !view.list, "Grid view"), "Grid view").clicked() {
                view.list = false;
            }
            if named(icons::button(ui, "align-justify", 24.0, view.list, "List view"), "List view").clicked() {
                view.list = true;
            }
            if named(icons::button(ui, "square-plus", 24.0, false, "Create new brush preset from the current settings"), "New Brush Preset").clicked() {
                pick = Some(Pick::NewPreset);
            }
            if named(icons::button(ui, "sliders-horizontal", 24.0, false, "Brush Settings…  (F5): every option of this brush"), "Brush Settings…").clicked()
            {
                pick = Some(Pick::OpenSettings);
            }
        });
    });
    ui.add_space(4.0);
    let filter = view.filter.trim().to_lowercase();
    egui::ScrollArea::vertical().id_salt("brush-picker-presets").max_height(list_height.max(60.0)).auto_shrink([false, false]).show_palette(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        for (label, items) in grouped_presets(presets) {
            let items: Vec<&BrushPreset> =
                items.into_iter().filter_map(|i| presets.get(i)).filter(|p| filter.is_empty() || p.name.to_lowercase().contains(&filter)).collect();
            if items.is_empty() {
                continue;
            }
            let mut open = !filter.is_empty() || !collapsed.contains(&label);
            if group_header(ui, &label, open, items.len()) && filter.is_empty() {
                open = !open;
                if open {
                    collapsed.remove(&label);
                } else {
                    collapsed.insert(label);
                }
            }
            if !open {
                continue;
            }
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
                let width = stroke_card_width(ui.available_width());
                for p in items {
                    if cell(ui, p, is_current(&p.brush, b), view.list, width) {
                        pick = Some(Pick::Preset(p.name.clone()));
                    }
                }
            });
            ui.add_space(3.0);
        }
        if presets.is_empty() {
            ui.label(RichText::new(tl!("No brush presets")).color(t.text_faint));
        }
    });
    ui.data_mut(|d| d.insert_temp(view_id(), view));
    pick
}

/// A group's header row: chevron, folder, name, count. Returns true when clicked (toggle).
fn group_header(ui: &mut egui::Ui, label: &str, open: bool, count: usize) -> bool {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, t.radius_sm, t.hover);
    }
    let x = r.left() + 2.0;
    let chevron = if open { "chevron-down" } else { "chevron-right" };
    icons::paint(ui, egui::Rect::from_min_size(pos2(x, r.center().y - 7.0), vec2(14.0, 14.0)), chevron, 12.0, t.text_dim);
    let folder = if open { "folder-open" } else { "folder" };
    icons::paint(ui, egui::Rect::from_min_size(pos2(x + 16.0, r.center().y - 8.0), vec2(16.0, 16.0)), folder, 14.0, t.icon);
    ui.painter().text(pos2(x + 37.0, r.center().y), egui::Align2::LEFT_CENTER, label, theme::semibold(12.0), t.text);
    ui.painter().text(pos2(r.right() - 6.0, r.center().y), egui::Align2::RIGHT_CENTER, count.to_string(), theme::medium(11.0), t.text_faint);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, true, label));
    resp.clicked()
}

/// One preset: its tip thumbnail (scaled by size) and its size. Returns true when clicked.
fn cell(ui: &mut egui::Ui, p: &BrushPreset, current: bool, list: bool, width: f32) -> bool {
    if list {
        return stroke_card(ui, p, current, width, Sense::click()).clicked();
    }
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(CELL, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, current, &p.name));
    if !ui.is_rect_visible(r) {
        return resp.clicked();
    }
    if current {
        ui.painter().rect_filled(r, 3.0, t.accent_soft);
        ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 3.0, t.hover);
    } else {
        ui.painter().rect_filled(r, 3.0, t.field);
    }
    let pb = &p.brush;
    // The Brushes tab's thumbnail slot, so both share one cached texture per preset.
    let tip = brush_preview::tip_texture(ui.ctx(), &format!("brushes-tip:{}", p.name), &pb.tip, (pb.hardness, pb.angle, pb.roundness), 36, t.text);
    let ir = egui::Rect::from_center_size(pos2(r.center().x, r.top() + 20.0), vec2(32.0, 32.0) * crate::brush_sections::thumb_scale(pb.size));
    ui.painter().image(tip.id(), ir, full_uv(), Color32::WHITE);
    ui.painter().text(
        pos2(r.center().x, r.bottom() - 7.0),
        egui::Align2::CENTER_CENTER,
        format!("{}", pb.size.round() as i64),
        egui::FontId::proportional(9.5),
        t.text_dim,
    );
    resp.on_hover_text(&p.name).clicked()
}

pub(crate) const STROKE_CARD_HEIGHT: f32 = 62.0;
pub(crate) const STROKE_CARD_GAP: f32 = 3.0;

pub(crate) fn stroke_card_columns(width: f32) -> usize {
    if !width.is_finite() {
        return 1;
    }
    (((width + STROKE_CARD_GAP) / 223.0).floor() as usize).clamp(1, 12)
}

pub(crate) fn stroke_card_width(width: f32) -> f32 {
    let columns = stroke_card_columns(width);
    ((width - STROKE_CARD_GAP * columns.saturating_sub(1) as f32) / columns as f32).max(80.0)
}

/// Shared preset card: the stroke owns the upper region; its name uses one full-width bottom
/// line. Keep the same response for the Brushes panel's selection and drag/drop behavior.
pub(crate) fn stroke_card(ui: &mut egui::Ui, p: &BrushPreset, current: bool, width: f32, sense: Sense) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(width, STROKE_CARD_HEIGHT), sense);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, current, &p.name));
    if ui.is_rect_visible(r) {
        let painter = ui.painter().with_clip_rect(r.intersect(ui.clip_rect()));
        painter.rect_filled(
            r,
            t.radius_sm,
            if current {
                t.accent_soft
            } else if resp.hovered() {
                t.hover
            } else {
                t.field
            },
        );
        if current {
            painter.rect_stroke(r, t.radius_sm, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
        }
        let pb = &p.brush;
        let tip = brush_preview::tip_texture(ui.ctx(), &format!("brushes-tip:{}", p.name), &pb.tip, (pb.hardness, pb.angle, pb.roundness), 36, t.text);
        let tip_rect = egui::Rect::from_center_size(r.min + vec2(19.0, 19.0), vec2(26.0, 26.0) * crate::brush_sections::thumb_scale(pb.size));
        painter.image(tip.id(), tip_rect, full_uv(), Color32::WHITE);
        painter.text(r.min + vec2(5.0, 40.0), egui::Align2::LEFT_BOTTOM, format!("{}", pb.size.round() as i64), egui::FontId::proportional(9.0), t.text_dim);
        let stroke = brush_preview::stroke_texture(ui.ctx(), &format!("brushes-stroke:{}", p.name), pb, 256, 44, t.text);
        let stroke_rect = egui::Rect::from_min_max(r.min + vec2(39.0, 3.0), pos2(r.right() - 5.0, r.top() + 42.0));
        painter.image(stroke.id(), stroke_rect, full_uv(), Color32::WHITE);
        let mut name = egui::text::LayoutJob::simple_singleline(p.name.clone(), egui::FontId::proportional(11.0), t.text);
        name.wrap = egui::text::TextWrapping::truncate_at_width((r.width() - 10.0).max(0.0));
        let galley = painter.layout_job(name);
        painter.galley(pos2(r.left() + 5.0, r.bottom() - 4.0 - galley.size().y), galley, t.text);
    }
    resp.on_hover_text(&p.name)
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use std::sync::{Arc, Mutex};

    fn picker_on(store: &Arc<Mutex<Option<String>>>) -> Harness<'static, PhotocraftApp> {
        let (reader, writer) = (store.clone(), store.clone());
        let services = crate::Services {
            load_prefs: Some(Box::new(move || reader.lock().unwrap().clone())),
            save_prefs: Some(Box::new(move |text| {
                *writer.lock().unwrap() = Some(text.to_owned());
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        app.ui.tool = crate::state::Tool::Brush;
        app.session.tools.presets = [("QA Collection A", "QA Tip A"), ("QA Collection B", "QA Tip B"), ("QA Collection C", "QA Tip C")]
            .into_iter()
            .map(|(group, name)| BrushPreset { name: name.into(), group: group.into(), brush: BrushSettings::default(), builtin: false })
            .collect();
        let h = Harness::builder().with_size(vec2(1000., 700.)).build_ui_state(
            |ui, app| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                    crate::paint_mouse::show_picker(app, ui.ctx());
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h
    }

    fn open(h: &mut Harness<'static, PhotocraftApp>) {
        crate::paint_mouse::pointer_secondary(h.state_mut(), true, egui::Modifiers::NONE, [30.0, 30.0]);
        h.run_steps(4);
    }

    #[test]
    fn right_click_picker_folders_restore_on_reopen_and_restart() {
        let store = Arc::new(Mutex::new(Some(r#"{"tools":{"brushPickerSize":[450,550]}}"#.to_owned())));
        let mut h = picker_on(&store);
        open(&mut h);
        for name in ["QA Tip A", "QA Tip B", "QA Tip C"] {
            assert!(h.query_by_label(name).is_some(), "legacy preferences default to open for {name}");
        }

        h.get_by_label("QA Collection A").click();
        h.run_steps(3);
        h.get_by_label("QA Collection C").click();
        h.run_steps(3);
        let check = |h: &Harness<'static, PhotocraftApp>| {
            assert!(h.query_by_label("QA Tip A").is_none());
            assert!(h.query_by_label("QA Tip B").is_some());
            assert!(h.query_by_label("QA Tip C").is_none());
        };
        check(&h);

        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(h.state().ui.brush_picker.is_none());
        open(&mut h);
        check(&h);
        assert_eq!(crate::prefs_ui::persist(h.state_mut(), f64::MAX), None);
        let json: serde_json::Value = serde_json::from_str(store.lock().unwrap().as_deref().unwrap()).unwrap();
        assert_eq!(json["tools"]["brushPickerCollapsedGroups"], json!(["QA Collection A", "QA Collection C"]));
        assert_eq!(json["tools"]["brushPickerSize"][0].as_f64(), Some(450.0));
        assert_eq!(json["tools"]["brushPickerSize"][1].as_f64(), Some(550.0));

        let mut restored = picker_on(&store);
        open(&mut restored);
        check(&restored);
        restored.get_by_label("QA Collection A").click();
        restored.run_steps(3);
        assert!(restored.query_by_label("QA Tip A").is_some());
        assert_eq!(crate::prefs_ui::persist(restored.state_mut(), f64::MAX), None);
        let mut reopened = picker_on(&store);
        open(&mut reopened);
        assert!(reopened.query_by_label("QA Tip A").is_some());
        assert!(reopened.query_by_label("QA Tip B").is_some());
        assert!(reopened.query_by_label("QA Tip C").is_none());
    }

    #[test]
    fn picker_switches_between_tip_grid_and_stroke_list() {
        let session = photocraft_engine::Session::new();
        let presets = session.tools.presets.clone();
        let mut h = Harness::builder().with_size(vec2(800., 700.)).build_ui_state(
            move |ui, brush: &mut BrushSettings| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                    return;
                }
                draw_body(ui, brush, &presets, &mut BTreeSet::new(), 750., 550.);
            },
            session.tools.brush.clone(),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(3);
        assert!(h.ctx.data(|d| d.get_temp::<View>(view_id())).unwrap().list);
        h.get_by_label("Grid view").click();
        h.run_steps(3);
        assert!(!h.ctx.data(|d| d.get_temp::<View>(view_id())).unwrap().list);
        h.get_by_label("List view").click();
        h.run_steps(3);
        assert!(h.ctx.data(|d| d.get_temp::<View>(view_id())).unwrap().list);
    }

    #[test]
    fn stroke_cards_use_available_width_without_a_name_sidebar() {
        for (width, columns) in [(300.0, 1), (600.0, 2), (940.0, 4)] {
            let presets: Vec<_> = (0..8)
                .map(|i| BrushPreset {
                    name: format!("A very long brush preset name with texture {i}"),
                    brush: Default::default(),
                    builtin: false,
                    group: "Example".into(),
                })
                .collect();
            let names: Vec<_> = presets.iter().map(|p| p.name.clone()).collect();
            let mut h = Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(
                move |ui, brush: &mut BrushSettings| {
                    if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                        draw_body(ui, brush, &presets, &mut BTreeSet::new(), width, 600.0);
                    }
                },
                BrushSettings::default(),
            );
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
            h.run_steps(3);
            let rects: Vec<_> = names.iter().take(columns + 1).map(|name| h.get_by_label(name).rect()).collect();
            for r in &rects[..columns] {
                assert!((r.top() - rects[0].top()).abs() < 1.0, "{width}: expected {columns} columns, got {rects:?}");
                assert!((r.height() - STROKE_CARD_HEIGHT).abs() < 1.0);
            }
            assert!(rects[columns].top() > rects[0].bottom(), "next card wraps to a new row");
            assert!(rects[columns - 1].right() - rects[0].left() <= width + 1.0);
        }
    }
}
