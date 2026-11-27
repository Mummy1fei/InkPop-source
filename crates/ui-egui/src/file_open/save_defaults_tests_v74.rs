use std::cell::RefCell;
use std::rc::Rc;

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, TextLayer};
use photocraft_geom::{Rect, Size};
use serde_json::{Value, json};

use super::*;
use crate::{PhotocraftApp, Services, menus};

type Writes = Rc<RefCell<Vec<(String, Vec<u8>)>>>;
type Suggestions = Rc<RefCell<Vec<String>>>;

#[derive(Clone, Copy)]
enum Choice {
    Suggested,
    Explicit(&'static str),
    Cancel,
}

fn document(extra_layer: bool) -> Document {
    let mut doc = Document::with_background("测试画布00000001.png", Size::new(4, 4), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    if extra_layer {
        let mut hidden = Layer::raster("hidden paint", PixelFormat::RGBA8);
        hidden.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 1, 1), &[1.0, 0.0, 0.0, 1.0]);
        hidden.visible = false;
        doc.layers.push(hidden);
    }
    doc
}

fn app(doc: Document, path: Option<&str>, choice: Choice) -> (PhotocraftApp, Suggestions, Writes) {
    let suggestions: Suggestions = Rc::default();
    let writes: Writes = Rc::default();
    let picks = suggestions.clone();
    let written = writes.clone();
    let services = Services {
        pick_save: Some(Box::new(move |suggested| {
            picks.borrow_mut().push(suggested.to_string());
            match choice {
                Choice::Suggested => Some(suggested.to_string()),
                Choice::Explicit(path) => Some(path.to_string()),
                Choice::Cancel => None,
            }
        })),
        export: Some(Box::new(|doc, path, _| {
            let result = photocraft_io::export(doc, path, &Default::default()).map_err(|error| error.to_string())?;
            Ok((result.bytes, result.warnings))
        })),
        write: Some(Box::new(move |path, bytes| {
            written.borrow_mut().push((path.to_string(), bytes.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.session.add_document(doc, path.map(str::to_owned));
    (app, suggestions, writes)
}

fn invoke(app: &mut PhotocraftApp, id: &str) -> Value {
    menus::invoke(app, &egui::Context::default(), id, json!({})).unwrap()
}

#[test]
fn layered_flat_save_suggests_psd_and_keeps_the_hidden_layer_in_real_bytes_v74() {
    for command in ["file.save", "file.saveAs"] {
        let source = "C:\\work.v1\\测试画布00000001.PNG";
        let expected = "C:\\work.v1\\测试画布00000001.psd";
        let (mut app, suggestions, writes) = app(document(true), Some(source), Choice::Suggested);
        let response = invoke(&mut app, command);
        assert_eq!(*suggestions.borrow(), [expected]);
        assert_eq!(response["path"], expected);
        assert_eq!(response["warnings"], json!([]));
        assert!(app.ui.notices.is_empty(), "normal layered save should not report flattening");
        let writes = writes.borrow();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].0, expected, "the source PNG must not be overwritten");
        assert!(photocraft_io::is_psd(&writes[0].1));
        let reopened = photocraft_io::import(expected, &writes[0].1).unwrap().document;
        assert_eq!(reopened.layer_count(), 2, "real PSD keeps the two separate layers");
        assert!(reopened.layers.iter().any(|layer| layer.name == "hidden paint" && !layer.visible));
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some(expected));
    }
}

#[test]
fn nested_hidden_and_text_layers_count_even_with_one_root_group_v74() {
    let mut doc = document(false);
    let mut text = Layer::new("empty editable type", LayerContent::Text(TextLayer::default()));
    text.visible = false;
    let mut children = std::mem::take(&mut doc.layers);
    children.push(text);
    doc.layers.push(Layer::group("outer", vec![Layer::group("inner", children)]));
    assert_eq!(doc.layers.len(), 1);
    let (mut app, suggestions, writes) = app(doc, Some("/pictures/map.jpg"), Choice::Cancel);
    assert_eq!(menus::invoke(&mut app, &egui::Context::default(), "file.save", json!({})).unwrap_err(), "cancelled");
    assert_eq!(*suggestions.borrow(), ["/pictures/map.psd"]);
    assert!(writes.borrow().is_empty());
}

#[test]
fn unassigned_documents_still_default_to_psd_and_single_flat_sources_keep_their_format_v74() {
    for extra in [false, true] {
        let doc = document(extra);
        assert_eq!(suggested_save_path(&doc, None), "测试画布00000001.psd");
    }
    for path in ["/pics/one.png", "C:\\pictures.with.dots\\image.JPG", "/pics/one.tiff"] {
        assert_eq!(suggested_save_path(&document(false), Some(path)), path);
    }
    for path in ["/pics/photo.JPEG", "/pics/photo.webp", "/pics/photo.tif"] {
        assert_eq!(suggested_save_path(&document(true), Some(path)), "/pics/photo.psd");
    }
    assert_eq!(suggested_save_path(&document(true), Some("C:\\work.v1\\未命名")), "C:\\work.v1\\未命名.psd");
    assert_eq!(suggested_save_path(&document(true), Some("/pics/.hidden")), "/pics/.hidden.psd");
    let (mut app, suggestions, writes) = app(document(false), Some("/pics/one.png"), Choice::Suggested);
    let response = invoke(&mut app, "file.save");
    assert!(suggestions.borrow().is_empty(), "ordinary single-image Save reuses the known PNG path");
    assert_eq!(response["path"], "/pics/one.png");
    assert_eq!(response["warnings"], json!([]));
    assert!(writes.borrow()[0].1.starts_with(b"\x89PNG\r\n\x1a\n"));
}

#[test]
fn an_existing_layered_path_saves_in_place_without_opening_a_picker_v74() {
    for path in ["/pics/map.PSD", "/pics/map.psb", "/pics/map.pcraft"] {
        let (mut app, suggestions, writes) = app(document(true), Some(path), Choice::Cancel);
        let response = invoke(&mut app, "file.save");
        assert_eq!(response["path"], path);
        assert!(suggestions.borrow().is_empty());
        assert_eq!(writes.borrow()[0].0, path);
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some(path));
        // Save As keeps the established layered extension too.
        app.services.pick_save = Some(Box::new(|suggestion| Some(suggestion.to_string())));
        assert_eq!(invoke(&mut app, "file.saveAs")["path"], path);
    }
}

#[test]
fn explicit_png_copy_warns_about_flattening_and_save_writes_back_to_the_original_psd() {
    let (mut app, suggestions, writes) = app(document(true), Some("/pics/source.psd"), Choice::Explicit("/pics/flattened.png"));
    let response = invoke(&mut app, "file.saveAs");
    assert_eq!(*suggestions.borrow(), ["/pics/source.psd"]);
    assert_eq!(response["path"], "/pics/flattened.png");
    assert!(response["warnings"].as_array().unwrap().iter().any(|warning| warning.as_str().unwrap_or_default().contains("flattened")));
    assert!(app.ui.status.contains("layers, masks and blend modes are not kept"));
    assert!(app.ui.notices.is_empty());
    assert!(writes.borrow()[0].1.starts_with(b"\x89PNG\r\n\x1a\n"));
    let picked: Suggestions = Rc::default();
    let got = picked.clone();
    app.services.pick_save = Some(Box::new(move |suggested| {
        got.borrow_mut().push(suggested.to_string());
        Some(suggested.to_string())
    }));
    assert_eq!(invoke(&mut app, "file.save")["path"], "/pics/source.psd");
    assert!(picked.borrow().is_empty());
    assert!(photocraft_io::is_psd(&writes.borrow()[1].1));
}

#[test]
fn cancelling_psd_save_never_writes_changes_path_or_marks_the_document_saved_v74() {
    let path = "/pictures/source.png";
    let (mut app, suggestions, writes) = app(document(true), Some(path), Choice::Cancel);
    app.run("layer.new.layer", json!({"name":"unsaved edit"})).unwrap();
    let before = app.session.active().unwrap();
    let (revision, saved_revision) = (before.revision, before.saved_revision);
    assert_ne!(revision, saved_revision);
    assert_eq!(menus::invoke(&mut app, &egui::Context::default(), "file.save", json!({})).unwrap_err(), "cancelled");
    assert_eq!(*suggestions.borrow(), ["/pictures/source.psd"]);
    let after = app.session.active().unwrap();
    assert_eq!((after.revision, after.saved_revision), (revision, saved_revision));
    assert_eq!(after.path.as_deref(), Some(path));
    assert!(writes.borrow().is_empty());
}

#[test]
fn explicit_export_png_keeps_its_format_warning_and_document_psd_path_v74() {
    let (mut app, suggestions, writes) = app(document(true), Some("/pics/master.psd"), Choice::Suggested);
    let id = crate::export_dialog::open(&mut app).unwrap();
    let fields = app.ui.dialogs.iter().find(|dialog| dialog.id == id).unwrap().fields.clone();
    let response = crate::export_dialog::confirm(&mut app, &fields).unwrap();
    assert_eq!(*suggestions.borrow(), ["测试画布00000001.png"]);
    assert_eq!(response["path"], "测试画布00000001.png");
    assert!(!response["warnings"].as_array().unwrap().is_empty(), "explicit flat export still tells the user what is flattened");
    assert!(writes.borrow()[0].1.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/master.psd"));
}
