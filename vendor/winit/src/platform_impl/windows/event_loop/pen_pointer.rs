//! Map Windows pen contact actions without turning a barrel secondary action into a tip stroke.
//!
//! WM_POINTERUPDATE documents that hovering pointers have no logical button flags. A physical
//! PEN_FLAG_BARREL is not a driver-mapped secondary action: hover-click drivers send their own
//! mouse button messages. Hover must only move the cursor and must not release or duplicate
//! those independently handled buttons.
use std::collections::HashMap;

use windows_sys::Win32::UI::Input::Pointer::{
    POINTER_FLAG_CANCELED, POINTER_FLAG_CAPTURECHANGED, POINTER_FLAG_DOWN, POINTER_FLAG_INCONTACT, POINTER_FLAG_SECONDBUTTON, POINTER_FLAG_UP,
};

use crate::dpi::PhysicalPosition;
use crate::event::{ElementState, MouseButton, Touch, TouchPhase, WindowEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Primary,
    Secondary,
    // Releasing the barrel while the tip remains down must not start a new brush stroke.
    Suppressed,
}

#[derive(Clone, Copy)]
struct Contact {
    mode: Mode,
    touch: Touch,
    screen: PhysicalPosition<f64>,
}

#[derive(Default)]
pub(crate) struct PenPointers {
    contacts: HashMap<u32, Contact>,
}

fn cursor(touch: Touch) -> WindowEvent {
    WindowEvent::CursorMoved { device_id: touch.device_id, position: touch.location }
}

fn button(touch: Touch, button: MouseButton, state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput { device_id: touch.device_id, button, state }
}

fn phase(mut touch: Touch, phase: TouchPhase) -> WindowEvent {
    touch.phase = phase;
    WindowEvent::Touch(touch)
}

fn finish(contact: Contact, cancelled: bool) -> Vec<WindowEvent> {
    let touch = contact.touch;
    match contact.mode {
        Mode::Primary if cancelled => vec![
            cursor(touch),
            // egui-winit's Touch::Cancelled drops the pointer but does not release the
            // emulated left button. Release first while its pointer position still exists.
            button(touch, MouseButton::Left, ElementState::Released),
            phase(touch, TouchPhase::Cancelled),
        ],
        Mode::Primary => vec![phase(touch, TouchPhase::Ended)],
        Mode::Secondary => {
            let mut events = vec![cursor(touch), button(touch, MouseButton::Right, ElementState::Released)];
            if cancelled {
                events.push(WindowEvent::CursorLeft { device_id: touch.device_id });
            }
            events
        }
        Mode::Suppressed => Vec::new(),
    }
}

impl PenPointers {
    pub(crate) fn route(&mut self, id: u32, flags: u32, touch: Touch, screen: PhysicalPosition<f64>) -> Vec<WindowEvent> {
        let contact = flags & POINTER_FLAG_INCONTACT != 0;
        let secondary = flags & POINTER_FLAG_SECONDBUTTON != 0;
        let cancelled = flags & (POINTER_FLAG_CANCELED | POINTER_FLAG_CAPTURECHANGED) != 0;
        let ended = flags & POINTER_FLAG_UP != 0;
        if cancelled || ended || !contact {
            return self
                .contacts
                .remove(&id)
                .map_or_else(|| if cancelled || ended { Vec::new() } else { vec![cursor(touch)] }, |old| finish(Contact { touch, screen, ..old }, cancelled));
        }

        let mut events = Vec::new();
        // A fresh DOWN starts a new physical contact even if a driver's old UP was lost.
        if flags & POINTER_FLAG_DOWN != 0 {
            if let Some(old) = self.contacts.remove(&id) {
                events.extend(finish(old, true));
            }
        }
        let mode = match self.contacts.get(&id).map(|c| c.mode) {
            None if secondary => {
                events.extend([cursor(touch), button(touch, MouseButton::Right, ElementState::Pressed)]);
                Mode::Secondary
            }
            None if flags & POINTER_FLAG_DOWN != 0 => {
                events.push(phase(touch, TouchPhase::Started));
                Mode::Primary
            }
            // Do not invent a stroke for a contact that started in another window.
            None => Mode::Suppressed,
            Some(Mode::Primary) if secondary => {
                events.extend([phase(touch, TouchPhase::Ended), cursor(touch), button(touch, MouseButton::Right, ElementState::Pressed)]);
                Mode::Secondary
            }
            Some(Mode::Primary) => {
                events.push(phase(touch, TouchPhase::Moved));
                Mode::Primary
            }
            Some(Mode::Secondary) if !secondary => {
                events.extend([cursor(touch), button(touch, MouseButton::Right, ElementState::Released)]);
                Mode::Suppressed
            }
            Some(Mode::Secondary) => {
                events.push(cursor(touch));
                Mode::Secondary
            }
            Some(Mode::Suppressed) => Mode::Suppressed,
        };
        self.contacts.insert(id, Contact { mode, touch, screen });
        events
    }

