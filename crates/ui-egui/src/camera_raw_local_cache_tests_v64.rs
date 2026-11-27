use super::*;
use masks::{LocalAdjustment, MaskShape};

fn source(w: usize, h: usize) -> Arc<[[f32; 4]]> {
    (0..w * h)
        .map(|i| [((i * 17) % 301) as f32 / 255., ((i * 31) % 256) as f32 / 255., ((i * 7) % 256) as f32 / 255., if i % 7 == 0 { 0. } else { 0.6 }])
        .collect::<Vec<_>>()
        .into()
}
fn group(spatial: bool) -> LocalAdjustment {
    LocalAdjustment {
        exposure: 0.3,
        texture: if spatial { 12. } else { 0. },
        vibrance: 13.,
        grain_amount: 8.,
        mask: MaskShape::Radial { center: [0.4, 0.5], radius: [0.3, 0.22], feather: 0.5, rotation: 28. },
        ..Default::default()
    }
}
fn exact(p: &CameraRaw, s: &Arc<[[f32; 4]]>, w: usize, h: usize, float: bool, previous: Option<&GlobalStage>) -> Output {
    let out = render(s, w, h, p, float, previous);
    let mut expected = s.to_vec();
    develop(&mut expected, w, h, p, float);
    assert_eq!(out.pixels.iter().flatten().map(|v| v.to_bits()).collect::<Vec<_>>(), expected.iter().flatten().map(|v| v.to_bits()).collect::<Vec<_>>());
    out
}
#[test]
fn local_checkpoint_reuses_only_identical_prefix_and_spatial_candidate() {
    let (w, h) = (64, 48);
    let s = source(w, h);
    for float in [false, true] {
        let mut p = CameraRaw { pixel_scale: 0.4, seed: 71, local_adjustments: vec![group(true), group(true), group(false)], ..Default::default() };
        p.local_adjustments[2].mask = MaskShape::Luminance { min: 0.2, max: 0.7, feather: 0.1 };
        let mut out = exact(&p, &s, w, h, float, None);
        p.local_adjustments[1].exposure = 0.6;
        out = exact(&p, &s, w, h, float, out.global.as_ref());
        assert_eq!(out.global.as_ref().unwrap().local.as_ref().unwrap().index, 1);
        p.local_adjustments[1].exposure = 0.8;
        out = exact(&p, &s, w, h, float, out.global.as_ref());
        assert!(out.global.as_ref().unwrap().prefix_hit());
        assert!(!out.global.as_ref().unwrap().candidate_hit());
        if let MaskShape::Radial { rotation, .. } = &mut p.local_adjustments[1].mask {
            *rotation = 55.;
        }
        out = exact(&p, &s, w, h, float, out.global.as_ref());
        assert!(out.global.as_ref().unwrap().prefix_hit());
        assert!(out.global.as_ref().unwrap().candidate_hit());
        p.local_adjustments[1].invert = true;
        out = exact(&p, &s, w, h, float, out.global.as_ref());
        assert!(out.global.as_ref().unwrap().candidate_hit());
        p.local_adjustments[0].hue = 40.;
        out = exact(&p, &s, w, h, float, out.global.as_ref());
        assert!(!out.global.as_ref().unwrap().candidate_hit());
        assert!(!out.global.as_ref().unwrap().prefix_hit());
        p.local_adjustments.remove(0);
        out = exact(&p, &s, w, h, float, out.global.as_ref());
        p.local_adjustments.reverse();
        out = exact(&p, &s, w, h, float, out.global.as_ref());
        p.local_adjustments.push(group(true));
        out = exact(&p, &s, w, h, float, out.global.as_ref());
        p.texture = 9.;
        out = exact(&p, &s, w, h, float, out.global.as_ref());
        assert!(!out.hit);
        p.seed += 1;
        out = exact(&p, &s, w, h, float, out.global.as_ref());
        assert!(!out.hit);
        p.pixel_scale = 0.25;
        out = exact(&p, &s, w, h, float, out.global.as_ref());
        assert!(!out.hit);
        assert!(out.global.as_ref().unwrap().local_bytes() <= LOCAL_CACHE_BYTES);
        let different: Arc<[[f32; 4]]> = s.to_vec().into();
        assert!(!exact(&p, &different, w, h, float, out.global.as_ref()).hit);
    }
}
#[test]
fn independent_caches_never_mutate_prior_frames_or_source_pixels() {
    let (w, h) = (32, 24);
    let s = source(w, h);
    let original = s.to_vec();
    let p = CameraRaw { local_adjustments: vec![group(true), group(false)], ..Default::default() };
    let first = exact(&p, &s, w, h, true, None);
    let retained = first.pixels.clone();
    let old_cache = first.global.as_ref().unwrap().local.as_ref().unwrap().candidate.as_ref().unwrap().clone();
    let old_pixels = old_cache.to_vec();
    let mut next = p.clone();
    next.local_adjustments[0].exposure = 1.;
    let _out = exact(&next, &s, w, h, true, first.global.as_ref());
    assert_eq!(s.as_ref(), original);
    assert_eq!(old_cache.as_ref(), old_pixels);
    assert_eq!(first.pixels, retained);
}
