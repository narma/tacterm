use crate::event::{ClickState, EventPayload, EventProxy, RioEvent, RioEventType};
use crate::ime::{Preedit, PreeditCursor};
use crate::renderer::utils::update_colors_based_on_theme;
use crate::router::{routes::RoutePath, Router};
use crate::scheduler::{Scheduler, TimerId, Topic};
use crate::screen::touch::on_touch;
use crate::watcher::configuration_file_updates;
#[cfg(all(
    feature = "audio",
    not(target_os = "macos"),
    not(target_os = "windows")
))]
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use raw_window_handle::HasDisplayHandle;
use rio_backend::clipboard::{Clipboard, ClipboardType};
use rio_backend::config::colors::{ColorRgb, NamedColor};
use rio_window::application::ApplicationHandler;
use rio_window::event::{
    ElementState, Ime, MouseButton, MouseScrollDelta, StartCause, TouchPhase, WindowEvent,
};
use rio_window::event_loop::ActiveEventLoop;
use rio_window::event_loop::ControlFlow;
use rio_window::event_loop::{DeviceEvents, EventLoop};
#[cfg(target_os = "macos")]
use rio_window::platform::macos::ActiveEventLoopExtMacOS;
#[cfg(target_os = "macos")]
use rio_window::platform::macos::WindowExtMacOS;
use rio_window::window::WindowId;
use rio_window::window::{CursorIcon, Fullscreen};
use std::error::Error;
use std::time::{Duration, Instant};

fn tab_left_strip(x: f64, y: f64, width: f64, scale: f64) -> bool {
    // A small dead band prevents accidental detach while reordering near an edge.
    let tolerance = 12.0 * scale;
    x < -tolerance
        || x > width + tolerance
        || y < -tolerance
        || y > (crate::renderer::island::ISLAND_HEIGHT as f64 * scale) + tolerance
}

fn tab_drop_transfers(
    has_destination: bool,
    x: f64,
    y: f64,
    width: f64,
    scale: f64,
) -> bool {
    // Maximized windows share strip coordinates; a visible destination takes
    // precedence over the source's detach threshold.
    has_destination || tab_left_strip(x, y, width, scale)
}

fn detached_window_mode(
    fullscreen: bool,
    maximized: bool,
) -> rio_backend::config::window::WindowMode {
    use rio_backend::config::window::WindowMode;
    if fullscreen {
        WindowMode::Fullscreen
    } else if maximized {
        WindowMode::Maximized
    } else {
        WindowMode::Windowed
    }
}

#[cfg(windows)]
fn update_tab_drag_preview(
    preview: &mut Option<std::io::Result<crate::platform::windows::TabDragPreview>>,
    route: &mut crate::router::Route<'_>,
    position: rio_window::dpi::PhysicalPosition<f64>,
    external: bool,
) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use windows_sys::Win32::Foundation::{HWND, POINT};
    let screen = &mut route.window.screen;
    if !external || !screen.tab_transfer_active() {
        *preview = None;
        return;
    }
    let Ok(origin) = route.window.winit_window.inner_position() else {
        return;
    };
    let point = POINT {
        x: origin.x.saturating_add(position.x as i32),
        y: origin.y.saturating_add(position.y as i32),
    };
    if preview.is_none() {
        let result = (|| {
            let handle = route
                .window
                .winit_window
                .window_handle()
                .map_err(std::io::Error::other)?;
            let RawWindowHandle::Win32(handle) = handle.as_raw() else {
                return Err(std::io::Error::other(
                    "drag preview requires a Windows window",
                ));
            };
            let (width, grab) = screen
                .tab_drag_preview_metrics()
                .ok_or_else(|| std::io::Error::other("tab drag is no longer active"))?;
            let island = screen
                .renderer
                .island
                .as_ref()
                .ok_or_else(|| std::io::Error::other("tab strip is hidden"))?;
            let background = screen
                .context_manager
                .current_grid()
                .custom_color
                .unwrap_or_else(|| {
                    let color = screen.renderer.dynamic_background.0;
                    [
                        (color[0] + 0.08).min(1.0),
                        (color[1] + 0.08).min(1.0),
                        (color[2] + 0.08).min(1.0),
                        1.0,
                    ]
                });
            crate::platform::windows::TabDragPreview::new(
                handle.hwnd.get() as HWND,
                &screen.context_manager.displayed_title_for_current_tab(),
                width,
                grab,
                screen.sugarloaf.scale_factor(),
                island.title_font_family.as_deref(),
                island.title_font_size,
                background,
                island.active_text_color,
                point,
            )
        })();
        if let Err(error) = &result {
            tracing::warn!("Could not create tab drag preview: {error}");
        }
        *preview = Some(result);
    }
    if let Some(Ok(image)) = preview {
        if let Err(error) = image.move_to(point) {
            tracing::warn!("Could not move tab drag preview: {error}");
            *preview = Some(Err(error));
        }
    }
    if matches!(preview, Some(Err(_))) {
        // Keep the in-window feedback if the desktop compositor rejects the image.
        if let Some(island) = &mut screen.renderer.island {
            island.set_drag_external(false);
        }
    }
}

#[cfg(test)]
mod tab_transfer_tests {
    use super::*;

    #[test]
    fn overlapping_maximized_strips_accept_transfer_after_focus_switch() {
        for scale in [1.0, 1.25, 2.0] {
            let (x, y, width) = (120.0 * scale, 16.0 * scale, 1920.0 * scale);
            assert!(tab_drop_transfers(true, x, y, width, scale));
            assert!(!tab_drop_transfers(false, x, y, width, scale));
            assert!(tab_drop_transfers(false, x, 100.0 * scale, width, scale));
        }
    }

    #[test]
    fn detached_window_inherits_live_parent_state() {
        use rio_backend::config::window::WindowMode;
        assert_eq!(detached_window_mode(false, false), WindowMode::Windowed);
        assert_eq!(detached_window_mode(false, true), WindowMode::Maximized);
        assert_eq!(detached_window_mode(true, true), WindowMode::Fullscreen);
        assert_eq!(detached_window_mode(true, false), WindowMode::Fullscreen);
    }

    #[test]
    fn tab_detach_threshold_scales_with_dpi() {
        for scale in [1.0, 1.25, 2.0] {
            let width = 800.0 * scale;
            let bottom = crate::renderer::island::ISLAND_HEIGHT as f64 * scale;
            assert!(!tab_left_strip(100.0, bottom + 10.0 * scale, width, scale));
            assert!(tab_left_strip(100.0, bottom + 13.0 * scale, width, scale));
            assert!(!tab_left_strip(-10.0 * scale, 10.0, width, scale));
            assert!(tab_left_strip(-13.0 * scale, 10.0, width, scale));
            assert!(tab_left_strip(width + 13.0 * scale, 10.0, width, scale));
            assert!(tab_left_strip(100.0, -13.0 * scale, width, scale));
        }
    }
}

pub struct Application<'a> {
    config: rio_backend::config::Config,
    /// The title template shows `{{columns}}`/`{{lines}}`, so a resize
    /// must re-render titles (the only title data with no PTY event;
    /// everything else arrives via OSC 0/2 and OSC 7).
    title_tracks_size: bool,
    event_proxy: EventProxy,
    router: Router<'a>,
    scheduler: Scheduler,
    app_id: Option<String>,
    global_hotkey: Option<crate::global_hotkey::GlobalHotkeys>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    desktop_tab_drag: Option<crate::platform::tab_drag::DesktopDrag>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    native_tab_drag_source: Option<rio_backend::event::WindowId>,
    tab_drag_position: Option<(
        rio_backend::event::WindowId,
        rio_window::dpi::PhysicalPosition<f64>,
    )>,
    #[cfg(windows)]
    tab_drag_preview: Option<std::io::Result<crate::platform::windows::TabDragPreview>>,
    #[cfg(windows)]
    tab_drag_sample: Option<(rio_backend::event::WindowId, i32, i32, bool)>,
    /// Frontmost app when the quake window was shown, re-activated
    /// when it hides so focus returns where the user was.
    #[cfg(target_os = "macos")]
    quake_previous_app: Option<i32>,
}

