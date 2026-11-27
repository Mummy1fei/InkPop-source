use crate::dpi::{PhysicalPosition, PhysicalSize, Size};
use crate::icon::Icon;
use crate::keyboard::ModifiersState;
use crate::platform_impl::platform::{event_loop, util, Fullscreen, SelectedCursor};
use crate::window::{Theme, WindowAttributes};
use bitflags::bitflags;
use std::io;
use std::sync::MutexGuard;
use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::InvalidateRgn;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRectEx, EnableMenuItem, GetMenu, GetSystemMenu, GetWindowLongW, SendMessageW,
    SetWindowLongW, SetWindowPos, ShowWindow, GWL_EXSTYLE, GWL_STYLE, HWND_BOTTOM, HWND_NOTOPMOST,
    HWND_TOPMOST, MF_BYCOMMAND, MF_DISABLED, MF_ENABLED, SC_CLOSE, SWP_ASYNCWINDOWPOS,
    SWP_FRAMECHANGED, SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOREPOSITION, SWP_NOSIZE,
    SWP_NOZORDER, SWP_SHOWWINDOW,
    SW_HIDE, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE, SW_SHOW, SW_SHOWNA, SW_SHOWNOACTIVATE, WINDOWPLACEMENT,
    WINDOW_EX_STYLE, WINDOW_STYLE, WS_BORDER, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_EX_ACCEPTFILES, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_NOREDIRECTIONBITMAP,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_EX_WINDOWEDGE, WS_MAXIMIZE, WS_MAXIMIZEBOX, WS_MINIMIZE,
    WS_MINIMIZEBOX, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_SIZEBOX, WS_SYSMENU, WS_VISIBLE,
};

/// Contains information about states and the window that the callback is going to use.
pub(crate) struct WindowState {
    pub mouse: MouseProperties,

    /// Used by `WM_GETMINMAXINFO`.
    pub min_size: Option<Size>,
    pub max_size: Option<Size>,

    pub resize_increments: Option<Size>,

    pub window_icon: Option<Icon>,
    pub taskbar_icon: Option<Icon>,

    pub saved_window: Option<SavedWindow>,
    pub scale_factor: f64,

    pub modifiers_state: ModifiersState,
    pub fullscreen: Option<Fullscreen>,
    pub current_theme: Theme,
    pub preferred_theme: Option<Theme>,

    pub window_flags: WindowFlags,

    pub ime_state: ImeState,
    pub ime_allowed: bool,

    // Used by WM_NCACTIVATE, WM_SETFOCUS and WM_KILLFOCUS
    pub is_active: bool,
    pub is_focused: bool,

    // Flag whether redraw was requested.
    pub redraw_requested: bool,

    pub dragging: bool,

    pub skip_taskbar: bool,
}

#[derive(Clone)]
pub struct SavedWindow {
    pub placement: WINDOWPLACEMENT,
}

