//! Windows selection capture using low-level input hooks and UI Automation.
//!
//! The hooks live on their own message-pumped thread. Their callbacks only
//! copy the small input record into an `mpsc` queue. Selection reads run in a
//! private, versioned helper process so a stuck third-party UI Automation/OLE
//! provider can be isolated and replaced without poisoning the hook/event
//! worker or later captures. All public Windows coordinates stay in physical
//! virtual-desktop pixels so mixed-DPI monitor regions remain unambiguous.

use super::{
    automatic_selection_fingerprint, automatic_selection_pointer_matches_bounds,
    classify_windows_mouse_selection, direction_from_points, selection_bounds_are_reasonable,
    union_selection_bounds, windows_text_budget, DismissEvent, SelectionBounds, SelectionDirection,
    SelectionError, SelectionEvent, SelectionMethod, SelectionMouse, SelectionPayload,
    SelectionPoint, SelectionTrigger, SourceApplication,
};
use crate::clipboard;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    cell::Cell,
    collections::{HashMap, HashSet},
    ffi::c_void,
    io::{self, BufReader, BufWriter, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError},
        Arc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows::{
    core::PWSTR,
    Win32::{
        Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        },
        Security::{
            GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
            TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
        },
        System::{
            Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, SAFEARRAY},
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            },
            Ole::{
                OleFlushClipboard, OleInitialize, OleUninitialize, SafeArrayAccessData,
                SafeArrayDestroy, SafeArrayGetDim, SafeArrayGetLBound, SafeArrayGetUBound,
                SafeArrayUnaccessData,
            },
            Threading::{
                GetCurrentProcessId, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
                PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            },
        },
        UI::{
            Accessibility::{
                CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
                IUIAutomationTextRange, SetWinEventHook, TextPatternRangeEndpoint_End,
                TextPatternRangeEndpoint_Start, UIA_DocumentControlTypeId, UIA_EditControlTypeId,
                UIA_HyperlinkControlTypeId, UIA_TextControlTypeId, UIA_TextPatternId,
                UnhookWinEvent,
            },
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
                KEYEVENTF_KEYUP, VK_A, VK_C, VK_CONTROL, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME,
                VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_NEXT, VK_PRIOR,
                VK_RCONTROL, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_UP,
            },
            WindowsAndMessaging::{
                CallNextHookEx, DispatchMessageW, GetAncestor, GetCursorPos, GetForegroundWindow,
                GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsZoomed,
                MsgWaitForMultipleObjectsEx, PeekMessageW, PostThreadMessageW, SetWindowsHookExW,
                TranslateMessage, UnhookWindowsHookEx, WindowFromPoint, EVENT_SYSTEM_FOREGROUND,
                GA_ROOT, HC_ACTION, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT, MWMO_INPUTAVAILABLE,
                PM_REMOVE, QS_ALLINPUT, WH_KEYBOARD_LL, WH_MOUSE_LL, WINEVENT_OUTOFCONTEXT,
                WINEVENT_SKIPOWNPROCESS, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
                WM_MBUTTONDOWN, WM_MOUSEHWHEEL, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN,
                WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN,
            },
        },
    },
};

const WORKER_PUMP_INTERVAL: Duration = Duration::from_millis(12);
const HOOK_HEALTH_INTERVAL: Duration = Duration::from_millis(50);
const HOOK_RESTART_DELAY: Duration = Duration::from_secs(1);
const HOOK_EXIT_RECHECK_DELAY: Duration = Duration::from_millis(10);
const HOOK_SUPERVISION_INTERVAL: Duration = Duration::from_secs(5);
const MAX_MESSAGES_PER_TICK: usize = 64;
// Low-level hooks run before the target application consumes mouse-up/key-up.
// One short frame is enough for the common UIA path; the retries below cover
// providers which publish their selection asynchronously.
const CAPTURE_SETTLE_DELAY: Duration = Duration::from_millis(12);
// After an action opens a result window, Windows may deliver the source
// application's mouse-up before its foreground activation has completed. Do
// not start a capture against the still-foreground TextLens window; wait for
// the original source process to become foreground for a short, bounded time.
// Normal selections are already foreground and therefore do not pay this
// delay.
const FOREGROUND_SETTLE_TIMEOUT: Duration = Duration::from_millis(500);
const FOREGROUND_SETTLE_RETRY: Duration = Duration::from_millis(8);
const ACCESSIBILITY_RETRY_DELAYS: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_millis(24),
    Duration::from_millis(72),
];
const CLIPBOARD_POLL_INTERVAL: Duration = Duration::from_millis(10);
const CLIPBOARD_POLL_ATTEMPTS: usize = 60;
const CLIPBOARD_STABLE_POLLS: usize = 3;
const MODIFIER_RELEASE_ATTEMPTS: usize = 12;
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const DOUBLE_CLICK_DISTANCE_SQUARED: f64 = 16.0;
const MAX_DRAG_DURATION: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const CAPTURE_TASK_TIMEOUT: Duration = Duration::from_millis(1_200);
// Leave enough room for the executor to send its reply and unwind before the
// outer task timeout declares a healthy STA lane stuck.
const CAPTURE_ENGINE_BUDGET: Duration = Duration::from_millis(1_080);
const HELPER_START_TIMEOUT: Duration = Duration::from_secs(2);
const HELPER_CANCEL_GRACE: Duration = Duration::from_millis(100);
const HELPER_CLIPBOARD_CANCEL_GRACE: Duration = Duration::from_millis(700);
const PENDING_UNSAFE_HELPER_GRACE: Duration = Duration::from_secs(2);
const CLIPBOARD_QUARANTINE_DURATION: Duration = Duration::from_secs(300);
const HELPER_SHUTDOWN_GRACE: Duration = Duration::from_millis(250);
const HELPER_POLL_INTERVAL: Duration = Duration::from_millis(10);
const HELPER_FRAME_LIMIT: usize = 16 * 1024 * 1024;
const SELECTION_HELPER_FLAG: &str = "--textlens-selection-helper";
const SELECTION_HELPER_PROTOCOL_VERSION: u32 = 1;
const WORKER_SHUTDOWN_GRACE: Duration = Duration::from_millis(250);
const MAX_UIA_ANCESTORS: usize = 32;
const MAX_UIA_RUNTIME_ID_VALUES: usize = 128;
const MAX_PROCESS_ANCESTORS: usize = 32;
const MAX_BOUNDING_VALUES: usize = 262_144;
const MAX_TOTAL_BOUNDING_VALUES: usize = 262_144;
const POINTER_BOUNDS_TOLERANCE: f64 = 24.0;
const AUTOMATIC_DUPLICATE_WINDOW: Duration = Duration::from_millis(1_500);
const SYNTHETIC_COPY_MARKER: usize = 0x544C_4350;

type UnitReply = SyncSender<Result<(), SelectionError>>;
type CaptureReply = SyncSender<Result<Option<SelectionPayload>, SelectionError>>;

pub(super) struct WindowsSelectionMonitor {
    inbox: Sender<WorkerMessage>,
    worker: Mutex<Option<JoinHandle<()>>>,
    shutdown_requested: AtomicBool,
}

impl WindowsSelectionMonitor {
    pub(super) fn new(
        _excluded_identifier: &str,
        event_sender: Sender<SelectionEvent>,
    ) -> Result<Self, SelectionError> {
        let (inbox, receiver) = mpsc::channel();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker_inbox = inbox.clone();
        let worker = thread::Builder::new()
            .name("textlens-selection-uia".to_owned())
            .spawn(move || {
                selection_worker_main(receiver, worker_inbox, event_sender, ready_sender);
            })
            .map_err(|_| SelectionError::NativeInitializationFailed)?;

        match ready_receiver.recv_timeout(REQUEST_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                inbox,
                worker: Mutex::new(Some(worker)),
                shutdown_requested: AtomicBool::new(false),
            }),
            Ok(Err(error)) => {
                reap_worker(worker, WORKER_SHUTDOWN_GRACE);
                Err(error)
            }
            Err(_) => {
                let _ = inbox.send(WorkerMessage::Shutdown);
                reap_worker(worker, WORKER_SHUTDOWN_GRACE);
                Err(SelectionError::NativeInitializationFailed)
            }
        }
    }

    pub(super) fn start(&self) -> Result<(), SelectionError> {
        if self.shutdown_requested.load(Ordering::Acquire) {
            return Err(SelectionError::Internal);
        }
        self.request_unit(WorkerMessage::Start)
    }

    pub(super) fn stop(&self) -> Result<(), SelectionError> {
        if self.shutdown_requested.load(Ordering::Acquire) {
            return Ok(());
        }
        self.request_unit(WorkerMessage::Stop)
    }

    pub(super) fn capture_current(&self) -> Result<Option<SelectionPayload>, SelectionError> {
        if self.shutdown_requested.load(Ordering::Acquire) {
            return Err(SelectionError::Internal);
        }
        let (reply_sender, reply_receiver) = mpsc::sync_channel(1);
        self.inbox
            .send(WorkerMessage::Capture(reply_sender))
            .map_err(|_| SelectionError::Internal)?;
        reply_receiver
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|_| SelectionError::Internal)?
    }

    /// Requests shutdown and gives the worker a small grace period to finish.
    ///
    /// UI Automation and third-party accessibility providers can block inside
    /// COM calls. Never leave the Tauri UI thread waiting indefinitely for
    /// such a provider: a background reaper owns the join after the grace
    /// period expires.
    pub(super) fn shutdown(&self) {
        if !self.shutdown_requested.swap(true, Ordering::AcqRel) {
            let _ = self.inbox.send(WorkerMessage::Shutdown);
        }
        let worker = self.worker.lock().ok().and_then(|mut worker| worker.take());
        if let Some(worker) = worker {
            reap_worker(worker, WORKER_SHUTDOWN_GRACE);
        }
    }

    fn request_unit(
        &self,
        make_message: impl FnOnce(UnitReply) -> WorkerMessage,
    ) -> Result<(), SelectionError> {
        let (reply_sender, reply_receiver) = mpsc::sync_channel(1);
        self.inbox
            .send(make_message(reply_sender))
            .map_err(|_| SelectionError::Internal)?;
        reply_receiver
            .recv_timeout(REQUEST_TIMEOUT)
            .map_err(|_| SelectionError::Internal)?
    }
}

impl Drop for WindowsSelectionMonitor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Joins a worker without allowing its caller to wait longer than `grace`.
/// The waiter thread is intentionally detached when the target does not stop
/// in time; process shutdown must not be held hostage by a stuck COM provider.
fn reap_worker(worker: JoinHandle<()>, grace: Duration) -> bool {
    if worker.is_finished() {
        let _ = worker.join();
        return true;
    }

    let (done_sender, done_receiver) = mpsc::sync_channel(1);
    let _reaper = thread::Builder::new()
        .name("textlens-selection-reaper".to_owned())
        .spawn(move || {
            let _ = worker.join();
            let _ = done_sender.send(());
        });
    done_receiver.recv_timeout(grace).is_ok()
}

enum WorkerMessage {
    Start(UnitReply),
    Stop(UnitReply),
    Capture(CaptureReply),
    Raw(RawInput),
    HookExited { instance_id: u64 },
    Shutdown,
}

#[derive(Debug, Clone, Copy)]
enum RawInput {
    Mouse {
        sequence: u64,
        message: u32,
        point: RawPoint,
        generation: u64,
        modifiers: ModifierSnapshot,
        timestamp_ms: u64,
    },
    Keyboard {
        sequence: u64,
        message: u32,
        virtual_key: u32,
        generation: u64,
        modifiers: ModifierSnapshot,
        timestamp_ms: u64,
    },
    Foreground {
        sequence: u64,
        window: isize,
        generation: u64,
        timestamp_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, Default)]
