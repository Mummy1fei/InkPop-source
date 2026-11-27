use super::*;
use std::{cell::Cell, rc::Rc};

fn canvas_hover_after_panel(
    h: &mut CanvasHarness,
    canvas: ViewportId,
    panel: ViewportId,
    focused: Option<ViewportId>,
    pointer: Pos2,
) -> egui::CustomCursorImage {
    h.time += 1.0 / 120.0;
    let mut input = egui::RawInput {
        viewport_id: canvas,
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, h.rect.max.to_vec2() + vec2(32.0, 32.0))),
        time: Some(h.time),
        focused: focused == Some(canvas),
        events: vec![Event::PointerMoved(pointer)],
        ..Default::default()
    };
    input.viewports.entry(canvas).or_default().focused = Some(focused == Some(canvas));
    input.viewports.entry(panel).or_default().focused = Some(focused == Some(panel));
    let mut output = h.ctx.run_ui(input, |ui| h.view = canvas_view(&mut h.app, ui, 0, h.rect, h.view.clone(), true));
    output.textures_delta.clear();
    output.platform_output.cursor_image.expect("the active canvas publishes a cursor while hovered")
}

fn panel_alt(h: &mut CanvasHarness, panel: ViewportId, modifiers: Modifiers, layer_command: Option<(&str, serde_json::Value)>) {
    h.time += 1.0 / 120.0;
    let mut input = egui::RawInput { viewport_id: panel, time: Some(h.time), events: vec![Event::ModifiersChanged(modifiers)], ..Default::default() };
    input.viewports.entry(panel).or_default().focused = Some(true);
    let mut output = h.ctx.run_ui(input, |_| {
        if let Some((command, params)) = &layer_command {
            h.app.run(command, params.clone()).unwrap();
        }
        crate::hold_keys::publish_panel_alt(&h.app, &h.ctx);
    });
    output.textures_delta.clear();
}

#[test]
fn alt_eyedropper_hover_uses_focused_floating_layers_after_create_and_switch() {
    let panel = ViewportId::from_hash_of("floating-layers-alt-focus");
    let detached = ViewportId::from_hash_of("detached-canvas-alt-focus");
    for canvas in [ViewportId::ROOT, detached] {
        let mut h = CanvasHarness::new(320, 200, true);
        h.ctx.set_embed_viewports(false);
        h.app.ui.tool = Tool::Brush;
        let base = h.app.session.active().unwrap().active_layer.unwrap().0;
        let point = h.doc_point(120.0, 80.0);
        h.hover(canvas, Modifiers::NONE, point);
        let tip = h.hover(canvas, Modifiers::NONE, point).platform_output.cursor_image.unwrap();
        let eyedropper = h.hover(canvas, Modifiers::ALT, point).platform_output.cursor_image.unwrap();
        assert_ne!(tip, eyedropper);
        h.hover(canvas, Modifiers::NONE, point);

        panel_alt(&mut h, panel, Modifiers::ALT, Some(("layer.new.layer", json!({"name": "New paint layer"}))));
        assert_eq!(canvas_hover_after_panel(&mut h, canvas, panel, Some(panel), point), eyedropper);
        panel_alt(&mut h, panel, Modifiers::NONE, None);
        assert_eq!(canvas_hover_after_panel(&mut h, canvas, panel, Some(panel), point), tip);

        panel_alt(&mut h, panel, Modifiers::ALT, Some(("layer.select", json!({"layer": base}))));
        assert_eq!(canvas_hover_after_panel(&mut h, canvas, panel, Some(panel), point), eyedropper);
        // The canvas may see deactivation before the last focused panel refreshes.
        assert_eq!(canvas_hover_after_panel(&mut h, canvas, panel, None, point), tip);
        assert_eq!(canvas_hover_after_panel(&mut h, canvas, panel, Some(canvas), point), tip);
        let app_active = egui::Id::new("photocraft-native-app-active");
        h.ctx.data_mut(|data| data.insert_temp(app_active, false));
        assert_eq!(canvas_hover_after_panel(&mut h, canvas, panel, Some(panel), point), tip);
        h.ctx.data_mut(|data| data.remove::<bool>(app_active));

        panel_alt(&mut h, panel, Modifiers { alt: true, ctrl: true, ..Modifiers::NONE }, None);
        assert_eq!(canvas_hover_after_panel(&mut h, canvas, panel, Some(panel), point), tip);
        assert_eq!(h.app.ui.tool, Tool::Brush);
        assert!(h.app.drag.is_none() && !h.app.alt_sampling);
    }
}

#[test]
fn alt_eyedropper_click_uses_press_modifiers_after_layer_change() {
    for same_frame in [false, true] {
        let mut h = CanvasHarness::new(320, 200, true);
        h.app.run("edit.fill", json!({"color": "#22aa55"})).unwrap();
        h.app.run("layer.new.layer", json!({"name": "New paint layer"})).unwrap();
        h.app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
        h.app.ui.tool = Tool::Brush;
        let p = h.doc_point(120.0, 80.0);
        let [r, g, b] = crate::sampling_ui::color(&mut h.app, 120.0, 80.0).unwrap();
        assert_ne!(h.app.session.tools.foreground, [r, g, b, 1.0]);
        let revision = h.app.session.active().unwrap().revision;
        h.hover(ViewportId::ROOT, Modifiers::ALT, p);
        let press = button(p, true, Modifiers::ALT);
        let release = button(p, false, Modifiers::NONE);
        if same_frame {
            h.frame(ViewportId::ROOT, Modifiers::ALT, vec![press, Event::ModifiersChanged(Modifiers::NONE), release]);
        } else {
            h.frame(ViewportId::ROOT, Modifiers::ALT, vec![press]);
            h.frame(ViewportId::ROOT, Modifiers::NONE, vec![release]);
        }
        assert_eq!(h.app.session.tools.foreground, [r, g, b, 1.0]);
        assert_eq!(h.app.session.active().unwrap().revision, revision, "a temporary eyedropper click must leave the layer untouched");
        assert_eq!(h.app.ui.tool, Tool::Brush);
        assert!(h.app.drag.is_none() && !h.app.alt_sampling);
    }
}

