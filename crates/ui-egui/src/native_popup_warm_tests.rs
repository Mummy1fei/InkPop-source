use super::*;

fn root_input() -> egui::RawInput {
    egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 760.0))), ..Default::default() }
}

#[test]
fn layer_context_warm_descriptor_is_hidden_deferred_normal_transparent_and_inert() {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let id = viewport("layer-context");
    for _ in 0..3 {
        let mut output = ctx.run_ui(root_input(), |ui| warm_layer_context(ui.ctx()));
        let descriptor = output.viewport_output.get(&id).expect("one predeclared layer host");
        assert!(descriptor.class == egui::ViewportClass::Deferred);
        assert!(descriptor.viewport_ui_cb.is_some(), "a deferred callback retains the host without synchronous root-frame painting");
        assert_eq!(descriptor.parent, ViewportId::ROOT);
        assert_eq!(descriptor.builder.visible, Some(false));
        assert_eq!(descriptor.builder.active, Some(false));
        assert_eq!(descriptor.builder.window_level, Some(egui::WindowLevel::Normal));
        assert_eq!(descriptor.builder.transparent, Some(true));
        assert_eq!(descriptor.builder.decorations, Some(false));
        assert_eq!(descriptor.builder.taskbar, Some(false));
        assert!(!descriptor.commands.iter().any(|command| matches!(command, ViewportCommand::Focus | ViewportCommand::Visible(true))));
        assert_eq!(output.viewport_output.len(), 2, "only ROOT and one layer context host may be retained");
        assert!(!is_open(&ctx, "layer-context"));
        assert_eq!(active_viewport(&ctx), None);
        assert!(!owns_viewport(&ctx, id));
        assert!(ctx.data(|data| data.get_temp::<State>(state_id())).is_none());
        let roles = ctx.data(|data| data.get_temp::<HashMap<ViewportId, u8>>(Id::new("photocraft-native-window-roles"))).unwrap_or_default();
        assert!(!roles.contains_key(&id), "a hidden cached host cannot participate in popup priority");
        output.textures_delta.clear();
    }
}

#[test]
fn layer_context_warm_never_declares_a_deferred_host_while_any_popup_is_active() {
    for key in ["layer-context", "workspace", "layer-adjustment-menu"] {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut output = ctx.run_ui(root_input(), |ui| {
            open(ui.ctx(), key, egui::pos2(100.0, 100.0), 0);
            warm_layer_context(ui.ctx());
        });
        assert!(is_open(&ctx, key));
        assert!(!output.viewport_output.contains_key(&viewport("layer-context")), "{key}: active menus must not get a second deferred declaration");
        output.textures_delta.clear();
    }
}

#[test]
fn deferred_warm_input_cannot_consume_root_automation_clicks() {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let at = egui::pos2(1500.0, 222.0);
    let pulse = vec![
        egui::Event::PointerMoved(at),
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() },
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() },
    ];
    app.synthetic.extend(pulse.clone());
    let mut child = egui::RawInput { viewport_id: viewport("layer-context"), ..Default::default() };
    child.viewports.entry(child.viewport_id).or_default();
    eframe::App::raw_input_hook(&mut app, &ctx, &mut child);
    assert!(child.events.is_empty(), "the hidden deferred callback has no ROOT row that can handle these events");
    assert_eq!(app.synthetic, pulse, "a child repaint must leave the ROOT input queue untouched");
    assert!(!app.automation_input, "an unrelated child pass cannot become an automation input frame");
    let mut root = root_input();
    eframe::App::raw_input_hook(&mut app, &ctx, &mut root);
    assert_eq!(root.events, pulse, "the requested ROOT pulse must arrive once, with press and release intact");
    assert!(app.synthetic.is_empty());
    assert!(app.automation_input);
    // The root UI normally resets this flag after consuming a frame.
    app.automation_input = false;
    for id in [viewport("layer-context"), ViewportId::ROOT] {
        let mut raw = egui::RawInput { viewport_id: id, ..Default::default() };
        raw.viewports.entry(id).or_default();
        eframe::App::raw_input_hook(&mut app, &ctx, &mut raw);
        assert!(raw.events.is_empty(), "neither a child nor ROOT may replay an already-consumed pulse");
        assert!(!app.automation_input);
    }
}

#[test]
fn first_frame_action_guard_only_blocks_the_fresh_native_layer_host() {
    let ctx = egui::Context::default();
    assert!(ctx.embed_viewports());
    open(&ctx, "layer-context", egui::pos2(100.0, 100.0), 0);
    let mut output = ctx.run_ui(root_input(), |ui| assert!(accepts_actions(ui.ctx()), "embedded menus remain interactive"));
    output.textures_delta.clear();
    ctx.set_embed_viewports(false);
    for (id, allowed) in [(ViewportId::ROOT, true), (viewport("workspace"), true), (viewport("layer-context"), false)] {
        let mut input = egui::RawInput { viewport_id: id, ..root_input() };
        input.viewports.entry(id).or_default();
        let mut output = ctx.run_ui(input, |ui| assert_eq!(accepts_actions(ui.ctx()), allowed));
        output.textures_delta.clear();
    }
    ctx.data_mut(|data| {
        let mut state = data.get_temp::<State>(state_id()).unwrap();
        state.opened = true;
        data.insert_temp(state_id(), state);
    });
    let id = viewport("layer-context");
    let mut input = egui::RawInput { viewport_id: id, ..root_input() };
    input.viewports.entry(id).or_default();
    let mut output = ctx.run_ui(input, |ui| assert!(accepts_actions(ui.ctx()), "the same host accepts actions after its first paint"));
    output.textures_delta.clear();
    open(&ctx, "workspace", egui::pos2(100.0, 100.0), 1);
    let mut input = egui::RawInput { viewport_id: id, ..root_input() };
    input.viewports.entry(id).or_default();
    let mut output = ctx.run_ui(input, |ui| assert!(accepts_actions(ui.ctx()), "another shared popup does not inherit the layer first-frame guard"));
    output.textures_delta.clear();
}

