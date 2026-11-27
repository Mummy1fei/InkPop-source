//! Lazy surface identities inside immutable document snapshots, without a tile walk per UI frame.
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_raster::{Surface, Tile};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
};

#[derive(Default)]
pub(crate) struct ThumbFingerprintCache {
    documents: HashMap<DocId, Snapshot>,
}

struct Snapshot {
    values: HashMap<(LayerId, u8), Fingerprint>,
}

struct Fingerprint {
    source: Weak<Document>,
    value: u64,
    /// Prevent in-place mutation/ABA of unique Tile Arcs without keeping pixel data alive.
    _tiles: Vec<Weak<Tile>>,
}

impl ThumbFingerprintCache {
    /// `surface` must belong to `source` and `(layer, kind)` must identify that surface there.
    /// Weak document and tile witnesses make Arc::make_mut dissociate both allocations, even
    /// when a single-strong document moves rather than clones its uniquely owned tile map.
    pub(crate) fn fingerprint(&mut self, source: &Arc<Document>, layer: LayerId, kind: u8, surface: &Surface) -> u64 {
        let snapshot = self.documents.entry(source.id).or_insert_with(|| Snapshot { values: HashMap::new() });
        let weak = Arc::downgrade(source);
        if let Some(cached) = snapshot.values.get(&(layer, kind))
            && cached.source.ptr_eq(&weak)
        {
            return cached.value;
        }
        // Keep other surfaces' witnesses until their own thumbnail is revalidated. A hidden
        // layer can survive several unrelated edits before its unique tile is changed in place.
        let value = crate::surface_fingerprint(surface);
        let tiles = surface.tiles().map(|(_, tile)| Arc::downgrade(tile)).collect();
        snapshot.values.insert((layer, kind), Fingerprint { source: weak, value, _tiles: tiles });
        value
    }

    /// Closed documents and stale live previews do not accumulate cache entries or retain pixels.
    pub(crate) fn prune(&mut self, documents: impl IntoIterator<Item = DocId>) {
        let live: HashSet<_> = documents.into_iter().collect();
        self.documents.retain(|id, _| live.contains(id));
    }

    /// Reuse the shell's existing live thumbnail-key union after its layer-tree traversal.
    pub(crate) fn retain_keys(&mut self, live: &HashSet<(LayerId, u8)>) {
        for snapshot in self.documents.values_mut() {
            snapshot.values.retain(|key, _| live.contains(key));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::Layer;
    use photocraft_geom::{Rect, Size};

    fn document() -> Arc<Document> {
        let mut doc = Document::new("Fingerprint fixture", Size::new(6000, 4000), ColorMode::Rgb, SampleType::U8);
        let mut layer = Layer::raster("Content", doc.pixel_format());
        layer.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 100, 100), &[0.2, 0.4, 0.6, 1.]);
        doc.layers.push(layer);
        Arc::new(doc)
    }

    fn get(cache: &mut ThumbFingerprintCache, source: &Arc<Document>) -> u64 {
        let layer = &source.layers[0];
        cache.fingerprint(source, layer.id, 0, layer.surface().unwrap())
    }

    #[test]
    fn stable_snapshot_is_cached_and_does_not_retain_pixels() {
        let doc = document();
        let mut cache = ThumbFingerprintCache::default();
        let expected = get(&mut cache, &doc);
        let key = (doc.layers[0].id, 0);
        // A sentinel proves the fast path doesn't recompute, independent of wall-clock timings.
        cache.documents.get_mut(&doc.id).unwrap().values.get_mut(&key).unwrap().value = expected ^ 1;
        assert_eq!(get(&mut cache, &doc), expected ^ 1);
        assert_eq!(Arc::strong_count(&doc), 1);
        let tile = doc.layers[0].surface().unwrap().tiles().next().unwrap().1;
        assert_eq!(Arc::strong_count(tile), 1);
        assert!(Arc::weak_count(tile) > 0, "identity witnesses must be weak");
        drop(doc);
        cache.prune(std::iter::empty());
        assert!(cache.documents.is_empty());
    }

    #[test]
    fn make_mut_invalidates_even_without_a_revision_or_second_strong_reference() {
        let mut doc = document();
        let mut cache = ThumbFingerprintCache::default();
        let before = get(&mut cache, &doc);
        let old = Arc::downgrade(&doc);
        assert!(Arc::get_mut(&mut doc).is_none(), "cached Weak must protect the snapshot");
        Arc::make_mut(&mut doc).layers[0].surface_mut().unwrap().fill_rect(Rect::new(10, 10, 20, 20), &[1., 0., 0., 1.]);
        assert!(!old.ptr_eq(&Arc::downgrade(&doc)));
        let after = get(&mut cache, &doc);
        assert_ne!(before, after);
        assert_eq!(after, crate::surface_fingerprint(doc.layers[0].surface().unwrap()));
    }