struct ModifierSnapshot {
    shift: bool,
    control: bool,
    alt: bool,
    windows: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct RawPoint {
    x: i32,
    y: i32,
}

impl RawPoint {
    fn distance_squared(self, other: Self) -> f64 {
        let dx = f64::from(self.x) - f64::from(other.x);
        let dy = f64::from(self.y) - f64::from(other.y);
        dx * dx + dy * dy
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct CaptureRequest {
    trigger: SelectionTrigger,
    start: Option<RawPoint>,
    end: Option<RawPoint>,
    current: RawPoint,
    generation: Option<u64>,
}

struct PendingCapture {
    due: Instant,
    expires_at: Instant,
    request: CaptureRequest,
    source_root_window: isize,
    source_process_id: u32,
    process_parents: Option<Option<HashMap<u32, u32>>>,
}

#[derive(Debug, Clone, Copy)]
struct RecentCaptureContext {
    generation: u64,
    source_root_window: isize,
    source_process_id: u32,
    valid_until: Instant,
}

const RECENT_CAPTURE_FOREGROUND_GRACE: Duration = Duration::from_millis(1_500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForegroundInteractionDecision {
    Ignore,
    PreserveMouseGesture,
    PreservePendingCapture,
    Dismiss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingForegroundDecision {
    Capture,
    Wait,
    Expire,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingUnsafeHelperObservation {
    None,
    Restored { clipboard_sequence: u32 },
    Completed,
    Disconnected { process_exited: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingUnsafeHelperAction {
    Keep,
    BecomeCustodian { clipboard_sequence: u32 },
    Shutdown,
    DropExited,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum HelperCancelReason {
    InputChanged,
    Timeout,
    Shutdown,
}

#[derive(Debug, Serialize, Deserialize)]
enum HelperCommand {
    Capture {
        request_id: u64,
        request: CaptureRequest,
        source_window: isize,
        source_process_id: u32,
        blocked_clipboard_process_ids: Vec<u32>,
    },
    Cancel {
        request_id: u64,
        reason: HelperCancelReason,
    },
    BecomeCustodian {
        clipboard_sequence: u32,
    },
    Shutdown,
    FinalShutdown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
enum HelperPhase {
    ClipboardPrepared { process_id: u32 },
    ClipboardInjected { process_id: u32 },
}

#[derive(Debug, Serialize, Deserialize)]
enum HelperCaptureResult {
    Selection {
        selection: SelectionPayload,
        source_window: isize,
        process_id: u32,
    },
    Empty,
    Error,
}

#[derive(Debug, Serialize, Deserialize)]
enum HelperEvent {
    Ready {
        protocol_version: u32,
    },
    Phase {
        request_id: u64,
        phase: HelperPhase,
    },
    Result {
        request_id: u64,
        result: HelperCaptureResult,
        clipboard: HelperClipboardState,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
struct HelperClipboardState {
    used: bool,
    restored: bool,
    restored_sequence: u32,
    safe_to_terminate: bool,
}

impl Default for HelperClipboardState {
    fn default() -> Self {
        Self {
            used: false,
            restored: false,
            restored_sequence: 0,
            safe_to_terminate: true,
        }
    }
}

fn selection_worker_main(
    receiver: Receiver<WorkerMessage>,
    inbox: Sender<WorkerMessage>,
    event_sender: Sender<SelectionEvent>,
    ready_sender: SyncSender<Result<(), SelectionError>>,
) {
    let _ = ready_sender.send(Ok(()));

    let mut worker = SelectionWorker {
        inbox,
        event_sender,
        own_process_id: unsafe { GetCurrentProcessId() },
        // Start the COM lane with the monitor instead of paying OleInitialize
        // and CoCreateInstance costs after every mouse-up. A timed-out lane is
        // discarded and replaced, preserving provider-hang isolation.
        capture_executor: CaptureExecutor::spawn(unsafe { GetCurrentProcessId() }).ok(),
        hook_thread: None,
        desired_listening: false,
        hook_restart_due: None,
        hook_exit_check_due: None,
        hook_health_due: None,
        mouse_down: None,
        mouse_down_at: None,
        mouse_down_on_self: false,
        mouse_down_target_root: 0,
        mouse_down_shift: false,
        last_mouse_up: None,
        last_click: None,
        keyboard_selection_key: None,
        pending_capture: None,
        last_automatic_fingerprint: None,
        recent_capture: None,
        last_raw_sequence: 0,
    };
    worker.run(receiver);
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, SelectionError> {
        // UI Automation and the OLE clipboard both live on this dedicated STA.
        // OleInitialize performs the required apartment initialization while
        // also making OleGetClipboard/OleSetClipboard safe to use below.
        unsafe { OleInitialize(None) }.map_err(|_| SelectionError::NativeInitializationFailed)?;
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { OleUninitialize() };
    }
}

struct SelectionWorker {
    inbox: Sender<WorkerMessage>,
    event_sender: Sender<SelectionEvent>,
    own_process_id: u32,
    capture_executor: Option<CaptureExecutor>,
    hook_thread: Option<HookThread>,
    desired_listening: bool,
    hook_restart_due: Option<Instant>,
    hook_exit_check_due: Option<(u64, Instant)>,
    hook_health_due: Option<Instant>,
    mouse_down: Option<RawPoint>,
    mouse_down_at: Option<Instant>,
    mouse_down_on_self: bool,
    mouse_down_target_root: isize,
    mouse_down_shift: bool,
    last_mouse_up: Option<RawPoint>,
    last_click: Option<(Instant, RawPoint)>,
    keyboard_selection_key: Option<u32>,
    pending_capture: Option<PendingCapture>,
    last_automatic_fingerprint: Option<(u64, u64, Instant)>,
    recent_capture: Option<RecentCaptureContext>,
    last_raw_sequence: u64,
}

impl SelectionWorker {
    fn run(&mut self, receiver: Receiver<WorkerMessage>) {
        let mut shutdown = false;
        while !shutdown {
            let now = Instant::now();
            let next_deadline = self
                .pending_capture
                .as_ref()
                .map(|pending| pending.due)
                .into_iter()
                .chain(self.hook_restart_due)
                .chain(self.hook_exit_check_due.map(|(_, due)| due))
                .chain(self.hook_health_due)
                .chain(
                    self.capture_executor
                        .as_ref()
                        .and_then(CaptureExecutor::maintenance_due),
                )
                .min();
            if let Some(deadline) = next_deadline {
                match receiver.recv_timeout(deadline.saturating_duration_since(now)) {
                    Ok(message) => shutdown = self.handle_message(message),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => shutdown = true,
                }
            } else {
                match receiver.recv() {
                    Ok(message) => shutdown = self.handle_message(message),
                    Err(_) => shutdown = true,
                }
            }
            let mut drained = 0usize;
            while !shutdown && drained < MAX_MESSAGES_PER_TICK {
                match receiver.try_recv() {
                    Ok(message) => {
                        drained += 1;
                        shutdown = self.handle_message(message);
                    }
                    Err(_) => break,
                }
            }
            self.restart_hooks_if_due();
            self.reap_exited_hook_if_due();
            self.check_hook_health_if_due();
            if let Some(executor) = self.capture_executor.as_mut() {
                executor.maintain_pending_helper();
            }
            self.run_pending_capture();
        }
        self.pending_capture = None;
        if let Some(mut hooks) = self.hook_thread.take() {
            let _ = hooks.stop();
        }
        if let Some(executor) = self.capture_executor.take() {
            executor.shutdown(WORKER_SHUTDOWN_GRACE);
        }
    }

    /// Returns true when the worker should terminate.
    fn handle_message(&mut self, message: WorkerMessage) -> bool {
        match message {
            WorkerMessage::Start(reply) => {
                self.desired_listening = true;
                let result = self.ensure_hooks_running();
                let _ = reply.send(result);
            }
            WorkerMessage::Stop(reply) => {
                self.desired_listening = false;
                self.hook_restart_due = None;
                self.hook_exit_check_due = None;
                self.hook_health_due = None;
                self.reset_interaction_state();
                let result = self
                    .hook_thread
                    .take()
                    .map_or(Ok(()), |mut thread| thread.stop());
                let _ = reply.send(result);
            }
            WorkerMessage::Capture(reply) => {
                let current = current_cursor_position();
                let result = self.capture_with_timeout(CaptureRequest {
                    trigger: SelectionTrigger::Manual,
                    start: None,
                    end: None,
                    current,
                    generation: None,
                });
                let _ = reply.send(result);
            }
            WorkerMessage::Raw(input) => {
                if self.hook_thread.is_some() {
                    self.handle_raw_input(input);
                }
            }
            WorkerMessage::HookExited { instance_id } => {
                self.note_hook_exit(instance_id);
            }
            WorkerMessage::Shutdown => return true,
        }
        false
    }

    fn ensure_hooks_running(&mut self) -> Result<(), SelectionError> {
        if self
            .hook_thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            self.hook_restart_due = None;
            self.hook_health_due = Some(Instant::now() + HOOK_SUPERVISION_INTERVAL);
            return Ok(());
        }
        if let Some(mut stale) = self.hook_thread.take() {
            if stale.is_finished() {
                if let Some(join) = stale.join.take() {
                    let _ = join.join();
                }
                let _ = set_hook_inbox(None);
            } else {
                let _ = stale.stop();
            }
        }
        match HookThread::start(self.inbox.clone()) {
            Ok(thread) => {
                self.hook_thread = Some(thread);
                self.hook_restart_due = None;
                self.hook_health_due = Some(Instant::now() + HOOK_SUPERVISION_INTERVAL);
                Ok(())
            }
            Err(error) => {
                if self.desired_listening {
                    self.hook_restart_due = Some(Instant::now() + HOOK_RESTART_DELAY);
                }
                Err(error)
            }
        }
    }

    fn restart_hooks_if_due(&mut self) {
        if !self.desired_listening
            || !self
                .hook_restart_due
                .is_some_and(|due| Instant::now() >= due)
        {
            return;
        }
        if let Err(error) = self.ensure_hooks_running() {
            eprintln!("[selection] failed to restart Windows input hooks: {error}");
        }
    }

    fn note_hook_exit(&mut self, instance_id: u64) {
        if !self
            .hook_thread
            .as_ref()
            .is_some_and(|thread| thread.instance_id == instance_id)
        {
            return;
        }
        if !self
            .hook_thread
            .as_ref()
            .is_some_and(HookThread::is_finished)
        {
            self.hook_exit_check_due =
                Some((instance_id, Instant::now() + HOOK_EXIT_RECHECK_DELAY));
            return;
        }
        if let Some(mut thread) = self.hook_thread.take() {
            if let Some(join) = thread.join.take() {
                let _ = join.join();
            }
        }
        self.hook_exit_check_due = None;
        self.reset_interaction_state();
        if self.desired_listening {
            self.hook_restart_due = Some(Instant::now() + HOOK_RESTART_DELAY);
        }
        self.hook_health_due = None;
    }

    fn reap_exited_hook_if_due(&mut self) {
        let Some((instance_id, due)) = self.hook_exit_check_due else {
            return;
        };
        if Instant::now() < due {
            return;
        }
        self.hook_exit_check_due = None;
        self.note_hook_exit(instance_id);
    }

    fn check_hook_health_if_due(&mut self) {
        if !self.desired_listening {
            self.hook_health_due = None;
            return;
        }
        if !self
            .hook_health_due
            .is_some_and(|due| Instant::now() >= due)
        {
            return;
        }
        self.hook_health_due = Some(Instant::now() + HOOK_SUPERVISION_INTERVAL);
        match self.hook_thread.as_ref() {
            Some(thread) if thread.is_finished() => {
                let instance_id = thread.instance_id;
                self.note_hook_exit(instance_id);
            }
            Some(_) => {}
            None => {
                self.hook_restart_due = Some(Instant::now());
            }
        }
    }

    fn handle_raw_input(&mut self, input: RawInput) {
        let sequence = match input {
            RawInput::Mouse { sequence, .. }
            | RawInput::Keyboard { sequence, .. }
            | RawInput::Foreground { sequence, .. } => sequence,
        };
        if sequence <= self.last_raw_sequence {
            return;
        }
        self.last_raw_sequence = sequence;
        match input {
            RawInput::Mouse {
                sequence: _,
                message,
                point,
                generation,
                modifiers,
                timestamp_ms,
            } => self.handle_mouse(message, point, generation, modifiers, timestamp_ms),
            RawInput::Keyboard {
                sequence: _,
                message,
                virtual_key,
                generation,
                modifiers,
                timestamp_ms,
            } => self.handle_keyboard(message, virtual_key, generation, modifiers, timestamp_ms),
            RawInput::Foreground {
                sequence: _,
                window,
                generation,
                timestamp_ms,
            } => self.handle_foreground(window, generation, timestamp_ms),
        }
    }

    fn handle_foreground(&mut self, window: isize, generation: u64, timestamp_ms: u64) {
        let window = HWND(window as *mut c_void);
        if unsafe { GetForegroundWindow() } != window {
            // WINEVENT_OUTOFCONTEXT may deliver an older activation after a
            // newer foreground transition has already completed.
            return;
        }
        let foreground_process_id = window_process_id(window);
        let foreground_root = root_window(window).0 as isize;
        if self.recent_capture_matches_foreground(
            foreground_root,
            foreground_process_id,
            generation,
        ) {
            // Out-of-context WinEvent callbacks can arrive after the helper
            // has emitted the selection. A same-source callback belongs to
            // that completed gesture and must not dismiss its toolbar.
            return;
        }
        match foreground_interaction_decision(
            !window.is_invalid(),
            foreground_process_id,
            self.own_process_id,
            self.mouse_down.is_some(),
            self.mouse_down_on_self,
            self.pending_capture.is_some(),
        ) {
            ForegroundInteractionDecision::Ignore => return,
            ForegroundInteractionDecision::PreserveMouseGesture => {
                self.keyboard_selection_key = None;
                self.last_click = None;
                self.last_automatic_fingerprint = None;
                return;
            }
            ForegroundInteractionDecision::PreservePendingCapture => {
                correlate_pending_capture_with_foreground(
                    &mut self.pending_capture,
                    foreground_root,
                    foreground_process_id,
                    generation,
                );
                self.keyboard_selection_key = None;
                self.last_click = None;
                self.last_automatic_fingerprint = None;
                return;
            }
            ForegroundInteractionDecision::Dismiss => {}
        }
        self.reset_interaction_state();
        self.emit_dismiss(
            "foregroundChanged",
            current_cursor_position(),
            window,
            timestamp_ms,
        );
    }

    fn reset_interaction_state(&mut self) {
        self.pending_capture = None;
        self.mouse_down = None;
        self.mouse_down_at = None;
        self.mouse_down_on_self = false;
        self.mouse_down_target_root = 0;
        self.mouse_down_shift = false;
        self.last_mouse_up = None;
        self.last_click = None;
        self.keyboard_selection_key = None;
        self.last_automatic_fingerprint = None;
        self.recent_capture = None;
    }

    fn recent_capture_matches_foreground(
        &mut self,
        foreground_root: isize,
        foreground_process_id: u32,
        generation: u64,
    ) -> bool {
        let Some(recent) = self.recent_capture else {
            return false;
        };
        if Instant::now() >= recent.valid_until {
            self.recent_capture = None;
            return false;
        }
        let same_source = capture_windows_are_related(
            recent.source_root_window,
            recent.source_process_id,
            foreground_root,
            foreground_process_id,
        );
        if same_source && generation <= recent.generation {
            return true;
        }
        if generation > recent.generation {
            self.recent_capture = None;
        }
        false
    }

    fn handle_mouse(
        &mut self,
        message: u32,
        point: RawPoint,
        generation: u64,
        modifiers: ModifierSnapshot,
        timestamp_ms: u64,
    ) {
        // Every unfiltered mouse event represents input newer than a queued
        // capture. Mouse-up may immediately replace it with a capture for the
        // gesture being completed below.
        self.pending_capture = None;
        self.recent_capture = None;
        let target_window = window_at_point(point);
        let target_root = root_window(target_window);
        let target_is_self = window_process_id(target_root) == self.own_process_id;
        match message {
            WM_LBUTTONDOWN => {
                self.keyboard_selection_key = None;
                if target_is_self {
                    self.last_click = None;
                    self.last_mouse_up = None;
                }
                self.mouse_down = Some(point);
                self.mouse_down_at = Some(Instant::now());
                self.mouse_down_on_self = target_is_self;
                self.mouse_down_target_root = target_root.0 as isize;
                self.mouse_down_shift =
                    modifiers.shift && !modifiers.control && !modifiers.alt && !modifiers.windows;
                // Self clicks still produce Dismiss so runtime can preserve a
                // click inside the no-activate toolbar while closing it for a
                // click elsewhere in a TextLens window. Mouse-up below never
                // schedules UIA capture for the self process.
                self.emit_dismiss("mouseDown", point, target_window, timestamp_ms);
            }
            WM_LBUTTONUP => {
                let start = self.mouse_down.take().unwrap_or(point);
                let now = Instant::now();
                let press_duration = self
                    .mouse_down_at
                    .take()
                    .map(|at| now.saturating_duration_since(at));
                let drag_duration_valid =
                    press_duration.is_some_and(|duration| duration <= MAX_DRAG_DURATION);
                let click_duration_valid =
                    press_duration.is_some_and(|duration| duration <= DOUBLE_CLICK_INTERVAL);
                let began_on_self = std::mem::take(&mut self.mouse_down_on_self);
                let gesture_root = std::mem::take(&mut self.mouse_down_target_root);
                let previous_mouse_up = self.last_mouse_up;
                if began_on_self || target_is_self {
                    return;
                }
                let is_double_click = self.last_click.is_some_and(|(at, previous)| {
                    now.saturating_duration_since(at) <= DOUBLE_CLICK_INTERVAL
                        && previous.distance_squared(point) <= DOUBLE_CLICK_DISTANCE_SQUARED
                });
                let start_point = raw_selection_point(start);
                let end_point = raw_selection_point(point);
                let trigger = classify_windows_mouse_selection(
                    start_point,
                    end_point,
                    self.mouse_down_shift,
                    is_double_click,
                    drag_duration_valid,
                );
                self.last_mouse_up = Some(point);
                if click_duration_valid
                    && start.distance_squared(point) < DOUBLE_CLICK_DISTANCE_SQUARED
                {
                    self.last_click = Some((now, point));
                } else {
                    self.last_click = None;
                }
                if let Some(trigger) = trigger {
                    let capture_start = match trigger {
                        SelectionTrigger::DoubleClick => point,
                        SelectionTrigger::ShiftClick => previous_mouse_up.unwrap_or(start),
                        _ => start,
                    };
                    self.schedule_capture(
                        CaptureRequest {
                            trigger,
                            start: Some(capture_start),
                            end: Some(point),
                            current: point,
                            generation: Some(generation),
                        },
                        if gesture_root == 0 {
                            target_root.0 as isize
                        } else {
                            gesture_root
                        },
                    );
                }
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                self.emit_dismiss("scroll", point, target_window, timestamp_ms);
            }
            WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
                self.emit_dismiss("mouseDown", point, target_window, timestamp_ms);
            }
            _ => {}
        }
    }

    fn handle_keyboard(
        &mut self,
        message: u32,
        virtual_key: u32,
        generation: u64,
        modifiers: ModifierSnapshot,
        timestamp_ms: u64,
    ) {
        if !is_modifier_virtual_key(virtual_key as u16) {
            // Synthetic TextLens keys are filtered in the hook callback. Any
            // remaining non-modifier key is genuine newer input and must
            // invalidate a capture which has not started yet.
            self.pending_capture = None;
            self.recent_capture = None;
        }
        let foreground = unsafe { GetForegroundWindow() };
        if window_process_id(foreground) == self.own_process_id {
            if (message == WM_KEYUP || message == WM_SYSKEYUP)
                && self.keyboard_selection_key == Some(virtual_key)
            {
                self.keyboard_selection_key = None;
            }
            return;
        }
        if message == WM_KEYUP || message == WM_SYSKEYUP {
            if self.keyboard_selection_key == Some(virtual_key) {
                self.keyboard_selection_key = None;
                let point = current_cursor_position();
                self.schedule_capture(
                    CaptureRequest {
                        trigger: SelectionTrigger::Keyboard,
                        start: None,
                        end: None,
                        current: point,
                        generation: Some(generation),
                    },
                    root_window(foreground).0 as isize,
                );
            }
            return;
        }
        if message != WM_KEYDOWN && message != WM_SYSKEYDOWN {
            return;
        }
        if is_modifier_virtual_key(virtual_key as u16) {
            return;
        }

        let point = current_cursor_position();
        self.emit_dismiss("keyDown", point, foreground, timestamp_ms);
        let navigation = matches!(
            virtual_key as u16,
            key if key == VK_LEFT.0
                || key == VK_RIGHT.0
                || key == VK_UP.0
                || key == VK_DOWN.0
                || key == VK_HOME.0
                || key == VK_END.0
                || key == VK_PRIOR.0
                || key == VK_NEXT.0
        );
        let select_all = modifiers.control
            && !modifiers.alt
            && !modifiers.windows
            && virtual_key as u16 == VK_A.0;
        if (modifiers.shift && navigation) || select_all {
            self.keyboard_selection_key = Some(virtual_key);
        } else {
            self.keyboard_selection_key = None;
        }
    }

    fn schedule_capture(&mut self, request: CaptureRequest, source_root_window: isize) {
        let now = Instant::now();
        let source_process_id = window_process_id(HWND(source_root_window as *mut c_void));
        trace_selection_capture("scheduled", &request, source_root_window, source_process_id);
        self.pending_capture = Some(PendingCapture {
            due: now + CAPTURE_SETTLE_DELAY,
            expires_at: now + FOREGROUND_SETTLE_TIMEOUT,
            request,
            source_root_window,
            source_process_id,
            process_parents: None,
        });
    }

    fn run_pending_capture(&mut self) {
        if !self
            .pending_capture
            .as_ref()
            .is_some_and(|pending| Instant::now() >= pending.due)
        {
            return;
        }
        let Some(pending) = self.pending_capture.as_mut() else {
            return;
        };

        // A result window can still own the foreground for a few frames after
        // the user starts the next drag. Wait only while there is no usable
        // external foreground. The HWND/PID observed at mouse-down is merely
        // correlation metadata: WindowFromPoint roots are not stable across
        // shell hosts, renderer processes, owned popups, or window recreation.
        // Once any external foreground exists, the helper's existing exact
        // HWND/PID/generation checks remain the capture authority.
        let foreground = unsafe { GetForegroundWindow() };
        let foreground_process_id = window_process_id(foreground);
        let foreground_root = root_window(foreground).0 as isize;
        let source_root_window = pending.source_root_window;
        let source_process_id = pending.source_process_id;
        let source_related = capture_windows_are_related_with_parents(
            source_root_window,
            source_process_id,
            foreground_root,
            foreground_process_id,
            &mut pending.process_parents,
        );
        match pending_foreground_decision(
            !foreground.is_invalid(),
            foreground_process_id,
            self.own_process_id,
            source_related,
            Instant::now() < pending.expires_at,
        ) {
            PendingForegroundDecision::Wait => {
                trace_selection_capture(
                    "wait-foreground",
                    &pending.request,
                    pending.source_root_window,
                    foreground_process_id,
                );
                pending.due = Instant::now() + FOREGROUND_SETTLE_RETRY;
                return;
            }
            PendingForegroundDecision::Expire => {
                trace_selection_capture(
                    "expired",
                    &pending.request,
                    pending.source_root_window,
                    foreground_process_id,
                );
                self.pending_capture = None;
                return;
            }
            PendingForegroundDecision::Capture => {}
        }

        let request = self
            .pending_capture
            .take()
            .expect("pending capture was checked above")
            .request;
        trace_selection_capture(
            "capture",
            &request,
            root_window(foreground).0 as isize,
            foreground_process_id,
        );
        match self.capture_with_timeout(request) {
            Ok(Some(selection)) => {
                self.deliver_captured_selection(
                    request,
                    selection,
                    root_window(foreground).0 as isize,
                    foreground_process_id,
                );
            }
            Ok(None) => trace_selection_capture(
                "empty",
                &request,
                root_window(foreground).0 as isize,
                foreground_process_id,
            ),
            Err(_) => trace_selection_capture(
                "error",
                &request,
                root_window(foreground).0 as isize,
                foreground_process_id,
            ),
        }
    }

    fn deliver_captured_selection(
        &mut self,
        request: CaptureRequest,
        mut selection: SelectionPayload,
        source_root_window: isize,
        source_process_id: u32,
    ) -> bool {
        if request
            .generation
            .is_some_and(|generation| generation != hook_generation().load(Ordering::Acquire))
        {
            trace_selection_capture(
                "discarded-generation",
                &request,
                source_root_window,
                source_process_id,
            );
            return false;
        }
        if request.trigger != SelectionTrigger::Manual {
            if request.trigger != SelectionTrigger::Keyboard
                && !automatic_selection_pointer_matches_bounds(&selection, POINTER_BOUNDS_TOLERANCE)
            {
                // Chromium/WebView2 providers occasionally report stale or
                // mixed-DPI rectangles even when the selected text is valid.
                // Keep the text and fall back to the physical mouse-up point
                // instead of dropping the whole selection.
                selection.bounds = None;
                selection.start_top = None;
                selection.start_bottom = None;
                selection.end_top = None;
                selection.end_bottom = None;
            }
            let fingerprint = automatic_selection_fingerprint(&selection);
            let generation = request.generation.unwrap_or_default();
            let now = Instant::now();
            if self.last_automatic_fingerprint.is_some_and(
                |(previous, previous_generation, emitted_at)| {
                    previous == fingerprint
                        && previous_generation == generation
                        && now.saturating_duration_since(emitted_at) <= AUTOMATIC_DUPLICATE_WINDOW
                },
            ) {
                return false;
            }
            self.last_automatic_fingerprint = Some((fingerprint, generation, now));
        }
        trace_selection_capture(
            "selection-event",
            &request,
            source_root_window,
            source_process_id,
        );
        let sent = self
            .event_sender
            .send(SelectionEvent::Selection(selection))
            .is_ok();
        if sent {
            self.recent_capture = Some(RecentCaptureContext {
                generation: request
                    .generation
                    .unwrap_or_else(|| hook_generation().load(Ordering::Acquire)),
                source_root_window,
                source_process_id,
                valid_until: Instant::now() + RECENT_CAPTURE_FOREGROUND_GRACE,
            });
        }
        sent
    }

    fn capture_with_timeout(
        &mut self,
        request: CaptureRequest,
    ) -> Result<Option<SelectionPayload>, SelectionError> {
        if self.capture_executor.is_none() {
            self.capture_executor = Some(CaptureExecutor::spawn(self.own_process_id)?);
        }
        let Some(executor) = self.capture_executor.as_mut() else {
            return Ok(None);
        };
        match executor.capture(request) {
            Ok(result) => {
                if result.is_err() {
                    // Initialization failures should not permanently poison a
                    // reusable lane. A later selection gets a fresh instance.
                    if let Some(executor) = self.capture_executor.take() {
                        executor.shutdown(Duration::ZERO);
                    }
                }
                result
            }
            Err(CaptureExecutorError::Disconnected) => Err(SelectionError::Internal),
            Err(CaptureExecutorError::TimedOut | CaptureExecutorError::Cancelled) => Ok(None),
            Err(CaptureExecutorError::UnsafeToTerminate { .. }) => Ok(None),
        }
    }

    fn emit_dismiss(&self, reason: &str, point: RawPoint, target_window: HWND, timestamp_ms: u64) {
        let _ = self
            .event_sender
            .send(SelectionEvent::Dismiss(DismissEvent {
                reason: reason.to_owned(),
                mouse: raw_selection_point(point),
                target_pid: i64::from(window_process_id(target_window)),
                timestamp_ms,
            }));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureExecutorError {
    Disconnected,
    TimedOut,
    Cancelled,
    UnsafeToTerminate { timed_out: bool, process_id: u32 },
}

struct HelperCaptureReply {
    result: Result<Option<SelectionPayload>, SelectionError>,
    clipboard: HelperClipboardState,
}

struct PendingUnsafeHelper {
    helper: SelectionHelperProcess,
    process_id: u32,
    expires_at: Instant,
}

struct CaptureExecutor {
    own_process_id: u32,
    helper: Option<SelectionHelperProcess>,
    clipboard_custodian: Option<SelectionHelperProcess>,
    pending_unsafe_helper: Option<PendingUnsafeHelper>,
    warming_helper: Option<Receiver<Result<SelectionHelperProcess, ()>>>,
    next_request_id: u64,
    blocked_clipboard_processes: HashMap<u32, Instant>,
}

impl CaptureExecutor {
    fn spawn(own_process_id: u32) -> Result<Self, SelectionError> {
        Ok(Self {
            own_process_id,
            helper: Some(SelectionHelperProcess::spawn()?),
            clipboard_custodian: None,
            pending_unsafe_helper: None,
            warming_helper: None,
            next_request_id: 1,
            blocked_clipboard_processes: HashMap::new(),
        })
    }

    fn capture(
        &mut self,
        request: CaptureRequest,
    ) -> Result<Result<Option<SelectionPayload>, SelectionError>, CaptureExecutorError> {
        self.refresh_pending_helper();
        self.adopt_warming_helper(false);
        if self.helper.is_none() {
            self.begin_warming_helper();
            self.adopt_warming_helper(true);
        }
        if self.helper.is_none() {
            return Err(CaptureExecutorError::Disconnected);
        }
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        let now = Instant::now();
        self.blocked_clipboard_processes
            .retain(|_, until| *until > now);
        let blocked = self
            .blocked_clipboard_processes
            .keys()
            .copied()
            .chain(self.pending_unsafe_helper.is_some().then_some(0))
            .collect::<Vec<_>>();
        let result = self
            .helper
            .as_mut()
            .expect("helper presence checked above")
            .capture(request_id, request, self.own_process_id, blocked);
        match result {
            Ok(reply) => {
                if reply.clipboard.restored && !reply.clipboard.safe_to_terminate {
                    if let Some(mut helper) = self.helper.take() {
                        let custodian_ready = helper
                            .send(&HelperCommand::BecomeCustodian {
                                clipboard_sequence: reply.clipboard.restored_sequence,
                            })
                            .is_ok();
                        if custodian_should_replace_existing(custodian_ready) {
                            if let Some(mut old) = self.clipboard_custodian.replace(helper) {
                                old.shutdown();
                            }
                        } else {
                            self.install_pending_unsafe_helper(helper, 0);
                        }
                    }
                    self.begin_warming_helper();
                }
                Ok(reply.result)
            }
            Err(CaptureExecutorError::UnsafeToTerminate {
                timed_out,
                process_id,
            }) => {
                if process_id != 0 {
                    self.blocked_clipboard_processes
                        .insert(process_id, Instant::now() + CLIPBOARD_QUARANTINE_DURATION);
                }
                if let Some(helper) = self.helper.take() {
                    self.install_pending_unsafe_helper(helper, process_id);
                }
                self.begin_warming_helper();
                Err(if timed_out {
                    CaptureExecutorError::TimedOut
                } else {
                    CaptureExecutorError::Cancelled
                })
            }
            Err(CaptureExecutorError::Disconnected) => {
                if let Some(mut helper) = self.helper.take() {
                    helper.terminate();
                }
                self.begin_warming_helper();
                Err(CaptureExecutorError::Disconnected)
            }
            Err(error) => {
                if let Some(mut helper) = self.helper.take() {
                    helper.terminate();
                }
                self.begin_warming_helper();
                Err(error)
            }
        }
    }

    fn maintenance_due(&self) -> Option<Instant> {
        self.pending_unsafe_helper
            .as_ref()
            .map(|pending| pending.expires_at)
    }

    fn install_pending_unsafe_helper(
        &mut self,
        mut helper: SelectionHelperProcess,
        process_id: u32,
    ) {
        if self.pending_unsafe_helper.is_none() {
            self.pending_unsafe_helper = Some(PendingUnsafeHelper {
                helper,
                process_id,
                expires_at: Instant::now() + PENDING_UNSAFE_HELPER_GRACE,
            });
        } else {
            // Clipboard fallback is disabled while another unsafe helper is
            // pending, so a second helper cannot own a newer transaction.
            helper.shutdown();
        }
    }

    fn maintain_pending_helper(&mut self) {
        self.refresh_pending_helper();
    }

    fn refresh_pending_helper(&mut self) {
        let observation = match self.pending_unsafe_helper.as_mut() {
            Some(pending) => loop {
                match pending.helper.try_recv_event() {
                    Some(Ok(HelperEvent::Result { clipboard, .. })) if clipboard.restored => {
                        break PendingUnsafeHelperObservation::Restored {
                            clipboard_sequence: clipboard.restored_sequence,
                        };
                    }
                    Some(Ok(HelperEvent::Result { .. })) => {
                        break PendingUnsafeHelperObservation::Completed;
                    }
                    // Phase events may have been queued immediately before a
                    // final Result. Drain them before applying the deadline so
                    // a restored transaction is not retired just because its
                    // Result was second in the channel.
                    Some(Ok(_)) => continue,
                    Some(Err(())) => {
                        break PendingUnsafeHelperObservation::Disconnected {
                            process_exited: pending.helper.has_exited(),
                        };
                    }
                    None => break PendingUnsafeHelperObservation::None,
                }
            },
            None => return,
        };
        let expired = self
            .pending_unsafe_helper
            .as_ref()
            .is_some_and(|pending| Instant::now() >= pending.expires_at);
        match pending_unsafe_helper_action(observation, expired) {
            PendingUnsafeHelperAction::Keep => {}
            PendingUnsafeHelperAction::BecomeCustodian { clipboard_sequence } => {
                let Some(mut pending) = self.pending_unsafe_helper.take() else {
                    return;
                };
                let custodian_ready = pending
                    .helper
                    .send(&HelperCommand::BecomeCustodian { clipboard_sequence })
                    .is_ok();
                if custodian_should_replace_existing(custodian_ready) {
                    if let Some(mut old) = self.clipboard_custodian.replace(pending.helper) {
                        old.shutdown();
                    }
                } else {
                    // Keep the existing known-good owner. The failed helper
                    // cannot be trusted to service delayed clipboard formats.
                    pending.helper.shutdown();
                }
            }
            PendingUnsafeHelperAction::Shutdown | PendingUnsafeHelperAction::DropExited => {
                let Some(mut pending) = self.pending_unsafe_helper.take() else {
                    return;
                };
                if pending.process_id != 0 {
                    self.blocked_clipboard_processes.insert(
                        pending.process_id,
                        Instant::now() + CLIPBOARD_QUARANTINE_DURATION,
                    );
                }
                pending.helper.shutdown();
            }
        }
    }

    fn begin_warming_helper(&mut self) {
        if self.helper.is_some() || self.warming_helper.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::sync_channel(1);
        match thread::Builder::new()
            .name("textlens-selection-helper-warmup".to_owned())
            .spawn(move || {
                for attempt in 0..3 {
                    match SelectionHelperProcess::spawn() {
                        Ok(helper) => {
                            let _ = sender.send(Ok(helper));
                            return;
                        }
                        Err(_) if attempt < 2 => thread::sleep(Duration::from_millis(200)),
                        Err(_) => break,
                    }
                }
                let _ = sender.send(Err(()));
            }) {
            Ok(_) => self.warming_helper = Some(receiver),
            Err(_) => self.warming_helper = None,
        }
    }

    fn adopt_warming_helper(&mut self, wait: bool) {
        let result = match self.warming_helper.as_ref() {
            Some(receiver) if wait => match receiver.recv_timeout(HELPER_START_TIMEOUT) {
                Ok(result) => Some(result),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => Some(Err(())),
            },
            Some(receiver) => match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(Err(())),
            },
            None => None,
        };
        let Some(result) = result else {
            return;
        };
        self.warming_helper.take();
        if let Ok(helper) = result {
            self.helper = Some(helper);
        }
    }

    fn shutdown(mut self, _grace: Duration) {
        if let Some(mut helper) = self.helper.take() {
            helper.shutdown();
        }
        if let Some(mut pending) = self.pending_unsafe_helper.take() {
            pending.helper.final_shutdown();
        }
        if let Some(mut helper) = self.clipboard_custodian.take() {
            helper.final_shutdown();
        }
    }
}

struct SelectionHelperProcess {
    child: Option<Child>,
    stdin: Option<BufWriter<ChildStdin>>,
    events: Receiver<Result<HelperEvent, ()>>,
    reader: Option<JoinHandle<()>>,
}

impl SelectionHelperProcess {
    fn spawn() -> Result<Self, SelectionError> {
        let executable = std::env::current_exe().map_err(|_| SelectionError::Internal)?;
        let mut child = Command::new(executable)
            .arg(SELECTION_HELPER_FLAG)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| SelectionError::Internal)?;
        let Some(stdin) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SelectionError::Internal);
        };
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(SelectionError::Internal);
        };
        let (event_sender, events) = mpsc::channel();
        let reader = match thread::Builder::new()
            .name("textlens-selection-helper-reader".to_owned())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    match read_helper_frame::<HelperEvent>(&mut reader) {
                        Ok(Some(event)) => {
                            if event_sender.send(Ok(event)).is_err() {
                                break;
                            }
                        }
                        Ok(None) => {
                            let _ = event_sender.send(Err(()));
                            break;
                        }
                        Err(_) => {
                            let _ = event_sender.send(Err(()));
                            break;
                        }
                    }
                }
            }) {
            Ok(reader) => reader,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(SelectionError::Internal);
            }
        };
        let mut helper = Self {
            child: Some(child),
            stdin: Some(BufWriter::new(stdin)),
            events,
            reader: Some(reader),
        };
        match helper.events.recv_timeout(HELPER_START_TIMEOUT) {
            Ok(Ok(HelperEvent::Ready { protocol_version }))
                if protocol_version == SELECTION_HELPER_PROTOCOL_VERSION =>
            {
                Ok(helper)
            }
            _ => {
                helper.terminate();
                Err(SelectionError::NativeInitializationFailed)
            }
        }
    }

    fn send(&mut self, command: &HelperCommand) -> Result<(), CaptureExecutorError> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or(CaptureExecutorError::Disconnected)?;
        write_helper_frame(stdin, command)
            .and_then(|()| stdin.flush())
            .map_err(|_| CaptureExecutorError::Disconnected)
    }

    fn try_recv_event(&self) -> Option<Result<HelperEvent, ()>> {
        match self.events.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(())),
        }
    }

