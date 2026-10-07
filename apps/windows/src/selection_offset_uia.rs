//! Windows adapter for the clipboard-free endpoint detector.
use super::selection_debug;
use super::selection_offset::{DetectionResult, Endpoint, Point, Reason, Rect, UiaAdapter};
use super::{
    bounding_rectangle_values, rectangles_from_values, root_window, window_process_id,
    CaptureControl,
};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::{
    Foundation::POINT,
    UI::{
        Accessibility::{
            IUIAutomationTextPattern, IUIAutomationTextRange, TextPatternRangeEndpoint,
            TextPatternRangeEndpoint_End, TextPatternRangeEndpoint_Start, TextUnit_Character,
        },
        HiDpi::{
            AreDpiAwarenessContextsEqual, GetThreadDpiAwarenessContext,
            SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT,
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        },
        WindowsAndMessaging::GetForegroundWindow,
    },
};

pub(super) struct DpiContext {
    previous: DPI_AWARENESS_CONTEXT,
}

impl DpiContext {
    pub(super) fn enter() -> Self {
        let previous =
            unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        Self { previous }
    }

    pub(super) fn ready() -> bool {
        unsafe {
            AreDpiAwarenessContextsEqual(
                GetThreadDpiAwarenessContext(),
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
            )
            .as_bool()
        }
    }
}

impl Drop for DpiContext {
    fn drop(&mut self) {
        if !self.previous.0.is_null() {
            unsafe {
                SetThreadDpiAwarenessContext(self.previous);
            }
        }
    }
}

pub(super) struct WindowsUiaAdapter<'a> {
    pattern: &'a IUIAutomationTextPattern,
    control: &'a CaptureControl,
    started: Instant,
    deadline: Instant,
}

impl<'a> WindowsUiaAdapter<'a> {
    pub(super) fn new(
        pattern: &'a IUIAutomationTextPattern,
        control: &'a CaptureControl,
        deadline: Instant,
    ) -> Self {
        Self {
            pattern,
            control,
            started: Instant::now(),
            deadline,
        }
    }
}

fn range_id(range: &IUIAutomationTextRange) -> String {
    format!("{:p}", range.as_raw())
}

// Record the calls already required by detection; no extra provider queries.
fn diagnosed<T>(
    operation: &str,
    input: Value,
    result: windows::core::Result<T>,
    output: impl FnOnce(&T) -> Value,
) -> DetectionResult<T> {
    match result {
        Ok(value) => {
            selection_debug::record(
                operation,
                json!({"input":input,"ok":true,"output":output(&value)}),
            );
            Ok(value)
        }
        Err(error) => {
            selection_debug::record(
                operation,
                json!({"input":input,"ok":false,"hresult":format!("0x{:08X}",error.code().0 as u32)}),
            );
            Err(Reason::UiaFailure)
        }
    }
}

fn endpoint(value: Endpoint) -> TextPatternRangeEndpoint {
    match value {
        Endpoint::Start => TextPatternRangeEndpoint_Start,
        Endpoint::End => TextPatternRangeEndpoint_End,
    }
}

