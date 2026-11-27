//! Chrome around the canvas: title bar, options bar, toolbar, status bar, dock cards, Properties.

use crate::theme::palette::ScrollAreaPaletteExt;
use egui::emath::GuiRounding as _;
use egui::{Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use photocraft_color::BlendMode;
use photocraft_doc::{Layer, LayerContent, LayerId};
use serde_json::{Value, json};

use crate::state::Tool;
use crate::theme::palette;
use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, icons, widgets};
use photocraft_engine::prefs::palette::ColorSlot;

// ----------------------------------------------------------------------------- toolbar

/// Tool groups separated by hairlines, Photoshop order.
/// Photoshop toolbar: divider-separated sections of slots; each slot is a flyout group whose button
/// shows the group's last-used tool.
const TOOL_SECTIONS: &[&[&[Tool]]] = &[
    &[&[Tool::Move]],
    &[
        &[Tool::RectMarquee, Tool::EllipseMarquee],
        &[Tool::Lasso, Tool::PolygonLasso, Tool::MagneticLasso],
        &[Tool::ObjectSelection, Tool::QuickSelection, Tool::MagicWand],
        &[Tool::Crop, Tool::Slice, Tool::SliceSelect],
        &[Tool::Eyedropper, Tool::Ruler, Tool::Note, Tool::Count],
    ],
    &[
        &[Tool::SpotHealing, Tool::Healing, Tool::Patch, Tool::ContentAwareMove],
        &[Tool::Brush, Tool::Pencil, Tool::MixerBrush],
        &[Tool::CloneStamp],
        &[Tool::HistoryBrush],
        &[Tool::Eraser, Tool::BackgroundEraser, Tool::MagicEraser],
        &[Tool::Gradient, Tool::PaintBucket],
        &[Tool::Blur, Tool::Sharpen, Tool::Smudge],
        &[Tool::Dodge, Tool::Burn, Tool::Sponge],
    ],
    &[
        &[Tool::Pen],
        &[Tool::Type, Tool::VerticalType],
        &[Tool::PathSelection, Tool::DirectSelection],
        &[Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Line, Tool::CustomShape],
    ],
    &[&[Tool::Hand, Tool::RotateView], &[Tool::Zoom]],
];

/// The tool a slot shows: the current tool if it belongs to the slot, else the last one used.
fn slot_tool(state: &crate::UiState, current: Tool, slot: &[Tool]) -> Tool {
    crate::tool_group_memory::displayed(state, current, slot)
}

pub fn toolbar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::tool_group_memory::remember(&mut app.ui);
    let t = Tokens::get(ui.ctx());
    let (w1, bx, m) = if t.pro { (40.0, 30.0, 5i8) } else { (50.0, 36.0, 7i8) };
    // Photoshop switches to a double-column toolbar only when one column doesn't fit.
    let slots: usize = TOOL_SECTIONS.iter().map(|g| g.len()).sum();
    let double = toolbar_needs_double(slots, TOOL_SECTIONS.len(), bx, t.pro, ui.available_rect_before_wrap().height());
    let w = if double { w1 + bx + 2.0 } else { w1 };
    egui::Panel::left("toolbar")
        .resizable(false)
        .exact_size(w)
        .frame(egui::Frame::NONE.fill(palette::color(ui.ctx(), ColorSlot::Toolbar)).inner_margin(egui::Margin::symmetric(m, 8)))
        .show(ui, |ui| {
            if t.pro {
                let r = ui.max_rect();
                ui.painter().line_segment([r.right_top() + vec2(m as f32, -8.0), r.right_bottom() + vec2(m as f32, 8.0)], Stroke::new(1.0, t.separator));
                // collapse chevrons like Photoshop's toolbar header
                let (cr, _) = ui.allocate_exact_size(vec2(bx, 14.0), Sense::hover());
                icons::paint(ui, cr, "chevrons-right", 11.0, t.text_faint);
                ui.add_space(4.0);
            }
            // Subtle violet wash at the bottom of the toolbar.
            let full = ui.max_rect();
            if !t.bevel && !t.pro && t.dark() {
                let mut mesh = egui::Mesh::default();
                let r = Rect::from_min_max(pos2(full.left() - 7.0, full.bottom() - 260.0), pos2(full.right() + 7.0, full.bottom() + 8.0));
                let top = Color32::TRANSPARENT;
                let bottom = Color32::from_rgba_unmultiplied(90, 70, 190, 34);
                mesh.colored_vertex(r.left_top(), top);
                mesh.colored_vertex(r.right_top(), top);
                mesh.colored_vertex(r.right_bottom(), bottom);
                mesh.colored_vertex(r.left_bottom(), bottom);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                ui.painter().add(mesh);
            }
            ui.spacing_mut().item_spacing = vec2(2.0, 3.0);
            let flyout_id = egui::Id::new("tool-flyout");
            let held_id = flyout_id.with("held");
            if ui.input(|i| i.pointer.any_pressed()) {
                ui.data_mut(|d| d.remove::<egui::Id>(held_id));
            }
            let mut slot_index = 0usize;
            for (si, section) in TOOL_SECTIONS.iter().enumerate() {
                // Photoshop 2026 draws one uninterrupted column (no group dividers).
                if si > 0 && !t.pro {
                    ui.add_space(4.0);
                    let (r, _) = ui.allocate_exact_size(vec2(if double { bx * 2.0 + 2.0 } else { bx }, 1.0), Sense::hover());
                    ui.painter().line_segment([r.left_center() + vec2(8.0, 0.0), r.right_center() - vec2(8.0, 0.0)], Stroke::new(1.0, t.separator));
                    ui.add_space(4.0);
                }
                let rows: Vec<&[&[Tool]]> = if double { section.chunks(2).collect() } else { section.chunks(1).collect() };
                for row in rows {
                    ui.horizontal(|ui| {
                        for slot in row.iter() {
                            let key = egui::Id::new(("tool-slot", slot_index));
                            slot_index += 1;
                            let active = crate::hold_keys::effective_tool(app, ui.ctx());
                            let tool = slot_tool(&app.ui, active, slot);
                            let sel = slot.contains(&active);
                            let tip = crate::shortcuts::tip_label(app, tool.label(), &tool.shortcut_id());
                            let resp = icons::button(ui, icons::tool_icon(tool), bx, sel, "");
                            // A held tool group owns its flyout. The ordinary tooltip
                            // otherwise sits below the source button and covers the
                            // next tool icon (especially visible in the light themes).
                            let show_tip = !ui.input(|input| input.pointer.any_down())
                                && !resp.secondary_clicked()
                                && ui.data(|data| data.get_temp::<(egui::Id, Rect)>(flyout_id)).is_none();
                            let resp = if show_tip { resp.on_hover_text(tip) } else { resp };
                            if slot.len() > 1 {
                                let r = resp.rect;
                                let tri = vec![r.right_bottom() + vec2(-2.0, -2.0), r.right_bottom() + vec2(-6.0, -2.0), r.right_bottom() + vec2(-2.0, -6.0)];
                                ui.painter().add(egui::Shape::convex_polygon(tri, t.text_faint, Stroke::NONE));
                            }
                            if resp.clicked() && ui.data(|d| d.get_temp::<egui::Id>(held_id)) != Some(key) {
                                crate::tool_group_memory::select(&mut app.ui, tool);
                            }
                            // Right-click or long-press opens the flyout (Photoshop).
                            let held_for = resp.is_pointer_button_down_on().then(|| ui.input(|i| i.pointer.press_start_time().map(|t0| i.time - t0))).flatten();
                            if slot.len() > 1
                                && let Some(seconds) = held_for
                                && seconds < 0.35
                            {
                                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(0.35 - seconds));
                            }
                            let long_press = held_for.is_some_and(|seconds| seconds >= 0.35);
                            if slot.len() > 1 && (resp.secondary_clicked() || (long_press && ui.data(|data| data.get_temp::<egui::Id>(held_id)) != Some(key))) {
                                if !ui.ctx().embed_viewports() {
                                    crate::native_popup_ui::open(ui.ctx(), "tool-flyout", resp.rect.right_top() + vec2(6.0, 0.0), app.frame);
                                }
                                ui.data_mut(|d| {
                                    d.insert_temp(flyout_id, (key, resp.rect));
                                    if long_press {
                                        d.insert_temp(held_id, key);
                                    }
                                });
                            }
                            if slot.len() > 1 && long_press {
                                ui.ctx().request_repaint();
                            }
                            if let Some((open_key, anchor)) = ui.data(|d| d.get_temp::<(egui::Id, Rect)>(flyout_id))
                                && open_key == key
                            {
                                let contents = |app: &mut PhotocraftApp, ui: &mut egui::Ui| tool_flyout_contents(app, ui, slot, tool, &t);
                                if !ui.ctx().embed_viewports() {
                                    let ctx = ui.ctx().clone();
                                    crate::native_popup_ui::show(
                                        app,
                                        &ctx,
                                        "tool-flyout",
                                        tl!("Tools"),
                                        tool_flyout_size(ui.style(), tool_flyout_width(ui, slot, &t), slot.len()),
                                        contents,
                                    );
                                    if !crate::native_popup_ui::is_open(&ctx, "tool-flyout") {
                                        ui.data_mut(|data| data.remove::<(egui::Id, Rect)>(flyout_id));
                                    }
                                } else {
                                    let area = egui::Area::new(key.with("flyout"))
                                        .order(egui::Order::Foreground)
                                        .fixed_pos(anchor.right_top() + vec2(6.0, 0.0))
                                        .show(ui.ctx(), |ui| {
                                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                                contents(app, ui);
                                            });
                                        });
                                    let held_release = resp.clicked() && ui.data(|d| d.get_temp::<egui::Id>(held_id)) == Some(key);
                                    let clicked_outside = ui.input(|i| i.pointer.any_click()) && !held_release && !area.response.hovered() && !resp.hovered();
                                    if clicked_outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                        ui.data_mut(|d| d.remove::<(egui::Id, Rect)>(flyout_id));
                                    }
                                }
                            }
                        }
                    });
                }
            }
            let ctx = ui.ctx().clone();
            if t.pro && icons::button(ui, "ellipsis", bx, false, tl!("Edit Toolbar…")).clicked() {
                let _ = crate::menus::invoke(app, &ctx, "edit.toolbar", json!({}));
            }
            ui.add_space(if t.pro { 8.0 } else { 14.0 });
            color_chips(app, ui);
            if t.pro {
                ui.add_space(8.0);
                let quick_mask = app.session.active().is_some_and(|s| s.doc.quick_mask.is_some());
                if icons::button(
                    ui,
                    "square-dashed",
                    bx,
                    quick_mask,
                    if quick_mask { tl!("Edit in Standard Mode  (Q)") } else { tl!("Edit in Quick Mask Mode  (Q)") },
                )
                .clicked()
                {
                    let _ = crate::menus::invoke(app, &ctx, "select.editInQuickMaskMode", json!({}));
                }
                let sm = icons::button(ui, "app-window", bx, false, tl!("Change Screen Mode  (F)"));
                if sm.clicked() {
                    let _ = crate::menus::invoke(app, &ctx, "view.screenMode.cycle", json!({}));
                }
                // Right-click (or long-press) lists the modes, like Photoshop's flyout.
                egui::Popup::context_menu(&sm).show(|ui| {
                    ui.set_min_width(220.0);
                    for (id, label) in [
                        ("view.screenMode.standard", tl!("Standard Screen Mode")),
                        ("view.screenMode.fullScreenWithMenuBar", tl!("Full Screen Mode With Menu Bar")),
                        ("view.screenMode.fullScreen", tl!("Full Screen Mode")),
                    ] {
                        let on = crate::view_cmds::checked(app, id).unwrap_or(false);
                        if ui.add(egui::Button::selectable(on, label)).clicked() {
                            let _ = crate::menus::invoke(app, &ctx, id, json!({}));
                            ui.close();
                        }
                    }
                });
            }
        });
}

fn tool_flyout_width(ui: &egui::Ui, slot: &[Tool], t: &Tokens) -> f32 {
    let label_width = slot
        .iter()
        .map(|item| ui.painter().layout_no_wrap(tl!(item.label()).to_string(), egui::FontId::proportional(12.5), t.text).size().x)
        .fold(0.0f32, f32::max);
    (label_width + 82.0).max(180.0)
}

fn tool_flyout_contents(app: &mut PhotocraftApp, ui: &mut egui::Ui, slot: &[Tool], tool: Tool, t: &Tokens) -> bool {
    let flyout_id = egui::Id::new("tool-flyout");
    // Menu rows already include their hit target and padding.
    // Inheriting the native child's 6pt item gap overflows the
    // exact-height HWND even for a three-item tool group.
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut selected = false;
    let fw = tool_flyout_width(ui, slot, t);
    for &item in slot.iter() {
        let (r, ir) = ui.allocate_exact_size(vec2(fw, 26.0), Sense::click());
        if ir.hovered() {
            ui.painter().rect_filled(r, 3.0, t.hover);
        }
        if item == tool {
            ui.painter().rect_filled(Rect::from_center_size(pos2(r.left() + 8.0, r.center().y), vec2(4.0, 4.0)), 0.0, t.text);
        }
        icons::paint(ui, Rect::from_center_size(pos2(r.left() + 26.0, r.center().y), vec2(18.0, 18.0)), icons::tool_icon(item), 14.0, t.icon);
        ui.painter().text(pos2(r.left() + 42.0, r.center().y), Align2::LEFT_CENTER, tl!(item.label()), egui::FontId::proportional(12.5), t.text);
        if let Some(shortcut) = crate::shortcuts::shortcut_label(app, &item.shortcut_id()) {
            ui.painter().text(pos2(r.right() - 8.0, r.center().y), Align2::RIGHT_CENTER, shortcut, egui::FontId::proportional(12.0), t.text_dim);
        }
        if ir.clicked() {
            crate::tool_group_memory::select(&mut app.ui, item);
            ui.data_mut(|d| d.remove::<(egui::Id, Rect)>(flyout_id));
            selected = true;
        }
    }
    selected
}

/// Tool rows are contiguous 26pt targets. Include the actual menu frame (margin
/// and stroke) rather than an unrelated fixed padding; the native ScrollArea
/// then only needs a scrollbar when the display really is too short.
fn tool_flyout_size(style: &egui::Style, width: f32, rows: usize) -> Vec2 {
    vec2(width, rows as f32 * 26.0) + crate::native_popup_ui::menu_frame(style).total_margin().sum()
}

/// Does the toolbar need two columns? Height of one column (header, tool slots, Edit Toolbar,
/// colour chips, Quick Mask and Screen Mode) against the height the toolbar gets.
pub fn toolbar_needs_double(slots: usize, sections: usize, bx: f32, pro: bool, avail_h: f32) -> bool {
    let pitch = bx + 3.0;
    let needed = if pro {
        // margins + header + slots + "…" + gap + chips (38 + swap row) + gap + 2 buttons
        16.0 + 21.0 + (slots + 1) as f32 * pitch + 8.0 + 59.0 + 8.0 + 2.0 * pitch
    } else {
        16.0 + slots as f32 * pitch + sections.saturating_sub(1) as f32 * 12.0 + 14.0 + 59.0
    };
    needed > avail_h
}

fn c32(c: [f32; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, (c[3] * 255.0) as u8)
}

fn color_chips(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(36.0, 38.0), Sense::hover());
    let bg = Rect::from_min_size(rect.min + vec2(13.0, 13.0), vec2(21.0, 21.0));
    let fg = Rect::from_min_size(rect.min + vec2(2.0, 2.0), vec2(21.0, 21.0));
    let p = ui.painter();
    p.rect_filled(bg, 5.0, c32(app.session.tools.background));
    p.rect_stroke(bg, 5.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    p.rect_filled(fg, 5.0, c32(app.session.tools.foreground));
    p.rect_stroke(fg, 5.0, Stroke::new(1.5, t.chrome), StrokeKind::Outside);
    p.rect_stroke(fg, 5.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    // Photoshop: clicking a chip opens the Color Picker for that colour.
    let bg_resp = ui.interact(bg, ui.id().with("bgchip"), Sense::click());
    let fg_resp = ui.interact(fg, ui.id().with("fgchip"), Sense::click());
    if fg_resp.on_hover_text(tl!("Set foreground color")).clicked() {
        crate::color_picker_ui::open(app, "foreground");
    } else if bg_resp.on_hover_text(tl!("Set background color")).clicked() {
        crate::color_picker_ui::open(app, "background");
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        if icons::button(ui, "arrow-left-right", 18.0, false, tl!("Swap colours (X)")).clicked() {
            let _ = app.run("tools.swapColors", json!({}));
        }
        if icons::button(ui, "contrast", 18.0, false, tl!("Default colours (D)")).clicked() {
            let _ = app.run("tools.defaultColors", json!({}));
        }
    });
}

// ----------------------------------------------------------------------------- title bar

