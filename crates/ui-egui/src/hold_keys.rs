//! Held keys: Photoshop's temporary tools (#249).
//!
//! - Space: the Hand tool while held.
//! - ⌘Space (Ctrl+Space off the Mac): Zoom In while held; a drag is a scrubby zoom.
//! - ⌘⌥Space: Zoom Out while held.
//! - Ctrl (or logical Command): Move while held; Auto-Select is temporarily on.
//! - Space while a marquee, lasso or shape is being dragged repositions it; releasing Space
//!   goes back to sizing it (the Crop tool does the same for its frame, `crop_ui`).
//!
//! The current tool is never changed, so releasing the key gives the previous tool back. A
//! temporary tool that started a drag lasts until the button is released, as in Photoshop.
//!
//! The keys are bindings, not commands: they come from
//! [`photocraft_engine::prefs::TEMPORARY_TOOLS`] with Edit › Keyboard Shortcuts overrides on top
//! (Tools › Temporary), and the reposition key is whatever the Hand key is.

use egui::{InputState, KeyboardShortcut};

use crate::PhotocraftApp;
use crate::shortcuts::parse;
use crate::state::Tool;

/// A tool in effect only while its key is held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Temporary {
    Hand,
    ZoomIn,
    ZoomOut,
    /// A modifier gesture, rather than an editable key/command binding.
    Move,
}

impl Temporary {
    pub fn id(self) -> &'static str {
        match self {
            Temporary::Hand => "tools.temporary.hand",
            Temporary::ZoomIn => "tools.temporary.zoomIn",
            Temporary::ZoomOut => "tools.temporary.zoomOut",
            Temporary::Move => "tools.temporary.move",
        }
    }

    pub fn tool(self) -> Tool {
        match self {
            Temporary::Hand => Tool::Hand,
            Temporary::ZoomIn | Temporary::ZoomOut => Tool::Zoom,
            Temporary::Move => Tool::Move,
        }
    }

    const ALL: [Temporary; 3] = [Temporary::Hand, Temporary::ZoomIn, Temporary::ZoomOut];
}

/// Is `id` a held temporary tool rather than a command?
pub fn is_temporary(id: &str) -> bool {
    id == Temporary::Move.id() || photocraft_engine::prefs::TEMPORARY_TOOLS.iter().any(|t| t.0 == id)
}

/// The key binding of a temporary tool in effect (`None` when removed or unparsable).
pub fn binding(app: &PhotocraftApp, t: Temporary) -> Option<KeyboardShortcut> {
    let def = photocraft_engine::prefs::TEMPORARY_TOOLS.iter().find(|x| x.0 == t.id()).map(|x| x.2);
    parse(app.session.prefs().shortcut(t.id(), def)?)
}

/// Is `sc` held: its key down with at least its modifiers (more may be held, so ⇧ still squares
/// a marquee while Space is down).
fn held(i: &InputState, sc: &KeyboardShortcut) -> bool {
    let (want, m) = (sc.modifiers, i.modifiers);
    i.key_down(sc.logical_key)
        && (!want.alt || m.alt)
        && (!want.shift || m.shift)
        && (!want.command || m.command)
        && (!want.ctrl || m.ctrl)
        && (!want.mac_cmd || m.mac_cmd)
}

/// The temporary tool whose key is held now, the most specific binding first (⌘⌥Space before
/// ⌘Space before Space). Never while a text field has the keyboard.
pub fn held_tool(app: &PhotocraftApp, ctx: &egui::Context) -> Option<Temporary> {
    if keyboard_protected(app, ctx) {
        return None;
    }
    let count = |sc: &KeyboardShortcut| {
        let m = sc.modifiers;
        m.alt as u8 + m.shift as u8 + (m.command || m.mac_cmd) as u8 + (m.ctrl && !m.command) as u8
    };
    let mut best: Option<(u8, Temporary)> = None;
    ctx.input(|i| {
        for t in Temporary::ALL {
            if let Some(sc) = binding(app, t)
                && held(i, &sc)
                && best.is_none_or(|(n, _)| count(&sc) > n)
            {
                best = Some((count(&sc), t));
            }
        }
    });
    best.map(|(_, t)| t)
        .or_else(|| ctx.input(|input| (ctrl_move_requested(app, input.modifiers) && !input.key_down(egui::Key::Space)).then_some(Temporary::Move)))
}