#[derive(Clone)]
pub struct MouseProperties {
    pub(crate) selected_cursor: SelectedCursor,
    pub capture_count: u32,
    cursor_flags: CursorFlags,
    pub last_position: Option<PhysicalPosition<f64>>,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct CursorFlags: u8 {
        const GRABBED   = 1 << 0;
        const HIDDEN    = 1 << 1;
        const IN_WINDOW = 1 << 2;
        const LOCKED    = 1 << 3;
    }
}
bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct WindowFlags: u32 {
        const RESIZABLE         = 1 << 0;
        const MINIMIZABLE       = 1 << 1;
        const MAXIMIZABLE       = 1 << 2;
        const CLOSABLE          = 1 << 3;
        const VISIBLE           = 1 << 4;
        const ON_TASKBAR        = 1 << 5;
        const ALWAYS_ON_TOP     = 1 << 6;
        const ALWAYS_ON_BOTTOM  = 1 << 7;
        const NO_BACK_BUFFER    = 1 << 8;
        const TRANSPARENT       = 1 << 9;
        const CHILD             = 1 << 10;
        const MAXIMIZED         = 1 << 11;
        const POPUP             = 1 << 12;

        /// Marker flag for fullscreen. Should always match `WindowState::fullscreen`, but is
        /// included here to make masking easier.
        const MARKER_EXCLUSIVE_FULLSCREEN = 1 << 13;
        const MARKER_BORDERLESS_FULLSCREEN = 1 << 14;

        /// The `WM_SIZE` event contains some parameters that can effect the state of `WindowFlags`.
        /// In most cases, it's okay to let those parameters change the state. However, when we're
        /// running the `WindowFlags::apply_diff` function, we *don't* want those parameters to
        /// effect our stored state, because the purpose of `apply_diff` is to update the actual
        /// window's state to match our stored state. This controls whether to accept those changes.
        const MARKER_RETAIN_STATE_ON_SIZE = 1 << 15;

        const MARKER_IN_SIZE_MOVE = 1 << 16;

        const MINIMIZED = 1 << 17;

        const IGNORE_CURSOR_EVENT = 1 << 18;

        /// Fully decorated window (incl. caption, border and drop shadow).
        const MARKER_DECORATIONS = 1 << 19;
        /// Drop shadow for undecorated windows.
        const MARKER_UNDECORATED_SHADOW = 1 << 20;

        const MARKER_ACTIVATE = 1 << 21;

        const CLIP_CHILDREN = 1 << 22;

        const EXCLUSIVE_FULLSCREEN_OR_MASK = WindowFlags::ALWAYS_ON_TOP.bits();
    }
}

#[derive(Eq, PartialEq)]
pub enum ImeState {
    Disabled,
    Enabled,
    Preedit,
}

impl WindowState {
    pub(crate) fn new(
        attributes: &WindowAttributes,
        scale_factor: f64,
        current_theme: Theme,
        preferred_theme: Option<Theme>,
    ) -> WindowState {
        WindowState {
            mouse: MouseProperties {
                selected_cursor: SelectedCursor::default(),
                capture_count: 0,
                cursor_flags: CursorFlags::empty(),
                last_position: None,
            },

            min_size: attributes.min_inner_size,
            max_size: attributes.max_inner_size,

            resize_increments: attributes.resize_increments,

            window_icon: attributes.window_icon.clone(),
            taskbar_icon: None,

            saved_window: None,
            scale_factor,

            modifiers_state: ModifiersState::default(),
            fullscreen: None,
            current_theme,
            preferred_theme,
            window_flags: WindowFlags::empty(),

            ime_state: ImeState::Disabled,
            ime_allowed: false,

            is_active: false,
            is_focused: false,
            redraw_requested: false,

            dragging: false,

            skip_taskbar: false,
        }
    }

    pub fn window_flags(&self) -> WindowFlags {
        self.window_flags
    }

    pub fn set_window_flags<F>(mut this: MutexGuard<'_, Self>, window: HWND, f: F)
    where
        F: FnOnce(&mut WindowFlags),
    {
        let old_flags = this.window_flags;
        f(&mut this.window_flags);
        let new_flags = this.window_flags;

        drop(this);
        old_flags.apply_diff(window, new_flags);
    }

    pub fn set_window_flags_in_place<F>(&mut self, f: F)
    where
        F: FnOnce(&mut WindowFlags),
    {
        f(&mut self.window_flags);
    }

    pub fn has_active_focus(&self) -> bool {
        self.is_active && self.is_focused
    }

    // Updates is_active and returns whether active-focus state has changed
    pub fn set_active(&mut self, is_active: bool) -> bool {
        let old = self.has_active_focus();
        self.is_active = is_active;
        old != self.has_active_focus()
    }

    // Updates is_focused and returns whether active-focus state has changed
    pub fn set_focused(&mut self, is_focused: bool) -> bool {
        let old = self.has_active_focus();
        self.is_focused = is_focused;
        old != self.has_active_focus()
    }
}

impl MouseProperties {
    pub fn cursor_flags(&self) -> CursorFlags {
        self.cursor_flags
    }

