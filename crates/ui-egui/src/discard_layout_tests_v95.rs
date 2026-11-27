use super::*;
use egui_kittest::{Harness, kittest::Queryable};

struct Fixture {
    ready: bool,
    native: bool,
    name: String,
    message: String,
    frame: egui::Rect,
}

#[test]
fn long_document_names_are_bounded_without_losing_the_extension_v95() {
    for name in ["long-download-".repeat(30) + ".jpg", "中文图片文件名".repeat(40) + ".psd"] {
        let shown = compact_document_name(&name);
        assert!(shown.chars().count() <= 80);
        assert!(shown.contains('…'));
        assert!(shown.starts_with(&name.chars().take(52).collect::<String>()));
        assert!(shown.ends_with(name.rsplit_once('.').map(|(_, extension)| extension).unwrap()));
    }
    assert_eq!(compact_document_name("短文件名.psd"), "短文件名.psd");
}

#[test]
fn actual_modal_recovers_from_a_narrow_sizing_pass_and_native_content_keeps_a_button_row_v95() {
    for theme in crate::theme::ThemeKind::ALL {
        for language in ["en", "zh-hans"] {
            crate::i18n::with_language(crate::i18n::Lang::from_code(language).unwrap(), || {
                for native in [false, true] {
                    let name = if language == "en" { "BrowserDownloadName_".repeat(30) + ".jpg" } else { "长文件名中文备份".repeat(35) + ".psd" };
                    let message = crate::i18n::fmt(
                        tl!("Do you want to save the changes you made to “{name}” before closing?"),
                        &[("name", &compact_document_name(&name))],
                    );
                    let mut h = Harness::builder().with_os(egui::os::OperatingSystem::Windows).with_size(egui::vec2(1000.0, 700.0)).build_ui_state(
                        |ui, state: &mut Fixture| {
                            if !state.ready {
                                return;
                            }
                            let layout = prompt_layout(ui.ctx(), &state.message, false);
                            if state.native {
                                let builder = native_builder(ui.ctx(), "Unsaved changes", true, layout.initial_size);
                                assert_eq!(builder.min_inner_size.map(|size| size.x), Some(layout.initial_size.x));
                                let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, layout.initial_size);
                                let mut surface = ui.new_child(egui::UiBuilder::new().max_rect(viewport));
                                let content = egui::Frame::central_panel(ui.style())
                                    .inner_margin(PROMPT_PADDING)
                                    .show(&mut surface, |body| document_prompt_content(body, None, &state.message, &state.name, false, &layout));
                                state.frame = content.response.rect;
                            } else {
                                // Match the actual Modal path, including its initially narrow
                                // Area. A generously sized preallocated Frame hid this bug.
                                let modal = confirmation_modal()
                                    .area(egui::Modal::default_area(egui::Id::new("discard-prompt")).default_width(84.0))
                                    .frame(egui::Frame::popup(ui.style()).inner_margin(PROMPT_PADDING))
                                    .show(ui.ctx(), |body| document_prompt_content(body, Some("Unsaved changes"), &state.message, &state.name, false, &layout));
                                state.frame = modal.response.rect;
                            }
                        },
                        Fixture { ready: false, native, name, message, frame: egui::Rect::NOTHING },
                    );
                    PhotocraftApp::setup_context(&h.ctx, theme);
                    h.state_mut().ready = true;
                    h.run_steps(5);
                    let frame = h.state().frame;
                    let buttons = [mnemonic("Yes", Key::Y), mnemonic("No", Key::N), tl!("Cancel").into()].map(|label| h.get_by_label(&label).rect());
                    assert!(frame.width() >= 368.0 && frame.width() <= 570.0, "{theme:?}/{language}/{native}: {frame:?}");
                    assert!(frame.height() < 300.0, "the filename must not become a tall wall of text: {frame:?}");
                    assert!(buttons.iter().all(|button| frame.contains_rect(*button)));
                    assert!((buttons[0].center().y - buttons[2].center().y).abs() < 1.0, "all actions must stay in one row");
                    assert!(buttons.windows(2).all(|pair| pair[0].right() < pair[1].left()));
                }
            });
        }
    }
}
