//! Rotation's memory, which belongs to `vellum` (the Daemon keeps only the
//! timer): one deck per Monitor (or per sync group), plus each Monitor's
//! recent wallpapers for `vellum prev`.
//!
//! - **Shuffle** keeps the remaining cards of a shuffled deck. When it runs
//!   out it is reshuffled, never starting with the card drawn last, so no
//!   wallpaper repeats across the boundary.
//! - **Sequential** keeps the last wallpaper drawn; the next one follows it
//!   in Library (or Favorites) order.
//!
//! Stored in `$XDG_STATE_HOME/vellum/rotation` as lines, behind an flock
//! on `rotation.lock` so concurrent ticks and keybinds never interleave:
//!
//! ```text
//! vellum-rotation 1
//! last <deck> <path>
//! card <deck> <path>
//! shown <monitor> <path>
//! ```

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use vellum_ipc::RotationOrder;

const HEADER: &str = "vellum-rotation 1";
/// How far back `vellum prev` can go.
const HISTORY: usize = 50;
/// The deck key of the sync group.
pub const SYNC_GROUP: &str = "*";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Deck {
    /// Shuffle: the cards still to draw, next first.
    pub cards: Vec<PathBuf>,
    /// The card drawn last: the position in sequential order.
    pub last: Option<PathBuf>,
}

impl Deck {
    /// Draws the next wallpaper from `pool`. Cards in `exclude` (failed to
    /// decode this run) are never returned; cards in `avoid` (shown on
    /// another Monitor) only when nothing else is left.
    pub fn draw(
        &mut self,
        pool: &[PathBuf],
        order: RotationOrder,
        avoid: &[PathBuf],
        exclude: &[PathBuf],
        rng: &mut fastrand::Rng,
    ) -> Option<PathBuf> {
        let pick = match order {
            RotationOrder::Shuffle => self.draw_shuffled(pool, avoid, exclude, rng),
            RotationOrder::Sequential => self.draw_sequential(pool, avoid, exclude),
        }?;
        self.last = Some(pick.clone());
        Some(pick)
    }

    fn draw_shuffled(
        &mut self,
        pool: &[PathBuf],
        avoid: &[PathBuf],
        exclude: &[PathBuf],
        rng: &mut fastrand::Rng,
    ) -> Option<PathBuf> {
        let in_pool: HashSet<&PathBuf> = pool.iter().collect();
        self.cards.retain(|c| in_pool.contains(c));
        if !self.cards.iter().any(|c| !exclude.contains(c)) {
            self.cards = pool.to_vec();
            rng.shuffle(&mut self.cards);
            let n = self.cards.len();
            if n > 1 && self.last.as_ref() == Some(&self.cards[0]) {
                self.cards.swap(0, rng.usize(1..n));
            }
        }
        let index = pick(&self.cards, avoid, exclude)?;
        Some(self.cards.remove(index))
    }

    fn draw_sequential(
        &mut self,
        pool: &[PathBuf],
        avoid: &[PathBuf],
        exclude: &[PathBuf],
    ) -> Option<PathBuf> {
        let start = self
            .last
            .as_ref()
            .and_then(|last| pool.iter().position(|p| p == last))
            .map_or(0, |i| i + 1);
        let ordered: Vec<PathBuf> = pool[start.min(pool.len())..]
            .iter()
            .chain(&pool[..start.min(pool.len())])
            .cloned()
            .collect();
        pick(&ordered, avoid, exclude).map(|i| ordered[i].clone())
    }
}

/// The first card neither avoided nor excluded, else the first not
/// excluded.
fn pick(cards: &[PathBuf], avoid: &[PathBuf], exclude: &[PathBuf]) -> Option<usize> {
    cards
        .iter()
        .position(|c| !exclude.contains(c) && !avoid.contains(c))
        .or_else(|| cards.iter().position(|c| !exclude.contains(c)))
}

