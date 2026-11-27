//! v67 native progress must be the client body, not a second modal card inside
//! a transparent native host. These tests exercise production layout and a real
//! nested viewport callback; OS composition is a separate native QA step.
use super::*;
use egui_kittest::{Harness, kittest::Queryable};

fn sample_job() -> JobInfo {
    JobInfo {
        id: JobId(67),
        command: "select.smart".to_string(),
        label: "Smart Object Selection".to_string(),
        document: None,
        state: "running",
        progress: 0.25,
        message: "Recognizing image objects".to_string(),
        elapsed_ms: 3600.0,
        result: None,
        error: None,
    }
}

struct ProgressFixture {
    job: JobInfo,
    lang: crate::i18n::Lang,
    body: Rect,
    cancelled: bool,
}

impl eframe::App for ProgressFixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let _language = crate::i18n::language_scope(self.lang);
        let response = native_dialog_body(ui, &self.job);
        self.body = response.response.rect;
        self.cancelled |= response.inner;
    }
}

fn harness(theme: crate::theme::ThemeKind, width: f32, long_status: bool) -> Harness<'static, ProgressFixture> {
    Harness::builder().with_size(vec2(width, NATIVE_DIALOG_SIZE.y)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut job = sample_job();
        if long_status {
            job.message = "Preparing a deliberately long local recognition status for a narrow progress window. ".repeat(3);
        }
        ProgressFixture { job, lang: crate::i18n::Lang::from_code("zh-hans").unwrap(), body: Rect::NOTHING, cancelled: false }
    })
}

fn fit_requested_height(harness: &mut Harness<'_, ProgressFixture>) {
    for _ in 0..4 {
        harness.run_steps(2);
        let requested = harness.output().viewport_output.values().flat_map(|output| &output.commands).find_map(|command| match command {
            egui::ViewportCommand::InnerSize(size) => Some(*size),
            _ => None,
        });
        let Some(size) = requested else { return };
        harness.set_size(size);
    }
    panic!("progress height failed to settle from the actual content layout");
}

fn opaque_body(shape: &egui::epaint::Shape, client: Rect, fill: Color32) -> bool {
    match shape {
        egui::epaint::Shape::Rect(rect) => rect.fill == fill && rect.fill.a() == 255 && rect.rect.contains_rect(client),
        egui::epaint::Shape::Vec(shapes) => shapes.iter().any(|shape| opaque_body(shape, client, fill)),
        _ => false,
    }
}

#[test]
fn native_progress_paints_the_full_client_and_fits_cancel_in_every_theme() {
    for theme in crate::theme::ThemeKind::ALL {
        for width in [360.0, NATIVE_DIALOG_SIZE.x] {
            for long in [false, true] {
                let mut h = harness(theme, width, long);
                fit_requested_height(&mut h);
                let body = h.state().body;
                let client = h.ctx.content_rect();
                assert_eq!(body, client, "{theme:?}: no transparent margin outside the actual panel");
                assert!(h.output().shapes.iter().any(|shape| opaque_body(&shape.shape, client, Tokens::get(&h.ctx).card)), "{theme:?}: full opaque theme fill");
                let cancel = crate::i18n::tr(h.state().lang, "Cancel");
                assert!(client.contains_rect(h.get_by_label(cancel).rect()), "{theme:?}: Cancel clipped at {width}");
                assert!(client.contains_rect(h.get_by_label("25%").rect()));
                h.get_by_label(cancel).click();
                h.run_steps(2);
                assert!(h.state().cancelled, "{theme:?}: body Cancel has the same action");
                // A user-enlarged client must not become a new intrinsic
                // measurement and start growing again on every progress tick.
                h.set_size(vec2(width, 600.0));
                h.run_steps(3);
                assert!(
                    !h.output()
                        .viewport_output
                        .values()
                        .flat_map(|output| &output.commands)
                        .any(|command| matches!(command, egui::ViewportCommand::InnerSize(_))),
                    "{theme:?}: intrinsic height must not depend on excess client height"
                );
            }
        }
    }
}

#[test]
fn progress_stage_is_translated_when_rendered_but_the_job_payload_is_unchanged() {
    let mut h = harness(crate::theme::ThemeKind::StudioLight, NATIVE_DIALOG_SIZE.x, false);
    fit_requested_height(&mut h);
    let expected = crate::i18n::tr(h.state().lang, "Recognizing image objects");
    assert_ne!(expected, "Recognizing image objects", "the Simplified Chinese phase must have a catalog entry");
    assert!(h.query_by_label(expected).is_some(), "the actual drawn phase must use the translation");
    assert!(h.query_by_label("Recognizing image objects").is_none());
    assert_eq!(h.state().job.message, "Recognizing image objects", "automation/raw job messages remain stable");
    assert_eq!(h.state().job.label, "Smart Object Selection");
}

#[test]
fn file_open_job_label_formats_the_known_template_without_translating_the_filename() {
    let language = crate::i18n::Lang::from_code("zh-hans").unwrap();
    let _language = crate::i18n::language_scope(language);
    for filename in ["Brushes", "Opening test image.png", "草图 {name}.psd"] {
        assert_eq!(display_job_label(OPEN_JOB, &format!("Opening {filename}")), crate::i18n::fmt(tl!("Opening {name}"), &[("name", filename)]));
    }
    assert_eq!(
        display_job_label("custom.plugin.operation", "Opening Brushes"),
        "Opening Brushes",
        "unknown command labels are not parsed as file-open templates"
    );
    assert_eq!(display_job_label("select.smart", "Smart Object Selection"), tl!("Smart Object Selection"));
}

