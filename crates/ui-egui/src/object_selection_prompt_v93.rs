//! Object Selection encloses an instance; foreground strokes are a separate, explicit gesture.
use crate::state::ObjectSelectionShape;
use serde_json::{Value, json};

const MAX_POLYGON_POINTS: usize = 256;

/// Closed, uniformly arc-length sampled outline. Dense coalesced samples do not
/// overweight one slowly drawn corner; the final list stays within the engine budget.
fn outline(path: &[[f64; 3]]) -> Vec<[f64; 2]> {
    let mut points = path.iter().filter(|p| p[0].is_finite() && p[1].is_finite()).map(|p| [p[0].clamp(-1e7, 1e7), p[1].clamp(-1e7, 1e7)]).collect::<Vec<_>>();
    points.dedup_by(|a, b| (a[0] - b[0]).hypot(a[1] - b[1]) < 1e-6);
    if points.len() > 1 && points.first() == points.last() {
        points.pop();
    }
    if points.len() <= MAX_POLYGON_POINTS {
        return points;
    }
    let mut lengths = Vec::with_capacity(points.len());
    let mut perimeter = 0.0;
    for (a, b) in points.iter().zip(points.iter().cycle().skip(1)).take(points.len()) {
        let distance = (a[0] - b[0]).hypot(a[1] - b[1]);
        lengths.push(distance);
        perimeter += distance;
    }
    if !perimeter.is_finite() || perimeter <= 1e-6 {
        return Vec::new();
    }
    let mut sampled = Vec::with_capacity(MAX_POLYGON_POINTS);
    let mut segment = 0;
    let mut before = 0.0;
    for index in 0..MAX_POLYGON_POINTS {
        let distance = perimeter * index as f64 / MAX_POLYGON_POINTS as f64;
        while segment + 1 < points.len() && lengths.get(segment).is_some_and(|length| before + length < distance) {
            before += lengths.get(segment).copied().unwrap_or(0.0);
            segment += 1;
        }
        let Some((a, b, length)) =
            points.get(segment).zip(points.get((segment + 1) % points.len())).zip(lengths.get(segment)).map(|((a, b), length)| (a, b, length))
        else {
            return Vec::new();
        };
        let t = if *length > 0.0 { ((distance - before) / length).clamp(0.0, 1.0) } else { 0.0 };
        sampled.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
    }
    sampled
}

