//! Model-free regressions for SAM semantic confidence -> document coverage.
use super::{JobCtx, Rect, Region, object_coverage};
use photocraft_ml::onnx_segment::Mask;

fn sample(region: &Region, x: i32, y: i32) -> u8 {
    if !region.bbox.contains(x, y) {
        return 0;
    }
    region.mask[(y - region.bbox.y0) as usize * region.bbox.width() as usize + (x - region.bbox.x0) as usize]
}

fn mask(width: usize, height: usize, probabilities: Vec<f32>) -> Mask {
    Mask { width, height, probabilities }
}

#[test]
fn low_positive_confidence_is_opaque_and_low_negative_background_has_no_ghost_v93() {
    let values = (0..49).map(|i| if (1..6).contains(&(i % 7)) && (1..6).contains(&(i / 7)) { 0.5001 } else { 0.4999 }).collect();
    let region = object_coverage(mask(7, 7, values), Rect::new(10, 20, 17, 27), &JobCtx::new()).unwrap();
    assert_eq!(region.bbox, Rect::new(11, 21, 16, 26));
    assert!(region.mask.iter().all(|alpha| *alpha == 255));
    assert_eq!(sample(&region, 10, 23), 0);
    assert_eq!(sample(&region, 13, 23), 255);
}

#[test]
fn real_hole_and_concave_notch_survive_semantic_reconstruction_v93() {
    let mut values = vec![0.51; 81];
    for y in 3..6 {
        for x in 3..6 {
            values[y * 9 + x] = 0.49;
        }
    }
    // A separate open notch must not be closed by a generic hole/shape repair.
    for y in 0..2 {
        for x in 4..6 {
            values[y * 9 + x] = 0.5;
        }
    }
    let region = object_coverage(mask(9, 9, values.clone()), Rect::new(0, 0, 9, 9), &JobCtx::new()).unwrap();
    for y in 0..9 {
        for x in 0..9 {
            assert_eq!(sample(&region, x, y), if values[y as usize * 9 + x as usize] > 0.5 { 255 } else { 0 });
        }
    }
}

#[test]
fn antialiasing_is_confined_to_document_boundary_after_large_upscale_v93() {
    let region = object_coverage(mask(2, 1, vec![0.49, 0.51]), Rect::new(0, 0, 101, 5), &JobCtx::new()).unwrap();
    for y in 0..5 {
        for x in 0..101 {
            let expected = if x < 50 {
                0
            } else if x == 50 {
                128
            } else {
                255
            };
            assert_eq!(sample(&region, x, y), expected);
        }
    }
}

#[test]
fn coverage_ignores_confidence_magnitude_when_semantic_membership_is_unchanged_v93() {
    let weak = object_coverage(mask(3, 2, vec![0.49, 0.51, 0.51, 0.49, 0.49, 0.51]), Rect::new(-3, 5, 14, 18), &JobCtx::new()).unwrap();
    let strong = object_coverage(mask(3, 2, vec![0.0, 1.0, 1.0, 0.0, 0.0, 1.0]), Rect::new(-3, 5, 14, 18), &JobCtx::new()).unwrap();
    assert_eq!(weak.bbox, strong.bbox);
    assert_eq!(weak.mask, strong.mask);
}

#[test]
fn malformed_shapes_values_and_output_budgets_fail_without_models_v93() {
    for input in [
        mask(0, 1, vec![]),
        mask(1, 0, vec![]),
        mask(2, 2, vec![1.0; 3]),
        mask(1, 1, vec![1.0, 1.0]),
        mask(usize::MAX, 2, vec![]),
        mask(1025, 1, vec![]),
        mask(1, 1, vec![f32::NAN]),
        mask(1, 1, vec![f32::INFINITY]),
        mask(1, 1, vec![-0.01]),
        mask(1, 1, vec![1.01]),
    ] {
        assert!(object_coverage(input, Rect::new(0, 0, 4, 4), &JobCtx::new()).is_err());
    }
    for bounds in [Rect::EMPTY, Rect::new(3, 0, 2, 1), Rect::new(0, 0, 100_000_001, 1), Rect::new(i32::MIN, 0, i32::MAX, 1)] {
        assert!(object_coverage(mask(1, 1, vec![1.0]), bounds, &JobCtx::new()).is_err());
    }
    assert!(object_coverage(mask(1, 1, vec![0.5]), Rect::new(0, 0, 1, 1), &JobCtx::new()).is_err());
}

#[test]
fn cancelled_semantic_reconstruction_returns_cancelled_before_allocation_v93() {
    let context = JobCtx::new();
    context.cancel();
    assert!(matches!(object_coverage(mask(1, 1, vec![1.0]), Rect::new(0, 0, 100_000_000, 1), &context), Err(crate::EngineError::Cancelled)));
}

#[test]
#[ignore = "24MP native measurement; root runs the final precompiled libtest without model/build competition"]
fn object_coverage_24mp_measurement_v93() {
    let probabilities = (0..256 * 256)
        .map(|i| {
            let x = i % 256;
            let y = i / 256;
            let dx = x as f64 - 127.5;
            let dy = y as f64 - 127.5;
            let radius = dx.hypot(dy);
            // A large ring with a real hole plus an attached thin rod.
            if (55.0..105.0).contains(&radius) || ((124..132).contains(&x) && y >= 180) { 0.51 } else { 0.49 }
        })
        .collect();
    let started = std::time::Instant::now();
    let region = object_coverage(mask(256, 256, probabilities), Rect::new(0, 0, 6000, 4000), &JobCtx::new()).unwrap();
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(sample(&region, 3000, 2000), 0, "the semantic ring hole is preserved");
    assert_eq!(sample(&region, 4875, 2000), 255, "ring body remains fully opaque");
    assert_eq!(sample(&region, 3000, 3900), 255, "thin attached rod remains selected");
    assert_eq!(sample(&region, 100, 100), 0, "distant background stays empty");
    assert!(region.mask.iter().any(|alpha| *alpha > 0 && *alpha < 255), "boundary has antialiasing");
    let selected = region.mask.iter().filter(|alpha| **alpha == 255).count();
    assert!((5_000_000..15_000_000).contains(&selected), "nondegenerate geometry");
    println!(
        "{}",
        serde_json::json!({
            "measurement":"object_coverage_24mp_v93",
            "ms":elapsed_ms,
            "outputBytes":region.mask.len(),
            "sourceWidth":256,
            "sourceHeight":256,
            "outputWidth":6000,
            "outputHeight":4000,
            "bounds":[region.bbox.x0,region.bbox.y0,region.bbox.x1,region.bbox.y1],
            "opaquePixels":selected
        })
    );
}