    pub fn set_cursor_flags<F>(&mut self, window: HWND, f: F) -> Result<(), io::Error>
    where
        F: FnOnce(&mut CursorFlags),
    {
        let old_flags = self.cursor_flags;
        f(&mut self.cursor_flags);
        match self.cursor_flags.refresh_os_cursor(window) {
            Ok(()) => (),
            Err(e) => {
                self.cursor_flags = old_flags;
                return Err(e);
            },
        }

        Ok(())
    }
}

impl WindowFlags {
    /// An owned transparent popup whose complete outline is painted by the client.
    /// Keep frame styles and both WM_NCCALCSIZE forms on the same policy.
    pub(crate) fn custom_transparent_owned_frame(self) -> bool {
        self.contains(WindowFlags::POPUP | WindowFlags::TRANSPARENT)
            && !self.intersects(
                WindowFlags::MARKER_DECORATIONS | WindowFlags::MARKER_UNDECORATED_SHADOW,
            )
    }

    /// ShowWindow's geometry commands can show a window as a side effect. While
    /// applying a hidden window's requested state, keep the native size and
    /// restoration placement without making its unfinished surface visible.
    pub(crate) fn preserve_hidden_position(self, flags: u32) -> u32 {
        if self.contains(WindowFlags::MARKER_RETAIN_STATE_ON_SIZE)
            && !self.contains(WindowFlags::VISIBLE)
            && flags & SWP_SHOWWINDOW != 0
        {
            (flags & !SWP_SHOWWINDOW) | SWP_HIDEWINDOW | SWP_NOACTIVATE
        } else {
            flags
        }
    }

    fn mask(mut self) -> WindowFlags {
        if self.contains(WindowFlags::MARKER_EXCLUSIVE_FULLSCREEN) {
            self |= WindowFlags::EXCLUSIVE_FULLSCREEN_OR_MASK;
        }
        self
    }

    pub fn to_window_styles(self) -> (WINDOW_STYLE, WINDOW_EX_STYLE) {
        // Required styles to properly support common window functionality like aero snap.
        let mut style = WS_CAPTION | WS_BORDER | WS_CLIPSIBLINGS | WS_SYSMENU;
        let mut style_ex = WS_EX_WINDOWEDGE | WS_EX_ACCEPTFILES;

        if self.contains(WindowFlags::RESIZABLE) {
            style |= WS_SIZEBOX;
        }
        if self.contains(WindowFlags::MAXIMIZABLE) {
            style |= WS_MAXIMIZEBOX;
        }
        if self.contains(WindowFlags::MINIMIZABLE) {
            style |= WS_MINIMIZEBOX;
        }
        if self.contains(WindowFlags::VISIBLE) {
            style |= WS_VISIBLE;
        }
        if self.contains(WindowFlags::ON_TASKBAR) {
            style_ex |= WS_EX_APPWINDOW;
        }
        if self.contains(WindowFlags::ALWAYS_ON_TOP) {
            style_ex |= WS_EX_TOPMOST;
        }
        if self.contains(WindowFlags::NO_BACK_BUFFER) {
            style_ex |= WS_EX_NOREDIRECTIONBITMAP;
        }
        if self.contains(WindowFlags::CHILD) {
            style |= WS_CHILD; // This is incompatible with WS_POPUP if that gets added eventually.

            // Remove decorations window styles for child
            if !self.contains(WindowFlags::MARKER_DECORATIONS) {
                style &= !(WS_CAPTION | WS_BORDER);
                style_ex &= !WS_EX_WINDOWEDGE;
            }
        }
        if self.contains(WindowFlags::POPUP) {
            style |= WS_POPUP;
        }
        // PhotoCraft's transparent owned document windows paint their frame in egui.
        // Expanding the client area in WM_NCCALCSIZE does not remove the native
        // caption/border styles: Windows can still paint them into transparent pixels,
        // including the initial small popup frame before the renderer is attached.
        // Remove the complete cosmetic/non-client frame. Detached documents implement edge
        // resizing in their client area, so retaining WS_SIZEBOX would make Windows draw a
        // second opaque resize frame around the transparent themed card.
        // Root, decorated, opaque and explicitly shadowed windows retain winit defaults.
        if self.custom_transparent_owned_frame() {
            style &= !(WS_CAPTION | WS_BORDER | WS_SIZEBOX);
            style_ex &= !WS_EX_WINDOWEDGE;
        }
        if self.contains(WindowFlags::MINIMIZED) {
            style |= WS_MINIMIZE;
        }
        if self.contains(WindowFlags::MAXIMIZED) {
            style |= WS_MAXIMIZE;
        }
        if self.contains(WindowFlags::IGNORE_CURSOR_EVENT) {
            style_ex |= WS_EX_TRANSPARENT | WS_EX_LAYERED;
        }
        if self.contains(WindowFlags::CLIP_CHILDREN) {
            style |= WS_CLIPCHILDREN;
        }

        if self.intersects(
            WindowFlags::MARKER_EXCLUSIVE_FULLSCREEN | WindowFlags::MARKER_BORDERLESS_FULLSCREEN,
        ) {
            style &= !WS_OVERLAPPEDWINDOW;
        }

        (style, style_ex)
    }

