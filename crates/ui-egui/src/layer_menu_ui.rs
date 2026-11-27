//! Layers panel right-click menu in Photoshop's order. Items are command ids from the menu catalog;
//! unavailable ones are greyed using the same enablement as the main menus. Items marked as menu
//! invocations (`Value::Null` params) go through `menus::invoke`, so dialogs open like in the menu bar.

use photocraft_doc::{Layer, LayerContent};
use serde_json::{Value, json};

/// One entry: label, command id. `None` = separator.
pub type Entry = Option<(&'static str, &'static str)>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ContentKind {
    Text,
    Shape,
    Smart,
    Fill,
    Other,
}

/// Menus need metadata, never raster tiles, smart-object payloads or a group's children.
/// Cloning a whole Layer here made popup preparation depend on document complexity.
struct Target {
    id: photocraft_doc::LayerId,
    name: String,
    label: photocraft_doc::LabelColor,
    clipped: bool,
    has_mask: bool,
    kind: ContentKind,
}

impl From<&Layer> for Target {
    fn from(layer: &Layer) -> Self {
        Self {
            id: layer.id,
            name: layer.name.clone(),
            label: layer.label,
            clipped: layer.clipped,
            has_mask: layer.mask.is_some(),
            kind: match &layer.content {
                LayerContent::Text(_) => ContentKind::Text,
                LayerContent::Shape(_) => ContentKind::Shape,
                LayerContent::Smart(_) => ContentKind::Smart,
                LayerContent::Fill(_) => ContentKind::Fill,
                _ => ContentKind::Other,
            },
        }
    }
}

/// The command behind "Add Layer Mask" (the panel button and this menu). Like Photoshop, an active
/// selection becomes the mask; ⌥ inverts it (Hide Selection, or Hide All without a selection).
pub fn add_mask_command(has_selection: bool, alt: bool) -> &'static str {
    match (has_selection, alt) {
        (true, false) => "layer.layerMask.revealSelection",
        (true, true) => "layer.layerMask.hideSelection",
        (false, false) => "layer.layerMask.revealAll",
        (false, true) => "layer.layerMask.hideAll",
    }
}

/// The context menu entries for a layer (Photoshop 2026 order, trimmed to the layer kind).
pub fn entries(l: &Layer, multi: bool, has_selection: bool) -> Vec<Entry> {
    target_entries(&Target::from(l), multi, has_selection)
}

fn target_entries(l: &Target, multi: bool, has_selection: bool) -> Vec<Entry> {
    let mut v: Vec<Entry> = vec![Some((tl!("Blending Options…"), "layer.layerStyle.blendingOptions"))];
    v.push(None);
    v.push(Some((if multi { tl!("Duplicate Layers…") } else { tl!("Duplicate Layer…") }, "layer.duplicate")));
    v.push(Some((if multi { tl!("Delete Layers") } else { tl!("Delete Layer") }, "layer.delete")));
    v.push(Some((tl!("Group from Layers…"), "layer.groupLayers")));
    v.push(None);
    v.push(Some((tl!("Quick Export As PNG"), "layer.quickExportAsPng")));
    v.push(Some((tl!("Export As…"), "layer.exportAs")));
    v.push(None);
    v.push(Some((tl!("Artboard from Layers…"), "layer.new.artboardFromLayers")));
    v.push(Some((tl!("Frame from Layers…"), "layer.new.frameFromLayers")));
    v.push(None);
    v.push(Some((tl!("Convert to Smart Object"), "layer.smartObjects.convertToSmartObject")));
    match l.kind {
        ContentKind::Text => v.push(Some(("Rasterize Type", "layer.rasterize.type"))),
        ContentKind::Shape => v.push(Some(("Rasterize Layer", "layer.rasterize.shape"))),
        ContentKind::Smart => {
            v.push(Some((tl!("Edit Contents"), "layer.smartObjects.editContents")));
            v.push(Some((tl!("Convert to Layers"), "layer.smartObjects.convertToLayers")));
            v.push(Some(("Rasterize Layer", "layer.rasterize.smartObject")));
        }
        ContentKind::Fill => v.push(Some(("Rasterize Layer", "layer.rasterize.fillContent"))),
        _ => v.push(Some(("Rasterize Layer", "layer.rasterize.layer"))),
    }
    v.push(None);
    if l.has_mask {
        v.push(Some((tl!("Disable Layer Mask"), "layer.layerMask.enabled")));
        v.push(Some((tl!("Apply Layer Mask"), "layer.layerMask.apply")));
        v.push(Some((tl!("Delete Layer Mask"), "layer.layerMask.delete")));
    } else {
        v.push(Some((tl!("Add Layer Mask"), add_mask_command(has_selection, false))));
    }
    v.push(Some((
        if l.clipped { tl!("Release Clipping Mask") } else { tl!("Create Clipping Mask") },
        if l.clipped { "layer.releaseClippingMask" } else { "layer.createClippingMask" },
    )));
    v.push(None);
    v.push(Some((tl!("Link Layers"), "layer.linkLayers")));
    v.push(Some((tl!("Select Linked Layers"), "layer.selectLinkedLayers")));
    v.push(None);
    v.push(Some((tl!("Copy Layer Style"), "layer.layerStyle.copyLayerStyle")));
    v.push(Some((tl!("Paste Layer Style"), "layer.layerStyle.pasteLayerStyle")));
    v.push(Some((tl!("Clear Layer Style"), "layer.layerStyle.clear")));
    v.push(None);
    if multi {
        v.push(Some((tl!("Merge Layers"), "layer.mergeLayers")));
    } else {
        v.push(Some((tl!("Merge Down"), "layer.mergeDown")));
    }
    v.push(Some((tl!("Merge Visible"), "layer.mergeVisible")));
    v.push(Some((tl!("Flatten Image"), "layer.flattenImage")));
    v
}

