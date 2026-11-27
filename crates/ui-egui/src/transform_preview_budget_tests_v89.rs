//! Staged v89 transform-preview regressions. Synthetic data only; no user presets/documents.
use super::*;
use crate::transform_tex::{MAX_PREVIEW_BYTES, MAX_PREVIEW_PIXELS, MAX_SOURCE_PIXELS, PreviewPlan, PreviewTextures, read_surface, texture_limit};
use photocraft_color::{BlendMode, Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Layer, LayerMask, SmartFilter, SmartObject, SmartSource};
use photocraft_geom::{Affine, Rect, Size};
use photocraft_raster::Surface;

fn context(limit: usize) -> egui::Context {
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(egui::RawInput { max_texture_side: Some(limit), ..Default::default() }, |_| {});
    output.textures_delta.clear();
    ctx
}

fn patterned(format: PixelFormat, b: Rect) -> Surface {
    let mut surface = Surface::new(format);
    let mut pixels = Vec::new();
    for y in b.y0..b.y1 {
        for x in b.x0..b.x1 {
            let seed = (x * 37 + y * 17).rem_euclid(251) as f32 / 250.0;
            for c in 0..format.mode.color_channels() {
                pixels.push((seed + c as f32 * 0.173).fract());
            }
            if format.alpha {
                pixels.push(match (x + 3 * y).rem_euclid(5) {
                    0 => 0.0,
                    1 => 1.0,
                    _ => 0.371,
                });
            }
        }
    }
    surface.write_region(b, &pixels);
    surface
}

fn smart(surface: Surface) -> Layer {
    Layer::new(
        "cached",
        LayerContent::Smart(SmartObject::new(
            SmartSource::Embedded { file_name: "synthetic.png".into(), bytes: Arc::new(Vec::new()) },
            Affine::IDENTITY,
            Some(surface),
        )),
    )
}

fn document(layer: Layer, mode: ColorMode, depth: SampleType) -> Document {
    let mut doc = Document::new("synthetic preview", Size::new(384, 384), mode, depth);
    doc.layers.push(layer);
    doc
}

/// Independent pre-v89 reference: full float render -> second float conversion -> RGBA8 surface.
/// Keep the expensive oracle small. It intentionally does not use the new banded fallback.
fn old_composed(doc: &Document, id: LayerId, b: Rect, side: usize) -> (egui::ColorImage, [f32; 2]) {
    let mut solo = doc.clone();
    for l in &mut solo.layers {
        if l.id != id && !contains(l, id) {
            l.visible = false;
        }
    }
    let buf = photocraft_compose::render(&solo, b);
    let mut s = Surface::new(PixelFormat::RGBA8);
    let floats: Vec<f32> = buf.px.iter().flat_map(|p| *p).collect();
    s.write_region(b, &floats);
    read_surface(&s, None, b, side).unwrap()
}

fn assert_same(a: &(egui::ColorImage, [f32; 2]), b: &(egui::ColorImage, [f32; 2])) {
    assert_eq!(a.0.size, b.0.size);
    assert_eq!(a.1, b.1, "UV must preserve document-space odd-edge geometry");
    assert_eq!(a.0.pixels, b.0.pixels, "colour and alpha must match at every texel");
}

#[test]
fn plain_smart_matches_raster_and_old_oracle_at_several_depths_and_offsets() {
    // More than one compositor band, negative x, nonzero y and odd extents.
    let b = Rect::new(-7, 11, 310, 284);
    for depth in SampleType::ALL {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
            for alpha in [true, false] {
                let fmt = PixelFormat::new(mode, depth, alpha);
                let surface = patterned(fmt, b);
                let mut doc = document(smart(surface.clone()), mode, depth);
                let smart_id = doc.layers[0].id;
                assert!(plain_smart_surface(&doc, &doc.layers[0]).is_some());
                for side in [4096, 128] {
                    let from_smart = preview_image(&doc, smart_id, None, b, side).unwrap();
                    let oracle = old_composed(&doc, smart_id, b, side);
                    assert_same(&from_smart, &oracle);
                    let raster = Layer::new("raster", LayerContent::Raster(surface.clone()));
                    let raster_id = raster.id;
                    doc.layers.push(raster);
                    assert_same(&from_smart, &preview_image(&doc, raster_id, None, b, side).unwrap());
                    doc.layers.pop();
                }
            }
        }
    }
}

