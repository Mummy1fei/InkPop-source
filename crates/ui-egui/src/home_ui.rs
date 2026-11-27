//! Home screen: local recent-document previews, with no UI-thread file I/O.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};

use egui::emath::GuiRounding as _;
use egui::{Color32, Rect, Sense, TextureHandle, pos2, vec2};

use crate::{PhotocraftApp, theme::Tokens};

const RECENT_LIMIT: usize = 100;
const GAP: f32 = 16.0;
const CAPTION_HEIGHT: f32 = 28.0;

struct Pending {
    path: String,
    receiver: Receiver<Option<egui::ColorImage>>,
    invalidated: bool,
}

struct Preview {
    texture: TextureHandle,
    has_alpha: bool,
}

/// At most the recent-list limit of completed previews and one request. Failures are cached too:
/// neither a missing file nor an unsupported format is retried every frame.
#[derive(Default)]
pub(crate) struct HomeState {
    cache: HashMap<String, Option<Preview>>,
    checker: Option<TextureHandle>,
    pending: Option<Pending>,
    last_frame: Option<u64>,
}

impl HomeState {
    pub(crate) fn invalidate(&mut self, path: &str) {
        self.cache.remove(path);
        if let Some(pending) = &mut self.pending
            && pending.path == path
        {
            pending.invalidated = true;
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: u64, recent: &[String], service: Option<&crate::RecentThumbnailFn>) {
        // A document was shown between Home frames. Recheck the service once;
        // its path/mtime/length cache determines whether pixels need rebuilding.
        if self.last_frame.is_some_and(|previous| frame > previous.saturating_add(1)) {
            self.cache.clear();
            if let Some(pending) = &mut self.pending {
                pending.invalidated = true;
            }
        }
        self.last_frame = Some(frame);
        self.cache.retain(|path, _| recent.contains(path));
        if let Some(pending) = &self.pending {
            let ready = match pending.receiver.try_recv() {
                Ok(image) => Some(image),
                Err(TryRecvError::Disconnected) => Some(None),
                Err(TryRecvError::Empty) => None,
            };
            if let Some(image) = ready {
                let path = pending.path.clone();
                let keep = !pending.invalidated && recent.contains(&path);
                self.pending = None;
                if keep {
                    let preview = image.filter(valid_image).map(|image| {
                        let has_alpha = image.pixels.iter().any(|pixel| pixel.a() < 255);
                        if has_alpha && self.checker.is_none() {
                            let (light, dark) = (Color32::from_gray(250), Color32::from_gray(214));
                            let tile = egui::ColorImage::new([2, 2], vec![light, dark, dark, light]);
                            self.checker = Some(ctx.load_texture("home-transparency", tile, egui::TextureOptions::NEAREST_REPEAT));
                        }
                        Preview { texture: ctx.load_texture("home-recent-preview", image, egui::TextureOptions::LINEAR), has_alpha }
                    });
                    self.cache.insert(path, preview);
                }
            }
        }
        if self.pending.is_none() {
            if let Some(service) = service {
                if let Some(path) = recent.iter().find(|path| !self.cache.contains_key(*path)) {
                    self.pending = Some(Pending { path: path.clone(), receiver: service(path), invalidated: false });
                }
            } else {
                self.cache.extend(recent.iter().cloned().map(|path| (path, None)));
            }
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(24));
        }
    }
}

fn valid_image(image: &egui::ColorImage) -> bool {
    let [width, height] = image.size;
    width > 0 && height > 0 && width <= 1024 && height <= 1024 && width.checked_mul(height) == Some(image.pixels.len())
}

fn columns(width: f32) -> usize {
    (((width + GAP) / (168.0 + GAP)).floor() as usize).clamp(1, 4)
}

pub(crate) fn drop_hint(is_wayland: bool) -> &'static str {
    if is_wayland { tl!("Use File › Open to open an image.") } else { tl!("Drop an image or PSD anywhere to open it.") }
}