/// A point remains a point; only explicit ForegroundStroke produces positive points.
pub(super) fn prompt(
    shape: ObjectSelectionShape,
    start: [f64; 2],
    end: [f64; 2],
    path: &[[f64; 3]],
    mode: &str,
    sample_all_layers: bool,
    bounds: [f64; 4],
) -> Option<Value> {
    if start.into_iter().chain(end).chain(bounds).any(|v| !v.is_finite()) || bounds[0] >= bounds[2] || bounds[1] >= bounds[3] {
        return None;
    }
    let extent = path
        .iter()
        .filter(|p| p[0].is_finite() && p[1].is_finite())
        .fold((start, start), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]));
    // Closed lassos can finish at the first point, so click detection uses the
    // path's complete extent rather than just the press/release displacement.
    let dragged = (extent.1[0] - extent.0[0]).max(extent.1[1] - extent.0[1]).max((end[0] - start[0]).abs()).max((end[1] - start[1]).abs()) >= 2.0;
    if shape == ObjectSelectionShape::ForegroundStroke {
        return Some(super::smart_object_prompt(start, path, mode, sample_all_layers, bounds));
    }
    if !dragged {
        return Some(super::smart_object_prompt(start, &[], mode, sample_all_layers, bounds));
    }
    let polygon = (shape == ObjectSelectionShape::Lasso).then(|| outline(path));
    let (lo, hi) = if let Some(polygon) = &polygon {
        if polygon.len() < 3 {
            return None;
        }
        polygon
            .iter()
            .fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]))
    } else {
        ([start[0].min(end[0]), start[1].min(end[1])], [start[0].max(end[0]), start[1].max(end[1])])
    };
    let (x0, y0, x1, y1) =
        (lo[0].clamp(bounds[0], bounds[2]), lo[1].clamp(bounds[1], bounds[3]), hi[0].clamp(bounds[0], bounds[2]), hi[1].clamp(bounds[1], bounds[3]));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let mut result = json!({"mode": mode, "sampleAllLayers": sample_all_layers, "rect": [x0, y0, x1 - x0, y1 - y0]});
    if let Some(polygon) = polygon {
        result["polygon"] = json!(polygon);
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    const BOUNDS: [f64; 4] = [0.0, 0.0, 100.0, 100.0];

    #[test]
    fn rectangles_use_only_corners_in_all_directions_even_with_dense_diagonal_samples() {
        for (start, end) in [([10.0, 20.0], [90.0, 80.0]), ([90.0, 80.0], [10.0, 20.0]), ([90.0, 20.0], [10.0, 80.0]), ([10.0, 80.0], [90.0, 20.0])] {
            let path = (0..1000)
                .map(|i| {
                    let t = i as f64 / 999.0;
                    [start[0] + (end[0] - start[0]) * t, start[1] + (end[1] - start[1]) * t, 1.0]
                })
                .collect::<Vec<_>>();
            let result = prompt(ObjectSelectionShape::Rectangle, start, end, &path, "subtract", true, BOUNDS).unwrap();
            assert_eq!(result["rect"], json!([10.0, 20.0, 80.0, 60.0]));
            assert!(result.get("points").is_none());
            assert!(result.get("polygon").is_none());
            assert_eq!(result["mode"], "subtract");
        }
    }

    #[test]
    fn closed_outline_is_an_enclosing_polygon_never_foreground_or_guessed_centroid() {
        let path = [[10.0, 10.0, 1.0], [10.0, 90.0, 1.0], [40.0, 90.0, 1.0], [40.0, 40.0, 1.0], [90.0, 40.0, 1.0], [90.0, 10.0, 1.0], [10.0, 10.0, 1.0]];
        let result = prompt(ObjectSelectionShape::Lasso, [10.0, 10.0], [10.0, 10.0], &path, "add", false, BOUNDS).unwrap();
        assert_eq!(result["rect"], json!([10.0, 10.0, 80.0, 80.0]));
        assert_eq!(result["polygon"].as_array().unwrap().len(), 6);
        assert!(result.get("points").is_none());
        assert_eq!(result["mode"], "add");
    }

    #[test]
    fn dense_lasso_sampling_follows_closed_arc_length_and_stays_bounded() {
        let mut path = vec![[10.0, 10.0, 1.0]; 1000];
        for i in 0..2000 {
            let t = i as f64 / 500.0;
            let (x, y) = if t < 1.0 {
                (10.0 + 80.0 * t, 10.0)
            } else if t < 2.0 {
                (90.0, 10.0 + 80.0 * (t - 1.0))
            } else if t < 3.0 {
                (90.0 - 80.0 * (t - 2.0), 90.0)
            } else {
                (10.0, 90.0 - 80.0 * (t - 3.0))
            };
            path.push([x, y, 1.0]);
        }
        let result = prompt(ObjectSelectionShape::Lasso, [10.0, 10.0], [10.0, 10.0], &path, "replace", true, BOUNDS).unwrap();
        let polygon = result["polygon"].as_array().unwrap();
        assert_eq!(polygon.len(), MAX_POLYGON_POINTS);
        for point in polygon {
            let x = point[0].as_f64().unwrap();
            let y = point[1].as_f64().unwrap();
            assert!((x - 10.0).abs() < 1e-6 || (x - 90.0).abs() < 1e-6 || (y - 10.0).abs() < 1e-6 || (y - 90.0).abs() < 1e-6);
        }
    }

    #[test]
    fn click_and_foreground_stroke_have_distinct_semantics() {
        for shape in [ObjectSelectionShape::Rectangle, ObjectSelectionShape::Lasso, ObjectSelectionShape::ForegroundStroke] {
            let result = prompt(shape, [20.0, 30.0], [20.0, 30.0], &[[20.0, 30.0, 1.0]; 10], "replace", false, BOUNDS).unwrap();
            assert_eq!(result["points"], json!([[20.0, 30.0]]));
            assert!(result.get("rect").is_none());
        }
        let path = [[10.0, 20.0, 1.0], [60.0, 20.0, 1.0]];
        let result = prompt(ObjectSelectionShape::ForegroundStroke, [10.0, 20.0], [60.0, 20.0], &path, "replace", true, BOUNDS).unwrap();
        assert_eq!(result["points"], json!([[10.0, 20.0], [60.0, 20.0]]));
        assert!(result.get("rect").is_none());
    }

    #[test]
    fn rectangle_clips_without_rounding_and_invalid_input_never_creates_a_prompt() {
        let result = prompt(ObjectSelectionShape::Rectangle, [-10.0, 0.5], [105.0, 50.25], &[], "replace", false, BOUNDS).unwrap();
        assert_eq!(result["rect"], json!([0.0, 0.5, 100.0, 49.75]));
        assert!(prompt(ObjectSelectionShape::Rectangle, [f64::NAN, 0.0], [10.0, 20.0], &[], "replace", false, BOUNDS).is_none());
        assert!(prompt(ObjectSelectionShape::Rectangle, [-20.0, 0.0], [-10.0, 20.0], &[], "replace", false, BOUNDS).is_none());
    }
}
