//! Regression oracles for local pickup lifetime, selection and delayed wet contact.
use super::*;
use crate::{Stroke, retouch::footprint, wet_paint::apply_spatial_mixer};
use photocraft_color::ColorMode;

fn loaded_state() -> MixerState {
    let mut state = MixerState::default();
    state.load([1., 0., 0., 1.]);
    state
}

#[test]
fn physical_contact_preserves_overlap_but_refreshes_reentered_paint() {
    let mut original = Surface::new(PixelFormat::RGBA32F);
    original.fill_rect(Rect::new(0, 0, 20, 8), &[0., 0., 1., 1.]);
    let settings = MixerSettings { simulate_paint: true, ..Default::default() };
    let mut live = MixerStroke::new(&original, None, &BrushSettings::default(), &settings, loaded_state());
    let mut work = Region::read(&original, Rect::new(0, 0, 20, 8));
    live.pickup_snapshot = Some(live.snapshot_at_contact(&work, Rect::new(0, 0, 8, 8), None, false));
    work.px_mut(4, 4).copy_from_slice(&[1., 0., 0., 1.]);
    let overlap = live.snapshot_at_contact(&work, Rect::new(2, 0, 10, 8), None, false);
    assert_eq!(overlap.px(4, 4), &[0., 0., 1., 1.], "do not re-sample immediately overlapping paint");
    live.pickup_snapshot = Some(live.snapshot_at_contact(&work, Rect::new(10, 0, 18, 8), None, false));
    let reentered = live.snapshot_at_contact(&work, Rect::new(0, 0, 8, 8), None, false);
    assert_eq!(reentered.px(4, 4), &[1., 0., 0., 1.], "revisited canvas must contain this stroke's paint");
}

#[test]
fn ordinary_contact_resamples_selected_visible_overlapping_paint_in_premultiplied_space() {
    let area = Rect::new(0, 0, 8, 8);
    let mut original = Surface::new(PixelFormat::RGBA32F);
    original.fill_rect(area, &[0., 0., 1., 0.25]);
    let mut live = MixerStroke::new(&original, None, &BrushSettings::default(), &MixerSettings::default(), loaded_state());
    live.pickup_snapshot = Some(Region::read(&original, area));
    let mut painted = original.clone();
    painted.fill_rect(area, &[1., 0., 0., 0.75]);
    let work = Region::read(&painted, area);
    let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    selection.fill_rect(Rect::new(0, 0, 4, 8), &[0.5]);
    let sample = live.snapshot_at_contact(&work, area, Some(&selection), false);
    assert_eq!(sample.px(6, 4), &[0., 0., 1., 0.25], "unselected internal paint must not be picked up");
    let c = sample.px(2, 4);
    assert!((c[0] - 0.75).abs() < 1e-6 && (c[2] - 0.25).abs() < 1e-6 && (c[3] - 0.5).abs() < 1e-6, "{c:?}");
}

#[test]
fn first_wet_contact_after_dry_pressure_sample_seeds_the_whole_nib() {
    let mut original = Surface::new(PixelFormat::RGBA32F);
    original.fill_rect(Rect::new(0, 0, 80, 80), &[0., 0., 1., 1.]);
    let brush = BrushSettings { size: 64., ..Default::default() };
    let mut live = MixerStroke::new(&original, None, &brush, &MixerSettings::default(), loaded_state());
    let source = Region::read(&original, Rect::new(0, 0, 80, 80));
    let mut work = source.clone();
    let mut dab = Dab::round(Point::new(30., 30.), 6., 1.);
    dab.wet = 0.;
    live.dab(&mut work, &footprint(&dab, 0, 1.), Some(&source), false);
    assert!(!live.pickup_seeded);
    dab.wet = 0.2;
    dab.center.x += 2.;
    live.dab(&mut work, &footprint(&dab, 1, 1.), Some(&source), false);
    assert!(live.pickup_seeded);
    // 12 px contact onto a 64x64 grid: forward binning alone leaves holes.
    for y in 20..44 {
        for x in 20..44 {
            assert!(live.state.nib[y * 64 + x][3] > 0.99, "empty carried cell {x}, {y}");
        }
    }
}