/// The app's top bar: brand mark, menus, the document title and the workspace controls. With
/// [`PhotocraftApp::custom_titlebar`] (Windows and Linux) it is also the window's title bar, as in
/// Photoshop on Windows: the caption buttons take its right end (`titlebar`).
pub fn title_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let custom = app.custom_titlebar;
    let left = if cfg!(target_os = "macos") && app.integrated_titlebar { 78 } else { 10 };
    let right = if custom { 0 } else { 10 };
    let bar = egui::Panel::top("title_bar")
        .exact_size(if t.pro { 32.0 } else { 38.0 })
        .frame(egui::Frame::NONE.fill(palette::color(ui.ctx(), ColorSlot::TitleBar)).inner_margin(egui::Margin { left, right, top: 0, bottom: 0 }))
        .show(ui, |ui| {
            let full = ui.max_rect();
            // Only the free gap between the menus and the right-hand controls drags the window: a
            // press on a menu title must open the menu, never move the window. The gap is last
            // frame's, as the menus are laid out after this.
            let span_id = ui.id().with("titledrag-span");
            let (gap_l, gap_r) = ui.ctx().data(|d| d.get_temp::<(f32, f32)>(span_id)).unwrap_or((full.right(), full.right()));
            let gap = egui::Rect::from_x_y_ranges(gap_l.max(full.left())..=gap_r.min(full.right()).max(gap_l), full.y_range());
            let drag = ui.interact(gap, ui.id().with("titledrag"), Sense::click_and_drag());
            if drag.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            if drag.double_clicked() {
                let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
            }
            let title = app
                .session
                .active()
                .map(|d| format!("{}{}", d.doc.name, if d.is_dirty() { "  •" } else { "" }))
                .unwrap_or_else(|| photocraft_engine::build_info::CUSTOM_PRODUCT_NAME.into());
            // The menus and the right-hand controls are laid out first; the title is centred in
            // whatever room is left between them, shortened or dropped rather than drawn over them.
            let (mut menus_right, mut controls_left) = (full.left(), full.right());
            ui.horizontal_centered(|ui| {
                let side = if t.pro { 18.0 } else { 20.0 };
                let (mark, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
                crate::brand::paint_mark(ui, mark);
                ui.add_space(6.0);
                // With the macOS menu bar the menus are at the top of the screen instead.
                menus_right = if app.services.native_menu.is_some() { ui.cursor().left() } else { crate::menus::menu_bar(app, ui) };
                // The menu bar takes the whole row, so the right-hand group gets its own rect:
                // from the menus to the bar's end, or to the caption buttons.
                let right_edge = if custom { full.right() - crate::titlebar::WIDTH - 4.0 } else { full.right() };
                let group = egui::Rect::from_min_max(egui::pos2((menus_right + TITLE_GAP).min(right_edge), full.top()), egui::pos2(right_edge, full.bottom()));
                let mut group_ui = ui.new_child(egui::UiBuilder::new().max_rect(group).layout(egui::Layout::right_to_left(egui::Align::Center)));
                controls_left = {
                    let ui = &mut group_ui;
                    ui.spacing_mut().item_spacing.x = 6.0;
                    // A narrow bar drops the theme toggle (Preferences), then search
                    // (Edit › Search), and narrows
                    // the switcher (Window › Workspace) before anything runs over.
                    let room = ui.available_width();
                    let icons_shown: u8 = if room >= WORKSPACE_MIN + 2.0 * ICON_SLOT {
                        2
                    } else if room >= WORKSPACE_MIN + ICON_SLOT {
                        1
                    } else {
                        0
                    };
                    let ws_width = (room - f32::from(icons_shown) * ICON_SLOT - 6.0).clamp(WORKSPACE_MIN, 130.0);
                    if room >= WORKSPACE_MIN {
                        crate::workspace_ui::selector(app, ui, ws_width);
                    }
                    if icons_shown >= 1
                        && icons::button(ui, "search", 28.0, app.ui.palette_open, &crate::shortcuts::tip_label(app, "Search commands", "edit.search")).clicked()
                    {
                        app.ui.palette_open = !app.ui.palette_open;
                    }
                    let theme_icon = if t.dark() { "sun" } else { "moon" };
                    if icons_shown >= 2 && icons::button(ui, theme_icon, 28.0, false, tl!("Switch theme")).clicked() {
                        let next = app.ui.theme.next();
                        app.set_theme(ui.ctx(), next);
                    }
                    ui.min_rect().left()
                };
            });
            ui.ctx().data_mut(|d| d.insert_temp(span_id, (menus_right, controls_left)));
            let font = theme::medium(13.0);
            let galley = ui.painter().layout_no_wrap(title.clone(), font.clone(), t.text_dim);
            if let Some(x) = title_x(full.center().x, menus_right, controls_left, galley.size().x) {
                ui.painter().galley(egui::pos2(x, full.center().y - galley.size().y / 2.0), galley, t.text_dim);
            } else if controls_left - menus_right > 80.0 {
                // Too narrow for the whole name: show the start of it, elided, in the free gap.
                let avail = controls_left - menus_right - 2.0 * TITLE_GAP;
                let mut job = egui::text::LayoutJob::simple_singleline(title, font, t.text_dim);
                job.wrap = egui::text::TextWrapping::truncate_at_width(avail);
                let g = ui.painter().layout_job(job);
                ui.painter().galley(egui::pos2(menus_right + TITLE_GAP, full.center().y - g.size().y / 2.0), g, t.text_dim);
            }
        });
    if custom {
        crate::titlebar::caption_buttons(app, ui, bar.response.rect);
    }
}

/// Minimum space kept between the window title and the menus or controls beside it.
const TITLE_GAP: f32 = 16.0;
/// The workspace switcher's narrowest width, and one 28 pt title-bar icon with its spacing.
const WORKSPACE_MIN: f32 = 90.0;
const ICON_SLOT: f32 = 34.0;

/// Left edge for a title `width` wide: centred on `center` when it fits between the menus
/// (ending at `menus_right`) and the right-hand controls (starting at `controls_left`), else
/// slid into that gap; None when the gap is too small for the whole title.
fn title_x(center: f32, menus_right: f32, controls_left: f32, width: f32) -> Option<f32> {
    let (lo, hi) = (menus_right + TITLE_GAP, controls_left - TITLE_GAP - width);
    if !(lo.is_finite() && hi.is_finite() && width.is_finite()) || hi < lo {
        return None;
    }
    if !center.is_finite() {
        return Some(lo);
    }
    Some((center - width / 2.0).clamp(lo, hi))
}

#[cfg(test)]
mod title_tests {
    use super::title_x;

    #[test]
    fn title_never_overlaps_menus_or_controls() {
        // Wide window: centred.
        assert_eq!(title_x(800.0, 420.0, 1300.0, 60.0), Some(770.0));
        // Centre would hit the menus: slid right into the gap.
        assert_eq!(title_x(400.0, 420.0, 1300.0, 60.0), Some(436.0));
        // Centre would hit the controls: slid left.
        assert_eq!(title_x(1280.0, 420.0, 1300.0, 60.0), Some(1224.0));
        // No room for the whole title: the caller elides or drops it.
        assert_eq!(title_x(400.0, 420.0, 480.0, 60.0), None);
        assert_eq!(title_x(f32::NAN, 420.0, 1300.0, 60.0), Some(436.0));
        assert_eq!(title_x(400.0, f32::INFINITY, 1300.0, 60.0), None);
    }
}

// ----------------------------------------------------------------------------- options bar

pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("options_bar")
        .exact_size(if t.pro { 36.0 } else { 42.0 })
        .frame(egui::Frame::NONE.fill(palette::color(ui.ctx(), ColorSlot::OptionsBar)).inner_margin(egui::Margin::symmetric(10, 0)))
        .show(ui, |ui| {
            let r = ui.max_rect();
            ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, t.separator));
            ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.separator));
            ui.horizontal_centered(|ui| {
                if t.pro {
                    crate::chrome_ui::home_button(app, ui);
                    widgets::vline(ui, 22.0);
                }
                let shown_tool = crate::hold_keys::effective_tool(app, ui.ctx());
                let _ = icons::button(ui, icons::tool_icon(shown_tool), if t.pro { 26.0 } else { 28.0 }, !t.pro, tl!(shown_tool.label()));
                if t.pro {
                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 20.0), Sense::hover());
                    icons::paint(ui, r, "chevron-down", 10.0, t.text_faint);
                }
                widgets::vline(ui, 22.0);
                if app.ui.transform.is_some() {
                    crate::transform_tool::options_bar(app, ui);
                    return;
                }
                if crate::distort_ui::options_bar(app, ui) {
                    return;
                }
                let tool = shown_tool;
                // Brush edits here go through `tools.setBrush`, one journal entry per gesture (Rule 1).
                if (tool.is_brushlike() && !matches!(tool, Tool::Brush | Tool::Pencil | Tool::MixerBrush | Tool::Eraser)) || tool == Tool::QuickSelection {
                    let before = app.session.tools.brush.clone();
                    let mut b = before.clone();
                    let pick = brush_preset_chip(app, ui, &mut b);
                    crate::brush_panel::commit_gesture(app, ui.ctx(), &before, &b);
                    crate::brush_picker::apply(app, ui.ctx(), pick);
                    crate::brush_picker::settings_toggle(app, ui);
                    widgets::vline(ui, 22.0);
                }
                if crate::eraser_ui::options_bar(app, ui, tool)
                    || crate::retouch_ui::options_bar(app, ui, tool)
                    || crate::vector_ui::options_bar(app, ui, tool)
                    || crate::analysis_ui::options_bar(app, ui, tool)
                    || crate::slice_ui::options_bar(app, ui, tool)
                {
                    return;
                }
                crate::paint_mouse::sync_tool_smoothing(app);
                let brush_before = app.session.tools.brush.clone();
                let mut brush = brush_before.clone();
                let b = &mut brush;
                let mut picked = None;
                match tool {
                    Tool::Brush | Tool::Eraser if t.pro => {
                        picked = brush_preset_chip(app, ui, b);
                        crate::brush_picker::settings_toggle(app, ui);
                        widgets::vline(ui, 22.0);
                        opt_label(ui, tl!("Mode"));
                        let mut mode = b.mode;
                        let opts: Vec<(BlendMode, &str)> = BlendMode::LAYER_MODES.iter().map(|m| (*m, m.label())).collect();
                        if widgets::dropdown(ui, "brush-mode", &mut mode, &opts, 96.0) {
                            b.mode = mode;
                        }
                        percent_field(ui, tl!("Opacity"), &mut b.opacity, 0.0..=100.0, 62.0);
                        if icons::button(ui, "circle-dot", 24.0, b.pressure_opacity, tl!("Always use pressure for opacity")).clicked() {
                            b.pressure_opacity = !b.pressure_opacity;
                        }
                        percent_field(ui, tl!("Flow"), &mut b.flow, 1.0..=100.0, 62.0);
                        let _ = icons::button(ui, "sparkles", 24.0, false, tl!("Enable airbrush-style build-up effects"));
                        opt_label(ui, tl!("Smoothing"));
                        smoothing_field(ui, b, 58.0);
                        let _ = icons::button(ui, "settings", 24.0, false, tl!("Set additional smoothing options"));
                        widgets::vline(ui, 22.0);
                        if icons::button(ui, "circle-dot", 24.0, b.pressure_size, tl!("Always use pressure for size")).clicked() {
                            b.pressure_size = !b.pressure_size;
                        }
                        let _ = icons::button(ui, "arrow-left-right", 24.0, false, tl!("Set painting symmetry options"));
                    }
                    // Pencil: Photoshop's options (no hardness or flow: the pencil is always hard).
                    Tool::Pencil => {
                        picked = brush_preset_chip(app, ui, b);
                        crate::brush_picker::settings_toggle(app, ui);
                        widgets::vline(ui, 22.0);
                        if !t.pro {
                            opt_label(ui, tl!("Size"));
                            widgets::value_field(ui, &mut b.size, 1.0..=photocraft_engine::paint::MAX_BRUSH_SIZE, "px", 76.0);
                            widgets::vline(ui, 22.0);
                        }
                        opt_label(ui, tl!("Mode"));
                        let mut mode = b.mode;
                        let opts: Vec<(BlendMode, &str)> = BlendMode::LAYER_MODES.iter().map(|m| (*m, m.label())).collect();
                        if widgets::dropdown(ui, "pencil-mode", &mut mode, &opts, 96.0) {
                            b.mode = mode;
                        }
                        percent_field(ui, tl!("Opacity"), &mut b.opacity, 0.0..=100.0, if t.pro { 62.0 } else { 66.0 });
                        opt_label(ui, tl!("Smoothing"));
                        smoothing_field(ui, b, if t.pro { 58.0 } else { 66.0 });
                        widgets::vline(ui, 22.0);
                        widgets::checkbox(ui, &mut app.ui.tool_options.pencil_auto_erase, tl!("Auto Erase"));
                    }
                    Tool::Brush | Tool::Eraser => {
                        picked = brush_preset_chip(app, ui, b);
                        crate::brush_picker::settings_toggle(app, ui);
                        widgets::vline(ui, 22.0);
                        opt_label(ui, tl!("Size"));
                        widgets::value_field(ui, &mut b.size, 1.0..=2500.0, "px", 76.0);
                        widgets::vline(ui, 22.0);
                        percent_field(ui, tl!("Hardness"), &mut b.hardness, 0.0..=100.0, 66.0);
                        percent_field(ui, tl!("Opacity"), &mut b.opacity, 0.0..=100.0, 66.0);
                        percent_field(ui, tl!("Flow"), &mut b.flow, 1.0..=100.0, 66.0);
                        opt_label(ui, tl!("Smoothing"));
                        smoothing_field(ui, b, 66.0);
                        widgets::vline(ui, 22.0);
                        widgets::toggle(ui, &mut b.pressure_size, tl!("Pressure for Size"));
                        widgets::toggle(ui, &mut b.pressure_opacity, tl!("Pressure for Opacity"));
                    }
                    Tool::MixerBrush => {
                        picked = brush_preset_chip(app, ui, b);
                        crate::brush_picker::settings_toggle(app, ui);
                        widgets::vline(ui, 22.0);
                        for (label, value) in [
                            (tl!("Wet"), &mut b.mixer.wet),
                            (tl!("Load"), &mut b.mixer.load),
                            (tl!("Mix"), &mut b.mixer.mix),
                            (tl!("Flow"), &mut b.mixer.flow),
                        ] {
                            percent_field(ui, label, value, 0.0..=100.0, 62.0);
                        }
                        widgets::vline(ui, 22.0);
                        if crate::sampling_ui::mixer_options(ui,&mut app.ui.sampling) {crate::sampling_ui::save(app);}
                        widgets::checkbox(ui, &mut b.mixer.simulate_paint, tl!("Simulate Paint"));
                        if ui.small_button(tl!("Clear")).clicked() {
                            let _=app.run("tools.mixer.load",json!({"clean":true}));
                        }
                    }
                    Tool::RectMarquee | Tool::EllipseMarquee => {
                        selection_mode_buttons(app,ui);
                        ui.spacing_mut().item_spacing.x = 8.0;
                        widgets::vline(ui, 22.0);
                        let o = &mut app.ui.tool_options;
                        opt_label(ui, tl!("Feather"));
                        widgets::value_field(ui, &mut o.feather, 0.0..=1000.0, "px", 62.0);
                        // Photoshop greys Anti-alias for the Rectangular Marquee (its edges are always hard).
                        ui.add_enabled_ui(app.ui.tool == Tool::EllipseMarquee, |ui| widgets::checkbox(ui, &mut o.anti_alias, tl!("Anti-alias")));
                        widgets::vline(ui, 22.0);
                        opt_label(ui, tl!("Style"));
                        let styles = [
                            ("normal".to_string(), tl!("Normal")),
                            ("fixedRatio".to_string(), tl!("Fixed Ratio")),
                            ("fixedSize".to_string(), tl!("Fixed Size")),
                        ];
                        if widgets::dropdown(ui, "marquee-style", &mut o.marquee_style, &styles, 96.0) {
                            (o.marquee_width, o.marquee_height) = if o.marquee_style == "fixedSize" { (64.0, 64.0) } else { (1.0, 1.0) };
                        }
                        let fixed = o.marquee_style != "normal";
                        let (unit, range) = if o.marquee_style == "fixedSize" { ("px", 1.0..=300_000.0) } else { ("", 0.001..=999.0) };
                        ui.add_enabled_ui(fixed, |ui| {
                            opt_label(ui, tl!("Width"));
                            widgets::value_field(ui, &mut o.marquee_width, range.clone(), unit, 62.0);
                            if icons::button(ui, "arrow-left-right", 22.0, false, tl!("Swaps height and width")).clicked() {
                                std::mem::swap(&mut o.marquee_width, &mut o.marquee_height);
                            }
                            opt_label(ui, tl!("Height"));
                            widgets::value_field(ui, &mut o.marquee_height, range, unit, 62.0);
                        });
                        widgets::vline(ui, 22.0);
                        if widgets::secondary_button(ui, tl!("Select and Mask…"), 0.0).clicked() {
                            let _ = crate::menus::invoke(app, ui.ctx(), "select.selectAndMask", json!({}));
                        }
                    }
                    Tool::Lasso | Tool::PolygonLasso | Tool::MagneticLasso => {
                        selection_mode_buttons(app, ui);
                        widgets::vline(ui, 22.0);
                        opt_label(ui, tl!("Feather"));
                        widgets::value_field(ui, &mut app.ui.tool_options.feather, 0.0..=1000.0, "px", 62.0);
                        widgets::checkbox(ui, &mut app.ui.tool_options.anti_alias, tl!("Anti-alias"));
                        widgets::vline(ui, 22.0);
                        if app.ui.tool == Tool::MagneticLasso {
                            let o = &mut app.ui.tool_options;
                            opt_label(ui, tl!("Width"));
                            widgets::value_field(ui, &mut o.magnetic_width, 1.0..=256.0, "px", 58.0)
                                .on_hover_text(tl!("Follows only edges this close to the pointer ([ and ] change it)"));
                            opt_label(ui, tl!("Contrast"));
                            widgets::value_field(ui, &mut o.magnetic_contrast, 1.0..=100.0, "%", 58.0)
                                .on_hover_text(tl!("Higher values follow only edges that contrast sharply with their surroundings"));
                            opt_label(ui, tl!("Frequency"));
                            widgets::value_field(ui, &mut o.magnetic_frequency, 0.0..=100.0, "", 50.0)
                                .on_hover_text(tl!("How often fastening points are placed by themselves"));
                            if icons::button(ui, "circle-dot", 24.0, o.magnetic_pressure, tl!("Use tablet pressure to change pen width")).clicked() {
                                o.magnetic_pressure = !o.magnetic_pressure;
                            }
                            widgets::vline(ui, 22.0);
                        }
                        if widgets::secondary_button(ui, tl!("Select and Mask…"), 0.0).clicked() {
                            let _ = crate::menus::invoke(app, ui.ctx(), "select.selectAndMask", json!({}));
                        }
                        if crate::lasso_ui::active(app) || (app.ui.tool == Tool::PolygonLasso && !app.ui.polygon.is_empty()) {
                            hint(
                                ui,
                                &crate::i18n::fmt(
                                    tl!("Click the first point or press {key} to close · Esc cancels"),
                                    &[("key", &crate::shortcuts::pretty("Enter"))],
                                ),
                            );
                        } else if app.ui.tool == Tool::Lasso {
                            hint(ui, tl!("Hold Ctrl while drawing for straight segments"));
                        }
                        if app.ui.tool == Tool::MagneticLasso && app.ui.magnetic.active() {
                            hint(
                                ui,
                                &crate::i18n::fmt(
                                    tl!("Click the first point or press {key} to close · {del} removes a point · Esc cancels"),
                                    &[("key", &crate::shortcuts::pretty("Enter")), ("del", &crate::shortcuts::pretty("Backspace"))],
                                ),
                            );
                        }
                    }
                    Tool::MagicWand => {
                        selection_mode_buttons(app, ui);
                        widgets::vline(ui, 22.0);
                        opt_label(ui, tl!("Tolerance"));
                        widgets::value_field(ui, &mut app.ui.tool_options.tolerance, 0.0..=255.0, "", 58.0);
                        widgets::checkbox(ui, &mut app.ui.tool_options.anti_alias, tl!("Anti-alias"));
                        widgets::checkbox(ui, &mut app.ui.tool_options.contiguous, tl!("Contiguous"));
                        widgets::checkbox(ui, &mut app.ui.tool_options.sample_all_layers, tl!("Sample All Layers"));
                        widgets::vline(ui, 22.0);
                        if widgets::secondary_button(ui, tl!("Select Subject"), 0.0).clicked() {
                            let _ = app.run("select.subject", json!({}));
                        }
                        if widgets::secondary_button(ui, tl!("Select and Mask…"), 0.0).clicked() {
                            let _ = crate::menus::invoke(app, ui.ctx(), "select.selectAndMask", json!({}));
                        }
                    }
                    Tool::PaintBucket if t.pro => {
                        opt_label(ui, tl!("Fill"));
                        let mut src = u8::from(app.ui.tool_options.bucket_fill_pattern);
                        if widgets::dropdown(ui, "bucket-src", &mut src, &[(0u8, tl!("Foreground")), (1, tl!("Pattern"))], 100.0) {
                            app.ui.tool_options.bucket_fill_pattern = src == 1;
                        }
                        opt_label(ui, tl!("Opacity"));
                        widgets::value_field(ui, &mut app.ui.tool_options.fill_opacity, 1.0..=100.0, "%", 62.0);
                        opt_label(ui, tl!("Tolerance"));
                        widgets::value_field(ui, &mut app.ui.tool_options.tolerance, 0.0..=255.0, "", 58.0);
                        widgets::checkbox(ui, &mut app.ui.tool_options.anti_alias, tl!("Anti-alias"));
                        widgets::checkbox(ui, &mut app.ui.tool_options.contiguous, tl!("Contiguous"));
                        widgets::checkbox(ui, &mut app.ui.tool_options.sample_all_layers, tl!("All Layers"));
                    }
                    Tool::Gradient if t.pro => {
                        // Live "Gradient" (a Gradient Fill layer) or "Classic gradient" (pixels).
                        let mut classic = app.ui.tool_options.gradient_classic;
                        if widgets::dropdown(ui, "gradient-mode", &mut classic, &[(false, "Gradient"), (true, "Classic gradient")], 118.0) {
                            app.ui.tool_options.gradient_classic = classic;
                        }
                        let (fg, bg) = (app.session.tools.foreground, app.session.tools.background);
                        if classic {
                            gradient_swatch(ui, &[(0.0, fg), (1.0, bg)], true);
                        } else {
                            gradient_swatch(ui, &app.session.presets.gradient.resolve(fg, bg), false);
                        }
                        widgets::vline(ui, 22.0);
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let before = app.ui.tool_options.clone();
                        for (style, icon, tip) in [
                            ("linear", "blend", tl!("Linear Gradient")),
                            ("radial", "circle", tl!("Radial Gradient")),
                            ("angle", "rotate-cw", tl!("Angle Gradient")),
                            ("reflected", "arrow-left-right", tl!("Reflected Gradient")),
                            ("diamond", "diamond", tl!("Diamond Gradient")),
                        ] {
                            if icons::button(ui, icon, 24.0, app.ui.tool_options.gradient_style == style, tip).clicked() {
                                app.ui.tool_options.gradient_style = style.into();
                            }
                        }
                        ui.spacing_mut().item_spacing.x = 8.0;
                        widgets::vline(ui, 22.0);
                        opt_label(ui, tl!("Mode"));
                        let opts: Vec<(BlendMode, &str)> = BlendMode::LAYER_MODES.iter().map(|m| (*m, m.label())).collect();
                        widgets::dropdown(ui, "gradient-blend-mode", &mut app.ui.tool_options.gradient_blend_mode, &opts, 96.0);
                        opt_label(ui, tl!("Opacity"));
                        widgets::value_field(ui, &mut app.ui.tool_options.fill_opacity, 1.0..=100.0, "%", 62.0);
                        widgets::checkbox(ui, &mut app.ui.tool_options.gradient_reverse, tl!("Reverse"));
                        widgets::checkbox(ui, &mut app.ui.tool_options.gradient_dither, tl!("Dither"));
                        // Live mode: the options also change a selected gradient fill layer.
                        crate::gradient_ui::options_changed(app, &before);
                    }
                    Tool::Crop if t.pro => {
                        let o = &mut app.ui.tool_options;
                        let ratios: Vec<(String, &str)> = crate::chrome_ui::CROP_RATIOS.iter().map(|(k, l)| (k.to_string(), *l)).collect();
                        widgets::dropdown(ui, "crop-ratio", &mut o.crop_ratio, &ratios, 120.0);
                        // Custom ratio fields (Photoshop shows the preset's numbers here).
                        let (mut rw, mut rh) = crate::chrome_ui::crop_ratio(&o.crop_ratio, 0.0, 0.0).map_or((0.0, 0.0), |(w, h)| (w as f32, h as f32));
                        let cw = widgets::value_field(ui, &mut rw, 0.0..=99_999.0, "", 54.0).changed();
                        let swap = icons::button(ui, "arrow-left-right", 22.0, false, tl!("Swaps height and width")).clicked();
                        let ch = widgets::value_field(ui, &mut rh, 0.0..=99_999.0, "", 54.0).changed();
                        if swap {
                            std::mem::swap(&mut rw, &mut rh);
                        }
                        if (cw || ch || swap) && rw > 0.0 && rh > 0.0 {
                            o.crop_ratio = format!("{}:{}", widgets::fmt_num(rw as f64), widgets::fmt_num(rh as f64));
                        }
                        widgets::vline(ui, 22.0);
                        if widgets::secondary_button(ui, tl!("Clear"), 0.0).clicked() {
                            o.crop_ratio.clear();
                        }
                        let _ = icons::button(ui, "grid-3x3", 24.0, true, tl!("Overlay: Rule of Thirds"));
                        widgets::checkbox(ui, &mut o.crop_delete, tl!("Delete Cropped Pixels"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if icons::button(
                                ui,
                                "check",
                                26.0,
                                false,
                                &crate::i18n::fmt(tl!("Commit current crop operation  ({key})"), &[("key", &crate::shortcuts::pretty("Enter"))]),
                            )
                            .clicked()
                            {
                                crate::canvas::commit_crop(app);
                            }
                            if icons::button(ui, "ban", 26.0, false, tl!("Cancel current crop operation  (Esc)")).clicked() {
                                app.ui.crop_rect = None;
                            }
                        });
                    }
                    Tool::Type | Tool::VerticalType => crate::type_tool::options_bar(app, ui),
                    Tool::Move => {
                        let mut auto_select = crate::hold_keys::auto_select_for_ui(app, ui.ctx());
                        let held = crate::hold_keys::auto_select_held(app, ui.ctx());
                        let changed = ui.add_enabled_ui(!held, |ui| widgets::checkbox(ui, &mut auto_select, tl!("Auto-Select:"))).inner.changed();
                        if changed {
                            app.ui.tool_options.move_auto_select = auto_select;
                        }
                        let o = &mut app.ui.tool_options;
                        widgets::dropdown(
                            ui,
                            "move-target",
                            &mut o.move_target,
                            &[("layer".to_string(), tl!("Layer")), ("group".to_string(), tl!("Group"))],
                            76.0,
                        );
                        widgets::vline(ui, 22.0);
                        widgets::checkbox(ui, &mut o.move_show_transform, tl!("Show Transform Controls"));
                        widgets::vline(ui, 22.0);
                        ui.spacing_mut().item_spacing.x = 0.0;
                        for (icon, tip, cmd) in [
                            ("panels-top-left", tl!("Align left edges"), "layer.align.leftEdges"),
                            ("app-window", tl!("Align horizontal centers"), "layer.align.horizontalCenters"),
                            ("panel-right", tl!("Align right edges"), "layer.align.rightEdges"),
                        ] {
                            let on = app.session.is_enabled(cmd);
                            if ui.add_enabled_ui(on, |ui| icons::button(ui, icon, 24.0, false, tip)).inner.clicked() {
                                let _ = app.run(cmd, json!({}));
                            }
                        }
                        ui.add_space(6.0);
                        let more = icons::button(ui, "ellipsis", 24.0, false, tl!("Align and distribute"));
                        let contents = |app: &mut PhotocraftApp, ui: &mut egui::Ui| {
                            let mut chosen = false;
                            ui.set_min_width(200.0);
                            let mut section = |ui: &mut egui::Ui, title: &str, prefix: &str| {
                                ui.label(egui::RichText::new(tl!(&title)).small().color(t.text_dim));
                                for c in photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with(prefix)) {
                                    if ui.add_enabled(app.session.is_enabled(c.id), egui::Button::new(tl!(c.label))).clicked() {
                                        let _ = app.run(c.id, json!({}));
                                        ui.close();
                                        chosen = true;
                                    }
                                }
                            };
                            section(ui, tl!("Align"), "layer.align.");
                            ui.separator();
                            section(ui, tl!("Distribute"), "layer.distribute.");
                            chosen
                        };
                        if ui.ctx().embed_viewports() {
                            egui::Popup::menu(&more).show(|ui| { contents(app, ui); });
                        } else {
                            let ctx = ui.ctx().clone();
                            if more.clicked() { crate::native_popup_ui::toggle(&ctx, "align-distribute", more.rect.left_bottom(), app.frame); }
                            crate::native_popup_ui::show(app, &ctx, "align-distribute", tl!("Align and distribute"), vec2(280.0, 440.0), contents);
                        }
                    }
                    Tool::Eyedropper => {
                        if crate::sampling_ui::eyedropper_options(ui,&mut app.ui.sampling) {crate::sampling_ui::save(app);}
                    }
                    Tool::Zoom => {
                        widgets::checkbox(ui, &mut app.ui.tool_options.zoom_scrubby, tl!("Scrubby Zoom"));
                        hint(
                            ui,
                            &crate::i18n::fmt(
                                tl!("Click to zoom in  ·  {key}-click to zoom out  ·  drag right/left to zoom in/out"),
                                &[("key", &crate::shortcuts::pretty("Alt"))],
                            ),
                        );
                        if widgets::secondary_button(ui, tl!("Fit Screen"), 0.0).clicked()
                            && let Some(i) = app.session.active_index()
                        {
                            app.ui.views[i].fit_pending = true;
                        }
                        if widgets::secondary_button(ui, "100%", 0.0).clicked()
                            && let Some(i) = app.session.active_index()
                        {
                            app.ui.views[i].zoom = 1.0;
                        }
                    }
                    Tool::RotateView => crate::rotate_view::options(app, ui),
                    Tool::Hand => hint(ui, tl!("Drag to pan  ·  hold Space with any tool")),
                    Tool::Crop => hint(
                        ui,
                        &crate::i18n::fmt(
                            tl!("Drag a crop box · drag inside to move · edges resize ({ratio} ratio, {centre} centre) · Space moves while drawing · {commit} commits · Esc cancels"),
                            &[
                                ("ratio", &crate::shortcuts::pretty("Shift")),
                                ("centre", &crate::shortcuts::pretty("Alt")),
                                ("commit", &crate::shortcuts::pretty("Enter")),
                            ],
                        ),
                    ),
                    Tool::Gradient => hint(ui, tl!("Drag to draw a gradient")),
                    Tool::PaintBucket => hint(ui, tl!("Click to fill similar colours")),
                    // Retouching and smart-selection tools draw their bar in `retouch_ui::options_bar`.
                    _ => {}
                }
                crate::brush_panel::commit_gesture(app, ui.ctx(), &brush_before, &brush);
                crate::brush_picker::apply(app, ui.ctx(), picked);
            });
        });
    let request = ui.data_mut(|data| {
        let id = egui::Id::new("native-options-brush-picker-request");
        let request = data.get_temp::<egui::Pos2>(id);
        data.remove::<egui::Pos2>(id);
        request
    });
    if let Some(position) = request {
        if crate::brush_picker::native_viewport(ui.ctx()).is_some() {
            crate::brush_picker::close(app, ui.ctx());
        } else if let Some(document) = app.session.active().map(|state| state.doc.id) {
            crate::brush_picker::open_at(app, ui.ctx(), egui::ViewportId::ROOT, document, [position.x, position.y]);
        }
    }
}

