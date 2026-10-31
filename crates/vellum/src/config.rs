//! The Config: the user's optional `config.toml`, parsed only here (ADR
//! 0003 keeps TOML out of the Daemon).
//!
//! [`Config::parse`] is pure: text in, values or a precise [`ConfigError`]
//! out. [`load`] is what commands call: it reads the file and, if the file
//! is malformed, falls back to the last good Config that `vellum render
//! --sync` saved, so a typo never changes what Vellum does.
//!
//! # Adding a section
//!
//! 1. Give the section a raw serde struct with `#[serde(default,
//!    deny_unknown_fields)]` (see [`TransitionSection`]) and a field on
//!    [`File`]. Top-level scalar keys go straight on [`File`]. Enum-like
//!    strings use `deserialize_with = "named"` plus a [`Named`] impl, so
//!    errors list the valid values.
//! 2. Add the resolved value to [`Config`], fill it in [`Config::defaults`]
//!    and [`Config::parse`].
//! 3. If the Daemon needs it (Rotation does), add it to
//!    `vellum_ipc::Settings` and to [`Config::settings`].
//! 4. Document the key in `docs/configuration.md`.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::{Deserializer, Error as _};
use serde::{Deserialize, de};
use std::collections::BTreeMap;
use toml::Spanned;

use vellum_ipc::{
    Direction, Easing, Override, RotationOrder, RotationSettings, RotationSource, Schedule,
    Settings, Transition, TransitionKind,
};

use crate::hook::Hook;
use crate::render::{Color, DEFAULT_LETTERBOX, FitMode};
use crate::tui::keymap::{self, Action, Remap};

/// The Library folder used when the Config names none.
pub const DEFAULT_LIBRARY: &str = "~/Pictures/Wallpapers";

/// Every Config value, defaults filled in.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Library folders, scanned recursively, `~` expanded.
    pub library: Vec<PathBuf>,
    /// The Fit mode `vellum set` uses without `--fit`.
    pub fit: FitMode,
    /// The bar color around a wallpaper in Fit mode.
    pub letterbox: Color,
    /// The Transition `vellum set` plays without flags.
    pub transition: Transition,
    /// The TUI's colors.
    pub theme: ThemeName,
    /// The Rotation schedule, Overrides already merged.
    pub rotation: RotationSettings,
    /// The command run after each new wallpaper is presented.
    pub on_change: Option<Hook>,
    /// `[keymap]`: TUI Actions whose keys the user replaced.
    pub keymap: Vec<Remap>,
    /// The most the TUI's on-disk preview cache may hold, in MiB. 0 turns
    /// the disk cache off.
    pub preview_cache_mb: u32,
}

/// The preview cache cap used when the Config sets none.
pub const DEFAULT_PREVIEW_CACHE_MB: u32 = 200;

/// `theme`: which palette the TUI uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThemeName {
    /// Midnight Bloom on truecolor terminals, ANSI colors elsewhere.
    #[default]
    Midnight,
    /// The terminal's own 16 colors and background.
    Terminal,
}

impl Config {
    /// The values used when there is no Config file. `home` expands `~`.
    pub fn defaults(home: Option<&Path>) -> Config {
        Config {
            library: expand(DEFAULT_LIBRARY, home).into_iter().collect(),
            fit: FitMode::default(),
            letterbox: DEFAULT_LETTERBOX,
            transition: Transition::default(),
            theme: ThemeName::default(),
            rotation: RotationSettings::default(),
            on_change: None,
            keymap: Vec::new(),
            preview_cache_mb: DEFAULT_PREVIEW_CACHE_MB,
        }
    }

    /// Parses Config text. Missing keys keep their defaults; unknown keys
    /// are errors, so a typo is never silently ignored.
    pub fn parse(text: &str, home: Option<&Path>) -> Result<Config, ConfigError> {
        let file: File = toml::from_str(text).map_err(|err| ConfigError::from_toml(text, &err))?;
        let mut config = Config::defaults(home);
        if let Some(folders) = file.library {
            config.library = folders
                .iter()
                .map(|folder| {
                    expand(folder.get_ref(), home).ok_or_else(|| {
                        ConfigError::at(
                            text,
                            folder.span(),
                            format!(
                                "`{}` is not a folder Vellum can find; \
                                 expected an absolute path or one starting with ~/",
                                folder.get_ref()
                            ),
                        )
                    })
                })
                .collect::<Result<_, _>>()?;
        }
        config.fit = file.fit.unwrap_or(config.fit);
        config.letterbox = file.letterbox.map_or(config.letterbox, |c| c.0);
        config.transition = file.transition.into();
        config.theme = file.theme.unwrap_or(config.theme);
        config.rotation = file.rotation.resolve();
        config.preview_cache_mb = file.preview.cache_mb;
        if let Some(command) = file.on_change {
            let hook = Hook::parse(command.get_ref(), home).map_err(|why| {
                ConfigError::at(text, command.span(), format!("the command {why}"))
            })?;
            config.on_change = Some(hook);
        }
        config.keymap = remaps(text, file.keymap)?;
        Ok(config)
    }