#[test]
fn composed_preview_keeps_masks_filters_blends_opacity_and_non_rgb_depths() {
    use photocraft_doc::blend_if::BlendRange;
    use photocraft_doc::effects::{Effect, FxCommon};
    use photocraft_doc::vector::{Path, Subpath, VectorMask};
    let b = Rect::new(-9, 7, 292, 272);
    for depth in SampleType::ALL {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            let fmt = PixelFormat::new(mode, depth, true);
            let base = smart(patterned(fmt, b));
            for case in 0..9 {
                let mut layer = base.clone();
                match case {
                    0 => {
                        layer.opacity = 0.613;
                        layer.fill_opacity = 0.781;
                    }
                    1 => {
                        let mut mask = LayerMask::reveal_all();
                        mask.surface.fill_rect(Rect::new(9, 29, 103, 211), &[0.17]);
                        mask.density = 0.73;
                        mask.feather = 1.7;
                        layer.mask = Some(mask);
                    }
                    2 => {
                        let path = Path::new(vec![Subpath::polygon(&[(-3.0, 11.0), (201.0, 23.0), (51.0, 255.0)])]);
                        layer.vector_mask = Some(VectorMask::new(path));
                    }
                    3 => {
                        layer.blend = BlendMode::Multiply;
                    }
                    4 => {
                        layer.excluded_channels = 1;
                    }
                    5 => {
                        layer.blend_if.set(0, [BlendRange::from_bytes([12, 71, 189, 242]), BlendRange::FULL]);
                    }
                    6 => {
                        layer.effects.items.push(Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 0.37), color: Color::rgb(0.89, 0.17, 0.43) });
                    }
                    7 => {
                        if let LayerContent::Smart(s) = &mut layer.content {
                            s.smart_filters.push(SmartFilter {
                                command: "filter.blur.gaussianBlur".into(),
                                params: json!({"radius": 1}),
                                blend: BlendMode::Normal,
                                opacity: 0.7,
                                visible: true,
                            });
                        }
                    }
                    _ => {
                        layer.clipped = true;
                    }
                }
                let id = layer.id;
                let doc = document(layer, mode, depth);
                assert!(plain_smart_surface(&doc, &doc.layers[0]).is_none());
                for side in [4096, 127] {
                    assert_same(&preview_image(&doc, id, None, b, side).unwrap(), &old_composed(&doc, id, b, side));
                }
            }
        }
    }
}

#[test]
fn shape_and_group_previews_keep_the_composed_path_and_ancestor_semantics() {
    use photocraft_doc::vector::ShapeLayer;
    let b = Rect::new(-5, -3, 40, 278);
    let mut sh = Layer::new("shape", LayerContent::Shape(ShapeLayer { cache: Some(patterned(PixelFormat::RGBA16, b)), ..Default::default() }));
    sh.opacity = 0.43;
    let doc = document(sh.clone(), ColorMode::Rgb, SampleType::U16);
    assert_same(&preview_image(&doc, sh.id, None, b, 128).unwrap(), &old_composed(&doc, sh.id, b, 128));
    let child = smart(patterned(PixelFormat::RGBA32F, b));
    let child_id = child.id;
    let mut group = Layer::group("ancestor", vec![child]);
    group.opacity = 0.53;
    let group_id = group.id;
    let doc = document(group, ColorMode::Rgb, SampleType::F32);
    assert!(plain_smart_surface(&doc, doc.layer(child_id).unwrap()).is_none());
    for id in [group_id, child_id] {
        assert_same(&preview_image(&doc, id, None, b, 128).unwrap(), &old_composed(&doc, id, b, 128));
    }
}

#[test]
fn independent_box_filter_matches_alpha_odd_edge_padding_and_mask_density() {
    let b = Rect::new(-2, 5, 3, 8);
    let surface = patterned(PixelFormat::RGBA16, b);
    let mut mask = LayerMask::reveal_all();
    mask.density = 0.69;
    mask.surface.fill_rect(Rect::new(-2, 5, 0, 8), &[0.2]);
    let moving = crate::transform_tex::PreviewMask { surface: mask.surface.clone(), density: mask.density };
    let got = read_surface(&surface, Some(&moving), b, 3).unwrap();
    assert_eq!(got.0.size, [3, 2]);
    assert_eq!(got.1, [5.0 / 6.0, 3.0 / 4.0]);
    let mut expected = Vec::new();
    for ty in 0..2 {
        for tx in 0..3 {
            let mut sums = [0u64; 4];
            let mut count = 0u64;
            for y in (b.y0 + ty * 2)..(b.y0 + (ty + 1) * 2).min(b.y1) {
                for x in (b.x0 + tx * 2)..(b.x0 + (tx + 1) * 2).min(b.x1) {
                    let mut rgba = [[0u8; 4]];
                    surface.read_rgba8_into(Rect::new(x, y, x + 1, y + 1), &mut rgba);
                    let [r, g, blue, a] = rgba[0];
                    let a = (f32::from(a) * mask.value(x, y)).round() as u8;
                    let premul = Color32::from_rgba_unmultiplied(r, g, blue, a).to_array();
                    for (sum, value) in sums.iter_mut().zip(premul) {
                        *sum += u64::from(value);
                    }
                    count += 1;
                }
            }
            let [r, g, blue, a] = sums.map(|sum| ((sum + count / 2) / count) as u8);
            expected.push(Color32::from_rgba_premultiplied(r, g, blue, a));
        }
    }
    assert_eq!(got.0.pixels, expected);
}

