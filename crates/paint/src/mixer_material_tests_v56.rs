//! Controlled Mixer regressions from explicit wetBrushTool experiments.
use super::*;
use crate::{GrayTile, Stroke, TipShape, mixer::apply_mixer_stroke, render_stroke, wet_paint::apply_spatial_mixer};

fn loaded(color: [f32; 4]) -> MixerState {
    let mut state = MixerState::default();
    state.load(color);
    state
}

fn soft_stroke(spacing: f32) -> Stroke {
    Stroke {
        brush: BrushSettings {
            size: 40.,
            spacing,
            pressure_size: false,
            pressure_opacity: false,
            tip: TipShape::Sampled(GrayTile::from_fn(8, 8, |_, _| 0.2)),
            ..Default::default()
        },
        points: (0..=40).map(|i| StrokePoint::new(40. + f64::from(i) * 4., 50., 1.)).collect(),
    }
}

#[test]
fn dry_density_matches_ordinary_contact_across_depths_and_partitions() {
    let area = Rect::new(0, 0, 240, 100);
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        for spacing in [0.01, 0.25] {
            let stroke = soft_stroke(spacing);
            let settings = MixerSettings { wet: 0., flow: 0.6, load: 1., mix: 1., ..Default::default() };
            let initial = Surface::new(fmt);
            let mut full = initial.clone();
            let mut state = loaded([1., 0., 0., 1.]);
            apply_spatial_mixer(&mut full, None, &stroke, &settings, &mut state, None, false);
            let mut ordinary = initial.clone();
            let mut brush = stroke.brush.clone();
            brush.flow = settings.flow;
            brush.color = [1., 0., 0., 1.];
            render_stroke(&mut ordinary, &brush, &stroke.points, None, false, 1.);
            let dry_pixels = full.read_region(area);
            let ordinary_pixels = ordinary.read_region(area);
            if fmt.sample == SampleType::F32 {
                // The coverage buffer and direct pixel-over path have different
                // float operation order. The diagnosed fixture differed only
                // in alpha (max 2*f32::EPSILON); keep native integer output and
                // RGB exact, and allow just 4 epsilon in this cross-model check.
                let mut alpha_error = 0.0f32;
                for (channel, (dry, ordinary)) in dry_pixels.iter().zip(&ordinary_pixels).enumerate() {
                    if channel % 4 == 3 {
                        alpha_error = alpha_error.max((dry - ordinary).abs());
                    } else {
                        assert_eq!(dry.to_bits(), ordinary.to_bits(), "ordinary RGB changed: {spacing}/{channel}");
                    }
                }
                assert!(alpha_error <= 4.0 * f32::EPSILON, "dry/ordinary alpha error={alpha_error}, spacing={spacing}");
            } else {
                assert_eq!(dry_pixels, ordinary_pixels, "dry contact differs from per-dab over: {fmt:?}/{spacing}");
            }
            for chunk in [1, 7, 41] {
                let mut actual = initial.clone();
                let mut live = MixerStroke::new(&initial, None, &stroke.brush, &settings, loaded([1., 0., 0., 1.]));
                for pts in stroke.points.chunks(chunk) {
                    live.push(pts, &mut actual, None, false);
                }
                live.finish(&mut actual, None, false);
                assert_eq!(actual.read_region(area), full.read_region(area), "dry partition mismatch: {fmt:?}/{spacing}/{chunk}");
                assert_eq!(state, live.state);
            }
        }
    }
}

#[test]
fn empty_canvas_mix100_uses_loaded_dry_contact_across_depths_and_partitions() {
    let area = Rect::new(0, 0, 240, 100);
    let stroke = soft_stroke(0.01);
    let dry = MixerSettings { wet: 0., mix: 1., load: 1., flow: 0.7, ..Default::default() };
    let wet = MixerSettings { wet: 1., ..dry.clone() };
    let gray = [115. / 255., 115. / 255., 115. / 255., 1.];
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let initial = Surface::new(fmt);
        let mut expected = initial.clone();
        apply_spatial_mixer(&mut expected, None, &stroke, &dry, &mut loaded(gray), None, false);
        let mut actual = initial.clone();
        let mut expected_state = loaded(gray);
        // An explicit empty sample isolates the no-pigment boundary from re-entry.
        apply_spatial_mixer(&mut actual, Some(&initial), &stroke, &wet, &mut expected_state, None, false);
        assert_eq!(actual.read_region(area), expected.read_region(area), "empty pickup must retain loaded contact: {fmt:?}");
        assert!(actual.rgba(120, 50)[3] > 0.99);
        for chunk in [1, 7, 41] {
            let mut partitioned = initial.clone();
            let mut live = MixerStroke::new(&initial, Some(&initial), &stroke.brush, &wet, loaded(gray));
            for pts in stroke.points.chunks(chunk) {
                live.push(pts, &mut partitioned, None, false);
            }
            live.finish(&mut partitioned, None, false);
            assert_eq!(partitioned.read_region(area), actual.read_region(area), "fallback partition mismatch: {fmt:?}/{chunk}");
            assert_eq!(live.state, expected_state);
        }
        let mut blue = Surface::new(fmt);
        blue.fill_rect(area, &[0., 0., 1., 1.]);
        let original = blue.clone();
        apply_spatial_mixer(&mut blue, None, &stroke, &wet, &mut loaded(gray), None, false);
        assert_eq!(blue.read_region(area), original.read_region(area), "positive pickup at Mix100 must remain canvas-only: {fmt:?}");
    }
}

