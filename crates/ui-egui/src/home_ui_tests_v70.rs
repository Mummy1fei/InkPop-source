use super::*;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc;

use crate::theme::ThemeKind;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

fn image(index: usize) -> egui::ColorImage {
    let size = match index % 4 {
        0 => [192, 96],
        1 => [108, 160],
        2 => [160, 160],
        _ => [192, 120],
    };
    let pixels = (0..size[1])
        .flat_map(|y| {
            (0..size[0]).map(move |x| {
                let band = (x / 32 + y / 30 + index) % 4;
                let colors = [[196, 104, 76], [69, 133, 152], [212, 176, 86], [100, 136, 102]];
                let [r, g, b] = colors[band];
                if (x as i32 - 110).pow(2) + (y as i32 - 48).pow(2) < 22 * 22 { Color32::from_rgb(242, 235, 214) } else { Color32::from_rgb(r, g, b) }
            })
        })
        .collect();
    egui::ColorImage::new(size, pixels)
}

fn recent() -> Vec<String> {
    (1..=8).map(|index| format!("/inkpop-synthetic/Study {index:02}.png")).collect()
}

struct Fixture {
    app: PhotocraftApp,
}

impl eframe::App for Fixture {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.app.frame += 1;
        // The real shell supplies the pasteboard fill before canvas::start_screen.
        // Reproduce it here so transparent dot texels are not captured as black.
        let backdrop = crate::prefs_ui::pasteboard_color(&self.app).unwrap_or(Tokens::get(ui.ctx()).canvas);
        ui.painter().rect_filled(ui.available_rect_before_wrap(), 0.0, backdrop);
        crate::canvas::start_screen(&mut self.app, ui);
    }
}

fn harness(kind: ThemeKind, size: egui::Vec2, gpu: bool) -> Harness<'static, Fixture> {
    let builder = Harness::builder().with_size(size).with_max_steps(64);
    let builder = if gpu { builder.wgpu_setup(crate::gpu_canvas::wgpu_setup()) } else { builder };
    builder.build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, kind);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.ui.recent_files = recent();
        app.services.request_recent_thumbnail = Some(Box::new(|path| {
            let index = path.bytes().rfind(u8::is_ascii_digit).unwrap_or(b'0').saturating_sub(b'0') as usize;
            let (sender, receiver) = mpsc::channel();
            sender.send(Some(image(index))).unwrap();
            receiver
        }));
        if kind == ThemeKind::StudioLight {
            let colors = photocraft_engine::prefs::palette::StudioLightColors {
                colors: photocraft_engine::prefs::palette::builtin::BUILTIN_PALETTES[0].colors(),
                presets: Vec::new(),
            };
            crate::theme::palette::apply(&cc.egui_ctx, kind, &colors);
        }
        Fixture { app }
    })
}

fn update_cache(state: &mut HomeState, ctx: &egui::Context, frame: u64, paths: &[String], service: &crate::RecentThumbnailFn) {
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| state.update(ui.ctx(), frame, paths, Some(service)));
    output.textures_delta.clear();
}

#[test]
fn home_preview_has_one_inflight_and_caches_success_and_failure_without_repeating_v70() {
    let ctx = egui::Context::default();
    let mut state = HomeState::default();
    let count = Rc::new(Cell::new(0));
    let counted = count.clone();
    let (sender, receiver) = mpsc::channel();
    let receiver = std::cell::RefCell::new(Some(receiver));
    let service: crate::RecentThumbnailFn = Box::new(move |_| {
        counted.set(counted.get() + 1);
        receiver.borrow_mut().take().unwrap()
    });
    let paths = vec!["/inkpop-synthetic/one.psd".into()];
    for frame in 0..12 {
        update_cache(&mut state, &ctx, frame, &paths, &service);
    }
    assert_eq!(count.get(), 1, "waiting for a worker must not start a request every frame");
    sender.send(None).unwrap();
    for frame in 12..24 {
        update_cache(&mut state, &ctx, frame, &paths, &service);
    }
    assert_eq!(count.get(), 1, "failed previews are memoized for this Home visit");
    assert!(state.cache[&paths[0]].is_none());
}

