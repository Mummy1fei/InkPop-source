//! Local-mask editing in the same normalized coordinates used by final rendering.
use crate::theme::palette::ScrollAreaPaletteExt;
use egui::{Color32, Pos2, Rect, Response, TextureHandle, pos2, vec2};
use photocraft_algo::camera_raw_masks::{
    BrushStroke, LocalAdjustment, MAX_COMPONENTS, MAX_MASKS, MAX_POINTS, MAX_TOTAL_SHAPES, MaskComponent, MaskShape, PreparedLocalMask,
};
use photocraft_engine::camera_raw_mask_cmds::{EditResult, MaskOperation};

type GeneratedMask = Result<(MaskShape, &'static str, bool, serde_json::Value), String>;

struct RecognitionJob(photocraft_engine::jobs::JobCtx);
impl Drop for RecognitionJob {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

// A dialog can be closed or a preset loaded while selection is still computing.
// The worker owns this permit, so dropping UI state cannot start overlapping work.
static AUTO_MASK_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
struct WorkerPermit(&'static std::sync::atomic::AtomicBool);
impl WorkerPermit {
    fn acquire(busy: &'static std::sync::atomic::AtomicBool) -> Option<Self> {
        busy.compare_exchange(false, true, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire).ok().map(|_| Self(busy))
    }
}
impl Drop for WorkerPermit {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handle {
    Move,
    RadiusX,
    RadiusY,
    Rotate,
    LinearStart,
    LinearEnd,
    Draw,
}
struct GeometryDrag {
    handle: Handle,
    origin: [f32; 2],
    original: MaskShape,
    aspect: [f32; 2],
}
#[derive(Clone, Copy)]
enum CreationTarget {
    Group,
    Component { group: usize, subtract: bool },
    Intersection { group: usize },
}
/// An armed gradient is view state until a nonempty drag is released. In
/// particular, its template must never enter CameraRaw::local_adjustments.
struct CreationDraft {
    template: MaskShape,
    label: &'static str,
    target: CreationTarget,
    drag: Option<GeometryDrag>,
    preview: Option<MaskShape>,
    previous_edit: bool,
}
fn screen(rect: Rect, uv: [f32; 2]) -> Pos2 {
    rect.min + rect.size() * vec2(uv[0], uv[1])
}
fn normalized(rect: Rect, p: Pos2) -> [f32; 2] {
    let q = (p - rect.min) / rect.size().max(vec2(1.0, 1.0));
    [q.x, q.y]
}
fn image_aspect(rect: Rect) -> [f32; 2] {
    let short = rect.width().min(rect.height()).max(1.0);
    [rect.width().max(1.0) / short, rect.height().max(1.0) / short]
}
fn radial_point(center: [f32; 2], local: [f32; 2], rotation: f32, aspect: [f32; 2]) -> [f32; 2] {
    let (sin, cos) = rotation.to_radians().sin_cos();
    [center[0] + (local[0] * cos - local[1] * sin) / aspect[0], center[1] + (local[0] * sin + local[1] * cos) / aspect[1]]
}
fn radial_local(center: [f32; 2], uv: [f32; 2], rotation: f32, aspect: [f32; 2]) -> [f32; 2] {
    let (sin, cos) = rotation.to_radians().sin_cos();
    let q = [(uv[0] - center[0]) * aspect[0], (uv[1] - center[1]) * aspect[1]];
    [q[0] * cos + q[1] * sin, -q[0] * sin + q[1] * cos]
}
/// Distance to an ellipse in screen points. A radial UV distance is not a
/// perpendicular distance, so it makes the outside target thin on long ellipses.
fn ellipse_edge_distance(q: [f32; 2], radius: [f32; 2]) -> f32 {
    let q = [q[0].abs(), q[1].abs()];
    let distance_sq = |angle: f32| {
        let x = radius[0] * angle.cos() - q[0];
        let y = radius[1] * angle.sin() - q[1];
        x * x + y * y
    };
    const STEPS: usize = 16;
    let step = std::f32::consts::FRAC_PI_2 / STEPS as f32;
    let best = (0..=STEPS).min_by(|a, b| distance_sq(*a as f32 * step).total_cmp(&distance_sq(*b as f32 * step))).unwrap_or(0);
    let mut lo = best.saturating_sub(1) as f32 * step;
    let mut hi = (best + 1).min(STEPS) as f32 * step;
    for _ in 0..24 {
        let left = (2.0 * lo + hi) / 3.0;
        let right = (lo + 2.0 * hi) / 3.0;
        if distance_sq(left) <= distance_sq(right) {
            hi = right;
        } else {
            lo = left;
        }
    }
    distance_sq((lo + hi) * 0.5).sqrt()
}
fn handles(mask: &MaskShape, aspect: [f32; 2]) -> Vec<(Handle, [f32; 2])> {
    match mask {
        MaskShape::Radial { center, radius, rotation, .. } => {
            let [rx, ry] = [radius[0] * aspect[0], radius[1] * aspect[1]];
            vec![
                (Handle::Move, *center),
                (Handle::RadiusX, radial_point(*center, [-rx, 0.0], *rotation, aspect)),
                (Handle::RadiusX, radial_point(*center, [rx, 0.0], *rotation, aspect)),
                (Handle::RadiusY, radial_point(*center, [0.0, -ry], *rotation, aspect)),
                (Handle::RadiusY, radial_point(*center, [0.0, ry], *rotation, aspect)),
            ]
        }
        MaskShape::Linear { start, end } => {
            vec![(Handle::LinearStart, *start), (Handle::LinearEnd, *end), (Handle::Move, [(start[0] + end[0]) * 0.5, (start[1] + end[1]) * 0.5])]
        }
        _ => Vec::new(),
    }
}
fn hit_handle(mask: &MaskShape, rect: Rect, p: Pos2) -> Option<Handle> {
    let aspect = image_aspect(rect);
    if let Some((handle, _)) = handles(mask, aspect)
        .into_iter()
        .filter(|(_, uv)| screen(rect, *uv).distance(p) <= 10.0)
        .min_by(|(_, a), (_, b)| screen(rect, *a).distance_sq(p).total_cmp(&screen(rect, *b).distance_sq(p)))
    {
        return Some(handle);
    }
    let uv = normalized(rect, p);
    match mask {
        MaskShape::Radial { center, radius, rotation, .. } => {
            let local = radial_local(*center, uv, *rotation, aspect);
            let distance = (local[0] / (radius[0] * aspect[0]).max(0.001)).hypot(local[1] / (radius[1] * aspect[1]).max(0.001));
            if distance < 1.0 {
                Some(Handle::Move)
            } else {
                let short = rect.width().min(rect.height());
                let outside = ellipse_edge_distance([local[0] * short, local[1] * short], [radius[0] * rect.width(), radius[1] * rect.height()]);
                // Leave the outline itself free. The 21 pt band is deliberately
                // outside it and has a stable size at every image zoom/DPI.
                (3.0..=24.0).contains(&outside).then_some(Handle::Rotate)
            }
        }
        MaskShape::Linear { start, end } => {
            let start = screen(rect, *start);
            let end = screen(rect, *end);
            let delta = end - start;
            let t = ((p - start).dot(delta) / delta.length_sq().max(1.0)).clamp(0.0, 1.0);
            (p.distance(start + delta * t) <= 8.0).then_some(Handle::Move)
        }
        _ => None,
    }
}
fn update_geometry(mask: &mut MaskShape, drag: &GeometryDrag, point: [f32; 2]) -> bool {
    if point.iter().any(|v| !v.is_finite()) {
        return false;
    }
    let p = point.map(|v| v.clamp(0.0, 1.0));
    let delta = [point[0] - drag.origin[0], point[1] - drag.origin[1]];
    let mut next = drag.original.clone();
    match &mut next {
        MaskShape::Radial { center, radius, rotation, .. } => match drag.handle {
            Handle::Draw => {
                *center = drag.origin.map(|v| v.clamp(0.0, 1.0));
                *radius = [(p[0] - center[0]).abs().clamp(0.001, 1.0), (p[1] - center[1]).abs().clamp(0.001, 1.0)];
            }
            Handle::Move => *center = [(center[0] + delta[0]).clamp(0.0, 1.0), (center[1] + delta[1]).clamp(0.0, 1.0)],
            Handle::RadiusX => radius[0] = (radial_local(*center, point, *rotation, drag.aspect)[0] / drag.aspect[0]).abs().clamp(0.001, 1.0),
            Handle::RadiusY => radius[1] = (radial_local(*center, point, *rotation, drag.aspect)[1] / drag.aspect[1]).abs().clamp(0.001, 1.0),
            Handle::Rotate => {
                let a = [(drag.origin[0] - center[0]) * drag.aspect[0], (drag.origin[1] - center[1]) * drag.aspect[1]];
                let b = [(point[0] - center[0]) * drag.aspect[0], (point[1] - center[1]) * drag.aspect[1]];
                if b[0].hypot(b[1]) < 1e-6 {
                    return false;
                }
                let delta = (b[1].atan2(b[0]) - a[1].atan2(a[0]) + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                *rotation = (*rotation + delta.to_degrees() + 180.0).rem_euclid(360.0) - 180.0;
            }
            _ => return false,
        },
        MaskShape::Linear { start, end } => match drag.handle {
            Handle::Draw => {
                *start = drag.origin.map(|v| v.clamp(0.0, 1.0));
                *end = p;
            }
            Handle::LinearStart => *start = p,
            Handle::LinearEnd => *end = p,
            Handle::Move => {
                for axis in 0..2 {
                    let d = delta[axis].clamp(-start[axis].min(end[axis]), 1.0 - start[axis].max(end[axis]));
                    start[axis] += d;
                    end[axis] += d;
                }
            }
            _ => return false,
        },
        _ => return false,
    }
    if matches!(&next, MaskShape::Linear { start, end } if start == end) || *mask == next {
        return false;
    }
    *mask = next;
    true
}
fn draw_geometry(ui: &egui::Ui, mask: &MaskShape, rect: Rect) {
    let painter = ui.painter();
    let outline = |points: Vec<Pos2>, closed| {
        for stroke in [egui::Stroke::new(2.5, Color32::from_black_alpha(180)), egui::Stroke::new(1.0, Color32::WHITE)] {
            painter.add(if closed { egui::Shape::closed_line(points.clone(), stroke) } else { egui::Shape::line(points.clone(), stroke) });
        }
    };
    match mask {
        MaskShape::Radial { center, radius, feather, rotation } => {
            let aspect = image_aspect(rect);
            for scale in [1.0, (1.0 - feather).max(0.001)] {
                let points = (0..96)
                    .map(|i| {
                        let angle = i as f32 * std::f32::consts::TAU / 96.0;
                        screen(
                            rect,
                            radial_point(
                                *center,
                                [radius[0] * aspect[0] * scale * angle.cos(), radius[1] * aspect[1] * scale * angle.sin()],
                                *rotation,
                                aspect,
                            ),
                        )
                    })
                    .collect();
                outline(points, true);
            }
        }
        MaskShape::Linear { start, end } => outline(vec![screen(rect, *start), screen(rect, *end)], false),
        _ => return,
    }
    let accent = crate::theme::Tokens::get(ui.ctx()).accent;
    for (handle, uv) in handles(mask, image_aspect(rect)) {
        let p = screen(rect, uv);
        painter.circle_filled(p, if handle == Handle::Move { 5.5 } else { 4.0 }, accent);
        painter.circle_stroke(p, if handle == Handle::Move { 5.5 } else { 4.0 }, egui::Stroke::new(1.5, Color32::WHITE));
        painter.circle_stroke(p, if handle == Handle::Move { 7.0 } else { 5.0 }, egui::Stroke::new(0.75, Color32::from_black_alpha(160)));
    }
}

#[derive(Clone, Copy)]
struct ListRow {
    group: usize,
    component: Option<usize>,
    primary: bool,
    rect: Rect,
    name_rect: Rect,
    checkbox_rect: Rect,
    delete_rect: Option<Rect>,
    thumbnail_rect: Option<Rect>,
    menu_rect: Rect,
    menu_visible: bool,
    eye_visible: bool,
}

struct Rename {
    index: usize,
    component: Option<usize>,
    name: String,
    focus: bool,
}

pub(super) struct State {
    pub selected: Option<usize>,
    pub selected_component: Option<usize>,
    pub edit: bool,
    pub overlay: bool,
    curve_state: crate::state::PointCurveState,
    curve_channel: super::CurveChannel,
    mixer_band: usize,
    manager_rect: Option<Rect>,
    list_rows: Vec<ListRow>,
    section_rects: Vec<(&'static str, Rect)>,
    radius: f32,
    feather: f32,
    erase: bool,
    start: Option<[f32; 2]>,
    stroke: Option<usize>,
    placing: bool,
    geometry_drag: Option<GeometryDrag>,
    creation: Option<CreationDraft>,
    thumbnails: Vec<Option<TextureHandle>>,
    overlay_key: Option<(usize, bool, [usize; 2])>,
    overlay_texture: Option<TextureHandle>,
    pub error: Option<String>,
    request: Option<&'static str>,
    pending: Option<std::sync::mpsc::Receiver<GeneratedMask>>,
    cancelled: bool,
    recognition_job: Option<RecognitionJob>,
    recognition: Option<serde_json::Value>,
    component_target: Option<(usize, bool)>,
    intersection_target: Option<usize>,
    operation: Option<MaskOperation>,
    rename: Option<Rename>,
    aspect: [f32; 2],
    tone_key: Option<photocraft_algo::camera_raw::CameraRaw>,
    tone_pixels: Vec<[f32; 4]>,
    tone_size: [usize; 2],
}
impl Default for State {
    fn default() -> Self {
        Self {
            selected: None,
            selected_component: None,
            edit: false,
            overlay: true,
            curve_state: Default::default(),
            curve_channel: Default::default(),
            mixer_band: 0,
            manager_rect: None,
            list_rows: Vec::new(),
            section_rects: Vec::new(),
            radius: 0.04,
            feather: 0.5,
            erase: false,
            start: None,
            stroke: None,
            placing: false,
            geometry_drag: None,
            creation: None,
            thumbnails: Vec::new(),
            overlay_key: None,
            overlay_texture: None,
            error: None,
            request: None,
            pending: None,
            cancelled: false,
            recognition_job: None,
            recognition: None,
            component_target: None,
            intersection_target: None,
            operation: None,
            rename: None,
            aspect: [1.0, 1.0],
            tone_key: None,
            tone_pixels: Vec::new(),
            tone_size: [0, 0],
        }
    }
}
impl State {
    pub(super) fn editing_after_cancel(&self) -> bool {
        self.creation.as_ref().map_or(self.edit, |draft| draft.previous_edit)
    }
    pub(super) fn selection_visible(&self, items: &[LocalAdjustment]) -> bool {
        if self.creation.is_some() {
            return true;
        }
        self.selected.and_then(|index| items.get(index)).is_some_and(|item| {
            item.enabled
                && self.selected_component.map_or(item.mask_enabled, |component| item.components.get(component).is_some_and(|component| component.enabled))
        })
    }
    pub(super) fn take_operation(&mut self) -> Option<MaskOperation> {
        self.operation.take()
    }
    pub(super) fn queue_operation(&mut self, operation: MaskOperation) -> Result<(), String> {
        if self.generating() {
            return Err("自动选择进行中，请结束后再修改蒙版列表".into());
        }
        if self.operation.is_some() {
            return Err("上一个蒙版操作尚未完成".into());
        }
        self.operation = Some(operation);
        Ok(())
    }
    pub(super) fn install_result(&mut self, items: &mut Vec<LocalAdjustment>, operation: &MaskOperation, result: EditResult) -> bool {
        let old_selected = self.selected;
        let old_component = self.selected_component;
        let mapped = old_selected.and_then(|index| result.old_to_new.get(index)).copied().flatten();
        let keep_selection =
            matches!(operation, MaskOperation::SetEnabled { .. } | MaskOperation::SetPrimaryEnabled { .. } | MaskOperation::SetComponentEnabled { .. });
        let next = if keep_selection { mapped } else { result.preferred_selection.or(mapped) }.filter(|index| *index < result.items.len());
        let mut component = if next == mapped { old_component } else { None };
        match operation {
            MaskOperation::Duplicate { .. } => component = None,
            MaskOperation::Intersect { index, .. } if next == Some(*index) => {
                component = result.items.get(*index).and_then(|item| item.components.len().checked_sub(1));
            }
            MaskOperation::DeleteComponent { index, component: removed } if old_selected == Some(*index) => {
                component = old_component.and_then(|current| {
                    if current == *removed {
                        None
                    } else if current > *removed {
                        current.checked_sub(1)
                    } else {
                        Some(current)
                    }
                });
            }
            _ => {}
        }
        component = component.filter(|index| next.and_then(|group| result.items.get(group)).is_some_and(|item| *index < item.components.len()));
        let changed = *items != result.items;
        end_gesture(self);
        *items = result.items;
        self.selected = next;
        self.selected_component = component;
        self.curve_state = Default::default();
        // An intersect command already contains its actual geometry. Its next
        // gesture edits handles rather than redrawing a default component.
        self.placing = false;
        self.edit = next.is_some() && (self.edit || matches!(operation, MaskOperation::Intersect { .. }));
        self.overlay_key = None;
        self.thumbnails.clear();
        self.rename = None;
        self.error = None;
        changed
    }
    pub(super) fn reset_geometry(&mut self) {
        self.cancel_generation();
        self.operation = None;
        self.rename = None;
        self.placing = false;
        end_gesture(self);
        self.overlay_key = None;
        self.thumbnails.clear();
    }
    fn invalidate_selected(&mut self) {
        self.overlay_key = None;
        if self.geometry_drag.is_none()
            && self.stroke.is_none()
            && let Some(slot) = self.selected.and_then(|i| self.thumbnails.get_mut(i))
        {
            *slot = None;
        }
    }
    pub(super) fn describe(&self, items: &[LocalAdjustment], rect: Option<Rect>) -> serde_json::Value {
        let selected = self.selected.and_then(|i| items.get(i));
        let selected_shape = if let Some(creation) = &self.creation { creation.preview.as_ref() } else { selected.map(|a| shape(a, self.selected_component)) };
        let handles = match (selected_shape, rect) {
            (Some(mask), Some(rect)) => handles(mask, image_aspect(rect))
                .into_iter()
                .map(|(handle, uv)| {
                    let p = screen(rect, uv);
                    serde_json::json!({"kind":format!("{handle:?}"),"normalized":uv,"position":[p.x,p.y]})
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        serde_json::json!({"selected":self.selected,"selectedComponent":self.selected_component,"editing":self.edit,"overlay":self.overlay,"error":self.error,
            "generating":self.generating(),"placing":self.placing,"dragging":self.geometry_drag.is_some() || self.creation.as_ref().is_some_and(|c| c.drag.is_some()),
            "creating":self.creation.as_ref().map(|c|c.label),
            "recognition":self.recognition,
            "shape":selected_shape.filter(|mask| matches!(mask, MaskShape::Radial { .. } | MaskShape::Linear { .. })),"handles":handles,
            "managerRect":self.manager_rect.map(|r| [r.min.x,r.min.y,r.max.x,r.max.y]),
            "rows":self.list_rows.iter().map(|row| serde_json::json!({"group":row.group,"component":row.component,
                "kind":if row.primary {"primary"} else if row.component.is_some() {"component"} else {"group"},
                "rect":[row.rect.min.x,row.rect.min.y,row.rect.max.x,row.rect.max.y],
                "nameRect":[row.name_rect.min.x,row.name_rect.min.y,row.name_rect.max.x,row.name_rect.max.y],
                "checkboxRect":[row.checkbox_rect.min.x,row.checkbox_rect.min.y,row.checkbox_rect.max.x,row.checkbox_rect.max.y],
                "deleteRect":row.delete_rect.map(|r| [r.min.x,r.min.y,r.max.x,r.max.y]),
                "menuRect":[row.menu_rect.min.x,row.menu_rect.min.y,row.menu_rect.max.x,row.menu_rect.max.y],
                "eyeRect":[row.checkbox_rect.min.x,row.checkbox_rect.min.y,row.checkbox_rect.max.x,row.checkbox_rect.max.y],
                "menuVisible":row.menu_visible,"eyeVisible":row.eye_visible,
                "thumbnailRect":row.thumbnail_rect.map(|r| [r.min.x,r.min.y,r.max.x,r.max.y])})).collect::<Vec<_>>(),
            "sections":self.section_rects.iter().map(|(name,r)|serde_json::json!({"name":name,"rect":[r.min.x,r.min.y,r.max.x,r.max.y]})).collect::<Vec<_>>(),
            "groups":items.iter().enumerate().map(|(index,a)| serde_json::json!({"index":index,"name":a.name,"enabled":a.enabled,
                "components":std::iter::once(serde_json::json!({"index":null,"name":shape_label(&a.mask),"operation":"add","enabled":a.mask_enabled}))
                    .chain(a.components.iter().enumerate().map(|(i,c)| serde_json::json!({"index":i,"name":c.name,"operation":if c.intersect {"intersect"} else if c.subtract {"subtract"} else {"add"},"enabled":c.enabled}))).collect::<Vec<_>>()
            })).collect::<Vec<_>>()})
    }
    pub(super) fn generating(&self) -> bool {
        self.request.is_some() || self.pending.is_some()
    }
    pub(super) fn generate(&mut self, kind: &str) -> Result<(), String> {
        self.generate_for(kind, None)
    }
    pub(super) fn create_gradient(&mut self, items: &mut Vec<LocalAdjustment>, kind: &str) -> Result<(), String> {
        let label = match kind {
            "linear" => "线性渐变",
            "radial" => "径向渐变",
            _ => return Err("create must be linear or radial".into()),
        };
        if self.generating() || self.operation.is_some() {
            return Err("请先完成当前蒙版操作".into());
        }
        if items.len() >= MAX_MASKS || shape_count(items) >= MAX_TOTAL_SHAPES {
            return Err("蒙版数量已达上限".into());
        }
        let mask = new_shapes().into_iter().find_map(|(name, mask)| (name == label).then_some(mask)).ok_or("Unknown gradient type")?;
        // This is the same view-only creation route used by the visible menu.
        create_manual(items, self, mask, label, CreationTarget::Group);
        Ok(())
    }
    fn generate_for(&mut self, kind: &str, component_target: Option<(usize, bool)>) -> Result<(), String> {
        let kind = match kind {
            "subject" => "subject",
            "sky" => "sky",
            "background" => "background",
            _ => return Err("generate must be subject, sky or background".into()),
        };
        if self.generating() || self.operation.is_some() {
            return Err("A mask generation is already running".into());
        }
        end_gesture(self);
        self.request = Some(kind);
        self.component_target = component_target;
        self.intersection_target = None;
        self.error = None;
        Ok(())
    }
    pub(super) fn cancel_generation(&mut self) {
        self.cancelled = true;
        if let Some(job) = &self.recognition_job {
            job.0.cancel();
        }
        self.request = None;
        self.intersection_target = None;
    }
}
fn shape(a: &LocalAdjustment, component: Option<usize>) -> &MaskShape {
    component.and_then(|i| a.components.get(i)).map_or(&a.mask, |c| &c.mask)
}
fn shape_mut(a: &mut LocalAdjustment, component: Option<usize>) -> &mut MaskShape {
    if let Some(component) = component.and_then(|i| a.components.get_mut(i)) { &mut component.mask } else { &mut a.mask }
}
fn shape_label(shape: &MaskShape) -> &'static str {
    match shape {
        MaskShape::Brush { .. } => "画笔",
        MaskShape::Linear { .. } => "线性渐变",
        MaskShape::Radial { .. } => "径向渐变",
        MaskShape::Luminance { .. } => "明亮度范围",
        MaskShape::Color { .. } => "颜色范围",
        MaskShape::Raster { .. } => "自动选择",
    }
}
fn new_shapes() -> [(&'static str, MaskShape); 5] {
    [
        ("画笔", MaskShape::Brush { strokes: Vec::new() }),
        ("线性渐变", MaskShape::Linear { start: [0.5, 0.25], end: [0.5, 0.75] }),
        ("径向渐变", MaskShape::Radial { center: [0.5, 0.5], radius: [0.25, 0.25], feather: 0.5, rotation: 0.0 }),
        ("颜色范围", MaskShape::Color { color: [0.5; 3], tolerance: 0.2, feather: 0.5 }),
        ("明亮度范围", MaskShape::Luminance { min: 0.4, max: 0.8, feather: 0.1 }),
    ]
}
fn shape_count(items: &[LocalAdjustment]) -> usize {
    items.iter().fold(0usize, |count, a| count.saturating_add(1).saturating_add(a.components.len()))
}
fn select(state: &mut State, index: usize, component: Option<usize>) {
    let cancelled_edit = state.creation.as_ref().map(|draft| draft.previous_edit);
    end_gesture(state);
    // A row click reclaims list focus even within the current group. A sticky
    // curve-point selection must not keep its Delete/Alt+X guard active.
    state.curve_state = Default::default();
    state.selected = Some(index);
    state.selected_component = component;
    state.edit = cancelled_edit.unwrap_or(true);
    state.placing = false;
    state.overlay_key = None;
}
pub(super) fn square_checkbox(ui: &mut egui::Ui, checked: &mut bool, label: &str) -> Response {
    let t = crate::theme::Tokens::get(ui.ctx());
    let galley = ui.painter().layout_no_wrap(label.into(), crate::theme::medium(12.0), t.text);
    let (rect, mut response) = ui.allocate_exact_size(vec2(14.0 + if label.is_empty() { 0.0 } else { 5.0 + galley.size().x }, 20.0), egui::Sense::click());
    if response.clicked() {
        *checked = !*checked;
        response.mark_changed();
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *checked, label));
    let icon = Rect::from_center_size(pos2(rect.left() + 7.0, rect.center().y), vec2(13.0, 13.0));
    ui.painter().rect_filled(icon, 0.0, Color32::TRANSPARENT);
    let outline = if !ui.is_enabled() {
        t.text_faint
    } else if response.hovered() {
        t.text
    } else {
        t.text_dim
    };
    ui.painter().rect_stroke(icon, 0.0, egui::Stroke::new(1.0, outline), egui::StrokeKind::Inside);
    if *checked {
        ui.painter().add(egui::Shape::line(
            vec![pos2(icon.left() + 2.5, icon.center().y), pos2(icon.left() + 5.5, icon.bottom() - 3.0), pos2(icon.right() - 2.0, icon.top() + 3.0)],
            egui::Stroke::new(1.6, if ui.is_enabled() { t.text } else { t.text_faint }),
        ));
    }
    if !label.is_empty() {
        ui.painter().galley(pos2(icon.right() + 5.0, rect.center().y - galley.size().y * 0.5), galley, t.text);
    }
    response
}
fn add(items: &mut Vec<LocalAdjustment>, state: &mut State, mask: MaskShape, label: &str) -> bool {
    if items.len() >= MAX_MASKS {
        state.error = Some("蒙版数量已达 64 个上限".into());
        return false;
    }
    items.push(LocalAdjustment { name: format!("{label} {}", items.len() + 1), mask, ..Default::default() });
    if let Err(e) = photocraft_algo::camera_raw_masks::validate(items) {
        items.pop();
        state.error = Some(e);
        return false;
    }
    state.selected = items.len().checked_sub(1);
    state.selected_component = None;
    state.edit = true;
    state.start = None;
    state.stroke = None;
    state.placing = matches!(items.last().map(|a| &a.mask), Some(MaskShape::Radial { .. } | MaskShape::Linear { .. }));
    state.geometry_drag = None;
    state.error = None;
    true
}
fn add_component(items: &mut [LocalAdjustment], state: &mut State, group: usize, mask: MaskShape, label: &str, subtract: bool) -> bool {
    let Some(a) = items.get_mut(group) else {
        state.error = Some("目标蒙版已不存在".into());
        return false;
    };
    if a.components.len() >= MAX_COMPONENTS {
        state.error = Some("每个蒙版最多添加 32 个组件".into());
        return false;
    }
    a.components.push(MaskComponent {
        name: format!("{label} {}", a.components.len() + 2),
        enabled: true,
        subtract,
        intersect: false,
        invert_previous: false,
        mask,
    });
    if let Err(error) = photocraft_algo::camera_raw_masks::validate(items) {
        if let Some(a) = items.get_mut(group) {
            a.components.pop();
        }
        state.error = Some(error);
        return false;
    }
    let component = items.get(group).and_then(|a| a.components.len().checked_sub(1));
    select(state, group, component);
    state.placing = items.get(group).is_some_and(|a| matches!(shape(a, component), MaskShape::Radial { .. } | MaskShape::Linear { .. }));
    state.invalidate_selected();
    state.error = None;
    true
}
fn create_manual(items: &mut Vec<LocalAdjustment>, state: &mut State, mask: MaskShape, label: &'static str, target: CreationTarget) -> bool {
    if state.generating() || state.operation.is_some() {
        state.error = Some("请先完成当前蒙版操作".into());
        return false;
    }
    if matches!(mask, MaskShape::Radial { .. } | MaskShape::Linear { .. }) {
        arm_gradient(state, mask, label, target);
        return false;
    }
    end_gesture(state);
    commit_manual(items, state, mask, label, target)
}
fn arm_gradient(state: &mut State, template: MaskShape, label: &'static str, target: CreationTarget) {
    end_gesture(state);
    state.creation = Some(CreationDraft { template, label, target, drag: None, preview: None, previous_edit: state.edit });
    state.placing = true;
    state.edit = true;
    state.error = None;
}
fn commit_manual(items: &mut Vec<LocalAdjustment>, state: &mut State, mask: MaskShape, label: &str, target: CreationTarget) -> bool {
    if shape_count(items) >= MAX_TOTAL_SHAPES {
        state.error = Some("蒙版形状数量已达上限".into());
        return false;
    }
    match target {
        CreationTarget::Group => add(items, state, mask, label),
        CreationTarget::Component { group, subtract } => add_component(items, state, group, mask, label, subtract),
        CreationTarget::Intersection { group: index } => {
            enqueue(state, MaskOperation::Intersect { index, shape: mask });
            false
        }
    }
}
pub(super) fn end_gesture(state: &mut State) {
    if (state.geometry_drag.is_some() || state.stroke.is_some())
        && let Some(slot) = state.selected.and_then(|i| state.thumbnails.get_mut(i))
    {
        *slot = None;
    }
    state.start = None;
    state.stroke = None;
    state.geometry_drag = None;
    if let Some(draft) = state.creation.take() {
        state.placing = false;
        state.edit = draft.previous_edit;
    }
}

fn thumbnail(ui: &mut egui::Ui, state: &mut State, index: usize, item: &LocalAdjustment) -> Option<Rect> {
    if state.thumbnails.len() <= index {
        state.thumbnails.resize_with(index.saturating_add(1), || None);
    }
    let slot = state.thumbnails.get_mut(index)?;
    if slot.is_none() {
        let prepared = PreparedLocalMask::new(item, state.aspect);
        let pixels = (0..32 * 32)
            .map(|i| {
                let uv = [((i % 32) as f32 + 0.5) / 32.0, ((i / 32) as f32 + 0.5) / 32.0];
                let m = prepared.weight(
                    uv,
                    if needs_tone(item) && !state.tone_pixels.is_empty() {
                        sample(&state.tone_pixels, state.tone_size[0], state.tone_size[1], uv)
                    } else {
                        [0.5, 0.5, 0.5, 1.0]
                    },
                );
                Color32::from_gray((m * 255.0).round() as u8)
            })
            .collect();
        let image = egui::ColorImage::new([32, 32], pixels);
        *slot = Some(ui.ctx().load_texture(format!("raw-mask-thumbnail-{index}"), image, egui::TextureOptions::LINEAR));
    }
    if let Some(texture) = slot {
        let response = ui.add(egui::Image::new(&*texture).fit_to_exact_size(vec2(30.0, 30.0)).sense(egui::Sense::click()));
        let color =
            if state.selected == Some(index) { crate::theme::Tokens::get(ui.ctx()).accent } else { ui.visuals().widgets.noninteractive.bg_stroke.color };
        ui.painter().rect_stroke(response.rect.expand(1.0), 2.0, egui::Stroke::new(1.5, color), egui::StrokeKind::Outside);
        if response.clicked() {
            select(state, index, None);
        }
        return Some(response.rect);
    }
    None
}

struct NameRow {
    rect: Rect,
    name_rect: Rect,
    checkbox_rect: Rect,
    delete_rect: Option<Rect>,
    selected: bool,
    menu_rect: Rect,
    menu_visible: bool,
    eye_visible: bool,
}

#[derive(Clone, Copy)]
enum RowKind {
    Group,
    Primary,
    Component(usize),
}
#[derive(Clone, Copy)]
struct RowTarget {
    group: usize,
    kind: RowKind,
}

fn enqueue(state: &mut State, operation: MaskOperation) {
    if let Err(error) = state.queue_operation(operation) {
        state.error = Some(error);
    }
}

fn row_menu(ui: &mut egui::Ui, name: &str, enabled: bool, target: RowTarget, state: &mut State) {
    let RowTarget { group: index, kind } = target;
    ui.set_min_width(190.0);
    ui.add_enabled_ui(!state.generating(), |ui| match kind {
        RowKind::Group => {
            if ui.button("重命名…").clicked() {
                state.rename = Some(Rename { index, component: None, name: name.into(), focus: true });
                ui.close();
            }
            if ui.button("反转蒙版    Alt+X").clicked() {
                enqueue(state, MaskOperation::Invert { index });
                ui.close();
            }
            if ui.button("复制并反转蒙版").clicked() {
                enqueue(state, MaskOperation::Duplicate { index, invert: true });
                ui.close();
            }
            ui.menu_button("与蒙版相交", |ui| {
                for (label, shape) in new_shapes() {
                    if ui.button(label).clicked() {
                        if matches!(shape, MaskShape::Radial { .. } | MaskShape::Linear { .. }) {
                            arm_gradient(state, shape, label, CreationTarget::Intersection { group: index });
                        } else {
                            enqueue(state, MaskOperation::Intersect { index, shape });
                        }
                        ui.close();
                    }
                }
                ui.separator();
                for (kind, label) in [("subject", "主体"), ("sky", "天空"), ("background", "背景")] {
                    if ui.button(label).clicked() {
                        match state.generate_for(kind, None) {
                            Ok(()) => state.intersection_target = Some(index),
                            Err(error) => state.error = Some(error),
                        }
                        ui.close();
                    }
                }
            });
            ui.separator();
            if ui.button("复制蒙版").clicked() {
                enqueue(state, MaskOperation::Duplicate { index, invert: false });
                ui.close();
            }
            if ui.button(if enabled { "隐藏蒙版" } else { "显示蒙版" }).clicked() {
                enqueue(state, MaskOperation::SetEnabled { index, enabled: !enabled });
                ui.close();
            }
            if ui.button("删除蒙版").clicked() {
                enqueue(state, MaskOperation::Delete { index });
                ui.close();
            }
            ui.separator();
            if ui.button("重置调整").clicked() {
                enqueue(state, MaskOperation::ResetAdjustments { index });
                ui.close();
            }
            if ui.button("删除当前空蒙版").clicked() {
                enqueue(state, MaskOperation::DeleteCurrentIfEmpty { index });
                ui.close();
            }
            if ui.button("删除所有空蒙版").clicked() {
                enqueue(state, MaskOperation::DeleteAllEmpty);
                ui.close();
            }
            if ui.button("删除所有蒙版").clicked() {
                enqueue(state, MaskOperation::DeleteAll);
                ui.close();
            }
        }
        RowKind::Primary => {
            if ui.button(if enabled { "隐藏基础组件" } else { "显示基础组件" }).clicked() {
                enqueue(state, MaskOperation::SetPrimaryEnabled { index, enabled: !enabled });
                ui.close();
            }
            if ui.button("删除所属蒙版组").clicked() {
                enqueue(state, MaskOperation::Delete { index });
                ui.close();
            }
        }
        RowKind::Component(component) => {
            if ui.button("重命名组件…").clicked() {
                state.rename = Some(Rename { index, component: Some(component), name: name.into(), focus: true });
                ui.close();
            }
            if ui.button(if enabled { "隐藏组件" } else { "显示组件" }).clicked() {
                enqueue(state, MaskOperation::SetComponentEnabled { index, component, enabled: !enabled });
                ui.close();
            }
            if ui.button("删除组件").clicked() {
                enqueue(state, MaskOperation::DeleteComponent { index, component });
                ui.close();
            }
        }
    });
}

fn rename_controls(ui: &mut egui::Ui, state: &mut State) {
    let Some(mut rename) = state.rename.take() else { return };
    let mut done = false;
    egui::Frame::NONE.stroke(egui::Stroke::new(1.0, crate::theme::Tokens::get(ui.ctx()).field_border)).inner_margin(6.0).show(ui, |ui| {
        ui.label(if rename.component.is_some() { "重命名组件" } else { "重命名蒙版" });
        let response = ui.add(egui::TextEdit::singleline(&mut rename.name).char_limit(80).desired_width(ui.available_width()).hint_text("名称"));
        if rename.focus {
            response.request_focus();
            rename.focus = false;
        }
        let confirm = (response.has_focus() || response.lost_focus())
            && ui.input_mut(|input| input.modifiers == egui::Modifiers::NONE && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        let cancel = ui.input_mut(|input| input.modifiers == egui::Modifiers::NONE && input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        ui.horizontal(|ui| {
            let accepted = ui.add_enabled(!state.generating() && valid_name(&rename.name), egui::Button::new("确认重命名")).clicked() || confirm;
            if accepted && valid_name(&rename.name) {
                enqueue(
                    state,
                    match rename.component {
                        Some(component) => MaskOperation::RenameComponent { index: rename.index, component, name: rename.name.clone() },
                        None => MaskOperation::Rename { index: rename.index, name: rename.name.clone() },
                    },
                );
                done = true;
            }
            if ui.button("取消重命名").clicked() || cancel {
                done = true;
            }
        });
    });
    if !done {
        state.rename = Some(rename);
    }
}

fn list_shortcuts(ui: &egui::Ui, state: &mut State, items: &[LocalAdjustment]) {
    if state.generating()
        || state.creation.is_some()
        || state.operation.is_some()
        || state.rename.is_some()
        || state.curve_state.selected.is_some()
        || state.curve_state.drag.is_some()
        || state.geometry_drag.is_some()
        || state.stroke.is_some()
        || ui.ctx().egui_wants_keyboard_input()
        || egui::Popup::is_any_open(ui.ctx())
    {
        return;
    }
    let Some(index) = state.selected.filter(|index| *index < items.len()) else { return };
    let deletion = ui.input_mut(|input| input.modifiers == egui::Modifiers::NONE && input.consume_key(egui::Modifiers::NONE, egui::Key::Delete));
    if deletion {
        enqueue(
            state,
            match state.selected_component {
                Some(component) => MaskOperation::DeleteComponent { index, component },
                None => MaskOperation::Delete { index },
            },
        );
    } else if ui.input_mut(|input| input.modifiers == egui::Modifiers::ALT && input.consume_key(egui::Modifiers::ALT, egui::Key::X)) {
        enqueue(state, MaskOperation::Invert { index });
    }
}

/// Keep labels independent of their length: trailing controls reserve their own
/// columns, and the label is clipped instead of pushing them out of the dock.
fn name_row(
    ui: &mut egui::Ui,
    name: &str,
    enabled: bool,
    selected: bool,
    decoration: (Option<&str>, Option<Rect>),
    target: RowTarget,
    state: &mut State,
) -> NameRow {
    let (operation, thumbnail_rect) = decoration;
    let tokens = crate::theme::Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(1.0), 22.0), egui::Sense::hover());
    let checkbox_rect = Rect::from_min_max(pos2(rect.right() - 22.0, rect.top()), rect.max);
    let menu_rect = Rect::from_min_size(pos2(checkbox_rect.left() - 24.0, rect.top()), vec2(22.0, 22.0));
    let name_rect = Rect::from_min_max(
        rect.min + vec2(if operation.is_some() { 12.0 } else { 0.0 }, 0.0),
        pos2((menu_rect.left() - 4.0).max(rect.left() + 1.0), rect.bottom()),
    );
    let hovered = ui.rect_contains_pointer(rect) || thumbnail_rect.is_some_and(|thumbnail| ui.rect_contains_pointer(thumbnail));
    let background = if selected {
        if tokens.row_selected == Color32::TRANSPARENT { tokens.accent_soft } else { tokens.row_selected }
    } else if hovered {
        tokens.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 0.0, background);
    let text_color = if selected && tokens.bevel {
        tokens.accent_text
    } else if !enabled {
        tokens.text_dim
    } else {
        tokens.text
    };
    if let Some(operation) = operation {
        ui.painter().text(pos2(rect.left(), rect.center().y), egui::Align2::LEFT_CENTER, operation, crate::theme::medium(11.0), tokens.text_dim);
    }
    let text = egui::RichText::new(name).color(text_color);
    let text = if selected { text.strong() } else { text };
    // Reserve the whole non-control area for selection, including whitespace
    // after a short label. The compact Label above it keeps its accessible name.
    let whitespace = ui.interact(Rect::from_min_max(rect.min, name_rect.max), ui.id().with("select-whitespace"), egui::Sense::click());
    // `Ui::put` always centers its compact child in the supplied rectangle.
    // Galley halign alone cannot change that outer allocation: place the label
    // in an explicitly left-to-right child so its first glyph starts at x=min.
    let mut name_ui = ui.new_child(egui::UiBuilder::new().id_salt("name").max_rect(name_rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
    name_ui.set_clip_rect(ui.clip_rect().intersect(name_rect));
    let label = name_ui.add(egui::Label::new(text).halign(egui::Align::Min).truncate().sense(egui::Sense::click()));
    label.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), selected, name));
    let picked = label.clicked() || whitespace.clicked();
    label.on_hover_text(name);
    let menu = ui.interact(menu_rect, ui.id().with("row-menu"), egui::Sense::click());
    menu.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, !state.generating(), format!("蒙版菜单：{name}")));
    let menu_visible = hovered || egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&menu));
    if menu_visible {
        crate::icons::paint(ui, menu_rect, "ellipsis", 15.0, text_color);
    }
    egui::Popup::menu(&menu).show(|ui| row_menu(ui, name, enabled, target, state));
    let eye = ui.interact(checkbox_rect, ui.id().with("row-eye"), if state.generating() { egui::Sense::hover() } else { egui::Sense::click() });
    eye.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, !state.generating(), enabled, format!("可见性：{name}")));
    let eye_visible = hovered || menu_visible;
    if eye_visible {
        crate::icons::paint(ui, checkbox_rect, if enabled { "eye" } else { "eye-off" }, 15.0, text_color);
    }
    if eye.clicked() && !state.generating() {
        let index = target.group;
        enqueue(
            state,
            match target.kind {
                RowKind::Group => MaskOperation::SetEnabled { index, enabled: !enabled },
                RowKind::Primary => MaskOperation::SetPrimaryEnabled { index, enabled: !enabled },
                RowKind::Component(component) => MaskOperation::SetComponentEnabled { index, component, enabled: !enabled },
            },
        );
    }
    NameRow { rect, name_rect, checkbox_rect, delete_rect: None, selected: picked, menu_rect, menu_visible, eye_visible }
}

#[cfg(test)]
fn remove_group(items: &mut Vec<LocalAdjustment>, state: &mut State, index: usize) -> bool {
    if state.generating() || index >= items.len() {
        return false;
    }
    end_gesture(state);
    items.remove(index);
    match state.selected {
        Some(selected) if selected == index => {
            state.selected = None;
            state.selected_component = None;
            state.edit = false;
        }
        Some(selected) if selected > index => state.selected = selected.checked_sub(1),
        _ => {}
    }
    state.curve_state = Default::default();
    state.placing = false;
    state.overlay_key = None;
    state.thumbnails.clear();
    true
}

#[cfg(test)]
fn remove_component(items: &mut [LocalAdjustment], state: &mut State, group: usize, component: usize) -> bool {
    if state.generating() || items.get(group).is_none_or(|item| component >= item.components.len()) {
        return false;
    }
    end_gesture(state);
    let Some(item) = items.get_mut(group) else { return false };
    item.components.remove(component);
    if state.selected == Some(group) {
        match state.selected_component {
            Some(selected) if selected == component => state.selected_component = None,
            Some(selected) if selected > component => state.selected_component = selected.checked_sub(1),
            _ => {}
        }
    }
    state.curve_state = Default::default();
    state.placing = false;
    state.invalidate_selected();
    true
}

/// Share normal Raw's translated headers/rows while giving local categories
/// an explicit divider that remains visible in every theme.
fn adjustment_section(ui: &mut egui::Ui, title: &'static str, open: bool, body: impl FnOnce(&mut egui::Ui)) -> Rect {
    let start = ui.cursor().min;
    let width = ui.available_width();
    let t = crate::theme::Tokens::get(ui.ctx());
    let (divider, _) = ui.allocate_exact_size(vec2(width, 2.0), egui::Sense::hover());
    ui.painter().line_segment([divider.left_center(), divider.right_center()], egui::Stroke::new(2.0, t.field_border));
    ui.push_id("raw-local-adjustments", |ui| super::section(ui, title, open, body));
    Rect::from_min_max(start, pos2(start.x + width, ui.cursor().min.y))
}

pub(super) fn panel(ui: &mut egui::Ui, items: &mut Vec<LocalAdjustment>, state: &mut State) -> bool {
    let mut dirty = false;
    let mut mask_dirty = false;
    state.manager_rect = None;
    state.list_rows.clear();
    state.section_rects.clear();
    if state.creation.is_some() && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        end_gesture(state);
    }
    if let Some(a) = state.selected.and_then(|i| items.get(i))
        && state.selected_component.is_some_and(|i| i >= a.components.len())
    {
        state.selected_component = None;
    }
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            crate::widgets::section_label(ui, "蒙版");
            ui.add_enabled_ui(!state.generating() && items.len() < MAX_MASKS && shape_count(items) < MAX_TOTAL_SHAPES, |ui| {
                ui.menu_button("＋ 创建新蒙版", |ui| {
                    for (label, mask) in new_shapes() {
                        if ui.button(label).clicked() {
                            mask_dirty |= create_manual(items, state, mask, label, CreationTarget::Group);
                            ui.close();
                        }
                    }
                })
            });
        });
        ui.horizontal(|ui| {
            ui.small("自动选择");
            for (kind, label) in [("subject", "主体"), ("sky", "天空"), ("background", "背景")] {
                if ui
                    .add_enabled(!state.generating() && items.len() < MAX_MASKS && shape_count(items) < MAX_TOTAL_SHAPES, egui::Button::new(label))
                    .on_hover_text("自动生成蒙版，请检查生成结果。")
                    .clicked()
                    && let Err(error) = state.generate(kind)
                {
                    state.error = Some(error);
                }
            }
        });
        if state.pending.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(if state.cancelled { "已取消，等待任务结束" } else { "正在选择…" });
                if ui.add_enabled(!state.cancelled, egui::Button::new("取消选择")).clicked() {
                    state.cancel_generation();
                }
            });
        }
        if items.is_empty() {
            ui.small("新建蒙版后，在预览上绘制或拖动，并调整下方参数。");
        }
        let manager_tokens = crate::theme::Tokens::get(ui.ctx());
        let manager_width = ui.available_width();
        let manager =
            egui::Frame::NONE.fill(manager_tokens.card).stroke(egui::Stroke::new(1.0, manager_tokens.field_border)).inner_margin(6.0).show(ui, |ui| {
                ui.set_min_width((manager_width - 14.0).max(1.0));
                egui::ScrollArea::vertical().id_salt("raw-mask-list").max_height(208.0).auto_shrink([false, true]).show_palette(ui, |ui| {
                    let tokens = crate::theme::Tokens::get(ui.ctx());
                    for (i, item) in items.iter_mut().enumerate() {
                        let group_width = ui.available_width();
                        egui::Frame::NONE.fill(tokens.card).inner_margin(4.0).show(ui, |ui| {
                            ui.set_min_width((group_width - 8.0).max(1.0));
                            ui.horizontal_top(|ui| {
                                let thumb = thumbnail(ui, state, i, item);
                                let names_width = ui.available_width();
                                ui.vertical(|ui| {
                                    ui.set_min_width(names_width);
                                    ui.set_max_width(names_width);
                                    let row = ui
                                        .push_id(("mask-group", i), |ui| {
                                            name_row(
                                                ui,
                                                &item.name,
                                                item.enabled,
                                                state.selected == Some(i),
                                                (None, thumb),
                                                RowTarget { group: i, kind: RowKind::Group },
                                                state,
                                            )
                                        })
                                        .inner;
                                    state.list_rows.push(ListRow {
                                        group: i,
                                        component: None,
                                        primary: false,
                                        rect: row.rect,
                                        name_rect: row.name_rect,
                                        checkbox_rect: row.checkbox_rect,
                                        delete_rect: row.delete_rect,
                                        thumbnail_rect: thumb,
                                        menu_rect: row.menu_rect,
                                        menu_visible: row.menu_visible,
                                        eye_visible: row.eye_visible,
                                    });
                                    if row.selected {
                                        select(state, i, None);
                                    }
                                    if state.selected != Some(i) {
                                        return;
                                    }
                                    let row = ui
                                        .push_id(("mask-primary", i), |ui| {
                                            name_row(
                                                ui,
                                                &format!("{}（基础）", shape_label(&item.mask)),
                                                item.mask_enabled,
                                                state.selected_component.is_none(),
                                                (None, None),
                                                RowTarget { group: i, kind: RowKind::Primary },
                                                state,
                                            )
                                        })
                                        .inner;
                                    state.list_rows.push(ListRow {
                                        group: i,
                                        component: None,
                                        primary: true,
                                        rect: row.rect,
                                        name_rect: row.name_rect,
                                        checkbox_rect: row.checkbox_rect,
                                        delete_rect: None,
                                        thumbnail_rect: None,
                                        menu_rect: row.menu_rect,
                                        menu_visible: row.menu_visible,
                                        eye_visible: row.eye_visible,
                                    });
                                    if row.selected {
                                        select(state, i, None);
                                    }
                                    for (j, component) in item.components.iter_mut().enumerate() {
                                        let row = ui
                                            .push_id(("mask-component", i, j), |ui| {
                                                name_row(
                                                    ui,
                                                    &component.name,
                                                    component.enabled,
                                                    state.selected_component == Some(j),
                                                    (
                                                        Some(if component.intersect {
                                                            "∩"
                                                        } else if component.subtract {
                                                            "−"
                                                        } else {
                                                            "+"
                                                        }),
                                                        None,
                                                    ),
                                                    RowTarget { group: i, kind: RowKind::Component(j) },
                                                    state,
                                                )
                                            })
                                            .inner;
                                        state.list_rows.push(ListRow {
                                            group: i,
                                            component: Some(j),
                                            primary: false,
                                            rect: row.rect,
                                            name_rect: row.name_rect,
                                            checkbox_rect: row.checkbox_rect,
                                            delete_rect: row.delete_rect,
                                            thumbnail_rect: None,
                                            menu_rect: row.menu_rect,
                                            menu_visible: row.menu_visible,
                                            eye_visible: row.eye_visible,
                                        });
                                        if row.selected {
                                            select(state, i, Some(j));
                                        }
                                    }
                                });
                            });
                        });
                    }
                });
                let mut addition = None;
                if let Some(group) = state.selected.filter(|i| *i < items.len()) {
                    let can_add =
                        !state.generating() && shape_count(items) < MAX_TOTAL_SHAPES && items.get(group).is_some_and(|a| a.components.len() < MAX_COMPONENTS);
                    ui.horizontal(|ui| {
                        for (subtract, label) in [(false, "添加"), (true, "减去")] {
                            ui.add_enabled_ui(can_add, |ui| {
                                ui.menu_button(label, |ui| {
                                    for (name, mask) in new_shapes() {
                                        if ui.button(name).clicked() {
                                            addition = Some((group, mask, name, subtract));
                                            ui.close();
                                        }
                                    }
                                    ui.separator();
                                    for (kind, name) in [("subject", "主体"), ("sky", "天空"), ("background", "背景")] {
                                        if ui.button(name).clicked() {
                                            if let Err(error) = state.generate_for(kind, Some((group, subtract))) {
                                                state.error = Some(error);
                                            }
                                            ui.close();
                                        }
                                    }
                                })
                            });
                        }
                    });
                }
                if let Some((group, mask, label, subtract)) = addition {
                    mask_dirty |= create_manual(items, state, mask, label, CreationTarget::Component { group, subtract });
                }
            });
        state.manager_rect = Some(manager.response.rect);
        rename_controls(ui, state);
        if let Some(creation) = &state.creation {
            ui.small(format!("等待绘制{}：在图像内拖动后创建。", creation.label));
            if ui.button("取消绘制").on_hover_text("Esc").clicked() {
                end_gesture(state);
            }
        }
        if state.creation.is_none()
            && let Some(i) = state.selected
            && let Some(a) = items.get_mut(i)
        {
            ui.horizontal(|ui| {
                square_checkbox(ui, &mut state.edit, "编辑蒙版");
                square_checkbox(ui, &mut state.overlay, "显示叠加");
                mask_dirty |= square_checkbox(ui, &mut a.invert, "反转").changed();
            });
            if let Some(c) = state.selected_component.and_then(|j| a.components.get_mut(j)) {
                ui.horizontal(|ui| {
                    if c.intersect {
                        ui.small("相交组件");
                    } else {
                        mask_dirty |= square_checkbox(ui, &mut c.subtract, "作为减去").changed();
                    }
                });
            }
            match shape_mut(a, state.selected_component) {
                MaskShape::Brush { .. } => {
                    ui.horizontal(|ui| {
                        square_checkbox(ui, &mut state.erase, "擦除");
                        if ui.button("结束绘制").clicked() {
                            state.edit = false;
                        }
                    });
                    ui.add(egui::Slider::new(&mut state.radius, 0.001..=0.3).text("大小"));
                    ui.add(egui::Slider::new(&mut state.feather, 0.0..=1.0).text("羽化"));
                }
                MaskShape::Radial { center, radius, feather, rotation } => {
                    mask_dirty |= ui.add(egui::Slider::new(feather, 0.0..=1.0).text("羽化")).changed();
                    ui.small(if state.placing {
                        "从中心向外拖动绘制椭圆。"
                    } else {
                        "拖动中心移动；拖动四边缩放；边缘外侧拖动旋转。"
                    });
                    egui::CollapsingHeader::new("位置与形状").id_salt("raw-mask-geometry").show(ui, |ui| {
                        mask_dirty |= ui.add(egui::Slider::new(&mut center[0], 0.0..=1.0).text("中心 X")).changed();
                        mask_dirty |= ui.add(egui::Slider::new(&mut center[1], 0.0..=1.0).text("中心 Y")).changed();
                        mask_dirty |= ui.add(egui::Slider::new(&mut radius[0], 0.001..=1.0).text("水平半径")).changed();
                        mask_dirty |= ui.add(egui::Slider::new(&mut radius[1], 0.001..=1.0).text("垂直半径")).changed();
                        mask_dirty |= ui.add(egui::Slider::new(rotation, -180.0..=180.0).text("旋转角度").suffix("°")).changed();
                    });
                }
                MaskShape::Linear { .. } => {
                    ui.small(if state.placing {
                        "沿渐变方向拖动；起点无调整，终点完全调整。"
                    } else {
                        "拖动端点调整渐变；拖动中点移动。"
                    });
                }
                MaskShape::Luminance { min, max, feather } => {
                    mask_dirty |= ui.add(egui::Slider::new(min, 0.0..=*max).text("下限")).changed();
                    mask_dirty |= ui.add(egui::Slider::new(max, *min..=1.0).text("上限")).changed();
                    mask_dirty |= ui.add(egui::Slider::new(feather, 0.0..=1.0).text("平滑")).changed();
                }
                MaskShape::Color { color, tolerance, feather } => {
                    mask_dirty |= crate::color_picker_ui::edit_rgb(ui, color).changed();
                    mask_dirty |= ui.add(egui::Slider::new(tolerance, 0.001..=1.0).text("范围")).changed();
                    mask_dirty |= ui.add(egui::Slider::new(feather, 0.0..=1.0).text("平滑")).changed();
                    ui.small("点击图像取样颜色。");
                }
                MaskShape::Raster { .. } => {
                    ui.small("自动选择结果；可添加、减去组件或反转整个蒙版组。");
                }
            }
            crate::widgets::section_label(ui, "局部调整");
            state.section_rects.push((
                "Light",
                adjustment_section(ui, "Light", true, |ui| {
                    super::row(ui, &mut dirty, "Exposure", &mut a.exposure, -5.0..=5.0, None);
                    super::row(ui, &mut dirty, "Contrast", &mut a.contrast, -100.0..=100.0, None);
                    super::row(ui, &mut dirty, "Highlights", &mut a.highlights, -100.0..=100.0, None);
                    super::row(ui, &mut dirty, "Shadows", &mut a.shadows, -100.0..=100.0, None);
                    super::row(ui, &mut dirty, "Whites", &mut a.whites, -100.0..=100.0, None);
                    super::row(ui, &mut dirty, "Blacks", &mut a.blacks, -100.0..=100.0, None);
                }),
            ));
            let mut advanced = a.advanced_params(1.0, 0);
            let mut advanced_dirty = false;
            let (temperature, tint, hue) = (super::temp_stops(), super::tint_stops(), crate::widgets::hue_stops());
            state.section_rects.push((
                "Color",
                adjustment_section(ui, "Color", true, |ui| {
                    super::row(ui, &mut dirty, "Temperature", &mut a.temperature, -100.0..=100.0, Some(&temperature));
                    super::row(ui, &mut dirty, "Tint", &mut a.tint, -100.0..=100.0, Some(&tint));
                    super::row(ui, &mut dirty, "Hue", &mut a.hue, -180.0..=180.0, Some(&hue));
                    super::row(ui, &mut advanced_dirty, "Vibrance", &mut advanced.vibrance, -100.0..=100.0, None);
                    super::row(ui, &mut dirty, "Saturation", &mut a.saturation, -100.0..=100.0, None);
                }),
            ));
            state.section_rects.push((
                "Effects",
                adjustment_section(ui, "Effects", false, |ui| {
                    super::row(ui, &mut advanced_dirty, "Texture", &mut advanced.texture, -100.0..=100.0, None);
                    super::row(ui, &mut advanced_dirty, "Clarity", &mut advanced.clarity, -100.0..=100.0, None);
                    super::row(ui, &mut advanced_dirty, "Dehaze", &mut advanced.dehaze, -100.0..=100.0, None);
                    super::subheading(ui, tl!("Grain"));
                    super::row(ui, &mut advanced_dirty, "Amount", &mut advanced.grain_amount, 0.0..=100.0, None);
                    super::row(ui, &mut advanced_dirty, "Size", &mut advanced.grain_size, 0.0..=100.0, None);
                    super::row(ui, &mut advanced_dirty, "Roughness", &mut advanced.grain_roughness, 0.0..=100.0, None);
                }),
            ));
            state.section_rects.push((
                "Curve",
                adjustment_section(ui, "Curve", false, |ui| {
                    super::curve_controls(ui, &mut advanced, &mut advanced_dirty, &mut state.curve_state, &mut state.curve_channel);
                    super::curve_editor(ui, &mut advanced, &mut advanced_dirty, &mut state.curve_state, state.curve_channel);
                    super::row(ui, &mut advanced_dirty, "Highlights", &mut advanced.curve_highlights, -100.0..=100.0, None);
                    super::row(ui, &mut advanced_dirty, "Lights", &mut advanced.curve_lights, -100.0..=100.0, None);
                    super::row(ui, &mut advanced_dirty, "Darks", &mut advanced.curve_darks, -100.0..=100.0, None);
                    super::row(ui, &mut advanced_dirty, "Shadows", &mut advanced.curve_shadows, -100.0..=100.0, None);
                }),
            ));
            state.section_rects.push((
                "Color Mixer",
                adjustment_section(ui, "Color Mixer", false, |ui| {
                    super::color_mixer(ui, &mut advanced, &mut state.mixer_band, &mut advanced_dirty);
                }),
            ));
            state.section_rects.push((
                "Color Grading",
                adjustment_section(ui, "Color Grading", false, |ui| {
                    super::wheel(ui, &mut advanced_dirty, "Shadows", &mut advanced.grade_shadows);
                    super::wheel(ui, &mut advanced_dirty, "Midtones", &mut advanced.grade_midtones);
                    super::wheel(ui, &mut advanced_dirty, "Highlights", &mut advanced.grade_highlights);
                    super::wheel(ui, &mut advanced_dirty, "Global", &mut advanced.grade_global);
                    super::row(ui, &mut advanced_dirty, "Blending", &mut advanced.grade_blending, 0.0..=100.0, None);
                    super::row(ui, &mut advanced_dirty, "Balance", &mut advanced.grade_balance, -100.0..=100.0, None);
                }),
            ));
            state.section_rects.push((
                "Detail",
                adjustment_section(ui, "Detail", false, |ui| {
                    super::subheading(ui, tl!("Sharpening"));
                    super::row(ui, &mut advanced_dirty, "Amount", &mut advanced.sharpen_amount, 0.0..=150.0, None);
                    super::row(ui, &mut advanced_dirty, "Radius", &mut advanced.sharpen_radius, 0.5..=3.0, None);
                    super::row(ui, &mut advanced_dirty, "Detail", &mut advanced.sharpen_detail, 0.0..=100.0, None);
                    super::row(ui, &mut advanced_dirty, "Masking", &mut advanced.sharpen_masking, 0.0..=100.0, None);
                    super::subheading(ui, tl!("Noise Reduction"));
                    super::row(ui, &mut advanced_dirty, "Luminance", &mut advanced.noise_luminance, 0.0..=100.0, None);
                    super::row(ui, &mut advanced_dirty, "Luminance Detail", &mut advanced.noise_luminance_detail, 0.0..=100.0, None);
                    super::row(ui, &mut advanced_dirty, "Color", &mut advanced.noise_color, 0.0..=100.0, None);
                    super::row(ui, &mut advanced_dirty, "Color Detail", &mut advanced.noise_color_detail, 0.0..=100.0, None);
                }),
            ));
            if advanced_dirty {
                a.update_advanced(&advanced);
                dirty = true;
            }
        }
        list_shortcuts(ui, state, items);
        if let Some(error) = &state.error {
            ui.label(error);
        }
    });
    if mask_dirty {
        state.invalidate_selected();
    }
    dirty || mask_dirty
}
fn sample(px: &[[f32; 4]], w: usize, h: usize, uv: [f32; 2]) -> [f32; 4] {
    let x = (uv[0] * w as f32).floor().max(0.0) as usize;
    let y = (uv[1] * h as f32).floor().max(0.0) as usize;
    px.get(y.min(h.saturating_sub(1)).saturating_mul(w).saturating_add(x.min(w.saturating_sub(1)))).copied().unwrap_or([0.0; 4])
}
fn needs_tone(a: &LocalAdjustment) -> bool {
    let needs = |mask: &MaskShape| matches!(mask, MaskShape::Color { .. } | MaskShape::Luminance { .. });
    a.mask_enabled && needs(&a.mask) || a.components.iter().any(|c| c.enabled && needs(&c.mask))
}
pub(super) fn prepare_tone(state: &mut State, params: &photocraft_algo::camera_raw::CameraRaw, source: &[[f32; 4]], w: usize, h: usize, float: bool) {
    if !state
        .selected
        .and_then(|i| params.local_adjustments.get(i))
        .is_some_and(|a| needs_tone(a) || matches!(shape(a, state.selected_component), MaskShape::Color { .. } | MaskShape::Luminance { .. }))
    {
        return;
    }
    let global = super::local_cache::global_params(params);
    if state.tone_key.as_ref() == Some(&global) {
        return;
    }
    let ow = 192usize;
    let oh = ((192.0 * h as f32 / w.max(1) as f32).round() as usize).clamp(1, 384);
    let mut pixels: Vec<_> = (0..ow * oh).map(|i| sample(source, w, h, [((i % ow) as f32 + 0.5) / ow as f32, ((i / ow) as f32 + 0.5) / oh as f32])).collect();
    let mut scaled = global.clone();
    scaled.pixel_scale *= ow as f32 / w.max(1) as f32;
    photocraft_algo::camera_raw::develop(&mut pixels, ow, oh, &scaled, float);
    state.tone_key = Some(global);
    state.tone_size = [ow, oh];
    state.tone_pixels = pixels;
    state.overlay_key = None;
    state.thumbnails.clear();
}

