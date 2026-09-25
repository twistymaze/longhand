//! A window that cannot maximize does not zoom (macOS).
//!
//! `.maximizable(false)` disables the window's zoom button, but a
//! double-click on the title bar still sends `zoom:` to the window, and the
//! window then fills the screen. Before `zoom:` changes the frame, AppKit
//! asks the window delegate `windowShouldZoom:toFrame:`. The delegate of
//! each window is tao's `TaoWindowDelegate`, which does not implement that
//! method. This module adds it to that class: the answer is the enabled
//! state of the window's zoom button. A window that can maximize (the main
//! window) thus zooms as before, and the Settings window does not zoom at
//! all, so nothing moves on the screen. The window stays resizable.

use std::ffi::CString;
use std::sync::Once;

use objc2::encode::{Encode, Encoding};
use objc2::ffi::class_addMethod;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::{msg_send, sel};

/// The delegate class of every tao window.
const TAO_DELEGATE_CLASS: &std::ffi::CStr = c"TaoWindowDelegate";

/// `NSWindowZoomButton` in `NSWindowButton`.
const ZOOM_BUTTON: usize = 2;

/// Add `windowShouldZoom:toFrame:` to tao's window delegate class, once.
/// Call it before the build of a window that must not zoom: AppKit can read
/// the methods of a delegate when the window gets it. The class exists
/// after the first tao window is built.
pub fn stop_zoom_of_windows_that_cannot_maximize() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| match AnyClass::get(TAO_DELEGATE_CLASS) {
        Some(class) => {
            if !add_should_zoom(class) {
                log::warn!("the window delegate already answers windowShouldZoom:toFrame:");
            }
        }
        None => log::warn!("the tao window delegate class was not found; a title-bar double-click can zoom the Settings window"),
    });
}

/// Add `windowShouldZoom:toFrame:` to `class`. False when `class` already
/// has the method; the method then does not change.
fn add_should_zoom(class: &AnyClass) -> bool {
    let types = CString::new(format!(
        "{}{}{}{}{}",
        Bool::ENCODING,
        <*mut AnyObject>::ENCODING,
        Sel::ENCODING,
        <*mut AnyObject>::ENCODING,
        Rect::ENCODING
    ))
    .expect("an encoding has no NUL byte");
    type ShouldZoom =
        unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject, Rect) -> Bool;
    let imp: ShouldZoom = should_zoom;
    // SAFETY: `imp` has the signature of `windowShouldZoom:toFrame:`, and
    // `types` is its encoding. The runtime calls it with those arguments
    // only. The runtime copies `types`.
    unsafe {
        class_addMethod(
            (class as *const AnyClass).cast_mut(),
            sel!(windowShouldZoom:toFrame:),
            std::mem::transmute::<ShouldZoom, Imp>(imp),
            types.as_ptr(),
        )
    }
    .as_bool()
}

/// `windowShouldZoom:toFrame:`: a window zooms only when its zoom button is
/// enabled. A window with no zoom button zooms, as it does with no delegate
/// method. The function cannot panic: a panic here would unwind into
/// AppKit.
unsafe extern "C-unwind" fn should_zoom(
    _this: *mut AnyObject,
    _cmd: Sel,
    window: *mut AnyObject,
    _frame: Rect,
) -> Bool {
    if window.is_null() {
        return Bool::YES;
    }
    // SAFETY: `window` is the NSWindow that asks.
    let button: *mut AnyObject = unsafe { msg_send![window, standardWindowButton: ZOOM_BUTTON] };
    if button.is_null() {
        return Bool::YES;
    }
    // SAFETY: `button` is the window's NSButton, which the window keeps.
    let enabled: Bool = unsafe { msg_send![button, isEnabled] };
    enabled
}

/// `CGPoint`, `CGSize` and `CGRect`, for the argument that the method gets.
#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    origin: Point,
    size: Size,
}

