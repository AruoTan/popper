use super::{CaptureCandidate, CaptureHitTest, CaptureRect};
use std::{
    collections::HashSet,
    sync::mpsc,
    time::{Duration, Instant},
};

/// Providers share a deadline, but the first useful result only waits a short
/// grace period for its peer. Missing results can be refined on the next probe.
pub fn collect_providers(
    results: mpsc::Receiver<CaptureHitTest>,
    providers: usize,
    timeout: Duration,
    grace: Duration,
) -> CaptureHitTest {
    let mut deadline = Instant::now() + timeout;
    let mut combined = CaptureHitTest::default();
    let mut completed = 0;
    while completed < providers {
        let Ok(result) = results.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        else {
            break;
        };
        completed += 1;
        combined.pending |= result.pending;
        if !result.candidates.is_empty() {
            deadline = deadline.min(Instant::now() + grace);
            combined.candidates.extend(result.candidates);
        }
    }
    combined.pending |= completed < providers;
    combined
}

pub fn area(rect: CaptureRect) -> u64 {
    u64::from(rect.width) * u64::from(rect.height)
}

/// Keep a nested hierarchy, so switching upward cannot select an overlapping sibling.
pub fn candidate_chain(
    mut candidates: Vec<CaptureCandidate>,
    x: i32,
    y: i32,
) -> Vec<CaptureCandidate> {
    candidates.retain(|candidate| candidate.rect.contains(x, y));
    candidates.sort_by_key(|candidate| area(candidate.rect));
    let mut result: Vec<CaptureCandidate> = Vec::new();
    for candidate in candidates {
        if result.last().is_none_or(|previous| {
            previous.rect != candidate.rect
                && previous.rect.intersect(candidate.rect) == Some(previous.rect)
        }) {
            result.push(candidate);
        }
    }
    result
}

pub trait HitTree {
    fn candidate(&self, node: usize) -> Option<&CaptureCandidate>;
    fn children(&mut self, node: usize) -> Vec<usize>;
}

/// Preserve geometry across focus changes, including branches away from the pointer.
/// Window-sized rectangles are already recorded separately. Copy useful subregions.
pub fn snapshot(
    tree: &mut impl HitTree,
    bounds: CaptureRect,
    budget: Duration,
) -> Vec<CaptureCandidate> {
    let started = Instant::now();
    let mut stack = vec![(0, 0)];
    let mut visited = 0;
    let mut rectangles = HashSet::new();
    let mut candidates = Vec::new();
    while let Some((node, depth)) = stack.pop() {
        if visited >= 8192 || candidates.len() >= 4096 || started.elapsed() >= budget {
            break;
        }
        visited += 1;
        if depth >= 64 {
            continue;
        }
        if let Some(candidate) = tree.candidate(node) {
            if let Some(rect) = candidate.rect.intersect(bounds) {
                if rect != bounds && rectangles.insert((rect.x, rect.y, rect.width, rect.height)) {
                    candidates.push(CaptureCandidate {
                        rect,
                        label: candidate.label.clone(),
                    });
                }
            }
        }
        // Layout containers can have missing or misleading bounds. Descendants
        // still need examination; the provider has already cached this subtree.
        stack.extend(
            tree.children(node)
                .into_iter()
                .map(|child| (child, depth + 1)),
        );
    }
    candidates
}

