//! Compare views are presentation state, with real Q/Y keyboard routing and no image edits.
use egui::{Key, Modifiers, vec2};
use egui_kittest::Harness;
use photocraft_ui_egui::{PhotocraftApp, camera_raw_ui, control, theme::ThemeKind};
use serde_json::{Value, json};

fn fixture(theme: ThemeKind, gpu: bool) -> Harness<'static, PhotocraftApp> {
    let builder = Harness::builder().with_size(vec2(1280.0, 860.0)).with_max_steps(64);
    let builder = if gpu { builder.wgpu() } else { builder };
    let mut h = builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut session = photocraft_engine::Session::new();
        session.execute("prefs.set", json!({"path":"interface.theme","value":theme.id()})).unwrap();
        let mut app = PhotocraftApp::new(session, Default::default());
        app.run("file.new", json!({"width":400,"height":300,"background":"#404040"})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("select.rect", json!({"x":40,"y":40,"width":100,"height":220})).unwrap();
        app.run("edit.fill", json!({"color":"#805020"})).unwrap();
        app.run("select.rect", json!({"x":220,"y":70,"width":100,"height":160})).unwrap();
        app.run("edit.fill", json!({"color":"#206080"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        camera_raw_ui::open(&mut app, &cc.egui_ctx).unwrap();
        camera_raw_ui::menu(&mut app, &cc.egui_ctx, "filter.cameraRaw", &json!({"ui":{"set":{"exposure":1.0},"view":{"maximized":true}}})).unwrap().unwrap();
        app
    });
    h.run_steps(5);
    assert_eq!(h.state().ui.theme, theme, "the app must retain the requested screenshot theme");
    h
}

fn inspect(h: &Harness<'_, PhotocraftApp>) -> Value {
    control::inspect(h.state(), &h.ctx)["cameraRaw"].clone()
}

fn menu(h: &mut Harness<'_, PhotocraftApp>, request: Value) -> Result<Value, String> {
    let ctx = h.ctx.clone();
    camera_raw_ui::menu(h.state_mut(), &ctx, "filter.cameraRaw", &json!({"ui":request})).unwrap()
}

const MODES: [&str; 5] = ["single", "sideBySide", "leftRightSplit", "topBottom", "topBottomSplit"];

#[test]
fn q_cycles_five_views_and_y_preserves_before_without_rendering_or_history() {
    let mut h = fixture(ThemeKind::ProMedium, false);
    let initial = inspect(&h);
    let history = h.state().session.active().unwrap().history.past_len();
    let document_revision = h.state().session.active().unwrap().revision;
    for mode in MODES.into_iter().skip(1).chain(["single"]) {
        h.key_press(Key::Q);
        h.run_steps(3);
        let state = inspect(&h);
        assert_eq!(state["view"]["comparison"], mode);
        assert_eq!(state["params"], initial["params"]);
        assert_eq!(state["previewRevision"], initial["previewRevision"]);
        assert_eq!(state["requestedRevision"], initial["requestedRevision"]);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), history);
        assert_eq!(h.state().session.active().unwrap().revision, document_revision);
    }
    for modifiers in [Modifiers::CTRL, Modifiers::ALT, Modifiers::SHIFT] {
        h.key_press_modifiers(modifiers, Key::Q);
        h.run_steps(2);
        assert_eq!(inspect(&h)["view"]["comparison"], "single", "modified Q must keep its own meaning");
    }
    h.key_press(Key::Y);
    h.run_steps(2);
    assert_eq!(inspect(&h)["before"], true);
    h.key_press(Key::Y);
    h.run_steps(2);
    assert_eq!(inspect(&h)["before"], false);
    assert_eq!(inspect(&h)["previewRevision"], initial["previewRevision"]);
    assert_eq!(h.state().session.active().unwrap().revision, document_revision);
}

#[test]
fn comparison_api_is_atomic_and_chosen_mode_is_remembered_when_reopening() {
    let mut h = fixture(ThemeKind::StudioLight, false);
    let original = inspect(&h);
    for value in [json!(null), json!(17), json!("invalid")] {
        assert!(menu(&mut h, json!({"view":{"comparison":value,"zoom":2.0},"before":true})).is_err());
        assert_eq!(inspect(&h), original);
    }
    for mode in MODES {
        menu(&mut h, json!({"view":{"comparison":mode,"swapped":true}})).unwrap();
        h.run_steps(3);
        assert_eq!(inspect(&h)["view"]["comparison"], mode);
    }
    menu(&mut h, json!({"view":{"zoom":2.0,"center":[0.7,0.7]}})).unwrap();
    menu(&mut h, json!({"cancel":true})).unwrap();
    let ctx = h.ctx.clone();
    camera_raw_ui::open(h.state_mut(), &ctx).unwrap();
    h.run_steps(3);
    let reopened = inspect(&h);
    assert_eq!(reopened["view"]["comparison"], "topBottomSplit");
    assert_eq!(reopened["view"]["swapped"], true);
    assert_eq!(reopened["view"]["zoom"], Value::Null, "a new image starts fitted even when comparison preference is retained");
    assert_eq!(reopened["view"]["center"], json!([0.5, 0.5]));
}

#[test]
#[ignore = "writes v62 five-theme Q comparison screenshots using production egui/wgpu"]
fn raw_comparison_gpu_visual_v62() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v62/comparison-gpu");
    assert!(!out.exists(), "preserve previous screenshot evidence");
    std::fs::create_dir_all(&out).unwrap();
    let mut captures = Vec::new();
    for theme in ThemeKind::ALL {
        let mut h = fixture(theme, true);
        let initial = inspect(&h);
        for mode in MODES {
            if mode != "single" {
                h.key_press(Key::Q);
                h.run_steps(4);
            }
            let state = inspect(&h);
            assert_eq!(state["view"]["comparison"], mode);
            assert_eq!(state["previewRevision"], initial["previewRevision"]);
            let path = format!("{}-{mode}.png", theme.id());
            h.render().unwrap().save(out.join(&path)).unwrap();
            captures.push(json!({"path":path,"theme":theme.id(),"mode":mode,"view":state["view"],
                "previewRevision":state["previewRevision"],"captureType":"production egui/wgpu, not OS compositor"}));
        }
    }
    std::fs::write(out.join("result.json"), serde_json::to_string_pretty(&json!({"passed":true,"captures":captures})).unwrap()).unwrap();
}