#[test]
fn budgets_reject_hostile_bounds_without_allocating_and_keep_24mp_one_to_one() {
    let ordinary = PreviewPlan::new(Rect::new(0, 0, 6000, 4000), 8192).unwrap();
    assert_eq!(ordinary.factor, 1);
    assert_eq!(ordinary.size, [6000, 4000]);
    assert_eq!(ordinary.uv, [1.0, 1.0]);
    assert!(ordinary.check_extra(MAX_PREVIEW_BYTES).is_err());
    assert!(ordinary.check_extra(u64::MAX).is_err(), "checked addition must reject overflow");
    let (mut low, mut high) = (0u64, MAX_PREVIEW_BYTES);
    while low + 1 < high {
        let mid = low + (high - low) / 2;
        if ordinary.check_extra(mid).is_ok() {
            low = mid;
        } else {
            high = mid;
        }
    }
    assert!(ordinary.check_extra(low).is_ok());
    assert!(ordinary.check_extra(low + 1).is_err(), "the exact byte-budget boundary");
    let full = Rect::new(0, 0, 6000, 4000);
    assert!(preflight_split(full, full, 8192, 4, 1, 0).is_ok(), "24 MP 8-bit selection fits without double-counting dead float buffers");
    assert!(preflight_split(full, full, 8192, 4, 4, 0).is_err(), "float-depth split must account for native storage too");
    assert!(PreviewPlan::new(Rect::new(0, 0, 8192, 8192), 8192).is_err(), "total pixels, not only side length");
    assert!(PreviewPlan::new(Rect::new(0, 0, 100_000_000, 1), 1024).is_err(), "independent source budget");
    assert!(PreviewPlan::new(Rect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX), 8192).is_err());
    assert!(PreviewPlan::new(Rect::new(0, 0, 8, 8), 0).is_err());
    let exact = PreviewPlan::new(Rect::new(0, 0, 8192, 4096), 8192).unwrap();
    assert_eq!(exact.source_pixels, MAX_PREVIEW_PIXELS);
    let wide = PreviewPlan::new(Rect::new(0, 0, 16384, 4096), 8192).unwrap();
    assert_eq!(wide.source_pixels, MAX_SOURCE_PIXELS);
    assert_eq!(wide.size, [8192, 2048]);
    let sparse = Surface::new(PixelFormat::RGBA8);
    assert!(read_surface(&sparse, None, Rect::new(0, 0, 8193, 8193), 8192).is_err());
}

#[test]
fn texture_limit_uses_the_smaller_actual_limit_and_errors_before_upload() {
    // epaint needs a viable font atlas even though this frame has no visible widgets.
    // Exercise smaller device limits without shrinking the context's font-atlas limit.
    let ctx = context(2048);
    assert_eq!(texture_limit(&ctx, Some(8192)).unwrap(), 2048);
    assert_eq!(texture_limit(&ctx, Some(64)).unwrap(), 64);
    assert!(texture_limit(&ctx, Some(0)).is_err());
    let wrong = egui::ColorImage::new([2049, 1], vec![Color32::WHITE; 2049]);
    assert!(PreviewTextures::new(&ctx, "context-oversize".into(), wrong, [1.0, 1.0], 8192).is_err());
    let device_wrong = egui::ColorImage::new([65, 1], vec![Color32::WHITE; 65]);
    assert!(PreviewTextures::new(&ctx, "device-oversize".into(), device_wrong, [1.0, 1.0], 64).is_err());
    let image = egui::ColorImage::new([2, 2], vec![Color32::WHITE; 4]);
    assert!(PreviewTextures::new(&ctx, "bad-uv".into(), image, [f32::NAN, 1.0], 64).is_err());
    let mut out = ctx.run_ui(Default::default(), |_| {});
    assert!(out.textures_delta.set.is_empty(), "rejected image never reached load_texture");
    out.textures_delta.clear();
    // Check a zero context limit directly, without starting a zero-sized font atlas.
    let zero_context = egui::Context::default();
    zero_context.input_mut(|input| input.max_texture_side = 0);
    assert!(texture_limit(&zero_context, None).is_err());
}

