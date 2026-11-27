//! Immutable starter palettes for Studio Light. They never occupy the user's saved library.
//! Original role colours derived from four product-colour references; photographs are not bundled.
use super::{ColorSlot, PaletteRgb};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub struct BuiltinPalette {
    pub id: &'static str,
    pub name: &'static str,
    values: [(ColorSlot, u32); 35],
}

impl BuiltinPalette {
    /// Read one immutable preview colour without allocating a complete draft.
    pub fn color(&self, slot: ColorSlot) -> Option<PaletteRgb> {
        self.values.iter().find(|&&(key, _)| key == slot).map(|&(_, value)| PaletteRgb::new([(value >> 16) as u8, (value >> 8) as u8, value as u8]))
    }

    pub fn matches(&self, colors: &BTreeMap<ColorSlot, PaletteRgb>) -> bool {
        colors.len() == self.values.len()
            && self
                .values
                .iter()
                .all(|&(slot, value)| colors.get(&slot).copied() == Some(PaletteRgb::new([(value >> 16) as u8, (value >> 8) as u8, value as u8])))
    }

    /// A fresh draft: editing it cannot mutate this starter or the saved palette library.
    pub fn colors(&self) -> BTreeMap<ColorSlot, PaletteRgb> {
        self.values.iter().map(|&(slot, value)| (slot, PaletteRgb::new([(value >> 16) as u8, (value >> 8) as u8, value as u8]))).collect()
    }
}

