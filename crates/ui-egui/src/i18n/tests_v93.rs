//! Explicit object-selection gesture text must not fall back to the misleading English hint.
use super::*;

const OBJECT_KEYS: &[&str] = &[
    "Foreground Stroke",
    "Click inside the object or drag a rectangle around it",
    "Draw an outline around the object; the outline is not foreground",
    "Paint inside the object; Shift adds, Alt subtracts",
    "Object clicks, lassos and foreground strokes require Smart Object Selection and its local model",
];

#[test]
fn every_complete_catalog_explains_object_selection_gestures_v93() {
    for language in Lang::all().filter(|language| language.complete_menus()) {
        for source in OBJECT_KEYS {
            assert!(has(language, source), "{} missing {source}", language.code());
            assert_ne!(tr(language, source), *source, "{} uses an English placeholder for {source}", language.code());
        }
        for source in ["Rectangle", "Lasso", "Mode:"] {
            // Some languages legitimately share the English spelling of these labels.
            assert!(has(language, source), "{} missing mode label {source}", language.code());
        }
    }
}

#[test]
fn chinese_object_modes_distinguish_enclosing_outlines_from_foreground_pixels_v93() {
    let simplified = Lang::from_code("zh-hans").expect("Simplified Chinese registered");
    let traditional = Lang::from_code("zh-hant").expect("Traditional Chinese registered");
    for (language, stroke, outline, error) in [
        (simplified, "前景涂划", "沿物体外围圈选；轮廓线不作为前景", "对象点选、套索和前景涂划需要启用智能对象选择，并安装本地模型"),
        (traditional, "前景塗畫", "沿物件外圍圈選；輪廓線不作為前景", "物件點選、套索及前景塗畫需要啟用智慧型物件選取，並安裝本機模型"),
    ] {
        assert_eq!(tr(language, "Rectangle"), "矩形");
        assert_eq!(tr(language, "Lasso"), "套索");
        assert_eq!(tr(language, OBJECT_KEYS[0]), stroke);
        assert_eq!(tr(language, OBJECT_KEYS[2]), outline);
        assert_eq!(tr(language, OBJECT_KEYS[4]), error);
        let paint = tr(language, OBJECT_KEYS[3]);
        assert!(paint.contains("Shift") && paint.contains("Alt"));
    }
}