fn label(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(s).color(t.text_dim));
}

/// Width of `s` as [`label`] draws it (body text).
fn body_text_width(ui: &egui::Ui, s: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter().layout_no_wrap(s.to_string(), font, Color32::WHITE).size().x
}

/// Options-bar label: Photoshop writes "Size:" with a colon.
fn opt_label(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    let text = if t.pro && !s.ends_with(':') { format!("{s}:") } else { s.to_string() };
    ui.label(RichText::new(tl!(&text)).color(t.text_dim));
}

fn percent_field(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, width: f32) {
    opt_label(ui, label);
    let mut percent = *value * 100.0;
    if widgets::value_field(ui, &mut percent, range, "%", width).changed() {
        *value = percent / 100.0;
    }
}

fn hint(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(s).color(t.text_faint));
}

// ----------------------------------------------------------------------------- status bar

pub fn status_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("status_bar")
        .exact_size(if t.pro { 24.0 } else { 30.0 })
        .frame(egui::Frame::NONE.fill(palette::color(ui.ctx(), ColorSlot::StatusBar)).inner_margin(egui::Margin::symmetric(10, 0)))
        .show(ui, |ui| {
            let r = ui.max_rect();
            ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, t.separator));
            ui.horizontal_centered(|ui| {
                if t.pro {
                    crate::chrome_ui::status_bar_pro(app, ui);
                    // Background job progress with Cancel, at the right end (#210).
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| crate::jobs_ui::status_progress(app, ui));
                    return;
                }
                if let (Some(st), Some(i)) = (app.session.active(), app.session.active_index()) {
                    let (w, h, mode, bits, layers) =
                        (st.doc.size.width, st.doc.size.height, crate::canvas::mode_label(&st.doc), st.doc.depth.bits(), st.doc.layer_count());
                    let mut pct = app.ui.views[i].zoom * 100.0;
                    if widgets::value_field(ui, &mut pct, 1.0..=3200.0, "%", 78.0).changed() {
                        app.ui.views[i].zoom = pct / 100.0;
                        app.ui.views[i].fit_pending = false;
                    }
                    widgets::vline(ui, 16.0);
                    label(ui, &crate::i18n::fmt(tl!("{mode} Color · {bits} bit"), &[("mode", mode), ("bits", &bits.to_string())]));
                    widgets::vline(ui, 16.0);
                    label(ui, &format!("{w} × {h} px"));
                    widgets::vline(ui, 16.0);
                    label(ui, &crate::i18n::trn(crate::i18n::current(), layers as u64, "{n} layer", "{n} layers"));
                } else {
                    label(ui, tl!("No document"));
                }
                if !app.ui.status.is_empty() {
                    widgets::vline(ui, 16.0);
                    let is_err = app.ui.status_error || app.ui.status.starts_with("Couldn");
                    ui.label(RichText::new(&app.ui.status).color(if is_err { t.warning } else { t.text_faint }));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if app.session.active().is_some()
                        && !t.pro
                        && widgets::secondary_button(ui, tl!("Fit"), 0.0).clicked()
                        && let Some(i) = app.session.active_index()
                    {
                        app.ui.views[i].fit_pending = true;
                    }
                    if app.session.has_jobs() {
                        ui.add_space(6.0);
                        crate::jobs_ui::status_progress(app, ui);
                    }
                });
            });
        });
}

// ----------------------------------------------------------------------------- dock

pub fn right_dock(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let p = app.ui.panels.clone();
    if t.pro {
        dock_panels(app, ui, &p, &t);
    }
    // Narrow icon rail (always visible): shows, expands or collapses panel groups.
    let (rw, rb) = if t.pro { (36.0, 28.0) } else { (44.0, 32.0) };
    egui::Panel::right("rail")
        .resizable(false)
        .exact_size(rw)
        .frame(egui::Frame::NONE.fill(palette::color(ui.ctx(), ColorSlot::Toolbar)).inner_margin(egui::Margin::symmetric(4, 8)))
        .show(ui, |ui| {
            let r = ui.max_rect();
            ui.painter().line_segment([r.left_top() - vec2(6.0, 8.0), r.left_bottom() + vec2(-6.0, 8.0)], Stroke::new(1.0, t.separator));
            ui.spacing_mut().item_spacing.y = 4.0;
            use crate::dock::Group;
            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("panel-icon-rail-rect"), r));
            let entries: [(&str, &str, Group); 7] = [
                ("sliders-horizontal", tl!("Properties"), Group::Properties),
                ("navigation", tl!("Navigator"), Group::Navigator),
                ("palette", tl!("Color & Swatches"), Group::Color),
                ("layers", tl!("Layers"), Group::Layers),
                ("clock", tl!("History"), Group::History),
                ("brush", tl!("Brushes"), Group::Brush),
                ("settings", tl!("Tool Presets"), Group::ToolPresets),
            ];
            for (icon, name, g) in entries {
                // Studio floats Properties outside the dock.
                let legacy_floating = match g {
                    Group::Brush => p.brush_floating,
                    Group::ToolPresets => p.tool_presets_floating,
                    _ => false,
                };
                let docked = (t.pro || g != Group::Properties) && !app.ui.dock.floating.contains(&g) && !legacy_floating;
                let on = crate::dock::rail_active(app, g, docked);
                let button = icons::rail_button(ui, icon, rb, on, name);
                let drag = ui.interact(button.rect, ui.id().with(("rail-panel-drag", g)), Sense::click_and_drag());
                drag.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
                if drag.drag_started() && !app.session.prefs().workspace_locked {
                    crate::panel_workspace::migrate(app, ui.ctx());
                    crate::dock::reveal(app, g);
                }
                if drag.dragged()
                    && !app.session.prefs().workspace_locked
                    && let Some(c) = app.ui.dock.containers.iter().find(|c| c.tabs.iter().any(|t| t.group == g))
                {
                    let tab = if c.tabs.iter().all(|t| t.group == g) { None } else { c.tabs.iter().position(|t| t.group == g) };
                    egui::DragAndDrop::set_payload(ui.ctx(), crate::panel_workspace::Payload { container: c.id, tab });
                }
                if drag.clicked() {
                    crate::dock::rail_click(app, g, docked);
                }
            }
        });
    if !t.pro {
        dock_panels(app, ui, &p, &t);
    }
    crate::dock::persist(app, ui.ctx());
}

/// The right dock's width range (points).
const DOCK_WIDTH: std::ops::RangeInclusive<f32> = 250.0..=520.0;

fn dock_width_id() -> egui::Id {
    egui::Id::new("dock-width-request")
}

/// Set the right dock's width on the next frame (`ui.set {dockWidth}`); clamped to its range.
pub fn request_dock_width(ctx: &egui::Context, w: f32) {
    let w = if w.is_finite() { w.clamp(*DOCK_WIDTH.start(), *DOCK_WIDTH.end()) } else { *DOCK_WIDTH.start() };
    ctx.data_mut(|d| d.insert_temp(dock_width_id(), w));
}

fn dock_panels(app: &mut PhotocraftApp, ui: &mut egui::Ui, p: &crate::state::Panels, t: &Tokens) {
    use crate::dock::Group;
    // Floating in Studio, Properties docks only in Pro (Photoshop).
    let shown: Vec<Group> = if !app.ui.dock.containers.is_empty() {
        crate::panel_workspace::docked_groups(app)
    } else {
        [
            (Group::Color, p.color),
            (Group::Properties, t.pro && p.properties),
            (Group::Character, p.character),
            (Group::Navigator, p.navigator),
            (Group::History, p.history),
            (Group::Brush, p.brush_settings && !p.brush_floating),
            (Group::ToolPresets, p.tool_presets && !p.tool_presets_floating),
            (Group::Layers, p.layers),
        ]
        .into_iter()
        .filter_map(|(g, on)| (on && !app.ui.dock.floating.contains(&g)).then_some(g))
        .collect()
    };
    if shown.is_empty() {
        return;
    }
    let margin = if t.pro { 2 } else { 8 };
    let mut panel = egui::Panel::right("dock").resizable(true).default_size(if t.pro { 290.0 } else { 300.0 }).size_range(DOCK_WIDTH);
    let restored = if std::mem::take(&mut app.ui.shell.restore_dock_width) { app.ui.dock.width } else { None };
    if let Some(w) = ui.ctx().data_mut(|d| d.remove_temp::<f32>(dock_width_id())).or(restored) {
        panel = panel.exact_size(w.clamp(*DOCK_WIDTH.start(), *DOCK_WIDTH.end()));
    }
    let response = panel.frame(egui::Frame::NONE.fill(t.dock).inner_margin(egui::Margin::same(margin))).show(ui, |ui| {
        // Groups keep their heights whatever they show (#88): see `dock`.
        crate::dock::show(app, ui, &shown, dock_body);
    });
    app.ui.dock.width = Some(response.response.rect.width());
}