impl Application<'_> {
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    fn active_tab_drag_source(&self) -> Option<rio_backend::event::WindowId> {
        self.router.routes.iter().find_map(|(id, route)| {
            route.window.screen.tab_transfer_active().then_some(*id)
        })
    }

    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    fn cancel_tab_transfer(&mut self, source_id: rio_backend::event::WindowId) {
        #[cfg(all(target_os = "linux", feature = "wayland"))]
        if self.native_tab_drag_source == Some(source_id)
            && self.desktop_tab_drag.is_none()
        {
            use rio_window::platform::wayland::WindowExtWayland;
            if let Some(route) = self.router.routes.get(&source_id) {
                route.window.winit_window.cancel_tab_drag();
            }
        }
        #[cfg(windows)]
        {
            self.tab_drag_preview = None;
            self.tab_drag_sample = None;
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            self.desktop_tab_drag = None;
            self.native_tab_drag_source = None;
        }
        self.tab_drag_position = None;
        if let Some(route) = self.router.routes.get_mut(&source_id) {
            route.window.screen.handle_tab_drag_release();
            route.window.screen.mouse.left_button_state = ElementState::Released;
            route.request_redraw();
        }
    }

    // Capture can be revoked by Alt+Tab. Poll only during a started gesture so
    // movement and release still belong to the source after focus changes.
    #[cfg(windows)]
    fn poll_tab_transfer(&mut self, event_loop: &ActiveEventLoop, released: bool) {
        let Some(source_id) = self.active_tab_drag_source() else {
            return;
        };
        let state = match crate::platform::windows::tab_drag_pointer() {
            Ok(state) => state,
            Err(error) => {
                tracing::warn!("Could not track tab drag: {error}");
                self.cancel_tab_transfer(source_id);
                return;
            }
        };
        if state.escape {
            self.cancel_tab_transfer(source_id);
            return;
        }
        let Some(source) = self.router.routes.get_mut(&source_id) else {
            return;
        };
        let Ok(origin) = source.window.winit_window.inner_position() else {
            self.cancel_tab_transfer(source_id);
            return;
        };
        let position = rio_window::dpi::PhysicalPosition::new(
            (state.position.x - origin.x) as f64,
            (state.position.y - origin.y) as f64,
        );
        self.tab_drag_position = Some((source_id, position));
        if released || !state.left_down {
            self.tab_drag_preview = None;
            if !self.finish_tab_drag(event_loop, source_id) {
                self.cancel_tab_transfer(source_id);
            }
            return;
        }
        let scale = source.window.screen.sugarloaf.scale_factor();
        let external = !source.window.winit_window.has_focus()
            || tab_left_strip(
                position.x,
                position.y,
                source.window.winit_window.inner_size().width as f64,
                scale as f64,
            );
        let sample = (source_id, state.position.x, state.position.y, external);
        if self.tab_drag_sample == Some(sample) {
            return;
        }
        self.tab_drag_sample = Some(sample);
        source
            .window
            .screen
            .handle_tab_drag_move(position.x as f32 / scale, external);
        update_tab_drag_preview(&mut self.tab_drag_preview, source, position, external);
        source.request_redraw();
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn tab_destination(
        &self,
        id: rio_backend::event::WindowId,
        point: rio_window::dpi::PhysicalPosition<f64>,
    ) -> Option<(rio_backend::event::WindowId, usize)> {
        let route = self.router.routes.get(&id)?;
        let screen = &route.window.screen;
        let scale = screen.sugarloaf.scale_factor() as f64;
        if route.path != RoutePath::Terminal
            || route.active_modal().is_some()
            || !screen
                .renderer
                .navigation
                .island_visible(screen.context_manager.len())
            || point.x < 0.0
            || point.y < 0.0
            || point.x >= route.window.winit_window.inner_size().width as f64
            || point.y > crate::renderer::island::ISLAND_HEIGHT as f64 * scale
        {
            return None;
        }
        Some((id, screen.tab_drop_index((point.x / scale) as f32)))
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn poll_desktop_tab_drag(&mut self, event_loop: &ActiveEventLoop, released: bool) {
        let Some(source_id) = self.active_tab_drag_source() else {
            self.desktop_tab_drag = None;
            self.native_tab_drag_source = None;
            return;
        };
        if self.native_tab_drag_source.is_none() {
            let Some(route) = self.router.routes.get(&source_id) else {
                return;
            };
            let screen = &route.window.screen;
            let Some((width, grab)) = screen.tab_drag_preview_metrics() else {
                return;
            };
            let Some(island) = screen.renderer.island.as_ref() else {
                return;
            };
            let scale = screen.sugarloaf.scale_factor();
            // Wayland should retain ordinary in-strip reordering until departure.
            #[cfg(all(target_os = "linux", feature = "wayland"))]
            {
                use rio_window::platform::wayland::ActiveEventLoopExtWayland;
                if event_loop.is_wayland()
                    && !self.tab_drag_position.is_some_and(|(_, p)| {
                        tab_left_strip(
                            p.x,
                            p.y,
                            route.window.winit_window.inner_size().width as f64,
                            scale as f64,
                        )
                    })
                {
                    return;
                }
            }
            #[allow(unused_mut)]
            let mut image_scale = scale;
            #[cfg(all(target_os = "linux", feature = "wayland"))]
            {
                use rio_window::platform::wayland::ActiveEventLoopExtWayland;
                if event_loop.is_wayland() {
                    image_scale = scale.ceil();
                }
            }
            #[allow(unused_mut)]
            let mut pixel_width =
                (width * image_scale).round().clamp(24.0, 4096.0) as u32;
            #[allow(unused_mut)]
            let mut pixel_height = (24.0 * image_scale).round().clamp(20.0, 256.0) as u32;
            #[cfg(all(target_os = "linux", feature = "wayland"))]
            {
                use rio_window::platform::wayland::ActiveEventLoopExtWayland;
                if event_loop.is_wayland() {
                    pixel_width -= pixel_width % image_scale as u32;
                    pixel_height -= pixel_height % image_scale as u32;
                }
            }
            let Some(image) = rio_backend::sugarloaf::font::drag_image::render(
                screen.sugarloaf.font_library(),
                &screen.context_manager.displayed_title_for_current_tab(),
                island.title_font_family.as_deref(),
                island.title_font_size * image_scale,
                pixel_width,
                pixel_height,
                screen
                    .context_manager
                    .current_grid()
                    .custom_color
                    .unwrap_or(screen.renderer.dynamic_background.0),
                island.active_text_color,
            ) else {
                self.cancel_tab_transfer(source_id);
                return;
            };
            match crate::platform::tab_drag::DesktopDrag::new(
                &route.window.winit_window,
                &image,
                grab * image_scale,
            ) {
                Ok(Some(drag)) => self.desktop_tab_drag = Some(drag),
                Ok(None) => {
                    #[cfg(all(target_os = "linux", feature = "wayland"))]
                    {
                        use rio_window::platform::wayland::WindowExtWayland;
                        let image = rio_window::window::TabDragImage {
                            rgba: image.rgba,
                            width: image.width,
                            height: image.height,
                            scale: image_scale.round().clamp(1.0, 8.0) as i32,
                            grab_x: (grab * image_scale) as i32,
                        };
                        if let Err(error) =
                            route.window.winit_window.start_tab_drag(image)
                        {
                            tracing::warn!("Could not start native tab drag: {error}");
                            self.cancel_tab_transfer(source_id);
                            return;
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!("Could not create tab preview: {error}");
                    self.cancel_tab_transfer(source_id);
                    return;
                }
            }
            self.native_tab_drag_source = Some(source_id);
        }
        let Some(drag) = self.desktop_tab_drag.as_mut() else {
            return;
        };
        let Some(source) = self.router.routes.get_mut(&source_id) else {
            return;
        };
        let pointer = match drag.sample(&source.window.winit_window) {
            Ok(p) => p,
            Err(error) => {
                tracing::warn!("Could not track tab: {error}");
                self.cancel_tab_transfer(source_id);
                return;
            }
        };
        if pointer.escape {
            self.cancel_tab_transfer(source_id);
            return;
        }
        if released || !pointer.down {
            if let Err(error) = drag.show(false) {
                tracing::warn!("Could not hide tab preview: {error}");
            }
            let drag = self.desktop_tab_drag.as_ref().unwrap();
            let destination = self
                .router
                .routes
                .iter()
                .filter(|(id, _)| **id != source_id)
                .find_map(|(id, route)| {
                    drag.target_position(&route.window.winit_window)
                        .and_then(|p| self.tab_destination(*id, p))
                });
            let source = &self.router.routes[&source_id];
            let scale = source.window.screen.sugarloaf.scale_factor() as f64;
            if !tab_drop_transfers(
                destination.is_some(),
                pointer.position.x,
                pointer.position.y,
                source.window.winit_window.inner_size().width as f64,
                scale,
            ) {
                self.cancel_tab_transfer(source_id);
                return;
            }
            let position = source.window.winit_window.inner_position().ok().map(|p| {
                rio_window::dpi::PhysicalPosition::new(
                    p.x + pointer.position.x as i32 - 80,
                    p.y + pointer.position.y as i32 - 16,
                )
            });
            if !self.transfer_tab(event_loop, source_id, destination, position) {
                self.cancel_tab_transfer(source_id);
            }
            return;
        }
        let scale = source.window.screen.sugarloaf.scale_factor();
        let external = !source.window.winit_window.has_focus()
            || tab_left_strip(
                pointer.position.x,
                pointer.position.y,
                source.window.winit_window.inner_size().width as f64,
                scale as f64,
            );
        if self.tab_drag_position != Some((source_id, pointer.position))
            || source
                .window
                .screen
                .renderer
                .island
                .as_ref()
                .is_some_and(|i| i.drag_is_external() != external)
        {
            self.tab_drag_position = Some((source_id, pointer.position));
            source
                .window
                .screen
                .handle_tab_drag_move(pointer.position.x as f32 / scale, external);
            source.request_redraw();
        }
        if let Err(error) = drag.show(external) {
            tracing::warn!("Could not move tab preview: {error}");
            self.cancel_tab_transfer(source_id);
        }
    }

    fn finish_tab_drag(
        &mut self,
        event_loop: &ActiveEventLoop,
        source_id: rio_backend::event::WindowId,
    ) -> bool {
        let Some((position_window, position)) = self.tab_drag_position else {
            return false;
        };
        if position_window != source_id {
            return false;
        }
        let Some(source) = self.router.routes.get(&source_id) else {
            return false;
        };
        let screen = &source.window.screen;
        if source.path != RoutePath::Terminal
            || source.active_modal().is_some()
            || !screen.tab_transfer_active()
        {
            return false;
        }
        let scale = screen.sugarloaf.scale_factor() as f64;
        let size = source.window.winit_window.inner_size();
        let Ok(origin) = source.window.winit_window.inner_position() else {
            return false;
        };
        let point = rio_window::dpi::PhysicalPosition::new(
            origin.x + position.x as i32,
            origin.y + position.y as i32,
        );

        // Windows mouse capture keeps delivering moves to the source. Query the
        // actual topmost window so an obscured tab strip cannot receive a drop.
        #[cfg(windows)]
        // SAFETY: Both functions only inspect the window tree; POINT is passed
        // by value and GetAncestor accepts the null result of WindowFromPoint.
        let under_pointer = unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                GetAncestor, WindowFromPoint, GA_ROOT,
            };
            GetAncestor(
                WindowFromPoint(windows_sys::Win32::Foundation::POINT {
                    x: point.x,
                    y: point.y,
                }),
                GA_ROOT,
            )
        };
        let destination = self.router.routes.iter().find_map(|(id, route)| {
            if *id == source_id || route.path != RoutePath::Terminal
                || !route.window.screen.renderer.navigation.island_visible(route.window.screen.context_manager.len())
                || route.active_modal().is_some()
                || route.window.winit_window.is_visible() == Some(false)
                || route.window.winit_window.is_minimized() == Some(true) { return None; }
            #[cfg(windows)]
            {
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                let Ok(handle) = route.window.winit_window.window_handle() else { return None; };
                if !matches!(handle.as_raw(), RawWindowHandle::Win32(handle) if handle.hwnd.get() as windows_sys::Win32::Foundation::HWND == under_pointer) { return None; }
            }
            let origin = route.window.winit_window.inner_position().ok()?;
            let x = (point.x - origin.x) as f64;
            let y = (point.y - origin.y) as f64;
            let scale = route.window.screen.sugarloaf.scale_factor() as f64;
            if x < 0.0 || x >= route.window.winit_window.inner_size().width as f64
                || y < 0.0 || y > crate::renderer::island::ISLAND_HEIGHT as f64 * scale { return None; }
            Some((*id, route.window.screen.tab_drop_index((x / scale) as f32)))
        });
        if !tab_drop_transfers(
            destination.is_some(),
            position.x,
            position.y,
            size.width as f64,
            scale,
        ) {
            return false;
        }
        self.transfer_tab(
            event_loop,
            source_id,
            destination,
            Some(rio_window::dpi::PhysicalPosition::new(
                point.x - 80,
                point.y - 16,
            )),
        )
    }

    fn transfer_tab(
        &mut self,
        event_loop: &ActiveEventLoop,
        source_id: rio_backend::event::WindowId,
        destination: Option<(rio_backend::event::WindowId, usize)>,
        position: Option<rio_window::dpi::PhysicalPosition<i32>>,
    ) -> bool {
        let Some(source) = self.router.routes.get(&source_id) else {
            return false;
        };
        if !source.window.screen.tab_transfer_active()
            || source.path != RoutePath::Terminal
            || source.active_modal().is_some()
        {
            return false;
        }
        let size = source.window.winit_window.inner_size();
        let scale = source.window.screen.sugarloaf.scale_factor() as f64;
        let new_window = destination.is_none();
        let mut detached_config = self.config.clone();
        detached_config.window.mode = detached_window_mode(
            source.window.winit_window.fullscreen().is_some(),
            source.window.winit_window.is_maximized(),
        );
        detached_config.window.width = (size.width as f64 / scale).round() as i32;
        detached_config.window.height = (size.height as f64 / scale).round() as i32;
        let (destination_id, index) = destination.unwrap_or_else(|| {
            let id = self.router.create_transfer_window(
                event_loop,
                self.event_proxy.clone(),
                &detached_config,
                self.app_id.as_deref(),
                position,
            );
            (id, 0)
        });
        let Some(mut source) = self.router.routes.remove(&source_id) else {
            return false;
        };
        let Some(destination) = self.router.routes.get_mut(&destination_id) else {
            self.router.routes.insert(source_id, source);
            return false;
        };
        let source_screen = &mut source.window.screen;
        source_screen.cancel_search(&mut self.router.clipboard);
        let tab = source_screen.context_manager.take_current_tab();
        let target_screen = &mut destination.window.screen;
        target_screen.cancel_search(&mut self.router.clipboard);
        if new_window {
            drop(target_screen.context_manager.take_current_tab());
        }
        for item in tab.contexts().values() {
            source_screen.grids.remove(&item.val.route_id);
            source_screen
                .sugarloaf
                .clear_image_overlays_for(item.val.rich_text_id);
            source_screen
                .sugarloaf
                .transfer_route_images(item.val.route_id, &mut target_screen.sugarloaf);
            self.scheduler
                .unschedule(TimerId::new(Topic::SelectionScrolling, item.val.route_id));
            self.scheduler
                .unschedule(TimerId::new(Topic::CursorBlinking, item.val.route_id));
            self.scheduler
                .unschedule(TimerId::new(Topic::RenderRoute, item.val.route_id));
            self.scheduler
                .unschedule(TimerId::new(Topic::ScheduledRenderRoute, item.val.route_id));
        }
        target_screen.context_manager.insert_tab(tab, index);
        target_screen.finish_tab_transfer(destination.window.winit_window.inner_size());
        destination.sync_window_title();
        destination.window.winit_window.focus_window();
        destination.request_redraw();
        if source_screen.context_manager.len() > 0 {
            source_screen.finish_tab_transfer(source.window.winit_window.inner_size());
            source.sync_window_title();
            source.request_redraw();
            self.router.routes.insert(source_id, source);
        } else {
            if self.router.quake_window_id == Some(source_id) {
                self.router.quake_window_id = None;
            }
            if self.router.config_route == Some(source_id) {
                self.router.config_route = None;
            }
        }
        self.tab_drag_position = None;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            self.desktop_tab_drag = None;
            self.native_tab_drag_source = None;
        }
        #[cfg(windows)]
        {
            self.tab_drag_sample = None;
        }
        true
    }

    pub fn new<'app>(
        config: rio_backend::config::Config,
        config_error: Option<rio_backend::config::ConfigError>,
        event_loop: &EventLoop<EventPayload>,
        app_id: Option<String>,
    ) -> Application<'app> {
        // SAFETY: Since this takes a pointer to the winit event loop, it MUST be dropped first,
        // which is done in `exiting`.
        let clipboard =
            unsafe { Clipboard::new(event_loop.display_handle().unwrap().as_raw()) };

        let mut router = Router::new(config.fonts.to_owned(), clipboard);
        if let Some(error) = config_error {
            router.propagate_error_to_next_route(error.into());
        }

        let proxy = event_loop.create_proxy();
        let event_proxy = EventProxy::new(proxy.clone());
        let _ = configuration_file_updates(
            rio_backend::config::config_dir_path(),
            event_proxy.clone(),
        );
        let scheduler = Scheduler::new(proxy);
        event_loop.listen_device_events(DeviceEvents::Never);

        #[cfg(any(target_os = "macos", target_os = "windows"))]
        event_loop.set_confirm_before_quit(config.confirm_before_quit);

        rio_notifier::request_authorization();

        Application {
            title_tracks_size: title_tracks_size(&config),
            config,
            event_proxy,
            router,
            scheduler,
            app_id,
            global_hotkey: None,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            desktop_tab_drag: None,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            native_tab_drag_source: None,
            tab_drag_position: None,
            #[cfg(windows)]
            tab_drag_preview: None,
            #[cfg(windows)]
            tab_drag_sample: None,
            #[cfg(target_os = "macos")]
            quake_previous_app: None,
        }
    }

    fn skip_window_event(event: &WindowEvent) -> bool {
        matches!(
            event,
            WindowEvent::KeyboardInput {
                is_synthetic: true,
                ..
            } | WindowEvent::ActivationTokenDone { .. }
                | WindowEvent::DoubleTapGesture { .. }
                | WindowEvent::TouchpadPressure { .. }
                | WindowEvent::RotationGesture { .. }
                | WindowEvent::CursorEntered { .. }
                | WindowEvent::PinchGesture { .. }
                | WindowEvent::AxisMotion { .. }
                | WindowEvent::PanGesture { .. }
                | WindowEvent::HoveredFileCancelled
                | WindowEvent::Destroyed
                | WindowEvent::HoveredFile(_)
                | WindowEvent::Moved(_)
        )
    }

    fn handle_audio_bell(&mut self) {
        #[cfg(target_os = "macos")]
        {
            // Use system bell sound on macOS
            unsafe {
                #[link(name = "AppKit", kind = "framework")]
                extern "C" {
                    fn NSBeep();
                }
                NSBeep();
            }
        }

        #[cfg(target_os = "windows")]
        {
            // Use MessageBeep on Windows with MB_OK (0x00000000) for default beep
            unsafe {
                windows_sys::Win32::System::Diagnostics::Debug::MessageBeep(0x00000000);
            }
        }

        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        {
            #[cfg(feature = "audio")]
            {
                std::thread::spawn(|| {
                    if let Err(e) = play_bell_sound() {
                        tracing::warn!("Failed to play bell sound: {}", e);
                    }
                });
            }
            #[cfg(not(feature = "audio"))]
            {
                tracing::debug!("Audio bell requested but audio feature is not enabled");
            }
        }
    }

    fn handle_desktop_notification(&self, title: &str, body: &str) {
        rio_notifier::send_notification(title, body);
    }

    pub fn run(
        &mut self,
        event_loop: EventLoop<EventPayload>,
    ) -> Result<(), Box<dyn Error>> {
        let result = event_loop.run_app(self);
        result.map_err(Into::into)
    }
}

