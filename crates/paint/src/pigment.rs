//! Opaque Kubelka–Munk mixing with synthetic pigment reflectance.
//!
//! Eight bands integrate the 380–730 nm observer approximations of Wyman et al.
//! (JCGT 2(2), eq. 4) against a 6504 K Planck spectrum. Each linear sRGB row is
//! normalized so that a flat reflector is neutral. The six RGB cube colors use
//! bounded least-squares spectra with a 0.003 squared second-difference penalty.
//! To model a blue paint with a broader short-wave shoulder, its fitted curve
//! B is extended by (1-B)*0.25*exp(-0.5*((wavelength_nm-515)/27)^2).
//! These are representative artist pigments; a display RGB cannot specify a
//! unique reflectance or a measured scattering coefficient.

use photocraft_color::convert::{linear_to_srgb, srgb_to_linear};
use std::sync::OnceLock;

const BANDS: usize = 8;
const RGB_RESPONSE: [[f32; BANDS]; 3] = [
    [0.01555262, 0.06032716, -0.1988586, -0.2572204, 0.5268453, 0.7206796, 0.1296587, 0.00301555],
    [-0.01302749, -0.07890656, 0.1901219, 0.5949529, 0.3413349, -0.02417733, -0.010845, 0.00054667],
    [0.08966832, 0.7633785, 0.2563277, -0.0450655, -0.05097194, -0.01185258, -0.00135745, -0.00012697],
];

// Red, green, yellow, blue, magenta, cyan. Consecutive bands of one pigment
// stay together for the inner mixing loop.
const REFLECTANCE: [[f32; BANDS]; 6] = [
    [0.137988, 0.025502, 0., 0., 0.237126, 1., 1., 1.],
    [0., 0., 0.544705, 1., 0.749207, 0., 0., 0.],
    [0., 0., 0.460662, 1., 1., 0.974363, 0.901971, 0.815291],
    [1., 1., 0.612744, 0.199565, 0.018094, 0.025752, 0.098029, 0.184709],
    [1., 1., 0.455295, 0., 0.250793, 1., 1., 1.],
    [0.862012, 0.974498, 1., 1., 0.762874, 0., 0., 0.],
];

const TRANSFER_STEPS: usize = 2048;
type TransferCurve = [f32; TRANSFER_STEPS + 1];

struct TransferCurves {
    decode: TransferCurve,
    encode: TransferCurve,
}

impl TransferCurves {
    fn get() -> &'static Self {
        static CURVES: OnceLock<TransferCurves> = OnceLock::new();
        CURVES.get_or_init(|| Self {
            decode: std::array::from_fn(|i| srgb_to_linear(i as f32 / TRANSFER_STEPS as f32)),
            // Concentrate encode samples near black, where gamma changes fastest.
            encode: std::array::from_fn(|i| {
                let x = i as f32 / TRANSFER_STEPS as f32;
                linear_to_srgb(x * x)
            }),
        })
    }
}

fn sample(curve: &TransferCurve, channel: f32) -> f32 {
    let x = channel * TRANSFER_STEPS as f32;
    let i = (x as usize).min(TRANSFER_STEPS - 1);
    curve[i] + (curve[i + 1] - curve[i]) * (x - i as f32)
}

struct Spectrum {
    white: f32,
    base: f32,
    primary: &'static [f32; BANDS],
    primary_weight: f32,
    secondary: &'static [f32; BANDS],
    secondary_weight: f32,
}

