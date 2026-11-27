use super::*;
use photocraft_psd::abr::{LegacyBrush, write_v6, write_v12};
use photocraft_psd::descriptor::{Id, UnicodeString};

fn sample(id: &str, w: u32, h: u32, depth: u16) -> AbrSample {
    let bpp = usize::from(depth / 8);
    let data = (0..w as usize * h as usize).flat_map(|i| std::iter::repeat_n(if i % 2 == 0 { 255u8 } else { 0 }, bpp)).collect();
    AbrSample { id: id.into(), width: w, height: h, depth, data }
}

fn t(s: &str) -> Value {
    Value::Text(UnicodeString::new_nul(s))
}
fn u(unit: &[u8; 4], v: f64) -> Value {
    Value::UnitFloat { unit: *unit, value: v }
}
fn prc(v: f64) -> Value {
    u(b"#Prc", v)
}
fn en(ty: &str, v: &str) -> Value {
    Value::Enumerated { type_id: Id::new(ty), value: Id::new(v) }
}
fn var(control: i32, jitter: f64, minimum: f64) -> Value {
    Value::Descriptor(
        Descriptor::new("brVr").with("bVTy", Value::Integer(control)).with("fStp", Value::Integer(40)).with("jitter", prc(jitter)).with("Mnm ", prc(minimum)),
    )
}

fn sampled_tip(id: &str) -> Descriptor {
    Descriptor::new("sampledBrush")
        .with("Dmtr", u(b"#Pxl", 64.0))
        .with("Angl", u(b"#Ang", 30.0))
        .with("Rndn", prc(80.0))
        .with("Spcn", prc(40.0))
        .with("Intr", Value::Boolean(true))
        .with("flipX", Value::Boolean(true))
        .with("flipY", Value::Boolean(false))
        .with("sampledData", t(id))
}

/// A preset touching every section.
fn full_preset() -> Descriptor {
    Descriptor::new("brushPreset")
        .with("Nm  ", t("Kitchen Sink"))
        .with("Brsh", Value::Descriptor(sampled_tip("$tip-1")))
        .with("useTipDynamics", Value::Boolean(true))
        .with("flipX", Value::Boolean(true))
        .with("flipY", Value::Boolean(false))
        .with("minimumDiameter", prc(25.0))
        .with("minimumRoundness", prc(10.0))
        .with("tiltScale", prc(0.0))
        .with("szVr", var(2, 20.0, 0.0))
        .with("angleDynamics", var(6, 10.0, 0.0))
        .with("roundnessDynamics", var(3, 5.0, 0.0))
        .with("useScatter", Value::Boolean(true))
        .with("Cnt ", Value::Integer(3))
        .with("bothAxes", Value::Boolean(true))
        .with("countDynamics", var(0, 50.0, 0.0))
        .with("scatterDynamics", var(1, 250.0, 0.0))
        .with("useTexture", Value::Boolean(true))
        .with("TxtC", Value::Boolean(true))
        .with("InvT", Value::Boolean(true))
        .with("textureScale", prc(150.0))
        .with("textureBrightness", Value::Integer(-75))
        .with("textureContrast", Value::Integer(25))
        .with("textureBlendMode", en("BlnM", "CBrn"))
        .with("textureDepth", prc(60.0))
        .with("minimumDepth", prc(20.0))
        .with("textureDepthDynamics", var(2, 30.0, 0.0))
        .with("Txtr", Value::Descriptor(Descriptor::new("Ptrn").with("Nm  ", t("Grit")).with("Idnt", t("pat-uuid"))))
        .with(
            "dualBrush",
            Value::Descriptor(
                Descriptor::new("dualBrush")
                    .with("useDualBrush", Value::Boolean(true))
                    .with("Flip", Value::Boolean(true))
                    .with("Brsh", Value::Descriptor(Descriptor::new("computedBrush").with("Dmtr", u(b"#Pxl", 12.0)).with("Hrdn", prc(50.0))))
                    .with("BlnM", en("BlnM", "hardMix"))
                    .with("Spcn", prc(30.0))
                    .with("Cnt ", Value::Integer(2))
                    .with("bothAxes", Value::Boolean(true))
                    .with("scatterDynamics", var(0, 120.0, 0.0)),
            ),
        )
        .with("useColorDynamics", Value::Boolean(true))
        .with("clVr", var(2, 40.0, 0.0))
        .with("H   ", prc(10.0))
        .with("Strt", prc(20.0))
        .with("Brgh", prc(30.0))
        .with("purity", prc(-50.0))
        .with("colorDynamicsPerTip", Value::Boolean(false))
        .with("usePaintDynamics", Value::Boolean(true))
        .with("opVr", var(2, 0.0, 10.0))
        .with("prVr", var(1, 15.0, 5.0))
        .with("wtVr", var(0, 0.0, 0.0))
        .with("useBrushPose", Value::Boolean(true))
        .with("brushPoseTiltX", Value::Integer(50))
        .with("brushPoseAngle", Value::Integer(90))
        .with("overridePoseAngle", Value::Boolean(true))
        .with("Wtdg", Value::Boolean(true))
        .with("Nose", Value::Boolean(true))
        .with("Rpt ", Value::Boolean(true))
        .with("someFutureKey", Value::Boolean(true))
        .with(
            "toolOptions",
            Value::Descriptor(
                Descriptor::new("currentToolOptions")
                    .with("Opct", Value::Integer(80))
                    .with("flow", Value::Integer(60))
                    .with("smoothing", Value::Boolean(true))
                    .with("smoothingValue", Value::Integer(40))
                    .with("usePressureOverridesSize", Value::Boolean(true)),
            ),
        )
}

