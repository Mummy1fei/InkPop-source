# PhotoCraft eframe patch

## v36: one visible frame for transparent owned windows

The ownership helper applies safe winit Windows attributes to transparent
undecorated child viewports: no undecorated shadow, no DWM border color and
DoNotRound corner preference. egui-winit 0.36.2 ignores has_shadow on Windows
and otherwise enables the undecorated shadow unconditionally. The explicit
creation-time override fixes this missing platform translation; setting the
egui ViewportBuilder flag alone does not disable the native shadow.
On Windows 11 these prevent a second system outline/radius around the themed
egui outline. Windows 10 is handled by the accompanying winit native style and
initial client-area policy; the Windows 11 hints alone are insufficient there.
Root and decorated dialog attributes, ownership, modal state and input are unchanged.

Version: eframe 0.36.2. Upstream repository: https://github.com/emilk/egui.
The package records upstream commit `49682f8baa058bf49e011035cfbd6e825f88a5ef`.
The original MIT and Apache-2.0 licenses are included unchanged; license texts
were retrieved from that commit on 2026-10-19.

This directory was copied from the local Cargo source used to build PhotoCraft
v34, rather than replacing it with a new eframe release. That source already
contains local changes to preserve transparent child windows and continue
rendering a root viewport when a visible immediate descendant needs it. Those
existing changes are preserved. Cargo registry files were not edited for v35.

## v35: Windows owned document windows

`src/native/owned_window.rs` applies winit's safe
`WindowAttributesExtWindows::with_owner_window` to each non-root viewport at
creation. The HWND comes from the live root `Window` through the safe
`HasWindowHandle` API. Both wgpu and glow creation paths call the helper; root
window creation and non-Windows behavior are unchanged. The root `Arc<Window>`
remains live through child creation.

Win32 ownership keeps a floating document above its application root without
`WS_EX_TOPMOST`, and lets Windows hide the owned document when its owner is
minimized. This is ownership of a separate top-level window, not `WS_CHILD`
parenting: detached documents may still move outside the root client area.
All secondary viewports currently belong to the root, including native modal
hosts. The application is responsible for modal input blocking and for raising
the modal host above sibling document windows.

The app must remove its previous focus-dependent `AlwaysOnTop` commands.
An egui modal painted inside the root remains behind owned windows; it must be
hosted in a native modal window, or the application must suspend floating window
visibility while that root modal is active. Ownership alone does not solve that
separate presentation requirement.

No new unsafe block, registry mutation, focus polling, topmost toggle, or
per-frame z-order operation is introduced by this patch.

## Native modal input contract

The UI publishes the active native modal's `egui::ViewportId` as temporary
context data at `egui::Id::new("photocraft-native-modal-viewport")`. Once its
native window exists, the Windows backend disables other viewport HWNDs using
winit's safe `WindowExtWindows::set_enable`. Removing the marker restores only
windows this backend disabled. Synchronization runs after immediate-window
creation and around viewport output processing, including before an omitted
modal window is destroyed. Non-Windows platforms do not change native enable
state.

The backend caches the set of WindowIds it disabled. winit 0.30.13 exposes no
enabled-state getter; the cache avoids repeated EnableWindow calls and avoids
enabling windows it never disabled (for example a native file dialog's owner).
This is state owned by this modal bridge, not a poll of arbitrary native state.

Before a synchronous native file dialog, the UI must remove the marker and
complete a frame so the backend restores owner windows. A modal button answer
can be queued for the next root frame to achieve this. Removing a context marker
and immediately entering a blocking file dialog within the same UI callback
does not give the backend an opportunity to restore native input state.

## Validation required

The main task owns Cargo checks and native visual verification. This patch was
not independently compiled by its authoring subagent. Check both creation paths
when their renderer features are enabled; test owner HWND, absence of TOPMOST,
tool/root focus changes, another application becoming foreground, root minimize
and restore, native modal ordering, and the last document closing. Synthetic UI
tests alone do not establish these operating-system behaviors.

References:

- https://learn.microsoft.com/en-us/windows/win32/winmsg/window-features#owned-windows
- https://docs.rs/winit/0.30.13/winit/platform/windows/trait.WindowAttributesExtWindows.html#tymethod.with_owner_window

## Immediate viewport screenshots
The wgpu immediate renderer now forwards Screenshot actions to its painter, as the root renderer already does. Previously child capture requests stayed queued forever. This enables native child surface QA; it does not capture desktop composition.

v43 also recognizes the independent `photocraft-native-preset-modal-viewport` marker for the New Brush Preset naming dialog. Existing discard modal takes precedence. Only backend-disabled windows are re-enabled after confirmation or cancellation; modal uses normal owner z-order, never topmost.

## v50 opt-in immediate viewport lifecycle evidence
Set PHOTOCRAFT_TRACE_VIEWPORTS only in an isolated QA launch to record viewport/native IDs, monotonic time and created/painted/show/hide/focus stages. Default production launches emit no trace. Painted records completion of the painter call, not a DWM/compositor or physical-display timestamp. This diagnostic logs no window titles, paths, image pixels or user input. Menu UI now creates first hosts hidden/inactive and queues visibility/focus after rendering; the immediate backend's existing paint-before-command order remains unchanged.


Reused immediate viewports also drain only queued OuterPosition/InnerSize builder changes before taking viewport input. Actual window size changes notify the painter before tessellation/present, so a newly tall menu is not painted into the former short surface. Reveal/hide/focus commands remain FIFO after paint. The production partition helper is tested directly by evidence/v50/geometry-fixture (separate cached offline harness); the excluded eframe standalone test could not resolve uncached optional egui_inspection while network indexing timed out. Native runtime tracing/geometry checks cover the full production backend.