    fn has_exited(&mut self) -> bool {
        self.child
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten())
            .is_some()
    }

    fn capture(
        &mut self,
        request_id: u64,
        request: CaptureRequest,
        own_process_id: u32,
        blocked_clipboard_process_ids: Vec<u32>,
    ) -> Result<HelperCaptureReply, CaptureExecutorError> {
        let expected_generation = request
            .generation
            .unwrap_or_else(|| hook_generation().load(Ordering::Acquire));
        if hook_generation().load(Ordering::Acquire) != expected_generation {
            return Ok(HelperCaptureReply {
                result: Ok(None),
                clipboard: HelperClipboardState::default(),
            });
        }
        let source_window = unsafe { GetForegroundWindow() };
        let source_process_id = window_process_id(source_window);
        if source_window.0 == ptr::null_mut()
            || source_process_id == 0
            || source_process_id == own_process_id
        {
            return Ok(HelperCaptureReply {
                result: Ok(None),
                clipboard: HelperClipboardState::default(),
            });
        }
        let mut helper_request = request;
        helper_request.generation = None;
        self.send(&HelperCommand::Capture {
            request_id,
            request: helper_request,
            source_window: source_window.0 as isize,
            source_process_id,
            blocked_clipboard_process_ids,
        })?;

        let task_deadline = Instant::now() + CAPTURE_TASK_TIMEOUT;
        let mut clipboard_injected = false;
        let mut clipboard_process_id = 0u32;
        let mut cancel_reason = None;
        let mut cancel_deadline = None;
        loop {
            if cancel_reason.is_none()
                && hook_generation().load(Ordering::Acquire) != expected_generation
            {
                let reason = HelperCancelReason::InputChanged;
                self.send(&HelperCommand::Cancel { request_id, reason })?;
                cancel_reason = Some(reason);
                cancel_deadline = Some(
                    Instant::now()
                        + if clipboard_injected {
                            HELPER_CLIPBOARD_CANCEL_GRACE
                        } else {
                            HELPER_CANCEL_GRACE
                        },
                );
            } else if cancel_reason.is_none() && Instant::now() >= task_deadline {
                let reason = HelperCancelReason::Timeout;
                self.send(&HelperCommand::Cancel { request_id, reason })?;
                cancel_reason = Some(reason);
                cancel_deadline = Some(
                    Instant::now()
                        + if clipboard_injected {
                            HELPER_CLIPBOARD_CANCEL_GRACE
                        } else {
                            HELPER_CANCEL_GRACE
                        },
                );
            }

            if cancel_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                if clipboard_injected {
                    return Err(CaptureExecutorError::UnsafeToTerminate {
                        timed_out: cancel_reason != Some(HelperCancelReason::InputChanged),
                        process_id: clipboard_process_id,
                    });
                }
                return Err(match cancel_reason {
                    Some(HelperCancelReason::InputChanged) => CaptureExecutorError::Cancelled,
                    _ => CaptureExecutorError::TimedOut,
                });
            }

            match self.events.recv_timeout(HELPER_POLL_INTERVAL) {
                Ok(Ok(HelperEvent::Phase {
                    request_id: event_request_id,
                    phase:
                        HelperPhase::ClipboardPrepared { process_id }
                        | HelperPhase::ClipboardInjected { process_id },
                })) if event_request_id == request_id => {
                    clipboard_injected = true;
                    clipboard_process_id = process_id;
                    if cancel_reason.is_some() {
                        cancel_deadline = Some(Instant::now() + HELPER_CLIPBOARD_CANCEL_GRACE);
                    }
                }
                Ok(Ok(HelperEvent::Result {
                    request_id: event_request_id,
                    result,
                    clipboard,
                })) if event_request_id == request_id => {
                    if cancel_reason.is_some() {
                        if clipboard.restored {
                            return Ok(HelperCaptureReply {
                                result: Ok(None),
                                clipboard,
                            });
                        }
                        return Err(CaptureExecutorError::Cancelled);
                    }
                    let result = match result {
                        HelperCaptureResult::Selection {
                            mut selection,
                            source_window,
                            process_id,
                        } if parent_capture_context_is_valid(
                            source_window,
                            process_id,
                            own_process_id,
                            expected_generation,
                        ) =>
                        {
                            selection.timestamp_ms = strict_parent_timestamp_ms();
                            Ok(Some(selection))
                        }
                        HelperCaptureResult::Selection { .. } | HelperCaptureResult::Empty => {
                            Ok(None)
                        }
                        HelperCaptureResult::Error => {
                            Err(SelectionError::NativeInitializationFailed)
                        }
                    };
                    return Ok(HelperCaptureReply { result, clipboard });
                }
                Ok(Ok(_)) | Err(RecvTimeoutError::Timeout) => {}
                Ok(Err(())) | Err(RecvTimeoutError::Disconnected) => {
                    return Err(if clipboard_injected {
                        CaptureExecutorError::UnsafeToTerminate {
                            timed_out: false,
                            process_id: clipboard_process_id,
                        }
                    } else {
                        CaptureExecutorError::Disconnected
                    })
                }
            }
        }
    }

    fn shutdown(&mut self) {
        let _ = self.send(&HelperCommand::Shutdown);
        let deadline = Instant::now() + HELPER_SHUTDOWN_GRACE;
        while Instant::now() < deadline {
            if self
                .child
                .as_mut()
                .and_then(|child| child.try_wait().ok().flatten())
                .is_some()
            {
                self.child.take();
                self.stdin.take();
                self.finish_reader();
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        self.terminate();
    }

    fn final_shutdown(&mut self) {
        let _ = self.send(&HelperCommand::FinalShutdown);
        let deadline = Instant::now() + HELPER_CLIPBOARD_CANCEL_GRACE;
        while Instant::now() < deadline {
            if self
                .child
                .as_mut()
                .and_then(|child| child.try_wait().ok().flatten())
                .is_some()
            {
                self.child.take();
                self.stdin.take();
                self.finish_reader();
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        self.terminate();
    }

    fn terminate(&mut self) {
        self.stdin.take();
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.finish_reader();
    }

    fn finish_reader(&mut self) {
        if let Some(reader) = self.reader.take() {
            reap_worker(reader, WORKER_SHUTDOWN_GRACE);
        }
    }
}

impl Drop for SelectionHelperProcess {
    fn drop(&mut self) {
        if self.child.is_some() || self.reader.is_some() {
            self.shutdown();
        }
    }
}

fn parent_capture_context_is_valid(
    source_window: isize,
    process_id: u32,
    own_process_id: u32,
    expected_generation: u64,
) -> bool {
    let foreground = unsafe { GetForegroundWindow() };
    foreground.0 as isize == source_window
        && process_id != 0
        && process_id != own_process_id
        && window_process_id(foreground) == process_id
        && hook_generation().load(Ordering::Acquire) == expected_generation
}

fn write_helper_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> io::Result<()> {
    let payload = serde_json::to_vec(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if payload.len() > HELPER_FRAME_LIMIT || payload.len() > u32::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "selection helper frame exceeds its limit",
        ));
    }
    writer.write_all(&(payload.len() as u32).to_le_bytes())?;
    writer.write_all(&payload)
}

fn read_helper_frame<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<Option<T>> {
    let mut length = [0u8; 4];
    match reader.read_exact(&mut length) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_le_bytes(length) as usize;
    if length == 0 || length > HELPER_FRAME_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid selection helper frame length",
        ));
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload)?;
    serde_json::from_slice(&payload)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn helper_cancel_reason_code(reason: HelperCancelReason) -> u32 {
    match reason {
        HelperCancelReason::InputChanged => 1,
        HelperCancelReason::Timeout => 2,
        HelperCancelReason::Shutdown => 3,
    }
}

fn helper_cancel_reason_from_code(code: u32) -> Option<HelperCancelReason> {
    match code {
        1 => Some(HelperCancelReason::InputChanged),
        2 => Some(HelperCancelReason::Timeout),
        3 => Some(HelperCancelReason::Shutdown),
        _ => None,
    }
}

fn helper_should_flush_on_final_shutdown(
    custodian_sequence: Option<u32>,
    current_sequence: u32,
) -> bool {
    custodian_sequence.is_some_and(|sequence| sequence == current_sequence)
}

struct CaptureControl {
    request_id: u64,
    cancelled_request_id: Arc<AtomicU64>,
    cancel_reason: Arc<AtomicU32>,
    event_writer: Arc<Mutex<BufWriter<std::io::Stdout>>>,
    blocked_clipboard_process_ids: HashSet<u32>,
    source_window: HWND,
    source_process_id: u32,
    clipboard_used: AtomicBool,
    clipboard_restored: AtomicBool,
    restored_clipboard_sequence: AtomicU32,
}

impl CaptureControl {
    fn cancel_reason(&self) -> Option<HelperCancelReason> {
        let cancelled_request_id = self.cancelled_request_id.load(Ordering::Acquire);
        (cancelled_request_id == self.request_id || cancelled_request_id == u64::MAX)
            .then(|| helper_cancel_reason_from_code(self.cancel_reason.load(Ordering::Acquire)))
            .flatten()
    }

    fn is_cancelled(&self) -> bool {
        self.cancel_reason().is_some()
    }

    fn clipboard_allowed(&self, process_id: u32) -> bool {
        !self.blocked_clipboard_process_ids.contains(&0)
            && !self.blocked_clipboard_process_ids.contains(&process_id)
    }

