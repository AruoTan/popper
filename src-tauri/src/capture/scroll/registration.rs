//! Locate a viewport in the entire captured vertical document.
use image::RgbaImage;
use std::collections::{HashMap, VecDeque};

const SAMPLES: usize = 32;
#[derive(Clone)]
pub(super) struct Row {
    pixels: [u8; SAMPLES * 3],
    pub key: Option<u32>,
}
fn difference(a: &[u8], b: &[u8]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(a, b)| f64::from(a.abs_diff(*b)))
        .sum::<f64>()
        / a.len() as f64
}
pub(super) fn sample(image: &RgbaImage, y: u32) -> Row {
    let mut pixels = [0; SAMPLES * 3];
    let mut luma = [0u32; SAMPLES];
    for x in 0..SAMPLES {
        let px = ((x + 2) as u64 * u64::from(image.width() - 1) / (SAMPLES + 3) as u64) as u32;
        let rgb = &image.get_pixel(px, y).0[..3];
        pixels[x * 3..x * 3 + 3].copy_from_slice(rgb);
        luma[x] = (u32::from(rgb[0]) * 77 + u32::from(rgb[1]) * 150 + u32::from(rgb[2]) * 29) / 256;
    }
    let mean = luma.iter().sum::<u32>() / SAMPLES as u32;
    let textured = pixels
        .chunks_exact(3)
        .skip(1)
        .any(|p| difference(p, &pixels[..3]) > 14.0);
    let key = textured.then(|| {
        luma.iter()
            .enumerate()
            .fold(0, |bits, (i, &v)| bits | (u32::from(v > mean) << i))
    });
    Row { pixels, key }
}
pub(super) fn rows(image: &RgbaImage) -> Vec<Row> {
    (0..image.height()).map(|y| sample(image, y)).collect()
}
pub(super) fn distance(a: &Row, b: &Row) -> f64 {
    difference(&a.pixels, &b.pixels)
}
pub fn settled(a: &RgbaImage, b: &RgbaImage) -> bool {
    if a.dimensions() != b.dimensions() {
        return false;
    }
    let mut changed = 0;
    let mut total = 0;
    for y in (0..a.height()).step_by((a.height() / 80).max(1) as usize) {
        let a = sample(a, y);
        let b = sample(b, y);
        changed += a
            .pixels
            .chunks_exact(3)
            .zip(b.pixels.chunks_exact(3))
            .filter(|(a, b)| difference(a, b) > 12.0)
            .count();
        total += SAMPLES;
    }
    changed * 200 <= total
}

#[derive(Default)]
pub(super) struct Index(HashMap<u32, Vec<i64>>);
impl Index {
    pub fn add(&mut self, rows: &[Row], start: i64) {
        for (i, row) in rows.iter().enumerate() {
            if let Some(key) = row.key {
                self.0.entry(key).or_default().push(start + i as i64);
            }
        }
    }
    pub fn locate(
        &self,
        document: &VecDeque<Row>,
        top: i64,
        viewport: &[Row],
        cursor: i64,
    ) -> Option<i64> {
        let mut votes: HashMap<i64, u32> = HashMap::new();
        votes.insert(cursor, 0);
        // Row fingerprints generate candidate locations without scanning every pixel
        // at every possible offset. A one-bit neighborhood tolerates rasterization noise.
        for (block, rows) in viewport.chunks((viewport.len() / 32).max(1)).enumerate() {
            let stride = (viewport.len() / 32).max(1);
            let Some((local, row)) = rows.iter().enumerate().find(|(_, r)| r.key.is_some()) else {
                continue;
            };
            let y = block * stride + local;
            let key = row.key.unwrap();
            for variant in std::iter::once(key).chain((0..32).map(|bit| key ^ (1 << bit))) {
                if let Some(positions) = self.0.get(&variant) {
                    // Extremely repetitive fingerprints have little locating value.
                    if positions.len() > 1024 {
                        continue;
                    }
                    for &position in positions {
                        *votes.entry(position - y as i64).or_default() += 1;
                    }
                }
            }
        }
        let bottom = top + document.len() as i64;
        let overlap = |position: i64| {
            (
                top.max(position),
                bottom.min(position + viewport.len() as i64),
            )
        };
        let minimum = (viewport.len() / 32).clamp(16, 48) as i64;
        let score = |position: i64, coarse: bool| -> Option<f64> {
            let (begin, end) = overlap(position);
            if end - begin < minimum {
                return None;
            }
            let step = if coarse {
                ((end - begin) / 96).max(1) as usize
            } else {
                1
            };
            let mut errors = Vec::new();
            for y in (begin..end).step_by(step) {
                let a = &document[(y - top) as usize];
                let b = &viewport[(y - position) as usize];
                if a.key.is_some() || b.key.is_some() {
                    errors.push(distance(a, b));
                }
            }
            if errors.len() < 6 {
                return None;
            }
            errors.sort_by(f64::total_cmp);
            let keep = (errors.len() * 9 / 10).max(1);
            Some(errors[..keep].iter().sum::<f64>() / keep as f64)
        };
        let mut candidates: Vec<_> = votes.into_iter().collect();
        candidates.sort_by_key(|&(position, count)| {
            (std::cmp::Reverse(count), position.abs_diff(cursor))
        });
        let mut scores: Vec<_> = candidates
            .into_iter()
            .take(256)
            .filter_map(|(p, _)| score(p, true).map(|s| (s, p)))
            .collect();
        scores.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut scores: Vec<_> = scores
            .into_iter()
            .take(48)
            .filter_map(|(_, p)| score(p, false).map(|s| (s, p)))
            .collect();
        scores.sort_by(|a, b| a.0.total_cmp(&b.0));
        let &(error, position) = scores.first()?;
        if error > 9.0
            || scores
                .iter()
                .any(|&(other, p)| p.abs_diff(position) > 3 && other <= error + 0.8)
        {
            return None;
        }
        Some(position)
    }
}

pub(super) fn fixed_edges(a: &[Row], b: &[Row]) -> (usize, usize) {
    let edge = |values: Vec<usize>| {
        let same: Vec<_> = values
            .into_iter()
            .take_while(|&y| distance(&a[y], &b[y]) < 1.0)
            .collect();
        // Uniform page margins alone do not establish a sticky header/footer.
        if same.iter().any(|&y| a[y].key.is_some()) {
            same.len()
        } else {
            0
        }
    };
    (
        edge((0..a.len() / 3).collect()),
        edge((a.len() - a.len() / 3..a.len()).rev().collect()),
    )
}
