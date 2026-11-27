//! Mixer and Smudge drags use the same engine command in an isolated session; document and tool state
//! are committed only on release. Cache by stroke contents with at most one render in flight.
use crate::{PhotocraftApp, state::Tool};
use photocraft_doc::Document;
use serde_json::json;
use std::{
    hash::{Hash, Hasher},
    sync::Arc,
};

#[derive(PartialEq)]
struct PaintSettings {
    brush: photocraft_engine::paint::BrushSettings,
    foreground: [f32; 4],
    background: [f32; 4],
}

pub(crate) struct MixerPreview {
    tool: Tool,
    settings: PaintSettings,
    settings_epoch: u64,
    key: u64,
    doc: Arc<Document>,
    source: photocraft_doc::DocId,
    revision: u64,
    area: photocraft_geom::Rect,
    keys: Vec<u64>,
    changes: Vec<photocraft_geom::Rect>,
    live: Option<photocraft_engine::brush_cmds::LiveMixer>,
    seed: Option<u64>,
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) struct PreviewJob {
    receiver: std::sync::mpsc::Receiver<Result<MixerPreview, String>>,
    valid: bool,
    tool: Tool,
}

/// Poll interval while a mixer worker is rendering. The worker publishes through a one-slot
/// channel and requests an immediate repaint when it finishes; this short fallback interval only
/// covers the case where completion lands just after a frame, keeping pen preview latency below a
/// display frame without rendering or polling when no job is active.
const PREVIEW_POLL: std::time::Duration = std::time::Duration::from_millis(4);

fn preview_tool(tool: Tool) -> bool {
    matches!(tool, Tool::MixerBrush | Tool::Smudge)
}

fn smudge_seed(params: &serde_json::Value) -> u64 {
    // The first point is stable while a stroke grows; hashing the whole path would
    // reshuffle sampled-tip jitter on every preview and again on release.
    let first = params.get("points").and_then(|p| p.get(0));
    photocraft_engine::paint::rng::seed_from_bytes(first.map(serde_json::Value::to_string).unwrap_or_default().as_bytes())
}

pub(crate) fn prepare_smudge_commit(app: &mut PhotocraftApp, params: &mut serde_json::Value) {
    params["seed"] = json!(smudge_seed(params));
    // Smudge is replayed by its command. Do not wait for an obsolete preview first.
    invalidate(app);
}

/// Keep a cancelled worker until it finishes: rapid strokes must never spawn an
/// unbounded number of expensive renders. Its result is discarded, without joining.
pub(crate) fn invalidate(app: &mut PhotocraftApp) {
    app.mixer_preview = None;
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(job) = &mut app.mixer_job {
        job.valid = false;
    }
}

