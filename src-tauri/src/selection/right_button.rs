//! Non-blocking, one-shot right-button hold recognition.
pub(super) const HOLD_MS: u64 = 250;

#[derive(Default)]
pub(super) struct RightButtonGesture {
    pressed_at: Option<u64>,
    fired: bool,
}

#[derive(Debug, PartialEq)]
pub(super) enum Release {
    Click,
    Hold,
    Consumed,
}

impl RightButtonGesture {
    pub(super) fn press(&mut self, now: u64) {
        self.pressed_at = Some(now);
        self.fired = false;
    }

    pub(super) fn poll(&mut self, now: u64) -> bool {
        if !self.fired
            && self
                .pressed_at
                .is_some_and(|at| now.saturating_sub(at) >= HOLD_MS)
        {
            self.fired = true;
            return true;
        }
        false
    }

    pub(super) fn cancel(&mut self) {
        self.fired = true;
    }

    pub(super) fn release(&mut self, now: u64) -> Release {
        let action = if self.pressed_at.is_none() || self.fired {
            Release::Consumed
        } else if self.poll(now) {
            Release::Hold
        } else {
            Release::Click
        };
        self.pressed_at = None;
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_press_replays_a_click() {
        let mut gesture = RightButtonGesture::default();
        gesture.press(100);
        assert!(!gesture.poll(349));
        assert_eq!(gesture.release(349), Release::Click);
        assert!(!gesture.poll(500));
    }

    #[test]
    fn hold_fires_at_threshold_once_and_consumes_release() {
        let mut gesture = RightButtonGesture::default();
        gesture.press(100);
        assert!(gesture.poll(350));
        assert!(!gesture.poll(351));
        assert!(!gesture.poll(10_000));
        assert_eq!(gesture.release(10_000), Release::Consumed);
        gesture.press(11_000);
        assert_eq!(gesture.release(11_250), Release::Hold);
        assert!(!gesture.poll(12_000));
    }

    #[test]
    fn cancellation_prevents_capture_in_a_new_context() {
        let mut gesture = RightButtonGesture::default();
        gesture.press(100);
        gesture.cancel();
        assert!(!gesture.poll(500));
        assert_eq!(gesture.release(500), Release::Consumed);
        gesture.press(600);
        assert_eq!(gesture.release(700), Release::Click);
    }
}
