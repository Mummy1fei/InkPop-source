//! Keep explicitly classified sibling windows in application-local priority order.
//! Reorder by lowering windows only, never by activating or entering a topmost band.

use std::collections::HashMap;

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetCapture;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetGUIThreadInfo, GetTopWindow, GetWindow, GetWindowLongPtrW,
    GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, SetWindowPos, GUITHREADINFO, GUI_INMOVESIZE, GWL_EXSTYLE,
    GW_HWNDNEXT, GW_OWNER, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE,
    WS_EX_TOPMOST,
};

use crate::window::Window;

/// Whether a window of this process currently owns foreground activation.
/// This query never changes activation and treats focus transfers between sibling
/// app windows (including native submenus) as staying inside the application.
pub fn is_current_process_foreground() -> bool {
    // SAFETY: queries only; the process-id output points to initialized local storage.
    unsafe {
        let foreground = GetForegroundWindow();
        let mut process = 0;
        foreground != 0
            && GetWindowThreadProcessId(foreground, &mut process) != 0
            && process == GetCurrentProcessId()
    }
}

/// Whether `window` belongs to the native ownership subtree rooted at `owner`.
pub fn is_owned_descendant(window: &Window, owner: &Window) -> bool {
    let target = owner.window.hwnd();
    let mut current = window.window.hwnd();
    // SAFETY: starting handles are borrowed live windows. Ownership queries do not
    // modify state, and bounded traversal handles a disappearing owner safely.
    unsafe {
        for _ in 0..128 {
            current = GetWindow(current, GW_OWNER);
            if current == 0 { return false; }
            if current == target { return true; }
        }
    }
    false
}

/// Repair priority inversions among visible, normal-level windows with the same owner.
///
/// Higher numbers belong above lower numbers. Equal priorities retain their current
/// relative order. Call on the event-loop thread with live windows from that event loop.
/// Hidden/minimized windows, unclassified windows and native owner relationships are
/// untouched. Captured input and native move/size gestures defer all ordering work.
pub fn maintain_owned_window_order(windows: &[(&Window, u8)]) {
    if windows.len() < 2 {
        return;
    }
    // SAFETY: GUI-thread queries take no borrowed pointer except our initialized struct.
    // GetCapture is thread-local; this function is called on the owning event-loop thread.
    unsafe {
        let mut gui: GUITHREADINFO = std::mem::zeroed();
        gui.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if GetCapture() != 0
            || (GetGUIThreadInfo(GetCurrentThreadId(), &mut gui) != 0
                && gui.flags & GUI_INMOVESIZE != 0)
        {
            return;
        }
    }
    let mut eligible = HashMap::new();
    for (window, role) in windows {
        let hwnd = window.window.hwnd();
        // SAFETY: all handles come from live borrowed winit Window values; queries do
        // not change state. Never turn a pre-existing topmost window into our anchor.
        unsafe {
            let owner = GetWindow(hwnd, GW_OWNER);
            if owner != 0
                && IsWindowVisible(hwnd) != 0
                && IsIconic(hwnd) == 0
                && GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST as isize == 0
            {
                eligible.insert(hwnd, (owner, *role));
            }
        }
    }
    if eligible.len() < 2 {
        return;
    }
    let mut groups: HashMap<HWND, Vec<(HWND, u8)>> = HashMap::new();
    // SAFETY: GetTopWindow/GetWindow enumerate live top-level windows; handle races
    // merely cause an entry not to match eligible. Bound traversal even on a changing list.
    unsafe {
        let mut hwnd = GetTopWindow(0);
        for _ in 0..8192 {
            if hwnd == 0 {
                break;
            }
            if let Some((owner, role)) = eligible.get(&hwnd) {
                groups.entry(*owner).or_default().push((hwnd, *role));
            }
            hwnd = GetWindow(hwnd, GW_HWNDNEXT);
        }
    }
    for (_, current) in groups {
        for (lower, anchor) in lowering_plan(&current) {
            // SAFETY: handles are live same-owner normal-level siblings. Positioning
            // immediately after an already lower window only lowers the target; flags
            // forbid activation, owner reordering, showing, resizing and movement.
            // No HWND_TOP/TOPMOST/BOTTOM pseudo-handle is ever used.
            unsafe {
                if SetWindowPos(lower, anchor, 0, 0, 0, 0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOOWNERZORDER) == 0
                {
                    break;
                }
            }
        }
    }
}

/// Plan stable descending priority using only downward moves in the current ordering.
fn lowering_plan(current: &[(HWND, u8)]) -> Vec<(HWND, HWND)> {
    let mut desired = current.to_vec();
    desired.sort_by_key(|(_, role)| std::cmp::Reverse(*role));
    let mut current: Vec<_> = current.iter().map(|(hwnd, _)| *hwnd).collect();
    let mut plan = Vec::new();
    for pair in desired.windows(2) {
        let [anchor, lower] = pair else { continue };
        let (Some(a), Some(b)) = (
            current.iter().position(|hwnd| *hwnd == anchor.0),
            current.iter().position(|hwnd| *hwnd == lower.0),
        ) else { continue };
        if b < a {
            let lower = current.remove(b);
            // Removing a higher entry shifts the anchor back one: insertion at the
            // former anchor index places this entry directly after it.
            current.insert(a, lower);
            plan.push((lower, anchor.0));
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn already_ordered_siblings_generate_no_native_changes() {
        assert!(lowering_plan(&[(1, 3), (2, 2), (3, 1), (4, 1), (5, 0)]).is_empty());
    }

    #[test]
    fn priority_repair_only_lowers_and_preserves_peers() {
        let initial = vec![(1, 0), (2, 1), (3, 0), (4, 3), (5, 2)];
        let mut order: Vec<_> = initial.iter().map(|(id, _)| *id).collect();
        for (lower, anchor) in lowering_plan(&initial) {
            let a = order.iter().position(|id| *id == anchor).unwrap();
            let b = order.iter().position(|id| *id == lower).unwrap();
            assert!(b < a, "must never promote a window");
            order.remove(b);
            order.insert(a, lower);
        }
        assert_eq!(order, vec![4, 5, 2, 1, 3]);
    }
}
