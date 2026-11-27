use super::*;
use photocraft_doc::Adjustment;
use photocraft_raster::Surface;
use std::sync::Arc;

fn fixture(depth: u32, mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":80,"height":60,"depth":depth,"mode":mode,"background":"transparent"})).unwrap();
    s
}

fn sample(s: &mut Session, x: f64, y: f64, size: u32, source: &str) -> [f32; 4] {
    serde_json::from_value(s.execute(COMMAND, json!({"x":x,"y":y,"size":size,"source":source})).unwrap()).unwrap()
}

fn fill_layer(name: &str, fmt: photocraft_color::PixelFormat, color: [f32; 4]) -> Layer {
    let mut surface = Surface::new(fmt);
    surface.fill_rect(Rect::new(0, 0, 80, 60), &photocraft_raster::from_rgba(&fmt, color));
    Layer::new(name, LayerContent::Raster(surface))
}

#[test]
fn sample_color_v79_point_and_average_preserve_native_depth_modes_and_hdr() {
    for depth in [8, 16, 32] {
        for mode in ["rgb", "grayscale", "cmyk", "lab"] {
            let mut s = fixture(depth, mode);
            s.execute("edit.fill", json!({"color":"#4070a0"})).unwrap();
            let current = sample(&mut s, 10.0, 10.0, 1, "current");
            let composite = sample(&mut s, 10.0, 10.0, 1, "all");
            let average = sample(&mut s, 10.0, 10.0, 11, "current");
            for ((a, b), c) in current.into_iter().zip(composite).zip(average) {
                assert!((a - b).abs() < 0.012, "{depth}/{mode}: current/composite {a} {b}");
                assert!((a - c).abs() < 0.00001, "{depth}/{mode}: a uniform average retains the point value");
            }
        }
    }
    let mut hdr = fixture(32, "rgb");
    hdr.edit("Synthetic HDR pixel", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().write_pixel(5, 5, &[1.7, -0.2, 0.5, 0.25]);
        Ok(())
    })
    .unwrap();
    let pixel = sample(&mut hdr, 5.0, 5.0, 1, "current");
    assert!(pixel[0] > 1.69 && pixel[1] < -0.19 && (pixel[3] - 0.25).abs() < 0.000001, "HDR data must not be quantized/clamped: {pixel:?}");
}

#[test]
fn sample_color_v79_transparent_rgb_does_not_pollute_average_and_edges_use_actual_area() {
    let mut s = fixture(32, "rgb");
    s.edit("Synthetic hidden RGB", |doc, active| {
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surface.fill_rect(Rect::new(0, 0, 8, 8), &[1.0, 0.0, 0.0, 0.0]);
        surface.write_pixel(0, 0, &[0.0, 0.0, 1.0, 1.0]);
        Ok(())
    })
    .unwrap();
    let center = sample(&mut s, 1.0, 1.0, 3, "current");
    assert_eq!(&center[..3], &[0.0, 0.0, 1.0]);
    assert!((center[3] - 1.0 / 9.0).abs() < 0.000001);
    assert_eq!(sample(&mut s, 0.0, 0.0, 3, "current"), [0.0, 0.0, 1.0, 0.25]);
    assert_eq!(sample(&mut s, -1.0, 0.0, 3, "current"), [0.0, 0.0, 1.0, 0.5]);
    assert_eq!(sample(&mut s, -10.0, -10.0, 1, "all"), [0.0; 4]);
}

