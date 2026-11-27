//! Real painted-glyph regression for menu check columns, not prefix assertions.
use super::*;
use egui::{Rect, Shape, pos2, vec2};

fn text_shapes<'a>(shape: &'a Shape, texts: &mut Vec<&'a egui::epaint::TextShape>) {
    match shape {
        Shape::Text(text) => texts.push(text),
        Shape::Vec(shapes) => {
            for shape in shapes {
                text_shapes(shape, texts);
            }
        }
        _ => {}
    }
}

fn glyph_x(shapes: &[egui::epaint::ClippedShape], label: &str) -> f32 {
    let mut texts = Vec::new();
    for shape in shapes {
        text_shapes(&shape.shape, &mut texts);
    }
    let text = texts.into_iter().find(|text| text.galley.job.text == label).unwrap_or_else(|| panic!("missing painted label {label}"));
    let row = text.galley.rows.first().unwrap();
    text.pos.x + row.pos.x + row.glyphs.first().unwrap().pos.x
}

fn fixture_items(checked: bool, depth: usize) -> Vec<MenuItem> {
    let path = vec!["View".to_string(); depth];
    let item = |id: &str, label: &str, checked, shortcut: Option<&str>, enabled| MenuItem {
        id: id.into(),
        label: label.into(),
        path: path.clone(),
        shortcut: shortcut.map(str::to_string),
        enabled,
        checked,
        color: None,
    };
    let mut items = vec![
        item("plain", "Plain command", None, None, true),
        item("checked", "Checked option", Some(checked), Some("Cmd+H"), true),
        item("disabled", "Disabled command", None, None, false),
    ];
    let mut child = item("child", "Child option", Some(!checked), Some("Cmd+R"), true);
    child.path.push("Submenu".into());
    items.push(child);
    items
}

fn embedded_rows(ui: &mut egui::Ui, checked: bool, depth: usize) {
    let items = fixture_items(checked, depth);
    let refs: Vec<_> = items.iter().collect();
    let mut nav = crate::menu_nav::Nav::default();
    let mut clicked = None;
    render_level_rows(ui, &refs, depth, &mut clicked, &mut nav);
}

fn native_rows(ui: &mut egui::Ui, checked: bool) {
    ui.spacing_mut().item_spacing.y = 0.0;
    for (label, shortcut, mark, enabled) in
        [("Plain command", "", false, true), ("Checked option", "Ctrl+H", checked, true), ("Disabled command", "", false, false), ("Submenu", "▶", false, true)]
    {
        ui.add_enabled_ui(enabled, |ui| {
            let button = crate::native_menu_ui::row_button(ui, label.into(), shortcut.into(), false, mark);
            ui.add_sized(vec2(ui.available_width(), 27.0), button);
        });
    }
}

fn native_frame(ctx: &egui::Context, checked: bool) -> egui::FullOutput {
    ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(340.0, 180.0))), ..Default::default() }, |ui| {
        egui::CentralPanel::default().frame(crate::native_menu_ui::menu_frame(ui.style())).show(ui, |ui| native_rows(ui, checked));
    })
}

#[test]
fn native_menu_glyphs_share_one_column_and_do_not_move_when_checked() {
    for theme in crate::theme::ThemeKind::ALL {
        for dpi in [1.0, 1.5] {
            let ctx = egui::Context::default();
            PhotocraftApp::setup_context(&ctx, theme);
            ctx.set_pixels_per_point(dpi);
            // Font/DPI installation is deferred until a pass. Establish an
            // unchanged baseline before testing a check transition.
            let mut warm = Vec::new();
            for pass in 0..5 {
                let mut output = native_frame(&ctx, false);
                warm.push((pass, glyph_x(&output.shapes, "Plain command"), glyph_x(&output.shapes, "Ctrl+H"), ctx.pixels_per_point()));
                output.textures_delta.clear();
            }
            eprintln!("menu coordinate warmup {theme:?} requested-dpi={dpi}: {warm:?}");
            assert!((warm[3].2 - warm[4].2).abs() < 0.01, "{theme:?} dpi={dpi}: unchanged layout must settle before check testing: {warm:?}");
            let mut previous: Option<f32> = None;
            let mut shortcut_x: Option<f32> = None;
            let mut transitions = Vec::new();
            for checked in [false, true, false] {
                let mut left = 0.0;
                for pass in 0..4 {
                    let mut output = native_frame(&ctx, checked);
                    left = glyph_x(&output.shapes, "Plain command");
                    for label in ["Checked option", "Disabled command", "Submenu"] {
                        assert!((glyph_x(&output.shapes, label) - left).abs() < 0.01, "{theme:?} dpi={dpi} {label}");
                    }
                    if checked {
                        assert!(glyph_x(&output.shapes, "✔") < left - 2.0, "indicator must stay inside its separate column");
                    }
                    let shortcut = glyph_x(&output.shapes, "Ctrl+H");
                    transitions.push((checked, pass, left, shortcut, ctx.pixels_per_point()));
                    if let Some(previous) = shortcut_x {
                        assert!(
                            (shortcut - previous).abs() < 0.01,
                            "{theme:?} dpi={dpi}: shortcut must not move with check state; previous={previous}, current={shortcut}; warm={warm:?}, transitions={transitions:?}"
                        );
                    }
                    shortcut_x = Some(shortcut);
                    output.textures_delta.clear();
                }
                if let Some(previous) = previous {
                    assert!((left - previous).abs() < 0.01, "{theme:?}: check transitions must preserve glyph x");
                }
                previous = Some(left);
            }
            eprintln!("menu coordinate transitions {theme:?} requested-dpi={dpi}: {transitions:?}");
        }
    }
}

