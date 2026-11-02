//! Disk-backed MP4 recording; all Media Foundation objects stay on the worker thread.
use super::{platform, CaptureRect, RecordingOptions, RecordingProgress};
use parking_lot::Mutex;
use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows::{
    core::{GUID, PCWSTR},
    Win32::{
        Media::MediaFoundation::*,
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED},
    },
};

#[path = "recording/timeline.rs"]
mod timeline;
use timeline::Timeline;
#[path = "recording/audio.rs"]
mod audio;
#[path = "recording/cursor.rs"]
mod cursor;

const FPS: u32 = 30;
const FRAME: Duration = Duration::from_nanos(1_000_000_000 / FPS as u64);

enum Command {
    Pause(bool, mpsc::SyncSender<Result<RecordingProgress, String>>),
    Stop,
}

pub struct Recording {
    directory: PathBuf,
    screen: CaptureRect,
    scale: f64,
    rect: CaptureRect,
    stop: Option<mpsc::Sender<Command>>,
    worker: Option<JoinHandle<Result<(), String>>>,
    result: Option<Result<(), String>>,
    progress: Arc<Mutex<RecordingProgress>>,
}
impl Recording {
    pub fn prepare(screen: CaptureRect, scale: f64, rect: CaptureRect) -> Result<Self, String> {
        let directory =
            std::env::temp_dir().join(format!("popper-recording-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).map_err(|e| format!("无法创建录屏临时目录：{e}"))?;
        Ok(Self {
            directory,
            screen,
            scale,
            rect,
            stop: None,
            worker: None,
            result: None,
            progress: Arc::new(Mutex::new(RecordingProgress::default())),
        })
    }

    pub fn start(&mut self, options: RecordingOptions) -> Result<RecordingProgress, String> {
        let status = self.progress();
        if status.started {
            return Ok(status);
        }
        let (stop, receiver) = mpsc::channel();
        let (ready, started) = mpsc::sync_channel(1);
        self.stop = Some(stop);
        let path = self.path();
        let progress = self.progress.clone();
        let (screen, scale, rect) = (self.screen, self.scale, self.rect);
        self.worker = Some(
            thread::Builder::new()
                .name("region-recording".into())
                .spawn(move || {
                    let mut ready = Some(ready);
                    let result: Result<(), String> = (|| {
                        let _runtime =
                            MediaRuntime::start().map_err(|e| format!("无法启动视频编码：{e}"))?;
                        let encoder = Encoder::new(&path, rect.width, rect.height, options.audio)
                            .map_err(|e| {
                            format!("无法创建 MP4 编码器，请确认系统已安装媒体功能：{e}")
                        })?;
                        let mut audio = if options.audio {
                            Some(audio::Audio::open()?)
                        } else {
                            None
                        };
                        let mut clock = Timeline::new(Instant::now());
                        if let Some(audio) = &mut audio {
                            audio.set_paused(false, Duration::ZERO)?;
                        }
                        let mut timestamp = 0;
                        let mut image = capture_frame(screen, rect, options.cursor)?;
                        // Retain one frame so its duration follows real elapsed time under load.
                        let capture = loop {
                            let command = if clock.paused() {
                                receiver
                                    .recv()
                                    .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                            } else {
                                let elapsed = clock.elapsed(Instant::now());
                                receiver.recv_timeout(FRAME.saturating_sub(
                                    elapsed.saturating_sub(Duration::from_nanos(
                                        timestamp as u64 * 100,
                                    )),
                                ))
                            };
                            let now = Instant::now();
                            let elapsed = clock.elapsed(now);
                            let next_time = elapsed.as_nanos() / 100;
                            if !clock.paused() {
                                if let Some(audio) = &mut audio {
                                    let result = audio
                                        .collect(elapsed, false)
                                        .and_then(|chunk| encoder.write_audio(&chunk));
                                    if let Err(error) = result {
                                        break Err(error);
                                    }
                                }
                            }
                            if !clock.paused() && next_time > timestamp as u128 {
                                if let Err(e) =
                                    encoder.write(&image, timestamp, next_time as i64 - timestamp)
                                {
                                    break Err(format!("无法写入录屏：{e}"));
                                }
                                timestamp = next_time as i64;
                            }
                            if let Some(ready) = ready.take() {
                                progress.lock().started = true;
                                let _ = ready.send(Ok(()));
                            }
                            progress.lock().elapsed_ms = elapsed.as_millis() as u64;
                            let reply = match command {
                                Ok(Command::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                                    break Ok(())
                                }
                                Ok(Command::Pause(paused, reply)) => {
                                    if let Some(audio) = &mut audio {
                                        if let Err(error) = audio
                                            .set_paused(paused, elapsed)
                                            .and_then(|chunk| encoder.write_audio(&chunk))
                                        {
                                            let _ = reply.send(Err(error.clone()));
                                            break Err(error);
                                        }
                                    }
                                    clock.set_paused(paused, now);
                                    progress.lock().paused = paused;
                                    Some(reply)
                                }
                                Err(mpsc::RecvTimeoutError::Timeout) => None,
                            };
                            if !clock.paused() {
                                let next = if platform::same_screen(screen, scale) {
                                    capture_frame(screen, rect, options.cursor)
                                } else {
                                    Err("显示器布局已变化，录制已停止".into())
                                };
                                match next {
                                    Ok(next) => image = next,
                                    Err(error) => {
                                        if let Some(reply) = reply {
                                            let _ = reply.send(Err(error.clone()));
                                        }
                                        break Err(error);
                                    }
                                }
                            }
                            if let Some(reply) = reply {
                                let _ = reply.send(Ok(progress.lock().clone()));
                            }
                        };
                        let capture = if let Some(audio) = &mut audio {
                            let final_audio = audio
                                .finish(Duration::from_nanos(timestamp as u64 * 100))
                                .and_then(|chunk| encoder.write_audio(&chunk));
                            capture.and(final_audio)
                        } else {
                            capture
                        };
                        drop(audio);
                        // Finalize even after capture failure, preserving the frames already encoded.
                        unsafe { encoder.writer.Finalize() }
                            .map_err(|e| format!("无法完成 MP4 文件：{e}"))?;
                        if ready.is_some() {
                            capture?;
                        } else {
                            progress.lock().error = capture.err();
                        }
                        Ok(())
                    })();
                    let mut status = progress.lock();
                    status.stopped = true;
                    status.paused = false;
                    if let Err(error) = &result {
                        status.error = Some(error.clone());
                    }
                    if let Some(ready) = ready {
                        let _ = ready.send(result.clone());
                    }
                    result
                })
                .map_err(|e| e.to_string())?,
        );
        let result = started
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "录屏启动超时".to_owned())
            .and_then(|result| result);
        if let Err(error) = result {
            let _ = self.stop();
            self.result = None;
            self.stop = None;
            *self.progress.lock() = RecordingProgress::default();
            let _ = std::fs::remove_file(self.path());
            return Err(error);
        }
        Ok(self.progress())
    }

    pub fn path(&self) -> PathBuf {
        self.directory.join("recording.mp4")
    }
    pub fn progress(&self) -> RecordingProgress {
        self.progress.lock().clone()
    }
    pub fn pause(&self, paused: bool) -> Result<RecordingProgress, String> {
        let status = self.progress();
        if !status.started || status.stopped {
            return Err("录制尚未开始或已经停止".into());
        }
        let (reply, result) = mpsc::sync_channel(1);
        self.stop
            .as_ref()
            .ok_or("录制尚未开始")?
            .send(Command::Pause(paused, reply))
            .map_err(|_| "录制已经停止".to_owned())?;
        result
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| "切换录屏暂停状态超时，请停止并保存".to_owned())?
    }
    pub fn interrupt(&mut self, message: &str) {
        let _ = self.stop();
        let mut progress = self.progress.lock();
        progress.stopped = true;
        progress.paused = false;
        if progress.error.is_none() {
            progress.error = Some(message.to_owned());
        }
    }

