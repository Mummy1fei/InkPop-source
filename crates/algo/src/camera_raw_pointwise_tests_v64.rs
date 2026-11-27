//! Frozen pre-v64 arithmetic for bit-exact refactor checks; not a Photoshop oracle.
use super::*;

fn pixels(w: usize, h: usize) -> Vec<[f32; 4]> {
    (0..w * h)
        .map(|i| {
            [
                ((i * 37 % 251) as f32 - 35.0) / 140.0,
                ((i * 13 % 229) as f32 - 11.0) / 180.0,
                ((i * 61 % 257) as f32 - 19.0) / 170.0,
                if i % 23 == 0 { 0.0 } else { 0.37 },
            ]
        })
        .collect()
}

#[test]
fn shared_prepared_color_and_grain_keep_frozen_v63_global_bits() {
    let cases = [
        serde_json::json!({"vibrance":63,"saturation":-28}),
        serde_json::json!({"curveHighlights":-17,"curveLights":23,"curveDarks":-31,"curveShadows":11,"curveSplits":[19,43,79]}),
        serde_json::json!({"pointCurve":[[0,13],[91,120],[255,248]],"pointCurveRed":[[0,0],[125,157],[255,255]],"pointCurveGreen":[[0,8],[131,107],[255,250]],"pointCurveBlue":[[0,6],[85,120],[255,255]]}),
        serde_json::json!({"hslHue":[15,-21,28,33,-12,-25,37,-43],"hslSat":[-18,29,-32,13,28,47,-31,21],"hslLum":[29,-27,11,42,-13,17,23,-33]}),
        serde_json::json!({"gradeShadows":{"hue":39,"sat":53,"lum":-17},"gradeMidtones":{"hue":137,"sat":35,"lum":11},"gradeHighlights":{"hue":234,"sat":19,"lum":23},"gradeGlobal":{"hue":319,"sat":17,"lum":-7},"gradeBlending":79,"gradeBalance":-23}),
        serde_json::json!({"grainAmount":63,"grainSize":47,"grainRoughness":79}),
        serde_json::json!({"exposure":0.4,"contrast":19,"texture":31,"clarity":-23,"dehaze":11,"highlights":31,"shadows":-17,"vibrance":27,"noiseColor":21,"sharpenAmount":53,"grainAmount":19,"pointCurveRed":[[0,0],[119,149],[255,255]],"vignetteAmount":-23}),
    ];
    let (w, h) = (37, 29);
    for changes in cases {
        for float in [false, true] {
            for scale in [0.01, 0.25, 1.0] {
                let mut p: CameraRaw = serde_json::from_value(changes.clone()).unwrap();
                p.pixel_scale = scale;
                p.seed = 171;
                let mut actual = pixels(w, h);
                let mut expected = actual.clone();
                develop(&mut actual, w, h, &p, float);
                frozen_develop(&mut expected, w, h, &p, float);
                assert_eq!(
                    actual.iter().map(|p| p.map(f32::to_bits)).collect::<Vec<_>>(),
                    expected.iter().map(|p| p.map(f32::to_bits)).collect::<Vec<_>>(),
                    "{changes}; float={float}; scale={scale}"
                );
            }
        }
    }
}