#[test]
fn embedded_menu_leaves_and_submenus_share_the_same_painted_column_at_each_depth() {
    for theme in crate::theme::ThemeKind::ALL {
        for depth in [1, 2] {
            let ctx = egui::Context::default();
            PhotocraftApp::setup_context(&ctx, theme);
            let mut previous: Option<f32> = None;
            for checked in [false, true, false] {
                let mut left = 0.0;
                for _ in 0..4 {
                    let mut output =
                        ctx.run_ui(egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 240.0))), ..Default::default() }, |ui| {
                            egui::Frame::menu(ui.style()).show(ui, |ui| {
                                ui.set_width(320.0);
                                embedded_rows(ui, checked, depth);
                            });
                        });
                    left = glyph_x(&output.shapes, "Plain command");
                    for label in ["Checked option", "Disabled command", "Submenu"] {
                        assert!((glyph_x(&output.shapes, label) - left).abs() < 0.01, "{theme:?} depth={depth} {label}");
                    }
                    output.textures_delta.clear();
                }
                if let Some(previous) = previous {
                    assert!((left - previous).abs() < 0.01);
                }
                previous = Some(left);
            }
        }
    }
}

#[test]
fn community_actions_are_hidden_from_all_shared_visible_menus_but_remain_dispatchable() {
    let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let items = menu_items(&app);
    for id in ["help.discord", "help.website", "help.artcraftWebsite", "help.github", "help.reportIssue"] {
        assert!(!items.iter().any(|item| item.id == id));
        assert!(is_live(id), "legacy command availability must remain independent of visible chrome");
        assert!(crate::links::url_for(id).is_some());
    }
    let help: Vec<_> = items.iter().filter(|item| item.path == ["Help"]).collect();
    assert!(!help.is_empty());
    assert_ne!(help.first().unwrap().label, "---");
    assert_eq!(items.iter().find(|item| item.id == "help.about").unwrap().label, "About InkPop");
    assert!(search_items(&items, "Discord", crate::i18n::Lang::EN).is_empty());
    let mac = crate::native_menu::photocraft_layout(&items, crate::i18n::Lang::EN, "auto");
    assert!(mac.bar.find("help.discord").is_none());
    assert!(mac.bar.find("help.about").is_some());
}

struct MenuVisual;

impl eframe::App for MenuVisual {
    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal_top(|ui| {
                for native in [true, false] {
                    ui.vertical(|ui| {
                        ui.set_width(340.0);
                        ui.label(if native { "Native menu rows" } else { "Embedded menu rows" });
                        egui::Frame::menu(ui.style()).show(ui, |ui| {
                            ui.set_width(320.0);
                            if native {
                                native_rows(ui, true);
                            } else {
                                embedded_rows(ui, true, 1);
                            }
                        });
                    });
                }
            });
        });
    }
}

#[test]
#[ignore = "GPU v67 menu column captures; root must visually inspect the PNGs"]
fn menu_alignment_gpu_visual_v67() {
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evidence/v67/menu-alignment-gpu");
    assert!(!output.exists(), "preserve prior captures");
    std::fs::create_dir_all(&output).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        let mut h = egui_kittest::Harness::builder().with_size(vec2(720.0, 230.0)).wgpu().build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, theme);
            // This minimal app never overwrites the chosen context theme.
            MenuVisual
        });
        h.run_steps(5);
        h.render().unwrap().save(output.join(format!("{}-menu-columns.png", theme.id()))).unwrap();
    }
}