impl Application<'_> {
    /// One pane's title data changed (OSC 0/2 title carried in
    /// `raw_title`, OSC 7 directory as `None`): re-render, repaint the
    /// strip when the displayed text changed, and poke the titlebar.
    fn handle_title_change(
        &mut self,
        window_id: rio_backend::event::WindowId,
        route_id: usize,
        raw_title: Option<&str>,
    ) {
        if let Some(route) = self.router.routes.get_mut(&window_id) {
            if route
                .window
                .screen
                .context_manager
                .on_title_change(route_id, raw_title)
            {
                route.request_overlay_redraw();
            }
            route.sync_window_title();
        }
    }
}

/// Whether the title template renders the pane size, the one title
/// input with no PTY event attached (resizes are locally known).
fn title_tracks_size(config: &rio_backend::config::Config) -> bool {
    let template = config.title.content.to_lowercase();
    template.contains("columns") || template.contains("lines")
}

impl Application<'_> {
    /// Register a system-wide hotkey for every `ToggleQuake` binding
    /// in the config, so the quake window opens while Rio is
    /// unfocused. No-op when quake is not bound; pure Wayland has no
    /// global hotkey API, the compositor keybinding + a regular
    /// binding cover it there.
    fn setup_quake_hotkey(&mut self) {
        // Drop any previous manager first: registering a chord the old
        // manager still holds fails on Windows and X11.
        self.global_hotkey = None;
        self.global_hotkey = crate::global_hotkey::setup(
            self.event_proxy.clone(),
            &self.config.bindings.keys,
        );
    }

    /// The monitor the quake window should drop down on: the one
    /// under the mouse cursor where the platform can tell us, the
    /// primary monitor otherwise.
    fn quake_monitor(
        &self,
        event_loop: &ActiveEventLoop,
    ) -> Option<rio_window::monitor::MonitorHandle> {
        event_loop
            .cursor_monitor()
            .or_else(|| event_loop.primary_monitor())
    }

    /// Anchor the quake window to the top of `monitor`, horizontally
    /// centered, sized by the configured percentages, then show it.
    fn show_quake_window(
        &mut self,
        id: rio_backend::event::WindowId,
        event_loop: &ActiveEventLoop,
    ) {
        #[cfg(target_os = "macos")]
        {
            self.quake_previous_app =
                rio_window::platform::macos::frontmost_application_pid();
        }

        let Some(route) = self.router.routes.get(&id) else {
            return;
        };
        let window = &route.window.winit_window;
        if let Some(monitor) = self.quake_monitor(event_loop) {
            let msize = monitor.size();
            let mpos = monitor.position();
            let width = (msize.width as f32
                * self.config.window.quake_width_percentage.clamp(0.1, 1.0))
                as u32;
            let height = (msize.height as f32
                * self.config.window.quake_height_percentage.clamp(0.1, 1.0))
                as u32;
            let x = mpos.x + (msize.width.saturating_sub(width) / 2) as i32;
            #[cfg(target_os = "macos")]
            {
                let scale = monitor.scale_factor();
                let size: rio_window::dpi::LogicalSize<f64> =
                    rio_window::dpi::PhysicalSize::new(width, height).to_logical(scale);
                let pos: rio_window::dpi::LogicalPosition<f64> =
                    rio_window::dpi::PhysicalPosition::new(x, mpos.y).to_logical(scale);
                let _ = window.request_inner_size(size);
                window.set_outer_position(pos);
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = window.request_inner_size(rio_window::dpi::PhysicalSize::new(
                    width, height,
                ));
                window.set_outer_position(rio_window::dpi::PhysicalPosition::new(
                    x, mpos.y,
                ));
            }
        }
        window.set_visible(true);
        window.focus_window();
    }

    /// Show, focus or hide the quake window; create it on first use.
    fn toggle_quake_window(&mut self, event_loop: &ActiveEventLoop) {
        let quake_id = self
            .router
            .quake_window_id
            .filter(|id| self.router.routes.contains_key(id));

        let Some(id) = quake_id else {
            self.router.quake_window_id = None;
            self.router.create_quake_window(
                event_loop,
                self.event_proxy.clone(),
                &self.config,
            );
            if let Some(id) = self.router.quake_window_id {
                self.show_quake_window(id, event_loop);
            }
            return;
        };

        if let Some(route) = self.router.routes.get_mut(&id) {
            let window = &route.window.winit_window;
            let visible = window.is_visible().unwrap_or(true);
            if !visible {
                self.show_quake_window(id, event_loop);
            } else if window.has_focus() {
                window.set_visible(false);
                #[cfg(target_os = "macos")]
                if let Some(pid) = self.quake_previous_app.take() {
                    rio_window::platform::macos::activate_application(pid);
                }
            } else {
                self.show_quake_window(id, event_loop);
            }
        }
    }
}