/// Use exactly the same filtered list for native sizing and drawing. Missing commands must
/// not leave reserved rows or orphaned separators in the popup's client area.
fn visible_entries(l: &Layer, multi: bool, has_selection: bool) -> Vec<Entry> {
    visible_target_entries(&Target::from(l), multi, has_selection)
}

fn visible_target_entries(l: &Target, multi: bool, has_selection: bool) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut separator = false;
    for entry in target_entries(l, multi, has_selection) {
        let Some((_, id)) = entry else {
            separator = !out.is_empty();
            continue;
        };
        if photocraft_engine::commands::find(id).is_none() && !crate::menu_catalog::CATALOG.iter().any(|item| item.3 == id) {
            continue;
        }
        if separator {
            out.push(None);
            separator = false;
        }
        out.push(entry);
    }
    out
}

/// Render the menu. Pushes `(command, params)` actions; `Value::Null` params mean "invoke like the
/// menu item" (opens the command's dialog when it has one).
pub fn show(app: &crate::PhotocraftApp, ui: &mut egui::Ui, l: &Layer, on_set: bool, actions: &mut Vec<(String, Value)>) -> bool {
    crate::widgets::menu_scroll(ui, |ui| contents(app, ui, l, on_set, actions, false))
}

const POPUP: &str = "layer-context";

#[derive(Clone, Copy)]
pub(crate) enum FooterMenu {
    Adjustment,
    Style,
}

impl FooterMenu {
    fn key(self) -> &'static str {
        match self {
            Self::Adjustment => "layer-adjustment-menu",
            Self::Style => "layer-style-menu",
        }
    }
}

#[derive(Clone)]
struct FooterContext {
    document: photocraft_doc::DocId,
    kind: FooterMenu,
}

enum FooterAction {
    Command(&'static str),
    Style(Option<&'static str>),
}

pub(crate) fn open_footer(app: &crate::PhotocraftApp, ctx: &egui::Context, kind: FooterMenu, at: egui::Pos2) {
    let Some(document) = app.session.active().map(|state| state.doc.id) else { return };
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("photocraft-layer-footer-menu"), FooterContext { document, kind }));
    crate::native_popup_ui::toggle(ctx, kind.key(), at, app.frame);
    ctx.request_repaint_of(egui::ViewportId::ROOT);
}

fn footer_contents(ui: &mut egui::Ui, kind: FooterMenu, action: &mut Option<FooterAction>, native: bool) {
    if native {
        ui.spacing_mut().item_spacing.y = 0.0;
    } else {
        ui.set_min_width(190.0);
    }
    let mut choose = |ui: &mut egui::Ui, label: &str, next| {
        if button(ui, label, true, native).clicked() {
            *action = Some(next);
            ui.close();
        }
    };
    match kind {
        FooterMenu::Adjustment => {
            for c in photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with("layer.newAdjustmentLayer.")) {
                choose(ui, tl!(c.label).trim_end_matches('…'), FooterAction::Command(c.id));
            }
            separator(ui, native);
            choose(ui, tl!("Solid Color…"), FooterAction::Command("layer.newFillLayer.solidColor"));
            choose(ui, tl!("Gradient…"), FooterAction::Command("layer.newFillLayer.gradient"));
        }
        FooterMenu::Style => {
            choose(ui, tl!("Blending Options…"), FooterAction::Style(None));
            separator(ui, native);
            for &(kind, label) in crate::layer_style::KINDS {
                choose(ui, &format!("{}…", tl!(label)), FooterAction::Style(Some(kind)));
            }
        }
    }
}