/// One dock group's tab content; `dock` bounds it and scrolls it when it's taller.
pub(crate) fn dock_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, group: crate::dock::Group, tab: usize) {
    use crate::dock::Group;
    let pro = Tokens::get(ui.ctx()).pro;
    match (group, tab) {
        (Group::Color, 2) => crate::preset_panels::gradients_panel(app, ui),
        (Group::Color, 3) => crate::preset_panels::patterns_panel(app, ui),
        (Group::Color, 0) if pro => color_field(app, ui),
        (Group::Color, _) if pro => swatches(app, ui),
        (Group::Color, 0) => swatches(app, ui),
        (Group::Color, _) => color_picker(app, ui),
        (Group::Properties, 0) => properties_body(app, ui),
        (Group::Properties, _) => adjustments_grid(app, ui),
        (Group::Character, tab) => crate::type_tool::character_panel(app, ui, tab == 1),
        (Group::Navigator, 0) => navigator(app, ui),
        (Group::Navigator, 1) => crate::tone::histogram_panel(app, ui),
        (Group::Navigator, _) => info_panel(app, ui),
        (Group::History, 0) => history(app, ui),
        (Group::History, 1) => crate::actions::panel(app, ui),
        (Group::History, _) => crate::comps_ui::panel(app, ui),
        (Group::Brush, tab) => crate::brush_panel::body(app, ui, tab),
        (Group::ToolPresets, _) => crate::preset_panels::tool_presets_panel(app, ui),
        (Group::Layers, 0) => layers(app, ui),
        (Group::Layers, 1) => channels(app, ui),
        (Group::Layers, _) => crate::vector_ui::paths_panel(app, ui),
    }
}

/// Photoshop's Info panel: colour under the pointer (RGB and CMYK), position, selection size.
fn info_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No document"));
        return;
    };
    let (rev, size) = (st.revision, st.doc.size);
    // Positions and sizes in the ruler unit (Preferences › Units & Rulers).
    let units = app.session.prefs().units_and_rulers.clone();
    let dpi = st.doc.resolution_dpi as f64;
    let fx = move |px: f64| units.format(px, dpi, size.width as f64);
    let units_y = app.session.prefs().units_and_rulers.clone();
    let fy = move |px: f64| units_y.format(px, dpi, size.height as f64);
    let sel = st.doc.selection.as_ref().map(photocraft_compose::bounds::content_bounds);
    let pos = app
        .hover_doc
        .map(|p| (p[0].floor() as i32, p[1].floor() as i32))
        .filter(|(x, y)| *x >= 0 && *y >= 0 && *x < size.width as i32 && *y < size.height as i32);
    let rgba = pos.and_then(|(x, y)| {
        if let Some(((cx, cy, cr), v)) = app.info_sample
            && (cx, cy, cr) == (x, y, rev)
        {
            return Some(v);
        }
        let v: Vec<f32> = serde_json::from_value(app.session.execute("document.pixel", json!({"x": x, "y": y})).ok()?).ok()?;
        let v = [v[0], v[1], v[2], v[3]];
        app.info_sample = Some(((x, y, rev), v));
        Some(v)
    });
    let mono = theme::mono(11.5);
    let row = |ui: &mut egui::Ui, k: &str, v: String| {
        ui.horizontal(|ui| {
            ui.add_sized([18.0, 16.0], egui::Label::new(RichText::new(k).color(t.text_dim).size(11.5)));
            ui.label(RichText::new(v).font(mono.clone()).color(t.text));
        });
    };
    ui.columns(2, |cols| {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
        let dash = || "—".to_string();
        row(&mut cols[0], "R:", rgba.map_or_else(dash, |c| q(c[0]).to_string()));
        row(&mut cols[0], "G:", rgba.map_or_else(dash, |c| q(c[1]).to_string()));
        row(&mut cols[0], "B:", rgba.map_or_else(dash, |c| q(c[2]).to_string()));
        // Naive device CMYK (no ICC profile yet), in percent like Photoshop's readout.
        let cmyk = rgba.map(|c| {
            let k = 1.0 - c[0].max(c[1]).max(c[2]);
            let f = |v: f32| if k >= 1.0 { 0.0 } else { (1.0 - v - k) / (1.0 - k) };
            [f(c[0]), f(c[1]), f(c[2]), k].map(|v| (v * 100.0).round() as i32)
        });
        for (i, k) in ["C:", "M:", "Y:", "K:"].iter().enumerate() {
            row(&mut cols[1], k, cmyk.map_or_else(dash, |c| format!("{}%", c[i])));
        }
    });
    widgets::hairline(ui);
    ui.columns(2, |cols| {
        let dash = || "—".to_string();
        row(&mut cols[0], "X:", pos.map_or_else(dash, |p| fx(p.0 as f64)));
        row(&mut cols[0], "Y:", pos.map_or_else(dash, |p| fy(p.1 as f64)));
        row(&mut cols[1], "W:", sel.map_or_else(dash, |r| fx(r.width() as f64)));
        row(&mut cols[1], "H:", sel.map_or_else(dash, |r| fy(r.height() as f64)));
    });
    widgets::hairline(ui);
    ui.label(
        RichText::new(crate::i18n::fmt(tl!("Doc: {w} × {h} px"), &[("w", &size.width.to_string()), ("h", &size.height.to_string())]))
            .color(t.text_dim)
            .size(11.5),
    );
}

fn navigator(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(idx) = app.session.active_index() else {
        empty(ui, tl!("No document"));
        return;
    };
    let ctx = ui.ctx().clone();
    let Some(tex) = crate::canvas::navigator_texture(app, &ctx, idx) else { return };
    let size = app.session.documents()[idx].doc.size;
    let avail = ui.available_width();
    let box_h = 150.0;
    let (frame, resp) = ui.allocate_exact_size(vec2(avail, box_h), Sense::click_and_drag());
    ui.painter().rect_filled(frame, t.radius_sm, t.canvas);
    let aspect = size.width as f32 / size.height.max(1) as f32;
    let (w, h) = if aspect > avail / box_h { (avail - 16.0, (avail - 16.0) / aspect) } else { ((box_h - 16.0) * aspect, box_h - 16.0) };
    let rect = Rect::from_center_size(frame.center(), vec2(w, h));
    widgets::checker(ui.painter(), rect, 6.0);
    ui.painter().image(tex, rect, Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
    let s = w / size.width as f32;
    if (resp.clicked() || resp.dragged())
        && let Some(pp) = resp.interact_pointer_pos()
    {
        let d = (pp - rect.min) / s;
        app.ui.views[idx].center = [d.x.clamp(0.0, size.width as f32), d.y.clamp(0.0, size.height as f32)];
        app.ui.views[idx].fit_pending = false;
    }
    // Visible-area rectangle.
    let v = app.ui.views[idx].clone();
    let canvas = app.last_canvas_rect;
    let vw = canvas.width() / v.zoom * s;
    let vh = canvas.height() / v.zoom * s;
    let c = pos2(rect.min.x + v.center[0] * s, rect.min.y + v.center[1] * s);
    let vr = Rect::from_center_size(c, vec2(vw, vh)).intersect(frame.shrink(1.0));
    ui.painter().rect_stroke(vr, 2.0, Stroke::new(1.5, Color32::from_rgb(255, 84, 84)), StrokeKind::Middle);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        label(ui, tl!("Zoom"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            for (lbl, z) in [("200%", 2.0), ("100%", 1.0)] {
                if widgets::pill_tab(ui, lbl, (v.zoom - z).abs() < 1e-3).clicked() {
                    app.ui.views[idx].zoom = z;
                    app.ui.views[idx].fit_pending = false;
                }
            }
            if widgets::pill_tab(ui, tl!("Fit"), false).clicked() {
                app.ui.views[idx].fit_pending = true;
            }
        });
    });
    let mut lz = v.zoom.max(0.01).log2();
    if widgets::slider(ui, &mut lz, -6.64..=5.0, None).changed() {
        app.ui.views[idx].zoom = 2f32.powf(lz);
        app.ui.views[idx].fit_pending = false;
    }
}

fn empty(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    ui.label(RichText::new(s).color(t.text_faint));
    ui.add_space(6.0);
}

fn swatches(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::color_panel_ui::swatches::show(app, ui);
}

fn color_picker(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::color_panel_ui::show(app, ui);
}

// ----------------------------------------------------------------------------- layers

/// The simple themes' single lock button. On the Background it converts the layer to a normal
/// one, as clicking the Background's lock does in Photoshop. Elsewhere it clears every lock when
/// any is set, and locks all otherwise. Toggling only "lock all" would leave the Background's
/// transparency lock (still showing as locked), and a second click would lock its pixels,
/// shutting out the Eraser and every other paint tool (#76).
#[cfg(test)]
fn simple_lock_toggle(background: bool, l: &Layer) -> (String, Value) {
    if background {
        return ("layer.new.layerFromBackground".into(), json!({}));
    }
    let k = &l.locks;
    let locked = k.transparency || k.pixels || k.position || k.artboard || k.all;
    let locks = if locked { json!({"transparency": false, "pixels": false, "position": false, "artboard": false, "all": false}) } else { json!({"all": true}) };
    ("layer.setProps".into(), json!({"layer": l.id.0, "locks": locks}))
}

fn blend_options(groups: bool) -> Vec<(BlendMode, &'static str)> {
    std::iter::once(BlendMode::PassThrough).filter(|_| groups).chain(BlendMode::LAYER_MODES).map(|m| (m, m.label())).collect()
}

fn layers(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::smart_ui::begin(app);
    // A new active layer opens its parent groups and is scrolled into view (#152).
    let filtering = app
        .session
        .active()
        .is_some_and(|state| crate::layer_filter_ui::is_filtering(&state.doc, &app.ui.layer_filter_options, &app.ui.layer_filter, &state.selected_layers()));
    let reveal = if filtering { None } else { crate::layer_reveal::track(app, ui.ctx()) };
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No document"));
        return;
    };
    let doc = st.doc.clone();
    let active = st.active_layer;
    let active_layer = active.and_then(|id| doc.layer(id));
    let selection = st.selected_layers();
    let isolated = st.isolated_layers.clone();
    let mut actions: Vec<(String, Value)> = Vec::new();

    let t = Tokens::get(ui.ctx());
    crate::layer_filter_ui::show(ui, &mut app.ui.layer_filter_options, &mut app.ui.layer_filter);
    if let Some(l) = active_layer {
        // Photoshop greys blend mode, Opacity and Fill for the Background layer.
        let bg = crate::doc_props_ui::is_background(&doc, l);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!bg, |ui| {
                let mut m = l.blend;
                // Leave room for the Opacity label and field: a translated label ("Непрозрачность:")
                // can be much wider than the English one, and must not slide under the dropdown.
                let opacity_label = if t.pro { tl!("Opacity:") } else { tl!("Opacity") };
                let right = (body_text_width(ui, opacity_label) + 66.0 + 2.0 * ui.spacing().item_spacing.x + 16.0).max(150.0);
                let w = ui.available_width() - right;
                if widgets::dropdown(ui, "blend", &mut m, &blend_options(l.is_group()), w.max(100.0)) {
                    actions.push(("layer.setProps".into(), json!({"layer": l.id.0, "blend": m.label()})));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut o = l.opacity * 100.0;
                    if widgets::value_field(ui, &mut o, 0.0..=100.0, "%", 66.0).changed() {
                        actions.push(("layer.setProps".into(), json!({"layer": l.id.0, "opacity": o / 100.0})));
                    }
                    label(ui, opacity_label);
                });
            });
        });
        // Docks zero the item spacing: keep the Opacity and Fill fields apart (#155).
        ui.add_space((theme::ROW_GAP - ui.spacing().item_spacing.y).max(0.0));
        ui.horizontal(|ui| {
            let lock_label = if t.pro { tl!("Lock:") } else { tl!("Lock") };
            let fill_label = if t.pro { tl!("Fill:") } else { tl!("Fill") };
            // In a narrow panel with long translated labels, drop the "Lock:" text (the icons keep
            // their tooltips) rather than let the Fill label run over the lock icons.
            let gap = 2.0;
            ui.spacing_mut().item_spacing.x = gap;
            // Fill has one internal label/value gap; its outer gap is reserved below.
            let fill_width = body_text_width(ui, fill_label) + 66.0 + gap + 2.0;
            let fits = body_text_width(ui, lock_label) + crate::layer_filter_ui::LOCKS_MIN_WIDTH + fill_width + 2.0 * gap <= ui.available_width();
            if fits {
                label(ui, lock_label);
            }
            let icons_w = crate::layer_filter_ui::locks_width((ui.available_width() - fill_width - gap).max(0.0));
            crate::layer_filter_ui::locks(ui, &doc, &selection, &mut actions, icons_w);
            ui.add_enabled_ui(!bg, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut f = l.fill_opacity * 100.0;
                    if widgets::value_field(ui, &mut f, 0.0..=100.0, "%", 66.0).changed() {
                        actions.push(("layer.setProps".into(), json!({"layer": l.id.0, "fill": f / 100.0})));
                    }
                    label(ui, fill_label);
                })
            });
        });
        ui.add_space(2.0);
    }

    // Top of the stack first, groups above their contents, closed groups' contents hidden (#126).
    let options = app.ui.layer_filter_options.clone();
    let kinds = app.ui.layer_filter.clone();
    let board_matches = (options.enabled && options.mode == crate::layer_filter_ui::Mode::Artboard)
        .then(|| crate::layer_filter_ui::artboard_match_ids(&doc, &selection))
        .flatten();
    let filtering = options.active(&kinds) && (options.mode != crate::layer_filter_ui::Mode::Artboard || board_matches.is_some());
    let rows: Vec<_> = crate::layer_filter_ui::rows(&doc, &options, &kinds, &selection)
        .into_iter()
        .filter(|(_, layer)| photocraft_engine::select_extra_cmds::isolation_shows(&doc, &isolated, layer.id))
        .collect();
    let ctx = ui.ctx().clone();
    let footer = 38.0;
    let fill = ui.available_height() > footer + 60.0;
    let rows_h = if fill { ui.available_height() - footer } else { f32::INFINITY };
    egui::ScrollArea::vertical()
        .id_salt("layer-rows")
        .max_height(rows_h)
        .min_scrolled_height(if fill { rows_h } else { 0.0 })
        .auto_shrink([false, !fill])
        .show_palette(ui, |ui| {
            let fx_collapsed = app.session.active().map(|d| d.fx_collapsed.clone()).unwrap_or_default();
            let expansions: Vec<_> =
                rows.iter().map(|(_, layer)| (layer.id.0, !fx_collapsed.contains(&layer.id), crate::smart_ui::expanded(app, layer.id))).collect();
            let clipping = crate::layer_row_ui::clipping::begin(ui, &doc, &rows, &expansions);
            let clipping_owns_pointer = clipping.owns_pointer;
            crate::layer_row_ui::begin(ui.ctx());
            let row_actions_start = actions.len();
            ui.scope(|ui| {
                if clipping_owns_pointer {
                    // Only this list is suppressed. Preserve its existing painter opacity:
                    // a modifier gesture must not flash a disabled-looking Layers panel.
                    let opacity = ui.opacity();
                    ui.disable();
                    ui.set_opacity(opacity);
                }
                for &(depth, l) in &rows {
                    let row = RowSel {
                        selected: selection.contains(&l.id),
                        primary: active == Some(l.id),
                        multi: selection.len() > 1,
                        filtering,
                        filter_context: filtering && board_matches.as_ref().map_or_else(|| !options.matches(l, &kinds, &selection), |ids| !ids.contains(&l.id)),
                    };
                    let top = ui.cursor().top();
                    layer_row(app, &ctx, ui, &doc, l, depth, row, &mut actions);
                    if reveal == Some(l.id) {
                        crate::layer_reveal::scroll_to_row(ui, top);
                    }
                    if !l.effects.items.is_empty() && fx_collapsed.iter().all(|id| *id != l.id) {
                        effect_rows(app, ui, l, depth);
                    }
                    crate::smart_ui::filter_rows(app, ui, l, depth, &mut actions);
                }
            });
            if clipping_owns_pointer {
                actions.truncate(row_actions_start);
            }
            clipping.finish(ui, &doc, &mut actions);
            // A rename whose row is gone (deleted, filtered out, inside a closed group) ends,
            // committed: nothing else could commit or cancel it.
            if let Some(layer) = crate::layer_row_ui::renaming(ui.ctx())
                && !crate::layer_row_ui::recorded(ui.ctx()).iter().any(|r| r.layer == layer)
                && let Some(done) = crate::layer_row_ui::end_rename(ui.ctx(), true)
            {
                actions.push(done);
            }
        });
    // A layer being dragged can also be dropped on the footer's Delete, New Layer and Group
    // buttons (#736); read the drag before it ends.
    let drag_cancelled = layer_drag_cancelled(&ctx);
    let footer_drag = ctx.data(|d| d.get_temp::<u64>(egui::Id::new("layer-drag"))).filter(|_| !drag_cancelled).map(|id| {
        let in_selection = selection.iter().any(|s| s.0 == id);
        (id, in_selection)
    });
    // End any layer drag after every row has had a chance to accept the drop.
    if ctx.input(|i| i.pointer.any_released()) || drag_cancelled {
        if drag_cancelled {
            crate::layer_transfer::cancel_panel_drag(&ctx);
        }
        ctx.data_mut(|d| {
            let key = egui::Id::new("layer-drag");
            d.remove::<u64>(key);
            d.remove::<Vec<u64>>(key.with("selection"));
        });
    }
    ui.add_space(4.0);
    widgets::hairline(ui);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let trash = icons::button(ui, "trash", 26.0, false, tl!("Delete layer"));
            if trash.clicked() {
                actions.push(("layer.delete".into(), json!({})));
            }
            actions.extend(footer_drop(ui, &trash, footer_drag, "layer.delete"));
            let new_layer = icons::button(ui, "square-plus", 26.0, false, &crate::shortcuts::tip_label(app, "Create a new layer", "layer.new.layer"));
            if new_layer.clicked() {
                actions.push(("layer.new.layer".into(), json!({})));
            }
            actions.extend(footer_drop(ui, &new_layer, footer_drag, "layer.duplicate"));
            let group = icons::button(ui, "folder", 26.0, false, tl!("Create a new group"));
            if group.clicked() {
                actions.push(("layer.new.group".into(), json!({})));
            }
            actions.extend(footer_drop(ui, &group, footer_drag, "layer.groupLayers"));
            let adj = icons::button(ui, "contrast", 26.0, false, tl!("Create new fill or adjustment layer"));
            if !ctx.embed_viewports() {
                if adj.clicked() {
                    crate::layer_menu_ui::open_footer(app, &ctx, crate::layer_menu_ui::FooterMenu::Adjustment, adj.rect.left_bottom());
                }
            } else {
                egui::Popup::menu(&adj).show(|ui| {
                    ui.set_min_width(190.0);
                    for c in photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with("layer.newAdjustmentLayer.")) {
                        if ui.button(tl!(c.label).trim_end_matches('…')).clicked() {
                            actions.push((c.id.into(), json!({})));
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button(tl!("Solid Color…")).clicked() {
                        actions.push(("layer.newFillLayer.solidColor".into(), json!({})));
                        ui.close();
                    }
                    if ui.button(tl!("Gradient…")).clicked() {
                        actions.push(("layer.newFillLayer.gradient".into(), json!({})));
                        ui.close();
                    }
                });
            }
            if icons::button(
                ui,
                "square-dot",
                26.0,
                false,
                &crate::i18n::fmt(tl!("Add a mask  (from the selection; {key} inverts)"), &[("key", &crate::shortcuts::pretty("Alt"))]),
            )
            .clicked()
            {
                let alt = ui.input(|i| i.modifiers.alt);
                actions.push((crate::layer_menu_ui::add_mask_command(doc.selection.is_some(), alt).into(), json!({})));
            }
            let fx = fx_button(ui, 26.0, tl!("Add a layer style"));
            if !ctx.embed_viewports() {
                if fx.clicked() {
                    crate::layer_menu_ui::open_footer(app, &ctx, crate::layer_menu_ui::FooterMenu::Style, fx.rect.left_bottom());
                }
            } else {
                egui::Popup::menu(&fx).show(|ui| {
                    ui.set_min_width(180.0);
                    if ui.button(tl!("Blending Options…")).clicked() {
                        crate::layer_style::open(app, None);
                        ui.close();
                    }
                    ui.separator();
                    for &(kind, label) in crate::layer_style::KINDS {
                        if ui.button(format!("{}…", tl!(label))).clicked() {
                            crate::layer_style::open(app, Some(kind));
                            ui.close();
                        }
                    }
                });
            }
            // Photoshop's footer starts with Link Layers (enabled with two or more layers selected).
            let can_link = app.session.is_enabled("layer.linkLayers");
            if ui.add_enabled_ui(can_link, |ui| icons::button(ui, "link", 26.0, false, tl!("Link layers"))).inner.clicked() {
                actions.push(("layer.linkLayers".into(), json!({})));
            }
        });
    });
    for (id, p) in actions {
        if id == crate::smart_ui::SET_EXPANDED {
            if let Err(error) = crate::menus::invoke(app, &ctx, &id, p) {
                app.ui.status = error;
                app.ui.status_error = true;
            }
            continue;
        }
        if id == "ui.maskTarget" {
            app.ui.mask_target = p.as_bool().unwrap_or(false);
            app.ui.vector_mask_target = false;
            continue;
        }
        if id == "ui.vectorMaskTarget" {
            app.ui.vector_mask_target = p.as_bool().unwrap_or(false);
            app.ui.mask_target = false;
            continue;
        }
        if p.is_null() {
            // Context-menu items behave like their menu-bar twins (dialogs included).
            let ctx = ui.ctx().clone();
            if let Err(e) = crate::menus::invoke(app, &ctx, &id, json!({})) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            continue;
        }
        if id.starts_with("layer.newAdjustmentLayer.") || id.starts_with("layer.newFillLayer.") {
            let _ = crate::menus::invoke(app, &ctx, &id, p);
        } else {
            let _ = app.run(&id, p);
        }
    }
}

