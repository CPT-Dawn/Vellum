//! The runtime: terminal setup, an input thread, a bounded worker pool, and
//! the loop that feeds Msgs to update, runs Cmds and redraws. No async.
//!
//! ```text
//!  input thread ──Key/Resize──┐
//!  timers (Cmd::After) ──Woke─┤
//!                             ▼
//!  workers ──results──▶ Msg channel ──▶ update ──Cmds──▶ job queues ──▶ workers
//!                                         │  └─ generations ──▶ Latest ──▶ workers
//!                                         └─ dirty? ──▶ view (synchronized output)
//! ```
//!
//! Workers take urgent jobs (previews, thumbnails, everything else) before
//! prefetches.

use std::collections::VecDeque;
use std::io::{self, IsTerminal, Write};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Instant;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::layout::Rect;
use ratatui_image::picker::{Picker, ProtocolType};

use super::model::Model;
use super::msg::{Cmd, Msg, Purpose, Wake};
use super::services::{Latest, Live, Services, perform};
use super::theme::{ColorEnv, Palette, Theme};
use super::{update, view};
use crate::config::Config;

pub fn run(config: Config) -> Result<(), String> {
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        return Err("the TUI needs a terminal; see `vellum --help` for the commands".into());
    }
    let theme = Theme::resolve(config.theme, ColorEnv::from_process());
    let mut terminal =
        ratatui::try_init().map_err(|err| format!("cannot set up the terminal: {err}"))?;
    let result = run_in(&mut terminal, config, theme);
    ratatui::restore();
    result
}

fn run_in(terminal: &mut DefaultTerminal, config: Config, theme: Theme) -> Result<(), String> {
    let size = terminal.size().map_err(|err| err.to_string())?;
    let home = std::env::var_os("HOME").map(Into::into);
    let mut model = Model::new(
        &config,
        theme,
        Rect::new(0, 0, size.width, size.height),
        home,
    );

    let (tx, rx) = mpsc::channel();
    let picker = picker(&model.theme);
    // The grid is cheap where kitty graphics are; elsewhere it is opt-in.
    model.grid.on = picker.protocol_type() == ProtocolType::Kitty;
    let thumbs = super::cache::thumbs_dir();
    let services: Arc<dyn Services> = Arc::new(Live::new(picker, config, thumbs));
    let latest = Arc::new(Latest::default());
    let pool = Pool::new(services, Arc::clone(&latest), tx.clone());
    spawn_input(tx);

    let mut rt = Loop {
        pool,
        latest,
        timers: Vec::new(),
    };
    for cmd in update::init(&model) {
        rt.run(cmd);
    }
    draw(terminal, &model)?;
    loop {
        let msg = match rt.timers.iter().map(|t| t.0).min() {
            None => rx.recv().map_err(|_| ()),
            Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(msg) => Ok(msg),
                Err(RecvTimeoutError::Timeout) => Ok(rt.fire_due()),
                Err(RecvTimeoutError::Disconnected) => Err(()),
            },
        };
        let Ok(mut msg) = msg else {
            return Ok(());
        };
        let mut redraw = false;
        // Fold everything already waiting into one redraw.
        loop {
            redraw |= rt.step(&mut model, msg);
            match rx.try_recv() {
                Ok(next) => msg = next,
                Err(_) => break,
            }
        }
        if model.quit {
            return Ok(());
        }
        if redraw {
            draw(terminal, &model)?;
        }
    }
}

/// What the loop owns besides the Model.
struct Loop {
    pool: Pool,
    latest: Arc<Latest>,
    /// Pending [`Cmd::After`]s: when, and what to wake update with.
    timers: Vec<(Instant, Wake)>,
}

impl Loop {
    /// Feeds one Msg to update and runs its Cmds. Returns whether to
    /// redraw.
    fn step(&mut self, model: &mut Model, msg: Msg) -> bool {
        let cmds = update::update(model, msg);
        // Before any job is queued, so workers see the new generations.
        self.latest.publish(model);
        for cmd in cmds {
            self.run(cmd);
        }
        model.dirty
    }