    fn report_phase(&self, phase: HelperPhase) {
        if let Ok(mut writer) = self.event_writer.lock() {
            let _ = write_helper_frame(
                &mut *writer,
                &HelperEvent::Phase {
                    request_id: self.request_id,
                    phase,
                },
            );
            let _ = writer.flush();
        }
    }

    fn note_clipboard_used(&self) {
        self.clipboard_used.store(true, Ordering::Release);
    }

    fn note_clipboard_restored(&self, sequence: u32) {
        self.restored_clipboard_sequence
            .store(sequence, Ordering::Relaxed);
        self.clipboard_restored.store(true, Ordering::Release);
    }

    fn clipboard_state(&self) -> HelperClipboardState {
        let restored = self.clipboard_restored.load(Ordering::Acquire);
        HelperClipboardState {
            used: self.clipboard_used.load(Ordering::Acquire),
            restored,
            restored_sequence: self.restored_clipboard_sequence.load(Ordering::Acquire),
            safe_to_terminate: !restored,
        }
    }
}

enum HelperWork {
    Capture {
        request_id: u64,
        request: CaptureRequest,
        source_window: isize,
        source_process_id: u32,
        blocked_clipboard_process_ids: Vec<u32>,
    },
    BecomeCustodian(u32),
    Shutdown,
    FinalShutdown,
}

pub(super) fn run_selection_helper_if_requested() -> bool {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(SELECTION_HELPER_FLAG)) {
        return false;
    }
    if let Err(error) = selection_helper_main() {
        eprintln!("selection helper failed: {error}");
    }
    true
}

