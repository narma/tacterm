//! Compositor-mediated, in-process DnD. The compositor owns the input grab and
//! image, including during window switching; destinations use surface coordinates.
use super::{state::WinitState, WindowId};
use crate::{
    dpi::{LogicalPosition, PhysicalPosition},
    event::WindowEvent,
};
use sctk::{
    reexports::client::{
        globals::GlobalList,
        protocol::{
            wl_data_device::{self, WlDataDevice},
            wl_data_device_manager::{DndAction, WlDataDeviceManager},
            wl_data_offer::{self, WlDataOffer},
            wl_data_source::{self, WlDataSource},
            wl_shm::Format,
            wl_surface::WlSurface,
        },
        Connection, Dispatch, Proxy, QueueHandle,
    },
    seat::SeatState,
    shell::WaylandSurface,
    shm::slot::{Buffer, SlotPool},
};
use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

pub struct DragState {
    manager: Option<WlDataDeviceManager>,
    devices: HashMap<sctk::reexports::client::backend::ObjectId, WlDataDevice>,
    active: Option<Active>,
    offers:
        HashMap<sctk::reexports::client::backend::ObjectId, (WlDataOffer, Vec<String>)>,
    queue: QueueHandle<WinitState>,
}
struct Active {
    source_id: WindowId,
    source: WlDataSource,
    icon: WlSurface,
    _buffer: Buffer,
    _pool: SlotPool,
    mime: String,
    target: Option<(WindowId, PhysicalPosition<f64>)>,
    offer: Option<WlDataOffer>,
    delivered: bool,
    dropped: bool,
}
impl Drop for Active {
    fn drop(&mut self) {
        self.source.destroy();
        self.icon.destroy();
    }
}
impl DragState {
    pub fn new(
        globals: &GlobalList,
        queue: &QueueHandle<WinitState>,
        seats: &SeatState,
    ) -> Self {
        let manager: Option<WlDataDeviceManager> = globals.bind(queue, 3..=3, ()).ok();
        let mut devices = HashMap::new();
        if let Some(manager) = &manager {
            for seat in seats.seats() {
                devices.insert(seat.id(), manager.get_data_device(&seat, queue, ()));
            }
        }
        Self {
            manager,
            devices,
            active: None,
            offers: HashMap::new(),
            queue: queue.clone(),
        }
    }
}
impl WinitState {
    pub fn start_pending_tab_drags(&mut self) {
        let cancelled = self.window_requests.borrow().iter().any(|(id, requests)| {
            requests.cancel_tab_drag.swap(false, Ordering::Relaxed)
                && self
                    .tab_drag
                    .active
                    .as_ref()
                    .is_some_and(|active| active.source_id == *id)
        });
        if cancelled {
            // The application already cancelled its gesture. Do not enqueue a
            // second completion which could cancel a subsequent drag.
            self.tab_drag.active = None;
            for (_, (offer, _)) in self.tab_drag.offers.drain() {
                offer.destroy();
            }
        }
        let requests: Vec<_> = self
            .window_requests
            .borrow()
            .iter()
            .filter_map(|(id, requests)| {
                requests
                    .tab_drag
                    .lock()
                    .unwrap()
                    .take()
                    .map(|image| (*id, image))
            })
            .collect();
        for (id, image) in requests {
            if let Err(error) = self.start_tab_drag(id, image) {
                tracing::warn!("Could not start Wayland tab drag: {error}");
                self.events_sink.push_window_event(
                    WindowEvent::TabDragFinished {
                        destination: None,
                        cancelled: true,
                    },
                    id,
                );
            }
        }
    }
    fn start_tab_drag(
        &mut self,
        id: WindowId,
        image: crate::window::TabDragImage,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if self.tab_drag.active.is_some() {
            return Err("a tab drag is already active".into());
        }
        let manager = self
            .tab_drag
            .manager
            .as_ref()
            .ok_or("compositor has no data device manager v3")?;
        let windows = self.windows.borrow();
        let window = windows
            .get(&id)
            .ok_or("source window closed")?
            .lock()
            .unwrap();
        let seat_and_serial = std::cell::RefCell::new(None);
        window.apply_on_pointer(|_, data| {
            *seat_and_serial.borrow_mut() =
                Some((data.seat().clone(), data.latest_button_serial()));
        });
        let (seat, serial) = seat_and_serial
            .into_inner()
            .ok_or("no active pointer press")?;
        if serial == 0 {
            return Err("missing pointer press serial".into());
        }
        let queue = &self.tab_drag.queue;
        let device = self
            .tab_drag
            .devices
            .entry(seat.id())
            .or_insert_with(|| manager.get_data_device(&seat, queue, ()));
        let mut pool = SlotPool::new(image.rgba.len(), &self.shm)?;
        let (buffer, canvas) = pool.create_buffer(
            image.width as i32,
            image.height as i32,
            image.width as i32 * 4,
            Format::Argb8888,
        )?;
        for (out, rgba) in canvas.chunks_exact_mut(4).zip(image.rgba.chunks_exact(4)) {
            let a = rgba[3] as u32;
            out.copy_from_slice(
                &((a << 24)
                    | ((rgba[0] as u32 * a / 255) << 16)
                    | ((rgba[1] as u32 * a / 255) << 8)
                    | (rgba[2] as u32 * a / 255))
                    .to_ne_bytes(),
            );
        }
        let source = manager.create_data_source(queue, ());
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let mime = format!(
            "application/x-tacterm-tab-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        source.offer(mime.clone());
        source.set_actions(DndAction::Move);
        let icon = self.compositor_state.create_surface(queue);
        icon.set_buffer_scale(image.scale);
        device.start_drag(
            Some(&source),
            window.window.wl_surface(),
            Some(&icon),
            serial,
        );
        // Initial icon displacement is in surface coordinates, not global pixels.
        let (x, y) = (
            -image.grab_x / image.scale,
            -(image.height as i32 / 2) / image.scale,
        );
        if icon.version() >= 5 {
            icon.offset(x, y);
            icon.attach(Some(buffer.wl_buffer()), 0, 0);
        } else {
            icon.attach(Some(buffer.wl_buffer()), x, y);
        }
        icon.damage(0, 0, image.width as i32, image.height as i32);
        icon.commit();
        self.tab_drag.active = Some(Active {
            source_id: id,
            source,
            icon,
            _buffer: buffer,
            _pool: pool,
            mime,
            target: None,
            offer: None,
            delivered: false,
            dropped: false,
        });
        Ok(())
    }
    fn finish_native_tab_drag(&mut self, cancelled: bool) {
        if let Some(active) = self.tab_drag.active.take() {
            if !active.delivered {
                self.events_sink.push_window_event(
                    WindowEvent::TabDragFinished {
                        destination: active
                            .target
                            .map(|(id, p)| (crate::window::WindowId(id), p)),
                        cancelled,
                    },
                    active.source_id,
                );
            }
        }
        for (_, (offer, _)) in self.tab_drag.offers.drain() {
            offer.destroy();
        }
    }
}
impl Dispatch<WlDataDeviceManager, ()> for WinitState {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<WlDataDevice, ()> for WinitState {
    sctk::reexports::client::event_created_child!(WinitState, WlDataDevice, [0 => (WlDataOffer, ())]);
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_device::Event::DataOffer { id } => {
                state.tab_drag.offers.insert(id.id(), (id, Vec::new()));
            }
            wl_data_device::Event::Enter {
                serial,
                surface,
                x,
                y,
                id,
            } => {
                let wid = super::make_wid(&surface);
                let Some(active) = state.tab_drag.active.as_mut() else {
                    if let Some(offer) = id {
                        offer.accept(serial, None);
                    }
                    return;
                };
                let Some(offer) = id else {
                    return;
                };
                let valid = state
                    .tab_drag
                    .offers
                    .get(&offer.id())
                    .is_some_and(|(_, types)| types.contains(&active.mime));
                let windows = state.windows.borrow();
                if valid {
                    if let Some(window) = windows.get(&wid) {
                        let scale = window.lock().unwrap().scale_factor();
                        active.target =
                            Some((wid, LogicalPosition::new(x, y).to_physical(scale)));
                        offer.accept(serial, Some(active.mime.clone()));
                        offer.set_actions(DndAction::Move, DndAction::Move);
                        active.offer = Some(offer);
                        return;
                    }
                }
                offer.accept(serial, None);
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                if let Some(active) = &mut state.tab_drag.active {
                    if let Some((id, point)) = &mut active.target {
                        if let Some(window) = state.windows.borrow().get(id) {
                            *point = LogicalPosition::new(x, y)
                                .to_physical(window.lock().unwrap().scale_factor());
                        }
                    }
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(active) = &mut state.tab_drag.active {
                    active.target = None;
                    active.offer = None;
                }
                for (_, (offer, _)) in state.tab_drag.offers.drain() {
                    offer.destroy();
                }
            }
            wl_data_device::Event::Drop => {
                if let Some(active) = &mut state.tab_drag.active {
                    if let Some(offer) = &active.offer {
                        offer.finish();
                    }
                    state.events_sink.push_window_event(
                        WindowEvent::TabDragFinished {
                            destination: active
                                .target
                                .map(|(id, p)| (crate::window::WindowId(id), p)),
                            cancelled: false,
                        },
                        active.source_id,
                    );
                    active.delivered = true;
                }
            }
            wl_data_device::Event::Selection { id: Some(offer) } => {
                offer.destroy();
                state.tab_drag.offers.remove(&offer.id());
            }
            _ => {}
        }
    }
}
impl Dispatch<WlDataOffer, ()> for WinitState {
    fn event(
        state: &mut Self,
        offer: &WlDataOffer,
        event: wl_data_offer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event {
            state
                .tab_drag
                .offers
                .entry(offer.id())
                .or_insert_with(|| (offer.clone(), Vec::new()))
                .1
                .push(mime_type);
        }
    }
}
impl Dispatch<WlDataSource, ()> for WinitState {
    fn event(
        state: &mut Self,
        source: &WlDataSource,
        event: wl_data_source::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if !state
            .tab_drag
            .active
            .as_ref()
            .is_some_and(|a| &a.source == source)
        {
            return;
        }
        match event {
            wl_data_source::Event::DndDropPerformed => {
                if let Some(active) = &mut state.tab_drag.active {
                    active.dropped = true;
                }
            }
            wl_data_source::Event::DndFinished => state.finish_native_tab_drag(false),
            wl_data_source::Event::Cancelled => {
                // The protocol conflates Escape and an unaccepted desktop drop.
                // Never detach on an ambiguous cancellation (and risk moving a
                // session after Escape). Dropping on a terminal body detaches.
                let cancelled =
                    !state.tab_drag.active.as_ref().is_some_and(|a| a.dropped);
                state.finish_native_tab_drag(cancelled);
            }
            wl_data_source::Event::Send { fd, .. } => drop(fd), // no terminal data leaves this process
            _ => {}
        }
    }
}
