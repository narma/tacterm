//! Desktop pointer tracking for Cocoa and X11. Wayland uses compositor DnD.
use rio_backend::sugarloaf::font::drag_image::DragImage;
use rio_window::{dpi::PhysicalPosition, window::Window};
use std::io;

#[cfg(target_os = "macos")]
#[path = "tab_drag_macos.rs"]
mod native;
#[cfg(all(target_os = "linux", feature = "x11"))]
#[path = "tab_drag_x11.rs"]
mod native;

pub struct Pointer {
    pub position: PhysicalPosition<f64>,
    pub down: bool,
    pub escape: bool,
}

pub struct DesktopDrag {
    #[cfg(any(target_os = "macos", feature = "x11"))]
    inner: native::Drag,
}

// Wayland-only builds have no desktop pointer backend; all samples arrive as
// compositor events. Keep the common frontend API without linking X11.
#[cfg_attr(
    all(target_os = "linux", not(feature = "x11")),
    allow(unused_variables)
)]
impl DesktopDrag {
    pub fn new(
        window: &Window,
        image: &DragImage,
        grab: f32,
    ) -> io::Result<Option<Self>> {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if matches!(
            window.window_handle().map_err(io::Error::other)?.as_raw(),
            RawWindowHandle::Wayland(_)
        ) {
            return Ok(None);
        }
        #[cfg(any(target_os = "macos", feature = "x11"))]
        {
            return native::Drag::new(window, image, grab)
                .map(|inner| Some(Self { inner }));
        }
        #[allow(unreachable_code)]
        Err(io::Error::other("desktop tab drag backend unavailable"))
    }
    pub fn sample(&mut self, window: &Window) -> io::Result<Option<Pointer>> {
        #[cfg(any(target_os = "macos", feature = "x11"))]
        {
            return self.inner.sample(window);
        }
        #[allow(unreachable_code)]
        Err(io::Error::other("desktop tab drag backend unavailable"))
    }
    pub fn show(&mut self, visible: bool) -> io::Result<()> {
        #[cfg(any(target_os = "macos", feature = "x11"))]
        {
            return self.inner.show(visible);
        }
        #[allow(unreachable_code)]
        Ok(())
    }
    pub fn target_position(&self, window: &Window) -> Option<PhysicalPosition<f64>> {
        #[cfg(any(target_os = "macos", feature = "x11"))]
        {
            return self.inner.target_position(window);
        }
        #[allow(unreachable_code)]
        None
    }
}
