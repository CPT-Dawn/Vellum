use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use crossterm::event::{self, Event};

#[derive(Debug)]
pub enum AppEvent {
    Input(Event),
    Tick,
}

pub fn spawn_event_thread(tick_rate: Duration) -> Receiver<AppEvent> {
    let (sender, receiver) = mpsc::channel();

    thread::spawn(move || {
        while let Ok(event_available) = event::poll(tick_rate) {
            let send_result = if event_available {
                match event::read() {
                    Ok(input) => sender.send(AppEvent::Input(input)),
                    Err(_) => break,
                }
            } else {
                sender.send(AppEvent::Tick)
            };

            if send_result.is_err() {
                break;
            }
        }
    });

    receiver
}
