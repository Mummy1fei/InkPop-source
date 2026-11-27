use super::*;
use crate::{BrushSettings, GrayTile, TipShape};

#[test]
fn pressure_flow_recharges_a_scattered_reservoir_instead_of_carrying_the_first_image_forever() {
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let mut surface = Surface::new(fmt);
        surface.fill_rect(Rect::new(0, 0, 40, 20), &[0., 0., 1., 1.]);
        surface.fill_rect(Rect::new(10, 0, 20, 20), &[1., 0., 0., 1.]);
        surface.fill_rect(Rect::new(20, 0, 40, 20), &[0., 1., 0., 1.]);
        let mut work = Region::read(&surface, Rect::new(0, 0, 40, 20));
        let mut brush = BrushSettings::default();
        brush.scattering.enabled = true;
        brush.scattering.scatter.jitter = 1.;
        let mut sm = Smudge::new(1., None, 4.).with_alpha(Some(3)).with_brush_bounds(&brush);
        for (i, x) in [5., 15., 25.].into_iter().enumerate() {
            let dab = crate::Dab::round(photocraft_geom::Point::new(x, 10.), 2., 0.5);
            sm.dab(&mut work, &footprint(&dab, i, 1.));
        }
        let final_pixel = work.px(25, 10);
        assert!(final_pixel[0] > 0.05, "red pickup was lost at partial pressure: {fmt:?}");
        assert!(final_pixel[1] > 0.1 && final_pixel[2] > 0.1);
        assert_eq!(final_pixel[3], 1.);
    }
}

#[test]
fn scattered_zero_flow_does_not_deposit_or_pick_up_pixels() {
    let brush = BrushSettings {
        scattering: crate::Scattering { enabled: true, scatter: crate::Dynamic { jitter: 1., ..Default::default() }, ..Default::default() },
        ..Default::default()
    };
    let mut work = Region::read(&split(PixelFormat::RGBA32F), Rect::new(0, 0, 400, 200));
    let before = work.clone();
    let mut sm = Smudge::new(1., None, 10.).with_alpha(Some(3)).with_brush_bounds(&brush);
    for (i, x) in [180., 220., 240.].into_iter().enumerate() {
        sm.dab(&mut work, &footprint(&crate::Dab::round(photocraft_geom::Point::new(x, 100.), 5., 0.), i, 1.));
    }
    assert_eq!(work, before);
    assert!(sm.carry.is_none() && sm.last_center.is_none());
}

fn stroke(spacing: f32, pressure: f32) -> Stroke {
    Stroke {
        brush: BrushSettings { size: 80., hardness: 1., spacing, pressure_size: false, pressure_opacity: false, ..Default::default() },
        points: (0..=80).map(|i| crate::StrokePoint::new(150. + f64::from(i) * 2.5, 100., pressure)).collect(),
    }
}
fn split(fmt: PixelFormat) -> Surface {
    let mut s = Surface::new(fmt);
    s.fill_rect(Rect::new(0, 0, 400, 200), &photocraft_raster::from_rgba(&fmt, [0., 0., 1., 1.]));
    s.fill_rect(Rect::new(200, 0, 400, 200), &photocraft_raster::from_rgba(&fmt, [1., 0., 0., 1.]));
    s
}
fn run(mut s: Surface, stroke: &Stroke, strength: f32) -> Surface {
    let mut sm = Smudge::new(strength, None, stroke.brush.size).with_alpha(alpha_index(&s.format())).with_brush_bounds(&stroke.brush);
    apply_smudge_stroke(&mut s, stroke, None, false, |work, fp| sm.dab(work, fp));
    s
}
#[test]
fn image_transport_advances_the_boundary_instead_of_only_fading_the_initial_color() {
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let target = run(split(fmt), &stroke(0.01, 1.), 0.5);
        // Independent hard-round PS exports: 50px past the original edge is
        // nearly blue, while 100px past it is predominantly the red canvas.
        assert!(target.rgba(250, 100)[2] > 0.9, "front was prematurely diluted: {fmt:?}");
        assert!(target.rgba(300, 100)[0] > 0.85, "front must not smear the whole stroke: {fmt:?}");
        assert_eq!(target.rgba(250, 100)[3], 1.);
    }
}
#[test]
fn coarse_tip_spacing_does_not_disconnect_smudge_transport() {
    let region = Rect::new(0, 0, 400, 200);
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F, PixelFormat::GRAYA8, PixelFormat::CMYKA8] {
        let initial = split(fmt);
        let reference = run(initial.clone(), &stroke(0.01, 1.), 0.5).read_region(region);
        for spacing in [0.25, 3.11] {
            let actual = run(initial.clone(), &stroke(spacing, 1.), 0.5);
            assert!(reference == actual.read_region(region), "spacing changed transport: {fmt:?}/{spacing}");
        }
    }
}
#[test]
fn pressure_opacity_controls_transport_and_zero_pressure_leaves_pixels_intact() {
    let region = Rect::new(0, 0, 400, 200);
    let initial = split(PixelFormat::RGBA32F);
    let mut st = stroke(0.01, 0.);
    st.brush.pressure_opacity = true;
    let actual = run(initial.clone(), &st, 1.);
    assert!(actual.read_region(region) == initial.read_region(region));
    for p in &mut st.points {
        p.pressure = 0.5;
    }
    let controlled = run(initial.clone(), &st, 1.);
    let reference = run(initial, &stroke(0.01, 1.), 0.5);
    assert!(controlled.read_region(region) == reference.read_region(region));
}