fn show_footer_native(app: &mut crate::PhotocraftApp, ctx: &egui::Context) {
    if ctx.embed_viewports() {
        return;
    }
    let Some(context) = ctx.data(|data| data.get_temp::<FooterContext>(egui::Id::new("photocraft-layer-footer-menu"))) else { return };
    if !crate::native_popup_ui::is_open(ctx, context.kind.key()) {
        return;
    }
    if app.session.active().is_none_or(|state| state.doc.id != context.document) {
        crate::native_popup_ui::close(ctx);
        return;
    }
    let rows = match context.kind {
        FooterMenu::Adjustment => photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with("layer.newAdjustmentLayer.")).count() + 2,
        FooterMenu::Style => crate::layer_style::KINDS.len() + 1,
    };
    let height = rows as f32 * 27.0 + 9.0 + egui::Frame::menu(&ctx.style_of(ctx.theme())).total_margin().sum().y;
    let mut action = None;
    crate::native_popup_ui::show(app, ctx, context.kind.key(), tl!("Layers"), egui::vec2(250.0, height), |_, ui| {
        footer_contents(ui, context.kind, &mut action, true);
        action.is_some()
    });
    match action {
        Some(FooterAction::Command(id)) => {
            if let Err(error) = crate::menus::invoke(app, ctx, id, json!({})) {
                app.ui.status = error;
                app.ui.status_error = true;
            }
        }
        Some(FooterAction::Style(kind)) => {
            crate::layer_style::open(app, kind);
        }
        None => {}
    }
}

#[derive(Clone)]
struct Context {
    document: photocraft_doc::DocId,
    layer: photocraft_doc::LayerId,
    on_set: bool,
    source: egui::ViewportId,
}

fn context_id() -> egui::Id {
    egui::Id::new("photocraft-layer-context")
}

#[derive(Clone)]
struct NativeLayout {
    entries: std::sync::Arc<[Entry]>,
    shortcuts: Vec<Option<String>>,
    font: egui::FontId,
    font_row_height: f32,
    leading_width: f32,
    pixels_per_point: f32,
    frame_height: f32,
    size: egui::Vec2,
}

#[cfg(test)]
fn native_layout(ctx: &egui::Context, entries: Vec<Entry>) -> NativeLayout {
    let shortcuts = vec![None; entries.len()];
    native_layout_with_shortcuts(ctx, entries, shortcuts)
}

fn command_shortcuts(app: &crate::PhotocraftApp, entries: &[Entry]) -> Vec<Option<String>> {
    entries.iter().map(|entry| entry.and_then(|(_, id)| crate::shortcuts::shortcut_label(app, id))).collect()
}

fn native_layout_with_shortcuts(ctx: &egui::Context, entries: Vec<Entry>, shortcuts: Vec<Option<String>>) -> NativeLayout {
    let id = egui::Id::new("photocraft-layer-context-layout");
    let style = ctx.style_of(ctx.theme());
    let font = egui::TextStyle::Button.resolve(&style);
    // Interface font size scales FontData, keeping FontId and viewport DPI unchanged.
    // Read the active face's cached metrics so those changes retire measured widths too.
    let font_row_height = ctx.fonts_mut(|fonts| fonts.row_height(&font));
    let leading_width = font_row_height.max(16.0) + style.spacing.icon_spacing;
    let pixels_per_point = ctx.pixels_per_point();
    let frame_height = egui::Frame::menu(&style).total_margin().sum().y;
    if let Some(cached) = ctx.data(|data| data.get_temp::<NativeLayout>(id))
        && cached.entries.as_ref() == entries.as_slice()
        && cached.shortcuts == shortcuts
        && cached.font == font
        && cached.font_row_height == font_row_height
        && cached.leading_width == leading_width
        && cached.pixels_per_point == pixels_per_point
        && cached.frame_height == frame_height
    {
        return cached;
    }
    let width = entries
        .iter()
        .zip(&shortcuts)
        .filter_map(|(entry, shortcut)| entry.map(|(label, _)| (label, shortcut)))
        .map(|(label, shortcut)| {
            let label_width = ctx.fonts_mut(|fonts| fonts.layout_no_wrap(tl!(label).to_owned(), font.clone(), egui::Color32::WHITE).size().x);
            let shortcut_width = shortcut
                .as_ref()
                .map_or(0.0, |shortcut| 24.0 + ctx.fonts_mut(|fonts| fonts.layout_no_wrap(shortcut.clone(), font.clone(), egui::Color32::WHITE).size().x));
            label_width + shortcut_width + 44.0 + leading_width
        })
        .fold(240.0f32, f32::max);
    let height = entries.iter().map(|entry| if entry.is_some() { 27.0 } else { 9.0 }).sum::<f32>() + 9.0 + 54.0 + frame_height;
    let layout = NativeLayout {
        entries: entries.into(),
        shortcuts,
        font,
        font_row_height,
        leading_width,
        pixels_per_point,
        frame_height,
        size: egui::vec2(width, height),
    };
    ctx.data_mut(|data| data.insert_temp(id, layout.clone()));
    layout
}

/// A layer row can live in ROOT or a detached tool viewport. Store the target independently
/// of that row's rendering, then draw the menu from ROOT in its own application-owned host.
pub(crate) fn open(app: &crate::PhotocraftApp, ctx: &egui::Context, layer: &Layer, on_set: bool, position: egui::Pos2) {
    let Some(document) = app.session.active().map(|state| state.doc.id) else { return };
    let source = ctx.viewport_id();
    ctx.data_mut(|data| data.insert_temp(context_id(), Context { document, layer: layer.id, on_set, source }));
    crate::native_popup_ui::open(ctx, POPUP, position, app.frame);
}

