use super::*;
use egui_kittest::{Harness, kittest::Queryable};

struct Fixture {
    ready: bool,
    native: bool,
    message: String,
    outer: egui::Rect,
    layout: Option<PromptLayout>,
}

fn harness(theme: crate::theme::ThemeKind, os: egui::os::OperatingSystem, native: bool, message: &str, size: egui::Vec2) -> Harness<'static, Fixture> {
    let mut harness = Harness::builder().with_size(size).with_os(os).build_ui_state(
        |ui, state: &mut Fixture| {
            if !state.ready {
                return;
            }
            let layout = prompt_layout(ui.ctx(), &state.message, false);
            let rect = egui::Rect::from_min_size(ui.ctx().content_rect().min, egui::vec2(layout.initial_size.x, layout.max_height));
            let mut surface = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            let frame = if state.native { egui::Frame::central_panel(ui.style()) } else { egui::Frame::popup(ui.style()) };
            let response = frame
                .inner_margin(PROMPT_PADDING)
                .show(&mut surface, |body| prompt_content(body, (!state.native).then_some("Unsaved changes"), &state.message, false, &layout));
            state.outer = response.response.rect;
            state.layout = Some(layout);
        },
        Fixture { ready: false, native, message: message.into(), outer: egui::Rect::NOTHING, layout: None },
    );
    PhotocraftApp::setup_context(&harness.ctx, theme);
    harness.state_mut().ready = true;
    harness.run_steps(5);
    harness
}

#[test]
fn confirmation_content_has_twenty_four_point_padding_and_clear_action_spacing_in_five_themes_v80() {
    let message = "Save changes to the synthetic document before closing?";
    for theme in crate::theme::ThemeKind::ALL {
        for os in [egui::os::OperatingSystem::Windows, egui::os::OperatingSystem::Mac] {
            for native in [true, false] {
                let h = harness(theme, os, native, message, egui::vec2(900.0, 600.0));
                let frame = h.state().outer;
                let text = h.get_by_label(message).rect();
                let labels = if os == egui::os::OperatingSystem::Mac { ["(D)on't Save", "(C)ancel", "(S)ave"] } else { ["(Y)es", "(N)o", "Cancel"] };
                let buttons = labels.map(|label| h.get_by_label(label).rect());
                let first_content = if native { text } else { h.get_by_label("Unsaved changes").rect() };
                assert!(first_content.top() - frame.top() >= PROMPT_PADDING - 0.5);
                assert!(text.left() - frame.left() >= PROMPT_PADDING - 0.5);
                assert!(frame.right() - text.right() >= PROMPT_PADDING - 0.5);
                assert!(frame.bottom() - buttons[2].bottom() >= PROMPT_PADDING - 0.5);
                assert!(buttons[0].top() - text.bottom() >= MESSAGE_BUTTON_GAP - 0.5);
                assert!((buttons[1].left() - buttons[0].right() - BUTTON_GAP).abs() <= 1.0);
                assert!((buttons[2].left() - buttons[1].right() - BUTTON_GAP).abs() <= 1.0);
                assert!(((buttons[0].left() + buttons[2].right()) * 0.5 - frame.center().x).abs() <= 1.0);
                assert!(frame.contains_rect(text) && buttons.iter().all(|button| frame.contains_rect(*button)));
            }
        }
    }
}

#[test]
fn wrapped_filenames_keep_their_text_and_never_end_in_a_punctuation_only_row_v80() {
    let message = "关闭前是否保存对“Example 图像 2000年01月01日 00_00_00-2.png”的更改？";
    for theme in crate::theme::ThemeKind::ALL {
        let h = harness(theme, egui::os::OperatingSystem::Windows, true, message, egui::vec2(900.0, 600.0));
        let style = h.ctx.global_style();
        let font = egui::TextStyle::Body.resolve(&style);
        let mut exercised_old_orphan = false;
        for width in (240..521).step_by(2) {
            let raw = h.ctx.fonts_mut(|fonts| fonts.layout(message.into(), font.clone(), style.visuals.text_color(), width as f32));
            exercised_old_orphan |= punctuation_only_last_row(&raw);
            let fixed = message_galley(&h.ctx, message, width as f32);
            assert_eq!(fixed.job.text, message);
            assert!(!punctuation_only_last_row(&fixed), "{theme:?}/{width}: final punctuation must stay with text");
        }
        assert!(exercised_old_orphan, "the fixture must exercise an actual dependency line-wrap boundary");
    }
}

#[test]
fn very_long_names_and_larger_body_fonts_keep_actions_inside_the_work_area_v80() {
    let message = format!("Save changes to “{}” before closing?", "synthetic document 长文件名 ".repeat(35));
    for theme in crate::theme::ThemeKind::ALL {
        for native in [true, false] {
            let mut h = harness(theme, egui::os::OperatingSystem::Windows, native, &message, egui::vec2(640.0, 440.0));
            h.ctx.global_style_mut(|style| {
                style.text_styles.insert(egui::TextStyle::Body, egui::FontId::proportional(20.0));
            });
            h.run_steps(5);
            let frame = h.state().outer;
            let layout = h.state().layout.as_ref().unwrap();
            assert!(layout.initial_size.x <= 616.0 && frame.width() <= 618.0);
            assert!(layout.initial_size.y <= 416.0 && frame.height() <= 418.0);
            for label in ["(Y)es", "(N)o", "Cancel"] {
                let button = h.get_by_label(label).rect();
                assert!(frame.contains_rect(button));
                assert!(frame.bottom() - button.bottom() >= PROMPT_PADDING - 0.5);
            }
        }
    }
}