fn text_rect(shape: &egui::epaint::Shape, wanted: &str) -> Option<Rect> {
    match shape {
        egui::epaint::Shape::Text(text) if text.galley.text() == wanted => Some(Rect::from_min_size(text.pos, text.galley.size())),
        egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_rect(shape, wanted)),
        _ => None,
    }
}

#[test]
fn real_native_progress_callback_is_opaque_and_cancel_or_esc_stops_the_job() {
    // The renderer is thread-local and has no unset API. Isolate it from the
    // other app harness tests while exercising a real ROOT -> child pass.
    std::thread::Builder::new()
        .name("v67-native-progress".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            use std::{
                cell::{Cell, RefCell},
                rc::Rc,
                sync::atomic::{AtomicBool, Ordering},
                time::Duration,
            };
            struct ReleaseGate(Arc<AtomicBool>);
            impl Drop for ReleaseGate {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Relaxed);
                }
            }
            let _language = crate::i18n::language_scope(crate::i18n::Lang::EN);
            let ctx = egui::Context::default();
            PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::StudioLight);
            ctx.set_embed_viewports(false);
            let size = Rc::new(Cell::new(NATIVE_DIALOG_SIZE));
            let events = Rc::new(RefCell::new(Vec::<egui::Event>::new()));
            let measured = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING, false)));
            let renderer_size = size.clone();
            let renderer_events = events.clone();
            let renderer_measured = measured.clone();
            egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
                if let Some(size) = viewport.builder.inner_size {
                    renderer_size.set(size);
                }
                let client = Rect::from_min_size(egui::Pos2::ZERO, renderer_size.get());
                let mut input =
                    egui::RawInput { viewport_id: viewport.ids.this, screen_rect: Some(client), events: renderer_events.take(), ..Default::default() };
                let info = input.viewports.entry(viewport.ids.this).or_default();
                info.parent = Some(viewport.ids.parent);
                info.inner_rect = Some(client);
                info.focused = Some(true);
                let mut output = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
                let button = output.shapes.iter().find_map(|shape| text_rect(&shape.shape, "Cancel")).unwrap_or(Rect::NOTHING);
                let filled = output.shapes.iter().any(|shape| opaque_body(&shape.shape, client, Tokens::get(ctx).card));
                renderer_measured.set((client, button, filled));
                for command in output.viewport_output.values().flat_map(|output| &output.commands) {
                    if let egui::ViewportCommand::InnerSize(size) = command {
                        renderer_size.set(*size);
                    }
                }
                output.textures_delta.clear();
            });
            for escape in [false, true] {
                let mut session = photocraft_engine::Session::new();
                session.execute("file.new", json!({"width":32,"height":24})).unwrap();
                let mut app = PhotocraftApp::new(session, Default::default());
                let before = app.session.active().unwrap().doc.clone();
                let gate = Arc::new(AtomicBool::new(false));
                let _release = ReleaseGate(gate.clone());
                let worker_gate = gate.clone();
                let job = app
                    .session
                    .start_job(
                        "test.v67-progress",
                        json!({}),
                        "Smart Object Selection",
                        true,
                        move |job| {
                            job.progress(0.25, "Recognizing image objects");
                            while !worker_gate.load(Ordering::Relaxed) {
                                job.check()?;
                                std::thread::sleep(Duration::from_millis(1));
                            }
                            Ok(())
                        },
                        |_, ()| Ok(json!({})),
                    )
                    .unwrap();
                let Started::Job(job) = job else { panic!("native job unexpectedly ran inline") };
                std::thread::sleep(Duration::from_millis(DIALOG_DELAY_MS as u64 + 20));
                let frame = |app: &mut PhotocraftApp, next_events: Vec<egui::Event>| {
                    *events.borrow_mut() = next_events;
                    let client = Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 800.0));
                    let mut input = egui::RawInput { screen_rect: Some(client), ..Default::default() };
                    input.viewports.entry(egui::ViewportId::ROOT).or_default().inner_rect = Some(client);
                    let mut output = ctx.run_ui(input, |ui| dialog(app, ui.ctx()));
                    output.textures_delta.clear();
                };
                for _ in 0..4 {
                    frame(&mut app, Vec::new());
                }
                let (client, button, filled) = measured.get();
                assert!(filled, "native callback must paint a full opaque client, not a nested modal card");
                assert!(button.is_positive() && client.contains_rect(button), "native Cancel must be reachable");
                if escape {
                    frame(
                        &mut app,
                        vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
                    );
                } else {
                    for pressed in [true, false] {
                        frame(
                            &mut app,
                            vec![
                                egui::Event::PointerMoved(button.center()),
                                egui::Event::PointerButton {
                                    pos: button.center(),
                                    button: egui::PointerButton::Primary,
                                    pressed,
                                    modifiers: Default::default(),
                                },
                            ],
                        );
                    }
                }
                gate.store(true, Ordering::Relaxed);
                assert!(app.session.job(job).is_none(), "Cancel/Esc must stop the running operation");
                assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "cancellation must not edit the document");
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
#[ignore = "exports v67 actual progress-body GPU screenshots in all five themes; root must inspect"]
fn progress_native_client_gpu_visual_v67() {
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v67/progress-gpu");
    assert!(!out.exists(), "preserve previous evidence");
    std::fs::create_dir_all(&out).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = Harness::builder().with_size(NATIVE_DIALOG_SIZE).with_max_steps(64).wgpu().build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, theme);
            ProgressFixture { job: sample_job(), lang: crate::i18n::Lang::from_code("zh-hans").unwrap(), body: Rect::NOTHING, cancelled: false }
        });
        fit_requested_height(&mut h);
        h.render().unwrap().save(out.join(format!("{}-progress.png", theme.id()))).unwrap();
    }
}