    /// Adjust the window client rectangle to the return value, if present.
    fn apply_diff(mut self, window: HWND, mut new: WindowFlags) {
        self = self.mask();
        new = new.mask();

        let mut diff = self ^ new;

        if diff == WindowFlags::empty() {
            return;
        }

        if new.contains(WindowFlags::VISIBLE) {
            let flag = if !self.contains(WindowFlags::MARKER_ACTIVATE) {
                self.set(WindowFlags::MARKER_ACTIVATE, true);
                // SW_SHOWNOACTIVATE resets the window to its normal bounds.
                if new.intersects(WindowFlags::MAXIMIZED | WindowFlags::MINIMIZED) {
                    SW_SHOWNA
                } else {
                    SW_SHOWNOACTIVATE
                }
            } else {
                SW_SHOW
            };
            unsafe {
                ShowWindow(window, flag);
            }
        }

        if diff.intersects(WindowFlags::ALWAYS_ON_TOP | WindowFlags::ALWAYS_ON_BOTTOM) {
            unsafe {
                SetWindowPos(
                    window,
                    match (
                        new.contains(WindowFlags::ALWAYS_ON_TOP),
                        new.contains(WindowFlags::ALWAYS_ON_BOTTOM),
                    ) {
                        (true, false) => HWND_TOPMOST,
                        (false, false) => HWND_NOTOPMOST,
                        (false, true) => HWND_BOTTOM,
                        (true, true) => unreachable!(),
                    },
                    0,
                    0,
                    0,
                    0,
                    SWP_ASYNCWINDOWPOS | SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                InvalidateRgn(window, 0, false.into());
            }
        }

        let maximize_changed = diff.contains(WindowFlags::MAXIMIZED);
        let minimize_changed = diff.contains(WindowFlags::MINIMIZED);
        // Restrict the guard to our own ShowWindow calls. The window procedure
        // leaves window-manager and user-initiated visibility changes alone.
        let keep_hidden = !new.contains(WindowFlags::VISIBLE) && (maximize_changed || minimize_changed);
        unsafe {
            if keep_hidden {
                SendMessageW(window, event_loop::SET_RETAIN_STATE_ON_SIZE_MSG_ID.get(), 1, 0);
            }
            if maximize_changed {
                ShowWindow(window, match new.contains(WindowFlags::MAXIMIZED) {
                    true => SW_MAXIMIZE,
                    false => SW_RESTORE,
                });
            }
            // Minimize operations should execute after maximize for proper window animations.
            if minimize_changed {
                ShowWindow(window, match new.contains(WindowFlags::MINIMIZED) {
                    true => SW_MINIMIZE,
                    false => SW_RESTORE,
                });
            }
            if keep_hidden {
                SendMessageW(window, event_loop::SET_RETAIN_STATE_ON_SIZE_MSG_ID.get(), 0, 0);
            }
        }
        if minimize_changed {
            diff.remove(WindowFlags::MINIMIZED);
        }

        if diff.contains(WindowFlags::CLOSABLE) || new.contains(WindowFlags::CLOSABLE) {
            let flags = MF_BYCOMMAND
                | if new.contains(WindowFlags::CLOSABLE) { MF_ENABLED } else { MF_DISABLED };

            unsafe {
                EnableMenuItem(GetSystemMenu(window, 0), SC_CLOSE, flags);
            }
        }

        if !new.contains(WindowFlags::VISIBLE) {
            unsafe {
                ShowWindow(window, SW_HIDE);
            }
        }

        if diff != WindowFlags::empty() {
            let (style, style_ex) = new.to_window_styles();

            unsafe {
                SendMessageW(window, event_loop::SET_RETAIN_STATE_ON_SIZE_MSG_ID.get(), 1, 0);

                // This condition is necessary to avoid having an unrestorable window
                if !new.contains(WindowFlags::MINIMIZED) {
                    SetWindowLongW(window, GWL_STYLE, style as i32);
                    SetWindowLongW(window, GWL_EXSTYLE, style_ex as i32);
                }

                let mut flags = SWP_NOZORDER | SWP_NOMOVE | SWP_NOSIZE | SWP_FRAMECHANGED;

                // We generally don't want style changes here to affect window
                // focus, but for fullscreen windows they must be activated
                // (i.e. focused) so that they appear on top of the taskbar
                if !new.contains(WindowFlags::MARKER_EXCLUSIVE_FULLSCREEN)
                    && !new.contains(WindowFlags::MARKER_BORDERLESS_FULLSCREEN)
                {
                    flags |= SWP_NOACTIVATE;
                }

                // Refresh the window frame
                SetWindowPos(window, 0, 0, 0, 0, 0, flags);
                SendMessageW(window, event_loop::SET_RETAIN_STATE_ON_SIZE_MSG_ID.get(), 0, 0);
            }
        }
    }

    pub fn adjust_rect(self, hwnd: HWND, mut rect: RECT) -> Result<RECT, io::Error> {
        unsafe {
            let mut style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            let style_ex = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;

            // Frameless style implemented by manually overriding the non-client area in
            // `WM_NCCALCSIZE`.
            if !self.contains(WindowFlags::MARKER_DECORATIONS) {
                style &= !(WS_CAPTION | WS_SIZEBOX);
            }

            util::win_to_err({
                let b_menu = GetMenu(hwnd) != 0;
                if let (Some(get_dpi_for_window), Some(adjust_window_rect_ex_for_dpi)) =
                    (*util::GET_DPI_FOR_WINDOW, *util::ADJUST_WINDOW_RECT_EX_FOR_DPI)
                {
                    let dpi = get_dpi_for_window(hwnd);
                    adjust_window_rect_ex_for_dpi(&mut rect, style, b_menu.into(), style_ex, dpi)
                } else {
                    AdjustWindowRectEx(&mut rect, style, b_menu.into(), style_ex)
                }
            })?;
            Ok(rect)
        }
    }

    pub fn adjust_size(self, hwnd: HWND, size: PhysicalSize<u32>) -> PhysicalSize<u32> {
        let (width, height): (u32, u32) = size.into();
        let rect = RECT { left: 0, right: width as i32, top: 0, bottom: height as i32 };
        let rect = self.adjust_rect(hwnd, rect).unwrap_or(rect);

        let outer_x = (rect.right - rect.left).abs();
        let outer_y = (rect.top - rect.bottom).abs();

        PhysicalSize::new(outer_x as _, outer_y as _)
    }

    pub fn set_size(self, hwnd: HWND, size: PhysicalSize<u32>) {
        unsafe {
            let (width, height): (u32, u32) = self.adjust_size(hwnd, size).into();
            SetWindowPos(
                hwnd,
                0,
                0,
                0,
                width as _,
                height as _,
                SWP_ASYNCWINDOWPOS | SWP_NOZORDER | SWP_NOREPOSITION | SWP_NOMOVE | SWP_NOACTIVATE,
            );
            InvalidateRgn(hwnd, 0, false.into());
        }
    }
}

#[cfg(test)]
mod frameless_owned_window_tests {
    use super::*;