    /// Pointer capture loss is distinct from mouse leave during a moving native window.
    pub(crate) fn cancel(&mut self, id: u32) -> Option<(PhysicalPosition<f64>, Vec<WindowEvent>)> {
        let contact = self.contacts.remove(&id)?;
        Some((contact.screen, finish(contact, true)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::Pointer::{POINTER_FLAG_FIRSTBUTTON, POINTER_FLAG_INRANGE, POINTER_FLAG_UPDATE};

    fn sample() -> Touch {
        Touch {
            device_id: super::super::DEVICE_ID,
            phase: TouchPhase::Moved,
            location: PhysicalPosition::new(24.5, 40.25),
            force: Some(crate::event::Force::Normalized(0.6)),
            id: 7,
        }
    }

    fn route(p: &mut PenPointers, flags: u32) -> Vec<WindowEvent> {
        p.route(7, flags, sample(), PhysicalPosition::new(800.5, 620.25))
    }

    fn is_right(event: &WindowEvent, state: ElementState) -> bool {
        matches!(event, WindowEvent::MouseInput { button: MouseButton::Right, state: s, .. } if *s == state)
    }

    #[test]
    fn primary_tip_keeps_touch_pressure_and_phases() {
        let mut p = PenPointers::default();
        for (flags, want) in [
            (POINTER_FLAG_DOWN | POINTER_FLAG_INCONTACT | POINTER_FLAG_FIRSTBUTTON, TouchPhase::Started),
            (POINTER_FLAG_UPDATE | POINTER_FLAG_INCONTACT | POINTER_FLAG_FIRSTBUTTON, TouchPhase::Moved),
            (POINTER_FLAG_UP, TouchPhase::Ended),
        ] {
            let events = route(&mut p, flags);
            assert_eq!(events, vec![phase(sample(), want)]);
        }
        assert!(p.contacts.is_empty());
    }

    #[test]
    fn barrel_contact_is_right_mouse_without_any_primary_touch() {
        let mut p = PenPointers::default();
        let down = route(&mut p, POINTER_FLAG_DOWN | POINTER_FLAG_INCONTACT | POINTER_FLAG_SECONDBUTTON);
        assert!(matches!(down[0], WindowEvent::CursorMoved { .. }));
        assert!(is_right(&down[1], ElementState::Pressed));
        let moved = route(&mut p, POINTER_FLAG_UPDATE | POINTER_FLAG_INCONTACT | POINTER_FLAG_SECONDBUTTON);
        assert!(matches!(moved.as_slice(), [WindowEvent::CursorMoved { .. }]));
        let up = route(&mut p, POINTER_FLAG_UP);
        assert!(is_right(&up[1], ElementState::Released));
        assert!(!down.iter().chain(&moved).chain(&up).any(|e| matches!(e, WindowEvent::Touch(_))));
        assert!(route(&mut p, POINTER_FLAG_UP).is_empty());
    }

    #[test]
    fn switching_barrel_mid_contact_ends_tip_and_never_restarts_until_lift() {
        let mut p = PenPointers::default();
        route(&mut p, POINTER_FLAG_DOWN | POINTER_FLAG_INCONTACT);
        let switched = route(&mut p, POINTER_FLAG_UPDATE | POINTER_FLAG_INCONTACT | POINTER_FLAG_SECONDBUTTON);
        assert!(matches!(switched[0], WindowEvent::Touch(Touch { phase: TouchPhase::Ended, .. })));
        assert!(is_right(&switched[2], ElementState::Pressed));
        let released = route(&mut p, POINTER_FLAG_UPDATE | POINTER_FLAG_INCONTACT);
        assert!(is_right(&released[1], ElementState::Released));
        assert!(route(&mut p, POINTER_FLAG_UPDATE | POINTER_FLAG_INCONTACT).is_empty());
        assert!(route(&mut p, POINTER_FLAG_UPDATE | POINTER_FLAG_INCONTACT | POINTER_FLAG_SECONDBUTTON).is_empty());
        assert!(route(&mut p, POINTER_FLAG_UP).is_empty());
        assert!(matches!(route(&mut p, POINTER_FLAG_DOWN | POINTER_FLAG_INCONTACT)[0], WindowEvent::Touch(Touch { phase: TouchPhase::Started, .. })));
    }

    #[test]
    fn hover_never_synthesizes_contact_or_barrel_click() {
        let mut p = PenPointers::default();
        for flags in [POINTER_FLAG_UPDATE | POINTER_FLAG_INRANGE, POINTER_FLAG_UPDATE | POINTER_FLAG_INRANGE | POINTER_FLAG_SECONDBUTTON] {
            assert!(matches!(route(&mut p, flags).as_slice(), [WindowEvent::CursorMoved { .. }]));
            assert!(p.contacts.is_empty());
        }
    }

    #[test]
    fn hover_cancel_does_not_release_a_driver_owned_mouse_button() {
        let mut p = PenPointers::default();
        // A driver may send WM_RBUTTONDOWN before these updates. That button is not owned
        // by the pen-contact router, so neither movement nor capture notifications release it.
        for _ in 0..3 {
            assert!(matches!(route(&mut p, POINTER_FLAG_UPDATE | POINTER_FLAG_INRANGE).as_slice(), [WindowEvent::CursorMoved { .. }]));
        }
        assert!(route(&mut p, POINTER_FLAG_CANCELED | POINTER_FLAG_UPDATE).is_empty());
        assert!(p.cancel(7).is_none());
        assert!(route(&mut p, POINTER_FLAG_UP).is_empty());
        assert!(p.contacts.is_empty());
    }

    #[test]
    fn secondary_lift_then_hover_never_repeats_click_and_next_tip_keeps_pressure() {
        let mut p = PenPointers::default();
        let down = route(&mut p, POINTER_FLAG_DOWN | POINTER_FLAG_INCONTACT | POINTER_FLAG_SECONDBUTTON);
        assert_eq!(down.iter().filter(|event| is_right(event, ElementState::Pressed)).count(), 1);
        let up = route(&mut p, POINTER_FLAG_UP | POINTER_FLAG_INRANGE);
        assert_eq!(up.iter().filter(|event| is_right(event, ElementState::Released)).count(), 1);
        for _ in 0..3 {
            assert!(matches!(route(&mut p, POINTER_FLAG_UPDATE | POINTER_FLAG_INRANGE).as_slice(), [WindowEvent::CursorMoved { .. }]));
        }
        assert!(route(&mut p, POINTER_FLAG_UP | POINTER_FLAG_INRANGE).is_empty());
        assert_eq!(route(&mut p, POINTER_FLAG_DOWN | POINTER_FLAG_INCONTACT | POINTER_FLAG_FIRSTBUTTON), vec![phase(sample(), TouchPhase::Started)]);
        assert_eq!(route(&mut p, POINTER_FLAG_UPDATE | POINTER_FLAG_INCONTACT | POINTER_FLAG_FIRSTBUTTON), vec![phase(sample(), TouchPhase::Moved)]);
        assert_eq!(route(&mut p, POINTER_FLAG_UP), vec![phase(sample(), TouchPhase::Ended)]);
        assert!(p.contacts.is_empty());
    }

    #[test]
    fn cancel_and_pointer_capture_loss_release_only_the_tracked_button() {
        for secondary in [false, true] {
            for capture in [false, true] {
                let mut p = PenPointers::default();
                let flag = if secondary { POINTER_FLAG_SECONDBUTTON } else { POINTER_FLAG_FIRSTBUTTON };
                route(&mut p, POINTER_FLAG_DOWN | POINTER_FLAG_INCONTACT | flag);
                let events = if capture {
                    let (screen, events) = p.cancel(7).unwrap();
                    assert_eq!(screen, PhysicalPosition::new(800.5, 620.25));
                    events
                } else {
                    route(&mut p, POINTER_FLAG_CANCELED | POINTER_FLAG_UP)
                };
                if secondary {
                    assert!(is_right(&events[1], ElementState::Released));
                    assert!(!events.iter().any(|e| matches!(e, WindowEvent::Touch(_))));
                } else {
                    assert!(matches!(events[1], WindowEvent::MouseInput { button: MouseButton::Left, state: ElementState::Released, .. }));
                    assert!(matches!(events[2], WindowEvent::Touch(Touch { phase: TouchPhase::Cancelled, .. })));
                }
                assert!(p.cancel(7).is_none());
                assert!(route(&mut p, POINTER_FLAG_UP).is_empty());
            }
        }
    }
}
