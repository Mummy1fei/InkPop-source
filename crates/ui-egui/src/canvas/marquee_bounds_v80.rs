//! Clip the live selection polygon in document coordinates. In particular an
//! ellipse crossing a canvas edge stays the original ellipse, rather than being
//! squeezed into its clipped bounding rectangle. No pixel buffers are touched.
use super::marquee_px;

pub fn clip_polygon(points: Vec<[f64; 2]>, size: [u32; 2]) -> Vec<[f64; 2]> {
    if size.contains(&0) || points.iter().any(|point| point.iter().any(|value| !value.is_finite())) {
        return Vec::new();
    }
    let mut polygon = points;
    for (axis, edge, lower) in [(0, 0.0, true), (0, f64::from(size[0]), false), (1, 0.0, true), (1, f64::from(size[1]), false)] {
        let Some(mut previous) = polygon.last().copied() else { return polygon };
        let inside = |point: [f64; 2]| if lower { point[axis] >= edge } else { point[axis] <= edge };
        let mut previous_inside = inside(previous);
        let mut clipped = Vec::with_capacity(polygon.len().saturating_add(4));
        for current in polygon {
            let current_inside = inside(current);
            if current_inside != previous_inside {
                let denominator = current[axis] - previous[axis];
                if denominator != 0.0 && denominator.is_finite() {
                    let t = ((edge - previous[axis]) / denominator).clamp(0.0, 1.0);
                    let mut crossing = [previous[0] + (current[0] - previous[0]) * t, previous[1] + (current[1] - previous[1]) * t];
                    crossing[axis] = edge;
                    if crossing.iter().all(|value| value.is_finite()) {
                        clipped.push(crossing);
                    }
                }
            }
            if current_inside {
                clipped.push(current);
            }
            previous = current;
            previous_inside = current_inside;
        }
        polygon = clipped;
    }
    polygon
}

/// Display-size readout follows the in-canvas portion; the underlying ellipse
/// geometry is deliberately retained for the existing bounded engine command.
pub fn clipped_readout(corners: ([f64; 2], [f64; 2]), size: [u32; 2]) -> [f64; 4] {
    let [x0, y0, x1, y1] = marquee_px(corners.0, corners.1);
    [x0.clamp(0.0, f64::from(size[0])), y0.clamp(0.0, f64::from(size[1])), x1.clamp(0.0, f64::from(size[0])), y1.clamp(0.0, f64::from(size[1]))]
}

#[cfg(test)]
#[path = "marquee_bounds_tests_v80.rs"]
mod tests;
