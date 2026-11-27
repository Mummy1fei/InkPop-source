//! Shared last-used tools for toolbar slots and effective shortcut groups.
//! Only the selected tool is recorded; modifier-held effective tools are views.

use serde::Deserializer;
use serde::de::{Error, IgnoredAny, SeqAccess, Visitor};

use crate::state::{Tool, UiState};

pub(crate) fn remember(ui: &mut UiState) {
    let tool = ui.tool;
    if ui.tool_history.first() == Some(&tool) && ui.tool_history.len() <= Tool::ALL.len() {
        return;
    }
    ui.tool_history.retain(|previous| *previous != tool);
    ui.tool_history.insert(0, tool);
    ui.tool_history.truncate(Tool::ALL.len());
}

pub(crate) fn select(ui: &mut UiState, tool: Tool) {
    remember(ui);
    ui.tool = tool;
    remember(ui);
}

/// Membership comes from the caller: toolbar slots and custom key groups need
/// not contain the same tools. Never recall an item removed from that group.
pub(crate) fn recalled(ui: &UiState, group: &[Tool]) -> Option<Tool> {
    ui.tool_history.iter().copied().find(|tool| group.contains(tool))
}

pub(crate) fn displayed(ui: &UiState, current: Tool, group: &[Tool]) -> Tool {
    if group.contains(&current) { current } else { recalled(ui, group).or_else(|| group.first().copied()).unwrap_or(current) }
}

/// Bound state-file collection growth before allocation, also for old files
/// with no history field (the enclosing field uses serde(default)).
pub(crate) fn deserialize_history<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Tool>, D::Error> {
    struct History;
    impl<'de> Visitor<'de> for History {
        type Value = Vec<Tool>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at most {} unique selected tools", Tool::ALL.len())
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
            let mut tools = Vec::with_capacity(Tool::ALL.len());
            for _ in 0..Tool::ALL.len() {
                match sequence.next_element()? {
                    Some(tool) if tools.contains(&tool) => return Err(A::Error::custom("duplicate selected tool in history")),
                    Some(tool) => tools.push(tool),
                    None => return Ok(tools),
                }
            }
            if sequence.next_element::<IgnoredAny>()?.is_some() {
                return Err(A::Error::custom("too many selected tools in history"));
            }
            Ok(tools)
        }
    }
    deserializer.deserialize_seq(History)
}