/// The modifier is inclusive: Ctrl never turns an already enabled Auto-Select off.
pub fn control_held(modifiers: egui::Modifiers) -> bool {
    modifiers.ctrl || modifiers.command || modifiers.mac_cmd
}

/// The options checkbox and pointer pickup use the same effective value. A UI
/// checkbox click edits only the stored value; releasing Ctrl reveals it again.
pub fn auto_select(app: &PhotocraftApp, modifiers: egui::Modifiers) -> bool {
    app.ui.tool_options.move_auto_select || control_held(crate::workspace_ui::sticky_mods(app, modifiers))
}

pub fn auto_select_for_ui(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    app.ui.tool_options.move_auto_select || auto_select_held(app, ctx)
}

pub fn auto_select_held(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    if let Some(state) = published_canvas(app, ctx) {
        return state.control;
    }
    !keyboard_protected(app, ctx) && ctx.input(|input| control_held(crate::workspace_ui::sticky_mods(app, input.modifiers)))
}

/// Context-independent protection also used by document-coordinate automation.
/// An existing stroke, outline or modal operation must finish with its owner.
pub fn ctrl_move_requested(app: &PhotocraftApp, modifiers: egui::Modifiers) -> bool {
    let modifiers = crate::workspace_ui::sticky_mods(app, modifiers);
    control_held(modifiers)
        && !modifiers.alt
        && app.ui.text_edit.is_none()
        && app.ui.dialogs.is_empty()
        && app.ui.transform.is_none()
        && app.discard.is_none()
        && app.camera_raw.is_none()
        && !app.distort.active()
        && app.distort.gallery.is_none()
        && app.brush_resize.is_none()
        && !app.alt_sampling
        && app.drag.as_ref().is_none_or(|drag| drag.tool == Tool::Move)
        && !crate::crop_ui::active(app)
        && app.ui.pen.is_none()
        && app.ui.polygon.is_empty()
        && !app.ui.magnetic.active()
        && app.ui.direct_selection.drag.is_none()
        && app.ui.analysis.drag.is_none()
        && app.ui.slices.drag.is_none()
}

pub(crate) fn keyboard_protected(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    ctx.text_edit_focused()
        || app.ui.text_edit.is_some()
        || !app.ui.dialogs.is_empty()
        || app.discard.is_some()
        || app.camera_raw.is_some()
        || app.distort.active()
        || app.distort.gallery.is_some()
        || crate::menu_nav::is_open(ctx)
        || egui::Popup::is_any_open(ctx)
}

fn panel_alt_id() -> egui::Id {
    egui::Id::new("pc-focused-panel-eyedropper-alt")
}

/// An owned native panel can keep keyboard focus while the pointer returns to the canvas.
pub(crate) fn publish_panel_alt(app: &PhotocraftApp, ctx: &egui::Context) {
    let viewport = ctx.viewport_id();
    if viewport == egui::ViewportId::ROOT || ctx.embed_viewports() {
        return;
    }
    let held = ctx.input(|input| input.focused && input.modifiers.alt && !input.modifiers.ctrl) && !keyboard_protected(app, ctx);
    let changed = ctx.data_mut(|data| {
        let old = data.get_temp::<egui::ViewportId>(panel_alt_id());
        if held && old != Some(viewport) {
            data.insert_temp(panel_alt_id(), viewport);
            true
        } else if !held && old == Some(viewport) {
            data.remove_temp::<egui::ViewportId>(panel_alt_id());
            true
        } else {
            false
        }
    });
    if changed {
        ctx.request_repaint_of(egui::ViewportId::ROOT);
    }
}

