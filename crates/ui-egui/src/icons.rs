//! Icon rendering. SVGs are embedded (see `icon_data.rs`), recoloured to white, rasterized by the
//! egui_extras SVG loader at the exact on-screen pixel size (crisp at any DPI), and tinted per use.
//!
//! (History: the first shell used Unicode glyphs; half of them rendered as tofu boxes because the
//! bundled fonts lacked them. Vector icons fix that for good.)

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use egui::emath::GuiRounding as _;
use egui::{Color32, Rect, Response, Sense, Vec2};

use crate::icon_data::ICONS;
use crate::state::Tool;
use crate::theme::Tokens;

fn white_icons() -> &'static HashMap<&'static str, Arc<[u8]>> {
    static MAP: OnceLock<HashMap<&'static str, Arc<[u8]>>> = OnceLock::new();
    MAP.get_or_init(|| {
        ICONS
            .iter()
            .map(|(name, bytes)| {
                let svg = String::from_utf8_lossy(bytes).replace("currentColor", "#ffffff").replace("stroke-width=\"2\"", "stroke-width=\"1.75\"");
                (*name, Arc::from(svg.into_bytes().into_boxed_slice()))
            })
            .collect()
    })
}

pub fn exists(name: &str) -> bool {
    white_icons().contains_key(name)
}

/// An egui image for an icon, tinted.
pub fn image(ctx: &egui::Context, name: &str, size: f32, tint: Color32) -> egui::Image<'static> {
    sized_image(ctx, name, Vec2::splat(size), tint)
}

fn sized_image(ctx: &egui::Context, name: &str, size: Vec2, tint: Color32) -> egui::Image<'static> {
    let bytes = white_icons().get(name).or_else(|| white_icons().get("square")).cloned().unwrap_or_default();
    // egui 0.36 ages multi-size SVG buckets with each viewport's local pass
    // number. A fresh flyout can reset that shared age while ROOT is still
    // painting, freeing live toolbar textures at ROOT's end_pass. Give each
    // raster size its own URI so sizes remain cached independently. Include
    // physical pixels: menus on a different-DPI monitor must not share a
    // multi-size bucket with the same logical-size icon in the main window.
    let pixels = (size * ctx.pixels_per_point()).round();
    let (width, height) = (pixels.x.max(1.0) as u32, pixels.y.max(1.0) as u32);
    egui::Image::from_bytes(format!("bytes://icons/{name}/{width}x{height}.svg"), egui::load::Bytes::Shared(bytes))
        .fit_to_exact_size(size)
        .maintain_aspect_ratio(false)
        .tint(tint)
}

fn paint_rect(ui: &egui::Ui, rect: Rect, name: &str, tint: Color32) {
    // Rounding the two edges can change a fractional-DPI box by one pixel.
    // Key the actual rounded rectangle, exactly as Image::paint_at will load it.
    let rect = rect.round_to_pixels(ui.pixels_per_point());
    sized_image(ui.ctx(), name, rect.size(), tint).paint_at(ui, rect);
}

/// Paint an icon centred in `rect`.
pub fn paint(ui: &egui::Ui, rect: Rect, name: &str, size: f32, tint: Color32) {
    let r = Rect::from_center_size(rect.center(), Vec2::splat(size));
    paint_rect(ui, r, name, tint);
}

fn cursor_area_id(ctx: &egui::Context) -> egui::Id {
    egui::Id::new(("pc-icon-cursor", ctx.viewport_id()))
}

/// An icon as the pointer, above every window: `name` with its hotspot `hot` (a fraction of the
/// icon box) on `p`, white with a dark outline so it reads on any image. The caller hides the OS
/// cursor (`CursorIcon::None`).
pub fn cursor(ctx: &egui::Context, name: &str, p: egui::Pos2, hot: Vec2, size: f32) {
    if white_icons().get(name).is_some_and(|svg| crate::native_cursor::glyph(ctx, name, svg, hot, size)) {
        return;
    }
    let rect = Rect::from_min_size(p - hot * size, Vec2::splat(size));
    // Each native viewport shares the root egui context. A single global Area id made the root
    // canvas and a detached document overwrite one another's cursor layer while the pointer
    // moved, producing a one-frame flash/jump. Include the viewport id so each canvas owns a
    // stable overlay layer.
    let area = egui::Area::new(cursor_area_id(ctx)).order(egui::Order::Tooltip).fixed_pos(rect.min).constrain(false).interactable(false);
    area.show(ctx, |ui| {
        for (dx, dy) in [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0)] {
            paint_rect(ui, rect.translate(egui::vec2(dx, dy)), name, Color32::from_black_alpha(200));
        }
        paint_rect(ui, rect, name, Color32::WHITE);
    });
}

