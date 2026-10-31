//! The Daemon's Wayland side: Monitors, background layer surfaces and frame
//! presentation, on waybackend (ADR 0004).
//!
//! Nothing here keeps pixels. A presented frame's memfd becomes a `wl_shm`
//! pool, the buffer is attached and committed, and the fd is closed right
//! after the flush. The buffer itself is destroyed on `wl_buffer.release`.
//!
//! Frames are physical-size buffers: logical size × the fractional scale from
//! fractional-scale-v1, shown at logical size through a viewporter viewport.
//! On a scale or size change the viewport destination follows the new
//! logical size, so the old frame stays on screen, stretched, until a frame
//! for the new size is presented. Without those protocols, the integer
//! `wl_output.scale` and `set_buffer_scale` are the fallback.
//!
//! Transitions live in [`transition`].

mod transition;

use std::os::fd::{AsFd, OwnedFd};
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use vellum_ipc::{MonitorInfo, PixelFormat, Present};
use waybackend::objman::ObjectManager;
use waybackend::types::ObjectId;
use waybackend::wire::{Error as WireError, Receiver};
use waybackend::{WL_DISPLAY, Waybackend};

use crate::protocols::*;

/// What each Wayland object id refers to.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Obj {
    Display,
    Registry,
    Callback,
    Compositor,
    Shm,
    ShmPool,
    Buffer,
    Surface,
    Output,
    LayerShell,
    LayerSurface,
    Viewporter,
    Viewport,
    FractionalScaleManager,
    FractionalScale,
    Subcompositor,
    Subsurface,
    AlphaModifier,
    AlphaModifierSurface,
    /// A `wl_surface.frame` callback pacing a Transition.
    FrameCallback,
}

/// Whether a test asked to skip alpha-modifier, forcing the CPU Fade, with
/// `VELLUMD_NO_ALPHA_MODIFIER`. Debug builds only: the e2e tests run the
/// debug Daemon, and a release Daemon never honors a test switch.
#[cfg(debug_assertions)]
fn alpha_modifier_disabled_for_tests() -> bool {
    std::env::var_os("VELLUMD_NO_ALPHA_MODIFIER").is_some()
}

#[cfg(not(debug_assertions))]
fn alpha_modifier_disabled_for_tests() -> bool {
    false
}

/// How long startup waits for every Monitor's first configure.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(3);

/// The connection to the compositor.
pub struct Wayland {
    receiver: Receiver,
    state: State,
}

struct State {
    backend: Waybackend,
    objects: ObjectManager<Obj>,
    registry: ObjectId,
    initial_sync: Option<ObjectId>,
    compositor: Option<ObjectId>,
    shm: Option<ObjectId>,
    layer_shell: Option<ObjectId>,
    viewporter: Option<ObjectId>,
    fractional_scale_manager: Option<ObjectId>,
    subcompositor: Option<ObjectId>,
    alpha_modifier: Option<ObjectId>,
    /// Buffers destroyed by us whose ids the compositor has not yet freed;
    /// a late `release` for one of them is ignored.
    dead_buffers: Vec<ObjectId>,
    monitors: Vec<Monitor>,
    fatal: Option<String>,
}

struct Monitor {
    /// The registry name of its `wl_output` global.
    global: u32,
    output: ObjectId,
    output_version: u32,
    /// Connector name from `wl_output.name`.
    name: String,
    /// Integer scale from `wl_output.scale`, the fallback.
    scale: i32,
    /// The surface's preferred scale in 120ths, from fractional-scale-v1.
    preferred_scale: Option<u32>,
    /// Seen `wl_output.done` at least once.
    described: bool,
    surface: Option<ObjectId>,
    layer: Option<ObjectId>,
    /// Present only when the compositor has viewporter.
    viewport: Option<ObjectId>,
    /// Present only when the compositor has both viewporter and
    /// fractional-scale-v1.
    fractional_scale: Option<ObjectId>,
    /// A frame is attached, so configure changes must be committed.
    has_frame: bool,
    /// Logical size from the last layer-surface configure.
    size: Option<(u32, u32)>,
    /// Physical size of the frame last presented.
    frame_size: Option<(u32, u32)>,
    /// Physical size a frame was last asked for by [`Wayland::take_frame_needs`].
    frame_asked: Option<(u32, u32)>,
    animation: Option<transition::Animation>,
}

