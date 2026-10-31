//! The runtime: terminal setup, an input thread, a bounded worker pool, and
//! the loop that feeds Msgs to update, runs Cmds and redraws. No async.
//!
//! ```text
//!  input thread ──Key/Resize──┐
//!                             ▼
//!  workers ──results──▶ Msg channel ──▶ update ──Cmds──▶ job queue ──▶ workers
//!                                         │
//!                                         └─ dirty? ──▶ view (synchronized output)
//! ```

use std::io::{self, IsTerminal, Write};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;

use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::layout::Rect;
use ratatui_image::picker::Picker;

use super::model::Model;
use super::msg::{Cmd, Msg};
use super::services::{Live, Services, perform};
use super::theme::{ColorEnv, Palette, Theme};
use super::{update, view};
use crate::config::Config;

/// Jobs that may wait for a free worker before `run_cmd` blocks the UI.
const QUEUE: usize = 64;

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
    let services: Arc<dyn Services> = Arc::new(Live { picker, config });
    let pool = Pool::new(services, tx.clone());
    spawn_input(tx);

    for cmd in update::init(&model) {
        pool.run(cmd);
    }
    draw(terminal, &model)?;
    loop {
        let Ok(msg) = rx.recv() else {
            return Ok(());
        };
        let mut redraw = step(&mut model, msg, &pool);
        // Fold everything already waiting into one redraw.
        while let Ok(msg) = rx.try_recv() {
            redraw |= step(&mut model, msg, &pool);
        }
        if model.quit {
            return Ok(());
        }
        if redraw {
            draw(terminal, &model)?;
        }
    }
}

/// Feeds one Msg to update and queues its Cmds. Returns whether to redraw.
fn step(model: &mut Model, msg: Msg, pool: &Pool) -> bool {
    for cmd in update::update(model, msg) {
        pool.run(cmd);
    }
    model.dirty
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

/// A fixed set of worker threads draining a bounded job queue.
struct Pool {
    jobs: SyncSender<Cmd>,
}

impl Pool {
    fn new(services: Arc<dyn Services>, results: Sender<Msg>) -> Pool {
        let workers = thread::available_parallelism().map_or(2, |n| n.get().clamp(2, 4));
        let (jobs, queue) = mpsc::sync_channel::<Cmd>(QUEUE);
        let queue: Arc<Mutex<Receiver<Cmd>>> = Arc::new(Mutex::new(queue));
        for _ in 0..workers {
            let queue = Arc::clone(&queue);
            let services = Arc::clone(&services);
            let results = results.clone();
            thread::spawn(move || {
                loop {
                    let next = queue.lock().map(|q| q.recv());
                    let Ok(Ok(cmd)) = next else {
                        return;
                    };
                    if results.send(perform(&*services, cmd)).is_err() {
                        return;
                    }
                }
            });
        }
        Pool { jobs }
    }

    fn run(&self, cmd: Cmd) {
        // Workers only stop once the UI is gone, so this cannot fail while
        // the loop runs.
        let _ = self.jobs.send(cmd);
    }
}