#[test]
fn recent_preview_shares_one_physical_boundary_with_its_transparency_grid_v117() {
    for ppp in [1.0_f32, 1.25, 1.5, 2.0] {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let paths = vec!["opaque".to_owned(), "alpha".to_owned()];
        let service: crate::RecentThumbnailFn = Box::new(|path| {
            let mut pixels = egui::ColorImage::filled([33, 22], Color32::RED);
            if path == "alpha" {
                pixels.pixels[330] = Color32::TRANSPARENT;
            }
            let (sender, receiver) = mpsc::channel();
            sender.send(Some(pixels)).unwrap();
            receiver
        });
        for frame in 0..4 {
            update_cache(&mut app.home, &ctx, frame, &paths, &service);
        }
        let checker_id = app.home.checker.as_ref().unwrap().id();
        for (index, path) in paths.iter().enumerate() {
            let preview = app.home.cache[path].as_ref().unwrap();
            assert_eq!(preview.has_alpha, index == 1);
            let mut input = egui::RawInput::default();
            input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
            let output = ctx.run_ui(input, |ui| {
                ui.add_space(11.35);
                card(&app, ui, path, 155.63, 103.75);
            });
            let clipped_rect = |id| {
                output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(r) if r.brush.as_ref().is_some_and(|b| b.fill_texture_id == id) => Some((r.rect, r.corner_radius, shape.clip_rect)),
                    _ => None,
                })
            };
            let (rect, corners, clip) = clipped_rect(preview.texture.id()).unwrap();
            assert_eq!(rect, rect.round_to_pixels(ppp));
            // The letterbox backing has the actual card painter's clip, including
            // any fractional edges inherited from the parent layout.
            let card_clip = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(r) if r.brush.is_none() && r.fill == Tokens::get(&ctx).field => Some(shape.clip_rect),
                    _ => None,
                })
                .unwrap();
            assert_eq!(clip, rect.intersect(card_clip));
            assert_eq!(clipped_rect(checker_id), (index == 1).then_some((rect, corners, clip)));
        }
    }
}

#[test]
fn home_preview_invalidation_reloads_once_and_bounds_the_completed_cache_to_eight_v70() {
    let ctx = egui::Context::default();
    let mut state = HomeState::default();
    let count = Rc::new(Cell::new(0));
    let counted = count.clone();
    let service: crate::RecentThumbnailFn = Box::new(move |_| {
        counted.set(counted.get() + 1);
        let (sender, receiver) = mpsc::channel();
        sender.send(Some(image(0))).unwrap();
        receiver
    });
    let paths = recent();
    for frame in 0..12 {
        update_cache(&mut state, &ctx, frame, &paths, &service);
    }
    assert_eq!(count.get(), 8);
    assert_eq!(state.cache.len(), 8);
    assert!(state.cache.values().all(Option::is_some));
    state.invalidate(&paths[0]);
    for frame in 12..16 {
        update_cache(&mut state, &ctx, frame, &paths, &service);
    }
    assert_eq!(count.get(), 9);
    assert_eq!(state.cache.len(), 8);
    // Returning from a document causes one service refresh of each path.
    for frame in 30..42 {
        update_cache(&mut state, &ctx, frame, &paths, &service);
    }
    assert_eq!(count.get(), 17);
    assert_eq!(state.cache.len(), 8);
}

#[test]
fn home_recent_cards_keep_order_and_reflow_without_horizontal_overflow_v70() {
    for (width, expected) in [(2400.0, 4), (1440.0, 4), (900.0, 3), (470.0, 2), (360.0, 1)] {
        let mut h = harness(ThemeKind::StudioLight, vec2(width, 900.0), false);
        h.run_steps(12);
        let first = h.get_by_label("Study 01.png").rect();
        let visible: Vec<_> = (1..=8).filter_map(|index| h.query_by_label(&format!("Study {index:02}.png")).map(|node| node.rect())).collect();
        let same_row = visible.iter().filter(|rect| (rect.top() - first.top()).abs() < 1.0).count();
        assert_eq!(same_row, expected, "actual response geometry at {width}");
        for rect in visible {
            assert!(rect.left() >= 0.0 && rect.right() <= width + 0.5, "card must remain horizontally reachable at {width}: {rect:?}");
            assert!(rect.bottom() < h.get_by_label(crate::welcome_links::COPYRIGHT).rect().top(), "only complete cards above the footer may be displayed");
        }
        if width >= 900.0 {
            assert!(h.get_by_label_contains("New document").rect().right() < first.left(), "actions belong to the left column");
        }
        let title = h.get_by_label(photocraft_engine::build_info::CUSTOM_PRODUCT_NAME).rect();
        let action = h.get_by_label_contains("New document").rect();
        assert!((title.left() - action.left()).abs() < 1.0, "Home title and actions should align at {width}");
        assert!(title.bottom() < action.top(), "the title should stay above its actions");
        assert_eq!(crate::welcome_links::COPYRIGHT, "©2026 Mummy-fei · InkPop");
        if width == 2400.0 {
            let last = h.get_by_label("Study 04.png").rect();
            let grid_center = (first.left() + last.right()) / 2.0;
            let actions = h.get_by_label_contains("New document").rect();
            assert!(actions.left() >= 300.0 && actions.top() >= 100.0, "wide Home actions need meaningful side and top breathing room: {actions:?}");
            let main_center = (actions.left() + 208.0 + 32.0 + width - actions.left()) / 2.0;
            assert!((grid_center - main_center).abs() <= 1.0, "wide-window grid must center in the main column: {grid_center} vs {main_center}");
            assert!((h.get_by_label("Recent").rect().left() - first.left()).abs() <= 1.0, "heading and card rows move together");
        }
        assert!(h.query_by_label("Join us on Discord").is_none());
        assert!(h.query_by_label("open source").is_none());
        assert!(h.get_by_label(crate::welcome_links::COPYRIGHT).rect().top() > first.bottom(), "copyright follows the cards, never overlays them");
    }
}

