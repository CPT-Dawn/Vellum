//! Rotation from the `vellum` side: `vellum rotation start|stop|status`,
//! `vellum next|prev|random`, and `vellum render --rotate`, which the
//! Daemon spawns on every tick.
//!
//! The Daemon only keeps time. Every pick happens here, from the Config's
//! Rotation rules and the decks in [`crate::deck`]. Every frame goes
//! through [`set::present`] with `restore: false`, the same path `vellum
//! set` takes, so whatever runs after a present (the `on_change` hook)
//! runs for Rotation too.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use vellum_ipc::{
    MonitorInfo, Request, Response, RotationOrder, RotationSettings, RotationSource, Schedule,
    Status, Transition,
};

use crate::client::{self, ask, unexpected};
use crate::config::{Config, Named, format_interval};
use crate::deck::{self, Decks};
use crate::set::{self, Look};

/// What to put up next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// The next card by the Rotation rules (also what a tick does).
    Next,
    /// Back to what the Monitor showed before.
    Prev,
    /// Any wallpaper from the Rotation's source.
    Random,
}

/// Who asked for the pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// A command typed or bound to a key: it may start the Daemon, and it
    /// restarts each Monitor's Rotation timer.
    Manual,
    /// `vellum render --rotate`, spawned by the Daemon's timer.
    Tick,
}

/// Puts the `pick` up on `monitors` (every Monitor if empty). Monitors in
/// one sync group get the same wallpaper; the others get different ones
/// where the source allows. `transition` overrides the Config's.
pub fn run(
    pick: Pick,
    monitors: &[String],
    transition: Option<Transition>,
    trigger: Trigger,
) -> Result<(), String> {
    let config = crate::config::load();
    let socket = match trigger {
        Trigger::Manual => client::daemon()?,
        Trigger::Tick => client::socket()?,
    };
    let all = client::monitors(&socket)?;
    let targets = match (trigger, monitors) {
        (_, []) => set::target(all, None)?,
        (Trigger::Manual, [one]) => set::target(all, Some(one))?,
        // A tick names Monitors the Daemon just listed; one may have gone.
        _ => all
            .into_iter()
            .filter(|m| monitors.contains(&m.name))
            .collect(),
    };
    let status = client::status(&socket)?;
    let current: HashMap<String, PathBuf> = status
        .monitors
        .into_iter()
        .filter_map(|m| Some((m.name, PathBuf::from(m.wallpaper?.path))))
        .collect();

    let mut picker = Picker {
        config: &config,
        rotation: &config.rotation,
        current: &current,
        locked: deck::Locked::open(),
        rng: fastrand::Rng::new(),
        pools: HashMap::new(),
        chosen: Vec::new(),
    };
    let look = Look {
        fit: config.fit,
        letterbox: config.letterbox,
        transition: transition.unwrap_or(config.transition),
        restore: false,
    };
    let mut errors: Vec<String> = Vec::new();
    for (key, members) in groups(&targets, &config.rotation) {
        let picked = match pick {
            Pick::Prev => members
                .iter()
                .map(|m| picker.previous(m).map(|p| (vec![*m], p)))
                .collect::<Result<Vec<_>, _>>(),
            // The sync group shares one card per tick.
            Pick::Next | Pick::Random if key == deck::GLOBAL && config.rotation.sync => picker
                .draw(&key, &members, pick)
                .map(|p| vec![(members, p)]),
            // Otherwise each Monitor takes its own card, from the shared
            // global deck or its Override's.
            Pick::Next | Pick::Random => members
                .iter()
                .map(|m| picker.draw(&key, &[*m], pick).map(|p| (vec![*m], p)))
                .collect::<Result<Vec<_>, _>>(),
        };
        let picked = match picked {
            Ok(picked) => picked,
            Err(err) => {
                errors.push(err);
                continue;
            }
        };
        for (members, (path, image)) in picked {
            let source = path.to_string_lossy();
            for monitor in members {
                match set::present(&socket, &image, monitor, &source, look) {
                    Ok(()) => {
                        if pick != Pick::Prev {
                            picker.locked.decks.shown(&monitor.name, &path);
                        }
                        if trigger == Trigger::Manual {
                            reset_timer(&socket, &monitor.name)?;
                        }
                    }
                    Err(err) => errors.push(err),
                }
            }
        }
    }
    picker.locked.save();
    match errors.is_empty() {
        true => Ok(()),
        false => Err(errors.join("\nvellum: ")),
    }
}

/// Tells the Daemon a wallpaper was applied by hand on `monitor`, so its
/// Rotation timer starts over.
pub fn reset_timer(socket: &Path, monitor: &str) -> Result<(), String> {
    match ask(socket, &Request::RotationReset(monitor.to_owned()), &[])? {
        Response::Ok => Ok(()),
        Response::Error(err) => Err(err),
        other => Err(unexpected(&other)),
    }
}

