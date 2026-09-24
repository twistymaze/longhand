//! Suspend and resume through `PowerRegisterSuspendResumeNotification`,
//! which calls back on a system thread without the hidden window a
//! `WM_POWERBROADCAST` listener needs.

use std::ffi::c_void;

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, HPOWERNOTIFY, PowerRegisterSuspendResumeNotification,
    PowerUnregisterSuspendResumeNotification,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DEVICE_NOTIFY_CALLBACK, PBT_APMRESUMEAUTOMATIC, PBT_APMSUSPEND,
};

use super::{PowerCallback, PowerEvent};

pub struct Watch {
    handle: HPOWERNOTIFY,
    /// Owned here and freed only after the registration is removed; the
    /// system holds a pointer to it until then.
    context: *mut PowerCallback,
    parameters: *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS,
}

// The raw pointers are this watch's own allocations, touched by the system's
// callback thread only between register and unregister.
unsafe impl Send for Watch {}

unsafe extern "system" fn notify(
    context: *const c_void,
    kind: u32,
    _setting: *const c_void,
) -> u32 {
    let callback = unsafe { &*(context as *const PowerCallback) };
    match kind {
        PBT_APMSUSPEND => callback(PowerEvent::WillSleep),
        PBT_APMRESUMEAUTOMATIC => callback(PowerEvent::DidWake),
        _ => {}
    }
    ERROR_SUCCESS
}

impl Watch {
    pub fn start(callback: PowerCallback) -> Option<Self> {
        let context = Box::into_raw(Box::new(callback));
        let parameters = Box::into_raw(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(notify),
            Context: context.cast(),
        }));
        let mut handle: *mut c_void = std::ptr::null_mut();
        let status = unsafe {
            PowerRegisterSuspendResumeNotification(
                DEVICE_NOTIFY_CALLBACK,
                parameters.cast(),
                &mut handle,
            )
        };
        if status != ERROR_SUCCESS {
            log::warn!("suspend notifications unavailable: error {status}");
            unsafe {
                drop(Box::from_raw(parameters));
                drop(Box::from_raw(context));
            }
            return None;
        }
        Some(Self {
            handle: handle as HPOWERNOTIFY,
            context,
            parameters,
        })
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        unsafe {
            PowerUnregisterSuspendResumeNotification(self.handle);
            drop(Box::from_raw(self.parameters));
            drop(Box::from_raw(self.context));
        }
    }
}