pub(crate) fn tick(app: &mut PhotocraftApp, ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.mixer_repaint = Some(ctx.clone());
        if app.drag.as_ref().is_none_or(|d| !preview_tool(d.tool)) {
            invalidate(app);
        }
        if let Some(job) = &mut app.mixer_job
            && app.drag.as_ref().is_none_or(|d| d.tool != job.tool)
        {
            job.valid = false;
        }
        let Some(job) = &app.mixer_job else { return };
        let valid = job.valid;
        match job.receiver.try_recv() {
            Ok(result) => {
                app.mixer_job = None;
                match result {
                    Ok(preview) if valid && app.session.active().is_some_and(|st| st.doc.id == preview.source && st.revision == preview.revision) => {
                        install(app, preview);
                    }
                    Err(error) if valid => {
                        app.ui.status = error;
                        app.ui.status_error = true;
                    }
                    _ => {}
                }
                ctx.request_repaint();
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(PREVIEW_POLL),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                app.mixer_job = None;
            }
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (app, ctx);
}

fn install(app: &mut PhotocraftApp, mut preview: MixerPreview) {
    if let Some(old) = &app.mixer_preview
        && old.source == preview.source
        && old.revision == preview.revision
        && old.tool == preview.tool
    {
        preview.area = preview.area.union(&old.area);
        let current_change = preview.changes.first().copied().unwrap_or(preview.area);
        preview.changes = old.changes.clone();
        preview.changes.push(current_change);
        preview.keys = old.keys.clone();
        if preview.keys.len() >= 16 {
            preview.keys.remove(0);
            preview.changes.remove(0);
        }
        preview.keys.push(preview.key);
    }
    app.mixer_preview = Some(preview);
}

pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    if app.session.active_index() != Some(idx) {
        return None;
    }
    if app.mixer_preview.as_ref().is_some_and(|p| app.session.active().is_none_or(|st| st.doc.id != p.source || st.revision != p.revision)) {
        invalidate(app);
    }
    let Some(drag) = app.drag.as_ref().filter(|d| preview_tool(d.tool)) else {
        invalidate(app);
        return None;
    };
    let tool = drag.tool;
    if app.mixer_preview.as_ref().is_some_and(|p| p.tool != tool) {
        invalidate(app);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if app.mixer_job.is_some() {
        return app.mixer_preview.as_ref().map(|p| (p.doc.clone(), p.key));
    }
    let st = app.session.documents().get(idx)?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    st.doc.id.hash(&mut hash);
    st.revision.hash(&mut hash);
    tool.hash(&mut hash);
    // Bitmap tips are shared, so equality is cheap for the same imported brush.
    // Avoid serialising their megapixels into the per-frame cache hash.
    let settings = PaintSettings { brush: app.session.tools.brush.clone(), foreground: app.session.tools.foreground, background: app.session.tools.background };
    let settings_epoch = app.mixer_preview.as_ref().map_or(0, |p| p.settings_epoch.wrapping_add(u64::from(p.settings != settings)));
    settings_epoch.hash(&mut hash);
    if tool == Tool::Smudge {
        app.ui.tool_options.strength.to_bits().hash(&mut hash);
        app.ui.tool_options.finger_painting.hash(&mut hash);
        app.ui.tool_options.sample_all_layers.hash(&mut hash);
    } else if tool == Tool::MixerBrush {
        app.ui.sampling.mixer_sample_all_layers.hash(&mut hash);
    }
    crate::canvas::paint_target(app).to_string().hash(&mut hash);
    st.active_layer.hash(&mut hash);
    let points = app.stylus.stroke_points(&app.drag.as_ref()?.points);
    for point in &points {
        for value in point {
            value.to_bits().hash(&mut hash);
        }
    }
    let key = hash.finish() | (1 << 62);
    if let Some(p) = &app.mixer_preview
        && p.key == key
    {
        return Some((p.doc.clone(), p.key));
    }
    let mut session = photocraft_engine::Session::new();
    session.add_document((*st.doc).clone(), None);
    if let Some(active) = session.active_mut() {
        active.doc = st.doc.clone();
        active.revision = st.revision;
        active.channel_view = st.channel_view.clone();
    }
    // Painting reads the current brush and colours, never the preset library. Imported tip
    // and texture bitmaps can make that library hundreds of MB; don't copy it per preview.
    session.tools.brush = app.session.tools.brush.clone();
    session.tools.foreground = app.session.tools.foreground;
    session.tools.background = app.session.tools.background;
    session.tools.mixer = app.session.tools.mixer.clone();
    let source = st.doc.id;
    let revision = st.revision;
    let mut params = json!({"points":points,"layer":st.active_layer.map(|id|id.0),"target":crate::canvas::paint_target(app)});
    if tool == Tool::MixerBrush {
        params["sampleAllLayers"] = json!(app.ui.sampling.mixer_sample_all_layers);
    }
    if tool == Tool::Smudge {
        params["strength"] = json!(app.ui.tool_options.strength);
        params["fingerPainting"] = json!(app.ui.tool_options.finger_painting);
        params["sampleAllLayers"] = json!(app.ui.tool_options.sample_all_layers);
        params["seed"] = json!(smudge_seed(&params));
    } else if let Some(seed) = app.mixer_preview.as_ref().filter(|p| p.tool == tool).and_then(|p| p.seed) {
        params["seed"] = json!(seed);
    }
    let live = app
        .mixer_preview
        .as_mut()
        .filter(|p| p.tool == Tool::MixerBrush && tool == Tool::MixerBrush)
        .and_then(|p| p.live.take())
        .filter(|live| live.matches(&app.session, &params));
    let render = move || -> Result<MixerPreview, String> {
        let live = match live {
            Some(mut live) => {
                live.update(&params).map_err(|e| e.to_string())?;
                Some(live)
            }
            None if tool == Tool::MixerBrush => photocraft_engine::brush_cmds::LiveMixer::begin(&session, &params).ok(),
            None => None,
        };
        if let Some(mut live) = live {
            let (doc, area) = live.preview().map_err(|e| e.to_string())?;
            let seed = Some(live.seed);
            return Ok(MixerPreview {
                tool,
                settings,
                settings_epoch,
                key,
                doc,
                source,
                revision,
                area,
                keys: vec![key],
                changes: vec![area],
                live: Some(live),
                seed,
            });
        }
        let command = if tool == Tool::Smudge { "paint.smudge" } else { "paint.mixerBrush" };
        session.execute(command, params).map_err(|e| e.to_string())?;
        let preview = session.active().ok_or("Mixer preview document is unavailable")?;
        let doc = preview.doc.clone();
        let area = preview.last_damage.unwrap_or(doc.bounds());
        Ok(MixerPreview { tool, settings, settings_epoch, key, doc, source, revision, area, keys: vec![key], changes: vec![area], live: None, seed: None })
    };
    #[cfg(not(target_arch = "wasm32"))]
    if app.background_jobs {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let repaint = app.mixer_repaint.clone();
        match std::thread::Builder::new().name("mixer-preview".into()).spawn(move || {
            let _ = sender.send(render());
            if let Some(ctx) = repaint {
                ctx.request_repaint();
            }
        }) {
            Ok(_) => app.mixer_job = Some(PreviewJob { receiver, valid: true, tool }),
            Err(error) => {
                app.ui.status = error.to_string();
                app.ui.status_error = true;
            }
        }
        return app.mixer_preview.as_ref().map(|p| (p.doc.clone(), p.key));
    }
    install(app, render().ok()?);
    app.mixer_preview.as_ref().map(|p| (p.doc.clone(), p.key))
}
pub(crate) fn damage(app: &PhotocraftApp, doc: photocraft_doc::DocId, revision: u64, seen: u64, now: u64) -> Option<photocraft_geom::Rect> {
    let p = app.mixer_preview.as_ref().filter(|p| p.source == doc && p.revision == revision)?;
    let end = p.keys.iter().position(|key| *key == now)?;
    if seen == 0 {
        return Some(p.area);
    }
    let start = p.keys.iter().position(|key| *key == seen)?;
    if start > end {
        return None;
    }
    Some(p.changes.get(start.saturating_add(1)..=end).unwrap_or_default().iter().fold(photocraft_geom::Rect::EMPTY, |r, d| r.union(d)))
}
/// Reuse accumulated pixels. On release, wait only for the one outstanding batch;
/// never replay the whole stroke concurrently with a worker that is already painting it.
pub(crate) fn prepare_commit(app: &mut PhotocraftApp, params: &mut serde_json::Value) {
    params["sampleAllLayers"] = json!(app.ui.sampling.mixer_sample_all_layers);
    #[cfg(not(target_arch = "wasm32"))]
    if app.mixer_job.as_ref().is_some_and(|job| job.valid)
        && let Some(job) = app.mixer_job.take()
        && let Ok(Ok(preview)) = job.receiver.recv()
        && app.session.active().is_some_and(|st| st.doc.id == preview.source && st.revision == preview.revision)
    {
        install(app, preview);
    }
    if let Some(p) = &mut app.mixer_preview {
        if let Some(seed) = p.seed {
            params["seed"] = json!(seed);
        }
        if let Some(live) = p.live.take() {
            let _ = live.prepare(&mut app.session, params);
        }
    }
}
pub(crate) fn committed(app: &mut PhotocraftApp) {
    if let Some(p) = &app.mixer_preview {
        let source = p.source;
        let keys = p.keys.clone();
        crate::canvas::shown_as_document(app, source, |k| keys.contains(&k));
    }
    invalidate(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{ToolEvent, tool_event};

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn smudge_preview_shows_pixels_without_editing_and_matches_release() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":120,"height":100,"background":"white"})).unwrap();
        app.run("paint.stroke", json!({"points":[[20,10],[20,90]],"size":30,"color":"#ff0000","brush":{"pressureSize":false}})).unwrap();
        app.run("tools.setBrush", json!({"brush":{"size":24,"pressureSize":false,"shapeDynamics":{"enabled":true,"size":{"jitter":0.22}}}})).unwrap();
        app.ui.tool = Tool::Smudge;
        app.ui.tool_options.strength = 85.;
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        tool_event(&mut app, ToolEvent::Down { x: 20., y: 50., pressure: 0.4 }, egui::Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Move { x: 85., y: 50., pressure: 0.8 }, egui::Modifiers::NONE);
        assert!(display_doc(&mut app, 0).is_none());
        assert!(app.mixer_job.is_some());
        assert!(display_doc(&mut app, 0).is_none());
        await_preview(&mut app);
        let first_key = app.mixer_preview.as_ref().unwrap().key;
        assert_eq!(app.session.active().unwrap().doc, before);
        assert_ne!(app.mixer_preview.as_ref().unwrap().doc, before);
        // Changing an option with no added point must invalidate the preview key.
        app.ui.tool_options.strength = 55.;
        display_doc(&mut app, 0);
        await_preview(&mut app);
        assert_ne!(app.mixer_preview.as_ref().unwrap().key, first_key);
        let strength_key = app.mixer_preview.as_ref().unwrap().key;
        app.session.tools.brush.size = 18.;
        display_doc(&mut app, 0);
        await_preview(&mut app);
        assert_ne!(app.mixer_preview.as_ref().unwrap().key, strength_key);
        app.ui.tool_options.finger_painting = true;
        display_doc(&mut app, 0);
        await_preview(&mut app);
        let finger_key = app.mixer_preview.as_ref().unwrap().key;
        app.session.tools.foreground = [0., 1., 0., 1.];
        display_doc(&mut app, 0);
        await_preview(&mut app);
        assert_ne!(app.mixer_preview.as_ref().unwrap().key, finger_key);
        let preview = app.mixer_preview.as_ref().unwrap().doc.clone();
        tool_event(&mut app, ToolEvent::Up { x: 85., y: 50. }, egui::Modifiers::NONE);
        assert_eq!(app.session.active().unwrap().doc, preview);
        assert!(app.mixer_preview.is_none());
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, before);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, preview);
        // A still-running preview is cancelled on release, never awaited or installed
        // later over the command's final document.
        tool_event(&mut app, ToolEvent::Down { x: 20., y: 70., pressure: 0.4 }, egui::Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Move { x: 85., y: 70., pressure: 0.8 }, egui::Modifiers::NONE);
        display_doc(&mut app, 0);
        tool_event(&mut app, ToolEvent::Up { x: 85., y: 70. }, egui::Modifiers::NONE);
        let committed = app.session.active().unwrap().doc.clone();
        await_preview(&mut app);
        assert!(app.mixer_preview.is_none());
        assert_eq!(app.session.active().unwrap().doc, committed);
    }

    #[test]
    fn completed_preview_accepts_a_turn_without_a_forty_millisecond_pause() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":120,"height":100,"background":"transparent"})).unwrap();
        app.run("tools.setBrush", json!({"brush":{"size":14,"pressureSize":false}})).unwrap();
        app.ui.tool = Tool::MixerBrush;
        tool_event(&mut app, ToolEvent::Down { x: 10., y: 30., pressure: 0.4 }, egui::Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Move { x: 80., y: 30., pressure: 0.8 }, egui::Modifiers::NONE);
        let (_, first) = display_doc(&mut app, 0).unwrap();
        // Another turn arrives in the same frame-time bucket. Completed work must
        // not impose the former 40 ms cooldown on this new input.
        tool_event(&mut app, ToolEvent::Move { x: 80., y: 80., pressure: 0.5 }, egui::Modifiers::NONE);
        let (preview, second) = display_doc(&mut app, 0).unwrap();
        assert_ne!(first, second);
        assert!(preview.layers[0].surface().unwrap().rgba(80, 70)[3] > 0.);
        tool_event(&mut app, ToolEvent::Up { x: 80., y: 80. }, egui::Modifiers::NONE);
        assert_eq!(app.session.active().unwrap().doc, preview);
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn await_preview(app: &mut PhotocraftApp) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let ctx = egui::Context::default();
        while app.mixer_job.is_some() {
            tick(app, &ctx);
            assert!(std::time::Instant::now() < deadline, "preview worker timed out");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn background_preview_is_isolated_matches_commit_and_discards_cancelled_work() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":120,"height":60,"background":"transparent"})).unwrap();
        app.run("tools.setBrush", json!({"brush":{"size":14,"pressureSize":false}})).unwrap();
        app.ui.tool = Tool::MixerBrush;
        app.background_jobs = true;
        let before = app.session.active().unwrap().doc.clone();
        let tools = app.session.tools.mixer.clone();
        tool_event(&mut app, ToolEvent::Down { x: 10., y: 30., pressure: 0.2 }, egui::Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Move { x: 80., y: 30., pressure: 0.8 }, egui::Modifiers::NONE);
        assert!(display_doc(&mut app, 0).is_none());
        assert!(app.mixer_job.is_some());
        assert!(display_doc(&mut app, 0).is_none(), "only one job may be in flight");
        await_preview(&mut app);
        let preview = app.mixer_preview.as_ref().unwrap().doc.clone();
        assert_eq!(app.session.active().unwrap().doc, before);
        assert_eq!(app.session.tools.mixer, tools);
        tool_event(&mut app, ToolEvent::Up { x: 80., y: 30. }, egui::Modifiers::NONE);
        assert_eq!(app.session.active().unwrap().doc, preview);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, before);
        tool_event(&mut app, ToolEvent::Down { x: 10., y: 30., pressure: 1. }, egui::Modifiers::NONE);
        display_doc(&mut app, 0);
        invalidate(&mut app);
        assert!(app.mixer_job.is_some(), "invalidating never starts a concurrent worker");
        await_preview(&mut app);
        assert!(app.mixer_preview.is_none(), "a cancelled result cannot reappear");
    }

    #[test]
    #[ignore = "24 MP release benchmark; cargo test --release -p photocraft-ui-egui mixer_preview_dispatch_perf -- --ignored --nocapture"]
    #[cfg(not(target_arch = "wasm32"))]
    fn mixer_preview_dispatch_perf() {
        for length in [300., 1200.] {
            for background in [false, true] {
                let mut dispatch = vec![];
                let mut total = vec![];
                for _ in 0..3 {
                    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                    app.run("file.new", json!({"width":6000,"height":4000,"background":"white"})).unwrap();
                    app.run("tools.setBrush", json!({"brush":{"size":200,"spacing":0.01,"pressureSize":false}})).unwrap();
                    app.ui.tool = Tool::MixerBrush;
                    app.background_jobs = background;
                    tool_event(&mut app, ToolEvent::Down { x: 2000., y: 2000., pressure: 1. }, egui::Modifiers::NONE);
                    tool_event(&mut app, ToolEvent::Move { x: 2000. + length, y: 2000., pressure: 1. }, egui::Modifiers::NONE);
                    let start = std::time::Instant::now();
                    display_doc(&mut app, 0);
                    dispatch.push(start.elapsed().as_secs_f64() * 1000.);
                    await_preview(&mut app);
                    total.push(start.elapsed().as_secs_f64() * 1000.);
                    assert!(app.mixer_preview.is_some());
                }
                dispatch.sort_by(f64::total_cmp);
                total.sort_by(f64::total_cmp);
                println!(
                    "length={length}, background={background}, UI_dispatch_median_ms={:.3}, completed_median_ms={:.3}, dispatch={dispatch:?}, total={total:?}",
                    dispatch[1], total[1]
                );
            }
        }
    }
    #[test]
    fn preview_does_not_edit_or_drain_tools_and_release_matches_it() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":120,"height":60,"background":"transparent"})).unwrap();
        app.run("tools.setBrush", json!({"brush":{"size":14,"pressureSize":false,"mixer":{"simulatePaint":true}}})).unwrap();
        app.ui.tool = Tool::MixerBrush;
        let before = app.session.active().unwrap().doc.clone();
        let tools = app.session.tools.mixer.clone();
        tool_event(&mut app, ToolEvent::Down { x: 10., y: 30., pressure: 1. }, egui::Modifiers::NONE);
        tool_event(&mut app, ToolEvent::Move { x: 80., y: 30., pressure: 1. }, egui::Modifiers::NONE);
        let (preview, key) = display_doc(&mut app, 0).unwrap();
        assert!(preview.layers[0].surface().unwrap().rgba(50, 30)[3] > 0.1);
        assert_eq!(app.session.active().unwrap().doc, before);
        assert_eq!(app.session.tools.mixer, tools);
        assert!(damage(&app, before.id, app.session.active().unwrap().revision, 0, key).is_some());
        tool_event(&mut app, ToolEvent::Up { x: 80., y: 30. }, egui::Modifiers::NONE);
        assert_eq!(app.session.active().unwrap().doc, preview);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc, before);
    }
}