fn can_warm_layer_context(app: &crate::PhotocraftApp, ctx: &egui::Context) -> bool {
    !ctx.embed_viewports()
        && ctx.viewport_id() == egui::ViewportId::ROOT
        && app.session.active().is_some()
        && app.discard.is_none()
        && app.ui.dialogs.is_empty()
        && app.camera_raw.is_none()
        && crate::dialogs::native_dialog_viewport(app).is_none()
        && ctx.data(|data| data.get_temp::<bool>(egui::Id::new("photocraft-native-app-active"))) != Some(false)
        && !ctx.input(|input| input.viewport().minimized.unwrap_or(false))
}

pub(crate) fn show_native(app: &mut crate::PhotocraftApp, ctx: &egui::Context) {
    show_footer_native(app, ctx);
    if can_warm_layer_context(app, ctx) {
        crate::native_popup_ui::warm_layer_context(ctx);
    }
    if ctx.embed_viewports() || !crate::native_popup_ui::is_open(ctx, POPUP) {
        return;
    }
    let context = ctx.data(|data| data.get_temp::<Context>(context_id()));
    let target = context.and_then(|context| {
        let document = app.session.active().filter(|state| state.doc.id == context.document)?;
        Some((Target::from(document.doc.layer(context.layer)?), context.on_set, context.source))
    });
    let Some((layer, on_set, source)) = target else {
        crate::native_popup_ui::close(ctx);
        return;
    };
    let mut actions = Vec::new();
    let mut rename = false;
    let list = visible_target_entries(&layer, on_set, app.session.active().is_some_and(|state| state.doc.selection.is_some()));
    let shortcuts = command_shortcuts(app, &list);
    let layout = native_layout_with_shortcuts(ctx, list, shortcuts);
    // Fixed row/separator sizes below keep the host snug when every command fits, and the
    // shared popup's scroll area keeps the final commands reachable on a short display.
    crate::native_popup_ui::show(app, ctx, POPUP, tl!("Layers"), layout.size, |app, ui| {
        rename = target_contents(app, ui, &layer, on_set, &mut actions, true, &layout.entries);
        rename || !actions.is_empty()
    });
    crate::native_popup_ui::show_child(app, ctx, POPUP, tl!("Layer Color"), |_, ui| {
        target_colors(ui, &layer, on_set, &mut actions, true);
        !actions.is_empty()
    });
    // Dispatch only after returning to ROOT. A command may open a settings dialog or a
    // document; using the tiny popup's context would parent it to the wrong native window.
    if rename {
        ctx.send_viewport_cmd_to(source, egui::ViewportCommand::Focus);
        ctx.request_repaint_of(source);
        if let Some(done) = crate::layer_row_ui::start_rename(ctx, layer.id.0, &layer.name) {
            actions.push(done);
        }
    }
    for (id, params) in actions {
        let result = if params.is_null() { crate::menus::invoke(app, ctx, &id, json!({})) } else { app.run(&id, params) };
        if let Err(error) = result {
            app.ui.status = error;
            app.ui.status_error = true;
            break;
        }
    }
}

fn button(ui: &mut egui::Ui, label: &str, enabled: bool, native: bool) -> egui::Response {
    button_with_shortcut(ui, label, enabled, native, None)
}

fn button_with_shortcut(ui: &mut egui::Ui, label: &str, enabled: bool, native: bool, shortcut: Option<String>) -> egui::Response {
    let mut button = crate::menus::row_button(ui, label, false);
    if let Some(shortcut) = shortcut {
        button = button.shortcut_text(shortcut);
    }
    if native {
        // A trailing grow atom aligns the label without adding empty shortcut text to
        // the accessible name (an empty text atom would append a spurious space).
        ui.add_enabled_ui(enabled, |ui| ui.add_sized(egui::vec2(ui.available_width(), 27.0), button.frame(true).frame_when_inactive(false))).inner
    } else {
        ui.add_enabled(enabled, button)
    }
}

fn separator(ui: &mut egui::Ui, native: bool) {
    if native {
        ui.add(egui::Separator::default().spacing(9.0));
    } else {
        ui.separator();
    }
}

fn contents(app: &crate::PhotocraftApp, ui: &mut egui::Ui, l: &Layer, on_set: bool, actions: &mut Vec<(String, Value)>, native: bool) -> bool {
    let entries = visible_entries(l, on_set, app.session.active().is_some_and(|s| s.doc.selection.is_some()));
    target_contents(app, ui, &Target::from(l), on_set, actions, native, &entries)
}