impl Spectrum {
    fn from_linear_rgb(rgb: [f32; 3]) -> Self {
        let [mut low, mut mid, mut high] = [0, 1, 2];
        if rgb[low] > rgb[mid] {
            std::mem::swap(&mut low, &mut mid);
        }
        if rgb[mid] > rgb[high] {
            std::mem::swap(&mut mid, &mut high);
        }
        if rgb[low] > rgb[mid] {
            std::mem::swap(&mut low, &mut mid);
        }
        // A black reflector needs a finite floor for the absorption ratio.
        let white = rgb[low];
        Self {
            white,
            base: REFLECTANCE_FLOOR + (1. - REFLECTANCE_FLOOR) * white,
            primary: &REFLECTANCE[(1 << high) - 1],
            primary_weight: (1. - REFLECTANCE_FLOOR) * (rgb[high] - rgb[mid]),
            secondary: &REFLECTANCE[6 - (1 << low)],
            secondary_weight: (1. - REFLECTANCE_FLOOR) * (rgb[mid] - white),
        }
    }

    fn band(&self, i: usize) -> f32 {
        self.base + self.primary_weight * self.primary[i] + self.secondary_weight * self.secondary[i]
    }
}

// Projecting a synthetic spectrum back to sRGB can overshoot a display primary,
// even when both inputs are in gamut. Roll off the last eighth of the
// available channel range with a continuous slope so a mixing ramp keeps detail.
fn display_channel(base: f32, shift: f32) -> f32 {
    let capacity = if shift < 0. { base } else { 1. - base };
    let shoulder = 0.125 * capacity;
    let distance = capacity - shift.abs();
    if distance >= shoulder {
        base + shift
    } else {
        let remaining = shoulder * shoulder / (2. * shoulder - distance);
        if shift < 0. { remaining } else { 1. - remaining }
    }
}

const TRACE_LIMIT: f32 = 1. / 32.;
const REFLECTANCE_FLOOR: f32 = 0.06;
// Additional reflectance available in the shoulder of the synthetic blue pigment.
// Generated from the quantized blue basis by evidence/v115/generate-v115-spectrum.py.
const BLUE_SHOULDER: [f32; BANDS] = [0., 0., 0.2936218, 0.7982616, 0.07237692, 0.00046291, 2e-7, 0.];

pub(super) fn mix_srgb(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    if a == b {
        return a;
    }
    let dose = t.min(1. - t);
    if dose >= TRACE_LIMIT {
        return mix_contact(a, b, t);
    }
    let (dominant, trace) = if t > 0.5 { (b, a) } else { (a, b) };
    if dose <= 0. {
        return dominant;
    }
    // A trace of paint contacts a fraction of the surface at a higher local
    // concentration. Coverage times contact concentration equals the nominal
    // dose. Interpolate in display space to smooth the first visible color steps.
    // At TRACE_LIMIT, coverage reaches one with zero slope.
    let contact_dose = dose + 0.5 * TRACE_LIMIT * (1. - dose / TRACE_LIMIT).powi(2);
    let contact = mix_contact(dominant, trace, contact_dose);
    let coverage = dose / contact_dose;
    std::array::from_fn(|i| dominant[i] + coverage * (contact[i] - dominant[i]))
}

fn white_scatter(white: f32, contact: f32) -> f32 {
    // A dark paint also contains a neutral RGB component. Boost scattering
    // smoothly only for bright reflectors, strongest at equal concentration.
    // This avoids repeatedly amplifying extremely small pickups of white paint.
    let light = ((white - 0.4) / 0.38).clamp(0., 1.);
    let near_white = light * light * (3. - 2. * light);
    1. + 3. * white * near_white * contact
}