#[cfg(test)]
mod incremental_perf_tests {
    use super::*;
    use crate::canvas::{ToolEvent, tool_event};
    #[test]
    #[ignore = "24 MP release curved-path mixer completion benchmark"]
    fn mixer_turning_updates_perf() {
        for smoothing in [0., 0.2] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width":6000,"height":4000,"background":"white"})).unwrap();
            app.run(
                "tools.setBrush",
                json!({"brush":{
                    "size":100,"spacing":0.01,"pressureSize":false,
                    "shapeDynamics":{"enabled":true,"size":{"control":"penPressure","minimum":0.47,"jitter":0.22},"angle":{"control":"direction"}},
                    "smoothing":{"amount":smoothing},"texture":{"enabled":true,"depth":0.2},
                    "mixer":{"wet":1.,"mix":0.,"flow":0.43}
                }}),
            )
            .unwrap();
            app.ui.tool = Tool::MixerBrush;
            tool_event(&mut app, ToolEvent::Down { x: 2200., y: 2000., pressure: 0.5 }, egui::Modifiers::NONE);
            let mut times = Vec::new();
            let mut last = (2200., 2000.);
            for frame in 1..=40 {
                for substep in 0..4 {
                    let t = f64::from((frame - 1) * 4 + substep + 1) * 0.025;
                    last = (2000. + 200. * t.cos(), 2000. + 140. * (t * 2.).sin());
                    tool_event(&mut app, ToolEvent::Move { x: last.0, y: last.1, pressure: 0.5 + t.sin() as f32 * 0.4 }, egui::Modifiers::NONE);
                }
                app.last_frame_time += 1. / 60.;
                let start = std::time::Instant::now();
                assert!(display_doc(&mut app, 0).is_some());
                times.push(start.elapsed().as_secs_f64() * 1000.);
            }
            let start = std::time::Instant::now();
            tool_event(&mut app, ToolEvent::Up { x: last.0, y: last.1 }, egui::Modifiers::NONE);
            let release = start.elapsed().as_secs_f64() * 1000.;
            let total: f64 = times.iter().sum();
            times.sort_by(f64::total_cmp);
            println!(
                "turning_24mp smoothing={smoothing}, synchronous_update_median_ms={:.3}, p95_ms={:.3}, total_ms={total:.3}, release_ms={release:.3}",
                times[20], times[38]
            );
        }
    }
    #[test]
    #[ignore = "release incremental versus whole-stroke replay benchmark"]
    fn incremental_updates_and_release_perf() {
        for incremental in [false, true] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width":6000,"height":4000,"background":"white"})).unwrap();
            app.run("tools.setBrush", json!({"brush":{"size":200,"spacing":0.01,"pressureSize":false,"mixer":{"wet":0,"flow":0.43}}})).unwrap();
            app.ui.tool = Tool::MixerBrush;
            tool_event(&mut app, ToolEvent::Down { x: 2000., y: 2000., pressure: 1. }, egui::Modifiers::NONE);
            let mut times = vec![];
            for i in 1..=20 {
                tool_event(&mut app, ToolEvent::Move { x: 2000. + i as f64 * 60., y: 2000., pressure: 1. }, egui::Modifiers::NONE);
                app.last_frame_time += 0.1;
                let start = std::time::Instant::now();
                if incremental {
                    display_doc(&mut app, 0);
                } else {
                    let mut session = photocraft_engine::Session::new();
                    session.add_document((**app.session.active().map(|s| &s.doc).unwrap()).clone(), None);
                    session.tools.brush = app.session.tools.brush.clone();
                    session
                        .execute("paint.mixerBrush", json!({"points":app.stylus.stroke_points(&app.drag.as_ref().unwrap().points),"target":"pixels"}))
                        .unwrap();
                }
                times.push(start.elapsed().as_secs_f64() * 1000.);
            }
            let start = std::time::Instant::now();
            tool_event(&mut app, ToolEvent::Up { x: 3200., y: 2000. }, egui::Modifiers::NONE);
            let release = start.elapsed().as_secs_f64() * 1000.;
            println!(
                "incremental={incremental}, first_ms={:.3}, last_ms={:.3}, total_updates_ms={:.3}, release_ms={release:.3}",
                times[0],
                times[19],
                times.iter().sum::<f64>()
            );
        }
    }
}

