//! Sparse, Studio Light-only interface colours. Stored preferences never alter another theme.
//!
//! Module colours remain separate from general tokens. `apply` only updates the context when
//! the active theme or colour map changes; editing the preset library cannot reset a widget.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Color32, Context, Id, Stroke, Ui};
use photocraft_engine::prefs::palette::{ColorSlot, PaletteRgb, StudioLightColors};

use super::{ThemeKind, Tokens};

#[derive(Clone, PartialEq)]
struct Applied {
    kind: ThemeKind,
    colors: BTreeMap<ColorSlot, PaletteRgb>,
}

fn applied_id() -> Id {
    Id::new("photocraft-studio-light-colors")
}

pub(super) fn invalidate(ctx: &Context) {
    ctx.data_mut(|d| d.remove::<Arc<Applied>>(applied_id()));
}

fn rgb(value: PaletteRgb) -> Color32 {
    let [r, g, b] = value.rgb();
    Color32::from_rgb(r, g, b)
}

/// Title and menu backgrounds form one visual surface. Keep reading the old
/// MenuBar key for existing files, but a current TitleBar value always wins.
/// This is a display alias only: stored maps and user libraries stay untouched.
fn configured_color(colors: &BTreeMap<ColorSlot, PaletteRgb>, slot: ColorSlot) -> Option<Color32> {
    let value = match slot {
        ColorSlot::TitleBar | ColorSlot::MenuBar => colors.get(&ColorSlot::TitleBar).or_else(|| colors.get(&ColorSlot::MenuBar)),
        _ => colors.get(&slot),
    };
    value.copied().map(rgb)
}

fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), alpha)
}

/// The original role colour in the Studio Light theme, before any override.
pub fn default_color(slot: ColorSlot) -> Color32 {
    token_color(&Tokens::for_kind(ThemeKind::StudioLight), slot)
}

fn token_color(t: &Tokens, slot: ColorSlot) -> Color32 {
    use ColorSlot::*;
    match slot {
        TitleBar | MenuBar | Toolbar | OptionsBar | StatusBar => t.chrome,
        Dock => t.dock,
        Card | MenuBackground | FloatingTitle => t.card,
        PanelTitle => {
            if t.pro {
                t.tab_strip
            } else {
                t.card
            }
        }
        CardBorder => t.card_border,
        MenuHover | Hover => t.hover,
        MenuSelection | Selection => {
            if t.pro || t.bevel {
                t.accent
            } else {
                t.accent_soft
            }
        }
        MenuText | Text => t.text,
        Field | ProgressTrack | ScrollbarTrack => t.field,
        FieldBorder | SliderTrack => t.field_border,
        Pressed => t.pressed,
        TextDim => t.text_dim,
        TextFaint => t.text_faint,
        Icon => t.icon,
        Accent | ProgressFill => t.accent,
        AccentText => t.accent_text,
        Separator => t.separator,
        SliderFill => {
            if t.bevel {
                t.accent
            } else {
                t.text_dim
            }
        }
        SliderHandle => {
            if t.pro {
                Color32::from_gray(236)
            } else {
                Color32::WHITE
            }
        }
        ScrollbarHandle => {
            if t.bevel {
                super::CLASSIC_SCROLLBAR_COLORS.idle
            } else {
                t.text
            }
        }
        Canvas => t.canvas,
        CanvasDot => t.canvas_dot,
    }
}

fn resolved_tokens(kind: ThemeKind, colors: &BTreeMap<ColorSlot, PaletteRgb>) -> Tokens {
    let mut t = Tokens::for_kind(kind);
    if kind != ThemeKind::StudioLight {
        return t;
    }
    for (&slot, &value) in colors {
        use ColorSlot::*;
        let c = rgb(value);
        match slot {
            Dock => t.dock = c,
            Card => t.card = c,
            CardBorder => t.card_border = c,
            Field => t.field = c,
            FieldBorder => t.field_border = c,
            Hover => t.hover = c,
            Pressed => t.pressed = c,
            Text => t.text = c,
            TextDim => t.text_dim = c,
            TextFaint => t.text_faint = c,
            Icon => t.icon = c,
            Accent => {
                t.accent = c;
                // Retain each built-in coverage: the hue changes without turning the subtle
                // focus/selection treatments into a solid block unless Selection is overridden.
                if !colors.contains_key(&Selection) {
                    t.accent_soft = with_alpha(c, t.accent_soft.a());
                }
                t.accent_border = with_alpha(c, t.accent_border.a());
            }
            AccentText => t.accent_text = c,
            Selection => {
                t.accent_soft = c;
                t.row_selected = c;
            }
            Separator => t.separator = c,
            Canvas => t.canvas = c,
            CanvasDot => t.canvas_dot = c,
            // These are module-specific roles, so they must not leak into generic controls.
            TitleBar | MenuBar | Toolbar | OptionsBar | StatusBar | PanelTitle | FloatingTitle | MenuBackground | MenuHover | MenuSelection | MenuText
            | ProgressTrack | ProgressFill | SliderTrack | SliderFill | SliderHandle | ScrollbarTrack | ScrollbarHandle => {}
        }
    }
    t
}

