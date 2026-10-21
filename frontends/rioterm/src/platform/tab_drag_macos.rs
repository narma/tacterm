use super::{DragImage, Pointer};
use objc2::{
    class, msg_send,
    runtime::{AnyObject, Bool},
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use rio_window::{dpi::PhysicalPosition, window::Window};
use std::{io, ptr};

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceKeyState(state: i32, key: u16) -> bool;
}

pub struct Drag {
    panel: *mut AnyObject,
    point: NSPoint,
    grab: f64,
    height: f64,
}

fn view(window: &Window) -> Option<*mut AnyObject> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::AppKit(h) => Some(h.ns_view.as_ptr().cast()),
        _ => None,
    }
}

impl Drag {
    pub fn new(window: &Window, image: &DragImage, grab: f32) -> io::Result<Self> {
        let scale = window.scale_factor();
        let size = NSSize::new(image.width as f64 / scale, image.height as f64 / scale);
        // SAFETY: All AppKit operations run on the event-loop main thread.
        // Allocated objects are released after ownership passes to the view;
        // the panel is owned by Drag and never accepts keyboard or mouse input.
        unsafe {
            let allocated: *mut AnyObject = msg_send![class!(NSPanel), alloc];
            let panel: *mut AnyObject = msg_send![allocated, initWithContentRect: NSRect::new(NSPoint::new(0.0,0.0), size)
                styleMask: 128usize backing: 2usize defer: Bool::NO];
            if panel.is_null() {
                return Err(io::Error::other("could not create tab preview panel"));
            }
            let mut drag = Self {
                panel,
                point: NSPoint::new(0.0, 0.0),
                grab: grab as f64 / scale,
                height: size.height,
            };
            let _: () = msg_send![panel, setReleasedWhenClosed: Bool::NO];
            let _: () = msg_send![panel, setIgnoresMouseEvents: Bool::YES];
            let _: () = msg_send![panel, setHidesOnDeactivate: Bool::NO];
            let _: () = msg_send![panel, setCollectionBehavior: 257usize]; // all Spaces + fullscreen auxiliary
            let _: () = msg_send![panel, setLevel: 500isize]; // dragging-window level
            let _: () = msg_send![panel, setAlphaValue: 0.93f64];
            let rep: *mut AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
            let color = NSString::from_str("NSDeviceRGBColorSpace");
            let rep: *mut AnyObject = msg_send![rep, initWithBitmapDataPlanes: ptr::null_mut::<*mut u8>()
                pixelsWide: image.width as isize pixelsHigh: image.height as isize
                bitsPerSample: 8isize samplesPerPixel: 4isize hasAlpha: Bool::YES
                isPlanar: Bool::NO colorSpaceName: &*color bytesPerRow: (image.width*4) as isize bitsPerPixel: 32isize];
            if rep.is_null() {
                return Err(io::Error::other("could not allocate tab preview bitmap"));
            }
            let pixels: *mut u8 = msg_send![rep, bitmapData];
            if pixels.is_null() {
                let _: () = msg_send![rep, release];
                return Err(io::Error::other("missing preview bitmap storage"));
            }
            ptr::copy_nonoverlapping(image.rgba.as_ptr(), pixels, image.rgba.len());
            let nsimage: *mut AnyObject = msg_send![class!(NSImage), alloc];
            let nsimage: *mut AnyObject = msg_send![nsimage, initWithSize: size];
            let _: () = msg_send![nsimage, addRepresentation: rep];
            let _: () = msg_send![rep, release];
            let image_view: *mut AnyObject = msg_send![class!(NSImageView), alloc];
            let image_view: *mut AnyObject = msg_send![image_view, initWithFrame: NSRect::new(NSPoint::new(0.0,0.0), size)];
            let _: () = msg_send![image_view, setImage: nsimage];
            let _: () = msg_send![nsimage, release];
            let _: () = msg_send![panel, setContentView: image_view];
            let _: () = msg_send![image_view, release];
            drag.sample(window)?;
            Ok(drag)
        }
    }
    fn local(&self, window: &Window) -> Option<PhysicalPosition<f64>> {
        let view = view(window)?;
        // SAFETY: Borrowed view and window remain alive during this call.
        unsafe {
            let native: *mut AnyObject = msg_send![view, window];
            let point: NSPoint = msg_send![native, convertPointFromScreen: self.point];
            let point: NSPoint =
                msg_send![view, convertPoint: point fromView: ptr::null::<AnyObject>()];
            let bounds: NSRect = msg_send![view, bounds];
            let flipped: Bool = msg_send![view, isFlipped];
            let y = if flipped.as_bool() {
                point.y
            } else {
                bounds.size.height - point.y
            };
            Some(PhysicalPosition::new(
                point.x * window.scale_factor(),
                y * window.scale_factor(),
            ))
        }
    }
    pub fn sample(&mut self, window: &Window) -> io::Result<Option<Pointer>> {
        // SAFETY: Class methods only read global input; they do not inject it.
        unsafe {
            self.point = msg_send![class!(NSEvent), mouseLocation];
            let buttons: usize = msg_send![class!(NSEvent), pressedMouseButtons];
            Ok(Some(Pointer {
                position: self
                    .local(window)
                    .ok_or_else(|| io::Error::other("missing Cocoa view"))?,
                down: buttons & 1 != 0,
                escape: CGEventSourceKeyState(1, 53),
            }))
        }
    }
    pub fn show(&mut self, visible: bool) -> io::Result<()> {
        // SAFETY: This is an independent non-activating panel, never an owned
        // child of the terminal. Ordering it cannot raise the source window.
        unsafe {
            if visible {
                let _: () = msg_send![self.panel, setFrameOrigin: NSPoint::new(self.point.x-self.grab, self.point.y-self.height/2.0)];
                let _: () = msg_send![self.panel, orderFrontRegardless];
            } else {
                let _: () = msg_send![self.panel, orderOut: ptr::null::<AnyObject>()];
            }
        }
        Ok(())
    }
    pub fn target_position(&self, window: &Window) -> Option<PhysicalPosition<f64>> {
        let view = view(window)?;
        // SAFETY: Read native stacking at the pointer after hiding the preview.
        unsafe {
            let native: *mut AnyObject = msg_send![view, window];
            let id: isize = msg_send![native, windowNumber];
            let hit: isize = msg_send![class!(NSWindow), windowNumberAtPoint: self.point belowWindowWithWindowNumber: 0isize];
            if id != hit {
                return None;
            }
        }
        self.local(window)
    }
}
impl Drop for Drag {
    fn drop(&mut self) {
        // SAFETY: Event-loop thread owns this non-released-on-close panel.
        unsafe {
            let _: () = msg_send![self.panel, close];
            let _: () = msg_send![self.panel, release];
        }
    }
}

