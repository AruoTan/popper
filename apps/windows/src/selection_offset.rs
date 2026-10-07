//! Clipboard-free mouse/UIA endpoint detection. This module also runs without
//! Windows or third-party crates, so the decision logic can be tested on Linux.

pub const GESTURE_TTL_MS: u64 = 60_000;
pub const CHARACTER_TOLERANCE: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(target_os = "windows", derive(serde::Serialize, serde::Deserialize))]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(target_os = "windows", derive(serde::Serialize, serde::Deserialize))]
pub enum GestureKind {
    Drag,
    MultiClick,
    ShiftClick,
    Click,
}

impl GestureKind {
    pub fn supports_detection(self) -> bool {
        matches!(self, Self::Drag | Self::MultiClick)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(target_os = "windows", derive(serde::Serialize, serde::Deserialize))]
pub struct MouseGesture {
    pub down: Point,
    pub up: Point,
    pub down_ms: u64,
    pub up_ms: u64,
    pub target_window: isize,
    pub target_pid: u32,
    pub kind: GestureKind,
}

impl MouseGesture {
    pub fn usable_at(self, now_ms: u64, window: isize, pid: u32) -> bool {
        now_ms >= self.up_ms
            && now_ms - self.up_ms <= GESTURE_TTL_MS
            && window != 0
            && pid != 0
            && self.target_window == window
            && self.target_pid == pid
    }
}

#[derive(Debug, Default)]
pub struct MouseSelectionTracker {
    pressed: Option<(Point, u64, isize, u32, bool, bool)>,
    completed: Option<MouseGesture>,
    last_click: Option<(Point, u64, isize)>,
}

impl MouseSelectionTracker {
    pub fn invalidate(&mut self) {
        self.pressed = None;
        self.completed = None;
        self.last_click = None;
    }

    pub fn down(
        &mut self,
        point: Point,
        at: u64,
        window: isize,
        pid: u32,
        shift: bool,
        double_click_ms: u64,
        click_size: (i32, i32),
    ) {
        self.completed = None;
        let multi = self.last_click.is_some_and(|(previous, time, target)| {
            target == window
                && at >= time
                && at - time <= double_click_ms
                && (i64::from(point.x) - i64::from(previous.x)).abs() <= i64::from(click_size.0)
                && (i64::from(point.y) - i64::from(previous.y)).abs() <= i64::from(click_size.1)
        });
        self.pressed = Some((point, at, window, pid, shift, multi));
    }

    pub fn up(&mut self, point: Point, at: u64, window: isize, pid: u32) {
        let Some((down, down_ms, target_window, target_pid, shift, multi)) = self.pressed.take()
        else {
            return;
        };
        if window != target_window || pid != target_pid || target_window == 0 || target_pid == 0 {
            self.invalidate();
            return;
        }
        let moved = (i64::from(down.x) - i64::from(point.x)).abs() > 3
            || (i64::from(down.y) - i64::from(point.y)).abs() > 3;
        let kind = if shift {
            GestureKind::ShiftClick
        } else if multi {
            // Word/paragraph dragging starts with a multi-click too.
            GestureKind::MultiClick
        } else if moved {
            GestureKind::Drag
        } else {
            GestureKind::Click
        };
        self.last_click = (!moved).then_some((point, at, window));
        self.completed = Some(MouseGesture {
            down,
            up: point,
            down_ms,
            up_ms: at,
            target_window,
            target_pid,
            kind,
        });
    }

    pub fn belongs_to(&self, window: isize, pid: u32) -> bool {
        self.pressed
            .map(|(_, _, w, p, _, _)| (w, p))
            .or_else(|| self.completed.map(|g| (g.target_window, g.target_pid)))
            == Some((window, pid))
    }

    pub fn foreground(&mut self, window: isize, pid: u32) {
        let target = self
            .pressed
            .map(|(_, _, window, pid, _, _)| (window, pid))
            .or_else(|| self.completed.map(|g| (g.target_window, g.target_pid)));
        if target.is_some_and(|target| target != (window, pid)) {
            self.invalidate();
        }
    }