    /// The part of the Config the Daemon holds.
    pub fn settings(&self) -> Settings {
        Settings {
            transition: self.transition,
            rotation: self.rotation.clone(),
        }
    }
}

/// `[keymap]` entries, checked: known Action names, valid keys, and no
/// key given to two Actions that are active at the same time.
fn remaps(text: &str, section: BTreeMap<String, Spanned<Keys>>) -> Result<Vec<Remap>, ConfigError> {
    let mut remaps: Vec<Remap> = Vec::new();
    for (name, keys) in section {
        let span = keys.span();
        let action = Action::from_name(&name).ok_or_else(|| {
            let names: Vec<&str> = Action::names().collect();
            ConfigError::at(
                text,
                span.clone(),
                format!(
                    "unknown action `{name}`, expected one of {}",
                    names.join(", ")
                ),
            )
        })?;
        let list = match keys.into_inner() {
            Keys::One(key) => vec![key],
            Keys::Many(keys) => keys,
        };
        let chords = list
            .iter()
            .map(|key| keymap::parse_chord(key))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|why| ConfigError::at(text, span.clone(), why))?;
        for chord in &chords {
            if let Some(other) = remaps
                .iter()
                .find(|r| r.action.scope() == action.scope() && r.keys.contains(chord))
            {
                return Err(ConfigError::at(
                    text,
                    span,
                    format!(
                        "\"{}\" is already bound to `{}`; a key can do only one thing",
                        keymap::display_chord(chord),
                        other.action.name()
                    ),
                ));
            }
        }
        remaps.push(Remap {
            action,
            keys: chords,
        });
    }
    Ok(remaps)
}

/// A `[keymap]` value: one key or a list. An empty list unbinds.
#[derive(Deserialize)]
#[serde(untagged)]
enum Keys {
    One(String),
    Many(Vec<String>),
}

/// `~` and `~/...` against `home`; other paths must be absolute.
fn expand(path: &str, home: Option<&Path>) -> Option<PathBuf> {
    if path == "~" {
        return home.map(Path::to_path_buf);
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home.map(|home| home.join(rest));
    }
    Some(PathBuf::from(path)).filter(|p| p.is_absolute())
}

/// The file as written. Every field is optional.
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct File {
    library: Option<Vec<Spanned<String>>>,
    #[serde(deserialize_with = "named_opt")]
    fit: Option<FitMode>,
    letterbox: Option<Hex>,
    #[serde(deserialize_with = "named_opt")]
    theme: Option<ThemeName>,
    on_change: Option<Spanned<String>>,
    transition: TransitionSection,
    rotation: RotationSection,
    keymap: BTreeMap<String, Spanned<Keys>>,
    preview: PreviewSection,
}

/// `[preview]`: the TUI's preview cache.
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PreviewSection {
    /// The disk cache cap in MiB.
    cache_mb: u32,
}

impl Default for PreviewSection {
    fn default() -> Self {
        PreviewSection {
            cache_mb: DEFAULT_PREVIEW_CACHE_MB,
        }
    }
}

/// `[rotation]`: the global Rotation, plus `[rotation.monitor.<name>]`
/// Overrides.
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RotationSection {
    #[serde(deserialize_with = "interval")]
    interval: u64,
    #[serde(deserialize_with = "named")]
    source: RotationSource,
    #[serde(deserialize_with = "named")]
    order: RotationOrder,
    sync: bool,
    monitor: BTreeMap<String, OverrideSection>,
}

impl Default for RotationSection {
    fn default() -> Self {
        let s = Schedule::default();
        RotationSection {
            interval: s.interval_ms,
            source: s.source,
            order: s.order,
            sync: false,
            monitor: BTreeMap::new(),
        }
    }
}

/// `[rotation.monitor.<name>]`: what this Monitor does differently.
#[derive(Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
struct OverrideSection {
    #[serde(deserialize_with = "interval_opt")]
    interval: Option<u64>,
    #[serde(deserialize_with = "named_opt")]
    source: Option<RotationSource>,
    #[serde(deserialize_with = "named_opt")]
    order: Option<RotationOrder>,
}

