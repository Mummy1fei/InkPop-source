//! Reuse completed live pixels only after the ordinary command's complete identity matches.
use super::*;
use photocraft_doc::{DocId, Document, LayerId};
use std::sync::Arc;

pub(super) const MAX_POINTS: usize = 1_000_000;

pub(super) struct Guard {
    pub(super) command: String,
    source: Arc<Document>,
    doc_id: DocId,
    revision: u64,
    layer: Option<LayerId>,
    brush: BrushSettings,
    background: [f32; 4],
    params: Value,
    symmetry: Option<Vec<[[f64; 2]; 2]>>,
    restrict: Option<usize>,
}

fn options(p: &Value) -> Value {
    match p.as_object() {
        // Do not clone the possibly long point array only to discard it.
        Some(p) => Value::Object(p.iter().filter(|(k, _)| !matches!(k.as_str(), "points" | "layer")).map(|(k, v)| (k.clone(), v.clone())).collect()),
        None => p.clone(),
    }
}

impl Guard {
    pub(super) fn new(s: &Session, cmd: &str, p: &Value, brush: &BrushSettings) -> Result<Self> {
        let active = s.active().ok_or(EngineError::NoDocument)?;
        let mut params = options(p);
        params["seed"] = json!(brush.seed);
        Ok(Self {
            command: cmd.into(),
            source: active.doc.clone(),
            doc_id: active.doc.id,
            revision: active.revision,
            layer: if crate::channel_cmds::is_channel_target(p) { None } else { Some(layer_id(s, p)?) },
            brush: brush.clone(),
            background: s.tools.background,
            params,
            symmetry: active.symmetry_path.as_ref().map(|axis| axis.segments.clone()),
            restrict: crate::channel_cmds::color_restriction(s, cmd, p),
        })
    }

    pub(super) fn matches(&self, s: &Session, cmd: &str, p: &Value) -> bool {
        if self.command != cmd
            || self.params != options(p)
            || self.background != s.tools.background
            || self.restrict != crate::channel_cmds::color_restriction(s, cmd, p)
            || !s.active().is_some_and(|active| {
                active.doc.id == self.doc_id
                    && active.revision == self.revision
                    && Arc::ptr_eq(&active.doc, &self.source)
                    && active.symmetry_path.as_ref().map(|axis| &axis.segments) == self.symmetry.as_ref()
            })
            || (if crate::channel_cmds::is_channel_target(p) { None } else { layer_id(s, p).ok() }) != self.layer
        {
            return false;
        }
        let brush = if cmd == "paint.pencil" { pencil_brush(s, p) } else { resolve_brush(s, p, cmd).map(|brush| with_blend_mode(brush, p)) };
        brush.is_ok_and(|brush| brush == self.brush)
    }
}

pub(crate) struct PreparedStroke {
    guard: Guard,
    points: Vec<StrokePoint>,
    surface: Surface,
    damage: Rect,
}

impl PreparedStroke {
    pub(super) fn new(guard: Guard, points: Vec<StrokePoint>, surface: Surface, damage: Rect) -> Self {
        Self { guard, points, surface, damage }
    }
}

/// A mismatch consumes the handoff and returns None: the caller executes its exact old renderer.
pub(super) fn commit(s: &mut Session, cmd: &str, p: &Value, points: &[StrokePoint], label: &str) -> Option<Result<Value>> {
    let prepared = s.prepared_stroke.take()?;
    if points != prepared.points || !prepared.guard.matches(s, cmd, p) {
        return None;
    }
    Some((|| {
        let damage = prepared.damage;
        s.edit(label, |doc, _| {
            let (surface, _) = crate::channel_cmds::target_surface(doc, prepared.guard.layer, p)?;
            *surface = prepared.surface;
            Ok(())
        })?;
        Ok(damage_json(s, damage))
    })())
}

#[cfg(test)]
mod tests;