impl Monitor {
    /// The scale in 120ths: the fractional preferred scale, or the integer
    /// output scale until (or unless) one arrives.
    fn scale_120(&self) -> u32 {
        match (self.fractional_scale, self.preferred_scale) {
            (Some(_), Some(preferred)) => preferred,
            _ => self.scale.max(1) as u32 * 120,
        }
    }

    /// Configured, and its fractional scale known if one is coming. Startup
    /// waits for this; a compositor that never sends a preferred scale
    /// gets the integer fallback after the startup timeout.
    fn is_settled(&self) -> bool {
        self.size.is_some() && (self.fractional_scale.is_none() || self.preferred_scale.is_some())
    }

    /// The exact size a frame must have: logical size × scale, rounded half
    /// away from zero as fractional-scale-v1 asks.
    fn physical_size(&self) -> Option<(u32, u32)> {
        let (w, h) = self.size?;
        let scale = self.scale_120();
        let scaled = |logical: u32| ((u64::from(logical) * u64::from(scale) + 60) / 120) as u32;
        Some((scaled(w), scaled(h)))
    }
}

impl Wayland {
    /// Connects to the compositor named by the environment and waits until
    /// every Monitor has a configured background surface.
    pub fn connect() -> Result<Wayland, String> {
        let (backend, mut objects, receiver) = waybackend::connect(Obj::Display)
            .map_err(|err| format!("cannot connect to the Wayland compositor: {err:?}"))?;
        let registry = objects.create(Obj::Registry);
        let callback = objects.create(Obj::Callback);
        let mut wayland = Wayland {
            receiver,
            state: State {
                backend,
                objects,
                registry,
                initial_sync: Some(callback),
                compositor: None,
                shm: None,
                layer_shell: None,
                viewporter: None,
                fractional_scale_manager: None,
                subcompositor: None,
                alpha_modifier: None,
                dead_buffers: Vec::new(),
                monitors: Vec::new(),
                fatal: None,
            },
        };
        let s = &mut wayland.state;
        wl_display::req::get_registry(&mut s.backend, WL_DISPLAY, registry).map_err(io)?;
        wl_display::req::sync(&mut s.backend, WL_DISPLAY, callback).map_err(io)?;
        s.backend.flush().map_err(io)?;

        let deadline = Instant::now() + STARTUP_TIMEOUT;
        while wayland.state.initial_sync.is_some() {
            wayland.dispatch()?;
        }
        if wayland.state.compositor.is_none() || wayland.state.shm.is_none() {
            return Err("the compositor lacks wl_compositor or wl_shm".into());
        }
        if wayland.state.layer_shell.is_none() {
            return Err("the compositor does not support wlr-layer-shell; \
                 Vellum needs a layer-shell compositor such as Hyprland, niri or Sway"
                .into());
        }
        wayland.flush()?;
        while !wayland.all_configured() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                eprintln!("vellumd: some monitors did not configure in time; continuing");
                break;
            }
            if wayland.wait_readable(left)? {
                wayland.dispatch()?;
            }
        }
        Ok(wayland)
    }

    fn all_configured(&self) -> bool {
        self.state.monitors.iter().all(Monitor::is_settled)
    }

    fn wait_readable(&self, timeout: Duration) -> Result<bool, String> {
        let mut fds = [PollFd::new(&self.state.backend.wayland_fd, PollFlags::IN)];
        let timeout = Timespec::try_from(timeout).ok();
        match poll(&mut fds, timeout.as_ref()) {
            Ok(n) => Ok(n > 0),
            Err(rustix::io::Errno::INTR) => Ok(false),
            Err(err) => Err(io(err)),
        }
    }

    /// The socket to poll for compositor events.
    pub fn fd(&self) -> impl AsFd + '_ {
        &self.state.backend.wayland_fd
    }

    /// Reads and handles whatever the compositor sent. Call when [`fd`]
    /// is readable. An error means the connection is unusable.
    ///
    /// [`fd`]: Wayland::fd
    pub fn dispatch(&mut self) -> Result<(), String> {
        let state = &mut self.state;
        let mut msgs = self
            .receiver
            .recv(&state.backend.wayland_fd)
            .map_err(|err| format!("lost the compositor connection: {err:?}"))?;
        let mut received_any = false;
        while let Some(sender) = msgs.next() {
            received_any = true;
            let sender = sender.map_err(|err| format!("{err:?}"))?;
            let Some(obj) = state.objects.get(sender) else {
                return Err(format!("event for unknown Wayland object {sender}"));
            };
            // Every event must be decoded, even ignored ones: decoding is
            // what advances the read cursor to the next message.
            let decoded = match obj {
                Obj::Display => wl_display::event(state, &mut msgs),
                Obj::Registry => wl_registry::event(state, &mut msgs),
                Obj::Callback => wl_callback::event(state, &mut msgs),
                Obj::Compositor => wl_compositor::event(state, &mut msgs),
                Obj::Shm => wl_shm::event(state, &mut msgs),
                Obj::ShmPool => wl_shm_pool::event(state, &mut msgs),
                Obj::Buffer => wl_buffer::event(state, &mut msgs),
                Obj::Surface => wl_surface::event(state, &mut msgs),
                Obj::Output => wl_output::event(state, &mut msgs),
                Obj::LayerShell => zwlr_layer_shell_v1::event(state, &mut msgs),
                Obj::LayerSurface => zwlr_layer_surface_v1::event(state, &mut msgs),
                Obj::Viewporter => wp_viewporter::event(state, &mut msgs),
                Obj::Viewport => wp_viewport::event(state, &mut msgs),
                Obj::FractionalScaleManager => {
                    wp_fractional_scale_manager_v1::event(state, &mut msgs)
                }
                Obj::FractionalScale => wp_fractional_scale_v1::event(state, &mut msgs),
                Obj::Subcompositor => wl_subcompositor::event(state, &mut msgs),
                Obj::Subsurface => wl_subsurface::event(state, &mut msgs),
                Obj::AlphaModifier => wp_alpha_modifier_v1::event(state, &mut msgs),
                Obj::AlphaModifierSurface => wp_alpha_modifier_surface_v1::event(state, &mut msgs),
                Obj::FrameCallback => wl_callback::event(state, &mut msgs),
            };
            decoded.map_err(|err| format!("malformed Wayland event: {err:?}"))?;
        }
        drop(msgs);
        if !received_any {
            return Err("the compositor closed the connection".into());
        }
        if let Some(fatal) = state.fatal.take() {
            return Err(fatal);
        }
        state.create_missing_surfaces().map_err(io)?;
        self.flush()
    }

    /// Sends queued requests to the compositor.
    pub fn flush(&mut self) -> Result<(), String> {
        self.state.backend.flush().map_err(io)
    }

    /// Every configured Monitor, sorted by name.
    pub fn monitors(&self) -> Vec<MonitorInfo> {
        let mut monitors: Vec<MonitorInfo> = self
            .state
            .monitors
            .iter()
            .filter_map(|m| {
                let (logical_width, logical_height) = m.size?;
                let (physical_width, physical_height) = m.physical_size()?;
                Some(MonitorInfo {
                    name: m.name.clone(),
                    logical_width,
                    logical_height,
                    physical_width,
                    physical_height,
                    scale_120: m.scale_120(),
                })
            })
            .collect();
        monitors.sort_by(|a, b| a.name.cmp(&b.name));
        monitors
    }

    /// Names the settled Monitors whose frame is missing or the wrong size
    /// for them now. Each Monitor is named once per physical size.
    pub fn take_frame_needs(&mut self) -> Vec<String> {
        let mut needs = Vec::new();
        for m in &mut self.state.monitors {
            if !m.is_settled() {
                continue;
            }
            let Some(size) = m.physical_size() else {
                continue;
            };
            if m.frame_size != Some(size) && m.frame_asked != Some(size) {
                m.frame_asked = Some(size);
                needs.push(m.name.clone());
            }
        }
        needs
    }

    /// Whether `frame` fits its Monitor now, and the Monitor lacks a frame
    /// of that size.
    pub fn wants_frame(&self, frame: &Present) -> bool {
        self.state.monitors.iter().any(|m| {
            m.name == frame.monitor
                && m.physical_size() == Some((frame.width, frame.height))
                && m.frame_size != m.physical_size()
        })
    }

    /// Shows `frame` on its Monitor through its Transition. The memfd is
    /// consumed: it is closed as soon as the compositor has it.
    pub fn present(&mut self, frame: &Present, memfd: OwnedFd) -> Result<(), String> {
        let s = &mut self.state;
        let index = s
            .monitors
            .iter()
            .position(|m| m.name == frame.monitor)
            .ok_or_else(|| format!("monitor {} is not connected", frame.monitor))?;
        let monitor = &s.monitors[index];
        let (Some(_), Some((width, height))) = (monitor.surface, monitor.physical_size()) else {
            return Err(format!("monitor {} is not ready yet", frame.monitor));
        };
        if (frame.width, frame.height) != (width, height) {
            return Err(format!(
                "frame is {}x{} but monitor {} is {width}x{height}",
                frame.width, frame.height, frame.monitor
            ));
        }
        let PixelFormat::Xrgb8888 = frame.format;
        let min_stride = width * frame.format.bytes_per_pixel();
        if frame.stride < min_stride {
            return Err(format!("stride {} is below {min_stride}", frame.stride));
        }
        let pool_size = u64::from(frame.stride) * u64::from(height);
        let pool_size = i32::try_from(pool_size).map_err(|_| "frame is too large".to_string())?;
        let file_size = rustix::fs::fstat(&memfd)
            .map_err(|err| format!("cannot inspect the frame: {err}"))?
            .st_size;
        if file_size < i64::from(pool_size) {
            return Err(format!(
                "frame memory holds {file_size} bytes but {pool_size} are needed"
            ));
        }

        // A newer frame wins: the running Transition jumps to its end, so
        // the next one starts from a settled screen.
        if let Some(animation) = s.monitors[index].animation.take() {
            s.finish_transition(index, animation).map_err(io)?;
        }

        let shm = s.shm.expect("bound at startup");
        let pool = s.objects.create(Obj::ShmPool);
        wl_shm::req::create_pool(&mut s.backend, shm, pool, &memfd, pool_size).map_err(io)?;
        let buffer = s.objects.create(Obj::Buffer);
        wl_shm_pool::req::create_buffer(
            &mut s.backend,
            pool,
            buffer,
            0,
            width as i32,
            height as i32,
            frame.stride as i32,
            wl_shm::Format::xrgb8888,
        )
        .map_err(io)?;
        let new = transition::NewFrame {
            pool,
            buffer,
            memfd: &memfd,
            stride: frame.stride,
        };
        if let Some(new) = s
            .start_transition(index, frame.transition, new)
            .map_err(io)?
        {
            wl_shm_pool::req::destroy(&mut s.backend, new.pool).map_err(io)?;
            s.show(index, new.buffer).map_err(io)?;
        }
        // waybackend sends fds on flush, so the memfd must outlive it.
        s.backend.flush().map_err(io)?;
        drop(memfd);
        // On screen now, or once the Transition lands.
        s.monitors[index].frame_size = Some((width, height));
        Ok(())
    }
}

