//! Point-based accessibility queries scoped to a captured application window.
use super::{hit_test::candidate_chain, CaptureCandidate, CaptureHitTest, CaptureRect};
use std::time::{Duration, Instant};
use windows::{
    core::{IUnknown, Interface},
    Win32::{
        Foundation::{HWND, POINT},
        Graphics::Gdi::ScreenToClient,
        System::{
            Com::IDispatch,
            Variant::{VARIANT, VT_DISPATCH, VT_I4},
        },
        UI::{
            Accessibility::{AccessibleObjectFromWindow, IAccessible},
            WindowsAndMessaging::{
                ChildWindowFromPointEx, CWP_SKIPINVISIBLE, CWP_SKIPTRANSPARENT, OBJID_CLIENT,
                OBJID_WINDOW,
            },
        },
    },
};

#[derive(Default)]
pub struct PointInspector {
    session: String,
    roots: Vec<(usize, IAccessible)>,
}
fn identity(object: &IAccessible) -> windows::core::Result<usize> {
    Ok(object.cast::<IUnknown>()?.as_raw() as usize)
}
fn point_window(handle: usize, x: i32, y: i32) -> usize {
    let mut window = HWND(handle as *mut _);
    for _ in 0..32 {
        let mut point = POINT { x, y };
        if !unsafe { ScreenToClient(window, &mut point) }.as_bool() {
            break;
        }
        let child = unsafe {
            ChildWindowFromPointEx(window, point, CWP_SKIPINVISIBLE | CWP_SKIPTRANSPARENT)
        };
        if child.is_invalid() || child == window {
            break;
        }
        window = child;
    }
    window.0 as usize
}
fn root_from_window(handle: usize) -> windows::core::Result<IAccessible> {
    root_object(handle, OBJID_CLIENT.0 as u32)
        .or_else(|_| root_object(handle, OBJID_WINDOW.0 as u32))
}
fn root_object(handle: usize, object: u32) -> windows::core::Result<IAccessible> {
    let mut raw = std::ptr::null_mut();
    unsafe {
        AccessibleObjectFromWindow(HWND(handle as *mut _), object, &IAccessible::IID, &mut raw)?;
    }
    if raw.is_null() {
        return Err(windows::core::Error::from_hresult(windows::core::HRESULT(
            0x80004003u32 as i32,
        )));
    }
    Ok(unsafe { IAccessible::from_raw(raw) })
}
fn bounds(object: &IAccessible, child: &VARIANT) -> Option<CaptureCandidate> {
    let (mut x, mut y, mut width, mut height) = (0, 0, 0, 0);
    unsafe { object.accLocation(&mut x, &mut y, &mut width, &mut height, child) }.ok()?;
    (width > 0 && height > 0).then_some(CaptureCandidate {
        rect: CaptureRect {
            x,
            y,
            width: width as u32,
            height: height as u32,
        },
        label: String::new(),
    })
}
impl PointInspector {
    pub fn inspect(
        &mut self,
        session: &str,
        handle: usize,
        x: i32,
        y: i32,
    ) -> windows::core::Result<CaptureHitTest> {
        if self.session != session {
            self.roots.clear();
            self.session = session.into();
        }
        let hit_window = point_window(handle, x, y);
        let root = if let Some(index) = self
            .roots
            .iter()
            .position(|(window, _)| *window == hit_window)
        {
            self.roots.remove(index).1
        } else {
            root_from_window(hit_window).or_else(|_| root_from_window(handle))?
        };
        if self.roots.len() >= 8 {
            self.roots.remove(0);
        }
        self.roots.push((hit_window, root.clone()));
        let root_id = identity(&root)?;
        let started = Instant::now();
        let budget = Duration::from_millis(80);
        let mut current = root;
        let mut seen = Vec::new();
        let mut candidates = Vec::new();
        let mut leaf_child = VARIANT::from(0i32);
        for _ in 0..64 {
            if started.elapsed() >= budget {
                break;
            }
            let id = identity(&current)?;
            if seen.contains(&id) {
                break;
            }
            seen.push(id);
            if let Some(candidate) = bounds(&current, &VARIANT::from(0i32)) {
                candidates.push(candidate);
            }
            let Ok(hit) = (unsafe { current.accHitTest(x, y) }) else {
                break;
            };
            match hit.vt() {
                VT_DISPATCH => {
                    let Ok(next) =
                        IDispatch::try_from(&hit).and_then(|child| child.cast::<IAccessible>())
                    else {
                        break;
                    };
                    if identity(&next)? == id {
                        break;
                    }
                    current = next;
                    leaf_child = VARIANT::from(0i32);
                }
                VT_I4 => {
                    // A simple element belongs to its parent's interface; zero means self.
                    let child = i32::try_from(&hit)?;
                    if child <= 0 {
                        break;
                    }
                    if let Some(candidate) = bounds(&current, &hit) {
                        candidates.push(candidate);
                    }
                    leaf_child = hit;
                    let next = unsafe { current.get_accChild(&leaf_child) }
                        .and_then(|child| child.cast::<IAccessible>());
                    let Ok(next) = next else {
                        break;
                    };
                    if identity(&next)? == id {
                        break;
                    }
                    current = next;
                    leaf_child = VARIANT::from(0i32);
                }
                _ => break,
            }
        }
        if started.elapsed() < budget {
            if let Ok(name) = unsafe { current.get_accName(&leaf_child) } {
                if let Some(candidate) = candidates.last_mut() {
                    candidate.label = name.to_string().chars().take(64).collect();
                }
            }
        }
        // A point hit may jump directly to a descendant. Recover intermediate parents
        // instead of replacing the hierarchy with arbitrary overlapping rectangles.
        for _ in 0..64 {
            if started.elapsed() >= budget || identity(&current)? == root_id {
                break;
            }
            let Ok(parent) =
                (unsafe { current.accParent() }).and_then(|parent| parent.cast::<IAccessible>())
            else {
                break;
            };
            let parent_id = identity(&parent)?;
            if parent_id == identity(&current)? {
                break;
            }
            if let Some(candidate) = bounds(&parent, &VARIANT::from(0i32)) {
                candidates.push(candidate);
            }
            current = parent;
            if parent_id == root_id {
                break;
            }
        }
        Ok(CaptureHitTest {
            candidates: candidate_chain(candidates, x, y),
            pending: started.elapsed() >= budget,
        })
    }
}