#[cfg(test)]
mod damage_tests {
    use super::*;
    use photocraft_geom::Rect;
    #[test]
    fn preview_damage_tracks_only_updates_since_the_displayed_key() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":100,"height":100})).unwrap();
        let st = app.session.active().unwrap();
        let (doc, source, revision) = (st.doc.clone(), st.doc.id, st.revision);
        for key in 1..=3 {
            let area = Rect::new(key as i32 * 10, 0, key as i32 * 10 + 20, 20);
            install(
                &mut app,
                MixerPreview {
                    tool: Tool::MixerBrush,
                    settings: PaintSettings { brush: Default::default(), foreground: [0.; 4], background: [0.; 4] },
                    settings_epoch: 0,
                    key,
                    doc: doc.clone(),
                    source,
                    revision,
                    area,
                    keys: vec![key],
                    changes: vec![area],
                    live: None,
                    seed: None,
                },
            );
        }
        assert_eq!(damage(&app, source, revision, 2, 3), Some(Rect::new(30, 0, 50, 20)));
        assert_eq!(damage(&app, source, revision, 1, 3), Some(Rect::new(20, 0, 50, 20)));
        assert_eq!(damage(&app, source, revision, 0, 3), Some(Rect::new(10, 0, 50, 20)));
        assert_eq!(damage(&app, source, revision, 3, 3), Some(Rect::EMPTY));
        assert_eq!(damage(&app, source, revision, 99, 3), None);
    }
}