fn io(err: rustix::io::Errno) -> String {
    format!("Wayland I/O error: {err}")
}

impl State {
    /// Puts `buffer` on Monitor `index`'s background surface.
    fn show(&mut self, index: usize, buffer: ObjectId) -> Result<(), rustix::io::Errno> {
        let m = &mut self.monitors[index];
        let Some(surface) = m.surface else {
            return self.destroy_buffer(buffer);
        };
        m.has_frame = true;
        let buffer_scale = match m.viewport {
            Some(_) => 1,
            None => m.scale.max(1),
        };
        wl_surface::req::set_buffer_scale(&mut self.backend, surface, buffer_scale)?;
        wl_surface::req::attach(&mut self.backend, surface, Some(buffer), 0, 0)?;
        wl_surface::req::damage_buffer(&mut self.backend, surface, 0, 0, i32::MAX, i32::MAX)?;
        wl_surface::req::commit(&mut self.backend, surface)
    }

    fn destroy_buffer(&mut self, buffer: ObjectId) -> Result<(), rustix::io::Errno> {
        self.dead_buffers.push(buffer);
        wl_buffer::req::destroy(&mut self.backend, buffer)
    }

    fn create_missing_surfaces(&mut self) -> Result<(), rustix::io::Errno> {
        let (Some(compositor), Some(layer_shell)) = (self.compositor, self.layer_shell) else {
            return Ok(());
        };
        for m in &mut self.monitors {
            if m.surface.is_some() || !m.described {
                continue;
            }
            let surface = self.objects.create(Obj::Surface);
            wl_compositor::req::create_surface(&mut self.backend, compositor, surface)?;
            let layer = self.objects.create(Obj::LayerSurface);
            zwlr_layer_shell_v1::req::get_layer_surface(
                &mut self.backend,
                layer_shell,
                layer,
                surface,
                Some(m.output),
                zwlr_layer_shell_v1::Layer::background,
                "vellum",
            )?;
            use zwlr_layer_surface_v1::Anchor;
            let all = Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT;
            zwlr_layer_surface_v1::req::set_anchor(&mut self.backend, layer, all)?;
            zwlr_layer_surface_v1::req::set_exclusive_zone(&mut self.backend, layer, -1)?;
            if let Some(viewporter) = self.viewporter {
                let viewport = self.objects.create(Obj::Viewport);
                wp_viewporter::req::get_viewport(&mut self.backend, viewporter, viewport, surface)?;
                m.viewport = Some(viewport);
                if let Some(manager) = self.fractional_scale_manager {
                    let fractional = self.objects.create(Obj::FractionalScale);
                    wp_fractional_scale_manager_v1::req::get_fractional_scale(
                        &mut self.backend,
                        manager,
                        fractional,
                        surface,
                    )?;
                    m.fractional_scale = Some(fractional);
                }
            }
            wl_surface::req::commit(&mut self.backend, surface)?;
            m.surface = Some(surface);
            m.layer = Some(layer);
        }
        Ok(())
    }