/// The targets grouped by deck: every Monitor following the global
/// Rotation draws from one shared deck, so they get different wallpapers
/// and nothing repeats until the whole source has been shown. A Monitor
/// with an Override has a deck of its own. In Monitor order.
fn groups<'a>(
    targets: &'a [MonitorInfo],
    rotation: &RotationSettings,
) -> Vec<(String, Vec<&'a MonitorInfo>)> {
    let mut groups: Vec<(String, Vec<&MonitorInfo>)> = Vec::new();
    for monitor in targets {
        let overridden = rotation.overrides.iter().any(|o| o.monitor == monitor.name);
        let key = match overridden {
            false => deck::GLOBAL.to_owned(),
            true => monitor.name.clone(),
        };
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(monitor),
            None => groups.push((key, vec![monitor])),
        }
    }
    groups
}

struct Picker<'a> {
    config: &'a Config,
    rotation: &'a RotationSettings,
    current: &'a HashMap<String, PathBuf>,
    locked: deck::Locked,
    rng: fastrand::Rng,
    pools: HashMap<u8, Vec<PathBuf>>,
    /// Wallpapers picked in this run, for different ones per Monitor.
    chosen: Vec<PathBuf>,
}

impl Picker<'_> {
    fn pool(&mut self, source: RotationSource) -> Result<Vec<PathBuf>, String> {
        let config = self.config;
        let pool = self
            .pools
            .entry(source as u8)
            .or_insert_with(|| match source {
                RotationSource::Library => crate::library::scan(&config.library),
                RotationSource::Favorites => crate::favorites::existing(true),
            });
        if !pool.is_empty() {
            return Ok(pool.clone());
        }
        Err(match source {
            RotationSource::Library => {
                if let Some(hint) = crate::library::missing_hint(&config.library) {
                    return Err(hint);
                }
                let folders: Vec<String> = config
                    .library
                    .iter()
                    .map(|f| f.display().to_string())
                    .collect();
                format!(
                    "the Library has no wallpapers ({}); put some there or set `library` in the Config",
                    folders.join(", ")
                )
            }
            RotationSource::Favorites => {
                "there are no Favorites to rotate through; add some with `vellum fav add <file>`"
                    .into()
            }
        })
    }

    /// The next (or a random) decodable wallpaper for one deck. Files that
    /// fail to decode are skipped with a warning.
    fn draw(
        &mut self,
        key: &str,
        members: &[&MonitorInfo],
        pick: Pick,
    ) -> Result<(PathBuf, image::RgbaImage), String> {
        let schedule: Schedule = *self.rotation.schedule_for(&members[0].name);
        let pool = self.pool(schedule.source)?;
        // Above all not what another Monitor just got, then not what any
        // Monitor shows now, where the source allows.
        let on_screen: Vec<PathBuf> = self.current.values().cloned().collect();
        let chosen = self.chosen.clone();
        let avoid: [&[PathBuf]; 2] = [&chosen, &on_screen];
        let mut exclude: Vec<PathBuf> = Vec::new();
        loop {
            let candidate = match pick {
                Pick::Random => random(&pool, &avoid, &exclude, &mut self.rng),
                _ => self.locked.decks.deck(key).draw(
                    &pool,
                    schedule.order,
                    &avoid,
                    &exclude,
                    &mut self.rng,
                ),
            };
            let Some(path) = candidate else {
                return Err(format!(
                    "none of the {} wallpapers in the Rotation could be decoded",
                    pool.len()
                ));
            };
            match crate::decode::load(&path) {
                Ok(image) => {
                    self.chosen.push(path.clone());
                    return Ok((path, image));
                }
                Err(err) => {
                    eprintln!("vellum: skipping {}: {err}", path.display());
                    exclude.push(path);
                }
            }
        }
    }

    /// What `monitor` showed before, from its history.
    fn previous(&mut self, monitor: &MonitorInfo) -> Result<(PathBuf, image::RgbaImage), String> {
        let decks: &mut Decks = &mut self.locked.decks;
        let current = self.current.get(&monitor.name).map(PathBuf::as_path);
        loop {
            let Some(path) = decks.previous(&monitor.name, current) else {
                return Err(format!(
                    "{}: no earlier wallpaper to go back to",
                    monitor.name
                ));
            };
            match crate::decode::load(&path) {
                Ok(image) => return Ok((path, image)),
                Err(err) => {
                    eprintln!("vellum: skipping {}: {err}", path.display());
                    decks.drop_latest(&monitor.name);
                }
            }
        }
    }
}

/// A random card from `pool`, steering clear of `avoid` tiers as
/// [`deck::pick`] does.
fn random(
    pool: &[PathBuf],
    avoid: &[&[PathBuf]],
    exclude: &[PathBuf],
    rng: &mut fastrand::Rng,
) -> Option<PathBuf> {
    let mut cards = pool.to_vec();
    rng.shuffle(&mut cards);
    deck::pick(&cards, avoid, exclude).map(|i| cards.swap_remove(i))
}