    pub fn stop(&mut self) -> Result<(), String> {
        if let Some(stop) = &self.stop {
            let _ = stop.send(Command::Stop);
        }
        if let Some(worker) = self.worker.take() {
            self.result = Some(
                worker
                    .join()
                    .unwrap_or_else(|_| Err("录屏线程异常结束".into())),
            );
        }
        self.result.clone().unwrap_or(Ok(()))
    }
}
impl Drop for Recording {
    fn drop(&mut self) {
        let _ = self.stop();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn capture_frame(
    screen: CaptureRect,
    rect: CaptureRect,
    include_cursor: bool,
) -> Result<image::RgbaImage, String> {
    let mut image = platform::capture_region(screen, rect)?;
    if include_cursor {
        cursor::draw(&mut image, screen, rect)?;
    }
    Ok(image)
}

struct MediaRuntime;
impl MediaRuntime {
    fn start() -> windows::core::Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            if let Err(e) = MFStartup(MF_VERSION, MFSTARTUP_FULL) {
                CoUninitialize();
                return Err(e);
            }
        }
        Ok(Self)
    }
}
impl Drop for MediaRuntime {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
            CoUninitialize();
        }
    }
}
struct Encoder {
    writer: IMFSinkWriter,
    stream: u32,
    audio_stream: Option<u32>,
    width: u32,
    height: u32,
}
impl Encoder {
    fn new(path: &Path, width: u32, height: u32, audio: bool) -> windows::core::Result<Self> {
        Self::create(path, width, height, true, audio)
            .or_else(|_| Self::create(path, width, height, false, audio))
    }
    fn create(
        path: &Path,
        width: u32,
        height: u32,
        hardware: bool,
        audio: bool,
    ) -> windows::core::Result<Self> {
        use std::os::windows::ffi::OsStrExt;
        // H.264 needs even dimensions; pad the right/bottom edge without sampling outside the selection.
        let width = (width + 1) & !1;
        let height = (height + 1) & !1;
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 2)?;
            let attributes = attributes.unwrap();
            attributes.SetUINT32(
                &MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS,
                u32::from(hardware),
            )?;
            attributes.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
            let writer = MFCreateSinkWriterFromURL(PCWSTR(path.as_ptr()), None, &attributes)?;
            let output = media_type(&MFVideoFormat_H264, width, height)?;
            output.SetUINT32(
                &MF_MT_AVG_BITRATE,
                (u64::from(width) * u64::from(height) * 4).clamp(1_000_000, 24_000_000) as u32,
            )?;
            let stream = writer.AddStream(&output)?;
            let input = media_type(&MFVideoFormat_RGB32, width, height)?;
            writer.SetInputMediaType(stream, &input, None)?;
            let audio_stream = if audio {
                let output = audio_type(&MFAudioFormat_AAC)?;
                output.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 16_000)?;
                output.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0)?;
                output.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                let stream = writer.AddStream(&output)?;
                let input = audio_type(&MFAudioFormat_PCM)?;
                input.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
                input.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)?;
                input.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 192_000)?;
                writer.SetInputMediaType(stream, &input, None)?;
                Some(stream)
            } else {
                None
            };
            writer.BeginWriting()?;
            Ok(Self {
                writer,
                stream,
                audio_stream,
                width,
                height,
            })
        }
    }
    fn write(
        &self,
        image: &image::RgbaImage,
        time: i64,
        duration: i64,
    ) -> windows::core::Result<()> {
        unsafe {
            let length = self.width * self.height * 4;
            let buffer = MFCreateMemoryBuffer(length)?;
            let mut data = std::ptr::null_mut();
            buffer.Lock(&mut data, None, None)?;
            bgra_bottom_up(
                image,
                self.width,
                self.height,
                std::slice::from_raw_parts_mut(data, length as usize),
            );
            buffer.Unlock()?;
            buffer.SetCurrentLength(length)?;
            self.write_buffer(&buffer, self.stream, time, duration)
        }
    }
    fn write_buffer(
        &self,
        buffer: &IMFMediaBuffer,
        stream: u32,
        time: i64,
        duration: i64,
    ) -> windows::core::Result<()> {
        unsafe {
            let sample = MFCreateSample()?;
            sample.AddBuffer(buffer)?;
            sample.SetSampleTime(time)?;
            sample.SetSampleDuration(duration)?;
            self.writer.WriteSample(stream, &sample)
        }
    }
    fn write_audio(&self, chunk: &audio::Chunk) -> Result<(), String> {
        if chunk.bytes.is_empty() {
            return Ok(());
        }
        let stream = self.audio_stream.ok_or("视频未配置音轨")?;
        let result = (|| -> windows::core::Result<()> {
            unsafe {
                let buffer = MFCreateMemoryBuffer(chunk.bytes.len() as u32)?;
                let mut data = std::ptr::null_mut();
                buffer.Lock(&mut data, None, None)?;
                std::ptr::copy_nonoverlapping(chunk.bytes.as_ptr(), data, chunk.bytes.len());
                buffer.Unlock()?;
                buffer.SetCurrentLength(chunk.bytes.len() as u32)?;
                let start = (chunk.start * 10_000_000 / 48_000) as i64;
                let end = (chunk.end * 10_000_000 / 48_000) as i64;
                self.write_buffer(&buffer, stream, start, end - start)
            }
        })();
        result.map_err(|e| format!("无法写入音频：{e}"))
    }
}
fn audio_type(format: &GUID) -> windows::core::Result<IMFMediaType> {
    unsafe {
        let media = MFCreateMediaType()?;
        media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
        media.SetGUID(&MF_MT_SUBTYPE, format)?;
        media.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)?;
        media.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48_000)?;
        Ok(media)
    }
}
fn media_type(format: &GUID, width: u32, height: u32) -> windows::core::Result<IMFMediaType> {
    unsafe {
        let media = MFCreateMediaType()?;
        media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media.SetGUID(&MF_MT_SUBTYPE, format)?;
        media.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        media.SetUINT64(
            &MF_MT_FRAME_SIZE,
            (u64::from(width) << 32) | u64::from(height),
        )?;
        media.SetUINT64(&MF_MT_FRAME_RATE, (u64::from(FPS) << 32) | 1)?;
        media.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1_u64 << 32) | 1)?;
        Ok(media)
    }
}
fn bgra_bottom_up(image: &image::RgbaImage, width: u32, height: u32, bytes: &mut [u8]) {
    let source_stride = image.width() as usize * 4;
    let stride = width as usize * 4;
    bytes[..(height - image.height()) as usize * stride].fill(0);
    for (source, destination) in image
        .as_raw()
        .chunks_exact(source_stride)
        .zip(bytes.chunks_exact_mut(stride).rev())
    {
        destination[..source_stride].copy_from_slice(source);
        destination[source_stride..].fill(0);
        for pixel in destination[..source_stride].chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preparing_does_not_start_a_worker_or_create_a_video() {
        let screen = CaptureRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let recording = Recording::prepare(screen, 1.0, screen).unwrap();
        let directory = recording.directory.clone();
        assert!(recording.worker.is_none());
        assert!(recording.stop.is_none());
        assert!(!recording.path().exists());
        assert!(!recording.progress().started);
        assert_eq!(recording.progress().elapsed_ms, 0);
        assert!(recording.pause(true).is_err());
        drop(recording);
        assert!(!directory.exists());
    }
    #[test]
    fn interruption_retains_the_video_until_explicit_session_cleanup() {
        let screen = CaptureRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let mut recording = Recording::prepare(screen, 1.0, screen).unwrap();
        std::fs::write(recording.path(), b"retained recording").unwrap();
        recording.progress.lock().started = true;
        recording.interrupt("Display disconnected");
        let path = recording.path();
        assert_eq!(std::fs::read(&path).unwrap(), b"retained recording");
        assert!(recording.progress().stopped);
        assert_eq!(
            recording.progress().error.as_deref(),
            Some("Display disconnected")
        );
        recording.stop().unwrap();
        assert!(path.exists());
        drop(recording);
        assert!(!path.exists());
    }

    #[test]
    fn writes_a_finalized_mp4_with_the_system_encoder() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.mp4");
        let _runtime = MediaRuntime::start().unwrap();
        let encoder = Encoder::new(&path, 63, 63, false).unwrap();
        assert!(encoder.audio_stream.is_none());
        let frame = image::RgbaImage::from_pixel(63, 63, image::Rgba([255, 0, 0, 255]));
        let duration = 10_000_000 / i64::from(FPS);
        encoder.write(&frame, 0, duration).unwrap();
        encoder.write(&frame, duration, duration).unwrap();
        unsafe { encoder.writer.Finalize() }.unwrap();
        drop(encoder);
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(&bytes[4..8], b"ftyp");
        assert!(bytes.windows(4).any(|box_type| box_type == b"moov"));
        assert!(bytes.windows(4).any(|box_type| box_type == b"mdat"));
    }
    #[test]
    fn writes_an_aac_audio_track_alongside_video() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("audio.mp4");
        let _runtime = MediaRuntime::start().unwrap();
        let encoder = Encoder::new(&path, 64, 64, true).unwrap();
        assert!(encoder.audio_stream.is_some());
        let frame = image::RgbaImage::from_pixel(64, 64, image::Rgba([20, 40, 60, 255]));
        for index in 0..3_u64 {
            let start = index * 1600;
            let end = start + 1600;
            let time = (start * 10_000_000 / 48_000) as i64;
            let duration = (end * 10_000_000 / 48_000) as i64 - time;
            encoder.write(&frame, time, duration).unwrap();
            encoder
                .write_audio(&audio::Chunk {
                    bytes: vec![0; 1600 * 4],
                    start,
                    end,
                })
                .unwrap();
        }
        unsafe { encoder.writer.Finalize() }.unwrap();
        drop(encoder);
        let bytes = std::fs::read(path).unwrap();
        assert!(bytes.windows(4).any(|value| value == b"soun"));
        assert!(bytes.windows(4).any(|value| value == b"mp4a"));
    }
    #[test]
    fn padded_bottom_row_is_black_and_captured_pixels_are_opaque() {
        let image = image::RgbaImage::from_pixel(1, 1, image::Rgba([20, 40, 60, 1]));
        let mut bytes = vec![9; 16];
        bgra_bottom_up(&image, 2, 2, &mut bytes);
        assert_eq!(bytes, [0, 0, 0, 0, 0, 0, 0, 0, 60, 40, 20, 255, 0, 0, 0, 0]);
    }
    #[test]
    fn rgb_is_bottom_up_bgra_and_odd_edges_are_padded() {
        let image = image::RgbaImage::from_fn(1, 2, |_, y| {
            if y == 0 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        let mut bytes = vec![9; 16];
        bgra_bottom_up(&image, 2, 2, &mut bytes);
        assert_eq!(
            bytes,
            [255, 0, 0, 255, 0, 0, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0]
        );
    }
}