impl RotationSection {
    fn resolve(self) -> RotationSettings {
        let schedule = Schedule {
            interval_ms: self.interval,
            source: self.source,
            order: self.order,
        };
        RotationSettings {
            schedule,
            sync: self.sync,
            overrides: self
                .monitor
                .into_iter()
                .map(|(monitor, o)| Override {
                    monitor,
                    schedule: Schedule {
                        interval_ms: o.interval.unwrap_or(schedule.interval_ms),
                        source: o.source.unwrap_or(schedule.source),
                        order: o.order.unwrap_or(schedule.order),
                    },
                })
                .collect(),
        }
    }
}

/// `[transition]`: the defaults `vellum set` plays.
#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct TransitionSection {
    #[serde(rename = "type", deserialize_with = "named")]
    kind: TransitionKind,
    #[serde(deserialize_with = "named")]
    direction: Direction,
    #[serde(deserialize_with = "millis")]
    duration: u32,
    #[serde(deserialize_with = "named")]
    easing: Easing,
}

impl Default for TransitionSection {
    fn default() -> Self {
        let t = Transition::default();
        TransitionSection {
            kind: t.kind,
            direction: t.direction,
            duration: t.duration_ms,
            easing: t.easing,
        }
    }
}

impl From<TransitionSection> for Transition {
    fn from(s: TransitionSection) -> Transition {
        Transition {
            kind: s.kind,
            direction: s.direction,
            duration_ms: s.duration,
            easing: s.easing,
        }
    }
}

/// A value spelled as one of a fixed set of names in the Config.
pub trait Named: Copy + PartialEq + 'static {
    /// What the value is, for error messages: "Fit mode".
    const WHAT: &'static str;
    /// Every name and its value. The first name of a value is canonical.
    const NAMES: &'static [(&'static str, Self)];

    /// The canonical name of `self`, as the Config spells it.
    fn name(self) -> &'static str {
        Self::NAMES
            .iter()
            .find(|(_, v)| *v == self)
            .map_or("?", |(n, _)| n)
    }
}

impl Named for FitMode {
    const WHAT: &'static str = "Fit mode";
    const NAMES: &'static [(&'static str, Self)] = &[
        ("fill", FitMode::Fill),
        ("fit", FitMode::Fit),
        ("center", FitMode::Center),
        ("stretch", FitMode::Stretch),
    ];
}

impl Named for ThemeName {
    const WHAT: &'static str = "theme";
    const NAMES: &'static [(&'static str, Self)] = &[
        ("midnight", ThemeName::Midnight),
        ("terminal", ThemeName::Terminal),
    ];
}

impl Named for TransitionKind {
    const WHAT: &'static str = "Transition";
    const NAMES: &'static [(&'static str, Self)] = &[
        ("fade", TransitionKind::Fade),
        ("wipe", TransitionKind::Wipe),
        ("slide", TransitionKind::Slide),
        ("none", TransitionKind::None),
    ];
}

impl Named for Direction {
    const WHAT: &'static str = "direction";
    const NAMES: &'static [(&'static str, Self)] = &[
        ("left", Direction::Left),
        ("right", Direction::Right),
        ("up", Direction::Up),
        ("down", Direction::Down),
    ];
}

impl Named for Easing {
    const WHAT: &'static str = "easing";
    const NAMES: &'static [(&'static str, Self)] = &[
        ("linear", Easing::Linear),
        ("ease-in", Easing::EaseIn),
        ("ease-out", Easing::EaseOut),
        ("ease-in-out", Easing::EaseInOut),
    ];
}

fn named<'de, D: Deserializer<'de>, T: Named>(d: D) -> Result<T, D::Error> {
    let name = String::deserialize(d)?;
    T::NAMES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| *v)
        .ok_or_else(|| {
            let valid: Vec<String> = T::NAMES.iter().map(|(n, _)| format!("\"{n}\"")).collect();
            D::Error::custom(format!(
                "unknown {} \"{name}\", expected one of {}",
                T::WHAT,
                valid.join(", ")
            ))
        })
}

impl Named for RotationSource {
    const WHAT: &'static str = "Rotation source";
    const NAMES: &'static [(&'static str, Self)] = &[
        ("library", RotationSource::Library),
        ("favorites", RotationSource::Favorites),
    ];
}

