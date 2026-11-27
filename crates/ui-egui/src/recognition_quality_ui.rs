//! One saved recognition-quality choice for menu selections and Raw masks.
use photocraft_engine::prefs::ImageRecognitionQuality;

pub(crate) fn selector(ui: &mut egui::Ui, quality: &mut ImageRecognitionQuality) -> bool {
    ui.label(tl!("Subject Quality:"));
    crate::widgets::dropdown(
        ui,
        "subject-recognition-quality",
        quality,
        &[(ImageRecognitionQuality::Fast, tl!("Fast")), (ImageRecognitionQuality::Precise, tl!("Precise (CPU)"))],
        112.0,
    )
}

pub(crate) fn toolbar(ui: &mut egui::Ui, app: &mut crate::PhotocraftApp) {
    let mut quality = app.session.prefs().performance.subject_recognition_quality;
    if selector(ui, &mut quality) {
        let _ = app.run("prefs.set", serde_json::json!({"path":"performance.subjectRecognitionQuality","value":quality.name()}));
    }
}