/// Every deck and every Monitor's history.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Decks {
    pub decks: BTreeMap<String, Deck>,
    /// Monitor name to the wallpapers Rotation showed there, latest last.
    pub history: BTreeMap<String, Vec<PathBuf>>,
}

impl Decks {
    pub fn deck(&mut self, key: &str) -> &mut Deck {
        self.decks.entry(key.to_owned()).or_default()
    }

    /// Remembers that `path` went up on `monitor`.
    pub fn shown(&mut self, monitor: &str, path: &Path) {
        let history = self.history.entry(monitor.to_owned()).or_default();
        if history.last().map(PathBuf::as_path) != Some(path) {
            history.push(path.to_owned());
        }
        if history.len() > HISTORY {
            history.drain(..history.len() - HISTORY);
        }
    }

    /// The wallpaper `monitor` showed before `current`, which becomes the
    /// latest history entry. `None` when there is nothing to go back to.
    pub fn previous(&mut self, monitor: &str, current: Option<&Path>) -> Option<PathBuf> {
        let history = self.history.get_mut(monitor)?;
        while history.last().map(PathBuf::as_path) == current && current.is_some() {
            history.pop();
        }
        history.last().cloned()
    }

    /// Forgets the latest history entry of `monitor` (it failed to decode).
    pub fn drop_latest(&mut self, monitor: &str) {
        if let Some(history) = self.history.get_mut(monitor) {
            history.pop();
        }
    }

    fn parse(text: &str) -> Decks {
        let mut decks = Decks::default();
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return decks;
        }
        for line in lines {
            let mut fields = line.splitn(3, ' ');
            let (Some(kind), Some(key), Some(path)) = (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let path = PathBuf::from(path);
            match kind {
                "last" => decks.deck(key).last = Some(path),
                "card" => decks.deck(key).cards.push(path),
                "shown" => decks.history.entry(key.to_owned()).or_default().push(path),
                _ => {}
            }
        }
        decks
    }

    fn format(&self) -> String {
        let mut text = format!("{HEADER}\n");
        let mut line = |kind: &str, key: &str, path: &Path| {
            let path = path.to_string_lossy();
            if !path.contains('\n') {
                text += &format!("{kind} {key} {path}\n");
            }
        };
        for (key, deck) in &self.decks {
            if let Some(last) = &deck.last {
                line("last", key, last);
            }
            for card in &deck.cards {
                line("card", key, card);
            }
        }
        for (monitor, history) in &self.history {
            for path in history {
                line("shown", monitor, path);
            }
        }
        text
    }
}

/// The decks, locked against other `vellum` processes until dropped.
pub struct Locked {
    pub decks: Decks,
    path: Option<PathBuf>,
    _lock: Option<OwnedFd>,
}

impl Locked {
    /// Takes the lock (waiting for another holder) and reads the decks. A
    /// missing or damaged file is a fresh start.
    pub fn open() -> Locked {
        let path = crate::client::state_dir().map(|dir| dir.join("rotation"));
        let lock = path.as_deref().and_then(|path| {
            fs::create_dir_all(path.parent()?).ok()?;
            let fd = rustix::fs::open(
                path.with_extension("lock"),
                rustix::fs::OFlags::CREATE | rustix::fs::OFlags::RDWR | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::from_raw_mode(0o600),
            )
            .ok()?;
            rustix::fs::flock(&fd, rustix::fs::FlockOperation::LockExclusive).ok()?;
            Some(fd)
        });
        let decks = path
            .as_deref()
            .and_then(|p| fs::read_to_string(p).ok())
            .map(|text| Decks::parse(&text))
            .unwrap_or_default();
        Locked {
            decks,
            path,
            _lock: lock,
        }
    }

