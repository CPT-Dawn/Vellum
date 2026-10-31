//! `vellum render --restore --monitor <name>`: put the wallpaper State
//! remembers back on a Monitor. Spawned by the Daemon whenever a Monitor
//! lacks a frame (startup, hotplug, a scale change); exits right after
//! presenting, taking all decoder memory with it.

use crate::client;
use crate::set::present;

pub fn run(monitor: &str) -> Result<(), String> {
    // Never auto-starts a Daemon: the Daemon is what runs this.
    let socket = client::socket()?;
    let status = client::status(&socket)?;
    let Some(wallpaper) = status
        .monitors
        .into_iter()
        .find(|m| m.name == monitor)
        .and_then(|m| m.wallpaper)
    else {
        return Ok(());
    };
    let Some(info) = client::monitors(&socket)?
        .into_iter()
        .find(|m| m.name == monitor)
    else {
        return Ok(()); // unplugged meanwhile
    };
    let image = crate::decode::load(wallpaper.path.as_ref())
        .map_err(|err| format!("cannot restore {monitor}: {err}"))?;
    present(
        &socket,
        &image,
        &info,
        wallpaper.fit.into(),
        &wallpaper.path,
        true,
    )
}
