use std::time::{Duration, Instant};

pub(super) struct Timeline {
    start: Instant,
    paused_at: Option<Instant>,
    paused_duration: Duration,
}
impl Timeline {
    pub fn new(now: Instant) -> Self {
        Self {
            start: now,
            paused_at: None,
            paused_duration: Duration::ZERO,
        }
    }
    pub fn elapsed(&self, now: Instant) -> Duration {
        self.paused_at.unwrap_or(now).duration_since(self.start) - self.paused_duration
    }
    pub fn paused(&self) -> bool {
        self.paused_at.is_some()
    }
    pub fn set_paused(&mut self, paused: bool, now: Instant) {
        if paused && self.paused_at.is_none() {
            self.paused_at = Some(now);
        }
        if !paused {
            if let Some(start) = self.paused_at.take() {
                self.paused_duration += now.duration_since(start);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pauses_freeze_video_time_and_repeated_commands_are_idempotent() {
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut time = Timeline::new(start);
        time.set_paused(true, at(2));
        time.set_paused(true, at(4));
        assert_eq!(time.elapsed(at(10)), Duration::from_secs(2));
        time.set_paused(false, at(10));
        time.set_paused(false, at(11));
        assert_eq!(time.elapsed(at(13)), Duration::from_secs(5));
        time.set_paused(true, at(14));
        time.set_paused(false, at(20));
        assert_eq!(time.elapsed(at(21)), Duration::from_secs(7));
    }
}
