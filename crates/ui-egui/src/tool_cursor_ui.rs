//! Tool-shaped canvas cursors, rendered by the app on platforms without native equivalents.
use egui::{Color32, CursorIcon, Painter, Pos2, Stroke, vec2};

/// Only these painting tools display the imported tip's silhouette. Retouching,
/// cloning and history show a stable round area without changing the preset.
/// The regular Eraser uses painting presets, including sampled tips and pressure.
pub(crate) fn uses_preset_outline(tool: crate::state::Tool) -> bool {
    matches!(tool, crate::state::Tool::Brush | crate::state::Tool::Eraser | crate::state::Tool::MixerBrush | crate::state::Tool::Smudge)
}

pub(crate) fn area_radius(tool: crate::state::Tool, size: f32, zoom: f32) -> Option<f32> {
    if !size.is_finite() || !zoom.is_finite() || zoom <= 0.0 {
        return None;
    }
    // Retouch parse_brush has a one-document-pixel minimum. The two painting
    // erasers keep the regular brush engine's half-pixel minimum instead.
    let minimum = if matches!(tool, crate::state::Tool::Eraser | crate::state::Tool::BackgroundEraser) { 0.5 } else { 1.0 };
    let radius = size.clamp(minimum, photocraft_engine::paint::MAX_BRUSH_SIZE) * zoom * 0.5;
    radius.is_finite().then_some(radius.max(1.0))
}

pub(crate) fn area_circle(painter: &Painter, p: Pos2, tool: crate::state::Tool, size: f32, zoom: f32, centre: bool) -> CursorIcon {
    if let Some(radius) = area_radius(tool, size, zoom) {
        if crate::native_cursor::area_circle(painter.ctx(), radius, centre.then_some(3.0)) {
            return CursorIcon::None;
        }
        for (width, color) in [(2.0, Color32::from_black_alpha(160)), (1.0, Color32::from_white_alpha(230))] {
            painter.circle_stroke(p, radius, Stroke::new(width, color));
        }
        if !centre && !crate::native_cursor::enabled(painter.ctx()) {
            return CursorIcon::None;
        }
    }
    // Giant circular areas retain their software outline and an OS-driven centre.
    if !crate::native_cursor::crosshair(painter.ctx(), 3.0) {
        for (width, color) in [(2.5, Color32::from_black_alpha(140)), (1.0, Color32::from_white_alpha(220))] {
            painter.line_segment([p - vec2(3.0, 0.0), p + vec2(3.0, 0.0)], Stroke::new(width, color));
            painter.line_segment([p - vec2(0.0, 3.0), p + vec2(0.0, 3.0)], Stroke::new(width, color));
        }
    }
    CursorIcon::None
}

pub(crate) fn quick_selection_radius(size: f32, zoom: f32) -> Option<f32> {
    if !size.is_finite() || !zoom.is_finite() || zoom <= 0.0 {
        return None;
    }
    // Match select.quick's diameter clamp. Hardness and tip shape do not change
    // the circular seed region that command receives.
    let radius = size.clamp(1.0, 5000.0) * zoom * 0.5;
    radius.is_finite().then_some(radius.max(1.0))
}

pub(crate) fn quick_selection(painter: &Painter, p: Pos2, size: f32, zoom: f32, precise: bool) -> CursorIcon {
    if !precise && let Some(radius) = quick_selection_radius(size, zoom) {
        if crate::native_cursor::selection_circle(painter.ctx(), radius) {
            return CursorIcon::None;
        }
        for (width, color) in [(2.0, Color32::from_black_alpha(160)), (1.0, Color32::from_white_alpha(230))] {
            painter.circle_stroke(p, radius, Stroke::new(width, color));
        }
        // A footprint larger than an OS cursor keeps its exact software circle
        // and an independently moving native hotspot, like large paint tips.
        if !crate::native_cursor::enabled(painter.ctx()) {
            return CursorIcon::None;
        }
    }
    if !crate::native_cursor::crosshair(painter.ctx(), 6.0) {
        for (width, color) in [(2.5, Color32::from_black_alpha(140)), (1.0, Color32::from_white_alpha(220))] {
            painter.line_segment([p - vec2(6.0, 0.0), p + vec2(6.0, 0.0)], Stroke::new(width, color));
            painter.line_segment([p - vec2(0.0, 6.0), p + vec2(0.0, 6.0)], Stroke::new(width, color));
        }
    }
    CursorIcon::None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum Shape {
    ZoomIn,
    Eyedropper,
}

#[allow(dead_code)]
pub(crate) fn draw(painter: &Painter, p: Pos2, shape: Shape) -> CursorIcon {
    // The tip / upper-left lens rim is the click position. Two contrasting outlines remain
    // readable over both white paper and dark paint, independent of the application theme.
    for (width, color) in [(3.5, Color32::BLACK), (1.5, Color32::WHITE)] {
        let stroke = Stroke::new(width, color);
        match shape {
            Shape::ZoomIn => {
                let center = p + vec2(7.0, 7.0);
                painter.circle_stroke(center, 7.0, stroke);
                painter.line_segment([p + vec2(12.0, 12.0), p + vec2(19.0, 19.0)], stroke);
                painter.line_segment([center - vec2(3.0, 0.0), center + vec2(3.0, 0.0)], stroke);
                if shape == Shape::ZoomIn {
                    painter.line_segment([center - vec2(0.0, 3.0), center + vec2(0.0, 3.0)], stroke);
                }
            }
            Shape::Eyedropper => {
                painter.line_segment([p, p + vec2(3.0, -7.0)], stroke);
                painter.line_segment([p, p + vec2(7.0, -3.0)], stroke);
                painter.line_segment([p + vec2(3.0, -7.0), p + vec2(14.0, -18.0)], stroke);
                painter.line_segment([p + vec2(7.0, -3.0), p + vec2(18.0, -14.0)], stroke);
                painter.line_segment([p + vec2(10.0, -18.0), p + vec2(18.0, -10.0)], stroke);
                painter.line_segment([p + vec2(14.0, -18.0), p + vec2(18.0, -14.0)], stroke);
                painter.line_segment([p + vec2(13.0, -21.0), p + vec2(21.0, -13.0)], stroke);
                painter.line_segment([p + vec2(13.0, -21.0), p + vec2(16.0, -24.0)], stroke);
                painter.line_segment([p + vec2(21.0, -13.0), p + vec2(24.0, -16.0)], stroke);
                painter.line_segment([p + vec2(16.0, -24.0), p + vec2(24.0, -16.0)], stroke);
            }
        }
    }
    CursorIcon::None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn custom_cursors_render_and_hide_the_system_pointer() {
        for shape in [Shape::ZoomIn, Shape::Eyedropper] {
            let ctx = egui::Context::default();
            let mut output = ctx.run_ui(Default::default(), |ui| {
                assert_eq!(draw(ui.painter(), egui::pos2(60.0, 60.0), shape), CursorIcon::None);
            });
            assert!(output.shapes.len() >= 6);
            output.textures_delta.clear();
        }
    }
}
