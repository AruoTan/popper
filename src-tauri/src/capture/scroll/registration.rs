//! Locate a viewport in the entire captured vertical document.
use image::RgbaImage;
use std::{
    collections::{HashMap, VecDeque},
    hash::{DefaultHasher, Hash, Hasher},
};

const SAMPLES: usize = 32;
#[derive(Clone)]
pub(super) struct Row {
    pixels: [u8; SAMPLES * 3],
    pub key: Option<u32>,
    exact: u64,
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
    let mut hash = DefaultHasher::new();
    pixels.hash(&mut hash);
    Row {
        pixels,
        key,
        exact: hash.finish(),
    }
}
pub(super) fn rows(image: &RgbaImage) -> Vec<Row> {
    (0..image.height()).map(|y| sample(image, y)).collect()
}
pub(super) fn distance(a: &Row, b: &Row) -> f64 {
    difference(&a.pixels, &b.pixels)
}
fn boundary(a: &Row, b: &Row) -> Option<(u64, u64)> {
    // Page separators can vary vertically while every individual row is uniform.
    (a.key.is_none() && b.key.is_none() && distance(a, b) >= 1.0).then_some((a.exact, b.exact))
}
pub fn settled(a: &RgbaImage, b: &RgbaImage) -> bool {
    // Sparse samples can miss small text and whole bands during fast motion.
    a.dimensions() == b.dimensions() && a.as_raw() == b.as_raw()
}

#[derive(Default)]
pub(super) struct Index {
    perceptual: HashMap<u32, Vec<i64>>,
    exact: HashMap<u64, Vec<i64>>,
    boundaries: HashMap<(u64, u64), Vec<i64>>,
}
impl Index {
    pub fn add(&mut self, rows: &[Row], start: i64) {
        for (i, row) in rows.iter().enumerate() {
            if let Some(key) = row.key {
                self.perceptual
                    .entry(key)
                    .or_default()
                    .push(start + i as i64);
                self.exact
                    .entry(row.exact)
                    .or_default()
                    .push(start + i as i64);
            }
        }
        for (i, pair) in rows.windows(2).enumerate() {
            self.add_boundary(&pair[0], &pair[1], start + i as i64);
        }
    }
    pub fn add_boundary(&mut self, a: &Row, b: &Row, position: i64) {
        if let Some(key) = boundary(a, b) {
            self.boundaries.entry(key).or_default().push(position);
        }
    }
    pub fn locate(
        &self,
        document: &VecDeque<Row>,
        top: i64,
        viewport: &[Row],
        cursor: i64,
    ) -> Option<i64> {
        let stride = (viewport.len() / 32).max(1);
        let edge = 48.min(viewport.len());
        let mut anchors: Vec<_> =
            viewport
                .chunks(stride)
                .enumerate()
                .filter_map(|(block, rows)| {
                    rows.iter()
                        .enumerate()
                        .find(|(_, row)| row.key.is_some())
                        .map(|(local, row)| (block * stride + local, row))
                })
                .chain(viewport.iter().enumerate().filter(|(y, row)| {
                    row.key.is_some() && (*y < edge || *y + edge >= viewport.len())
                }))
                .collect();
        anchors.sort_by_key(|&(y, _)| y);
        anchors.dedup_by_key(|(y, _)| *y);
        let mut exact_votes: HashMap<i64, (u32, u32)> = HashMap::new();
        for &(y, row) in &anchors {
            if let Some(positions) = self.exact.get(&row.exact).filter(|p| p.len() <= 1024) {
                for &position in positions {
                    exact_votes.entry(position - y as i64).or_default().0 += 1;
                }
            }
        }
        for (y, pair) in viewport.windows(2).enumerate() {
            if let Some(positions) = boundary(&pair[0], &pair[1])
                .and_then(|key| self.boundaries.get(&key))
                .filter(|p| p.len() <= 1024)
            {
                for &position in positions {
                    exact_votes.entry(position - y as i64).or_default().1 += 1;
                }
            }
        }
        let bottom = top + document.len() as i64;
        let mut exact_match = None;
        // Verify every sampled row, including blank rows; hashes only propose offsets.
        // Dense edge anchors retain tiny overlaps after a near-viewport-sized jump.
        for (position, (text, boundaries)) in exact_votes {
            if text < 6 && boundaries < 2 {
                continue;
            }
            let begin = top.max(position);
            let end = bottom.min(position + viewport.len() as i64);
            if end - begin < 8 {
                continue;
            }
            let mut evidence = 0;
            let mut boundary_evidence = 0;
            let identical = (begin..end).all(|y| {
                let a = &document[(y - top) as usize];
                let b = &viewport[(y - position) as usize];
                evidence += usize::from(a.key.is_some());
                boundary_evidence += usize::from(
                    y + 1 < end && boundary(a, &document[(y + 1 - top) as usize]).is_some(),
                );
                a.pixels == b.pixels
            });
            // A complete separator needs two transitions; blank overlap alone cannot locate it.
            if identical
                && (evidence >= 6 || boundary_evidence >= 2)
                && exact_match.replace(position).is_some()
            {
                return None;
            }
        }
        if exact_match.is_some() {
            return exact_match;
        }
        let mut votes: HashMap<i64, u32> = HashMap::new();
        votes.insert(cursor, 0);
        // Row fingerprints generate candidate locations without scanning every pixel
        // at every possible offset. A one-bit neighborhood tolerates rasterization noise.
        for &(y, row) in &anchors {
            let key = row.key.unwrap();
            for variant in std::iter::once(key).chain((0..32).map(|bit| key ^ (1 << bit))) {
                if let Some(positions) = self.perceptual.get(&variant) {
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
        let overlap = |position: i64| {
            (
                top.max(position),
                bottom.min(position + viewport.len() as i64),
            )
        };
        let minimum = 8;
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
