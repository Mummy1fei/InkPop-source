//! Camera Raw recipe controls. Mutations run after the dialog borrow ends.
use crate::PhotocraftApp;
use photocraft_engine::camera_raw_preset_cmds::{MAX_PRESET_BYTES, PRESET_FORMAT, RawPreset};
use serde_json::{Value, json};

#[derive(Default)]
pub(super) struct State {
    pub name: String,
    pub selected: Option<String>,
    pub message: Option<String>,
    confirm_delete: bool,
}
pub(super) enum Action {
    Save(String),
    Load(String),
    Delete(String),
    Import,
    Export(String),
    ImportData(Value),
    ExportData(String),
}
fn export_data(app: &mut PhotocraftApp, name: String) -> Result<Value, String> {
    let value = app.run("cameraRaw.presets.get", json!({"name":name}))?;
    let preset: RawPreset = serde_json::from_value(value).map_err(|e| format!("Invalid preset: {e}"))?;
    preset.export_value()
}
fn import_data(app: &mut PhotocraftApp, value: Value) -> Result<Value, String> {
    let object = value.as_object().ok_or("预设必须是对象")?;
    if object.keys().any(|key| !matches!(key.as_str(), "format" | "name" | "params")) || value.get("format").and_then(Value::as_str) != Some(PRESET_FORMAT) {
        return Err("不是有效的 InkPop Camera Raw 预设文件".into());
    }
    if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > MAX_PRESET_BYTES {
        return Err("预设文件超过 1 MiB".into());
    }
    app.run("cameraRaw.presets.save", json!({"name":value["name"],"params":value["params"]}))
}
pub(super) fn panel(ui: &mut egui::Ui, state: &mut State, presets: &[RawPreset]) -> Option<Action> {
    let mut action = None;
    ui.vertical(|ui| {
        crate::widgets::section_label(ui, tl!("Presets"));
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut state.name).hint_text("预设名称").desired_width(175.0).char_limit(80));
            if ui.button("保存").clicked() {
                action = Some(Action::Save(state.name.trim().into()));
            }
        });
        egui::ComboBox::from_id_salt("raw-preset-list").selected_text(state.selected.as_deref().unwrap_or("选择预设")).width(240.0).show_ui(ui, |ui| {
            for preset in presets {
                if ui.selectable_label(state.selected.as_deref() == Some(&preset.name), &preset.name).clicked() {
                    state.selected = Some(preset.name.clone());
                    state.confirm_delete = false;
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.add_enabled(state.selected.is_some(), egui::Button::new("载入")).clicked()
                && let Some(name) = &state.selected
            {
                action = Some(Action::Load(name.clone()));
            }
            if ui.add_enabled(state.selected.is_some(), egui::Button::new("删除")).clicked() {
                state.confirm_delete = true;
            }
            if ui.button("导入…").clicked() {
                action = Some(Action::Import);
            }
            if ui.add_enabled(state.selected.is_some(), egui::Button::new("导出…")).clicked()
                && let Some(name) = &state.selected
            {
                action = Some(Action::Export(name.clone()));
            }
        });
        if state.confirm_delete {
            ui.horizontal_wrapped(|ui| {
                ui.label("删除所选预设？");
                if ui.button("确认删除").clicked()
                    && let Some(name) = &state.selected
                {
                    action = Some(Action::Delete(name.clone()));
                    state.confirm_delete = false;
                }
                if ui.button("保留").clicked() {
                    state.confirm_delete = false;
                }
            });
        }
        if let Some(message) = &state.message {
            ui.label(message);
        }
        ui.small("预设包含全局调整和蒙版；保存不提交到图像。");
    });
    action
}
pub(super) fn execute(app: &mut PhotocraftApp, ctx: &egui::Context, action: Action) -> Result<Value, String> {
    let persists = matches!(&action, Action::Save(_) | Action::Delete(_) | Action::Import | Action::ImportData(_));
    let result = match action {
        Action::Save(name) => {
            let d = app.camera_raw.as_ref().ok_or("Camera Raw is closed")?;
            let params = d.command_params();
            app.run("cameraRaw.presets.save", json!({"name":name,"params":params}))?
        }
        Action::Load(name) => {
            let value = app.run("cameraRaw.presets.get", json!({"name":name}))?;
            let mut params =
                serde_json::from_value(value.get("params").cloned().ok_or("Missing preset settings")?).map_err(|e| format!("Invalid preset: {e}"))?;
            if let Some(d) = app.camera_raw.as_mut() {
                d.bind_color_source(&mut params, true)?;
                d.params = params;
                d.curve_state = Default::default();
                d.masks = Default::default();
                d.dirty = true;
                d.queue_render(ctx);
            }
            value
        }
        Action::Delete(name) => {
            let value = app.run("cameraRaw.presets.delete", json!({"name":name}))?;
            if let Some(d) = app.camera_raw.as_mut() {
                d.presets.selected = None;
            }
            value
        }
        Action::Import => {
            let (_, bytes) = app.pick_file_bytes().ok_or("已取消导入")??;
            if bytes.len() > MAX_PRESET_BYTES {
                return Err("预设文件超过 1 MiB".into());
            }
            let value: Value = serde_json::from_slice(&bytes).map_err(|e| format!("无法读取预设：{e}"))?;
            import_data(app, value)?
        }
        Action::Export(name) => {
            let value = export_data(app, name)?;
            let bytes = serde_json::to_vec(&value).map_err(|e| e.to_string())?;
            let path = app.services.pick_save.as_mut().and_then(|f| f("CameraRaw.pcrawpreset")).ok_or("已取消导出")?;
            app.services.write.as_mut().ok_or("无法写入文件")?(&path, &bytes)?;
            json!({"path":path})
        }
        Action::ImportData(value) => import_data(app, value)?,
        Action::ExportData(name) => export_data(app, name)?,
    };
    if persists && app.services.save_prefs.is_some() {
        crate::prefs_ui::save_preferences(app).map_err(|e| format!("预设更改保留在内存中，写入失败：{e}。软件将重试保存。"))?;
    }
    let message = if persists && app.services.save_prefs.is_none() { "已保存到本次会话（当前环境未配置持久存储）" } else { "已完成" };
    if let Some(d) = app.camera_raw.as_mut() {
        d.presets.message = Some(message.into());
    }
    Ok(result)
}

#[cfg(test)]
mod branding_tests_v78 {
    use super::*;

    #[test]
    fn invalid_recipe_reports_inkpop_and_retains_legacy_format_compatibility_v78() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let error = import_data(&mut app, json!({"format":"unknown","name":"Invalid","params":{}})).unwrap_err();
        assert_eq!(error, "不是有效的 InkPop Camera Raw 预设文件");
        // The display-name change must not invalidate an older recipe's protocol marker.
        let value = json!({"format":PRESET_FORMAT,"name":"Legacy-compatible recipe","params":{}});
        import_data(&mut app, value).unwrap();
        let exported = export_data(&mut app, "Legacy-compatible recipe".into()).unwrap();
        assert_eq!(exported["format"], PRESET_FORMAT);
        assert_eq!(exported["name"], "Legacy-compatible recipe");
    }
}