    #[test]
    fn preserved_id_reopen_and_undo_replacements_recompute() {
        let original = document();
        let mut cache = ThumbFingerprintCache::default();
        let before = get(&mut cache, &original);
        let mut reopened = Arc::new((*original).clone());
        Arc::make_mut(&mut reopened).layers[0].surface_mut().unwrap().fill_rect(Rect::new(5, 5, 10, 10), &[0.8, 0.1, 0.2, 1.]);
        assert_eq!(reopened.id, original.id);
        assert_ne!(get(&mut cache, &reopened), before);
        assert_eq!(get(&mut cache, &original), before);
    }

    #[test]
    fn layers_masks_and_document_ids_have_independent_entries() {
        let mut doc = document();
        Arc::make_mut(&mut doc).layers[0].mask = Some(photocraft_doc::LayerMask::reveal_all());
        let other = document();
        let mut cache = ThumbFingerprintCache::default();
        let layer = &doc.layers[0];
        let mask = layer.mask.as_ref().unwrap();
        let expected = get(&mut cache, &doc);
        let masked = cache.fingerprint(&doc, layer.id, 1, &mask.surface);
        assert_ne!(expected, masked);
        let other_expected = get(&mut cache, &other);
        assert_eq!(get(&mut cache, &doc), expected);
        assert_eq!(get(&mut cache, &other), other_expected);
        assert_eq!(cache.documents.len(), 2);
    }

    #[test]
    fn changed_default_mask_pixels_invalidate_even_with_no_allocated_tiles() {
        let mut doc = document();
        Arc::make_mut(&mut doc).layers[0].mask = Some(photocraft_doc::LayerMask::reveal_all());
        let mut cache = ThumbFingerprintCache::default();
        let layer = &doc.layers[0];
        let surface = &layer.mask.as_ref().unwrap().surface;
        assert_eq!(surface.tile_count(), 0);
        let before = cache.fingerprint(&doc, layer.id, 1, surface);
        Arc::make_mut(&mut doc).layers[0].mask = Some(photocraft_doc::LayerMask::hide_all());
        let layer = &doc.layers[0];
        let surface = &layer.mask.as_ref().unwrap().surface;
        assert_eq!(surface.tile_count(), 0);
        let after = cache.fingerprint(&doc, layer.id, 1, surface);
        assert_ne!(before, after, "white and black default coverage are different pixels");
    }

    #[test]
    fn hidden_surface_witness_survives_an_unrelated_snapshot_replacement() {
        let mut doc = document();
        let mut other = Layer::raster("Hidden content", doc.pixel_format());
        other.surface_mut().unwrap().fill_rect(Rect::new(200, 200, 230, 230), &[0.8, 0.2, 0.1, 1.]);
        Arc::make_mut(&mut doc).layers.push(other);
        let mut cache = ThumbFingerprintCache::default();
        let unchanged = get(&mut cache, &doc);
        let hidden = &doc.layers[1];
        let before = cache.fingerprint(&doc, hidden.id, 0, hidden.surface().unwrap());
        Arc::make_mut(&mut doc).name = "Unrelated metadata edit".into();
        // The shell knows this document is still open even though the old snapshot moved.
        cache.prune([doc.id]);
        assert_eq!(get(&mut cache, &doc), unchanged, "unrelated edits reuse the existing texture fingerprint");
        Arc::make_mut(&mut doc).layers[1].surface_mut().unwrap().fill_rect(Rect::new(205, 205, 215, 215), &[0.1, 0.7, 0.4, 1.]);
        let hidden = &doc.layers[1];
        let after = cache.fingerprint(&doc, hidden.id, 0, hidden.surface().unwrap());
        assert_ne!(before, after, "hidden surface keeps its tile identity witnesses until revalidation");
    }

    #[test]
    fn removed_keys_and_closed_documents_release_all_identity_witnesses() {
        let doc = document();
        let other = document();
        let mut cache = ThumbFingerprintCache::default();
        get(&mut cache, &doc);
        get(&mut cache, &other);
        let layer = &doc.layers[0];
        let tile = layer.surface().unwrap().tiles().next().unwrap().1;
        assert!(Arc::weak_count(tile) > 0);
        cache.retain_keys(&HashSet::from([(other.layers[0].id, 0)]));
        assert_eq!(Arc::weak_count(tile), 0);
        cache.prune([doc.id]);
        assert_eq!(cache.documents.len(), 1);
        assert!(cache.documents[&doc.id].values.is_empty());
        cache.prune(std::iter::empty());
        assert!(cache.documents.is_empty());
    }
}
