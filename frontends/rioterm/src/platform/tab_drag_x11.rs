use super::{DragImage, Pointer};
use crate::platform::tab_drag_poll::Poller;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use rio_window::{dpi::PhysicalPosition, window::Window};
use std::{io, os::fd::AsRawFd, sync::Arc};
use x11rb::{
    connection::Connection,
    protocol::{
        shape::{self, ConnectionExt as _},
        xproto::*,
    },
    rust_connection::RustConnection,
};

pub struct Drag {
    connection: Arc<RustConnection>,
    sampler: Option<Poller<Sample>>,
    targets: Vec<(u32, PhysicalPosition<f64>)>,
    preview: u32,
    gc: u32,
    pixmap: u32,
    grab: i16,
    half_height: i16,
    point: (i16, i16),
    visible: bool,
    escape_key: u8,
}

fn id(window: &Window) -> Option<u32> {
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => u32::try_from(h.window).ok(),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}

impl Drag {
    pub fn new(window: &Window, image: &DragImage, grab: f32) -> io::Result<Self> {
        let (connection, screen) = x11rb::connect(None).map_err(io::Error::other)?;
        let setup = connection.setup();
        let screen = &setup.roots[screen];
        let root = screen.root;
        let depth = screen.root_depth;
        let format = setup
            .pixmap_formats
            .iter()
            .find(|f| f.depth == depth)
            .ok_or_else(|| io::Error::other("missing X11 pixmap format"))?;
        if format.bits_per_pixel != 32 || setup.image_byte_order != ImageOrder::LSB_FIRST
        {
            return Err(io::Error::other(
                "tab preview requires little-endian 32-bit X11 pixels",
            ));
        }
        let preview = connection.generate_id().map_err(io::Error::other)?;
        let gc = connection.generate_id().map_err(io::Error::other)?;
        let pixmap = connection.generate_id().map_err(io::Error::other)?;
        connection
            .create_window(
                depth,
                preview,
                root,
                0,
                0,
                image.width as u16,
                image.height as u16,
                1,
                WindowClass::INPUT_OUTPUT,
                screen.root_visual,
                &CreateWindowAux::new()
                    .override_redirect(1)
                    .background_pixel(screen.black_pixel),
            )
            .map_err(io::Error::other)?
            .check()
            .map_err(io::Error::other)?;
        // Own resources immediately so every later error destroys the popup.
        let mut drag = Self {
            connection: Arc::new(connection),
            sampler: None,
            targets: Vec::new(),
            preview,
            gc,
            pixmap,
            grab: grab as i16,
            half_height: (image.height / 2) as i16,
            point: (0, 0),
            visible: false,
            escape_key: 9,
        };
        drag.connection
            .shape_rectangles(
                shape::SO::SET,
                shape::SK::INPUT,
                ClipOrdering::UNSORTED,
                preview,
                0,
                0,
                &[],
            )
            .map_err(io::Error::other)?
            .check()
            .map_err(io::Error::other)?;
        drag.connection
            .create_pixmap(
                depth,
                pixmap,
                preview,
                image.width as u16,
                image.height as u16,
            )
            .map_err(io::Error::other)?;
        drag.connection
            .change_window_attributes(
                preview,
                &ChangeWindowAttributesAux::new().background_pixmap(pixmap),
            )
            .map_err(io::Error::other)?;
        drag.connection
            .create_gc(gc, preview, &CreateGCAux::new())
            .map_err(io::Error::other)?;
        let mut pixels = image.rgba.clone();
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        // Bound each request below the server's minimum 64 KiB request limit.
        let rows = (60000 / (image.width as usize * 4)).max(1);
        for (index, chunk) in pixels.chunks(rows * image.width as usize * 4).enumerate() {
            drag.connection
                .put_image(
                    ImageFormat::Z_PIXMAP,
                    pixmap,
                    gc,
                    image.width as u16,
                    (chunk.len() / (image.width as usize * 4)) as u16,
                    0,
                    (index * rows) as i16,
                    0,
                    depth,
                    chunk,
                )
                .map_err(io::Error::other)?;
        }
        let setup = drag.connection.setup();
        let min = setup.min_keycode;
        let mapping = drag
            .connection
            .get_keyboard_mapping(min, setup.max_keycode - min + 1)
            .map_err(io::Error::other)?
            .reply()
            .map_err(io::Error::other)?;
        if let Some(index) = mapping
            .keysyms
            .chunks(mapping.keysyms_per_keycode as usize)
            .position(|keys| keys.contains(&0xff1b))
        {
            drag.escape_key = min + index as u8;
        }
        let connection = Arc::clone(&drag.connection);
        let source = id(window).ok_or_else(|| io::Error::other("missing X11 source"))?;
        let escape = drag.escape_key;
        drag.sampler = Some(Poller::spawn(move || {
            let sample = sample_pointer(&connection, source, root, escape)?;
            let finished = !sample.pointer.down || sample.pointer.escape;
            Ok((sample, finished))
        })?);
        Ok(drag)
    }
    pub fn sample(&mut self, _window: &Window) -> io::Result<Option<Pointer>> {
        let Some(sample) = self.sampler.as_ref().and_then(Poller::take) else {
            return Ok(None);
        };
        let sample = sample?;
        self.point = sample.root_position;
        self.targets = sample.targets;
        Ok(Some(sample.pointer))
    }
    pub fn show(&mut self, visible: bool) -> io::Result<()> {
        if visible {
            self.connection
                .configure_window(
                    self.preview,
                    &ConfigureWindowAux::new()
                        .x(self.point.0 as i32 - self.grab as i32)
                        .y(self.point.1 as i32 - self.half_height as i32)
                        .stack_mode(StackMode::ABOVE),
                )
                .map_err(io::Error::other)?;
        }
        if visible != self.visible {
            if visible {
                self.connection
                    .map_window(self.preview)
                    .map_err(io::Error::other)?;
            } else {
                self.connection
                    .unmap_window(self.preview)
                    .map_err(io::Error::other)?;
            }
            self.visible = visible;
        }
        self.connection.flush().map_err(io::Error::other)
    }
    pub fn target_position(&self, window: &Window) -> Option<PhysicalPosition<f64>> {
        let target = id(window)?;
        self.targets
            .iter()
            .find_map(|(id, point)| (*id == target).then_some(*point))
    }
}
impl Drop for Drag {
    fn drop(&mut self) {
        let _ = self.connection.free_gc(self.gc);
        let _ = self.connection.free_pixmap(self.pixmap);
        let _ = self.connection.destroy_window(self.preview);
        let _ = self.connection.flush();
        self.sampler = None;
        // SAFETY: Arc keeps this socket alive through shutdown, including while
        // the worker awaits a reply. It is this gesture's private connection.
        unsafe {
            libc::shutdown(self.connection.stream().as_raw_fd(), libc::SHUT_RDWR);
        }
    }
}

