//! "Rasterize?" prompt: painting the pixels of a type, shape, Smart Object or fill layer with a
//! pixel tool asks first, as Photoshop does ("This type layer must be rasterized before
//! proceeding. Its text will no longer be editable. Rasterize the type?"), instead of failing
//! with a status-bar message.
//!
//! The prompt is an ordinary dialog in the UI state (`ui.inspect` shows it, `ui.dialog.confirm`
//! and `ui.dialog.cancel` answer it). OK only runs the matching `layer.rasterize.*` command.
//! The triggering gesture is consumed; painting requires a new press on the canvas.
//! Cancel does nothing.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};

const MARK: &str = "__rasterize";

/// A layer kind that must be rasterized before its pixels can be painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Type,
    Shape,
    SmartObject,
    Fill,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Type, Kind::Shape, Kind::SmartObject, Kind::Fill];

    fn key(self) -> &'static str {
        match self {
            Kind::Type => "type",
            Kind::Shape => "shape",
            Kind::SmartObject => "smartObject",
            Kind::Fill => "fill",
        }
    }

    /// The command that rasterizes it.
    pub fn command(self) -> &'static str {
        match self {
            Kind::Type => "layer.rasterize.type",
            Kind::Shape => "layer.rasterize.shape",
            Kind::SmartObject => "layer.rasterize.smartObject",
            Kind::Fill => "layer.rasterize.fillContent",
        }
    }

    /// Photoshop's question.
    pub fn message(self) -> &'static str {
        match self {
            Kind::Type => "This type layer must be rasterized before proceeding. Its text will no longer be editable. Rasterize the type?",
            Kind::Shape => "This shape layer must be rasterized before proceeding. It will no longer be editable as a shape. Rasterize the shape?",
            Kind::SmartObject => "This Smart Object must be rasterized before proceeding. Its contents will no longer be editable. Rasterize the Smart Object?",
            Kind::Fill => "This fill layer must be rasterized before proceeding. Its fill content will no longer be editable. Rasterize the layer?",
        }
    }

    fn from_key(k: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|x| x.key() == k)
    }
}

/// Does `tool` paint the layer's pixels (so a vector or Smart Object layer must be rasterized)?
pub fn paints_pixels(app: &PhotocraftApp, tool: Tool) -> bool {
    tool.is_brushlike() || matches!(tool, Tool::PaintBucket | Tool::MagicEraser) || (tool == Tool::Gradient && app.ui.tool_options.gradient_classic)
}

/// The active layer's kind when painting its pixels needs rasterizing first: the tool targets
/// the layer's pixels (not its mask, an alpha channel or the Quick Mask) and the layer is a
/// type, shape, Smart Object or fill layer that isn't locked.
pub fn needed(app: &PhotocraftApp) -> Option<(Kind, photocraft_doc::LayerId)> {
    if crate::canvas::paint_target(app) != json!("pixels") {
        return None;
    }
    let st = app.session.active()?;
    let id = st.active_layer?;
    let l = st.doc.layer(id)?;
    let locks = st.doc.effective_locks(id);
    if locks.all || locks.pixels {
        return None;
    }
    use photocraft_doc::LayerContent as C;
    let kind = match &l.content {
        C::Text(_) => Kind::Type,
        C::Shape(_) => Kind::Shape,
        C::Smart(_) => Kind::SmartObject,
        C::Fill(_) => Kind::Fill,
        _ => return None,
    };
    Some((kind, id))
}

/// Consume a tool's pointer press when its layer must be rasterized first.
/// The prompt confirms the conversion only; the original press is never replayed.
pub fn intercept(app: &mut PhotocraftApp, tool: Tool, x: f64, y: f64, pressure: f32) -> bool {
    if !paints_pixels(app, tool) {
        return false;
    }
    let Some((kind, layer)) = needed(app) else { return false };
    if !app.ui.dialogs.iter().any(|d| owns(&d.fields)) {
        open(app, kind, layer, tool, [x, y, f64::from(pressure)]);
    }
    true
}

/// Open the prompt for `layer`. `tool` and `at` retain input provenance for the
/// control protocol; confirming must not turn them into a deferred paint operation.
pub fn open(app: &mut PhotocraftApp, kind: Kind, layer: photocraft_doc::LayerId, tool: Tool, at: [f64; 3]) -> u64 {
    let mut f = Map::new();
    f.insert(MARK.into(), json!(kind.key()));
    f.insert("__command".into(), json!(kind.command()));
    f.insert("__label".into(), json!(photocraft_engine::build_info::CUSTOM_PRODUCT_NAME));
    f.insert("message".into(), json!(kind.message()));
    f.insert("layer".into(), json!(layer.0));
    f.insert("tool".into(), json!(format!("{tool:?}")));
    f.insert("at".into(), json!(at));
    app.ui.open_dialog(DialogKind::Command, f)
}

/// Is this dialog the prompt?
pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.contains_key(MARK)
}

/// The prompt's body: the question, with a caution mark.
pub fn body(ui: &mut egui::Ui, f: &Map<String, Value>) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let text = f.get("message").and_then(Value::as_str).unwrap_or("This layer must be rasterized before proceeding.");
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;
        let (r, _) = ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
        crate::icons::paint(ui, r, "triangle-alert", 30.0, t.warning);
        ui.add(egui::Label::new(egui::RichText::new(tl!(text)).color(t.text).size(13.0)).wrap());
    });
}

/// OK converts the requested layer only. A fresh canvas press owns any later painting.
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let kind = f.get(MARK).and_then(Value::as_str).and_then(Kind::from_key).ok_or("not a rasterize prompt")?;
    let layer = f.get("layer").and_then(Value::as_u64).ok_or("the prompt has no layer")?;
    let r = app.run(kind.command(), json!({ "layer": layer }))?;
    Ok(json!({ "rasterized": r, "painted": false }))
}

#[cfg(test)]
#[path = "rasterize_prompt_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "rasterize_confirmation_tests_v89.rs"]
mod confirmation_tests_v89;