fn selection_helper_main() -> io::Result<()> {
    let own_process_id = unsafe { GetCurrentProcessId() };
    let engine = CaptureEngine::initialize(own_process_id)
        .map_err(|error| io::Error::new(io::ErrorKind::Other, error.to_string()))?;
    let writer = Arc::new(Mutex::new(BufWriter::new(std::io::stdout())));
    {
        let mut output = writer
            .lock()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "helper output lock poisoned"))?;
        write_helper_frame(
            &mut *output,
            &HelperEvent::Ready {
                protocol_version: SELECTION_HELPER_PROTOCOL_VERSION,
            },
        )?;
        output.flush()?;
    }

    let cancelled_request_id = Arc::new(AtomicU64::new(0));
    let cancel_reason = Arc::new(AtomicU32::new(0));
    let reader_cancelled_id = cancelled_request_id.clone();
    let reader_cancel_reason = cancel_reason.clone();
    let (command_sender, command_receiver) = mpsc::channel();
    let reader = thread::Builder::new()
        .name("textlens-selection-helper-input".to_owned())
        .spawn(move || {
            let mut input = BufReader::new(std::io::stdin());
            loop {
                match read_helper_frame::<HelperCommand>(&mut input) {
                    Ok(Some(HelperCommand::Capture {
                        request_id,
                        request,
                        source_window,
                        source_process_id,
                        blocked_clipboard_process_ids,
                    })) => {
                        if command_sender
                            .send(HelperWork::Capture {
                                request_id,
                                request,
                                source_window,
                                source_process_id,
                                blocked_clipboard_process_ids,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(Some(HelperCommand::Cancel { request_id, reason })) => {
                        reader_cancel_reason
                            .store(helper_cancel_reason_code(reason), Ordering::Relaxed);
                        reader_cancelled_id.store(request_id, Ordering::Release);
                    }
                    Ok(Some(HelperCommand::BecomeCustodian { clipboard_sequence })) => {
                        if command_sender
                            .send(HelperWork::BecomeCustodian(clipboard_sequence))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(Some(HelperCommand::Shutdown)) => {
                        reader_cancel_reason.store(
                            helper_cancel_reason_code(HelperCancelReason::Shutdown),
                            Ordering::Relaxed,
                        );
                        reader_cancelled_id.store(u64::MAX, Ordering::Release);
                        let _ = command_sender.send(HelperWork::Shutdown);
                        break;
                    }
                    Ok(Some(HelperCommand::FinalShutdown)) | Ok(None) | Err(_) => {
                        reader_cancel_reason.store(
                            helper_cancel_reason_code(HelperCancelReason::Shutdown),
                            Ordering::Relaxed,
                        );
                        reader_cancelled_id.store(u64::MAX, Ordering::Release);
                        let _ = command_sender.send(HelperWork::FinalShutdown);
                        break;
                    }
                }
            }
        })?;

    let mut shutdown = false;
    let mut custodian_sequence = None;
    while !shutdown {
        match command_receiver.recv_timeout(WORKER_PUMP_INTERVAL) {
            Ok(HelperWork::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                shutdown = custodian_sequence
                    .is_none_or(|sequence| clipboard::windows_clipboard_sequence() != sequence);
            }
            Ok(HelperWork::FinalShutdown) => {
                if helper_should_flush_on_final_shutdown(
                    custodian_sequence,
                    clipboard::windows_clipboard_sequence(),
                ) {
                    let _ = unsafe { OleFlushClipboard() };
                }
                shutdown = true;
            }
            Ok(HelperWork::BecomeCustodian(sequence)) => {
                custodian_sequence = Some(sequence);
            }
            Ok(HelperWork::Capture {
                request_id,
                request,
                source_window,
                source_process_id,
                blocked_clipboard_process_ids,
            }) => {
                let control = CaptureControl {
                    request_id,
                    cancelled_request_id: cancelled_request_id.clone(),
                    cancel_reason: cancel_reason.clone(),
                    event_writer: writer.clone(),
                    blocked_clipboard_process_ids: blocked_clipboard_process_ids
                        .into_iter()
                        .collect(),
                    source_window: HWND(source_window as *mut c_void),
                    source_process_id,
                    clipboard_used: AtomicBool::new(false),
                    clipboard_restored: AtomicBool::new(false),
                    restored_clipboard_sequence: AtomicU32::new(0),
                };
                let result = if control.is_cancelled() {
                    HelperCaptureResult::Empty
                } else {
                    match engine.capture(request, &control) {
                        Ok(Some(selection)) => HelperCaptureResult::Selection {
                            selection,
                            source_window: control.source_window.0 as isize,
                            process_id: control.source_process_id,
                        },
                        Ok(None) => HelperCaptureResult::Empty,
                        Err(_) => HelperCaptureResult::Error,
                    }
                };
                let mut output = writer.lock().map_err(|_| {
                    io::Error::new(io::ErrorKind::Other, "helper output lock poisoned")
                })?;
                write_helper_frame(
                    &mut *output,
                    &HelperEvent::Result {
                        request_id,
                        result,
                        clipboard: control.clipboard_state(),
                    },
                )?;
                output.flush()?;
            }
            Err(RecvTimeoutError::Timeout) => {
                if custodian_sequence
                    .is_some_and(|sequence| clipboard::windows_clipboard_sequence() != sequence)
                {
                    shutdown = true;
                }
            }
        }
        pump_sta_messages();
    }
    let _ = reader.join();
    Ok(())
}

struct CaptureEngine {
    // Rust drops fields in declaration order. Every COM interface must be
    // released before OleUninitialize runs, so the apartment guard stays last.
    automation: IUIAutomation,
    own_process_id: u32,
    deadline: Cell<Instant>,
    _apartment: ComApartment,
}

struct CaptureTarget {
    element: IUIAutomationElement,
    process_id: u32,
    source_window: HWND,
    source_app: SourceApplication,
    clipboard_password_safe: bool,
    text_surface: bool,
}

enum CaptureTargetLookup {
    Found(CaptureTarget),
    Retryable,
    Stop,
}

enum AccessibilityCapture {
    Selection(SelectionPayload),
    Protected,
    NotFound,
}

enum TextPatternSearch {
    Found(IUIAutomationTextPattern),
    Protected,
    NotFound,
}

impl CaptureEngine {
    fn initialize(own_process_id: u32) -> Result<Self, SelectionError> {
        let apartment = ComApartment::initialize()?;
        let automation = unsafe {
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
        }
        .map_err(|_| SelectionError::NativeInitializationFailed)?;
        Ok(Self {
            automation,
            own_process_id,
            deadline: Cell::new(Instant::now() + CAPTURE_ENGINE_BUDGET),
            _apartment: apartment,
        })
    }

    fn capture(
        &self,
        request: CaptureRequest,
        control: &CaptureControl,
    ) -> Result<Option<SelectionPayload>, SelectionError> {
        // A reused lane gets a fresh bounded budget for every request.
        self.deadline.set(Instant::now() + CAPTURE_ENGINE_BUDGET);
        let retry_started = Instant::now();
        let mut clipboard_target = None;
        // Chromium/Electron validation may need a system-wide process-tree
        // snapshot. Capture it lazily and at most once across UIA retries.
        let mut process_parents = None;
        let mut source_app = None;
        for (attempt, retry_delay) in ACCESSIBILITY_RETRY_DELAYS.into_iter().enumerate() {
            let remaining = retry_delay.saturating_sub(retry_started.elapsed());
            if !remaining.is_zero() {
                thread::sleep(remaining);
            }
            if control.is_cancelled() {
                return Ok(None);
            }
            let point_target = match self.acquire_capture_target(
                request,
                control,
                &mut process_parents,
                &mut source_app,
                false,
            ) {
                CaptureTargetLookup::Found(target) => Some(target),
                CaptureTargetLookup::Retryable => None,
                CaptureTargetLookup::Stop => return Ok(None),
            };
            if let Some(target) = point_target {
                match self.capture_accessibility(
                    &target.element,
                    target.source_window,
                    target.source_app.clone(),
                    request,
                    control,
                )? {
                    AccessibilityCapture::Selection(selection)
                        if capture_context_still_valid(
                            control,
                            target.source_window,
                            target.process_id,
                        ) =>
                    {
                        return Ok(Some(selection));
                    }
                    AccessibilityCapture::Protected => return Ok(None),
                    _ => {}
                }
                clipboard_target = Some(target);
            }

            if !matches!(
                request.trigger,
                SelectionTrigger::Keyboard | SelectionTrigger::Manual
            ) {
                let focused_target = match self.acquire_capture_target(
                    request,
                    control,
                    &mut process_parents,
                    &mut source_app,
                    true,
                ) {
                    CaptureTargetLookup::Found(target) => Some(target),
                    CaptureTargetLookup::Retryable => None,
                    CaptureTargetLookup::Stop if clipboard_target.is_some() => None,
                    CaptureTargetLookup::Stop => return Ok(None),
                };
                if let Some(target) = focused_target {
                    match self.capture_accessibility(
                        &target.element,
                        target.source_window,
                        target.source_app.clone(),
                        request,
                        control,
                    )? {
                        AccessibilityCapture::Selection(selection)
                            if capture_context_still_valid(
                                control,
                                target.source_window,
                                target.process_id,
                            ) =>
                        {
                            return Ok(Some(selection));
                        }
                        AccessibilityCapture::Protected if clipboard_target.is_some() => {}
                        AccessibilityCapture::Protected => return Ok(None),
                        _ => {}
                    }
                    if clipboard_target.is_none()
                        || (!clipboard_target
                            .as_ref()
                            .is_some_and(|candidate| candidate.clipboard_password_safe)
                            && target.clipboard_password_safe)
                    {
                        clipboard_target = Some(target);
                    }
                }
            }
            // After one short UIA retry, leave the remaining retry budget when
            // clipboard is already a safe option. This covers both:
            // - Found target + empty TextPattern (classic WPS/WeChat path)
            // - Persistent Retryable (multi-process CEF host under WPS) where
            //   we fall through to foreground clipboard for allowlisted apps
            if should_break_uia_for_clipboard(
                attempt,
                clipboard_target.as_ref().map(|target| {
                    (
                        target.clipboard_password_safe,
                        compatible_clipboard_application(&target.source_app.bundle_id),
                    )
                }),
                clipboard_target.is_none().then(|| {
                    resolve_cached_source_app(
                        &mut source_app,
                        control.source_process_id,
                        control.source_window,
                    )
                    .map(|app| compatible_clipboard_application(&app.bundle_id))
                    .unwrap_or(false)
                }),
            ) {
                break;
            }
        }

        // Prefer a UIA-validated target when we have one. When UIA never
        // resolved (WPS multi-process CEF panes, transient providers), still
        // attempt Ctrl+C against the unchanged foreground window for known
        // custom-rendered applications.
        if let Some(target) = clipboard_target {
            if Instant::now() >= self.deadline.get()
                || !capture_context_still_valid(control, target.source_window, target.process_id)
                || !control.clipboard_allowed(target.process_id)
                || !clipboard_fallback_allowed(
                    &target.source_app.bundle_id,
                    true,
                    target.clipboard_password_safe,
                    target.text_surface,
                )
                || !process_allows_input_injection(target.process_id)
            {
                return Ok(None);
            }
            return self.capture_clipboard(
                request,
                target.process_id,
                target.source_window,
                target.source_app,
                &mut process_parents,
                control,
            );
        }

        self.capture_foreground_clipboard(request, control, &mut process_parents, &mut source_app)
    }

    /// Clipboard-only path when UIA never produced a related element.
    ///
    /// Zero extra cost on the successful UIA path (only reached after retries
    /// leave `clipboard_target` empty). Restricted to allowlisted apps so
    /// unknown hosts still fail closed without an accessibility target.
    fn capture_foreground_clipboard(
        &self,
        request: CaptureRequest,
        control: &CaptureControl,
        process_parents: &mut Option<Option<HashMap<u32, u32>>>,
        source_app: &mut Option<(u32, SourceApplication)>,
    ) -> Result<Option<SelectionPayload>, SelectionError> {
        if Instant::now() >= self.deadline.get() || control.is_cancelled() {
            return Ok(None);
        }
        let foreground = unsafe { GetForegroundWindow() };
        let process_id = window_process_id(foreground);
        if foreground != control.source_window
            || process_id != control.source_process_id
            || foreground.0 == ptr::null_mut()
            || process_id == 0
            || process_id == self.own_process_id
        {
            return Ok(None);
        }
        let Some(application) =
            resolve_cached_source_app(source_app, process_id, foreground)
        else {
            return Ok(None);
        };
        // Allowlisted custom-rendered apps only. Unknown apps still require a
        // UIA text surface so we never Ctrl+C into arbitrary foreground hosts.
        if !compatible_clipboard_application(&application.bundle_id)
            || prohibited_clipboard_application(&application.bundle_id)
            || !control.clipboard_allowed(process_id)
            || !process_allows_input_injection(process_id)
            || !capture_context_still_valid(control, foreground, process_id)
        {
            return Ok(None);
        }
        // Best-effort password probe on the focused element when UIA can
        // surface it; if focus is unavailable (common for WPS CEF panes), the
        // allowlist gate above is the remaining safety boundary.
        if focused_element_is_password(&self.automation) {
            return Ok(None);
        }
        self.capture_clipboard(
            request,
            process_id,
            foreground,
            application,
            process_parents,
            control,
        )
    }

    fn acquire_capture_target(
        &self,
        request: CaptureRequest,
        control: &CaptureControl,
        process_parents: &mut Option<Option<HashMap<u32, u32>>>,
        source_app: &mut Option<(u32, SourceApplication)>,
        use_focused: bool,
    ) -> CaptureTargetLookup {
        if Instant::now() >= self.deadline.get() || control.is_cancelled() {
            return CaptureTargetLookup::Stop;
        }
        let foreground = unsafe { GetForegroundWindow() };
        let process_id = window_process_id(foreground);
        if foreground != control.source_window
            || process_id != control.source_process_id
            || foreground.0 == ptr::null_mut()
            || process_id == 0
            || process_id == self.own_process_id
        {
            return CaptureTargetLookup::Stop;
        }
        let element = match (use_focused, request.trigger) {
            (true, _) | (_, SelectionTrigger::Keyboard | SelectionTrigger::Manual) => {
                match unsafe { self.automation.GetFocusedElement() } {
                    Ok(element) => element,
                    Err(_) => return CaptureTargetLookup::Retryable,
                }
            }
            _ => match unsafe {
                self.automation.ElementFromPoint(POINT {
                    x: request.current.x,
                    y: request.current.y,
                })
            } {
                Ok(element) => element,
                Err(_) => return CaptureTargetLookup::Retryable,
            },
        };
        let element_process_id = match unsafe { element.CurrentProcessId() } {
            Ok(value) if value > 0 => value as u32,
            _ => return CaptureTargetLookup::Retryable,
        };
        if element_process_id == self.own_process_id {
            return CaptureTargetLookup::Stop;
        }
        // Chromium/Electron accessibility providers and their native child
        // HWNDs commonly run in renderer PIDs rather than the foreground
        // browser PID. Accept only descendants of that foreground process;
        // unrelated providers still fail closed.
        let element_window_process_id = unsafe { element.CurrentNativeWindowHandle() }
            .ok()
            .filter(|window| window.0 != ptr::null_mut())
            .map(window_process_id)
            .unwrap_or(0);
        if !uia_processes_belong_to_target(
            process_id,
            element_process_id,
            element_window_process_id,
            process_parents,
        ) {
            return CaptureTargetLookup::Retryable;
        }
        let password_state = unsafe { element.CurrentIsPassword() }
            .ok()
            .map(|value| value.as_bool());
        if password_state == Some(true) {
            return CaptureTargetLookup::Stop;
        }
        let source_app = match source_app {
            Some((cached_process_id, application)) if *cached_process_id == process_id => {
                application.clone()
            }
            slot => {
                let application = source_application(process_id, foreground);
                *slot = Some((process_id, application.clone()));
                application
            }
        };
        // Custom-rendered hosts (WPS, WeChat, etc.) often omit IsPassword.
        // Treat unknown as safe only for allowlisted apps; others still need
        // an explicit false before clipboard fallback.
        let compatible_app = compatible_clipboard_application(&source_app.bundle_id);
        CaptureTargetLookup::Found(CaptureTarget {
            text_surface: uia_element_is_text_surface(&element),
            element,
            process_id,
            source_window: foreground,
            source_app,
            clipboard_password_safe: clipboard_password_gate(password_state, compatible_app),
        })
    }

    fn capture_accessibility(
        &self,
        focused: &IUIAutomationElement,
        source_window: HWND,
        source_app: SourceApplication,
        request: CaptureRequest,
        control: &CaptureControl,
    ) -> Result<AccessibilityCapture, SelectionError> {
        let pattern = match self.find_text_pattern(focused, control) {
            TextPatternSearch::Found(pattern) => pattern,
            TextPatternSearch::Protected => return Ok(AccessibilityCapture::Protected),
            TextPatternSearch::NotFound => return Ok(AccessibilityCapture::NotFound),
        };
        let ranges = match unsafe { pattern.GetSelection() } {
            Ok(ranges) => ranges,
            Err(_) => return Ok(AccessibilityCapture::NotFound),
        };
        let range_count = unsafe { ranges.Length() }.unwrap_or(0);
        if range_count <= 0 || range_count > 64 {
            return Ok(AccessibilityCapture::NotFound);
        }

        let mut texts = Vec::new();
        let mut total_text_chars = 0usize;
        let mut total_bounding_values = 0usize;
        let mut rectangles = Vec::new();
        let mut bounds_valid = true;
        for index in 0..range_count {
            if Instant::now() >= self.deadline.get() || control.is_cancelled() {
                return Ok(AccessibilityCapture::NotFound);
            }
            let Ok(range) = (unsafe { ranges.GetElement(index) }) else {
                continue;
            };
            let separator_chars = usize::from(!texts.is_empty());
            let Some((remaining, request_limit)) =
                windows_text_budget(total_text_chars, separator_chars != 0)
            else {
                return Ok(AccessibilityCapture::NotFound);
            };
            if let Ok(text) = unsafe { range.GetText(request_limit) } {
                let text = text.to_string();
                if !text.trim().is_empty() {
                    let text_chars = text.chars().count();
                    if text_chars > remaining {
                        return Ok(AccessibilityCapture::NotFound);
                    }
                    total_text_chars += separator_chars + text_chars;
                    texts.push(text);
                }
            }
            if bounds_valid {
                match bounding_rectangle_values(&range) {
                    Ok(values) => {
                        let next_values = total_bounding_values.checked_add(values.len());
                        if next_values.is_some_and(|total| total <= MAX_TOTAL_BOUNDING_VALUES) {
                            total_bounding_values = next_values.unwrap_or_default();
                            rectangles.extend(rectangles_from_values(&values));
                        } else {
                            bounds_valid = false;
                            rectangles.clear();
                        }
                    }
                    Err(_) => {
                        // Text is still useful when a provider returns broken,
                        // enormous or temporarily unavailable rectangles. The
                        // toolbar falls back to the physical mouse-up point.
                        bounds_valid = false;
                        rectangles.clear();
                    }
                }
            }
        }
        if texts.is_empty() {
            return Ok(AccessibilityCapture::NotFound);
        }
        let text = texts.join("\n");

        let mouse_start = request.start.map(raw_selection_point);
        let mouse_end = request.end.map(raw_selection_point);
        let mouse_current = raw_selection_point(request.current);
        let direction = direction_from_points(mouse_start, mouse_end);
        let bounds = union_selection_bounds(&rectangles);
        let (start_top, start_bottom, end_top, end_bottom) =
            endpoint_points(&rectangles, direction);

        Ok(AccessibilityCapture::Selection(SelectionPayload {
            text,
            source_app,
            bounds,
            start_top,
            start_bottom,
            end_top,
            end_bottom,
            mouse: SelectionMouse {
                start: mouse_start,
                end: mouse_end,
                current: mouse_current,
            },
            direction,
            is_fullscreen: window_is_fullscreen(source_window),
            method: SelectionMethod::Accessibility,
            trigger: request.trigger,
            timestamp_ms: timestamp_ms(),
        }))
    }

    fn capture_clipboard(
        &self,
        request: CaptureRequest,
        process_id: u32,
        source_window: HWND,
        source_app: SourceApplication,
        process_parents: &mut Option<Option<HashMap<u32, u32>>>,
        control: &CaptureControl,
    ) -> Result<Option<SelectionPayload>, SelectionError> {
        if Instant::now() >= self.deadline.get()
            || !clipboard_capture_is_valid(source_window, process_id, control)
        {
            return Ok(None);
        }
        let Some(snapshot) = clipboard::snapshot_windows_clipboard() else {
            return Ok(None);
        };
        if !clipboard_capture_is_valid(source_window, process_id, control) {
            return Ok(None);
        }
        // Capture the process tree once for the whole transaction. Electron
        // may publish clipboard data from a renderer child; repeatedly taking
        // a system-wide process snapshot inside the 10 ms polling loop would
        // add avoidable latency.
        let process_parents = process_parents
            .get_or_insert_with(process_parent_map)
            .as_ref();
        // Re-sample immediately before SendInput. In particular, never trust
        // a Ctrl state observed before the OLE/process snapshots: releasing
        // Ctrl in that interval must not turn the injected shortcut into a
        // plain `c` keystroke in the source application.
        let Some(modifiers) = wait_for_copy_modifiers() else {
            return Ok(None);
        };
        if !clipboard_capture_is_valid(source_window, process_id, control) {
            return Ok(None);
        }
        control.report_phase(HelperPhase::ClipboardPrepared { process_id });
        if !post_copy_shortcut(modifiers.control) {
            return Ok(None);
        }
        control.note_clipboard_used();
        control.report_phase(HelperPhase::ClipboardInjected { process_id });
        let Some(copied_sequence) = wait_for_clipboard_change(
            snapshot.sequence(),
            source_window,
            process_id,
            self.deadline.get(),
            process_parents,
            control,
        ) else {
            return Ok(None);
        };

        let cancel_reason = control.cancel_reason();
        let context_valid = clipboard_source_context_is_valid(source_window, process_id)
            && clipboard_owner_matches_target(process_id, process_parents)
            && (cancel_reason.is_some() || Instant::now() < self.deadline.get());
        let text = cancel_reason
            .is_none()
            .then(|| clipboard::read_windows_clipboard_text(copied_sequence))
            .flatten();
        let transaction_unchanged = clipboard_restore_allowed(
            copied_sequence,
            clipboard::windows_clipboard_sequence(),
            context_valid,
        );
        if transaction_unchanged {
            if snapshot.restore_if_unchanged(copied_sequence) {
                control.note_clipboard_restored(clipboard::windows_clipboard_sequence());
            }
        }
        if cancel_reason.is_some() {
            return Ok(None);
        }
        let Some(text) = text.filter(|text| !text.trim().is_empty()) else {
            return Ok(None);
        };

        if transaction_unchanged && spreadsheet_copy_mode_application(&source_app.bundle_id) {
            let _ = post_synthetic_key(VK_ESCAPE);
        }

        Ok(Some(SelectionPayload {
            text,
            source_app,
            bounds: None,
            start_top: None,
            start_bottom: None,
            end_top: None,
            end_bottom: None,
            mouse: SelectionMouse {
                start: request.start.map(raw_selection_point),
                end: request.end.map(raw_selection_point),
                current: raw_selection_point(request.current),
            },
            direction: direction_from_points(
                request.start.map(raw_selection_point),
                request.end.map(raw_selection_point),
            ),
            is_fullscreen: window_is_fullscreen(source_window),
            method: SelectionMethod::Clipboard,
            trigger: request.trigger,
            timestamp_ms: timestamp_ms(),
        }))
    }

    fn find_text_pattern(
        &self,
        focused: &IUIAutomationElement,
        control: &CaptureControl,
    ) -> TextPatternSearch {
        let walkers = [
            unsafe { self.automation.ControlViewWalker() }.ok(),
            unsafe { self.automation.RawViewWalker() }.ok(),
        ];
        let mut visited_runtime_ids = HashSet::new();
        let mut visited_without_runtime_id = Vec::<IUIAutomationElement>::new();
        for walker in walkers.into_iter().flatten() {
            let mut element = focused.clone();
            for _ in 0..MAX_UIA_ANCESTORS {
                if Instant::now() >= self.deadline.get() || control.is_cancelled() {
                    return TextPatternSearch::NotFound;
                }
                // Runtime IDs turn the Control/Raw View de-duplication into a
                // constant-time local lookup. CompareElements remains only as
                // a fail-safe for providers which do not expose a runtime ID;
                // the old all-pairs COM comparison became very slow on deep
                // Chromium accessibility trees.
                let runtime_id = element_runtime_id(&element);
                let duplicate = runtime_id.as_ref().map_or_else(
                    || {
                        visited_without_runtime_id.iter().any(|previous| {
                            unsafe { self.automation.CompareElements(previous, &element) }
                                .ok()
                                .is_some_and(|same| same.as_bool())
                        })
                    },
                    |runtime_id| !visited_runtime_ids.insert(runtime_id.clone()),
                );
                if !duplicate {
                    // An explicit password node blocks every capture path.
                    // Some Chromium/WebView2 structural ancestors do not
                    // expose this optional property; that must not prevent a
                    // safe UIA TextPattern read from a known non-password leaf.
                    match unsafe { element.CurrentIsPassword() }
                        .ok()
                        .map(|value| value.as_bool())
                    {
                        Some(true) => return TextPatternSearch::Protected,
                        Some(false) | None => {}
                    }
                    if let Ok(pattern) = unsafe {
                        element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                    } {
                        return TextPatternSearch::Found(pattern);
                    }
                    if runtime_id.is_none() {
                        visited_without_runtime_id.push(element.clone());
                    }
                }
                element = match unsafe { walker.GetParentElement(&element) } {
                    Ok(parent) => parent,
                    Err(_) => break,
                };
            }
        }
        TextPatternSearch::NotFound
    }
}

fn capture_context_still_valid(
    control: &CaptureControl,
    source_window: HWND,
    process_id: u32,
) -> bool {
    let foreground = unsafe { GetForegroundWindow() };
    foreground == source_window
        && window_process_id(foreground) == process_id
        && !control.is_cancelled()
}

fn foreground_interaction_decision(
    window_valid: bool,
    foreground_process_id: u32,
    own_process_id: u32,
    has_mouse_down: bool,
    mouse_down_on_self: bool,
    has_pending_capture: bool,
) -> ForegroundInteractionDecision {
    if !window_valid || foreground_process_id == 0 || foreground_process_id == own_process_id {
        ForegroundInteractionDecision::Ignore
    } else if has_mouse_down && !mouse_down_on_self {
        // Foreground activation is often delivered between the low-level Down
        // and Up callbacks. Preserve the gesture even when a shell host or
        // Chromium/Electron replaces the root HWND; the helper validates the
        // final foreground process and pointer target before returning text.
        ForegroundInteractionDecision::PreserveMouseGesture
    } else if has_pending_capture {
        ForegroundInteractionDecision::PreservePendingCapture
    } else {
        ForegroundInteractionDecision::Dismiss
    }
}

fn correlate_pending_capture_with_foreground(
    pending: &mut Option<PendingCapture>,
    foreground_root: isize,
    foreground_process_id: u32,
    generation: u64,
) {
    let Some(pending) = pending.as_mut() else {
        return;
    };
    if foreground_root == 0 || foreground_process_id == 0 {
        return;
    }
    // The foreground window observed after mouse-up is a stronger source
    // candidate than WindowFromPoint sampled inside the low-level hook. Keep
    // it for correlation/diagnostics only; the helper remains authoritative.
    let source_root_window = pending.source_root_window;
    let source_process_id = pending.source_process_id;
    if !capture_windows_are_related_with_parents(
        source_root_window,
        source_process_id,
        foreground_root,
        foreground_process_id,
        &mut pending.process_parents,
    ) {
        pending.due = Instant::now() + FOREGROUND_SETTLE_RETRY;
        return;
    }
    pending.source_root_window = foreground_root;
    pending.source_process_id = foreground_process_id;
    if pending
        .request
        .generation
        .is_none_or(|current| generation >= current)
    {
        pending.request.generation = Some(generation);
    }
    pending.due = Instant::now();
}

fn capture_windows_are_related(
    source_root_window: isize,
    source_process_id: u32,
    foreground_root_window: isize,
    foreground_process_id: u32,
) -> bool {
    let mut process_parents = None;
    capture_windows_are_related_with_parents(
        source_root_window,
        source_process_id,
        foreground_root_window,
        foreground_process_id,
        &mut process_parents,
    )
}

fn capture_windows_are_related_with_parents(
    source_root_window: isize,
    source_process_id: u32,
    foreground_root_window: isize,
    foreground_process_id: u32,
    process_parents: &mut Option<Option<HashMap<u32, u32>>>,
) -> bool {
    if source_root_window == 0 || foreground_root_window == 0 {
        return source_process_id != 0
            && foreground_process_id != 0
            && source_process_id == foreground_process_id;
    }
    if source_root_window == foreground_root_window
        || (source_process_id != 0 && source_process_id == foreground_process_id)
    {
        return true;
    }
    let Some(parents) = process_parents
        .get_or_insert_with(process_parent_map)
        .as_ref()
    else {
        return false;
    };
    process_descends_from(source_process_id, foreground_process_id, &parents)
        || process_descends_from(foreground_process_id, source_process_id, &parents)
}

fn pending_foreground_decision(
    foreground_valid: bool,
    foreground_process_id: u32,
    own_process_id: u32,
    source_related: bool,
    before_expiry: bool,
) -> PendingForegroundDecision {
    if foreground_valid
        && foreground_process_id != 0
        && foreground_process_id != own_process_id
        && source_related
    {
        PendingForegroundDecision::Capture
    } else if before_expiry {
        PendingForegroundDecision::Wait
    } else {
        PendingForegroundDecision::Expire
    }
}

fn pending_unsafe_helper_action(
    observation: PendingUnsafeHelperObservation,
    expired: bool,
) -> PendingUnsafeHelperAction {
    match observation {
        PendingUnsafeHelperObservation::Restored { clipboard_sequence } => {
            PendingUnsafeHelperAction::BecomeCustodian { clipboard_sequence }
        }
        PendingUnsafeHelperObservation::Completed => PendingUnsafeHelperAction::Shutdown,
        PendingUnsafeHelperObservation::Disconnected {
            process_exited: true,
        } => PendingUnsafeHelperAction::DropExited,
        PendingUnsafeHelperObservation::None
        | PendingUnsafeHelperObservation::Disconnected {
            process_exited: false,
        } if expired => PendingUnsafeHelperAction::Shutdown,
        PendingUnsafeHelperObservation::None
        | PendingUnsafeHelperObservation::Disconnected {
            process_exited: false,
        } => PendingUnsafeHelperAction::Keep,
    }
}

fn custodian_should_replace_existing(command_sent: bool) -> bool {
    command_sent
}

fn password_state_allows_clipboard(is_password: Option<bool>) -> bool {
    matches!(is_password, Some(false))
}

/// Clipboard password gate used after a UIA element has been resolved.
///
/// - `Some(true)` always blocks
/// - `Some(false)` always allows
/// - `None` fails closed for unknown apps, but allowlisted custom-rendered
///   applications (WPS, WeChat, Chromium shells, …) rarely expose IsPassword
///   on document surfaces — treat unknown as safe so Ctrl+C fallback works
fn clipboard_password_gate(is_password: Option<bool>, compatible_app: bool) -> bool {
    match is_password {
        Some(true) => false,
        Some(false) => true,
        None => compatible_app,
    }
}

/// Decide whether remaining UIA retries can be skipped in favour of clipboard.
///
/// `found_target` is `(password_safe, compatible)` when UIA resolved a target.
/// `foreground_compatible` is set when UIA never resolved but the foreground
/// process is already on the clipboard allowlist (multi-process WPS/CEF).
fn should_break_uia_for_clipboard(
    attempt: usize,
    found_target: Option<(bool, bool)>,
    foreground_compatible: Option<bool>,
) -> bool {
    if attempt < 1 {
        return false;
    }
    if found_target.is_some_and(|(password_safe, compatible)| password_safe && compatible) {
        return true;
    }
    found_target.is_none() && foreground_compatible.unwrap_or(false)
}

fn resolve_cached_source_app(
    cache: &mut Option<(u32, SourceApplication)>,
    process_id: u32,
    window: HWND,
) -> Option<SourceApplication> {
    if process_id == 0 {
        return None;
    }
    match cache {
        Some((cached_process_id, application)) if *cached_process_id == process_id => {
            Some(application.clone())
        }
        slot => {
            let application = source_application(process_id, window);
            *slot = Some((process_id, application.clone()));
            Some(application)
        }
    }
}

fn focused_element_is_password(automation: &IUIAutomation) -> bool {
    match unsafe { automation.GetFocusedElement() } {
        Ok(element) => unsafe { element.CurrentIsPassword() }
            .ok()
            .is_some_and(|value| value.as_bool()),
        Err(_) => false,
    }
}

fn uia_element_is_text_surface(element: &IUIAutomationElement) -> bool {
    unsafe { element.CurrentControlType() }
        .ok()
        .is_some_and(|control_type| {
            matches!(
                control_type,
                value if value == UIA_DocumentControlTypeId
                    || value == UIA_EditControlTypeId
                    || value == UIA_TextControlTypeId
                    || value == UIA_HyperlinkControlTypeId
            )
        })
}

fn uia_processes_belong_to_target(
    target_process_id: u32,
    element_process_id: u32,
    element_window_process_id: u32,
    process_parents: &mut Option<Option<HashMap<u32, u32>>>,
) -> bool {
    if target_process_id == 0 || element_process_id == 0 {
        return false;
    }
    if element_process_id == target_process_id
        && (element_window_process_id == 0 || element_window_process_id == target_process_id)
    {
        return true;
    }
    let related_by_tree = process_parents
        .get_or_insert_with(process_parent_map)
        .as_ref()
        .is_some_and(|parents| {
            uia_processes_belong_to_target_with_parents(
                target_process_id,
                element_process_id,
                element_window_process_id,
                parents,
            )
        });
    related_by_tree
        || (processes_share_executable(target_process_id, element_process_id)
            && (element_window_process_id == 0
                || processes_share_executable(target_process_id, element_window_process_id)))
}

fn uia_processes_belong_to_target_with_parents(
    target_process_id: u32,
    element_process_id: u32,
    element_window_process_id: u32,
    parents: &HashMap<u32, u32>,
) -> bool {
    let related = |process_id| {
        process_descends_from(process_id, target_process_id, parents)
            || process_descends_from(target_process_id, process_id, parents)
    };
    related(element_process_id)
        && (element_window_process_id == 0 || related(element_window_process_id))
}

fn processes_share_executable(left_process_id: u32, right_process_id: u32) -> bool {
    process_image_path(left_process_id)
        .zip(process_image_path(right_process_id))
        .is_some_and(|(left, right)| executables_share_application_family(&left, &right))
}

/// Same executable name, or members of the same multi-process application
/// family (WPS CEF hosts, WeChat AppEx, QQ NT helpers, …).
fn executables_share_application_family(left_image: &str, right_image: &str) -> bool {
    let left = executable_name(left_image);
    let right = executable_name(right_image);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    left == right
        || (wps_suite_process(&left) && wps_suite_process(&right))
        || (wechat_suite_process(&left) && wechat_suite_process(&right))
        || (qq_suite_process(&left) && qq_suite_process(&right))
}

/// WPS Office ships several cooperating processes; ElementFromPoint often
/// lands on the CEF plugin host while the foreground window stays on wps.exe.
fn wps_suite_process(executable: &str) -> bool {
    matches!(
        executable,
        "wps.exe"
            | "wpsoffice.exe"
            | "et.exe"
            | "wpp.exe"
            | "wpspdf.exe"
            | "promecefpluginhost.exe"
            | "ksolaunch.exe"
            | "wpscloudsvr.exe"
            | "wpscenter.exe"
    )
}

fn wechat_suite_process(executable: &str) -> bool {
    matches!(
        executable,
        "wechat.exe" | "weixin.exe" | "wechatappex.exe" | "wxwork.exe" | "wxworkweb.exe"
    )
}

fn qq_suite_process(executable: &str) -> bool {
    matches!(executable, "qq.exe" | "qqnt.exe" | "tim.exe")
}

fn clipboard_fallback_allowed(
    image_path: &str,
    foreground_matches: bool,
    password_safe: bool,
    text_surface: bool,
) -> bool {
    foreground_matches
        && password_safe
        && !prohibited_clipboard_application(image_path)
        && (compatible_clipboard_application(image_path) || text_surface)
}

fn compatible_clipboard_application(image_path: &str) -> bool {
    let executable = executable_name(image_path);
    matches!(
        executable.as_str(),
        // Browsers and PDF readers.
        "chrome.exe"
            | "msedge.exe"
            | "microsoftedge.exe"
            | "firefox.exe"
            | "brave.exe"
            | "opera.exe"
            | "vivaldi.exe"
            | "arc.exe"
            | "acrord32.exe"
            | "acrobat.exe"
            | "sumatrapdf.exe"
            | "foxitpdfreader.exe"
            | "foxitreader.exe"
            // Microsoft Office, LibreOffice and WPS Office (including PDF /
            // CEF hosts used by modern WPS document panes).
            | "winword.exe"
            | "excel.exe"
            | "powerpnt.exe"
            | "soffice.exe"
            | "soffice.bin"
            | "wps.exe"
            | "wpsoffice.exe"
            | "et.exe"
            | "wpp.exe"
            | "wpspdf.exe"
            | "promecefpluginhost.exe"
            | "ksolaunch.exe"
            // Chinese communication applications.
            | "wechat.exe"
            | "weixin.exe"
            | "wechatappex.exe"
            | "wxwork.exe"
            | "wxworkweb.exe"
            | "qq.exe"
            | "qqnt.exe"
            | "tim.exe"
            | "tencentmeeting.exe"
            | "wemeetapp.exe"
            | "dingtalk.exe"
            | "feishu.exe"
            | "lark.exe"
            // Other custom-rendered communication and note applications.
            | "teams.exe"
            | "ms-teams.exe"
            | "slack.exe"
            | "discord.exe"
            | "telegram.exe"
            | "notion.exe"
            | "obsidian.exe"
            // Cherry Studio is distributed under different executable names
            // across installer/channel variants.
            | "cherry studio.exe"
            | "cherrystudio.exe"
            | "cherry-studio.exe"
            | "chatgpt.exe"
            | "codex.exe"
            // Editors use custom Chromium text surfaces. Console hosts remain
            // explicitly prohibited below.
            | "emeditor.exe"
            | "code.exe"
            | "codeg.exe"
            | "cursor.exe"
            | "windsurf.exe"
            | "notepad++.exe"
            | "sublime_text.exe"
            | "typora.exe"
    )
}

fn prohibited_clipboard_application(image_path: &str) -> bool {
    matches!(
        executable_name(image_path).as_str(),
        // Shells and terminal hosts.
        "cmd.exe"
            | "powershell.exe"
            | "pwsh.exe"
            | "wt.exe"
            | "windowsterminal.exe"
            | "openconsole.exe"
            | "conhost.exe"
            // Password managers.
            | "1password.exe"
            | "bitwarden.exe"
            | "keepass.exe"
            | "keepassxc.exe"
            | "dashlane.exe"
            | "enpass.exe"
            | "lastpass.exe"
            | "protonpass.exe"
            // Remote-control clients must never receive an automatic Ctrl+C.
            | "mstsc.exe"
            | "teamviewer.exe"
            | "anydesk.exe"
            | "rustdesk.exe"
            | "todesk.exe"
            | "sunloginclient.exe"
            | "parsecd.exe"
    )
}

fn spreadsheet_copy_mode_application(image_path: &str) -> bool {
    matches!(executable_name(image_path).as_str(), "excel.exe" | "et.exe")
}

fn executable_name(image_path: &str) -> String {
    image_path
        .rsplit(|character| character == '\\' || character == '/')
        .next()
        .unwrap_or(image_path)
        .trim()
        .to_ascii_lowercase()
}

fn wait_for_copy_modifiers() -> Option<ModifierSnapshot> {
    for attempt in 0..=MODIFIER_RELEASE_ATTEMPTS {
        let modifiers = current_modifiers();
        if !modifiers.shift && !modifiers.alt && !modifiers.windows {
            return Some(modifiers);
        }
        if attempt < MODIFIER_RELEASE_ATTEMPTS {
            thread::sleep(CLIPBOARD_POLL_INTERVAL);
        }
    }
    None
}

fn post_copy_shortcut(control_already_down: bool) -> bool {
    let key = |virtual_key, flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                dwFlags: flags,
                dwExtraInfo: SYNTHETIC_COPY_MARKER,
                ..Default::default()
            },
        },
    };

    let mut inputs = Vec::with_capacity(4);
    if !control_already_down {
        inputs.push(key(VK_CONTROL, Default::default()));
    }
    inputs.push(key(VK_C, Default::default()));
    inputs.push(key(VK_C, KEYEVENTF_KEYUP));
    if !control_already_down {
        inputs.push(key(VK_CONTROL, KEYEVENTF_KEYUP));
    }

    let inserted = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if inserted as usize == inputs.len() {
        return true;
    }

    // A UIPI rejection normally inserts zero events. If Windows reports a
    // partial insertion, release only keys TextLens may have pressed.
    let mut releases = vec![key(VK_C, KEYEVENTF_KEYUP)];
    if !control_already_down {
        releases.push(key(VK_CONTROL, KEYEVENTF_KEYUP));
    }
    let _ = unsafe { SendInput(&releases, std::mem::size_of::<INPUT>() as i32) };
    false
}

