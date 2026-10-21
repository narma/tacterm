//! Tab gesture lifecycle, native feedback, drop targeting and session transfer.
//! Application routes input here; platform backends own only native resources.
use super::*;

#[derive(Default)]
pub(super) struct TabDragState {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    native: NativeDrag,
    position: Option<(
        rio_backend::event::WindowId,
        rio_window::dpi::PhysicalPosition<f64>,
    )>,
    #[cfg(windows)]
    preview: Option<std::io::Result<crate::platform::windows::TabDragPreview>>,
    #[cfg(windows)]
    sample: Option<(rio_backend::event::WindowId, i32, i32, bool)>,
}

// Source and native resources enter and leave the gesture together. In
// particular a Wayland gesture has a source but no desktop polling resource.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Default)]
enum NativeDrag {
    #[default]
    Idle,
    Desktop {
        source: rio_backend::event::WindowId,
        drag: crate::platform::tab_drag::DesktopDrag,
    },
    Compositor {
        source: rio_backend::event::WindowId,
    },
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl NativeDrag {
    fn source(&self) -> Option<rio_backend::event::WindowId> {
        match self {
            Self::Idle => None,
            Self::Desktop { source, .. } | Self::Compositor { source } => Some(*source),
        }
    }
    fn desktop(&self) -> Option<&crate::platform::tab_drag::DesktopDrag> {
        match self {
            Self::Desktop { drag, .. } => Some(drag),
            _ => None,
        }
    }
    fn desktop_mut(&mut self) -> Option<&mut crate::platform::tab_drag::DesktopDrag> {
        match self {
            Self::Desktop { drag, .. } => Some(drag),
            _ => None,
        }
    }
}

impl TabDragState {
    pub(super) fn move_in_strip(
        &mut self,
        route: &mut crate::router::Route<'_>,
        position: rio_window::dpi::PhysicalPosition<f64>,
    ) {
        let scale = route.window.screen.sugarloaf.scale_factor();
        let external = tab_left_strip(
            position.x,
            position.y,
            route.window.winit_window.inner_size().width as f64,
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
        update_tab_drag_preview(&mut self.preview, route, position, external);
    }
}

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
        if self.tab_drag.native.source() == Some(source_id)
            && self.tab_drag.native.desktop().is_none()
        {
            use rio_window::platform::wayland::WindowExtWayland;
            if let Some(route) = self.router.routes.get(&source_id) {
                route.window.winit_window.cancel_tab_drag();
            }
        }
        #[cfg(windows)]
        {
            self.tab_drag.preview = None;
            self.tab_drag.sample = None;
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            self.tab_drag.native = NativeDrag::Idle;
        }
        self.tab_drag.position = None;
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
        self.tab_drag.position = Some((source_id, position));
        if released || !state.left_down {
            self.tab_drag.preview = None;
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
        if self.tab_drag.sample == Some(sample) {
            return;
        }
        self.tab_drag.sample = Some(sample);
        source
            .window
            .screen
            .handle_tab_drag_move(position.x as f32 / scale, external);
        update_tab_drag_preview(&mut self.tab_drag.preview, source, position, external);
        source.request_redraw();
    }