fn actions(app: &mut PhotocraftApp, ui: &mut egui::Ui, compact: bool) {
    let t = Tokens::get(ui.ctx());
    ui.label(
        egui::RichText::new(photocraft_engine::build_info::CUSTOM_PRODUCT_NAME).font(crate::theme::semibold(if compact { 26.0 } else { 30.0 })).color(t.text),
    );
    ui.add_space(if compact { 8.0 } else { 16.0 });
    let new_label = if compact { tl!("New document…").to_string() } else { crate::shortcuts::command_label(app, "New document…", "file.new") };
    let open_label = if compact { tl!("Open…").to_string() } else { crate::shortcuts::command_label(app, "Open…", "file.open") };
    let mut buttons = |ui: &mut egui::Ui| {
        let width = if compact { ((ui.available_width() - 12.0) / 2.0).max(80.0) } else { ui.available_width().min(208.0) };
        if crate::widgets::primary_button(ui, &new_label, width).clicked() {
            app.ui.open_dialog(crate::state::DialogKind::NewDocument, crate::state::UiState::new_document_fields());
        }
        if !compact {
            ui.add_space(8.0);
        }
        if crate::widgets::secondary_button(ui, &open_label, width).clicked() {
            app.open_dialog_file();
        }
    };
    if compact {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            buttons(ui);
        });
    } else {
        buttons(ui);
        ui.add_space(18.0);
        ui.label(egui::RichText::new(tl!("Create a new document or open an existing file.")).size(12.5).color(t.text_dim));
    }
}

fn card(app: &PhotocraftApp, ui: &mut egui::Ui, path: &str, width: f32, preview_h: f32) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(width, preview_h + CAPTION_HEIGHT), Sense::click());
    let painter = ui.painter_at(rect);
    let ppp = ui.pixels_per_point();
    let preview = Rect::from_min_size(rect.min, vec2(width, preview_h)).round_to_pixels(ppp);
    painter.rect_filled(preview, t.radius_sm, t.field);
    if let Some(Some(Preview { texture, has_alpha })) = app.home.cache.get(path) {
        let size = texture.size_vec2();
        let scale = (preview.width() / size.x).min(preview.height() / size.y);
        let image = Rect::from_center_size(preview.center(), size * scale).round_to_pixels(ppp).intersect(preview);
        let inset = image.min - preview.min;
        let radius = (t.radius_sm - inset.x.max(inset.y)).max(0.0);
        let surface = egui::epaint::RectShape::filled(image, radius, Color32::WHITE);
        // Use one pixel-aligned outline for the checker, image and scissor. The
        // final partial checker tile must never rasterize beyond the photograph.
        let image_painter = painter.with_clip_rect(image);
        if *has_alpha && let Some(tile) = &app.home.checker {
            let uv = Rect::from_min_size(egui::Pos2::ZERO, image.size() / 12.0);
            image_painter.add(surface.clone().with_texture(tile.id(), uv));
        }
        image_painter.add(surface.with_texture(texture.id(), Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0))));
    } else {
        crate::icons::paint(ui, preview, "image", 28.0, t.text_faint);
    }
    let name = crate::file_open::display_name(path);
    let galley = crate::tab_strip::elided(ui, &name, egui::FontId::proportional(12.5), t.text, width);
    let caption = pos2(rect.center().x - galley.size().x / 2.0, preview.bottom() + 6.0);
    painter.galley(caption, galley, t.text);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    response.on_hover_text(path).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn recent_grid(app: &mut PhotocraftApp, ui: &mut egui::Ui, recent: &[String]) {
    let t = Tokens::get(ui.ctx());
    let bounds = ui.available_rect_before_wrap();
    let count = columns(bounds.width());
    let width = ((bounds.width() - GAP * (count - 1) as f32) / count as f32).min(320.0);
    let grid_width = width * count as f32 + GAP * (count - 1) as f32;
    let left = bounds.left() + ((bounds.width() - grid_width) / 2.0).max(0.0);
    let mut heading = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(left, bounds.top()), bounds.max)));
    heading.label(egui::RichText::new(tl!("Recent")).font(crate::theme::semibold(17.0)).color(t.text));
    heading.add_space(12.0);
    let top = heading.cursor().top();
    let height = (bounds.bottom() - top).max(0.0);
    // Home is deliberately a single screen. Reserve a readable minimum card
    // size, show at most five complete rows, and leave older entries in File.
    let rows = (((height + GAP) / (96.0 + CAPTION_HEIGHT + GAP)).floor() as usize).min(5);
    let visible = recent.get(..recent.len().min(rows * count)).unwrap_or_default();
    app.home.update(ui.ctx(), app.frame, visible, app.services.request_recent_thumbnail.as_ref());
    if recent.is_empty() {
        heading.label(egui::RichText::new(tl!("Create a new document or open an existing file.")).color(t.text_dim));
        return;
    }
    if rows == 0 {
        return;
    }
    let preview_h = ((height - GAP * rows.saturating_sub(1) as f32) / rows as f32 - CAPTION_HEIGHT).clamp(96.0, (width * 0.68).clamp(96.0, 174.0));
    let card_h = preview_h + CAPTION_HEIGHT;
    let mut open = None;
    for (index, path) in visible.iter().enumerate() {
        let rect = Rect::from_min_size(pos2(left + (index % count) as f32 * (width + GAP), top + (index / count) as f32 * (card_h + GAP)), vec2(width, card_h));
        let mut cell = ui.new_child(egui::UiBuilder::new().id_salt(index).max_rect(rect));
        if card(app, &mut cell, path, width, preview_h) {
            open = Some(path.clone());
        }
    }
    if let Some(path) = open
        && let Err(error) = app.open_path(&path)
    {
        app.open_failed(&crate::file_open::display_name(&path), &error);
    }
}

