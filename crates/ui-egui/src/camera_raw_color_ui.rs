//! Working-space controls and an explicit, independently converted output copy.
use photocraft_algo::camera_raw_color::{ColorSpaces, RgbSpace};

pub(super) fn localize_error(error: String) -> String {
    if error.contains(photocraft_algo::camera_raw_color::HDR_TABLE_ERROR) {
        tl!("Table TRC profiles cannot preserve 32-bit HDR. Convert the source document to an analytic RGB profile before changing color spaces.").to_string()
    } else {
        error
    }
}

/// Output-copy settings are independent from the non-destructive filter parameters.
pub(super) struct OutputOptions {
    pub depth: u8,
    pub resolution: f64,
}

impl OutputOptions {
    pub fn from_prefs(defaults: &photocraft_engine::prefs::RawDefaults) -> Self {
        Self {
            depth: match defaults.bit_depth {
                photocraft_engine::prefs::RawDepth::Eight => 8,
                photocraft_engine::prefs::RawDepth::Sixteen => 16,
            },
            resolution: defaults.resolution,
        }
    }
}

pub(super) fn default_output_space(defaults: &photocraft_engine::prefs::RawDefaults) -> RgbSpace {
    use photocraft_engine::prefs::RawColorSpace;
    match defaults.color_space {
        RawColorSpace::Srgb => RgbSpace::Srgb,
        RawColorSpace::AdobeRgb => RgbSpace::AdobeRgb,
        RawColorSpace::ProPhoto => RgbSpace::ProPhotoRgb,
        RawColorSpace::DisplayP3 => RgbSpace::DisplayP3,
    }
}

#[derive(Default)]
pub(super) struct Geometry {
    pub working: Option<egui::Rect>,
    pub output: Option<egui::Rect>,
    pub create: Option<egui::Rect>,
}

pub(super) fn panel(ui: &mut egui::Ui, spaces: &mut ColorSpaces, dirty: &mut bool, geometry: &mut Geometry, output: &mut OutputOptions) -> bool {
    let options = [
        (RgbSpace::Document, tl!("Keep document color space")),
        (RgbSpace::Srgb, "sRGB"),
        (RgbSpace::DisplayP3, "Display P3"),
        (RgbSpace::AdobeRgb, tl!("Adobe RGB-compatible")),
        (RgbSpace::ProPhotoRgb, tl!("ProPhoto RGB-compatible")),
    ];
    ui.label(tl!("Working color space"));
    let row = ui.horizontal(|ui| {
        *dirty |= crate::widgets::dropdown(ui, "raw-working-space", &mut spaces.working, &options, ui.available_width().min(260.0));
    });
    geometry.working = Some(row.response.rect);
    ui.small(tl!("Working space is used for all Camera Raw adjustments and converted back to the document."));
    ui.add_space(4.0);
    ui.label(tl!("Output color space"));
    let row = ui.horizontal(|ui| {
        crate::widgets::dropdown(ui, "raw-output-space", &mut spaces.output, &options, ui.available_width().min(260.0));
    });
    geometry.output = Some(row.response.rect);
    egui::Grid::new("raw-output-format").num_columns(2).show(ui, |ui| {
        ui.label(tl!("Output bit depth"));
        crate::widgets::dropdown(ui, "raw-output-depth", &mut output.depth, &[(8, "8"), (16, "16")], 100.0);
        ui.end_row();
        ui.label(tl!("Resolution"));
        ui.add(egui::DragValue::new(&mut output.resolution).range(1.0..=10000.0).speed(1.0).suffix(" ppi"));
        ui.end_row();
    });
    ui.small(tl!("Output creates a merged document copy; the original is retained."));
    let create = ui.button(tl!("Create Output Copy"));
    geometry.create = Some(create.rect);
    create.clicked()
}
