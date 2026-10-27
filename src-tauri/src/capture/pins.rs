use super::{CaptureRect, PinEditorState, PinUpdate, PinnedImageState};
use std::{collections::HashMap, sync::Arc};
#[path = "pins/toolbar.rs"]
pub mod toolbar;
const MAX_PINS: usize = 12;
const PIN_BUDGET: u64 = 256 * 1024 * 1024;
pub const SHADOW_INSET: u32 = 24;

/// The transparent frame surrounds the image without changing its screen coordinates.
pub fn image_frame(image: CaptureRect, inset: u32) -> CaptureRect {
    CaptureRect {
        x: image.x - inset as i32,
        y: image.y - inset as i32,
        width: image.width + inset * 2,
        height: image.height + inset * 2,
    }
}

pub fn resized_frame(
    meta: &PinnedImageState,
    frame: CaptureRect,
    update: &PinUpdate,
) -> CaptureRect {
    if update.zoom == meta.zoom {
        return frame;
    }
    let dimension = |original: u32, zoom: f64| (original as f64 * zoom).round().max(1.0) as u32;
    let width = dimension(meta.width, update.zoom);
    let height = dimension(meta.height, update.zoom);
    // A monitor DPI transition can resize the whole window, including its margins.
    let inset_x = frame.width as f64 * meta.shadow_inset as f64
        / (dimension(meta.width, meta.zoom) + meta.shadow_inset * 2) as f64;
    let inset_y = frame.height as f64 * meta.shadow_inset as f64
        / (dimension(meta.height, meta.zoom) + meta.shadow_inset * 2) as f64;
    CaptureRect {
        x: frame.x
            + (inset_x + (frame.width as f64 - inset_x * 2.0 - width as f64) * update.anchor_x
                - meta.shadow_inset as f64)
                .round() as i32,
        y: frame.y
            + (inset_y + (frame.height as f64 - inset_y * 2.0 - height as f64) * update.anchor_y
                - meta.shadow_inset as f64)
                .round() as i32,
        width: width + meta.shadow_inset * 2,
        height: height + meta.shadow_inset * 2,
    }
}
pub struct Pin {
    pub meta: PinnedImageState,
    pub png: Arc<Vec<u8>>,
    pub editor: PinEditorState,
    pub frame: Option<CaptureRect>,
    pub scale: f64,
    pub visible: bool,
    pub closed: bool,
    pub creating: bool,
    pub ready: bool,
    pub toolbar: toolbar::Toolbar,
}
#[derive(Default)]
pub struct Pins {
    pub items: HashMap<String, Pin>,
    pub closed: Option<String>,
    pub hidden: bool,
}
impl Pins {
    pub fn insert(&mut self, pin: Pin) -> Result<(), String> {
        let allocated = self.allocated();
        let needed = u64::from(pin.meta.width) * u64::from(pin.meta.height) * 4;
        if self.items.len() >= MAX_PINS || needed > PIN_BUDGET || allocated > PIN_BUDGET - needed {
            return Err("贴图数量或内存已达上限，请先销毁部分贴图".into());
        }
        self.items.insert(pin.meta.id.clone(), pin);
        Ok(())
    }
    pub fn allocated(&self) -> u64 {
        self.items
            .values()
            .map(|pin| u64::from(pin.meta.width) * u64::from(pin.meta.height) * 4)
            .sum()
    }
    /// Replace the single data-only recovery item and release its predecessor.
    pub fn close(&mut self, id: &str, destroy: bool) -> Result<Option<String>, String> {
        if destroy {
            self.items.remove(id);
            if self.closed.as_deref() == Some(id) {
                self.closed = None;
            }
            return Ok(None);
        }
        let pin = self.items.get_mut(id).ok_or("贴图已结束")?;
        pin.closed = true;
        pin.visible = false;
        pin.creating = false;
        pin.ready = false;
        let previous = self.closed.replace(id.to_owned()).filter(|old| old != id);
        if let Some(previous) = &previous {
            self.items.remove(previous);
        }
        Ok(previous)
    }
    pub fn window_destroyed(&mut self, id: &str) {
        // A failed creation is still owned by its caller, which either drops
        // a new image or returns a failed recovery to the data-only cache.
        if self
            .items
            .get(id)
            .is_some_and(|pin| pin.closed || pin.creating)
        {
            return;
        }
        self.items.remove(id);
        if self.closed.as_deref() == Some(id) {
            self.closed = None;
        }
    }
    pub fn take_closed(&mut self) -> Option<Pin> {
        let id = self.closed.take()?;
        self.items.remove(&id)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn pin(id: &str, width: u32, height: u32) -> Pin {
        Pin {
            meta: PinnedImageState {
                id: id.into(),
                width,
                height,
                zoom: 1.0,
                opacity: 1.0,
                shadow_inset: SHADOW_INSET,
                toolbar_visible: false,
                annotations_visible: true,
                always_on_top: true,
            },
            png: Arc::new(vec![1, 2, 3]),
            editor: PinEditorState::new(Vec::new()),
            frame: None,
            scale: 1.0,
            visible: true,
            closed: false,
            creating: false,
            ready: true,
            toolbar: Default::default(),
        }
    }
    #[test]
    fn image_position_is_preserved_with_shadow_margins_on_negative_screens() {
        let image = CaptureRect {
            x: -1720,
            y: -920,
            width: 400,
            height: 240,
        };
        let frame = image_frame(image, SHADOW_INSET);
        assert_eq!(
            (frame.x + SHADOW_INSET as i32, frame.y + SHADOW_INSET as i32),
            (image.x, image.y)
        );
        assert_eq!(
            (
                frame.width - SHADOW_INSET * 2,
                frame.height - SHADOW_INSET * 2
            ),
            (image.width, image.height)
        );
    }
    #[test]
    fn zoom_preserves_the_image_anchor_without_scaling_shadow_margins() {
        let meta = pin("zoom", 400, 240).meta;
        let frame = image_frame(
            CaptureRect {
                x: -1720,
                y: -920,
                width: 400,
                height: 240,
            },
            SHADOW_INSET,
        );
        for (anchor_x, anchor_y) in [(0.0, 0.0), (0.25, 0.75), (0.5, 0.5), (1.0, 1.0)] {
            for zoom in [0.1, 1.1, 4.0] {
                let update = PinUpdate {
                    zoom,
                    opacity: 1.0,
                    anchor_x,
                    anchor_y,
                };
                let resized = resized_frame(&meta, frame, &update);
                let width = resized.width - SHADOW_INSET * 2;
                let height = resized.height - SHADOW_INSET * 2;
                assert!(
                    ((frame.x + SHADOW_INSET as i32) as f64 + 400.0 * anchor_x
                        - ((resized.x + SHADOW_INSET as i32) as f64 + width as f64 * anchor_x))
                        .abs()
                        <= 0.5
                );
                assert!(
                    ((frame.y + SHADOW_INSET as i32) as f64 + 240.0 * anchor_y
                        - ((resized.y + SHADOW_INSET as i32) as f64 + height as f64 * anchor_y))
                        .abs()
                        <= 0.5
                );
            }
        }
        let opacity_only = PinUpdate {
            zoom: 1.0,
            opacity: 0.5,
            anchor_x: 0.5,
            anchor_y: 0.5,
        };
        assert_eq!(resized_frame(&meta, frame, &opacity_only), frame);
    }
    #[test]
    fn zoom_accounts_for_window_margins_after_a_monitor_dpi_transition() {
        let meta = pin("dpi", 400, 240).meta;
        let frame = CaptureRect {
            x: -1000,
            y: 100,
            width: 896,
            height: 576,
        };
        let update = PinUpdate {
            zoom: 1.1,
            opacity: 1.0,
            anchor_x: 0.25,
            anchor_y: 0.75,
        };
        let resized = resized_frame(&meta, frame, &update);
        assert_eq!(resized.width, 488);
        assert_eq!(resized.height, 312);
        assert_eq!(resized.x + 24 + 110, frame.x + 48 + 200);
        assert_eq!(resized.y + 24 + 198, frame.y + 48 + 360);
    }
    #[test]
    fn close_restore_cache_is_bounded_across_one_hundred_cycles() {
        let mut pins = Pins::default();
        let mut previous = None;
        for index in 0..100 {
            let id = index.to_string();
            let item = pin(&id, 1920, 1080);
            let reference = Arc::downgrade(&item.png);
            pins.insert(item).unwrap();
            pins.close(&id, false).unwrap();
            pins.window_destroyed(&id);
            assert_eq!(pins.items.len(), 1);
            assert_eq!(pins.allocated(), 1920 * 1080 * 4);
            if let Some(old) = previous.take() {
                let old: std::sync::Weak<Vec<u8>> = old;
                assert!(old.upgrade().is_none());
            }
            previous = Some(reference);
        }
        let id = pins.closed.clone().unwrap();
        pins.close(&id, true).unwrap();
        assert_eq!(pins.allocated(), 0);
        assert!(previous.unwrap().upgrade().is_none());
    }
    #[test]
    fn closed_window_release_preserves_recovery_and_old_events_cannot_remove_its_replacement() {
        let mut pins = Pins::default();
        let mut item = pin("original", 400, 240);
        item.meta.zoom = 1.7;
        item.meta.opacity = 0.4;
        item.meta.always_on_top = false;
        item.meta.toolbar_visible = true;
        item.frame = Some(CaptureRect {
            x: -800,
            y: -120,
            width: 728,
            height: 456,
        });
        item.scale = 1.5;
        item.editor.color = "#123456".into();
        item.editor.line_width = 7;
        let image = Arc::downgrade(&item.png);
        pins.insert(item).unwrap();
        pins.close("original", false).unwrap();
        pins.window_destroyed("original");
        let mut restored = pins.take_closed().unwrap();
        assert!(pins.items.is_empty());
        assert!(pins.closed.is_none());
        assert!(image.upgrade().is_some());
        assert_eq!(restored.frame.unwrap().x, -800);
        assert_eq!(restored.meta.zoom, 1.7);
        assert_eq!(restored.meta.opacity, 0.4);
        assert!(!restored.meta.always_on_top);
        assert!(restored.meta.toolbar_visible);
        assert_eq!(restored.scale, 1.5);
        assert_eq!(restored.editor.line_width, 7);
        assert_eq!(restored.editor.color, "#123456");
        restored.meta.id = "restored".into();
        restored.closed = false;
        restored.creating = true;
        restored.ready = true;
        pins.insert(restored).unwrap();
        pins.window_destroyed("original");
        assert!(pins.items.contains_key("restored"));
        // A failed restore retains model data until its caller re-caches it.
        pins.window_destroyed("restored");
        pins.close("restored", false).unwrap();
        assert!(pins.take_closed().is_some());
        assert!(image.upgrade().is_none());
    }

    #[test]
    fn unexpected_active_window_destruction_releases_image_and_explicit_destruction_has_no_recovery(
    ) {
        let mut pins = Pins::default();
        let item = pin("active", 4, 4);
        let reference = Arc::downgrade(&item.png);
        pins.insert(item).unwrap();
        pins.window_destroyed("active");
        assert!(reference.upgrade().is_none());
        pins.insert(pin("closed", 4, 4)).unwrap();
        pins.close("closed", false).unwrap();
        pins.close("closed", true).unwrap();
        pins.window_destroyed("closed");
        assert!(pins.take_closed().is_none());
        assert_eq!(pins.allocated(), 0);
    }
    #[test]
    fn hidden_and_closed_images_still_count_toward_capacity() {
        let mut pins = Pins::default();
        for index in 0..12 {
            let mut item = pin(&index.to_string(), 1, 1);
            item.visible = false;
            pins.insert(item).unwrap();
        }
        assert!(pins.insert(pin("overflow", 1, 1)).is_err());
        assert_eq!(pins.items.len(), 12);
        let mut pins = Pins::default();
        pins.insert(pin("large", 8192, 8192)).unwrap();
        assert!(pins.insert(pin("small", 1, 1)).is_err());
        pins.close("large", true).unwrap();
        assert!(pins.insert(pin("small", 1, 1)).is_ok());
    }
}