/// Resolve a draft without publishing it (for the Preferences swatches and preview).
pub fn resolved_color(kind: ThemeKind, config: &StudioLightColors, slot: ColorSlot) -> Color32 {
    let t = resolved_tokens(kind, &config.colors);
    if kind == ThemeKind::StudioLight
        && let Some(value) = configured_color(&config.colors, slot)
    {
        value
    } else {
        token_color(&t, slot)
    }
}

/// An explicit active override; absent in every other theme and after a default reset.
pub fn override_color(ctx: &Context, slot: ColorSlot) -> Option<Color32> {
    if Tokens::get(ctx).kind != ThemeKind::StudioLight {
        return None;
    }
    ctx.data(|d| d.get_temp::<Arc<Applied>>(applied_id())).filter(|a| a.kind == ThemeKind::StudioLight).and_then(|a| configured_color(&a.colors, slot))
}

pub fn has_override(ctx: &Context, slot: ColorSlot) -> bool {
    override_color(ctx, slot).is_some()
}

/// Read a module colour, with its ordinary current-theme token as the fallback.
pub fn color(ctx: &Context, slot: ColorSlot) -> Color32 {
    override_color(ctx, slot).unwrap_or_else(|| token_color(&Tokens::get(ctx), slot))
}

/// Apply only an active palette change. Saved presets have no effect until loaded.
pub fn apply(ctx: &Context, kind: ThemeKind, config: &StudioLightColors) {
    let previous = ctx.data(|d| d.get_temp::<Arc<Applied>>(applied_id()));
    let unchanged = previous
        .as_ref()
        .is_some_and(|old| old.kind == kind && if kind == ThemeKind::StudioLight { old.colors == config.colors } else { old.colors.is_empty() });
    if unchanged && Tokens::get(ctx).kind == kind {
        return;
    }
    let colors = if kind == ThemeKind::StudioLight { config.colors.clone() } else { BTreeMap::new() };
    let desired = Arc::new(Applied { kind, colors });
    super::apply_tokens(ctx, resolved_tokens(kind, &desired.colors));
    // Native settings/dialog clients use panel_fill rather than window_fill.
    // An explicit panel colour covers those clients too; an empty map keeps
    // each original theme's distinct chrome/card defaults untouched.
    if let Some(value) = desired.colors.get(&ColorSlot::Card) {
        ctx.global_style_mut(|style| style.visuals.panel_fill = rgb(*value));
    }
    ctx.data_mut(|d| d.insert_temp(applied_id(), desired));
    ctx.request_repaint();
}

/// Style the contents of a menu after entering its local UI. The caller owns its frame fill.
/// No local style is changed when there are no menu overrides.
pub fn menu_scope(ui: &mut Ui) {
    let background = override_color(ui.ctx(), ColorSlot::MenuBackground);
    let hover = override_color(ui.ctx(), ColorSlot::MenuHover);
    let selected = override_color(ui.ctx(), ColorSlot::MenuSelection);
    let text = override_color(ui.ctx(), ColorSlot::MenuText);
    if background.is_none() && hover.is_none() && selected.is_none() && text.is_none() {
        return;
    }
    let v = &mut ui.style_mut().visuals;
    if let Some(c) = background {
        v.window_fill = c;
    }
    if let Some(c) = hover {
        v.widgets.hovered.bg_fill = c;
        v.widgets.hovered.weak_bg_fill = c;
        v.widgets.open.bg_fill = c;
        v.widgets.open.weak_bg_fill = c;
    }
    if let Some(c) = selected {
        v.selection.bg_fill = c;
        v.widgets.active.bg_fill = c;
        v.widgets.active.weak_bg_fill = c;
    }
    if let Some(c) = text {
        v.override_text_color = Some(c);
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.fg_stroke = Stroke::new(w.fg_stroke.width, c);
        }
    }
}

