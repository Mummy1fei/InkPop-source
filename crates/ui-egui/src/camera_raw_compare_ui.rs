//! Before/after presentation uses existing image textures; changing views never develops pixels.
use egui::{Key, KeyboardShortcut, Modifiers, Rect, Stroke, Ui, pos2};

use crate::state::{CameraRawComparison, CameraRawPreviewState};

impl CameraRawComparison {
    pub(crate) const ALL: [Self; 5] = [Self::Single, Self::SideBySide, Self::LeftRightSplit, Self::TopBottom, Self::TopBottomSplit];

    pub(crate) fn next(self) -> Self {
        match self {
            Self::Single => Self::SideBySide,
            Self::SideBySide => Self::LeftRightSplit,
            Self::LeftRightSplit => Self::TopBottom,
            Self::TopBottom => Self::TopBottomSplit,
            Self::TopBottomSplit => Self::Single,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Single => tl!("Single View"),
            Self::SideBySide => tl!("Side by Side"),
            Self::LeftRightSplit => tl!("Left / Right Split"),
            Self::TopBottom => tl!("Top / Bottom"),
            Self::TopBottomSplit => tl!("Top / Bottom Split"),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Pane {
    /// A split shares this full viewport between its two textures. A pair fits each image here.
    pub viewport: Rect,
    pub clip: Rect,
    pub before: bool,
}

pub(super) fn panes(state: &CameraRawPreviewState, view: Rect, show_before: bool) -> Vec<Pane> {
    let single = |viewport, before| Pane { viewport, clip: viewport, before };
    let gap = 4.0f32.min(view.width().min(view.height()).max(0.0) * 0.1);
    match state.comparison {
        CameraRawComparison::Single => vec![single(view, show_before)],
        CameraRawComparison::SideBySide => vec![
            single(Rect::from_min_max(view.min, pos2(view.center().x - gap, view.bottom())), !state.swapped),
            single(Rect::from_min_max(pos2(view.center().x + gap, view.top()), view.max), state.swapped),
        ],
        CameraRawComparison::TopBottom => vec![
            single(Rect::from_min_max(view.min, pos2(view.right(), view.center().y - gap)), !state.swapped),
            single(Rect::from_min_max(pos2(view.left(), view.center().y + gap), view.max), state.swapped),
        ],
        CameraRawComparison::LeftRightSplit => vec![
            Pane { viewport: view, clip: Rect::from_min_max(view.min, pos2(view.center().x, view.bottom())), before: !state.swapped },
            Pane { viewport: view, clip: Rect::from_min_max(pos2(view.center().x, view.top()), view.max), before: state.swapped },
        ],
        CameraRawComparison::TopBottomSplit => vec![
            Pane { viewport: view, clip: Rect::from_min_max(view.min, pos2(view.right(), view.center().y)), before: !state.swapped },
            Pane { viewport: view, clip: Rect::from_min_max(pos2(view.left(), view.center().y), view.max), before: state.swapped },
        ],
    }
}

pub(super) fn divider(ui: &Ui, mode: CameraRawComparison, view: Rect) {
    let points = match mode {
        CameraRawComparison::LeftRightSplit => Some([pos2(view.center().x, view.top()), pos2(view.center().x, view.bottom())]),
        CameraRawComparison::TopBottomSplit => Some([pos2(view.left(), view.center().y), pos2(view.right(), view.center().y)]),
        _ => None,
    };
    if let Some(points) = points {
        let t = crate::theme::Tokens::get(ui.ctx());
        ui.painter().line_segment(points, Stroke::new(2.0, t.chrome));
        ui.painter().line_segment(points, Stroke::new(1.0, t.text_dim));
    }
}

pub(super) fn controls(ui: &mut Ui, state: &mut CameraRawPreviewState, show_before: &mut bool) {
    if state.comparison == CameraRawComparison::Single {
        crate::camera_raw_ui::square_checkbox(ui, show_before, tl!("Before (Y)"));
    } else if ui.button(tl!("Swap Before/After")).clicked() {
        state.swapped = !state.swapped;
    }
    egui::ComboBox::from_id_salt("cr-comparison-mode")
        .selected_text(state.comparison.label())
        .show_ui(ui, |ui| {
            for mode in CameraRawComparison::ALL {
                ui.selectable_value(&mut state.comparison, mode, mode.label());
            }
        })
        .response
        .on_hover_text("Q");
}

pub(super) fn keys(ctx: &egui::Context, state: &mut CameraRawPreviewState, show_before: &mut bool) {
    // A mask name, preset name or numeric field owns its typing; modifiers retain their meaning.
    if ctx.text_edit_focused() {
        return;
    }
    let cycle = crate::shortcuts::consume(ctx, &KeyboardShortcut::new(Modifiers::NONE, Key::Q));
    let before = crate::shortcuts::consume(ctx, &KeyboardShortcut::new(Modifiers::NONE, Key::Y));
    if cycle {
        state.comparison = state.comparison.next();
    }
    if before {
        *show_before = !*show_before;
    }
    if cycle || before {
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::vec2;

    #[test]
    fn comparison_cycle_and_serde_cover_five_modes_without_losing_old_names() {
        let mut mode = CameraRawComparison::Single;
        for expected in CameraRawComparison::ALL {
            assert_eq!(mode, expected);
            let value = serde_json::to_value(mode).unwrap();
            assert_eq!(serde_json::from_value::<CameraRawComparison>(value).unwrap(), mode);
            mode = mode.next();
        }
        assert_eq!(mode, CameraRawComparison::Single);
        assert_eq!(serde_json::from_str::<CameraRawComparison>("\"sideBySide\"").unwrap(), CameraRawComparison::SideBySide);
        assert_eq!(serde_json::from_str::<CameraRawComparison>("\"topBottom\"").unwrap(), CameraRawComparison::TopBottom);
    }

    #[test]
    fn split_images_share_registration_and_pair_images_fit_separate_panes() {
        let view = Rect::from_min_size(pos2(10.0, 20.0), vec2(800.0, 600.0));
        for mode in CameraRawComparison::ALL {
            for swapped in [false, true] {
                let state = CameraRawPreviewState { comparison: mode, swapped, ..Default::default() };
                let result = panes(&state, view, false);
                for pane in &result {
                    assert!(view.contains_rect(pane.viewport) && pane.viewport.contains_rect(pane.clip));
                    assert!(pane.viewport.is_finite() && pane.clip.is_finite());
                }
                if mode == CameraRawComparison::Single {
                    assert_eq!(result.len(), 1);
                    assert!(!result[0].before);
                } else {
                    assert_eq!(result.len(), 2);
                    assert_eq!(result[0].before, !swapped);
                    assert_eq!(result[1].before, swapped);
                    if matches!(mode, CameraRawComparison::LeftRightSplit | CameraRawComparison::TopBottomSplit) {
                        assert_eq!(result[0].viewport, view);
                        assert_eq!(result[1].viewport, view);
                        assert!((result[0].clip.area() + result[1].clip.area() - view.area()).abs() < 0.01);
                    } else {
                        assert_ne!(result[0].viewport, result[1].viewport);
                    }
                }
            }
        }
    }

    #[test]
    fn a_focused_name_field_keeps_q_and_y_typing() {
        let ctx = egui::Context::default();
        let mut state = CameraRawPreviewState::default();
        let mut before = false;
        let mut name = "Mask".to_string();
        for events in [
            vec![],
            vec![egui::Event::Key { key: Key::Q, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }, egui::Event::Text("q".into())],
            vec![egui::Event::Key { key: Key::Y, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }, egui::Event::Text("y".into())],
        ] {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                ui.add(egui::TextEdit::singleline(&mut name).id(egui::Id::new("mask-name-v62"))).request_focus();
                keys(ui.ctx(), &mut state, &mut before);
            });
            out.textures_delta.clear();
        }
        assert!(name.contains('q') && name.contains('y'), "the field must receive the typed text: {name}");
        assert_eq!(state.comparison, CameraRawComparison::Single);
        assert!(!before);
    }
}