    fn tab_destination(
        &self,
        id: rio_backend::event::WindowId,
        point: rio_window::dpi::PhysicalPosition<f64>,
    ) -> Option<(rio_backend::event::WindowId, usize)> {
        let route = self.router.routes.get(&id)?;
        let screen = &route.window.screen;
        let scale = screen.sugarloaf.scale_factor() as f64;
        if route.path != RoutePath::Terminal
            || route.window.winit_window.is_visible() == Some(false)
            || route.window.winit_window.is_minimized() == Some(true)
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
    fn poll_desktop_tab_drag(&mut self, event_loop: &ActiveEventLoop) {
        let Some(source_id) = self.active_tab_drag_source() else {
            self.tab_drag.native = NativeDrag::Idle;
            return;
        };
        if self.tab_drag.native.source().is_none() {
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
                    && !self.tab_drag.position.is_some_and(|(_, p)| {
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
                (pixel_width, pixel_height),
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
                Ok(Some(drag)) => {
                    self.tab_drag.native = NativeDrag::Desktop {
                        source: source_id,
                        drag,
                    }
                }
                Ok(None) => {
                    self.tab_drag.native = NativeDrag::Compositor { source: source_id };
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
        }
        let Some(drag) = self.tab_drag.native.desktop_mut() else {
            return;
        };
        let Some(source) = self.router.routes.get_mut(&source_id) else {
            return;
        };
        let pointer = match drag.sample(&source.window.winit_window) {
            Ok(Some(p)) => p,
            Ok(None) => return,
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
        if !pointer.down {
            if let Err(error) = drag.show(false) {
                tracing::warn!("Could not hide tab preview: {error}");
            }
            let Some(drag) = self.tab_drag.native.desktop() else {
                return;
            };
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
        if self.tab_drag.position != Some((source_id, pointer.position))
            || source
                .window
                .screen
                .renderer
                .island
                .as_ref()
                .is_some_and(|i| i.drag_is_external() != external)
        {
            self.tab_drag.position = Some((source_id, pointer.position));
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
        let Some((position_window, position)) = self.tab_drag.position else {
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
            if *id == source_id { return None; }
            #[cfg(windows)]
            {
                use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                let handle = route.window.winit_window.window_handle().ok()?;
                if !matches!(handle.as_raw(), RawWindowHandle::Win32(handle) if handle.hwnd.get() as windows_sys::Win32::Foundation::HWND == under_pointer) { return None; }
            }
            let origin = route.window.winit_window.inner_position().ok()?;
            self.tab_destination(*id, rio_window::dpi::PhysicalPosition::new(
                (point.x - origin.x) as f64, (point.y - origin.y) as f64))
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
        if !new_window {
            target_screen
                .context_manager
                .current_mut()
                .report_focus(false);
        }
        target_screen.context_manager.insert_tab(tab, index);
        target_screen.finish_tab_transfer(destination.window.winit_window.inner_size());
        // An already active destination produces no native Focused event.
        target_screen
            .context_manager
            .current_mut()
            .report_focus(destination.window.winit_window.has_focus());
        destination.sync_window_title();
        destination.window.winit_window.focus_window();
        destination.request_redraw();
        if source_screen.context_manager.len() > 0 {
            source_screen.finish_tab_transfer(source.window.winit_window.inner_size());
            source_screen
                .context_manager
                .current_mut()
                .report_focus(source.window.winit_window.has_focus());
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
        self.tab_drag.position = None;
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            self.tab_drag.native = NativeDrag::Idle;
        }
        #[cfg(windows)]
        {
            self.tab_drag.sample = None;
        }
        true
    }

    pub(super) fn handle_tab_drag_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: rio_backend::event::WindowId,
        event: &WindowEvent,
    ) -> bool {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            if let WindowEvent::TabDragFinished {
                destination,
                cancelled,
            } = event
            {
                if self.tab_drag.native.source() == Some(window_id) {
                    let destination = destination
                        .and_then(|(id, point)| self.tab_destination(id.into(), point));
                    if *cancelled
                        || !self.transfer_tab(event_loop, window_id, destination, None)
                    {
                        self.cancel_tab_transfer(window_id);
                    }
                }
                return true;
            }
            if let Some(source_id) = self.active_tab_drag_source() {
                if matches!(event, WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed
                    && event.logical_key == rio_window::keyboard::Key::Named(rio_window::keyboard::NamedKey::Escape))
                {
                    self.cancel_tab_transfer(source_id);
                    return true;
                }
                if self.tab_drag.native.source().is_some()
                    && matches!(
                        event,
                        WindowEvent::CursorMoved { .. } | WindowEvent::MouseInput { .. }
                    )
                {
                    self.poll_desktop_tab_drag(event_loop);
                    return true;
                }
            }
        }

        #[cfg(windows)]
        if let Some(source_id) = self.active_tab_drag_source() {
            if matches!(event, WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.logical_key == rio_window::keyboard::Key::Named(rio_window::keyboard::NamedKey::Escape))
            {
                self.cancel_tab_transfer(source_id);
                return true;
            }
            if matches!(
                event,
                WindowEvent::CursorMoved { .. } | WindowEvent::MouseInput { .. }
            ) {
                let released = matches!(
                    event,
                    WindowEvent::MouseInput {
                        state: ElementState::Released,
                        button: MouseButton::Left,
                        ..
                    }
                );
                self.poll_tab_transfer(event_loop, released);
                return true;
            }
        }

        #[cfg(windows)]
        if matches!(
            event,
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } | WindowEvent::CloseRequested
        ) || matches!(event, WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed
                && event.logical_key == rio_window::keyboard::Key::Named(rio_window::keyboard::NamedKey::Escape))
        {
            // Remove the click-through image before choosing the drop target.
            self.tab_drag.preview = None;
        }

        if let WindowEvent::CursorMoved { position, .. } = event {
            self.tab_drag.position = Some((window_id, *position));
        }
        if matches!(
            event,
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            }
        ) && self.finish_tab_drag(event_loop, window_id)
        {
            return true;
        }

        let route = match self.router.routes.get_mut(&window_id) {
            Some(window) => window,
            None => return false,
        };

        if matches!(event, WindowEvent::KeyboardInput { event, .. }
            if event.state == ElementState::Pressed
                && event.logical_key == rio_window::keyboard::Key::Named(rio_window::keyboard::NamedKey::Escape))
        {
            if let Some(island) = &mut route.window.screen.renderer.island {
                if island.is_dragging() {
                    island.cancel_drag();
                    route.window.screen.mouse.left_button_state = ElementState::Released;
                    self.tab_drag.position = None;
                    route.request_redraw();
                    return true;
                }
            }
        }

        false
    }
    pub(super) fn poll_tab_drag(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(windows)]
        self.poll_tab_transfer(event_loop, false);
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        self.poll_desktop_tab_drag(event_loop);
        #[cfg(windows)]
        if !self
            .tab_drag
            .position
            .and_then(|(id, _)| self.router.routes.get(&id))
            .is_some_and(|route| {
                route.window.screen.tab_transfer_active()
                    && route.window.screen.mouse.left_button_state
                        == ElementState::Pressed
            })
        {
            self.tab_drag.preview = None;
        }
    }
    pub(super) fn tab_drag_control_flow(&self, control_flow: ControlFlow) -> ControlFlow {
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
        let control_flow = if self.tab_drag.native.desktop().is_some() {
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
        control_flow
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
