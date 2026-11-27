//! Fixed, optional crash checkpoints. This records CPU/UI progress, never GPU completion.
use crate::{DiagnosticEvent, DiagnosticOperation, DiagnosticStage, PhotocraftApp, state::TransformSession};
use photocraft_doc::{LayerContent, LayerId};

pub(crate) fn record(app: &mut PhotocraftApp, event: DiagnosticEvent) {
    let Some(callback) = app.services.diagnostic_event.as_ref() else { return };
    callback(event);
    app.diagnostic_frame_pending = Some(event);
}

pub(crate) fn source_ready(app: &mut PhotocraftApp, operation: DiagnosticOperation) {
    if app.services.diagnostic_event.is_none() {
        return;
    }
    let mut event = DiagnosticEvent::new(DiagnosticStage::PlaceSourceReady, app.ui.tool);
    event.operation = operation;
    event.success = Some(true);
    if let Some(state) = app.session.active() {
        event.target_size = Some([state.doc.size.width, state.doc.size.height]);
        if let Some(layer) = state.active_layer.and_then(|id| state.doc.layer(id)) {
            let bounds = photocraft_engine::transform_cmds::transform_bounds(&state.doc, layer);
            event.source_size = Some([bounds.width(), bounds.height()]);
            event.is_smart = Some(matches!(layer.content, LayerContent::Smart(_)));
        }
    }
    record(app, event);
}

pub(crate) fn transform(app: &PhotocraftApp, session: &TransformSession, stage: DiagnosticStage) -> DiagnosticEvent {
    let mut event = DiagnosticEvent::new(stage, app.ui.tool);
    event.operation = if session.made == Some(crate::state::MadeLayer::Place) { DiagnosticOperation::Place } else { DiagnosticOperation::Transform };
    let size = |points: &[[f64; 2]; 4]| {
        if points.iter().flatten().any(|value| !value.is_finite()) {
            return None;
        }
        let min_x = points.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
        let max_x = points.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
        let min_y = points.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
        let max_y = points.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max);
        Some([(max_x.ceil() - min_x.floor()).max(0.0) as u32, (max_y.ceil() - min_y.floor()).max(0.0) as u32])
    };
    let [x0, y0, x1, y1] = session.rect;
    event.source_size = size(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]]);
    event.target_size = size(&session.quad);
    event.is_smart = app.session.active().and_then(|st| st.doc.layer(LayerId(session.layer))).map(|layer| matches!(layer.content, LayerContent::Smart(_)));
    event
}

pub(crate) fn command(app: &mut PhotocraftApp, id: &str, params: serde_json::Value, event: DiagnosticEvent) -> Result<serde_json::Value, String> {
    let result = app.run(id, params);
    let mut finished = event;
    finished.stage = DiagnosticStage::TransformCommandFinished;
    finished.success = Some(result.is_ok());
    record(app, finished);
    result
}

/// Only once after a recorded transition; an idle frame performs no filesystem work.
/// A built egui frame does not prove that the GPU submitted or presented that frame.
pub(crate) fn frame_built(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if let Some(mut event) = app.diagnostic_frame_pending.take() {
        event.stage = DiagnosticStage::UiFrameBuilt;
        if let Some(callback) = app.services.diagnostic_event.as_ref() {
            callback(event);
        }
    }
    if app.services.diagnostic_idle.as_ref().is_some_and(|idle| idle()) {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}
