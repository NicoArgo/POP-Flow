// Live window-thumbnail capture backend for the alt-tab switcher.
//
// Runs a dedicated wayland connection on its own thread. It enumerates open
// toplevels via `ext-foreign-toplevel-list` / cosmic toplevel-info, and — while
// the switcher is active — captures a live image of each window through the
// `ext-image-copy-capture` (screencopy) protocol, streaming the results back to
// the iced app as a `Subscription`.
//
// The heavy lifting (protocol handlers, buffer allocation) mirrors the cctk
// `screenshot-screencopy` example and cosmic-workspaces, adapted to a
// toplevel `CaptureSource` and an `image::Handle` output.

use std::collections::HashMap;
use std::sync::Mutex;
use std::thread;

use cosmic::cctk::screencopy::{
    CaptureFrame, CaptureOptions, CaptureSession, CaptureSource, FailureReason, Formats,
    ScreencopyFrameData, ScreencopyFrameDataExt, ScreencopyHandler, ScreencopySessionData,
    ScreencopySessionDataExt, ScreencopyState,
};
use cosmic::cctk::sctk::{
    self,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    shm::{raw::RawPool, Shm, ShmHandler},
};
use cosmic::cctk::toplevel_info::{ToplevelInfoHandler, ToplevelInfoState};
use cosmic::cctk::wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_buffer, wl_output, wl_shm},
    Connection, QueueHandle, WEnum,
};
use cosmic::cctk::wayland_protocols::ext::foreign_toplevel_list::v1::client::ext_foreign_toplevel_handle_v1::ExtForeignToplevelHandleV1;
use cosmic::iced::futures::executor::block_on;
use cosmic::iced::futures::channel::mpsc;
use cosmic::iced::futures::{SinkExt, StreamExt};
use cosmic::widget::image;

/// Events streamed from the capture thread up to the iced app.
#[derive(Clone)]
pub enum Event {
    /// Handshake: gives the app a channel to send commands back to the thread.
    Ready(calloop::channel::Sender<Cmd>),
    /// A fresh thumbnail for the toplevel identified by `identifier`.
    Thumbnail {
        identifier: String,
        title: String,
        app_id: String,
        image: image::Handle,
    },
    /// A toplevel went away.
    Closed(String),
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Event::Ready(_) => f.write_str("Event::Ready(..)"),
            Event::Thumbnail {
                identifier,
                title,
                app_id,
                ..
            } => f
                .debug_struct("Event::Thumbnail")
                .field("identifier", identifier)
                .field("title", title)
                .field("app_id", app_id)
                .finish_non_exhaustive(),
            Event::Closed(id) => f.debug_tuple("Event::Closed").field(id).finish(),
        }
    }
}

/// Commands sent from the app down to the capture thread.
#[derive(Debug)]
pub enum Cmd {
    /// Start/stop actively capturing thumbnails. Capturing is only worthwhile
    /// while the alt-tab switcher is on screen.
    SetActive(bool),
}

/// The iced subscription that owns the capture thread for the app's lifetime.
pub fn subscription() -> cosmic::iced::Subscription<Event> {
    cosmic::iced::Subscription::run_with_id(
        "cosmic-launcher-thumbnails",
        cosmic::iced_futures::stream::channel(20, |mut output| async move {
            let Ok(conn) = Connection::connect_to_env() else {
                tracing::warn!("thumbnail backend: no wayland connection");
                std::future::pending::<()>().await;
                unreachable!();
            };
            let mut receiver = start(conn);
            while let Some(event) = receiver.next().await {
                let _ = output.send(event).await;
            }
        }),
    )
}

struct AppData {
    qh: QueueHandle<Self>,
    registry_state: RegistryState,
    output_state: OutputState,
    shm_state: Shm,
    screencopy_state: ScreencopyState,
    toplevel_info_state: ToplevelInfoState,
    sender: mpsc::Sender<Event>,
    active: bool,
    /// identifier -> live capture session (kept alive so it isn't dropped).
    sessions: HashMap<String, CaptureSession>,
}

impl AppData {
    fn send_event(&mut self, event: Event) {
        let _ = block_on(self.sender.send(event));
    }

