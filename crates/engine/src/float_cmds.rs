//! Floating selection: the selected pixels of a layer cut out and moved as a piece, without
//! touching the document until the piece is dropped.
//!
//! - `select.float {"dx","dy","copy"}` cuts or copies the selected pixels into a floating piece
//!   (the first time) and moves it by whole pixels. Further calls move the same piece; nothing new
//!   is cut. On the Background, a cut fills the vacated area with the background colour.
//! - `select.drop` drops the piece into its layer and moves the selection with it, as one history
//!   step.
//! - Any other command drops the piece first (so it lands exactly where it was shown), except Undo,
//!   which puts it back where it was cut (nothing was changed yet).
//!
//! While floating, the document is unchanged; [`displayed`] gives the document with the piece at
//! its offset, for the canvas. The selection stays where it was until the drop.

use std::sync::Arc;

use photocraft_color::PixelFormat;
use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{DocState, EngineError, Result, Session};

/// A layer split by its selection: the layer with the selected pixels cut out, and the cut pixels
/// (cropped to the selection). Splitting touches only the selection's area, and placing the piece
/// ([`CutParts::moved`]) only the piece's: the cut-out layer's other tiles are shared.
#[derive(Clone, Debug)]
pub struct CutParts {
    pub layer: LayerId,
    rest: Surface,
    piece: Surface,
    copy: bool,
}

impl CutParts {
    /// Split `layer` of `doc` by its selection. Errors without a selection or pixels.
    pub fn new(doc: &Document, layer: LayerId) -> Result<Self> {
        Self::new_with_copy(doc, layer, false)
    }

    /// Capture the selected pixels once. A copy keeps the source pixels in place;
    /// a cut removes them from the preview background. Neither modifies `doc`.
    pub fn new_with_copy(doc: &Document, layer: LayerId, copy: bool) -> Result<Self> {
        Self::capture(doc, layer, copy, [1.0; 4])
    }

    fn capture(doc: &Document, layer: LayerId, copy: bool, background_color: [f32; 4]) -> Result<Self> {
        let sel = doc.selection.as_ref().ok_or_else(|| EngineError::Other("no selection".into()))?;
        let l = doc.layer(layer).ok_or(EngineError::NoLayer(layer))?;
        let LayerContent::Raster(surf) = &l.content else {
            return Err(EngineError::Other("the layer has no pixels to move".into()));
        };
        let fmt = surf.format();
        let with_alpha = PixelFormat::new(fmt.mode, fmt.sample, true);
        let mut rest = if fmt == with_alpha { surf.clone() } else { surf.convert(with_alpha) };
        let background = is_base_background(doc, l);
        let mut piece = Surface::new(with_alpha);
        let mut has_pixels = false;
        let b = sel.content_bounds().intersect(&surf.content_bounds());
        if !b.is_empty() {
            let n = with_alpha.channels();
            let a = n - 1;
            let mut pp = rest.read_region(b);
            let mut rp = if copy || background { Vec::new() } else { pp.clone() };
            let mut rest_pixels = rp.chunks_exact_mut(n);
            let w = b.width() as usize;
            for (i, p) in pp.chunks_exact_mut(n).enumerate() {
                let k = sel.sample_channel(b.x0 + (i % w) as i32, b.y0 + (i / w) as i32, 0);
                if k <= 0.0 {
                    p.fill(0.0);
                } else {
                    p[a] *= k;
                    has_pixels |= p[a] > 0.0;
                }
                if let Some(r) = rest_pixels.next() {
                    r[a] *= 1.0 - k;
                }
            }
            if !copy {
                if background {
                    // The flat Background stays opaque, as with Edit → Cut/Clear.
                    crate::pixels::fill_surface(&mut rest, b, background_color, Some(sel), true);
                } else {
                    rest.write_region(b, &rp);
                }
                rest.prune();
            }
            piece.write_region(b, &pp);
            piece.prune();
        }
        if !has_pixels {
            return Err(EngineError::Other("the selection contains no pixels on the active layer".into()));
        }
        Ok(Self { layer, rest, piece, copy })
    }