    /// Writes the decks atomically. A failure costs only the memory of
    /// where Rotation was, so it is a warning.
    pub fn save(&self) {
        let Some(path) = &self.path else { return };
        let tmp = path.with_extension("tmp");
        let result = fs::write(&tmp, self.decks.format()).and_then(|()| fs::rename(&tmp, path));
        if let Err(err) = result {
            eprintln!("vellum: cannot save {}: {err}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(n: usize) -> Vec<PathBuf> {
        (0..n)
            .map(|i| PathBuf::from(format!("/w/{i}.png")))
            .collect()
    }

    #[test]
    fn shuffle_shows_everything_once_per_round_and_never_repeats_at_the_seam() {
        let pool = pool(5);
        for seed in 0..200 {
            let mut rng = fastrand::Rng::with_seed(seed);
            let mut deck = Deck::default();
            let drawn: Vec<PathBuf> = (0..15)
                .map(|_| {
                    deck.draw(&pool, RotationOrder::Shuffle, &[], &[], &mut rng)
                        .unwrap()
                })
                .collect();
            for round in drawn.chunks(5) {
                let mut round = round.to_vec();
                round.sort();
                assert_eq!(round, pool, "seed {seed}");
            }
            for pair in drawn.windows(2) {
                assert_ne!(pair[0], pair[1], "seed {seed}");
            }
        }
    }

    #[test]
    fn a_shuffled_deck_survives_its_file() {
        let pool = pool(4);
        let mut rng = fastrand::Rng::with_seed(7);
        let mut decks = Decks::default();
        let first = decks
            .deck("DP-1")
            .draw(&pool, RotationOrder::Shuffle, &[], &[], &mut rng);
        decks.shown("DP-1", first.as_deref().unwrap());
        let mut reread = Decks::parse(&decks.format());
        assert_eq!(reread, decks);
        let rest: Vec<PathBuf> = (0..3)
            .map(|_| {
                reread
                    .deck("DP-1")
                    .draw(&pool, RotationOrder::Shuffle, &[], &[], &mut rng)
                    .unwrap()
            })
            .collect();
        let mut all: Vec<PathBuf> = rest.into_iter().chain(first).collect();
        all.sort();
        assert_eq!(all, pool);
    }

    #[test]
    fn sequential_continues_after_the_last_and_wraps() {
        let pool = pool(3);
        let mut rng = fastrand::Rng::with_seed(1);
        let mut deck = Deck {
            cards: vec![],
            last: Some(pool[1].clone()),
        };
        let mut next = || {
            deck.draw(&pool, RotationOrder::Sequential, &[], &[], &mut rng)
                .unwrap()
        };
        assert_eq!(next(), pool[2]);
        assert_eq!(next(), pool[0]);
        assert_eq!(next(), pool[1]);
    }

    #[test]
    fn excluded_cards_are_skipped_and_avoided_ones_only_as_a_last_resort() {
        let pool = pool(3);
        let mut rng = fastrand::Rng::with_seed(1);
        let mut deck = Deck::default();
        let got = deck.draw(
            &pool,
            RotationOrder::Sequential,
            &[pool[1].clone()],
            &[pool[0].clone()],
            &mut rng,
        );
        assert_eq!(got, Some(pool[2].clone()));
        let only_avoided = deck.draw(&pool[..1], RotationOrder::Sequential, &pool, &[], &mut rng);
        assert_eq!(only_avoided, Some(pool[0].clone()));
        assert_eq!(
            deck.draw(&pool, RotationOrder::Shuffle, &[], &pool, &mut rng),
            None
        );
    }

    #[test]
    fn previous_walks_back_through_history() {
        let p = pool(3);
        let mut decks = Decks::default();
        for path in &p {
            decks.shown("A", path);
        }
        assert_eq!(decks.previous("A", Some(&p[2])), Some(p[1].clone()));
        assert_eq!(decks.previous("A", Some(&p[1])), Some(p[0].clone()));
        assert_eq!(decks.previous("A", Some(&p[0])), None);
        // After a manual set, prev returns to the latest Rotation pick.
        decks.shown("B", &p[0]);
        assert_eq!(
            decks.previous("B", Some(Path::new("/elsewhere.png"))),
            Some(p[0].clone())
        );
    }
}