/// `vellum rotation start` and `stop`.
pub fn control(start: bool) -> Result<(), String> {
    let socket = client::daemon()?;
    let request = match start {
        true => Request::RotationStart,
        false => Request::RotationStop,
    };
    match ask(&socket, &request, &[])? {
        Response::Ok => {}
        Response::Error(err) => return Err(err),
        other => return Err(unexpected(&other)),
    }
    println!("rotation: {}", summary(&client::status(&socket)?));
    Ok(())
}

/// `vellum rotation status`.
pub fn status() -> Result<(), String> {
    let status = client::status(&client::daemon()?)?;
    print!("{}", report(&status));
    Ok(())
}

/// One line for `vellum status`: "running, every 30m, shuffle from the
/// Library".
pub fn summary(status: &Status) -> String {
    if !status.rotation_running {
        return "stopped".into();
    }
    let r = &status.settings.rotation;
    let mut out = format!("running, {}", schedule(&r.schedule));
    if r.sync {
        out += ", sync";
    }
    match r.overrides.len() {
        0 => {}
        1 => out += ", 1 Override",
        n => out += &format!(", {n} Overrides"),
    }
    out
}

fn schedule(s: &Schedule) -> String {
    let source = match s.source {
        RotationSource::Library => "the Library",
        RotationSource::Favorites => "Favorites",
    };
    let order = match s.order {
        RotationOrder::Shuffle => "shuffle",
        RotationOrder::Sequential => "in order",
    };
    format!(
        "every {}, {order} from {source}",
        format_interval(s.interval_ms)
    )
}

fn report(status: &Status) -> String {
    let r = &status.settings.rotation;
    let mut out = format!(
        "rotation: {}\nsync: {}\n\n",
        if status.rotation_running {
            "running"
        } else {
            "stopped"
        },
        if r.sync { "on" } else { "off" }
    );
    let rows: Vec<[String; 5]> = status
        .monitors
        .iter()
        .map(|m| {
            let s = r.schedule_for(&m.name);
            [
                m.name.clone(),
                s.source.name().to_owned(),
                s.order.name().to_owned(),
                format_interval(s.interval_ms),
                m.next_change_ms.map_or("-".into(), countdown),
            ]
        })
        .collect();
    let rows: Vec<Vec<&str>> = rows
        .iter()
        .map(|row| row.iter().map(String::as_str).collect())
        .collect();
    out += &crate::status::table(&["MONITOR", "SOURCE", "ORDER", "INTERVAL", "NEXT"], &rows);
    out
}

/// "in 4m 10s", rounded up to whole seconds.
fn countdown(ms: u64) -> String {
    let secs = ms.div_ceil(1000);
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    match (h, m) {
        (0, 0) => format!("in {s}s"),
        (0, _) => format!("in {m}m {s}s"),
        _ => format!("in {h}h {m}m"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vellum_ipc::{MonitorStatus, Override, Settings};

    fn status(running: bool) -> Status {
        let mut settings = Settings::default();
        settings.rotation.overrides.push(Override {
            monitor: "DP-2".into(),
            schedule: Schedule {
                interval_ms: 90_000,
                source: RotationSource::Favorites,
                order: RotationOrder::Sequential,
            },
        });
        Status {
            version: "2.0.0".into(),
            pid: 1,
            rss_bytes: 0,
            monitors: ["DP-1", "DP-2"]
                .into_iter()
                .zip([Some(250_500), Some(3_700_000)])
                .map(|(name, next)| MonitorStatus {
                    name: name.into(),
                    wallpaper: None,
                    next_change_ms: next.filter(|_| running),
                })
                .collect(),
            settings,
            rotation_running: running,
        }
    }

    #[test]
    fn rotation_status_shows_source_order_interval_and_time_to_the_next_change() {
        assert_eq!(
            report(&status(true)),
            "rotation: running\nsync: off\n\n\
             MONITOR  SOURCE     ORDER       INTERVAL  NEXT\n\
             DP-1     library    shuffle     30m       in 4m 11s\n\
             DP-2     favorites  sequential  1m30s     in 1h 1m\n"
        );
        assert!(report(&status(false)).contains("DP-1     library    shuffle     30m       -\n"));
    }

    #[test]
    fn the_summary_line_reads_like_a_sentence() {
        assert_eq!(summary(&status(false)), "stopped");
        assert_eq!(
            summary(&status(true)),
            "running, every 30m, shuffle from the Library, 1 Override"
        );
    }

    #[test]
    fn random_prefers_what_is_not_on_screen() {
        let pool: Vec<PathBuf> = ["/a", "/b"].map(PathBuf::from).to_vec();
        let mut rng = fastrand::Rng::with_seed(3);
        for _ in 0..20 {
            assert_eq!(
                random(&pool, &[&pool[..1]], &[], &mut rng),
                Some(pool[1].clone())
            );
        }
        assert_eq!(random(&pool, &[], &pool, &mut rng), None);
    }
}
