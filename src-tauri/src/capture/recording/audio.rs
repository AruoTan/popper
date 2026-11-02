//! WASAPI loopback plus microphone, synchronized using each packet's QPC timestamp.
use std::time::Duration;
use windows::Win32::{
    Media::Audio::*,
    System::{
        Com::{CoCreateInstance, CLSCTX_ALL},
        Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
    },
};
#[path = "audio/mixer.rs"]
mod mixer;
use mixer::{packet_frame, Mixer, CHANNELS, RATE};

pub struct Chunk {
    pub bytes: Vec<u8>,
    pub start: u64,
    pub end: u64,
}
struct Endpoint {
    client: IAudioClient,
    capture: IAudioCaptureClient,
}
impl Endpoint {
    fn open(enumerator: &IMMDeviceEnumerator, flow: EDataFlow) -> windows::core::Result<Self> {
        unsafe {
            let device = enumerator.GetDefaultAudioEndpoint(flow, eConsole)?;
            let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let format = WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_PCM as u16,
                nChannels: CHANNELS as u16,
                nSamplesPerSec: RATE as u32,
                nAvgBytesPerSec: RATE as u32 * CHANNELS as u32 * 2,
                nBlockAlign: CHANNELS as u16 * 2,
                wBitsPerSample: 16,
                cbSize: 0,
            };
            let flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY
                | if flow == eRender {
                    AUDCLNT_STREAMFLAGS_LOOPBACK
                } else {
                    0
                };
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                flags,
                10_000_000,
                0,
                &format,
                None,
            )?;
            let capture = client.GetService()?;
            Ok(Self { client, capture })
        }
    }
    fn read(&self, mixer: &mut Mixer, origin: u64, phase: u64) -> Result<(), String> {
        unsafe {
            while self
                .capture
                .GetNextPacketSize()
                .map_err(|e| e.to_string())?
                > 0
            {
                let (mut data, mut frames, mut flags, mut qpc) = (std::ptr::null_mut(), 0, 0, 0);
                self.capture
                    .GetBuffer(&mut data, &mut frames, &mut flags, None, Some(&mut qpc))
                    .map_err(|e| e.to_string())?;
                let result = (|| {
                    if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 != 0 {
                        return Err("音频设备返回了无效时间戳".into());
                    }
                    if frames as u64 > RATE {
                        return Err("音频设备返回的数据过大".into());
                    }
                    let position = packet_frame(qpc, origin, phase)?;
                    if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 {
                        return Ok(());
                    }
                    if data.is_null() {
                        return Err("音频设备未返回采样数据".into());
                    }
                    let samples =
                        std::slice::from_raw_parts(data.cast::<i16>(), frames as usize * CHANNELS);
                    mixer.mix(position, samples)
                })();
                let released = self
                    .capture
                    .ReleaseBuffer(frames)
                    .map_err(|e| e.to_string());
                result.and(released)?;
            }
        }
        Ok(())
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}

pub struct Audio {
    system: Endpoint,
    microphone: Endpoint,
    mixer: Mixer,
    origin: u64,
    phase: u64,
    paused: bool,
}
impl Audio {
    pub fn open() -> Result<Self, String> {
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| format!("无法枚举录音设备：{e}"))?;
            let system = Endpoint::open(&enumerator, eRender)
                .map_err(|e| format!("无法录制系统声音：{e}"))?;
            let microphone = Endpoint::open(&enumerator, eCapture)
                .map_err(|e| format!("无法录制麦克风，请检查设备和麦克风访问权限：{e}"))?;
            Ok(Self {
                system,
                microphone,
                mixer: Mixer::default(),
                origin: 0,
                phase: 0,
                paused: true,
            })
        }
    }
    pub fn set_paused(&mut self, paused: bool, elapsed: Duration) -> Result<Chunk, String> {
        if paused == self.paused {
            return self.chunk(self.mixer.position);
        }
        if paused {
            unsafe {
                self.system
                    .client
                    .Stop()
                    .and_then(|_| self.microphone.client.Stop())
                    .map_err(|e| format!("无法暂停录音：{e}"))?;
            }
            self.paused = true;
            let chunk = self.collect(elapsed, true)?;
            self.mixer.discard_pending();
            Ok(chunk)
        } else {
            unsafe {
                self.system
                    .client
                    .Reset()
                    .and_then(|_| self.microphone.client.Reset())
                    .map_err(|e| e.to_string())?;
                self.origin = qpc_time()?;
                self.phase = frames(elapsed);
                self.system
                    .client
                    .Start()
                    .and_then(|_| self.microphone.client.Start())
                    .map_err(|e| format!("无法开始录音：{e}"))?;
            }
            self.paused = false;
            self.chunk(self.mixer.position)
        }
    }
    pub fn collect(&mut self, elapsed: Duration, flush: bool) -> Result<Chunk, String> {
        self.system
            .read(&mut self.mixer, self.origin, self.phase)
            .map_err(|e| format!("系统声音采集失败：{e}"))?;
        self.microphone
            .read(&mut self.mixer, self.origin, self.phase)
            .map_err(|e| format!("麦克风采集失败：{e}"))?;
        // A short staging window lets both devices deliver packets before filling gaps with silence.
        let end = if flush {
            elapsed
        } else {
            elapsed.saturating_sub(Duration::from_millis(100))
        };
        self.chunk(frames(end).max(self.mixer.position))
    }
    pub fn finish(&mut self, elapsed: Duration) -> Result<Chunk, String> {
        if !self.paused {
            self.set_paused(true, elapsed)
        } else {
            self.chunk(self.mixer.position)
        }
    }
    fn chunk(&mut self, end: u64) -> Result<Chunk, String> {
        let start = self.mixer.position;
        Ok(Chunk {
            bytes: self.mixer.take_until(end)?,
            start,
            end,
        })
    }
}
fn frames(elapsed: Duration) -> u64 {
    (elapsed.as_nanos() * RATE as u128 / 1_000_000_000) as u64
}
fn qpc_time() -> Result<u64, String> {
    unsafe {
        let (mut counter, mut frequency) = (0, 0);
        QueryPerformanceCounter(&mut counter)
            .and_then(|_| QueryPerformanceFrequency(&mut frequency))
            .map_err(|e| e.to_string())?;
        if counter < 0 || frequency <= 0 {
            return Err("音频同步时钟不可用".into());
        }
        Ok((counter as u128 * 10_000_000 / frequency as u128) as u64)
    }
}
