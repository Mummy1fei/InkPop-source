//! Candidate-policy regressions use independent known masks; ignored QA runs the real weights.
use super::*;

fn candidate(rect: [usize; 4]) -> Vec<f32> {
    let [x0, y0, x1, y1] = rect;
    let mut mask = vec![-4.0; MASK_PIXELS];
    for y in y0..y1 {
        for x in x0..x1 {
            mask[y * MASK_SIDE + x] = 4.0;
        }
    }
    mask
}

fn legacy_choice(masks: &[f32], scores: &[f32], points: &[(f64, f64, bool)], width: usize, height: usize) -> usize {
    let best = scores.iter().copied().filter(|score| score.is_finite()).fold(f32::NEG_INFINITY, f32::max);
    let mut chosen = None;
    for (index, mask) in masks.as_chunks::<MASK_PIXELS>().0.iter().enumerate() {
        if scores.get(index).copied().unwrap_or(f32::NEG_INFINITY) < best - 0.15 {
            continue;
        }
        if !points.iter().all(|(x, y, foreground)| {
            let ix = (*x * width as f64 / 4.).clamp(0., 255.) as usize;
            let iy = (*y * height as f64 / 4.).clamp(0., 255.) as usize;
            mask.get(iy * MASK_SIDE + ix).is_some_and(|value| (*value > 0.) == *foreground)
        }) {
            continue;
        }
        let area = mask.iter().filter(|value| **value > 0.).count();
        if chosen.is_none_or(|(_, previous)| area > previous) {
            chosen = Some((index, area));
        }
    }
    chosen.map(|(index, _)| index).unwrap_or_else(|| {
        scores.iter().enumerate().filter(|(_, score)| score.is_finite()).max_by(|(_, a), (_, b)| a.total_cmp(b)).map_or(0, |(index, _)| index)
    })
}

#[test]
fn quality_score_beats_a_larger_lower_quality_background_candidate_v82() {
    let object = candidate([32, 32, 96, 96]);
    let oversized = candidate([0, 0, 180, 180]);
    let part = candidate([48, 48, 80, 80]);
    let masks = [object.clone(), oversized, part].concat();
    let scores = [0.94, 0.83, 0.81];
    let point = [(0.25, 0.25, true)];
    assert_eq!(legacy_choice(&masks, &scores, &point, 1024, 1024), 1, "exercise the former largest-mask error");
    let chosen = select_candidate(&masks, &scores, &point, 1024, 1024).unwrap();
    assert_eq!(chosen, 0);
    assert_eq!(&masks[chosen * MASK_PIXELS..(chosen + 1) * MASK_PIXELS], object);
}

#[test]
fn explicit_foreground_background_points_beat_quality_scores_when_inconsistent_v82() {
    let object = candidate([32, 32, 96, 96]);
    let background = candidate([0, 0, 180, 120]);
    let wrong_object = candidate([120, 20, 180, 110]);
    let masks = [background, wrong_object, object].concat();
    let points = [(0.25, 0.5, true), (0.6, 0.5, false)];
    assert_eq!(select_candidate(&masks, &[0.99, 0.98, 0.79], &points, 1024, 512).unwrap(), 2);
}

#[test]
fn invalid_candidate_dimensions_logits_and_quality_scores_are_rejected_v82() {
    let good = candidate([20, 20, 80, 80]);
    assert!(select_candidate(&good, &[], &[], 1024, 512).is_err());
    assert!(select_candidate(&good[..MASK_PIXELS - 1], &[0.9], &[], 1024, 512).is_err());
    assert!(select_candidate(&good, &[f32::NAN], &[], 1024, 512).is_err());
    let mut bad = good.clone();
    bad[0] = f32::NAN;
    let masks = [bad, good].concat();
    assert_eq!(select_candidate(&masks, &[0.99, 0.8], &[], 1024, 512).unwrap(), 1);
}

