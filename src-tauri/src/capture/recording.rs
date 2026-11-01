//! Disk-backed MP4 recording; all Media Foundation objects stay on the worker thread.
use super::{platform, CaptureRect, RecordingProgress};
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

const FPS: u32 = 15;
const FRAME: Duration = Duration::from_nanos(1_000_000_000 / FPS as u64);

pub struct Recording {
    directory: PathBuf,
    stop: mpsc::Sender<()>,
    worker: Option<JoinHandle<Result<(), String>>>,
    result: Option<Result<(), String>>,
    progress: Arc<Mutex<RecordingProgress>>,
}
impl Recording {
    pub fn start(screen: CaptureRect, scale: f64, rect: CaptureRect) -> Result<Self, String> {
        let directory =
            std::env::temp_dir().join(format!("popper-recording-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).map_err(|e| format!("无法创建录屏临时目录：{e}"))?;
        let (stop, receiver) = mpsc::channel();
        let (ready, started) = mpsc::sync_channel(1);
        let progress = Arc::new(Mutex::new(RecordingProgress::default()));
        let mut recording = Self {
            directory,
            stop,
            worker: None,
            result: None,
            progress,
        };
        let path = recording.path();
        let progress = recording.progress.clone();
        recording.worker = Some(
            thread::Builder::new()
                .name("region-recording".into())
                .spawn(move || {
                    let mut ready = Some(ready);
                    let result: Result<(), String> = (|| {
                        let _runtime =
                            MediaRuntime::start().map_err(|e| format!("无法启动视频编码：{e}"))?;
                        let encoder =
                            Encoder::new(&path, rect.width, rect.height).map_err(|e| {
                                format!("无法创建 MP4 编码器，请确认系统已安装媒体功能：{e}")
                            })?;
                        let start = Instant::now();
                        let mut timestamp = 0;
                        let mut image = platform::capture_region(screen, rect)?;
                        // Retain one frame so its duration follows real elapsed time under load.
                        let capture = loop {
                            let stopping = receiver.recv_timeout(FRAME.saturating_sub(
                                start.elapsed() - Duration::from_nanos(timestamp as u64 * 100),
                            ));
                            let next_time = (start.elapsed().as_nanos() / 100)
                                .max(timestamp as u128 + 1)
                                as i64;
                            if let Err(e) = encoder.write(&image, timestamp, next_time - timestamp)
                            {
                                break Err(format!("无法写入录屏：{e}"));
                            }
                            if let Some(ready) = ready.take() {
                                let _ = ready.send(Ok(()));
                            }
                            progress.lock().elapsed_ms = start.elapsed().as_millis() as u64;
                            if !matches!(stopping, Err(mpsc::RecvTimeoutError::Timeout)) {
                                break Ok(());
                            }
                            if !platform::same_screen(screen, scale) {
                                break Err("显示器布局已变化，录制已停止".into());
                            }
                            match platform::capture_region(screen, rect) {
                                Ok(next) => image = next,
                                Err(e) => break Err(e),
                            }
                            timestamp = next_time;
                        };
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
        started
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| "录屏启动超时".to_owned())??;
        Ok(recording)
    }

    pub fn path(&self) -> PathBuf {
        self.directory.join("recording.mp4")
    }
    pub fn progress(&self) -> RecordingProgress {
        self.progress.lock().clone()
    }
    pub fn stop(&mut self) -> Result<(), String> {
        let _ = self.stop.send(());
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
    width: u32,
    height: u32,
}
impl Encoder {
    fn new(path: &Path, width: u32, height: u32) -> windows::core::Result<Self> {
        use std::os::windows::ffi::OsStrExt;
        // H.264 needs even dimensions; pad the right/bottom edge without sampling outside the selection.
        let width = (width + 1) & !1;
        let height = (height + 1) & !1;
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            let writer = MFCreateSinkWriterFromURL(PCWSTR(path.as_ptr()), None, None)?;
            let output = media_type(&MFVideoFormat_H264, width, height)?;
            output.SetUINT32(
                &MF_MT_AVG_BITRATE,
                (u64::from(width) * u64::from(height) * 4).clamp(1_000_000, 24_000_000) as u32,
            )?;
            let stream = writer.AddStream(&output)?;
            let input = media_type(&MFVideoFormat_RGB32, width, height)?;
            writer.SetInputMediaType(stream, &input, None)?;
            writer.BeginWriting()?;
            Ok(Self {
                writer,
                stream,
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
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime(time)?;
            sample.SetSampleDuration(duration)?;
            self.writer.WriteSample(self.stream, &sample)
        }
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
    bytes.fill(0);
    for (x, y, pixel) in image.enumerate_pixels() {
        let offset = (((height - 1 - y) * width + x) * 4) as usize;
        bytes[offset..offset + 4].copy_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writes_a_finalized_mp4_with_the_system_encoder() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("test.mp4");
        let _runtime = MediaRuntime::start().unwrap();
        let encoder = Encoder::new(&path, 63, 63).unwrap();
        let frame = image::RgbaImage::from_pixel(63, 63, image::Rgba([255, 0, 0, 255]));
        encoder.write(&frame, 0, 666_667).unwrap();
        encoder.write(&frame, 666_667, 666_667).unwrap();
        unsafe { encoder.writer.Finalize() }.unwrap();
        drop(encoder);
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(&bytes[4..8], b"ftyp");
        assert!(bytes.windows(4).any(|box_type| box_type == b"moov"));
        assert!(bytes.windows(4).any(|box_type| box_type == b"mdat"));
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
