//! Transitions: how a presented frame replaces what a Monitor shows.
//!
//! The new frame goes on a temporary desynchronized subsurface above the
//! Monitor's background surface, which keeps showing the old wallpaper (the
//! compositor still has it; the Daemon does not). Every frame callback moves
//! the Transition along by wall-clock progress, so the Monitor's refresh rate
//! sets the pace and no timer is involved:
//!
//! - Fade ramps the subsurface's wp_alpha_modifier multiplier (GPU blend).
//!   Without alpha-modifier, a short CPU blend writes the new frame with
//!   premultiplied alpha into two alternating ARGB buffers and the compositor
//!   blends those over the old wallpaper.
//! - Wipe and Slide only move viewport rectangles and the subsurface
//!   position; no pixels are touched.
//!
//! At the end the frame moves onto the background surface and every
//! temporary object, buffer and mapping is freed.

use std::os::fd::OwnedFd;
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use rustix::fs::{MemfdFlags, SealFlags};
use rustix::io::Errno;
use rustix::mm::{MapFlags, ProtFlags};
use vellum_ipc::{Direction, Easing, Transition, TransitionKind};
use waybackend::types::{ObjectId, WlFixed};

use super::{Obj, State};
use crate::protocols::*;

/// The CPU Fade touches every pixel each frame, so it is kept short.
const CPU_FADE_MAX: Duration = Duration::from_millis(300);

/// A running Transition on one Monitor.
pub(super) struct Animation {
    effect: Effect,
    easing: Easing,
    start: Instant,
    duration: Duration,
    /// The Monitor's size in surface-local (logical) units.
    logical: (u32, u32),
    /// The frame's size in pixels.
    physical: (u32, u32),
    stride: u32,
    surface: ObjectId,
    subsurface: ObjectId,
    viewport: ObjectId,
    /// The pool of the new frame. The final buffer on the background surface
    /// is cut from it, so the subsurface's buffer never has two owners.
    pool: ObjectId,
    /// The new frame as attached to the subsurface (not for the CPU Fade).
    buffer: Option<ObjectId>,
    /// The pending frame callback.
    callback: Option<ObjectId>,
}

enum Effect {
    Fade { alpha: ObjectId },
    CpuFade { source: Mapping, blends: [Blend; 2] },
    Sweep { kind: Sweep, direction: Direction },
}

/// One of the CPU Fade's alternating buffers.
struct Blend {
    memory: Mapping,
    buffer: ObjectId,
    /// Attached and not yet released by the compositor.
    busy: bool,
}

/// Transitions that move rectangles rather than blend.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Sweep {
    /// The new frame stays put; a moving edge reveals it.
    Wipe,
    /// The new frame moves in from the edge over the old one.
    Slide,
}

/// Where the subsurface shows which part of the new frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Placement {
    /// Source rectangle in buffer pixels: x, y, width, height.
    pub source: (f64, f64, f64, f64),
    /// Destination size in surface-local units.
    pub size: (i32, i32),
    /// Subsurface position in surface-local units.
    pub position: (i32, i32),
}

/// The Wipe or Slide geometry at eased progress `p` in `0..=1`. At least one
/// unit is always shown, because a viewport cannot be empty.
pub(super) fn placement(
    sweep: Sweep,
    direction: Direction,
    p: f32,
    logical: (u32, u32),
    physical: (u32, u32),
) -> Placement {
    let horizontal = matches!(direction, Direction::Left | Direction::Right);
    let (along, across) = if horizontal {
        (logical.0, logical.1)
    } else {
        (logical.1, logical.0)
    };
    let (pixels_along, pixels_across) = if horizontal {
        (physical.0, physical.1)
    } else {
        (physical.1, physical.0)
    };
    let shown = ((p.clamp(0.0, 1.0) * along as f32).round() as u32).clamp(1, along.max(1));
    let shown_pixels = f64::from(shown) * f64::from(pixels_along) / f64::from(along.max(1));
    // Moving right or down, a Wipe reveals the frame's start and a Slide
    // shows the frame's end entering at the start. Left and up mirror it.
    let forward = matches!(direction, Direction::Right | Direction::Down);
    let from_frame_start = forward == (sweep == Sweep::Wipe);
    let offset = if from_frame_start {
        0.0
    } else {
        f64::from(pixels_along) - shown_pixels
    };
    let position = if forward { 0 } else { (along - shown) as i32 };
    if horizontal {
        Placement {
            source: (offset, 0.0, shown_pixels, f64::from(pixels_across)),
            size: (shown as i32, across as i32),
            position: (position, 0),
        }
    } else {
        Placement {
            source: (0.0, offset, f64::from(pixels_across), shown_pixels),
            size: (across as i32, shown as i32),
            position: (0, position),
        }
    }
}