struct Sample {
    pointer: Pointer,
    root_position: (i16, i16),
    targets: Vec<(u32, PhysicalPosition<f64>)>,
}

fn sample_pointer(
    connection: &RustConnection,
    source: u32,
    root: u32,
    escape: u8,
) -> io::Result<Sample> {
    // Send both requests before waiting: one server round trip, on the worker.
    // Querying the source gives both local and root coordinates without calling
    // Window::inner_position (another synchronous X request) on the UI thread.
    let pointer = connection.query_pointer(source).map_err(io::Error::other)?;
    let keys = connection.query_keymap().map_err(io::Error::other)?;
    let pointer = pointer.reply().map_err(io::Error::other)?;
    let keys = keys.reply().map_err(io::Error::other)?;
    let down = pointer.mask.contains(KeyButMask::BUTTON1);
    let escape = keys.keys[escape as usize / 8] & (1 << (escape % 8)) != 0;
    let mut targets = Vec::new();
    if !down && !escape {
        // Walk stacking once per drop, not once per candidate window. The input
        // transparent preview cannot intercept this path.
        let mut current = root;
        for _ in 0..64 {
            let hit = connection
                .query_pointer(current)
                .map_err(io::Error::other)?
                .reply()
                .map_err(io::Error::other)?;
            targets.push((
                current,
                PhysicalPosition::new(hit.win_x as f64, hit.win_y as f64),
            ));
            if hit.child == 0 {
                break;
            }
            current = hit.child;
        }
    }
    Ok(Sample {
        pointer: Pointer {
            position: PhysicalPosition::new(pointer.win_x as f64, pointer.win_y as f64),
            down,
            escape,
        },
        root_position: (pointer.root_x, pointer.root_y),
        targets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rio_window::{event_loop::EventLoop, platform::x11::EventLoopBuilderExtX11};

    #[test]
    #[ignore = "requires an isolated X11 server (run under xvfb-run)"]
    #[allow(deprecated)]
    fn preview_keeps_pixels_focus_and_drop_target() {
        let mut builder = EventLoop::builder();
        builder.with_x11().with_any_thread(true);
        let event_loop = builder.build().unwrap();
        let source = event_loop
            .create_window(
                Window::default_attributes()
                    .with_inner_size(rio_window::dpi::PhysicalSize::new(320, 200)),
            )
            .unwrap();
        let target = event_loop
            .create_window(
                Window::default_attributes()
                    .with_inner_size(rio_window::dpi::PhysicalSize::new(320, 200)),
            )
            .unwrap();
        let image = DragImage {
            rgba: [12, 34, 56, 255].repeat(240 * 24),
            width: 240,
            height: 24,
        };
        let (connection, _) = x11rb::connect(None).unwrap();
        let target_id = id(&target).unwrap();
        connection
            .set_input_focus(InputFocus::POINTER_ROOT, target_id, x11rb::CURRENT_TIME)
            .unwrap()
            .check()
            .unwrap();
        connection
            .warp_pointer(0u32, target_id, 0, 0, 0, 0, 100, 50)
            .unwrap()
            .check()
            .unwrap();
        let mut drag = Drag::new(&source, &image, 20.0).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while drag.sample(&source).unwrap().is_none() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        drag.show(true).unwrap();
        let pixels = drag
            .connection
            .get_image(ImageFormat::Z_PIXMAP, drag.preview, 0, 0, 240, 24, u32::MAX)
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(&pixels.data[..3], &[56, 34, 12]);
        assert_eq!(
            drag.connection
                .get_input_focus()
                .unwrap()
                .reply()
                .unwrap()
                .focus,
            target_id
        );
        assert!(
            drag.target_position(&target).is_some(),
            "preview must not intercept the drop"
        );
        drag.show(false).unwrap();
        drag.show(true).unwrap();
        let pixels = drag
            .connection
            .get_image(ImageFormat::Z_PIXMAP, drag.preview, 0, 0, 1, 1, u32::MAX)
            .unwrap()
            .reply()
            .unwrap();
        assert_eq!(&pixels.data[..3], &[56, 34, 12], "title survives remapping");
    }
}