    fn remove_monitor(&mut self, index: usize) {
        let mut m = self.monitors.remove(index);
        if let Some(animation) = m.animation.take() {
            let _ = self.drop_transition(animation);
        }
        if let Some(fractional) = m.fractional_scale {
            let _ = wp_fractional_scale_v1::req::destroy(&mut self.backend, fractional);
        }
        if let Some(viewport) = m.viewport {
            let _ = wp_viewport::req::destroy(&mut self.backend, viewport);
        }
        if let Some(layer) = m.layer {
            let _ = zwlr_layer_surface_v1::req::destroy(&mut self.backend, layer);
        }
        if let Some(surface) = m.surface {
            let _ = wl_surface::req::destroy(&mut self.backend, surface);
        }
        if m.output_version >= 3 {
            let _ = wl_output::req::release(&mut self.backend, m.output);
        }
    }

    fn monitor_mut(&mut self, output: ObjectId) -> Option<&mut Monitor> {
        self.monitors.iter_mut().find(|m| m.output == output)
    }

    fn bind(&mut self, name: u32, interface: &str, version: u32, obj: Obj) -> ObjectId {
        let id = self.objects.create(obj);
        if let Err(err) = wl_registry::req::bind(
            &mut self.backend,
            self.registry,
            name,
            id,
            interface,
            version,
        ) {
            self.fatal = Some(io(err));
        }
        id
    }
}

