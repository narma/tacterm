//! A small, non-activating drag image. It owns no terminal or GPU surface.

use std::{io, mem::size_of, ptr};
use windows_sys::Win32::Foundation::{HWND, POINT, RECT, SIZE};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub struct TabDragPreview {
    window: HWND,
    offset: POINT,
}

struct PreviewSurface {
    dc: HDC,
    bitmap: HBITMAP,
    font: HFONT,
    old_bitmap: HGDIOBJ,
    old_font: HGDIOBJ,
    #[cfg(test)]
    pixels: *mut core::ffi::c_void,
}

impl PreviewSurface {
    #[allow(clippy::too_many_arguments)]
    fn render(
        title: &str,
        width: i32,
        height: i32,
        scale: f32,
        font_family: Option<&str>,
        font_size: f32,
        background: [f32; 4],
        foreground: [f32; 4],
    ) -> io::Result<Self> {
        if !(24..=8192).contains(&width) || !(20..=256).contains(&height) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid tab preview dimensions",
            ));
        }
        // SAFETY: The DIB is bounded above, all selected objects are owned by
        // this guard, and every UTF-16 buffer lives through its native call.
        unsafe {
            let mut surface = PreviewSurface {
                dc: CreateCompatibleDC(ptr::null_mut()),
                bitmap: ptr::null_mut(),
                font: ptr::null_mut(),
                old_bitmap: ptr::null_mut(),
                old_font: ptr::null_mut(),
                #[cfg(test)]
                pixels: ptr::null_mut(),
            };
            if surface.dc.is_null() {
                return Err(io::Error::last_os_error());
            }
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: width,
                    biHeight: -height,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = ptr::null_mut();
            surface.bitmap = CreateDIBSection(
                surface.dc,
                &info,
                DIB_RGB_COLORS,
                &mut pixels,
                ptr::null_mut(),
                0,
            );
            if surface.bitmap.is_null() {
                return Err(io::Error::last_os_error());
            }
            #[cfg(test)]
            {
                surface.pixels = pixels;
            }
            surface.old_bitmap = SelectObject(surface.dc, surface.bitmap);
            if surface.old_bitmap.is_null() {
                return Err(io::Error::last_os_error());
            }
            let family: Vec<u16> = font_family
                .unwrap_or("Segoe UI")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            surface.font = CreateFontW(
                -(font_size * scale).round().clamp(8.0, 128.0) as i32,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                ANTIALIASED_QUALITY as u32,
                DEFAULT_PITCH as u32,
                family.as_ptr(),
            );
            if surface.font.is_null() {
                return Err(io::Error::last_os_error());
            }
            surface.old_font = SelectObject(surface.dc, surface.font);

            let bounds = RECT {
                left: 0,
                top: 0,
                right: width,
                bottom: height,
            };
            let brush = CreateSolidBrush(color_ref(background));
            if brush.is_null() {
                return Err(io::Error::last_os_error());
            }
            let filled = FillRect(surface.dc, &bounds, brush);
            DeleteObject(brush);
            if filled == 0 {
                return Err(io::Error::last_os_error());
            }
            let brush = CreateSolidBrush(color_ref(foreground));
            if brush.is_null() {
                return Err(io::Error::last_os_error());
            }
            FrameRect(surface.dc, &bounds, brush);
            DeleteObject(brush);
            SetBkMode(surface.dc, TRANSPARENT as i32);
            SetTextColor(surface.dc, color_ref(foreground));
            let text: Vec<u16> = title.encode_utf16().collect();
            let padding = (12.0 * scale).round() as i32;
            let mut text_bounds = RECT {
                left: padding,
                right: width - padding,
                ..bounds
            };
            DrawTextW(
                surface.dc,
                text.as_ptr(),
                text.len().min(i32::MAX as usize) as i32,
                &mut text_bounds,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );

            Ok(surface)
        }
    }
}

impl Drop for PreviewSurface {
    fn drop(&mut self) {
        // SAFETY: All handles were created on this event-loop thread. Restore
        // borrowed GDI objects before releasing the objects owned by the surface.
        unsafe {
            if !self.old_font.is_null() {
                SelectObject(self.dc, self.old_font);
            }
            if !self.old_bitmap.is_null() {
                SelectObject(self.dc, self.old_bitmap);
            }
            if !self.dc.is_null() {
                DeleteDC(self.dc);
            }
            if !self.font.is_null() {
                DeleteObject(self.font);
            }
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
        }
    }
}

