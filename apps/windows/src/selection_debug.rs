//! Opt-in Windows selection diagnostics, enabled only for the current app run.
use super::{selection_detection_debug_enabled, trace_selection_line, SelectionMethod};
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::Instant,
};

struct CaptureDebug {
    generation: u64,
    started: Instant,
    report: Value,
    steps: Vec<Value>,
}

// Odd generations are enabled. Transitions invalidate in-flight reports;
// helpers adopt the parent's generation without persisting anything.
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub(super) fn generation() -> u64 {
    GENERATION.load(Ordering::Acquire)
}

pub(super) fn enabled() -> bool {
    generation() % 2 == 1
}

pub(super) fn set_enabled(enabled: bool) -> u64 {
    let previous = GENERATION.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        ((current % 2 == 1) != enabled).then_some(current + 1)
    });
    if previous.is_ok() {
        clear_history();
    }
    generation()
}

pub(super) fn apply_generation(next: u64) {
    if GENERATION.fetch_max(next, Ordering::AcqRel) < next {
        clear_history();
    }
}

fn clear_history() {
    if let Ok(mut reports) = history().lock() {
        reports.clear();
    }
    if let Ok(mut events) = inputs().lock() {
        events.clear();
    }
}

fn is_current(expected: u64) -> bool {
    expected % 2 == 1 && generation() == expected
}

thread_local! {
    static CAPTURE: RefCell<Option<CaptureDebug>> = const { RefCell::new(None) };
}

fn history() -> &'static Mutex<VecDeque<(u64, Value)>> {
    static HISTORY: OnceLock<Mutex<VecDeque<(u64, Value)>>> = OnceLock::new();
    HISTORY.get_or_init(|| Mutex::new(VecDeque::new()))
}

fn inputs() -> &'static Mutex<VecDeque<Value>> {
    static INPUTS: OnceLock<Mutex<VecDeque<Value>>> = OnceLock::new();
    INPUTS.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub(super) fn input(stage: &str, details: Value) {
    if !selection_detection_debug_enabled() {
        return;
    }
    let expected = generation();
    let entry = json!({"stage": stage, "timestampMs": super::timestamp_ms(), "details": details});
    if let Ok(mut events) = inputs().lock() {
        if !is_current(expected) {
            return;
        }
        if events.len() == 48 {
            events.pop_front();
        }
        events.push_back(entry.clone());
    }
    trace_selection_line(format!("[selection-debug-input] {entry}"));
}

pub(super) fn input_snapshot() -> Vec<Value> {
    if !enabled() {
        return Vec::new();
    }
    inputs()
        .lock()
        .map(|events| events.iter().cloned().collect())
        .unwrap_or_default()
}

pub(super) fn begin(
    id: u64,
    request: super::CaptureRequest,
    control: &super::CaptureControl,
    expected: u64,
) {
    CAPTURE.with(|slot| slot.replace(None));
    if !is_current(expected) {
        return;
    }
    CAPTURE.with(|slot| slot.replace(Some(CaptureDebug {
        generation: expected,
        started: Instant::now(),
        report: json!({
            "version": "selection-offset-dev-v1", "debugGeneration": expected, "captureId": id,
            "timestampMs": super::timestamp_ms(),
            "gesture": request.mouse_gesture,
            "source": {"hwnd": format!("{:#x}",control.source_window.0 as usize), "pid": control.source_process_id},
            "dpiReady": super::DpiContext::ready(), "fallbackAttempted": false, "copyInjected": false,
            "method": null, "provider": null, "outcome": "pending", "selectionText": null,
            "logPath": super::selection_trace_directory().map(|d| d.join("selection-diagnostic.log").to_string_lossy().into_owned()),
        }),
        steps: Vec::new(),
    })));
}

pub(super) fn record(stage: &str, details: Value) {
    if !selection_detection_debug_enabled() {
        return;
    }
    CAPTURE.with(|slot| {
        if let Some(capture) = slot.borrow_mut().as_mut() {
            if !is_current(capture.generation) { return; }
            if stage == "route.clipboard" { capture.report["fallbackAttempted"] = json!(true); }
            if stage == "clipboard-ctrl-c-posted" { capture.report["copyInjected"] = json!(true); }
            if stage == "provider.accepted" { capture.report["provider"] = details["provider"].clone(); }
            if stage == "detector.initial" { capture.report["initialHealth"] = details["health"].clone(); }
            // Preserve both early lookup evidence and the final routing calls.
            if capture.steps.len() == 192 {
                capture.steps.remove(128);
                capture.report["stepsTruncated"] = json!(true);
            }
            capture.steps.push(json!({"stage":stage,"elapsedMs":capture.started.elapsed().as_secs_f64()*1000.0,"details":details}));
        }
    });
}

pub(super) fn finish(
    result: &super::HelperCaptureResult,
    health: &super::SelectionHealthResult,
) -> Option<Value> {
    CAPTURE.with(|slot| {
        let mut capture = slot.borrow_mut().take()?;
        if !is_current(capture.generation) {
            return None;
        }
        capture.report["durationMs"] = json!(capture.started.elapsed().as_secs_f64() * 1000.0);
        capture.report["health"] = json!(health);
        capture.report["steps"] = json!(capture.steps);
        match result {
            super::HelperCaptureResult::Selection { selection, .. } => {
                capture.report["outcome"] = json!("selection");
                capture.report["method"] = json!(selection.method);
                capture.report["selectionLength"] = json!(selection.text.chars().count());
                capture.report["selectionText"] =
                    json!(super::bounded_detection_text(&selection.text));
                if selection.method == SelectionMethod::Clipboard {
                    capture.report["provider"] = json!("guarded-clipboard");
                }
            }
            super::HelperCaptureResult::Empty => capture.report["outcome"] = json!("empty"),
            super::HelperCaptureResult::Error => capture.report["outcome"] = json!("error"),
        }
        trace_selection_line(format!("[selection-debug-report] {}", capture.report));
        Some(capture.report)
    })
}

