//! Per-window bitmap ownership and application state. No OS calls or per-frame allocation.

use egui::CustomCursorImage;

pub(super) fn lifecycle_invalidates_cursor(event: &winit::event::WindowEvent) -> bool {
    use winit::event::WindowEvent;
    matches!(
        event,
        WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::Focused(_)
            | WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
            | WindowEvent::Occluded(_)
    )
}

pub(super) struct CustomCursorCache<T> {
    entry: Option<(CustomCursorImage, T)>,
    applied: bool,
}

impl<T> Default for CustomCursorCache<T> {
    fn default() -> Self {
        Self { entry: None, applied: false }
    }
}

impl<T> CustomCursorCache<T> {
    pub(super) fn get(&self, image: &CustomCursorImage) -> Option<&T> {
        self.entry.as_ref().and_then(|(source, cursor)| {
            (std::sync::Arc::ptr_eq(&source.rgba, &image.rgba) && source.size == image.size && source.hotspot == image.hotspot).then_some(cursor)
        })
    }

    pub(super) fn is_applied(&self, image: &CustomCursorImage) -> bool {
        self.applied && self.get(image).is_some()
    }

    pub(super) fn store(&mut self, image: &CustomCursorImage, cursor: T) {
        // Keep the Arc alive: a bare address could be recycled for another image.
        self.entry = Some((image.clone(), cursor));
        self.applied = false;
    }

    pub(super) fn mark_applied(&mut self) {
        self.applied = true;
    }

    pub(super) fn invalidate(&mut self) {
        // Retain the native bitmap, but re-apply its shape and visibility next time.
        self.applied = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn image() -> CustomCursorImage {
        CustomCursorImage { rgba: Arc::from([255; 16]), size: [2, 2], hotspot: [0, 0] }
    }

    #[test]
    fn warm_cursor_applies_once_instead_of_every_frame() {
        let image = image();
        let mut cache = CustomCursorCache::default();
        let mut uploads = 0;
        let mut applies = 0;
        for _ in 0..240 {
            if cache.is_applied(&image) {
                continue;
            }
            if cache.get(&image).is_none() {
                uploads += 1;
                cache.store(&image, 7);
            }
            applies += 1;
            cache.mark_applied();
        }
        assert_eq!((uploads, applies), (1, 1));
        println!("CURSOR_CACHE_COUNTS frames=240 legacy_uploads=1 legacy_applies=240 candidate_uploads={uploads} candidate_applies={applies}");
    }

    #[test]
    fn invalidated_cursor_reapplies_without_upload() {
        let image = image();
        let mut cache = CustomCursorCache::default();
        cache.store(&image, 7);
        cache.mark_applied();
        // Each lifecycle invalidation must independently permit a fresh OS apply.
        for _ in 0..8 {
            cache.invalidate();
            assert!(!cache.is_applied(&image));
            assert_eq!(cache.get(&image), Some(&7));
            cache.mark_applied();
            assert!(cache.is_applied(&image));
        }
    }

    #[test]
    fn window_lifecycle_reapplies_but_ordinary_pointer_movement_does_not() {
        use winit::event::{DeviceId, WindowEvent};
        let device_id = DeviceId::dummy();
        for event in [
            WindowEvent::CursorEntered { device_id },
            WindowEvent::CursorLeft { device_id },
            WindowEvent::Focused(false),
            WindowEvent::Focused(true),
            WindowEvent::Resized(winit::dpi::PhysicalSize::new(400, 300)),
            WindowEvent::Occluded(true),
            WindowEvent::Occluded(false),
        ] {
            assert!(lifecycle_invalidates_cursor(&event));
        }
        assert!(!lifecycle_invalidates_cursor(&WindowEvent::CursorMoved { device_id, position: winit::dpi::PhysicalPosition::new(100.0, 200.0) }));
        assert!(!lifecycle_invalidates_cursor(&WindowEvent::RedrawRequested));
    }

    #[test]
    fn geometry_content_and_window_identity_are_not_conflated() {
        let original = image();
        let mut first = CustomCursorCache::default();
        first.store(&original, 1);
        first.mark_applied();
        let mut changed = original.clone();
        changed.hotspot = [1, 1];
        assert!(first.get(&changed).is_none());
        changed = original.clone();
        changed.size = [1, 4];
        assert!(first.get(&changed).is_none());
        changed = image();
        assert!(first.get(&changed).is_none());
        // Same bytes in a separate Arc are deliberately a new source identity.
        let second: CustomCursorCache<u8> = CustomCursorCache::default();
        assert!(!second.is_applied(&original));
        assert!(first.is_applied(&original));
    }

    #[test]
    fn cache_owns_source_until_replaced_so_addresses_cannot_be_reused() {
        let source = image();
        let weak = Arc::downgrade(&source.rgba);
        let mut cache = CustomCursorCache::default();
        cache.store(&source, 1);
        drop(source);
        assert!(weak.upgrade().is_some());
        cache.store(&image(), 2);
        assert!(weak.upgrade().is_none());
    }
}