#[test]
fn home_action_buttons_keep_new_dialog_and_file_picker_behaviors_v70() {
    let mut h = harness(ThemeKind::Pro, vec2(1440.0, 900.0), false);
    h.run_steps(12);
    h.get_by_label_contains("New document").click();
    h.run_steps(3);
    assert_eq!(h.state().app.ui.dialogs.len(), 1);
    assert_eq!(h.state().app.ui.dialogs[0].kind, crate::state::DialogKind::NewDocument);
    let calls = Rc::new(Cell::new(0));
    let counted = calls.clone();
    h.state_mut().app.services.pick_open_paths = Some(Box::new(move || {
        counted.set(counted.get() + 1);
        None
    }));
    h.get_by_label_contains("Open…").click();
    h.run_steps(3);
    assert_eq!(calls.get(), 1);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn home_recent_card_click_opens_the_corresponding_local_document_v70() {
    use std::io::Write;
    static NEXT_FILE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    struct Temporary(std::path::PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let key = NEXT_FILE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let file = Temporary(std::env::temp_dir().join(format!("inkpop-home-v70-{}-{key}.pcraft", std::process::id())));
    std::fs::OpenOptions::new().write(true).create_new(true).open(&file.0).unwrap().write_all(b"home test synthetic document").unwrap();
    let path = file.0.to_string_lossy().into_owned();
    let mut h = harness(ThemeKind::Pro, vec2(1440.0, 900.0), false);
    let mut source = photocraft_engine::Session::new();
    source.execute("file.new", serde_json::json!({"width":4,"height":3})).unwrap();
    let document = source.active().unwrap().doc.as_ref().clone();
    let calls = Rc::new(Cell::new(0));
    let counted = calls.clone();
    h.state_mut().app.services.import = Some(Box::new(move |_, bytes| {
        assert_eq!(bytes, b"home test synthetic document");
        counted.set(counted.get() + 1);
        Ok((document.clone(), Vec::new()))
    }));
    h.state_mut().app.ui.recent_files = vec![path.clone()];
    h.run_steps(4);
    h.get_by_label(&crate::file_open::display_name(&path)).click();
    h.run_steps(3);
    assert_eq!(calls.get(), 1);
    assert_eq!(h.state().app.session.active().unwrap().path.as_deref(), Some(path.as_str()));
    assert_eq!(h.state().app.ui.recent_files.first(), Some(&path));
}

#[test]
#[ignore = "writes five-theme, reference-size, empty and small synthetic Home screenshots for root review"]
fn home_gpu_visual_v99() {
    let _language = crate::i18n::language_scope(crate::i18n::Lang::from_code("zh-hans").unwrap());
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v99/home-gpu");
    assert!(!out.exists(), "preserve previous visual evidence");
    std::fs::create_dir_all(&out).unwrap();
    for kind in ThemeKind::ALL {
        let size = if kind == ThemeKind::StudioLight { vec2(2400.0, 1350.0) } else { vec2(1440.0, 900.0) };
        let mut h = harness(kind, size, true);
        h.state_mut().app.ui.recent_files = (1..=20).map(|index| format!("/inkpop-synthetic/Study {index:02}.png")).collect();
        h.run_steps(24);
        assert!((12..=20).contains(&h.state().app.home.cache.len()));
        h.render().unwrap().save(out.join(format!("{}.png", kind.id()))).unwrap();
        if kind == ThemeKind::Classic {
            let card = h.get_by_label("Study 01.png").rect();
            h.event(egui::Event::PointerMoved(card.center()));
            h.run_steps(2);
            h.render().unwrap().save(out.join("classic-hover.png")).unwrap();
        }
    }
    let mut h = harness(ThemeKind::StudioLight, vec2(470.0, 760.0), true);
    h.state_mut().app.ui.recent_files = (1..=20).map(|index| format!("/inkpop-synthetic/Study {index:02}.png")).collect();
    h.run_steps(24);
    h.render().unwrap().save(out.join("small.png")).unwrap();
    h.event(egui::Event::PointerMoved(pos2(240.0, 350.0)));
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -5000.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(24);
    h.render().unwrap().save(out.join("small-after-wheel.png")).unwrap();
    let mut h = harness(ThemeKind::StudioLight, vec2(2190.0, 1290.0), true);
    h.state_mut().app.ui.recent_files = (1..=20).map(|index| format!("/inkpop-synthetic/Study {index:02}.png")).collect();
    h.run_steps(24);
    h.render().unwrap().save(out.join("reference-size.png")).unwrap();
    h.state_mut().app.ui.recent_files.clear();
    h.run_steps(3);
    h.render().unwrap().save(out.join("empty.png")).unwrap();
}

#[test]
fn home_twenty_recent_documents_use_five_complete_rows_on_one_page_v71() {
    let mut h = harness(ThemeKind::StudioLight, vec2(2400.0, 1350.0), false);
    h.state_mut().app.ui.recent_files = (1..=20).map(|index| format!("/inkpop-synthetic/Study {index:02}.png")).collect();
    h.run_steps(24);
    let first = h.get_by_label("Study 01.png").rect();
    let last = h.get_by_label("Study 20.png").rect();
    assert!(last.top() > first.bottom() * 2.0, "the list must extend beyond the original two rows");
    assert!(last.bottom() < 1290.0, "five rows should fit a tall viewport: {last:?}");
    assert_eq!(h.state().app.home.cache.len(), 20);
    assert!(h.get_by_label(crate::welcome_links::COPYRIGHT).rect().top() > last.bottom());
    h.state_mut().app.ui.recent_files = (1..=120).map(|index| format!("/inkpop-synthetic/Study {index:03}.png")).collect();
    h.run_steps(104);
    assert_eq!(h.state().app.home.cache.len(), 20, "only the visible page receives preview textures");
    assert!(h.query_by_label("Study 020.png").is_some());
    assert!(h.query_by_label("Study 021.png").is_none());
    assert_eq!(h.state().app.ui.recent_files.len(), 120, "Home never truncates the stored history");
}

#[test]
fn home_small_window_displays_only_complete_cards_and_does_not_scroll_v71() {
    let mut h = harness(ThemeKind::Pro, vec2(470.0, 760.0), false);
    h.state_mut().app.ui.recent_files = (1..=20).map(|index| format!("/inkpop-synthetic/Study {index:02}.png")).collect();
    h.run_steps(24);
    let first = h.get_by_label("Study 01.png").rect();
    let shown: Vec<_> = (1..=20).filter_map(|index| h.query_by_label(&format!("Study {index:02}.png")).map(|node| node.rect())).collect();
    assert!(!shown.is_empty() && shown.len() < 20);
    let footer = h.get_by_label(crate::welcome_links::COPYRIGHT).rect();
    for rect in &shown {
        assert!(rect.top() > 0.0 && rect.bottom() < footer.top(), "each displayed card must fit in full: {rect:?}");
    }
    assert!(h.query_by_label("Study 20.png").is_none());
    h.event(egui::Event::PointerMoved(pos2(240.0, 350.0)));
    h.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -5000.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    });
    h.run_steps(24);
    assert_eq!(h.get_by_label("Study 01.png").rect(), first, "wheel input must not move the single-screen Home");
    assert_eq!(h.state().app.home.cache.len(), shown.len());
    assert_eq!(h.state().app.ui.recent_files.len(), 20);
    assert!(h.get_by_label(crate::welcome_links::COPYRIGHT).rect().bottom() < 742.0, "footer must remain on screen");
}