#[cfg(test)]
#[path = "layer_filter_panel_tests_v79.rs"]
mod layer_filter_panel_tests_v79;

/// A layer row dragged onto a Layers panel footer button, as in Photoshop: onto Delete deletes it,
/// onto New Layer duplicates it, onto New Group groups it. A row that is part of the selection
/// carries the whole selection; any other row goes alone. Highlights the button while over it and
/// returns the command on release.
fn footer_drop(ui: &egui::Ui, button: &egui::Response, drag: Option<(u64, bool)>, command: &str) -> Option<(String, Value)> {
    // Named for screen readers (and tests) after the command a drop runs.
    button.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, footer_label(command)));
    let (layer, in_selection) = drag?;
    let p = ui.ctx().input(|i| i.pointer.interact_pos())?;
    if !button.rect.contains(p) {
        return None;
    }
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_stroke(button.rect, t.radius_sm, Stroke::new(2.0, t.accent), StrokeKind::Inside);
    if !ui.ctx().input(|i| i.pointer.any_released()) {
        return None;
    }
    Some((command.into(), if in_selection { json!({}) } else { json!({"layer": layer}) }))
}

fn footer_label(command: &str) -> &'static str {
    match command {
        "layer.delete" => tl!("Delete layer"),
        "layer.duplicate" => tl!("Create a new layer"),
        _ => tl!("Create a new group"),
    }
}

/// Photoshop's italic "fx" footer button (no icon-font equivalent).
fn fx_button(ui: &mut egui::Ui, size: f32, tip: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, t.radius_sm, t.hover);
    }
    let mut job = egui::text::LayoutJob::default();
    job.append("fx", 0.0, egui::TextFormat { font_id: egui::FontId::proportional(15.0), color: t.icon, italics: true, ..Default::default() });
    let g = ui.painter().layout_job(job);
    ui.painter().galley(r.center() - g.size() / 2.0, g, t.icon);
    resp.on_hover_text(tip)
}

/// How a Layers panel row is selected: in the (multi-)selection, the primary/active layer, and
/// whether several layers are selected.
#[derive(Clone, Copy)]
struct RowSel {
    selected: bool,
    primary: bool,
    multi: bool,
    filtering: bool,
    filter_context: bool,
}

/// `layer.select` mode for a click with these modifiers (Photoshop: ⌘-click toggles, ⇧-click
/// selects a range, a plain click selects one layer).
fn select_mode(m: egui::Modifiers) -> &'static str {
    if m.command {
        "toggle"
    } else if m.shift {
        "range"
    } else {
        "replace"
    }
}

/// A drag down the Layers panel's eye column (Photoshop): the first eye toggles and every row
/// swept over gets the same visibility, once per row, all in one history step.
#[derive(Clone)]
struct EyeSweep {
    visible: bool,
    /// The `coalesce` key shared by the drag's `layer.setProps` calls.
    key: u64,
    swept: Vec<u64>,
}

/// Starts an eye-column sweep from `eye`, or applies a running one to the row `row` of `l`.
fn eye_sweep(ctx: &egui::Context, l: &Layer, row: Rect, eye: &egui::Response, actions: &mut Vec<(String, Value)>) {
    let id = egui::Id::new("layer-eye-sweep");
    let set =
        |visible: bool, key: u64| ("layer.setProps".to_string(), json!({"layer": l.id.0, "visible": visible, "coalesce": format!("layer-eye-sweep:{key}")}));
    if eye.drag_started() {
        let sweep = EyeSweep { visible: !l.visible, key: ctx.cumulative_pass_nr(), swept: vec![l.id.0] };
        actions.push(set(sweep.visible, sweep.key));
        ctx.data_mut(|d| d.insert_temp(id, sweep));
        return;
    }
    let Some(mut sweep) = ctx.data(|d| d.get_temp::<EyeSweep>(id)) else { return };
    let (down, pos, delta) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.interact_pos(), i.pointer.delta()));
    if !down {
        ctx.data_mut(|d| d.remove::<EyeSweep>(id));
        return;
    }
    // Everything the pointer passed since the last frame, so a fast drag skips no row.
    let Some(p) = pos else { return };
    let (y0, y1) = ((p.y - delta.y).min(p.y), (p.y - delta.y).max(p.y));
    if y1 < row.top() || y0 >= row.bottom() || sweep.swept.contains(&l.id.0) {
        return;
    }
    if l.visible != sweep.visible {
        actions.push(set(sweep.visible, sweep.key));
    }
    sweep.swept.push(l.id.0);
    ctx.data_mut(|d| d.insert_temp(id, sweep));
}

#[allow(clippy::too_many_arguments)]
fn layer_row(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    ui: &mut egui::Ui,
    doc: &photocraft_doc::Document,
    l: &Layer,
    depth: usize,
    row: RowSel,
    actions: &mut Vec<(String, Value)>,
) {
    let selected = row.selected;
    let t = Tokens::get(ctx);
    // Photoshop's default (medium) thumbnails: 32 pt rows.
    let row_h = if t.pro { 32.0 } else { 46.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click_and_drag());
    layer_drag_and_drop(app, ctx, ui, l, rect, &resp, actions);
    if resp.drag_started() {
        crate::layer_transfer::begin_from_panel(app, ctx, l.id);
    }
    // Rows are painted: name them for screen readers and UI tests.
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected, &l.name));
    // A row scrolled out of view only keeps its place (#125): a layout's hundreds of rows would
    // otherwise lay out names, icons and thumbnails every frame. Group rows stay whole: their
    // disclosure triangle is a widget (accessibility, scroll-to).
    if !ui.is_rect_visible(rect) && !l.is_group() && !resp.context_menu_opened() && crate::layer_row_ui::renaming(ctx) != Some(l.id.0) {
        return;
    }
    let painter = ui.painter_at(rect.expand(1.0));
    if t.pro {
        if selected {
            painter.rect_filled(rect, 0.0, t.row_selected);
        } else if resp.hovered() {
            painter.rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
        }
        // eye column divider + row divider, as in Photoshop
        painter.line_segment([pos2(rect.left() + 30.0, rect.top()), pos2(rect.left() + 30.0, rect.bottom())], Stroke::new(1.0, t.separator));
        painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.separator));
    } else if selected {
        painter.rect_filled(rect, t.radius, t.hover);
        painter.rect_stroke(rect, t.radius, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    } else if resp.hovered() {
        painter.rect_filled(rect, t.radius, t.hover.gamma_multiply(0.5));
    }
    // Calculate indentation before painting: the label spans the visibility/indent gutter,
    // and is drawn behind its eye icon without reaching the thumbnail or selection text.
    let others = 6.0
        + 28.0
        + if l.is_group() { crate::layer_tree_ui::TRIANGLE_W } else { 0.0 }
        + if l.clipped { 12.0 } else { 0.0 }
        + crate::layer_row_ui::reserved_width(l);
    let ts = crate::mask_thumbs_ui::thumb_size(l, if t.pro { 24.0 } else { 34.0 }, rect.width() - others);
    let fixed = others + ts + 6.0 + crate::mask_thumbs_ui::width(l, ts);
    let indent = crate::layer_row_ui::indent(depth, rect.width(), fixed);
    let label_rect = crate::layer_row_ui::paint_label(&painter, rect, indent, l.label, &t);
    let mut x = rect.left() + 6.0;
    let eye = Rect::from_min_size(pos2(x, rect.center().y - 11.0), vec2(22.0, 22.0));
    // The eye takes drags too (so a drag starting on it never reorders the row): dragging down
    // the eyes gives every row swept over the visibility the first eye toggled to.
    let eye_resp = ui.interact(eye, ui.id().with(("eye", l.id.0)), Sense::click_and_drag());
    let visibility_icon = if l.visible { "eye" } else { "eye-off" };
    icons::paint(ui, eye, visibility_icon, 15.0, crate::layer_row_ui::visibility_tint(l.label, &t));
    eye_resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, l.visible, format!("{} {}", tl!("Visibility"), l.name)));
    eye_sweep(ctx, l, rect, &eye_resp, actions);
    if eye_resp.clicked() {
        // ⌥-click shows only this layer; ⌥-click it again to restore the others.
        if ui.input(|i| i.modifiers.alt) {
            actions.push(("layer.showOnly".into(), json!({"layer": l.id.0})));
        } else {
            actions.push(("layer.setProps".into(), json!({"layer": l.id.0, "visible": !l.visible})));
        }
    }
    x += 28.0 + indent;
    let toggled = if row.filtering {
        crate::layer_tree_ui::disclosure_filtered(ui, rect, &mut x, l, true, actions)
    } else {
        crate::layer_tree_ui::disclosure(ui, rect, &mut x, l, actions)
    };
    if l.clipped {
        crate::layer_row_ui::paint_clipping_arrow(&painter, pos2(x + 5.0, rect.center().y), t.text);
        x += 12.0;
    }
    let thumb = Rect::from_min_size(pos2(x, rect.center().y - ts / 2.0), vec2(ts, ts));
    draw_layer_thumb(app, ctx, ui, doc, l, thumb, row.primary);
    x += ts + 6.0;
    // Link chains and pixel / vector mask thumbnails (#153).
    let masks = crate::mask_thumbs_ui::paint(app, ctx, ui, &painter, doc, l, &mut x, rect.center().y, ts, actions);
    let mask_rect = masks.thumb(crate::mask_thumbs_ui::MaskKind::Pixel);
    let vector_rect = masks.thumb(crate::mask_thumbs_ui::MaskKind::Vector);
    // Photoshop frames the targeted thumbnail (pixels, mask or vector mask) of the active layer
    // with corner brackets.
    if row.primary {
        // A group has a folder glyph, not a pixel thumbnail. Actual group masks
        // retain their target brackets, but the folder itself needs no frame.
        let layer_target = (!l.is_group()).then_some(thumb);
        let target = if app.ui.vector_mask_target && vector_rect.is_some() {
            vector_rect
        } else if app.ui.mask_target {
            mask_rect.or(layer_target)
        } else {
            layer_target
        };
        if let Some(r) = target {
            crate::mask_thumbs_ui::paint_brackets(&painter, r, t.text);
        }
    }
    // Right-hand indicators first; the name gets what is left and ends in "…" (#144).
    let fx_open = app.session.active().is_none_or(|d| !d.fx_collapsed.contains(&l.id));
    let expansion = crate::layer_row_ui::Expansion { effects: fx_open, smart_filters: crate::smart_ui::expanded(app, l.id) };
    let (name_right, indicators, fx_toggled) = crate::layer_row_ui::indicators(ui, &painter, rect, x, l, expansion, actions);
    if let Some((_, disclosure)) = indicators.iter().find(|(kind, _)| *kind == crate::layer_row_ui::Indicator::SmartFiltersTriangle) {
        crate::smart_ui::record_layer(app, doc.id, l, rect, *disclosure);
    }
    let name_color = if row.filter_context {
        t.text_dim
    } else if l.visible {
        t.text
    } else {
        t.text_faint
    };
    let font = if selected && !t.pro { theme::medium(13.0) } else { egui::FontId::proportional(if t.pro { 12.0 } else { 13.0 }) };
    // Photoshop before 2026 set the Background layer's name in italics; 2026 sets it upright.
    let italic = !t.pro && l.name == "Background" && l.locks.transparency;
    // Photoshop rows show only the name; the kind sub-label is a Studio-theme addition.
    let is_pixel = t.pro || matches!(l.content, LayerContent::Raster(_));
    let name_rect = crate::layer_row_ui::truncated(&painter, &l.name, font, name_color, italic, name_right - x).map(|galley| {
        let text_pos = pos2(x, rect.center().y - galley.size().y / 2.0 - if is_pixel { 0.0 } else { 7.0 });
        let r = Rect::from_min_size(text_pos, galley.size());
        painter.galley(text_pos, galley, name_color);
        r
    });
    if !is_pixel {
        let sub = match &l.content {
            LayerContent::Adjustment(a) => tl!(a.label()).to_string(),
            LayerContent::Group(g) => crate::i18n::trn(crate::i18n::current(), g.children.len() as u64, "Group · {n} layer", "Group · {n} layers"),
            other => tl!(other.kind_name()).to_string(),
        };
        crate::layer_row_ui::label(&painter, x, rect.center().y + 8.0, name_right, &sub, egui::FontId::proportional(11.0), t.text_faint);
    }
    crate::layer_row_ui::record(
        ctx,
        crate::layer_row_ui::RowRects {
            layer: l.id.0,
            row: rect,
            thumbnail: thumb,
            name: name_rect,
            clipping_name_area: Rect::from_min_max(pos2(x, rect.top()), pos2(name_right.max(x), rect.bottom())),
            indicators,
            eye,
            visibility_icon,
            label: label_rect,
        },
    );
    // ⌘-click a layer, mask or vector-mask thumbnail loads its transparency / mask / path as a
    // selection (⇧ add, ⌥ subtract, ⇧⌥ intersect) instead of changing the layer selection.
    let thumb_load = resp.clicked().then(|| (ui.input(|i| i.modifiers), resp.interact_pointer_pos())).and_then(|(m, pos)| {
        let p = pos.filter(|_| m.command)?;
        if let Some(kind) = masks.hit(p) {
            return Some(crate::mask_thumbs_ui::load_params(l, kind, m));
        }
        thumb.expand(2.0).contains(p).then(|| json!({"channel": "transparency", "layer": l.id.0, "operation": crate::channels_panel::load_operation(m)}))
    });
    // ⇧-click a mask thumbnail: disable / enable that mask; ⌥-click a layer mask: view it.
    let mask_toggle = resp.clicked().then(|| (ui.input(|i| i.modifiers), resp.interact_pointer_pos())).and_then(|(m, pos)| {
        let kind = masks.hit(pos?)?;
        crate::mask_thumbs_ui::click_command(l, kind, m)
    });
    if let Some(p) = thumb_load {
        actions.push(("select.loadSelection".into(), p));
    } else if let Some(cmd) = mask_toggle {
        actions.push(cmd);
    } else if resp.clicked() && !eye_resp.clicked() && !toggled && !fx_toggled && !masks.clicked {
        crate::layer_row_ui::claim_keyboard_focus(ctx, doc.id, resp.id);
        let mode = select_mode(ui.input(|i| i.modifiers));
        actions.push(("layer.select".into(), json!({"layer": l.id.0, "mode": mode})));
        // Clicking a thumbnail picks what painting targets; adjustment/fill layers target their mask.
        let pos = resp.interact_pointer_pos();
        let on_mask = mask_rect.zip(pos).is_some_and(|(r, p)| r.expand(2.0).contains(p));
        let on_thumb = pos.is_some_and(|p| thumb.expand(2.0).contains(p));
        let on_vector = pos.and_then(|p| masks.hit(p)) == Some(crate::mask_thumbs_ui::MaskKind::Vector);
        // Clicking the layer thumbnail leaves mask view (#196).
        let viewing = app.session.active().and_then(photocraft_engine::mask_view_cmds::current).is_some_and(|v| v.layer == l.id);
        if on_thumb && viewing {
            actions.push((photocraft_engine::mask_view_cmds::ID.into(), json!({"layer": l.id.0, "mode": "off"})));
        }
        let content_less = matches!(l.content, LayerContent::Adjustment(_) | LayerContent::Fill(_));
        if on_vector {
            // The vector mask thumbnail targets the vector mask: the path tools edit it and the
            // Paths panel selects the layer's path.
            app.ui.selected_path = Some("layer".into());
            actions.push(("ui.vectorMaskTarget".into(), json!(true)));
        } else if on_mask || (content_less && l.mask.is_some()) {
            actions.push(("ui.maskTarget".into(), json!(true)));
        } else if on_thumb || !row.primary {
            actions.push(("ui.maskTarget".into(), json!(false)));
        }
    }
    // Double-click: the name renames in place (over the row's full height, not just the glyphs:
    // #651); the Background, which can't be renamed while it's locked, becomes a normal layer; an
    // adjustment or fill thumbnail opens its settings and a Smart Object thumbnail its contents,
    // and a type thumbnail edits its text; anywhere else on the row opens Layer Style (#350, #537).
    // The first click already made this the active layer.
    // A fast sequence after another panel button can be counted as a triple click
    // by egui. Repeated activation of an adjustment/fill thumbnail still opens its
    // editor; it must not quietly turn into selection-only behavior.
    let repeated_content_click = resp.triple_clicked()
        && matches!(l.content, LayerContent::Adjustment(_) | LayerContent::Fill(_))
        && resp.interact_pointer_pos().is_some_and(|p| thumb.expand(2.0).contains(p));
    if resp.double_clicked() || repeated_content_click {
        let pos = resp.interact_pointer_pos();
        let on = |r: Rect| pos.is_some_and(|p| r.expand(2.0).contains(p));
        if crate::doc_props_ui::is_background(doc, l) {
            actions.push(("layer.new.layerFromBackground".into(), json!({})));
        } else if name_rect.is_some_and(|n| on(Rect::from_x_y_ranges(n.x_range(), rect.y_range()))) {
            if let Some(done) = crate::layer_row_ui::start_rename(ctx, l.id.0, &l.name) {
                // One rename at a time: starting this one commits any other (#314).
                actions.push(done);
            }
        } else if pos.and_then(|p| masks.hit(p)).is_none() {
            let id = match &l.content {
                LayerContent::Adjustment(_) | LayerContent::Fill(_) if on(thumb) => "layer.layerContentOptions",
                LayerContent::Smart(_) if on(thumb) => "layer.smartObjects.editContents",
                LayerContent::Text(_) if on(thumb) => "type.editText",
                _ => "layer.layerStyle.blendingOptions",
            };
            if crate::menus::is_enabled(app, id) {
                // Null params: run like the menu item, dialog included.
                actions.push((id.into(), Value::Null));
            }
        }
    }
    let edit_rect = Rect::from_min_max(pos2(x - 3.0, rect.center().y - 11.0), pos2(name_right.max(x + 40.0), rect.center().y + 11.0));
    if let Some(done) = crate::layer_row_ui::rename_field(ui, l.id.0, edit_rect) {
        actions.push(done);
    }
    // Native tool/document windows cannot be covered by a popup painted into ROOT's
    // framebuffer. Detach this transient menu too, keeping embedded/Web behavior unchanged.
    if !ctx.embed_viewports() {
        if let Some(position) = crate::layer_row_ui::context_menu_press(&resp) {
            crate::layer_menu_ui::open(app, ctx, l, row.multi && selected, position);
        }
    } else {
        resp.context_menu(|ui| {
            // Right-clicking inside a multi-selection keeps it and acts on every selected layer.
            let on_set = row.multi && selected;
            if crate::layer_menu_ui::show(app, ui, l, on_set, actions)
                && let Some(done) = crate::layer_row_ui::start_rename(ui.ctx(), l.id.0, &l.name)
            {
                actions.push(done);
            }
        });
    }
}

