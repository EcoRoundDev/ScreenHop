use anyhow::{ensure, Result};
use std::cell::Cell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::rc::Rc;

use crate::{MouseEvent, Point};

type EventTapCallback =
    unsafe extern "C" fn(*const c_void, u32, *const c_void, *mut c_void) -> *const c_void;

extern "C" {
    fn pthread_main_np() -> i32;
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: EventTapCallback,
        user_info: *mut c_void,
    ) -> *const c_void;
    fn CGEventTapEnable(tap: *const c_void, enable: bool);
    fn CGEventTapIsEnabled(tap: *const c_void) -> bool;
    fn CGEventGetIntegerValueField(event: *const c_void, field: u32) -> i64;
    fn CGEventGetLocation(event: *const c_void) -> core_graphics::geometry::CGPoint;
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: *const c_void,
        order: isize,
    ) -> *const c_void;
    fn CFMachPortIsValid(port: *const c_void) -> u8;
    fn CFMachPortInvalidate(port: *const c_void);
    fn CFRunLoopGetCurrent() -> *const c_void;
    fn CFRunLoopAddSource(rl: *const c_void, source: *const c_void, mode: *const c_void);
    fn CFRunLoopRemoveSource(rl: *const c_void, source: *const c_void, mode: *const c_void);
    fn CFRunLoopSourceInvalidate(source: *const c_void);
    fn CFRetain(value: *const c_void) -> *const c_void;
    fn CFRelease(value: *const c_void);
    static kCFRunLoopCommonModes: *const c_void;
}

struct CallbackContext {
    callback: Box<dyn Fn(MouseEvent) -> bool + Send>,
    disabled_reason: Cell<Option<u32>>,
    panicked: Cell<bool>,
}

pub struct MacMouseHook {
    tap: *const c_void,
    source: *const c_void,
    run_loop: *const c_void,
    context: Option<Box<CallbackContext>>,
    main_thread: PhantomData<Rc<()>>,
}

impl MacMouseHook {
    pub fn new() -> Self {
        Self {
            tap: ptr::null(),
            source: ptr::null(),
            run_loop: ptr::null(),
            context: None,
            main_thread: PhantomData,
        }
    }

    pub fn install_event_tap<F>(&mut self, callback: F) -> Result<()>
    where
        F: Fn(MouseEvent) -> bool + Send + 'static,
    {
        ensure!(
            unsafe { pthread_main_np() } != 0,
            "鼠标钩子必须在主线程安装"
        );
        if self.is_installed() {
            return Ok(());
        }
        self.stop();
        self.context = Some(Box::new(CallbackContext {
            callback: Box::new(callback),
            disabled_reason: Cell::new(None),
            panicked: Cell::new(false),
        }));
        let user_info = self.context.as_deref_mut().unwrap() as *mut CallbackContext;
        unsafe {
            self.tap = CGEventTapCreate(
                0,
                0,
                0,
                (1 << 25) | (1 << 26),
                event_tap_callback,
                user_info.cast(),
            );
            if self.tap.is_null() {
                self.stop();
                anyhow::bail!("CGEventTapCreate 失败 — 请确认已授予辅助功能权限");
            }
            self.source = CFMachPortCreateRunLoopSource(ptr::null(), self.tap, 0);
            if self.source.is_null() {
                self.stop();
                anyhow::bail!("CFMachPortCreateRunLoopSource 失败");
            }
            self.run_loop = CFRetain(CFRunLoopGetCurrent());
            CFRunLoopAddSource(self.run_loop, self.source, kCFRunLoopCommonModes);
            CGEventTapEnable(self.tap, true);
        }
        if !self.is_enabled() {
            self.stop();
            anyhow::bail!("CGEventTap 创建后未能启用");
        }
        log::info!("CGEventTap 已安装到 RunLoop");
        Ok(())
    }

    pub fn stop(&mut self) {
        if self.tap.is_null() && self.source.is_null() && self.context.is_none() {
            return;
        }
        assert!(
            unsafe { pthread_main_np() } != 0,
            "鼠标钩子必须在主线程停止"
        );
        unsafe {
            if !self.tap.is_null() {
                CGEventTapEnable(self.tap, false);
            }
            if !self.source.is_null() {
                if !self.run_loop.is_null() {
                    CFRunLoopRemoveSource(self.run_loop, self.source, kCFRunLoopCommonModes);
                }
                CFRunLoopSourceInvalidate(self.source);
            }
            if !self.tap.is_null() {
                CFMachPortInvalidate(self.tap);
            }
            if !self.source.is_null() {
                CFRelease(self.source);
            }
            if !self.tap.is_null() {
                CFRelease(self.tap);
            }
            if !self.run_loop.is_null() {
                CFRelease(self.run_loop);
            }
        }
        self.tap = ptr::null();
        self.source = ptr::null();
        self.run_loop = ptr::null();
        self.context = None;
    }

    pub fn is_installed(&self) -> bool {
        !self.tap.is_null()
            && !self.source.is_null()
            && !self.run_loop.is_null()
            && unsafe { CFMachPortIsValid(self.tap) != 0 }
    }

    pub fn is_enabled(&self) -> bool {
        self.is_installed()
            && self
                .context
                .as_ref()
                .is_some_and(|context| !context.panicked.get())
            && unsafe { CGEventTapIsEnabled(self.tap) }
    }

    pub fn take_disabled_reason(&self) -> Option<u32> {
        self.context
            .as_ref()
            .and_then(|context| context.disabled_reason.take())
    }

    pub fn reenable(&mut self) -> Result<()> {
        ensure!(
            unsafe { pthread_main_np() } != 0,
            "鼠标钩子必须在主线程恢复"
        );
        ensure!(self.is_installed(), "鼠标钩子尚未安装或已失效");
        ensure!(
            self.context
                .as_ref()
                .is_some_and(|context| !context.panicked.get()),
            "鼠标钩子回调发生 panic，需停止后重新安装"
        );
        unsafe { CGEventTapEnable(self.tap, true) };
        ensure!(self.is_enabled(), "CGEventTap 恢复失败");
        Ok(())
    }
}

impl Drop for MacMouseHook {
    fn drop(&mut self) {
        self.stop();
    }
}

unsafe extern "C" fn event_tap_callback(
    _proxy: *const c_void,
    event_type: u32,
    event: *const c_void,
    user_info: *mut c_void,
) -> *const c_void {
    let Some(context) = user_info.cast::<CallbackContext>().as_ref() else {
        return event;
    };
    let result = catch_unwind(AssertUnwindSafe(|| {
        if event_type == 0xFFFFFFFE || event_type == 0xFFFFFFFF {
            context.disabled_reason.set(Some(event_type));
            return event;
        }
        if context.panicked.get() || event_type != 25 || event.is_null() {
            return event;
        }
        if CGEventGetIntegerValueField(event, 3) != 2 {
            return event;
        }
        let location = CGEventGetLocation(event);
        let mouse_event = MouseEvent {
            point: Point {
                x: location.x,
                y: location.y,
            },
            button: 2,
        };
        if (context.callback)(mouse_event) {
            ptr::null()
        } else {
            event
        }
    }));
    match result {
        Ok(event) => event,
        Err(payload) => {
            context.panicked.set(true);
            std::mem::forget(payload);
            event
        }
    }
}