    fn document_flags() -> WindowFlags {
        WindowFlags::POPUP
            | WindowFlags::TRANSPARENT
            | WindowFlags::RESIZABLE
            | WindowFlags::MINIMIZABLE
            | WindowFlags::MAXIMIZABLE
            | WindowFlags::CLOSABLE
            | WindowFlags::VISIBLE
    }

    #[test]
    fn hidden_geometry_change_suppresses_only_its_show_side_effect() {
        let resizing = WindowFlags::MAXIMIZED | WindowFlags::MARKER_RETAIN_STATE_ON_SIZE;
        let flags = SWP_SHOWWINDOW | SWP_NOZORDER;
        assert_eq!(resizing.preserve_hidden_position(flags), SWP_HIDEWINDOW | SWP_NOACTIVATE | SWP_NOZORDER);
        assert_eq!(resizing.preserve_hidden_position(SWP_NOZORDER), SWP_NOZORDER);
        assert_eq!((resizing | WindowFlags::VISIBLE).preserve_hidden_position(flags), flags);
        assert_eq!(WindowFlags::MAXIMIZED.preserve_hidden_position(flags), flags);
    }

    #[test]
    fn custom_owned_frame_policy_excludes_other_frame_owners() {
        assert!(document_flags().custom_transparent_owned_frame());
        for flags in [
            document_flags() | WindowFlags::MARKER_DECORATIONS,
            document_flags() | WindowFlags::MARKER_UNDECORATED_SHADOW,
            document_flags() & !WindowFlags::POPUP,
            document_flags() & !WindowFlags::TRANSPARENT,
        ] {
            assert!(!flags.custom_transparent_owned_frame(), "{flags:?}");
        }
        for state in [WindowFlags::MINIMIZED, WindowFlags::MAXIMIZED] {
            assert!((document_flags() | state).custom_transparent_owned_frame());
        }
    }