    fn handle_cmd(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::SetActive(active) => {
                if self.active == active {
                    return;
                }
                self.active = active;
                if active {
                    // Capture every currently-open toplevel. Collect first to
                    // release the borrow on `toplevel_info_state`.
                    let targets: Vec<(ExtForeignToplevelHandleV1, String, String, String)> = self
                        .toplevel_info_state
                        .toplevels()
                        .map(|info| {
                            (
                                info.foreign_toplevel.clone(),
                                info.identifier.clone(),
                                info.title.clone(),
                                info.app_id.clone(),
                            )
                        })
                        .collect();
                    for (handle, identifier, title, app_id) in targets {
                        self.start_capture(handle, identifier, title, app_id);
                    }
                } else {
                    // Dropping the sessions tears down the screencopy sessions.
                    self.sessions.clear();
                }
            }
        }
    }

    fn start_capture(
        &mut self,
        handle: ExtForeignToplevelHandleV1,
        identifier: String,
        title: String,
        app_id: String,
    ) {
        if self.sessions.contains_key(&identifier) {
            return;
        }
        let res = self.screencopy_state.capturer().create_session(
            &CaptureSource::Toplevel(handle),
            CaptureOptions::empty(),
            &self.qh,
            SessionData {
                session_data: ScreencopySessionData::default(),
                identifier: identifier.clone(),
                title,
                app_id,
            },
        );
        match res {
            Ok(session) => {
                self.sessions.insert(identifier, session);
            }
            Err(err) => {
                tracing::warn!("failed to create capture session: {err}");
            }
        }
    }
}

impl ScreencopyHandler for AppData {
    fn screencopy_state(&mut self) -> &mut ScreencopyState {
        &mut self.screencopy_state
    }

    fn init_done(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        session: &CaptureSession,
        formats: &Formats,
    ) {
        let (width, height) = formats.buffer_size;
        if width == 0 || height == 0 {
            return;
        }
        let Some(data) = session.data::<SessionData>() else {
            return;
        };
        let identifier = data.identifier.clone();
        let title = data.title.clone();
        let app_id = data.app_id.clone();

        let mut pool =
            match RawPool::new(width as usize * height as usize * 4, &self.shm_state) {
                Ok(pool) => pool,
                Err(err) => {
                    tracing::warn!("failed to allocate shm pool: {err}");
                    return;
                }
            };
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            width as i32 * 4,
            wl_shm::Format::Abgr8888,
            (),
            qh,
        );
        session.capture(
            &buffer,
            &[],
            qh,
            FrameData {
                frame_data: ScreencopyFrameData::default(),
                identifier,
                title,
                app_id,
                pool: Mutex::new(pool),
                size: (width, height),
            },
        );
    }

    fn ready(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        capture_frame: &CaptureFrame,
        _frame: cosmic::cctk::screencopy::Frame,
    ) {
        let Some(data) = capture_frame.data::<FrameData>() else {
            return;
        };
        let (width, height) = data.size;
        let mut pixels = {
            let mut pool = data.pool.lock().unwrap();
            pool.mmap().to_vec()
        };
        round_corners(&mut pixels, width, height);
        let handle = image::Handle::from_rgba(width, height, pixels);
        self.send_event(Event::Thumbnail {
            identifier: data.identifier.clone(),
            title: data.title.clone(),
            app_id: data.app_id.clone(),
            image: handle,
        });
    }

    fn stopped(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _session: &CaptureSession) {}

    fn failed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        capture_frame: &CaptureFrame,
        reason: WEnum<FailureReason>,
    ) {
        if let Some(data) = capture_frame.data::<FrameData>() {
            tracing::warn!("thumbnail capture failed for {}: {:?}", data.title, reason);
        }
    }
}

impl ToplevelInfoHandler for AppData {
    fn toplevel_info_state(&mut self) -> &mut ToplevelInfoState {
        &mut self.toplevel_info_state
    }

    fn new_toplevel(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        toplevel: &ExtForeignToplevelHandleV1,
    ) {
        if !self.active {
            return;
        }
        let Some(info) = self.toplevel_info_state.info(toplevel) else {
            return;
        };
        let handle = info.foreign_toplevel.clone();
        let identifier = info.identifier.clone();
        let title = info.title.clone();
        let app_id = info.app_id.clone();
        self.start_capture(handle, identifier, title, app_id);
    }

    fn update_toplevel(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _toplevel: &ExtForeignToplevelHandleV1,
    ) {
    }

    fn toplevel_closed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        toplevel: &ExtForeignToplevelHandleV1,
    ) {
        let Some(identifier) = self
            .toplevel_info_state
            .info(toplevel)
            .map(|info| info.identifier.clone())
        else {
            return;
        };
        self.sessions.remove(&identifier);
        self.send_event(Event::Closed(identifier));
    }
}

