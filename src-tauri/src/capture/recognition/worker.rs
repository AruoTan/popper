//! Latest-request scheduling and ownership of recognition helper processes.
use super::{Probe, Provider, FLAG};
use crate::capture::CaptureHitTest;
use parking_lot::{Condvar, Mutex};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};

struct Job {
    probe: Probe,
    reply: Option<mpsc::SyncSender<CaptureHitTest>>,
}
struct Worker {
    process: Child,
    input: ChildStdin,
    output: mpsc::Receiver<CaptureHitTest>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
impl Worker {
    fn spawn(provider: Provider) -> Result<Self, String> {
        use std::os::windows::process::CommandExt;
        let mut process = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .arg(FLAG)
            .arg(provider.flag())
            .creation_flags(0x08000000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = process.stdin.take().ok_or("无法连接控件识别进程")?;
        let output = process.stdout.take().ok_or("无法读取控件识别结果")?;
        let (tx, rx) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("capture-probe-reader".into())
            .spawn(move || {
                for line in BufReader::new(output).lines() {
                    let Ok(line) = line else {
                        break;
                    };
                    if line.len() > 128 * 1024 {
                        break;
                    }
                    let Ok(value) = serde_json::from_str::<CaptureHitTest>(&line) else {
                        break;
                    };
                    if tx.send(value).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| {
                let _ = process.kill();
                let _ = process.wait();
                error.to_string()
            })?;
        Ok(Self {
            process,
            input,
            output: rx,
        })
    }
    fn probe(&mut self, probe: &Probe) -> Result<CaptureHitTest, String> {
        let line = serde_json::to_string(probe).map_err(|e| e.to_string())?;
        writeln!(self.input, "{line}")
            .and_then(|_| self.input.flush())
            .map_err(|e| e.to_string())?;
        self.output
            .recv_timeout(Duration::from_millis(300))
            .map_err(|_| "控件识别超时".into())
    }
}
pub(super) struct Lane {
    pending: Arc<(Mutex<Option<Job>>, Condvar)>,
    stopped: Arc<AtomicBool>,
}
impl Lane {
    pub(super) fn new(provider: Provider) -> Self {
        let pending = Arc::new((Mutex::new(None::<Job>), Condvar::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let queue = pending.clone();
        let stop = stopped.clone();
        let _ = std::thread::Builder::new()
            .name(
                match provider {
                    Provider::Accessibility => "capture-accessibility",
                    Provider::Automation => "capture-automation",
                }
                .into(),
            )
            .spawn(move || {
                let mut worker: Option<Worker> = None;
                loop {
                    let job = {
                        let mut slot = queue.0.lock();
                        while slot.is_none() && !stop.load(Ordering::Acquire) {
                            queue.1.wait(&mut slot);
                        }
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        slot.take()
                    };
                    let Some(job) = job else {
                        continue;
                    };
                    if worker.is_none() {
                        worker = Worker::spawn(provider).ok();
                    }
                    let result = worker
                        .as_mut()
                        .ok_or_else(|| "控件识别不可用".to_owned())
                        .and_then(|w| w.probe(&job.probe));
                    match result {
                        Ok(value) => {
                            if let Some(reply) = job.reply {
                                let _ = reply.try_send(value);
                            }
                        }
                        Err(_) => {
                            worker = None;
                            if let Some(reply) = job.reply {
                                let _ = reply.try_send(CaptureHitTest::default());
                            }
                        }
                    }
                }
            });
        Self { pending, stopped }
    }
    pub(super) fn submit(&self, probe: Probe, reply: Option<mpsc::SyncSender<CaptureHitTest>>) {
        if !self.stopped.load(Ordering::Acquire) {
            *self.pending.0.lock() = Some(Job { probe, reply });
            self.pending.1.notify_one();
        }
    }
    pub(super) fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.pending.0.lock().take();
        self.pending.1.notify_all();
    }
}
impl Drop for Lane {
    fn drop(&mut self) {
        self.stop();
    }
}