#[test]
fn sample_color_v79_five_sources_respect_adjustments_opacity_and_above_below_order() {
    let mut s = fixture(32, "rgb");
    s.edit("Synthetic source stack", |doc, active| {
        let fmt = doc.pixel_format();
        let bottom = fill_layer("Red", fmt, [1.0, 0.0, 0.0, 1.0]);
        let lower_invert = Layer::new("Below adjustment", LayerContent::Adjustment(Adjustment::Invert));
        let mut current = fill_layer("Current blue", fmt, [0.0, 0.0, 1.0, 1.0]);
        current.opacity = 0.5;
        *active = Some(current.id);
        let above = fill_layer("Above green", fmt, [0.0, 1.0, 0.0, 1.0]);
        let top_invert = Layer::new("Top adjustment", LayerContent::Adjustment(Adjustment::Invert));
        doc.layers = vec![bottom, lower_invert, current, above, top_invert];
        Ok(())
    })
    .unwrap();
    assert_eq!(sample(&mut s, 10.0, 10.0, 1, "current"), [0.0, 0.0, 1.0, 1.0]);
    let below = sample(&mut s, 10.0, 10.0, 1, "currentBelow");
    assert!(below[0] < 0.01 && below[1] > 0.1 && below[2] > 0.99, "lower adjustment participates: {below:?}");
    let below_without = sample(&mut s, 10.0, 10.0, 1, "currentBelowNoAdjustments");
    assert!(below_without[0] > 0.1 && below_without[1] < 0.01 && below_without[2] > 0.1, "adjustments skipped: {below_without:?}");
    let all = sample(&mut s, 10.0, 10.0, 1, "all");
    assert!(all[0] > 0.99 && all[1] < 0.01 && all[2] > 0.99);
    assert_eq!(sample(&mut s, 10.0, 10.0, 1, "allNoAdjustments"), [0.0, 1.0, 0.0, 1.0]);
}

#[test]
fn sample_color_v79_current_group_keeps_children_and_below_respects_ancestor_visibility() {
    let mut s = fixture(8, "rgb");
    let (outer, group) = s
        .edit("Synthetic nested source", |doc, active| {
            let fmt = doc.pixel_format();
            let current = Layer::group("Current group", vec![fill_layer("Yellow child", fmt, [1.0, 1.0, 0.0, 1.0])]);
            let group = current.id;
            let outer = Layer::group(
                "Outer",
                vec![fill_layer("Lower blue sibling", fmt, [0.0, 0.0, 1.0, 1.0]), current, fill_layer("Above magenta sibling", fmt, [1.0, 0.0, 1.0, 1.0])],
            );
            let id = outer.id;
            doc.layers = vec![fill_layer("Lower root red", fmt, [1.0, 0.0, 0.0, 1.0]), outer, fill_layer("Above root green", fmt, [0.0, 1.0, 0.0, 1.0])];
            *active = Some(group);
            Ok((id, group))
        })
        .unwrap();
    assert_eq!(sample(&mut s, 10.0, 10.0, 1, "current"), [1.0, 1.0, 0.0, 1.0]);
    assert_eq!(sample(&mut s, 10.0, 10.0, 1, "currentBelow"), [1.0, 1.0, 0.0, 1.0], "a current group includes its child stack");
    assert_eq!(sample(&mut s, 10.0, 10.0, 1, "all"), [0.0, 1.0, 0.0, 1.0]);
    s.execute("layer.setProps", json!({"layer":outer.0,"visible":false})).unwrap();
    assert_eq!(sample(&mut s, 10.0, 10.0, 1, "currentBelow"), [1.0, 0.0, 0.0, 1.0], "hidden ancestor removes its contents from composite sources");
    assert_eq!(s.active().unwrap().active_layer, Some(group));
}

#[test]
fn sample_color_v79_queries_and_malformed_inputs_do_not_edit_documents_or_tool_state() {
    let mut s = fixture(16, "rgb");
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    let revision = s.active().unwrap().revision;
    let brush = s.tools.brush.clone();
    for size in SIZES {
        let _ = sample(&mut s, 10.0, 10.0, size, "all");
    }
    for request in [
        Value::Null,
        json!({}),
        json!({"x":0,"y":0,"size":2}),
        json!({"x":0,"y":0,"size":1.5}),
        json!({"x":0,"y":0,"size":null}),
        json!({"x":1e100,"y":0}),
        json!({"x":"1","y":0}),
        json!({"x":0,"y":0,"source":"bad"}),
        json!({"x":0,"y":0,"layer":u64::MAX}),
        json!({"x":0,"y":0,"unexpected":true}),
    ] {
        assert!(s.execute(COMMAND, request).is_err());
    }
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &before));
    assert_eq!(s.active().unwrap().history.past_len(), history);
    assert_eq!(s.active().unwrap().revision, revision);
    assert_eq!(s.tools.brush, brush);
}

