//! A narrowly guarded bridge from a hidden-layer transform backing to committed pixels.
//! This never relabels a canvas cache before its ROI has actually been composited.
use std::sync::{Arc, Weak};

use photocraft_doc::{DocId, Document, LayerContent, LayerId};
use photocraft_geom::Rect;

use crate::{
    PhotocraftApp,
    state::{MadeLayer, TransformSession},
    transform_tool::TransformPreview,
};

pub(crate) struct Origin {
    document: DocId,
    source: Weak<Document>,
    revision: u64,
    layer: LayerId,
    bounds: Rect,
}

pub(crate) struct Prepared {
    origin: Origin,
    session: u64,
    preview_key: u64,
}

/// Lightweight and non-serialised. Retained for each output cache to catch up independently;
/// an unrelated edit or a new transform makes its exact snapshot/revision checks fail.
pub(crate) struct Transition {
    document: DocId,
    target: Weak<Document>,
    before_revision: u64,
    after_revision: u64,
    session: u64,
    preview_key: u64,
    damage: Rect,
}

/// Deliberately only the topmost root layer. Nothing above it, or around it, may turn a
/// local content change into a non-local adjustment/effect/clipping/group dependency.
fn eligible_bounds(doc: &Document, id: LayerId) -> Option<Rect> {
    if doc.selection.is_some() || !matches!(doc.mode, photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale) {
        return None;
    }
    let layer = doc.layers.last().filter(|layer| layer.id == id)?;
    if !layer.visible
        || layer.blend != photocraft_color::BlendMode::Normal
        || layer.opacity != 1.0
        || layer.fill_opacity != 1.0
        || layer.clipped
        || layer.mask.is_some()
        || layer.vector_mask.is_some()
        || !layer.effects.items.is_empty()
        || layer.effects.psd_raw.is_some()
        || !layer.psd_blocks.is_empty()
        || layer.excluded_channels != 0
        || !layer.blend_if.is_default()
        || layer.video.is_some()
        || layer.link_group.is_some()
    {
        return None;
    }
    let surface = match &layer.content {
        LayerContent::Raster(surface) => surface,
        LayerContent::Smart(smart)
            if smart.smart_filters.is_empty()
                && smart.filter_mask.is_none()
                && smart.warp.is_none()
                && smart.stack_mode.is_none()
                && smart.perspective.is_none()
                && smart.psd_raw.is_none() =>
        {
            smart.cache.as_ref()?
        }
        _ => return None,
    };
    // A finite content box is sufficient only when the surface is transparent outside it.
    if !surface.format().alpha || !surface.default_pixel().last().is_some_and(|alpha| *alpha == 0.0) {
        return None;
    }
    let bounds = surface.content_bounds();
    (!bounds.is_empty()).then_some(bounds)
}

impl Origin {
    pub(crate) fn capture(doc: &Arc<Document>, revision: u64, layer: LayerId) -> Option<Self> {
        Some(Self { document: doc.id, source: Arc::downgrade(doc), revision, layer, bounds: eligible_bounds(doc, layer)? })
    }
}

pub(crate) fn prepare(app: &PhotocraftApp, session: &TransformSession, preview: &TransformPreview) -> Option<Prepared> {
    if session.made != Some(MadeLayer::Place)
        || session.selection
        || session.target.is_some()
        || session.warp.is_some()
        || session.session != preview.session
        || app.session.has_jobs()
    {
        return None;
    }
    let origin = preview.damage_origin.as_ref()?;
    let state = app.session.active()?;
    if state.doc.id != origin.document
        || state.revision != origin.revision
        || session.layer != origin.layer.0
        || preview.doc.id != origin.document
        || preview.doc.size != state.doc.size
        || !origin.source.ptr_eq(&Arc::downgrade(&state.doc))
    {
        return None;
    }
    eligible_bounds(&state.doc, origin.layer)?;
    Some(Prepared {
        origin: Origin { document: origin.document, source: origin.source.clone(), revision: origin.revision, layer: origin.layer, bounds: origin.bounds },
        session: session.session,
        preview_key: (1u64 << 40).checked_add(session.session)?,
    })
}

pub(crate) fn finish(app: &mut PhotocraftApp, prepared: Option<Prepared>, changed: bool) {
    let Some(prepared) = prepared else { return };
    let Some(state) = app.session.active() else { return };
    let expected_revision = if changed { prepared.origin.revision.checked_add(1) } else { Some(prepared.origin.revision) };
    if Some(state.revision) != expected_revision
        || state.doc.id != prepared.origin.document
        || app.session.has_jobs()
        || app.ui.transform.is_some()
        || app.transform_preview.is_some()
    {
        return;
    }
    let Some(after) = eligible_bounds(&state.doc, prepared.origin.layer) else { return };
    let bounds = prepared.origin.bounds.union(&after);
    // Match sampling/filter edge support without overflowing hostile coordinates.
    let damage = Rect::new(bounds.x0.saturating_sub(4), bounds.y0.saturating_sub(4), bounds.x1.saturating_add(4), bounds.y1.saturating_add(4))
        .intersect(&state.doc.bounds());
    if damage.is_empty() {
        return;
    }
    app.transform_damage = Some(Transition {
        document: state.doc.id,
        target: Arc::downgrade(&state.doc),
        before_revision: prepared.origin.revision,
        after_revision: state.revision,
        session: prepared.session,
        preview_key: prepared.preview_key,
        damage,
    });
}

pub(crate) fn damage(app: &PhotocraftApp, index: usize, seen: (u64, u64), now: (u64, u64), display_key: u64) -> Option<Rect> {
    let transition = app.transform_damage.as_ref()?;
    if app.ui.transform.is_some()
        || app.transform_preview.is_some()
        || app.session.has_jobs()
        || now.1 != 0
        || (1u64 << 40).checked_add(transition.session) != Some(transition.preview_key)
        || seen != (transition.before_revision, transition.preview_key ^ display_key)
        || now.0 != transition.after_revision
    {
        return None;
    }
    let state = app.session.documents().get(index)?;
    if state.doc.id != transition.document
        || state.revision != transition.after_revision
        || !transition.target.ptr_eq(&Arc::downgrade(&state.doc))
        || app.session.active().is_none_or(|active| active.doc.id != transition.document)
    {
        return None;
    }
    Some(transition.damage)
}

#[cfg(test)]
#[path = "transform_damage_tests_v91.rs"]
mod tests;