    fn run(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::After { delay, wake } => {
                let kind = std::mem::discriminant(&wake);
                self.timers
                    .retain(|(_, w)| std::mem::discriminant(w) != kind);
                self.timers.push((Instant::now() + delay, wake));
            }
            Cmd::RenderPreview(job) if job.purpose == Purpose::Prefetch => {
                self.pool.run(Cmd::RenderPreview(job), Priority::Low);
            }
            cmd => self.pool.run(cmd, Priority::High),
        }
    }

    /// Removes the earliest due timer and returns its Msg.
    fn fire_due(&mut self) -> Msg {
        let index = (0..self.timers.len())
            .min_by_key(|&i| self.timers[i].0)
            .expect("only called with a timer pending");
        Msg::Woke(self.timers.swap_remove(index).1)
    }
}

/// The best graphics protocol the terminal answers for, else halfblocks.
/// Must run before the input thread starts reading stdin.
fn picker(theme: &Theme) -> Picker {
    let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    if theme.palette == Palette::MidnightBloom {
        picker.set_background_color(Some(image::Rgba([0x0B, 0x0D, 0x14, 0xFF])));
    }
    picker
}

/// Draws one frame inside synchronized output, so the terminal shows it
/// all at once.
fn draw(terminal: &mut DefaultTerminal, model: &Model) -> Result<(), String> {
    let fail = |err: io::Error| format!("cannot draw: {err}");
    execute!(terminal.backend_mut(), BeginSynchronizedUpdate).map_err(fail)?;
    let drawn = terminal.draw(|f| view::view(model, f)).map(drop);
    execute!(terminal.backend_mut(), EndSynchronizedUpdate).map_err(fail)?;
    terminal.backend_mut().flush().map_err(fail)?;
    drawn.map_err(fail)
}

/// Reads terminal events forever, forwarding the ones update cares about.
/// Ends when the UI hangs up.
fn spawn_input(tx: Sender<Msg>) {
    thread::spawn(move || {
        while let Ok(event) = event::read() {
            let msg = match event {
                Event::Key(key) if key.kind != KeyEventKind::Release => Msg::Key(key),
                Event::Resize(width, height) => Msg::Resize { width, height },
                _ => continue,
            };
            if tx.send(msg).is_err() {
                return;
            }
        }
    });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Priority {
    High,
    Low,
}

/// The two job queues: urgent work first, prefetches when idle.
#[derive(Default)]
struct Queues {
    high: VecDeque<Cmd>,
    low: VecDeque<Cmd>,
}

/// A fixed set of worker threads draining two job queues. Stale preview
/// jobs are cheap to drain: workers check [`Latest`] before any I/O.
struct Pool {
    queues: Arc<(Mutex<Queues>, Condvar)>,
}

impl Pool {
    fn new(services: Arc<dyn Services>, latest: Arc<Latest>, results: Sender<Msg>) -> Pool {
        let workers = thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4));
        let queues = Arc::new((Mutex::new(Queues::default()), Condvar::new()));
        for _ in 0..workers {
            let queues = Arc::clone(&queues);
            let services = Arc::clone(&services);
            let latest = Arc::clone(&latest);
            let results = results.clone();
            thread::spawn(move || {
                loop {
                    let Some(cmd) = next_job(&queues) else {
                        return;
                    };
                    if let Some(msg) = perform(&*services, cmd, &latest)
                        && results.send(msg).is_err()
                    {
                        return;
                    }
                }
            });
        }
        Pool { queues }
    }

    fn run(&self, cmd: Cmd, priority: Priority) {
        let (lock, ready) = &*self.queues;
        // A worker only panics while holding the lock if a Service does;
        // keep going with the queue as it is.
        let mut queues = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match priority {
            Priority::High => queues.high.push_back(cmd),
            Priority::Low => queues.low.push_back(cmd),
        }
        ready.notify_one();
    }
}

/// Blocks until there is a job, urgent ones first.
fn next_job(queues: &(Mutex<Queues>, Condvar)) -> Option<Cmd> {
    let (lock, ready) = queues;
    let mut q = lock.lock().ok()?;
    loop {
        if let Some(cmd) = q.high.pop_front().or_else(|| q.low.pop_front()) {
            return Some(cmd);
        }
        q = ready.wait(q).ok()?;
    }
}
