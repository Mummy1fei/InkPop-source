use super::*;
use egui_kittest::{Harness, kittest::Queryable};

fn painted_text(shape: &egui::Shape, label: &str) -> bool {
    match shape {
        egui::Shape::Text(text) => text.galley.text() == label,
        egui::Shape::Vec(shapes) => shapes.iter().any(|shape| painted_text(shape, label)),
        _ => false,
    }
}

#[test]
fn layer_effect_rows_show_and_announce_localized_labels() {
    let mut layer = Layer::raster("Image", photocraft_color::PixelFormat::RGBA8);
    for (kind, _) in crate::layer_style::KINDS {
        layer.effects.items.push(photocraft_engine::layer_style::effect_from_params(kind, &json!({})).unwrap());
    }
    let mut h = Harness::builder().with_size(vec2(320., 420.)).build_ui_state(
        move |ui, (app, lang): &mut (PhotocraftApp, crate::i18n::Lang)| {
            crate::i18n::with_language(*lang, || effect_rows(app, ui, &layer, 0));
        },
        (PhotocraftApp::new(photocraft_engine::Session::new(), Default::default()), crate::i18n::Lang::EN),
    );
    PhotocraftApp::setup_context(&h.ctx, theme::ThemeKind::StudioLight);

    for (language, expected) in [
        (
            "en",
            [
                "Effects",
                "Bevel & Emboss",
                "Stroke",
                "Inner Shadow",
                "Inner Glow",
                "Satin",
                "Color Overlay",
                "Gradient Overlay",
                "Pattern Overlay",
                "Outer Glow",
                "Drop Shadow",
            ],
        ),
        ("zh-hans", ["效果", "斜面和浮雕", "描边", "内阴影", "内发光", "光泽", "颜色叠加", "渐变叠加", "图案叠加", "外发光", "投影"]),
        ("zh-hant", ["效果", "斜角和浮雕", "筆畫", "內陰影", "內光暈", "緞面", "顏色覆蓋", "漸層覆蓋", "圖樣覆蓋", "外光暈", "陰影"]),
    ] {
        h.state_mut().1 = crate::i18n::Lang::from_code(language).unwrap();
        h.run_steps(3);
        for label in expected {
            assert!(h.output().shapes.iter().any(|shape| painted_text(&shape.shape, label)), "{language}: {label} must be drawn");
            assert!(h.query_by_label(label).is_some(), "{language}: {label} must be accessible");
        }
    }
}