pub(crate) fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let recent: Vec<String> = if cfg!(target_arch = "wasm32") { Vec::new() } else { app.ui.recent_files.iter().take(RECENT_LIMIT).cloned().collect() };
    let area = ui.available_rect_before_wrap().shrink2(vec2(24.0, 18.0));
    let compact = area.width() < 720.0;
    let inset = ((area.width() - 960.0) * 0.1).clamp(0.0, 160.0);
    let body_width = (area.width() - inset * 2.0).min(1760.0);
    let left = area.center().x - body_width / 2.0;
    let footer_top = (area.bottom() - 64.0).max(area.top());
    let bottom = (footer_top - 44.0).max(area.top());
    let top = (area.top() + if compact { 12.0 } else { (area.height() * 0.08).clamp(24.0, 104.0) }).min(bottom);
    let body = Rect::from_min_max(pos2(left, top), pos2(left + body_width, bottom));
    let mut main = ui.new_child(egui::UiBuilder::new().id_salt("home-body").max_rect(body));
    main.set_clip_rect(main.clip_rect().intersect(body));
    if compact {
        actions(app, &mut main, true);
        main.add_space(24.0);
        recent_grid(app, &mut main, &recent);
    } else {
        let actions_rect = Rect::from_min_max(body.min, pos2(body.left() + 208.0, body.bottom()));
        let mut sidebar = main.new_child(egui::UiBuilder::new().max_rect(actions_rect));
        actions(app, &mut sidebar, false);
        let grid = Rect::from_min_max(pos2(actions_rect.right() + 32.0, body.top()), body.max);
        let mut content = main.new_child(egui::UiBuilder::new().max_rect(grid));
        recent_grid(app, &mut content, &recent);
    }
    let mut hint = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(left, bottom + 10.0), pos2(left + body_width, footer_top))));
    hint.label(egui::RichText::new(drop_hint(app.services.is_wayland)).size(12.0).color(Tokens::get(ui.ctx()).text_faint));
    let mut footer = ui.new_child(egui::UiBuilder::new().id_salt("home-footer").max_rect(Rect::from_min_max(pos2(area.left(), footer_top), area.max)));
    crate::welcome_links::show(&mut footer);
}

#[cfg(test)]
#[path = "home_ui_tests_v70.rs"]
mod tests;