impl TabDragPreview {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        owner: HWND,
        title: &str,
        width: f32,
        grab_offset: f32,
        scale: f32,
        font_family: Option<&str>,
        font_size: f32,
        background: [f32; 4],
        foreground: [f32; 4],
        point: POINT,
    ) -> io::Result<Self> {
        let scale = scale.clamp(0.5, 8.0);
        let width = (width * scale).round().clamp(24.0, 8192.0) as i32;
        let height = (24.0 * scale).round().clamp(20.0, 256.0) as i32;
        let mut preview = Self {
            window: ptr::null_mut(),
            offset: POINT {
                x: (grab_offset * scale).round().clamp(0.0, width as f32) as i32,
                y: height / 2,
            },
        };
        // SAFETY: STATIC is a system window class. The popup is disabled and
        // click-through, so it cannot take capture/focus or become a drop target.
        // Strings and bitmap storage remain valid for their synchronous calls.
        unsafe {
            preview.window = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
                windows_sys::core::w!("STATIC"),
                ptr::null(),
                WS_POPUP | WS_DISABLED,
                0,
                0,
                width,
                height,
                owner,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            if preview.window.is_null() {
                return Err(io::Error::last_os_error());
            }

            let surface = PreviewSurface::render(
                title,
                width,
                height,
                scale,
                font_family,
                font_size,
                background,
                foreground,
            )?;
            let position = preview.position(point);
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 235,
                AlphaFormat: 0,
            };
            if UpdateLayeredWindow(
                preview.window,
                ptr::null_mut(),
                &position,
                &SIZE {
                    cx: width,
                    cy: height,
                },
                surface.dc,
                &POINT { x: 0, y: 0 },
                0,
                &blend,
                ULW_ALPHA,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            preview.move_to(point)?;
            ShowWindow(preview.window, SW_SHOWNOACTIVATE);
        }
        Ok(preview)
    }

    fn position(&self, point: POINT) -> POINT {
        POINT {
            x: point.x.saturating_sub(self.offset.x),
            y: point.y.saturating_sub(self.offset.y),
        }
    }

    pub fn move_to(&self, point: POINT) -> io::Result<()> {
        let position = self.position(point);
        // SAFETY: The preview is owned by this event-loop thread. Moving it
        // never activates a window or changes mouse capture. NOOWNERZORDER is
        // essential: otherwise moving the owned popup raises the source over
        // the destination selected with Alt+Tab, even with NOACTIVATE.
        if unsafe {
            SetWindowPos(
                self.window,
                HWND_TOPMOST,
                position.x,
                position.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for TabDragPreview {
    fn drop(&mut self) {
        // SAFETY: The popup is owned and destroyed on the same event-loop thread.
        if !self.window.is_null() {
            unsafe {
                DestroyWindow(self.window);
            }
        }
    }
}

fn color_ref(color: [f32; 4]) -> u32 {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    channel(color[0]) | (channel(color[1]) << 8) | (channel(color[2]) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_preview_moves_only_title_rectangle_and_releases_window() {
        let preview = TabDragPreview::new(
            ptr::null_mut(),
            "tab title",
            240.0,
            20.0,
            1.0,
            None,
            12.0,
            [0.12, 0.14, 0.18, 1.0],
            [0.9, 0.92, 0.95, 1.0],
            POINT {
                x: -30000,
                y: -30000,
            },
        )
        .unwrap();
        let window = preview.window;
        preview
            .move_to(POINT {
                x: -29000,
                y: -29000,
            })
            .unwrap();
        let mut bounds = RECT::default();
        // SAFETY: Query only the live popup created by this test, off-screen.
        unsafe {
            assert_ne!(GetWindowRect(window, &mut bounds), 0);
            assert_eq!(bounds.right - bounds.left, 240);
            assert_eq!(bounds.bottom - bounds.top, 24);
            assert_eq!(bounds.left, -29020);
            assert_eq!(bounds.top, -29012);
            assert_ne!(
                GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_NOACTIVATE,
                0
            );
            assert_ne!(GetWindowLongPtrW(window, GWL_STYLE) as u32 & WS_DISABLED, 0);
        }
        drop(preview);
        // SAFETY: IsWindow accepts a handle after its window has been destroyed.
        assert_eq!(unsafe { IsWindow(window) }, 0);
    }

    #[test]
    fn moving_preview_keeps_destination_above_source() {
        struct TestWindow(HWND);
        impl Drop for TestWindow {
            fn drop(&mut self) {
                // SAFETY: This test owns both windows on the current thread.
                unsafe {
                    DestroyWindow(self.0);
                }
            }
        }
        // SAFETY: System class, no callbacks or borrowed state; all windows are
        // off-screen and shown without activation. No user input is generated.
        unsafe {
            let make_window = || {
                let window = CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    windows_sys::core::w!("STATIC"),
                    ptr::null(),
                    WS_POPUP | WS_DISABLED,
                    -30000,
                    -30000,
                    240,
                    100,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null(),
                );
                assert!(!window.is_null());
                ShowWindow(window, SW_SHOWNOACTIVATE);
                TestWindow(window)
            };
            let source = make_window();
            let destination = make_window();
            let preview = TabDragPreview::new(
                source.0,
                "tab",
                240.0,
                20.0,
                1.0,
                None,
                12.0,
                [0.1, 0.1, 0.1, 1.0],
                [1.0; 4],
                POINT {
                    x: -30000,
                    y: -30000,
                },
            )
            .unwrap();
            // Model switching to the destination without stealing desktop focus.
            assert_ne!(
                SetWindowPos(
                    destination.0,
                    HWND_TOP,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE
                ),
                0
            );
            let destination_above_source = || {
                let mut window = GetWindow(source.0, GW_HWNDPREV);
                while !window.is_null() {
                    if window == destination.0 {
                        return true;
                    }
                    window = GetWindow(window, GW_HWNDPREV);
                }
                false
            };
            assert!(destination_above_source());
            let foreground = GetForegroundWindow();
            for x in [-29990, -29980, -29970] {
                preview.move_to(POINT { x, y: -30000 }).unwrap();
                assert!(
                    destination_above_source(),
                    "preview raised its source owner"
                );
                assert_eq!(GetForegroundWindow(), foreground);
            }
        }
    }
    #[test]
    fn native_preview_renders_unicode_title() {
        let width = 640;
        let height = 48;
        let surface = PreviewSurface::render(
            "PowerShell · Проект · 日本語",
            width,
            height,
            2.0,
            Some("Segoe UI"),
            12.0,
            [0.12, 0.14, 0.18, 1.0],
            [0.9, 0.92, 0.95, 1.0],
        )
        .unwrap();
        // SAFETY: The live DIB owns width*height pixels, with a 32-bit,
        // top-down layout and no row padding. Flush GDI before reading it.
        let pixels = unsafe {
            GdiFlush();
            assert!(!surface.pixels.is_null());
            std::slice::from_raw_parts(
                surface.pixels.cast::<u8>(),
                width as usize * height as usize * 4,
            )
        };
        let background = [46, 36, 31];
        let text_pixels = (4..height as usize - 4)
            .flat_map(|y| {
                (24..width as usize - 24).map(move |x| (y * width as usize + x) * 4)
            })
            .filter(|offset| pixels[*offset..*offset + 3] != background)
            .count();
        assert!(
            text_pixels > 100,
            "the title should paint inside the border"
        );
        if let Some(path) = std::env::var_os("TACTERM_TAB_PREVIEW_CAPTURE") {
            let rgba: Vec<u8> = pixels
                .chunks_exact(4)
                .flat_map(|p| [p[2], p[1], p[0], 255])
                .collect();
            image_rs::RgbaImage::from_raw(width as u32, height as u32, rgba)
                .unwrap()
                .save(path)
                .unwrap();
        }
    }
}

/// Global input is sampled only for the lifetime of a started tab gesture.
pub struct TabDragPointer {
    pub position: POINT,
    pub left_down: bool,
    pub escape: bool,
}

pub fn tab_drag_pointer() -> io::Result<TabDragPointer> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON,
    };
    let mut position = POINT { x: 0, y: 0 };
    // SAFETY: GetCursorPos receives a valid writable POINT; GetAsyncKeyState
    // only reads current key state. No hooks, capture, or input injection.
    unsafe {
        if GetCursorPos(&mut position) == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(TabDragPointer {
            position,
            left_down: GetAsyncKeyState(VK_LBUTTON as i32) < 0,
            escape: GetAsyncKeyState(VK_ESCAPE as i32) < 0,
        })
    }
}
