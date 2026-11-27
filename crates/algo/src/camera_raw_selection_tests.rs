//! Reference parity and opt-in timing for atmospheric-light selection only.
use super::*;

fn stable_reference(dark: &[f32]) -> Vec<usize> {
    // Original implementation: stable sort by dark value, with input indices
    // initially ascending. Keep this independent of the optimized comparator.
    let mut indices: Vec<_> = (0..dark.len()).collect();
    indices.sort_by(|&a, &b| dark[b].total_cmp(&dark[a]));
    indices.truncate((dark.len() / 1000).max(1).min(dark.len()));
    indices
}

fn synthetic_dark(count: usize, mode: usize) -> Vec<f32> {
    let special = [
        0.0,
        -0.0,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::from_bits(0x7fc0_0001),
        f32::from_bits(0x7fc0_0002),
        f32::from_bits(0xffc0_0001),
        f32::from_bits(0x7f80_0001),
        f32::from_bits(0xff80_0001),
        1.0,
        -1.0,
    ];
    (0..count)
        .map(|i| {
            let bits = (i as u32).wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            match mode {
                0 => 0.5,
                1 => ((bits >> 16) % 17) as f32 / 16.0,
                2 => special[i % special.len()],
                3 => f32::from_bits(bits),
                _ => (bits >> 8) as f32 / 16_777_215.0,
            }
        })
        .collect()
}

#[test]
fn atmospheric_selection_matches_stable_sort_for_ties_nonfinite_and_boundaries() {
    for count in [0, 1, 2, 999, 1000, 1001, 1999, 2000, 131_072, 182_527] {
        for mode in 0..5 {
            let dark = synthetic_dark(count, mode);
            assert_eq!(brightest_dark_indices(&dark), stable_reference(&dark), "count={count} mode={mode}");
        }
    }
}

fn image(width: usize, height: usize) -> Vec<[f32; 4]> {
    (0..width * height)
        .map(|i| {
            let x = i % width;
            let y = i / width;
            let base = ((x / 7 + y / 11) % 31) as f32 / 30.0;
            [base, ((x * 37 + y * 13) % 257) as f32 / 256.0, ((x * 19 + y * 43) % 251) as f32 / 250.0, (i % 5) as f32 / 4.0]
        })
        .collect()
}

fn assert_pixels_identical(actual: &[[f32; 4]], expected: &[[f32; 4]]) {
    assert_eq!(actual.len(), expected.len());
    for (index, (a, b)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(a.map(f32::to_bits), b.map(f32::to_bits), "pixel={index}");
    }
}

#[test]
fn dehaze_output_is_bitwise_identical_to_full_stable_sort_reference() {
    // 769 crosses the transmission downsampling threshold; thin axes and the
    // one-pixel domain exercise clamping and the minimum top-count separately.
    for (width, height) in [(1, 1), (1, 31), (37, 1), (61, 43), (769, 3)] {
        for irregular in [false, true] {
            let mut source = image(width, height);
            if irregular {
                for (i, pixel) in source.iter_mut().enumerate() {
                    match i % 19 {
                        0 => *pixel = [-0.0, 0.0, 1.0, 0.25],
                        1 => *pixel = [f32::INFINITY, f32::NEG_INFINITY, 2.0, 0.5],
                        2 => *pixel = [f32::from_bits(0x7fc0_0001), 0.25, -0.5, 0.0],
                        _ => {}
                    }
                }
            }
            for amount in [-1.0, -0.3, 0.4, 1.0] {
                let mut actual = source.clone();
                let mut expected = source.clone();
                dehaze(&mut actual, width, height, amount);
                dehaze_with_selection(&mut expected, width, height, amount, stable_reference);
                assert_pixels_identical(&actual, &expected);
            }
        }
    }
}

/// cargo test -p photocraft-algo --release dehaze_selection_release_benchmark --lib -- --ignored --nocapture --test-threads=1
#[test]
#[ignore = "release CPU benchmark: 182k/131k selection and dehaze, plus 24 MP; run without concurrent builds"]
fn dehaze_selection_release_benchmark() {
    use std::{hint::black_box, time::Instant};
    let mut selection = Vec::new();
    for count in [182_527, 131_072] {
        for (mode, name) in [(0, "constant"), (1, "ties"), (4, "varied")] {
            let dark = synthetic_dark(count, mode);
            assert_eq!(brightest_dark_indices(&dark), stable_reference(&dark));
            let run = |optimized| {
                let start = Instant::now();
                let result = if optimized { brightest_dark_indices(black_box(&dark)) } else { stable_reference(black_box(&dark)) };
                let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                (result, elapsed)
            };
            let mut optimized_ms = Vec::new();
            let mut reference_ms = Vec::new();
            for repetition in 0..9 {
                let (a, b) = if repetition % 2 == 0 {
                    let a = run(true);
                    (a, run(false))
                } else {
                    let b = run(false);
                    (run(true), b)
                };
                assert_eq!(a.0, b.0);
                optimized_ms.push(a.1);
                reference_ms.push(b.1);
            }
            selection.push(serde_json::json!({"count":count,"distribution":name,"optimized_ms":optimized_ms,"reference_ms":reference_ms}));
        }
    }
    let mut stage = Vec::new();
    let mut warm = image(64, 64);
    dehaze(&mut warm, 64, 64, 0.4);
    for (width, height) in [(523, 349), (512, 256), (6000, 4000)] {
        let source = image(width, height);
        let run = |optimized| {
            // Match both algorithms' input and exclude the caller's image clone.
            let mut result = source.clone();
            let start = Instant::now();
            if optimized {
                dehaze(&mut result, width, height, 0.4);
            } else {
                dehaze_with_selection(&mut result, width, height, 0.4, stable_reference);
            }
            (result, start.elapsed().as_secs_f64() * 1000.0)
        };
        let mut optimized_ms = Vec::new();
        let mut reference_ms = Vec::new();
        for repetition in 0..3 {
            let (a, b) = if repetition % 2 == 0 {
                let a = run(true);
                (a, run(false))
            } else {
                let b = run(false);
                (run(true), b)
            };
            assert_pixels_identical(&a.0, &b.0);
            optimized_ms.push(a.1);
            reference_ms.push(b.1);
        }
        stage.push(serde_json::json!({"source":[width,height],"optimized_ms":optimized_ms,"reference_ms":reference_ms,"bitwise_equal":true}));
    }
    println!(
        "{}",
        serde_json::json!({"benchmark":"camera_raw_dehaze_selection", "selection":selection,"dehaze":stage,
        "measurement":"CPU only; internal allocations included, source generation/clone excluded; no GPU/upload/UI/input latency"})
    );
}