    #[test]
    fn transparent_owned_frame_has_no_native_caption_or_edge() {
        let (style, extended) = document_flags().to_window_styles();
        assert_eq!(style & (WS_CAPTION | WS_BORDER | WS_SIZEBOX), 0);
        assert_eq!(extended & WS_EX_WINDOWEDGE, 0);
        let behavior = WS_POPUP | WS_MINIMIZEBOX | WS_MAXIMIZEBOX | WS_SYSMENU;
        assert_eq!(style & behavior, behavior);
        assert_eq!(extended & WS_EX_TOPMOST, 0);
        assert_eq!(style & WS_CHILD, 0);
    }

    #[test]
    fn transparent_owned_frame_keeps_minimize_and_maximize_state() {
        for state in [WindowFlags::MINIMIZED, WindowFlags::MAXIMIZED] {
            let (style, extended) = (document_flags() | state).to_window_styles();
            assert_eq!(style & (WS_CAPTION | WS_BORDER | WS_SIZEBOX), 0);
            assert_eq!(extended & WS_EX_WINDOWEDGE, 0);
            let expected = if state == WindowFlags::MINIMIZED { WS_MINIMIZE } else { WS_MAXIMIZE };
            assert_eq!(style & expected, expected);
            assert_eq!(style & WS_SIZEBOX, 0);
        }
    }

    #[test]
    fn taskbar_owned_document_keeps_popup_ownership_without_native_frame_or_topmost() {
        for state in [WindowFlags::empty(), WindowFlags::MINIMIZED, WindowFlags::MAXIMIZED] {
            let flags = document_flags() | WindowFlags::ON_TASKBAR | state;
            let (style, extended) = flags.to_window_styles();
            assert_ne!(extended & WS_EX_APPWINDOW, 0);
            assert_eq!(extended & (WS_EX_TOPMOST | WS_EX_WINDOWEDGE), 0);
            assert_ne!(style & WS_POPUP, 0);
            assert_eq!(style & (WS_CHILD | WS_CAPTION | WS_BORDER | WS_SIZEBOX), 0);
        }
        let (_, hidden) = document_flags().to_window_styles();
        assert_eq!(hidden & WS_EX_APPWINDOW, 0, "owned popup dialogs can still skip the taskbar");
    }