#[test]
fn returning_stroke_picks_up_its_own_deposited_color_but_external_sample_stays_authoritative() {
    let mut original = Surface::new(PixelFormat::RGBA32F);
    original.fill_rect(Rect::new(0, 0, 140, 140), &[0., 0., 1., 1.]);
    let brush = BrushSettings { size: 20., spacing: 0.1, hardness: 1., pressure_size: false, pressure_opacity: false, ..Default::default() };
    let stroke = Stroke {
        brush,
        points: [(20., 20.), (100., 20.), (100., 100.), (20., 100.), (20., 20.), (65., 20.)].map(|(x, y)| StrokePoint::new(x, y, 1.)).to_vec(),
    };
    let settings = MixerSettings { wet: 1., load: 1., mix: 0., ..Default::default() };
    let mut local = original.clone();
    let mut external = original.clone();
    let mut local_state = loaded_state();
    let mut external_state = loaded_state();
    apply_spatial_mixer(&mut local, None, &stroke, &settings, &mut local_state, None, false);
    apply_spatial_mixer(&mut external, Some(&original), &stroke, &settings, &mut external_state, None, false);
    assert!(local_state.pickup.unwrap()[0] > 0.03, "local pickup must contain the deposited red");
    assert!(external_state.pickup.unwrap()[0] < 1e-6, "explicit sample is still blue");
}

#[test]
fn self_crossing_pickup_matches_across_batches_depths_and_feathered_selection() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        let mut original = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
        let area = Rect::new(0, 0, 100, 100);
        original.fill_rect(area, &[0., 0., 1., 0.4]);
        let mut selection = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        selection.fill_rect(area, &[0.37]);
        let brush = BrushSettings { size: 16., spacing: 0.08, pressure_size: false, ..Default::default() };
        let stroke = Stroke {
            brush,
            points: [(20., 20.), (75., 20.), (75., 75.), (20., 75.), (20., 20.), (50., 20.)].map(|(x, y)| StrokePoint::new(x, y, 0.6)).to_vec(),
        };
        let settings = MixerSettings { wet: 0.8, mix: 0.25, flow: 0.7, ..Default::default() };
        let mut expected = original.clone();
        let mut expected_state = loaded_state();
        apply_spatial_mixer(&mut expected, None, &stroke, &settings, &mut expected_state, Some(&selection), false);
        for chunk in [1, 2, 4] {
            let mut actual = original.clone();
            let mut live = MixerStroke::new(&original, None, &stroke.brush, &settings, loaded_state());
            for points in stroke.points.chunks(chunk) {
                live.push(points, &mut actual, Some(&selection), false);
            }
            live.finish(&mut actual, Some(&selection), false);
            assert_eq!(actual.read_region(area), expected.read_region(area), "{depth:?}, chunk={chunk}");
            assert_eq!(live.state, expected_state);
        }
    }
}

#[test]
fn transfer_opacity_pressure_controls_wet_and_dry_deposition_once() {
    for wet in [0., 0.7] {
        for multicolor in [false, true] {
            for pressure in [0., 0.2, 1.] {
                let brush = BrushSettings {
                    size: 20.,
                    pressure_size: false,
                    hardness: 1.,
                    transfer: crate::Transfer { enabled: true, opacity: crate::Dynamic::controlled(crate::Control::PenPressure), ..Default::default() },
                    ..Default::default()
                };
                let stroke = Stroke { brush, points: vec![StrokePoint::new(20., 20., pressure)] };
                let settings = MixerSettings { wet, mix: 0., flow: 0.4, load: 1., multicolor, ..Default::default() };
                let mut target = Surface::new(PixelFormat::RGBA32F);
                crate::mixer::apply_mixer_stroke(&mut target, None, &stroke, &settings, &mut loaded_state(), None, false);
                let actual = target.rgba(20, 20)[3];
                assert!((actual - pressure * 0.4).abs() < 1e-5, "wet={wet}, spatial={multicolor}, pressure={pressure}: {actual}");
            }
        }
    }
}