fn striped_mixer() -> Session {
    let mut s = fixture(32, "rgb");
    s.edit("Synthetic red/blue band", |doc, active| {
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        surface.fill_rect(Rect::new(0, 0, 80, 60), &[1.0, 0.0, 0.0, 1.0]);
        surface.fill_rect(Rect::new(30, 0, 50, 60), &[0.0, 0.0, 1.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s.tools.brush.size = 135.0;
    s.tools.brush.angle = 37.0;
    s.tools.brush.roundness = 1.0;
    s.tools.brush.mixer.load_solid_only = false;
    s
}

#[test]
fn sample_color_v79_mixer_point_is_independent_of_tip_size_and_legacy_footprint_still_works() {
    let mut s = striped_mixer();
    let brush = s.tools.brush.clone();
    s.execute("tools.mixer.load", json!({"at":[40,30]})).unwrap();
    assert!(s.tools.mixer.loaded_nib.iter().any(|p| p[0] > 0.9), "omitted sampleSize keeps the large legacy footprint");
    s.execute("tools.mixer.load", json!({"at":[40,30],"sampleSize":1,"sampleAllLayers":true})).unwrap();
    assert!(!s.tools.mixer.loaded_nib.is_empty());
    assert!(s.tools.mixer.loaded_nib.iter().all(|p| *p == [0.0, 0.0, 1.0, 1.0]), "point sample never captures the surrounding red band");
    assert_eq!(s.tools.brush, brush, "sampling controls do not rewrite the preset");
}

#[test]
fn sample_color_v79_mixer_multicolor_grid_and_solid_average_remain_distinct() {
    let mut s = striped_mixer();
    s.tools.brush.angle = 0.0;
    s.execute("tools.mixer.load", json!({"at":[30,30],"sampleSize":3,"sampleAllLayers":false})).unwrap();
    assert!(s.tools.mixer.reservoir.is_none());
    assert!(s.tools.mixer.loaded_nib.iter().any(|p| p[0] > 0.99));
    assert!(s.tools.mixer.loaded_nib.iter().any(|p| p[2] > 0.99));
    s.tools.brush.mixer.load_solid_only = true;
    s.execute("tools.mixer.load", json!({"at":[30,30],"sampleSize":3,"sampleAllLayers":false})).unwrap();
    assert!(s.tools.mixer.loaded_nib.is_empty());
    let color = s.tools.mixer.reservoir.unwrap();
    assert!((color[0] - 1.0 / 3.0).abs() < 0.00001 && (color[2] - 2.0 / 3.0).abs() < 0.00001, "solid mode averages only the requested 3x3 region: {color:?}");
}

#[test]
fn sample_color_v79_mixer_source_override_and_invalid_inputs_preserve_brush_and_document() {
    let mut s = striped_mixer();
    s.execute("layer.new.layer", json!({"name":"Empty sampling target"})).unwrap();
    let brush = s.tools.brush.clone();
    s.execute("tools.mixer.load", json!({"at":[40,30],"sampleSize":1,"sampleAllLayers":false})).unwrap();
    assert!(s.tools.mixer.loaded_nib.iter().all(|p| p[3] == 0.0));
    s.execute("tools.mixer.load", json!({"at":[40,30],"sampleSize":1,"sampleAllLayers":true})).unwrap();
    assert!(s.tools.mixer.loaded_nib.iter().all(|p| p[2] > 0.99 && p[3] > 0.99));
    let state = s.tools.mixer.clone();
    let doc = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    for request in [
        json!({"at":[0,0],"sampleSize":2}),
        json!({"at":[0,0],"sampleSize":"3"}),
        json!({"at":[0,0],"sampleAllLayers":"yes"}),
        json!({"at":[0,0],"source":"bad"}),
        json!({"at":[0,0],"source":"all","sampleAllLayers":true}),
        json!({"at":[1e100,0]}),
        json!({"clean":"true"}),
    ] {
        assert!(s.execute("tools.mixer.load", request).is_err());
        assert_eq!(s.tools.mixer, state);
    }
    assert_eq!(s.tools.brush, brush);
    assert!(Arc::ptr_eq(&s.active().unwrap().doc, &doc));
    assert_eq!(s.active().unwrap().history.past_len(), history);
}
