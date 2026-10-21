#[cfg(target_os = "macos")]
use rio_backend::sugarloaf::font::drag_image::DragImage;
#[cfg(target_os = "macos")]
struct Pointer {
    position: rio_window::dpi::PhysicalPosition<f64>,
    down: bool,
    escape: bool,
}
#[cfg(target_os = "macos")]
#[path = "../src/platform/tab_drag_macos.rs"]
mod native;

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn main() {
    let event_loop = rio_window::event_loop::EventLoop::new().unwrap();
    let source = event_loop
        .create_window(
            rio_window::window::Window::default_attributes().with_visible(false),
        )
        .unwrap();
    native::verify_preview_lifecycle(&source);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
