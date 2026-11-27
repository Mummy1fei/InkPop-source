//! Deterministic wet-paint evolution; explicit time steps are undoable and replayable.
use crate::{EngineError, Result, Session, commands::CommandSpec};
use serde_json::{Value, json};

pub fn load(s: &mut Session, p: &Value) -> Result<Value> {
    use crate::sample_color_cmds::{self as sample, Source};
    let object = p.as_object().ok_or_else(|| EngineError::Other("mixer load requires an object".into()))?;
    if object.keys().any(|key| !matches!(key.as_str(), "at" | "clean" | "sampleSize" | "sampleAllLayers" | "source" | "coalesce")) {
        return Err(EngineError::Other("unknown mixer load property".into()));
    }
    let clean = p.get("clean").map(|value| value.as_bool().ok_or_else(|| EngineError::Other("clean must be a boolean".into()))).transpose()?.unwrap_or(false);
    let explicit_size = p.get("sampleSize").map(sample::size).transpose()?;
    let sample_all =
        p.get("sampleAllLayers").map(|value| value.as_bool().ok_or_else(|| EngineError::Other("sampleAllLayers must be a boolean".into()))).transpose()?;
    if sample_all.is_some() && p.get("source").is_some() {
        return Err(EngineError::Other("use source or sampleAllLayers, not both".into()));
    }
    let source = p
        .get("source")
        .map(sample::source)
        .transpose()?
        .unwrap_or_else(|| if sample_all.unwrap_or(s.tools.brush.mixer.sample_all_layers) { Source::All } else { Source::Current });
    if clean {
        s.tools.mixer = Default::default();
        s.tools.mixer.explicitly_empty = true;
        return Ok(json!({"loaded":false}));
    }
    let at: [f64; 2] =
        serde_json::from_value(p.get("at").cloned().unwrap_or(Value::Null)).map_err(|_| EngineError::Other("at must be two finite coordinates".into()))?;
    if at.iter().any(|v| !v.is_finite() || v.abs() > 1_000_000.0) {
        return Err(EngineError::Other("at is outside the supported coordinate range".into()));
    }
    let brush = &s.tools.brush;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let id = st.active_layer;
    if brush.mixer.load_solid_only || explicit_size == Some(1) {
        let size = explicit_size.unwrap_or(1);
        let radius = (size / 2) as i32;
        let requested = photocraft_geom::Rect::from_xywh(at[0].floor() as i32 - radius, at[1].floor() as i32 - radius, size, size);
        let area = if explicit_size.is_some() { requested.intersect(&st.doc.bounds()) } else { requested };
        let color = sample::average(&sample::read(&st.doc, id, source, area)?);
        let cells = if brush.mixer.load_solid_only {
            s.tools.mixer.load(color);
            1
        } else {
            let side = photocraft_paint::wet_paint::pickup_side(size as f32);
            s.tools.mixer.clean();
            s.tools.mixer.reservoir = None;
            s.tools.mixer.loaded_nib = vec![color; side * side];
            s.tools.mixer.amount = 1.0;
            s.tools.mixer.explicitly_empty = false;
            side * side
        };
        s.tools.mixer.canvas_loaded = true;
        s.tools.mixer.foreground_at_load = Some(s.tools.foreground);
        return Ok(json!({"loaded":true,"cells":cells,"sampleSize":size,"source":source}));
    }
    // Omitted sampleSize retains the legacy CLI footprint. UI sampling controls
    // always send their independent size rather than changing the brush preset.
    let size = explicit_size.map(|size| size as f32).unwrap_or(brush.size.clamp(1.0, 4096.0));
    let sample_at = if explicit_size.is_some() { [at[0].floor() + 0.5, at[1].floor() + 0.5] } else { at };
    let (sin, cos) = brush.angle.to_radians().sin_cos();
    let radius = f64::from(size);
    let area = photocraft_geom::Rect::new(
        (sample_at[0] - radius) as i32,
        (sample_at[1] - radius) as i32,
        (sample_at[0] + radius + 1.0) as i32,
        (sample_at[1] + radius + 1.0) as i32,
    )
    .intersect(&st.doc.bounds());
    let surface = sample::raw_surface(&st.doc, id, source)?;
    let pixels = if surface.is_some() { Vec::new() } else { sample::read(&st.doc, id, source, area)? };
    // Apply the same work limit to direct reads without allocating that region.
    if u64::from(area.width()) * u64::from(area.height()) > 16_000_000 {
        return Err(EngineError::Other("mixer load region exceeds 16 million pixels".into()));
    }
    let cmyk = photocraft_compose::cmyk_space(&st.doc);
    let side = photocraft_paint::wet_paint::pickup_side(size);
    let mut nib = Vec::with_capacity(side * side);
    photocraft_color::convert::with_cmyk_space(cmyk.as_ref(), || {
        for y in 0..side {
            for x in 0..side {
                let mut u = (x as f32 + 0.5) / side as f32 - 0.5;
                let mut v = ((y as f32 + 0.5) / side as f32 - 0.5) * brush.roundness;
                if brush.flip_x {
                    u = -u;
                }
                if brush.flip_y {
                    v = -v;
                }
                let px = (sample_at[0] + f64::from((u * cos + v * sin) * size)).floor() as i32;
                let py = (sample_at[1] + f64::from((-u * sin + v * cos) * size)).floor() as i32;
                let color = if let Some(surface) = surface {
                    surface.rgba(px, py)
                } else if area.contains(px, py) {
                    let index = (py - area.y0) as usize * area.width() as usize + (px - area.x0) as usize;
                    pixels.get(index).copied().unwrap_or([0.0; 4])
                } else {
                    [0.0; 4]
                };
                nib.push(color);
            }
        }
    });
    s.tools.mixer.clean();
    s.tools.mixer.reservoir = None;
    s.tools.mixer.loaded_nib = nib;
    s.tools.mixer.amount = 1.0;
    s.tools.mixer.explicitly_empty = false;
    s.tools.mixer.canvas_loaded = true;
    s.tools.mixer.foreground_at_load = Some(s.tools.foreground);
    Ok(json!({"loaded":true,"cells":side * side,"sampleSize":size,"source":source}))
}