pub(crate) fn frozen_develop(px: &mut [[f32; 4]], w: usize, h: usize, p: &CameraRaw, float: bool) {
    if px.is_empty() || p.is_identity() {
        return;
    }
    let ps = p.pixel_scale.clamp(0.01, 1.0);
    let long = w.max(h) as f32;
    // 1. White balance + exposure in linear light.
    if !p.wb_neutral() {
        let t = p.temperature / 100.0;
        let tn = p.tint / 100.0;
        let mut g = [1.0 + 0.35 * t, 1.0 - 0.25 * tn, 1.0 - 0.35 * t];
        let norm = luma(g);
        g = g.map(|v| v / norm * 2f32.powf(p.exposure));
        par_rows(px, w, 1, |_, row| {
            for q in row.iter_mut() {
                for c in 0..3 {
                    q[c] = linear_to_srgb((srgb_to_linear(q[c]) * g[c]).max(0.0));
                }
            }
        });
    }
    // 2. Dehaze.
    if p.dehaze != 0.0 {
        dehaze(px, w, h, p.dehaze / 100.0);
    }
    // 3. Tone on luminance.
    if !p.tone_neutral() {
        let l: Vec<f32> = px.iter().map(|q| luma([q[0], q[1], q[2]]).max(0.0)).collect();
        let lc: Vec<f32> = l.iter().map(|v| v.min(1.0)).collect();
        let base = if p.highlights != 0.0 || p.shadows != 0.0 {
            let r = ((long * 0.015) as usize).max(2);
            Some(guided_fast(&lc, &lc, w, h, r, 0.02))
        } else {
            None
        };
        let clar = (p.clarity != 0.0).then(|| guided_fast(&lc, &lc, w, h, ((long * 0.02) as usize).max(3), 0.005));
        let tex = (p.texture != 0.0).then(|| gauss(&lc, w, h, (long * 0.002).max(1.5 * ps)));
        let k_con = p.contrast / 100.0 * 0.6;
        let tau = std::f32::consts::TAU;
        par_rows(px, w, 1, |y, row| {
            for (x, q) in row.iter_mut().enumerate() {
                let i = y * w + x;
                let l0 = l[i];
                let mut lv = lc[i];
                if let Some(b) = &base {
                    let bv = b[i].clamp(0.0, 1.0);
                    let hm = smoothstep(0.45, 0.95, bv);
                    let sm = 1.0 - smoothstep(0.05, 0.5, bv);
                    lv *= 2f32.powf(p.highlights / 100.0 * 1.2 * hm + p.shadows / 100.0 * 1.4 * sm);
                }
                if let Some(c) = &clar {
                    let mw = 1.0 - (2.0 * lv.clamp(0.0, 1.0) - 1.0).powi(2);
                    lv += p.clarity / 100.0 * 0.8 * (lc[i] - c[i]) * mw;
                }
                if let Some(t) = &tex {
                    lv += p.texture / 100.0 * 0.9 * (lc[i] - t[i]).clamp(-0.1, 0.1);
                }
                if p.whites != 0.0 {
                    lv += p.whites / 100.0 * 0.25 * smoothstep(0.4, 1.0, lv);
                }
                if p.blacks != 0.0 {
                    lv += p.blacks / 100.0 * 0.15 * (1.0 - smoothstep(0.0, 0.45, lv));
                }
                if k_con != 0.0 {
                    let xx = lv.clamp(0.0, 1.0);
                    lv += -k_con * (tau * xx).sin() / tau;
                }
                let lv = lv.max(0.0) + (l0 - lc[i]);
                let ratio = if l0 > 1e-5 { lv / l0 } else { 1.0 };
                for c in 0..3 {
                    q[c] = if l0 > 1e-5 { q[c] * ratio } else { q[c] + (lv - l0) };
                }
            }
        });
    }
    // 4–7: per-pixel colour stages.
    let need_color = p.vibrance != 0.0 || p.saturation != 0.0 || !p.curves_neutral() || !p.hsl_neutral() || !p.grade_neutral();
    if need_color {
        let par = (!(p.curve_highlights == 0.0 && p.curve_lights == 0.0 && p.curve_darks == 0.0 && p.curve_shadows == 0.0)).then(|| parametric_lut(p, 1024));
        let pts = |c: &Vec<[f32; 2]>| (!is_linear(c)).then(|| curve_lut(c, 1024));
        let (master, cr, cg, cb) = (pts(&p.point_curve), pts(&p.point_curve_red), pts(&p.point_curve_green), pts(&p.point_curve_blue));
        let hsl = !p.hsl_neutral();
        let grade = !p.grade_neutral();
        let wheel_tint = |wh: &Wheel| -> [f32; 3] {
            let c = hsv_to_rgb(wh.hue, 1.0, 1.0);
            let l = luma(c);
            [c[0] - l, c[1] - l, c[2] - l].map(|v| v * wh.sat / 100.0 * 0.3)
        };
        let tints = [wheel_tint(&p.grade_shadows), wheel_tint(&p.grade_midtones), wheel_tint(&p.grade_highlights), wheel_tint(&p.grade_global)];
        let blend = 0.15 + p.grade_blending.clamp(0.0, 100.0) / 100.0 * 0.35;
        let bal = p.grade_balance.clamp(-100.0, 100.0) / 100.0 * 0.25;
        par_rows(px, w, 1, |_, row| {
            for q in row.iter_mut() {
                let mut c = [q[0], q[1], q[2]];
                if p.vibrance != 0.0 || p.saturation != 0.0 {
                    let l = luma(c);
                    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
                    let sat = if mx > 1e-6 { (mx - mn) / mx } else { 0.0 };
                    // Vibrance protects skin-ish hues (oranges) a little, like ACR.
                    let hue = rgb_to_hsv(c)[0];
                    let skin = (1.0 - ((hue - 25.0).abs() / 25.0)).clamp(0.0, 1.0) * 0.5;
                    let k = 1.0 + p.saturation / 100.0 + p.vibrance / 100.0 * (1.0 - sat) * (1.0 - skin);
                    c = c.map(|v| l + (v - l) * k.max(0.0));
                }
                if let Some(lut) = &par {
                    c = c.map(|v| lut_eval(lut, v));
                }
                if let Some(lut) = &master {
                    c = c.map(|v| lut_eval(lut, v));
                }
                for (ch, lut) in [&cr, &cg, &cb].iter().enumerate() {
                    if let Some(lut) = lut {
                        c[ch] = lut_eval(lut, c[ch]);
                    }
                }
                if hsl {
                    let [hh, s, v] = rgb_to_hsv(c.map(|x| x.max(0.0)));
                    if s > 1e-4 {
                        let wts = band_weights(hh);
                        let (mut dh, mut ds, mut dl) = (0.0f32, 0.0f32, 0.0f32);
                        for k in 0..8 {
                            dh += wts[k] * p.hsl_hue[k];
                            ds += wts[k] * p.hsl_sat[k];
                            dl += wts[k] * p.hsl_lum[k];
                        }
                        let nh = hh + dh / 100.0 * 30.0;
                        let ns = (s * (1.0 + ds / 100.0)).clamp(0.0, 1.0);
                        let mut out = hsv_to_rgb(nh, ns, v);
                        // Keep the luminance, then apply the luminance shift weighted by saturation.
                        let (l0, l1) = (luma(c), luma(out));
                        if l1 > 1e-6 {
                            out = out.map(|x| x * l0 / l1);
                        }
                        let k = 2f32.powf(dl / 100.0 * 1.0 * s);
                        c = out.map(|x| x * k);
                    }
                }
                if grade {
                    let l = luma(c).clamp(0.0, 1.0);
                    let sh = 1.0 - smoothstep(0.0, 0.5 + bal + blend, l);
                    let hi = smoothstep(0.5 + bal - blend, 1.0, l);
                    let mid = (1.0 - sh - hi).max(0.0);
                    let masks = [sh, mid, hi, 1.0];
                    let lums = [p.grade_shadows.lum, p.grade_midtones.lum, p.grade_highlights.lum, p.grade_global.lum];
                    for k in 0..4 {
                        for ch in 0..3 {
                            c[ch] += masks[k] * tints[k][ch];
                        }
                        if lums[k] != 0.0 {
                            let f = 2f32.powf(masks[k] * lums[k] / 100.0 * 0.6);
                            c = c.map(|x| x * f);
                        }
                    }
                }
                q[0] = c[0];
                q[1] = c[1];
                q[2] = c[2];
            }
        });
    }
    // 8. Detail.
    if p.noise_luminance > 0.0 || p.noise_color > 0.0 || p.sharpen_amount > 0.0 {
        detail(px, w, h, p, ps);
    }
    // 9. Effects.
    if p.vignette_amount != 0.0 {
        vignette(px, w, h, p);
    }
    if p.grain_amount > 0.0 {
        let cell = 1.0 + p.grain_size / 25.0;
        let rough = p.grain_roughness / 100.0;
        let amt = p.grain_amount / 100.0 * 0.12;
        let seed = p.seed as u64;
        let pscale = 1.0 / ps;
        par_rows(px, w, 1, |y, row| {
            for (x, q) in row.iter_mut().enumerate() {
                // Sample in full-resolution coordinates so previews match the result.
                let (fx, fy) = (x as f32 * pscale / cell, y as f32 * pscale / cell);
                let n = value_noise(fx, fy, seed) * (1.0 - rough) + (value_noise(fx * 2.3, fy * 2.3, seed + 7) - 0.0) * rough;
                let l = luma([q[0], q[1], q[2]]).clamp(0.0, 1.0);
                let g = (n - 0.5) * 2.0 * amt * (0.4 + 2.4 * l * (1.0 - l));
                for c in 0..3 {
                    q[c] += g;
                }
            }
        });
    }
    if !float {
        for q in px.iter_mut() {
            for c in 0..3 {
                q[c] = q[c].clamp(0.0, 1.0);
            }
        }
    }
    if !p.local_adjustments.is_empty() {
        assert!(p.local_adjustments.is_empty(), "frozen global oracle is leaf-only");
    }
}