/// Explore every containing branch rather than committing to the smallest container.
/// Nodes without bounds may be layout containers and must not hide their descendants.
pub fn search(tree: &mut impl HitTree, x: i32, y: i32, budget: Duration) -> CaptureHitTest {
    let started = Instant::now();
    let mut pending = vec![(0, Vec::new(), 0)];
    let mut best = Vec::new();
    let mut best_area = u64::MAX;
    let mut visited = 0;
    let mut incomplete = false;
    while let Some((node, mut path, depth)) = pending.pop() {
        if visited >= 1024 {
            break;
        }
        if started.elapsed() >= budget {
            incomplete = true;
            break;
        }
        if depth >= 64 {
            continue;
        }
        visited += 1;
        if let Some(candidate) = tree.candidate(node) {
            if !candidate.rect.contains(x, y) {
                continue;
            }
            let size = area(candidate.rect);
            path.push(candidate.clone());
            if size < best_area || (size == best_area && path.len() > best.len()) {
                best_area = size;
                best = path.clone();
            }
        }
        let mut children = tree.children(node);
        children.retain(|&child| tree.candidate(child).is_none_or(|c| c.rect.contains(x, y)));
        // The stack visits small containers first, but still checks overlapping branches.
        children.sort_by_key(|&child| {
            std::cmp::Reverse(tree.candidate(child).map_or(0, |c| area(c.rect)))
        });
        for child in children {
            pending.push((child, path.clone(), depth + 1));
        }
    }
    CaptureHitTest {
        candidates: candidate_chain(best, x, y),
        pending: incomplete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Tree(Vec<(Option<CaptureCandidate>, Vec<usize>)>);
    impl HitTree for Tree {
        fn candidate(&self, node: usize) -> Option<&CaptureCandidate> {
            self.0[node].0.as_ref()
        }
        fn children(&mut self, node: usize) -> Vec<usize> {
            self.0[node].1.clone()
        }
    }
    fn candidate(size: u32, label: &str) -> CaptureCandidate {
        CaptureCandidate {
            rect: CaptureRect {
                x: -100,
                y: -100,
                width: size,
                height: size,
            },
            label: label.into(),
        }
    }
    #[test]
    fn frozen_geometry_keeps_siblings_available_after_the_source_closes() {
        let bounds = candidate(1000, "window").rect;
        let left = candidate(40, "first item");
        let mut right = candidate(40, "second item");
        right.rect.x = 100;
        let mut tree = Tree(vec![
            (Some(candidate(1000, "window")), vec![1]),
            (None, vec![2, 3]),
            (Some(left), vec![]),
            (Some(right), vec![]),
        ]);
        let frozen = snapshot(&mut tree, bounds, Duration::from_secs(1));
        tree.0.clear();
        assert_eq!(
            candidate_chain(frozen.clone(), -90, -90)[0].label,
            "first item"
        );
        assert_eq!(candidate_chain(frozen, 110, -90)[0].label, "second item");
    }
    #[test]
    fn snapshot_clips_negative_screen_coordinates_and_removes_duplicates() {
        let bounds = CaptureRect {
            x: -90,
            y: -90,
            width: 200,
            height: 200,
        };
        let mut outside = candidate(40, "outside");
        outside.rect.x = 1000;
        let mut tree = Tree(vec![
            (Some(candidate(1000, "window")), vec![1, 2, 3, 4]),
            (Some(candidate(40, "clipped item")), vec![]),
            (Some(candidate(40, "duplicate")), vec![]),
            (Some(outside), vec![]),
            (Some(candidate(1000, "root layout")), vec![]),
        ]);
        let result = snapshot(&mut tree, bounds, Duration::from_secs(1));
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].rect,
            CaptureRect {
                x: -90,
                y: -90,
                width: 30,
                height: 30
            }
        );
    }
    #[test]
    fn snapshot_traverses_layout_containers_with_misleading_bounds() {
        let mut layout = candidate(40, "layout");
        layout.rect.x = 2000;
        let mut tree = Tree(vec![
            (Some(candidate(1000, "window")), vec![1]),
            (Some(layout), vec![2]),
            (Some(candidate(40, "visible item")), vec![]),
        ]);
        let result = snapshot(
            &mut tree,
            candidate(1000, "window").rect,
            Duration::from_secs(1),
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "visible item");
    }
    #[test]
    fn snapshot_is_bounded_even_for_a_cyclic_provider_tree() {
        let mut tree = Tree(vec![(Some(candidate(1000, "window")), vec![0])]);
        assert!(snapshot(
            &mut tree,
            candidate(1000, "window").rect,
            Duration::from_secs(1)
        )
        .is_empty());
        assert!(snapshot(&mut tree, candidate(1000, "window").rect, Duration::ZERO).is_empty());
    }
    #[test]
    fn a_root_only_snapshot_does_not_invent_fine_grained_controls() {
        let mut tree = Tree(vec![(Some(candidate(1000, "window")), vec![])]);
        assert!(snapshot(
            &mut tree,
            candidate(1000, "window").rect,
            Duration::from_secs(1)
        )
        .is_empty());
    }
    #[test]
    fn snapshot_keeps_a_provider_root_smaller_than_its_native_window() {
        let mut tree = Tree(vec![(Some(candidate(300, "popup surface")), vec![])]);
        let result = snapshot(
            &mut tree,
            candidate(1000, "native host").rect,
            Duration::from_secs(1),
        );
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].label, "popup surface");
    }
    #[test]
    fn a_smaller_overlapping_container_does_not_hide_a_deeper_control() {
        let mut tree = Tree(vec![
            (Some(candidate(1000, "window")), vec![1, 2]),
            (Some(candidate(300, "overlay container")), vec![]),
            (Some(candidate(800, "content")), vec![3]),
            (Some(candidate(40, "text")), vec![]),
        ]);
        let result = search(&mut tree, -90, -90, Duration::from_secs(1));
        assert_eq!(
            result
                .candidates
                .iter()
                .map(|c| c.label.as_str())
                .collect::<Vec<_>>(),
            ["text", "content", "window"]
        );
    }
    #[test]
    fn unbounded_layout_nodes_and_deep_raw_elements_are_traversed() {
        let mut nodes = vec![(Some(candidate(1000, "window")), vec![1])];
        for index in 1..24 {
            nodes.push((None, vec![index + 1]));
        }
        nodes.push((Some(candidate(20, "image")), vec![]));
        let result = search(&mut Tree(nodes), -90, -90, Duration::from_secs(1));
        assert_eq!(result.candidates[0].label, "image");
    }
    #[test]
    fn duplicate_rectangles_and_overlapping_siblings_are_not_extra_parents() {
        let rect = CaptureRect {
            x: -95,
            y: -95,
            width: 200,
            height: 200,
        };
        let result = candidate_chain(
            vec![
                candidate(20, "leaf"),
                candidate(20, "duplicate"),
                CaptureCandidate {
                    rect,
                    label: "sibling".into(),
                },
                candidate(300, "window"),
            ],
            -90,
            -90,
        );
        assert_eq!(
            result.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(),
            ["leaf", "window"]
        );
    }
    #[test]
    fn invisible_and_outside_branches_do_not_become_targets() {
        let mut outside = candidate(20, "outside");
        outside.rect.x = 500;
        let result = search(
            &mut Tree(vec![
                (Some(candidate(1000, "window")), vec![1]),
                (Some(outside), vec![]),
            ]),
            0,
            0,
            Duration::from_secs(1),
        );
        assert_eq!(result.candidates.len(), 1);
        assert!(candidate_chain(vec![candidate(0, "empty")], -100, -100).is_empty());
    }
    #[test]
    fn exhausted_time_budget_requests_refinement_instead_of_claiming_a_complete_result() {
        let result = search(
            &mut Tree(vec![(Some(candidate(100, "window")), vec![])]),
            -90,
            -90,
            Duration::ZERO,
        );
        assert!(result.pending);
    }
    #[test]
    fn a_depth_limited_layout_branch_does_not_skip_other_siblings() {
        let mut nodes = vec![(Some(candidate(1000, "window")), vec![1, 65])];
        for index in 1..64 {
            nodes.push((None, vec![index + 1]));
        }
        nodes.push((None, vec![]));
        nodes.push((Some(candidate(20, "control")), vec![]));
        let result = search(&mut Tree(nodes), -90, -90, Duration::from_secs(1));
        assert_eq!(result.candidates[0].label, "control");
        assert!(!result.pending);
    }
    #[test]
    fn a_stalled_provider_cannot_suppress_a_direct_point_result() {
        let (sender, receiver) = mpsc::sync_channel(2);
        sender
            .send(CaptureHitTest {
                candidates: vec![candidate(20, "control")],
                pending: false,
            })
            .unwrap();
        let result = collect_providers(
            receiver,
            2,
            Duration::from_millis(100),
            Duration::from_millis(1),
        );
        assert_eq!(result.candidates[0].label, "control");
        assert!(result.pending);
        drop(sender);
    }
    #[test]
    fn successful_point_and_tree_results_are_merged_and_stop_retrying() {
        let (sender, receiver) = mpsc::sync_channel(2);
        for label in ["direct control", "tree parent"] {
            sender
                .send(CaptureHitTest {
                    candidates: vec![candidate(20, label)],
                    pending: false,
                })
                .unwrap();
        }
        let result = collect_providers(
            receiver,
            2,
            Duration::from_millis(100),
            Duration::from_millis(1),
        );
        assert_eq!(result.candidates.len(), 2);
        assert!(!result.pending);
    }
    #[test]
    fn a_failed_provider_does_not_discard_its_successful_peer() {
        let (sender, receiver) = mpsc::sync_channel(2);
        sender.send(CaptureHitTest::default()).unwrap();
        sender
            .send(CaptureHitTest {
                candidates: vec![candidate(20, "control")],
                pending: false,
            })
            .unwrap();
        let result = collect_providers(
            receiver,
            2,
            Duration::from_millis(100),
            Duration::from_millis(1),
        );
        assert_eq!(result.candidates[0].label, "control");
        assert!(!result.pending);
    }
}
