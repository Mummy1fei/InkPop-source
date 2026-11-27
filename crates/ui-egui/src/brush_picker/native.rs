//! The canvas quick picker is a separate owned desktop window, never a topmost overlay.

use egui::{Pos2, Vec2, ViewportCommand, ViewportId, vec2};
use photocraft_doc::DocId;
use serde_json::{Value, json};

use crate::{PhotocraftApp, brush_picker, state::Tool};

#[derive(Clone, Debug)]
struct NativePicker {
    parent: ViewportId,
    document: DocId,
    tool: Tool,
    position: Pos2,
    size: Vec2,
    opened: bool,
    focused_once: bool,
}

fn key() -> egui::Id {
    egui::Id::new("canvas-brush-picker-native-state")
}

fn viewport(parent: ViewportId) -> ViewportId {
    ViewportId::from_hash_of(("canvas-brush-picker-native", parent))
}

fn state(ctx: &egui::Context) -> Option<NativePicker> {
    ctx.data(|d| d.get_temp::<NativePicker>(key()))
}

fn set_role(ctx: &egui::Context, id: ViewportId, open: bool) {
    ctx.data_mut(|data| {
        let roles = data.get_temp_mut_or_default::<std::collections::HashMap<ViewportId, u8>>(egui::Id::new("photocraft-native-window-roles"));
        if open {
            roles.insert(id, 2);
        } else {
            roles.remove(&id);
        }
    });
}

fn begin_render(ctx: &egui::Context, parent: ViewportId, frame: u64) -> bool {
    ctx.data_mut(|data| {
        let id = egui::Id::new(("native-brush-picker-frame", parent));
        if data.get_temp::<u64>(id) == Some(frame) {
            return false;
        }
        data.insert_temp(id, frame);
        true
    })
}

pub(crate) fn native_viewport(ctx: &egui::Context) -> Option<ViewportId> {
    state(ctx).map(|s| viewport(s.parent))
}

pub(crate) fn inspect(ctx: &egui::Context) -> Value {
    state(ctx).map_or(Value::Null, |s| {
        json!({"document": s.document.0, "viewport": format!("{:?}", viewport(s.parent)), "parent": format!("{:?}", s.parent),
            "position": [s.position.x, s.position.y], "size": [s.size.x, s.size.y], "opened": s.opened})
    })
}

fn bounded_size(size: [f32; 2]) -> Vec2 {
    vec2(if size[0].is_finite() { size[0].clamp(260.0, 1600.0) } else { 324.0 }, if size[1].is_finite() { size[1].clamp(280.0, 1400.0) } else { 480.0 })
}

fn remember_size(app: &mut PhotocraftApp, size: Vec2) {
    let size = bounded_size([size.x, size.y]);
    let value = [size.x.round(), size.y.round()];
    if app.session.prefs().tools.brush_picker_size != value {
        app.session.prefs.edit(|p| p.tools.brush_picker_size = value);
    }
}

/// Open at a pointer position in the initiating canvas viewport's client coordinates.
/// The control channel uses this same entry point to exercise floating-canvas ownership.
pub(crate) fn open_at(app: &mut PhotocraftApp, ctx: &egui::Context, parent: ViewportId, document: DocId, at: [f32; 2]) {
    close(app, ctx);
    if !at.into_iter().all(f32::is_finite) || !app.session.documents().iter().any(|st| st.doc.id == document) {
        return;
    }
    app.ui.brush_picker = Some(at);
    if ctx.embed_viewports() {
        return;
    }
    let origin = ctx.input(|i| i.raw.viewports.get(&parent).and_then(|v| v.inner_rect).map_or(Pos2::ZERO, |r| r.min));
    let picker = NativePicker {
        parent,
        document,
        tool: app.ui.tool,
        position: origin + vec2(at[0], at[1]),
        size: bounded_size(app.session.prefs().tools.brush_picker_size),
        opened: false,
        focused_once: false,
    };
    ctx.data_mut(|d| d.insert_temp(key(), picker));
    set_role(ctx, viewport(parent), true);
    ctx.request_repaint_of(parent);
}

