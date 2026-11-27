use super::*;

#[test]
fn shortcut_assignment_notices_are_translated_without_changing_command_names_v75() {
    for lang in Lang::all().filter(|lang| lang.complete_menus()) {
        for source in ["{shortcut} assigned.", "{shortcut} is already in use by {commands} and will be removed from it when you click OK."] {
            assert!(has(lang, source), "{}: {source}", lang.code());
            let message = fmt(tr(lang, source), &[("shortcut", "Ctrl+K"), ("commands", "User {custom} command")]);
            assert!(message.contains("Ctrl+K"));
            if source.contains("{commands}") {
                assert!(message.contains("User {custom} command"));
            }
            assert_ne!(tr(lang, source), source);
        }
    }
}

#[test]
fn chinese_recognition_and_open_progress_stages_have_real_translations() {
    const STAGES: &[&str] = &[
        "Recognizing image objects",
        "Loading local selection model",
        "Reading image",
        "Tracing selected object",
        "Snapping selection to image edges",
        "Preparing recognition image",
        "Loading local recognition model",
        "Recognizing sky with SkySeg U2NETP",
        "Recognizing foreground with U2NETP",
        "Analyzing classical selection cues",
        "Recognition mask ready",
        "Finding subject",
        "Refining edge",
        "Reading",
        "Decoding",
        "Importing brushes",
        "Photomerge: aligning",
        "Photomerge: warping",
        "Photomerge: blending",
        "Photomerge: filling edges",
    ];
    for code in ["zh-hans", "zh-hant"] {
        let lang = Lang::from_code(code).unwrap();
        for stage in STAGES {
            assert!(has(lang, stage), "{code}: missing dynamic stage {stage}");
            assert_ne!(tr(lang, stage), *stage, "{code}: fallback English stage {stage}");
        }
    }
}

#[test]
fn imported_names_are_kept_when_completion_messages_are_localized() {
    for code in ["zh-hans", "zh-hant"] {
        let lang = Lang::from_code(code).unwrap();
        let group = "Brush Tool / 用户 {custom}";
        let message = fmt(tr(lang, "Imported {n} {what} from {group}"), &[("n", "7"), ("what", tr(lang, "Tool Presets")), ("group", group)]);
        assert!(message.contains(group) && message.contains('7'));
        assert!(!message.starts_with("Imported"));
        let file = "Background 用户.png";
        assert!(fmt(tr(lang, "Opened {name}"), &[("name", file)]).contains(file));
    }
}

#[test]
fn dynamic_tool_and_menu_color_labels_have_plain_translations_v75() {
    for lang in Lang::all().filter(|lang| lang.complete_menus()) {
        for tool in crate::state::Tool::ALL {
            assert!(has(lang, tool.label()), "{}: missing dynamic tool label {}", lang.code(), tool.label());
        }
        for (source, capitalized) in [
            ("none", "None"),
            ("red", "Red"),
            ("orange", "Orange"),
            ("yellow", "Yellow"),
            ("green", "Green"),
            ("blue", "Blue"),
            ("violet", "Violet"),
            ("gray", "Gray"),
        ] {
            assert!(has(lang, source), "{}: missing raw menu colour value {source}", lang.code());
            let canonical = if capitalized == "Violet" && !has(lang, capitalized) { "Purple" } else { capitalized };
            assert_eq!(tr(lang, source), tr(lang, canonical), "{}: the raw value must use the existing colour translation", lang.code());
        }
    }
}
