use super::{CaptureCandidate, CaptureRect};
use parking_lot::{Condvar, Mutex};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};
use windows::{
    core::Interface,
    Win32::{
        Foundation::HWND,
        System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_MULTITHREADED,
        },
        UI::Accessibility::{CUIAutomation, IUIAutomation},
    },
};

const FLAG: &str = "--popper-capture-helper";
#[derive(Serialize, Deserialize)]
struct Probe {
    handle: usize,
    x: i32,
    y: i32,
}
struct Job {
    probe: Probe,
    reply: mpsc::SyncSender<Vec<CaptureCandidate>>,
}
struct Worker {
    process: Child,
    input: ChildStdin,
    output: mpsc::Receiver<Vec<CaptureCandidate>>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
impl Worker {
    fn spawn() -> Result<Self, String> {
        use std::os::windows::process::CommandExt;
        let mut process = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
            .arg(FLAG)
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
                    let Ok(value) = serde_json::from_str::<Vec<CaptureCandidate>>(&line) else {
                        break;
                    };
                    if tx.send(value).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            process,
            input,
            output: rx,
        })
    }
    fn probe(&mut self, probe: &Probe) -> Result<Vec<CaptureCandidate>, String> {
        let line = serde_json::to_string(probe).map_err(|e| e.to_string())?;
        writeln!(self.input, "{line}")
            .and_then(|_| self.input.flush())
            .map_err(|e| e.to_string())?;
        self.output
            .recv_timeout(Duration::from_millis(300))
            .map_err(|_| "控件识别超时".into())
    }
}
pub struct Recognition {
    pending: Arc<(Mutex<Option<Job>>, Condvar)>,
    stopped: Arc<AtomicBool>,
}
impl Recognition {
    pub fn new() -> Self {
        let pending = Arc::new((Mutex::new(None::<Job>), Condvar::new()));
        let stopped = Arc::new(AtomicBool::new(false));
        let queue = pending.clone();
        let stop = stopped.clone();
        let _ = std::thread::Builder::new()
            .name("capture-recognition".into())
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
                        worker = Worker::spawn().ok();
                    }
                    let result = worker
                        .as_mut()
                        .ok_or_else(|| "控件识别不可用".to_owned())
                        .and_then(|w| w.probe(&job.probe));
                    match result {
                        Ok(value) => {
                            let _ = job.reply.try_send(value);
                        }
                        Err(_) => {
                            worker = None;
                            let _ = job.reply.try_send(Vec::new());
                        }
                    }
                }
            });
        Self { pending, stopped }
    }
    pub fn query(&self, handle: usize, x: i32, y: i32) -> Vec<CaptureCandidate> {
        if self.stopped.load(Ordering::Acquire) {
            return Vec::new();
        }
        let (reply, result) = mpsc::sync_channel(1);
        // Replacing this slot discards superseded pointer requests immediately.
        *self.pending.0.lock() = Some(Job {
            probe: Probe { handle, x, y },
            reply,
        });
        self.pending.1.notify_one();
        result
            .recv_timeout(Duration::from_millis(150))
            .unwrap_or_default()
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.pending.0.lock().take();
        self.pending.1.notify_all();
    }
}
impl Drop for Recognition {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn run_helper_if_requested() -> bool {
    if !std::env::args_os().skip(1).any(|value| value == FLAG) {
        return false;
    }
    unsafe {
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
            return true;
        }
        let automation =
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER);
        if let Ok(automation) = automation {
            let stdin = std::io::stdin();
            let mut output = std::io::stdout().lock();
            for line in stdin.lock().lines() {
                let Ok(line) = line else {
                    break;
                };
                if line.len() > 1024 {
                    break;
                }
                let Ok(probe) = serde_json::from_str::<Probe>(&line) else {
                    break;
                };
                let candidates = inspect(&automation, probe).unwrap_or_default();
                let Ok(json) = serde_json::to_string(&candidates) else {
                    break;
                };
                if writeln!(output, "{json}")
                    .and_then(|_| output.flush())
                    .is_err()
                {
                    break;
                }
            }
        }
        CoUninitialize();
    }
    true
}
unsafe fn inspect(
    automation: &IUIAutomation,
    probe: Probe,
) -> windows::core::Result<Vec<CaptureCandidate>> {
    let mut element = unsafe { automation.ElementFromHandle(HWND(probe.handle as *mut _))? };
    let walker = unsafe { automation.ControlViewWalker()? };
    let mut candidates = Vec::new();
    let mut visited = 0;
    for _ in 0..16 {
        let rect = unsafe { element.CurrentBoundingRectangle()? };
        if rect.right <= rect.left || rect.bottom <= rect.top {
            break;
        }
        let rect = CaptureRect {
            x: rect.left,
            y: rect.top,
            width: (rect.right - rect.left) as u32,
            height: (rect.bottom - rect.top) as u32,
        };
        if !rect.contains(probe.x, probe.y) {
            break;
        }
        if candidates
            .last()
            .is_none_or(|c: &CaptureCandidate| c.rect != rect)
        {
            let label = unsafe { element.CurrentName() }
                .map(|s| s.to_string().chars().take(64).collect())
                .unwrap_or_default();
            candidates.push(CaptureCandidate { rect, label });
        }
        let mut child = unsafe { walker.GetFirstChildElement(&element) }.ok();
        let mut smallest = None;
        let mut area = u64::MAX;
        while let Some(current) = child {
            visited += 1;
            if visited > 512 {
                break;
            }
            if let Ok(r) = unsafe { current.CurrentBoundingRectangle() } {
                if r.right > r.left && r.bottom > r.top {
                    let candidate = CaptureRect {
                        x: r.left,
                        y: r.top,
                        width: (r.right - r.left) as u32,
                        height: (r.bottom - r.top) as u32,
                    };
                    let size = u64::from(candidate.width) * u64::from(candidate.height);
                    if candidate.contains(probe.x, probe.y) && size < area {
                        area = size;
                        smallest = Some(current.clone());
                    }
                }
            }
            child = unsafe { walker.GetNextSiblingElement(&current) }.ok();
        }
        let Some(next) = smallest else {
            break;
        };
        if next.as_raw() == element.as_raw() {
            break;
        }
        element = next;
    }
    candidates.reverse();
    Ok(candidates)
}
