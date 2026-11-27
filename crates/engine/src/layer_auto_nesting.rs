//! Move-tool automatic artboard membership. Explicit Layers-panel placement is separate.
use photocraft_doc::{Document, Layer, LayerId};

use crate::{EngineError, Result};

fn subtree_prevents_nesting(layer: &Layer) -> bool {
    layer.locks.artboard || layer.children().is_some_and(|children| children.iter().any(subtree_prevents_nesting))
}

/// Update membership after the complete moving block has reached its new geometry.
/// The document uses global coordinates inside groups, so reparenting never adds
/// a second pixel translation. Ordinary groups move as a unit, artboards stay roots.
pub(super) fn after_move(doc: &mut Document, ids: &[LayerId]) -> Result<()> {
    if !doc.has_artboards() {
        return Ok(());
    }
    // Preserve display order when multiple layers enter/leave the same board.
    let roots = super::top_level(doc, ids);
    for id in roots {
        let layer = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        if layer.artboard().is_some() || doc.effective_locks(id).artboard || subtree_prevents_nesting(layer) {
            continue;
        }
        let Some(bounds) = super::layer_bounds(layer) else { continue };
        let center = [(f64::from(bounds.x0) + f64::from(bounds.x1)) * 0.5, (f64::from(bounds.y0) + f64::from(bounds.y1)) * 0.5];
        let previous = doc.artboard_of(id);
        // Artboards are top-level, bottom-to-top in the model. The visible top
        // board wins overlapping hit regions; do not fall through a locked one.
        let target = doc.artboards().into_iter().rev().find_map(|(board, _, artboard)| {
            let r = artboard.rect;
            (center[0] >= f64::from(r.x0) && center[0] < f64::from(r.x1) && center[1] >= f64::from(r.y0) && center[1] < f64::from(r.y1)).then_some(board)
        });
        if target == previous {
            continue;
        }
        if let Some(target) = target {
            let locks = doc.effective_locks(target);
            if locks.artboard || locks.all {
                continue;
            }
        }
        let layer = doc.remove(id).ok_or(EngineError::NoLayer(id))?;
        if let Some(target) = target {
            doc.layer_mut(target).and_then(Layer::children_mut).ok_or(EngineError::NoLayer(target))?.push(layer);
        } else {
            doc.layers.push(layer);
        }
    }
    super::check_group_depth(doc, "Automatic Artboard Nesting")
}