fn draw_layer_thumb(app: &mut PhotocraftApp, ctx: &egui::Context, ui: &egui::Ui, doc: &photocraft_doc::Document, l: &Layer, rect: Rect, selected: bool) {
    let t = Tokens::get(ctx);
    let p = ui.painter();
    if l.is_group() {
        icons::paint(ui, rect, "folder", 18.0, t.icon);
        return;
    }
    match &l.content {
        LayerContent::Adjustment(_) => {
            p.rect_filled(rect, 6.0, t.field);
            icons::paint(ui, rect, "contrast", 18.0, t.icon);
        }
        LayerContent::Text(_) => {
            // Photoshop: type layers show a "T" tile rather than a pixel thumbnail.
            p.rect_filled(rect, if t.pro { 0.0 } else { 6.0 }, Color32::from_gray(if t.pro { 222 } else { 236 }));
            icons::paint(ui, rect, "type", 20.0, Color32::from_gray(40));
        }
        // Gradient fills show the gradient itself (Photoshop); solid ones their colour.
        LayerContent::Fill(f) if crate::gradient_ui::paint_thumbnail(ui, l.id, f, rect) => {}
        LayerContent::Fill(f) => {
            let c = match f {
                photocraft_doc::Fill::Solid(c) => c.to_rgba8(),
                _ => [128, 128, 128, 255],
            };
            p.rect_filled(rect, 6.0, Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]));
        }
        _ => {
            // Keep row layout and outside thumbnail decorations when the image is clipped.
            if ui.is_rect_visible(rect) {
                widgets::checker(p, rect, 5.0);
                let tex = app.layer_thumb(ctx, doc, l);
                p.image(tex, rect, Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
            }
        }
    }
    let stroke = if t.pro {
        Stroke::new(1.0, Color32::from_gray(20))
    } else if selected {
        Stroke::new(1.5, t.text)
    } else {
        Stroke::new(1.0, t.field_border)
    };
    p.rect_stroke(rect, CornerRadius::same(if t.pro { 0 } else { 5 }), stroke, StrokeKind::Outside);
    crate::smart_ui::thumb_badge(ui, l, rect);
}

fn channels(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::channels_panel::show(app, ui);
}

fn history(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No document"));
        return;
    };
    let doc = st.doc.clone();
    let entries = st.history.entries();
    let current = entries.len() - 1;
    let redo: Vec<String> = st.history.redo_labels().map(str::to_string).collect();
    // Snapshot row (Photoshop shows the document's opening state with a thumbnail).
    {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
        let thumb = Rect::from_min_size(pos2(rect.left() + 28.0, rect.center().y - 14.0), vec2(28.0, 28.0));
        let ctx = ui.ctx().clone();
        if let Some(layer) = doc.layers.first() {
            widgets::checker(ui.painter(), thumb, 4.0);
            let tex = app.layer_thumb(&ctx, &doc, layer);
            ui.painter().image(tex, thumb, Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        }
        ui.painter().rect_stroke(thumb, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
        let text_left = thumb.right() + 8.0;
        let name = crate::tab_strip::elided(ui, &doc.name, egui::FontId::proportional(12.0), t.text, (rect.right() - text_left - 6.0).max(1.0));
        ui.painter().with_clip_rect(rect).galley(pos2(text_left, rect.center().y - name.size().y * 0.5), name, t.text);
        resp.on_hover_text(&doc.name);
        ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.separator));
    }
    let mut target: Option<isize> = None;
    let all = entries.iter().map(|e| (e.clone(), false)).chain(redo.iter().map(|e| (e.clone(), true)));
    // The dock gives History a fixed height: the rows scroll above the footer.
    let footer = if t.pro { 34.0 } else { 0.0 };
    let max_h = (ui.available_height() - footer).max(40.0);
    egui::ScrollArea::vertical().id_salt("history-rows").max_height(max_h).auto_shrink([false, false]).show_palette(ui, |ui| {
        for (i, (e, is_redo)) in all.enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
            if i == current {
                ui.painter().rect_filled(rect, if t.pro { 0.0 } else { t.radius_sm }, if t.pro { t.row_selected } else { t.accent_soft });
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
            }
            let icon = match e.as_str() {
                s if s.contains("Brush") || s.contains("Eraser") => "brush",
                s if s.contains("Layer") => "layers",
                s if s.contains("Select") || s.contains("Marquee") || s.contains("Deselect") || s.contains("Lasso") => "square-dashed",
                s if s.contains("Fill") => "paint-bucket",
                "Open" => "image",
                _ => "sliders-horizontal",
            };
            let color = if is_redo { t.text_faint } else { t.text };
            icons::paint(
                ui,
                Rect::from_center_size(pos2(rect.left() + 42.0, rect.center().y), vec2(16.0, 16.0)),
                icon,
                12.0,
                if is_redo { t.text_faint } else { t.icon },
            );
            ui.painter().text(pos2(rect.left() + 58.0, rect.center().y), Align2::LEFT_CENTER, &e, egui::FontId::proportional(12.0), color);
            if resp.clicked() {
                target = Some(i as isize - current as isize);
            }
        }
    });
    if t.pro {
        widgets::hairline(ui);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let _ = icons::button(ui, "trash", 24.0, false, tl!("Delete current state"));
                let _ = icons::button(ui, "scan", 24.0, false, tl!("Create new snapshot"));
                let _ = icons::button(ui, "file-plus", 24.0, false, tl!("Create new document from current state"));
            });
        });
    }
    // An open Free Transform owns Undo (transform_tool::intercept): stepping the document's history under
    // its box would leave it transforming pixels that changed.
    if let Some(delta) = target.filter(|_| app.ui.transform.is_none()) {
        let (cmd, n) = if delta < 0 { ("edit.undo", -delta) } else { ("edit.redo", delta) };
        for _ in 0..n {
            if app.run(cmd, json!({})).is_err() {
                break;
            }
        }
    }
}

// ----------------------------------------------------------------------------- properties

/// An explicit request must restore the live container, not just its legacy visibility flag.
pub(crate) fn reveal_properties(app: &mut PhotocraftApp, ctx: &egui::Context) {
    crate::panel_workspace::migrate(app, ctx);
    app.ui.dock_tabs.properties = 0;
    crate::dock::reveal(app, crate::dock::Group::Properties);
    crate::panel_workspace::native::request_group_focus(app, ctx, crate::dock::Group::Properties);
    ctx.request_repaint_of(egui::ViewportId::ROOT);
}

/// Floating Properties card anchored to the canvas' top-right corner.
pub fn properties_window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // Pro (Photoshop) docks Properties; Studio floats it over the canvas.
    if !app.ui.dock.containers.is_empty() || !app.ui.panels.properties || Tokens::get(ctx).pro || app.ui.dock.floating.contains(&crate::dock::Group::Properties)
    {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let Some(id) = st.active_layer else { return };
    let Some(layer) = st.doc.layer(id) else { return };
    // The floating card appears for adjustment and fill layers (their controls live here).
    if !matches!(layer.content, LayerContent::Adjustment(_) | LayerContent::Fill(_)) {
        return;
    }
    let layer = layer.clone();
    let t = Tokens::get(ctx);
    let canvas = app.last_canvas_rect;
    let width = 320.0;
    let pos = pos2(canvas.right() - width - 12.0, canvas.top() + 44.0);
    let frame = egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius_lg as u8))
        .shadow(egui::Shadow { offset: [0, 12], blur: 36, spread: 0, color: t.shadow })
        .inner_margin(egui::Margin::same(14));
    // A floating panel (Order::Middle like other panels, so menus, popups and dialogs stay above
    // it), anchored to the canvas corner and dragged by its title off the part being worked on.
    let card_id = egui::Id::new("properties-card");
    let offset: egui::Vec2 = ctx.data(|m| m.get_temp(card_id)).unwrap_or_default();
    let mut drag = egui::Vec2::ZERO;
    let shown = egui::Area::new(card_id).order(egui::Order::Middle).fixed_pos(pos + offset).show(ctx, |ui| {
        frame.show(ui, |ui| {
            ui.set_width(width - 28.0);
            let title = ui.horizontal(|ui| {
                ui.label(RichText::new(tl!("Properties")).font(theme::semibold(13.5)).color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::button(ui, "minus", 22.0, false, tl!("Hide Properties")).clicked() {
                        app.ui.panels.properties = false;
                    }
                });
            });
            let bar = title.response.rect.with_max_x(title.response.rect.right() - 28.0);
            let grip = ui.interact(bar, card_id.with("title"), Sense::drag());
            if grip.hovered() || grip.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            drag = grip.drag_delta();
            ui.add_space(6.0);
            let icon = match &layer.content {
                LayerContent::Adjustment(_) => "sliders-horizontal",
                LayerContent::Group(_) => "folder",
                LayerContent::Fill(_) => "paint-bucket",
                _ => "image",
            };
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                ui.painter().rect_filled(r, t.radius_sm, t.field);
                icons::paint(ui, r, icon, 17.0, t.icon);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&layer.name).font(theme::medium(13.0)).color(t.text));
                    let kind = match &layer.content {
                        LayerContent::Adjustment(a) => crate::i18n::fmt(tl!("{name} Properties"), &[("name", tl!(a.label()))]),
                        // Whole phrases ("Type Layer"), as the Properties header translates them.
                        other => tl!(&format!("{} Layer", other.kind_name())).to_string(),
                    };
                    ui.label(RichText::new(kind).small().color(t.text_faint));
                });
            });
            ui.add_space(8.0);
            widgets::hairline(ui);
            ui.add_space(8.0);
            // Per-layer ids, so text still being typed for one layer can't commit to the next.
            ui.push_id(id, |ui| {
                if let LayerContent::Adjustment(adj) = &layer.content {
                    adjustment_controls(app, ui, id, adj);
                } else {
                    layer_controls(app, ui, &layer);
                }
            });
        });
    });
    if drag != egui::Vec2::ZERO && canvas.is_positive() {
        // Keep the title bar inside the canvas.
        let r = shown.response.rect.translate(drag);
        let dx = (canvas.left() - r.left()).max(0.0) - (r.right() - canvas.right()).max(0.0);
        let dy = (canvas.top() - r.top()).max(0.0) - (r.top() + 40.0 - canvas.bottom()).max(0.0);
        ctx.data_mut(|m| m.insert_temp(card_id, offset + drag + egui::vec2(dx, dy)));
    }
}

fn adjustment_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: LayerId, adj: &photocraft_doc::Adjustment) {
    crate::adjust_editors::layer_editor(app, ui, id, adj);
    ui.add_space(6.0);
    let layer = app.session.active().and_then(|s| s.doc.layer(id).cloned());
    if let Some(l) = layer {
        ui.horizontal(|ui| {
            let mut vis = l.visible;
            if widgets::toggle(ui, &mut vis, tl!("Adjustment visible")).changed() {
                let _ = app.run("layer.setProps", json!({"layer": id.0, "visible": vis}));
            }
            let mut clip = l.clipped;
            if widgets::toggle(ui, &mut clip, tl!("Clip to layer below")).changed() {
                let _ = app.run("layer.setProps", json!({"layer": id.0, "clipped": clip}));
            }
        });
    }
    ui.add_space(6.0);
    if widgets::secondary_button(ui, tl!("Reset to defaults"), ui.available_width()).clicked() {
        let _ = app.run("layer.setAdjustment", json!({"layer": id.0}));
        app.live_adjust = None;
    }
}

fn layer_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let t = Tokens::get(ui.ctx());
    if let Some(s) = layer.surface() {
        let b = app.cached_bounds(layer.id.0, s);
        // Use both halves of the panel instead of stacking four short values at the left.
        egui::Grid::new("props-grid").num_columns(4).spacing([8.0, 3.0]).show(ui, |ui| {
            for row in [[("X", i64::from(b.x0)), ("W", i64::from(b.width()))], [("Y", i64::from(b.y0)), ("H", i64::from(b.height()))]] {
                for (label, value) in row {
                    ui.label(RichText::new(label).color(t.text_dim));
                    ui.label(RichText::new(format!("{value} px")).font(theme::mono(12.0)).color(t.text));
                }
                ui.end_row();
            }
        });
        ui.add_space(6.0);
    }
    layer_opacity_control(app, ui, layer.id, "opacity", tl!("Opacity"), layer.opacity);
    layer_opacity_control(app, ui, layer.id, "fill", tl!("Fill"), layer.fill_opacity);
}

/// Update the document on every slider/number-field change so the next frame and canvas retain
/// the value. A pointer gesture shares one history key; a later drag gets a fresh key.
fn layer_opacity_control(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: LayerId, field: &str, label: &str, value: f32) -> egui::Response {
    let document = app.session.active().map(|s| s.doc.id);
    let scope = (document, layer.0, field);
    let key = ui.id().with(("layer-property-gesture", scope));
    if ui.input(|i| !i.pointer.any_down() && !i.pointer.any_released()) {
        ui.data_mut(|d| d.remove::<u64>(key));
    }
    if ui.input(|i| i.pointer.any_pressed()) {
        let frame = ui.ctx().cumulative_frame_nr();
        ui.data_mut(|d| d.insert_temp(key, frame));
    }
    let mut percent = value * 100.0;
    let response = ui.push_id(scope, |ui| widgets::slider_row(ui, label, &mut percent, 0.0..=100.0, "%", None)).inner;
    if response.changed() {
        let mut params = json!({"layer": layer.0, field: percent / 100.0});
        if let Some(frame) = ui.data(|d| d.get_temp::<u64>(key)) {
            params["coalesce"] = json!(format!("{key:?}:{frame}:{field}"));
        }
        if let Err(error) = app.run("layer.setProps", params) {
            app.ui.status = error;
        }
    }
    if !ui.input(|i| i.pointer.any_down()) {
        ui.data_mut(|d| d.remove::<u64>(key));
    }
    response
}

/// Docked Properties body (Pro theme).
fn properties_body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No properties"));
        return;
    };
    // Photoshop shows the Document properties when nothing or the Background layer is selected.
    if crate::doc_props_ui::shows_document(&st.doc, st.active_layer) {
        crate::doc_props_ui::properties(app, ui);
        return;
    }
    let Some(id) = st.active_layer else { return };
    // Borrowed from the document snapshot: cloning the layer every frame copied whole groups.
    let doc = st.doc.clone();
    let Some(layer) = doc.layer(id) else { return };
    if let LayerContent::Adjustment(adj) = &layer.content {
        // These editors already group their controls. Do not stack generic section spacing on
        // top of the tab strip, the identity row and the editor's own channel/slider rows.
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            crate::props_layout::compact_header(ui, layer);
            widgets::hairline(ui);
            adjustment_controls(app, ui, id, adj);
        });
        return;
    }
    // Header: kind icon, layer name and kind (#155); sections below draw their own separators.
    crate::props_layout::header(ui, layer);
    if layer.artboard().is_some() || !t.pro {
        ui.add_space(4.0);
        widgets::hairline(ui);
        ui.add_space(6.0);
    }
    if layer.artboard().is_some() {
        crate::artboard_ui::properties(app, ui, layer);
    } else if t.pro {
        // Transform, Align, the kind's sections and Quick Actions.
        crate::layer_props_ui::properties(app, ui, layer);
    } else {
        if matches!(layer.content, LayerContent::Fill(photocraft_doc::Fill::Gradient { .. })) {
            crate::gradient_ui::properties(app, ui, layer);
            ui.add_space(6.0);
        }
        layer_controls(app, ui, layer);
        if matches!(layer.content, LayerContent::Text(_)) {
            crate::type_tool::type_properties(app, ui);
        }
        if matches!(layer.content, LayerContent::Shape(_)) {
            crate::vector_ui::shape_properties(app, ui, id);
        }
    }
}

/// Photoshop's Adjustments panel: a grid of one-click adjustment layers.
fn adjustments_grid(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(tl!("Add an adjustment")).color(t.text_dim));
    ui.add_space(4.0);
    let items: [(&str, &str, &str); 16] = [
        ("brightnessContrast", "sun", tl!("Brightness/Contrast")),
        ("levels", "gauge", tl!("Levels")),
        ("curves", "pen-tool", tl!("Curves")),
        ("exposure", "scan", tl!("Exposure")),
        ("vibrance", "sparkles", tl!("Vibrance")),
        ("hueSaturation", "droplet", tl!("Hue/Saturation")),
        ("colorBalance", "blend", tl!("Color Balance")),
        ("blackWhite", "contrast", tl!("Black & White")),
        ("photoFilter", "circle-dot", tl!("Photo Filter")),
        ("channelMixer", "sliders-horizontal", tl!("Channel Mixer")),
        ("invert", "squares-subtract", tl!("Invert")),
        ("posterize", "layers", tl!("Posterize")),
        ("threshold", "square", tl!("Threshold")),
        ("gradientMap", "palette", tl!("Gradient Map")),
        ("selectiveColor", "swatch-book", tl!("Selective Color")),
        ("colorLookup", "grid-3x3", tl!("Color Lookup")),
    ];
    let mut run = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
        for (kind, icon, tip) in items {
            let response = icons::button(ui, icon, 30.0, false, tip);
            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), tip));
            if response.clicked() {
                run = Some(format!("layer.newAdjustmentLayer.{kind}"));
            }
        }
    });
    if let Some(id) = run {
        let _ = crate::menus::invoke(app, ui.ctx(), &id, json!({}));
    }
}

