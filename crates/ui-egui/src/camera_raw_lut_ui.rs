//! Camera Raw LUT import uses the platform Services picker; recipes embed the table.
use crate::{PhotocraftApp, widgets};
use photocraft_algo::camera_raw_lut::{ColorLookup, MAX_SIDE, sample};
use serde_json::{Value, json};
use std::sync::Arc;

const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;

// CMS's general creative-LUT 1D expansion deliberately clamps generated looks.
// Raw is float-aware: expand a true 1D Cube here without clipping its outputs.
fn cube_1d(bytes: &[u8]) -> Result<Option<(photocraft_cms::lutfile::LutFile, bool)>, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut size = None;
    let mut has_3d = false;
    for line in text.lines() {
        let mut words = line.split('#').next().unwrap_or("").split_whitespace();
        match words.next() {
            Some("LUT_1D_SIZE") => {
                if size.is_some() {
                    return Err("Duplicate LUT_1D_SIZE".into());
                }
                let count =
                    words.next().and_then(|v| v.parse::<usize>().ok()).filter(|v| (2..=65_536).contains(v)).ok_or("LUT_1D_SIZE must be between 2 and 65536")?;
                size = Some(count);
            }
            Some("LUT_3D_SIZE") => has_3d = true,
            _ => {}
        }
    }
    let Some(size) = size else {
        return Ok(None);
    };
    if has_3d {
        return Err("Combined 1D shaper and 3D Cube LUTs are not supported".into());
    }
    let mut rows = Vec::with_capacity(size * 3);
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some(first) = line.split_whitespace().next() else {
            continue;
        };
        if first.parse::<f32>().is_ok() {
            let values: Result<Vec<f32>, _> = line.split_whitespace().map(str::parse::<f32>).collect();
            let values = values.map_err(|_| "Invalid 1D LUT data row")?;
            if values.len() != 3 || values.iter().any(|v| !v.is_finite()) || rows.len() >= size * 3 {
                return Err("Invalid or excessive 1D LUT data".into());
            }
            rows.extend(values);
        }
    }
    if rows.len() != size * 3 {
        return Err("Incomplete 1D LUT table".into());
    }
    let curve = |channel: usize, value: f32| {
        let position = value * (size - 1) as f32;
        let index = (position.floor() as usize).min(size - 2);
        let fraction = position - index as f32;
        let a = rows.get(index * 3 + channel).copied().unwrap_or(0.0);
        let b = rows.get((index + 1) * 3 + channel).copied().unwrap_or(0.0);
        a * (1.0 - fraction) + b * fraction
    };
    let mut data = Vec::with_capacity(MAX_SIDE.pow(3) * 3);
    for b in 0..MAX_SIDE {
        for g in 0..MAX_SIDE {
            for r in 0..MAX_SIDE {
                data.extend([
                    curve(0, r as f32 / (MAX_SIDE - 1) as f32),
                    curve(1, g as f32 / (MAX_SIDE - 1) as f32),
                    curve(2, b as f32 / (MAX_SIDE - 1) as f32),
                ]);
            }
        }
    }
    Ok(Some((photocraft_cms::lutfile::LutFile { title: String::new(), size: MAX_SIDE, data }, size > MAX_SIDE)))
}

