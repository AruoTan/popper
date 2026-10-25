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
    let panel_width = ((216.0 * scale).round() as u32).min(width.saturating_sub(margin * 2).max(1));
    let panel_height =
        ((534.0 * scale).round() as u32).min(height.saturating_sub(margin * 2).max(1));
    let right = width.saturating_sub(panel_width + margin) as i32;
    let bottom = height.saturating_sub(panel_height + margin) as i32;
    let (x, y) = [
        (selection.x + selection.width as i32 + gap, selection.y),
        (selection.x - panel_width as i32 - gap, selection.y),
        (selection.x, selection.y + selection.height as i32 + gap),
        (selection.x, selection.y - panel_height as i32 - gap),
        (right, margin as i32),
        (margin as i32, margin as i32),
        (right, bottom),
        (margin as i32, bottom),
    ]
    .into_iter()
    .map(|(x, y)| (x.clamp(0, right.max(0)), y.clamp(0, bottom.max(0))))
    .min_by_key(|&(x, y)| {
        let selection_overlap = selection
            .intersect(CaptureRect {
                x,
                y,
                width: panel_width,
                height: panel_height,
            })
            .map_or(0, |rect| u64::from(rect.width) * u64::from(rect.height));
        let toolbar_overlap = toolbar
            .intersect(CaptureRect {
                x,
                y,
                width: panel_width,
                height: panel_height,
            })
            .map_or(0, |rect| u64::from(rect.width) * u64::from(rect.height));
        selection_overlap + toolbar_overlap * 4
    })
    .unwrap();
    ScrollLayout {
        selection,
        toolbar,
        panel: CaptureRect {
            x,
            y,
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