#[test]
fn empty_loaded_and_pickup_wells_never_create_pigment() {
    let stroke = soft_stroke(0.1);
    let area = Rect::new(0, 0, 240, 100);
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        for multicolor in [false, true] {
            for wet in [0., 1.] {
                for mut state in [MixerState::default(), loaded([1., 0., 0., 0.]), MixerState { pickup: Some([0., 1., 0., 0.]), ..Default::default() }] {
                    let mut target = Surface::new(fmt);
                    let before = target.read_region(area);
                    let settings = MixerSettings { wet, mix: 1., multicolor, ..Default::default() };
                    apply_mixer_stroke(&mut target, None, &stroke, &settings, &mut state, None, false);
                    assert_eq!(target.read_region(area), before, "empty wells produced paint: {fmt:?}/{multicolor}/{wet}");
                }
            }
        }
    }
}

#[test]
fn carried_pickup_survives_a_blank_canvas_and_beats_loaded_at_mix100() {
    let mut stroke = soft_stroke(0.25);
    stroke.points = vec![StrokePoint::new(80., 50., 1.)];
    let area = Rect::new(0, 0, 240, 100);
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let initial = Surface::new(fmt);
        let mut state = loaded([0.4, 0.4, 0.4, 1.]);
        state.nib = vec![[1., 0., 0., 0.7]; pickup_side(stroke.brush.size).pow(2)];
        let carried = state.nib.clone();
        let mut target = initial.clone();
        let settings = MixerSettings { wet: 1., mix: 1., load: 1., flow: 1., ..Default::default() };
        apply_spatial_mixer(&mut target, Some(&initial), &stroke, &settings, &mut state, None, false);
        let center = target.rgba(80, 50);
        assert!(center[3] > 0. && center[0] > 0.999 && center[1] < 1e-6 && center[2] < 1e-6, "loaded gray replaced carried red: {center:?}");
        assert_eq!(state.nib, carried, "an empty sample cleared existing pickup");
        assert!(target.content_bounds().intersect(&area).width() > 0);

        let mut scalar = loaded([0.4, 0.4, 0.4, 1.]);
        scalar.pickup = Some([1., 0., 0., 0.7]);
        let mut target = initial;
        apply_mixer_stroke(&mut target, None, &stroke, &MixerSettings { multicolor: false, ..settings }, &mut scalar, None, false);
        let center = target.rgba(80, 50);
        assert!(center[0] > 0.999 && center[1] < 1e-6 && center[3] > 0., "scalar carried pickup was lost: {center:?}");
    }
}

#[test]
fn nonzero_half_alpha_pickup_keeps_the_existing_wet_weight() {
    let mut initial = Surface::new(PixelFormat::RGBA32F);
    initial.fill_rect(Rect::new(0, 0, 80, 80), &[0., 0., 1., 0.5]);
    let stroke = Stroke {
        brush: BrushSettings { size: 20., hardness: 1., pressure_size: false, pressure_opacity: false, ..Default::default() },
        points: vec![StrokePoint::new(40., 40., 1.)],
    };
    let mut actual = initial.clone();
    let settings = MixerSettings { wet: 1., mix: 1., flow: 0.4, ..Default::default() };
    apply_spatial_mixer(&mut actual, Some(&initial), &stroke, &settings, &mut loaded([1., 0., 0., 1.]), None, false);
    let center = actual.rgba(40, 40);
    // Existing positive pickup deposits .4 * .5 on the initial .5 alpha;
    // the new no-pigment fallback must not introduce loaded red or full opacity.
    assert!(center[0] < 1e-6 && center[2] > 0.999 && (center[3] - 0.6).abs() < 1e-6, "{center:?}");
}
