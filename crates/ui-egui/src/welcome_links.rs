//! Welcome copyright and the About window's public contact links.

use crate::PhotocraftApp;

pub(crate) const COPYRIGHT: &str = "©2026 Mummy-fei · InkPop";
pub(crate) const EMAIL: &str = "mayunfei1991@live.cn";
pub(crate) const EMAIL_URL: &str = "mailto:mayunfei1991@live.cn";
pub(crate) const WEIBO: &str = "https://weibo.com/u/6854236416";
pub(crate) const XIAOHONGSHU: &str = "https://www.xiaohongshu.com/user/profile/65a5deeb000000000203e1bf";

pub(crate) fn show(ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.vertical_centered(|ui| {
        ui.add(egui::Label::new(egui::RichText::new(COPYRIGHT).size(12.0).color(t.text_dim)).wrap());
    });
}

/// Only explicit clicks invoke the platform URL service. Rendering never opens
/// a browser or sends mail; the email link opens the user's mail composer.
pub(crate) fn contacts(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let font = egui::FontId::proportional(12.0);
    let contacts = [(tl!("Xiaohongshu"), XIAOHONGSHU), (tl!("Weibo"), WEIBO), (EMAIL, EMAIL_URL)];
    let gap = 16.0;
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (label, url) in contacts {
            let hint = if url == EMAIL_URL { format!("{}: {EMAIL}", tl!("Email")) } else { url.to_string() };
            if ui.link(egui::RichText::new(label).font(font.clone()).color(t.text_dim).underline()).on_hover_text(hint).clicked() {
                clicked = Some(url);
            }
        }
    });
    if let Some(url) = clicked {
        crate::links::open(app, ui.ctx(), url);
    }
}

#[cfg(test)]
#[path = "welcome_links_tests_v70.rs"]
mod tests;