fn post_synthetic_key(
    virtual_key: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY,
) -> bool {
    let input = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                dwFlags: flags,
                dwExtraInfo: SYNTHETIC_COPY_MARKER,
                ..Default::default()
            },
        },
    };
    let inputs = [input(Default::default()), input(KEYEVENTF_KEYUP)];
    (unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) }) as usize == inputs.len()
}

fn clipboard_source_context_is_valid(source_window: HWND, process_id: u32) -> bool {
    let foreground = unsafe { GetForegroundWindow() };
    let modifiers = current_modifiers();
    foreground == source_window
        && window_process_id(foreground) == process_id
        && !modifiers.shift
        && !modifiers.alt
        && !modifiers.windows
}

fn clipboard_capture_is_valid(
    source_window: HWND,
    process_id: u32,
    control: &CaptureControl,
) -> bool {
    !control.is_cancelled() && clipboard_source_context_is_valid(source_window, process_id)
}

fn clipboard_restore_allowed(
    expected_sequence: u32,
    current_sequence: u32,
    context_valid: bool,
) -> bool {
    context_valid && expected_sequence == current_sequence
}

fn clipboard_owner_matches_target(
    process_id: u32,
    process_parents: Option<&HashMap<u32, u32>>,
) -> bool {
    let Ok(owner) = (unsafe { windows::Win32::System::DataExchange::GetClipboardOwner() }) else {
        // A genuinely ownerless clipboard is valid (including delayed empty
        // states). Sequence stability remains authoritative in this case.
        return true;
    };
    clipboard_owner_process_allowed_with_parents(
        window_process_id(owner),
        process_id,
        process_parents,
    )
}

fn clipboard_owner_process_allowed_with_parents(
    owner_process_id: u32,
    target_process_id: u32,
    parents: Option<&HashMap<u32, u32>>,
) -> bool {
    owner_process_id == 0
        || owner_process_id == target_process_id
        || parents.is_some_and(|parents| {
            process_descends_from(owner_process_id, target_process_id, parents)
        })
}

fn wait_for_clipboard_change(
    original_sequence: u32,
    source_window: HWND,
    process_id: u32,
    deadline: Instant,
    process_parents: Option<&HashMap<u32, u32>>,
    control: &CaptureControl,
) -> Option<u32> {
    for _ in 0..CLIPBOARD_POLL_ATTEMPTS {
        thread::sleep(CLIPBOARD_POLL_INTERVAL);
        let cancel_reason = control.cancel_reason();
        if !clipboard_source_context_is_valid(source_window, process_id) {
            return None;
        }
        let current = clipboard::windows_clipboard_sequence();
        if current != original_sequence {
            if !clipboard_owner_matches_target(process_id, process_parents) {
                return None;
            }
            for _ in 0..CLIPBOARD_STABLE_POLLS {
                thread::sleep(CLIPBOARD_POLL_INTERVAL);
                if !clipboard_source_context_is_valid(source_window, process_id)
                    || clipboard::windows_clipboard_sequence() != current
                    || !clipboard_owner_matches_target(process_id, process_parents)
                {
                    return None;
                }
            }
            return Some(current);
        }
        if cancel_reason.is_some() || Instant::now() >= deadline {
            return None;
        }
    }
    None
}

fn is_textlens_synthetic_input(extra_info: usize) -> bool {
    extra_info == SYNTHETIC_COPY_MARKER
}

fn raw_selection_point(point: RawPoint) -> SelectionPoint {
    SelectionPoint {
        x: f64::from(point.x),
        y: f64::from(point.y),
    }
}

fn current_cursor_position() -> RawPoint {
    let mut point = POINT::default();
    if unsafe { GetCursorPos(&mut point) }.is_ok() {
        RawPoint {
            x: point.x,
            y: point.y,
        }
    } else {
        RawPoint { x: 0, y: 0 }
    }
}

fn window_at_point(point: RawPoint) -> HWND {
    let window = unsafe {
        WindowFromPoint(POINT {
            x: point.x,
            y: point.y,
        })
    };
    if window.is_invalid() {
        unsafe { GetForegroundWindow() }
    } else {
        window
    }
}

fn root_window(window: HWND) -> HWND {
    if window.is_invalid() {
        return window;
    }
    let root = unsafe { GetAncestor(window, GA_ROOT) };
    if root.is_invalid() {
        window
    } else {
        root
    }
}

fn key_is_down(virtual_key: u16) -> bool {
    unsafe { GetAsyncKeyState(i32::from(virtual_key)) as u16 & 0x8000 != 0 }
}

fn current_modifiers() -> ModifierSnapshot {
    ModifierSnapshot {
        shift: key_is_down(VK_SHIFT.0),
        control: key_is_down(VK_CONTROL.0),
        alt: key_is_down(VK_MENU.0),
        windows: key_is_down(VK_LWIN.0) || key_is_down(VK_RWIN.0),
    }
}

fn is_modifier_virtual_key(virtual_key: u16) -> bool {
    matches!(
        virtual_key,
        key if key == VK_SHIFT.0
            || key == VK_LSHIFT.0
            || key == VK_RSHIFT.0
            || key == VK_CONTROL.0
            || key == VK_LCONTROL.0
            || key == VK_RCONTROL.0
            || key == VK_MENU.0
            || key == VK_LMENU.0
            || key == VK_RMENU.0
            || key == VK_LWIN.0
            || key == VK_RWIN.0
    )
}

fn hook_generation() -> &'static AtomicU64 {
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    &GENERATION
}

fn trace_selection_capture(
    stage: &str,
    request: &CaptureRequest,
    source_root_window: isize,
    source_process_id: u32,
) {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if !*ENABLED.get_or_init(|| {
        std::env::var_os("TEXTLENS_SELECTION_TRACE").is_some_and(|value| {
            let value = value.to_string_lossy();
            value == "1" || value.eq_ignore_ascii_case("true")
        })
    }) {
        return;
    }
    eprintln!(
        "[selection] stage={stage} trigger={:?} root={source_root_window:#x} pid={source_process_id} generation={:?}",
        request.trigger, request.generation
    );
}

fn next_raw_input_sequence() -> u64 {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    SEQUENCE.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
}

fn rectangles_from_values(values: &[f64]) -> Vec<SelectionBounds> {
    values
        .chunks_exact(4)
        .filter_map(|chunk| {
            let rectangle = SelectionBounds {
                x: chunk[0],
                y: chunk[1],
                width: chunk[2],
                height: chunk[3],
            };
            selection_bounds_are_reasonable(rectangle).then_some(rectangle)
        })
        .collect()
}

fn endpoint_points(
    rectangles: &[SelectionBounds],
    direction: SelectionDirection,
) -> (
    Option<SelectionPoint>,
    Option<SelectionPoint>,
    Option<SelectionPoint>,
    Option<SelectionPoint>,
) {
    let (Some(first), Some(last)) = (rectangles.first(), rectangles.last()) else {
        return (None, None, None, None);
    };
    let left = |rectangle: &SelectionBounds| {
        (
            SelectionPoint {
                x: rectangle.x,
                y: rectangle.y,
            },
            SelectionPoint {
                x: rectangle.x,
                y: rectangle.y + rectangle.height,
            },
        )
    };
    let right = |rectangle: &SelectionBounds| {
        (
            SelectionPoint {
                x: rectangle.x + rectangle.width,
                y: rectangle.y,
            },
            SelectionPoint {
                x: rectangle.x + rectangle.width,
                y: rectangle.y + rectangle.height,
            },
        )
    };
    let ((start_top, start_bottom), (end_top, end_bottom)) =
        if direction == SelectionDirection::Backward {
            (right(last), left(first))
        } else {
            (left(first), right(last))
        };
    (
        Some(start_top),
        Some(start_bottom),
        Some(end_top),
        Some(end_bottom),
    )
}

fn bounding_rectangle_values(
    range: &windows::Win32::UI::Accessibility::IUIAutomationTextRange,
) -> Result<Vec<f64>, SelectionError> {
    let array = unsafe { range.GetBoundingRectangles() }.map_err(|_| SelectionError::Internal)?;
    if array.is_null() {
        return Ok(Vec::new());
    }
    let array = OwnedSafeArray(array);
    if unsafe { SafeArrayGetDim(array.0) } != 1 {
        return Ok(Vec::new());
    }
    let lower = unsafe { SafeArrayGetLBound(array.0, 1) }.map_err(|_| SelectionError::Internal)?;
    let upper = unsafe { SafeArrayGetUBound(array.0, 1) }.map_err(|_| SelectionError::Internal)?;
    if upper < lower {
        return Ok(Vec::new());
    }
    let length = usize::try_from(i64::from(upper) - i64::from(lower) + 1)
        .map_err(|_| SelectionError::Internal)?;
    if length > MAX_BOUNDING_VALUES {
        return Err(SelectionError::Internal);
    }
    if length % 4 != 0 {
        return Err(SelectionError::Internal);
    }
    let mut data = ptr::null_mut::<c_void>();
    unsafe { SafeArrayAccessData(array.0, &mut data) }.map_err(|_| SelectionError::Internal)?;
    let access = SafeArrayAccess { array: array.0 };
    if data.is_null() || length == 0 {
        return Ok(Vec::new());
    }
    // UI Automation specifies VT_R8 for text range bounding rectangles.
    let values = unsafe { std::slice::from_raw_parts(data.cast::<f64>(), length) }.to_vec();
    drop(access);
    Ok(values)
}

fn element_runtime_id(element: &IUIAutomationElement) -> Option<Vec<i32>> {
    let array = unsafe { element.GetRuntimeId() }.ok()?;
    if array.is_null() {
        return None;
    }
    let array = OwnedSafeArray(array);
    if unsafe { SafeArrayGetDim(array.0) } != 1 {
        return None;
    }
    let lower = unsafe { SafeArrayGetLBound(array.0, 1) }.ok()?;
    let upper = unsafe { SafeArrayGetUBound(array.0, 1) }.ok()?;
    if upper < lower {
        return None;
    }
    let length = usize::try_from(i64::from(upper) - i64::from(lower) + 1).ok()?;
    if length == 0 || length > MAX_UIA_RUNTIME_ID_VALUES {
        return None;
    }
    let mut data = ptr::null_mut::<c_void>();
    unsafe { SafeArrayAccessData(array.0, &mut data) }.ok()?;
    let access = SafeArrayAccess { array: array.0 };
    if data.is_null() {
        return None;
    }
    // UI Automation specifies VT_I4 for element runtime IDs.
    let values = unsafe { std::slice::from_raw_parts(data.cast::<i32>(), length) }.to_vec();
    drop(access);
    Some(values)
}

struct OwnedSafeArray(*mut SAFEARRAY);

impl Drop for OwnedSafeArray {
    fn drop(&mut self) {
        let _ = unsafe { SafeArrayDestroy(self.0) };
    }
}

struct SafeArrayAccess {
    array: *mut SAFEARRAY,
}

impl Drop for SafeArrayAccess {
    fn drop(&mut self) {
        let _ = unsafe { SafeArrayUnaccessData(self.array) };
    }
}

fn source_application(process_id: u32, window: HWND) -> SourceApplication {
    let image = process_image_path(process_id);
    let name = image
        .as_deref()
        .and_then(|path| Path::new(path).file_stem())
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| window_title(window))
        .unwrap_or_else(|| format!("Process {process_id}"));
    SourceApplication {
        bundle_id: image.unwrap_or_else(|| format!("windows.pid.{process_id}")),
        name,
    }
}

/// Best-effort: collapse the focused UIA text selection when it still equals `text`.
///
/// Never injects clipboard content or synthetic clicks. Returns `false` on any
/// failure (no focused text pattern, mismatch, password field, COM error).
pub fn clear_matching_text(bundle_id: Option<&str>, text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    clear_matching_text_inner(bundle_id, text).unwrap_or(false)
}

fn clear_matching_text_inner(bundle_id: Option<&str>, text: &str) -> Result<bool, SelectionError> {
    let _apartment = ComApartment::initialize()?;
    let automation =
        unsafe { CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|_| SelectionError::NativeInitializationFailed)?;

    let focused = match unsafe { automation.GetFocusedElement() } {
        Ok(element) => element,
        Err(_) => return Ok(false),
    };
    let own_process_id = unsafe { GetCurrentProcessId() };
    let element_process_id = match unsafe { focused.CurrentProcessId() } {
        Ok(value) if value > 0 => value as u32,
        _ => return Ok(false),
    };
    if element_process_id == own_process_id {
        return Ok(false);
    }
    if unsafe { focused.CurrentIsPassword() }
        .ok()
        .is_some_and(|value| value.as_bool())
    {
        return Ok(false);
    }

    let Some(pattern) = find_text_pattern_for_element(&automation, &focused) else {
        return Ok(false);
    };
    let ranges = match unsafe { pattern.GetSelection() } {
        Ok(ranges) => ranges,
        Err(_) => return Ok(false),
    };
    if unsafe { ranges.Length() }.unwrap_or(0) <= 0 {
        return Ok(false);
    }
    let range = match unsafe { ranges.GetElement(0) } {
        Ok(range) => range,
        Err(_) => return Ok(false),
    };
    let live_text = match unsafe { range.GetText(-1) } {
        Ok(value) => value.to_string(),
        Err(_) => return Ok(false),
    };

    let foreground = unsafe { GetForegroundWindow() };
    let process_id = {
        let foreground_pid = window_process_id(foreground);
        if foreground_pid != 0 {
            foreground_pid
        } else {
            element_process_id
        }
    };
    let source_app = source_application(process_id, foreground);
    if !super::should_clear_host_selection(
        text,
        &live_text,
        bundle_id,
        Some(source_app.bundle_id.as_str()),
    ) {
        return Ok(false);
    }

    collapse_text_range(&range)?;
    Ok(true)
}

fn find_text_pattern_for_element(
    automation: &IUIAutomation,
    focused: &IUIAutomationElement,
) -> Option<IUIAutomationTextPattern> {
    let walkers = [
        unsafe { automation.ControlViewWalker() }.ok(),
        unsafe { automation.RawViewWalker() }.ok(),
    ];
    let mut visited_runtime_ids = HashSet::new();
    let mut visited_without_runtime_id = Vec::<IUIAutomationElement>::new();
    for walker in walkers.into_iter().flatten() {
        let mut element = focused.clone();
        for _ in 0..MAX_UIA_ANCESTORS {
            let runtime_id = element_runtime_id(&element);
            let duplicate = runtime_id.as_ref().map_or_else(
                || {
                    visited_without_runtime_id.iter().any(|previous| {
                        unsafe { automation.CompareElements(previous, &element) }
                            .ok()
                            .is_some_and(|same| same.as_bool())
                    })
                },
                |runtime_id| !visited_runtime_ids.insert(runtime_id.clone()),
            );
            if !duplicate {
                match unsafe { element.CurrentIsPassword() }
                    .ok()
                    .map(|value| value.as_bool())
                {
                    Some(true) => return None,
                    Some(false) | None => {}
                }
                if let Ok(pattern) = unsafe {
                    element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                } {
                    return Some(pattern);
                }
                if runtime_id.is_none() {
                    visited_without_runtime_id.push(element.clone());
                }
            }
            element = match unsafe { walker.GetParentElement(&element) } {
                Ok(parent) => parent,
                Err(_) => break,
            };
        }
    }
    None
}

fn collapse_text_range(range: &IUIAutomationTextRange) -> Result<(), SelectionError> {
    // Move the End endpoint onto Start so the range length becomes 0, then
    // apply that collapsed caret as the active selection.
    unsafe {
        range
            .MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                range,
                TextPatternRangeEndpoint_Start,
            )
            .map_err(|_| SelectionError::Internal)?;
        range.Select().map_err(|_| SelectionError::Internal)?;
    }
    Ok(())
}

fn process_image_path(process_id: u32) -> Option<String> {
    let handle =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
    let handle = OwnedHandle(handle);
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            handle.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    }
    .ok()?;
    String::from_utf16(&buffer[..length as usize]).ok()
}