/// Writes XRGB8888 `source` into ARGB8888 `out` at opacity `alpha`, with
/// premultiplied colour as Wayland expects.
pub(super) fn premultiply(source: &[u8], out: &mut [u8], alpha: u8) {
    let a = u32::from(alpha);
    let (source, _) = source.as_chunks::<4>();
    let (out, _) = out.as_chunks_mut::<4>();
    for (s, o) in source.iter().zip(out) {
        o[0] = ((u32::from(s[0]) * a + 127) / 255) as u8;
        o[1] = ((u32::from(s[1]) * a + 127) / 255) as u8;
        o[2] = ((u32::from(s[2]) * a + 127) / 255) as u8;
        o[3] = alpha;
    }
}

/// A shared mapping of a memfd. Pixel memory lives only in mappings like
/// this, never on the heap, so freeing it really returns it to the system.
struct Mapping {
    ptr: NonNull<u8>,
    len: usize,
    /// Kept open until the compositor has its copy (fds go out on flush).
    fd: Option<OwnedFd>,
}

impl Mapping {
    /// A fresh zeroed memfd of `len` bytes, mapped read-write.
    fn anonymous(len: usize) -> Result<Mapping, Errno> {
        let fd = rustix::fs::memfd_create("vellumd-blend", MemfdFlags::CLOEXEC)?;
        rustix::fs::ftruncate(&fd, len as u64)?;
        let mut mapping = Mapping::map(&fd, len, ProtFlags::READ | ProtFlags::WRITE)?;
        mapping.fd = Some(fd);
        Ok(mapping)
    }

    /// Maps the first `len` bytes of a client's frame read-only. The client
    /// must have sealed it against shrinking, or a later truncation would
    /// crash the Daemon with SIGBUS.
    fn frame(fd: &OwnedFd, len: usize) -> Result<Mapping, Errno> {
        if !rustix::fs::fcntl_get_seals(fd)?.contains(SealFlags::SHRINK) {
            return Err(Errno::PERM);
        }
        Mapping::map(fd, len, ProtFlags::READ)
    }

    fn map(fd: &OwnedFd, len: usize, prot: ProtFlags) -> Result<Mapping, Errno> {
        // SAFETY: a fresh shared mapping of a file at least `len` bytes long
        // (sized by us, or checked by `present` and sealed by the client).
        let ptr =
            unsafe { rustix::mm::mmap(std::ptr::null_mut(), len, prot, MapFlags::SHARED, fd, 0)? };
        let ptr = NonNull::new(ptr.cast()).expect("mmap never returns null");
        Ok(Mapping { ptr, len, fd: None })
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: the mapping is `len` readable bytes and lives as long as
        // `self`.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    /// Only for mappings made by [`Mapping::anonymous`].
    fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: as in `bytes`, mapped writable, and `&mut self` makes this
        // the only reference.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: our own mapping, unmapped exactly once.
        let _ = unsafe { rustix::mm::munmap(self.ptr.as_ptr().cast(), self.len) };
    }
}

impl Animation {
    fn progress(&self, now: Instant) -> f32 {
        let t = now.saturating_duration_since(self.start).as_secs_f32()
            / self.duration.as_secs_f32().max(f32::EPSILON);
        t.min(1.0)
    }