/// Refresh Alt and Ctrl without changing the other modifiers on the input event.
fn eyedropper_canvas_modifiers(app: &PhotocraftApp, ctx: &egui::Context, mut local: egui::Modifiers) -> egui::Modifiers {
    if let Some(read) = &app.services.eyedropper_cursor_modifiers {
        let active = ctx.data(|data| data.get_temp::<bool>(egui::Id::new("photocraft-native-app-active"))) != Some(false);
        // The canvas can be entered or activated while another owned window receives Alt.
        let physical = if active { read() } else { egui::Modifiers::NONE };
        local.alt = physical.alt;
        local.ctrl = physical.ctrl;
        return local;
    }
    if !local.alt
        && !ctx.embed_viewports()
        && let Some(panel) = ctx.data(|data| {
            if data.get_temp::<bool>(egui::Id::new("photocraft-native-app-active")) == Some(false) {
                return None;
            }
            data.get_temp::<egui::ViewportId>(panel_alt_id())
        })
        && ctx.input(|input| !input.focused && input.raw.viewports.get(&panel).and_then(|info| info.focused) == Some(true))
    {
        local.alt = true;
    }
    local
}

pub(crate) fn eyedropper_hover_modifiers(app: &PhotocraftApp, ctx: &egui::Context, local: egui::Modifiers) -> egui::Modifiers {
    crate::workspace_ui::sticky_mods(app, eyedropper_canvas_modifiers(app, ctx, local))
}

/// egui emits the click or first drag after the press, possibly after Alt changed.
/// Keep only the press owned by this canvas; a different tool or viewport cannot reuse it.
pub(crate) fn canvas_primary_press_modifiers(
    app: &PhotocraftApp,
    response: &egui::Response,
    owner: (photocraft_doc::DocId, Tool),
    enabled: bool,
    recover_eyedropper: bool,
) -> Option<egui::Modifiers> {
    let ctx = &response.ctx;
    let button = egui::PointerButton::Primary;
    let starting = response.drag_started_by(button);
    let (pressed, released, cancelled) = ctx.input(|input| {
        (
            input.pointer.button_pressed(button),
            input.pointer.button_released(button),
            input.events.iter().any(|event| matches!(event, egui::Event::WindowFocused(false))),
        )
    });
    if !pressed && !released && !starting && !cancelled {
        return None;
    }
    let id = response.id.with(("canvas-primary-press", ctx.viewport_id()));
    let owns_pointer = response.contains_pointer() || response.is_pointer_button_down_on() || response.clicked_by(button) || starting;
    let (press, modifier_stable, lost_after_press) = if enabled && pressed && owns_pointer {
        ctx.input(|input| {
            let Some((index, mods)) = input.events.iter().enumerate().rev().find_map(|(index, event)| match event {
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers } if response.interact_rect.contains(*pos) => {
                    Some((index, *modifiers))
                }
                _ => None,
            }) else {
                return (None, false, cancelled);
            };
            let mut later = input.events.iter().skip(index + 1);
            let stable = !later.clone().any(|e| match e {
                egui::Event::ModifiersChanged(m) | egui::Event::PointerButton { modifiers: m, .. } => (m.alt, m.ctrl) != (mods.alt, mods.ctrl),
                _ => false,
            });
            let lost = later.any(|e| matches!(e, egui::Event::WindowFocused(false)));
            (Some(mods), stable, lost)
        })
    } else {
        (None, false, cancelled)
    };
    // Keep an explicit press-time Alt (and any Ctrl+Alt chord). Recover a missing Alt
    // only on the owned press frame, when no later event reports a key transition.
    let press = press.map(|mods| {
        let mut mods = mods;
        if recover_eyedropper && modifier_stable && !mods.alt && !mods.ctrl {
            let live = eyedropper_canvas_modifiers(app, ctx, mods);
            mods.alt = live.alt && !live.ctrl;
        }
        mods
    });
    ctx.data_mut(|data| {
        let mods = if pressed {
            press
        } else if enabled && !lost_after_press {
            data.get_temp::<((photocraft_doc::DocId, Tool), egui::Modifiers)>(id).filter(|(saved, _)| *saved == owner).map(|(_, mods)| mods)
        } else {
            None
        };
        if let Some(mods) = mods.filter(|_| pressed && !released && !starting && !lost_after_press) {
            data.insert_temp(id, (owner, mods));
        } else {
            data.remove::<((photocraft_doc::DocId, Tool), egui::Modifiers)>(id);
        }
        mods.filter(|_| !lost_after_press)
    })
}

