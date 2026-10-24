use super::{
    accessibility::PointInspector,
    hit_test::{self, HitTree},
};
use super::{CaptureCandidate, CaptureHitTest, CaptureRect};
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
use windows::Win32::{
    Foundation::HWND,
    System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    },
    UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationCacheRequest, IUIAutomationCondition,
        IUIAutomationElement, TreeScope_Children, TreeScope_Element,
        UIA_BoundingRectanglePropertyId, UIA_IsOffscreenPropertyId, UIA_NamePropertyId,
    },
};

const FLAG: &str = "--popper-capture-helper";
#[derive(Clone, Copy)]
enum Provider {
    Accessibility,
    Automation,
}
impl Provider {
    fn flag(self) -> &'static str {
        match self {
            Self::Accessibility => "--capture-provider=accessibility",
            Self::Automation => "--capture-provider=automation",
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Probe {
    session: String,
    handle: usize,
    x: i32,
    y: i32,
}
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
struct Lane {
    pending: Arc<(Mutex<Option<Job>>, Condvar)>,
    stopped: Arc<AtomicBool>,
}
impl Lane {
    fn new(provider: Provider) -> Self {
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
    fn submit(&self, probe: Probe, reply: Option<mpsc::SyncSender<CaptureHitTest>>) {
        if !self.stopped.load(Ordering::Acquire) {
            *self.pending.0.lock() = Some(Job { probe, reply });
            self.pending.1.notify_one();
        }
    }
    fn stop(&self) {
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

pub struct Recognition {
    accessibility: Lane,
    automation: Lane,
}
impl Recognition {
    pub fn new() -> Self {
        Self {
            accessibility: Lane::new(Provider::Accessibility),
            automation: Lane::new(Provider::Automation),
        }
    }
    pub fn warm_up(&self, session: &str, handle: usize, x: i32, y: i32) {
        let probe = Probe {
            session: session.into(),
            handle,
            x,
            y,
        };
        self.accessibility.submit(probe.clone(), None);
        self.automation.submit(probe, None);
    }
    pub fn query(&self, session: &str, handle: usize, x: i32, y: i32) -> CaptureHitTest {
        let probe = Probe {
            session: session.into(),
            handle,
            x,
            y,
        };
        let (reply, results) = mpsc::sync_channel(2);
        self.accessibility
            .submit(probe.clone(), Some(reply.clone()));
        self.automation.submit(probe, Some(reply));
        hit_test::collect_providers(
            results,
            2,
            Duration::from_millis(150),
            Duration::from_millis(12),
        )
    }
    pub fn stop(&self) {
        self.accessibility.stop();
        self.automation.stop();
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
        let point_provider = std::env::args_os().any(|arg| arg == Provider::Accessibility.flag());
        let automation = if point_provider {
            None
        } else {
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .ok()
                .map(Inspector::new)
        };
        let mut automation = automation;
        let mut point_inspector = PointInspector::default();
        if point_provider || automation.is_some() {
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
                let candidates = if point_provider {
                    point_inspector.inspect(&probe.session, probe.handle, probe.x, probe.y)
                } else {
                    automation.as_mut().unwrap().inspect(probe)
                }
                .unwrap_or_default();
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
struct Node {
    element: IUIAutomationElement,
    candidate: Option<CaptureCandidate>,
    offscreen: bool,
    children: Option<Vec<usize>>,
}
impl Node {
    fn cached(element: IUIAutomationElement) -> Self {
        let candidate = unsafe { element.CachedBoundingRectangle() }
            .ok()
            .and_then(|rect| {
                (rect.right > rect.left && rect.bottom > rect.top).then(|| CaptureCandidate {
                    rect: CaptureRect {
                        x: rect.left,
                        y: rect.top,
                        width: (i64::from(rect.right) - i64::from(rect.left)) as u32,
                        height: (i64::from(rect.bottom) - i64::from(rect.top)) as u32,
                    },
                    label: unsafe { element.CachedName() }
                        .map(|name| name.to_string().chars().take(64).collect())
                        .unwrap_or_default(),
                })
            });
        let offscreen = unsafe { element.CachedIsOffscreen() }.is_ok_and(|value| value.as_bool());
        Self {
            element,
            candidate,
            offscreen,
            children: None,
        }
    }
}
struct CachedTree {
    nodes: Vec<Node>,
    cache: IUIAutomationCacheRequest,
    condition: IUIAutomationCondition,
}
impl HitTree for CachedTree {
    fn candidate(&self, node: usize) -> Option<&CaptureCandidate> {
        self.nodes[node].candidate.as_ref()
    }
    fn children(&mut self, node: usize) -> Vec<usize> {
        if let Some(children) = &self.nodes[node].children {
            return children.clone();
        }
        let mut children = Vec::new();
        if !self.nodes[node].offscreen && self.nodes.len() < 8192 {
            // Batch all child properties in one provider round trip. TrueCondition
            // includes raw text, images and layout elements omitted by Control View.
            if let Ok(elements) = unsafe {
                self.nodes[node].element.FindAllBuildCache(
                    TreeScope_Children,
                    &self.condition,
                    &self.cache,
                )
            } {
                let count = unsafe { elements.Length() }
                    .unwrap_or_default()
                    .min((8192 - self.nodes.len()) as i32);
                for index in 0..count {
                    if let Ok(element) = unsafe { elements.GetElement(index) } {
                        let child = Node::cached(element);
                        if child.offscreen {
                            continue;
                        }
                        children.push(self.nodes.len());
                        self.nodes.push(child);
                    }
                }
            }
        }
        self.nodes[node].children = Some(children.clone());
        children
    }
}
struct Inspector {
    automation: IUIAutomation,
    session: String,
    // Bound provider references and reuse sibling geometry while moving within a window.
    trees: Vec<(usize, CachedTree)>,
}
impl Inspector {
    fn new(automation: IUIAutomation) -> Self {
        Self {
            automation,
            session: String::new(),
            trees: Vec::new(),
        }
    }
    fn inspect(&mut self, probe: Probe) -> windows::core::Result<CaptureHitTest> {
        if self.session != probe.session {
            self.trees.clear();
            self.session = probe.session;
        }
        let tree = if let Some(index) = self
            .trees
            .iter()
            .position(|(handle, _)| *handle == probe.handle)
        {
            self.trees.remove(index).1
        } else {
            let cache = unsafe { self.automation.CreateCacheRequest()? };
            unsafe {
                cache.SetTreeScope(TreeScope_Element)?;
                cache.SetTreeFilter(&self.automation.RawViewCondition()?)?;
                cache.AddProperty(UIA_BoundingRectanglePropertyId)?;
                cache.AddProperty(UIA_NamePropertyId)?;
                cache.AddProperty(UIA_IsOffscreenPropertyId)?;
            }
            let root = unsafe {
                self.automation
                    .ElementFromHandleBuildCache(HWND(probe.handle as *mut _), &cache)?
            };
            CachedTree {
                nodes: vec![Node::cached(root)],
                cache,
                condition: unsafe { self.automation.CreateTrueCondition()? },
            }
        };
        if self.trees.len() >= 8 {
            self.trees.remove(0);
        }
        self.trees.push((probe.handle, tree));
        let tree = &mut self.trees.last_mut().unwrap().1;
        Ok(hit_test::search(
            tree,
            probe.x,
            probe.y,
            Duration::from_millis(100),
        ))
    }
}