#[cfg(test)]
#[path = "camera_raw_mask_tests_v62.rs"]
mod tests_v62;
#[cfg(test)]
#[path = "camera_raw_mask_tests_v63.rs"]
mod tests_v63;
#[cfg(test)]
#[path = "camera_raw_mask_tests_v64.rs"]
mod tests_v64;
#[cfg(test)]
#[path = "camera_raw_mask_tests_v65.rs"]
mod tests_v65;
#[cfg(test)]
#[path = "camera_raw_mask_tests_v66.rs"]
mod tests_v66;
pub(super) struct Preview<'a> {
    pub rect: Rect,
    pub response: &'a Response,
    pub source: &'a [[f32; 4]],
    pub size: [usize; 2],
    pub can_edit: bool,
}
fn point_count(a: &LocalAdjustment) -> usize {
    std::iter::once(&a.mask).chain(a.components.iter().map(|c| &c.mask)).fold(0usize, |total, mask| {
        total.saturating_add(match mask {
            MaskShape::Brush { strokes } => strokes.iter().fold(0usize, |n, s| n.saturating_add(s.points.len())),
            _ => 0,
        })
    })
}
fn resize_cursor(mask: &MaskShape, handle: Handle) -> egui::CursorIcon {
    let angle = match mask {
        MaskShape::Radial { rotation, .. } => *rotation,
        _ => 0.0,
    } + if handle == Handle::RadiusY { 90.0 } else { 0.0 };
    match ((angle.rem_euclid(180.0) + 22.5) / 45.0).floor() as i32 % 4 {
        0 => egui::CursorIcon::ResizeHorizontal,
        1 => egui::CursorIcon::ResizeNwSe,
        2 => egui::CursorIcon::ResizeVertical,
        _ => egui::CursorIcon::ResizeNeSw,
    }
}
fn preview_creation(ui: &egui::Ui, state: &mut State, items: &mut Vec<LocalAdjustment>, rect: Rect, response: &Response, can_edit: bool) -> bool {
    // Preview runs before the panel. Escape must win over a release in the
    // same frame, before committing or losing the draft's previous edit mode.
    // Consuming it also keeps the outer Raw dialog from closing on that key.
    if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        end_gesture(state);
        return false;
    }
    let Some(mut creation) = state.creation.take() else { return false };
    if !can_edit || !ui.input(|i| i.viewport().focused.unwrap_or(i.focused)) {
        creation.drag = None;
        creation.preview = None;
        state.creation = Some(creation);
        return false;
    }
    let primary = egui::PointerButton::Primary;
    let press = ui.input(|input| {
        input.events.iter().find_map(|event| match event {
            egui::Event::PointerButton { pos, button, pressed: true, .. } if *button == primary => Some(*pos),
            _ => None,
        })
    });
    let press = press.filter(|&pos| {
        response.enabled() && rect.contains(pos) && response.interact_rect.contains(pos) && ui.ctx().layer_id_at(pos) == Some(response.layer_id)
    });
    // The preview owns a press inside the visible image. Follow its native
    // events in order: a pen lift sends button-up then PointerGone together.
    // egui may also see a whole short drag between two paints.
    let (release, pointer) = ui.input(|input| {
        let mut release = None;
        for event in &input.events {
            match event {
                egui::Event::PointerButton { pos, button, pressed: true, .. } if *button == primary => {
                    creation.drag = press.filter(|origin| origin == pos).map(|origin| GeometryDrag {
                        handle: Handle::Draw,
                        origin: normalized(rect, origin),
                        original: creation.template.clone(),
                        aspect: image_aspect(rect),
                    });
                }
                egui::Event::PointerButton { pos, button, pressed: false, .. } if *button == primary && creation.drag.is_some() => {
                    release = Some(*pos);
                    break;
                }
                egui::Event::PointerGone | egui::Event::WindowFocused(false) | egui::Event::Touch { phase: egui::TouchPhase::Cancel, .. } => {
                    creation.drag = None
                }
                _ => {}
            }
        }
        if release.is_none() && !input.pointer.button_down(primary) {
            creation.drag = None;
        }
        (release, input.pointer.latest_pos())
    });
    creation.preview = None;
    if let (Some(drag), Some(point)) = (&creation.drag, release.or(pointer))
        && screen(rect, drag.origin).distance(point) >= 3.0
    {
        let clipped = normalized(rect, point).map(|v| v.clamp(0.0, 1.0));
        if clipped.iter().all(|v| v.is_finite()) && clipped != drag.origin {
            let mut mask = creation.template.clone();
            // Draw may intentionally reproduce the template's dimensions;
            // equality then means valid geometry, not an empty gesture.
            update_geometry(&mut mask, drag, clipped);
            creation.preview = Some(mask);
        }
    }
    if release.is_some()
        && let Some(mask) = creation.preview.take()
    {
        let dirty = commit_manual(items, state, mask, creation.label, creation.target);
        if !dirty && state.operation.is_none() {
            state.edit = creation.previous_edit;
        }
        state.placing = false;
        state.overlay_key = None;
        state.thumbnails.clear();
        return dirty;
    }
    // A click with no range leaves the tool armed for the next drag.
    if release.is_some() {
        creation.drag = None;
    }
    if let Some(mask) = &creation.preview {
        draw_geometry(ui, mask, rect);
    }
    if response.hovered() && pointer.is_some_and(|p| rect.contains(p)) || creation.drag.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    state.creation = Some(creation);
    false
}
pub(super) fn preview(ui: &egui::Ui, state: &mut State, items: &mut Vec<LocalAdjustment>, input: Preview<'_>) -> bool {
    let Preview { rect, response, source, size: [w, h], can_edit } = input;
    if state.creation.is_some() {
        return preview_creation(ui, state, items, rect, response, can_edit);
    }
    if !state.selection_visible(items) {
        end_gesture(state);
        return false;
    }
    let total = items.iter().fold(0usize, |n, a| n.saturating_add(point_count(a)));
    let aspect = image_aspect(rect);
    if state.aspect != aspect {
        state.aspect = aspect;
        state.overlay_key = None;
        state.thumbnails.clear();
    }
    let Some(a) = state.selected.and_then(|i| items.get_mut(i)) else { return false };
    if state.selected_component.is_some_and(|i| i >= a.components.len()) {
        state.selected_component = None;
    }
    let mut dirty = false;
    let pointer = response.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.latest_pos()));
    let point = pointer.filter(|p| rect.contains(*p)).map(|p| normalized(rect, p).map(|v| v.clamp(0.0, 1.0)));
    let primary = egui::PointerButton::Primary;
    let current = shape_mut(a, state.selected_component);
    let geometry = matches!(current, MaskShape::Radial { .. } | MaskShape::Linear { .. });
    if geometry && state.edit && can_edit {
        if response.drag_started_by(primary)
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        {
            let handle = if state.placing && rect.contains(origin) { Some(Handle::Draw) } else { hit_handle(current, rect, origin) };
            state.geometry_drag = handle.map(|handle| GeometryDrag { handle, origin: normalized(rect, origin), original: current.clone(), aspect });
        }
        if (response.dragged_by(primary) || response.drag_stopped_by(primary))
            && let (Some(drag), Some(p)) = (&state.geometry_drag, pointer)
        {
            dirty |= update_geometry(current, drag, normalized(rect, p));
        }
    }
    if state.edit
        && can_edit
        && !geometry
        && let Some(p) = point
    {
        if response.drag_started_by(primary) {
            state.start = ui.input(|i| i.pointer.press_origin()).filter(|p| rect.contains(*p)).map(|p| normalized(rect, p));
            state.stroke = None;
        }
        if response.dragged_by(primary) || response.clicked_by(primary) {
            match current {
                MaskShape::Brush { strokes } => {
                    if total >= MAX_POINTS {
                        state.error = Some("画笔路径达到上限，请新建调整或简化路径".into());
                    } else {
                        if state.stroke.is_none() {
                            strokes.push(BrushStroke {
                                points: vec![state.start.unwrap_or(p)],
                                radius: state.radius,
                                feather: state.feather,
                                erase: state.erase,
                            });
                            state.stroke = strokes.len().checked_sub(1);
                            dirty = true;
                        }
                        if let Some(stroke) = state.stroke.and_then(|i| strokes.get_mut(i))
                            && stroke.points.last().is_none_or(|last| (last[0] - p[0]).hypot(last[1] - p[1]) > 0.002)
                            && total + 1 < MAX_POINTS
                        {
                            stroke.points.push(p);
                            dirty = true;
                        }
                    }
                }
                MaskShape::Color { color, .. } => {
                    let c = sample(&state.tone_pixels, state.tone_size[0], state.tone_size[1], p);
                    *color = [c[0].clamp(0.0, 1.0), c[1].clamp(0.0, 1.0), c[2].clamp(0.0, 1.0)];
                    dirty = true;
                }
                _ => {}
            }
        }
    }
    if response.drag_stopped_by(primary) || response.clicked_by(primary) || state.geometry_drag.is_some() && !ui.input(|i| i.pointer.button_down(primary)) {
        if state.geometry_drag.is_some() {
            state.placing = false;
        }
        end_gesture(state);
    }
    if dirty {
        state.invalidate_selected();
    }
    if state.overlay && a.enabled {
        let key = (state.selected.unwrap_or(0), a.invert, [w, h]);
        if state.overlay_key.as_ref() != Some(&key) {
            let ow = 192usize;
            let oh = ((192.0 * h as f32 / w.max(1) as f32).round() as usize).clamp(1, 384);
            let short = w.min(h).max(1) as f32;
            let aspect = [w as f32 / short, h as f32 / short];
            let prepared = PreparedLocalMask::new(a, aspect);
            let tone = needs_tone(a);
            let pixels = (0..ow * oh)
                .map(|i| {
                    let uv = [((i % ow) as f32 + 0.5) / ow as f32, ((i / ow) as f32 + 0.5) / oh as f32];
                    let color = if tone { sample(&state.tone_pixels, state.tone_size[0], state.tone_size[1], uv) } else { sample(source, w, h, uv) };
                    Color32::from_rgba_unmultiplied(240, 40, 70, (prepared.weight(uv, color) * 100.0) as u8)
                })
                .collect();
            state.overlay_texture =
                Some(ui.ctx().load_texture("raw-local-mask-overlay", egui::ColorImage::new([ow, oh], pixels), egui::TextureOptions::LINEAR));
            state.overlay_key = Some(key);
        }
        if let Some(tex) = &state.overlay_texture {
            ui.painter().image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
    }
    let current = shape(a, state.selected_component);
    if state.edit && can_edit && geometry {
        draw_geometry(ui, current, rect);
    }
    if state.edit
        && can_edit
        && let Some(p) = pointer
            .filter(|p| state.geometry_drag.is_some() || response.hovered() && (rect.contains(*p) || geometry && hit_handle(current, rect, *p).is_some()))
    {
        let cursor = state.geometry_drag.as_ref().map(|d| d.handle).or_else(|| hit_handle(current, rect, p));
        if cursor == Some(Handle::Rotate) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
            crate::icons::cursor(ui.ctx(), "rotate-cw", p, vec2(0.5, 0.5), 22.0);
        } else {
            ui.ctx().set_cursor_icon(match cursor {
                Some(Handle::Move) => egui::CursorIcon::Move,
                Some(h @ (Handle::RadiusX | Handle::RadiusY)) => resize_cursor(current, h),
                _ => egui::CursorIcon::Crosshair,
            });
        }
        if matches!(current, MaskShape::Brush { .. }) {
            ui.painter().circle_stroke(p, state.radius * rect.width().min(rect.height()), egui::Stroke::new(1.0, Color32::WHITE));
        }
    }
    dirty
}
pub(super) struct RecognitionOptions<'a> {
    pub transform: Option<&'a std::sync::Arc<photocraft_cms::Transform>>,
    pub quality: photocraft_engine::prefs::ImageRecognitionQuality,
}

