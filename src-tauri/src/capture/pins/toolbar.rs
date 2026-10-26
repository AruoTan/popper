//! Toolbar placement in physical pixels; image coordinates are read-only.
use super::super::{CaptureRect, PinToolbarState};

pub const WIDTH: f64 = 464.0;
pub const HEIGHT: f64 = 49.0;

#[derive(Default)]
pub struct Toolbar {
    pub state: Option<PinToolbarState>,
    pub ready: bool,
    pub creating: bool,
    pub height: f64,
    pub placement: Placement,
    pub topmost: Option<bool>,
    pub sync_pending: bool,
}

#[derive(Default)]
pub struct Placement {
    bottom_locked: bool,
}

impl Placement {
    pub fn layout(
        &mut self,
        image: CaptureRect,
        work: CaptureRect,
        height: f64,
        scale: f64,
    ) -> CaptureRect {
        let margin = (8.0 * scale).round().max(1.0) as i64;
        let gap = (8.0 * scale).round().max(1.0) as i64;
        let width = (WIDTH * scale)
            .round()
            .max(1.0)
            .min((i64::from(work.width) - 2 * margin).max(1) as f64) as u32;
        let height = (height * scale)
            .round()
            .max(1.0)
            .min((i64::from(work.height) - 2 * margin).max(1) as f64) as u32;
        let left = i64::from(work.x) + margin.min(i64::from(work.width.saturating_sub(width)) / 2);
        let top = i64::from(work.y) + margin.min(i64::from(work.height.saturating_sub(height)) / 2);
        let right =
            (i64::from(work.x) + i64::from(work.width) - margin - i64::from(width)).max(left);
        let bottom =
            (i64::from(work.y) + i64::from(work.height) - margin - i64::from(height)).max(top);
        let image_bottom = i64::from(image.y) + i64::from(image.height);
        let work_bottom = i64::from(work.y) + i64::from(work.height);
        let below = image_bottom + gap;
        let above = i64::from(image.y) - gap - i64::from(height);
        let fits_below = below >= top && below <= bottom;
        let fits_above = above >= top && above <= bottom;
        if i64::from(image.y) < i64::from(work.y) && image_bottom > work_bottom {
            self.bottom_locked = true;
        }
        let x = i64::from(image.x).clamp(left, right);
        let (x, y) = if self.bottom_locked {
            // Follow drag distance, rather than running a delayed animation after release.
            if fits_below {
                self.bottom_locked = false;
            }
            (x, below.clamp(top, bottom))
        } else if fits_below {
            (x, below)
        } else if fits_above {
            (x, above)
        } else {
            (x, bottom)
        };
        CaptureRect {
            x: x as i32,
            y: y as i32,
            width,
            height,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const WORK: CaptureRect = CaptureRect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1040,
    };
    fn image(y: i32, height: u32) -> CaptureRect {
        CaptureRect {
            x: 400,
            y,
            width: 800,
            height,
        }
    }
    #[test]
    fn prefers_below_and_uses_above_when_only_top_has_space() {
        for y in [0, 50, 300] {
            let input = image(y, 240);
            let output = Placement::default().layout(input, WORK, HEIGHT, 1.0);
            assert_eq!((output.x, output.y), (400, y + 240 + 8));
            assert_eq!(input, image(y, 240));
        }
        let output = Placement::default().layout(image(800, 240), WORK, HEIGHT, 1.0);
        assert_eq!(output.y, 800 - HEIGHT as i32 - 8);
    }
    #[test]
    fn oversized_images_keep_bottom_docking_when_top_enters_screen() {
        let mut placement = Placement::default();
        let output = placement.layout(image(-400, 1800), WORK, HEIGHT, 1.0);
        assert_eq!((output.x, output.y), (400, 983));
        for y in -399..=300 {
            let output = placement.layout(image(y, 1800), WORK, HEIGHT, 1.0);
            assert_eq!((output.x, output.y), (400, 983));
        }
    }
    #[test]
    fn bottom_recovery_is_continuous_and_driven_only_by_image_movement() {
        let mut placement = Placement::default();
        let mut previous = placement.layout(image(-400, 1800), WORK, HEIGHT, 1.0);
        for y in (-1100..=-400).rev() {
            let input = image(y, 1800);
            let output = placement.layout(input, WORK, HEIGHT, 1.0);
            assert_eq!(output.x, 400);
            assert!((output.y - previous.y).abs() <= 1);
            assert!(output.y >= 8 && output.y + output.height as i32 <= 1032);
            assert_eq!(output, placement.layout(input, WORK, HEIGHT, 1.0));
            assert_eq!(input, image(y, 1800));
            previous = output;
        }
        assert_eq!(previous.y, 708);
    }
    #[test]
    fn normal_bottom_approach_never_pushes_image_upward() {
        let mut placement = Placement::default();
        for y in 650..900 {
            let input = image(y, 240);
            let output = placement.layout(input, WORK, HEIGHT, 1.0);
            assert!(output.y >= 8 && output.y + output.height as i32 <= 1032);
            assert_eq!(input.y, y);
            if y + 240 + 8 <= 983 {
                assert_eq!(output.y, y + 248);
            } else {
                assert_eq!(output.y, y - 57);
            }
        }
    }
    #[test]
    fn clamps_negative_origin_high_dpi_and_narrow_work_areas() {
        for scale in [1.0, 1.25, 1.5, 2.0, 4.0] {
            for work in [
                CaptureRect {
                    x: -1920,
                    y: -1080,
                    width: 1920,
                    height: 1040,
                },
                CaptureRect {
                    x: -300,
                    y: 50,
                    width: 300,
                    height: 300,
                },
            ] {
                for x in [-10000, -1920, -20, 10000] {
                    let mut placement = Placement::default();
                    let output = placement.layout(
                        CaptureRect {
                            x,
                            y: work.y - 800,
                            width: 400,
                            height: 2400,
                        },
                        work,
                        HEIGHT,
                        scale,
                    );
                    assert!(output.x >= work.x && output.y >= work.y);
                    assert!(
                        i64::from(output.x) + i64::from(output.width)
                            <= i64::from(work.x) + i64::from(work.width)
                    );
                    assert!(
                        i64::from(output.y) + i64::from(output.height)
                            <= i64::from(work.y) + i64::from(work.height)
                    );
                }
            }
        }
    }
}