impl UiaAdapter for WindowsUiaAdapter<'_> {
    type Range = IUIAutomationTextRange;

    fn checkpoint(&self, sample_ms: u64) -> DetectionResult<()> {
        if !DpiContext::ready() {
            selection_debug::record("uia.checkpoint", json!({"reason":"DpiUnavailable"}));
            return Err(Reason::DpiUnavailable);
        }
        let due = self.started + Duration::from_millis(sample_ms);
        loop {
            if self.control.is_cancelled() {
                selection_debug::record(
                    "uia.checkpoint",
                    json!({"reason":"ContextChanged","cancelled":true}),
                );
                return Err(Reason::ContextChanged);
            }
            let foreground = unsafe { GetForegroundWindow() };
            if root_window(foreground) != root_window(self.control.source_window)
                || window_process_id(foreground) != self.control.source_process_id
            {
                selection_debug::record(
                    "uia.checkpoint",
                    json!({"reason":"ContextChanged","foregroundHwnd":format!("{:#x}",foreground.0 as usize),"foregroundPid":window_process_id(foreground)}),
                );
                return Err(Reason::ContextChanged);
            }
            let now = Instant::now();
            if now >= self.deadline {
                selection_debug::record(
                    "uia.checkpoint",
                    json!({"reason":"BudgetExceeded","sampleMs":sample_ms,"elapsedMs":self.started.elapsed().as_secs_f64()*1000.0}),
                );
                return Err(Reason::BudgetExceeded);
            }
            if now >= due {
                return Ok(());
            }
            std::thread::sleep((due - now).min(Duration::from_millis(4)));
        }
    }

    fn selection(&self) -> DetectionResult<Vec<Self::Range>> {
        self.checkpoint(0)?;
        let ranges = diagnosed(
            "uia.GetSelection",
            json!({}),
            unsafe { self.pattern.GetSelection() },
            |_| json!(null),
        )?;
        let count = diagnosed(
            "uia.Selection.Length",
            json!({}),
            unsafe { ranges.Length() },
            |count| json!(count),
        )?;
        if count < 0 {
            return Err(Reason::UiaFailure);
        }
        (0..count.min(2))
            .map(|i| {
                self.checkpoint(0)?;
                diagnosed(
                    "uia.Selection.GetElement",
                    json!({"index":i}),
                    unsafe { ranges.GetElement(i) },
                    |range| json!({"range":range_id(range)}),
                )
            })
            .collect()
    }

    fn range_from_point(&self, point: Point) -> DetectionResult<Self::Range> {
        self.checkpoint(0)?;
        diagnosed(
            "uia.RangeFromPoint",
            json!({"point":point}),
            unsafe {
                self.pattern.RangeFromPoint(POINT {
                    x: point.x,
                    y: point.y,
                })
            },
            |range| json!({"range":range_id(range)}),
        )
    }

    fn has_text(&self, range: &Self::Range) -> DetectionResult<bool> {
        self.checkpoint(0)?;
        // One UTF-16 unit is sufficient for emptiness; do not retrieve the
        // document text or rely on a potentially broken CompareEndpoints.
        diagnosed(
            "uia.GetText",
            json!({"range":range_id(range),"maxLength":1}),
            unsafe { range.GetText(1) },
            |text| json!({"hasText":!text.is_empty()}),
        )
        .map(|text| !text.is_empty())
    }

    fn clone_range(&self, range: &Self::Range) -> DetectionResult<Self::Range> {
        self.checkpoint(0)?;
        diagnosed(
            "uia.Clone",
            json!({"range":range_id(range)}),
            unsafe { range.Clone() },
            |cloned| json!({"range":range_id(cloned)}),
        )
    }

    fn compare(
        &self,
        a: &Self::Range,
        ae: Endpoint,
        b: &Self::Range,
        be: Endpoint,
    ) -> DetectionResult<i32> {
        self.checkpoint(0)?;
        diagnosed(
            "uia.CompareEndpoints",
            json!({"a":range_id(a),"aEndpoint":format!("{ae:?}"),"b":range_id(b),"bEndpoint":format!("{be:?}")}),
            unsafe { a.CompareEndpoints(endpoint(ae), b, endpoint(be)) },
            |order| json!({"order":order}),
        )
    }

    fn collapse(&self, range: &Self::Range, value: Endpoint) -> DetectionResult<()> {
        self.checkpoint(0)?;
        let other = match value {
            Endpoint::Start => Endpoint::End,
            Endpoint::End => Endpoint::Start,
        };
        diagnosed(
            "uia.MoveEndpointByRange",
            json!({"range":range_id(range),"endpoint":format!("{other:?}"),"targetEndpoint":format!("{value:?}")}),
            unsafe { range.MoveEndpointByRange(endpoint(other), range, endpoint(value)) },
            |_| json!(null),
        )
    }

    fn move_character(
        &self,
        range: &Self::Range,
        value: Endpoint,
        count: i32,
    ) -> DetectionResult<i32> {
        self.checkpoint(0)?;
        diagnosed(
            "uia.MoveEndpointByUnit",
            json!({"range":range_id(range),"endpoint":format!("{value:?}"),"requested":count}),
            unsafe { range.MoveEndpointByUnit(endpoint(value), TextUnit_Character, count) },
            |moved| json!({"actual":moved}),
        )
    }

    fn rectangles(&self, range: &Self::Range) -> DetectionResult<Vec<Rect>> {
        self.checkpoint(0)?;
        let values = bounding_rectangle_values(range).map_err(|error| {
            selection_debug::record(
                "uia.GetBoundingRectangles",
                json!({"range":range_id(range),"ok":false,"error":format!("{error:?}")}),
            );
            Reason::UiaFailure
        })?;
        selection_debug::record(
            "uia.GetBoundingRectangles",
            json!({"range":range_id(range),"ok":true,"values":values.iter().take(256).collect::<Vec<_>>(),"valueCount":values.len()}),
        );
        Ok(rectangles_from_values(&values)
            .into_iter()
            .take(64)
            .map(|r| Rect {
                x: r.x,
                y: r.y,
                width: r.width,
                height: r.height,
            })
            .collect())
    }
}