// SAFETY: each struct is `repr(C)` with the fields that its encoding names.
unsafe impl Encode for Point {
    const ENCODING: Encoding = Encoding::Struct("CGPoint", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for Size {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl Encode for Rect {
    const ENCODING: Encoding = Encoding::Struct("CGRect", &[Point::ENCODING, Size::ENCODING]);
}

#[cfg(test)]
mod tests {
    use objc2::ClassType;
    use objc2::rc::Retained;
    use objc2::runtime::{ClassBuilder, NSObject};

    use super::*;

    /// A class with no methods of its own, as tao's delegate class has no
    /// `windowShouldZoom:toFrame:`.
    fn new_class(name: &std::ffi::CStr) -> &'static AnyClass {
        ClassBuilder::new(name, NSObject::class())
            .expect("the test class name is free")
            .register()
    }

    /// A stand-in for an NSButton whose `isEnabled` is `enabled`.
    fn button_class(name: &std::ffi::CStr, enabled: bool) -> &'static AnyClass {
        extern "C-unwind" fn yes(_this: *mut AnyObject, _cmd: Sel) -> Bool {
            Bool::YES
        }
        extern "C-unwind" fn no(_this: *mut AnyObject, _cmd: Sel) -> Bool {
            Bool::NO
        }
        let mut builder = ClassBuilder::new(name, NSObject::class()).expect("free name");
        let imp: extern "C-unwind" fn(*mut AnyObject, Sel) -> Bool = if enabled { yes } else { no };
        // SAFETY: the signature matches `isEnabled`.
        unsafe { builder.add_method(sel!(isEnabled), imp) };
        builder.register()
    }

    /// A stand-in for an NSWindow whose zoom button is an instance of
    /// `button`. The button is kept for the rest of the test run.
    fn window_with(name: &std::ffi::CStr, button: &'static AnyClass) -> Retained<AnyObject> {
        let button: Retained<AnyObject> = unsafe { msg_send![button, new] };
        let button = Retained::into_raw(button);
        BUTTONS.with(|b| b.set(button));
        extern "C-unwind" fn standard_window_button(
            _this: *mut AnyObject,
            _cmd: Sel,
            _which: usize,
        ) -> *mut AnyObject {
            BUTTONS.with(|b| b.get())
        }
        let mut builder = ClassBuilder::new(name, NSObject::class()).expect("free name");
        // SAFETY: the signature matches `standardWindowButton:`.
        unsafe {
            builder.add_method(
                sel!(standardWindowButton:),
                standard_window_button
                    as extern "C-unwind" fn(*mut AnyObject, Sel, usize) -> *mut AnyObject,
            )
        };
        let class = builder.register();
        unsafe { msg_send![class, new] }
    }

    thread_local! {
        static BUTTONS: std::cell::Cell<*mut AnyObject> = const { std::cell::Cell::new(std::ptr::null_mut()) };
    }

    fn should_zoom_of(delegate: &AnyObject, window: *const AnyObject) -> bool {
        let frame = Rect {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 100.0,
                height: 100.0,
            },
        };
        let answer: Bool = unsafe { msg_send![delegate, windowShouldZoom: window, toFrame: frame] };
        answer.as_bool()
    }

    #[test]
    fn a_window_with_a_disabled_zoom_button_does_not_zoom() {
        let delegate_class = new_class(c"LonghandZoomTestDelegateA");
        assert!(add_should_zoom(delegate_class));
        let delegate: Retained<AnyObject> = unsafe { msg_send![delegate_class, new] };
        let window = window_with(
            c"LonghandZoomTestWindowA",
            button_class(c"LonghandZoomTestButtonA", false),
        );
        assert!(!should_zoom_of(&delegate, &*window));
    }

    #[test]
    fn a_window_with_an_enabled_zoom_button_zooms() {
        let delegate_class = new_class(c"LonghandZoomTestDelegateB");
        assert!(add_should_zoom(delegate_class));
        let delegate: Retained<AnyObject> = unsafe { msg_send![delegate_class, new] };
        let window = window_with(
            c"LonghandZoomTestWindowB",
            button_class(c"LonghandZoomTestButtonB", true),
        );
        assert!(should_zoom_of(&delegate, &*window));
    }

    #[test]
    fn a_window_with_no_zoom_button_zooms() {
        let delegate_class = new_class(c"LonghandZoomTestDelegateC");
        assert!(add_should_zoom(delegate_class));
        let delegate: Retained<AnyObject> = unsafe { msg_send![delegate_class, new] };
        assert!(should_zoom_of(&delegate, std::ptr::null()));
    }

    #[test]
    fn a_second_add_changes_nothing() {
        let delegate_class = new_class(c"LonghandZoomTestDelegateD");
        assert!(!delegate_class.responds_to(sel!(windowShouldZoom:toFrame:)));
        assert!(add_should_zoom(delegate_class));
        assert!(delegate_class.responds_to(sel!(windowShouldZoom:toFrame:)));
        assert!(!add_should_zoom(delegate_class));
    }
}