    /// Whether this piece preserves the original selected pixels.
    pub fn is_copy(&self) -> bool {
        self.copy
    }

    /// `doc` with the piece moved by whole pixels over the cut-out or original layer.
    pub fn moved(&self, doc: &Document, dx: i32, dy: i32) -> Result<Document> {
        // Cutting and compositing a soft/transparent selection at its original
        // position is not an identity. A zero-offset preview must be one.
        if dx == 0 && dy == 0 {
            return Ok(doc.clone());
        }
        let mut d = doc.clone();
        let surf = d
            .layer_mut(self.layer)
            .ok_or(EngineError::NoLayer(self.layer))?
            .surface_mut()
            .ok_or_else(|| EngineError::Other("the layer has no pixels to move".into()))?;
        let mut out = self.rest.clone();
        crate::transform_cmds::composite_over(&mut out, &photocraft_algo::resample::translate_surface(&self.piece, dx, dy));
        *surf = out;
        Ok(d)
    }
}

fn is_base_background(doc: &Document, layer: &Layer) -> bool {
    doc.layers.first().is_some_and(|bottom| bottom.id == layer.id) && crate::extra_cmds::is_background(layer)
}

/// A piece floating `offset` whole pixels from where it was cut, over document `revision`.
#[derive(Clone, Debug)]
pub struct Floating {
    pub layer: LayerId,
    pub offset: (i32, i32),
    pub revision: u64,
    pub parts: Arc<CutParts>,
}

/// The floating piece of `st`, if it is still valid (the document hasn't changed under it).
pub fn floating(st: &DocState) -> Option<&Floating> {
    st.floating.as_ref().filter(|f| f.revision == st.revision && st.doc.selection.is_some())
}

/// The document as shown while a piece floats `extra` further than its offset (a drag of it in
/// progress); `None` when nothing floats.
pub fn displayed(st: &DocState, extra: (i32, i32)) -> Option<Document> {
    let f = floating(st)?;
    f.parts.moved(&st.doc, f.offset.0.saturating_add(extra.0), f.offset.1.saturating_add(extra.1)).ok()
}

fn int_param(p: &Value, key: &str) -> i32 {
    p.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).map_or(0, |v| v.round().clamp(-1e7, 1e7) as i32)
}

fn can_float(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document")?;
    if floating(st).is_some() {
        return Ok(());
    }
    if st.doc.selection.as_ref().is_none_or(|selection| selection.content_bounds().is_empty()) {
        return Err("there is no nonempty selection".into());
    }
    if st.selected_layers().len() != 1 {
        return Err("select one pixel layer to move selected pixels".into());
    }
    let l = st.active_layer.and_then(|id| st.doc.layer(id)).ok_or("no active layer")?;
    if !matches!(l.content, LayerContent::Raster(_)) {
        return Err("the active layer has no pixels to move".into());
    }
    let locks = st.doc.effective_locks(l.id);
    // A native Background's implicit position/alpha locks still apply to the
    // layer as a whole. Selected pixels can be moved within that layer.
    if locks.all || locks.pixels || ((locks.position || locks.transparency) && !is_base_background(&st.doc, l)) {
        return Err(format!("layer \"{}\" is locked", l.name));
    }
    Ok(())
}

