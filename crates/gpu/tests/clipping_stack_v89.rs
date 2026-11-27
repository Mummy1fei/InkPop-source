//! Independent analytic oracle for a painted layer and two adjustments on one base.
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Adjustment, Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_gpu::{Compositor, render_to_vec};

fn fixture(sample: SampleType) -> Document {
    let mut doc = Document::new("clipping stack", Size::new(32, 24), ColorMode::Rgb, sample);
    let fmt = PixelFormat::new(ColorMode::Rgb, sample, true);
    let mut background = Layer::raster("background", fmt);
    background.surface_mut().unwrap().fill_rect(doc.bounds(), &[0.2, 0.3, 0.4, 1.0]);
    let mut base = Layer::raster("base", fmt);
    base.surface_mut().unwrap().fill_rect(Rect::new(8, 6, 24, 18), &[0.2, 0.4, 0.6, 0.5]);
    let mut paint = Layer::raster("paint beyond base", fmt);
    paint.surface_mut().unwrap().fill_rect(doc.bounds(), &[1.0, 0.0, 0.0, 0.5]);
    paint.clipped = true;
    let mut adjustment = Layer::new("invert once", LayerContent::Adjustment(Adjustment::Invert));
    adjustment.clipped = true;
    let mut adjustment2 = Layer::new("invert twice", LayerContent::Adjustment(Adjustment::Invert));
    adjustment2.clipped = true;
    doc.layers = vec![background, base, paint, adjustment, adjustment2];
    doc
}

fn assert_pixel(actual: [f32; 4], expected: [f32; 4], label: &str) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 3.0 / 255.0, "{label}: {actual} vs {expected}");
    }
}

#[test]
fn multiple_paint_and_adjustment_clips_share_base_alpha_cpu_gpu_v89() {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("GPU validation requires an adapter");
    eprintln!("clipping adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let mut comp = Compositor::try_new_with_format(&device, wgpu::TextureFormat::Rgba32Float).unwrap();
    for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut doc = fixture(sample);
        let original_paint = doc.layers[2].surface().unwrap().clone();
        for state in ["two adjustments", "one adjustment", "hidden base", "released adjustment"] {
            match state {
                "one adjustment" => doc.layers[4].visible = false,
                "hidden base" => doc.layers[1].visible = false,
                "released adjustment" => {
                    doc.layers[1].visible = true;
                    doc.layers[3].clipped = false;
                }
                _ => {}
            }
            let cpu = photocraft_compose::render(&doc, doc.bounds()).px;
            let gpu = render_to_vec(&mut comp, &device, &queue, &doc, doc.bounds()).unwrap();
            assert_eq!(cpu.len(), gpu.len());
            for (i, (c, g)) in cpu.iter().zip(&gpu).enumerate() {
                assert_pixel(*g, *c, &format!("{sample:?}/{state}/pixel{i}"));
            }
            let outside = cpu[2 * 32 + 2];
            let inside = cpu[10 * 32 + 12];
            match state {
                "two adjustments" => {
                    assert_pixel(outside, [0.2, 0.3, 0.4, 1.0], state);
                    assert_pixel(inside, [0.4, 0.25, 0.35, 1.0], state);
                }
                "one adjustment" => {
                    assert_pixel(outside, [0.2, 0.3, 0.4, 1.0], state);
                    assert_pixel(inside, [0.3, 0.55, 0.55, 1.0], state);
                }
                "hidden base" => assert_pixel(inside, [0.2, 0.3, 0.4, 1.0], state),
                "released adjustment" => {
                    assert_pixel(outside, [0.8, 0.7, 0.6, 1.0], state);
                    assert_pixel(inside, [0.6, 0.75, 0.65, 1.0], state);
                }
                other => panic!("unexpected clipping state: {other}"),
            }
            assert_eq!(doc.layers[2].surface().unwrap(), &original_paint, "clipping preserves source paint outside base");
        }
    }
}
