//! Explicit native window commands, kept separate from observed window state.
use std::cell::RefCell;
use std::collections::HashMap;

use crate::window::WindowId;

/// Native system command changing a window's show state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowStateRequest {
    Restore,
    Maximize,
    Minimize,
}

thread_local! {
    static RESTORE_REQUESTS: RefCell<HashMap<WindowId, WindowStateRequest>> = RefCell::new(HashMap::new());
}

/// Consume an explicit SC_RESTORE/SC_MAXIMIZE/SC_MINIMIZE command for this window.
///
/// Owner visibility changes and programmatic `set_minimized(false)` do not count.
/// Call on the event-loop thread. Destruction clears the record before HWND reuse.
pub fn take_window_state_request(window: WindowId) -> Option<WindowStateRequest> {
    RESTORE_REQUESTS.try_with(|requests| requests.try_borrow_mut().ok()?.remove(&window)).ok().flatten()
}

pub(crate) fn record_window_state_request(window: WindowId, request: WindowStateRequest) {
    let _ = RESTORE_REQUESTS.try_with(|requests| {
        if let Ok(mut requests) = requests.try_borrow_mut() {
            requests.insert(window, request);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_requests_are_per_window_and_consumed_once() {
        let root = WindowId::from(100);
        let child = WindowId::from(101);
        record_window_state_request(root, WindowStateRequest::Restore);
        assert_eq!(take_window_state_request(child), None);
        assert_eq!(take_window_state_request(root), Some(WindowStateRequest::Restore));
        assert_eq!(take_window_state_request(root), None);
        record_window_state_request(child, WindowStateRequest::Maximize);
        assert_eq!(take_window_state_request(child), Some(WindowStateRequest::Maximize));
        assert_eq!(take_window_state_request(child), None);
        record_window_state_request(child, WindowStateRequest::Minimize);
        assert_eq!(take_window_state_request(child), Some(WindowStateRequest::Minimize));
        assert_eq!(take_window_state_request(root), None);
    }
}