pub(super) fn publish(timestamp: u64, mut report: Value, input_events: &[Value]) {
    let expected = report["debugGeneration"].as_u64().unwrap_or(0);
    if !is_current(expected) {
        return;
    }
    report["selectionTimestampMs"] = json!(timestamp);
    report["inputEvents"] = json!(input_events);
    trace_selection_line(format!("[selection-debug-published] {report}"));
    if let Ok(mut reports) = history().lock() {
        if !is_current(expected) {
            return;
        }
        if reports.len() == 16 {
            reports.pop_front();
        }
        reports.push_back((timestamp, report));
    }
}

pub(super) fn get(timestamp: u64) -> Option<Value> {
    if !selection_detection_debug_enabled() {
        return None;
    }
    history()
        .lock()
        .ok()?
        .iter()
        .rev()
        .find(|(at, _)| *at == timestamp)
        .map(|(_, report)| report.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    static TEST_LOCK: Mutex<()> = Mutex::new(());
    struct DebugTestGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }
    impl Drop for DebugTestGuard {
        fn drop(&mut self) {
            set_enabled(false);
        }
    }
    fn enable_test_debug() -> DebugTestGuard {
        let guard = DebugTestGuard {
            _lock: TEST_LOCK.lock().unwrap(),
        };
        set_enabled(true);
        guard
    }
    fn start_test_capture() {
        CAPTURE.with(|slot| {
            slot.replace(Some(CaptureDebug {
                generation: generation(),
                started: Instant::now(),
                report: json!({}),
                steps: Vec::new(),
            }))
        });
    }

    #[test]
    fn preserves_initial_failure_and_distinguishes_attempted_from_completed_copy() {
        let _guard = enable_test_debug();
        start_test_capture();
        let initial = super::super::SelectionHealthResult::unavailable(
            super::super::DetectionReason::BudgetExceeded,
        );
        record("detector.initial", json!({"health":initial}));
        let final_health = super::super::SelectionHealthResult::unavailable(
            super::super::DetectionReason::NoMouseContext,
        );
        record(
            "detector.candidate-revalidated",
            json!({"health":final_health}),
        );
        record("route.clipboard", json!({"health":final_health}));
        record("clipboard-blocked", json!({}));
        let report = finish(&super::super::HelperCaptureResult::Empty, &final_health).unwrap();
        assert_eq!(report["initialHealth"]["reason"], "BudgetExceeded");
        assert_eq!(report["health"]["reason"], "NoMouseContext");
        assert_eq!(report["fallbackAttempted"], true);
        assert_eq!(report["outcome"], "empty");
    }

    #[test]
    fn bounded_steps_retain_the_initial_context_and_final_route() {
        let _guard = enable_test_debug();
        start_test_capture();
        for index in 0..250 {
            record("probe", json!({"index":index}));
        }
        record("route.clipboard", json!({}));
        let health = super::super::SelectionHealthResult::unavailable(
            super::super::DetectionReason::NoMouseContext,
        );
        let report = finish(&super::super::HelperCaptureResult::Empty, &health).unwrap();
        let steps = report["steps"].as_array().unwrap();
        assert_eq!(steps.len(), 192);
        assert_eq!(steps[0]["details"]["index"], 0);
        assert_eq!(steps[127]["details"]["index"], 127);
        assert_eq!(steps.last().unwrap()["stage"], "route.clipboard");
        assert_eq!(report["stepsTruncated"], true);
    }

    #[test]
    fn reports_are_scoped_to_the_exact_selection_timestamp() {
        let _guard = enable_test_debug();
        let at = u64::MAX - 2;
        publish(
            at,
            json!({"captureId":101,"debugGeneration":generation()}),
            &[json!({"stage":"left-up"})],
        );
        assert_eq!(get(at).unwrap()["captureId"], 101);
        assert!(get(at + 1).is_none());
        assert_eq!(get(at).unwrap()["inputEvents"][0]["stage"], "left-up");
    }

    #[test]
    fn disabled_diagnostics_do_not_collect_or_publish() {
        let _guard = enable_test_debug();
        set_enabled(false);
        input("ignored", json!({}));
        publish(123, json!({"debugGeneration":generation()}), &[]);
        assert!(input_snapshot().is_empty());
        assert!(get(123).is_none());
    }

    #[test]
    fn disabling_clears_history_and_rejects_late_reports_after_reenabling() {
        let _guard = enable_test_debug();
        let old = generation();
        input("left-up", json!({}));
        publish(123, json!({"debugGeneration":old}), &[]);
        assert!(get(123).is_some());
        start_test_capture();
        set_enabled(false);
        record("ignored", json!({}));
        assert!(input_snapshot().is_empty());
        assert!(get(123).is_none());
        let health = super::super::SelectionHealthResult::unavailable(
            super::super::DetectionReason::NoMouseContext,
        );
        assert!(finish(&super::super::HelperCaptureResult::Empty, &health).is_none());
        set_enabled(true);
        publish(123, json!({"debugGeneration":old}), &[]);
        assert!(get(123).is_none());
        assert!(input_snapshot().is_empty());
    }

    #[test]
    fn helper_state_cannot_be_rolled_back_by_an_older_capture() {
        let _guard = enable_test_debug();
        let enabled_generation = generation();
        set_enabled(false);
        let disabled_generation = generation();
        apply_generation(enabled_generation);
        assert_eq!(generation(), disabled_generation);
        assert!(!enabled());
        assert_eq!(set_enabled(false), disabled_generation);
    }
}