type Handled = Result<(), WireError>;

impl wl_display::EvHandler for State {
    fn error(&mut self, _: ObjectId, object: ObjectId, code: u32, message: &str) -> Handled {
        self.fatal = Some(format!(
            "Wayland protocol error on object {object} (code {code}): {message}"
        ));
        Ok(())
    }

    fn delete_id(&mut self, _: ObjectId, id: u32) -> Handled {
        self.dead_buffers.retain(|b| b.get().get() != id);
        self.objects.remove(id);
        Ok(())
    }
}

impl wl_registry::EvHandler for State {
    fn global(&mut self, _: ObjectId, name: u32, interface: &str, version: u32) -> Handled {
        match interface {
            wl_compositor::NAME if self.compositor.is_none() && version >= 4 => {
                self.compositor = Some(self.bind(name, interface, 4, Obj::Compositor));
            }
            wl_shm::NAME if self.shm.is_none() => {
                self.shm = Some(self.bind(name, interface, 1, Obj::Shm));
            }
            zwlr_layer_shell_v1::NAME if self.layer_shell.is_none() => {
                self.layer_shell =
                    Some(self.bind(name, interface, version.min(4), Obj::LayerShell));
            }
            wp_viewporter::NAME if self.viewporter.is_none() => {
                self.viewporter = Some(self.bind(name, interface, 1, Obj::Viewporter));
            }
            wp_fractional_scale_manager_v1::NAME if self.fractional_scale_manager.is_none() => {
                self.fractional_scale_manager =
                    Some(self.bind(name, interface, 1, Obj::FractionalScaleManager));
            }
            wl_subcompositor::NAME if self.subcompositor.is_none() => {
                self.subcompositor = Some(self.bind(name, interface, 1, Obj::Subcompositor));
            }
            wp_alpha_modifier_v1::NAME
                if self.alpha_modifier.is_none() && !alpha_modifier_disabled_for_tests() =>
            {
                self.alpha_modifier = Some(self.bind(name, interface, 1, Obj::AlphaModifier));
            }
            wl_output::NAME => {
                let version = version.min(4);
                let output = self.bind(name, interface, version, Obj::Output);
                self.monitors.push(Monitor {
                    global: name,
                    output,
                    output_version: version,
                    name: format!("monitor-{name}"),
                    scale: 1,
                    preferred_scale: None,
                    described: false,
                    surface: None,
                    layer: None,
                    viewport: None,
                    fractional_scale: None,
                    has_frame: false,
                    size: None,
                    frame_size: None,
                    frame_asked: None,
                    animation: None,
                });
            }
            _ => {}
        }
        Ok(())
    }

