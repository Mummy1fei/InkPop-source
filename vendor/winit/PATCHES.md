# PhotoCraft Windows patches

Base: crates.io `winit` 0.30.13, upstream source preserved under its Apache-2.0
license (see `LICENSE`). This directory is a Cargo path patch; the registry cache
is not edited. No new unsafe code is added by this patch.

## v36: transparent owned window frame

`window_state.rs` defines a shared policy for transparent, undecorated, shadowless
owned popups. These windows omit WS_CAPTION/WS_BORDER/WS_EX_WINDOWEDGE from the
initial style and every subsequent style update. WS_SIZEBOX, minimize/maximize
capabilities and owner behavior remain. Other window kinds retain upstream styles.

`event_loop.rs` also uses that policy for WM_NCCALCSIZE with wParam=FALSE:
the proposed RECT remains the client rectangle, matching the later TRUE path.
Calling DefWindowProc there previously applied the sizing frame at creation even
though later layouts used the entire client area. The existing maximized TRUE
path still bounds the window to monitor work area. Pointer/contact and activation
handling are unchanged.

Five pure policy/style tests cover scope, native resizing and state bits; actual
HWND styles and theme surface screenshots are checked by the v36 native QA script.
Surface screenshots do not prove the absence of DWM composition artifacts.

Changed files:

- `src/platform/windows.rs`: exports the safe Windows-only
  `pointer_screen_position(pointer_id: u64, at_start: bool)` API.
- `src/platform/windows/pointer_screen.rs`: bounded, event-thread-local storage
  for 32 contacts, fixed down anchors and latest physical desktop positions.
- `src/platform_impl/windows/event_loop.rs`: records the existing precise screen
  coordinates before `ScreenToClient` in WM_POINTER history and WM_TOUCH handling.
  Existing Touch delivery, pressure, event ordering, and history remain in place.

Why capture here: the Windows runner buffers Touch events during reentrant event
handling (`event_loop/runner.rs::send_event`). Reconstructing a screen point in
egui-winit from `touch.location + window.inner_position()` may combine an older
client coordinate with a newer native origin. Polling GetCursorPos instead reads
the mouse cursor, which need not follow an active Windows Ink contact. Capture at
the conversion boundary avoids both assumptions.

Read the API on the event-loop thread using the ID from Touch. `at_start=true`
returns DOWN even if several MOVE events have already arrived in the same batch.
UP retains its final point; subsequent hover updates do not replace it. A new
DOWN resets the anchor. Missing DOWN, another thread, teardown, and evicted
contacts return None. Non-finite positions are rejected. The application must
still use Touch End/Cancel and focus loss for lifecycle; a retained position does
not mean a contact is active. It must not substitute GetCursorPos for this feed
during a Touch gesture.

Pure unit tests cover continuous contact with an unchanged mouse cursor, buffered
coordinates surviving an origin change, DOWN/UP/hover reuse, independent contacts,
the storage bound, and non-finite rejection. These tests do not claim hardware
validation. Actual PTH-660 input, mixed DPI, native composition, and cancellation
behaviour still require runtime validation.

Official API references:

- https://learn.microsoft.com/en-us/windows/win32/inputmsg/wm-pointerupdate
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getpointerinfo
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-pointer_info

GetPointerInfo data is tied to the current pointer message and can expire after
the thread retrieves the next message. This patch reuses winit's existing
GetPointerFrameInfoHistory capture; it does not poll GetPointerInfo from UI frames.
