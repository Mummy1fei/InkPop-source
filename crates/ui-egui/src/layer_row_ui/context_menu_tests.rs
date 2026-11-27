use super::*;

fn pointer(at: Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos: at, button, pressed, modifiers: Default::default() }
}

#[test]
fn context_menu_opens_on_secondary_press_without_waiting_for_release() {
    let ctx = egui::Context::default();
    let frame = |events| {
        let mut opened = None;
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            let response = ui.allocate_response(vec2(280.0, 32.0), Sense::click_and_drag());
            opened = context_menu_press(&response);
        });
        out.textures_delta.clear();
        opened
    };
    assert!(frame(vec![]).is_none());
    let at = pos2(80.0, 20.0);
    assert_eq!(frame(vec![egui::Event::PointerMoved(at), pointer(at, egui::PointerButton::Secondary, true)]), Some(at));
    assert!(frame(vec![egui::Event::PointerMoved(at + vec2(40.0, 0.0))]).is_none(), "held barrel button must not reopen");
    assert!(frame(vec![pointer(at + vec2(40.0, 0.0), egui::PointerButton::Secondary, false)]).is_none(), "release must not reopen");
}

#[test]
fn context_menu_accepts_a_buffered_pen_pulse_and_ignores_primary_drag() {
    let ctx = egui::Context::default();
    let at = pos2(80.0, 20.0);
    let frame = |events| {
        let mut opened = None;
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            let response = ui.allocate_response(vec2(280.0, 32.0), Sense::click_and_drag());
            opened = context_menu_press(&response);
        });
        out.textures_delta.clear();
        opened
    };
    frame(vec![]);
    assert_eq!(
        frame(vec![egui::Event::PointerMoved(at), pointer(at, egui::PointerButton::Secondary, true), pointer(at, egui::PointerButton::Secondary, false)]),
        Some(at)
    );
    frame(vec![pointer(at, egui::PointerButton::Primary, true)]);
    assert!(frame(vec![egui::Event::PointerMoved(at + vec2(20.0, 0.0)), pointer(at, egui::PointerButton::Secondary, true)]).is_none());
}

#[test]
fn secondary_press_only_opens_the_enabled_row_under_the_pointer() {
    let ctx = egui::Context::default();
    let frame = |events| {
        let mut opened = Vec::new();
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            for index in 0..3 {
                let response = ui.add_enabled(index != 2, egui::Button::new(format!("Row {index}")).min_size(vec2(280.0, 32.0)));
                if let Some(at) = context_menu_press(&response) {
                    opened.push((index, at));
                }
            }
        });
        out.textures_delta.clear();
        opened
    };
    frame(vec![]);
    let at = pos2(80.0, 55.0);
    assert_eq!(frame(vec![egui::Event::PointerMoved(at), pointer(at, egui::PointerButton::Secondary, true)]), vec![(1, at)]);
    frame(vec![pointer(at, egui::PointerButton::Secondary, false)]);
    let disabled = pos2(80.0, 90.0);
    assert!(frame(vec![egui::Event::PointerMoved(disabled), pointer(disabled, egui::PointerButton::Secondary, true)]).is_empty());
}