/// Close before displaying a modal or switching to another document/tool.
pub(crate) fn close(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if let Some(s) = state(ctx) {
        remember_size(app, s.size);
        ctx.send_viewport_cmd_to(viewport(s.parent), ViewportCommand::Visible(false));
        ctx.request_repaint_of(s.parent);
        set_role(ctx, viewport(s.parent), false);
        ctx.data_mut(|data| data.remove::<u64>(egui::Id::new(("native-brush-picker-frame", s.parent))));
    }
    ctx.data_mut(|d| d.remove::<NativePicker>(key()));
    app.ui.brush_picker = None;
}

/// Root-frame cleanup also runs when closing the source document stops its canvas callback.
pub(crate) fn maintain(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(s) = state(ctx) else { return };
    let owner_hidden = ctx.input(|i| [ViewportId::ROOT, s.parent].iter().any(|id| i.raw.viewports.get(id).is_some_and(|v| v.minimized == Some(true))));
    let modal = app.discard.is_some() || app.camera_raw.is_some() || !app.ui.dialogs.is_empty() || app.distort.liquify.is_some();
    let owner_exists = s.parent == ViewportId::ROOT || app.ui.windows.iter().any(|w| w.open && ViewportId::from_hash_of(("docwin", w.id)) == s.parent);
    if app.ui.brush_picker.is_none()
        || app.ui.tool != s.tool
        || modal
        || owner_hidden
        || !owner_exists
        || !app.session.documents().iter().any(|doc| doc.doc.id == s.document)
    {
        close(app, ctx);
    }
}

/// Native test/control resize: persistence observes the resulting actual client geometry.
pub(crate) fn resize_native(ctx: &egui::Context, size: [f32; 2]) -> bool {
    let Some(id) = native_viewport(ctx) else { return false };
    ctx.send_viewport_cmd_to(id, ViewportCommand::InnerSize(bounded_size(size)));
    ctx.request_repaint_of(id);
    true
}

fn builder(s: &NativePicker) -> egui::ViewportBuilder {
    let mut b = egui::ViewportBuilder::default()
        .with_title(tl!("Brush Preset picker"))
        .with_transparent(false)
        .with_resizable(true)
        .with_min_inner_size(vec2(260.0, 280.0))
        .with_max_inner_size(vec2(1600.0, 1400.0))
        .with_minimize_button(false)
        .with_maximize_button(false)
        .with_taskbar(false)
        .with_window_level(egui::WindowLevel::Normal);
    if !s.opened {
        b = b.with_position(s.position).with_inner_size(s.size).with_active(true);
    }
    b
}

fn reveal(ctx: &egui::Context, id: ViewportId) {
    // A same-frame reopen can still have the previous window's Hide queued.
    ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(true));
    ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
}

