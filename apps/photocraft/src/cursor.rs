//! Where the pointer is, read from the OS, so a file drop can tell the canvas (place as a layer)
//! from the tab strip (open at that tab position) and the panels (open as a document), and the tab
//! strip can show where a dragged file would go. winit 0.30's file drops carry no position, and
//! during an OS drag the window gets no pointer events, so egui's last pointer is stale.
//!
//! - **macOS**: `NSEvent.mouseLocation`, converted into the content view of our window under it.
//! - **X11**: `QueryPointer` on our window (a second, pure-Rust X connection).
//! - **Windows**: `GetCursorPos`, relative to the window's client area.
//! - **Wayland**: winit 0.30 delivers no file drops there, so there's nothing to place.
//!
//! No `unsafe`: every OS call goes through a safe binding.
//!
//! [`service`] builds the pointer-position service for the app's window, `None` where it can't
//! be read.

pub use platform::service;

/// Read the pen/contact feed captured before winit converts a Windows event to client space.
/// GetCursorPos is intentionally not a fallback for a contact: Ink and the mouse can diverge.
pub fn contact_service() -> Option<photocraft_ui_egui::ContactScreenPosFn> {
    #[cfg(target_os = "windows")]
    {
        Some(Box::new(|id, at_start| {
            winit::platform::windows::pointer_screen_position(id, at_start).map(|(x, y)| egui::pos2(x as f32, y as f32)).filter(|p| p.is_finite())
        }))
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Window movement needs a fresh desktop coordinate, rather than a buffered client coordinate.
pub fn screen_service() -> Option<photocraft_ui_egui::ScreenCursorPosFn> {
    #[cfg(target_os = "windows")]
    {
        Some(Box::new(|| winsafe::GetCursorPos().ok().map(|p| egui::pos2(p.x as f32, p.y as f32))))
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Resolve the visible viewport at a physical screen coordinate, independent of capture.
pub fn screen_viewport_service() -> Option<photocraft_ui_egui::ScreenViewportFn> {
    #[cfg(target_os = "windows")]
    {
        use std::collections::HashMap;

        type Handles = HashMap<isize, egui::ViewportId>;
        let key = egui::util::id_type_map::RawKey::new::<Handles>(egui::Id::new("pc-windows-viewport-handles"));
        Some(Box::new(move |ctx, position| {
            if !position.is_finite() {
                return None;
            }
            let point = winsafe::POINT { x: position.x.round() as i32, y: position.y.round() as i32 };
            let window = winsafe::HWND::WindowFromPhysicalPoint(point)?;
            // GA_ROOT follows the parent chain, preserving independently owned canvas windows.
            let root = window.GetAncestor(winsafe::co::GA::ROOT).unwrap_or(window);
            ctx.data(|data| data.get_temp_raw(key)?.downcast_ref::<Handles>()?.get(&(root.ptr() as isize)).copied())
        }))
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Native work areas account for monitor origins and the actual taskbar location.
pub fn monitor_work_area_service() -> Option<photocraft_ui_egui::MonitorWorkAreaFn> {
    #[cfg(target_os = "windows")]
    {
        Some(Box::new(|point| {
            if !point.is_finite() {
                return None;
            }
            let point = winsafe::POINT { x: point.x.round() as i32, y: point.y.round() as i32 };
            let monitor = winsafe::HMONITOR::MonitorFromPoint(point, winsafe::co::MONITOR::DEFAULTTONEAREST);
            let area = monitor.GetMonitorInfo().ok()?.rcWork;
            Some(egui::Rect::from_min_max(egui::pos2(area.left as f32, area.top as f32), egui::pos2(area.right as f32, area.bottom as f32)))
        }))
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// The window receiving Alt can differ from the one displaying the canvas cursor.
pub fn eyedropper_modifiers_service() -> Option<photocraft_ui_egui::CursorModifiersFn> {
    #[cfg(target_os = "windows")]
    {
        Some(Box::new(|| egui::Modifiers {
            alt: winsafe::GetAsyncKeyState(winsafe::co::VK::MENU),
            ctrl: winsafe::GetAsyncKeyState(winsafe::co::VK::CONTROL),
            ..Default::default()
        }))
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEvent, NSWindow};
    use photocraft_ui_egui::CursorPosFn;

    pub fn service(_cc: &eframe::CreationContext<'_>) -> Option<CursorPosFn> {
        Some(Box::new(|ctx: &egui::Context| in_window_points().map(|(x, y)| egui::pos2(x as f32, y as f32) / ctx.zoom_factor())))
    }

    /// The pointer in points from the top-left of the content view of our frontmost window under
    /// it (`None` when it's over none of ours, or off the main thread). Only our own windows are
    /// searched: a drop that reached us landed on one of them.
    fn in_window_points() -> Option<(f64, f64)> {
        let mtm = MainThreadMarker::new()?;
        let at = NSEvent::mouseLocation();
        let contains = |w: &NSWindow| {
            let f = w.frame();
            (f.origin.x..f.origin.x + f.size.width).contains(&at.x) && (f.origin.y..f.origin.y + f.size.height).contains(&at.y)
        };
        let window = NSApplication::sharedApplication(mtm).orderedWindows().iter().find(|w| w.isVisible() && contains(w))?;
        let view = window.contentView()?;
        let p = view.convertPoint_fromView(window.convertPointFromScreen(at), None);
        let y = if view.isFlipped() { p.y } else { view.bounds().size.height - p.y };
        Some((p.x, y))
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use eframe::wgpu::rwh::{HasWindowHandle, RawWindowHandle};
    use photocraft_ui_egui::CursorPosFn;
    use x11rb::protocol::xproto::ConnectionExt;
    use x11rb::rust_connection::RustConnection;

    pub fn service(cc: &eframe::CreationContext<'_>) -> Option<CursorPosFn> {
        let window = match cc.window_handle().ok()?.as_raw() {
            RawWindowHandle::Xlib(h) => u32::try_from(h.window).ok()?,
            RawWindowHandle::Xcb(h) => h.window.get(),
            _ => return None,
        };
        // Opened on first use and kept: the pointer is read every frame while files hover.
        let mut conn: Option<RustConnection> = None;
        Some(Box::new(move |ctx: &egui::Context| match pointer_in(&mut conn, window) {
            Ok(p) => p.map(|(x, y)| egui::pos2(f32::from(x), f32::from(y)) / ctx.pixels_per_point()),
            Err(e) => {
                log::warn!("couldn't read the pointer position: {e}");
                conn = None;
                None
            }
        }))
    }

    /// The pointer in pixels relative to `window` (`None` when it's on another screen).
    fn pointer_in(conn: &mut Option<RustConnection>, window: u32) -> Result<Option<(i16, i16)>, String> {
        let conn = match conn {
            Some(c) => c,
            None => conn.insert(RustConnection::connect(None).map_err(|e| e.to_string())?.0),
        };
        let reply = conn.query_pointer(window).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        Ok(reply.same_screen.then_some((reply.win_x, reply.win_y)))
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use photocraft_ui_egui::CursorPosFn;

    pub fn service(_cc: &eframe::CreationContext<'_>) -> Option<CursorPosFn> {
        Some(Box::new(|ctx: &egui::Context| {
            let pt = winsafe::GetCursorPos().map_err(|e| log::warn!("couldn't read the pointer position: {e}")).ok()?;
            let inner = ctx.input(|i| i.viewport().inner_rect)?;
            Some(super::client_points(egui::pos2(pt.x as f32, pt.y as f32), inner.min, ctx.pixels_per_point()))
        }))
    }
}

/// A screen position in physical pixels to points in the window whose client area starts at
/// `inner_min` (egui's `inner_rect.min`: the client origin in pixels over `ppp`). On Windows
/// `GetCursorPos` and the client origin are both physical pixels (winit makes the process
/// per-monitor DPI aware) and the window draws at one scale wherever it sits, so this holds across
/// monitors with different DPIs.
#[cfg(any(target_os = "windows", test))]
fn client_points(screen_px: egui::Pos2, inner_min: egui::Pos2, ppp: f32) -> egui::Pos2 {
    ((screen_px - inner_min * ppp) / ppp).to_pos2()
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
mod platform {
    use photocraft_ui_egui::CursorPosFn;

    pub fn service(_cc: &eframe::CreationContext<'_>) -> Option<CursorPosFn> {
        None
    }
}

#[cfg(test)]
mod tests {
    use egui::pos2;

    #[test]
    fn client_points_on_a_second_monitor_at_another_scale() {
        // A 150% monitor left of a 100% primary: the window's client area starts at (-2400, 300) px.
        let inner_min = pos2(-2400.0, 300.0) / 1.5;
        assert_eq!(super::client_points(pos2(-2100.0, 450.0), inner_min, 1.5), pos2(200.0, 100.0));
        // The same window on the 100% primary.
        assert_eq!(super::client_points(pos2(500.0, 400.0), pos2(300.0, 300.0), 1.0), pos2(200.0, 100.0));
    }
}
