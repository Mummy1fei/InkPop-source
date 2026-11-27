//! The InkPop brand mark at the left of the title bar: the user's supplied logo
//! (`assets/app-icon/inkpop/`, see its provenance). The PNG retains its white rounded tile and
//! transparent corners; it is decoded once per context into a mipmapped texture, so it stays
//! crisp at the title bar's size on any display scale.

use egui::{Color32, Context, Id, Rect, TextureHandle, TextureOptions, Ui, pos2};
use photocraft_codecs::{ChannelLayout, SampleType};

/// 128 px: sharp at 20 pt on a 3× display, and the mipmaps keep it clean at 1×.
const ICON_PNG: &[u8] = include_bytes!("../../../assets/app-icon/inkpop/inkpop-128.png");

/// The icon's pixels (one transparent pixel if it couldn't be decoded, which a test rules out).
fn decode() -> egui::ColorImage {
    let rgba = photocraft_codecs::decode(ICON_PNG).ok().map(|img| img.convert(ChannelLayout::Rgba, SampleType::U8));
    match rgba {
        Some(img) if img.data().len() == img.width() as usize * img.height() as usize * 4 => {
            egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.data())
        }
        _ => egui::ColorImage::filled([1, 1], Color32::TRANSPARENT),
    }
}

/// The icon texture, uploaded on first use and kept in the context.
fn texture(ctx: &Context) -> TextureHandle {
    let id = Id::new("photocraft-brand-mark");
    if let Some(tex) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return tex;
    }
    let options = TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..TextureOptions::LINEAR };
    let tex = ctx.load_texture("photocraft-brand-mark", decode(), options);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// Where the mark was painted this frame (tests and screenshots).
fn rect_id() -> Id {
    Id::new("photocraft-brand-mark-rect")
}

/// Paint the mark into `r` (square).
pub fn paint_mark(ui: &Ui, r: Rect) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    ui.painter().image(texture(ui.ctx()).id(), r, uv, Color32::WHITE);
    ui.ctx().data_mut(|d| d.insert_temp(rect_id(), r));
}

/// The rect the mark was last painted into.
#[cfg(test)]
pub fn mark_rect(ctx: &Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp(rect_id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inkpop_logo_decodes_to_the_supplied_rounded_128_px_tile_v73() {
        let img = decode();
        assert_eq!(img.size, [128, 128]);
        // Rounded corners: the corner pixel is transparent, the centre opaque.
        assert_eq!(img.pixels[0].a(), 0);
        assert_eq!(img.pixels[64 * 128 + 64].a(), 255);
        assert_eq!(img.pixels[64 * 128 + 64], Color32::from_rgb(176, 106, 235), "fingerprint of the user's gradient logo, not the upstream icon");
    }

    #[test]
    fn the_title_bar_paints_the_inkpop_logo_from_one_cached_texture_v73() {
        let ctx = Context::default();
        crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut frame = || {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::panels::title_bar(&mut app, ui));
            let uploads = out.textures_delta.set.values().flat_map(|d| d.iter()).filter(|d| d.image.size() == [128, 128]).count();
            out.textures_delta.clear();
            uploads
        };
        assert_eq!(frame(), 1, "uploaded on first use");
        assert_eq!(frame(), 0, "then reused");
        assert!(mark_rect(&ctx).is_some());
    }

    #[test]
    #[ignore = "writes five real title-bar theme screenshots for root logo review"]
    fn brand_gpu_visual_v73() {
        use crate::theme::ThemeKind;
        struct TitleBar(crate::PhotocraftApp);
        impl eframe::App for TitleBar {
            fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
                ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, crate::theme::Tokens::get(ui.ctx()).canvas);
                crate::panels::title_bar(&mut self.0, ui);
            }
        }
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v73/brand-gpu");
        assert!(!out.exists(), "preserve previous brand visual evidence");
        std::fs::create_dir_all(&out).unwrap();
        for kind in ThemeKind::ALL {
            let mut h = egui_kittest::Harness::builder()
                .with_size(egui::vec2(1400.0, 96.0))
                .with_pixels_per_point(2.0)
                .with_max_steps(64)
                .wgpu_setup(crate::gpu_canvas::wgpu_setup())
                .build_eframe(move |cc| {
                    crate::PhotocraftApp::setup_context(&cc.egui_ctx, kind);
                    if kind == ThemeKind::StudioLight {
                        let colors = photocraft_engine::prefs::palette::StudioLightColors {
                            colors: photocraft_engine::prefs::palette::builtin::BUILTIN_PALETTES[0].colors(),
                            presets: Vec::new(),
                        };
                        crate::theme::palette::apply(&cc.egui_ctx, kind, &colors);
                    }
                    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                    app.custom_titlebar = true;
                    TitleBar(app)
                });
            h.run_steps(4);
            let mark = mark_rect(&h.ctx).expect("production title bar paints the logo");
            assert!((18.0..=20.0).contains(&mark.width()) && mark.width() == mark.height());
            h.render().unwrap().save(out.join(format!("{}.png", kind.id()))).unwrap();
        }
    }
}
