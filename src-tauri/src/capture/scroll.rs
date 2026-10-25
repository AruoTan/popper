//! A bounded vertical document that can grow above or below its initial viewport.
#[path = "scroll/registration.rs"]
mod registration;
#[path = "scroll/stream.rs"]
pub mod stream;
use super::{ScrollProgress, ScrollStatus};
use image::RgbaImage;
pub use registration::settled;
use registration::{fixed_edges, rows, Index, Row};
use std::{collections::VecDeque, sync::Arc};

const MAX_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_HEIGHT: u32 = 32768;
fn fits(width: u32, height: u64) -> bool {
    width <= MAX_HEIGHT
        && height <= u64::from(MAX_HEIGHT)
        && height * u64::from(width) <= MAX_PIXELS
}

pub struct ScrollStitcher {
    strips: VecDeque<Arc<RgbaImage>>,
    rows: VecDeque<Row>,
    index: Index,
    last: Arc<RgbaImage>,
    top: i64,
    bottom: i64,
    cursor: i64,
    edges: Option<(usize, usize)>,
    header: Option<Arc<RgbaImage>>,
    footer: Option<Arc<RgbaImage>>,
    frames: usize,
}
fn crop(image: &RgbaImage, y: usize, height: usize) -> Arc<RgbaImage> {
    Arc::new(image::imageops::crop_imm(image, 0, y as u32, image.width(), height as u32).to_image())
}
impl ScrollStitcher {
    pub fn new(image: RgbaImage) -> Result<Self, String> {
        if image.width() < 32 || image.height() < 96 {
            return Err("滚动截图选区至少需要 32 × 96 像素".into());
        }
        if !fits(image.width(), u64::from(image.height())) {
            return Err("滚动截图图片尺寸超过限制".into());
        }
        let sampled = rows(&image);
        let mut index = Index::default();
        index.add(&sampled, 0);
        let height = image.height();
        let image = Arc::new(image);
        Ok(Self {
            strips: VecDeque::from([image.clone()]),
            rows: sampled.into(),
            index,
            last: image,
            top: 0,
            bottom: i64::from(height),
            cursor: 0,
            edges: None,
            header: None,
            footer: None,
            frames: 1,
        })
    }
    pub fn progress(&self, status: ScrollStatus) -> ScrollProgress {
        let (header, footer) = self.edges.unwrap_or_default();
        ScrollProgress {
            width: self.last.width(),
            height: (self.bottom - self.top) as u32 + (header + footer) as u32,
            frames: self.frames,
            status,
        }
    }
    pub fn push(&mut self, image: RgbaImage) -> ScrollProgress {
        self.push_frame(Arc::new(image))
    }
    pub fn push_frame(&mut self, image: Arc<RgbaImage>) -> ScrollProgress {
        if image.dimensions() != self.last.dimensions() {
            return self.progress(ScrollStatus::Unmatched);
        }
        if settled(&self.last, &image) {
            return self.progress(ScrollStatus::Unchanged);
        }
        let sampled = rows(&image);
        let (header, footer) = self
            .edges
            .unwrap_or_else(|| fixed_edges(&rows(&self.last), &sampled));
        if header + footer + 16 >= sampled.len() {
            return self.progress(ScrollStatus::Unmatched);
        }
        let body = &sampled[header..sampled.len() - footer];
        let initial = self.edges.is_none();
        let position = if initial {
            let reference: Vec<_> = self
                .rows
                .iter()
                .skip(header)
                .take(self.rows.len() - header - footer)
                .cloned()
                .collect();
            let mut index = Index::default();
            index.add(&reference, 0);
            index.locate(&reference.into(), 0, body, self.cursor)
        } else {
            self.index.locate(&self.rows, self.top, body, self.cursor)
        };
        let Some(position) = position else {
            return self.progress(ScrollStatus::Unmatched);
        };
        let current_bottom = if initial {
            self.bottom - (header + footer) as i64
        } else {
            self.bottom
        };
        let end = position + body.len() as i64;
        let top = self.top.min(position);
        let bottom = current_bottom.max(end);
        let height = (bottom - top) as u64 + (header + footer) as u64;
        if !fits(image.width(), height) {
            return self.progress(ScrollStatus::Limit);
        }
        if initial {
            self.edges = Some((header, footer));
            self.strips = VecDeque::from([crop(&self.last, header, body.len())]);
            self.rows = self
                .rows
                .iter()
                .skip(header)
                .take(body.len())
                .cloned()
                .collect();
            self.index = Index::default();
            self.index.add(self.rows.make_contiguous(), 0);
            self.header = (header > 0).then(|| crop(&self.last, 0, header));
            self.footer = (footer > 0).then(|| crop(&self.last, sampled.len() - footer, footer));
            self.bottom = current_bottom;
        }
        let mut extended = false;
        if position < self.top {
            let added = (self.top - position) as usize;
            self.strips.push_front(crop(&image, header, added));
            self.index.add(&body[..added], position);
            for row in body[..added].iter().rev() {
                self.rows.push_front(row.clone());
            }
            self.header = (header > 0).then(|| crop(&image, 0, header));
            self.top = position;
            extended = true;
        }
        if end > self.bottom {
            let start = (self.bottom - position) as usize;
            self.strips
                .push_back(crop(&image, header + start, body.len() - start));
            self.index.add(&body[start..], self.bottom);
            self.rows.extend(body[start..].iter().cloned());
            self.footer = (footer > 0).then(|| crop(&image, sampled.len() - footer, footer));
            self.bottom = end;
            extended = true;
        }
        self.cursor = position;
        self.last = image;
        self.frames += 1;
        self.progress(if extended {
            ScrollStatus::Appended
        } else {
            ScrollStatus::Tracked
        })
    }
    pub fn compose(&self) -> RgbaImage {
        let mut image = RgbaImage::new(
            self.last.width(),
            self.progress(ScrollStatus::Unchanged).height,
        );
        let mut offset = 0;
        for strip in self
            .header
            .iter()
            .chain(self.strips.iter())
            .chain(self.footer.iter())
        {
            let bytes = strip.as_raw();
            image.as_mut()[offset..offset + bytes.len()].copy_from_slice(bytes);
            offset += bytes.len();
        }
        image
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dimension_and_pixel_budgets_include_both_extension_directions() {
        assert!(fits(2048, 32768));
        assert!(!fits(2048, 32769));
        assert!(fits(4096, 16384));
        assert!(!fits(4096, 16385));
        assert!(!fits(32769, 96));
    }
    fn page() -> RgbaImage {
        RgbaImage::from_fn(160, 1200, |x, y| {
            let mut seed = x.wrapping_mul(761).wrapping_add(y.wrapping_mul(9301));
            seed ^= seed >> 7;
            seed = seed.wrapping_mul(0x45d9f3b);
            image::Rgba([seed as u8, (seed >> 8) as u8, (seed >> 16) as u8, 255])
        })
    }
    fn frame(page: &RgbaImage, y: u32) -> RgbaImage {
        image::imageops::crop_imm(page, 0, y, 160, 240).to_image()
    }
    #[test]
    fn stitches_variable_displacements_at_original_resolution() {
        let page = page();
        let mut stitcher = ScrollStitcher::new(frame(&page, 0)).unwrap();
        for y in [31, 100, 217, 303, 421] {
            assert_eq!(
                stitcher.push(frame(&page, y)).status,
                ScrollStatus::Appended
            );
        }
        assert_eq!(
            stitcher.compose(),
            image::imageops::crop_imm(&page, 0, 0, 160, 661).to_image()
        );
    }
    #[test]
    fn registers_sparse_text_with_small_antialiasing_and_animation_changes() {
        let mut document = RgbaImage::from_pixel(640, 1600, image::Rgba([250, 250, 250, 255]));
        for y in 0..1600 {
            for x in 40..600 {
                if y % 28 < 12 && (x * 7 + y / 28 * 11) % 31 < 15 {
                    document.put_pixel(x, y, image::Rgba([(y / 28) as u8, 25, 60, 255]));
                }
            }
        }
        let first = image::imageops::crop_imm(&document, 0, 0, 640, 600).to_image();
        let mut next = image::imageops::crop_imm(&document, 0, 121, 640, 600).to_image();
        for pixel in next.pixels_mut() {
            pixel.0[0] = pixel.0[0].saturating_add(1);
        }
        for y in 200..212 {
            for x in 220..250 {
                next.put_pixel(x, y, image::Rgba([0, 160, 240, 255]));
            }
        }
        let mut stitcher = ScrollStitcher::new(first).unwrap();
        assert_eq!(stitcher.push(next).status, ScrollStatus::Appended);
        assert_eq!(stitcher.compose().height(), 721);
    }
    #[test]
    fn registers_continuous_text_motion_without_waiting_for_stationary_frames() {
        let page = RgbaImage::from_fn(640, 1600, |x, y| {
            if y % 28 < 12 && x > 40 && x < 600 && (x * 7 + y / 28 * 11) % 31 < 15 {
                image::Rgba([(y / 28) as u8, 25, 60, 255])
            } else {
                image::Rgba([250, 250, 250, 255])
            }
        });
        let at = |y| image::imageops::crop_imm(&page, 0, y, 640, 600).to_image();
        let mut stitcher = ScrollStitcher::new(at(0)).unwrap();
        for y in (37..520).step_by(37) {
            let image = RgbaImage::from_fn(640, 600, |x, row| {
                let a = page.get_pixel(x, y + row);
                let b = page.get_pixel(x, y + row + 1);
                image::Rgba(std::array::from_fn(|c| {
                    ((u16::from(a[c]) + u16::from(b[c])) / 2) as u8
                }))
            });
            assert!(
                matches!(
                    stitcher.push(image).status,
                    ScrollStatus::Appended | ScrollStatus::Tracked
                ),
                "motion at {y}"
            );
        }
        assert!(matches!(
            stitcher.push(at(550)).status,
            ScrollStatus::Appended | ScrollStatus::Tracked
        ));
        assert_eq!(stitcher.compose().height(), 1150);
    }
    #[test]
    fn tiny_scrolls_retain_only_added_rows_and_the_latest_viewport() {
        let page = page();
        let mut stitcher = ScrollStitcher::new(frame(&page, 0)).unwrap();
        for y in 1..=20 {
            assert_eq!(
                stitcher.push(frame(&page, y)).status,
                ScrollStatus::Appended
            );
        }
        let stored: usize = stitcher
            .strips
            .iter()
            .map(|strip| strip.as_raw().len())
            .sum();
        assert_eq!(stored, 160 * 260 * 4);
        assert_eq!(
            stitcher.compose(),
            image::imageops::crop_imm(&page, 0, 0, 160, 260).to_image()
        );
    }
    #[test]
    fn animation_and_geometry_changes_are_not_treated_as_settled_frames() {
        let page = page();
        let a = frame(&page, 0);
        let b = frame(&page, 8);
        assert!(!settled(&a, &b));
        assert!(!settled(&a, &RgbaImage::new(100, 100)));
        let mut stitcher = ScrollStitcher::new(a.clone()).unwrap();
        assert_eq!(
            stitcher.push(RgbaImage::new(100, 100)).status,
            ScrollStatus::Unmatched
        );
        assert_eq!(stitcher.compose(), a);
        assert!(ScrollStitcher::new(RgbaImage::new(32, 32769)).is_err());
    }
    #[test]
    fn revisits_are_tracked_and_unrelated_frames_preserve_the_document() {
        let page = page();
        let mut stitcher = ScrollStitcher::new(frame(&page, 0)).unwrap();
        stitcher.push(frame(&page, 70));
        let before = stitcher.compose();
        assert_eq!(
            stitcher.push(frame(&page, 70)).status,
            ScrollStatus::Unchanged
        );
        assert_eq!(stitcher.push(frame(&page, 0)).status, ScrollStatus::Tracked);
        assert_eq!(
            stitcher.push(frame(&page, 800)).status,
            ScrollStatus::Unmatched
        );
        assert_eq!(stitcher.compose(), before);
        assert_eq!(
            stitcher.push(frame(&page, 120)).status,
            ScrollStatus::Appended
        );
    }
    #[test]
    fn keeps_fixed_header_and_footer_only_once() {
        let page = page();
        let sticky = |y| {
            let mut image = frame(&page, y);
            for row in 0..18 {
                for x in 0..160 {
                    image.put_pixel(x, row, *page.get_pixel(x, 1000 + row));
                }
            }
            for row in 220..240 {
                for x in 0..160 {
                    image.put_pixel(x, row, *page.get_pixel(x, 900 + row));
                }
            }
            image
        };
        let first = sticky(0);
        let second = sticky(63);
        let third = sticky(110);
        let mut stitcher = ScrollStitcher::new(first.clone()).unwrap();
        assert_eq!(stitcher.push(second).status, ScrollStatus::Appended);
        assert_eq!(stitcher.push(third.clone()).status, ScrollStatus::Appended);
        let result = stitcher.compose();
        assert_eq!(result.height(), 350);
        assert_eq!(
            stitcher
                .header
                .iter()
                .chain(stitcher.strips.iter())
                .chain(stitcher.footer.iter())
                .map(|strip| strip.as_raw().len())
                .sum::<usize>(),
            result.as_raw().len()
        );
        assert_eq!(
            image::imageops::crop_imm(&result, 0, 0, 160, 220).to_image(),
            image::imageops::crop_imm(&first, 0, 0, 160, 220).to_image()
        );
        assert_eq!(
            image::imageops::crop_imm(&result, 0, 220, 160, 110).to_image(),
            image::imageops::crop_imm(&page, 0, 220, 160, 110).to_image()
        );
        assert_eq!(
            image::imageops::crop_imm(&result, 0, 330, 160, 20).to_image(),
            image::imageops::crop_imm(&third, 0, 220, 160, 20).to_image()
        );
    }
    #[test]
    fn blank_and_repeated_content_cannot_claim_an_unambiguous_match() {
        let blank = RgbaImage::from_pixel(160, 240, image::Rgba([255; 4]));
        let mut stitcher = ScrollStitcher::new(blank.clone()).unwrap();
        assert_eq!(stitcher.push(blank).status, ScrollStatus::Unchanged);
        let page = page();
        let repeated = RgbaImage::from_fn(160, 240, |x, y| *page.get_pixel(x, y % 16));
        let shifted = RgbaImage::from_fn(160, 240, |x, y| *repeated.get_pixel(x, (y + 5) % 240));
        let mut stitcher = ScrollStitcher::new(repeated).unwrap();
        assert_eq!(stitcher.push(shifted).status, ScrollStatus::Unmatched);
    }
    #[test]
    fn repeated_revisits_do_not_exhaust_a_frame_count_budget() {
        let page = page();
        let mut stitcher = ScrollStitcher::new(frame(&page, 0)).unwrap();
        stitcher.push(frame(&page, 40));
        let before = stitcher.compose();
        for _ in 0..130 {
            assert_eq!(stitcher.push(frame(&page, 0)).status, ScrollStatus::Tracked);
            assert_eq!(
                stitcher.push(frame(&page, 40)).status,
                ScrollStatus::Tracked
            );
        }
        assert_eq!(stitcher.compose(), before);
        assert!(ScrollStitcher::new(RgbaImage::new(31, 240)).is_err());
        assert!(ScrollStitcher::new(RgbaImage::new(160, 95)).is_err());
    }
    #[test]
    fn grows_above_and_below_the_initial_viewport_in_page_order() {
        let page = page();
        let mut stitcher = ScrollStitcher::new(frame(&page, 600)).unwrap();
        for y in [440, 300, 500, 240, 600, 740, 420, 100] {
            assert!(
                matches!(
                    stitcher.push(frame(&page, y)).status,
                    ScrollStatus::Appended | ScrollStatus::Tracked
                ),
                "position {y}"
            );
        }
        assert_eq!(
            stitcher.compose(),
            image::imageops::crop_imm(&page, 0, 100, 160, 880).to_image()
        );
    }
    #[test]
    fn large_steps_with_a_small_overlap_and_returning_to_an_old_region_are_located() {
        let page = page();
        let mut stitcher = ScrollStitcher::new(frame(&page, 0)).unwrap();
        for y in [210, 420, 630, 840] {
            assert_eq!(
                stitcher.push(frame(&page, y)).status,
                ScrollStatus::Appended
            );
        }
        let before = stitcher.compose();
        for y in [40, 620, 230, 800, 0] {
            assert_eq!(stitcher.push(frame(&page, y)).status, ScrollStatus::Tracked);
        }
        assert_eq!(stitcher.compose(), before);
    }
    #[test]
    fn missing_content_does_not_create_fabricated_rows_and_recovery_can_use_any_known_region() {
        let page = page();
        let mut stitcher = ScrollStitcher::new(frame(&page, 0)).unwrap();
        stitcher.push(frame(&page, 150));
        let before = stitcher.compose();
        assert_eq!(
            stitcher.push(frame(&page, 800)).status,
            ScrollStatus::Unmatched
        );
        assert_eq!(stitcher.compose(), before);
        assert_eq!(
            stitcher.push(frame(&page, 20)).status,
            ScrollStatus::Tracked
        );
        assert_eq!(
            stitcher.push(frame(&page, 220)).status,
            ScrollStatus::Appended
        );
        assert_eq!(
            stitcher.compose(),
            image::imageops::crop_imm(&page, 0, 0, 160, 460).to_image()
        );
    }
    #[test]
    fn fixed_edges_are_not_duplicated_during_bidirectional_extension_and_revisits() {
        let page = page();
        let sticky = |y| {
            let mut image = frame(&page, y);
            for row in 0..18 {
                for x in 0..160 {
                    image.put_pixel(x, row, *page.get_pixel(x, 1000 + row));
                }
            }
            for row in 220..240 {
                for x in 0..160 {
                    image.put_pixel(x, row, *page.get_pixel(x, 900 + row));
                }
            }
            image
        };
        let mut stitcher = ScrollStitcher::new(sticky(400)).unwrap();
        for y in [300, 200, 400, 520, 300, 100] {
            assert!(matches!(
                stitcher.push(sticky(y)).status,
                ScrollStatus::Appended | ScrollStatus::Tracked
            ));
        }
        let result = stitcher.compose();
        assert_eq!(result.height(), 660);
        assert_eq!(
            image::imageops::crop_imm(&result, 0, 18, 160, 622).to_image(),
            image::imageops::crop_imm(&page, 0, 118, 160, 622).to_image()
        );
        assert_eq!(
            image::imageops::crop_imm(&result, 0, 0, 160, 18).to_image(),
            image::imageops::crop_imm(&sticky(100), 0, 0, 160, 18).to_image()
        );
        assert_eq!(
            image::imageops::crop_imm(&result, 0, 640, 160, 20).to_image(),
            image::imageops::crop_imm(&sticky(520), 0, 220, 160, 20).to_image()
        );
    }
}
