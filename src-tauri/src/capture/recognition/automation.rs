//! UI Automation traversal and per-session provider caches.
use super::Probe;
use crate::capture::{
    hit_test::{self, HitTree},
    CaptureCandidate, CaptureHitTest, CaptureRect,
};
use std::time::Duration;
use windows::Win32::{
    Foundation::HWND,
    UI::Accessibility::{
        IUIAutomation, IUIAutomationCacheRequest, IUIAutomationCondition, IUIAutomationElement,
        TreeScope_Children, TreeScope_Element, UIA_BoundingRectanglePropertyId,
        UIA_IsOffscreenPropertyId, UIA_NamePropertyId,
    },
};

struct Node {
    element: IUIAutomationElement,
    candidate: Option<CaptureCandidate>,
    offscreen: bool,
    children: Option<Vec<usize>>,
}
impl Node {
    fn cached(element: IUIAutomationElement) -> Self {
        let candidate = unsafe { element.CachedBoundingRectangle() }
            .ok()
            .and_then(|rect| {
                (rect.right > rect.left && rect.bottom > rect.top).then(|| CaptureCandidate {
                    rect: CaptureRect {
                        x: rect.left,
                        y: rect.top,
                        width: (i64::from(rect.right) - i64::from(rect.left)) as u32,
                        height: (i64::from(rect.bottom) - i64::from(rect.top)) as u32,
                    },
                    label: unsafe { element.CachedName() }
                        .map(|name| name.to_string().chars().take(64).collect())
                        .unwrap_or_default(),
                })
            });
        let offscreen = unsafe { element.CachedIsOffscreen() }.is_ok_and(|value| value.as_bool());
        Self {
            element,
            candidate,
            offscreen,
            children: None,
        }
    }
}
struct CachedTree {
    nodes: Vec<Node>,
    cache: IUIAutomationCacheRequest,
    condition: IUIAutomationCondition,
}
impl HitTree for CachedTree {
    fn candidate(&self, node: usize) -> Option<&CaptureCandidate> {
        self.nodes[node].candidate.as_ref()
    }
    fn children(&mut self, node: usize) -> Vec<usize> {
        if let Some(children) = &self.nodes[node].children {
            return children.clone();
        }
        let mut children = Vec::new();
        if !self.nodes[node].offscreen && self.nodes.len() < 8192 {
            // Batch all child properties in one provider round trip. TrueCondition
            // includes raw text, images and layout elements omitted by Control View.
            if let Ok(elements) = unsafe {
                self.nodes[node].element.FindAllBuildCache(
                    TreeScope_Children,
                    &self.condition,
                    &self.cache,
                )
            } {
                let count = unsafe { elements.Length() }
                    .unwrap_or_default()
                    .min((8192 - self.nodes.len()) as i32);
                for index in 0..count {
                    if let Ok(element) = unsafe { elements.GetElement(index) } {
                        let child = Node::cached(element);
                        if child.offscreen {
                            continue;
                        }
                        children.push(self.nodes.len());
                        self.nodes.push(child);
                    }
                }
            }
        }
        self.nodes[node].children = Some(children.clone());
        children
    }
}
pub(super) struct Inspector {
    automation: IUIAutomation,
    session: String,
    // Bound provider references and reuse sibling geometry while moving within a window.
    trees: Vec<(usize, CachedTree)>,
}
impl Inspector {
    pub(super) fn new(automation: IUIAutomation) -> Self {
        Self {
            automation,
            session: String::new(),
            trees: Vec::new(),
        }
    }
    pub(super) fn inspect(&mut self, probe: Probe) -> windows::core::Result<CaptureHitTest> {
        if self.session != probe.session {
            self.trees.clear();
            self.session = probe.session;
        }
        let tree = if let Some(index) = self
            .trees
            .iter()
            .position(|(handle, _)| *handle == probe.handle)
        {
            self.trees.remove(index).1
        } else {
            let cache = unsafe { self.automation.CreateCacheRequest()? };
            unsafe {
                cache.SetTreeScope(TreeScope_Element)?;
                cache.SetTreeFilter(&self.automation.RawViewCondition()?)?;
                cache.AddProperty(UIA_BoundingRectanglePropertyId)?;
                cache.AddProperty(UIA_NamePropertyId)?;
                cache.AddProperty(UIA_IsOffscreenPropertyId)?;
            }
            let root = unsafe {
                self.automation
                    .ElementFromHandleBuildCache(HWND(probe.handle as *mut _), &cache)?
            };
            CachedTree {
                nodes: vec![Node::cached(root)],
                cache,
                condition: unsafe { self.automation.CreateTrueCondition()? },
            }
        };
        if self.trees.len() >= 8 {
            self.trees.remove(0);
        }
        self.trees.push((probe.handle, tree));
        let tree = &mut self.trees.last_mut().unwrap().1;
        Ok(hit_test::search(
            tree,
            probe.x,
            probe.y,
            Duration::from_millis(100),
        ))
    }
}
