use std::collections::VecDeque;

pub const RATE: u64 = 48_000;
pub const CHANNELS: usize = 2;
const MAX_FRAMES: u64 = RATE * 4;

pub fn packet_frame(qpc: u64, origin: u64, phase: u64) -> Result<i64, String> {
    let position = phase as i128 + (qpc as i128 - origin as i128) * RATE as i128 / 10_000_000;
    i64::try_from(position).map_err(|_| "音频时间戳无效".into())
}

#[derive(Default)]
pub struct Mixer {
    pub position: u64,
    samples: VecDeque<i32>,
}
impl Mixer {
    pub fn mix(&mut self, frame: i64, samples: &[i16]) -> Result<(), String> {
        if samples.len() % CHANNELS != 0 {
            return Err("音频数据未按声道对齐".into());
        }
        let end = frame.saturating_add((samples.len() / CHANNELS) as i64);
        if end <= self.position as i64 {
            return Ok(());
        }
        if end > self.position.saturating_add(MAX_FRAMES) as i64 {
            return Err("音频时间戳超出同步范围".into());
        }
        let skip = (self.position as i64 - frame).max(0) as usize * CHANNELS;
        let offset = (frame - self.position as i64).max(0) as usize * CHANNELS;
        self.samples
            .resize(self.samples.len().max(offset + samples.len() - skip), 0);
        // Equal gain leaves headroom when the microphone and system sound overlap.
        for (index, sample) in samples[skip..].iter().enumerate() {
            self.samples[offset + index] += i32::from(*sample) / 2;
        }
        Ok(())
    }
    pub fn discard_pending(&mut self) {
        self.samples.clear();
    }
    pub fn take_until(&mut self, target: u64) -> Result<Vec<u8>, String> {
        if target < self.position || target - self.position > MAX_FRAMES {
            return Err("音频输出时间范围无效".into());
        }
        let mut bytes = Vec::with_capacity((target - self.position) as usize * CHANNELS * 2);
        for _ in 0..(target - self.position) as usize * CHANNELS {
            bytes.extend_from_slice(
                &(self
                    .samples
                    .pop_front()
                    .unwrap_or(0)
                    .clamp(i16::MIN as i32, i16::MAX as i32) as i16)
                    .to_le_bytes(),
            );
        }
        self.position = target;
        Ok(bytes)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aligns_and_mixes_sources_and_fills_silence() {
        let mut mixer = Mixer::default();
        mixer
            .mix(-1, &[1000, 1000, 2000, 2000, 3000, 3000])
            .unwrap();
        mixer.mix(1, &[4000, 4000]).unwrap();
        let bytes = mixer.take_until(3).unwrap();
        let samples: Vec<i16> = bytes
            .chunks_exact(2)
            .map(|v| i16::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert_eq!(samples, [1000, 1000, 3500, 3500, 0, 0]);
        assert!(mixer.mix(0, &[1000, 1000]).is_ok());
        assert!(mixer.mix((RATE * 5) as i64, &[1, 1]).is_err());
        assert!(mixer.mix(3, &[1]).is_err());
    }
    #[test]
    fn packet_timestamps_rebase_after_a_pause_without_a_silent_gap() {
        assert_eq!(packet_frame(10_000_000, 10_000_000, 0).unwrap(), 0);
        assert_eq!(packet_frame(20_000_000, 10_000_000, 0).unwrap(), 48_000);
        assert_eq!(packet_frame(9_000_000, 10_000_000, 0).unwrap(), -4800);
        assert_eq!(
            packet_frame(900_100_000, 900_000_000, 96_000).unwrap(),
            96_480
        );
        assert!(packet_frame(1, 1, u64::MAX).is_err());
    }
    #[test]
    fn pause_discards_queued_audio_after_the_video_boundary() {
        let mut mixer = Mixer::default();
        mixer.mix(0, &[2000; 12]).unwrap();
        mixer.take_until(3).unwrap();
        mixer.discard_pending();
        mixer.mix(3, &[4000, 4000]).unwrap();
        let bytes = mixer.take_until(5).unwrap();
        let samples: Vec<i16> = bytes
            .chunks_exact(2)
            .map(|v| i16::from_le_bytes(v.try_into().unwrap()))
            .collect();
        assert_eq!(samples, [2000, 2000, 0, 0]);
    }
}