    pub fn context(&self, at: u64, window: isize, pid: u32) -> Option<MouseGesture> {
        self.completed.filter(|g| g.usable_at(at, window, pid))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(target_os = "windows", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(target_os = "windows", serde(rename_all = "SCREAMING_SNAKE_CASE"))]
pub enum SelectionHealth {
    Normal,
    Suspicious,
    OffsetDetected,
    UiaUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(target_os = "windows", derive(serde::Serialize, serde::Deserialize))]
pub enum Direction {
    Forward,
    Reverse,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(target_os = "windows", derive(serde::Serialize, serde::Deserialize))]
pub enum Reason {
    EndpointsMatch,
    ClickMatchesSelection,
    ClickOutsideSelection,
    ClickMappingUnavailable,
    BothEndpointsMismatch,
    OneEndpointMismatch,
    GeometryMismatch,
    MultipleSelections,
    SelectionUnstable,
    NoMouseContext,
    UnsupportedGesture,
    EqualPointRanges,
    EmptySelection,
    UiaFailure,
    ContextChanged,
    BudgetExceeded,
    Protected,
    DpiUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(target_os = "windows", derive(serde::Serialize, serde::Deserialize))]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn valid(self) -> bool {
        [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.width > 0.0
            && self.height > 0.0
            && self.width < 250_000.0
            && self.height < 250_000.0
    }

    fn distance(self, point: Point) -> f64 {
        let x = f64::from(point.x);
        let y = f64::from(point.y);
        let dx = (self.x - x).max(0.0).max(x - self.x - self.width);
        let dy = (self.y - y).max(0.0).max(y - self.y - self.height);
        dx.hypot(dy)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(target_os = "windows", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(target_os = "windows", serde(rename_all = "camelCase"))]
pub struct ClickPointHealth {
    pub point: Point,
    pub in_selection: bool,
    pub geometry_valid: Option<bool>,
    pub distance: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(target_os = "windows", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(target_os = "windows", serde(rename_all = "camelCase"))]
pub struct ClickValidation {
    pub down: ClickPointHealth,
    pub up: ClickPointHealth,
    pub selection_rectangles: Vec<Rect>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(target_os = "windows", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(target_os = "windows", serde(rename_all = "camelCase"))]
pub struct SelectionHealthResult {
    pub status: SelectionHealth,
    pub applicable: bool,
    pub reason: Reason,
    pub selection_direction: Direction,
    pub start_endpoint_valid: Option<bool>,
    pub end_endpoint_valid: Option<bool>,
    pub start_geometry_valid: Option<bool>,
    pub end_geometry_valid: Option<bool>,
    pub start_distance: Option<f64>,
    pub end_distance: Option<f64>,
    /// Evidence completeness, NOT a probability of visual correctness.
    pub confidence: f32,
    pub stable: bool,
    pub start_rectangles: Vec<Rect>,
    pub end_rectangles: Vec<Rect>,
    #[cfg_attr(
        target_os = "windows",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub click_validation: Option<ClickValidation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionRoute {
    ExistingProviders,
    GuardedClipboard,
    RejectCapture,
}

impl SelectionHealthResult {
    pub fn unavailable(reason: Reason) -> Self {
        Self {
            status: SelectionHealth::UiaUnavailable,
            applicable: false,
            reason,
            selection_direction: Direction::Unknown,
            start_endpoint_valid: None,
            end_endpoint_valid: None,
            start_geometry_valid: None,
            end_geometry_valid: None,
            start_distance: None,
            end_distance: None,
            confidence: 0.0,
            stable: false,
            start_rectangles: Vec::new(),
            end_rectangles: Vec::new(),
            click_validation: None,
        }
    }

    pub fn suspicious(reason: Reason, applicable: bool) -> Self {
        Self {
            status: SelectionHealth::Suspicious,
            applicable,
            ..Self::unavailable(reason)
        }
    }

    pub fn route(&self) -> SelectionRoute {
        if matches!(self.reason, Reason::Protected | Reason::ContextChanged) {
            SelectionRoute::RejectCapture
        } else if self.requires_clipboard() {
            SelectionRoute::GuardedClipboard
        } else {
            SelectionRoute::ExistingProviders
        }
    }

    pub fn requires_clipboard(&self) -> bool {
        self.applicable
            && matches!(
                self.status,
                SelectionHealth::OffsetDetected | SelectionHealth::Suspicious
            )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    Start,
    End,
}

pub type DetectionResult<T> = Result<T, Reason>;

/// All methods operate on one TextPattern. No clipboard capability is exposed.
pub trait UiaAdapter {
    type Range;
    /// Wait until this sample offset, and check context/cancellation/deadline.
    fn checkpoint(&self, sample_ms: u64) -> DetectionResult<()>;
    fn selection(&self) -> DetectionResult<Vec<Self::Range>>;
    fn range_from_point(&self, point: Point) -> DetectionResult<Self::Range>;
    /// Read at most one UTF-16 unit to distinguish selected text from a caret.
    fn has_text(&self, range: &Self::Range) -> DetectionResult<bool>;
    fn clone_range(&self, range: &Self::Range) -> DetectionResult<Self::Range>;
    fn compare(
        &self,
        a: &Self::Range,
        ae: Endpoint,
        b: &Self::Range,
        be: Endpoint,
    ) -> DetectionResult<i32>;
    fn collapse(&self, range: &Self::Range, endpoint: Endpoint) -> DetectionResult<()>;
    fn move_character(
        &self,
        range: &Self::Range,
        endpoint: Endpoint,
        count: i32,
    ) -> DetectionResult<i32>;
    fn rectangles(&self, range: &Self::Range) -> DetectionResult<Vec<Rect>>;
}

pub struct Detection<R> {
    pub health: SelectionHealthResult,
    pub selection: Option<R>,
}

pub struct SelectionOffsetDetector;

impl SelectionOffsetDetector {
    pub fn analyze<A: UiaAdapter>(adapter: &A, gesture: MouseGesture) -> Detection<A::Range> {
        if !gesture.kind.supports_detection() {
            return Detection {
                health: SelectionHealthResult::suspicious(Reason::UnsupportedGesture, false),
                selection: None,
            };
        }
        if gesture.kind == GestureKind::Drag && gesture.down == gesture.up {
            return Detection {
                health: SelectionHealthResult::suspicious(Reason::EqualPointRanges, false),
                selection: None,
            };
        }
        let mut previous = None;
        let mut any_selection = false;
        for offset in [0, 28, 56] {
            let sample = (|| {
                adapter.checkpoint(offset)?;
                let ranges = adapter.selection()?;
                if ranges.len() > 1 {
                    return Ok((
                        None,
                        Some(SelectionHealthResult::suspicious(
                            Reason::MultipleSelections,
                            true,
                        )),
                    ));
                }
                let current = ranges.into_iter().next();
                let Some(current) = current else {
                    previous = None;
                    return Ok((None, None));
                };
                any_selection = true;
                let stable = match previous.as_ref() {
                    Some(old) => {
                        adapter.compare(&current, Endpoint::Start, old, Endpoint::Start)? == 0
                            && adapter.compare(&current, Endpoint::End, old, Endpoint::End)? == 0
                    }
                    None => false,
                };
                if stable {
                    let mut health = Self::analyze_snapshot(adapter, &current, gesture);
                    health.stable = true;
                    return Ok((Some(current), Some(health)));
                }
                previous = Some(adapter.clone_range(&current)?);
                Ok((None, None))
            })();
            match sample {
                Ok((selection, Some(health))) => return Detection { health, selection },
                Ok(_) => {}
                Err(reason) => {
                    return Detection {
                        health: SelectionHealthResult::unavailable(reason),
                        selection: None,
                    }
                }
            }
        }
        Detection {
            health: if any_selection {
                SelectionHealthResult::suspicious(Reason::SelectionUnstable, true)
            } else {
                SelectionHealthResult::unavailable(Reason::EmptySelection)
            },
            selection: None,
        }
    }

    pub fn analyze_snapshot<A: UiaAdapter>(
        adapter: &A,
        selection: &A::Range,
        gesture: MouseGesture,
    ) -> SelectionHealthResult {
        if !gesture.kind.supports_detection() {
            return SelectionHealthResult::suspicious(Reason::UnsupportedGesture, false);
        }
        if gesture.kind == GestureKind::Drag && gesture.down == gesture.up {
            return SelectionHealthResult::suspicious(Reason::EqualPointRanges, false);
        }
        let result = match gesture.kind {
            GestureKind::MultiClick => Self::clicked_selection(adapter, selection, gesture),
            _ => Self::endpoints(adapter, selection, gesture),
        };
        match result {
            Ok(health) => health,
            Err(reason) => SelectionHealthResult::unavailable(reason),
        }
    }

    fn endpoints<A: UiaAdapter>(
        adapter: &A,
        selection: &A::Range,
        gesture: MouseGesture,
    ) -> DetectionResult<SelectionHealthResult> {
        adapter.checkpoint(0)?;
        // Check actual text rather than trusting another endpoint comparison
        // from a provider whose point mapping may be unusable. A caret must
        // never become an applicable anomaly, regardless of mouse mapping.
        if !adapter.has_text(selection)? {
            return Err(Reason::EmptySelection);
        }
        let down = adapter.range_from_point(gesture.down)?;
        let up = adapter.range_from_point(gesture.up)?;
        for range in [&down, &up] {
            // Embedded objects can return non-degenerate ranges.
            if adapter.compare(range, Endpoint::Start, range, Endpoint::End)? != 0 {
                return Err(Reason::UiaFailure);
            }
        }
        let order = adapter.compare(&down, Endpoint::Start, &up, Endpoint::Start)?;
        if order == 0 {
            // The entry points have already excluded identical screen points.
            // A real drag selecting text whose distinct mouse points map to
            // one insertion position has untrustworthy UIA point mapping.
            // Route through guarded copy without claiming a measured offset.
            return Ok(SelectionHealthResult::suspicious(
                Reason::EqualPointRanges,
                true,
            ));
        }
        let (start, end, start_point, end_point, direction) = if order < 0 {
            (&down, &up, gesture.down, gesture.up, Direction::Forward)
        } else {
            (&up, &down, gesture.up, gesture.down, Direction::Reverse)
        };
        let start_ok = endpoint_near(adapter, selection, Endpoint::Start, start)?;
        let end_ok = endpoint_near(adapter, selection, Endpoint::End, end)?;
        let (start_geometry, start_distance, start_rectangles) =
            geometry(adapter, selection, Endpoint::Start, start, start_point);
        let (end_geometry, end_distance, end_rectangles) =
            geometry(adapter, selection, Endpoint::End, end, end_point);
        // Geometry failure is advisory, but cancellation/deadlines are not.
        adapter.checkpoint(0)?;
        let (status, reason) = match (start_ok, end_ok) {
            (false, false) => (
                SelectionHealth::OffsetDetected,
                Reason::BothEndpointsMismatch,
            ),
            (false, true) | (true, false) => {
                (SelectionHealth::Suspicious, Reason::OneEndpointMismatch)
            }
            (true, true) if start_geometry == Some(false) || end_geometry == Some(false) => {
                (SelectionHealth::Suspicious, Reason::GeometryMismatch)
            }
            _ => (SelectionHealth::Normal, Reason::EndpointsMatch),
        };
        Ok(SelectionHealthResult {
            status,
            applicable: true,
            reason,
            selection_direction: direction,
            start_endpoint_valid: Some(start_ok),
            end_endpoint_valid: Some(end_ok),
            start_geometry_valid: start_geometry,
            end_geometry_valid: end_geometry,
            start_distance,
            end_distance,
            confidence: 0.7
                + if start_geometry.is_some() { 0.15 } else { 0.0 }
                + if end_geometry.is_some() { 0.15 } else { 0.0 },
            stable: false,
            start_rectangles,
            end_rectangles,
            click_validation: None,
        })
    }

    fn clicked_selection<A: UiaAdapter>(
        adapter: &A,
        selection: &A::Range,
        gesture: MouseGesture,
    ) -> DetectionResult<SelectionHealthResult> {
        adapter.checkpoint(0)?;
        if !adapter.has_text(selection)? {
            return Err(Reason::EmptySelection);
        }
        // A word/paragraph selection encloses its click, rather than having
        // endpoints at the click. Validate containment on an expanded clone.
        let validation = (|| {
            let probe = adapter.clone_range(selection)?;
            adapter.move_character(&probe, Endpoint::Start, -CHARACTER_TOLERANCE)?;
            adapter.move_character(&probe, Endpoint::End, CHARACTER_TOLERANCE)?;
            let rectangles: Vec<_> = adapter
                .rectangles(selection)
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.valid())
                .collect();
            let down = clicked_point(adapter, &probe, &rectangles, gesture.down)?;
            let up = if gesture.up == gesture.down {
                down
            } else {
                clicked_point(adapter, &probe, &rectangles, gesture.up)?
            };
            // Missing rectangles remain advisory; cancellation/deadlines don't.
            adapter.checkpoint(0)?;
            let (status, reason) = if !down.in_selection || !up.in_selection {
                (SelectionHealth::Suspicious, Reason::ClickOutsideSelection)
            } else if down.geometry_valid == Some(false) || up.geometry_valid == Some(false) {
                (SelectionHealth::Suspicious, Reason::GeometryMismatch)
            } else {
                (SelectionHealth::Normal, Reason::ClickMatchesSelection)
            };
            Ok(SelectionHealthResult {
                status,
                applicable: true,
                reason,
                confidence: 0.7
                    + if down.geometry_valid.is_some() {
                        0.15
                    } else {
                        0.0
                    }
                    + if up.geometry_valid.is_some() {
                        0.15
                    } else {
                        0.0
                    },
                click_validation: Some(ClickValidation {
                    down,
                    up,
                    selection_rectangles: rectangles,
                }),
                ..SelectionHealthResult::unavailable(reason)
            })
        })();
        match validation {
            // We know selected text exists, but cannot verify its click mapping.
            // Reuse guarded copy; context/deadline errors still propagate.
            Err(Reason::UiaFailure) => {
                adapter.checkpoint(0)?;
                Ok(SelectionHealthResult::suspicious(
                    Reason::ClickMappingUnavailable,
                    true,
                ))
            }
            other => other,
        }
    }
}

fn clicked_point<A: UiaAdapter>(
    adapter: &A,
    selection_probe: &A::Range,
    rectangles: &[Rect],
    point: Point,
) -> DetectionResult<ClickPointHealth> {
    adapter.checkpoint(0)?;
    let mouse = adapter.range_from_point(point)?;
    if adapter.compare(&mouse, Endpoint::Start, &mouse, Endpoint::End)? != 0 {
        return Err(Reason::UiaFailure);
    }
    let in_selection =
        adapter.compare(&mouse, Endpoint::Start, selection_probe, Endpoint::Start)? >= 0
            && adapter.compare(&mouse, Endpoint::Start, selection_probe, Endpoint::End)? <= 0;
    let closest = rectangles
        .iter()
        .copied()
        .min_by(|a, b| a.distance(point).total_cmp(&b.distance(point)));
    // A nearest-text mapping in whitespace isn't a reliable geometry anchor.
    let anchor = [Endpoint::Start, Endpoint::End].into_iter().any(|side| {
        character_rectangles(adapter, &mouse, side)
            .unwrap_or_default()
            .iter()
            .any(|r| r.distance(point) <= 3.0)
    });
    let geometry_valid = anchor
        .then(|| closest.map(|r| r.distance(point) <= 24.0f64.max(2.0 * r.height)))
        .flatten();
    Ok(ClickPointHealth {
        point,
        in_selection,
        geometry_valid,
        distance: closest.map(|r| r.distance(point)),
    })
}

fn endpoint_near<A: UiaAdapter>(
    adapter: &A,
    selection: &A::Range,
    endpoint: Endpoint,
    mouse: &A::Range,
) -> DetectionResult<bool> {
    adapter.checkpoint(0)?;
    let probe = adapter.clone_range(mouse)?;
    adapter.move_character(&probe, Endpoint::Start, -CHARACTER_TOLERANCE)?;
    adapter.move_character(&probe, Endpoint::End, CHARACTER_TOLERANCE)?;
    Ok(
        adapter.compare(selection, endpoint, &probe, Endpoint::Start)? >= 0
            && adapter.compare(selection, endpoint, &probe, Endpoint::End)? <= 0,
    )
}

fn character_rectangles<A: UiaAdapter>(
    adapter: &A,
    range: &A::Range,
    endpoint: Endpoint,
) -> DetectionResult<Vec<Rect>> {
    adapter.checkpoint(0)?;
    let probe = adapter.clone_range(range)?;
    adapter.collapse(&probe, endpoint)?;
    let (moving, count) = match endpoint {
        Endpoint::Start => (Endpoint::End, 1),
        Endpoint::End => (Endpoint::Start, -1),
    };
    if adapter.move_character(&probe, moving, count)? == 0 {
        return Ok(Vec::new());
    }
    Ok(adapter
        .rectangles(&probe)?
        .into_iter()
        .filter(|r| r.valid())
        .collect())
}

fn geometry<A: UiaAdapter>(
    adapter: &A,
    selection: &A::Range,
    endpoint: Endpoint,
    mouse: &A::Range,
    point: Point,
) -> (Option<bool>, Option<f64>, Vec<Rect>) {
    let rectangles = character_rectangles(adapter, selection, endpoint).unwrap_or_default();
    let closest = rectangles
        .iter()
        .copied()
        .min_by(|a, b| a.distance(point).total_cmp(&b.distance(point)));
    let distance = closest.map(|r| r.distance(point));
    // The pointer must actually be on a neighboring character. RangeFromPoint
    // maps whitespace to nearest text too; that is not a trustworthy geometry anchor.
    let anchor = [Endpoint::Start, Endpoint::End].into_iter().any(|side| {
        character_rectangles(adapter, mouse, side)
            .unwrap_or_default()
            .iter()
            .any(|r| r.distance(point) <= 3.0)
    });
    let valid = anchor
        .then(|| closest.map(|r| r.distance(point) <= 24.0f64.max(2.0 * r.height)))
        .flatten();
    (valid, distance, rectangles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[derive(Debug)]
    struct Range {
        endpoints: RefCell<(i32, i32)>,
        mouse: bool,
    }
    impl Range {
        fn new(start: i32, end: i32, mouse: bool) -> Self {
            Self {
                endpoints: RefCell::new((start, end)),
                mouse,
            }
        }
        fn endpoint(&self, endpoint: Endpoint) -> i32 {
            let pair = *self.endpoints.borrow();
            match endpoint {
                Endpoint::Start => pair.0,
                Endpoint::End => pair.1,
            }
        }
    }

    struct MockUia {
        samples: Vec<Vec<(i32, i32)>>,
        sample: Cell<usize>,
        down: i32,
        up: i32,
        mouse_down_x: i32,
        point_failure: Option<Reason>,
        text_failure: Option<Reason>,
        move_failure: Option<Reason>,
        failure: Option<Reason>,
        stop_at: Option<(u64, Reason)>,
        rectangles_missing: bool,
        selection_rectangle_shift: f64,
        document_end: i32,
        checkpoints: RefCell<Vec<u64>>,
    }

    impl MockUia {
        fn selection(start: i32, end: i32) -> Self {
            Self {
                samples: vec![vec![(start, end)]],
                sample: Cell::new(0),
                down: 10,
                up: 30,
                mouse_down_x: 100,
                point_failure: None,
                text_failure: None,
                move_failure: None,
                failure: None,
                stop_at: None,
                rectangles_missing: false,
                selection_rectangle_shift: 0.0,
                document_end: 100,
                checkpoints: RefCell::new(Vec::new()),
            }
        }
    }

    impl UiaAdapter for MockUia {
        type Range = Range;
        fn checkpoint(&self, ms: u64) -> DetectionResult<()> {
            self.checkpoints.borrow_mut().push(ms);
            if let Some((stop, reason)) = self.stop_at {
                if ms >= stop {
                    return Err(reason);
                }
            }
            Ok(())
        }
        fn selection(&self) -> DetectionResult<Vec<Range>> {
            if let Some(reason) = self.failure {
                return Err(reason);
            }
            let index = self.sample.get().min(self.samples.len() - 1);
            self.sample.set(self.sample.get() + 1);
            Ok(self.samples[index]
                .iter()
                .map(|&(a, b)| Range::new(a, b, false))
                .collect())
        }
        fn range_from_point(&self, point: Point) -> DetectionResult<Range> {
            if let Some(reason) = self.point_failure {
                return Err(reason);
            }
            let index = if point.x == self.mouse_down_x {
                self.down
            } else {
                self.up
            };
            Ok(Range::new(index, index, true))
        }
        fn has_text(&self, range: &Range) -> DetectionResult<bool> {
            self.checkpoint(0)?;
            if let Some(reason) = self.text_failure {
                return Err(reason);
            }
            Ok(range.endpoint(Endpoint::Start) != range.endpoint(Endpoint::End))
        }
        fn clone_range(&self, range: &Range) -> DetectionResult<Range> {
            let (a, b) = *range.endpoints.borrow();
            Ok(Range::new(a, b, range.mouse))
        }
        fn compare(
            &self,
            a: &Range,
            ae: Endpoint,
            b: &Range,
            be: Endpoint,
        ) -> DetectionResult<i32> {
            // Deliberately return only signs: callers must not use this as a distance.
            Ok(match a.endpoint(ae).cmp(&b.endpoint(be)) {
                std::cmp::Ordering::Less => -99,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 99,
            })
        }
        fn collapse(&self, range: &Range, endpoint: Endpoint) -> DetectionResult<()> {
            let pos = range.endpoint(endpoint);
            range.endpoints.replace((pos, pos));
            Ok(())
        }
        fn move_character(
            &self,
            range: &Range,
            endpoint: Endpoint,
            count: i32,
        ) -> DetectionResult<i32> {
            if let Some(reason) = self.move_failure {
                return Err(reason);
            }
            let old = range.endpoint(endpoint);
            let next = (old + count).clamp(0, self.document_end);
            let mut pair = range.endpoints.borrow_mut();
            match endpoint {
                Endpoint::Start => {
                    pair.0 = next;
                    pair.1 = pair.1.max(next);
                }
                Endpoint::End => {
                    pair.1 = next;
                    pair.0 = pair.0.min(next);
                }
            }
            Ok(next - old)
        }
        fn rectangles(&self, range: &Range) -> DetectionResult<Vec<Rect>> {
            if self.rectangles_missing {
                return Err(Reason::UiaFailure);
            }
            let (a, b) = *range.endpoints.borrow();
            if a == b {
                return Ok(vec![]);
            }
            Ok(vec![Rect {
                x: f64::from(a * 10)
                    + if range.mouse {
                        0.0
                    } else {
                        self.selection_rectangle_shift
                    },
                y: 0.0,
                width: f64::from((b - a) * 10),
                height: 20.0,
            }])
        }
    }

    fn gesture() -> MouseGesture {
        MouseGesture {
            down: Point { x: 100, y: 10 },
            up: Point { x: 300, y: 10 },
            down_ms: 10,
            up_ms: 20,
            target_window: 42,
            target_pid: 100,
            kind: GestureKind::Drag,
        }
    }

    #[test]
    fn normal_selection_has_complete_evidence_without_mutating_ranges() {
        let detection = SelectionOffsetDetector::analyze(&MockUia::selection(10, 30), gesture());
        let h = detection.health;
        assert_eq!(h.status, SelectionHealth::Normal);
        assert_eq!(h.start_endpoint_valid, Some(true));
        assert_eq!(h.end_endpoint_valid, Some(true));
        assert_eq!(h.selection_direction, Direction::Forward);
        assert!(h.stable);
        assert!((h.confidence - 1.0).abs() < 0.0001);
        assert_eq!(*detection.selection.unwrap().endpoints.borrow(), (10, 30));
        assert!(!h.requires_clipboard());
    }

    #[test]
    fn overall_offset_in_either_direction_rejects_all_native_routes() {
        for delta in [-4, 4] {
            let h = SelectionOffsetDetector::analyze(
                &MockUia::selection(10 + delta, 30 + delta),
                gesture(),
            )
            .health;
            assert_eq!(h.status, SelectionHealth::OffsetDetected);
            assert_eq!(h.reason, Reason::BothEndpointsMismatch);
            assert!(h.requires_clipboard());
        }
    }

    #[test]
    fn character_tolerance_is_inclusive_and_compare_is_not_a_distance() {
        for start_delta in -2..=2 {
            for end_delta in -2..=2 {
                let h = SelectionOffsetDetector::analyze(
                    &MockUia::selection(10 + start_delta, 30 + end_delta),
                    gesture(),
                )
                .health;
                assert_eq!(
                    h.status,
                    SelectionHealth::Normal,
                    "{start_delta}, {end_delta}"
                );
            }
        }
    }

    #[test]
    fn one_bad_endpoint_is_applicable_suspicious_and_uses_fallback() {
        for range in [(6, 30), (10, 34)] {
            let h =
                SelectionOffsetDetector::analyze(&MockUia::selection(range.0, range.1), gesture())
                    .health;
            assert_eq!(h.status, SelectionHealth::Suspicious);
            assert_eq!(h.reason, Reason::OneEndpointMismatch);
            assert!(h.requires_clipboard());
        }
    }

    #[test]
    fn reverse_multiline_and_rtl_use_text_order_instead_of_screen_order() {
        let mut mock = MockUia::selection(10, 30);
        mock.down = 30;
        mock.up = 10;
        mock.rectangles_missing = true;
        let mut g = gesture();
        g.down.y = 200;
        g.up.y = 5;
        let h = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(h.status, SelectionHealth::Normal);
        assert_eq!(h.selection_direction, Direction::Reverse);
        mock.down = 10;
        mock.up = 30;
        g.down.y = 5;
        g.up.y = 200;
        let h = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(h.selection_direction, Direction::Forward);
        assert_eq!(h.status, SelectionHealth::Normal);
    }

    #[test]
    fn document_boundaries_allow_shorter_character_moves() {
        let mut mock = MockUia::selection(0, 100);
        mock.down = 0;
        mock.up = 100;
        mock.rectangles_missing = true;
        assert_eq!(
            SelectionOffsetDetector::analyze(&mock, gesture())
                .health
                .status,
            SelectionHealth::Normal
        );
    }

    #[test]
    fn geometry_contradiction_is_suspicious_but_missing_geometry_is_advisory() {
        let mut mock = MockUia::selection(10, 30);
        mock.selection_rectangle_shift = 1000.0;
        let h = SelectionOffsetDetector::analyze(&mock, gesture()).health;
        assert_eq!(h.reason, Reason::GeometryMismatch);
        assert!(h.requires_clipboard());
        mock.rectangles_missing = true;
        let h = SelectionOffsetDetector::analyze(&mock, gesture()).health;
        assert_eq!(h.status, SelectionHealth::Normal);
        assert_eq!(h.start_geometry_valid, None);
        assert_eq!(h.start_distance, None);
        assert!((h.confidence - 0.7).abs() < 0.0001);
    }

    #[test]
    fn whitespace_outside_text_is_not_a_geometry_ground_truth() {
        let mut g = gesture();
        g.down.y = 500;
        g.up.y = 500;
        let h = SelectionOffsetDetector::analyze(&MockUia::selection(10, 30), g).health;
        assert_eq!(h.status, SelectionHealth::Normal);
        assert_eq!(h.start_geometry_valid, None);
        assert!(h.start_distance.unwrap() > 400.0);
    }

    #[test]
    fn multiple_selection_uses_suspicious_policy() {
        let mut mock = MockUia::selection(10, 30);
        mock.samples = vec![vec![(10, 20), (25, 30)]];
        let h = SelectionOffsetDetector::analyze(&mock, gesture()).health;
        assert_eq!(h.reason, Reason::MultipleSelections);
        assert!(h.requires_clipboard());
    }

    #[test]
    fn empty_and_failed_uia_continue_existing_provider_order() {
        let mut mock = MockUia::selection(10, 30);
        mock.samples = vec![vec![]];
        let h = SelectionOffsetDetector::analyze(&mock, gesture()).health;
        assert_eq!(h.reason, Reason::EmptySelection);
        assert!(!h.requires_clipboard());
        mock.failure = Some(Reason::UiaFailure);
        let h = SelectionOffsetDetector::analyze(&mock, gesture()).health;
        assert_eq!(h.status, SelectionHealth::UiaUnavailable);
        assert!(!h.requires_clipboard());
    }

    #[test]
    fn delayed_provider_is_sampled_until_two_consecutive_ranges_agree() {
        let mut mock = MockUia::selection(10, 30);
        mock.samples = vec![vec![], vec![(10, 30)], vec![(10, 30)]];
        let h = SelectionOffsetDetector::analyze(&mock, gesture()).health;
        assert_eq!(h.status, SelectionHealth::Normal);
        assert!(h.stable);
        assert!(mock.checkpoints.borrow().contains(&56));
    }

    #[test]
    fn unstable_selection_cannot_be_accepted_as_normal() {
        let mut mock = MockUia::selection(10, 30);
        mock.samples = vec![vec![(1, 2)], vec![(3, 4)], vec![(10, 30)]];
        let h = SelectionOffsetDetector::analyze(&mock, gesture()).health;
        assert_eq!(h.reason, Reason::SelectionUnstable);
        assert!(h.requires_clipboard());
    }

    #[test]
    fn cancellation_and_deadline_abort_instead_of_injecting_copy() {
        for reason in [
            Reason::ContextChanged,
            Reason::BudgetExceeded,
            Reason::DpiUnavailable,
        ] {
            let mut mock = MockUia::selection(10, 30);
            mock.stop_at = Some((28, reason));
            let h = SelectionOffsetDetector::analyze(&mock, gesture()).health;
            assert_eq!(h.reason, reason);
            assert!(!h.requires_clipboard());
        }
    }

    #[test]
    fn shift_click_and_plain_click_do_not_use_strict_drag_policy() {
        for kind in [GestureKind::ShiftClick, GestureKind::Click] {
            let mut g = gesture();
            g.kind = kind;
            let mock = MockUia::selection(0, 50);
            let h = SelectionOffsetDetector::analyze(&mock, g).health;
            assert!(!h.applicable);
            assert!(!h.requires_clipboard());
            assert_eq!(mock.sample.get(), 0);
        }
    }

    fn multi_click(point: Point) -> MouseGesture {
        MouseGesture {
            down: point,
            up: point,
            kind: GestureKind::MultiClick,
            ..gesture()
        }
    }

    #[test]
    fn double_click_inside_a_word_is_normal_without_drag_endpoint_alignment() {
        let mut mock = MockUia::selection(10, 30);
        mock.down = 20;
        mock.up = 20;
        mock.mouse_down_x = 200;
        let detection =
            SelectionOffsetDetector::analyze(&mock, multi_click(Point { x: 200, y: 10 }));
        let h = detection.health;
        assert_eq!(h.status, SelectionHealth::Normal);
        assert_eq!(h.reason, Reason::ClickMatchesSelection);
        assert!(h.applicable && h.stable);
        assert_eq!(h.route(), SelectionRoute::ExistingProviders);
        assert_eq!(h.start_endpoint_valid, None);
        assert_eq!(h.end_endpoint_valid, None);
        assert!((h.confidence - 1.0).abs() < 0.0001);
        let click = h.click_validation.unwrap();
        assert!(click.down.in_selection && click.up.in_selection);
        assert_eq!(click.down.geometry_valid, Some(true));
        assert_eq!(click.down.distance, Some(0.0));
        assert_eq!(*detection.selection.unwrap().endpoints.borrow(), (10, 30));
    }

    #[test]
    fn zotero_double_click_mapping_to_unrelated_text_uses_guarded_copy() {
        let mut mock = MockUia::selection(40, 49);
        mock.down = 0;
        mock.up = 0;
        mock.mouse_down_x = 1345;
        let g = multi_click(Point { x: 1345, y: 411 });
        let h = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(h.status, SelectionHealth::Suspicious);
        assert_eq!(h.reason, Reason::ClickOutsideSelection);
        assert_eq!(h.route(), SelectionRoute::GuardedClipboard);
        assert!(h.stable);
        let click = h.click_validation.unwrap();
        assert!(!click.down.in_selection && !click.up.in_selection);
        let h = SelectionOffsetDetector::analyze_snapshot(&mock, &Range::new(40, 49, false), g);
        assert_eq!(h.route(), SelectionRoute::GuardedClipboard);
    }

    #[test]
    fn triple_click_paragraph_and_multi_click_drag_validate_containment() {
        let mut mock = MockUia::selection(0, 100);
        mock.rectangles_missing = true;
        mock.down = 50;
        mock.up = 50;
        let g = multi_click(Point { x: 100, y: 10 });
        let h = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(h.status, SelectionHealth::Normal);
        assert_eq!(h.reason, Reason::ClickMatchesSelection);
        assert_eq!(h.click_validation.unwrap().down.geometry_valid, None);
        for reverse in [false, true] {
            mock.down = if reverse { 80 } else { 20 };
            mock.up = if reverse { 20 } else { 80 };
            let g = MouseGesture {
                up: Point { x: 300, y: 100 },
                ..g
            };
            let h = SelectionOffsetDetector::analyze(&mock, g).health;
            assert_eq!(h.status, SelectionHealth::Normal);
            let click = h.click_validation.unwrap();
            assert!(click.down.in_selection && click.up.in_selection);
        }
        mock.up = 100;
        let g = MouseGesture {
            up: Point { x: 300, y: 100 },
            ..g
        };
        let h = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(h.status, SelectionHealth::Normal);
    }

    #[test]
    fn multi_click_containment_allows_two_character_units_and_document_edges() {
        for (range, positions) in [
            ((10, 30), vec![8, 9, 10, 20, 30, 31, 32]),
            ((0, 100), vec![0, 100]),
        ] {
            for position in positions {
                let mut mock = MockUia::selection(range.0, range.1);
                mock.down = position;
                mock.up = position;
                mock.rectangles_missing = true;
                let h =
                    SelectionOffsetDetector::analyze(&mock, multi_click(Point { x: 100, y: 10 }))
                        .health;
                assert_eq!(h.status, SelectionHealth::Normal, "{range:?} {position}");
            }
        }
        for position in [7, 33] {
            let mut mock = MockUia::selection(10, 30);
            mock.down = position;
            mock.up = position;
            mock.rectangles_missing = true;
            let h = SelectionOffsetDetector::analyze(&mock, multi_click(Point { x: 100, y: 10 }))
                .health;
            assert_eq!(h.reason, Reason::ClickOutsideSelection);
            assert_eq!(h.route(), SelectionRoute::GuardedClipboard);
        }
    }

    #[test]
    fn multi_click_drag_checks_both_anchors() {
        for (down, up) in [(0, 20), (20, 40)] {
            let mut mock = MockUia::selection(10, 30);
            mock.down = down;
            mock.up = up;
            mock.rectangles_missing = true;
            let g = MouseGesture {
                kind: GestureKind::MultiClick,
                ..gesture()
            };
            let h = SelectionOffsetDetector::analyze(&mock, g).health;
            assert_eq!(h.reason, Reason::ClickOutsideSelection);
            assert_eq!(h.route(), SelectionRoute::GuardedClipboard);
            let click = h.click_validation.unwrap();
            assert_eq!(click.down.in_selection, down == 20);
            assert_eq!(click.up.in_selection, up == 20);
        }
    }

    #[test]
    fn multi_click_geometry_contradictions_use_guarded_copy_but_whitespace_is_unknown() {
        let mut mock = MockUia::selection(10, 30);
        mock.down = 20;
        mock.up = 20;
        mock.mouse_down_x = 200;
        mock.selection_rectangle_shift = 1000.0;
        let g = multi_click(Point { x: 200, y: 10 });
        let h = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(h.reason, Reason::GeometryMismatch);
        assert_eq!(h.route(), SelectionRoute::GuardedClipboard);
        assert_eq!(h.click_validation.unwrap().down.geometry_valid, Some(false));
        let h =
            SelectionOffsetDetector::analyze(&mock, multi_click(Point { x: 200, y: 500 })).health;
        assert_eq!(h.status, SelectionHealth::Normal);
        assert_eq!(h.click_validation.unwrap().down.geometry_valid, None);
    }

    #[test]
    fn multi_click_unverifiable_mapping_with_selected_text_uses_guarded_copy() {
        let mut mock = MockUia::selection(10, 30);
        mock.point_failure = Some(Reason::UiaFailure);
        let g = multi_click(Point { x: 100, y: 10 });
        let h = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(h.reason, Reason::ClickMappingUnavailable);
        assert_eq!(h.route(), SelectionRoute::GuardedClipboard);
        mock.point_failure = None;
        mock.move_failure = Some(Reason::UiaFailure);
        let h = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(h.reason, Reason::ClickMappingUnavailable);
        assert_eq!(h.route(), SelectionRoute::GuardedClipboard);
    }

    #[test]
    fn multi_click_context_and_deadline_errors_never_promote_to_copy() {
        let g = multi_click(Point { x: 100, y: 10 });
        for reason in [
            Reason::ContextChanged,
            Reason::BudgetExceeded,
            Reason::DpiUnavailable,
            Reason::Protected,
        ] {
            let mut mock = MockUia::selection(10, 30);
            mock.point_failure = Some(reason);
            let h = SelectionOffsetDetector::analyze(&mock, g).health;
            assert_eq!(h.reason, reason);
            assert!(!h.requires_clipboard());
        }
    }

    #[test]
    fn multi_click_empty_selection_or_unavailable_text_does_not_force_copy() {
        let g = multi_click(Point { x: 100, y: 10 });
        let mut mock = MockUia::selection(20, 20);
        let h = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(h.reason, Reason::EmptySelection);
        assert!(!h.requires_clipboard());
        mock = MockUia::selection(10, 30);
        mock.text_failure = Some(Reason::UiaFailure);
        let h = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(h.reason, Reason::UiaFailure);
        assert!(!h.requires_clipboard());
    }

    #[test]
    fn multi_click_delayed_unstable_and_multiple_selections_use_shared_policy() {
        let g = multi_click(Point { x: 200, y: 10 });
        let mut mock = MockUia::selection(10, 30);
        mock.down = 20;
        mock.up = 20;
        mock.mouse_down_x = 200;
        mock.samples = vec![vec![], vec![(10, 30)], vec![(10, 30)]];
        let h = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(h.status, SelectionHealth::Normal);
        assert!(h.stable);
        assert!(mock.checkpoints.borrow().contains(&56));
        mock.samples = vec![vec![(1, 2)], vec![(3, 4)], vec![(10, 30)]];
        mock.sample.set(0);
        let h = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(h.reason, Reason::SelectionUnstable);
        assert_eq!(h.route(), SelectionRoute::GuardedClipboard);
        mock.samples = vec![vec![(10, 20), (25, 30)]];
        mock.sample.set(0);
        let h = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(h.reason, Reason::MultipleSelections);
        assert_eq!(h.route(), SelectionRoute::GuardedClipboard);
    }

    #[test]
    fn distinct_drag_points_with_equal_uia_mapping_use_guarded_copy() {
        // Reproduce the Zotero report: a 63 px drag, a nine-character selected
        // range, and both mouse coordinates mapping to one insertion point.
        for reverse in [false, true] {
            let mut mock = MockUia::selection(40, 49);
            mock.down = 0;
            mock.up = 0;
            mock.rectangles_missing = true;
            let mut g = gesture();
            g.down = Point { x: 1296, y: 409 };
            g.up = Point { x: 1359, y: 410 };
            if reverse {
                std::mem::swap(&mut g.down, &mut g.up);
            }
            mock.mouse_down_x = g.down.x;
            let detection = SelectionOffsetDetector::analyze(&mock, g);
            let h = detection.health;
            assert_eq!(h.status, SelectionHealth::Suspicious);
            assert_eq!(h.reason, Reason::EqualPointRanges);
            assert!(h.applicable);
            assert!(h.stable);
            assert_eq!(h.route(), SelectionRoute::GuardedClipboard);
            assert_eq!(h.confidence, 0.0);
            assert_eq!(h.selection_direction, Direction::Unknown);
            assert_eq!(h.start_endpoint_valid, None);
            assert_eq!(h.end_endpoint_valid, None);
            assert_eq!(*detection.selection.unwrap().endpoints.borrow(), (40, 49));
        }
    }

    #[test]
    fn candidate_with_equal_uia_mapping_uses_the_same_guarded_copy_policy() {
        let mut mock = MockUia::selection(40, 49);
        mock.up = mock.down;
        let h =
            SelectionOffsetDetector::analyze_snapshot(&mock, &Range::new(40, 49, false), gesture());
        assert_eq!(h.reason, Reason::EqualPointRanges);
        assert_eq!(h.route(), SelectionRoute::GuardedClipboard);
    }

    #[test]
    fn identical_mouse_coordinates_skip_strict_drag_detection() {
        // Defend against an inconsistent Drag label as well as plain clicks.
        let mock = MockUia::selection(40, 49);
        let mut g = gesture();
        g.up = g.down;
        let h = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(h.reason, Reason::EqualPointRanges);
        assert!(!h.applicable);
        assert_eq!(h.route(), SelectionRoute::ExistingProviders);
        let h = SelectionOffsetDetector::analyze_snapshot(&mock, &Range::new(40, 49, false), g);
        assert!(!h.applicable);
        assert_eq!(h.route(), SelectionRoute::ExistingProviders);
        assert_eq!(mock.sample.get(), 0);
        assert!(mock.checkpoints.borrow().is_empty());
    }

    #[test]
    fn collapsed_selection_never_forces_copy_regardless_of_point_mapping() {
        for equal_mapping in [false, true] {
            let mut mock = MockUia::selection(10, 10);
            if equal_mapping {
                mock.up = mock.down;
            }
            let h = SelectionOffsetDetector::analyze(&mock, gesture()).health;
            assert_eq!(h.status, SelectionHealth::UiaUnavailable);
            assert_eq!(h.reason, Reason::EmptySelection);
            assert!(!h.applicable);
            assert_eq!(h.route(), SelectionRoute::ExistingProviders);
        }
    }

    #[test]
    fn failed_text_probe_does_not_force_copy_for_equal_point_mapping() {
        let mut mock = MockUia::selection(40, 49);
        mock.up = mock.down;
        mock.text_failure = Some(Reason::UiaFailure);
        let h = SelectionOffsetDetector::analyze(&mock, gesture()).health;
        assert_eq!(h.reason, Reason::UiaFailure);
        assert_eq!(h.status, SelectionHealth::UiaUnavailable);
        assert_eq!(h.route(), SelectionRoute::ExistingProviders);
    }

    #[test]
    fn equal_point_mapping_does_not_bypass_cancellation_or_deadline() {
        for reason in [
            Reason::ContextChanged,
            Reason::BudgetExceeded,
            Reason::DpiUnavailable,
        ] {
            let mut mock = MockUia::selection(40, 49);
            mock.up = mock.down;
            mock.stop_at = Some((28, reason));
            let h = SelectionOffsetDetector::analyze(&mock, gesture()).health;
            assert_eq!(h.reason, reason);
            assert!(!h.requires_clipboard());
        }
    }

    fn tracked_drag() -> MouseSelectionTracker {
        let mut tracker = MouseSelectionTracker::default();
        let g = gesture();
        tracker.down(g.down, 10, 42, 100, false, 500, (2, 2));
        tracker.up(g.up, 20, 42, 100);
        tracker
    }

    #[test]
    fn tracker_preserves_drag_across_target_activation_and_manual_trigger_reads() {
        let mut tracker = MouseSelectionTracker::default();
        let g = gesture();
        tracker.down(g.down, 10, 42, 100, false, 500, (2, 2));
        tracker.foreground(42, 100);
        tracker.up(g.up, 20, 42, 100);
        assert_eq!(tracker.context(21, 42, 100), Some(g));
        assert_eq!(tracker.context(100, 42, 100), Some(g));
        assert!(tracker.context(100, 43, 100).is_none());
        assert!(tracker.context(100, 42, 101).is_none());
    }

    #[test]
    fn tracker_expiry_new_press_cross_window_and_invalidations_remove_old_anchors() {
        let mut tracker = tracked_drag();
        assert!(tracker.context(60_020, 42, 100).is_some());
        assert!(tracker.context(60_021, 42, 100).is_none());
        assert!(tracker.context(19, 42, 100).is_none());
        tracker.down(Point { x: 100, y: 10 }, 30, 42, 100, false, 500, (2, 2));
        assert!(tracker.context(31, 42, 100).is_none());
        tracker.up(Point { x: 300, y: 10 }, 40, 43, 101);
        assert!(tracker.context(41, 43, 101).is_none());
        for _ in 0..3 {
            tracker = tracked_drag();
            tracker.invalidate();
            assert!(tracker.context(30, 42, 100).is_none());
        }
        tracker = tracked_drag();
        tracker.foreground(43, 100);
        assert!(tracker.context(30, 42, 100).is_none());
    }

    #[test]
    fn tracker_identifies_double_and_triple_clicks_and_shift_drag() {
        let mut tracker = MouseSelectionTracker::default();
        let point = Point { x: 100, y: 10 };
        for (time, expected) in [
            (10, GestureKind::Click),
            (100, GestureKind::MultiClick),
            (200, GestureKind::MultiClick),
        ] {
            tracker.down(point, time, 42, 100, false, 500, (2, 2));
            tracker.up(point, time + 1, 42, 100);
            assert_eq!(tracker.context(time + 2, 42, 100).unwrap().kind, expected);
        }
        tracker.down(point, 1000, 42, 100, true, 500, (2, 2));
        tracker.up(Point { x: 300, y: 10 }, 1001, 42, 100);
        assert_eq!(
            tracker.context(1002, 42, 100).unwrap().kind,
            GestureKind::ShiftClick
        );
    }

    #[test]
    fn rectangle_distance_supports_negative_monitors_and_rejects_invalid_values() {
        let rect = Rect {
            x: -100.0,
            y: -20.0,
            width: 50.0,
            height: 20.0,
        };
        assert!(rect.valid());
        assert_eq!(rect.distance(Point { x: -75, y: -10 }), 0.0);
        assert_eq!(rect.distance(Point { x: -40, y: 0 }), 10.0);
        assert!(!Rect {
            height: f64::NAN,
            ..rect
        }
        .valid());
        assert!(!Rect { width: 0.0, ..rect }.valid());
    }
    #[test]
    fn capture_gate_bypasses_legacy_and_msaa_for_reliable_anomalies() {
        for status in [SelectionHealth::OffsetDetected, SelectionHealth::Suspicious] {
            let mut health = SelectionHealthResult::suspicious(Reason::OneEndpointMismatch, true);
            health.status = status;
            assert_eq!(health.route(), SelectionRoute::GuardedClipboard);
            health.applicable = false;
            assert_eq!(health.route(), SelectionRoute::ExistingProviders);
        }
        for reason in [Reason::Protected, Reason::ContextChanged] {
            assert_eq!(
                SelectionHealthResult::unavailable(reason).route(),
                SelectionRoute::RejectCapture
            );
        }
        assert_eq!(
            SelectionHealthResult::unavailable(Reason::NoMouseContext).route(),
            SelectionRoute::ExistingProviders
        );
        assert_eq!(
            SelectionHealthResult::unavailable(Reason::UiaFailure).route(),
            SelectionRoute::ExistingProviders
        );
    }

    #[test]
    fn empty_sample_breaks_consecutive_stability() {
        let mut mock = MockUia::selection(10, 30);
        mock.samples = vec![vec![(10, 30)], vec![], vec![(10, 30)]];
        let health = SelectionOffsetDetector::analyze(&mock, gesture()).health;
        assert_eq!(health.reason, Reason::SelectionUnstable);
        assert!(health.requires_clipboard());
    }
    #[test]
    fn forward_multiline_flow_can_start_to_the_right_of_release() {
        let mut mock = MockUia::selection(10, 30);
        mock.rectangles_missing = true;
        mock.mouse_down_x = 800;
        let mut g = gesture();
        g.down = Point { x: 800, y: 10 };
        g.up = Point { x: 400, y: 100 };
        let health = SelectionOffsetDetector::analyze(&mock, g).health;
        assert_eq!(health.selection_direction, Direction::Forward);
        assert_eq!(health.status, SelectionHealth::Normal);
    }

    #[test]
    fn range_from_point_and_character_move_failures_are_unavailable() {
        let mut mock = MockUia::selection(10, 30);
        mock.point_failure = Some(Reason::UiaFailure);
        let health = SelectionOffsetDetector::analyze(&mock, gesture()).health;
        assert_eq!(health.status, SelectionHealth::UiaUnavailable);
        assert!(!health.requires_clipboard());
        mock.point_failure = None;
        mock.move_failure = Some(Reason::UiaFailure);
        let health = SelectionOffsetDetector::analyze(&mock, gesture()).health;
        assert_eq!(health.status, SelectionHealth::UiaUnavailable);
        assert!(!health.requires_clipboard());
    }
}
