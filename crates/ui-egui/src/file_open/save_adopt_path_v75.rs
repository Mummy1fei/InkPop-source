//! Adopt a successfully written main-document identity, never an export/copy identity.
use std::sync::Arc;

use photocraft_doc::{DocId, Document};

use crate::PhotocraftApp;

fn saved_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().filter(|name| !name.is_empty()).unwrap_or(path)
}

/// Native .pcraft embeds Document::name. Encode the destination identity in
/// an isolated COW snapshot without changing the live document before write success.
pub(crate) fn export_named_snapshot(doc: &Arc<Document>, path: &str) -> Arc<Document> {
    let mut snapshot = Arc::clone(doc);
    let name = saved_name(path);
    if snapshot.name != name {
        Arc::make_mut(&mut snapshot).name = name.to_string();
    }
    snapshot
}

impl PhotocraftApp {
    /// Call only after the encoder and platform writer both succeeded. `saved_revision`
    /// is the revision actually encoded; a later change must remain dirty.
    pub(crate) fn adopt_saved_path(&mut self, document: DocId, saved_revision: u64, path: &str) -> Result<(), String> {
        let state = self.session.active_mut().ok_or("the saved document was closed")?;
        if state.doc.id != document || saved_revision > state.revision {
            return Err("the saved document changed; select the saved document before saving again".into());
        }
        let name = saved_name(path);
        if state.doc.name != name {
            Arc::make_mut(&mut state.doc).name = name.to_string();
        }
        state.path = Some(path.to_string());
        state.saved_revision = saved_revision;
        self.push_recent(path);
        Ok(())
    }
}

#[cfg(test)]
#[path = "save_adopt_path_tests_v75.rs"]
mod tests;