    fn owns_buffer(&self, buffer: ObjectId) -> bool {
        self.buffer == Some(buffer)
            || matches!(&self.effect, Effect::CpuFade { blends, .. }
                if blends.iter().any(|b| b.buffer == buffer))
    }
}

/// The new frame, already in a pool and a buffer, ready for a Transition.
pub(super) struct NewFrame<'a> {
    pub pool: ObjectId,
    pub buffer: ObjectId,
    pub memfd: &'a OwnedFd,
    pub stride: u32,
}

impl State {
    /// Starts `transition` on Monitor `index` with `frame`. Returns the frame
    /// back if this Transition cannot run here; the caller then swaps
    /// immediately. Otherwise the Transition owns the pool and buffer.
    pub(super) fn start_transition<'a>(
        &mut self,
        index: usize,
        transition: Transition,
        frame: NewFrame<'a>,
    ) -> Result<Option<NewFrame<'a>>, Errno> {
        let m = &self.monitors[index];
        let (Some(parent), Some(logical), Some(physical)) = (m.surface, m.size, m.physical_size())
        else {
            return Ok(Some(frame));
        };
        let (Some(compositor), Some(subcompositor), Some(viewporter)) =
            (self.compositor, self.subcompositor, self.viewporter)
        else {
            return Ok(Some(frame));
        };
        if !m.has_frame || transition.duration_ms == 0 || logical.0 == 0 || logical.1 == 0 {
            return Ok(Some(frame));
        }
        let mut duration = Duration::from_millis(transition.duration_ms.into());
        let frame_len = frame.stride as usize * physical.1 as usize;

        // Decide everything that can fail before creating any object.
        enum Plan {
            Fade(ObjectId),
            CpuFade(Mapping, [Mapping; 2]),
            Sweep(Sweep, Direction),
        }
        let plan = match transition.kind {
            TransitionKind::None => return Ok(Some(frame)),
            TransitionKind::Fade => match self.alpha_modifier {
                Some(manager) => Plan::Fade(manager),
                None => {
                    let blend_len = physical.0 as usize * 4 * physical.1 as usize;
                    let mapped = Mapping::frame(frame.memfd, frame_len).and_then(|source| {
                        Ok((
                            source,
                            [
                                Mapping::anonymous(blend_len)?,
                                Mapping::anonymous(blend_len)?,
                            ],
                        ))
                    });
                    match mapped {
                        Ok((source, blends)) => Plan::CpuFade(source, blends),
                        Err(err) => {
                            eprintln!("vellumd: no memory for a CPU fade ({err}); swapping");
                            return Ok(Some(frame));
                        }
                    }
                }
            },
            TransitionKind::Wipe => Plan::Sweep(Sweep::Wipe, transition.direction),
            TransitionKind::Slide => Plan::Sweep(Sweep::Slide, transition.direction),
        };

        let surface = self.objects.create(Obj::Surface);
        wl_compositor::req::create_surface(&mut self.backend, compositor, surface)?;
        let subsurface = self.objects.create(Obj::Subsurface);
        wl_subcompositor::req::get_subsurface(
            &mut self.backend,
            subcompositor,
            subsurface,
            surface,
            parent,
        )?;
        wl_subsurface::req::set_desync(&mut self.backend, subsurface)?;
        wl_subsurface::req::set_position(&mut self.backend, subsurface, 0, 0)?;
        let viewport = self.objects.create(Obj::Viewport);
        wp_viewporter::req::get_viewport(&mut self.backend, viewporter, viewport, surface)?;
        let full = (logical.0 as i32, logical.1 as i32);

        let (effect, attach, buffer) = match plan {
            Plan::Fade(manager) => {
                let alpha = self.objects.create(Obj::AlphaModifierSurface);
                wp_alpha_modifier_v1::req::get_surface(&mut self.backend, manager, alpha, surface)?;
                wp_alpha_modifier_surface_v1::req::set_multiplier(&mut self.backend, alpha, 0)?;
                wp_viewport::req::set_destination(&mut self.backend, viewport, full.0, full.1)?;
                (Effect::Fade { alpha }, frame.buffer, Some(frame.buffer))
            }
            Plan::CpuFade(source, memories) => {
                duration = duration.min(CPU_FADE_MAX);
                let shm = self.shm.expect("bound at startup");
                let mut blends = Vec::with_capacity(2);
                for memory in memories {
                    let pool = self.objects.create(Obj::ShmPool);
                    let fd = memory
                        .fd
                        .as_ref()
                        .expect("anonymous mappings keep their fd");
                    wl_shm::req::create_pool(&mut self.backend, shm, pool, fd, memory.len as i32)?;
                    let buffer = self.objects.create(Obj::Buffer);
                    wl_shm_pool::req::create_buffer(
                        &mut self.backend,
                        pool,
                        buffer,
                        0,
                        physical.0 as i32,
                        physical.1 as i32,
                        physical.0 as i32 * 4,
                        wl_shm::Format::argb8888,
                    )?;
                    wl_shm_pool::req::destroy(&mut self.backend, pool)?;
                    blends.push(Blend {
                        memory,
                        buffer,
                        busy: false,
                    });
                }
                let mut blends: [Blend; 2] = blends.try_into().ok().expect("two blends");
                // The first blend is all zeros: fully transparent.
                blends[0].busy = true;
                wp_viewport::req::set_destination(&mut self.backend, viewport, full.0, full.1)?;
                let attach = blends[0].buffer;
                (Effect::CpuFade { source, blends }, attach, None)
            }
            Plan::Sweep(kind, direction) => (
                Effect::Sweep { kind, direction },
                frame.buffer,
                Some(frame.buffer),
            ),
        };

        let mut animation = Animation {
            effect,
            easing: transition.easing,
            start: Instant::now(),
            duration,
            logical,
            physical,
            stride: frame.stride,
            surface,
            subsurface,
            viewport,
            pool: frame.pool,
            buffer,
            callback: None,
        };
        if let Effect::Sweep { kind, direction } = animation.effect {
            self.place(
                &animation,
                placement(kind, direction, 0.0, logical, physical),
            )?;
        }
        wl_surface::req::attach(&mut self.backend, surface, Some(attach), 0, 0)?;
        wl_surface::req::damage_buffer(&mut self.backend, surface, 0, 0, i32::MAX, i32::MAX)?;
        self.request_frame(&mut animation)?;
        wl_surface::req::commit(&mut self.backend, surface)?;
        // The subsurface's position and stacking apply with its parent.
        wl_surface::req::commit(&mut self.backend, parent)?;
        // The blend memfds must survive until the flush carries them over.
        self.backend.flush()?;
        if let Effect::CpuFade { blends, .. } = &mut animation.effect {
            for blend in blends {
                blend.memory.fd = None;
            }
        }
        self.monitors[index].animation = Some(animation);
        Ok(None)
    }

    fn request_frame(&mut self, animation: &mut Animation) -> Result<(), Errno> {
        let callback = self.objects.create(Obj::FrameCallback);
        wl_surface::req::frame(&mut self.backend, animation.surface, callback)?;
        animation.callback = Some(callback);
        Ok(())
    }

    fn place(&mut self, animation: &Animation, at: Placement) -> Result<(), Errno> {
        let (x, y, w, h) = at.source;
        wp_viewport::req::set_source(
            &mut self.backend,
            animation.viewport,
            WlFixed::from(x),
            WlFixed::from(y),
            WlFixed::from(w),
            WlFixed::from(h),
        )?;
        wp_viewport::req::set_destination(
            &mut self.backend,
            animation.viewport,
            at.size.0,
            at.size.1,
        )?;
        wl_subsurface::req::set_position(
            &mut self.backend,
            animation.subsurface,
            at.position.0,
            at.position.1,
        )
    }

    /// A frame callback fired: if it is a Transition's, move it along.
    /// Returns whether the callback belonged to one.
    pub(super) fn transition_frame(&mut self, callback: ObjectId) -> bool {
        let Some(index) = self.monitors.iter().position(|m| {
            m.animation
                .as_ref()
                .is_some_and(|a| a.callback == Some(callback))
        }) else {
            return false;
        };
        let mut animation = self.monitors[index].animation.take().expect("found above");
        animation.callback = None;
        if let Err(err) = self.step(index, animation) {
            self.fatal = Some(super::io(err));
        }
        true
    }

    fn step(&mut self, index: usize, mut animation: Animation) -> Result<(), Errno> {
        let t = animation.progress(Instant::now());
        if t >= 1.0 {
            return self.finish_transition(index, animation);
        }
        let p = animation.easing.apply(t);
        let surface = animation.surface;
        let mut commit_parent = false;
        match &mut animation.effect {
            Effect::Fade { alpha } => {
                let factor = (f64::from(p) * f64::from(u32::MAX)) as u32;
                wp_alpha_modifier_surface_v1::req::set_multiplier(
                    &mut self.backend,
                    *alpha,
                    factor,
                )?;
            }
            Effect::CpuFade { source, blends } => {
                // If the compositor still holds both buffers, skip this frame.
                if let Some(blend) = blends.iter_mut().find(|b| !b.busy) {
                    let alpha = (p * 255.0).round() as u8;
                    premultiply(source.bytes(), blend.memory.bytes_mut(), alpha);
                    blend.busy = true;
                    wl_surface::req::attach(&mut self.backend, surface, Some(blend.buffer), 0, 0)?;
                    wl_surface::req::damage_buffer(
                        &mut self.backend,
                        surface,
                        0,
                        0,
                        i32::MAX,
                        i32::MAX,
                    )?;
                }
            }
            Effect::Sweep { kind, direction } => {
                let at = placement(*kind, *direction, p, animation.logical, animation.physical);
                self.place(&animation, at)?;
                commit_parent = true;
            }
        }
        self.request_frame(&mut animation)?;
        wl_surface::req::commit(&mut self.backend, surface)?;
        if commit_parent && let Some(parent) = self.monitors[index].surface {
            wl_surface::req::commit(&mut self.backend, parent)?;
        }
        self.monitors[index].animation = Some(animation);
        Ok(())
    }

    /// Jumps a Transition to its end: the new frame goes onto the Monitor's
    /// background surface and everything temporary is freed.
    pub(super) fn finish_transition(
        &mut self,
        index: usize,
        animation: Animation,
    ) -> Result<(), Errno> {
        let buffer = self.objects.create(Obj::Buffer);
        let (width, height) = animation.physical;
        wl_shm_pool::req::create_buffer(
            &mut self.backend,
            animation.pool,
            buffer,
            0,
            width as i32,
            height as i32,
            animation.stride as i32,
            wl_shm::Format::xrgb8888,
        )?;
        self.show(index, buffer)?;
        self.drop_transition(animation)
    }

    /// Destroys a Transition's objects without showing its frame.
    pub(super) fn drop_transition(&mut self, animation: Animation) -> Result<(), Errno> {
        let b = &mut self.backend;
        match &animation.effect {
            Effect::Fade { alpha } => wp_alpha_modifier_surface_v1::req::destroy(b, *alpha)?,
            Effect::CpuFade { .. } | Effect::Sweep { .. } => {}
        }
        wp_viewport::req::destroy(b, animation.viewport)?;
        wl_subsurface::req::destroy(b, animation.subsurface)?;
        wl_surface::req::destroy(b, animation.surface)?;
        wl_shm_pool::req::destroy(b, animation.pool)?;
        if let Some(buffer) = animation.buffer {
            self.destroy_buffer(buffer)?;
        }
        if let Effect::CpuFade { blends, .. } = &animation.effect {
            for blend in blends {
                self.destroy_buffer(blend.buffer)?;
            }
        }
        // Dropping the animation unmaps the frame and blend memory.
        Ok(())
    }

    /// Whether a running Transition still needs `buffer`. Marks a released
    /// CPU Fade buffer as free for the next frame.
    pub(super) fn transition_keeps(&mut self, buffer: ObjectId) -> bool {
        for m in &mut self.monitors {
            let Some(animation) = &mut m.animation else {
                continue;
            };
            if !animation.owns_buffer(buffer) {
                continue;
            }
            if let Effect::CpuFade { blends, .. } = &mut animation.effect {
                for blend in blends.iter_mut().filter(|b| b.buffer == buffer) {
                    blend.busy = false;
                }
            }
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOGICAL: (u32, u32) = (100, 50);
    const PHYSICAL: (u32, u32) = (200, 100);

    fn at(sweep: Sweep, direction: Direction, p: f32) -> Placement {
        placement(sweep, direction, p, LOGICAL, PHYSICAL)
    }

    #[test]
    fn a_rightward_wipe_reveals_the_left_of_the_new_frame_in_place() {
        let p = at(Sweep::Wipe, Direction::Right, 0.25);
        assert_eq!(p.source, (0.0, 0.0, 50.0, 100.0));
        assert_eq!(p.size, (25, 50));
        assert_eq!(p.position, (0, 0));
    }

    #[test]
    fn a_leftward_wipe_reveals_the_right_of_the_new_frame_in_place() {
        let p = at(Sweep::Wipe, Direction::Left, 0.25);
        assert_eq!(p.source, (150.0, 0.0, 50.0, 100.0));
        assert_eq!(p.size, (25, 50));
        assert_eq!(p.position, (75, 0));
    }

    #[test]
    fn a_rightward_slide_enters_from_the_left_leading_with_its_right_edge() {
        let p = at(Sweep::Slide, Direction::Right, 0.25);
        assert_eq!(p.source, (150.0, 0.0, 50.0, 100.0));
        assert_eq!(p.size, (25, 50));
        assert_eq!(p.position, (0, 0));
    }

    #[test]
    fn an_upward_slide_enters_from_the_bottom_leading_with_its_top_edge() {
        let p = at(Sweep::Slide, Direction::Up, 0.2);
        assert_eq!(p.source, (0.0, 0.0, 200.0, 20.0));
        assert_eq!(p.size, (100, 10));
        assert_eq!(p.position, (0, 40));
    }

    #[test]
    fn a_downward_wipe_reveals_the_top_first() {
        let p = at(Sweep::Wipe, Direction::Down, 0.2);
        assert_eq!(p.source, (0.0, 0.0, 200.0, 20.0));
        assert_eq!(p.size, (100, 10));
        assert_eq!(p.position, (0, 0));
    }

    #[test]
    fn the_start_shows_one_unit_and_the_end_shows_everything() {
        let start = at(Sweep::Slide, Direction::Left, 0.0);
        assert_eq!(start.size, (1, 50));
        assert_eq!(start.position, (99, 0));
        assert_eq!(start.source, (0.0, 0.0, 2.0, 100.0));
        for direction in [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ] {
            for sweep in [Sweep::Wipe, Sweep::Slide] {
                let end = at(sweep, direction, 1.0);
                assert_eq!(
                    end.source,
                    (0.0, 0.0, 200.0, 100.0),
                    "{sweep:?} {direction:?}"
                );
                assert_eq!(end.size, (100, 50));
                assert_eq!(end.position, (0, 0));
            }
        }
    }

    #[test]
    fn premultiplying_scales_colour_by_alpha() {
        let source = [200, 100, 50, 7, 255, 255, 255, 0];
        let mut out = [0; 8];
        premultiply(&source, &mut out, 128);
        assert_eq!(out, [100, 50, 25, 128, 128, 128, 128, 128]);
        premultiply(&source, &mut out, 255);
        assert_eq!(out, [200, 100, 50, 255, 255, 255, 255, 255]);
        premultiply(&source, &mut out, 0);
        assert_eq!(out, [0; 8]);
    }
}