pub fn tool_icon(t: Tool) -> &'static str {
    match t {
        Tool::Move => "move",
        Tool::RectMarquee => "square-dashed",
        Tool::EllipseMarquee => "circle-dashed",
        Tool::Brush => "brush",
        Tool::Pencil => "pencil",
        Tool::MixerBrush => "palette",
        Tool::Eraser => "eraser",
        Tool::BackgroundEraser => "eraser-background",
        Tool::MagicEraser => "eraser-magic",
        Tool::Eyedropper => "pipette",
        Tool::Ruler => "ruler",
        Tool::Note => "message-square",
        Tool::Count => "circle-dot",
        Tool::Lasso => "lasso",
        Tool::PolygonLasso => "pentagon",
        Tool::MagneticLasso => "lasso-magnetic",
        Tool::MagicWand => "wand-sparkles",
        Tool::Crop => "crop",
        Tool::Slice => "slice-knife",
        Tool::SliceSelect => "square-dashed-mouse-pointer",
        Tool::Gradient => "blend",
        Tool::PaintBucket => "paint-bucket",
        Tool::Type | Tool::VerticalType => "type",
        Tool::Hand => "hand",
        Tool::RotateView => "rotate-cw",
        Tool::Zoom => "zoom-in",
        Tool::SpotHealing | Tool::Healing => "bandage",
        Tool::Patch => "lasso-select",
        Tool::ContentAwareMove => "arrow-left-right",
        Tool::CloneStamp => "stamp",
        Tool::HistoryBrush => "clock",
        Tool::Blur => "droplet",
        Tool::Sharpen => "triangle",
        Tool::Smudge => "pointer",
        Tool::Dodge => "lollipop",
        Tool::Burn => "flame",
        Tool::Sponge => "cloud",
        Tool::QuickSelection => "quick-selection",
        Tool::ObjectSelection => "square-dashed-mouse-pointer",
        Tool::Pen => "pen-tool",
        Tool::PathSelection => "mouse-pointer-2",
        Tool::DirectSelection => "direct-select",
        Tool::Rectangle => "rectangle-horizontal",
        Tool::EllipseShape => "circle",
        Tool::Triangle => "triangle",
        Tool::Polygon => "pentagon",
        Tool::Line => "slash",
        Tool::CustomShape => "cloud",
    }
}

/// Selected and hover fill shared by square icon buttons. Returns the icon tint.
pub fn button_chrome(ui: &egui::Ui, rect: Rect, selected: bool, hovered: bool) -> Color32 {
    let t = Tokens::get(ui.ctx());
    if selected {
        ui.painter().rect_filled(rect, t.radius_sm, t.accent_soft);
        ui.painter().rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, t.accent_border), egui::StrokeKind::Inside);
    } else if hovered {
        ui.painter().rect_filled(rect, t.radius_sm, t.hover);
    }
    if selected {
        t.accent_text
    } else if hovered {
        t.text
    } else {
        t.icon
    }
}

/// Square icon button: transparent until hovered; `selected` gets the accent treatment.
pub fn button(ui: &mut egui::Ui, name: &str, box_size: f32, selected: bool, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(box_size), Sense::click());
    let tint = button_chrome(ui, rect, selected, resp.hovered());
    paint(ui, rect, name, (box_size * 0.52).round(), tint);
    if tooltip.is_empty() { resp } else { resp.on_hover_text(tl!(tooltip)) }
}

/// Rail toggle: "on" gets a quiet filled background and full-strength icon (no accent).
pub fn rail_button(ui: &mut egui::Ui, name: &str, box_size: f32, on: bool, tooltip: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(box_size), Sense::click());
    if on {
        ui.painter().rect_filled(rect, t.radius_sm, t.card);
        ui.painter().rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, t.radius_sm, t.hover);
    }
    let tint = if on || resp.hovered() { t.text } else { t.text_faint };
    paint(ui, rect, name, (box_size * 0.52).round(), tint);
    if tooltip.is_empty() { resp } else { resp.on_hover_text(tl!(tooltip)) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_an_icon() {
        for t in Tool::ALL {
            assert!(exists(tool_icon(t)), "{t:?}");
        }
    }

    #[test]
    fn quick_selection_does_not_reuse_the_ellipse_marquee_symbol_v67() {
        let icons = white_icons();
        assert_ne!(icons.get(tool_icon(Tool::QuickSelection)), icons.get(tool_icon(Tool::EllipseMarquee)));
    }

    #[test]
    fn icons_are_recoloured() {
        let m = white_icons();
        assert!(m.len() >= 60);
        for (name, b) in m.iter() {
            let s = std::str::from_utf8(b).unwrap();
            assert!(!s.contains("currentColor"), "{name}");
        }
    }

    #[test]
    fn cursor_overlay_id_is_scoped_to_viewport() {
        let ctx = egui::Context::default();
        let root = cursor_area_id(&ctx);
        let child = egui::Id::new(("pc-icon-cursor", egui::ViewportId::from_hash_of("detached")));
        assert_ne!(root, child, "cursor overlays in detached viewports must not share one Area id");
    }
}
