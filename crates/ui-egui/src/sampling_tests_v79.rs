use super::*;
use crate::{
    canvas::{ToolEvent, tool_event},
    state::Tool,
};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width":200,"height":40,"background":"white"})).unwrap();
    app
}

#[test]
fn mixer_alt_uses_independent_pickup_size_and_all_layers_without_rewriting_preset_v79() {
    let mut app = app();
    app.run("shape.create", json!({"kind":"rect","rect":[90,0,20,40],"fill":"#0000ff"})).unwrap();
    app.run("layer.new.layer", json!({"name":"Empty painting layer"})).unwrap();
    app.run("tools.setBrush", json!({"brush":{"size":135,"mixer":{"sampleAllLayers":false}}})).unwrap();
    app.ui.tool = Tool::MixerBrush;
    app.sync_tool_brush();
    let before = app.session.tools.brush.clone();
    for all in [true, false] {
        app.ui.sampling.mixer_sample_all_layers = all;
        tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 20.0, pressure: 1.0 }, egui::Modifiers::ALT);
        let (_, params) = app.session.journal.iter().rev().find(|(id, _)| id == "tools.mixer.load").unwrap();
        assert_eq!(params["sampleSize"], json!(1));
        assert_eq!(params["sampleAllLayers"], json!(all));
        let nib = &app.session.tools.mixer.loaded_nib;
        assert!(!nib.is_empty());
        if all {
            assert!(
                nib.iter().all(|color| color[2] > 0.99 && color[0] < 0.01 && color[1] < 0.01),
                "point pickup samples the narrow blue band, not the135px white surroundings"
            );
        } else {
            assert!(nib.iter().all(|color| color[3] == 0.0), "current empty layer is a real distinct source");
        }
        tool_event(&mut app, ToolEvent::Up { x: 100.0, y: 20.0 }, egui::Modifiers::ALT);
        assert_eq!(app.session.tools.brush, before, "UI override must not rewrite a private preset");
    }
}

#[test]
fn sample_settings_round_trip_preserves_explicit_false_and_excludes_runtime_cache_v79() {
    let mut app = app();
    assert!(app.ui.sampling.mixer_sample_all_layers);
    app.ui.sampling.size = 31;
    app.ui.sampling.source = Source::CurrentBelowNoAdjustments;
    app.ui.sampling.show_ring = false;
    app.ui.sampling.mixer_size = 5;
    app.ui.sampling.mixer_sample_all_layers = false;
    begin(&mut app, false);
    save(&mut app);
    let stored = app.session.prefs().dialogs.get(STORAGE_KEY).unwrap();
    assert_eq!(stored["mixerSampleAllLayers"], json!(false));
    assert!(stored.get("ring").is_none() && stored.get("cache").is_none());
    let text = app.session.prefs_to_json();
    let mut restored = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    restored.session.load_prefs_json(&text).unwrap();
    load(&mut restored);
    assert_eq!(restored.ui.sampling.size, 31);
    assert_eq!(restored.ui.sampling.source, Source::CurrentBelowNoAdjustments);
    assert_eq!(restored.ui.sampling.mixer_size, 5);
    assert!(!restored.ui.sampling.show_ring && !restored.ui.sampling.mixer_sample_all_layers);
    assert!(restored.ui.sampling.ring.is_none() && restored.ui.sampling.cache.is_none());
}