impl ApplicationHandler<EventPayload> for Application<'_> {
    fn resumed(&mut self, _active_event_loop: &ActiveEventLoop) {}

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if cause != StartCause::Init
            && cause != StartCause::CreateWindow
            && cause != StartCause::MacOSReopen
        {
            return;
        }

        if cause == StartCause::MacOSReopen && !self.router.routes.is_empty() {
            // Reopen (dock click) with every window minimized should
            // restore one; otherwise clicking the dock icon does
            // nothing at all.
            let all_minimized = self
                .router
                .routes
                .values()
                .all(|route| route.window.winit_window.is_minimized() == Some(true));
            if all_minimized {
                if let Some(route) = self.router.routes.values().next() {
                    route.window.winit_window.set_minimized(false);
                    route.window.winit_window.focus_window();
                }
            }
            return;
        }

        #[cfg(all(
            any(feature = "x11", feature = "wayland"),
            unix,
            not(any(target_os = "redox", target_family = "wasm", target_os = "macos"))
        ))]
        if cause == StartCause::Init
            && self.config.adaptive_colors.is_some()
            && self.config.force_theme.is_none()
        {
            use rio_window::platform::linux::ActiveEventLoopExtLinux;
            event_loop.start_system_theme_monitor();
        }

        let theme = self
            .config
            .force_theme
            .map(|t| t.to_window_theme())
            .or_else(|| event_loop.system_theme());
        update_colors_based_on_theme(&mut self.config, theme);

        self.router.create_window(
            event_loop,
            self.event_proxy.clone(),
            &self.config,
            None,
            self.app_id.as_deref(),
        );

        if cause == StartCause::Init {
            self.setup_quake_hotkey();
        }

        tracing::info!("Initialisation complete");
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: EventPayload) {
        let window_id = event.resolved_window_id();
        match event.payload {
            RioEventType::Rio(RioEvent::Render) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    // Skip rendering for unfocused windows if configured
                    if self.config.renderer.disable_unfocused_render
                        && !route.window.is_focused
                        && route.window.focus_seen
                    {
                        return;
                    }

                    // Skip rendering for occluded windows if configured, unless we need to render after occlusion
                    if self.config.renderer.disable_occluded_render
                        && route.window.is_occluded
                        && !route.window.needs_render_after_occlusion
                    {
                        return;
                    }

                    // Clear the one-time render flag if it was set
                    if route.window.needs_render_after_occlusion {
                        route.window.needs_render_after_occlusion = false;
                    }

                    route.request_redraw();
                }
            }
            RioEventType::Rio(RioEvent::RenderRoute(route_id)) => {
                if self.config.renderer.strategy.is_event_based() {
                    if let Some(route) = self.router.routes.get_mut(&window_id) {
                        // Skip rendering for unfocused windows if configured
                        if self.config.renderer.disable_unfocused_render
                            && !route.window.is_focused
                            && route.window.focus_seen
                        {
                            if route.window.screen.renderer.scrollbar.needs_redraw() {
                                route.request_redraw();
                            }
                            return;
                        }

                        // Skip rendering for occluded windows if configured, unless we need to render after occlusion
                        if self.config.renderer.disable_occluded_render
                            && route.window.is_occluded
                            && !route.window.needs_render_after_occlusion
                        {
                            return;
                        }

                        // Clear the one-time render flag if it was set
                        if route.window.needs_render_after_occlusion {
                            route.window.needs_render_after_occlusion = false;
                        }

                        // Mark the renderable content as needing to render
                        if let Some(ctx_item) =
                            route.window.screen.ctx_mut().get_by_route_id(route_id)
                        {
                            ctx_item.val.renderable_content.pending_update.set_dirty();
                        }

                        // Check if we need to throttle based on timing
                        if let Some(wait_duration) = route.window.wait_until() {
                            // We need to wait before rendering again
                            let timer_id = TimerId::new(Topic::RenderRoute, route_id);
                            let event = EventPayload::new(
                                RioEventType::Rio(RioEvent::Render),
                                window_id,
                            );

                            // Only schedule if not already scheduled
                            if !self.scheduler.scheduled(timer_id) {
                                self.scheduler.schedule(
                                    event,
                                    wait_duration,
                                    false,
                                    timer_id,
                                );
                            }
                        } else {
                            // We can render immediately
                            route.request_redraw();
                        }
                    }
                }
            }

            RioEventType::Rio(RioEvent::TerminalDamaged(route_id)) => {
                if self.config.renderer.strategy.is_event_based() {
                    if let Some(route) = self.router.routes.get_mut(&window_id) {
                        if self.config.renderer.disable_unfocused_render
                            && !route.window.is_focused
                            && route.window.focus_seen
                        {
                            return;
                        }
                        if self.config.renderer.disable_occluded_render
                            && route.window.is_occluded
                            && !route.window.needs_render_after_occlusion
                        {
                            return;
                        }

                        if let Some(ctx_item) =
                            route.window.screen.ctx_mut().get_by_route_id(route_id)
                        {
                            // Just mark dirty — damage will be extracted from
                            // the terminal when the renderer locks it.
                            ctx_item.val.renderable_content.pending_update.set_dirty();
                            route.request_redraw();
                        }
                    }
                }
            }
            RioEventType::Rio(RioEvent::UpdateGraphics { route_id, queues }) => {
                use rio_backend::sugarloaf::route_image_key;
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    // A batch the VT thread queued before the user
                    // closed its tab or split would otherwise land
                    // under a route nothing will ever release.
                    if route
                        .window
                        .screen
                        .context_manager
                        .get_by_route_id(route_id)
                        .is_none()
                    {
                        return;
                    }

                    // Process graphics directly in sugarloaf
                    let sugarloaf = &mut route.window.screen.sugarloaf;

                    // Atlas graphics (sixel/iTerm2) share the per-image
                    // texture store with kitty images, in a disjoint key
                    // namespace.
                    for graphic_data in queues.pending {
                        let key = route_image_key(
                            route_id,
                            crate::renderer::atlas_image_key(graphic_data.id.get()),
                        );
                        sugarloaf.image_data.insert(
                            key,
                            rio_backend::sugarloaf::GraphicDataEntry::from_graphic_data(
                                graphic_data,
                            ),
                        );
                    }

                    // Removals arrive as final image keys (atlas refs
                    // dropped off scrollback, kitty evictions) and free
                    // both the pixel store and the cached GPU texture.
                    // They run before the kitty uploads so a batch that
                    // frees a key and resends it keeps the new pixels.
                    for key in queues.remove_queue {
                        sugarloaf.remove_image(route_image_key(route_id, key));
                    }

                    // Image textures (kitty) → separate store, no clone
                    for (image_id, graphic_data) in queues.pending_images {
                        sugarloaf.image_data.insert(
                            route_image_key(
                                route_id,
                                crate::renderer::kitty_image_key(image_id),
                            ),
                            rio_backend::sugarloaf::GraphicDataEntry::from_graphic_data(
                                graphic_data,
                            ),
                        );
                    }

                    // Mark the panel dirty: the renderer skips non-dirty
                    // panels, so a bare redraw after the pixels arrive
                    // would no-op and leave the image blank until the
                    // next unrelated damage.
                    if let Some(ctx_item) =
                        route.window.screen.ctx_mut().get_by_route_id(route_id)
                    {
                        ctx_item.val.renderable_content.pending_update.set_dirty();
                    }

                    // Request a redraw to display the updated graphics
                    route.request_redraw();
                }
            }
            RioEventType::Rio(RioEvent::PrepareUpdateConfig) => {
                let timer_id = TimerId::new(Topic::UpdateConfig, 0);
                let event = EventPayload::new(
                    RioEventType::Rio(RioEvent::UpdateConfig),
                    window_id,
                );

                if !self.scheduler.scheduled(timer_id) {
                    self.scheduler.schedule(
                        event,
                        Duration::from_millis(250),
                        false,
                        timer_id,
                    );
                }
            }
            RioEventType::Rio(RioEvent::ReportToAssistant(error)) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    route.report_error(&error);
                }
            }
            RioEventType::Rio(RioEvent::UpdateConfig) => {
                let (mut config, config_error) =
                    match rio_backend::config::Config::try_load() {
                        Ok(config) => (config, None),
                        Err(error) => {
                            (rio_backend::config::Config::default(), Some(error))
                        }
                    };
                config.overwrite_based_on_platform();

                let has_font_updates = self.config.fonts != config.fonts;
                let has_binding_updates = self.config.bindings != config.bindings;

                let font_library_errors = if has_font_updates {
                    let new_font_library = rio_backend::sugarloaf::font::FontLibrary::new(
                        config.fonts.to_owned(),
                    );
                    *self.router.font_library = new_font_library.0;
                    new_font_library.1
                } else {
                    None
                };

                self.config = config;
                self.title_tracks_size = title_tracks_size(&self.config);

                // Dropping the old manager unregisters its hotkeys, so
                // ToggleQuake binding edits apply without restarting.
                if has_binding_updates {
                    self.setup_quake_hotkey();
                }

                let mut has_checked_adaptive_colors = false;
                for (_id, route) in self.router.routes.iter_mut() {
                    // Apply system theme to ensure colors are consistent
                    if !has_checked_adaptive_colors {
                        let system_theme = event_loop.system_theme();
                        let theme = self
                            .config
                            .force_theme
                            .map(|t| t.to_window_theme())
                            .or(system_theme);
                        update_colors_based_on_theme(&mut self.config, theme);
                        has_checked_adaptive_colors = true;
                    }

                    if has_font_updates {
                        if let Some(ref err) = font_library_errors {
                            route
                                .window
                                .screen
                                .context_manager
                                .report_error_fonts_not_found(
                                    err.fonts_not_found.clone(),
                                );
                        }
                    }

                    route.update_config(
                        &self.config,
                        &self.router.font_library,
                        has_font_updates,
                    );
                    if !self.config.bell.tab_indicator {
                        route.window.screen.context_manager.clear_all_bells();
                    }
                    route.window.configure_window(&self.config);

                    if let Some(error) = &config_error {
                        route.report_error(&error.to_owned().into());
                    } else {
                        route.clear_errors();
                    }

                    route.request_redraw();
                }

                self.router.update_titles();
            }
            RioEventType::Rio(RioEvent::Exit | RioEvent::Quit) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    if self.config.confirm_before_quit {
                        route.confirm_quit();
                    } else {
                        event_loop.exit();
                    }
                }
            }
            RioEventType::Rio(RioEvent::GlyphProtocolInstalled {
                route_id,
                registry,
            }) => {
                if let Some(route) = self.router.routes.get(&window_id) {
                    route
                        .window
                        .screen
                        .sugarloaf
                        .font_library()
                        .install_glyph_registry(route_id, registry);
                }
            }
            RioEventType::Rio(RioEvent::GlyphProtocolQuery { route_id, cp }) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    use rio_backend::ansi::glyph_protocol::{
                        format_query_response, QueryStatus,
                    };
                    let library = route.window.screen.sugarloaf.font_library();
                    let in_glossary = library
                        .glyph_registry_for(route_id)
                        .is_some_and(|r| r.contains(cp));
                    let in_system = library.covers_codepoint(cp);
                    let status = match (in_glossary, in_system) {
                        (true, true) => QueryStatus::Both,
                        (true, false) => QueryStatus::Glossary,
                        (false, true) => QueryStatus::System,
                        (false, false) => QueryStatus::Free,
                    };
                    let resp = format_query_response(cp, status);
                    if let Some(item) = route
                        .window
                        .screen
                        .context_manager
                        .current_grid_mut()
                        .get_by_route_id(route_id)
                    {
                        item.context_mut().messenger.send_bytes(resp.into_bytes());
                    }
                }
            }
            RioEventType::Rio(RioEvent::CloseTerminal(route_id)) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    route
                        .window
                        .screen
                        .sugarloaf
                        .font_library()
                        .remove_glyph_registry(route_id);

                    if route
                        .window
                        .screen
                        .context_manager
                        .should_close_context_manager(
                            route_id,
                            &mut route.window.screen.sugarloaf,
                        )
                    {
                        self.router.routes.remove(&window_id);

                        // Unschedule pending events.
                        self.scheduler.unschedule_window(route_id);

                        if self.router.routes.is_empty() {
                            event_loop.exit();
                        }
                    } else {
                        route.window.screen.refresh_titles();
                        let size = route.window.screen.context_manager.len();
                        route.window.screen.resize_top_or_bottom_line(size);
                    }
                }
            }
            RioEventType::Rio(RioEvent::CursorBlinkingChange) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    route.request_redraw();
                }
            }
            RioEventType::Rio(RioEvent::CursorBlinkingChangeOnRoute(route_id)) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    if route_id == route.window.screen.ctx().current_route() {
                        // Cursor blink toggles the cursor sprite (a
                        // separate quad), not cell content — so we
                        // signal `CursorOnly` and the GPU emit skips
                        // per-row rebuild while the cursor uniform
                        // updates downstream.
                        route
                            .window
                            .screen
                            .ctx_mut()
                            .current_mut()
                            .renderable_content
                            .pending_update
                            .set_terminal_damage(
                                rio_backend::event::TerminalDamage::CursorOnly,
                            );

                        route.request_redraw();
                    }
                }
            }
            RioEventType::Rio(RioEvent::ProgressReport(report)) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    if let Some(island) = &mut route.window.screen.renderer.island {
                        island.set_progress_report(report);
                        route.request_overlay_redraw();
                    }
                }
            }
            RioEventType::Rio(RioEvent::SelectionScrollTick) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    route.window.screen.selection_scroll_tick();
                    route.request_redraw();
                }
            }
            RioEventType::Rio(RioEvent::Bell(route_id)) => {
                // Handle audio bell
                if self.config.bell.audio {
                    self.handle_audio_bell();
                }

                if self.config.bell.tab_indicator {
                    if let Some(route) = self.router.routes.get_mut(&window_id) {
                        let focused = route.window.is_focused;
                        if route
                            .window
                            .screen
                            .context_manager
                            .ring_bell(route_id, focused)
                        {
                            route.request_overlay_redraw();
                        }
                    }
                }
            }
            RioEventType::Rio(RioEvent::DesktopNotification { title, body }) => {
                self.handle_desktop_notification(&title, &body);
            }
            RioEventType::Rio(RioEvent::PrepareRender(millis)) => {
                if let Some(route) = self.router.routes.get(&window_id) {
                    let timer_id = TimerId::new(
                        Topic::Render,
                        route.window.screen.ctx().current_route(),
                    );
                    let event =
                        EventPayload::new(RioEventType::Rio(RioEvent::Render), window_id);

                    if !self.scheduler.scheduled(timer_id) {
                        self.scheduler.schedule(
                            event,
                            Duration::from_millis(millis),
                            false,
                            timer_id,
                        );
                    }
                }
            }
            RioEventType::Rio(RioEvent::PrepareRenderOnRoute(millis, route_id)) => {
                let timer_id = TimerId::new(Topic::ScheduledRenderRoute, route_id);
                let event = EventPayload::new(
                    RioEventType::Rio(RioEvent::RenderRoute(route_id)),
                    window_id,
                );

                if !self.scheduler.scheduled(timer_id) {
                    self.scheduler.schedule(
                        event,
                        Duration::from_millis(millis),
                        false,
                        timer_id,
                    );
                }
            }
            RioEventType::Rio(RioEvent::BlinkCursor(millis, route_id)) => {
                let timer_id = TimerId::new(Topic::CursorBlinking, route_id);
                let event = EventPayload::new(
                    RioEventType::Rio(RioEvent::CursorBlinkingChangeOnRoute(route_id)),
                    window_id,
                );

                if !self.scheduler.scheduled(timer_id) {
                    self.scheduler.schedule(
                        event,
                        Duration::from_millis(millis),
                        false,
                        timer_id,
                    );
                }
            }
            RioEventType::Rio(RioEvent::Title(route_id, title)) => {
                self.handle_title_change(window_id, route_id, Some(&title));
            }
            RioEventType::Rio(RioEvent::CurrentDirectoryChanged(route_id)) => {
                self.handle_title_change(window_id, route_id, None);
            }
            RioEventType::Rio(RioEvent::SyncWindowTitle) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    route.sync_window_title();
                }
            }
            RioEventType::Rio(RioEvent::TitleWithSubtitle(title, subtitle)) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    route.set_window_title(&title);
                    route.set_window_subtitle(&subtitle);
                }
            }
            RioEventType::Rio(RioEvent::MouseCursorDirty) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    route.window.screen.reset_mouse();
                }
            }
            RioEventType::Rio(RioEvent::Scroll(scroll)) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    let mut terminal = route
                        .window
                        .screen
                        .context_manager
                        .current_mut()
                        .terminal
                        .lock();
                    terminal.scroll_display(scroll);
                    drop(terminal);
                }
            }
            RioEventType::Rio(RioEvent::ClipboardLoad(
                route_id,
                clipboard_type,
                format,
            )) => {
                let Router {
                    routes, clipboard, ..
                } = &mut self.router;
                if let Some(route) = routes.get_mut(&window_id) {
                    // `!focus_seen` mirrors the render gates: a window
                    // that never received a focus event yet (created in
                    // the background) must not have OSC 52 silently
                    // dropped by an is_focused that never initialized.
                    if route.window.is_focused || !route.window.focus_seen {
                        let text = format(clipboard.get(clipboard_type).as_str());
                        // Route the paste back to the panel that asked for it
                        // (OSC 52 reply), not whichever panel happens to be
                        // focused now.
                        if let Some(item) = route
                            .window
                            .screen
                            .context_manager
                            .get_by_route_id(route_id)
                        {
                            item.val.messenger.send_bytes(text.into_bytes());
                        }
                    }
                }
            }
            RioEventType::Rio(RioEvent::ClipboardStore(clipboard_type, content)) => {
                let Router {
                    routes, clipboard, ..
                } = &mut self.router;
                if let Some(route) = routes.get_mut(&window_id) {
                    if route.window.is_focused || !route.window.focus_seen {
                        clipboard.set(clipboard_type, content);
                    }
                }
            }
            RioEventType::Rio(RioEvent::PtyWrite(route_id, text)) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    // Route reply bytes (CSI / OSC responses) back to the
                    // PTY of the panel that emitted them, not whichever
                    // panel happens to be focused.
                    if let Some(item) = route
                        .window
                        .screen
                        .context_manager
                        .get_by_route_id(route_id)
                    {
                        item.val.messenger.send_bytes(text.into_bytes());
                    }
                }
            }
            RioEventType::Rio(RioEvent::TextAreaSizeRequest(route_id, format)) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    if let Some(item) = route
                        .window
                        .screen
                        .context_manager
                        .get_by_route_id(route_id)
                    {
                        let dimension = item.val.dimension;
                        let text = format(crate::renderer::utils::terminal_dimensions(
                            &dimension,
                        ));
                        item.val.messenger.send_bytes(text.into_bytes());
                    }
                }
            }
            RioEventType::Rio(RioEvent::ColorRequest(route_id, index, format)) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    // Read the originating panel's terminal colors and
                    // route the reply back to that same panel — color
                    // theme overrides via OSC 4 / OSC 10-19 are
                    // per-context, so reading from `current()` would
                    // mis-report when the user has focused a different
                    // split mid-flight.
                    let renderer_color = route.window.screen.renderer.colors[index];
                    let Some(item) = route
                        .window
                        .screen
                        .context_manager
                        .get_by_route_id(route_id)
                    else {
                        return;
                    };
                    let terminal = item.val.terminal.lock();
                    let color: ColorRgb = match terminal.colors()[index] {
                        Some(color) => ColorRgb::from_color_arr(color),
                        // Ignore cursor color requests unless it was changed.
                        None if index
                            == crate::crosswords::NamedColor::Cursor as usize =>
                        {
                            return
                        }
                        None => ColorRgb::from_color_arr(renderer_color),
                    };
                    drop(terminal);

                    item.val.messenger.send_bytes(format(color).into_bytes());
                }
            }
            RioEventType::Rio(RioEvent::CreateWindow) => {
                self.router.create_window(
                    event_loop,
                    self.event_proxy.clone(),
                    &self.config,
                    None,
                    self.app_id.as_deref(),
                );
            }
            RioEventType::Rio(RioEvent::ToggleQuake) => {
                self.toggle_quake_window(event_loop);
            }
            #[cfg(target_os = "macos")]
            RioEventType::Rio(RioEvent::CreateNativeTab(working_dir_overwrite)) => {
                if let Some(route) = self.router.routes.get(&window_id) {
                    // This case happens only for native tabs
                    // every time that a new tab is created through context
                    // it also reaches for the foreground process path if
                    // config.use_current_path is true
                    // For these case we need to make a workaround
                    let config = if working_dir_overwrite.is_some() {
                        rio_backend::config::Config {
                            working_dir: working_dir_overwrite,
                            ..self.config.clone()
                        }
                    } else {
                        self.config.clone()
                    };

                    self.router.create_native_tab(
                        event_loop,
                        self.event_proxy.clone(),
                        &config,
                        Some(&route.window.winit_window.tabbing_identifier()),
                        None,
                    );
                }
            }
            RioEventType::Rio(RioEvent::CreateConfigEditor) => {
                if self.config.navigation.open_config_with_split {
                    self.router.open_config_split(&self.config);
                } else {
                    self.router.open_config_window(
                        event_loop,
                        self.event_proxy.clone(),
                        &self.config,
                    );
                }
            }
            #[cfg(target_os = "macos")]
            RioEventType::Rio(RioEvent::CloseWindow) => {
                self.router.routes.remove(&window_id);
                if self.router.routes.is_empty() && !self.config.confirm_before_quit {
                    event_loop.exit();
                }
            }
            #[cfg(target_os = "macos")]
            RioEventType::Rio(RioEvent::SelectNativeTabByIndex(tab_index)) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    route.window.winit_window.select_tab_at_index(tab_index);
                }
            }
            #[cfg(target_os = "macos")]
            RioEventType::Rio(RioEvent::SelectNativeTabLast) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    route
                        .window
                        .winit_window
                        .select_tab_at_index(route.window.winit_window.num_tabs() - 1);
                }
            }
            #[cfg(target_os = "macos")]
            RioEventType::Rio(RioEvent::SelectNativeTabNext) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    route.window.winit_window.select_next_tab();
                }
            }
            #[cfg(target_os = "macos")]
            RioEventType::Rio(RioEvent::SelectNativeTabPrev) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    route.window.winit_window.select_previous_tab();
                }
            }
            #[cfg(target_os = "macos")]
            RioEventType::Rio(RioEvent::Hide) => {
                event_loop.hide_application();
            }
            #[cfg(target_os = "macos")]
            RioEventType::Rio(RioEvent::HideOtherApplications) => {
                event_loop.hide_other_applications();
            }
            RioEventType::Rio(RioEvent::Minimize(set_minimize)) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    route.window.winit_window.set_minimized(set_minimize);
                }
            }
            RioEventType::Rio(RioEvent::ToggleFullScreen) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    match route.window.winit_window.fullscreen() {
                        None => route
                            .window
                            .winit_window
                            .set_fullscreen(Some(Fullscreen::Borderless(None))),
                        _ => route.window.winit_window.set_fullscreen(None),
                    }
                }
            }
            RioEventType::Rio(RioEvent::ToggleAppearanceTheme) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    use rio_backend::config::theme::AppearanceTheme;
                    let current = self
                        .config
                        .force_theme
                        .or_else(|| {
                            route
                                .window
                                .winit_window
                                .theme()
                                .map(AppearanceTheme::from_window_theme)
                        })
                        .unwrap_or(AppearanceTheme::Dark);
                    let toggled = current.toggled();
                    self.config.force_theme = Some(toggled);
                    update_colors_based_on_theme(
                        &mut self.config,
                        Some(toggled.to_window_theme()),
                    );
                    route.window.screen.update_config(
                        &self.config,
                        &self.router.font_library,
                        false,
                    );
                    route.window.configure_window(&self.config);
                }
            }
            RioEventType::Rio(RioEvent::ColorChange(route_id, index, color)) => {
                if let Some(route) = self.router.routes.get_mut(&window_id) {
                    let screen = &mut route.window.screen;
                    // Background color is index 1 relative to NamedColor::Foreground
                    if index == NamedColor::Foreground as usize + 1 {
                        let grid = screen.context_manager.current_grid_mut();
                        // The event carries a `route_id: usize` (global
                        // counter). `ContextGrid::get_mut` is keyed on
                        // taffy `NodeId` — a different identifier space,
                        // so `get_mut(route_id.into())` effectively
                        // never matches. Look the panel up by its
                        // actual route id.
                        if let Some(context_item) = grid.get_by_route_id(route_id) {
                            use crate::context::renderable::BackgroundState;
                            context_item.context_mut().renderable_content.background =
                                Some(match color {
                                    Some(c) => BackgroundState::Set(c.to_wgpu()),
                                    None => BackgroundState::Reset,
                                });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    #[cfg(target_os = "macos")]
    fn open_urls(&mut self, active_event_loop: &ActiveEventLoop, urls: Vec<String>) {
        if !self.config.navigation.is_native() {
            let config = &self.config;
            for url in urls {
                self.router.create_window(
                    active_event_loop,
                    self.event_proxy.clone(),
                    config,
                    Some(url),
                    self.app_id.as_deref(),
                );
            }
            return;
        }

        let mut tab_id = None;

        // In case only have one window
        for (_, route) in self.router.routes.iter() {
            if tab_id.is_none() {
                tab_id = Some(route.window.winit_window.tabbing_identifier());
            }

            if route.window.is_focused {
                tab_id = Some(route.window.winit_window.tabbing_identifier());
                break;
            }
        }

        if tab_id.is_some() {
            let config = &self.config;
            for url in urls {
                self.router.create_native_tab(
                    active_event_loop,
                    self.event_proxy.clone(),
                    config,
                    tab_id.as_deref(),
                    Some(url),
                );
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        // Ignore all events we do not care about.
        if Self::skip_window_event(&event) {
            return;
        }

        // The event loop keys on rio-window's id; the router keys on the
        // core's `WindowId`. Convert once at this boundary.
        let window_id: rio_backend::event::WindowId = window_id.into();

        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            if let WindowEvent::TabDragFinished {
                destination,
                cancelled,
            } = &event
            {
                if self.native_tab_drag_source == Some(window_id) {
                    let destination = destination
                        .and_then(|(id, point)| self.tab_destination(id.into(), point));
                    if *cancelled
                        || !self.transfer_tab(event_loop, window_id, destination, None)
                    {
                        self.cancel_tab_transfer(window_id);
                    }
                }
                return;
            }
            if let Some(source_id) = self.active_tab_drag_source() {
                if matches!(&event, WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed
                    && event.logical_key == rio_window::keyboard::Key::Named(rio_window::keyboard::NamedKey::Escape))
                {
                    self.cancel_tab_transfer(source_id);
                    return;
                }
                if self.native_tab_drag_source.is_some()
                    && matches!(
                        &event,
                        WindowEvent::CursorMoved { .. } | WindowEvent::MouseInput { .. }
                    )
                {
                    let released = matches!(
                        &event,
                        WindowEvent::MouseInput {
                            state: ElementState::Released,
                            button: MouseButton::Left,
                            ..
                        }
                    );
                    self.poll_desktop_tab_drag(event_loop, released);
                    return;
                }
            }
        }

        #[cfg(windows)]
        if let Some(source_id) = self.active_tab_drag_source() {
            if matches!(&event, WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.logical_key == rio_window::keyboard::Key::Named(rio_window::keyboard::NamedKey::Escape))
            {
                self.cancel_tab_transfer(source_id);
                return;
            }
            if matches!(
                &event,
                WindowEvent::CursorMoved { .. } | WindowEvent::MouseInput { .. }
            ) {
                let released = matches!(
                    &event,
                    WindowEvent::MouseInput {
                        state: ElementState::Released,
                        button: MouseButton::Left,
                        ..
                    }
                );
                self.poll_tab_transfer(event_loop, released);
                return;
            }
        }

        #[cfg(windows)]
        if matches!(
            &event,
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } | WindowEvent::CloseRequested
        ) || matches!(&event, WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed
                && event.logical_key == rio_window::keyboard::Key::Named(rio_window::keyboard::NamedKey::Escape))
        {
            // Remove the click-through image before choosing the drop target.
            self.tab_drag_preview = None;
        }

        if let WindowEvent::CursorMoved { position, .. } = &event {
            self.tab_drag_position = Some((window_id, *position));
        }
        if matches!(
            &event,
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            }
        ) && self.finish_tab_drag(event_loop, window_id)
        {
            return;
        }

        let route = match self.router.routes.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };

        if matches!(&event, WindowEvent::KeyboardInput { event, .. }
            if event.state == ElementState::Pressed
                && event.logical_key == rio_window::keyboard::Key::Named(rio_window::keyboard::NamedKey::Escape))
        {
            if let Some(island) = &mut route.window.screen.renderer.island {
                if island.is_dragging() {
                    island.cancel_drag();
                    route.window.screen.mouse.left_button_state = ElementState::Released;
                    self.tab_drag_position = None;
                    route.request_redraw();
                    return;
                }
            }
        }

        match event {
            WindowEvent::CloseRequested => {
                // macOS: Cmd+Q quit confirmation is handled by
                // `applicationShouldTerminate` in rio-window.
                // Windows: per-window close confirmation is handled
                // by `MessageBoxW` in rio-window's WM_CLOSE handler
                // (see `set_confirm_before_quit` plumbing).
                // Either way, by the time we see `CloseRequested`
                // the user has already confirmed — just close.
                if cfg!(any(target_os = "macos", target_os = "windows")) {
                    self.router.routes.remove(&window_id);
                    if self.router.routes.is_empty() {
                        event_loop.exit();
                    }
                    return;
                }

                if self.config.confirm_before_quit {
                    route.confirm_quit();
                    return;
                } else {
                    self.router.routes.remove(&window_id);
                }

                if self.router.routes.is_empty() {
                    event_loop.exit();
                }
            }

            WindowEvent::ModifiersChanged(modifiers) => {
                route.window.screen.set_modifiers(modifiers);

                // Hint mods (cmd on macOS) are pressed with the pointer
                // already parked over the link, and `CursorMoved` only
                // recomputes hints when the pointer crosses a cell
                // boundary. Without refreshing here the link is never
                // highlighted, so the click that follows has nothing to
                // activate. The set half only runs with the pointer inside
                // the text area (a clamped chrome position must not light
                // up a link it is not over), but the clear half always
                // runs: a highlight left behind would outlive its modifier
                // and hijack the next plain click.
                if route.path == RoutePath::Terminal
                    && (if route.window.screen.mouse.inside_text_area {
                        route.window.screen.update_highlighted_hints()
                    } else {
                        route.window.screen.clear_highlighted_hint()
                    })
                {
                    route
                        .window
                        .winit_window
                        .set_cursor(route.window.screen.mouse_cursor_icon());
                    route.window.screen.context_manager.request_render();
                }
            }

            WindowEvent::MouseInput { state, button, .. } => {
                if route.path != RoutePath::Terminal
                    || route.window.screen.renderer.confirm_quit.is_active()
                {
                    #[cfg(target_os = "macos")]
                    if state == ElementState::Pressed
                        && button == MouseButton::Left
                        && route.window.screen.allow_manual_dragging
                    {
                        use crate::renderer::island::ISLAND_HEIGHT;
                        let scale = route.window.screen.sugarloaf.scale_factor();
                        if route.window.screen.mouse.y <= (ISLAND_HEIGHT * scale) as f64 {
                            let _ = route.window.winit_window.drag_window();
                        }
                    }
                    if state == ElementState::Pressed {
                        let _ = route.window.screen.take_chrome_press();
                    } else if state == ElementState::Released
                        && button == MouseButton::Left
                    {
                        route.window.screen.mouse.left_button_state =
                            ElementState::Released;
                        // A release swallowed here must also drop the hint
                        // latch, or a later chrome-consumed press would
                        // release against a hint it never landed on.
                        route.window.screen.mouse.hint_click_latched = None;
                        if let Some(ref mut island) = route.window.screen.renderer.island
                        {
                            island.cancel_drag();
                        }
                        route.window.screen.renderer.scrollbar.end_drag();
                        route.window.screen.resize_state = None;
                    }
                    return;
                }

                if self.config.hide_cursor_when_typing {
                    route.window.winit_window.set_cursor_visible(true);
                }

                match button {
                    MouseButton::Left => {
                        route.window.screen.mouse.left_button_state = state
                    }
                    MouseButton::Middle => {
                        route.window.screen.mouse.middle_button_state = state
                    }
                    MouseButton::Right => {
                        route.window.screen.mouse.right_button_state = state
                    }
                    _ => (),
                }

                match state {
                    ElementState::Pressed => {
                        // Calculate time since the last click to handle double/triple clicks.
                        // Do this early so island clicks can use the click state
                        let now = Instant::now();
                        let elapsed =
                            now - route.window.screen.mouse.last_click_timestamp;
                        route.window.screen.mouse.last_click_timestamp = now;

                        let threshold = crate::constants::MULTI_CLICK_THRESHOLD;
                        let mouse = &route.window.screen.mouse;
                        route.window.screen.mouse.click_state = match mouse.click_state {
                            // Reset click state if button has changed.
                            _ if button != mouse.last_click_button => {
                                route.window.screen.mouse.last_click_button = button;
                                ClickState::Click
                            }
                            ClickState::Click if elapsed < threshold => {
                                ClickState::DoubleClick
                            }
                            ClickState::DoubleClick if elapsed < threshold => {
                                ClickState::TripleClick
                            }
                            _ => ClickState::Click,
                        };

                        let chrome_press = route.window.screen.take_chrome_press();

                        if let MouseButton::Left = button {
                            // Check if clicking on a panel border to start resize
                            {
                                let mx = route.window.screen.mouse.x as f32;
                                let my = route.window.screen.mouse.y as f32;
                                let grid =
                                    route.window.screen.context_manager.current_grid();
                                if let Some(border) = grid.find_border_at_position(mx, my)
                                {
                                    let start_pos = match border.direction {
                                        crate::layout::BorderDirection::Vertical => mx,
                                        crate::layout::BorderDirection::Horizontal => my,
                                    };
                                    let size_a = grid.get_panel_size(
                                        border.left_or_top,
                                        border.direction,
                                    );
                                    let size_b = grid.get_panel_size(
                                        border.right_or_bottom,
                                        border.direction,
                                    );
                                    route.window.screen.resize_state =
                                        Some(crate::layout::ResizeState {
                                            border,
                                            start_pos,
                                            original_sizes: (size_a, size_b),
                                        });
                                    return;
                                }
                            }

                            if route.window.screen.handle_assistant_click() {
                                route.request_redraw();
                                return;
                            }

                            if route
                                .window
                                .screen
                                .handle_palette_click(&mut self.router.clipboard)
                            {
                                route.request_redraw();
                                return;
                            }

                            if route
                                .window
                                .screen
                                .handle_search_click(&mut self.router.clipboard)
                            {
                                route.request_redraw();
                                return;
                            }

                            let handled_by_island =
                                route.window.screen.handle_island_click(
                                    &route.window.winit_window,
                                    &mut self.router.clipboard,
                                    false,
                                    chrome_press,
                                );

                            if handled_by_island {
                                route.request_redraw();
                                return;
                            }

                            #[cfg(target_os = "macos")]
                            if route.window.screen.allow_manual_dragging {
                                use crate::renderer::island::ISLAND_HEIGHT;
                                let scale = route.window.screen.sugarloaf.scale_factor();
                                if route.window.screen.mouse.y
                                    <= (ISLAND_HEIGHT * scale) as f64
                                {
                                    route
                                        .window
                                        .screen
                                        .start_window_drag(&route.window.winit_window);
                                }
                            }

                            if route.window.screen.handle_scrollbar_click() {
                                route.request_redraw();
                                return;
                            }
                        } else if let MouseButton::Right = button {
                            let handled_by_island =
                                route.window.screen.handle_island_click(
                                    &route.window.winit_window,
                                    &mut self.router.clipboard,
                                    true,
                                    chrome_press,
                                );

                            if handled_by_island {
                                route.request_redraw();
                                return;
                            }
                        }

                        // Always try panel switching first: if the click
                        // targets a different panel, switch to it regardless
                        // of mouse mode (e.g. neovim capturing clicks).
                        //
                        // A left click on a highlighted hint bypasses mouse
                        // reporting the same way shift does: the hint's mods
                        // are held, so the user is following the link, not
                        // clicking inside the application. The hint itself is
                        // latched for the release handler: re-evaluating
                        // there would split a press from its release when
                        // the modifier changes mid-click, and the release
                        // must know which hint the press landed on.
                        let latched = if button == MouseButton::Left {
                            route.window.screen.highlighted_hint().cloned()
                        } else {
                            None
                        };
                        let hint_click = latched.is_some();
                        if button == MouseButton::Left {
                            route.window.screen.mouse.hint_click_latched = latched;
                        }

                        if route.window.screen.select_current_based_on_mouse() {
                            route.request_redraw();
                        } else if !route.window.screen.modifiers.state().shift_key()
                            && !hint_click
                            && route.window.screen.mouse_mode()
                        {
                            // Process mouse press before bindings to update the `click_state`.
                            route.window.screen.mouse.click_state = ClickState::None;

                            let code = match button {
                                MouseButton::Left => 0,
                                MouseButton::Middle => 1,
                                MouseButton::Right => 2,
                                // Can't properly report more than three buttons..
                                MouseButton::Back
                                | MouseButton::Forward
                                | MouseButton::Other(_) => return,
                            };

                            route
                                .window
                                .screen
                                .mouse_report(code, ElementState::Pressed);

                            route.window.screen.process_mouse_bindings(
                                button,
                                &mut self.router.clipboard,
                            );
                        } else {
                            // Load mouse point, treating message bar and padding as the closest square.
                            let display_offset = route.window.screen.display_offset();

                            if let MouseButton::Left = button {
                                let pos =
                                    route.window.screen.mouse_position(display_offset);
                                route
                                    .window
                                    .screen
                                    .on_left_click(pos, &mut self.router.clipboard);
                            }

                            route.request_redraw();
                        }
                        route
                            .window
                            .screen
                            .process_mouse_bindings(button, &mut self.router.clipboard);
                    }
                    ElementState::Released => {
                        // Stop selection auto-scroll on button release.
                        if let MouseButton::Left | MouseButton::Right = button {
                            let scroll_timer_id =
                                route.window.screen.ctx().current_route();
                            let timer_id =
                                TimerId::new(Topic::SelectionScrolling, scroll_timer_id);
                            self.scheduler.unschedule(timer_id);
                        }

                        if button == MouseButton::Left
                            && route
                                .window
                                .screen
                                .renderer
                                .island
                                .as_ref()
                                .is_some_and(|i| i.is_dragging())
                        {
                            let started = route.window.screen.handle_tab_drag_release();
                            if started {
                                route.request_redraw();
                                return;
                            }
                        }

                        if route.window.screen.renderer.scrollbar.is_dragging() {
                            route.window.screen.handle_scrollbar_release();
                            route.request_redraw();
                            return;
                        }

                        if route.window.screen.resize_state.is_some() {
                            route.window.screen.resize_state = None;
                            route.window.winit_window.set_cursor(CursorIcon::Default);
                            return;
                        }

                        // Consume the press handler's latched hint so press
                        // and release always take the same path, even when
                        // the hint modifier changed mid-click. The
                        // application never sees one without the other.
                        let latched_hint = if button == MouseButton::Left {
                            route.window.screen.mouse.hint_click_latched.take()
                        } else {
                            None
                        };
                        let hint_click = latched_hint.is_some();

                        if !route.window.screen.modifiers.state().shift_key()
                            && !hint_click
                            && route.window.screen.mouse_mode()
                        {
                            let code = match button {
                                MouseButton::Left => 0,
                                MouseButton::Middle => 1,
                                MouseButton::Right => 2,
                                // Can't properly report more than three buttons.
                                MouseButton::Back
                                | MouseButton::Forward
                                | MouseButton::Other(_) => return,
                            };
                            route
                                .window
                                .screen
                                .mouse_report(code, ElementState::Released);
                            return;
                        }

                        // Releasing a drag selection copies it (with
                        // copy-on-select) and must not activate a hint
                        // sitting under the release point; hints fire on
                        // plain clicks only, when no selection exists.
                        if route.window.screen.selection_is_empty() {
                            if button == MouseButton::Left {
                                // Only a latched press opens a link, and only
                                // when the release lands on the same span the
                                // press did. Mouse mode never turns the drag
                                // into a selection, so without the span check
                                // a press on one link released over another
                                // would open the wrong one; and a press the
                                // chrome consumed (which never latches) must
                                // not open a highlight it never touched. The
                                // latched match is what executes: a modifier
                                // change mid-click can swap which hint config
                                // the same span resolves to.
                                if let Some(latched) = latched_hint {
                                    let same_span = route
                                        .window
                                        .screen
                                        .highlighted_hint()
                                        .is_some_and(|h| {
                                            h.text == latched.text
                                                && h.start == latched.start
                                                && h.end == latched.end
                                        });
                                    if same_span {
                                        route.window.screen.open_latched_hint(
                                            latched,
                                            &mut self.router.clipboard,
                                        );
                                        // Paint the cleared highlight now: an
                                        // action that steals no focus (Copy)
                                        // schedules no frame of its own.
                                        route
                                            .window
                                            .screen
                                            .context_manager
                                            .request_render();
                                    }
                                }
                            }
                        } else if matches!(button, MouseButton::Left | MouseButton::Right)
                            && self.config.copy_on_select
                        {
                            route.window.screen.copy_selection(
                                ClipboardType::Clipboard,
                                &mut self.router.clipboard,
                            );
                        }
                    }
                }
            }

            WindowEvent::CursorLeft { .. } => {
                if route.window.screen.clear_close_button_hover() {
                    route.request_redraw();
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                if self.config.hide_cursor_when_typing {
                    route.window.winit_window.set_cursor_visible(true);
                }

                let layout = route.window.screen.sugarloaf.window_size();

                // Keep f64 precision all the way to the cell-grid
                // divide. The old `as usize` cast here dropped
                // subpixel info from HiDPI events.
                let x = position.x.clamp(0.0, (layout.width as i32 - 1) as f64);
                let y = position.y.clamp(0.0, (layout.height as i32 - 1) as f64);

                route.window.screen.mouse.x = x;
                route.window.screen.mouse.y = y;
                route.window.screen.mouse.raw_y = position.y;

                if route.path != RoutePath::Terminal
                    || route.window.screen.renderer.confirm_quit.is_active()
                {
                    route.window.winit_window.set_cursor(CursorIcon::Default);
                    return;
                }

                // Handle assistant overlay hover
                if route.window.screen.renderer.assistant.is_active() {
                    let scale = route.window.screen.sugarloaf.scale_factor();
                    let win_w = route.window.screen.sugarloaf.window_size().width;
                    let mx = x as f32 / scale;
                    let my = y as f32 / scale;
                    if route
                        .window
                        .screen
                        .renderer
                        .assistant
                        .hover(mx, my, win_w, scale)
                    {
                        route.request_overlay_redraw();
                    }

                    if route
                        .window
                        .screen
                        .renderer
                        .assistant
                        .hovered_button()
                        .is_some()
                    {
                        route.window.winit_window.set_cursor(CursorIcon::Pointer);
                    } else {
                        route.window.winit_window.set_cursor(CursorIcon::Default);
                    }
                    return;
                }

                // Handle command palette hover
                if route.window.screen.renderer.command_palette.is_enabled() {
                    let scale = route.window.screen.sugarloaf.scale_factor();
                    let win_w = route.window.screen.sugarloaf.window_size().width;
                    let mx = x as f32 / scale;
                    let my = y as f32 / scale;
                    if route
                        .window
                        .screen
                        .renderer
                        .command_palette
                        .hover(mx, my, win_w, scale)
                    {
                        route.request_overlay_redraw();
                    }
                    route.window.winit_window.set_cursor(CursorIcon::Default);
                    return;
                }

                // Handle search overlay hover
                if route.window.screen.renderer.search.is_active() {
                    let scale = route.window.screen.sugarloaf.scale_factor();
                    let win_w = route.window.screen.sugarloaf.window_size().width;
                    let mx = x as f32 / scale;
                    let my = y as f32 / scale;
                    if route
                        .window
                        .screen
                        .renderer
                        .search
                        .hover(mx, my, win_w, scale)
                    {
                        // UI-only change (hover highlight). `set_dirty`
                        // passes `Renderer::run`'s per-context gate;
                        // the inner damage match hits
                        // `(None, None) => TerminalDamage::Noop` so
                        // no rows rebuild. The search overlay itself
                        // is drawn unconditionally after the per-context
                        // loop in `Renderer::run`.
                        route
                            .window
                            .screen
                            .ctx_mut()
                            .current_mut()
                            .renderable_content
                            .pending_update
                            .set_dirty();
                        route.request_redraw();
                    }
                }

                if route.window.screen.mouse.left_button_state == ElementState::Pressed
                    && route
                        .window
                        .screen
                        .renderer
                        .island
                        .as_ref()
                        .is_some_and(|i| i.is_dragging())
                {
                    let scale = route.window.screen.sugarloaf.scale_factor();
                    let external = tab_left_strip(
                        position.x,
                        position.y,
                        layout.width as f64,
                        scale as f64,
                    );
                    if external {
                        if let Some(island) = &mut route.window.screen.renderer.island {
                            island.activate_drag();
                        }
                    }
                    route
                        .window
                        .screen
                        .handle_tab_drag_move(position.x as f32 / scale, external);
                    #[cfg(windows)]
                    update_tab_drag_preview(
                        &mut self.tab_drag_preview,
                        route,
                        position,
                        external,
                    );
                    route.window.winit_window.set_cursor(CursorIcon::Default);
                    route.request_redraw();
                    return;
                }

                if route.window.screen.update_close_button_hover(x, y) {
                    route.request_redraw();
                }

                // The macOS full-size content view keeps this band as custom
                // window chrome even when hide-if-single hides the island.
                // Other platforms only reserve it while the island is drawn.
                use crate::renderer::island::ISLAND_HEIGHT;
                let scale_factor = route.window.screen.sugarloaf.scale_factor();
                let island_height_px = (ISLAND_HEIGHT * scale_factor) as f64;
                let num_tabs = route.window.screen.ctx().len();
                let nav = &route.window.screen.renderer.navigation;
                if nav.chrome_band_reserved(num_tabs) && y <= island_height_px {
                    route.window.winit_window.set_cursor(CursorIcon::Default);
                    return;
                }

                // Handle scrollbar drag
                if route.window.screen.renderer.scrollbar.is_dragging() {
                    let scale = route.window.screen.sugarloaf.scale_factor();
                    let mouse_y = y as f32 / scale;
                    route.window.screen.handle_scrollbar_drag(mouse_y);
                    route.window.winit_window.set_cursor(CursorIcon::Default);
                    route.request_redraw();
                    return;
                }

                // Handle panel border resize
                if route.window.screen.resize_state.is_some() {
                    let state = route.window.screen.resize_state.unwrap();
                    let current_pos = match state.border.direction {
                        crate::layout::BorderDirection::Vertical => x as f32,
                        crate::layout::BorderDirection::Horizontal => y as f32,
                    };
                    let delta = current_pos - state.start_pos;
                    let border = state.border;
                    let original_sizes = state.original_sizes;
                    route
                        .window
                        .screen
                        .context_manager
                        .current_grid_mut()
                        .resize_border(
                            &border,
                            original_sizes,
                            delta,
                            &mut route.window.screen.sugarloaf,
                        );
                    // Dragging a split divider displaces panel origins;
                    // that is layout, not cursor travel.
                    route.window.screen.renderer.trail_cursor.snap();
                    let cursor = match border.direction {
                        crate::layout::BorderDirection::Vertical => CursorIcon::ColResize,
                        crate::layout::BorderDirection::Horizontal => {
                            CursorIcon::RowResize
                        }
                    };
                    route.window.winit_window.set_cursor(cursor);
                    route.window.screen.context_manager.request_render();
                    route.request_redraw();
                    return;
                }

                // Check if hovering over a panel border
                {
                    let grid = route.window.screen.context_manager.current_grid();
                    if let Some(border) = grid.find_border_at_position(x as f32, y as f32)
                    {
                        let cursor = match border.direction {
                            crate::layout::BorderDirection::Vertical => {
                                CursorIcon::ColResize
                            }
                            crate::layout::BorderDirection::Horizontal => {
                                CursorIcon::RowResize
                            }
                        };
                        route.window.winit_window.set_cursor(cursor);
                        route.window.screen.mouse.on_border = true;
                        return;
                    }
                }

                // Check if hovering over scrollbar
                if route.window.screen.is_hovering_scrollbar() {
                    route.window.winit_window.set_cursor(CursorIcon::Default);
                    return;
                }

                // Track leaving a border to force cursor reset below
                let was_on_border = route.window.screen.mouse.on_border;
                route.window.screen.mouse.on_border = false;

                let lmb_pressed =
                    route.window.screen.mouse.left_button_state == ElementState::Pressed;
                let rmb_pressed =
                    route.window.screen.mouse.right_button_state == ElementState::Pressed;

                let has_selection = !route.window.screen.selection_is_empty();
                if has_selection && (lmb_pressed || rmb_pressed) {
                    // Only start the timer when the mouse enters the scroll
                    // zone. Once running, the tick reads mouse.raw_y each
                    // iteration so it keeps scrolling after CursorMoved
                    // stops (mouse left window). Cancelled on button release.
                    let delta = route.window.screen.selection_scroll_delta(position.y);
                    if delta != 0 {
                        let scroll_timer_id = route.window.screen.ctx().current_route();
                        let timer_id =
                            TimerId::new(Topic::SelectionScrolling, scroll_timer_id);
                        if !self.scheduler.scheduled(timer_id) {
                            let event = EventPayload::new(
                                RioEventType::Rio(RioEvent::SelectionScrollTick),
                                window_id,
                            );
                            self.scheduler.schedule(
                                event,
                                Duration::from_millis(15),
                                true,
                                timer_id,
                            );
                        }
                    }
                }

                let display_offset = route.window.screen.display_offset();
                let point = route.window.screen.mouse_position(display_offset);

                // Compare *cell* coordinates, not pixel coordinates, so
                // subpixel HiDPI jitter inside the same cell doesn't
                // re-fire hint / OSC-8 / hyperlink work every event.
                let prev_cell = route.window.screen.mouse.last_cell;
                let cell_changed = prev_cell != Some(point);
                route.window.screen.mouse.last_cell = Some(point);

                let inside_text_area = route.window.screen.contains_point(x, y);
                let square_side = route.window.screen.side_by_pos(x);

                // If the cursor hasn't changed cells, do nothing.
                // Force update when transitioning off a border so the cursor resets.
                if !cell_changed
                    && !was_on_border
                    && route.window.screen.mouse.square_side == square_side
                    && route.window.screen.mouse.inside_text_area == inside_text_area
                {
                    return;
                }

                // Skip hint/hyperlink highlighting during active selection
                // drag to avoid unnecessary terminal locks and regex matching.
                let is_selecting = (lmb_pressed || rmb_pressed)
                    && (route.window.screen.modifiers.state().shift_key()
                        || !route.window.screen.mouse_mode());

                if !is_selecting {
                    let hint_changed = route.window.screen.update_highlighted_hints();
                    route
                        .window
                        .winit_window
                        .set_cursor(route.window.screen.mouse_cursor_icon());

                    if hint_changed {
                        route.window.screen.context_manager.request_render();
                    }
                }

                route.window.screen.mouse.inside_text_area = inside_text_area;
                route.window.screen.mouse.square_side = square_side;

                if is_selecting {
                    route.window.screen.update_selection(point, square_side);
                    route.window.screen.context_manager.request_render();
                } else if cell_changed && route.window.screen.has_mouse_motion_and_drag()
                {
                    if lmb_pressed {
                        // A latched hint click hides its press and release
                        // from the application; a drag report leaking out
                        // mid-click would arrive with no press around it.
                        if route.window.screen.mouse.hint_click_latched.is_none() {
                            route.window.screen.mouse_report(32, ElementState::Pressed);
                        }
                    } else if route.window.screen.mouse.middle_button_state
                        == ElementState::Pressed
                    {
                        route.window.screen.mouse_report(33, ElementState::Pressed);
                    } else if route.window.screen.mouse.right_button_state
                        == ElementState::Pressed
                    {
                        route.window.screen.mouse_report(34, ElementState::Pressed);
                    } else if route.window.screen.has_mouse_motion() {
                        route.window.screen.mouse_report(35, ElementState::Pressed);
                    }
                }
            }

            WindowEvent::MouseWheel { delta, phase, .. } => {
                if route.path != RoutePath::Terminal
                    || route.window.screen.renderer.confirm_quit.is_active()
                {
                    return;
                }

                if self.config.hide_cursor_when_typing {
                    route.window.winit_window.set_cursor_visible(true);
                }

                match delta {
                    MouseScrollDelta::LineDelta(columns, lines) => {
                        // One wheel notch is one line/column. Convert
                        // with the cell size: scroll() divides the
                        // accumulated pixels by it, and converting
                        // with font_size (smaller than a cell) made
                        // single notches floor to zero lines (#1350).
                        let cell =
                            route.window.screen.ctx().current().dimension.dimension;
                        if cell.width > 0.0 && cell.height > 0.0 {
                            let new_scroll_px_x = columns * cell.width;
                            let new_scroll_px_y = lines * cell.height;
                            route
                                .window
                                .screen
                                .scroll(new_scroll_px_x as f64, new_scroll_px_y as f64);
                        }
                    }
                    MouseScrollDelta::PixelDelta(mut lpos) => {
                        match phase {
                            TouchPhase::Started => {
                                // Reset offset to zero.
                                route.window.screen.mouse.accumulated_scroll =
                                    Default::default();
                            }
                            TouchPhase::Moved => {
                                // When the angle between (x, 0) and (x, y) is lower than ~25 degrees
                                // (cosine is larger that 0.9) we consider this scrolling as horizontal.
                                if lpos.x.abs() / lpos.x.hypot(lpos.y) > 0.9 {
                                    lpos.y = 0.;
                                } else {
                                    lpos.x = 0.;
                                }

                                route.window.screen.scroll(lpos.x, lpos.y);
                            }
                            _ => (),
                        }
                    }
                }

                route.request_redraw();
            }

            WindowEvent::KeyboardInput {
                is_synthetic: false,
                event: key_event,
                ..
            } => {
                if route.has_key_wait(&key_event, &mut self.router.clipboard) {
                    if route.path != RoutePath::Terminal
                        && key_event.state == ElementState::Released
                    {
                        // Scheduler must be cleaned after leave the terminal route
                        self.scheduler.unschedule(TimerId::new(
                            Topic::Render,
                            route.window.screen.ctx().current_route(),
                        ));
                    }
                    return;
                }

                route.window.screen.context_manager.set_last_typing();
                route
                    .window
                    .screen
                    .process_key_event(&key_event, &mut self.router.clipboard);
                // `process_key_event` used to call `self.render()` for
                // local-only keystrokes (VI mode, search input, hint
                // mode). Now it just marks `pending_update.set_dirty()`
                // through `mark_dirty`. Request a redraw so the next
                // vsync fires `RedrawRequested` — PTY-bound keystrokes
                // also flow through here but their render is idempotent
                // with the PTY-damage-driven redraw.
                route.request_redraw();

                if key_event.state == ElementState::Released
                    && self.config.hide_cursor_when_typing
                {
                    route.window.winit_window.set_cursor_visible(false);
                }
            }

            WindowEvent::Ime(ime) => {
                // Modal overlays own keyboard input (`modal_owns_input`
                // walks the `has_key_wait` roster): while one is up,
                // composition input must not reach the terminal, but
                // any stored preedit must still CLEAR: a live
                // composition would keep painting into the grid behind
                // the overlay and its key gate would swallow every
                // plain keystroke after the overlay closes. Search
                // stays open to IME (commits route into the search
                // input via `paste`).
                match ime {
                    Ime::Commit(text) => {
                        // Text-input overlays (island rename, palette)
                        // consume commits first, in `has_key_wait`'s
                        // order; other modals swallow them; only a bare
                        // terminal receives the text.
                        if route.overlay_commit_text(&text) {
                            return;
                        }
                        if route.modal_owns_input() {
                            return;
                        }
                        // Don't use bracketed paste for single char input.
                        route.window.screen.paste(&text, text.chars().count() > 1);
                    }
                    Ime::Preedit(text, cursor_offset) => {
                        let preedit = if text.is_empty() || route.modal_owns_input() {
                            None
                        } else {
                            // The platform's `None` means the IME asked
                            // for a hidden caret (candidate paging),
                            // NOT end-of-text, which arrives as an
                            // explicit offset.
                            let cursor = match cursor_offset {
                                Some((start, _)) => PreeditCursor::Byte(start),
                                None => PreeditCursor::Hidden,
                            };
                            Some(Preedit::new(text, cursor))
                        };

                        // `set_ime_preedit` owns the composing side
                        // effects (scroll snap, selection, dirty mark)
                        // and their search-mode exceptions.
                        if route.window.screen.set_ime_preedit(preedit) {
                            route.request_redraw();
                        }
                    }
                    Ime::Enabled => {
                        route.window.screen.ime.set_enabled(true);
                    }
                    Ime::Disabled => {
                        // Disabling wipes any live preedit (input
                        // source switched mid-composition): mark dirty
                        // and repaint like the Preedit arm, or the
                        // block ghosts on screen until unrelated
                        // damage.
                        let had_preedit = route.window.screen.ime.preedit().is_some();
                        route.window.screen.ime.set_enabled(false);
                        if had_preedit {
                            route.window.screen.mark_dirty();
                            route.request_redraw();
                        }
                    }
                }
            }
            WindowEvent::Touch(touch) => {
                on_touch(route, touch, &mut self.router.clipboard);
            }

            WindowEvent::Focused(focused) => {
                if self.config.hide_cursor_when_typing {
                    route.window.winit_window.set_cursor_visible(true);
                }

                let focus_changed = route.window.is_focused != focused;
                route.window.is_focused = focused;
                route.window.focus_seen = true;

                // Focus is a cheap checkpoint to catch backing-scale changes
                // whose ScaleFactorChanged never arrived (sleep/wake display
                // reconfiguration).
                if focused
                    && route
                        .window
                        .screen
                        .reconcile_scale(&route.window.winit_window)
                {
                    route.window.update_vblank_interval();
                    route.request_redraw();
                } else if focus_changed {
                    route.request_redraw();
                }

                route.window.screen.on_focus_change(focused);
            }

            WindowEvent::Occluded(occluded) => {
                let was_occluded = route.window.is_occluded;
                route.window.is_occluded = occluded;

                // If window was occluded and is now visible, mark for one-time render
                if was_occluded && !occluded {
                    route.window.needs_render_after_occlusion = true;
                    // Same checkpoint as focus: the un-occlusion after wake
                    // is often the first event the window receives.
                    if route
                        .window
                        .screen
                        .reconcile_scale(&route.window.winit_window)
                    {
                        route.window.update_vblank_interval();
                    }
                    // An idle terminal produces no PTY traffic to trigger the
                    // deferred post-occlusion render; request it directly.
                    route.request_redraw();
                }
            }

            WindowEvent::ThemeChanged(new_theme) => {
                if self.config.force_theme.is_some() {
                    return;
                }
                update_colors_based_on_theme(&mut self.config, Some(new_theme));
                route.window.screen.update_config(
                    &self.config,
                    &self.router.font_library,
                    false,
                );
                route.window.configure_window(&self.config);
                route.request_redraw();
            }

            WindowEvent::DroppedFile(path) => {
                if route.window.screen.renderer.assistant.is_active() {
                    return;
                }

                let path = crate::platform::shell_escape(&path.to_string_lossy());
                route.window.screen.paste(&(path + " "), true);
            }

            WindowEvent::Resized(new_size) => {
                if new_size.width == 0 || new_size.height == 0 {
                    return;
                }

                route.window.screen.resize(new_size);
                if self.title_tracks_size {
                    route.window.screen.refresh_titles();
                }
                route.request_redraw();
            }

            WindowEvent::ScaleFactorChanged {
                inner_size_writer: _,
                scale_factor,
            } => {
                let scale = scale_factor as f32;
                route
                    .window
                    .screen
                    .set_scale(scale, route.window.winit_window.inner_size());
                route.window.update_vblank_interval();
                route.request_redraw();
            }

            WindowEvent::RedrawRequested => {
                route.begin_render();
                let continuous_present = route
                    .window
                    .needs_continuous_present(self.config.renderer.use_cpu);
                let mut frame_presented = false;

                match route.path {
                    RoutePath::Welcome => {
                        route.window.screen.render_welcome();
                    }
                    RoutePath::Terminal => {
                        let (window_update, presented) =
                            route.window.screen.render(continuous_present);
                        frame_presented = presented;
                        if let Some(window_update) = window_update {
                            use crate::context::renderable::{
                                BackgroundState, WindowUpdate,
                            };
                            match window_update {
                                WindowUpdate::Background(bg_state) => {
                                    // for now setting this as allowed because it fails on linux builds
                                    #[allow(unused_variables)]
                                    let bg_color = match bg_state {
                                        BackgroundState::Set(color) => color,
                                        BackgroundState::Reset => {
                                            self.config.colors.background.1
                                        }
                                    };

                                    #[cfg(target_os = "macos")]
                                    {
                                        route.window.winit_window.set_background_color(
                                            bg_color.r, bg_color.g, bg_color.b,
                                            bg_color.a,
                                        );
                                    }

                                    #[cfg(target_os = "windows")]
                                    {
                                        use rio_window::platform::windows::WindowExtWindows;
                                        route
                                            .window
                                            .winit_window
                                            .set_title_bar_background_color(
                                                bg_color.r, bg_color.g, bg_color.b,
                                                bg_color.a,
                                            );
                                    }
                                }
                            }
                        }

                        // Update IME cursor position after rendering to ensure it's current
                        route.window.screen.update_ime_cursor_position_if_needed(
                            &route.window.winit_window,
                        );
                    }
                }

                #[cfg(target_os = "windows")]
                if !route.window.initial_frame_rendered {
                    // Keep the native window cloaked until the first complete
                    // render has been submitted. Uncloaking earlier can expose
                    // the blank native surface during GPU/PTY startup.
                    use rio_window::platform::windows::WindowExtWindows;
                    route.window.winit_window.set_cloaked(false);
                    route.window.initial_frame_rendered = true;
                }

                // let duration = start.elapsed();
                // println!("Time elapsed in render() is: {:?}", duration);
                // }

                // Continuous Windows presentation is paced by the FIFO
                // swapchain, not by an additional DwmFlush tick. Fall back
                // to display-paced requests after a failed acquisition so
                // an unavailable surface cannot spin the event loop.
                if self.config.renderer.strategy.is_game() {
                    route.request_redraw();
                    event_loop.set_control_flow(ControlFlow::Poll);
                } else {
                    if continuous_present && frame_presented {
                        #[cfg(target_os = "windows")]
                        {
                            use rio_window::platform::windows::WindowExtWindows;
                            route.window.winit_window.request_redraw_swapchain_paced();
                        }
                    } else if continuous_present
                        || route
                            .window
                            .screen
                            .ctx()
                            .current()
                            .renderable_content
                            .pending_update
                            .is_dirty()
                    {
                        route.request_redraw();
                    }
                    event_loop.set_control_flow(ControlFlow::Wait);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(windows)]
        self.poll_tab_transfer(event_loop, false);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        self.poll_desktop_tab_drag(event_loop, false);
        #[cfg(windows)]
        if !self
            .tab_drag_position
            .and_then(|(id, _)| self.router.routes.get(&id))
            .is_some_and(|route| {
                route.window.screen.tab_transfer_active()
                    && route.window.screen.mouse.left_button_state
                        == ElementState::Pressed
            })
        {
            self.tab_drag_preview = None;
        }
        // A confirmed quit can be set from any input path (keys today,
        // a future mouse or menu handler tomorrow); honoring it here,
        // once per event batch, keeps the exit independent of which
        // handler set it. exiting() then drops every route.
        if self
            .router
            .routes
            .values()
            .any(|route| route.quit_requested)
        {
            event_loop.exit();
            return;
        }
        let control_flow = match self.scheduler.update() {
            Some(instant) => ControlFlow::WaitUntil(instant),
            None => ControlFlow::Wait,
        };
        #[cfg(windows)]
        let control_flow = if self.active_tab_drag_source().is_some() {
            let next = Instant::now() + Duration::from_millis(16);
            match control_flow {
                ControlFlow::WaitUntil(deadline) => {
                    ControlFlow::WaitUntil(deadline.min(next))
                }
                _ => ControlFlow::WaitUntil(next),
            }
        } else {
            control_flow
        };
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let control_flow = if self.desktop_tab_drag.is_some() {
            let next = Instant::now() + Duration::from_millis(16);
            match control_flow {
                ControlFlow::WaitUntil(deadline) => {
                    ControlFlow::WaitUntil(deadline.min(next))
                }
                _ => ControlFlow::WaitUntil(next),
            }
        } else {
            control_flow
        };
        event_loop.set_control_flow(control_flow);
    }

    fn open_config(&mut self, event_loop: &ActiveEventLoop) {
        if self.config.navigation.open_config_with_split {
            self.router.open_config_split(&self.config);
        } else {
            self.router.open_config_window(
                event_loop,
                self.event_proxy.clone(),
                &self.config,
            );
        }
    }

    fn hook_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        key: &rio_window::event::KeyEvent,
        modifiers: &rio_window::event::Modifiers,
    ) {
        let window_id = match self.router.get_focused_route() {
            Some(window_id) => window_id,
            None => return,
        };

        let route = match self.router.routes.get_mut(&window_id) {
            Some(window) => window,
            None => return,
        };

        // For menu-triggered events, we need to temporarily set the correct modifiers
        // since menu events don't trigger ModifiersChanged events.
        let original_modifiers = route.window.screen.modifiers;

        // Use the modifiers passed from the menu action
        route.window.screen.set_modifiers(*modifiers);

        // Process the key event
        route
            .window
            .screen
            .process_key_event(key, &mut self.router.clipboard);

        // Restore the original modifiers
        route.window.screen.set_modifiers(original_modifiers);
    }

    // Emitted when the event loop is being shut down.
    // This is irreversible - if this event is emitted, it is guaranteed to be the last event that gets emitted.
    // You generally want to treat this as an “do on quit” event.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        #[cfg(windows)]
        {
            self.tab_drag_preview = None;
        }
        // Ensure that all the windows are dropped, so the destructors for
        // Renderer and contexts ran.
        self.router.routes.clear();

        // SAFETY: The clipboard must be dropped before the event loop, so
        // replace it with a safe no-op placeholder.
        self.router.clipboard = Clipboard::new_nop();

        std::process::exit(0);
    }
}

#[cfg(all(
    feature = "audio",
    not(target_os = "macos"),
    not(target_os = "windows")
))]
fn play_bell_sound() -> Result<(), Box<dyn Error>> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("No output device available")?;

    let config = device.default_output_config()?;

    match config.sample_format() {
        cpal::SampleFormat::F32 => run_bell::<f32>(&device, &config.into()),
        cpal::SampleFormat::I16 => run_bell::<i16>(&device, &config.into()),
        cpal::SampleFormat::U16 => run_bell::<u16>(&device, &config.into()),
        _ => Err("Unsupported sample format".into()),
    }
}

#[cfg(all(
    feature = "audio",
    not(target_os = "macos"),
    not(target_os = "windows")
))]
fn run_bell<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
) -> Result<(), Box<dyn Error>>
where
    T: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32>,
{
    let sample_rate = config.sample_rate.0 as f32;
    let channels = config.channels as usize;
    let duration_secs = crate::constants::BELL_DURATION.as_secs_f32();
    let total_samples = (sample_rate * duration_secs) as usize;

    let mut sample_clock = 0f32;
    let mut samples_played = 0usize;

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            for frame in data.chunks_mut(channels) {
                if samples_played >= total_samples {
                    for sample in frame.iter_mut() {
                        *sample = T::from_sample(0.0);
                    }
                } else {
                    let value = (sample_clock * 440.0 * 2.0 * std::f32::consts::PI
                        / sample_rate)
                        .sin()
                        * 0.2;
                    for sample in frame.iter_mut() {
                        *sample = T::from_sample(value);
                    }
                    sample_clock += 1.0;
                    samples_played += 1;
                }
            }
        },
        |err| tracing::error!("Audio stream error: {}", err),
        None,
    )?;

    stream.play()?;
    std::thread::sleep(crate::constants::BELL_DURATION);

    Ok(())
}
