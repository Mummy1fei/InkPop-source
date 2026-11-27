//! Camera Raw's persistent page state and compact right-hand icon rail.
use egui::{Rect, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(super) const RAIL_WIDTH: f32 = 36.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Tab {
    #[default]
    Edit,
    Presets,
    Masks,
    ColorSpaces,
    ColorLookup,
}

impl Tab {
    pub const ALL: [Self; 5] = [Self::Edit, Self::Masks, Self::Presets, Self::ColorSpaces, Self::ColorLookup];

    pub fn label(self) -> &'static str {
        match self {
            Self::Edit => tl!("Edit"),
            Self::Presets => tl!("Presets"),
            Self::Masks => tl!("Masking"),
            Self::ColorSpaces => tl!("Color Spaces"),
            Self::ColorLookup => tl!("Color Lookup (LUT)"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Edit => "sliders-horizontal",
            Self::Presets => "blend",
            Self::Masks => "circle-dashed",
            Self::ColorSpaces => "palette",
            Self::ColorLookup => "grid-3x3",
        }
    }
}

#[derive(Default)]
pub(super) struct Geometry {
    pub rail: Option<Rect>,
    pub content: Option<Rect>,
    pub buttons: [Option<Rect>; 5],
}

impl Geometry {
    pub fn describe(&self, active: Tab) -> Value {
        let rect = |rect: Option<Rect>| rect.map(|r| [r.left(), r.top(), r.right(), r.bottom()]);
        json!({"active":active,"railRect":rect(self.rail),"contentRect":rect(self.content),
            "buttons":Tab::ALL.into_iter().zip(self.buttons).map(|(tab,r)|
                json!({"tab":tab,"label":tab.label(),"selected":tab==active,"rect":rect(r)})).collect::<Vec<_>>()})
    }
}

/// Keep the whole dock's width fixed: reserve only the outer strip for the rail.
pub(super) fn content_rect(dock: Rect) -> Rect {
    Rect::from_min_max(dock.min, pos2((dock.right() - RAIL_WIDTH).max(dock.left()), dock.bottom()))
}

pub(super) fn rail(ui: &mut Ui, dock: Rect, active: Tab, geometry: &mut Geometry) -> Option<Tab> {
    let content = content_rect(dock);
    let rail = Rect::from_min_max(pos2(content.right(), dock.top()), dock.max);
    geometry.rail = Some(rail);
    geometry.content = Some(content);
    geometry.buttons = [None; 5];
    let tokens = crate::theme::Tokens::get(ui.ctx());
    ui.painter().rect_filled(rail, 0.0, tokens.chrome);
    ui.painter().line_segment([rail.left_top(), rail.left_bottom()], egui::Stroke::new(1.0, tokens.separator));
    let mut icons = ui.new_child(egui::UiBuilder::new().id_salt("camera-raw-tab-rail").max_rect(rail.shrink2(vec2(4.0, 8.0))));
    icons.set_clip_rect(rail.intersect(ui.clip_rect()));
    icons.spacing_mut().item_spacing.y = 6.0;
    let mut clicked = None;
    for (index, tab) in Tab::ALL.into_iter().enumerate() {
        let selected = active == tab;
        let response = crate::icons::rail_button(&mut icons, tab.icon(), 28.0, selected, "");
        geometry.buttons[index] = Some(response.rect);
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, ui.is_enabled(), selected, tab.label()));
        if response.on_hover_cursor(egui::CursorIcon::Default).on_hover_text(tab.label()).clicked() {
            clicked = Some(tab);
        }
    }
    clicked
}
