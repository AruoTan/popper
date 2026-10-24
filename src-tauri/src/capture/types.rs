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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturePoint {
    pub x: i32,
    pub y: i32,
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
    pub click_through: bool,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PinUpdate {
    pub zoom: f64,
    pub opacity: f64,
    pub click_through: bool,
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