#[test]
fn windows_eyedropper_cursor_tracks_live_modifiers_without_pointer_movement() {
    let detached = ViewportId::from_hash_of("detached-live-alt-cursor");
    for viewport in [ViewportId::ROOT, detached] {
        let mut h = CanvasHarness::new(320, 200, true);
        h.app.ui.tool = Tool::Eyedropper;
        let point = h.doc_point(120.0, 80.0);
        h.hover(viewport, Modifiers::NONE, point);
        let pipette = h.frame(viewport, Modifiers::NONE, vec![]).platform_output.cursor_image.unwrap();

        let keys = Rc::new(Cell::new(Modifiers::NONE));
        let native_keys = Rc::clone(&keys);
        h.app.services.eyedropper_cursor_modifiers = Some(Box::new(move || native_keys.get()));
        h.app.ui.tool = Tool::Brush;
        let tip = h.frame(viewport, Modifiers::NONE, vec![]).platform_output.cursor_image.unwrap();
        assert_ne!(tip, pipette);

        keys.set(Modifiers::ALT);
        // Alt went to a different native window. The canvas saw no pointer move
        // and no local Alt event, but must publish the same bitmap as Eyedropper.
        let pressed = h.frame(viewport, Modifiers::NONE, vec![]).platform_output.cursor_image.unwrap();
        assert_eq!(pressed, pipette);
        keys.set(Modifiers::NONE);
        let released = h.frame(viewport, Modifiers::ALT, vec![]).platform_output.cursor_image.unwrap();
        assert_eq!(released, tip);

        if viewport == ViewportId::ROOT
            && let Some(path) = std::env::var_os("PHOTOCRAFT_V103_CURSOR_EVIDENCE")
        {
            let data = [&tip, &pressed, &released].map(|image| json!({"size":image.size,"hotspot":image.hotspot,"rgba":image.rgba.as_ref()}));
            std::fs::write(path, serde_json::to_vec(&data).unwrap()).unwrap();
        }

        keys.set(Modifiers { ctrl: true, ..Modifiers::ALT });
        assert_eq!(h.frame(viewport, Modifiers::ALT, vec![]).platform_output.cursor_image, Some(tip.clone()));
        keys.set(Modifiers::ALT);
        h.ctx.data_mut(|data| data.insert_temp(egui::Id::new("photocraft-native-app-active"), false));
        assert_eq!(h.frame(viewport, Modifiers::NONE, vec![]).platform_output.cursor_image, Some(tip));
        assert!(!h.app.alt_sampling && h.app.drag.is_none());
    }
}

#[test]
fn alt_eyedropper_targets_the_first_press_after_a_document_switch() {
    let detached = ViewportId::from_hash_of("detached-new-document-live-alt");
    for viewport in [ViewportId::ROOT, detached] {
        let mut h = CanvasHarness::new(320, 200, true);
        h.app.run("edit.fill", json!({"color": "#dd2222"})).unwrap();
        h.app.run("file.new", json!({"width": 320, "height": 200, "background": "transparent"})).unwrap();
        h.app.run("edit.fill", json!({"color": "#2255dd"})).unwrap();
        h.app.run("tools.setColors", json!({"foreground": "#00ff00"})).unwrap();
        let [r, g, b] = crate::sampling_ui::color(&mut h.app, 120.0, 80.0).unwrap();
        let p = h.doc_point(120.0, 80.0);
        h.app.ui.tool = Tool::Eyedropper;
        h.frame_document(viewport, 1, true, Modifiers::NONE, vec![Event::PointerMoved(p)]);
        let pipette = h.frame_document(viewport, 1, true, Modifiers::NONE, vec![]).platform_output.cursor_image.unwrap();

        let keys = Rc::new(Cell::new(Modifiers::ALT));
        let native_keys = Rc::clone(&keys);
        h.app.services.eyedropper_cursor_modifiers = Some(Box::new(move || native_keys.get()));
        h.app.ui.tool = Tool::Brush;
        h.app.session.set_active(0);
        let passive = h.frame_document(viewport, 1, false, Modifiers::NONE, vec![]);
        assert_eq!(passive.platform_output.cursor_image, Some(pipette.clone()));
        assert_eq!(h.app.session.active_index(), Some(0), "hover must leave the current document active");

        // Match the native window's first canvas pass after activation. The press
        // arrived while viewport-local Alt was stale; a same-frame focus handoff
        // delivered the loss of focus notification before this owned press.
        h.app.session.set_active(1);
        let revision = h.app.session.active().unwrap().revision;
        h.frame_document(viewport, 1, true, Modifiers::NONE, vec![Event::WindowFocused(false), Event::WindowFocused(true), button(p, true, Modifiers::NONE)]);
        keys.set(Modifiers::NONE);
        let released = h.frame_document(viewport, 1, true, Modifiers::NONE, vec![button(p, false, Modifiers::NONE)]);
        assert_eq!(h.app.session.tools.foreground, [r, g, b, 1.0]);
        assert_eq!(h.app.session.active().unwrap().revision, revision, "the first Alt click must not paint");
        assert_ne!(released.platform_output.cursor_image, Some(pipette));
        assert_eq!(h.app.ui.tool, Tool::Brush);
        assert!(h.app.drag.is_none() && !h.app.alt_sampling);
    }
}