/// The Color panel's foreground and background chips. A click picks the colour the field edits,
/// framed; a double-click opens the Color Picker on it.
pub(crate) fn field_chips(app: &mut PhotocraftApp, ui: &mut egui::Ui, chips: Rect) {
    let t = Tokens::get(ui.ctx());
    let bgr = Rect::from_min_size(chips.min + vec2(13.0, 13.0), vec2(22.0, 22.0));
    let fgr = Rect::from_min_size(chips.min + vec2(3.0, 3.0), vec2(22.0, 22.0));
    let frame = Stroke::new(1.0, t.text_dim);
    let bg_active = app.ui.color_panel.background;
    let p = ui.painter();
    p.rect_filled(bgr, 2.0, c32(app.session.tools.background));
    p.rect_stroke(bgr, 2.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    if bg_active {
        p.rect_stroke(bgr.expand(2.0), 2.0, frame, StrokeKind::Outside);
    }
    p.rect_filled(fgr, 2.0, c32(app.session.tools.foreground));
    p.rect_stroke(fgr, 2.0, Stroke::new(1.0, Color32::from_gray(210)), StrokeKind::Outside);
    if !bg_active {
        p.rect_stroke(fgr.expand(2.0), 2.0, frame, StrokeKind::Outside);
    }
    // The foreground is on top, so it takes the clicks where the two overlap.
    let bg_resp = ui.interact(bgr, ui.id().with("field-bg"), Sense::click());
    let fg_resp = ui.interact(fgr, ui.id().with("field-fg"), Sense::click());
    let picked = if fg_resp.clicked() { false } else { bg_resp.clicked() || bg_active };
    app.ui.color_panel.background = picked;
    if fg_resp.double_clicked() || bg_resp.double_clicked() {
        crate::color_picker_ui::open(app, if picked { "background" } else { "foreground" });
    }
}

/// Photoshop Color panel: saturation/brightness field + hue strip, drawn as shaded meshes.
fn color_field(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    color_picker(app, ui);
}

/// Tool colours are sRGB-encoded floats; egui's Hsva works on sRGB bytes via these helpers
/// (its `from_rgba_unmultiplied` expects *linear* RGB, which gave wrong readouts).
pub(crate) fn srgb_bytes(c: [f32; 4]) -> [u8; 3] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [q(c[0]), q(c[1]), q(c[2])]
}

#[cfg(test)]
fn srgb_hsva(c: [f32; 4]) -> egui::ecolor::Hsva {
    egui::ecolor::Hsva::from_srgb(srgb_bytes(c))
}

#[cfg(test)]
fn hsva_srgb(h: egui::ecolor::Hsva) -> [f32; 4] {
    let [r, g, b] = h.to_srgb();
    [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0]
}

/// Photoshop's brush preset picker chip: a soft/hard round tip preview with the size underneath.
/// Draw a round brush tip preview (hard core fading to a soft edge).
fn brush_tip(p: &egui::Painter, c: egui::Pos2, rad: f32, hardness: f32, color: Color32) {
    let inner = rad * hardness.clamp(0.05, 1.0);
    let steps = 8;
    for k in (0..=steps).rev() {
        let f = k as f32 / steps as f32;
        let rr = inner + (rad - inner) * f;
        let a = if hardness >= 0.99 { 1.0 } else { (1.0 - f).powf(1.5) };
        p.circle_filled(c, rr, color.gamma_multiply(a));
    }
}

/// Options-bar Smoothing % (the brush's stroke smoothing; the live stroke and the commit use it).
fn smoothing_field(ui: &mut egui::Ui, b: &mut photocraft_engine::BrushSettings, width: f32) {
    let mut sm = (b.smoothing.amount * 100.0).round();
    if widgets::value_field(ui, &mut sm, 0.0..=100.0, "%", width).changed() {
        b.smoothing.amount = (sm / 100.0).clamp(0.0, 1.0);
    }
}

/// Options-bar brush chip; opens Photoshop's Brush Preset picker (size, hardness, the preset
/// library). Returns what the picker asked for beyond the size and hardness edits in `b`.
fn brush_preset_chip(app: &mut PhotocraftApp, ui: &mut egui::Ui, b: &mut photocraft_engine::BrushSettings) -> Option<crate::brush_picker::Pick> {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(44.0, 30.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, t.radius_sm, t.hover);
    }
    let c = pos2(r.left() + 14.0, r.top() + 11.0);
    brush_tip(ui.painter(), c, 7.0, b.hardness, Color32::WHITE);
    ui.painter().text(pos2(c.x, r.bottom() - 5.0), Align2::CENTER_CENTER, format!("{}", b.size.round() as i64), egui::FontId::proportional(9.5), t.text_dim);
    icons::paint(ui, Rect::from_center_size(pos2(r.right() - 9.0, c.y), vec2(10.0, 10.0)), "chevron-down", 9.0, t.text_faint);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, tl!("Brush Preset picker")));
    let resp = resp.on_hover_text(tl!("Brush Preset picker"));
    if !ui.ctx().embed_viewports() {
        if resp.clicked() {
            ui.data_mut(|data| data.insert_temp(egui::Id::new("native-options-brush-picker-request"), resp.rect.left_bottom()));
        }
        return None;
    }
    egui::Popup::from_toggle_button_response(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| crate::brush_picker::body(app, ui, b))
        .and_then(|r| r.inner)
}

/// Drag a layer row to reorder: drop on the upper/lower half to place above/below, or on the middle
/// of a group to move into it. Held Alt at drop copies instead. One `layer.moveTo`
/// command prepares the whole destination in a single undo step.
fn layer_drag_cancelled(ctx: &egui::Context) -> bool {
    ctx.input(|input| {
        input.key_pressed(egui::Key::Escape)
            || input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false)))
            || !input.focused
            || (!input.pointer.primary_down() && !input.pointer.any_released())
    })
}

fn layer_drag_and_drop(
    app: &PhotocraftApp,
    ctx: &egui::Context,
    ui: &egui::Ui,
    l: &Layer,
    rect: Rect,
    resp: &egui::Response,
    actions: &mut Vec<(String, Value)>,
) {
    let t = Tokens::get(ctx);
    let key = egui::Id::new("layer-drag");
    let selection_key = key.with("selection");
    if layer_drag_cancelled(ctx) {
        ctx.data_mut(|data| {
            data.remove::<u64>(key);
            data.remove::<Vec<u64>>(selection_key);
        });
        crate::layer_transfer::cancel_panel_drag(ctx);
        return;
    }
    if resp.drag_started() {
        let layers = app.session.active().filter(|st| st.is_layer_selected(l.id)).map(|st| st.selected_layers()).unwrap_or_else(|| vec![l.id]);
        ctx.data_mut(|d| {
            d.insert_temp(key, l.id.0);
            d.insert_temp(selection_key, layers.iter().map(|id| id.0).collect::<Vec<_>>());
        });
    }
    let Some(dragged) = ctx.data(|d| d.get_temp::<u64>(key)) else { return };
    let layers = ctx.data(|d| d.get_temp::<Vec<u64>>(selection_key)).unwrap_or_else(|| vec![dragged]);
    let pointer = ctx.input(|i| i.pointer.interact_pos());
    let released = ctx.input(|i| i.pointer.any_released());
    let copy = ctx.input(|i| i.modifiers.alt);
    if copy {
        ctx.set_cursor_icon(egui::CursorIcon::Copy);
    }
    if dragged == l.id.0 {
        // Ghost label following the pointer.
        if let Some(p) = pointer {
            let label =
                if layers.len() > 1 { crate::i18n::trn(crate::i18n::current(), layers.len() as u64, "{n} layer", "{n} layers") } else { l.name.clone() };
            crate::layer_transfer::ghost(ctx, p, &if copy { format!("+ {label}") } else { label });
        }
        if !copy {
            return;
        }
    }
    if !copy && layers.contains(&l.id.0) {
        return;
    }
    let Some(p) = pointer else { return };
    if !rect.contains(p) {
        return;
    }
    let f = (p.y - rect.top()) / rect.height();
    let position = if l.is_group() && (0.3..0.7).contains(&f) {
        "into"
    } else if f < 0.5 {
        "above"
    } else {
        "below"
    };
    // A copy has fresh ids, but disallow dropping an ancestor into its source
    // subtree too: it is ambiguous and can otherwise repeatedly grow depth.
    if let Some(state) = app.session.active()
        && let Some(target) = state.doc.path_of(l.id)
        && layers
            .iter()
            .filter_map(|id| state.doc.path_of(photocraft_doc::LayerId(*id)))
            .any(|source| target.starts_with(&source) && (target != source || position == "into"))
    {
        ctx.set_cursor_icon(egui::CursorIcon::NotAllowed);
        return;
    }
    let painter = ui.painter();
    match position {
        "into" => {
            painter.rect_stroke(rect.shrink(1.0), t.radius_sm, Stroke::new(2.0, t.accent), StrokeKind::Inside);
        }
        "above" => {
            painter.line_segment([rect.left_top(), rect.right_top()], Stroke::new(2.0, t.accent));
        }
        _ => {
            painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(2.0, t.accent));
        }
    }
    if released {
        actions.push(("layer.moveTo".into(), json!({"layers": layers, "target": l.id.0, "position": position, "copy": copy})));
    }
}

#[cfg(test)]
#[path = "panels/layer_alt_drag_tests_v75.rs"]
mod layer_alt_drag_tests_v75;

#[cfg(test)]
#[path = "panels/layer_effect_labels_tests.rs"]
mod layer_effect_labels_tests;

/// Photoshop shows a layer's effects as indented sub-rows ("Effects", then each effect).
fn effect_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, l: &Layer, depth: usize) {
    let t = Tokens::get(ui.ctx());
    let indent = 30.0 + depth as f32 * 14.0 + 34.0;
    let rows = std::iter::once(("Effects", l.effects.enabled, None)).chain(l.effects.items.iter().map(|effect| {
        let kind = photocraft_engine::layer_style::kind_of(effect);
        (effect.label(), effect.enabled(), Some(kind))
    }));
    for (i, (label, on, kind)) in rows.enumerate() {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            continue;
        }
        if resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.35));
        }
        if t.pro {
            ui.painter().line_segment([pos2(rect.left() + 30.0, rect.top()), pos2(rect.left() + 30.0, rect.bottom())], Stroke::new(1.0, t.separator));
        }
        let eye = Rect::from_min_size(pos2(rect.left() + 6.0, rect.center().y - 9.0), vec2(18.0, 18.0));
        if on {
            icons::paint(ui, eye, "eye", 12.0, t.icon);
        }
        let x = rect.left() + indent + if i == 0 { 0.0 } else { 16.0 };
        if i == 0 {
            icons::paint(ui, Rect::from_center_size(pos2(x - 12.0, rect.center().y), vec2(14.0, 14.0)), "sparkles", 11.0, t.text_dim);
        }
        let right = rect.right() - crate::layer_row_ui::RIGHT_PAD;
        let name = tl!(label);
        crate::layer_row_ui::label(ui.painter(), x, rect.center().y, right, name, egui::FontId::proportional(11.5), if on { t.text_dim } else { t.text_faint });
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
        if resp.double_clicked() {
            crate::layer_style::open(app, kind);
        }
    }
}

/// New / add / subtract / intersect selection buttons (shared by selection tools).
fn selection_mode_buttons(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.x = 2.0;
    let held = crate::workspace_ui::sticky_mods(app, ui.input(|i| i.modifiers));
    let mode = crate::tool_feedback::effective_selection_mode(app, app.ui.tool, held);
    let effective = match mode {
        "add" => 1,
        "subtract" => 2,
        "intersect" => 3,
        _ => 0,
    };
    for (i, tip) in [
        tl!("New selection").to_string(),
        crate::i18n::fmt(tl!("Add to selection  ({key})"), &[("key", &crate::shortcuts::pretty("Shift"))]),
        crate::i18n::fmt(tl!("Subtract from selection  ({key})"), &[("key", &crate::shortcuts::pretty("Alt"))]),
        crate::i18n::fmt(tl!("Intersect with selection  ({key})"), &[("key", &crate::shortcuts::pretty("Shift+Alt"))]),
    ]
    .iter()
    .enumerate()
    {
        let (rect, response) = ui.allocate_exact_size(vec2(26.0, 24.0), Sense::click());
        let color = icons::button_chrome(ui, rect, effective == i, response.hovered());
        let stroke = Stroke::new(1.2, color);
        let top_left = rect.center() - vec2(8.0, 8.0);
        let a = Rect::from_min_size(top_left, vec2(11.0, 11.0));
        let b = a.translate(vec2(5.0, 5.0));
        if i == 0 {
            ui.painter().rect_filled(Rect::from_center_size(rect.center(), vec2(12.0, 12.0)), 0.0, color);
        } else {
            ui.painter().rect_stroke(a, 0.0, stroke, StrokeKind::Inside);
            ui.painter().rect_stroke(b, 0.0, stroke, StrokeKind::Inside);
            match i {
                1 => {
                    ui.painter().rect_filled(a, 0.0, color);
                    ui.painter().rect_filled(b, 0.0, color);
                }
                2 => {
                    ui.painter().rect_filled(Rect::from_min_max(a.min, egui::pos2(a.right(), b.top())), 0.0, color);
                    ui.painter().rect_filled(Rect::from_min_max(egui::pos2(a.left(), b.top()), egui::pos2(b.left(), a.bottom())), 0.0, color);
                }
                _ => {
                    ui.painter().rect_filled(a.intersect(b), 0.0, color);
                }
            }
        }
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, effective == i, tip));
        if response.on_hover_text(tip).clicked() {
            app.ui.selection_mode = i as u8;
        }
    }
    ui.spacing_mut().item_spacing.x = 8.0;
}

/// Options-bar swatch for the current preset or the Classic foreground-to-background ramp.
fn gradient_swatch(ui: &mut egui::Ui, stops: &[(f32, [f32; 4])], classic: bool) {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(96.0, 20.0), if classic { Sense::click() } else { Sense::hover() });
    // Egui snaps rectangles but uses the mesh coordinates as supplied. Give the mesh and
    // the transparency checker the same physical bounds as the themed border.
    let rect = rect.round_to_pixels(ui.pixels_per_point());
    let painter = ui.painter();
    if stops.iter().any(|(_, color)| color[3] < 1.0) {
        widgets::checker(painter, rect, 5.0);
    }
    crate::gradient_ui::paint_ramp(painter, rect, |u| photocraft_algo::paint::sample_stops(stops, u));
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    let _ = resp.on_hover_text(if classic { tl!("Click to edit the gradient") } else { tl!("The current gradient (pick one in Window › Gradients)") });
}

#[cfg(test)]
#[test]
fn gradient_options_swatch_has_a_pixel_aligned_frame_and_checker_only_for_transparency() {
    for scale in [1.0_f32, 1.25, 1.5, 2.0] {
        for (classic, alpha) in [(false, 1.0), (false, 0.5), (true, 1.0)] {
            let ctx = egui::Context::default();
            let mut input = egui::RawInput::default();
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
            let stops = [(0.0, [0.1, 0.2, 0.3, 1.0]), (1.0, [0.2, 0.6, 0.8, alpha])];
            let output = ctx.run_ui(input, |ui| {
                ui.add_space(7.35);
                gradient_swatch(ui, &stops, classic);
            });
            let frame = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(frame) if frame.stroke.color == Tokens::get(&ctx).field_border && frame.stroke.width == 1.0 => Some(frame),
                    _ => None,
                })
                .unwrap();
            let mesh = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh) => Some(mesh),
                    _ => None,
                })
                .unwrap();
            let checker = output.shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::Rect(fill) => fill.rect == frame.rect && fill.fill == Color32::from_gray(250),
                _ => false,
            });
            assert_eq!(frame.stroke_kind, StrokeKind::Inside);
            assert_eq!(frame.rect, mesh.calc_bounds());
            assert_eq!(frame.rect, frame.rect.round_to_pixels(scale));
            assert_eq!(checker, alpha < 1.0);
        }
    }
}

#[cfg(test)]
mod color_tests {
    use super::*;

