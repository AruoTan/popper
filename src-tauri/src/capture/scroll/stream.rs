//! Session-owned capture and registration workers, independent of renderer polling.
use super::{settled, ScrollProgress, ScrollStatus, ScrollStitcher};
use image::RgbaImage;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

struct Shared {
    document: Mutex<ScrollStitcher>,
    progress: Mutex<ScrollProgress>,
    error: Mutex<Option<String>>,
}
pub struct ScrollStream {
    stop: Arc<AtomicBool>,
    shared: Arc<Shared>,
    producer: Option<JoinHandle<()>>,
    consumer: Option<JoinHandle<()>>,
}
impl ScrollStream {
    pub fn start(
        initial: RgbaImage,
        mut capture: impl FnMut() -> Result<Option<RgbaImage>, String> + Send + 'static,
        interval: Duration,
    ) -> Result<Self, String> {
        // Queue memory stays bounded even for large selections. Backpressure keeps
        // intermediate frames in order rather than replacing them with a later jump.
        let capacity = (32 * 1024 * 1024 / initial.as_raw().len().max(1)).clamp(1, 12);
        let document = ScrollStitcher::new(initial)?;
        let progress = document.progress(ScrollStatus::Unchanged);
        let shared = Arc::new(Shared {
            document: Mutex::new(document),
            progress: Mutex::new(progress),
            error: Mutex::new(None),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::sync_channel::<Arc<RgbaImage>>(capacity);
        let state = shared.clone();
        let consumer = thread::Builder::new()
            .name("capture-register".into())
            .spawn(move || {
                while let Ok(frame) = receiver.recv() {
                    let progress = {
                        let mut document = state.document.lock().unwrap();
                        document.push_frame(frame)
                    };
                    let limited = progress.status == ScrollStatus::Limit;
                    *state.progress.lock().unwrap() = progress;
                    if limited {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let state = shared.clone();
        let stopped = stop.clone();
        let producer = thread::Builder::new()
            .name("capture-sample".into())
            .spawn(move || {
                let mut previous: Option<Arc<RgbaImage>> = None;
                while !stopped.load(Ordering::Acquire) {
                    let started = Instant::now();
                    match capture() {
                        Ok(Some(image)) => {
                            *state.error.lock().unwrap() = None;
                            if !previous.as_ref().is_some_and(|last| settled(last, &image)) {
                                let image = Arc::new(image);
                                if sender.send(image.clone()).is_err() {
                                    break;
                                }
                                previous = Some(image);
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            *state.error.lock().unwrap() = Some(error);
                        }
                    }
                    if let Some(wait) = interval.checked_sub(started.elapsed()) {
                        thread::sleep(wait);
                    }
                }
            });
        match producer {
            Ok(producer) => Ok(Self {
                stop,
                shared,
                producer: Some(producer),
                consumer: Some(consumer),
            }),
            Err(error) => {
                let _ = consumer.join();
                Err(error.to_string())
            }
        }
    }
    pub fn progress(&self) -> Result<ScrollProgress, String> {
        if let Some(error) = self.shared.error.lock().unwrap().clone() {
            return Err(error);
        }
        Ok(self.snapshot())
    }
    pub fn snapshot(&self) -> ScrollProgress {
        self.shared.progress.lock().unwrap().clone()
    }
    pub fn preview(&self) -> super::preview::ScrollPreview {
        self.shared.document.lock().unwrap().preview()
    }
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(producer) = self.producer.take() {
            let _ = producer.join();
        }
        if let Some(consumer) = self.consumer.take() {
            let _ = consumer.join();
        }
    }
    pub fn compose(&mut self, final_frame: Option<RgbaImage>) -> RgbaImage {
        self.stop();
        let mut document = self.shared.document.lock().unwrap();
        if let Some(image) = final_frame {
            document.push(image);
        }
        document.compose()
    }
}
impl Drop for ScrollStream {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::atomic::AtomicUsize};
    fn page() -> RgbaImage {
        RgbaImage::from_fn(160, 1200, |x, y| {
            let mut value = x.wrapping_mul(761).wrapping_add(y.wrapping_mul(9301));
            value ^= value >> 7;
            value = value.wrapping_mul(0x45d9f3b);
            image::Rgba([value as u8, (value >> 8) as u8, (value >> 16) as u8, 255])
        })
    }
    fn frame(page: &RgbaImage, y: u32) -> RgbaImage {
        image::imageops::crop_imm(page, 0, y, 160, 240).to_image()
    }
    fn wait_for(mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "capture workers did not finish");
            thread::yield_now();
        }
    }
    #[test]
    fn captures_intermediate_positions_without_any_renderer_polling() {
        let page = page();
        let captured = Arc::new(AtomicUsize::new(0));
        let observed = captured.clone();
        let mut images: VecDeque<_> = [210, 420, 630, 840, 100, 600]
            .into_iter()
            .map(|y| frame(&page, y))
            .collect();
        let mut stream = ScrollStream::start(
            frame(&page, 0),
            move || {
                let image = images.pop_front();
                if image.is_some() {
                    observed.fetch_add(1, Ordering::Release);
                }
                Ok(image)
            },
            Duration::from_millis(1),
        )
        .unwrap();
        wait_for(|| captured.load(Ordering::Acquire) == 6);
        stream.stop();
        assert_eq!(
            stream.compose(None),
            image::imageops::crop_imm(&page, 0, 0, 160, 1080).to_image()
        );
        assert_eq!(stream.snapshot().status, ScrollStatus::Tracked);
    }
    #[test]
    fn finish_drains_queued_frames_before_registering_the_final_viewport() {
        let page = page();
        let count = Arc::new(AtomicUsize::new(0));
        let observed = count.clone();
        let mut images: VecDeque<_> = (1..=20).map(|i| frame(&page, i * 20)).collect();
        let mut stream = ScrollStream::start(
            frame(&page, 0),
            move || {
                let image = images.pop_front();
                if image.is_some() {
                    observed.fetch_add(1, Ordering::Release);
                }
                Ok(image)
            },
            Duration::ZERO,
        )
        .unwrap();
        wait_for(|| count.load(Ordering::Acquire) == 20);
        let result = stream.compose(Some(frame(&page, 560)));
        assert_eq!(
            result,
            image::imageops::crop_imm(&page, 0, 0, 160, 800).to_image()
        );
        stream.stop(); // Stopping twice does not leave a worker or queued frame behind.
    }
    #[test]
    fn capture_errors_preserve_the_document_and_sampling_can_recover() {
        let page = page();
        let phase = Arc::new(AtomicUsize::new(0));
        let current = phase.clone();
        let next = frame(&page, 100);
        let mut stream = ScrollStream::start(
            frame(&page, 0),
            move || match current.load(Ordering::Acquire) {
                0 => Err("source unavailable".into()),
                1 => Ok(Some(next.clone())),
                _ => Ok(None),
            },
            Duration::from_millis(1),
        )
        .unwrap();
        wait_for(|| stream.progress().is_err());
        assert_eq!(stream.snapshot().height, 240);
        phase.store(1, Ordering::Release);
        wait_for(|| stream.snapshot().height == 340);
        assert!(stream.progress().is_ok());
        phase.store(2, Ordering::Release);
        assert_eq!(
            stream.compose(None),
            image::imageops::crop_imm(&page, 0, 0, 160, 340).to_image()
        );
    }
    #[test]
    fn dropping_a_stream_stops_and_releases_its_capture_source() {
        let page = page();
        let source = Arc::new(());
        let owned = source.clone();
        let weak = Arc::downgrade(&source);
        drop(source);
        let stream = ScrollStream::start(
            frame(&page, 0),
            move || {
                let _ = &owned;
                Ok(None)
            },
            Duration::from_millis(1),
        )
        .unwrap();
        drop(stream);
        assert!(weak.upgrade().is_none());
    }
}