#[test]
fn mixer_live_preview_and_final_commit_share_independent_sample_all_override_v79() {
    let mut app = app();
    app.run("layer.new.layer", json!({"name":"Paint"})).unwrap();
    app.run(
        "tools.setBrush",
        json!({"brush":{"size":12,"pressureSize":false,
        "mixer":{"wet":0,"load":1,"mix":0,"flow":1,"multicolor":true,"sampleAllLayers":false}}}),
    )
    .unwrap();
    app.ui.tool = Tool::MixerBrush;
    app.sync_tool_brush();
    app.background_jobs = false;
    let before = app.session.tools.brush.clone();
    tool_event(&mut app, ToolEvent::Down { x: 30.0, y: 20.0, pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(&mut app, ToolEvent::Move { x: 60.0, y: 20.0, pressure: 1.0 }, egui::Modifiers::NONE);
    let first = crate::mixer_preview::display_doc(&mut app, 0).unwrap().1;
    app.ui.sampling.mixer_sample_all_layers = false;
    let second = crate::mixer_preview::display_doc(&mut app, 0).unwrap().1;
    assert_ne!(first, second, "changing the independent source invalidates live preview without an extra stroke point");
    tool_event(&mut app, ToolEvent::Up { x: 60.0, y: 20.0 }, egui::Modifiers::NONE);
    let (_, params) = app.session.journal.iter().rev().find(|(id, _)| id == "paint.mixerBrush").unwrap();
    assert_eq!(params["sampleAllLayers"], json!(false));
    assert_eq!(app.session.tools.brush, before, "preview/commit source override does not rewrite the selected preset");
}

#[test]
fn sampler_cache_keys_include_document_layer_size_and_source_v79() {
    let mut app = app();
    app.run("shape.create", json!({"kind":"rect","rect":[0,0,200,40],"fill":"#ff0000"})).unwrap();
    app.ui.sampling.size = 3;
    assert_eq!(color(&mut app, 50.0, 20.0), Some([1.0, 0.0, 0.0]));
    let journal = app.session.journal.len();
    assert_eq!(color(&mut app, 50.1, 20.1), Some([1.0, 0.0, 0.0]));
    assert_eq!(app.session.journal.len(), journal, "same source pixel/settings need no new command");
    app.run("layer.new.layer", json!({"name":"Empty"})).unwrap();
    app.ui.sampling.source = Source::Current;
    assert!(color(&mut app, 50.0, 20.0).is_none(), "transparent current layer must not return cached composite red");
    app.ui.sampling.source = Source::All;
    assert_eq!(color(&mut app, 50.0, 20.0), Some([1.0, 0.0, 0.0]));
    app.run("file.new", json!({"width":200,"height":40,"background":"#0000ff"})).unwrap();
    assert_eq!(color(&mut app, 50.0, 20.0), Some([0.0, 0.0, 1.0]), "same coordinates/revision in another document must not reuse red");
    app.ui.sampling.size = 101;
    assert_eq!(color(&mut app, 100.0, 20.0), Some([0.0, 0.0, 1.0]));
    for invalid in [f64::NAN, f64::INFINITY] {
        assert!(color(&mut app, invalid, 20.0).is_none());
    }
}

#[test]
fn actual_sampling_ring_shapes_show_current_and_previous_only_while_enabled_and_pressed_v79() {
    let mut app = app();
    app.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
    begin(&mut app, false);
    sampled(&mut app, [0.0, 0.0, 1.0]);
    let ctx = egui::Context::default();
    let at = egui::pos2(100.0, 100.0);
    for show in [true, false] {
        app.ui.sampling.show_ring = show;
        let input = egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
            ],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| draw_ring(&app, ui.painter(), at));
        let paths: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Path(path) => Some(path),
                _ => None,
            })
            .collect();
        assert_eq!(paths.len(), if show { 2 } else { 0 });
        if show {
            assert!(paths.iter().any(|path| path.stroke.color == egui::epaint::ColorMode::Solid(Color32::BLUE)));
            assert!(paths.iter().any(|path| path.stroke.color == egui::epaint::ColorMode::Solid(Color32::RED)));
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,Shape::Circle(circle) if circle.center==at&&circle.radius==33.0)));
        }
        output.textures_delta.clear();
    }
    end(&mut app);
    assert!(app.ui.sampling.ring.is_none());
}

struct Toolbar {
    options: Options,
    mixer: bool,
}
impl eframe::App for Toolbar {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui.horizontal(|ui| {
            if self.mixer {
                mixer_options(ui, &mut self.options);
            } else {
                eyedropper_options(ui, &mut self.options);
            }
        });
    }
}

#[test]
fn sample_size_source_ring_and_mixer_default_controls_use_real_pointer_input_five_themes_v79() {
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = Harness::builder().with_size(vec2(1200.0, 500.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, theme);
            Toolbar { options: Options::default(), mixer: false }
        });
        h.run_steps(4);
        h.query_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some(tl!("Point Sample"))).expect("sample size combobox").click();
        h.run_steps(2);
        for label in ["3 by 3 Average", "5 by 5 Average", "11 by 11 Average", "31 by 31 Average", "51 by 51 Average", "101 by 101 Average"] {
            assert!(h.query_by_label(tl!(label)).is_some(), "{theme:?}: offered {label}");
        }
        h.get_by_label(tl!("3 by 3 Average")).click();
        h.run_steps(2);
        assert_eq!(h.state().options.size, 3);
        h.query_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some(tl!("All Layers"))).expect("sample source combobox").click();
        h.run_steps(2);
        h.get_by_label(tl!("All Layers (No Adjustments)")).click();
        h.run_steps(2);
        assert_eq!(h.state().options.source, Source::AllNoAdjustments);
        h.get_by_label(tl!("Show Sampling Ring")).click();
        h.run_steps(2);
        assert!(!h.state().options.show_ring);
        h.state_mut().mixer = true;
        h.run_steps(3);
        assert!(h.state().options.mixer_sample_all_layers);
        h.get_by_label(tl!("Sample All Layers")).click();
        h.run_steps(2);
        assert!(!h.state().options.mixer_sample_all_layers);
        h.query_all_by_role(Role::ComboBox).find(|node| node.value().as_deref() == Some(tl!("Point Sample"))).expect("mixer pickup size combobox").click();
        h.run_steps(2);
        h.get_by_label(tl!("31 by 31 Average")).click();
        h.run_steps(2);
        assert_eq!(h.state().options.mixer_size, 31);
        assert_eq!(h.state().options.size, 3, "mixer's independent range never changes eyedropper averaging");
    }
}