fn target_contents(
    app: &crate::PhotocraftApp,
    ui: &mut egui::Ui,
    l: &Target,
    on_set: bool,
    actions: &mut Vec<(String, Value)>,
    native: bool,
    entries: &[Entry],
) -> bool {
    ui.set_min_width(if native { 0.0 } else { 220.0 });
    let accepts_actions = !native || crate::native_popup_ui::accepts_actions(ui.ctx());
    // A native context menu owns its own keyboard input. Collect its command just like a
    // click; show_native invokes it after returning to ROOT, with the same layer target.
    if accepts_actions && let Some(id) = take_menu_shortcut(app, ui.ctx(), entries) {
        let is_active = app.session.active().is_some_and(|state| state.active_layer == Some(l.id));
        if (!on_set && !is_active) || crate::menus::is_enabled(app, &id) {
            if !on_set {
                actions.push(("layer.select".into(), json!({"layer": l.id.0})));
            }
            actions.push((id, Value::Null));
            ui.close();
            return false;
        }
    }
    if native {
        ui.spacing_mut().item_spacing.y = 0.0;
    }
    let mut rename = false;
    for &e in entries {
        match e {
            None => separator(ui, native),
            Some((label, id)) => {
                let is_active = app.session.active().is_some_and(|s| s.active_layer == Some(l.id));
                let enabled = if on_set || is_active { crate::menus::is_enabled(app, id) } else { true };
                let response = button_with_shortcut(ui, tl!(&label), enabled, native, crate::shortcuts::shortcut_label(app, id));
                if native && accepts_actions && response.hovered() {
                    crate::native_popup_ui::close_child(ui.ctx());
                }
                if accepts_actions && response.clicked() {
                    if !on_set {
                        actions.push(("layer.select".into(), json!({"layer": l.id.0})));
                    }
                    actions.push((id.into(), Value::Null));
                    ui.close();
                }
            }
        }
    }
    separator(ui, native);
    let response = button(ui, tl!("Rename Layer…"), true, native);
    if native && accepts_actions && response.hovered() {
        crate::native_popup_ui::close_child(ui.ctx());
    }
    if accepts_actions && response.clicked() {
        rename = true;
        ui.close();
    }
    if native {
        let response = ui.add_sized(
            egui::vec2(ui.available_width(), 27.0),
            crate::menus::row_button(ui, tl!("Layer Color"), false).shortcut_text("▶").frame(true).frame_when_inactive(false),
        );
        if accepts_actions && (response.hovered() || response.clicked() || (response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::ArrowRight))))
        {
            let frame_height = egui::Frame::menu(ui.style()).total_margin().sum().y;
            crate::native_popup_ui::open_child(ui.ctx(), POPUP, "color", response.rect, egui::vec2(180.0, 8.0 * 27.0 + frame_height));
        }
    } else {
        crate::menus::submenu_button(ui, tl!("Layer Color"), |ui| target_colors(ui, l, on_set, actions, false));
    }
    rename
}

fn take_menu_shortcut(app: &crate::PhotocraftApp, ctx: &egui::Context, entries: &[Entry]) -> Option<String> {
    if !ctx.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Key { pressed: true, .. }))) {
        return None;
    }
    let focus = crate::shortcut_dispatch::Focus::of(ctx);
    let bindings = crate::shortcut_dispatch::bindings(app);
    ctx.input_mut(|input| {
        let (at, id) = input.events.iter().enumerate().find_map(|(at, event)| {
            let egui::Event::Key { key, pressed: true, modifiers, .. } = event else { return None };
            let (id, _) = bindings.iter().find(|(id, shortcut)| {
                entries.iter().flatten().any(|(_, entry)| *entry == id.as_str())
                    && focus.allows(shortcut)
                    && crate::shortcuts::key_matches(shortcut, *key, *modifiers)
            })?;
            Some((at, id.clone()))
        })?;
        input.events.remove(at);
        Some(id)
    })
}

#[cfg(test)]
fn colors(ui: &mut egui::Ui, l: &Layer, on_set: bool, actions: &mut Vec<(String, Value)>, native: bool) {
    target_colors(ui, &Target::from(l), on_set, actions, native);
}

fn target_colors(ui: &mut egui::Ui, l: &Target, on_set: bool, actions: &mut Vec<(String, Value)>, native: bool) {
    let accepts_actions = !native || crate::native_popup_ui::accepts_actions(ui.ctx());
    if native {
        ui.spacing_mut().item_spacing.y = 0.0;
    }
    for (name, label, title) in [
        ("none", photocraft_doc::LabelColor::None, "No Color"),
        ("red", photocraft_doc::LabelColor::Red, "Red"),
        ("orange", photocraft_doc::LabelColor::Orange, "Orange"),
        ("yellow", photocraft_doc::LabelColor::Yellow, "Yellow"),
        ("green", photocraft_doc::LabelColor::Green, "Green"),
        ("blue", photocraft_doc::LabelColor::Blue, "Blue"),
        ("violet", photocraft_doc::LabelColor::Violet, "Violet"),
        ("gray", photocraft_doc::LabelColor::Gray, "Gray"),
    ] {
        let response = if native {
            ui.add_sized(
                egui::vec2(ui.available_width(), 27.0),
                crate::menus::row_button(ui, tl!(title), l.label == label).selected(l.label == label).frame(true).frame_when_inactive(l.label == label),
            )
        } else {
            ui.add(crate::menus::row_button(ui, tl!(title), l.label == label).selected(l.label == label))
        };
        if accepts_actions && response.clicked() {
            if !on_set {
                actions.push(("layer.select".into(), json!({"layer":l.id.0})));
            }
            actions.push(("layer.setLabel".into(), json!({"color":name})));
            ui.close();
        }
    }
}