    fn global_remove(&mut self, _: ObjectId, name: u32) -> Handled {
        if let Some(index) = self.monitors.iter().position(|m| m.global == name) {
            self.remove_monitor(index);
        }
        Ok(())
    }
}

impl wl_callback::EvHandler for State {
    fn done(&mut self, sender: ObjectId, _: u32) -> Handled {
        if self.initial_sync == Some(sender) {
            self.initial_sync = None;
        } else {
            self.transition_frame(sender);
        }
        Ok(())
    }
}

impl wl_compositor::EvHandler for State {}
impl wl_shm_pool::EvHandler for State {}
impl zwlr_layer_shell_v1::EvHandler for State {}
impl wp_viewporter::EvHandler for State {}
impl wp_viewport::EvHandler for State {}
impl wp_fractional_scale_manager_v1::EvHandler for State {}
impl wl_subcompositor::EvHandler for State {}
impl wl_subsurface::EvHandler for State {}
impl wp_alpha_modifier_v1::EvHandler for State {}
impl wp_alpha_modifier_surface_v1::EvHandler for State {}

impl wp_fractional_scale_v1::EvHandler for State {
    fn preferred_scale(&mut self, fractional: ObjectId, scale: u32) -> Handled {
        // Only the size of the next frame changes; the frame on screen keeps
        // covering the logical size through the viewport.
        if let Some(m) = self
            .monitors
            .iter_mut()
            .find(|m| m.fractional_scale == Some(fractional))
        {
            m.preferred_scale = Some(scale);
        }
        Ok(())
    }
}