fn float(s: &mut Session, p: &Value) -> Result<Value> {
    let (dx, dy) = (int_param(p, "dx"), int_param(p, "dy"));
    let copy = p
        .get("copy")
        .map(|value| value.as_bool().ok_or_else(|| EngineError::BadParams { cmd: "select.float".into(), msg: "`copy` must be a boolean".into() }))
        .transpose()?;
    let background_color = s.tools.background;
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    if floating(st).is_none() {
        let layer = st.active_layer.ok_or_else(|| EngineError::Other("no active layer".into()))?;
        let parts = CutParts::capture(&st.doc, layer, copy.unwrap_or(false), background_color)?;
        st.floating = Some(Floating { layer, offset: (0, 0), revision: st.revision, parts: Arc::new(parts) });
    }
    let f = st.floating.as_mut().ok_or(EngineError::NoDocument)?;
    if copy.is_some_and(|copy| copy != f.parts.is_copy()) {
        return Err(EngineError::BadParams { cmd: "select.float".into(), msg: "copy mode cannot change while a selection is floating".into() });
    }
    f.offset = (f.offset.0.saturating_add(dx), f.offset.1.saturating_add(dy));
    Ok(json!({"layer": f.layer.0, "offset": [f.offset.0, f.offset.1]}))
}

/// Drop the floating piece into its layer and move the selection with it (one history step).
pub fn drop_floating(s: &mut Session) -> Result<Value> {
    let Some(st) = s.active_mut() else { return Ok(json!({"dropped": false})) };
    let Some(f) = floating(st).cloned() else {
        st.floating = None;
        return Ok(json!({"dropped": false}));
    };
    st.floating = None;
    let (dx, dy) = f.offset;
    if (dx, dy) == (0, 0) {
        return Ok(json!({"dropped": true, "offset": [0, 0]}));
    }
    s.edit(if f.parts.is_copy() { "Copy Selected Pixels" } else { "Move Selected Pixels" }, |doc, _| {
        let moved = f.parts.moved(doc, dx, dy)?;
        let surf = moved.layer(f.layer).and_then(|l| l.surface()).cloned().ok_or(EngineError::NoLayer(f.layer))?;
        *doc.layer_mut(f.layer).ok_or(EngineError::NoLayer(f.layer))?.surface_mut().ok_or(EngineError::NoLayer(f.layer))? = surf;
        doc.selection = doc.selection.as_ref().map(|sel| photocraft_algo::resample::translate_surface(sel, dx, dy));
        Ok(())
    })?;
    Ok(json!({"dropped": true, "offset": [dx, dy]}))
}

/// Commands that leave a floating piece floating: its own, and ones that don't touch documents.
fn keeps_floating(id: &str) -> bool {
    matches!(id, "select.float" | "select.drop") || ["view.", "window.", "help.", "prefs."].iter().any(|p| id.starts_with(p))
}

/// Before any command (`Session::dispatch`): Undo puts a floating piece back (and is used up:
/// returns its result); every other command but the floating ones drops it first.
pub(crate) fn before_command(s: &mut Session, id: &str) -> Result<Option<Value>> {
    let Some(st) = s.active_mut() else { return Ok(None) };
    if floating(st).is_none() {
        st.floating = None;
        return Ok(None);
    }
    match id {
        "edit.undo" | "edit.stepBackward" => {
            st.floating = None;
            Ok(Some(json!({"floating": "returned"})))
        }
        _ if keeps_floating(id) => Ok(None),
        _ => drop_floating(s).map(|_| None),
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "select.float",
            label: "Float Selection",
            menu: &[],
            shortcut: None,
            params: r##"{"dx":px=0,"dy":px=0,"copy":bool=false (first call; omit on subsequent calls)} → {layer, offset} (cuts or copies selected pixels into a floating piece, then moves that same piece by whole pixels; copy preserves the source on the same layer; dropped by select.drop or other commands, canceled by edit.undo; changing copy mode while floating is rejected)"##,
            enabled: can_float,
            journal: true,
            run: float,
        },
        CommandSpec {
            id: "select.drop",
            label: "Drop Floating Selection",
            menu: &[],
            shortcut: None,
            params: r##"{} → {dropped, offset} (drops the floating piece into its layer and moves the selection with it: one history step)"##,
            enabled: |s| s.active().map(|_| ()).ok_or_else(|| "no document".into()),
            journal: true,
            run: |s, _| drop_floating(s),
        },
    ]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "float_cmds/copy_tests_v81.rs"]
mod copy_tests_v81;
