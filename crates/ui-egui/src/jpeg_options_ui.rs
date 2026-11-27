//! JPEG quality prompt used by the desktop save flow. The root handles the
//! answer after the native dialog has released keyboard and mouse input.

use egui::{Key, Modifiers, Ui};
use photocraft_doc::DocId;

use crate::PhotocraftApp;
use crate::widgets::{ButtonRole, DialogButton};

pub(crate) const DIALOG_KEY: &str = "jpeg-options";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Prompt {
    pub document: DocId,
    pub path: String,
    pub as_copy: bool,
    pub quality: u8,
}

impl Prompt {
    pub(crate) fn new(document: DocId, path: String, as_copy: bool, quality: u8) -> Self {
        Self { document, path, as_copy, quality: quality.clamp(1, 100) }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Save(Prompt),
    Cancel,
}

pub(crate) fn is_jpeg_path(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.')
        .is_some_and(|(stem, extension)| !stem.is_empty() && photocraft_codecs::from_extension(extension) == Some(photocraft_codecs::Format::Jpeg))
}

/// Render and take the pending prompt when it is answered. The caller owns the
/// write, path adoption and any outstanding close confirmation.
pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) -> Option<Answer> {
    if ctx.viewport_id() != egui::ViewportId::ROOT || app.jpeg_options.is_none() {
        return None;
    }
    let title = tl!("JPEG Options");
    let native = !ctx.embed_viewports();
    let choice = if native {
        let mut choice = None;
        let closed = crate::dialogs::show_auxiliary_viewport(app, ctx, DIALOG_KEY, title, egui::vec2(420.0, 200.0), |app, ui| {
            if let Some(prompt) = &mut app.jpeg_options {
                choice =
                    egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style()).inner_margin(20.0)).show(ui, |ui| contents(ui, prompt)).inner;
            }
        });
        if closed { Some(false) } else { choice }
    } else {
        let prompt = app.jpeg_options.as_mut()?;
        let modal = egui::Modal::new(egui::Id::new(DIALOG_KEY)).frame(egui::Frame::popup(&ctx.global_style()).inner_margin(20.0)).show(ctx, |ui| {
            ui.set_width(340.0);
            ui.label(egui::RichText::new(title).strong());
            ui.add_space(8.0);
            contents(ui, prompt)
        });
        if modal.should_close() { Some(false) } else { modal.inner }
    };
    let confirm = choice?;
    let prompt = app.jpeg_options.take()?;
    if native {
        crate::dialogs::finish_auxiliary(app, ctx, DIALOG_KEY);
    }
    Some(if confirm { Answer::Save(prompt) } else { Answer::Cancel })
}