    #[test]
    fn other_window_kinds_retain_native_frame_styles() {
        for flags in [
            document_flags() | WindowFlags::MARKER_DECORATIONS,
            document_flags() | WindowFlags::MARKER_UNDECORATED_SHADOW,
            document_flags() & !WindowFlags::POPUP,
            document_flags() & !WindowFlags::TRANSPARENT,
        ] {
            let (style, extended) = flags.to_window_styles();
            assert_eq!(style & WS_CAPTION, WS_CAPTION, "{flags:?}");
            assert_ne!(extended & WS_EX_WINDOWEDGE, 0, "{flags:?}");
            assert_ne!(style & WS_SIZEBOX, 0, "{flags:?}");
        }
    }

    #[test]
    fn non_resizable_owned_window_does_not_gain_a_sizing_frame() {
        let (style, _) = (document_flags() & !WindowFlags::RESIZABLE).to_window_styles();
        assert_eq!(style & (WS_CAPTION | WS_BORDER | WS_SIZEBOX), 0);
    }
}

impl CursorFlags {
    fn refresh_os_cursor(self, window: HWND) -> Result<(), io::Error> {
        let client_rect = util::WindowArea::Inner.get_rect(window)?;

        if util::is_focused(window) {
            let cursor_clip = match self.contains(CursorFlags::GRABBED) {
                true => {
                    if self.contains(CursorFlags::LOCKED) {
                        if let Ok(pos) = util::get_cursor_position() {
                            Some(RECT {
                                left: pos.x,
                                right: pos.x + 1,
                                top: pos.y,
                                bottom: pos.y + 1,
                            })
                        } else {
                            // If lock is applied while the cursor is not available, lock it to the
                            // middle of the window.
                            let cx = (client_rect.left + client_rect.right) / 2;
                            let cy = (client_rect.top + client_rect.bottom) / 2;
                            Some(RECT { left: cx, right: cx + 1, top: cy, bottom: cy + 1 })
                        }
                    } else if self.contains(CursorFlags::HIDDEN) {
                        // Confine the cursor to the center of the window if the cursor is hidden.
                        // This avoids problems with the cursor activating
                        // the taskbar if the window borders or overlaps that.
                        let cx = (client_rect.left + client_rect.right) / 2;
                        let cy = (client_rect.top + client_rect.bottom) / 2;
                        Some(RECT { left: cx, right: cx + 1, top: cy, bottom: cy + 1 })
                    } else {
                        Some(client_rect)
                    }
                },
                false => None,
            };

            let rect_to_tuple = |rect: RECT| (rect.left, rect.top, rect.right, rect.bottom);
            let active_cursor_clip = rect_to_tuple(util::get_cursor_clip()?);
            let desktop_rect = rect_to_tuple(util::get_desktop_rect());

            let active_cursor_clip = match desktop_rect == active_cursor_clip {
                true => None,
                false => Some(active_cursor_clip),
            };

            // We do this check because calling `set_cursor_clip` incessantly will flood the event
            // loop with `WM_MOUSEMOVE` events, and `refresh_os_cursor` is called by
            // `set_cursor_flags` which at times gets called once every iteration of the
            // eventloop.
            if active_cursor_clip != cursor_clip.map(rect_to_tuple) {
                util::set_cursor_clip(cursor_clip)?;
            }
        }

        let cursor_in_client = self.contains(CursorFlags::IN_WINDOW);
        if cursor_in_client {
            util::set_cursor_hidden(self.contains(CursorFlags::HIDDEN));
        } else {
            util::set_cursor_hidden(false);
        }

        Ok(())
    }
}