#[test]
#[ignore = "runs local MobileSAM weights on original synthetic labelled geometry; writes raw candidates for root quality review"]
fn actual_mobile_sam_quality_v82() {
    use std::io::Write;
    use std::path::Path;
    let model = std::env::var_os("PHOTOCRAFT_SMART_SELECTION_MODEL").expect("root must set the existing model path");
    let output = std::env::var_os("PHOTOCRAFT_SMART_QA_DIR").expect("root must set a new evidence output directory");
    let output = Path::new(&output);
    std::fs::create_dir(output).expect("preserve earlier results; output must not exist");
    let (width, height) = (1024, 512);
    let inside = |x: f64, y: f64| ((x - 280.) / 145.).powi(2) + ((y - 255.) / 180.).powi(2) <= 1.;
    let mut rgb = Vec::with_capacity(width * height * 3);
    for y in 0..height {
        for x in 0..width {
            let object = inside(x as f64 + 0.5, y as f64 + 0.5);
            let stripe = ((x / 19 + y / 23) % 2) as f32;
            let pixel = if object {
                [220. - y as f32 * 0.08, 115. + x as f32 * 0.04, 48. + stripe * 6.]
            } else if (590..900).contains(&x) && (105..410).contains(&y) {
                [72. + stripe * 5., 151. - y as f32 * 0.04, 89.]
            } else {
                [42. + stripe * 3., 66. + y as f32 * 0.02, 100. + x as f32 * 0.01]
            };
            rgb.extend(pixel);
        }
    }
    let mut input = std::fs::OpenOptions::new().write(true).create_new(true).open(output.join("synthetic-input.ppm")).unwrap();
    write!(input, "P6\n{width} {height}\n255\n").unwrap();
    input.write_all(&rgb.iter().map(|value| *value as u8).collect::<Vec<_>>()).unwrap();
    let truth: Vec<bool> = (0..128).flat_map(|y| (0..256).map(move |x| inside((x as f64 + 0.5) * 4., (y as f64 + 0.5) * 4.))).collect();
    let save_mask = |name: &str, mask: &[f32]| {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(output.join(name)).unwrap();
        write!(file, "P5\n256 128\n255\n").unwrap();
        file.write_all(&mask.iter().take(256 * 128).map(|value| if *value > 0. { 255 } else { 0 }).collect::<Vec<u8>>()).unwrap();
    };
    let truth_logits: Vec<f32> = truth.iter().map(|value| if *value { 4. } else { -4. }).chain(std::iter::repeat_n(-4., 256 * 128)).collect();
    save_mask("truth.pgm", &truth_logits);
    let weights = std::fs::read(model).unwrap();
    let mut segmenter = Segmenter::new(weights).unwrap();
    let start = std::time::Instant::now();
    segmenter.set_image(82, rgb, width, height).unwrap();
    let encoder_ms = start.elapsed().as_secs_f64() * 1000.;
    let cases = [
        ("one-point", vec![(280. / 1024., 255. / 512., true)], None),
        ("two-interior-points", vec![(280. / 1024., 165. / 512., true), (280. / 1024., 345. / 512., true)], None),
        ("positive-negative", vec![(280. / 1024., 255. / 512., true), (700. / 1024., 255. / 512., false)], None),
        ("box", Vec::new(), Some([125. / 1024., 65. / 512., 435. / 1024., 445. / 512.])),
    ];
    let iou = |mask: &[f32]| {
        let (mut intersection, mut union) = (0usize, 0usize);
        for (value, truth) in mask.iter().zip(&truth) {
            intersection += usize::from(*value > 0. && *truth);
            union += usize::from(*value > 0. || *truth);
        }
        intersection as f64 / union.max(1) as f64
    };
    let mut records = Vec::new();
    for (name, points, rect) in cases {
        let old_multi = rect.is_none();
        let new_multi = points.len() == 1 && rect.is_none();
        let (old_masks, old_scores) = segmenter.decode(&points, rect, old_multi).unwrap();
        let (new_masks, new_scores) =
            if old_multi == new_multi { (old_masks.clone(), old_scores.clone()) } else { segmenter.decode(&points, rect, new_multi).unwrap() };
        let old_index = if old_multi { legacy_choice(&old_masks, &old_scores, &points, width, height) } else { 0 };
        let new_index = if new_multi { select_candidate(&new_masks, &new_scores, &points, width, height).unwrap() } else { 0 };
        let selected = segmenter.predict(&points, rect).unwrap();
        assert_eq!(selected, new_masks[new_index * MASK_PIXELS..(new_index + 1) * MASK_PIXELS]);
        for (policy, masks) in [("legacy", &old_masks), ("candidate", &new_masks)] {
            let mut raw = std::fs::OpenOptions::new().write(true).create_new(true).open(output.join(format!("{name}-{policy}-all-logits-f32le.bin"))).unwrap();
            for value in masks {
                raw.write_all(&value.to_le_bytes()).unwrap();
            }
            for (index, mask) in masks.as_chunks::<MASK_PIXELS>().0.iter().enumerate() {
                save_mask(&format!("{name}-{policy}-{index}.pgm"), mask);
            }
        }
        let old = &old_masks[old_index * MASK_PIXELS..(old_index + 1) * MASK_PIXELS];
        let new = &new_masks[new_index * MASK_PIXELS..(new_index + 1) * MASK_PIXELS];
        let old_iou = iou(old);
        let new_iou = iou(new);
        let point_json = points.iter().map(|(x, y, foreground)| format!("[{x},{y},{foreground}]")).collect::<Vec<_>>().join(",");
        let box_json = rect.map_or_else(|| "null".into(), |rect| format!("{rect:?}"));
        records.push(format!("{{\"case\":\"{name}\",\"legacyIndex\":{old_index},\"candidateIndex\":{new_index},\"legacyScores\":{old_scores:?},\"candidateScores\":{new_scores:?},\"legacyIou\":{old_iou},\"candidateIou\":{new_iou},\"points\":[{point_json}],\"box\":{box_json}}}"));
        eprintln!("{name}: legacyIoU={old_iou:.6}, candidateIoU={new_iou:.6}, index={old_index}->{new_index}");
    }
    let summary = format!(
        "{{\"encoderMs\":{encoder_ms},\"cases\":[{}],\"scope\":\"original synthetic ellipse vs separate distractor; low-resolution mask IoU before final reconstruction/refinement, not a natural-image accuracy dataset or Photoshop parity\"}}",
        records.join(",")
    );
    std::fs::write(output.join("quality-records.json"), summary).unwrap();
}