fn grit_pattern() -> PsdPattern {
    PsdPattern {
        mode: 1,
        width: 4,
        height: 2,
        name: "Grit".into(),
        id: "pat-uuid".into(),
        palette: None,
        depth: 8,
        channels: vec![vec![0, 64, 128, 255, 255, 128, 64, 0]],
        alpha: None,
    }
}

fn tpl_fixture(tools: &[(&str, Descriptor)]) -> Vec<u8> {
    use photocraft_psd::descriptor::VersionedDescriptor;
    let abr = write_v6(1, &[sample("$tip-1", 4, 2, 8)], &[grit_pattern()], &[], true).unwrap();
    let mut out = b"8BTP".to_vec();
    out.extend(3u32.to_be_bytes());
    out.extend(3u32.to_be_bytes());
    let mut pos = 4;
    while pos + 12 <= abr.len() {
        let key = &abr[pos + 4..pos + 8];
        let len = u32::from_be_bytes(abr[pos + 8..pos + 12].try_into().unwrap()) as usize;
        if matches!(key, b"samp" | b"patt") {
            out.extend(b"8BIM");
            out.extend(if key == b"samp" { b"tpbd" } else { b"tppa" });
            out.extend((len as u32).to_be_bytes());
            out.extend(&abr[pos + 12..pos + 12 + len]);
        }
        pos += 12 + len;
        if pos % 4 != 0 {
            pos += 4 - pos % 4;
        }
    }
    let mut body = (tools.len() as u32).to_be_bytes().to_vec();
    for (name, desc) in tools {
        let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
        body.extend((name.len() as u32).to_be_bytes());
        for c in name {
            body.extend(c.to_be_bytes());
        }
        body.extend(VersionedDescriptor::new(desc.clone()).to_bytes());
    }
    out.extend(b"8BIMtptp");
    out.extend((body.len() as u32).to_be_bytes());
    out.extend(body);
    out
}

#[test]
fn tpl_smudge_strength_uses_pressure_key_without_changing_brush_flow() {
    for (key, value) in [("Prs ", 98.0), ("pressure", 65.0)] {
        let d = Descriptor::new("SmTl")
            .with("Brsh", Value::Descriptor(sampled_tip("$tip-1")))
            .with(key, Value::Double(value))
            .with("Strn", Value::Integer(17))
            .with("flow", Value::Integer(41));
        let imported = read_tpl_with(&tpl_fixture(&[("smudge strength", d)]), &photocraft_raster::Interrupt::NONE).unwrap();
        let p = &imported.presets[0];
        assert_eq!(p.tool, "smudge");
        assert_eq!(p.strength, Some(value as f32));
        assert_eq!(p.brush.flow, 0.41);
        assert_eq!(p.brush.opacity, 1.0);
    }
}

