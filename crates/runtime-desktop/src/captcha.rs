//! Lets clicks reach Desktop Phoenix's hCaptcha on macOS.
//!
//! Build 0.1.0 (2233) embeds the captcha as a 303×78 `WryWebView` inside the
//! main `GPUIWindow`. `AppKit`'s hit test picks the web view, but its
//! `mouseDown:` and `mouseUp:` are never called, so the checkbox cannot be
//! clicked and every login ends as `cancelled`. This hooks the window's
//! `sendEvent:` and hands mouse events over a `WKWebView` to it directly.

use std::{
   ffi::{
      CStr,
      c_char,
      c_void,
   },
   sync::atomic::{
      AtomicBool,
      AtomicUsize,
      Ordering,
   },
   thread,
   time::Duration,
};

type Id = *mut c_void;
type Sel = *const c_void;
type SendEvent = unsafe extern "C" fn(Id, Sel, Id);

/// How often the main thread looks for `GPUIWindow` until it exists.
const POLL: Duration = Duration::from_millis(500);
/// `NSEventType` values from `NSEvent.h`.
const LEFT_DOWN: usize = 1;
const LEFT_UP: usize = 2;
const RIGHT_DOWN: usize = 3;
const RIGHT_UP: usize = 4;
const MOVED: usize = 5;
const LEFT_DRAGGED: usize = 6;
const RIGHT_DRAGGED: usize = 7;

#[link(name = "objc", kind = "dylib")]
unsafe extern "C" {
   fn objc_getClass(name: *const c_char) -> Id;
   fn sel_registerName(name: *const c_char) -> Sel;
   fn objc_msgSend();
   fn class_replaceMethod(
      class: Id,
      name: Sel,
      imp: *const c_void,
      types: *const c_char,
   ) -> *const c_void;
   fn class_getSuperclass(class: Id) -> Id;
   fn class_getMethodImplementation(class: Id, name: Sel) -> *const c_void;
}

unsafe extern "C" {
   static _dispatch_main_q: u8;
   fn dispatch_async_f(
      queue: *const c_void,
      context: *mut c_void,
      work: extern "C" fn(*mut c_void),
   );
}

/// Set once `sendEvent:` is hooked, which ends the watcher.
static INSTALLED: AtomicBool = AtomicBool::new(false);
/// Set while a check is queued, so a busy main thread never piles them up.
static QUEUED: AtomicBool = AtomicBool::new(false);
/// `GPUIWindow`'s own `sendEvent:`, or `NSWindow`'s when it has none.
static ORIGINAL: AtomicUsize = AtomicUsize::new(0);
/// The retained web view a press went to, which gets the drags and the
/// release even when they leave it.
static CAPTURED: AtomicUsize = AtomicUsize::new(0);
/// The retained first responder from before a press gave the web view focus,
/// given focus back on the next press outside it.
static RESTORE: AtomicUsize = AtomicUsize::new(0);

#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
   x: f64,
   y: f64,
}

/// Starts waiting for GPUI's window class, then hooks it from the main
/// thread.
pub fn install() {
   let spawned = thread::Builder::new()
      .name("phoenix-captcha".to_owned())
      .spawn(|| {
         while !INSTALLED.load(Ordering::Acquire) {
            thread::sleep(POLL);
            if !QUEUED.swap(true, Ordering::AcqRel) {
               // SAFETY: the main queue lives for the whole process and `hook`
               // ignores its context.
               unsafe {
                  dispatch_async_f(
                     (&raw const _dispatch_main_q).cast(),
                     std::ptr::null_mut(),
                     hook,
                  );
               }
            }
         }
      });
   if let Err(err) = spawned {
      log::warn!("captcha clicks: could not start the watcher: {err}");
   }
}

fn sel(name: &CStr) -> Sel {
   // SAFETY: `name` is NUL-terminated and selectors are never freed.
   unsafe { sel_registerName(name.as_ptr()) }
}

/// Sends `name` with no arguments.
///
/// # Safety
///
/// `obj` is a live object or nil, and `R` is the method's return type.
unsafe fn send<R>(obj: Id, name: &CStr) -> R {
   // SAFETY: `objc_msgSend` takes the receiver, the selector and the method's
   // own arguments, and returns what the method returns; on arm64 every
   // signature goes through the one entry point.
   let f: unsafe extern "C" fn(Id, Sel) -> R =
      unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
   // SAFETY: the caller's contract.
   unsafe { f(obj, sel(name)) }
}

/// Sends `name` with one argument.
///
/// # Safety
///
/// `obj` is a live object or nil, and `A` and `R` are the method's argument
/// and return types.
unsafe fn send1<A, R>(obj: Id, name: &CStr, arg: A) -> R {
   // SAFETY: as in `send`.
   let f: unsafe extern "C" fn(Id, Sel, A) -> R =
      unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
   // SAFETY: the caller's contract.
   unsafe { f(obj, sel(name), arg) }
}

/// Hooks `GPUIWindow`'s `sendEvent:` once the class exists. Runs on the main
/// queue, so no event is being sent while the method changes.
extern "C" fn hook(_context: *mut c_void) {
   QUEUED.store(false, Ordering::Release);
   if INSTALLED.load(Ordering::Acquire) {
      return;
   }
   // SAFETY: Objective-C runtime calls on a registered class, on the main
   // thread. `send_event` has `sendEvent:`'s `v@:@` signature.
   unsafe {
      let class = objc_getClass(c"GPUIWindow".as_ptr());
      if class.is_null() {
         return;
      }
      let name = sel(c"sendEvent:");
      let own = class_replaceMethod(class, name, send_event as *const c_void, c"v@:@".as_ptr());
      let original = if own.is_null() {
         class_getMethodImplementation(class_getSuperclass(class), name)
      } else {
         own
      };
      ORIGINAL.store(original.addr(), Ordering::Release);
      INSTALLED.store(true, Ordering::Release);
      log::info!(
         "captcha clicks: hooked GPUIWindow sendEvent: (its own: {})",
         !own.is_null()
      );
   }
}

