//! PhotoCraft Windows viewport ownership.
//!
//! egui's viewport parent controls rendering, but does not create Win32 ownership.
//! Owned top-level windows stay above their initiating viewport and follow its minimize state without
//! setting the global topmost flag. They remain separate desktop windows, not WS_CHILD.

use winit::window::{Window, WindowAttributes};

pub(super) fn attributes(
    attributes: WindowAttributes,
    viewport: egui::ViewportId,
    owner: Option<&Window>,
) -> WindowAttributes {
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
        use winit::platform::windows::{CornerPreference, WindowAttributesExtWindows as _};

        if viewport != egui::ViewportId::ROOT
            && let Some(owner) = owner
            && let Ok(handle) = owner.window_handle()
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            let attributes = attributes.with_owner_window(handle.hwnd.get());
            // The transparent document surface owns the themed outline and its alpha
            // corners. Do not add a second DWM outline or clip it to Windows' radius.
            // These hints apply on Windows 11; the vendored winit style policy also
            // removes the native caption/border on Windows 10.
            return if attributes.transparent && !attributes.decorations {
                // egui-winit 0.36.2 honors ViewportBuilder::has_shadow only on macOS;
                // its Windows creation path enables shadows for all undecorated windows.
                // Override that default before creation so Windows does not add an opaque
                // non-client edge/shadow around our transparent themed card.
                attributes
                    .with_undecorated_shadow(false)
                    .with_border_color(None)
                    .with_corner_preference(CornerPreference::DoNotRound)
            } else {
                attributes
            };
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (viewport, owner);

    attributes
}

/// Honor the app's native modal marker without repeatedly toggling HWND state.
/// Only windows disabled by this backend are re-enabled; an external native
/// dialog may independently disable its owner while it runs a nested event loop.
pub(super) fn sync_modal<'a>(
    ctx: &egui::Context,
    windows: impl Iterator<Item = (egui::ViewportId, &'a Window)>,
) {
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
        use std::collections::{HashMap, HashSet};
        use winit::platform::windows::WindowExtWindows as _;

        let windows: Vec<_> = windows.collect();
        // The app resolves a captured drag against the real top-level HWND under the pointer.
        // Publish the live handles without querying screen geometry or mouse position here.
        ctx.data_mut(|data| {
            let handles = data.get_temp_mut_or_default::<HashMap<isize, egui::ViewportId>>(
                egui::Id::new("pc-windows-viewport-handles"),
            );
            handles.clear();
            handles.extend(windows.iter().filter_map(|(viewport, window)| {
                let RawWindowHandle::Win32(handle) = window.window_handle().ok()?.as_raw() else {
                    return None;
                };
                Some((handle.hwnd.get(), *viewport))
            }));
        });
        ctx.data_mut(|data| data.insert_temp(
            egui::Id::new("photocraft-native-app-active"),
            winit::platform::windows::is_current_process_foreground(),
        ));
        // Publish native restore intent before app UI runs. A false minimized state
        // alone cannot distinguish taskbar restore from Windows showing an owned child.
        for (viewport, window) in &windows {
            let requested = winit::platform::windows::take_window_state_request(window.id());
            ctx.data_mut(|data| {
                data.insert_temp(egui::Id::new(("photocraft-native-restore-supported", *viewport)), true);
                if let Some(requested) = requested {
                    // 1 = restore, 2 = maximize, 3 = minimize. App crate need not
                    // depend on winit just to consume this platform-specific signal.
                    let kind = match requested {
                        winit::platform::windows::WindowStateRequest::Restore => 1_u8,
                        winit::platform::windows::WindowStateRequest::Maximize => 2_u8,
                        winit::platform::windows::WindowStateRequest::Minimize => 3_u8,
                    };
                    data.insert_temp(egui::Id::new(("photocraft-native-restore-request", *viewport)), kind);
                }
            });
        }
        let modal = ctx.data(|data| {
            data.get_temp::<egui::ViewportId>(egui::Id::new("photocraft-native-modal-viewport"))
                .or_else(|| data.get_temp::<egui::ViewportId>(egui::Id::new("photocraft-native-dialog-modal-viewport")))
                .or_else(|| data.get_temp::<egui::ViewportId>(egui::Id::new("photocraft-native-preset-modal-viewport")))
        });
        // Do not disable the owner unless the modal has actually been created.
        let modal = modal.filter(|modal| windows.iter().any(|(id, _)| id == modal));
        let state_id = egui::Id::new("photocraft-backend-modal-disabled-windows");
        let previous = ctx.data(|data| {
            data.get_temp::<HashSet<winit::window::WindowId>>(state_id)
                .unwrap_or_default()
        });
        let mut disabled = HashSet::new();
        // The app grants only read-only canvas navigation/sampling for generic
        // forms. Tie the grant to this exact modal so discard/job dialogs cannot
        // inherit an earlier dialog's exceptions.
        let canvas_input = ctx.data(|data| data.get_temp::<(egui::ViewportId, Vec<egui::ViewportId>)>(
            egui::Id::new("photocraft-native-modal-canvas-input"),
        )).filter(|(owner, _)| Some(*owner) == modal).map(|(_, allowed)| allowed).unwrap_or_default();
        let modal_window = modal.and_then(|modal| windows.iter().find(|(id, _)| *id == modal).map(|(_, window)| *window));
        for (viewport, window) in &windows {
            let should_disable = modal.is_some_and(|modal| *viewport != modal)
                && !canvas_input.contains(viewport)
                && !modal_window.is_some_and(|modal| winit::platform::windows::is_owned_descendant(window, modal));
            let was_disabled = previous.contains(&window.id());
            if should_disable != was_disabled {
                window.set_enable(!should_disable);
            }
            if should_disable {
                disabled.insert(window.id());
            }
        }
        ctx.data_mut(|data| data.insert_temp(state_id, disabled));
        // App role registry: document=0, panel=1, popup=2, modal=3. Only live
        // registered windows participate. Do not prune before a just-published
        // viewport has been created; creation and this sync may share a UI pass.
        let roles = ctx.data(|data| {
            data.get_temp::<HashMap<egui::ViewportId, u8>>(egui::Id::new("photocraft-native-window-roles"))
                .unwrap_or_default()
        });
        let ordered: Vec<_> = windows.iter().filter_map(|(viewport, window)| {
            if *viewport == egui::ViewportId::ROOT {
                return None;
            }
            let role = if Some(*viewport) == modal { Some(3) } else { roles.get(viewport).copied() }?;
            Some((*window, role))
        }).collect();
        winit::platform::windows::maintain_owned_window_order(&ordered);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (ctx, windows);
}
