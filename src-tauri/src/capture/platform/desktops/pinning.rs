//! Window-scoped Shell pinning. No application-wide pinning or desktop switching.
use super::Apartment;
use std::ffi::c_void;
use windows::{
    core::{IUnknown, IUnknown_Vtbl, Interface, GUID, HRESULT},
    Win32::{
        Foundation::{E_POINTER, HWND},
        System::Com::IServiceProvider,
        System::Com::{CoCreateInstance, CLSCTX_LOCAL_SERVER},
    },
};

#[repr(transparent)]
#[derive(Clone)]
struct ApplicationViews(IUnknown);
// This interface is used only through the verified ABI slots declared below.
unsafe impl Interface for ApplicationViews {
    type Vtable = ApplicationViewsVtbl;
    const IID: GUID = GUID::from_u128(0x1841c6d7_4f9d_42c0_af41_8747538f10e5);
}
#[repr(C)]
pub struct ApplicationViewsVtbl {
    base: IUnknown_Vtbl,
    // Unused ABI slots remain in their original positions.
    get_views: usize,
    get_views_by_z_order: usize,
    get_views_by_app_id: usize,
    get_view_for_hwnd:
        unsafe extern "system" fn(*mut c_void, HWND, *mut Option<IUnknown>) -> HRESULT,
}

#[repr(transparent)]
#[derive(Clone)]
struct PinnedViews(IUnknown);
// This interface is used only through the verified ABI slots declared below.
unsafe impl Interface for PinnedViews {
    type Vtable = PinnedViewsVtbl;
    const IID: GUID = GUID::from_u128(0x4ce81583_1e4c_4632_a621_07a53543148f);
}
#[repr(C)]
pub struct PinnedViewsVtbl {
    base: IUnknown_Vtbl,
    is_app_id_pinned: usize,
    pin_app_id: usize,
    unpin_app_id: usize,
    is_view_pinned:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut windows::core::BOOL) -> HRESULT,
    pin_view: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
    unpin_view: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

pub(super) struct Pinning {
    view: IUnknown,
    pinned: PinnedViews,
    // Interfaces must be released before leaving this apartment.
    _apartment: Apartment,
}
impl Pinning {
    pub(super) fn for_window(hwnd: HWND) -> Result<Self, String> {
        let apartment = Apartment::enter()?;
        let shell: IServiceProvider = unsafe {
            CoCreateInstance(
                &GUID::from_u128(0xc2f03a33_21f5_47fa_b4bb_156362a2f239),
                None,
                CLSCTX_LOCAL_SERVER,
            )
        }
        .map_err(|e| e.to_string())?;
        let views: ApplicationViews =
            unsafe { shell.QueryService(&ApplicationViews::IID) }.map_err(|e| e.to_string())?;
        let pinned: PinnedViews =
            unsafe { shell.QueryService(&GUID::from_u128(0xb5a399e7_1c87_46b8_88e9_fc5747b171bd)) }
                .map_err(|e| e.to_string())?;
        let mut view = None;
        unsafe { (views.vtable().get_view_for_hwnd)(views.as_raw(), hwnd, &mut view) }
            .ok()
            .map_err(|e| e.to_string())?;
        let view = view.ok_or_else(|| windows::core::Error::from_hresult(E_POINTER).to_string())?;
        Ok(Self {
            view,
            pinned,
            _apartment: apartment,
        })
    }

    pub(super) fn is_pinned(&self) -> Result<bool, String> {
        let mut pinned = windows::core::BOOL::default();
        unsafe {
            (self.pinned.vtable().is_view_pinned)(
                self.pinned.as_raw(),
                self.view.as_raw(),
                &mut pinned,
            )
        }
        .ok()
        .map_err(|e| e.to_string())?;
        Ok(pinned.as_bool())
    }

    pub(super) fn set_pinned(&self, pinned: bool) -> Result<(), String> {
        if self.is_pinned()? == pinned {
            return Ok(());
        }
        unsafe {
            let method = if pinned {
                self.pinned.vtable().pin_view
            } else {
                self.pinned.vtable().unpin_view
            };
            method(self.pinned.as_raw(), self.view.as_raw())
        }
        .ok()
        .map_err(|e| e.to_string())?;
        if self.is_pinned()? != pinned {
            return Err("系统未能更新贴图的桌面归属".into());
        }
        Ok(())
    }
}