fn process_parent_map() -> Option<HashMap<u32, u32>> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }.ok()?;
    let snapshot = OwnedHandle(snapshot);
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    unsafe { Process32FirstW(snapshot.0, &mut entry) }.ok()?;

    let mut parents = HashMap::new();
    loop {
        if entry.th32ProcessID != 0 {
            parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
        }
        if unsafe { Process32NextW(snapshot.0, &mut entry) }.is_err() {
            break;
        }
    }
    Some(parents)
}

fn process_descends_from(
    mut process_id: u32,
    ancestor_process_id: u32,
    parents: &HashMap<u32, u32>,
) -> bool {
    if process_id == 0 || ancestor_process_id == 0 {
        return false;
    }
    for _ in 0..=MAX_PROCESS_ANCESTORS {
        if process_id == ancestor_process_id {
            return true;
        }
        let Some(parent_process_id) = parents.get(&process_id).copied() else {
            return false;
        };
        if parent_process_id == 0 || parent_process_id == process_id {
            return false;
        }
        process_id = parent_process_id;
    }
    false
}

fn process_allows_input_injection(target_process_id: u32) -> bool {
    match (
        process_integrity_level(unsafe { GetCurrentProcessId() }),
        process_integrity_level(target_process_id),
    ) {
        (Some(current), Some(target)) => integrity_allows_input_injection(current, target),
        // Integrity is a security boundary. If it cannot be established, rely
        // on UIA only and do not synthesize a clipboard shortcut.
        _ => false,
    }
}

fn integrity_allows_input_injection(current: u32, target: u32) -> bool {
    current >= target
}

fn process_integrity_level(process_id: u32) -> Option<u32> {
    let process =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
    let process = OwnedHandle(process);
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) }.ok()?;
    let token = OwnedHandle(token);

    let mut required = 0u32;
    let _ = unsafe { GetTokenInformation(token.0, TokenIntegrityLevel, None, 0, &mut required) };
    if required < std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 {
        return None;
    }
    // usize storage guarantees sufficient alignment for TOKEN_MANDATORY_LABEL.
    let word_size = std::mem::size_of::<usize>();
    let word_count = (required as usize).saturating_add(word_size - 1) / word_size;
    let mut buffer = vec![0usize; word_count];
    unsafe {
        GetTokenInformation(
            token.0,
            TokenIntegrityLevel,
            Some(buffer.as_mut_ptr().cast()),
            required,
            &mut required,
        )
    }
    .ok()?;
    let label = unsafe { &*(buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()) };
    if label.Label.Sid.0.is_null() {
        return None;
    }
    let sub_authority_count = unsafe { GetSidSubAuthorityCount(label.Label.Sid) };
    if sub_authority_count.is_null() || unsafe { *sub_authority_count } == 0 {
        return None;
    }
    let last_index = u32::from(unsafe { *sub_authority_count }) - 1;
    let level = unsafe { GetSidSubAuthority(label.Label.Sid, last_index) };
    (!level.is_null()).then(|| unsafe { *level })
}

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn window_title(window: HWND) -> Option<String> {
    if window.0 == ptr::null_mut() {
        return None;
    }
    let mut buffer = vec![0u16; 512];
    let length = unsafe { GetWindowTextW(window, &mut buffer) };
    (length > 0)
        .then(|| String::from_utf16_lossy(&buffer[..length as usize]))
        .filter(|title| !title.trim().is_empty())
}

fn window_process_id(window: HWND) -> u32 {
    if window.0 == ptr::null_mut() {
        return 0;
    }
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
    process_id
}

fn window_is_fullscreen(window: HWND) -> bool {
    if window.0 == ptr::null_mut() {
        return false;
    }
    let mut window_rectangle = RECT::default();
    if unsafe { GetWindowRect(window, &mut window_rectangle) }.is_err() {
        return false;
    }
    let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return unsafe { IsZoomed(window).as_bool() };
    }
    let mut monitor_info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut monitor_info) }.as_bool() {
        return unsafe { IsZoomed(window).as_bool() };
    }
    let monitor_rectangle = monitor_info.rcMonitor;
    (window_rectangle.left - monitor_rectangle.left).abs() <= 1
        && (window_rectangle.top - monitor_rectangle.top).abs() <= 1
        && (window_rectangle.right - monitor_rectangle.right).abs() <= 1
        && (window_rectangle.bottom - monitor_rectangle.bottom).abs() <= 1
}

fn timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn strict_parent_timestamp_ms() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let wall_clock = timestamp_ms();
    let mut previous = LAST.load(Ordering::Acquire);
    loop {
        let next = wall_clock.max(previous.saturating_add(1));
        match LAST.compare_exchange_weak(previous, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return next,
            Err(actual) => previous = actual,
        }
    }
}

fn pump_sta_messages() {
    let mut message = MSG::default();
    while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

struct HookThread {
    thread_id: u32,
    instance_id: u64,
    stop_requested: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl HookThread {
    fn start(inbox: Sender<WorkerMessage>) -> Result<Self, SelectionError> {
        let instance_id = next_hook_instance_id();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let published_thread_id = Arc::new(AtomicU32::new(0));
        let stop_requested = Arc::new(AtomicBool::new(false));
        let hook_thread_id = published_thread_id.clone();
        let hook_stop_requested = stop_requested.clone();
        let join = thread::Builder::new()
            .name("textlens-selection-hooks".to_owned())
            .spawn(move || {
                hook_thread_main(
                    inbox,
                    ready_sender,
                    hook_thread_id,
                    hook_stop_requested,
                    instance_id,
                )
            })
            .map_err(|_| SelectionError::EventTapFailed)?;
        match ready_receiver.recv_timeout(REQUEST_TIMEOUT) {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                instance_id,
                stop_requested,
                join: Some(join),
            }),
            Ok(Err(error)) => {
                let _ = join.join();
                Err(error)
            }
            Err(_) => {
                // Never detach a hook thread after a startup timeout. The
                // stop flag covers the race before its message queue exists;
                // once its thread ID is published WM_QUIT promptly wakes the
                // timed message pump.
                stop_requested.store(true, Ordering::Release);
                let thread_id = published_thread_id.load(Ordering::Acquire);
                if thread_id != 0 {
                    let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
                }
                let _ = join.join();
                Err(SelectionError::EventTapFailed)
            }
        }
    }

    fn stop(&mut self) -> Result<(), SelectionError> {
        self.stop_requested.store(true, Ordering::Release);
        // WM_QUIT is only a prompt wake-up. The atomic flag is authoritative,
        // so a transient PostThreadMessage failure cannot leave join waiting
        // forever for a message notification.
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(join) = self.join.take() {
            if join.join().is_err() {
                let _ = set_hook_inbox(None);
                return Err(SelectionError::Internal);
            }
        }
        let _ = set_hook_inbox(None);
        Ok(())
    }

    fn is_finished(&self) -> bool {
        self.join.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for HookThread {
    fn drop(&mut self) {
        if self.join.is_some() {
            let _ = self.stop();
        }
    }
}

fn hook_thread_main(
    inbox: Sender<WorkerMessage>,
    ready_sender: SyncSender<Result<u32, SelectionError>>,
    published_thread_id: Arc<AtomicU32>,
    stop_requested: Arc<AtomicBool>,
    instance_id: u64,
) {
    let exit_inbox = inbox.clone();
    // Calling PeekMessage creates this thread's message queue before its ID is
    // published to the worker, eliminating the PostThreadMessage startup race.
    let mut message = MSG::default();
    let _ = unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) };
    let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
    published_thread_id.store(thread_id, Ordering::Release);
    if stop_requested.load(Ordering::Acquire) {
        let _ = ready_sender.send(Err(SelectionError::EventTapFailed));
        return;
    }
    if !set_hook_inbox(Some(inbox)) {
        let _ = ready_sender.send(Err(SelectionError::EventTapFailed));
        return;
    }

    let mouse_hook =
        match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_callback), None, 0) } {
            Ok(hook) => hook,
            Err(_) => {
                set_hook_inbox(None);
                let _ = ready_sender.send(Err(SelectionError::EventTapFailed));
                return;
            }
        };
    let keyboard_hook =
        match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook_callback), None, 0) } {
            Ok(hook) => hook,
            Err(_) => {
                let _ = unsafe { UnhookWindowsHookEx(mouse_hook) };
                set_hook_inbox(None);
                let _ = ready_sender.send(Err(SelectionError::EventTapFailed));
                return;
            }
        };
    let foreground_hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground_event_callback),
            0,
            0,
            WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
        )
    };
    if stop_requested.load(Ordering::Acquire) {
        if !foreground_hook.is_invalid() {
            let _ = unsafe { UnhookWinEvent(foreground_hook) };
        }
        let _ = unsafe { UnhookWindowsHookEx(keyboard_hook) };
        let _ = unsafe { UnhookWindowsHookEx(mouse_hook) };
        set_hook_inbox(None);
        let _ = ready_sender.send(Err(SelectionError::EventTapFailed));
        return;
    }
    let _ = ready_sender.send(Ok(thread_id));

    'message_loop: loop {
        if stop_requested.load(Ordering::Acquire) {
            break;
        }
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            if message.message == WM_QUIT || stop_requested.load(Ordering::Acquire) {
                break 'message_loop;
            }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        let _ = unsafe {
            MsgWaitForMultipleObjectsEx(
                None,
                HOOK_HEALTH_INTERVAL.as_millis().min(u128::from(u32::MAX)) as u32,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            )
        };
    }
    if !foreground_hook.is_invalid() {
        let _ = unsafe { UnhookWinEvent(foreground_hook) };
    }
    let _ = unsafe { UnhookWindowsHookEx(keyboard_hook) };
    let _ = unsafe { UnhookWindowsHookEx(mouse_hook) };
    set_hook_inbox(None);
    let _ = exit_inbox.send(WorkerMessage::HookExited { instance_id });
}

fn next_hook_instance_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::AcqRel)
}

fn hook_inbox() -> &'static Mutex<Option<Sender<WorkerMessage>>> {
    static INBOX: OnceLock<Mutex<Option<Sender<WorkerMessage>>>> = OnceLock::new();
    INBOX.get_or_init(|| Mutex::new(None))
}

fn set_hook_inbox(inbox: Option<Sender<WorkerMessage>>) -> bool {
    let Ok(mut slot) = hook_inbox().lock() else {
        return false;
    };
    if inbox.is_some() && slot.is_some() {
        return false;
    }
    *slot = inbox;
    true
}

fn enqueue_hook_input(input: RawInput) {
    let inbox = hook_inbox()
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().cloned());
    if let Some(inbox) = inbox {
        let _ = inbox.send(WorkerMessage::Raw(input));
    }
}

