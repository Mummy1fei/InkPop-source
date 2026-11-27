//! Preserve reveal/focus FIFO while applying reused-host geometry before painting.

pub(super) fn take_prepaint_geometry(
    commands: &mut Vec<egui::ViewportCommand>,
) -> Vec<egui::ViewportCommand> {
    let mut geometry = Vec::new();
    let mut deferred = Vec::new();
    for command in core::mem::take(commands) {
        if matches!(
            command,
            egui::ViewportCommand::OuterPosition(_) | egui::ViewportCommand::InnerSize(_)
        ) {
            geometry.push(command);
        } else {
            deferred.push(command);
        }
    }
    *commands = deferred;
    geometry
}

#[cfg(test)]
mod immediate_geometry_tests {
    use super::take_prepaint_geometry;

    #[test]
    fn resizing_a_reused_menu_precedes_paint_but_visibility_and_focus_wait() {
        use egui::ViewportCommand as C;
        let mut commands = vec![
            C::Visible(false),
            C::OuterPosition(egui::pos2(40.0, 25.0)),
            C::InnerSize(egui::vec2(280.0, 700.0)),
            C::Visible(true),
            C::Focus,
        ];
        let geometry = take_prepaint_geometry(&mut commands);
        assert!(matches!(
            geometry.as_slice(),
            [C::OuterPosition(_), C::InnerSize(_)]
        ));
        assert!(matches!(
            commands.as_slice(),
            [C::Visible(false), C::Visible(true), C::Focus]
        ));
        assert!(take_prepaint_geometry(&mut commands).is_empty());
        assert!(matches!(
            commands.as_slice(),
            [C::Visible(false), C::Visible(true), C::Focus]
        ));
    }
}