    #[test]
    fn srgb_hsva_roundtrip() {
        for c in [[0.847, 0.271, 0.180, 1.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0], [0.2, 0.6, 0.4, 1.0]] {
            let back = hsva_srgb(srgb_hsva(c));
            for i in 0..3 {
                assert!((back[i] - c[i]).abs() <= 1.0 / 255.0, "{c:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn hue_and_saturation_fields_take_values_on_black() {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
        let mut h = egui_kittest::Harness::builder().with_size(vec2(300.0, 450.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    color_picker(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(2);
        // Hue, Saturation, Brightness, typed one key per frame. Black has no hue or saturation of its
        // own, so the first two used to reset to 0 and this ended on white.
        for (field, typed) in [(0, "120"), (1, "100"), (2, "100")] {
            h.query_all_by_role(egui::accesskit::Role::SpinButton).nth(field).unwrap().click();
            h.run_steps(1);
            for ch in typed.chars() {
                h.event(egui::Event::Text(ch.to_string()));
                h.run_steps(1);
            }
            h.key_press(egui::Key::Tab);
            h.run_steps(2);
        }
        assert_eq!(srgb_bytes(h.state().session.tools.foreground), [0, 255, 0]);
    }

    fn field_harness(app: PhotocraftApp) -> egui_kittest::Harness<'static, PhotocraftApp> {
        // 60 fps steps, so two clicks a frame apart are a double-click.
        let mut h = egui_kittest::Harness::builder().with_size(vec2(300.0, 330.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    color_field(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(2);
        h
    }

    fn click(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, p: egui::Pos2) {
        h.hover_at(p);
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
        }
    }

    /// A click on the background chip makes the field edit the background, not the foreground.
    #[test]
    fn background_chip_retargets_the_color_field() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.tools.foreground = [0.2, 0.4, 0.6, 1.0];
        app.session.tools.background = [0.0, 0.0, 0.0, 1.0];
        let mut h = field_harness(app);
        let min = h.ctx.input(|i| i.viewport_rect()).min;
        // The background chip's corner that the foreground chip doesn't cover.
        click(&mut h, min + vec2(40.0, 40.0));
        // The white vertex of the new saturation/value triangle.
        let wheel = h.ctx.data(|d| d.get_temp::<Rect>(egui::Id::new("color-wheel-rect"))).unwrap();
        let inner = wheel.width() * 0.46 * 0.78;
        click(&mut h, wheel.center() + vec2(-inner * 0.45, -inner * 0.7794));
        assert!(h.state().ui.color_panel.background);
        let tools = &h.state().session.tools;
        assert_eq!(tools.foreground, [0.2, 0.4, 0.6, 1.0]);
        assert!(tools.background[..3].iter().all(|&v| v > 0.9), "{:?}", tools.background);
    }

    #[test]
    fn double_clicking_a_chip_opens_its_color_picker() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = field_harness(app);
        let min = h.ctx.input(|i| i.viewport_rect()).min;
        // The foreground point is where the chips overlap: the foreground is on top there.
        for (p, target) in [(vec2(40.0, 40.0), "background"), (vec2(30.0, 30.0), "foreground")] {
            h.state_mut().ui.dialogs.clear();
            // Past the last double-click, so this one isn't counted as a triple-click.
            h.run_steps(40);
            click(&mut h, min + p);
            assert!(h.state().ui.dialogs.is_empty(), "a single click only picks the chip");
            click(&mut h, min + p);
            let [d] = h.state().ui.dialogs.as_slice() else { panic!("one Color Picker") };
            assert_eq!(d.fields.get("__colorPicker").and_then(Value::as_str), Some(target));
        }
    }

    /// Black has no hue of its own, so each chip remembers the hue last picked for it.
    #[test]
    fn each_chip_keeps_its_own_hue_on_black() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.tools.foreground = [0.0, 0.0, 0.0, 1.0];
        app.session.tools.background = [0.0, 0.0, 0.0, 1.0];
        let mut h = field_harness(app);
        let min = h.ctx.input(|i| i.viewport_rect()).min;
        let wheel = h.ctx.data(|d| d.get_temp::<Rect>(egui::Id::new("color-wheel-rect"))).unwrap();
        let ring = wheel.width() * 0.46 * 0.89;
        click(&mut h, wheel.center() + Vec2::angled(std::f32::consts::TAU / 3.0) * ring);
        click(&mut h, min + vec2(40.0, 40.0));
        click(&mut h, wheel.center() + Vec2::angled(std::f32::consts::TAU * 2.0 / 3.0) * ring);
        click(&mut h, min + vec2(16.0, 16.0));
        let inner = wheel.width() * 0.46 * 0.78;
        click(&mut h, wheel.center() + vec2(inner * 0.90, 0.0));
        let [r, g, b] = srgb_bytes(h.state().session.tools.foreground);
        assert!(g > 200 && r < 64 && b < 64, "the foreground's green, not the background's blue: {:?}", [r, g, b]);
    }
}

#[cfg(test)]
mod history_transform_tests {
    use super::*;
    use egui_kittest::Harness;

    fn click(h: &mut Harness<'static, PhotocraftApp>, p: egui::Pos2) {
        h.hover_at(p);
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
        }
    }

    /// While Free Transform is open, clicking a History state leaves the document alone: the
    /// transform owns Undo. Without a transform the same click steps back as usual.
    #[test]
    fn history_rows_wait_for_an_open_transform() {
        for transforming in [true, false] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("select.rect", json!({"x": 20, "y": 20, "width": 60, "height": 40})).unwrap();
            app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
            app.run("select.deselect", json!({})).unwrap();
            let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(|ui, app: &mut PhotocraftApp| history(app, ui), app);
            h.run_steps(2);
            if transforming {
                let ctx = h.ctx.clone();
                crate::transform_tool::begin(h.state_mut(), &ctx).unwrap();
            }
            let steps = h.state().session.active().unwrap().history.entries().len();
            // The first state's row, below the 38 pt snapshot row.
            let row = h.ctx.input(|i| i.viewport_rect()).min + vec2(60.0, 60.0);
            click(&mut h, row);
            let after = h.state().session.active().unwrap().history.entries().len();
            if transforming {
                assert_eq!(after, steps, "the document's history is untouched");
                assert!(h.state().ui.transform.is_some());
            } else {
                assert!(after < steps, "the click steps back: {steps} -> {after}");
            }
        }
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;
    use crate::canvas::{ToolEvent, tool_event};
    use egui::Modifiers;

    fn click_lock(app: &mut PhotocraftApp) {
        let st = app.session.active().unwrap();
        let l = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
        let (cmd, p) = simple_lock_toggle(crate::doc_props_ui::is_background(&st.doc, l), l);
        app.run(&cmd, p).unwrap();
    }

    fn erase_line(app: &mut PhotocraftApp) {
        app.ui.tool = Tool::Eraser;
        let m = Modifiers::NONE;
        tool_event(app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Move { x: 100.0, y: 40.0, pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Up { x: 100.0, y: 40.0 }, m);
    }

    #[test]
    fn simple_lock_button_never_locks_the_eraser_out_of_the_background() {
        // #76: the simple themes' lock button used to toggle "lock all" on the Background, which
        // kept showing as locked and then refused every paint tool.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
        app.run("tools.setColors", json!({"background": "#ff0000"})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0}})).unwrap();
        // Clicking the Background's lock converts it to a normal layer (Photoshop).
        click_lock(&mut app);
        let l = &app.session.active().unwrap().doc.layers[0];
        assert_eq!(l.name, "Layer 0");
        assert!(!(l.locks.transparency || l.locks.pixels || l.locks.position || l.locks.all));
        erase_line(&mut app);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap().rgba(50, 40)[3], 0.0, "erased to transparency");
        // On a normal layer the button locks all, then unlocks everything.
        click_lock(&mut app);
        assert!(app.session.active().unwrap().doc.layers[0].locks.all);
        click_lock(&mut app);
        let k = app.session.active().unwrap().doc.layers[0].locks;
        assert!(!(k.transparency || k.pixels || k.position || k.artboard || k.all));
        // A layer with only transparency locked unlocks in one click.
        app.run("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
        click_lock(&mut app);
        assert!(!app.session.active().unwrap().doc.layers[0].locks.transparency);
    }
}

#[cfg(test)]
mod swatch_type_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    /// Clicking a swatch while characters are selected recolours them, not just the foreground.
    #[test]
    fn clicking_a_swatch_recolours_selected_type() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        let id = app.run("type.create", json!({"text": "Hello world", "size": 40, "x": 20, "y": 100, "color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
        app.ui.tool = Tool::Type;
        app.ui.text_edit =
            Some(crate::state::TextEdit { layer: id, caret: 0, anchor: 5, session: "s".into(), created: false, dragging: false, resize: None, preedit: None });
        let mut h = Harness::builder().with_size(vec2(300.0, 200.0)).build_ui_state(|ui, app: &mut PhotocraftApp| swatches(app, ui), app);
        h.run_steps(2);
        h.get_by_label("Black  #000000").click();
        h.run_steps(2);
        let s = [0, 0, 0];
        assert_eq!(h.state().session.tools.foreground, [s[0] as f32 / 255.0, s[1] as f32 / 255.0, s[2] as f32 / 255.0, 1.0]);
        let st = h.state().session.active().unwrap();
        let Some(photocraft_doc::LayerContent::Text(t)) = st.doc.layer(photocraft_doc::LayerId(id)).map(|l| &l.content) else { panic!("type layer") };
        let runs = t.char_runs();
        assert_eq!(runs[0].len, 5, "the selection is its own run");
        assert_eq!(runs[0].style.color.to_rgba8(), [s[0], s[1], s[2], 255]);
        assert_eq!(runs[1].style.color.to_rgba8(), [255, 255, 255, 255]);
    }

    /// The HSB sliders only act on an edit. Their h/s/v round trip isn't exact for every colour
    /// (#D8452E isn't), and comparing values used to rewrite the foreground every frame, which
    /// would recolour selected type the moment it was selected.
    #[test]
    fn idle_hsb_sliders_leave_the_foreground_and_selected_type_alone() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        let id = app.run("type.create", json!({"text": "Hello world", "size": 40, "x": 20, "y": 100, "color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
        app.run("tools.setColors", json!({"foreground": "#d8452e"})).unwrap();
        let fg = app.session.tools.foreground;
        app.ui.tool = Tool::Type;
        app.ui.text_edit =
            Some(crate::state::TextEdit { layer: id, caret: 0, anchor: 5, session: "s".into(), created: false, dragging: false, resize: None, preedit: None });
        let mut h = Harness::builder().with_size(vec2(300.0, 300.0)).build_ui_state(|ui, app: &mut PhotocraftApp| color_picker(app, ui), app);
        h.run_steps(4);
        assert_eq!(h.state().session.tools.foreground, fg);
        let st = h.state().session.active().unwrap();
        let Some(photocraft_doc::LayerContent::Text(t)) = st.doc.layer(photocraft_doc::LayerId(id)).map(|l| &l.content) else { panic!("type layer") };
        assert_eq!(t.char_runs().len(), 1, "still one white run");
    }
}

#[cfg(test)]
mod type_flyout_tests {
    use super::*;

    fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, time: f64, events: Vec<egui::Event>) {
        let mut out = ctx.run_ui(
            egui::RawInput { time: Some(time), events, screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 1800.0))), ..Default::default() },
            |ui| toolbar(app, ui),
        );
        out.textures_delta.clear();
    }

    #[test]
    fn long_press_type_button_selects_vertical_without_selecting_on_release() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let initial = app.ui.tool;
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        frame(&mut app, &ctx, 0.0, vec![]);
        frame(&mut app, &ctx, 0.1, vec![]);
        let index = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.contains(&Tool::Type)).unwrap();
        let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
        let mut buttons: Vec<Rect> = ctx.viewport(|v| {
            v.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.rect.size() == Vec2::splat(bx) && w.sense.senses_click())
                .map(|w| w.rect)
                .collect()
        });
        buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
        let at = buttons[index].center();
        let pointer = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, 1.0, vec![egui::Event::PointerMoved(at), pointer(at, true)]);
        frame(&mut app, &ctx, 1.36, vec![]);
        assert_eq!(ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::new("tool-flyout"))).map(|(id, _)| id), Some(egui::Id::new(("tool-slot", index))));
        frame(&mut app, &ctx, 1.4, vec![pointer(at, false)]);
        frame(&mut app, &ctx, 1.45, vec![]);
        assert_eq!(app.ui.tool, initial);
        let key = egui::Id::new(("tool-slot", index));
        let menu = ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
        let row = egui::pos2(menu.left() + 65.0, menu.top() + 39.0 + 8.0);
        frame(&mut app, &ctx, 2.0, vec![egui::Event::PointerMoved(row), pointer(row, true)]);
        frame(&mut app, &ctx, 2.05, vec![pointer(row, false)]);
        assert_eq!(app.ui.tool, Tool::VerticalType);
        assert!(ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::new("tool-flyout"))).is_none());
    }
}

#[cfg(test)]
mod properties_card_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    /// Arithmetic left uncommitted in one fill layer's card never lands on the layer selected next.
    #[test]
    fn uncommitted_arithmetic_stays_with_its_layer() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        let mut ids = Vec::new();
        for color in ["#00ff00", "#0000ff"] {
            s.execute("layer.newFillLayer.solidColor", json!({"color": color})).unwrap();
            ids.push(s.active().unwrap().active_layer.unwrap());
        }
        // The layer selected next already shows 25, the value the edited one has when `25*` stops it.
        s.execute("layer.setProps", json!({"layer": ids[0].0, "opacity": 0.25})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.ui.panels.properties = true;
        let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                    properties_window(app, ui.ctx());
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(2);
        h.query_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap().click();
        h.run_steps(1);
        for ch in "25*2".chars() {
            h.event(egui::Event::Text(ch.to_string()));
            h.run_steps(1);
        }
        // The Layers panel selects the other layer in the frame the click leaves the field.
        h.state_mut().session.execute("layer.select", json!({"layer": ids[0].0})).unwrap();
        h.ctx.memory_mut(|m| m.stop_text_input());
        h.run_steps(3);
        let doc = &h.state().session.active().unwrap().doc;
        assert_eq!(ids.iter().map(|&id| doc.layer(id).unwrap().opacity).collect::<Vec<_>>(), [0.25, 0.25]);
    }
}

#[cfg(test)]
#[path = "tool_flyout_tests_v60.rs"]
mod tool_flyout_tests_v60;

#[cfg(test)]
mod native_tool_flyout_tests_v59 {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    fn buttons(ctx: &egui::Context, side: f32) -> Vec<Rect> {
        let mut rects = ctx.viewport(|v| {
            v.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, widgets)| widgets.iter())
                .filter(|w| w.rect.size() == Vec2::splat(side) && w.sense.senses_click())
                .map(|w| w.rect)
                .collect::<Vec<_>>()
        });
        rects.sort_by(|a, b| a.top().total_cmp(&b.top()).then_with(|| a.left().total_cmp(&b.left())));
        rects
    }

    #[test]
    fn long_press_native_flyout_cannot_free_live_root_icons_or_pollute_later_chrome() {
        // Renderer is thread-local without an unset API; keep it off the test worker.
        std::thread::Builder::new().name("v59-native-flyout".into()).stack_size(16 * 1024 * 1024).spawn(|| {
            for theme in crate::theme::ThemeKind::ALL {
              for (root_scale, child_scale) in [(1.0,1.0),(1.25,2.0),(2.0,1.25)] {
                let ctx = egui::Context::default();
                PhotocraftApp::setup_context(&ctx, theme);
                ctx.set_embed_viewports(false);
                let side = if Tokens::get(&ctx).pro {30.0} else {36.0};
                let slot_count: usize = TOOL_SECTIONS.iter().map(|s| s.len()).sum();
                let calls = Rc::new(Cell::new(0usize));
                let child_calls = calls.clone();
                let freed = Rc::new(RefCell::new(Vec::<egui::TextureId>::new()));
                let child_freed = freed.clone();
                egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
                    assert_eq!(ctx.viewport_id(), egui::ViewportId::ROOT);
                    let pass = ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT);
                    child_calls.set(child_calls.get()+1);
                    let mut input = egui::RawInput { viewport_id:viewport.ids.this, screen_rect:Some(Rect::from_min_size(egui::Pos2::ZERO,vec2(300.0,160.0))), ..Default::default() };
                    let info = input.viewports.entry(viewport.ids.this).or_default();
                    info.parent = Some(viewport.ids.parent);
                    info.focused = Some(true);
                    info.native_pixels_per_point = Some(child_scale);
                    info.inner_rect = Some(Rect::from_min_size(pos2(50.0,200.0),vec2(300.0,160.0)));
                    let mut output = ctx.run_ui(input, |ui| { assert_eq!(ui.ctx().pixels_per_point(),child_scale); (viewport.viewport_ui_cb)(ui) });
                    child_freed.borrow_mut().extend(output.textures_delta.free.iter().copied());
                    output.textures_delta.clear();
                    assert_eq!(ctx.viewport_id(),egui::ViewportId::ROOT);
                    assert_eq!(ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT),pass);
                });
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                let before_icon = Rect::from_min_size(pos2(1000.0,150.0),vec2(24.0,24.0));
                let after_icon = Rect::from_min_size(pos2(1050.0,150.0),vec2(24.0,24.0));
                let frame = |app: &mut PhotocraftApp, time, events| {
                    freed.borrow_mut().clear();
                    let mut input = egui::RawInput { time:Some(time), events,
                        screen_rect:Some(Rect::from_min_size(egui::Pos2::ZERO,vec2(1200.0,1800.0))), ..Default::default() };
                    input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(root_scale);
                    let mut output = ctx.run_ui(input, |ui| {
                        assert_eq!(ui.ctx().pixels_per_point(),root_scale);
                        let clip = ui.painter().clip_rect();
                        let pass = ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT);
                        icons::paint(ui,before_icon,"brush",24.0,Color32::WHITE);
                        toolbar(app,ui);
                        assert_eq!(ui.ctx().viewport_id(),egui::ViewportId::ROOT);
                        assert_eq!(ui.painter().clip_rect(),clip);
                        assert_eq!(ctx.cumulative_pass_nr_for(egui::ViewportId::ROOT),pass);
                        // A later chrome/canvas sibling must remain sound as well. This
                        // same icon at a different size exposes shared loader eviction.
                        egui::CentralPanel::default().show(ui, |ui| {
                            let response = ui.button("Root sibling after native popup");
                            ui.data_mut(|d| d.insert_temp(egui::Id::new("v59-root-sibling"),response.rect));
                            icons::paint(ui,after_icon,"brush",24.0,Color32::WHITE);
                        });
                    });
                    freed.borrow_mut().extend(output.textures_delta.free.iter().copied());
                    output.textures_delta.clear();
                    output
                };
                // ROOT must be older than the newly-created native viewport. The
                // old global loader pass index silently jumps backward on child end.
                for i in 0..24 { let mut out=frame(&mut app,i as f64*0.01,vec![]); out.textures_delta.clear(); }
                let baseline = buttons(&ctx,side);
                let sibling = ctx.data(|d| d.get_temp::<Rect>(egui::Id::new("v59-root-sibling"))).unwrap();
                let index = TOOL_SECTIONS.iter().flat_map(|s| s.iter()).position(|slot| slot.contains(&Tool::Brush)).unwrap();
                let at = baseline[index].center();
                let pointer = |pressed| egui::Event::PointerButton { pos:at, button:egui::PointerButton::Primary, pressed, modifiers:Default::default() };
                frame(&mut app,1.0,vec![egui::Event::PointerMoved(at),pointer(true)]).textures_delta.clear();
                let initial_tool = app.ui.tool;
                for time in [1.36,1.4,1.5] {
                    let mut output = frame(&mut app,time,vec![]);
                    assert_eq!(buttons(&ctx,side),baseline,"{theme:?}: source slot geometry stable");
                    assert_eq!(ctx.data(|d| d.get_temp::<Rect>(egui::Id::new("v59-root-sibling"))),Some(sibling));
                    let primitives = ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
                    for primitive in &primitives {
                        if let egui::epaint::Primitive::Mesh(mesh) = &primitive.primitive {
                            assert!(!freed.borrow().contains(&mesh.texture_id), "{theme:?}: root still references freed icon {:?} at {:?}",mesh.texture_id,mesh.calc_bounds());
                        }
                    }
                    for rect in baseline.iter().take(slot_count).chain([&before_icon,&after_icon]) {
                        assert!(primitives.iter().any(|primitive| matches!(&primitive.primitive,egui::epaint::Primitive::Mesh(mesh) if mesh.texture_id != egui::TextureId::Managed(0) && mesh.calc_bounds().contains(rect.center()) && primitive.clip_rect.contains(rect.center()))), "{theme:?}: toolbar and surrounding chrome retain visible icon meshes at {rect:?}");
                    }
                    output.textures_delta.clear();
                }
                assert!(calls.get() >= 3);
                frame(&mut app,1.6,vec![pointer(false)]).textures_delta.clear();
                assert_eq!(app.ui.tool,initial_tool,"long-press release must not select the group default");
                crate::native_popup_ui::close(&ctx);
                frame(&mut app,1.7,vec![]).textures_delta.clear();
                assert_eq!(buttons(&ctx,side),baseline);
                assert!(ctx.data(|d| d.get_temp::<(egui::Id,Rect)>(egui::Id::new("tool-flyout"))).is_none());
              }
            }
        }).unwrap().join().unwrap();
    }
}

#[cfg(test)]
#[path = "layer_opacity_controls_tests_v77.rs"]
mod layer_opacity_controls_tests_v77;

#[cfg(test)]
#[path = "layer_row_ui/clipping_tests_v89.rs"]
mod clipping_tests_v89;
