//! An immutable strip snapshot, filtered without holding the registration lock.
use super::super::ScrollProgress;
use image::{GenericImageView, Rgba, RgbaImage};
use std::sync::Arc;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrollPreview {
    #[serde(flatten)]
    pub progress: ScrollProgress,
    pub origin_y: i64,
    #[serde(skip)]
    pub strips: Vec<Arc<RgbaImage>>,
}

struct StripView<'a> {
    width: u32,
    rows: Vec<&'a [u8]>,
}
impl GenericImageView for StripView<'_> {
    type Pixel = Rgba<u8>;
    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.rows.len() as u32)
    }
    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        let start = x as usize * 4;
        Rgba(self.rows[y as usize][start..start + 4].try_into().unwrap())
    }
}

impl ScrollPreview {
    pub fn render(&self, max_width: u32, max_height: u32) -> RgbaImage {
        let (width, height) = (self.progress.width, self.progress.height);
        let scale = (max_width.clamp(1, 1024) as f64 / width as f64)
            .min(max_height.clamp(1, 2048) as f64 / height as f64)
            .min(1.0);
        let view = StripView {
            width,
            rows: self
                .strips
                .iter()
                .flat_map(|strip| strip.as_raw().chunks_exact(width as usize * 4))
                .collect(),
        };
        // Area averaging preserves thin text that nearest-pixel sampling can miss.
        image::imageops::thumbnail(
            &view,
            (width as f64 * scale).round().max(1.0) as u32,
            (height as f64 * scale).round().max(1.0) as u32,
        )
    }
}