#[test]
fn rejected_begin_keeps_document_history_dirty_state_and_session_unmodified() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("sparse oversized layer", |doc, active| {
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            surface.fill_rect(Rect::new(0, 0, 1, 1), &[0.3, 0.6, 0.8, 1.0]);
            surface.fill_rect(Rect::new(8192, 8192, 8193, 8193), &[0.8, 0.6, 0.3, 1.0]);
            Ok(())
        })
        .unwrap();
    app.sync_views();
    let state = app.session.active().unwrap();
    let before = state.doc.clone();
    let history = state.history.past_len();
    let dirty = state.is_dirty();
    let ctx = context(16384);
    assert!(begin(&mut app, &ctx).is_err());
    let state = app.session.active().unwrap();
    assert_eq!(state.doc.as_ref(), before.as_ref());
    assert_eq!(state.history.past_len(), history);
    assert_eq!(state.is_dirty(), dirty);
    assert!(app.ui.transform.is_none());
    assert!(app.transform_preview.is_none());
    let mut out = ctx.run_ui(Default::default(), |_| {});
    assert!(out.textures_delta.set.is_empty());
    out.textures_delta.clear();
}

#[test]
fn placed_pixel_transform_retains_the_pixel_history_label() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("place fixture", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(3, 4, 16, 17), &[0.2, 0.4, 0.6, 1.0]);
            Ok(())
        })
        .unwrap();
    let ctx = context(2048);
    begin_placed(&mut app, &ctx).unwrap();
    let transform = app.ui.transform.as_mut().unwrap();
    transform.quad = transform.quad.map(|[x, y]| [x + 5.0, y + 3.0]);
    commit(&mut app);
    assert_eq!(app.session.active().unwrap().history.entries().last().map(String::as_str), Some(photocraft_engine::file_cmds::PLACE_PIXELS));
}

#[test]
fn skinny_mask_and_split_are_budgeted_as_whole_native_tiles() {
    use photocraft_doc::vector::{Path, Subpath, VectorMask};
    let b = Rect::new(0, 0, 2_000_000, 1);
    let mut layer = Layer::raster("sparse masked strip", PixelFormat::RGBA8);
    layer.vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (2_000_000.0, 0.0), (2_000_000.0, 1.0), (0.0, 1.0)])])));
    let id = layer.id;
    let doc = document(layer, ColorMode::Rgb, SampleType::U8);
    assert!(preview_preflight(&doc, id, b, 8192).is_err(), "mask's tiled native surface exceeds the byte budget");
    assert!(preflight_split(b, Rect::new(0, 0, 8, 1), 8192, 4, 1, 0).is_err(), "a tiny selection must not allocate a huge whole-source split");
}

#[test]
fn failed_placed_preview_rolls_back_the_made_step() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let history = app.session.active().unwrap().history.past_len();
    app.session
        .edit("place sparse oversized fixture", |doc, active| {
            let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
            surface.fill_rect(Rect::new(0, 0, 1, 1), &[1.0, 0.0, 0.0, 1.0]);
            surface.fill_rect(Rect::new(8192, 8192, 8193, 8193), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let ctx = context(16384);
    assert!(begin_placed(&mut app, &ctx).is_err());
    let state = app.session.active().unwrap();
    assert_eq!(state.doc.as_ref(), before.as_ref());
    assert_eq!(state.history.past_len(), history);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
}

#[test]
#[ignore = "isolated 24MP preview benchmark; INKPOP_PREVIEW_BENCH=legacy|current"]
fn preview_24mp_measurement_v89() {
    let mode = std::env::var("INKPOP_PREVIEW_BENCH").unwrap();
    let mut surface = Surface::new(PixelFormat::RGBA8);
    let b = Rect::new(0, 0, 6000, 4000);
    surface.fill_rect(b, &[0.2, 0.6, 0.9, 1.0]);
    let layer = smart(surface);
    let id = layer.id;
    let doc = document(layer, ColorMode::Rgb, SampleType::U8);
    println!("BENCH_READY");
    use std::io::Write;
    std::io::stdout().flush().unwrap();
    let started = std::time::Instant::now();
    let result = if mode == "legacy" { old_composed(&doc, id, b, 8192) } else { preview_image(&doc, id, None, b, 8192).unwrap() };
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(result.0.size, [6000, 4000]);
    assert_eq!(result.1, [1.0, 1.0]);
    assert_eq!(result.0.pixels[12_000_000], Color32::from_rgb(51, 153, 230));
    println!("BENCH_RESULT {}", json!({"mode":mode,"milliseconds":elapsed,"width":6000,"height":4000,"oneToOne":true}));
}