impl ShmHandler for AppData {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm_state
    }
}

impl OutputHandler for AppData {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ProvidesRegistryState for AppData {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    sctk::registry_handlers!(OutputState);
}

/// Session user-data: carries which toplevel this session is for.
struct SessionData {
    session_data: ScreencopySessionData,
    identifier: String,
    title: String,
    app_id: String,
}

impl ScreencopySessionDataExt for SessionData {
    fn screencopy_session_data(&self) -> &ScreencopySessionData {
        &self.session_data
    }
}

/// Frame user-data: carries the buffer + which toplevel it belongs to.
struct FrameData {
    frame_data: ScreencopyFrameData,
    identifier: String,
    title: String,
    app_id: String,
    pool: Mutex<RawPool>,
    size: (u32, u32),
}

impl ScreencopyFrameDataExt for FrameData {
    fn screencopy_frame_data(&self) -> &ScreencopyFrameData {
        &self.frame_data
    }
}

fn start(conn: Connection) -> mpsc::Receiver<Event> {
    let (sender, receiver) = mpsc::channel(20);

    let (globals, event_queue) = registry_queue_init(&conn).unwrap();
    let qh = event_queue.handle();

    thread::spawn(move || {
        let registry_state = RegistryState::new(&globals);
        let mut app_data = AppData {
            qh: qh.clone(),
            output_state: OutputState::new(&globals, &qh),
            shm_state: Shm::bind(&globals, &qh).unwrap(),
            screencopy_state: ScreencopyState::new(&globals, &qh),
            toplevel_info_state: ToplevelInfoState::new(&registry_state, &qh),
            registry_state,
            sender,
            active: false,
            sessions: HashMap::new(),
        };

        let (cmd_sender, cmd_channel) = calloop::channel::channel();
        app_data.send_event(Event::Ready(cmd_sender));

        let mut event_loop = calloop::EventLoop::try_new().unwrap();
        calloop_wayland_source::WaylandSource::new(conn, event_queue)
            .insert(event_loop.handle())
            .unwrap();
        event_loop
            .handle()
            .insert_source(cmd_channel, |event, _, app_data| {
                if let calloop::channel::Event::Msg(msg) = event {
                    app_data.handle_cmd(msg);
                }
            })
            .unwrap();

        loop {
            if event_loop.dispatch(None, &mut app_data).is_err() {
                break;
            }
        }
    });

    receiver
}

/// Apply rounded corners to an RGBA image in place by zeroing the alpha of
/// pixels outside a rounded rectangle (with 1px anti-aliasing). The radius is
/// proportional to the image size so it looks consistent once scaled down.
fn round_corners(pixels: &mut [u8], w: u32, h: u32) {
    let wi = w as i64;
    let hi = h as i64;
    let r = ((w.min(h) as f32) * 0.05).round() as i64;
    if r < 2 || pixels.len() < (wi * hi * 4) as usize {
        return;
    }
    let rf = r as f32;
    // (box origin x, box origin y, arc center x, arc center y)
    let corners = [
        (0i64, 0i64, rf, rf),
        (wi - r, 0, (wi - r) as f32, rf),
        (0, hi - r, rf, (hi - r) as f32),
        (wi - r, hi - r, (wi - r) as f32, (hi - r) as f32),
    ];
    for (bx, by, cx, cy) in corners {
        for yy in by..(by + r) {
            for xx in bx..(bx + r) {
                if xx < 0 || yy < 0 || xx >= wi || yy >= hi {
                    continue;
                }
                let dx = xx as f32 - cx;
                let dy = yy as f32 - cy;
                let dist = (dx * dx + dy * dy).sqrt();
                let factor = if dist <= rf - 1.0 {
                    1.0
                } else if dist >= rf {
                    0.0
                } else {
                    rf - dist
                };
                if factor < 1.0 {
                    let idx = ((yy * wi + xx) * 4 + 3) as usize;
                    pixels[idx] = (pixels[idx] as f32 * factor) as u8;
                }
            }
        }
    }
}

sctk::delegate_registry!(AppData);
sctk::delegate_shm!(AppData);
sctk::delegate_output!(AppData);
cosmic::cctk::delegate_screencopy!(AppData);
cosmic::cctk::delegate_toplevel_info!(AppData);
cosmic::cctk::wayland_client::delegate_noop!(AppData: ignore wl_buffer::WlBuffer);
