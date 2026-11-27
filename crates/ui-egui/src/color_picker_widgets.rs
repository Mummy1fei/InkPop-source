//! Deferred color-well adapter: every well uses the same independently movable dialog.
//! The original widget consumes a confirmed result only; Cancel never changes its value.

use egui::{Color32, Context, Id, Response, Sense, Stroke, StrokeKind, Ui, vec2};
use serde_json::json;

use crate::{PhotocraftApp, theme::Tokens};

const REQUEST: &str = "shared-color-picker-request-v74";
const SCOPE: &str = "shared-color-picker-scope-v74";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WidgetResult {
    pub(crate) id: u64,
    pub(crate) original: [f32; 4],
    pub(crate) color: Option<[f32; 4]>,
    pub(crate) document: Option<u64>,
    pub(crate) layer: Option<u64>,
}

#[derive(Clone, Default)]
struct Request {
    id: u64,
    original: [f32; 4],
    alpha: bool,
    none: bool,
    scope: Scope,
}

#[derive(Clone, Copy, Default, PartialEq)]
struct Scope {
    document: Option<u64>,
    layer: Option<u64>,
}

fn scope(app: &PhotocraftApp) -> Scope {
    app.session.active().map_or_else(Scope::default, |st| Scope { document: Some(st.doc.id.0), layer: st.active_layer.map(|id| id.0) })
}

fn result_id() -> Id {
    Id::new("shared-color-picker-result-v74")
}

/// Run before and after the app's widgets. Native and embedded dialogs share this bridge.
pub fn service(app: &mut PhotocraftApp, ctx: &Context) {
    let now = scope(app);
    ctx.data_mut(|data| data.insert_temp(Id::new(SCOPE), now));
    for result in std::mem::take(&mut app.ui.color_picker_results) {
        if result.document == now.document && result.layer == now.layer {
            ctx.data_mut(|data| data.insert_temp(result_id(), result));
        }
    }
    let request = ctx.data_mut(|data| data.remove_temp::<Request>(Id::new(REQUEST)));
    if let Some(request) = request {
        // One pointer can open one picker; repeated press frames must not stack duplicates.
        if app.ui.dialogs.last().is_some_and(|d| super::owns(&d.fields)) || request.scope != now {
            return;
        }
        let parent = app.ui.dialogs.last().map(|d| d.id);
        let id = super::open_widget(app, request.original, request.alpha, request.none);
        if let Some(dialog) = app.ui.dialog_mut(id) {
            dialog.fields.insert("__widget".into(), json!(request.id));
            dialog.fields.insert("__widgetParent".into(), json!(parent));
            dialog.fields.insert("__widgetOriginal".into(), json!(request.original));
            dialog.fields.insert("__widgetDocument".into(), json!(now.document));
            dialog.fields.insert("__widgetLayer".into(), json!(now.layer));
        }
        ctx.request_repaint();
    }
}

/// The existing shape swatch keeps its gradient/pattern preview and No Color action.
pub fn swatch(response: &mut Response, rgba: [f32; 4], alpha: bool, allow_none: bool) -> Option<Option<[f32; 4]>> {
    let ctx = &response.ctx;
    let id = response.id.value();
    let result =
        ctx.data_mut(|data| data.get_temp::<WidgetResult>(result_id()).filter(|r| r.id == id).and_then(|_| data.remove_temp::<WidgetResult>(result_id())));
    if response.clicked() {
        let original = rgba.map(|v| if v.is_finite() { v } else { 0.0 });
        let request = Request { id, original, alpha, none: allow_none, scope: ctx.data(|data| data.get_temp(Id::new(SCOPE))).unwrap_or_default() };
        ctx.data_mut(|data| data.insert_temp(Id::new(REQUEST), request));
        ctx.request_repaint();
    }
    let now = ctx.data(|data| data.get_temp::<Scope>(Id::new(SCOPE))).unwrap_or_default();
    result.filter(|r| r.original == rgba && r.document == now.document && r.layer == now.layer).and_then(|r| {
        if r.color == Some(r.original) {
            return None;
        }
        response.mark_changed();
        Some(r.color)
    })
}

fn well(ui: &mut Ui, rgba: [f32; 4], alpha: bool) -> (Response, Option<[f32; 4]>) {
    let t = Tokens::get(ui.ctx());
    let (rect, mut response) = ui.allocate_exact_size(vec2(32.0, 18.0), Sense::click());
    if rgba[3] < 1.0 {
        crate::widgets::checker(ui.painter(), rect, 4.0);
    }
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    ui.painter().rect_filled(rect, t.radius_sm.min(2.0), Color32::from_rgba_unmultiplied(q(rgba[0]), q(rgba[1]), q(rgba[2]), q(rgba[3])));
    ui.painter().rect_stroke(rect, t.radius_sm.min(2.0), Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ColorButton, ui.is_enabled(), tl!("Color")));
    let result = swatch(&mut response, rgba, alpha, false).flatten();
    (response, result)
}

pub fn edit_srgb(ui: &mut Ui, color: &mut [u8; 3]) -> Response {
    let rgba = [f32::from(color[0]) / 255.0, f32::from(color[1]) / 255.0, f32::from(color[2]) / 255.0, 1.0];
    let (response, result) = well(ui, rgba, false);
    if let Some(next) = result {
        *color = [next[0], next[1], next[2]].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    }
    response
}

/// egui's floating point RGB wells use linear RGB. The shared picker edits encoded sRGB.
pub fn edit_rgb(ui: &mut Ui, color: &mut [f32; 3]) -> Response {
    let encoded = color.map(photocraft_color::convert::linear_to_srgb);
    let (response, result) = well(ui, [encoded[0], encoded[1], encoded[2], 1.0], false);
    if let Some(next) = result {
        *color = [next[0], next[1], next[2]].map(photocraft_color::convert::srgb_to_linear);
    }
    response
}

pub fn edit_srgba(ui: &mut Ui, color: &mut Color32) -> Response {
    let rgba = color.to_srgba_unmultiplied().map(|v| f32::from(v) / 255.0);
    let (response, result) = well(ui, rgba, true);
    if let Some(next) = result {
        let [r, g, b, a] = next.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        *color = Color32::from_rgba_unmultiplied(r, g, b, a);
    }
    response
}

pub fn edit_rgba_unmultiplied(ui: &mut Ui, color: &mut [f32; 4]) -> Response {
    let encoded = [color[0], color[1], color[2]].map(photocraft_color::convert::linear_to_srgb);
    let (response, result) = well(ui, [encoded[0], encoded[1], encoded[2], color[3]], true);
    if let Some(next) = result {
        let linear = if [next[0], next[1], next[2]] == encoded {
            [color[0], color[1], color[2]]
        } else {
            [next[0], next[1], next[2]].map(photocraft_color::convert::srgb_to_linear)
        };
        *color = [linear[0], linear[1], linear[2], next[3]];
    }
    response
}