#[cfg(test)]
#[path = "layer_menu_latency_tests.rs"]
mod latency_tests;

#[cfg(test)]
#[path = "layer_menu_shortcut_tests_v70.rs"]
mod shortcut_tests_v70;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_adjustment_and_style_actions_are_selectable_in_native_hosts() {
        use egui_kittest::{Harness, kittest::Queryable};
        for (kind, label) in [(FooterMenu::Adjustment, "Curves"), (FooterMenu::Style, "Blending Options…")] {
            let mut harness = Harness::builder()
                .with_size(egui::vec2(300.0, 1000.0))
                .build_ui_state(|ui, action: &mut Option<FooterAction>| footer_contents(ui, kind, action, true), None);
            harness.get_by_label(label).click();
            harness.run_steps(3);
            match harness.state() {
                Some(FooterAction::Command(id)) => assert_eq!(*id, "layer.newAdjustmentLayer.curves"),
                Some(FooterAction::Style(effect)) => assert!(effect.is_none()),
                None => panic!("the clicked footer action must be returned to ROOT"),
            }
        }
    }

    #[test]
    fn footer_menu_uses_source_viewport_origin_and_dismisses_on_document_change() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":16,"height":16})).unwrap();
        let source = egui::ViewportId::from_hash_of("floating-layer-panel");
        let mut input = egui::RawInput { viewport_id: source, ..Default::default() };
        input.viewports.insert(
            source,
            egui::ViewportInfo {
                inner_rect: Some(egui::Rect::from_min_size(egui::pos2(-800.0, 120.0), egui::vec2(280.0, 700.0))),
                monitor_size: Some(egui::vec2(1600.0, 900.0)),
                ..Default::default()
            },
        );
        ctx.run_ui(input, |ui| open_footer(&app, ui.ctx(), FooterMenu::Adjustment, egui::pos2(80.0, 650.0))).textures_delta.clear();
        assert!(crate::native_popup_ui::is_open(&ctx, FooterMenu::Adjustment.key()));
        let id = crate::native_popup_ui::active_viewport(&ctx).unwrap();
        assert_ne!(id, source, "the menu needs its own host so floating panels cannot occlude it");
        app.run("file.new", json!({"width":16,"height":16})).unwrap();
        show_footer_native(&mut app, &ctx);
        assert!(!crate::native_popup_ui::is_open(&ctx, FooterMenu::Adjustment.key()));
    }

    #[test]
    fn native_layer_rows_have_hover_feedback_without_idle_button_boxes() {
        fn has_fill(shapes: &[egui::epaint::ClippedShape], fill: egui::Color32) -> bool {
            fn matches(shape: &egui::Shape, fill: egui::Color32) -> bool {
                match shape {
                    egui::Shape::Rect(rect) => rect.fill == fill,
                    egui::Shape::Vec(shapes) => shapes.iter().any(|s| matches(s, fill)),
                    _ => false,
                }
            }
            shapes.iter().any(|shape| matches(&shape.shape, fill))
        }
        for theme in crate::theme::ThemeKind::ALL {
            let ctx = egui::Context::default();
            crate::PhotocraftApp::setup_context(&ctx, theme);
            let mut row = egui::Rect::NOTHING;
            for hover in [false, true, false] {
                let mut filled = false;
                for _ in 0..4 {
                    let events = if hover { vec![egui::Event::PointerMoved(row.center())] } else { vec![egui::Event::PointerGone] };
                    let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                        row = button(ui, "Delete Layer", true, true).rect;
                    });
                    output.textures_delta.clear();
                    filled = has_fill(&output.shapes, ctx.style_of(ctx.theme()).visuals.widgets.hovered.weak_bg_fill);
                }
                assert_eq!(filled, hover, "{theme:?}: hover feedback must follow the pointer");
            }
        }
    }

    #[test]
    fn native_context_actions_keep_the_selection_or_target_the_clicked_row() {
        use egui_kittest::{Harness, kittest::Queryable};
        for on_set in [false, true] {
            let mut session = photocraft_engine::Session::new();
            session.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
            let first = session.execute("layer.new.layer", json!({"name": "First"})).unwrap()["layer"].as_u64().unwrap();
            session.execute("layer.new.layer", json!({"name": "Second"})).unwrap();
            session.execute("layer.select", json!({"layer":first, "mode":"add"})).unwrap();
            let app = crate::PhotocraftApp::new(session, crate::Services::default());
            let layer = app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(first)).unwrap().clone();
            let mut harness = Harness::builder().with_size(egui::vec2(500.0, 1200.0)).build_ui_state(
                |ui, state: &mut (crate::PhotocraftApp, Layer, Vec<(String, Value)>)| {
                    contents(&state.0, ui, &state.1, on_set, &mut state.2, true);
                },
                (app, layer, Vec::<(String, Value)>::new()),
            );
            harness.get_by_label(if on_set { "Delete Layers" } else { "Delete Layer" }).click();
            harness.run_steps(3);
            let actions = &harness.state().2;
            if on_set {
                assert_eq!(actions, &[("layer.delete".into(), Value::Null)], "a multi-selection must not be reduced to one layer");
            } else {
                assert_eq!(actions, &[("layer.select".into(), json!({"layer":first})), ("layer.delete".into(), Value::Null)]);
            }
            let actions = std::mem::take(&mut harness.state_mut().2);
            for (id, params) in actions {
                harness.state_mut().0.run(&id, if params.is_null() { json!({}) } else { params }).unwrap();
            }
            assert_eq!(harness.state().0.session.active().unwrap().doc.layers.len(), if on_set { 1 } else { 2 });
        }
    }

    #[test]
    fn native_color_flyout_keeps_multi_selection_and_rename_remains_available() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut session = photocraft_engine::Session::new();
        session.execute("file.new", json!({"width":16, "height":16})).unwrap();
        let app = crate::PhotocraftApp::new(session, crate::Services::default());
        let layer = app.session.active().unwrap().doc.layers[0].clone();
        let mut harness = Harness::builder().with_size(egui::vec2(280.0, 260.0)).build_ui_state(
            |ui, state: &mut (Layer, Vec<(String, Value)>)| colors(ui, &state.0, true, &mut state.1, true),
            (layer, Vec::<(String, Value)>::new()),
        );
        harness.get_by_label("Red").click();
        harness.run_steps(3);
        assert_eq!(harness.state().1, [("layer.setLabel".into(), json!({"color":"red"}))]);
        let layer = harness.state().0.clone();
        let mut harness = Harness::builder().with_size(egui::vec2(400.0, 1200.0)).build_ui_state(
            |ui, renamed: &mut bool| {
                *renamed |= contents(&app, ui, &layer, false, &mut Vec::new(), true);
            },
            false,
        );
        harness.get_by_label("Rename Layer…").click();
        harness.run_steps(3);
        assert!(*harness.state());
    }

    #[test]
    fn detached_context_keeps_its_source_for_inline_rename() {
        let ctx = egui::Context::default();
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":16,"height":16})).unwrap();
        let layer = app.session.active().unwrap().doc.layers[0].clone();
        let source = egui::ViewportId::from_hash_of("detached-layers");
        let mut input = egui::RawInput { viewport_id: source, ..Default::default() };
        input.viewports.entry(source).or_default();
        let mut output = ctx.run_ui(input, |ui| open(&app, ui.ctx(), &layer, false, egui::pos2(100.0, 100.0)));
        output.textures_delta.clear();
        let context = ctx.data(|data| data.get_temp::<Context>(context_id())).unwrap();
        assert_eq!(context.source, source, "the rename field belongs to the originating Layers window");
        assert_ne!(context.source, egui::ViewportId::ROOT);
    }

    #[test]
    fn native_layer_context_closes_when_its_document_is_no_longer_active() {
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":16,"height":16})).unwrap();
        let layer = app.session.active().unwrap().doc.layers[0].clone();
        open(&app, &ctx, &layer, false, egui::pos2(100.0, 100.0));
        assert!(crate::native_popup_ui::is_open(&ctx, POPUP));
        app.run("file.new", json!({"width":16,"height":16})).unwrap();
        show_native(&mut app, &ctx);
        assert!(!crate::native_popup_ui::is_open(&ctx, POPUP));
    }

    #[test]
    fn group_from_layers_is_available_for_multiple_selected_rows() {
        let layer = Layer::group("Group", vec![]);
        assert!(entries(&layer, true, false).iter().flatten().any(|entry| entry.1 == "layer.groupLayers"));
    }

    #[test]
    fn smart_object_context_menu_edits_the_clicked_layer() {
        use crate::PhotocraftApp;
        use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
        use egui_kittest::{Harness, kittest::Queryable};

        fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, button: PointerButton) {
            h.hover_at(at);
            h.step();
            for pressed in [true, false] {
                h.event(Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE });
                h.step();
            }
            h.run_steps(3);
        }

        for ppp in [1.0, 2.0] {
            let mut s = photocraft_engine::Session::new();
            s.execute("file.new", json!({"width": 640, "height": 480})).unwrap();
            s.execute("layer.new.layer", json!({"name": "Smart source"})).unwrap();
            s.execute("paint.stroke", json!({"points": [[100, 100]], "size": 20})).unwrap();
            let smart = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
            s.execute("layer.new.layer", json!({"name": "Other active layer"})).unwrap();
            let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_pixels_per_point(ppp).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(
                move |cc| {
                    PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                    PhotocraftApp::new(s, crate::Services::default())
                },
            );
            let ctx = h.ctx.clone();
            let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
            crate::control::handle(h.state_mut(), &ctx, &req);
            h.run_steps(8);
            let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == smart).expect("layer row drawn").row;
            // Top-level Pro row: 6 pt padding, 28 pt eye column, 24 pt layer thumbnail.
            let at = pos2(row.left() + 6.0 + 28.0 + 12.0, row.center().y);
            click(&mut h, at, PointerButton::Secondary);
            let at = h.get_by_label("Edit Contents").rect().center();
            click(&mut h, at, PointerButton::Primary);
            assert_eq!(h.state().session.active_index(), Some(1), "@{ppp}x: contents tab opens");
            assert!(h.state().session.is_enabled("layer.smartObjects.saveContents"));
            assert!(h.state().session.active().unwrap().doc.walk().iter().any(|(_, _, l)| l.name == "Smart source"));
            h.state_mut().session.set_active(0);
            assert_eq!(h.state().session.active().unwrap().active_layer, Some(photocraft_doc::LayerId(smart)));
        }
    }

    #[test]
    fn a_long_layer_menu_stays_inside_a_short_window_and_scrolls() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
        let app = crate::PhotocraftApp::new(s, crate::Services::default());
        let layer = {
            let st = app.session.active().unwrap();
            st.doc.layer(st.active_layer.unwrap()).unwrap().clone()
        };
        let ctx = egui::Context::default();
        crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 300.0));
        let id = egui::Id::new("layer-menu-probe");
        let mut content = 0.0;
        for _ in 0..4 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                // Opened low in the window, like a right-click on a bottom Layers row.
                egui::Area::new(id).order(egui::Order::Foreground).default_pos(egui::pos2(300.0, 250.0)).show(ui.ctx(), |ui| {
                    egui::Frame::menu(ui.style()).show(ui, |ui| {
                        let start = ui.next_widget_position().y;
                        show(&app, ui, &layer, false, &mut Vec::new());
                        content = ui.min_rect().bottom() - start;
                    });
                });
            });
            out.textures_delta.clear();
        }
        let rect = ctx.memory(|m| m.area_rect(id)).expect("the menu was shown");
        assert!(screen.contains_rect(rect), "the menu moves up and stays inside the window: {rect:?}");
        assert!(rect.height() < screen.height(), "{rect:?}");
        let rows = entries(&layer, false, false).iter().flatten().count();
        assert!(rows >= 15, "Photoshop's layer menu is long: {rows} rows");
        assert!(content < rect.height() + 1.0, "the rows scroll inside the menu instead of running off the window");
    }

    #[test]
    fn edit_contents_is_available_only_for_smart_objects() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Source"})).unwrap();
        let layer = |s: &photocraft_engine::Session| {
            let st = s.active().unwrap();
            st.doc.layer(st.active_layer.unwrap()).unwrap().clone()
        };
        assert!(!entries(&layer(&s), false, false).iter().flatten().any(|e| e.1 == "layer.smartObjects.editContents"));
        s.execute("paint.stroke", json!({"points": [[12, 12]], "size": 8})).unwrap();
        s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        let ids: Vec<_> = entries(&layer(&s), false, false).into_iter().flatten().map(|e| e.1).collect();
        let edit = ids.iter().position(|id| *id == "layer.smartObjects.editContents").expect("Edit Contents entry");
        assert_eq!(ids[edit + 1..edit + 3], ["layer.smartObjects.convertToLayers", "layer.rasterize.smartObject"]);
    }

    #[test]
    fn entries_follow_layer_state() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let st = s.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
        let ids: Vec<&str> = entries(&l, false, false).into_iter().flatten().map(|e| e.1).collect();
        assert_eq!(ids.first(), Some(&"layer.layerStyle.blendingOptions"));
        assert!(ids.contains(&"layer.mergeDown") && !ids.contains(&"layer.mergeLayers"));
        assert!(ids.contains(&"layer.layerMask.revealAll"));
        let ids: Vec<&str> = entries(&l, false, true).into_iter().flatten().map(|e| e.1).collect();
        assert!(ids.contains(&"layer.layerMask.revealSelection") && !ids.contains(&"layer.layerMask.revealAll"));
        let ids: Vec<&str> = entries(&l, true, false).into_iter().flatten().map(|e| e.1).collect();
        assert!(ids.contains(&"layer.mergeLayers"));
    }

    #[test]
    fn add_mask_uses_the_selection() {
        assert_eq!(add_mask_command(false, false), "layer.layerMask.revealAll");
        assert_eq!(add_mask_command(false, true), "layer.layerMask.hideAll");
        assert_eq!(add_mask_command(true, true), "layer.layerMask.hideSelection");
        // With a selection the new mask is the selection: inside revealed, outside hidden.
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 10})).unwrap();
        s.execute(add_mask_command(true, false), json!({})).unwrap();
        let st = s.active().unwrap();
        let mask = &st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface;
        assert!(mask.pixel(1, 5)[0] > 0.99 && mask.pixel(8, 5)[0] < 0.01);
    }

    #[test]
    fn every_entry_is_a_known_command() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
        let l = s.active().unwrap().doc.layers[0].clone();
        for (_, id) in entries(&l, false, true).into_iter().chain(entries(&l, false, false)).flatten() {
            assert!(crate::menu_catalog::CATALOG.iter().any(|m| m.3 == id) || photocraft_engine::commands::find(id).is_some(), "{id}");
        }
    }
}