#[test]
fn smudge_pressure_transfer_controls_strength_instead_of_just_color_fade() {
    let area = Rect::new(0, 0, 400, 200);
    let initial = split(PixelFormat::RGBA32F);
    let mut st = stroke(0.01, 0.5);
    st.brush.transfer.enabled = true;
    st.brush.transfer.opacity.control = crate::Control::Off;
    st.brush.transfer.flow.control = crate::Control::PenPressure;
    st.brush.transfer.flow.minimum = 0.0;
    // PS Prs100 with prVr pen-pressure50 matches static Prs50, independently
    // exported in r5. Both alter the position of the same boundary.
    let controlled = run(initial.clone(), &st, 1.0);
    let reference = run(initial, &stroke(0.01, 1.0), 0.5);
    assert!(controlled.read_region(area) == reference.read_region(area));
}
#[test]
fn rotated_sampled_tip_keeps_its_corners_at_full_strength() {
    let fmt = PixelFormat::RGBA8;
    let mut initial = Surface::new(fmt);
    initial.fill_rect(Rect::new(0, 0, 400, 220), &[1., 0., 0., 1.]);
    initial.fill_rect(Rect::new(0, 0, 180, 220), &[0., 0., 1., 1.]);
    let st = Stroke {
        brush: BrushSettings {
            size: 100.,
            angle: 45.,
            tip: TipShape::Sampled(GrayTile::from_fn(8, 8, |_, _| 1.)),
            pressure_size: false,
            pressure_opacity: false,
            ..Default::default()
        },
        points: vec![crate::StrokePoint::new(100., 100., 1.), crate::StrokePoint::new(200., 100., 1.)],
    };
    let actual = run(initial, &st, 1.);
    assert!(actual.rgba(260, 100)[2] > 0.9, "rotated square corner was clipped by the carry buffer");
}
#[test]
fn transparent_hidden_rgb_cannot_contaminate_advected_colors() {
    for fmt in [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F] {
        let mut a = split(fmt);
        a.fill_rect(Rect::new(200, 0, 400, 200), &[0.; 4]);
        let mut b = a.clone();
        b.fill_rect(Rect::new(200, 0, 400, 200), &[1., 1., 0., 0.]);
        let a = run(a, &stroke(0.01, 1.), 0.5);
        let b = run(b, &stroke(0.01, 1.), 0.5);
        let mut touched = 0;
        for y in 60..140 {
            for x in 200..390 {
                let left = a.rgba(x, y);
                let right = b.rgba(x, y);
                assert_eq!(left[3], right[3]);
                if left[3] > 0.01 {
                    touched += 1;
                    assert!(left[2] > 0.999);
                    assert_eq!(left, right);
                }
            }
        }
        assert!(touched > 100);
    }
}

#[test]
fn returning_to_full_pressure_does_not_stamp_a_stale_image_from_before_the_ramp() {
    let mut surface = Surface::new(PixelFormat::RGBA32F);
    surface.fill_rect(Rect::new(0, 0, 240, 100), &[0.0, 1.0, 0.0, 1.0]);
    surface.fill_rect(Rect::new(0, 0, 90, 100), &[0.0, 0.0, 1.0, 1.0]);
    let mut work = Region::read(&surface, Rect::new(0, 0, 240, 100));
    let brush = BrushSettings { size: 20.0, pressure_size: false, ..Default::default() };
    let ctx = crate::BrushContext::new(&brush);
    let mut sm = Smudge::new(1.0, None, 20.0).with_alpha(Some(3));
    for x in 50..=200 {
        let mut dab = crate::Dab::round(photocraft_geom::Point::new(f64::from(x), 50.0), 10.0, 1.0);
        dab.opacity = if (80..=160).contains(&x) { 0.5 } else { 1.0 };
        sm.dab(&mut work, &ctx.footprint(&dab, x as usize));
    }
    assert!(work.px(190, 50)[1] > 0.99 && work.px(190, 50)[2] < 0.01, "full pressure must carry the recent green image, not the initial blue patch");
}

#[test]
fn uniform_hdr_and_tiny_positive_alpha_survive_transport_without_a_threshold_cutoff() {
    let mut initial = Surface::new(PixelFormat::RGBA32F);
    let area = Rect::new(0, 0, 400, 200);
    initial.fill_rect(area, &[100.0, -2.0, 1.0, 1e-7]);
    let output = run(initial.clone(), &stroke(0.01, 1.0), 0.5);
    assert!(initial.read_region(area) == output.read_region(area));
}

#[test]
fn ordinary_round_brush_does_not_allocate_a_sampled_square_reservoir() {
    let brush = BrushSettings { size: 5000.0, ..Default::default() };
    let sm = Smudge::new(1.0, None, brush.size).with_brush_bounds(&brush);
    assert_eq!(sm.radius, 2502);
}