#[test]
fn tpl_strength_fallback_remains_scoped_to_smudge() {
    for (class, prs, strn, expected) in [
        ("SmTl", Some(100.0), Some(22), Some(100.0)),
        ("SmTl", None, Some(22), Some(22.0)),
        ("SmTl", None, None, None),
        ("BlTl", Some(98.0), Some(22), Some(22.0)),
        ("ShTl", Some(98.0), None, None),
        ("PbTl", Some(98.0), None, None),
    ] {
        let mut d = Descriptor::new(class).with("Brsh", Value::Descriptor(sampled_tip("$tip-1")));
        if let Some(v) = prs {
            d = d.with("Prs ", Value::Double(v));
        }
        if let Some(v) = strn {
            d = d.with("Strn", Value::Integer(v));
        }
        let imported = read_tpl_with(&tpl_fixture(&[("strength scope", d)]), &photocraft_raster::Interrupt::NONE).unwrap();
        assert_eq!(imported.presets[0].strength, expected, "{class}");
    }
}

#[test]
fn tpl_resolves_sample_texture_dynamics_and_tool_settings() {
    let mut d = full_preset();
    d.class_id = Id::new("MixB");
    d = d
        .with("Opct", Value::Integer(37))
        .with("flow", Value::Integer(61))
        .with("wetness", Value::Double(80.0))
        .with("dryness", Value::Double(25.0))
        .with("mix", Value::Double(45.0));
    d = d.with("autoClean", Value::Boolean(false)).with("autoFill", Value::Boolean(false));
    let file = tpl_fixture(&[("水彩", d), ("shape", Descriptor::new("rectangleTool"))]);
    let imported = read_tpl_with(&file, &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(imported.presets.len(), 1);
    assert_eq!(imported.skipped, 1);
    let preset = &imported.presets[0];
    assert_eq!(preset.name, "水彩");
    assert_eq!(preset.tool, "mixerBrush");
    assert!(matches!(preset.brush.tip, TipShape::Sampled(_)));
    assert!(preset.brush.texture.enabled && matches!(preset.brush.texture.pattern, Pattern::Tile(_)));
    assert!(preset.brush.shape_dynamics.enabled);
    assert!((preset.brush.opacity - 0.37).abs() < 1e-6);
    assert!((preset.brush.flow - 0.61).abs() < 1e-6);
    assert!((preset.brush.mixer.wet - 0.8).abs() < 1e-6);
    assert!((preset.brush.mixer.load - 0.25).abs() < 1e-6);
    assert!(!preset.brush.mixer.clean_after_stroke && !preset.brush.mixer.load_after_stroke);
    assert!(imported.warnings.iter().any(|w| w.contains("rectangleTool")));
}

#[test]
fn tpl_sampled_dual_normalizes_secondary_geometry_without_changing_primary() {
    for (angle, roundness) in [(90.0, 80.0), (0.0, 25.0), (0.0, 100.0)] {
        let mut secondary = sampled_tip("$tip-1");
        secondary.items.retain(|(k, _)| !k.is("Angl") && !k.is("Rndn"));
        secondary = secondary.with("Angl", u(b"#Ang", angle)).with("Rndn", prc(roundness));
        let d = Descriptor::new("PbTl").with("Brsh", Value::Descriptor(sampled_tip("$tip-1"))).with(
            "dualBrush",
            Value::Descriptor(Descriptor::new("dualBrush").with("useDualBrush", Value::Boolean(true)).with("Brsh", Value::Descriptor(secondary))),
        );
        let imported = read_tpl_with(&tpl_fixture(&[("sampled secondary geometry", d)]), &photocraft_raster::Interrupt::NONE).unwrap();
        let b = &imported.presets[0].brush;
        assert_eq!((b.size, b.angle, b.roundness, b.spacing, b.flip_x), (64.0, 30.0, 0.8, 0.4, true));
        let dual = &b.dual_brush;
        assert!(dual.enabled && dual.random_rotation);
        assert_eq!((dual.angle, dual.roundness, dual.size, dual.spacing), (0.0, 1.0, 64.0, 0.4));
        // Normalizing the transform must not rotate, squeeze, or replace source pixels.
        assert_eq!(dual.tip, b.tip);
        let roundtrip: BrushSettings = serde_json::from_value(serde_json::to_value(b).unwrap()).unwrap();
        assert_eq!(&roundtrip, b);
    }
}

#[test]
fn abr_computed_and_missing_dual_tips_keep_explicit_geometry() {
    for secondary in
        [Descriptor::new("computedBrush").with("Dmtr", u(b"#Pxl", 12.0)).with("Angl", u(b"#Ang", 90.0)).with("Rndn", prc(25.0)), sampled_tip("missing")]
    {
        let expected_angle = num(&secondary, "Angl").unwrap() as f32;
        let expected_roundness = pct(&secondary, "Rndn").unwrap();
        let d = Descriptor::new("PbTl").with("Brsh", Value::Descriptor(sampled_tip("$tip-1"))).with(
            "dualBrush",
            Value::Descriptor(Descriptor::new("dualBrush").with("useDualBrush", Value::Boolean(true)).with("Brsh", Value::Descriptor(secondary))),
        );
        // The mapper intentionally resolves an unmatched ID to the sole sample
        // in old files. Include two resources so "missing" is genuinely absent.
        let bytes = write_v6(1, &[sample("$tip-1", 4, 2, 8), sample("$tip-2", 2, 4, 8)], &[], &[d], true).unwrap();
        let imported = read_abr(&bytes, "round secondary geometry").unwrap();
        let dual = &imported.presets[0].brush.dual_brush;
        assert!(dual.enabled && !dual.random_rotation);
        assert_eq!(dual.tip, TipShape::Round);
        assert_eq!((dual.angle, dual.roundness), (expected_angle, expected_roundness));
    }
}

#[test]
fn tpl_disabled_sampled_dual_does_not_enable_random_rotation() {
    let d = Descriptor::new("PbTl").with("Brsh", Value::Descriptor(sampled_tip("$tip-1"))).with(
        "dualBrush",
        Value::Descriptor(Descriptor::new("dualBrush").with("useDualBrush", Value::Boolean(false)).with("Brsh", Value::Descriptor(sampled_tip("$tip-1")))),
    );
    let imported = read_tpl_with(&tpl_fixture(&[("disabled secondary", d)]), &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(imported.presets[0].brush.dual_brush, DualBrush::default());
}

#[test]
fn tpl_dual_spacing_keeps_secondary_percent_and_fractional_size_or_scatter() {
    let mut tip = sampled_tip("$tip-1");
    tip.items.retain(|(k, _)| !k.is("Dmtr") && !k.is("Spcn"));
    tip = tip.with("Dmtr", u(b"#Pxl", 10.448275862068968)).with("Spcn", prc(12.0));
    let d = Descriptor::new("PbTl")
        .with("Brsh", Value::Descriptor(Descriptor::new("computedBrush").with("Dmtr", u(b"#Pxl", 6.0))))
        .with("Md  ", en("BlnM", "Mltp"))
        .with(
            "dualBrush",
            Value::Descriptor(
                Descriptor::new("dualBrush")
                    .with("useDualBrush", Value::Boolean(true))
                    .with("Brsh", Value::Descriptor(tip))
                    .with("Spcn", prc(100.0))
                    .with("BlnM", en("BlnM", "Ovrl"))
                    .with("scatterDynamics", var(0, 117.0, 0.0)),
            ),
        );
    let imported = read_tpl_with(&tpl_fixture(&[("secondary spacing", d)]), &photocraft_raster::Interrupt::NONE).unwrap();
    let b = &imported.presets[0].brush;
    assert!((b.dual_brush.size - 10.448276).abs() < 1e-5);
    assert_eq!((b.dual_brush.spacing * 100.0).round(), 12.0);
    assert!((b.dual_brush.scatter - 1.17).abs() < 1e-6);
    assert_eq!(b.dual_brush.mode, MaskMode::Overlay);
    assert_eq!(b.mode, photocraft_color::BlendMode::Multiply);
}

#[test]
fn tpl_dual_spacing_without_outer_value_is_independent_of_both_diameters() {
    // The observed 250 px primary / 8.8195 px secondary / 29% preset must keep
    // 29%, not become 822%. Resizing either diameter does not rewrite a percentage.
    for primary in [6.0, 250.0, 764.0] {
        for secondary in [8.819508753362442, 36.0] {
            let mut tip = sampled_tip("$tip-1");
            tip.items.retain(|(k, _)| !k.is("Dmtr") && !k.is("Spcn"));
            tip = tip.with("Dmtr", u(b"#Pxl", secondary)).with("Spcn", prc(29.0));
            let d = Descriptor::new("PbTl").with("Brsh", Value::Descriptor(Descriptor::new("computedBrush").with("Dmtr", u(b"#Pxl", primary)))).with(
                "dualBrush",
                Value::Descriptor(Descriptor::new("dualBrush").with("useDualBrush", Value::Boolean(true)).with("Brsh", Value::Descriptor(tip))),
            );
            let imported = read_tpl_with(&tpl_fixture(&[("secondary 29 percent", d)]), &photocraft_raster::Interrupt::NONE).unwrap();
            let b = &imported.presets[0].brush;
            assert!((b.dual_brush.spacing - 0.29).abs() < 1e-6, "primary {primary}, secondary {secondary}");
            assert!((b.dual_brush.size - secondary as f32).abs() < 1e-6);
            assert_eq!(b.size, primary as f32);
        }
    }
}

#[test]
fn tpl_dual_spacing_uses_outer_only_when_nested_spacing_is_absent() {
    for (inner, outer, expected) in [(Some(29.0), Some(20.0), 0.29), (None, Some(37.0), 0.37), (None, None, 0.25)] {
        let mut tip = sampled_tip("$tip-1");
        tip.items.retain(|(k, _)| !k.is("Spcn"));
        if let Some(v) = inner {
            tip = tip.with("Spcn", prc(v));
        }
        let mut dual = Descriptor::new("dualBrush").with("useDualBrush", Value::Boolean(true)).with("Brsh", Value::Descriptor(tip));
        if let Some(v) = outer {
            dual = dual.with("Spcn", prc(v));
        }
        let d = Descriptor::new("PbTl")
            .with("Brsh", Value::Descriptor(Descriptor::new("computedBrush").with("Dmtr", u(b"#Pxl", 250.0))))
            .with("dualBrush", Value::Descriptor(dual));
        let imported = read_tpl_with(&tpl_fixture(&[("spacing precedence", d)]), &photocraft_raster::Interrupt::NONE).unwrap();
        assert!((imported.presets[0].brush.dual_brush.spacing - expected).abs() < 1e-6, "inner {inner:?}, outer {outer:?}");
    }
}

#[test]
fn tpl_missing_tip_is_skipped_and_cancellation_is_respected() {
    let d = Descriptor::new("PbTl").with("Brsh", Value::Descriptor(sampled_tip("missing")));
    let file = tpl_fixture(&[("missing", d)]);
    let imported = read_tpl_with(&file, &photocraft_raster::Interrupt::NONE).unwrap();
    assert_eq!(imported.skipped, 1);
    assert!(imported.presets.is_empty());
    assert!(read_tpl_with(&file, &photocraft_raster::Interrupt::new(&|| true, &|_| {})).is_err());
}

#[test]
fn v6_settings_map_onto_brush_sections() {
    for sub in [1, 2] {
        let computed = Descriptor::new("brushPreset")
            .with("Nm  ", t("Round 19"))
            .with("Brsh", Value::Descriptor(Descriptor::new("computedBrush").with("Dmtr", u(b"#Pxl", 19.0)).with("Hrdn", prc(0.0)).with("Spcn", prc(10.0))));
        let bytes = write_v6(sub, &[sample("$tip-1", 16, 8, 8)], &[grit_pattern()], &[full_preset(), computed], true).unwrap();
        let imp = read_abr(&bytes, "Test").unwrap();
        assert_eq!(imp.presets.len(), 2);
        let p = &imp.presets[0];
        assert_eq!((p.name.as_str(), p.group.as_str(), p.builtin), ("Kitchen Sink", "Test", false));
        let b = &p.brush;
        let TipShape::Sampled(tile) = &b.tip else { panic!("sampled tip expected") };
        assert_eq!((tile.width, tile.height), (16, 8));
        assert_eq!((tile.data[0], tile.data[1]), (65535, 0));
        assert_eq!((b.size, b.angle, b.roundness, b.spacing, b.flip_x, b.flip_y), (64.0, 30.0, 0.8, 0.4, true, false));
        let sd = &b.shape_dynamics;
        assert!(sd.enabled && sd.flip_x_jitter && !sd.flip_y_jitter);
        assert_eq!((sd.size.control, sd.size.jitter, sd.size.minimum), (Control::PenPressure, 0.2, 0.25));
        assert_eq!((sd.angle.control, sd.roundness.control, sd.roundness.minimum), (Control::Direction, Control::PenTilt, 0.1));
        let sc = &b.scattering;
        assert!(sc.enabled && sc.both_axes);
        assert_eq!((sc.count, sc.scatter.jitter, sc.scatter.control, sc.scatter.fade_steps, sc.count_jitter.jitter), (3, 2.5, Control::Fade, 40, 0.5));
        let tx = &b.texture;
        assert!(tx.enabled && tx.each_tip && tx.invert);
        assert_eq!((tx.scale, tx.brightness, tx.contrast, tx.mode, tx.depth), (1.5, -0.5, 0.25, MaskMode::ColorBurn, 0.6));
        assert_eq!((tx.depth_jitter.jitter, tx.depth_jitter.minimum), (0.3, 0.2));
        let Pattern::Tile(pt) = &tx.pattern else { panic!("embedded pattern expected, got {:?}", tx.pattern) };
        assert_eq!((pt.width, pt.height, pt.data[3]), (4, 2, 65535));
        let db = &b.dual_brush;
        assert!(db.enabled && db.flip && db.both_axes && db.tip == TipShape::Round);
        assert_eq!((db.size, db.hardness, db.mode, db.spacing, db.count, db.scatter), (12.0, 0.5, MaskMode::HardMix, 0.3, 2, 1.2));
        let cd = &b.color_dynamics;
        assert!(cd.enabled && !cd.per_tip);
        assert_eq!((cd.fg_bg.jitter, cd.hue_jitter, cd.saturation_jitter, cd.brightness_jitter, cd.purity), (0.4, 0.1, 0.2, 0.3, -0.5));
        assert!(b.transfer.enabled);
        assert_eq!((b.transfer.opacity.control, b.transfer.opacity.minimum), (Control::PenPressure, 0.1));
        assert_eq!((b.transfer.flow.control, b.transfer.flow.jitter), (Control::Fade, 0.15));
        assert!(b.pose.enabled && b.pose.override_rotation && !b.pose.override_tilt);
        assert_eq!((b.pose.tilt_x, b.pose.rotation), (45.0, 90.0));
        assert!(b.wet_edges && b.noise && b.build_up);
        assert_eq!((b.opacity, b.flow, b.smoothing.amount, b.pressure_size), (0.8, 0.6, 0.4, true));
        assert!(imp.warnings.iter().any(|w| w.contains("someFutureKey")), "{:?}", imp.warnings);
        // The computed preset.
        let r = &imp.presets[1].brush;
        assert_eq!((r.tip.clone(), r.size, r.hardness, r.spacing, r.pressure_size), (TipShape::Round, 19.0, 0.0, 0.1, false));
    }
}

#[test]
fn missing_pattern_and_tip_are_reported() {
    let orphan = Descriptor::new("brushPreset").with("Nm  ", t("Orphan")).with("Brsh", Value::Descriptor(sampled_tip("$nowhere")));
    let bytes = write_v6(2, &[sample("$tip-1", 4, 4, 16), sample("$tip-2", 4, 4, 8)], &[], &[full_preset(), orphan], false).unwrap();
    let imp = read_abr(&bytes, "x").unwrap();
    assert_eq!(imp.presets.len(), 1);
    assert!(matches!(imp.presets[0].brush.texture.pattern, Pattern::Procedural { .. }));
    assert!(imp.warnings.iter().any(|w| w.contains("Grit")), "{:?}", imp.warnings);
    assert!(imp.warnings.iter().any(|w| w.contains("Orphan")), "{:?}", imp.warnings);
}

#[test]
fn v1_v2_brushes_become_presets() {
    let brushes = vec![
        LegacyBrush { name: String::new(), spacing: 25, anti_alias: true, tip: LegacyTip::Computed { diameter: 13, hardness: 50, angle: 20, roundness: 60 } },
        LegacyBrush { name: "Dune Grass".into(), spacing: 0, anti_alias: false, tip: LegacyTip::Sampled(sample("", 30, 10, 8)) },
    ];
    for version in [1, 2] {
        let imp = read_abr(&write_v12(version, &brushes, true).unwrap(), "Old").unwrap();
        assert_eq!(imp.version, version);
        assert_eq!(imp.presets.len(), 2);
        let c = &imp.presets[0];
        assert_eq!(c.name, "13 px Round");
        assert_eq!((c.brush.size, c.brush.hardness, c.brush.angle, c.brush.roundness, c.brush.spacing), (13.0, 0.5, 20.0, 0.6, 0.25));
        let s = &imp.presets[1];
        assert_eq!(s.name, if version == 2 { "Dune Grass" } else { "Sampled Brush 2" });
        assert_eq!((s.brush.size, s.brush.spacing, s.brush.aliased), (30.0, 0.25, true));
    }
}

#[test]
fn v6_without_settings_lists_tips() {
    let bytes = write_v6(1, &[sample("$a", 8, 8, 8), sample("$b", 3, 9, 16)], &[], &[], true).unwrap();
    // An empty `Brsh` list: presets come from the tips.
    let imp = read_abr(&bytes, "g").unwrap();
    assert_eq!(imp.presets.iter().map(|p| p.brush.size).collect::<Vec<_>>(), vec![8.0, 9.0]);
}

#[test]
fn big_tips_are_downsampled() {
    let s = AbrSample { id: "$big".into(), width: 3000, height: 1500, depth: 8, data: vec![200; 3000 * 1500] };
    let bytes = write_v6(2, &[s], &[], &[], true).unwrap();
    let imp = read_abr(&bytes, "g").unwrap();
    let TipShape::Sampled(t) = &imp.presets[0].brush.tip else { panic!() };
    assert_eq!((t.width, t.height), (MAX_TIP_EDGE, 1250));
    assert_eq!(imp.presets[0].brush.size, 3000.0);
    assert!(imp.warnings.iter().any(|w| w.contains("downsampled")));
}

#[test]
fn tilt_scale_projection_spacing_and_mixer_fields_map() {
    let tip = Descriptor::new("computedBrush").with("Dmtr", u(b"#Pxl", 30.0)).with("Spcn", prc(35.0)).with("Intr", Value::Boolean(false));
    let preset = Descriptor::new("brushPreset")
        .with("Nm  ", t("Wet Pen"))
        .with("Brsh", Value::Descriptor(tip))
        .with("useTipDynamics", Value::Boolean(true))
        .with("szVr", var(3, 0.0, 0.0))
        .with("tiltScale", prc(150.0))
        .with("brushProjection", Value::Boolean(true))
        .with("usePaintDynamics", Value::Boolean(true))
        .with("wtVr", var(2, 30.0, 10.0))
        .with("mxVr", var(1, 45.0, 0.0))
        .with(
            "toolOptions",
            Value::Descriptor(
                Descriptor::new("currentToolOptions")
                    .with("wetness", prc(80.0))
                    .with("dryness", prc(20.0))
                    .with("mix", prc(65.0))
                    .with("flow", prc(70.0))
                    .with("sampleMerged", Value::Boolean(true)),
            ),
        );
    let plain =
        Descriptor::new("brushPreset").with("Nm  ", t("Plain")).with("Brsh", Value::Descriptor(Descriptor::new("computedBrush").with("Dmtr", u(b"#Pxl", 9.0))));
    let bytes = write_v6(2, &[], &[], &[preset, plain], true).unwrap();
    let imp = read_abr(&bytes, "Mixer").unwrap();
    let b = &imp.presets[0].brush;
    assert!(!b.spacing_enabled && (b.spacing - 0.35).abs() < 1e-6, "Intr off = Spacing unchecked, the value kept");
    let sd = &b.shape_dynamics;
    assert!(sd.brush_projection && (sd.tilt_scale - 1.5).abs() < 1e-6 && sd.size.control == Control::PenTilt);
    let tr = &b.transfer;
    assert_eq!((tr.wetness.control, tr.wetness.jitter, tr.wetness.minimum), (Control::PenPressure, 0.3, 0.1));
    assert_eq!((tr.mix.control, tr.mix.jitter), (Control::Fade, 0.45));
    let m = &b.mixer;
    assert_eq!((m.wet, m.load, m.mix, m.flow, m.sample_all_layers), (0.8, 0.2, 0.65, 0.7, true));
    assert!(!imp.warnings.iter().any(|w| w.contains("Tilt Scale") || w.contains("Projection") || w.contains("Mixer")), "{:?}", imp.warnings);
    // Defaults when the file says nothing.
    let p = &imp.presets[1].brush;
    assert!(p.spacing_enabled && !p.shape_dynamics.brush_projection && p.shape_dynamics.tilt_scale == 0.0);
    assert_eq!(p.mixer, photocraft_paint::MixerSettings::default());
    assert_eq!((p.transfer.wetness, p.transfer.mix), (Dynamic::default(), Dynamic::default()));
}

#[test]
fn garbage_is_an_error() {
    assert!(read_abr(b"", "g").is_err());
    assert!(read_abr(b"8BPS not a brush", "g").is_err());
    assert!(read_abr(&[0, 6, 0, 2], "g").is_err());
}

#[test]
fn downsampled_pattern_keeps_native_period_and_user_scale() {
    let mut p = grit_pattern();
    p.width = 2048;
    p.height = 1024;
    p.channels = vec![vec![128; 2048 * 1024]];
    let file = AbrFile { patterns: vec![p], samples: vec![sample("$tip-1", 64, 64, 8)], ..Default::default() };
    let mut m = Mapper { file: &file, warnings: Default::default(), unknown: Default::default() };
    let b = m.preset(&full_preset()).unwrap();
    assert_eq!(b.texture.scale, 1.5);
    assert_eq!(b.texture.tile_scale, [2., 2.]);
    let Pattern::Tile(t) = b.texture.pattern else { panic!("embedded tile") };
    assert_eq!((t.width, t.height), (1024, 512));
}
#[test]
fn fractional_dual_size_survives_tool_preset_import_and_serialization() {
    let mut d = full_preset();
    d.class_id = Id::new("MixB");
    d.items.retain(|(k, _)| !k.is("dualBrush"));
    d = d.with(
        "dualBrush",
        Value::Descriptor(
            Descriptor::new("dualBrush")
                .with("useDualBrush", Value::Boolean(true))
                .with("Brsh", Value::Descriptor(Descriptor::new("computedBrush").with("Dmtr", u(b"#Pxl", 3017.375)))),
        ),
    );
    let imp = read_tpl_with(&tpl_fixture(&[("Large dual", d)]), &photocraft_raster::Interrupt::NONE).unwrap();
    let b = &imp.presets[0].brush;
    assert_eq!(b.dual_brush.size, 3017.375);
    let round: BrushSettings = serde_json::from_value(serde_json::to_value(b).unwrap()).unwrap();
    assert_eq!(&round, b);
}

#[test]
fn subpixel_dual_tip_keeps_the_photoshop_diameter() {
    let d = Descriptor::new("brushPreset").with("Brsh", Value::Descriptor(Descriptor::new("computedBrush").with("Dmtr", u(b"#Pxl", 80.77777777777644)))).with(
        "dualBrush",
        Value::Descriptor(
            Descriptor::new("dualBrush")
                .with("useDualBrush", Value::Boolean(true))
                .with("Brsh", Value::Descriptor(Descriptor::new("computedBrush").with("Dmtr", u(b"#Pxl", 0.9284802043422681)))),
        ),
    );
    let file = AbrFile::default();
    let mut m = Mapper { file: &file, warnings: Default::default(), unknown: Default::default() };
    let b = m.preset(&d).unwrap();
    assert!((b.dual_brush.size - 0.9284802).abs() < 1e-7);
}
