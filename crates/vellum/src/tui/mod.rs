//! The TUI: what `vellum` with no arguments opens.
//!
//! It follows The Elm Architecture:
//!
//! - [`model::Model`]: all state, plain data.
//! - [`msg::Msg`]: what happened (keys, resizes, worker results).
//! - [`update::update`]: `(&mut Model, Msg) -> Vec<Cmd>`, pure.
//! - [`msg::Cmd`]: work for the runtime, as data. [`services::perform`]
//!   runs one Cmd and returns its one Msg.
//! - [`view::view`]: draws the Model, pure.
//! - [`runtime`]: terminal, input thread, bounded worker pool, std channels.
//!
//! Tests drive update and view through [`harness`] (fake Daemon, fake
//! Library, placeholder images), without a terminal.

mod cache;
/// The Config's `[keymap]` section parses into this module's types.
pub mod keymap;
mod layout;
mod model;
mod msg;
mod preview;
mod runtime;
mod search;
mod services;
mod theme;
mod update;
mod view;

#[cfg(test)]
mod harness;

pub use runtime::run;