fn mix_contact(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let curves = TransferCurves::get();
    let linear_a = a.map(|v| sample(&curves.decode, v));
    let linear_b = b.map(|v| sample(&curves.decode, v));
    let first = Spectrum::from_linear_rgb(linear_a);
    let second = Spectrum::from_linear_rgb(linear_b);
    let contact = 4. * t * (1. - t);
    let first_scatter = (1. - t) * white_scatter(first.white, contact);
    let second_scatter = t * white_scatter(second.white, contact);
    let total_scatter = first_scatter + second_scatter;
    let wa = first_scatter / total_scatter;
    let wb = second_scatter / total_scatter;

    // Increase short-wave overlap in proportion to the blue pigment and the
    // other paint's yellow component. A neutral tint supplies no yellow.
    let blue = |[r, g, b]: [f32; 3]| (b - r.max(g)).max(0.);
    let yellow = |[r, g, b]: [f32; 3]| ((r.min(g) - b).max(0.) / (r.max(g) + 0.0001)).powi(2);
    let shoulder_scale = (1. - REFLECTANCE_FLOOR) * 0.65;
    let blue_overlap_a = shoulder_scale * blue(linear_a) * yellow(linear_b);
    let blue_overlap_b = shoulder_scale * blue(linear_b) * yellow(linear_a);
    let change: [f32; BANDS] = std::array::from_fn(|i| {
        let ra = first.band(i) + blue_overlap_a * BLUE_SHOULDER[i];
        let rb = second.band(i) + blue_overlap_b * BLUE_SHOULDER[i];
        // K/S(r)=(1-r)^2/(2r). Keep the weighted sum as a fraction until
        // evaluating reflectance R=1/(1+K/S+sqrt((K/S)^2+2K/S)).
        let da = 1. - ra;
        let db = 1. - rb;
        let numerator = wa * rb * da * da + wb * ra * db * db;
        let denominator = 2. * ra * rb;
        let mixed = denominator / (denominator + numerator + (numerator * (numerator + 2. * denominator)).sqrt());
        mixed - (ra * wa + rb * wb)
    });
    // Apply the spectral shift to the source colors. This keeps the RGB
    // endpoints even if their synthesized spectra have a reconstruction error.
    let spectral = std::array::from_fn(|i| {
        let shift: f32 = RGB_RESPONSE[i].iter().zip(change).map(|(weight, delta)| weight * delta).sum();
        let channel = display_channel((linear_a[i] * wa + linear_b[i] * wb).clamp(0., 1.), shift);
        sample(&curves.encode, channel.clamp(0., 1.).sqrt())
    });
    // In a tint, a synthetic spectrum can shift a pigment's hue excessively.
    // At the calculated luma, pull chroma toward the line between the two
    // source colors in proportion to their estimated neutral-white content.
    let luma = |[r, g, b]: [f32; 3]| 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let from = luma(a);
    let contrast = luma(b) - from;
    // Project onto the endpoint line in a luma-weighted RGB metric. Including
    // the channel differences keeps the solution stable for two distinct hues
    // of equal luma. For almost identical colors the regularizer favors t.
    let delta: [f32; 3] = std::array::from_fn(|i| b[i] - a[i]);
    let residual: [f32; 3] = std::array::from_fn(|i| spectral[i] - a[i]);
    let rgb_projection: f32 = delta.iter().zip(residual).map(|(s, r)| s * r).sum();
    let squared_rgb_distance: f32 = delta.iter().map(|v| v * v).sum();
    let tint_position = ((contrast * (luma(spectral) - from) + 0.15 * rgb_projection + 0.0001 * t)
        / (contrast * contrast + 0.15 * squared_rgb_distance + 0.0001))
        .clamp(0., 1.);
    let white = first.white.max(second.white);
    std::array::from_fn(|i| spectral[i] + white * ((1. - tint_position) * a[i] + tint_position * b[i] - spectral[i]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_tints_lift_monotonically_and_keep_the_color_order() {
        let umber = [109., 62., 41.].map(|v| v / 255.);
        let white = [245., 241., 228.].map(|v| v / 255.);
        let mut last = umber;
        for step in 1..=20 {
            let shade = mix_srgb(umber, white, step as f32 / 20.);
            assert!(last.iter().zip(shade).all(|(before, after)| *before < after));
            assert!(shade[0] > shade[1] && shade[1] > shade[2]);
            last = shade;
        }
        assert!(mix_srgb(umber, white, 0.5)[1] > 130. / 255.);
    }

    #[test]
    fn varied_blue_and_yellow_paints_keep_a_visible_green_overlap() {
        for (blue, yellow) in [([18., 60., 172.], [251., 209., 25.]), ([0., 64., 255.], [255., 225., 45.]), ([24., 108., 182.], [222., 186., 18.])] {
            let [r, g, b] = mix_srgb(blue.map(|v| v / 255.), yellow.map(|v| v / 255.), 0.5);
            assert!(g > r.max(b) + 0.075);
        }
    }

    #[test]
    fn red_and_blue_keep_detail_through_the_shadow_end_of_the_display_gamut() {
        let red = [222., 52., 42.].map(|v| v / 255.);
        let blue = [37., 74., 213.].map(|v| v / 255.);
        for step in 1..100 {
            let shade = mix_srgb(red, blue, step as f32 / 100.);
            assert!(shade.into_iter().all(|v| v > 0. && v < 1.));
        }
    }

    #[test]
    fn violet_tints_keep_their_red_green_order_as_white_is_added() {
        let violet = [91., 48., 151.].map(|v| v / 255.);
        for near_white in [[1.; 3], [249., 247., 247.].map(|v| v / 255.)] {
            for t in [0.1, 0.35, 0.5, 0.75, 0.9] {
                let [r, g, b] = mix_srgb(violet, near_white, t);
                assert!(b > r && r > g);
            }
        }
    }

    #[test]
    fn trace_coverage_connects_continuously_and_symmetrically_to_full_contact() {
        let first = [0.05, 0.24, 0.84];
        let second = [0.99, 0.92, 0.15];
        assert_eq!(mix_srgb(first, second, 0.), first);
        assert_eq!(mix_srgb(first, second, 1.), second);
        assert_ne!(mix_srgb(first, second, 1. / 65536.), first);
        for mid in [TRACE_LIMIT, 1. - TRACE_LIMIT] {
            let step = 1. / 65536.;
            let left = mix_srgb(first, second, mid - step);
            let center = mix_srgb(first, second, mid);
            let right = mix_srgb(first, second, mid + step);
            assert_eq!(center, mix_contact(first, second, mid));
            for i in 0..3 {
                assert!((left[i] - right[i]).abs() < 0.001);
                assert!((right[i] - 2. * center[i] + left[i]).abs() < 0.00001);
            }
        }
        for k in 0..=128 {
            let t = k as f32 / 128.;
            let shade = mix_srgb(first, second, t);
            let reverse = mix_srgb(second, first, 1. - t);
            assert!(shade.into_iter().zip(reverse).all(|(a, b)| (a - b).abs() < 0.000001));
        }
    }

    #[test]
    fn white_scattering_requires_light_reflectance_and_interacting_pigments() {
        assert_eq!(white_scatter(0.4, 1.), 1.);
        assert_eq!(white_scatter(1., 0.), 1.);
        assert_eq!(white_scatter(1., 1.), 4.);
        assert!(white_scatter(0.6, 0.5) > 1.);
        assert!(white_scatter(0.6, 0.5) < white_scatter(0.6, 1.));
    }

    #[test]
    fn adding_white_in_small_steps_lifts_each_channel_throughout_the_ramp() {
        for source in [[0.1, 0.4, 0.7], [0.7, 0.05, 0.35], [0.13, 0.34, 0.08], [0.6, 0.4, 0.18], [0.; 3]] {
            let mut previous = source;
            for k in 1..=256 {
                let next = mix_srgb(source, [1.; 3], k as f32 / 256.);
                assert!(previous.into_iter().zip(next).all(|(a, b)| a < b));
                previous = next;
            }
        }
    }

    #[test]
    fn transfer_interpolation_and_round_trip_stay_within_one_sixteen_bit_code() {
        let curves = TransferCurves::get();
        for v in 0..=65535 {
            let v = v as f32 / 65535.;
            let linear = sample(&curves.decode, v);
            assert!((sample(&curves.encode, v.sqrt()) - linear_to_srgb(v)).abs() < 1. / 65535.);
            assert!((linear - srgb_to_linear(v)).abs() < 1. / 65535.);
            assert!((sample(&curves.encode, linear.sqrt()) - v).abs() < 1. / 65535.);
        }
    }
}