/// `GPUIWindow`'s replacement `sendEvent:`.
unsafe extern "C" fn send_event(window: Id, cmd: Sel, event: Id) {
   // SAFETY: AppKit calls this on the main thread with a live window and
   // event.
   if unsafe { route(window, event) } {
      return;
   }
   let original = ORIGINAL.load(Ordering::Acquire);
   if original == 0 {
      return;
   }
   // SAFETY: `original` is the `sendEvent:` implementation `hook` replaced.
   unsafe {
      let original: SendEvent = std::mem::transmute(original);
      original(window, cmd, event);
   }
}

/// Hands `event` to a web view and returns true when it belongs to one.
///
/// # Safety
///
/// Main thread only, with a live `window` and `event`.
unsafe fn route(window: Id, event: Id) -> bool {
   // SAFETY: `NSEvent`, `NSWindow` and `NSView` methods on live objects, on
   // the main thread; captured views are retained until released here.
   unsafe {
      let kind: usize = send(event, c"type");
      match kind {
         LEFT_DOWN | RIGHT_DOWN => {
            let Some(web_view) = web_view_at(window, event) else {
               restore_focus(window);
               return false;
            };
            let responder: Id = send(window, c"firstResponder");
            if RESTORE.load(Ordering::Acquire) == 0 && !is_web_view(responder) {
               let _: Id = send(responder, c"retain");
               RESTORE.store(responder.addr(), Ordering::Release);
            }
            let _: bool = send1(window, c"makeFirstResponder:", web_view);
            let _: Id = send(web_view, c"retain");
            let previous = CAPTURED.swap(web_view.addr(), Ordering::AcqRel);
            if previous != 0 {
               send::<()>(previous as Id, c"release");
            }
            forward(web_view, kind, event);
            true
         },
         LEFT_DRAGGED | RIGHT_DRAGGED => {
            let captured = CAPTURED.load(Ordering::Acquire);
            if captured == 0 {
               return false;
            }
            forward(captured as Id, kind, event);
            true
         },
         LEFT_UP | RIGHT_UP => {
            let captured = CAPTURED.swap(0, Ordering::AcqRel);
            if captured == 0 {
               return false;
            }
            forward(captured as Id, kind, event);
            send::<()>(captured as Id, c"release");
            true
         },
         MOVED => {
            let Some(web_view) = web_view_at(window, event) else {
               return false;
            };
            forward(web_view, kind, event);
            true
         },
         _ => false,
      }
   }
}

/// Sends `event` to `web_view` as the `NSResponder` method for its type.
///
/// # Safety
///
/// Main thread only, with a live `web_view` and `event`.
unsafe fn forward(web_view: Id, kind: usize, event: Id) {
   let name = match kind {
      LEFT_DOWN => c"mouseDown:",
      LEFT_UP => c"mouseUp:",
      RIGHT_DOWN => c"rightMouseDown:",
      RIGHT_UP => c"rightMouseUp:",
      LEFT_DRAGGED => c"mouseDragged:",
      RIGHT_DRAGGED => c"rightMouseDragged:",
      _ => c"mouseMoved:",
   };
   // SAFETY: every `NSView` implements these `v@:@` responder methods.
   unsafe { send1::<Id, ()>(web_view, name, event) };
}

/// The web view under `event`'s location, if any.
///
/// # Safety
///
/// Main thread only, with a live `window` and `event`.
unsafe fn web_view_at(window: Id, event: Id) -> Option<Id> {
   // SAFETY: `NSWindow`, `NSView` and `NSEvent` methods on live objects, on
   // the main thread.
   unsafe {
      let content: Id = send(window, c"contentView");
      let frame_view: Id = send(content, c"superview");
      let root = if frame_view.is_null() {
         content
      } else {
         frame_view
      };
      if root.is_null() {
         return None;
      }
      let point: Point = send(event, c"locationInWindow");
      let mut view: Id = send1(root, c"hitTest:", point);
      while !view.is_null() {
         if is_web_view(view) {
            return Some(view);
         }
         view = send(view, c"superview");
      }
      None
   }
}

/// Whether `obj` is a `WKWebView`. False before `WebKit` is loaded.
///
/// # Safety
///
/// Main thread only, with `obj` a live object or nil.
unsafe fn is_web_view(obj: Id) -> bool {
   // SAFETY: `isKindOfClass:` on a live object or nil.
   unsafe {
      let class = objc_getClass(c"WKWebView".as_ptr());
      !class.is_null() && !obj.is_null() && send1::<Id, bool>(obj, c"isKindOfClass:", class)
   }
}

/// Gives focus back to what had it before a press went to a web view, when a
/// web view or nothing still has it.
///
/// # Safety
///
/// Main thread only, with a live `window`.
unsafe fn restore_focus(window: Id) {
   let saved = RESTORE.swap(0, Ordering::AcqRel);
   if saved == 0 {
      return;
   }
   // SAFETY: `saved` was retained when it was stored; `NSWindow` methods on
   // the main thread.
   unsafe {
      let responder: Id = send(window, c"firstResponder");
      if responder.is_null() || responder == window || is_web_view(responder) {
         let _: bool = send1(window, c"makeFirstResponder:", saved as Id);
      }
      send::<()>(saved as Id, c"release");
   }
}
