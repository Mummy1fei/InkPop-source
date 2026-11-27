//! A minimized document relinquishes editing focus without restoring its native window.
use super::*;

fn history_id() -> Id {
    Id::new("visible-floating-document-focus-history-v75")
}

fn root_visible(ctx: &egui::Context) -> bool {
    ctx.input(|input| {
        input.raw.viewports.get(&ViewportId::ROOT).is_none_or(|root| {
            root.minimized != Some(true) && root.visible() != Some(false) && !root.inner_rect.is_some_and(|rect| rect.width() < 16.0 || rect.height() < 16.0)
        })
    })
}

pub(super) fn available(app: &PhotocraftApp, ctx: &egui::Context, document: usize) -> bool {
    if app.session.documents().get(document).is_none() || !root_visible(ctx) {
        return false;
    }
    let Some(window) = app.ui.windows.iter().find(|window| window.open && window.document == document) else { return true };
    if manually_minimized(ctx, window.id) {
        return false;
    }
    let viewport = ViewportId::from_hash_of(("docwin", window.id));
    ctx.input(|input| input.raw.viewports.get(&viewport).is_none_or(|info| info.minimized != Some(true) && info.visible() != Some(false)))
}

pub(super) fn remember(app: &PhotocraftApp, ctx: &egui::Context, document: usize) {
    let Some(id) = app.session.documents().get(document).map(|state| state.doc.id.0) else { return };
    ctx.data_mut(|data| {
        let mut history = data.get_temp::<Vec<u64>>(history_id()).unwrap_or_default();
        history.retain(|old| *old != id);
        history.push(id);
        if history.len() > 32 {
            history.remove(0);
        }
        data.insert_temp(history_id(), history);
    });
}

/// Only an explicit child minimize hands off focus. Minimizing the whole application keeps
/// the original document active. With no visible document left, retain the last document so
/// Window > Float in Window can restore it; its hidden canvas remains input-disabled.
pub(super) fn reconcile(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if !root_visible(ctx)
        || app.ui.transform.is_some()
        || app.discard.is_some()
        || app.camera_raw.is_some()
        || app.distort.liquify.is_some()
        || !app.ui.dialogs.is_empty()
    {
        return;
    }
    let Some(active) = app.session.active_index() else { return };
    if !app.ui.windows.iter().any(|window| window.open && window.document == active && manually_minimized(ctx, window.id)) {
        return;
    }
    // Native minimize can interrupt a press before the child receives its release. Finish
    // the recorded gesture in the old document; a hidden canvas cannot receive that release.
    if let Some(drag) = app.drag.take() {
        crate::canvas::finish_gesture(app, drag);
    }
    let docked = crate::canvas::root_document_v74::index(app, ctx);
    let history = ctx.data(|data| data.get_temp::<Vec<u64>>(history_id())).unwrap_or_default();
    let recent = history
        .into_iter()
        .rev()
        .find_map(|id| app.session.documents().iter().position(|state| state.doc.id.0 == id).filter(|index| available(app, ctx, *index)));
    let visible = app.ui.windows.iter().rev().find_map(|window| {
        let viewport = ViewportId::from_hash_of(("docwin", window.id));
        let exists = ctx.input(|input| input.raw.viewports.contains_key(&viewport));
        (window.open && exists && available(app, ctx, window.document)).then_some(window.document)
    });
    let Some(next) = docked.or(recent).or(visible) else { return };
    // Finish text in its own document before changing Session's implicit command target.
    crate::type_tool::commit(app);
    if app.ui.text_edit.is_some() {
        return;
    }
    app.session.set_active(next);
    app.sync_views();
    ctx.request_repaint_of(ViewportId::ROOT);
}

#[cfg(test)]
#[path = "focus_tests_v75.rs"]
mod tests;