/// Invoked in the initiating canvas callback, so egui's native parent is that exact viewport.
pub(crate) fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    crate::brush_preview::background(ctx, app.background_jobs);
    maintain(app, ctx);
    let Some(at) = app.ui.brush_picker else {
        close(app, ctx);
        return;
    };
    if app.discard.is_some() || app.camera_raw.is_some() || !app.ui.dialogs.is_empty() || app.distort.liquify.is_some() {
        close(app, ctx);
        return;
    }
    if state(ctx).is_none() {
        let Some(document) = app.session.active().map(|s| s.doc.id) else { return };
        open_at(app, ctx, ctx.viewport_id(), document, at);
    }
    let Some(mut s) = state(ctx) else { return };
    if app.session.active().is_none_or(|doc| doc.doc.id != s.document) || app.ui.tool != s.tool {
        close(app, ctx);
        return;
    }
    if ctx.viewport_id() != s.parent {
        return;
    }
    // ROOT can now host the options-bar picker even with only floating documents.
    // Its canvas callback and root fallback may both run in one application frame.
    if !begin_render(ctx, s.parent, app.frame) {
        return;
    }
    set_role(ctx, viewport(s.parent), true);
    // Once shown, a click in its owner is outside the picker. Ignore the opening release.
    if s.opened && ctx.input(|i| i.pointer.any_pressed() || i.key_pressed(egui::Key::Escape)) {
        close(app, ctx);
        return;
    }
    let mut dismissed = false;
    ctx.show_viewport_immediate(viewport(s.parent), builder(&s), |ui, _| {
        app.collect_screenshots(ui.ctx());
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
        if !s.opened {
            reveal(ui.ctx(), ui.ctx().viewport_id());
        }
        let (focused, closed, cancel, size, position) = ui.input(|i| {
            let v = i.viewport();
            (
                v.focused.unwrap_or(i.focused),
                v.close_requested(),
                i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter),
                v.inner_rect.filter(|r| r.is_positive() && r.is_finite()).map(|r| r.size()),
                v.outer_rect.filter(|r| r.is_finite()).map(|r| r.min),
            )
        });
        dismissed = closed || cancel || (s.focused_once && !focused);
        s.focused_once |= focused;
        if let Some(size) = size {
            s.size = bounded_size([size.x, size.y]);
            remember_size(app, s.size);
        }
        if let Some(position) = position {
            s.position = position;
        }
        let before = app.session.tools.brush.clone();
        let mut brush = before.clone();
        let mut pick = None;
        egui::CentralPanel::default().frame(egui::Frame::central_panel(ui.style()).inner_margin(10.0)).show(ui, |ui| {
            let available = ui.available_size();
            pick = brush_picker::body_sized(app, ui, &mut brush, available.x, available.y - 155.0);
        });
        if !dismissed {
            crate::brush_panel::commit_gesture(app, ui.ctx(), &before, &brush);
            dismissed |= pick == Some(brush_picker::Pick::OpenSettings);
            brush_picker::apply(app, ui.ctx(), pick);
        }
        if dismissed {
            ui.ctx().send_viewport_cmd(ViewportCommand::Visible(false));
        }
    });
    s.opened = true;
    ctx.data_mut(|d| d.insert_temp(key(), s));
    if dismissed {
        close(app, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canvas_and_options_fallback_render_the_same_picker_once_per_frame() {
        let ctx = egui::Context::default();
        assert!(begin_render(&ctx, ViewportId::ROOT, 10));
        assert!(!begin_render(&ctx, ViewportId::ROOT, 10));
        assert!(begin_render(&ctx, ViewportId::ROOT, 11));
        let child = ViewportId::from_hash_of("floating-canvas");
        assert!(begin_render(&ctx, child, 11));
        assert!(!begin_render(&ctx, child, 11));
    }

    #[test]
    fn picker_popup_role_cleanup_preserves_other_native_roles() {
        let ctx = egui::Context::default();
        let picker = viewport(ViewportId::ROOT);
        let other = ViewportId::from_hash_of("another-popup");
        set_role(&ctx, picker, true);
        set_role(&ctx, other, true);
        set_role(&ctx, picker, false);
        let roles = ctx.data(|data| data.get_temp::<std::collections::HashMap<ViewportId, u8>>(egui::Id::new("photocraft-native-window-roles"))).unwrap();
        assert_eq!(roles.get(&other), Some(&2));
        assert!(!roles.contains_key(&picker));
    }

    #[test]
    fn native_picker_builder_is_resizable_owned_and_never_topmost() {
        let picker = NativePicker {
            parent: ViewportId::from_hash_of("floating-canvas"),
            document: DocId(1),
            tool: Tool::Brush,
            position: egui::pos2(900.0, 600.0),
            size: vec2(520.0, 620.0),
            opened: false,
            focused_once: false,
        };
        let first = builder(&picker);
        assert_eq!(first.window_level, Some(egui::WindowLevel::Normal));
        assert_eq!(first.taskbar, Some(false));
        assert_eq!(first.resizable, Some(true));
        assert_eq!(first.position, Some(picker.position));
        assert_eq!(first.inner_size, Some(picker.size));
        let later = builder(&NativePicker { opened: true, ..picker });
        assert!(later.position.is_none() && later.inner_size.is_none() && later.active.is_none(), "later frames never reset native geometry or steal focus");
    }

    #[test]
    fn picker_size_round_trips_through_preferences_and_clamps_bad_dimensions() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        remember_size(&mut app, vec2(620.0, 710.0));
        let serialized = serde_json::to_string(app.session.prefs()).unwrap();
        let restored: photocraft_engine::prefs::Preferences = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored.tools.brush_picker_size, [620.0, 710.0]);
        assert_eq!(bounded_size([f32::NAN, f32::INFINITY]), vec2(324.0, 480.0));
        assert_eq!(bounded_size([-20.0, 9000.0]), vec2(260.0, 1400.0));
    }

    #[test]
    fn native_picker_uses_desktop_coordinates_and_remembers_size_on_reopen() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let document = app.session.active().unwrap().doc.id;
        let parent = ViewportId::from_hash_of("floating-canvas");
        let mut input = egui::RawInput::default();
        input.viewports.insert(
            parent,
            egui::ViewportInfo { inner_rect: Some(egui::Rect::from_min_size(egui::pos2(-800.0, 200.0), vec2(300.0, 220.0))), ..Default::default() },
        );
        ctx.run_ui(input, |ui| {
            let ctx = ui.ctx();
            open_at(&mut app, ctx, parent, document, [270.0, 190.0]);
            let mut s = state(ctx).unwrap();
            assert_eq!(s.position, egui::pos2(-530.0, 390.0), "desktop origin including negative-monitor positions");
            assert!(s.position.x + s.size.x > -500.0, "picker can extend beyond its 300-point-wide owner");
            s.size = vec2(640.0, 720.0);
            ctx.data_mut(|d| d.insert_temp(key(), s));
            close(&mut app, ctx);
            assert!(native_viewport(ctx).is_none());
            open_at(&mut app, ctx, parent, document, [270.0, 190.0]);
            assert_eq!(state(ctx).unwrap().size, vec2(640.0, 720.0));
            assert_eq!(state(ctx).unwrap().parent, parent);
        })
        .textures_delta
        .clear();
    }

    #[test]
    fn changing_tool_or_removing_source_closes_native_picker_without_changing_another_document() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let document = app.session.active().unwrap().doc.id;
        app.ui.tool = Tool::Brush;
        open_at(&mut app, &ctx, ViewportId::ROOT, document, [50.0, 50.0]);
        app.ui.tool = Tool::Move;
        maintain(&mut app, &ctx);
        assert!(state(&ctx).is_none() && app.ui.brush_picker.is_none());
        app.ui.tool = Tool::Brush;
        open_at(&mut app, &ctx, ViewportId::ROOT, document, [50.0, 50.0]);
        app.session.execute("file.close", json!({})).unwrap();
        maintain(&mut app, &ctx);
        assert!(state(&ctx).is_none());
    }

    #[test]
    fn occluding_owner_keeps_picker_open_but_minimizing_closes_it() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let document = app.session.active().unwrap().doc.id;
        for minimized in [false, true] {
            let mut input = egui::RawInput::default();
            input.viewports.insert(ViewportId::ROOT, egui::ViewportInfo { occluded: Some(true), minimized: Some(minimized), ..Default::default() });
            ctx.run_ui(input, |ui| {
                open_at(&mut app, ui.ctx(), ViewportId::ROOT, document, [50.0, 50.0]);
                maintain(&mut app, ui.ctx());
                assert_eq!(state(ui.ctx()).is_none(), minimized);
            })
            .textures_delta
            .clear();
        }
    }

    #[test]
    fn same_frame_reopen_reveals_after_the_old_hide_command() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let document = app.session.active().unwrap().doc.id;
        let id = viewport(ViewportId::ROOT);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            // Register a live native viewport without a desktop renderer. Commands alone do
            // not keep an unshown viewport alive in egui's end-of-frame output.
            ui.ctx().show_viewport_deferred(id, egui::ViewportBuilder::default(), |_, _| {});
            open_at(&mut app, ui.ctx(), ViewportId::ROOT, document, [50.0, 50.0]);
            open_at(&mut app, ui.ctx(), ViewportId::ROOT, document, [80.0, 80.0]);
            reveal(ui.ctx(), id);
        });
        output.textures_delta.clear();
        let commands = &output.viewport_output.get(&id).unwrap().commands;
        let visible: Vec<_> = commands
            .iter()
            .filter_map(|command| match command {
                ViewportCommand::Visible(value) => Some(*value),
                _ => None,
            })
            .collect();
        assert_eq!(visible, [false, true]);
        assert_eq!(commands.last(), Some(&ViewportCommand::Focus));
    }
}