pub const BUILTIN_PALETTES: [BuiltinPalette; 4] = [
    BuiltinPalette {
        id: "warm-gray-orange",
        name: "Warm Gray Orange",
        values: [
            (ColorSlot::TitleBar, 0xd2cec1),
            (ColorSlot::MenuBar, 0xd2cec1),
            (ColorSlot::Toolbar, 0xddd9ce),
            (ColorSlot::OptionsBar, 0xebe7de),
            (ColorSlot::StatusBar, 0xdedacf),
            (ColorSlot::Dock, 0xdcd8ce),
            (ColorSlot::Card, 0xf5f2eb),
            (ColorSlot::CardBorder, 0xa49e90),
            (ColorSlot::PanelTitle, 0xe5e0d4),
            (ColorSlot::FloatingTitle, 0xd8d1c2),
            (ColorSlot::MenuBackground, 0xfaf7f0),
            (ColorSlot::MenuHover, 0xefe1cd),
            (ColorSlot::MenuSelection, 0xefd2ac),
            (ColorSlot::MenuText, 0x34312c),
            (ColorSlot::Field, 0xfaf8f2),
            (ColorSlot::FieldBorder, 0x999183),
            (ColorSlot::Hover, 0xe9e1d3),
            (ColorSlot::Pressed, 0xd7c6ac),
            (ColorSlot::Text, 0x30312e),
            (ColorSlot::TextDim, 0x5d5b53),
            (ColorSlot::TextFaint, 0x767168),
            (ColorSlot::Icon, 0x645849),
            (ColorSlot::Accent, 0xb34f19),
            (ColorSlot::AccentText, 0xfffaf2),
            (ColorSlot::Selection, 0xeed9c0),
            (ColorSlot::Separator, 0xc5beaf),
            (ColorSlot::ProgressTrack, 0xe6dfd1),
            (ColorSlot::ProgressFill, 0xca6929),
            (ColorSlot::SliderTrack, 0xddd5c6),
            (ColorSlot::SliderFill, 0xbc5c23),
            (ColorSlot::SliderHandle, 0x854019),
            (ColorSlot::ScrollbarTrack, 0xeae5da),
            (ColorSlot::ScrollbarHandle, 0x827768),
            (ColorSlot::Canvas, 0xd9d5cb),
            (ColorSlot::CanvasDot, 0xbfb9ac),
        ],
    },
    BuiltinPalette {
        id: "mist-gray-lime",
        name: "Mist Gray Lime",
        values: [
            (ColorSlot::TitleBar, 0xdedfd7),
            (ColorSlot::MenuBar, 0xdedfd7),
            (ColorSlot::Toolbar, 0xe7e8df),
            (ColorSlot::OptionsBar, 0xf0f1e9),
            (ColorSlot::StatusBar, 0xe3e5da),
            (ColorSlot::Dock, 0xe0e3d8),
            (ColorSlot::Card, 0xf8f9f2),
            (ColorSlot::CardBorder, 0x9aa18b),
            (ColorSlot::PanelTitle, 0xe9ecdf),
            (ColorSlot::FloatingTitle, 0xdce2c7),
            (ColorSlot::MenuBackground, 0xfafbf4),
            (ColorSlot::MenuHover, 0xedf1d7),
            (ColorSlot::MenuSelection, 0xdce6a9),
            (ColorSlot::MenuText, 0x293020),
            (ColorSlot::Field, 0xfcfcf6),
            (ColorSlot::FieldBorder, 0x969e83),
            (ColorSlot::Hover, 0xe9eddc),
            (ColorSlot::Pressed, 0xd4dcb6),
            (ColorSlot::Text, 0x2c3227),
            (ColorSlot::TextDim, 0x59624c),
            (ColorSlot::TextFaint, 0x737b65),
            (ColorSlot::Icon, 0x58613a),
            (ColorSlot::Accent, 0xc1d837),
            (ColorSlot::AccentText, 0x273019),
            (ColorSlot::Selection, 0xe0e8ae),
            (ColorSlot::Separator, 0xc3cbb0),
            (ColorSlot::ProgressTrack, 0xe2e7d3),
            (ColorSlot::ProgressFill, 0x71851c),
            (ColorSlot::SliderTrack, 0xdbe1ce),
            (ColorSlot::SliderFill, 0x7b9025),
            (ColorSlot::SliderHandle, 0x4e631a),
            (ColorSlot::ScrollbarTrack, 0xedf0e3),
            (ColorSlot::ScrollbarHandle, 0x7a8561),
            (ColorSlot::Canvas, 0xdce0d2),
            (ColorSlot::CanvasDot, 0xbdc5ad),
        ],
    },
    BuiltinPalette {
        id: "bright-blue-yellow",
        name: "Bright Blue Yellow",
        values: [
            (ColorSlot::TitleBar, 0xd6e9f7),
            (ColorSlot::MenuBar, 0xd6e9f7),
            (ColorSlot::Toolbar, 0xe0edf7),
            (ColorSlot::OptionsBar, 0xedf4fa),
            (ColorSlot::StatusBar, 0xdce9f2),
            (ColorSlot::Dock, 0xdce6ed),
            (ColorSlot::Card, 0xf7fafc),
            (ColorSlot::CardBorder, 0x8ea9bb),
            (ColorSlot::PanelTitle, 0xdfecf5),
            (ColorSlot::FloatingTitle, 0xc8e2f5),
            (ColorSlot::MenuBackground, 0xfbfcfe),
            (ColorSlot::MenuHover, 0xe1effb),
            (ColorSlot::MenuSelection, 0xffe395),
            (ColorSlot::MenuText, 0x24394b),
            (ColorSlot::Field, 0xffffff),
            (ColorSlot::FieldBorder, 0x8ca5b8),
            (ColorSlot::Hover, 0xe4f0fa),
            (ColorSlot::Pressed, 0xc5dff2),
            (ColorSlot::Text, 0x243540),
            (ColorSlot::TextDim, 0x526675),
            (ColorSlot::TextFaint, 0x6c7e8a),
            (ColorSlot::Icon, 0x206a9e),
            (ColorSlot::Accent, 0x0063a6),
            (ColorSlot::AccentText, 0xffffff),
            (ColorSlot::Selection, 0xffe595),
            (ColorSlot::Separator, 0xb9cedc),
            (ColorSlot::ProgressTrack, 0xe7e5d7),
            (ColorSlot::ProgressFill, 0xd8ac19),
            (ColorSlot::SliderTrack, 0xd9e6ef),
            (ColorSlot::SliderFill, 0x1483c9),
            (ColorSlot::SliderHandle, 0x005e9c),
            (ColorSlot::ScrollbarTrack, 0xe9f0f5),
            (ColorSlot::ScrollbarHandle, 0x688699),
            (ColorSlot::Canvas, 0xd9e4eb),
            (ColorSlot::CanvasDot, 0xb4c8d5),
        ],
    },
    BuiltinPalette {
        id: "porcelain-cobalt",
        name: "Porcelain Cobalt",
        values: [
            (ColorSlot::TitleBar, 0xe5e9f5),
            (ColorSlot::MenuBar, 0xe5e9f5),
            (ColorSlot::Toolbar, 0xebeff9),
            (ColorSlot::OptionsBar, 0xf3f5fc),
            (ColorSlot::StatusBar, 0xe4e8f3),
            (ColorSlot::Dock, 0xe0e4ef),
            (ColorSlot::Card, 0xfcfcff),
            (ColorSlot::CardBorder, 0x9ba4bb),
            (ColorSlot::PanelTitle, 0xe9edf8),
            (ColorSlot::FloatingTitle, 0xd8e0f8),
            (ColorSlot::MenuBackground, 0xfdfdff),
            (ColorSlot::MenuHover, 0xe8edff),
            (ColorSlot::MenuSelection, 0xd0dcff),
            (ColorSlot::MenuText, 0x273353),
            (ColorSlot::Field, 0xffffff),
            (ColorSlot::FieldBorder, 0x929db8),
            (ColorSlot::Hover, 0xe8edfc),
            (ColorSlot::Pressed, 0xcad5f1),
            (ColorSlot::Text, 0x283047),
            (ColorSlot::TextDim, 0x56617c),
            (ColorSlot::TextFaint, 0x727d96),
            (ColorSlot::Icon, 0x3a5094),
            (ColorSlot::Accent, 0x2247c9),
            (ColorSlot::AccentText, 0xffffff),
            (ColorSlot::Selection, 0xd4dfff),
            (ColorSlot::Separator, 0xc3cbe0),
            (ColorSlot::ProgressTrack, 0xe4e7f0),
            (ColorSlot::ProgressFill, 0xbc691e),
            (ColorSlot::SliderTrack, 0xdce3f3),
            (ColorSlot::SliderFill, 0x3c61dc),
            (ColorSlot::SliderHandle, 0x233fa4),
            (ColorSlot::ScrollbarTrack, 0xecf0f9),
            (ColorSlot::ScrollbarHandle, 0x7684a8),
            (ColorSlot::Canvas, 0xdce1ed),
            (ColorSlot::CanvasDot, 0xb9c3d9),
        ],
    },
];

#[cfg(test)]
#[path = "builtin_tests_v69.rs"]
mod tests;