pub(super) fn parse(name: &str, bytes: &[u8]) -> Result<(ColorLookup, bool), String> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err("LUT file exceeds 64 MiB".into());
    }
    let cube_format = !name.to_ascii_lowercase().ends_with(".3dl") && !name.to_ascii_lowercase().ends_with(".look");
    let one_dimensional = if cube_format { cube_1d(bytes)? } else { None };
    let (parsed, adapted_1d) = match one_dimensional {
        Some((table, adapted)) => (table, adapted),
        None => (photocraft_cms::lutfile::parse(name, bytes).map_err(|e| e.to_string())?, false),
    };
    let mut lut = ColorLookup {
        name: name.rsplit(['/', '\\']).next().unwrap_or(name).chars().filter(|c| !c.is_control()).take(80).collect(),
        enabled: true,
        size: parsed.size.min(MAX_SIDE),
        ..Default::default()
    };
    // The CMS file loader returns a normalized table; retain the Cube input domain explicitly.
    if !name.to_ascii_lowercase().ends_with(".3dl") && !name.to_ascii_lowercase().ends_with(".look") {
        for line in String::from_utf8_lossy(bytes).lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            let mut words = line.split_whitespace();
            let key = words.next().unwrap_or("");
            if matches!(key, "DOMAIN_MIN" | "DOMAIN_MAX" | "LUT_3D_INPUT_RANGE" | "LUT_1D_INPUT_RANGE") {
                let values: Result<Vec<f32>, _> = words.map(str::parse).collect();
                let values = values.map_err(|_| "Invalid LUT input domain")?;
                match (key, values.as_slice()) {
                    ("DOMAIN_MIN", [r, g, b]) => lut.domain_min = [*r, *g, *b],
                    ("DOMAIN_MAX", [r, g, b]) => lut.domain_max = [*r, *g, *b],
                    ("LUT_3D_INPUT_RANGE" | "LUT_1D_INPUT_RANGE", [min, max]) => {
                        lut.domain_min = [*min; 3];
                        lut.domain_max = [*max; 3];
                    }
                    _ => return Err("Invalid LUT input domain".into()),
                }
            }
        }
    }
    let adapted = parsed.size > MAX_SIDE || adapted_1d;
    lut.values = Arc::new(if adapted {
        let mut table = Vec::with_capacity(MAX_SIDE.pow(3) * 3);
        for b in 0..MAX_SIDE {
            for g in 0..MAX_SIDE {
                for r in 0..MAX_SIDE {
                    table.extend(sample(
                        &parsed.data,
                        parsed.size,
                        [r as f32 / (MAX_SIDE - 1) as f32, g as f32 / (MAX_SIDE - 1) as f32, b as f32 / (MAX_SIDE - 1) as f32],
                        true,
                    ));
                }
            }
        }
        table
    } else {
        parsed.data
    });
    // Name budget is bytes, rather than Unicode scalar count.
    while lut.name.len() > 256 {
        lut.name.pop();
    }
    lut.validate()?;
    Ok((lut, adapted))
}

pub(super) fn panel(ui: &mut egui::Ui, lut: &mut ColorLookup, dirty: &mut bool, message: Option<&str>) -> bool {
    let mut import = false;
    ui.horizontal(|ui| {
        if ui.button(tl!("Import LUT…")).clicked() {
            import = true;
        }
        if ui.add_enabled(lut.size > 0, egui::Button::new(tl!("Remove LUT"))).clicked() {
            *lut = Default::default();
            *dirty = true;
        }
    });
    if lut.size > 0 {
        ui.label(format!("{} · {}³", lut.name, lut.size));
        *dirty |= super::square_checkbox(ui, &mut lut.enabled, tl!("Enable LUT")).changed();
        *dirty |= widgets::slider_row(ui, tl!("LUT strength"), &mut lut.amount, 0.0..=100.0, "%", None).changed();
        ui.horizontal(|ui| {
            ui.label(tl!("LUT interpolation"));
            *dirty |= widgets::dropdown(ui, "raw-lut-interpolation", &mut lut.tetrahedral, &[(true, tl!("Tetrahedral")), (false, tl!("Trilinear"))], 115.0);
        });
    } else {
        ui.label(tl!("No LUT loaded"));
    }
    ui.small(tl!("LUTs use the working RGB values; the document profile is retained."));
    if let Some(message) = message {
        ui.label(message);
    }
    import
}

pub(super) fn execute(app: &mut PhotocraftApp, ctx: &egui::Context, request: Option<&Value>) -> Result<Value, String> {
    let (name, bytes) = if let Some(request) = request {
        let object = request.as_object().ok_or("LUT import must be an object")?;
        if object.keys().any(|key| !matches!(key.as_str(), "fileName" | "data")) {
            return Err("Unknown LUT import property".into());
        }
        let name = request.get("fileName").and_then(Value::as_str).ok_or("LUT fileName is required")?;
        let data = request.get("data").and_then(Value::as_str).ok_or("LUT data is required")?;
        if data.len() > MAX_IMPORT_BYTES {
            return Err("LUT file exceeds 64 MiB".into());
        }
        (name.to_string(), data.as_bytes().to_vec())
    } else {
        let picker = app.services.pick_lut.as_mut().ok_or("LUT file picker unavailable")?;
        let Some((name, bytes)) = picker() else {
            return Ok(json!({"cancelled":true}));
        };
        (name, bytes?)
    };
    let (lut, adapted) = parse(&name, &bytes)?;
    let d = app.camera_raw.as_mut().ok_or("Camera Raw is closed")?;
    d.params.color_lookup = lut;
    d.lut_message = adapted.then(|| tl!("Large LUT adapted to a 33³ grid").to_string());
    d.dirty = true;
    d.queue_render(ctx);
    Ok(json!({"name":d.params.color_lookup.name,"size":d.params.color_lookup.size,"adapted":adapted}))
}