fn contents(ui: &mut Ui, prompt: &mut Prompt) -> Option<bool> {
    let name = prompt.path.rsplit(['/', '\\']).next().unwrap_or(&prompt.path);
    ui.add(egui::Label::new(name).truncate()).on_hover_text(&prompt.path);
    ui.add_space(10.0);
    let mut quality = f32::from(prompt.quality);
    crate::widgets::slider_row(ui, "Quality", &mut quality, 1.0..=100.0, "", None);
    prompt.quality = quality.round().clamp(1.0, 100.0) as u8;
    ui.add_space(12.0);
    let buttons = [DialogButton::new(ButtonRole::Default, tl!("OK"), 84.0), DialogButton::new(ButtonRole::Cancel, tl!("Cancel"), 84.0)];
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let clicked = crate::widgets::dialog_buttons(ui, &buttons);
        let enter = ui.is_enabled() && !ui.ctx().text_edit_focused() && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Enter));
        match clicked {
            Some(ButtonRole::Cancel) => Some(false),
            Some(ButtonRole::Default) => Some(true),
            _ if enter => Some(true),
            _ => None,
        }
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;

    fn pending() -> (PhotocraftApp, DocId) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width":32,"height":24})).unwrap();
        let id = app.session.active().unwrap().doc.id;
        app.jpeg_options = Some(Prompt::new(id, "example.jpg".to_string(), true, 85));
        (app, id)
    }

    #[test]
    fn jpeg_file_names_match_the_encoder() {
        for name in ["output.jpg", "dir/output.JPEG", "C:\\photos\\output.jpe", "output.JFIF"] {
            assert!(is_jpeg_path(name), "{name}");
        }
        for name in ["jpg", "output.jpg.png", "dir.jpg/output", "output.png", ".jpeg"] {
            assert!(!is_jpeg_path(name), "{name}");
        }
    }

    #[test]
    fn quality_dialog_accepts_slider_and_typed_value_for_parked_document() {
        let _language = crate::i18n::language_scope(crate::i18n::Lang::EN);
        let (app, id) = pending();
        let mut ui = Harness::builder().with_size(egui::vec2(700.0, 450.0)).build_ui_state(
            |ui, (app, answer): &mut (PhotocraftApp, Option<Answer>)| {
                if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                if let Some(result) = show(app, ui.ctx()) {
                    *answer = Some(result);
                }
            },
            (app, None),
        );
        PhotocraftApp::setup_context(&ui.ctx, crate::theme::ThemeKind::StudioLight);
        ui.run_steps(4);
        assert!(ui.state().1.is_none());
        let label = ui.get_by_label("Quality").rect();
        let track = egui::pos2(label.left() + 70.0, label.bottom() + 14.0);
        ui.hover_at(track);
        ui.run_steps(1);
        ui.drag_at(track);
        ui.run_steps(1);
        ui.drop_at(track);
        ui.run_steps(3);
        let quality = ui.state().0.jpeg_options.as_ref().unwrap().quality;
        assert!((1..50).contains(&quality), "slider result {quality}");

        ui.query_all_by_role(Role::SpinButton).find(|node| node.value().as_deref() == Some(quality.to_string().as_str())).unwrap().click();
        ui.run_steps(1);
        ui.event(egui::Event::Text("64".into()));
        ui.run_steps(2);
        ui.get_by_label("OK").click();
        ui.run_steps(3);
        assert_eq!(ui.state().1, Some(Answer::Save(Prompt::new(id, "example.jpg".to_string(), true, 64))));
        assert!(ui.state().0.jpeg_options.is_none());
    }

    #[test]
    fn escape_cancels_embedded_prompt_without_save_answer() {
        let (mut app, _) = pending();
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::StudioLight);
        let mut answer = None;
        for events in [vec![], vec![], vec![key(Key::Escape)]] {
            let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                answer = show(&mut app, ui.ctx());
            });
            output.textures_delta.clear();
        }
        assert_eq!(answer, Some(Answer::Cancel));
        assert!(app.jpeg_options.is_none());
    }

    fn key(key: Key) -> egui::Event {
        egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    #[test]
    fn jpeg_native_dialog_supersedes_save_confirmation_and_returns_answer_to_root() {
        std::thread::spawn(|| {
            let (mut app, id) = pending();
            app.session.execute("layer.new.layer", json!({})).unwrap();
            assert!(crate::discard_ui::intercept(&mut app, "file.close", &serde_json::Value::Null));
            let ctx = egui::Context::default();
            PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::StudioLight);
            ctx.set_embed_viewports(false);
            let viewport_id = crate::dialogs::native_dialog_viewport(&app).unwrap();
            egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
                assert_eq!(viewport.ids.this, viewport_id);
                let size = viewport.builder.inner_size.unwrap_or(egui::vec2(420.0, 200.0));
                let mut input = egui::RawInput {
                    viewport_id,
                    events: vec![key(Key::Enter)],
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                };
                input.viewports.entry(viewport_id).or_default().focused = Some(true);
                let mut output = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
                output.textures_delta.clear();
            });
            let mut answer = None;
            let mut output = ctx.run_ui(Default::default(), |ui| {
                answer = show(&mut app, ui.ctx());
            });
            output.textures_delta.clear();
            assert_eq!(answer, Some(Answer::Save(Prompt::new(id, "example.jpg".to_string(), true, 85))));
            assert!(app.jpeg_options.is_none() && app.discard.is_some());
            let native = &output.viewport_output[&viewport_id];
            assert!(native.commands.contains(&egui::ViewportCommand::Visible(false)));
        })
        .join()
        .unwrap();
    }
}
