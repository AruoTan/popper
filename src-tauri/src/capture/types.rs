use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl CaptureRect {
    pub fn contains(self, x: i32, y: i32) -> bool {
        i64::from(x) >= i64::from(self.x)
            && i64::from(y) >= i64::from(self.y)
            && i64::from(x) < i64::from(self.x) + i64::from(self.width)
            && i64::from(y) < i64::from(self.y) + i64::from(self.height)
    }
    pub fn valid_within(self, width: u32, height: u32) -> bool {
        self.x >= 0
            && self.y >= 0
            && self.width > 0
            && self.height > 0
            && u64::from(self.x as u32) + u64::from(self.width) <= u64::from(width)
            && u64::from(self.y as u32) + u64::from(self.height) <= u64::from(height)
    }
    pub fn intersect(self, other: Self) -> Option<Self> {
        let x = i64::from(self.x).max(i64::from(other.x));
        let y = i64::from(self.y).max(i64::from(other.y));
        let right = (i64::from(self.x) + i64::from(self.width))
            .min(i64::from(other.x) + i64::from(other.width));
        let bottom = (i64::from(self.y) + i64::from(self.height))
            .min(i64::from(other.y) + i64::from(other.height));
        (right > x && bottom > y).then_some(Self {
            x: x as i32,
            y: y as i32,
            width: (right - x).max(0) as u32,
            height: (bottom - y).max(0) as u32,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePoint {
    pub x: i32,
    pub y: i32,
}
impl CapturePoint {
    pub fn step_within(self, bounds: CaptureRect, dx: i32, dy: i32) -> Result<Self, String> {
        if u64::from(dx.unsigned_abs()) + u64::from(dy.unsigned_abs()) != 1 {
            return Err("鼠标每次只能沿一个方向移动一个像素".into());
        }
        let right = i64::from(bounds.x) + i64::from(bounds.width) - 1;
        let bottom = i64::from(bounds.y) + i64::from(bounds.height) - 1;
        if bounds.width == 0
            || bounds.height == 0
            || right > i64::from(i32::MAX)
            || bottom > i64::from(i32::MAX)
        {
            return Err("屏幕边界无效".into());
        }
        Ok(Self {
            x: (i64::from(self.x) + i64::from(dx)).clamp(i64::from(bounds.x), right) as i32,
            y: (i64::from(self.y) + i64::from(dy)).clamp(i64::from(bounds.y), bottom) as i32,
        })
    }
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSession {
    pub id: String,
    pub screen: CaptureRect,
    pub scale: f64,
    pub color: String,
    pub line_width: u8,
    pub targets: Vec<CaptureTarget>,
    pub pointer: Option<CapturePoint>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDocument {
    pub width: u32,
    pub height: u32,
    pub version: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScrollStatus {
    Appended,
    Unchanged,
    Unmatched,
    Tracked,
    Limit,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrollProgress {
    pub width: u32,
    pub height: u32,
    pub revision: u64,
    pub viewport: CaptureRect,
    pub status: ScrollStatus,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrollLayout {
    pub selection: CaptureRect,
    pub panel: CaptureRect,
    pub toolbar: CaptureRect,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrollCaptureSession {
    pub progress: ScrollProgress,
    pub layout: ScrollLayout,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureTarget {
    pub rect: CaptureRect,
    pub label: String,
    pub controls: Vec<CaptureCandidate>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureCandidate {
    pub rect: CaptureRect,
    pub label: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureHitTest {
    pub candidates: Vec<CaptureCandidate>,
    pub pending: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinnedImageState {
    pub id: String,
    pub width: u32,
    pub height: u32,
    pub zoom: f64,
    pub opacity: f64,
    pub shadow_inset: u32,
    pub toolbar_visible: bool,
    pub annotations_visible: bool,
    pub always_on_top: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinToolbarState {
    pub tool: CaptureTool,
    pub color: String,
    pub line_width: u8,
    pub enabled: bool,
    pub busy: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub can_delete: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum CaptureTool {
    Select,
    Rectangle,
    Ellipse,
    Arrow,
    Pen,
    Text,
    Mosaic,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum PinToolbarAction {
    Tool { tool: CaptureTool },
    Color { color: String },
    Width { width: u8 },
    Undo,
    Redo,
    Delete,
    Copy,
    Save,
    Hide,
    Close { destroy: bool },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureAnnotation {
    pub id: String,
    pub tool: AnnotationTool,
    pub points: Vec<AnnotationPoint>,
    pub color: String,
    pub width: f64,
    pub text: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnnotationTool {
    Rectangle,
    Ellipse,
    Arrow,
    Pen,
    Text,
    Mosaic,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnotationPoint {
    pub x: f64,
    pub y: f64,
}

pub fn validate_annotations(items: &[CaptureAnnotation]) -> Result<(), String> {
    let points: usize = items.iter().map(|item| item.points.len()).sum();
    if items.len() > 2048
        || points > 262144
        || items.iter().any(|item| {
            item.id.len() > 128
                || item.points.is_empty()
                || item.color.len() != 7
                || !item.color.starts_with('#')
                || !item.color[1..].bytes().all(|c| c.is_ascii_hexdigit())
                || !item.width.is_finite()
                || !(1.0..=32.0).contains(&item.width)
                || item.points.iter().any(|point| {
                    !point.x.is_finite()
                        || !point.y.is_finite()
                        || point.x.abs() > 1e7
                        || point.y.abs() > 1e7
                })
                || item
                    .text
                    .as_ref()
                    .is_some_and(|text| text.chars().count() > 4000)
        })
    {
        return Err("标注数据无效或过大".into());
    }
    Ok(())
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinUpdate {
    pub zoom: f64,
    pub opacity: f64,
    pub anchor_x: f64,
    pub anchor_y: f64,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingAttachment {
    pub id: String,
    pub attachment: crate::quick_ask::Attachment,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn annotation_layers_preserve_fractional_coordinates_and_reject_invalid_payloads() {
        let value = serde_json::json!({"id":"shape", "tool":"rectangle", "points":[{"x":-12.5,"y":30.25},{"x":300.5,"y":140.75}], "color":"#ff4545", "width":3.0});
        let item: CaptureAnnotation = serde_json::from_value(value.clone()).unwrap();
        validate_annotations(&[item.clone()]).unwrap();
        assert_eq!(item.points[0].x, -12.5);
        for patch in [
            serde_json::json!({"tool":"select"}),
            serde_json::json!({"points":[]}),
            serde_json::json!({"width":0}),
            serde_json::json!({"color":"invalid"}),
            serde_json::json!({"points":[{"x":1e20,"y":0}]}),
        ] {
            let mut invalid = value.clone();
            for (key, value) in patch.as_object().unwrap() {
                invalid[key] = value.clone();
            }
            assert!(serde_json::from_value::<CaptureAnnotation>(invalid)
                .map(|item| validate_annotations(&[item]).is_err())
                .unwrap_or(true));
        }
        assert!(validate_annotations(&vec![item; 2049]).is_err());
    }
    #[test]
    fn drag_anchor_decodes_nullable_signed_pixel_coordinates() {
        let empty = serde_json::from_str::<Option<CapturePoint>>("null").unwrap();
        assert!(empty.is_none());
        for (x, y) in [(124, 84), (-1920, -1080), (i32::MIN, i32::MAX)] {
            let value = serde_json::json!({ "x": x, "y": y });
            let point = serde_json::from_value::<Option<CapturePoint>>(value.clone())
                .unwrap()
                .unwrap();
            assert_eq!((point.x, point.y), (x, y));
            assert_eq!(serde_json::to_value(point).unwrap(), value);
        }
        for value in [
            serde_json::json!({ "x": 1 }),
            serde_json::json!({ "x": "124", "y": 84 }),
            serde_json::json!({ "x": 1.5, "y": 84 }),
            serde_json::json!({ "x": 2147483648_i64, "y": 84 }),
        ] {
            assert!(serde_json::from_value::<Option<CapturePoint>>(value).is_err());
        }
    }
    #[test]
    fn pointer_steps_use_physical_pixels_on_negative_origin_screens() {
        let bounds = CaptureRect {
            x: -1920,
            y: -1080,
            width: 1920,
            height: 1080,
        };
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let point = CapturePoint { x: -960, y: -540 }
                .step_within(bounds, dx, dy)
                .unwrap();
            assert_eq!((point.x, point.y), (-960 + dx, -540 + dy));
        }
        for (x, y, dx, dy) in [
            (-1920, -1080, -1, 0),
            (-1920, -1080, 0, -1),
            (-1, -1, 1, 0),
            (-1, -1, 0, 1),
        ] {
            let point = CapturePoint { x, y }.step_within(bounds, dx, dy).unwrap();
            assert_eq!((point.x, point.y), (x, y));
        }
        let point = CapturePoint {
            x: i32::MAX,
            y: i32::MIN,
        }
        .step_within(bounds, 1, 0)
        .unwrap();
        assert_eq!((point.x, point.y), (-1, -1080));
        for (dx, dy) in [(0, 0), (1, 1), (2, 0), (i32::MIN, 0)] {
            assert!(CapturePoint { x: -960, y: -540 }
                .step_within(bounds, dx, dy)
                .is_err());
        }
        for bounds in [
            CaptureRect::default(),
            CaptureRect {
                x: i32::MAX,
                y: 0,
                width: 2,
                height: 1,
            },
        ] {
            assert!(CapturePoint { x: 0, y: 0 }
                .step_within(bounds, 1, 0)
                .is_err());
        }
    }
    #[test]
    fn image_rectangles_reject_empty_negative_and_overflow() {
        assert!(CaptureRect {
            x: 0,
            y: 0,
            width: 3840,
            height: 2160
        }
        .valid_within(3840, 2160));
        for rect in [
            CaptureRect {
                x: -1,
                y: 0,
                width: 1,
                height: 1,
            },
            CaptureRect {
                x: 1,
                y: 0,
                width: u32::MAX,
                height: 1,
            },
            CaptureRect::default(),
        ] {
            assert!(!rect.valid_within(3840, 2160));
        }
    }
    #[test]
    fn global_negative_screen_rectangles_clip_without_changing_origin() {
        let screen = CaptureRect {
            x: -1920,
            y: -200,
            width: 1920,
            height: 1080,
        };
        let window = CaptureRect {
            x: -2000,
            y: -300,
            width: 400,
            height: 400,
        };
        assert_eq!(
            window.intersect(screen),
            Some(CaptureRect {
                x: -1920,
                y: -200,
                width: 320,
                height: 300
            })
        );
        assert!(screen.contains(-1, 0));
        assert!(!screen.contains(0, 0));
    }
}
