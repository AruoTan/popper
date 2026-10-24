//! Coordinate independent point and tree recognition without coupling providers.
mod automation;
mod worker;

use super::{accessibility::PointInspector, hit_test, CaptureHitTest};
use automation::Inspector;
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, Write},
    sync::mpsc,
    time::Duration,
};
use windows::Win32::{
    System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_MULTITHREADED,
    },
    UI::Accessibility::{CUIAutomation, IUIAutomation},
};
use worker::Lane;

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
        let inspector = if point_provider {
            Ok(HelperInspector::Accessibility(PointInspector::default()))
        } else {
            CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
                .map(Inspector::new)
                .map(HelperInspector::Automation)
        };
        if let Ok(mut inspector) = inspector {
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
                let candidates = inspector.inspect(probe).unwrap_or_default();
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

enum HelperInspector {
    Accessibility(PointInspector),
    Automation(Inspector),
}
impl HelperInspector {
    fn inspect(&mut self, probe: Probe) -> windows::core::Result<CaptureHitTest> {
        match self {
            Self::Accessibility(inspector) => {
                inspector.inspect(&probe.session, probe.handle, probe.x, probe.y)
            }
            Self::Automation(inspector) => inspector.inspect(probe),
        }
    }
}