#[test]
fn wet_brush_replenishes_loaded_colour_along_one_document_rgb_mixture() {
    // The bundled water/oil preset uses Wet=100 %, Load=100 %, Mix=0 % and a
    // pressure-controlled wetness transfer. A full-pressure stroke must still
    // visibly exchange canvas colour; otherwise the preset paints an isolated
    // opaque foreground stripe instead of a wet brush mark.
    let mut target = Surface::new(PixelFormat::RGBA32F);
    target.fill_rect(Rect::new(0, 0, 120, 32), &[0.0, 0.0, 1.0, 1.0]);
    let brush = BrushSettings {
        size: 18.,
        spacing: 0.1,
        hardness: 1.,
        pressure_size: false,
        pressure_opacity: false,
        transfer: crate::Transfer { enabled: true, wetness: crate::Dynamic::controlled(crate::Control::PenPressure), ..Default::default() },
        ..Default::default()
    };
    let stroke = Stroke { brush, points: vec![StrokePoint::new(14., 16., 1.), StrokePoint::new(106., 16., 1.)] };
    for mix in [0., 0.5, 1.] {
        let mut actual = target.clone();
        let settings = MixerSettings { wet: 1., load: 1., mix, flow: 1., ..Default::default() };
        let mut state = MixerState::default();
        state.load([1., 1., 0., 1.]);
        apply_spatial_mixer(&mut actual, None, &stroke, &settings, &mut state, None, false);
        let early = actual.rgba(25, 16);
        let late = actual.rgba(90, 16);
        for c in [early, late] {
            assert!((c[0] - c[1]).abs() < 1e-5 && (c[0] + c[2] - 1.).abs() < 1e-5, "channel pickup fractions differ: {c:?}");
        }
        if mix < 1. {
            assert!(early[0] > 0. && early[2] > 0.1 && late[0] > early[0] + 0.03, "mix={mix} early={early:?} late={late:?}");
        } else {
            assert_eq!(late, [0., 0., 1., 1.], "pure pickup must keep the uniform substrate");
        }
    }
}

#[test]
fn carried_pigment_on_transparency_refills_only_from_an_available_load() {
    let brush = BrushSettings { size: 18., spacing: 0.12, hardness: 1., pressure_size: false, pressure_opacity: false, ..Default::default() };
    let stroke = Stroke { brush, points: vec![StrokePoint::new(15., 16., 1.), StrokePoint::new(130., 16., 1.)] };
    let render = |mix, load, initial_load: bool| {
        let mut state = MixerState::default();
        if initial_load {
            state.load([1., 1., 0., 1.]);
        }
        state.nib = vec![[0., 0., 1., 1.]; pickup_side(stroke.brush.size).pow(2)];
        let mut target = Surface::new(PixelFormat::RGBA32F);
        let m = MixerSettings { wet: 1., load, mix, flow: 1., ..Default::default() };
        apply_spatial_mixer(&mut target, None, &stroke, &m, &mut state, None, false);
        (target.rgba(25, 16), target.rgba(115, 16), state.amount)
    };
    let (early, late, _) = render(0.25, 1., true);
    assert!(late[0] > early[0] + 0.15, "refill along the stroke: {early:?} to {late:?}");
    let (_, low, amount) = render(0.25, 0.1, true);
    assert!(amount < 0.05 && low[0] < late[0] - 0.1, "low reservoir: {low:?} vs {late:?}, left {amount}");
    for (mix, loaded) in [(1., true), (0.5, false)] {
        let (_, end, _) = render(mix, 1., loaded);
        assert!(end[2] > 0.999 && end[0] < 1e-6 && end[3] > 0.1, "pure pickup: {end:?}");
    }
}

#[test]
fn low_wet_stroke_retains_some_uniform_substrate_far_from_the_start() {
    let brush = BrushSettings { size: 18., spacing: 0.06, pressure_size: false, hardness: 1., ..Default::default() };
    let stroke = Stroke { brush, points: vec![StrokePoint::new(18., 16., 1.), StrokePoint::new(380., 16., 1.)] };
    for mix in [0., 0.5] {
        let mut target = Surface::new(PixelFormat::RGBA32F);
        target.fill_rect(Rect::new(0, 0, 400, 32), &[0., 0., 1., 1.]);
        let m = MixerSettings { wet: 0.25, load: 1., mix, flow: 1., ..Default::default() };
        apply_spatial_mixer(&mut target, None, &stroke, &m, &mut loaded_state(), None, false);
        let middle = target.rgba(250, 16);
        let end = target.rgba(350, 16);
        assert!(end[2] > 0.1 && end[0] > 0.4 && (middle[0] - end[0]).abs() < 0.04, "mix={mix} middle={middle:?}, end={end:?}");
    }
}