/// Local styling for an egui ProgressBar. Call inside the bar's existing UI scope.
pub fn progress_scope(ui: &mut Ui) {
    let track = override_color(ui.ctx(), ColorSlot::ProgressTrack);
    let fill = override_color(ui.ctx(), ColorSlot::ProgressFill);
    if track.is_none() && fill.is_none() {
        return;
    }
    let v = &mut ui.style_mut().visuals;
    if let Some(c) = track {
        v.extreme_bg_color = c;
    }
    if let Some(c) = fill {
        v.selection.bg_fill = c;
    }
}

/// Independent scrollbar colours without recolouring the body or changing its IDs/layout.
/// egui 0.36 shares the scrollbar's channels with ordinary widget visuals; only the parent
/// scroll container receives those temporary channels, and its content UI gets the original.
fn scrollbar_style(ui: &mut Ui) -> Option<std::sync::Arc<egui::Style>> {
    let track = override_color(ui.ctx(), ColorSlot::ScrollbarTrack);
    let handle = override_color(ui.ctx(), ColorSlot::ScrollbarHandle);
    if track.is_none() && handle.is_none() {
        return None;
    }
    let original = ui.style().clone();
    let v = &mut ui.style_mut().visuals;
    if let Some(c) = track {
        v.extreme_bg_color = c;
    }
    if let Some(c) = handle {
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
            w.fg_stroke = Stroke::new(w.fg_stroke.width, c);
        }
    }
    Some(original)
}

/// Desktop forms and lists scroll through their bars or the wheel, never by
/// dragging their contents. Upstream egui's `OnTouch` policy also becomes active for
/// later pointer events once a touch-capable device has been recorded.
///
/// This is the shared UI policy, independent of palette overrides. Existing
/// callers use Default or ALL, both of which enable bars and the wheel.
/// Browser touch navigation keeps the dependency's ordinary OnTouch default.
pub(crate) fn desktop_scroll_source() -> egui::containers::scroll_area::ScrollSource {
    #[cfg(target_arch = "wasm32")]
    {
        egui::containers::scroll_area::ScrollSource::default()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        egui::containers::scroll_area::ScrollSource { scroll_bar: true, drag: egui::containers::scroll_area::DragScroll::Never, mouse_wheel: true }
    }
}

pub fn scroll_area<R>(area: egui::ScrollArea, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> egui::scroll_area::ScrollAreaOutput<R> {
    let area = area.scroll_source(desktop_scroll_source()).reserve_scroll_bar_space(!cfg!(target_arch = "wasm32"));
    let Some(original) = scrollbar_style(ui) else { return area.show(ui, add_contents) };
    let result = area.show(ui, |body| {
        body.set_style(original.clone());
        add_contents(body)
    });
    ui.set_style(original);
    result
}

pub trait ScrollAreaPaletteExt {
    fn show_palette<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> egui::scroll_area::ScrollAreaOutput<R>;

    fn show_palette_rows<R>(
        self,
        ui: &mut Ui,
        row_height_sans_spacing: f32,
        total_rows: usize,
        add_contents: impl FnOnce(&mut Ui, std::ops::Range<usize>) -> R,
    ) -> egui::scroll_area::ScrollAreaOutput<R>;
}

impl ScrollAreaPaletteExt for egui::ScrollArea {
    fn show_palette<R>(self, ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> egui::scroll_area::ScrollAreaOutput<R> {
        scroll_area(self, ui, add_contents)
    }

    fn show_palette_rows<R>(
        self,
        ui: &mut Ui,
        row_height_sans_spacing: f32,
        total_rows: usize,
        add_contents: impl FnOnce(&mut Ui, std::ops::Range<usize>) -> R,
    ) -> egui::scroll_area::ScrollAreaOutput<R> {
        let area = self.scroll_source(desktop_scroll_source()).reserve_scroll_bar_space(!cfg!(target_arch = "wasm32"));
        let Some(original) = scrollbar_style(ui) else { return area.show_rows(ui, row_height_sans_spacing, total_rows, add_contents) };
        let result = area.show_rows(ui, row_height_sans_spacing, total_rows, |body, range| {
            body.set_style(original.clone());
            add_contents(body, range)
        });
        ui.set_style(original);
        result
    }
}

#[cfg(test)]
#[path = "palette_tests_v68.rs"]
mod tests;

#[cfg(test)]
#[path = "palette_tests_v69.rs"]
mod topbar_tests;

#[cfg(test)]
#[path = "scroll_interaction_tests_v70.rs"]
mod scroll_interaction_tests;

#[cfg(test)]
#[path = "scrollbar_gutter_v78/tests.rs"]
mod scrollbar_gutter_tests;
