//! Validated edits of a Camera Raw dialog's draft mask list. No document/history side effects.
use crate::commands::CommandSpec;
use crate::presets::{always, bad};
use crate::{Result, Session};
use photocraft_algo::camera_raw_masks::{self as masks, LocalAdjustment, MaskComponent, MaskShape};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const EDIT: &str = "cameraRaw.masks.edit";

/// Group operations are independent of toolkit selection, dialogs and document ownership.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum MaskOperation {
    Rename { index: usize, name: String },
    Invert { index: usize },
    Duplicate { index: usize, invert: bool },
    Intersect { index: usize, shape: MaskShape },
    SetEnabled { index: usize, enabled: bool },
    SetPrimaryEnabled { index: usize, enabled: bool },
    Delete { index: usize },
    ResetAdjustments { index: usize },
    DeleteCurrentIfEmpty { index: usize },
    DeleteAllEmpty,
    DeleteAll,
    SetComponentEnabled { index: usize, component: usize, enabled: bool },
    RenameComponent { index: usize, component: usize, name: String },
    DeleteComponent { index: usize, component: usize },
}

/// Internally tagged serde unit variants do not enforce `deny_unknown_fields`.
/// Compare the supplied keys to the canonical variant so every operation, including
/// unit operations, rejects unknown fields before any draft is installed.
pub fn parse_operation(value: Value) -> std::result::Result<MaskOperation, String> {
    let operation: MaskOperation = serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
    let canonical = serde_json::to_value(&operation).map_err(|error| error.to_string())?;
    let input = value.as_object().ok_or("Mask operation must be an object")?;
    let allowed = canonical.as_object().ok_or("Invalid serialized mask operation")?;
    if let Some(key) = input.keys().find(|key| !allowed.contains_key(*key)) {
        return Err(format!("Unknown mask operation property {key}"));
    }
    Ok(operation)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EditResult {
    #[serde(deserialize_with = "masks::deserialize_items")]
    pub items: Vec<LocalAdjustment>,
    /// Each old group maps to its new index, or None when removed.
    pub old_to_new: Vec<Option<usize>>,
    /// Explicit operation target/new duplicate. Bulk deletion leaves selection remapping to caller.
    pub preferred_selection: Option<usize>,
}

fn target(items: &[LocalAdjustment], index: usize) -> std::result::Result<&LocalAdjustment, String> {
    items.get(index).ok_or_else(|| "Mask index out of range".into())
}

/// Retain whole UTF-8 characters while leaving room for a generated suffix.
fn suffixed_name(name: &str, suffix: &str) -> String {
    let available = 256usize.saturating_sub(suffix.len());
    let mut result = String::new();
    for c in name.chars() {
        if result.len().saturating_add(c.len_utf8()) > available {
            break;
        }
        result.push(c);
    }
    result.push_str(suffix);
    result
}

fn duplicate_name(name: &str, items: &[LocalAdjustment], invert: bool) -> std::result::Result<String, String> {
    let suffix = if invert { " 反转副本" } else { " 副本" };
    // At most MAX_MASKS existing names can collide; this bounded extra candidate must be free.
    for number in 1..=masks::MAX_MASKS.saturating_add(1) {
        let suffix = if number == 1 { suffix.into() } else { format!("{suffix} {number}") };
        let candidate = suffixed_name(name, &suffix);
        if items.iter().all(|item| item.name != candidate) {
            return Ok(candidate);
        }
    }
    Err("Could not allocate a distinct mask name".into())
}

fn reset_adjustments(item: &LocalAdjustment) -> LocalAdjustment {
    LocalAdjustment {
        name: item.name.clone(),
        enabled: item.enabled,
        invert: item.invert,
        mask: item.mask.clone(),
        mask_enabled: item.mask_enabled,
        components: item.components.clone(),
        ..Default::default()
    }
}

/// Apply one atomic model transaction. Both input and complete output must satisfy the
/// same aggregate budgets as rendering/presets. On Err the borrowed input is untouched.
pub fn apply(items: &[LocalAdjustment], operation: &MaskOperation) -> std::result::Result<EditResult, String> {
    masks::validate(items)?;
    let mut result = EditResult { items: items.to_vec(), old_to_new: (0..items.len()).map(Some).collect(), preferred_selection: None };
    match operation {
        MaskOperation::Rename { index, name } => {
            target(items, *index)?;
            if let Some(item) = result.items.get_mut(*index) {
                item.name = name.clone();
            }
            result.preferred_selection = Some(*index);
        }
        MaskOperation::Invert { index } => {
            target(items, *index)?;
            if let Some(item) = result.items.get_mut(*index) {
                item.invert = !item.invert;
            }
            result.preferred_selection = Some(*index);
        }
        MaskOperation::Duplicate { index, invert } => {
            let source = target(items, *index)?;
            if items.len() >= masks::MAX_MASKS {
                return Err("Mask limit reached (64)".into());
            }
            let mut duplicate = source.clone();
            duplicate.name = duplicate_name(&source.name, items, *invert)?;
            if *invert {
                duplicate.invert = !duplicate.invert;
            }
            let insertion = index.saturating_add(1);
            result.items.insert(insertion, duplicate);
            for mapped in result.old_to_new.iter_mut().flatten() {
                if *mapped >= insertion {
                    *mapped = mapped.saturating_add(1);
                }
            }
            result.preferred_selection = Some(insertion);
        }
        MaskOperation::Intersect { index, shape } => {
            let source = target(items, *index)?;
            if source.components.len() >= masks::MAX_COMPONENTS {
                return Err("Component limit reached (32)".into());
            }
            if let Some(item) = result.items.get_mut(*index) {
                item.components.push(MaskComponent {
                    name: format!("相交区域 {}", source.components.len().saturating_add(1)),
                    enabled: true,
                    subtract: false,
                    intersect: true,
                    invert_previous: source.invert,
                    mask: shape.clone(),
                });
                item.invert = false;
            }
            result.preferred_selection = Some(*index);
        }
        MaskOperation::SetEnabled { index, enabled } => {
            target(items, *index)?;
            if let Some(item) = result.items.get_mut(*index) {
                item.enabled = *enabled;
            }
            result.preferred_selection = Some(*index);
        }
        MaskOperation::SetPrimaryEnabled { index, enabled } => {
            target(items, *index)?;
            if let Some(item) = result.items.get_mut(*index) {
                item.mask_enabled = *enabled;
            }
            result.preferred_selection = Some(*index);
        }
        MaskOperation::ResetAdjustments { index } => {
            let source = target(items, *index)?;
            if let Some(item) = result.items.get_mut(*index) {
                *item = reset_adjustments(source);
            }
            result.preferred_selection = Some(*index);
        }
        MaskOperation::Delete { index } | MaskOperation::DeleteCurrentIfEmpty { index } => {
            let item = target(items, *index)?;
            let remove = matches!(operation, MaskOperation::Delete { .. }) || masks::group_is_empty(item);
            if remove {
                result.items.remove(*index);
                for (old, mapped) in result.old_to_new.iter_mut().enumerate() {
                    *mapped = if old == *index { None } else { Some(if old > *index { old.saturating_sub(1) } else { old }) };
                }
            } else {
                result.preferred_selection = Some(*index);
            }
        }
        MaskOperation::DeleteAllEmpty => {
            let mut next = 0usize;
            for (item, mapped) in items.iter().zip(&mut result.old_to_new) {
                if masks::group_is_empty(item) {
                    *mapped = None;
                } else {
                    *mapped = Some(next);
                    next = next.saturating_add(1);
                }
            }
            result.items.retain(|item| !masks::group_is_empty(item));
        }
        MaskOperation::DeleteAll => {
            result.items.clear();
            result.old_to_new.fill(None);
        }
        MaskOperation::SetComponentEnabled { index, component, enabled } => {
            let group = result.items.get_mut(*index).ok_or("Mask index out of range")?;
            let item = group.components.get_mut(*component).ok_or("Mask component index out of range")?;
            item.enabled = *enabled;
            result.preferred_selection = Some(*index);
        }
        MaskOperation::RenameComponent { index, component, name } => {
            let group = result.items.get_mut(*index).ok_or("Mask index out of range")?;
            let item = group.components.get_mut(*component).ok_or("Mask component index out of range")?;
            item.name = name.clone();
            result.preferred_selection = Some(*index);
        }
        MaskOperation::DeleteComponent { index, component } => {
            let group = result.items.get_mut(*index).ok_or("Mask index out of range")?;
            if *component >= group.components.len() {
                return Err("Mask component index out of range".into());
            }
            let removed = group.components.remove(*component);
            if removed.invert_previous {
                if let Some(next) = group.components.get_mut(*component) {
                    next.invert_previous = !next.invert_previous;
                } else {
                    group.invert = !group.invert;
                }
            }
            result.preferred_selection = Some(*index);
        }
    }
    masks::validate(&result.items)?;
    Ok(result)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    #[serde(deserialize_with = "masks::deserialize_items")]
    items: Vec<LocalAdjustment>,
    operation: Value,
}

fn run(_: &mut Session, params: &Value) -> Result<Value> {
    let request: Request = serde_json::from_value(params.clone()).map_err(|error| bad(EDIT, error.to_string()))?;
    let operation = parse_operation(request.operation).map_err(|error| bad(EDIT, error))?;
    let result = apply(&request.items, &operation).map_err(|error| bad(EDIT, error))?;
    serde_json::to_value(result).map_err(|error| bad(EDIT, error.to_string()))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: EDIT,
        label: "Edit Camera Raw Masks",
        menu: &[],
        shortcut: None,
        params: r#"{"items":[LocalAdjustment],"operation":{"kind":"rename|invert|duplicate|intersect|setEnabled|setPrimaryEnabled|delete|resetAdjustments|deleteCurrentIfEmpty|deleteAllEmpty|deleteAll|setComponentEnabled|renameComponent|deleteComponent",...}} -> {items,oldToNew,preferredSelection}; validates a draft only"#,
        enabled: always,
        run,
        journal: false,
    }]
}

#[cfg(test)]
#[path = "camera_raw_mask_cmds_tests_v65.rs"]
mod tests_v65;

#[cfg(test)]
#[path = "camera_raw_mask_cmds_tests_v66.rs"]
mod tests_v66;