/// Pure view of the active tool for options, toolbar highlighting and cursors.
/// The saved tool and its per-tool brush slots are never changed by a held key.
pub fn effective_tool(app: &PhotocraftApp, ctx: &egui::Context) -> Tool {
    if let Some(drag) = &app.drag {
        return drag.tool;
    }
    if let Some(state) = published_canvas(app, ctx) {
        return state.temporary.map_or(app.ui.tool, Temporary::tool);
    }
    let latch = latch_id(ctx);
    if ctx.input(|input| input.pointer.primary_down())
        && let Some(tool) = ctx.data(|data| data.get_temp::<Option<Temporary>>(latch)).flatten()
    {
        return tool.tool();
    }
    held_tool(app, ctx).map_or(app.ui.tool, Temporary::tool)
}

/// Is the reposition key (the Hand key, any modifiers) down?
pub fn reposition_held(app: &PhotocraftApp, ctx: &egui::Context) -> bool {
    !ctx.text_edit_focused() && binding(app, Temporary::Hand).is_some_and(|sc| ctx.input(|i| i.key_down(sc.logical_key)))
}

/// Tools whose in-progress drag the reposition key moves.
pub fn repositions(t: Tool) -> bool {
    matches!(t, Tool::RectMarquee | Tool::EllipseMarquee | Tool::Lasso | Tool::ObjectSelection) || crate::vector_ui::is_shape_tool(t)
}

fn latch_id(ctx: &egui::Context) -> egui::Id {
    egui::Id::new(("pc-temporary-tool", ctx.viewport_id()))
}

#[derive(Clone, Copy, PartialEq)]
struct CanvasHeld {
    viewport: egui::ViewportId,
    document: Option<photocraft_doc::DocId>,
    temporary: Option<Temporary>,
    control: bool,
}

fn publication_id() -> egui::Id {
    egui::Id::new("pc-focused-canvas-held-tool")
}

/// Native child input is viewport-local. Its focused canvas publishes the
/// resolved tool for the ROOT toolbar instead of reading ROOT's stale Ctrl.
fn published_canvas(app: &PhotocraftApp, ctx: &egui::Context) -> Option<CanvasHeld> {
    if ctx.viewport_id() != egui::ViewportId::ROOT || ctx.embed_viewports() || keyboard_protected(app, ctx) {
        return None;
    }
    let state = ctx.data(|data| data.get_temp::<CanvasHeld>(publication_id()))?;
    (state.viewport != egui::ViewportId::ROOT
        && state.document == app.session.active().map(|doc| doc.doc.id)
        && ctx.input(|input| input.raw.viewports.get(&state.viewport).and_then(|info| info.focused) == Some(true)))
    .then_some(state)
}

/// The temporary tool for this canvas frame: the held one, except while a selection is being
/// drawn (the key repositions it instead); a temporary tool that was in effect when the button
/// went down stays until the button comes up.
pub fn for_frame(app: &PhotocraftApp, ctx: &egui::Context, drawing: bool) -> Option<Temporary> {
    let down = ctx.input(|i| i.pointer.primary_down());
    let latch = latch_id(ctx);
    let latched = ctx.data(|d| d.get_temp::<Option<Temporary>>(latch)).flatten();
    let released = ctx.input(|input| input.pointer.button_released(egui::PointerButton::Primary));
    let t = if (down || released) && latched.is_some() {
        latched
    } else if app.drag.as_ref().is_some_and(|drag| drag.tool == Tool::Move) && app.ui.tool != Tool::Move {
        Some(Temporary::Move)
    } else if drawing {
        None
    } else {
        held_tool(app, ctx)
    };
    ctx.data_mut(|d| d.insert_temp(latch, if down { t } else { None }));
    if ctx.input(|input| input.focused) {
        let state = CanvasHeld {
            viewport: ctx.viewport_id(),
            document: app.session.active().map(|doc| doc.doc.id),
            temporary: t,
            control: !keyboard_protected(app, ctx) && ctx.input(|input| control_held(crate::workspace_ui::sticky_mods(app, input.modifiers))),
        };
        let changed = ctx.data(|data| data.get_temp::<CanvasHeld>(publication_id())) != Some(state);
        ctx.data_mut(|data| data.insert_temp(publication_id(), state));
        if changed && state.viewport != egui::ViewportId::ROOT {
            ctx.request_repaint_of(egui::ViewportId::ROOT);
        }
    }
    t
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod move_tests_v74;