pub(super) fn poll_and_start(
    ctx: &egui::Context,
    state: &mut State,
    items: &mut Vec<LocalAdjustment>,
    source: &std::sync::Arc<[[f32; 4]]>,
    w: usize,
    h: usize,
    options: RecognitionOptions<'_>,
) -> bool {
    let mut dirty = false;
    if let Some(receiver) = &state.pending {
        match receiver.try_recv() {
            Ok(Ok((mut mask, label, invert, diagnostics))) => {
                state.pending = None;
                state.recognition_job = None;
                if !state.cancelled {
                    state.recognition = Some(diagnostics);
                    let target = state.component_target.take();
                    let intersection = state.intersection_target.take();
                    if let Some(index) = intersection {
                        if invert && let MaskShape::Raster { values, .. } = &mut mask {
                            for value in values {
                                *value = 255 - *value;
                            }
                        }
                        enqueue(state, MaskOperation::Intersect { index, shape: mask });
                        return false;
                    }
                    if let Some((group, subtract)) = target {
                        // Background selection is an inverted generated raster. Inverting the
                        // component must not invert all of its sibling shapes.
                        if invert && let MaskShape::Raster { values, .. } = &mut mask {
                            for value in values {
                                *value = 255 - *value;
                            }
                        }
                        dirty = add_component(items, state, group, mask, label, subtract);
                    } else {
                        dirty = add(items, state, mask, label);
                    }
                    if dirty && let Some(a) = state.selected.and_then(|i| items.get_mut(i)) {
                        if target.is_none() {
                            a.invert = invert;
                        }
                        state.edit = false;
                    }
                }
            }
            Ok(Err(error)) => {
                state.pending = None;
                state.recognition_job = None;
                if !state.cancelled {
                    state.error = Some(error);
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                state.pending = None;
                state.recognition_job = None;
                state.error = Some("自动选择任务中断".into());
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                ctx.request_repaint_after(std::time::Duration::from_millis(50));
            }
        }
    }
    if let Some(kind) = state.request.take().filter(|_| state.pending.is_none()) {
        let Some(permit) = WorkerPermit::acquire(&AUTO_MASK_BUSY) else {
            state.error = Some("上一次自动选择仍在结束，请稍后重试".into());
            return dirty;
        };
        let pixels = source.clone();
        let transform = options.transform.cloned();
        let quality = options.quality;
        let repaint = ctx.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        state.pending = Some(rx);
        state.error = None;
        state.cancelled = false;
        state.recognition = None;
        let job = photocraft_engine::jobs::JobCtx::new();
        state.recognition_job = Some(RecognitionJob(job.clone()));
        let run = move || {
            let _permit = permit;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                use photocraft_engine::image_recognition::{self, Kind};
                job.check().map_err(|e| e.to_string())?;
                let mut analysis = pixels.to_vec();
                if let Some(transform) = transform {
                    for row in analysis.chunks_mut(w.max(1)) {
                        job.check().map_err(|e| e.to_string())?;
                        for pixel in row {
                            let mut rgb = [0.0; 3];
                            transform.eval(&pixel[..3], &mut rgb);
                            pixel[..3].copy_from_slice(&rgb);
                        }
                    }
                }
                let target = match kind {
                    "sky" => Kind::Sky,
                    "background" => Kind::Background,
                    _ => Kind::Subject,
                };
                let backend = if target == Kind::Sky { image_recognition::BackendPreference::Auto } else { quality.backend() };
                let recognized =
                    image_recognition::recognize_with_options(&analysis, w, h, target, image_recognition::Options { backend, ..Default::default() }, &job)
                        .map_err(|e| e.to_string())?;
                if !recognized.values.iter().any(|v| *v != 0) {
                    return Err("未检测到可靠区域；请使用画笔或范围蒙版".into());
                }
                let diagnostics = serde_json::json!({"backend": recognized.backend, "details": recognized.diagnostics});
                Ok((
                    MaskShape::Raster { width: recognized.width as u32, height: recognized.height as u32, values: recognized.values },
                    if kind == "sky" {
                        "天空"
                    } else if kind == "background" {
                        "背景"
                    } else {
                        "主体"
                    },
                    false,
                    diagnostics,
                ))
            }))
            .unwrap_or_else(|_| Err("自动选择失败，文档未修改".into()));
            let _ = tx.send(result);
            repaint.request_repaint();
        };
        #[cfg(not(target_arch = "wasm32"))]
        if let Err(error) = std::thread::Builder::new().name("raw-auto-mask".into()).spawn(run) {
            state.pending = None;
            state.recognition_job = None;
            state.error = Some(error.to_string());
        }
        #[cfg(target_arch = "wasm32")]
        run();
    }
    dirty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognition_cancellation_follows_editor_lifetime() {
        let job = photocraft_engine::jobs::JobCtx::new();
        let mut state = State { recognition_job: Some(RecognitionJob(job.clone())), ..Default::default() };
        state.cancel_generation();
        assert!(job.cancelled());
        let job = photocraft_engine::jobs::JobCtx::new();
        let state = State { recognition_job: Some(RecognitionJob(job.clone())), ..Default::default() };
        drop(state);
        assert!(job.cancelled(), "closing or replacing Raw must cancel its own recognition work");
    }

    #[test]
    fn radial_edit_moves_and_resizes_axes_without_replacing_other_geometry() {
        let original = MaskShape::Radial { center: [0.4, 0.45], radius: [0.2, 0.1], feather: 0.6, rotation: 0.0 };
        let rect = Rect::from_min_size(pos2(20.0, 40.0), vec2(800.0, 600.0));
        let mut mask = original.clone();
        assert_eq!(hit_handle(&mask, rect, screen(rect, [0.4, 0.45])), Some(Handle::Move));
        assert_eq!(hit_handle(&mask, rect, screen(rect, [0.6, 0.45])), Some(Handle::RadiusX));
        assert_eq!(hit_handle(&mask, rect, screen(rect, [0.4, 0.35])), Some(Handle::RadiusY));
        assert_eq!(hit_handle(&mask, rect, screen(rect, [0.01, 0.01])), None);
        let tiny = MaskShape::Radial { center: [0.4, 0.45], radius: [0.005, 0.005], feather: 0.6, rotation: 0.0 };
        assert_eq!(hit_handle(&tiny, rect, screen(rect, [0.405, 0.45])), Some(Handle::RadiusX), "nearest edge wins even inside the center's hit radius");
        let drag = GeometryDrag { handle: Handle::Move, origin: [0.4, 0.45], original: original.clone(), aspect: [4.0 / 3.0, 1.0] };
        assert!(!update_geometry(&mut mask, &drag, [0.4, 0.45]), "clicking a loaded ellipse must not redraw it");
        assert!(update_geometry(&mut mask, &drag, [0.6, 0.6]));
        let MaskShape::Radial { center, radius, feather, .. } = &mask else { panic!() };
        assert!((center[0] - 0.6).abs() < 1e-6 && (center[1] - 0.6).abs() < 1e-6);
        assert_eq!(*radius, [0.2, 0.1]);
        assert_eq!(*feather, 0.6);
        let drag = GeometryDrag { handle: Handle::RadiusX, origin: [0.8, 0.6], original: mask.clone(), aspect: [4.0 / 3.0, 1.0] };
        assert!(update_geometry(&mut mask, &drag, [0.9, 0.7]));
        let MaskShape::Radial { center, radius, feather, .. } = &mask else { panic!() };
        assert!((radius[0] - 0.3).abs() < 1e-6);
        assert_eq!(radius[1], 0.1);
        assert_eq!(*center, [0.6, 0.6]);
        assert_eq!(*feather, 0.6);
        let drag = GeometryDrag { handle: Handle::RadiusY, origin: [0.6, 0.5], original: mask.clone(), aspect: [4.0 / 3.0, 1.0] };
        assert!(update_geometry(&mut mask, &drag, [0.7, 0.3]));
        let MaskShape::Radial { radius, .. } = &mask else { panic!() };
        assert!((radius[0] - 0.3).abs() < 1e-6 && (radius[1] - 0.3).abs() < 1e-6);
        assert!(photocraft_algo::camera_raw_masks::validate(&[LocalAdjustment { mask: mask.clone(), ..Default::default() }]).is_ok());
        assert!(!update_geometry(&mut mask, &drag, [f32::NAN, 0.0]));
    }

    #[test]
    fn linear_edit_preserves_direction_on_move_and_never_makes_zero_length_mask() {
        let original = MaskShape::Linear { start: [0.3, 0.2], end: [0.6, 0.8] };
        let mut mask = original.clone();
        let drag = GeometryDrag { handle: Handle::Move, origin: [0.45, 0.5], original: original.clone(), aspect: [4.0 / 3.0, 1.0] };
        assert!(update_geometry(&mut mask, &drag, [1.0, 1.0]));
        let MaskShape::Linear { start, end } = &mask else { panic!() };
        assert!((end[0] - start[0] - 0.3).abs() < 1e-6 && (end[1] - start[1] - 0.6).abs() < 1e-6);
        assert_eq!(*end, [1.0, 1.0]);
        let drag = GeometryDrag { handle: Handle::LinearStart, origin: *start, original: mask.clone(), aspect: [4.0 / 3.0, 1.0] };
        let before = mask.clone();
        assert!(!update_geometry(&mut mask, &drag, [1.0, 1.0]));
        assert_eq!(mask, before);
    }

    #[test]
    fn worker_permit_outlives_cancelled_or_replaced_editor_state() {
        static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let permit = WorkerPermit::acquire(&BUSY).unwrap();
        let mut state = State::default();
        state.generate("subject").unwrap();
        state.cancel_generation();
        drop(state);
        let _replacement = State::default();
        assert!(WorkerPermit::acquire(&BUSY).is_none(), "resetting the dialog must not release running work");
        drop(permit);
        assert!(WorkerPermit::acquire(&BUSY).is_some(), "completed or failed work must release its permit");
    }

    #[test]
    fn mask_name_editor_matches_engine_byte_and_control_limits() {
        for name in ["蒙".repeat(80), "😀".repeat(64), "正常名称".into()] {
            assert!(valid_name(&name));
            let item = LocalAdjustment { name, ..Default::default() };
            assert!(photocraft_algo::camera_raw_masks::validate(&[item]).is_ok());
        }
        for name in ["😀".repeat(65), "\tname".into(), "  ".into()] {
            assert!(!valid_name(&name));
            let item = LocalAdjustment { name, ..Default::default() };
            assert!(photocraft_algo::camera_raw_masks::validate(&[item]).is_err());
        }
    }
}