impl wl_shm::EvHandler for State {
    fn format(&mut self, _: ObjectId, _: wl_shm::Format) -> Handled {
        Ok(())
    }
}

impl wl_buffer::EvHandler for State {
    fn release(&mut self, buffer: ObjectId) -> Handled {
        if self.dead_buffers.contains(&buffer) || self.transition_keeps(buffer) {
            return Ok(());
        }
        // The compositor is done with the pixels; let them go.
        if let Err(err) = self.destroy_buffer(buffer) {
            self.fatal = Some(io(err));
        }
        Ok(())
    }
}

impl wl_surface::EvHandler for State {
    fn enter(&mut self, _: ObjectId, _: ObjectId) -> Handled {
        Ok(())
    }
    fn leave(&mut self, _: ObjectId, _: ObjectId) -> Handled {
        Ok(())
    }
    fn preferred_buffer_scale(&mut self, _: ObjectId, _: i32) -> Handled {
        Ok(())
    }
    fn preferred_buffer_transform(&mut self, _: ObjectId, _: wl_output::Transform) -> Handled {
        Ok(())
    }
}

impl wl_output::EvHandler for State {
    #[allow(clippy::too_many_arguments)]
    fn geometry(
        &mut self,
        _: ObjectId,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: wl_output::Subpixel,
        _: &str,
        _: &str,
        _: wl_output::Transform,
    ) -> Handled {
        Ok(())
    }

    fn mode(&mut self, _: ObjectId, _: wl_output::Mode, _: i32, _: i32, _: i32) -> Handled {
        Ok(())
    }

    fn done(&mut self, output: ObjectId) -> Handled {
        if let Some(m) = self.monitor_mut(output) {
            m.described = true;
        }
        Ok(())
    }

    fn scale(&mut self, output: ObjectId, factor: i32) -> Handled {
        if let Some(m) = self.monitor_mut(output) {
            m.scale = factor;
        }
        Ok(())
    }

    fn name(&mut self, output: ObjectId, name: &str) -> Handled {
        if let Some(m) = self.monitor_mut(output) {
            m.name = name.to_owned();
        }
        Ok(())
    }

    fn description(&mut self, _: ObjectId, _: &str) -> Handled {
        Ok(())
    }
}

impl zwlr_layer_surface_v1::EvHandler for State {
    fn configure(&mut self, layer: ObjectId, serial: u32, width: u32, height: u32) -> Handled {
        if let Err(err) =
            zwlr_layer_surface_v1::req::ack_configure(&mut self.backend, layer, serial)
        {
            self.fatal = Some(io(err));
        }
        let Some(index) = self.monitors.iter().position(|m| m.layer == Some(layer)) else {
            return Ok(());
        };
        // A running Transition is laid out for the old size: land it first.
        if self.monitors[index].size != Some((width, height))
            && let Some(animation) = self.monitors[index].animation.take()
            && let Err(err) = self.finish_transition(index, animation)
        {
            self.fatal = Some(io(err));
        }
        let m = &mut self.monitors[index];
        m.size = Some((width, height));
        // Stretch the frame on screen over the new logical size, so a scale
        // change never uncovers black while the new frame is rendered.
        if let (Some(viewport), Some(surface)) = (m.viewport, m.surface)
            && width > 0
            && height > 0
        {
            let has_frame = m.has_frame;
            let (w, h) = (width as i32, height as i32);
            let result = wp_viewport::req::set_destination(&mut self.backend, viewport, w, h)
                .and_then(|()| match has_frame {
                    true => wl_surface::req::commit(&mut self.backend, surface),
                    false => Ok(()),
                });
            if let Err(err) = result {
                self.fatal = Some(io(err));
            }
        }
        Ok(())
    }

    fn closed(&mut self, layer: ObjectId) -> Handled {
        if let Some(index) = self.monitors.iter().position(|m| m.layer == Some(layer)) {
            self.remove_monitor(index);
        }
        Ok(())
    }
}
