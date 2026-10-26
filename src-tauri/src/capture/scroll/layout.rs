//! Physical-pixel placement shared by the native cutout and its visual panel.
use super::super::{CaptureRect, ScrollLayout};

pub fn layout(width: u32, height: u32, scale: f64, selection: CaptureRect) -> ScrollLayout {
    let margin = (12.0 * scale).round() as u32;
    let gap = (16.0 * scale).round() as i32;
    let toolbar_width =
        ((272.0 * scale).round() as u32).min(width.saturating_sub(margin * 2).max(1));
    let toolbar_height =
        ((48.0 * scale).round() as u32).min(height.saturating_sub(margin * 2).max(1));
    let toolbar_bottom = height.saturating_sub(toolbar_height + margin) as i32;
    let below = selection.y + selection.height as i32 + gap / 2;
    let toolbar = CaptureRect {
        x: (selection.x + selection.width as i32 - toolbar_width as i32)
            .clamp(0, width.saturating_sub(toolbar_width + margin) as i32),
        y: if below <= toolbar_bottom {
            below
        } else {
            selection.y + selection.height as i32 - toolbar_height as i32 - gap / 2
        }
        .clamp(0, toolbar_bottom),
        width: toolbar_width,
        height: toolbar_height,
    };
    let panel_width = ((216.0 * scale).round() as u32).min(width.max(1));
    let selection_right = selection.x + selection.width as i32;
    let x = if selection_right + gap + panel_width as i32 <= width as i32 {
        selection_right + gap
    } else if selection.x >= gap + panel_width as i32 {
        selection.x - gap - panel_width as i32
    } else {
        width.saturating_sub(panel_width) as i32
    };
    // Reserve the vertical growth path so the preview reaches the screen top
    // before its aspect ratio is constrained by the available height.
    let panel_height =
        (selection.y + selection.height as i32).clamp(1, height.max(1) as i32) as u32;
    ScrollLayout {
        selection,
        toolbar,
        panel: CaptureRect {
            x,
            y: 0,
            width: panel_width,
            height: panel_height,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prefers_free_space_beside_the_selection() {
        let selected = CaptureRect {
            x: 200,
            y: 100,
            width: 800,
            height: 600,
        };
        let layout = layout(1920, 1080, 1.5, selected);
        assert!(layout.panel.x > selected.x + selected.width as i32);
        assert!(layout.panel.intersect(selected).is_none());
        assert!(layout.toolbar.intersect(layout.panel).is_none());
        assert_eq!(
            layout.toolbar.x + layout.toolbar.width as i32,
            selected.x + selected.width as i32
        );
        assert!(layout.toolbar.y > selected.y + selected.height as i32);
    }
    #[test]
    fn prefers_right_then_left_and_falls_back_to_the_screen_right_edge() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let panel_width = (216.0_f64 * scale).round() as i32;
            let gap = (16.0_f64 * scale).round() as i32;
            for (x, selected_width, expected_x) in [
                (600, 400, 1000 + gap),
                (1600, 300, 1600 - gap - panel_width),
                (100, 1720, 1920 - panel_width),
                (0, 1920, 1920 - panel_width),
            ] {
                let selected = CaptureRect {
                    x,
                    y: 100,
                    width: selected_width,
                    height: 600,
                };
                let panel = layout(1920, 1080, scale, selected).panel;
                assert_eq!(panel.x, expected_x);
                assert_eq!(panel.y, 0);
                assert_eq!(panel.height, 700);
                assert!(panel.valid_within(1920, 1080));
            }
        }
    }

    #[test]
    fn stays_on_screen_for_edge_and_full_screen_selections_at_each_scale() {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            for selection in [
                CaptureRect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
                CaptureRect {
                    x: 1800,
                    y: 900,
                    width: 120,
                    height: 180,
                },
                CaptureRect {
                    x: 0,
                    y: 0,
                    width: 32,
                    height: 96,
                },
            ] {
                let result = layout(1920, 1080, scale, selection);
                assert!(result.panel.valid_within(1920, 1080));
                assert!(result.toolbar.valid_within(1920, 1080));
            }
        }
        assert!(layout(
            160,
            100,
            2.0,
            CaptureRect {
                x: 0,
                y: 0,
                width: 160,
                height: 100
            }
        )
        .panel
        .valid_within(160, 100));
    }
}
