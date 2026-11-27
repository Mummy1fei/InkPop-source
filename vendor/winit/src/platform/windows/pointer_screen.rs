//! Physical contact positions captured before Windows client-coordinate conversion.
//!
//! The event runner can buffer a Touch event across a native window move. Reading
//! the window origin later cannot reconstruct that event's original screen point.

use std::cell::RefCell;
use std::collections::VecDeque;

use crate::event::TouchPhase;

const MAX_CONTACTS: usize = 32;

#[derive(Clone, Copy, Debug)]
struct Contact {
    id: u64,
    start: (f64, f64),
    latest: (f64, f64),
    active: bool,
}

#[derive(Default)]
struct Positions(VecDeque<Contact>);

impl Positions {
    fn record(&mut self, id: u64, phase: TouchPhase, point: (f64, f64)) {
        if !point.0.is_finite() || !point.1.is_finite() {
            return;
        }
        if phase == TouchPhase::Started {
            self.0.retain(|contact| contact.id != id);
            if self.0.len() >= MAX_CONTACTS {
                self.0.pop_front();
            }
            self.0.push_back(Contact { id, start: point, latest: point, active: true });
        } else if let Some(contact) = self.0.iter_mut().find(|contact| contact.id == id) {
            // A hover UPDATE after UP must not move the final contact sample.
            if contact.active {
                contact.latest = point;
                if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                    contact.active = false;
                }
            }
        }
    }

    fn get(&self, id: u64, at_start: bool) -> Option<(f64, f64)> {
        self.0.iter().find(|contact| contact.id == id).map(|contact| if at_start { contact.start } else { contact.latest })
    }
}

thread_local! {
    static POSITIONS: RefCell<Positions> = RefCell::new(Positions::default());
}

/// Read a Windows contact's physical desktop position on the event-loop thread.
///
/// `pointer_id` is the ID from [`crate::event::Touch`]. With `at_start`, returns
/// the contact's original down position, otherwise the latest delivered physical
/// position. Coordinates are captured before client conversion and remain valid
/// if the window moves before the UI consumes an event. They do not depend on the
/// system mouse cursor. UP keeps the final point; hover updates cannot replace it.
/// A new DOWN with the same ID starts a new contact. At most 32 contacts are kept.
///
/// Returns `None` before a matching DOWN, on another thread, or during teardown.
/// This feed supplies window geometry only; use Touch events for input lifecycle
/// and pressure, and do not infer that a retained position means contact is down.
pub fn pointer_screen_position(pointer_id: u64, at_start: bool) -> Option<(f64, f64)> {
    POSITIONS.try_with(|positions| positions.try_borrow().ok()?.get(pointer_id, at_start)).ok().flatten()
}

pub(crate) fn record_pointer_screen_position(pointer_id: u64, phase: TouchPhase, x: f64, y: f64) {
    let _ = POSITIONS.try_with(|positions| {
        if let Ok(mut positions) = positions.try_borrow_mut() {
            positions.record(pointer_id, phase, (x, y));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_contact_does_not_require_a_moving_mouse_cursor() {
        let mut positions = Positions::default();
        let frozen_mouse = (400.0, 200.0);
        positions.record(7, TouchPhase::Started, frozen_mouse);
        for point in [(425.5, 230.25), (475.0, 290.0), (600.0, 350.0)] {
            positions.record(7, TouchPhase::Moved, point);
            assert_eq!(positions.get(7, false), Some(point));
            assert_eq!(positions.get(7, true), Some(frozen_mouse));
        }
    }

    #[test]
    fn buffered_event_keeps_its_screen_point_after_window_moves() {
        let mut positions = Positions::default();
        let screen = (-1200.25, 325.5);
        positions.record(2, TouchPhase::Started, screen);
        let client_at_dispatch = (100.0, 25.0);
        let moved_origin = (-900.0, 400.0);
        assert_ne!(screen, (client_at_dispatch.0 + moved_origin.0, client_at_dispatch.1 + moved_origin.1));
        assert_eq!(positions.get(2, false), Some(screen));
    }

    #[test]
    fn end_retains_final_point_and_new_down_replaces_anchor() {
        let mut positions = Positions::default();
        positions.record(1, TouchPhase::Started, (0.0, 0.0));
        positions.record(1, TouchPhase::Ended, (100.0, 40.0));
        positions.record(1, TouchPhase::Moved, (999.0, 999.0));
        assert_eq!(positions.get(1, false), Some((100.0, 40.0)));
        positions.record(1, TouchPhase::Started, (300.0, 500.0));
        assert_eq!(positions.get(1, true), Some((300.0, 500.0)));
        assert_eq!(positions.get(1, false), Some((300.0, 500.0)));
    }

    #[test]
    fn contacts_are_isolated_bounded_and_reject_non_finite_samples() {
        let mut positions = Positions::default();
        positions.record(99, TouchPhase::Moved, (1.0, 2.0));
        assert_eq!(positions.get(99, false), None);
        for id in 0..33 {
            positions.record(id, TouchPhase::Started, (id as f64, -20.0));
        }
        assert_eq!(positions.0.len(), MAX_CONTACTS);
        assert_eq!(positions.get(0, false), None);
        positions.record(1, TouchPhase::Moved, (f64::NAN, 9.0));
        assert_eq!(positions.get(1, false), Some((1.0, -20.0)));
        assert_eq!(positions.get(32, false), Some((32.0, -20.0)));
    }
}