unsafe extern "system" fn mouse_hook_callback(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        // SAFETY: Windows supplies a valid MSLLHOOKSTRUCT for HC_ACTION during
        // this callback. We copy the point and retain no borrowed pointer.
        let message = wparam.0 as u32;
        if matches!(
            message,
            WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_RBUTTONDOWN
                | WM_MBUTTONDOWN
                | WM_XBUTTONDOWN
                | WM_MOUSEWHEEL
                | WM_MOUSEHWHEEL
        ) {
            let input = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            enqueue_hook_input(RawInput::Mouse {
                sequence: next_raw_input_sequence(),
                message,
                point: RawPoint {
                    x: input.pt.x,
                    y: input.pt.y,
                },
                generation: hook_generation().fetch_add(1, Ordering::AcqRel) + 1,
                modifiers: current_modifiers(),
                timestamp_ms: strict_parent_timestamp_ms(),
            });
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn keyboard_hook_callback(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code == HC_ACTION as i32 && lparam.0 != 0 {
        // SAFETY: Windows supplies a valid KBDLLHOOKSTRUCT for HC_ACTION during
        // this callback. Only scalar fields are copied into the queue.
        let message = wparam.0 as u32;
        if matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP) {
            let input = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            if is_textlens_synthetic_input(input.dwExtraInfo) {
                return unsafe { CallNextHookEx(None, code, wparam, lparam) };
            }
            let generation = if is_modifier_virtual_key(input.vkCode as u16) {
                hook_generation().load(Ordering::Acquire)
            } else {
                hook_generation().fetch_add(1, Ordering::AcqRel) + 1
            };
            enqueue_hook_input(RawInput::Keyboard {
                sequence: next_raw_input_sequence(),
                message,
                virtual_key: input.vkCode,
                generation,
                modifiers: current_modifiers(),
                timestamp_ms: strict_parent_timestamp_ms(),
            });
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn foreground_event_callback(
    _hook: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    event: u32,
    window: HWND,
    _object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if event != EVENT_SYSTEM_FOREGROUND || window.0 == ptr::null_mut() {
        return;
    }
    enqueue_hook_input(RawInput::Foreground {
        sequence: next_raw_input_sequence(),
        window: window.0 as isize,
        // Foreground activation commonly lands between the mouse down/up that
        // moved focus from a TextLens result back to the source application.
        // Treating it as fresh user input cancels that legitimate capture.
        // UIA/clipboard paths already validate the foreground HWND/PID during
        // and after capture, so unrelated programmatic focus changes still
        // fail closed without advancing the input generation here.
        generation: hook_generation().load(Ordering::Acquire),
        timestamp_ms: strict_parent_timestamp_ms(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_worker(event_sender: Sender<SelectionEvent>) -> SelectionWorker {
        let (inbox, _receiver) = mpsc::channel();
        SelectionWorker {
            inbox,
            event_sender,
            own_process_id: 100,
            capture_executor: None,
            hook_thread: None,
            desired_listening: false,
            hook_restart_due: None,
            hook_exit_check_due: None,
            hook_health_due: None,
            mouse_down: None,
            mouse_down_at: None,
            mouse_down_on_self: false,
            mouse_down_target_root: 0,
            mouse_down_shift: false,
            last_mouse_up: None,
            last_click: None,
            keyboard_selection_key: None,
            pending_capture: None,
            last_automatic_fingerprint: None,
            recent_capture: None,
            last_raw_sequence: 0,
        }
    }

    fn test_selection(trigger: SelectionTrigger) -> SelectionPayload {
        SelectionPayload {
            text: "selected text".to_owned(),
            source_app: SourceApplication {
                bundle_id: r"C:\Windows\System32\notepad.exe".to_owned(),
                name: "Notepad".to_owned(),
            },
            bounds: None,
            start_top: None,
            start_bottom: None,
            end_top: None,
            end_bottom: None,
            mouse: SelectionMouse {
                start: Some(SelectionPoint { x: 10.0, y: 10.0 }),
                end: Some(SelectionPoint { x: 80.0, y: 10.0 }),
                current: SelectionPoint { x: 80.0, y: 10.0 },
            },
            direction: SelectionDirection::Forward,
            is_fullscreen: false,
            method: SelectionMethod::Accessibility,
            trigger,
            timestamp_ms: 1,
        }
    }

    #[test]
    fn external_foreground_preserves_an_active_mouse_gesture_across_root_changes() {
        assert_eq!(
            foreground_interaction_decision(true, 200, 100, true, false, false),
            ForegroundInteractionDecision::PreserveMouseGesture
        );
    }

    #[test]
    fn foreground_events_ignore_textlens_and_dismiss_only_without_active_work() {
        assert_eq!(
            foreground_interaction_decision(true, 100, 100, true, false, false),
            ForegroundInteractionDecision::Ignore
        );
        assert_eq!(
            foreground_interaction_decision(false, 0, 100, true, false, false),
            ForegroundInteractionDecision::Ignore
        );
        assert_eq!(
            foreground_interaction_decision(true, 200, 100, false, false, false),
            ForegroundInteractionDecision::Dismiss
        );
        assert_eq!(
            foreground_interaction_decision(true, 200, 100, true, true, false),
            ForegroundInteractionDecision::Dismiss
        );
    }

    #[test]
    fn external_foreground_after_mouse_up_preserves_pending_capture() {
        assert_eq!(
            foreground_interaction_decision(true, 200, 100, false, false, true),
            ForegroundInteractionDecision::PreservePendingCapture
        );
    }

    #[test]
    fn foreground_after_mouse_up_updates_only_capture_correlation() {
        let original_due = Instant::now() + Duration::from_millis(12);
        let mut pending = Some(PendingCapture {
            due: original_due,
            expires_at: Instant::now() + FOREGROUND_SETTLE_TIMEOUT,
            request: CaptureRequest {
                trigger: SelectionTrigger::Drag,
                start: Some(RawPoint { x: 10, y: 10 }),
                end: Some(RawPoint { x: 80, y: 10 }),
                current: RawPoint { x: 80, y: 10 },
                generation: Some(7),
            },
            source_root_window: 42,
            source_process_id: 100,
            process_parents: None,
        });

        correlate_pending_capture_with_foreground(&mut pending, 43, 100, 8);
        let pending = pending.expect("pending capture remains available");
        assert_eq!(pending.source_root_window, 43);
        assert_eq!(pending.source_process_id, 100);
        assert_eq!(pending.request.generation, Some(8));
        assert!(pending.due <= original_due);
    }

    #[test]
    fn older_foreground_generation_cannot_rewind_a_pending_capture() {
        let mut pending = Some(PendingCapture {
            due: Instant::now() + Duration::from_millis(12),
            expires_at: Instant::now() + FOREGROUND_SETTLE_TIMEOUT,
            request: CaptureRequest {
                trigger: SelectionTrigger::Drag,
                start: Some(RawPoint { x: 10, y: 10 }),
                end: Some(RawPoint { x: 80, y: 10 }),
                current: RawPoint { x: 80, y: 10 },
                generation: Some(9),
            },
            source_root_window: 42,
            source_process_id: 100,
            process_parents: None,
        });

        correlate_pending_capture_with_foreground(&mut pending, 43, 100, 8);
        assert_eq!(
            pending.expect("pending capture").request.generation,
            Some(9)
        );
    }

    #[test]
    fn late_same_source_foreground_is_correlated_with_the_completed_capture() {
        let (event_sender, _event_receiver) = mpsc::channel();
        let mut worker = test_worker(event_sender);
        worker.recent_capture = Some(RecentCaptureContext {
            generation: 12,
            source_root_window: 42,
            source_process_id: 100,
            valid_until: Instant::now() + RECENT_CAPTURE_FOREGROUND_GRACE,
        });

        assert!(worker.recent_capture_matches_foreground(42, 100, 12));
        assert!(!worker.recent_capture_matches_foreground(42, 100, 13));
        assert!(worker.recent_capture.is_none());
    }

    #[test]
    fn pending_capture_requires_a_related_external_foreground() {
        assert_eq!(
            pending_foreground_decision(true, 200, 100, true, true),
            PendingForegroundDecision::Capture
        );
        assert_eq!(
            pending_foreground_decision(true, 999, 100, false, true),
            PendingForegroundDecision::Wait
        );
    }

    #[test]
    fn pending_capture_waits_only_for_missing_or_textlens_foreground() {
        assert_eq!(
            pending_foreground_decision(false, 0, 100, false, true),
            PendingForegroundDecision::Wait
        );
        assert_eq!(
            pending_foreground_decision(true, 100, 100, false, true),
            PendingForegroundDecision::Wait
        );
        assert_eq!(
            pending_foreground_decision(false, 0, 100, false, false),
            PendingForegroundDecision::Expire
        );
        assert_eq!(
            pending_foreground_decision(true, 100, 100, false, false),
            PendingForegroundDecision::Expire
        );
    }

    #[test]
    fn pending_unsafe_helper_is_bounded_and_prefers_a_safe_result() {
        assert_eq!(
            pending_unsafe_helper_action(PendingUnsafeHelperObservation::None, false),
            PendingUnsafeHelperAction::Keep
        );
        assert_eq!(
            pending_unsafe_helper_action(PendingUnsafeHelperObservation::None, true),
            PendingUnsafeHelperAction::Shutdown
        );
        assert_eq!(
            pending_unsafe_helper_action(
                PendingUnsafeHelperObservation::Disconnected {
                    process_exited: false,
                },
                false,
            ),
            PendingUnsafeHelperAction::Keep
        );
        assert_eq!(
            pending_unsafe_helper_action(
                PendingUnsafeHelperObservation::Disconnected {
                    process_exited: true,
                },
                false,
            ),
            PendingUnsafeHelperAction::DropExited
        );
        assert_eq!(
            pending_unsafe_helper_action(
                PendingUnsafeHelperObservation::Restored {
                    clipboard_sequence: 42,
                },
                true,
            ),
            PendingUnsafeHelperAction::BecomeCustodian {
                clipboard_sequence: 42
            }
        );
        assert_eq!(
            pending_unsafe_helper_action(PendingUnsafeHelperObservation::Completed, false),
            PendingUnsafeHelperAction::Shutdown
        );
        assert!(PENDING_UNSAFE_HELPER_GRACE > HELPER_CLIPBOARD_CANCEL_GRACE);
    }

    #[test]
    fn failed_custodian_command_keeps_the_existing_owner() {
        assert!(custodian_should_replace_existing(true));
        assert!(!custodian_should_replace_existing(false));
    }

    #[test]
    fn helper_selection_result_emits_exactly_one_automatic_event() {
        let (event_sender, event_receiver) = mpsc::channel();
        let mut worker = test_worker(event_sender);
        let generation = hook_generation().load(Ordering::Acquire);
        let request = CaptureRequest {
            trigger: SelectionTrigger::Drag,
            start: Some(RawPoint { x: 10, y: 10 }),
            end: Some(RawPoint { x: 80, y: 10 }),
            current: RawPoint { x: 80, y: 10 },
            generation: Some(generation),
        };
        let selection = test_selection(SelectionTrigger::Drag);

        assert!(worker.deliver_captured_selection(request, selection.clone(), 42, 200));
        assert!(!worker.deliver_captured_selection(request, selection, 42, 200));
        assert!(matches!(
            event_receiver.recv_timeout(Duration::from_millis(50)),
            Ok(SelectionEvent::Selection(_))
        ));
        assert!(matches!(
            event_receiver.try_recv(),
            Err(TryRecvError::Empty)
        ));
    }

    #[test]
    fn invalid_uia_bounds_fall_back_to_the_release_point_without_losing_text() {
        let (event_sender, event_receiver) = mpsc::channel();
        let mut worker = test_worker(event_sender);
        let generation = hook_generation().load(Ordering::Acquire);
        let request = CaptureRequest {
            trigger: SelectionTrigger::Drag,
            start: Some(RawPoint { x: 10, y: 10 }),
            end: Some(RawPoint { x: 80, y: 10 }),
            current: RawPoint { x: 80, y: 10 },
            generation: Some(generation),
        };
        let mut selection = test_selection(SelectionTrigger::Drag);
        selection.bounds = Some(SelectionBounds {
            x: 5_000.0,
            y: 5_000.0,
            width: 100.0,
            height: 20.0,
        });

        assert!(worker.deliver_captured_selection(request, selection, 42, 200));
        let SelectionEvent::Selection(selection) = event_receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("selection event")
        else {
            panic!("expected selection event");
        };
        assert_eq!(selection.text, "selected text");
        assert_eq!(selection.bounds, None);
    }

    #[test]
    fn password_state_fails_closed_before_clipboard_fallback() {
        assert!(password_state_allows_clipboard(Some(false)));
        assert!(!password_state_allows_clipboard(Some(true)));
        assert!(!password_state_allows_clipboard(None));
    }

    #[test]
    fn clipboard_password_gate_allows_unknown_only_for_compatible_apps() {
        // Explicit password fields always block.
        assert!(!clipboard_password_gate(Some(true), true));
        assert!(!clipboard_password_gate(Some(true), false));
        // Explicit non-password always allows.
        assert!(clipboard_password_gate(Some(false), true));
        assert!(clipboard_password_gate(Some(false), false));
        // Unknown IsPassword: allowlisted custom-rendered apps (WPS) must not
        // lose clipboard fallback; unknown hosts still fail closed.
        assert!(clipboard_password_gate(None, true));
        assert!(!clipboard_password_gate(None, false));
    }

    #[test]
    fn uia_break_for_clipboard_keeps_first_frame_free_and_exits_after_one_retry() {
        // Never skip the first UIA attempt — zero cost on the happy path.
        assert!(!should_break_uia_for_clipboard(0, Some((true, true)), None));
        assert!(!should_break_uia_for_clipboard(0, None, Some(true)));
        // Found allowlisted target after one retry → clipboard immediately.
        assert!(should_break_uia_for_clipboard(1, Some((true, true)), None));
        assert!(!should_break_uia_for_clipboard(1, Some((false, true)), None));
        assert!(!should_break_uia_for_clipboard(1, Some((true, false)), None));
        // Multi-process WPS: no UIA target, but foreground is allowlisted.
        assert!(should_break_uia_for_clipboard(1, None, Some(true)));
        assert!(!should_break_uia_for_clipboard(1, None, Some(false)));
        assert!(!should_break_uia_for_clipboard(1, None, None));
    }

    #[test]
    fn multi_process_suites_share_an_application_family() {
        assert!(executables_share_application_family(
            r"C:\Program Files\WPS Office\office6\wps.exe",
            r"C:\Program Files\WPS Office\office6\wps.exe",
        ));
        assert!(executables_share_application_family(
            r"C:\Program Files\WPS Office\office6\wps.exe",
            r"C:\Program Files\WPS Office\office6\promecefpluginhost.exe",
        ));
        assert!(executables_share_application_family(
            r"C:\WPS\et.exe",
            r"C:\WPS\wpspdf.exe",
        ));
        assert!(executables_share_application_family(
            r"C:\Program Files\Tencent\WeChat\WeChat.exe",
            r"C:\Program Files\Tencent\WeChat\WeChatAppEx.exe",
        ));
        assert!(executables_share_application_family(
            r"C:\Program Files\Tencent\QQNT\QQ.exe",
            r"C:\Program Files\Tencent\QQNT\QQNT.exe",
        ));
        assert!(wps_suite_process("wps.exe"));
        assert!(wps_suite_process("promecefpluginhost.exe"));
        // Unrelated hosts must not collapse into a suite family.
        assert!(!executables_share_application_family(
            r"C:\Program Files\WPS Office\office6\wps.exe",
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        ));
        assert!(!executables_share_application_family(
            r"C:\Program Files\WPS Office\office6\wps.exe",
            r"C:\Program Files\Tencent\WeChat\WeChat.exe",
        ));
        assert!(!executables_share_application_family(
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        ));
    }

    #[test]
    fn clipboard_fallback_supports_known_apps_and_verified_text_surfaces() {
        for path in [
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            r"C:\Program Files\Vivaldi\Application\vivaldi.exe",
            r"C:\Program Files\Foxit Software\Foxit PDF Reader\FoxitPDFReader.exe",
            r"C:\Program Files\Microsoft Office\root\Office16\WINWORD.EXE",
            r"C:\Program Files\WPS Office\office6\wps.exe",
            r"C:\Program Files\WPS Office\office6\wpspdf.exe",
            r"C:\Program Files\WPS Office\office6\promecefpluginhost.exe",
            r"C:\Program Files\WXWork\WXWork.exe",
            r"C:\Users\User\AppData\Local\Feishu\Feishu.exe",
            r"C:\Users\User\AppData\Local\slack\slack.exe",
            r"C:\Users\User\AppData\Local\Programs\Notion\Notion.exe",
            r"C:\Users\User\AppData\Local\Programs\Microsoft VS Code\Code.exe",
            r"C:\Users\User\AppData\Local\Programs\Cursor\Cursor.exe",
            r"C:\Users\User\AppData\Local\Programs\Cherry Studio\Cherry Studio.exe",
            r"C:\Portable\CherryStudio.exe",
            r"C:\Portable\cherry-studio.exe",
            r"C:\Program Files\WindowsApps\OpenAI.Codex\app\ChatGPT.exe",
            r"C:\Program Files\OpenAI\Codex\codex.exe",
            r"C:\Users\User\AppData\Local\Programs\OpenAI\ChatGPT.exe",
            r"C:\Program Files\EmEditor\EmEditor.exe",
            r"C:\Program Files\codeg\codeg.exe",
        ] {
            assert!(
                clipboard_fallback_allowed(path, true, true, false),
                "{path}"
            );
        }
        assert!(
            compatible_clipboard_application(r"C:\Program Files\codeg\codeg.exe"),
            "CodeG content panes need clipboard fallback when UIA text is empty"
        );
        assert!(clipboard_fallback_allowed(
            r"C:\Program Files\codeg\codeg.exe",
            true,
            true,
            false, // not a verified text_surface — must still allow by app list
        ));
        assert!(!clipboard_fallback_allowed(
            r"C:\Windows\System32\notepad.exe",
            true,
            true,
            false,
        ));
        assert!(clipboard_fallback_allowed(
            r"C:\Apps\UnknownEditor.exe",
            true,
            true,
            true,
        ));
        assert!(!clipboard_fallback_allowed(
            r"C:\Program Files\Google\Chrome\chrome.exe.helper",
            true,
            true,
            false,
        ));
        assert!(!clipboard_fallback_allowed(
            r"C:\Program Files\OpenAI\Codex\codex.exe.helper",
            true,
            true,
            false,
        ));
        assert!(!clipboard_fallback_allowed(
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            false,
            true,
            true,
        ));
        assert!(!clipboard_fallback_allowed(
            r"C:\Program Files\Google\Chrome\Application\chrome.exe",
            true,
            false,
            true,
        ));
    }

    #[test]
    fn clipboard_fallback_rejects_sensitive_and_remote_control_processes() {
        for path in [
            r"C:\Windows\System32\cmd.exe",
            r"C:\Program Files\PowerShell\7\pwsh.exe",
            r"C:\Program Files\WindowsApps\Microsoft.WindowsTerminal\WindowsTerminal.exe",
            r"C:\Users\User\AppData\Local\1Password\app\1Password.exe",
            r"C:\Program Files\Bitwarden\Bitwarden.exe",
            r"C:\Program Files\AnyDesk\AnyDesk.exe",
            r"C:\Program Files\RustDesk\rustdesk.exe",
        ] {
            assert!(prohibited_clipboard_application(path), "{path}");
            assert!(
                !clipboard_fallback_allowed(path, true, true, true),
                "{path}"
            );
        }
    }

    #[test]
    fn spreadsheet_copy_mode_cleanup_is_narrowly_scoped() {
        assert!(spreadsheet_copy_mode_application(r"C:\Office\EXCEL.EXE"));
        assert!(spreadsheet_copy_mode_application(r"C:\WPS\et.exe"));
        assert!(!spreadsheet_copy_mode_application(r"C:\WPS\wps.exe"));
        assert!(!spreadsheet_copy_mode_application(r"C:\Office\WINWORD.EXE"));
    }

    #[test]
    fn clipboard_restore_requires_both_sequence_and_capture_context() {
        assert!(clipboard_restore_allowed(12, 12, true));
        assert!(!clipboard_restore_allowed(12, 13, true));
        assert!(!clipboard_restore_allowed(12, 12, false));
        assert!(clipboard_restore_allowed(0, 0, true));
    }

    #[test]
    fn clipboard_owner_must_be_target_child_or_genuinely_ownerless() {
        let parents = HashMap::from([(42, 41), (43, 42), (52, 51)]);
        assert!(clipboard_owner_process_allowed_with_parents(
            41,
            41,
            Some(&parents)
        ));
        assert!(clipboard_owner_process_allowed_with_parents(
            0,
            41,
            Some(&parents)
        ));
        assert!(clipboard_owner_process_allowed_with_parents(
            42,
            41,
            Some(&parents)
        ));
        assert!(clipboard_owner_process_allowed_with_parents(
            43,
            41,
            Some(&parents)
        ));
        assert!(!clipboard_owner_process_allowed_with_parents(
            52,
            41,
            Some(&parents)
        ));
        assert!(!clipboard_owner_process_allowed_with_parents(42, 41, None));
    }

    #[test]
    fn electron_uia_elements_must_stay_inside_the_foreground_process_tree() {
        let parents = HashMap::from([(101, 100), (102, 101), (201, 200)]);
        assert!(uia_processes_belong_to_target_with_parents(
            100, 101, 102, &parents
        ));
        assert!(uia_processes_belong_to_target_with_parents(
            100, 100, 101, &parents
        ));
        assert!(uia_processes_belong_to_target_with_parents(
            101, 100, 100, &parents
        ));
        assert!(!uia_processes_belong_to_target_with_parents(
            100, 201, 0, &parents
        ));
        assert!(!uia_processes_belong_to_target_with_parents(
            100, 101, 201, &parents
        ));
    }

    #[test]
    fn same_process_uia_fast_path_never_takes_a_process_snapshot() {
        let mut process_parents = None;
        assert!(uia_processes_belong_to_target(
            100,
            100,
            100,
            &mut process_parents
        ));
        assert!(process_parents.is_none());
    }

    #[test]
    fn input_injection_never_crosses_to_a_higher_integrity_process() {
        assert!(integrity_allows_input_injection(0x2000, 0x2000));
        assert!(integrity_allows_input_injection(0x3000, 0x2000));
        assert!(!integrity_allows_input_injection(0x2000, 0x3000));
    }

    #[test]
    fn accessibility_retries_and_ancestor_budget_match_windows_profiles() {
        assert_eq!(CAPTURE_SETTLE_DELAY, Duration::from_millis(12));
        assert_eq!(
            ACCESSIBILITY_RETRY_DELAYS,
            [
                Duration::ZERO,
                Duration::from_millis(24),
                Duration::from_millis(72),
            ]
        );
        assert_eq!(
            CAPTURE_SETTLE_DELAY + ACCESSIBILITY_RETRY_DELAYS[0],
            Duration::from_millis(12)
        );
        assert_eq!(
            CAPTURE_SETTLE_DELAY + ACCESSIBILITY_RETRY_DELAYS[2],
            Duration::from_millis(84)
        );
        assert_eq!(MAX_UIA_ANCESTORS, 32);
        assert_eq!(CLIPBOARD_POLL_ATTEMPTS, 60);
        assert_eq!(CLIPBOARD_STABLE_POLLS, 3);

        // The common path should reach its first UIA query within one frame,
        // while even the last UIA retry must remain well below the clipboard
        // fallback's bounded 600 ms polling window.
        let first_uia_query = CAPTURE_SETTLE_DELAY + ACCESSIBILITY_RETRY_DELAYS[0];
        let final_uia_query = CAPTURE_SETTLE_DELAY
            + *ACCESSIBILITY_RETRY_DELAYS
                .last()
                .expect("the UIA retry schedule cannot be empty");
        assert!(first_uia_query <= Duration::from_millis(16));
        assert!(final_uia_query <= Duration::from_millis(100));
        assert!(ACCESSIBILITY_RETRY_DELAYS
            .windows(2)
            .all(|pair| pair[0] < pair[1]));
        assert!(CAPTURE_TASK_TIMEOUT > final_uia_query);
        assert!(CAPTURE_ENGINE_BUDGET < CAPTURE_TASK_TIMEOUT);
    }

    #[test]
    fn helper_protocol_round_trips_without_starting_the_test_harness_as_a_child() {
        let command = HelperCommand::Capture {
            request_id: 7,
            request: CaptureRequest {
                trigger: SelectionTrigger::Drag,
                start: Some(RawPoint { x: -20, y: 30 }),
                end: Some(RawPoint { x: 40, y: 50 }),
                current: RawPoint { x: 40, y: 50 },
                generation: None,
            },
            source_window: 123,
            source_process_id: 456,
            blocked_clipboard_process_ids: vec![42, 43],
        };
        let mut bytes = Vec::new();
        write_helper_frame(&mut bytes, &command).expect("serialize helper command");
        let decoded = read_helper_frame::<HelperCommand>(&mut bytes.as_slice())
            .expect("read helper command")
            .expect("helper command frame");
        match decoded {
            HelperCommand::Capture {
                request_id,
                request,
                blocked_clipboard_process_ids,
                source_window,
                source_process_id,
            } => {
                assert_eq!(request_id, 7);
                assert_eq!(request.current, RawPoint { x: 40, y: 50 });
                assert_eq!(source_window, 123);
                assert_eq!(source_process_id, 456);
                assert_eq!(blocked_clipboard_process_ids, vec![42, 43]);
            }
            _ => panic!("expected capture command"),
        }
    }

    #[test]
    fn helper_shutdown_cancels_any_active_request() {
        assert_eq!(
            helper_cancel_reason_from_code(helper_cancel_reason_code(HelperCancelReason::Shutdown)),
            Some(HelperCancelReason::Shutdown)
        );
        assert!(CAPTURE_ENGINE_BUDGET < CAPTURE_TASK_TIMEOUT);
        assert!(HELPER_CLIPBOARD_CANCEL_GRACE > HELPER_CANCEL_GRACE);
    }

    #[test]
    fn helper_protocol_is_versioned_and_final_shutdown_flushes_only_its_own_sequence() {
        let event = HelperEvent::Ready {
            protocol_version: SELECTION_HELPER_PROTOCOL_VERSION,
        };
        let mut bytes = Vec::new();
        write_helper_frame(&mut bytes, &event).expect("serialize ready event");
        assert!(matches!(
            read_helper_frame::<HelperEvent>(&mut bytes.as_slice())
                .expect("read ready event")
                .expect("ready event frame"),
            HelperEvent::Ready { protocol_version }
                if protocol_version == SELECTION_HELPER_PROTOCOL_VERSION
        ));
        assert!(helper_should_flush_on_final_shutdown(Some(42), 42));
        assert!(!helper_should_flush_on_final_shutdown(Some(42), 43));
        assert!(!helper_should_flush_on_final_shutdown(None, 42));
    }

    #[test]
    fn raw_input_sequence_is_strictly_monotonic() {
        let first = next_raw_input_sequence();
        let second = next_raw_input_sequence();
        assert!(second > first);
        let first_timestamp = strict_parent_timestamp_ms();
        let second_timestamp = strict_parent_timestamp_ms();
        assert!(second_timestamp > first_timestamp);
        assert!(MAX_MESSAGES_PER_TICK > 0);
    }

    #[test]
    fn only_textlens_copy_events_are_filtered_from_the_keyboard_hook() {
        assert!(is_textlens_synthetic_input(SYNTHETIC_COPY_MARKER));
        assert!(!is_textlens_synthetic_input(0));
        assert!(!is_textlens_synthetic_input(SYNTHETIC_COPY_MARKER + 1));
    }

    #[test]
    fn modifier_keys_are_classified_without_async_state_races() {
        for key in [
            VK_SHIFT.0,
            VK_LSHIFT.0,
            VK_RSHIFT.0,
            VK_CONTROL.0,
            VK_LCONTROL.0,
            VK_RCONTROL.0,
            VK_MENU.0,
            VK_LMENU.0,
            VK_RMENU.0,
            VK_LWIN.0,
            VK_RWIN.0,
        ] {
            assert!(is_modifier_virtual_key(key));
        }
        // Caps Lock is user input, not a held shortcut modifier. It must
        // advance the capture generation and dismiss stale selections.
        assert!(!is_modifier_virtual_key(0x14));
        assert!(!is_modifier_virtual_key(VK_A.0));
        assert!(!is_modifier_virtual_key(VK_LEFT.0));
    }

    #[test]
    fn uia_rectangles_remain_physical_virtual_desktop_coordinates() {
        assert_eq!(
            rectangles_from_values(&[-2_400.0, 300.0, 150.0, 30.0]),
            vec![SelectionBounds {
                x: -2_400.0,
                y: 300.0,
                width: 150.0,
                height: 30.0,
            }]
        );
    }

    #[test]
    fn slow_worker_reaping_is_bounded() {
        let worker = thread::spawn(|| thread::sleep(Duration::from_millis(100)));
        let started = Instant::now();
        assert!(!reap_worker(worker, Duration::from_millis(5)));
        assert!(started.elapsed() < Duration::from_millis(80));
    }
}