#[cfg(test)]
// Called by the main-thread integration harness; unused in the binary's tests.
#[allow(dead_code)]
pub(super) fn verify_preview_lifecycle(window: &Window) {
    let image = DragImage {
        rgba: [12, 34, 56, 255].repeat(240 * 24),
        width: 240,
        height: 24,
    };
    let mut drag = Drag::new(window, &image, 20.0).unwrap();
    // SAFETY: Dedicated test executable runs on the AppKit main thread. The
    // popup remains off-screen and no keyboard or pointer input is generated.
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let before: *mut AnyObject = msg_send![app, keyWindow];
        drag.point = NSPoint::new(-30000.0, -30000.0);
        drag.show(true).unwrap();
        let frame: NSRect = msg_send![drag.panel, frame];
        assert_eq!(frame.size.width, 240.0 / window.scale_factor());
        assert_eq!(frame.size.height, 24.0 / window.scale_factor());
        let ignores: Bool = msg_send![drag.panel, ignoresMouseEvents];
        assert!(ignores.as_bool());
        drag.point.x += 20.0;
        drag.show(true).unwrap();
        let after: *mut AnyObject = msg_send![app, keyWindow];
        assert_eq!(before, after, "drag preview changed key window");
        drag.show(false).unwrap();
        let visible: Bool = msg_send![drag.panel, isVisible];
        assert!(!visible.as_bool());
    }
}
