use super::PinnedImageState;
use std::{collections::HashMap, sync::Arc};
const MAX_PINS: usize = 12;
const PIN_BUDGET: u64 = 256 * 1024 * 1024;
pub struct Pin {
    pub meta: PinnedImageState,
    pub png: Arc<Vec<u8>>,
    pub visible: bool,
    pub closed: bool,
    pub ready: bool,
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
    /// Return the previously closed window whose image has just been released.
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
        let previous = self.closed.replace(id.to_owned()).filter(|old| old != id);
        if let Some(previous) = &previous {
            self.items.remove(previous);
        }
        Ok(previous)
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
                click_through: false,
            },
            png: Arc::new(vec![1, 2, 3]),
            visible: true,
            closed: false,
            ready: true,
        }
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