impl Named for RotationOrder {
    const WHAT: &'static str = "Rotation order";
    const NAMES: &'static [(&'static str, Self)] = &[
        ("shuffle", RotationOrder::Shuffle),
        ("sequential", RotationOrder::Sequential),
    ];
}

fn named_opt<'de, D: Deserializer<'de>, T: Named>(d: D) -> Result<Option<T>, D::Error> {
    named(d).map(Some)
}

/// A length in whole milliseconds.
fn millis<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    struct Millis;
    impl de::Visitor<'_> for Millis {
        type Value = u32;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a whole number of milliseconds, like 600")
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<u32, E> {
            u32::try_from(v).map_err(|_| E::invalid_value(de::Unexpected::Signed(v), &self))
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<u32, E> {
            u32::try_from(v).map_err(|_| E::invalid_value(de::Unexpected::Unsigned(v), &self))
        }
    }
    d.deserialize_u32(Millis)
}

/// A Rotation interval: a string with units (`"30m"`, `"1h30m"`, `"90s"`,
/// `"250ms"`) or a whole number of minutes. In milliseconds.
fn interval<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    struct Interval;
    const EXPECTED: &str = "an interval like \"30m\", \"1h30m\" or \"90s\", or whole minutes";
    impl de::Visitor<'_> for Interval {
        type Value = u64;
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(EXPECTED)
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<u64, E> {
            match u64::try_from(v) {
                Ok(minutes) if minutes > 0 => Ok(minutes.saturating_mul(60_000)),
                _ => Err(E::invalid_value(de::Unexpected::Signed(v), &self)),
            }
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<u64, E> {
            self.visit_i64(i64::try_from(v).unwrap_or(i64::MAX))
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<u64, E> {
            parse_interval(v)
                .filter(|&ms| ms > 0)
                .ok_or_else(|| E::custom(format!("\"{v}\" is not {EXPECTED}")))
        }
    }
    d.deserialize_any(Interval)
}

fn interval_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    interval(d).map(Some)
}

/// `"1h 30m"`, `"90s"`, `"250ms"` in milliseconds. Every number needs a
/// unit.
fn parse_interval(text: &str) -> Option<u64> {
    let mut rest = text.trim();
    if rest.is_empty() {
        return None;
    }
    let mut total: u64 = 0;
    while !rest.is_empty() {
        let digits = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        if digits == 0 {
            return None;
        }
        let n: u64 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let unit_len = rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(rest.len());
        let scale = match &rest[..unit_len] {
            "ms" => 1,
            "s" => 1000,
            "m" => 60_000,
            "h" => 3_600_000,
            _ => return None,
        };
        total = total.checked_add(n.checked_mul(scale)?)?;
        rest = rest[unit_len..].trim_start();
    }
    Some(total)
}

/// Milliseconds the way a Config spells them: "30m", "1h30m", "1.5s".
pub fn format_interval(ms: u64) -> String {
    if ms < 1000 {
        return format!("{ms}ms");
    }
    if ms < 60_000 && !ms.is_multiple_of(1000) {
        return format!("{}s", ms as f64 / 1000.0);
    }
    let secs = ms / 1000;
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    let mut out = String::new();
    for (n, unit) in [(h, "h"), (m, "m"), (s, "s")] {
        if n > 0 {
            out += &format!("{n}{unit}");
        }
    }
    out
}

/// A `"#rrggbb"` color.
struct Hex(Color);

impl<'de> Deserialize<'de> for Hex {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        parse_hex(&text).map(Hex).ok_or_else(|| {
            de::Error::custom(format!(
                "\"{text}\" is not a color, expected \"#rrggbb\" like \"#1a1b2e\""
            ))
        })
    }
}

fn parse_hex(text: &str) -> Option<Color> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

/// Why a Config was rejected: where, which key, and what was expected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    /// 1-based line, if known.
    pub line: Option<usize>,
    /// The dotted key the error is about, e.g. `transition.duration`.
    pub key: Option<String>,
    /// What is wrong and what was expected.
    pub message: String,
    /// The offending line with a caret under the problem.
    snippet: Option<String>,
}

impl ConfigError {
    fn from_toml(text: &str, err: &toml::de::Error) -> ConfigError {
        let message = err.message().trim().to_owned();
        match err.span() {
            Some(span) => ConfigError::at(text, span, message),
            None => ConfigError {
                line: None,
                key: None,
                message,
                snippet: None,
            },
        }
    }