pub fn step(s: &mut Session, p: &Value) -> Result<Value> {
    let seconds = p
        .get("seconds")
        .map_or(Some(1.0), Value::as_f64)
        .filter(|v| v.is_finite() && *v > 0.0 && *v <= 600.0)
        .ok_or_else(|| EngineError::Other("seconds must be finite and in (0,600]".into()))? as f32;
    let id = s.active().ok_or(EngineError::NoDocument)?.active_layer.ok_or(EngineError::Other("no selected layer".into()))?;
    let mut m = s.tools.brush.mixer.clone();
    m.pigment_profile = Some(crate::color_cmds::document_profile(&s.active().ok_or(EngineError::NoDocument)?.doc).to_bytes());
    let damage = s.edit("Advance Wet Paint", |doc, _| {
        let locks = doc.effective_locks(id);
        if locks.pixels || locks.all {
            return Err(EngineError::Other("selected layer is locked".into()));
        }
        let mut medium = doc.wet_paint.remove(&id).ok_or_else(|| EngineError::Other("selected layer has no wet paint".into()))?;
        let area = medium.bounds.intersect(&doc.bounds());
        if u64::from(area.width()) * u64::from(area.height()) > 16_000_000 {
            return Err(EngineError::Other("wet paint evolution region exceeds 16 million pixels".into()));
        }
        let selection = doc.selection.clone();
        photocraft_paint::wet_paint::step(&mut medium.pigment, &mut medium.water, &mut medium.thickness, area, seconds, &m, selection.as_ref());
        let layer = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let target = layer.surface_mut().ok_or_else(|| EngineError::Other("wet paint requires a pixel layer".into()))?;
        photocraft_paint::wet_paint::render_masked(&medium.pigment, &medium.thickness, target, area, selection.as_ref(), locks.transparency);
        doc.wet_paint.insert(id, medium);
        Ok(area)
    })?;
    Ok(json!({"seconds": seconds, "damage": [damage.x0, damage.y0, damage.x1, damage.y1]}))
}
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "tools.mixer.load",
            label: "Load Mixer Brush",
            menu: &[],
            shortcut: None,
            params: r#"{"at":[x,y],"sampleSize":1|3|5|11|31|51|101?,"sampleAllLayers":bool?,"source":"current|currentBelow|all|allNoAdjustments|currentBelowNoAdjustments"?} loads a multicolor footprint independently of brush size when sampleSize is supplied; source and sampleAllLayers are exclusive; {"clean":true} empties the brush"#,
            enabled: |s| if s.active().is_some() { Ok(()) } else { Err("no document".into()) },
            run: load,
            journal: true,
        },
        CommandSpec {
            id: "paint.medium.step",
            label: "Advance Wet Paint",
            menu: &[],
            shortcut: None,
            params: r#"{"seconds":0<seconds<=600=1}; selected layer, deterministic diffusion and drying"#,
            enabled: |s| if s.active().is_some() { Ok(()) } else { Err("no document".into()) },
            run: step,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choosing_foreground_replaces_sampled_paint_and_pickup_at_every_depth() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":120,"height":80,"depth":depth,"background":"#ff0000"})).unwrap();
            s.tools.brush.size = 12.;
            s.execute("tools.mixer.load", json!({"at":[12,12]})).unwrap();
            s.execute("tools.setColors", json!({"foreground":"#00ff00"})).unwrap();
            for y in [25, 50] {
                // Isolate replacing sampled color from the default 50% load's
                // gradual depletion (whose opacity is covered by paint tests).
                s.execute("paint.mixerBrush", json!({"wet":0,"mix":0,"load":100,"flow":100,"simulatePaint":true,"points":[[30,y,1],[90,y,1]]})).unwrap();
                let st = s.active().unwrap();
                let px = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(60, y);
                assert!(px[1] > 0.9 && px[0] < 0.1, "{depth}: {px:?}");
                assert!(!s.tools.mixer.canvas_loaded);
                assert_eq!(s.tools.mixer.reservoir, Some([0., 1., 0., 1.]));
                assert!(s.tools.mixer.loaded_nib.is_empty());
            }
            s.execute("tools.mixer.load", json!({"at":[12,12]})).unwrap();
            // Some existing controls edit tool colours directly; the stroke also catches that.
            s.tools.foreground = [0., 0., 1., 1.];
            s.execute("paint.mixerBrush", json!({"wet":0,"mix":0,"simulatePaint":true,"points":[[30,65,1],[90,65,1]]})).unwrap();
            assert_eq!(s.tools.mixer.reservoir, Some([0., 0., 1., 1.]));
        }
    }

    #[test]
    fn mixer_load_samples_within_one_brush_diameter() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":200,"height":40,"background":"transparent"})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        s.edit("sample stripe", |doc, _| {
            let surface = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.surface_mut().ok_or_else(|| EngineError::Other("pixel layer required".into()))?;
            surface.fill_rect(photocraft_geom::Rect::new(80, 0, 120, 40), &[1., 0., 0., 1.]);
            Ok(())
        })
        .unwrap();
        s.tools.brush.size = 40.;
        s.tools.brush.angle = 0.;
        s.execute("tools.mixer.load", json!({"at":[100,20]})).unwrap();
        let nib = &s.tools.mixer.loaded_nib;
        assert!(!nib.is_empty());
        assert!(nib.iter().all(|px| px[3] > 0.99), "the loaded footprint must stay inside the 40 px stripe");
    }
    #[test]
    fn sampled_paint_survives_many_strokes_and_autofill_until_cleared() {
        for depth in [8, 16, 32] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":180,"height":100,"depth":depth,"background":"transparent"})).unwrap();
            let id = s.active().unwrap().active_layer.unwrap();
            s.edit("source", |doc, _| {
                let surf = doc.layer_mut(id).unwrap().surface_mut().unwrap();
                surf.fill_rect(photocraft_geom::Rect::new(0, 0, 30, 20), &[1., 0., 0., 1.]);
                surf.fill_rect(photocraft_geom::Rect::new(0, 20, 30, 40), &[0., 0., 1., 1.]);
                Ok(())
            })
            .unwrap();
            s.tools.brush.size = 20.;
            s.execute("tools.mixer.load", json!({"at":[15,20]})).unwrap();
            let nib = s.tools.mixer.loaded_nib.clone();
            s.execute("tools.setColors", json!({"background":"#00ff00"})).unwrap();
            for y in [15, 40, 65, 90] {
                s.execute(
                    "paint.mixerBrush",
                    json!({"wet":0,"mix":0,"load":1,"loadAfterStroke":true,"cleanAfterStroke":true,"simulatePaint":true,"points":[[50,y,1],[160,y,1]]}),
                )
                .unwrap();
                assert_eq!(s.tools.mixer.loaded_nib, nib);
                assert_eq!(s.tools.mixer.amount, 1.);
                let surf = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap();
                assert!(surf.rgba(100, y - 4)[0] > 0.8);
                assert!(surf.rgba(100, y + 4)[2] > 0.8);
            }
            s.execute("tools.mixer.load", json!({"clean":true})).unwrap();
            assert!(!s.tools.mixer.canvas_loaded);
            assert!(s.tools.mixer.loaded_nib.is_empty());
        }
    }
    #[test]
    fn clearing_the_brush_does_not_inject_foreground_before_the_next_stroke() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":40,"height":30,"background":"transparent"})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        s.execute("tools.mixer.load", json!({"clean":true})).unwrap();
        assert!(s.tools.mixer.explicitly_empty);
        s.execute("paint.mixerBrush", json!({"size":8,"wet":0,"mix":0,"simulatePaint":true,"loadAfterStroke":false,"points":[[10,10,1],[25,10,1]]})).unwrap();
        let px = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().rgba(18, 10);
        assert_eq!(px[3], 0., "{px:?}");
        assert!(s.tools.mixer.reservoir.is_none());
    }
    #[test]
    fn canvas_load_keeps_spatial_colors_and_direct_edits_invalidate_medium() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":80,"height":60,"background":"transparent"})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        s.edit("test stripes", |doc, _| {
            let surface = doc.layer_mut(id).unwrap().surface_mut().unwrap();
            surface.fill_rect(photocraft_geom::Rect::new(0, 0, 30, 30), &[1., 0., 0., 1.]);
            surface.fill_rect(photocraft_geom::Rect::new(0, 30, 30, 60), &[0., 0., 1., 1.]);
            Ok(())
        })
        .unwrap();
        s.tools.brush.size = 20.;
        s.execute("tools.mixer.load", json!({"at":[15,30]})).unwrap();
        assert_eq!(s.tools.mixer.loaded_nib.len(), 4096);
        assert!(s.tools.mixer.loaded_nib[0][0] > 0.9);
        assert!(s.tools.mixer.loaded_nib[4095][2] > 0.9);
        s.execute("paint.mixerBrush", json!({"wet":0,"mix":0,"simulatePaint":true,"loadAfterStroke":false,"points":[[40,30,1],[65,30,1]]})).unwrap();
        let surface = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap();
        assert!(surface.rgba(50, 25)[0] > 0.8);
        assert!(surface.rgba(50, 35)[2] > 0.8);
        assert!(s.active().unwrap().doc.wet_paint.contains_key(&id));
        s.execute("paint.stroke", json!({"points":[[50,30,1]],"color":"#00ff00","size":10})).unwrap();
        assert!(s.active().unwrap().doc.wet_paint.is_empty());
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.wet_paint.contains_key(&id));
        assert!(s.execute("tools.mixer.load", json!({"at":[1e100,0]})).is_err());
    }
    #[test]
    fn paint_fields_are_undoable_and_time_steps_reject_bad_input() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":80,"height":60,"background":"transparent"})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        s.execute(
            "paint.mixerBrush",
            json!({"size":16,"color":"#d09020","wet":100,"simulatePaint":true,
            "points":[[10,20,1],[60,20,1]]}),
        )
        .unwrap();
        assert!(s.active().unwrap().doc.wet_paint.contains_key(&id));
        let before = s.active().unwrap().doc.clone();
        s.execute("paint.medium.step", json!({"seconds":60})).unwrap();
        let dry = s.active().unwrap().doc.wet_paint.get(&id).unwrap();
        assert!(dry.water.pixel(30, 20)[0] < before.wet_paint.get(&id).unwrap().water.pixel(30, 20)[0]);
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(&*s.active().unwrap().doc, &*before);
        for p in [json!({"seconds":-1}), json!({"seconds":null}), json!({"seconds":1e100})] {
            assert!(s.execute("paint.medium.step", p).is_err());
        }
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.wet_paint.is_empty());
    }
}