#[test]
fn first_and_reopened_layer_context_reject_stale_releases_then_accept_new_clicks() {
    // The integration renderer is thread-local and has no public unset API. A dedicated
    // thread gives this test a real nested viewport pass without leaking it to other tests.
    std::thread::spawn(|| {
        use std::{
            cell::{Cell, RefCell},
            collections::VecDeque,
            rc::Rc,
        };

        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let events = Rc::new(RefCell::new(VecDeque::<Vec<egui::Event>>::new()));
        let calls = Rc::new(Cell::new(0usize));
        let renderer_events = events.clone();
        let renderer_calls = calls.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
            renderer_calls.set(renderer_calls.get() + 1);
            let mut input = egui::RawInput {
                viewport_id: viewport.ids.this,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(320.0, 240.0))),
                events: renderer_events.borrow_mut().pop_front().unwrap_or_default(),
                ..Default::default()
            };
            let info = input.viewports.entry(viewport.ids.this).or_default();
            info.parent = Some(viewport.ids.parent);
            info.focused = Some(true);
            info.inner_rect = Some(Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(320.0, 240.0)));
            let mut output = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
            output.textures_delta.clear();
        });
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let row = Cell::new(Rect::NOTHING);
        let actions = Cell::new(0usize);
        let enabled = Cell::new(false);
        let accepted = Cell::new(false);
        let frame = |app: &mut PhotocraftApp, next_events: Vec<egui::Event>| {
            events.borrow_mut().push_back(next_events);
            let mut output = ctx.run_ui(root_input(), |ui| {
                warm_layer_context(ui.ctx());
                show(app, ui.ctx(), "layer-context", "Layers", egui::vec2(320.0, 240.0), |_, ui| {
                    let response = ui.add_sized(egui::vec2(280.0, 27.0), egui::Button::new("Enabled first menu action"));
                    row.set(response.rect);
                    enabled.set(response.enabled());
                    accepted.set(accepts_actions(ui.ctx()));
                    if accepted.get() && response.clicked() {
                        actions.set(actions.get() + 1);
                    }
                    false
                });
            });
            assert!(
                output.viewport_output.get(&viewport("layer-context")).unwrap().viewport_ui_cb.is_none(),
                "active drawing must replace the deferred callback"
            );
            output.textures_delta.clear();
            app.frame += 1;
        };
        let pointer = |at, pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        let mut output = ctx.run_ui(root_input(), |ui| warm_layer_context(ui.ctx()));
        output.textures_delta.clear();
        assert_eq!(calls.get(), 0, "predeclaring a deferred host cannot invoke the immediate renderer");

        open(&ctx, "layer-context", egui::pos2(100.0, 100.0), app.frame);
        let at = egui::pos2(30.0, 20.0);
        frame(&mut app, vec![egui::Event::PointerMoved(at), pointer(at, true), pointer(at, false)]);
        assert!(enabled.get(), "the first visible frame must register enabled hit geometry for the next press");
        assert!(!accepted.get(), "the first visible layer-menu frame must be action-inert");
        assert_eq!(actions.get(), 0);
        let at = row.get().center();
        frame(&mut app, vec![egui::Event::PointerMoved(at), pointer(at, true)]);
        assert!(enabled.get(), "interaction must resume on the following frame");
        assert!(accepted.get());
        frame(&mut app, vec![pointer(at, false)]);
        assert_eq!(actions.get(), 1, "a genuine click after opening must invoke the action");

        // Hide with a pressed widget; its release occurs outside the cached HWND and was
        // never delivered to the child. The deferred warm pass must not repaint it.
        frame(&mut app, vec![pointer(at, true)]);
        close(&ctx);
        let before_warm = calls.get();
        let mut output = ctx.run_ui(root_input(), |ui| warm_layer_context(ui.ctx()));
        output.textures_delta.clear();
        assert_eq!(calls.get(), before_warm);
        open(&ctx, "layer-context", egui::pos2(160.0, 120.0), app.frame);
        frame(&mut app, vec![pointer(at, false)]);
        assert!(enabled.get());
        assert!(!accepted.get());
        assert_eq!(actions.get(), 1, "old captured presses cannot execute when their reused host receives a release");
        let at = row.get().center();
        frame(&mut app, vec![egui::Event::PointerMoved(at), pointer(at, true)]);
        frame(&mut app, vec![pointer(at, false)]);
        assert_eq!(actions.get(), 2, "reopening must not suppress subsequent genuine clicks");
    })
    .join()
    .expect("isolated native popup harness");
}