    fn at(text: &str, span: std::ops::Range<usize>, message: String) -> ConfigError {
        let start = span.start.min(text.len());
        let line_start = text[..start].rfind('\n').map_or(0, |i| i + 1);
        let line_end = text[start..].find('\n').map_or(text.len(), |i| start + i);
        let line = text[..start].matches('\n').count() + 1;
        let source = &text[line_start..line_end];
        let column = text[line_start..start].chars().count();
        let width = text[start..span.end.clamp(start, line_end)]
            .chars()
            .count()
            .max(1);
        let gutter = format!("{line} | ");
        let snippet = format!(
            "{gutter}{source}\n{}{}",
            " ".repeat(gutter.len() + column),
            "^".repeat(width)
        );
        ConfigError {
            line: Some(line),
            key: key_at(text, line_start, line_end),
            message,
            snippet: Some(snippet),
        }
    }
}

/// The dotted key a line is about: its own key under the nearest table
/// header above it, or the header itself.
fn key_at(text: &str, line_start: usize, line_end: usize) -> Option<String> {
    let header = |line: &str| {
        let line = line.trim();
        line.strip_prefix('[')
            .map(|rest| rest.trim_start_matches('['))
            .and_then(|rest| rest.split(']').next())
            .map(|name| name.trim().to_owned())
    };
    let line = &text[line_start..line_end];
    if let Some(table) = header(line) {
        return Some(table);
    }
    let own = line
        .split_once('=')
        .map(|(key, _)| key.trim().trim_matches('"').to_owned())
        .filter(|key| !key.is_empty() && !key.starts_with('#'));
    let table = text[..line_start].lines().rev().find_map(header);
    match (table, own) {
        (Some(table), Some(own)) => Some(format!("{table}.{own}")),
        (None, Some(own)) => Some(own),
        (table, None) => table,
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, &self.key) {
            (Some(line), Some(key)) => write!(f, "line {line}, `{key}`: ")?,
            (Some(line), None) => write!(f, "line {line}: ")?,
            _ => {}
        }
        f.write_str(&self.message)?;
        if let Some(snippet) = &self.snippet {
            write!(f, "\n{snippet}")?;
        }
        Ok(())
    }
}

/// Where the Config file lives.
pub fn path() -> Option<PathBuf> {
    vellum_ipc::config_dir_from_env().map(|dir| dir.join("config.toml"))
}

/// Where `vellum render --sync` keeps the last good Config.
fn last_good_path() -> Option<PathBuf> {
    crate::client::state_dir().map(|dir| dir.join("config.last-good.toml"))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The Config file's text, or `None` if there is no Config.
fn read_text(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(format!("cannot read {}: {err}", path.display())),
    }
}

/// The Config as it is on disk right now, with no fallback, and its text
/// (`None` without a file). For `vellum reload` and `vellum render --sync`.
pub fn read_current() -> Result<(Config, Option<String>), String> {
    let home = home();
    let Some(path) = path() else {
        return Ok((Config::defaults(home.as_deref()), None));
    };
    let text = read_text(&path)?;
    let config = match &text {
        Some(text) => Config::parse(text, home.as_deref())
            .map_err(|err| format!("{}: {err}", path.display()))?,
        None => Config::defaults(home.as_deref()),
    };
    Ok((config, text))
}

/// The last good Config `vellum render --sync` saved, or the defaults.
pub fn last_good() -> Config {
    let home = home();
    last_good_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| Config::parse(&text, home.as_deref()).ok())
        .unwrap_or_else(|| Config::defaults(home.as_deref()))
}

/// Records `text` (`None`: no Config file) as the last good Config.
pub fn save_last_good(text: Option<&str>) -> Result<(), String> {
    let Some(path) = last_good_path() else {
        return Ok(());
    };
    let result = match text {
        None => match std::fs::remove_file(&path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
        Some(text) => (|| {
            std::fs::create_dir_all(path.parent().expect("state dir"))?;
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, text)?;
            std::fs::rename(&tmp, &path)
        })(),
    };
    result.map_err(|err| format!("cannot save {}: {err}", path.display()))
}

/// The Config a command should use. A malformed Config is reported on
/// stderr and the last good one is used instead.
/// It also arms the `on_change` hook for this process.
pub fn load() -> Config {
    let config = match read_current() {
        Ok((config, _)) => config,
        Err(err) => {
            eprintln!("vellum: {err}\nvellum: using the last good Config until this is fixed");
            last_good()
        }
    };
    crate::hook::remember(config.on_change.as_ref());
    config
}

/// Writing the Config from the TUI, keeping the user's formatting.
pub mod edit;

#[cfg(test)]
mod tests;
