//! The Action registry: every key binding, in one table.
//!
//! The same [`Keymap`] decides what a key does ([`Keymap::lookup`]),
//! writes the footer hints ([`footer`]), the `?` help overlay
//! ([`help_rows`]) and the command palette ([`palette_rows`]). Config
//! `[keymap]` entries replace an Action's default keys, so a remap shows
//! up everywhere at once.
//!
//! Adding an Action: a variant, a row in [`ACTIONS`] (Config name, scope,
//! help text, default keys), an arm in `update::perform`, and, if it
//! deserves a footer hint, a place in [`footer`].
//!
//! A binding may be a sequence (`"g g"`). A key that is both bound on its
//! own and the start of a sequence waits [`CHORD_TIMEOUT`] for the rest
//! (see `update`), like Vim's `timeoutlen`.

use std::time::Duration;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::model::Mode;

/// How long a key that starts a sequence waits for the next key before it
/// counts on its own.
pub const CHORD_TIMEOUT: Duration = Duration::from_millis(400);

/// Something the user can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Action {
    Down,
    Up,
    /// The tile to the left, in the grid. Nothing in the list.
    Left,
    /// The tile to the right, in the grid. Nothing in the list.
    Right,
    /// Switch between the list and the thumbnail grid.
    ToggleGrid,
    Top,
    Bottom,
    PageDown,
    PageUp,
    /// Apply the selection to the targeted Monitor.
    Apply,
    /// Apply the selection to every Monitor.
    ApplyAll,
    /// Apply a random wallpaper from the list to the targeted Monitor.
    Random,
    /// Target the next Monitor.
    Monitor,
    /// Toggle the selection's Favorite.
    Favorite,
    /// Show only Favorites, or everything again.
    FavoritesOnly,
    Search,
    Help,
    Palette,
    /// Clear the search and Favorites filter, else quit.
    Back,
    Quit,
    /// In a prompt (search, help, palette): the next row.
    Next,
    Prev,
    /// In a prompt: keep the search, run the command, close help.
    Confirm,
    /// In a prompt: clear and close.
    Cancel,
    /// On the welcome screen: complete the folder name typed so far.
    Complete,
    /// Open the Rotation panel.
    Rotation,
    /// Choose a Library folder (the welcome screen).
    Library,
    /// On a narrow terminal: show the preview instead of the list, or back.
    TogglePreview,
    /// Ask the Daemon again, starting it if needed.
    Reconnect,
    /// In the Rotation panel: the next or previous setting.
    FieldDown,
    FieldUp,
    /// In the Rotation panel: the setting's previous or next value.
    Decrease,
    Increase,
    /// In the Rotation panel: start or stop Rotation, or change the value.
    Toggle,
    /// In the Rotation panel: drop the Monitor's Override.
    RemoveOverride,
    /// Close the Rotation panel.
    Close,
}

/// Which keymap an Action lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Browsing the list.
    Browse,
    /// Typing in search, help, the palette or the welcome screen. Unbound
    /// printable keys are typed into the prompt.
    Prompt,
    /// The Rotation panel.
    Panel,
}

impl Scope {
    pub fn of(mode: Mode) -> Scope {
        match mode {
            Mode::Normal => Scope::Browse,
            Mode::Search | Mode::Help | Mode::Palette | Mode::Welcome => Scope::Prompt,
            Mode::Rotation => Scope::Panel,
        }
    }
}

struct Info {
    action: Action,
    /// The Config spelling, under `[keymap]`.
    name: &'static str,
    scope: Scope,
    /// What the help overlay and the palette say.
    about: &'static str,
    defaults: &'static [&'static str],
}

const fn info(
    action: Action,
    name: &'static str,
    scope: Scope,
    about: &'static str,
    defaults: &'static [&'static str],
) -> Info {
    Info {
        action,
        name,
        scope,
        about,
        defaults,
    }
}

use Scope::{Browse, Panel, Prompt};

/// Every Action, in help and palette order.
const ACTIONS: &[Info] = &[
    info(Action::Down, "down", Browse, "Move down", &["j", "down"]),
    info(Action::Up, "up", Browse, "Move up", &["k", "up"]),
    info(
        Action::Left,
        "left",
        Browse,
        "Move left in the grid",
        &["h", "left"],
    ),
    info(
        Action::Right,
        "right",
        Browse,
        "Move right in the grid",
        &["l", "right"],
    ),
    info(
        Action::ToggleGrid,
        "grid",
        Browse,
        "Switch between list and grid",
        &["g"],
    ),
    info(
        Action::Top,
        "top",
        Browse,
        "Jump to the top",
        &["g g", "home"],
    ),
    info(
        Action::Bottom,
        "bottom",
        Browse,
        "Jump to the bottom",
        &["G", "end"],
    ),
    info(
        Action::PageDown,
        "page_down",
        Browse,
        "Half a page down",
        &["ctrl-d", "pagedown"],
    ),
    info(
        Action::PageUp,
        "page_up",
        Browse,
        "Half a page up",
        &["ctrl-u", "pageup"],
    ),
    info(
        Action::Apply,
        "apply",
        Browse,
        "Apply to the targeted monitor",
        &["enter"],
    ),
    info(
        Action::ApplyAll,
        "apply_all",
        Browse,
        "Apply to every monitor",
        &["a"],
    ),
    info(
        Action::Random,
        "random",
        Browse,
        "Apply a random wallpaper",
        &["r"],
    ),
    info(
        Action::Monitor,
        "monitor",
        Browse,
        "Target the next monitor",
        &["m"],
    ),
    info(
        Action::Favorite,
        "favorite",
        Browse,
        "Favorite or unfavorite",
        &["f"],
    ),
    info(
        Action::FavoritesOnly,
        "favorites_only",
        Browse,
        "Show only Favorites",
        &["F"],
    ),
    info(
        Action::Search,
        "search",
        Browse,
        "Search the Library",
        &["/"],
    ),
    info(
        Action::Rotation,
        "rotation",
        Browse,
        "Open the Rotation panel",
        &["s"],
    ),
    info(
        Action::Library,
        "library",
        Browse,
        "Choose a Library folder",
        &["o"],
    ),
    info(
        Action::TogglePreview,
        "preview",
        Browse,
        "Show the preview or the list (narrow terminals)",
        &["p"],
    ),
    info(
        Action::Reconnect,
        "reconnect",
        Browse,
        "Reconnect to vellumd, starting it if needed",
        &["R"],
    ),
    info(Action::Help, "help", Browse, "Show every key", &["?"]),
    info(
        Action::Palette,
        "palette",
        Browse,
        "Open the command palette",
        &[":", "ctrl-p"],
    ),
    info(
        Action::Back,
        "back",
        Browse,
        "Clear the filter, or quit",
        &["esc"],
    ),
    info(Action::Quit, "quit", Browse, "Quit", &["q"]),
    info(
        Action::Next,
        "next",
        Prompt,
        "Next row",
        &["down", "ctrl-n"],
    ),
    info(
        Action::Prev,
        "prev",
        Prompt,
        "Previous row",
        &["up", "ctrl-p"],
    ),
    info(
        Action::Confirm,
        "confirm",
        Prompt,
        "Keep the search or run the command",
        &["enter"],
    ),
    info(
        Action::Cancel,
        "cancel",
        Prompt,
        "Clear and close",
        &["esc"],
    ),
    info(
        Action::Complete,
        "complete",
        Prompt,
        "Complete a folder name",
        &["tab"],
    ),
    info(
        Action::FieldDown,
        "panel_down",
        Panel,
        "Next setting",
        &["j", "down", "tab"],
    ),
    info(
        Action::FieldUp,
        "panel_up",
        Panel,
        "Previous setting",
        &["k", "up", "shift-tab"],
    ),
    info(
        Action::Decrease,
        "decrease",
        Panel,
        "Previous value",
        &["h", "left", "-"],
    ),
    info(
        Action::Increase,
        "increase",
        Panel,
        "Next value",
        &["l", "right", "+"],
    ),
    info(
        Action::Toggle,
        "toggle",
        Panel,
        "Start or stop Rotation, or change the setting",
        &["space", "enter"],
    ),
    info(
        Action::RemoveOverride,
        "remove_override",
        Panel,
        "Use the global Rotation on this monitor",
        &["x", "delete"],
    ),
    info(
        Action::Close,
        "close",
        Panel,
        "Close the panel",
        &["esc", "s", "q"],
    ),
];

impl Action {
    fn info(self) -> &'static Info {
        ACTIONS
            .iter()
            .find(|i| i.action == self)
            .expect("every Action has a row in ACTIONS")
    }

    /// The Config spelling.
    pub fn name(self) -> &'static str {
        self.info().name
    }

    pub fn scope(self) -> Scope {
        self.info().scope
    }

    pub fn from_name(name: &str) -> Option<Action> {
        ACTIONS.iter().find(|i| i.name == name).map(|i| i.action)
    }

    /// Every Config name, for error messages.
    pub fn names() -> impl Iterator<Item = &'static str> {
        ACTIONS.iter().map(|i| i.name)
    }
}

/// One key press, normalized: Shift is folded into the character, only
/// Ctrl and Alt are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

/// A binding: one key, or a sequence like `g g`.
pub type Chord = Vec<Key>;

impl Key {
    pub fn from_event(event: KeyEvent) -> Key {
        let mut mods = event.modifiers & (KeyModifiers::CONTROL | KeyModifiers::ALT);
        let code = match event.code {
            // Some terminals report Shift-Tab as Tab plus Shift.
            KeyCode::Tab if event.modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
            KeyCode::Char(c) if mods.contains(KeyModifiers::CONTROL) => {
                // Ctrl-D and Ctrl-d are the same key to a terminal.
                KeyCode::Char(c.to_ascii_lowercase())
            }
            code => code,
        };
        if code == KeyCode::Null {
            mods = KeyModifiers::NONE;
        }
        Key { code, mods }
    }

    /// `"j"`, `"G"`, `"enter"`, `"ctrl-d"`, `"alt-x"`, `"f5"`, `"space"`.
    pub fn parse(text: &str) -> Result<Key, String> {
        let mut mods = KeyModifiers::NONE;
        let mut rest = text;
        loop {
            if let Some(after) = rest.strip_prefix("ctrl-").filter(|a| !a.is_empty()) {
                mods |= KeyModifiers::CONTROL;
                rest = after;
            } else if let Some(after) = rest.strip_prefix("alt-").filter(|a| !a.is_empty()) {
                mods |= KeyModifiers::ALT;
                rest = after;
            } else {
                break;
            }
        }
        let mut chars = rest.chars();
        let code = match (chars.next(), chars.next()) {
            (Some(c), None) if !c.is_whitespace() => {
                if mods.contains(KeyModifiers::CONTROL) {
                    KeyCode::Char(c.to_ascii_lowercase())
                } else {
                    KeyCode::Char(c)
                }
            }
            _ => match rest {
                "enter" | "return" => KeyCode::Enter,
                "esc" | "escape" => KeyCode::Esc,
                "tab" => KeyCode::Tab,
                "backtab" | "shift-tab" => KeyCode::BackTab,
                "backspace" => KeyCode::Backspace,
                "delete" | "del" => KeyCode::Delete,
                "insert" => KeyCode::Insert,
                "space" => KeyCode::Char(' '),
                "up" => KeyCode::Up,
                "down" => KeyCode::Down,
                "left" => KeyCode::Left,
                "right" => KeyCode::Right,
                "home" => KeyCode::Home,
                "end" => KeyCode::End,
                "pageup" => KeyCode::PageUp,
                "pagedown" => KeyCode::PageDown,
                f if f.starts_with('f')
                    && f[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) =>
                {
                    KeyCode::F(f[1..].parse().unwrap_or(1))
                }
                _ => {
                    return Err(format!(
                        "\"{text}\" is not a key; write a character like \"j\" or \"G\", \
                         a name like \"enter\", \"esc\", \"space\", \"up\" or \"f5\", \
                         optionally after \"ctrl-\" or \"alt-\""
                    ));
                }
            },
        };
        Ok(Key { code, mods })
    }

    /// How the footer, help and palette print the key.
    pub fn display(&self) -> String {
        let name = match self.code {
            KeyCode::Enter => "⏎".to_owned(),
            KeyCode::Esc => "esc".into(),
            KeyCode::Tab => "tab".into(),
            KeyCode::BackTab => "shift-tab".into(),
            KeyCode::Backspace => "⌫".into(),
            KeyCode::Delete => "del".into(),
            KeyCode::Insert => "ins".into(),
            KeyCode::Char(' ') => "space".into(),
            KeyCode::Char(c) => c.to_string(),
            KeyCode::Up => "↑".into(),
            KeyCode::Down => "↓".into(),
            KeyCode::Left => "←".into(),
            KeyCode::Right => "→".into(),
            KeyCode::Home => "home".into(),
            KeyCode::End => "end".into(),
            KeyCode::PageUp => "pgup".into(),
            KeyCode::PageDown => "pgdn".into(),
            KeyCode::F(n) => format!("f{n}"),
            other => format!("{other:?}").to_lowercase(),
        };
        let mut out = String::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            out += "ctrl-";
        }
        if self.mods.contains(KeyModifiers::ALT) {
            out += "alt-";
        }
        out + &name
    }
}

/// `"g g"` or `"ctrl-d"`: keys separated by spaces.
pub fn parse_chord(text: &str) -> Result<Chord, String> {
    if text.trim() == "space" || text == " " {
        return Ok(vec![Key::parse("space")?]);
    }
    let keys: Chord = text
        .split_whitespace()
        .map(Key::parse)
        .collect::<Result<_, _>>()?;
    if keys.is_empty() {
        return Err("a key cannot be empty; name one like \"j\" or \"ctrl-d\"".into());
    }
    Ok(keys)
}

/// `gg`, `ctrl-d`, `⏎`.
pub fn display_chord(chord: &[Key]) -> String {
    let parts: Vec<String> = chord.iter().map(Key::display).collect();
    if chord
        .iter()
        .all(|k| matches!(k.code, KeyCode::Char(_)) && k.mods.is_empty())
    {
        parts.concat()
    } else {
        parts.join(" ")
    }
}

/// A Config `[keymap]` entry: these keys, and only these, do `action`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remap {
    pub action: Action,
    pub keys: Vec<Chord>,
}

/// What a key sequence means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    Action(Action),
    /// The start of a longer binding. `alone` is what the keys do if
    /// nothing follows in time.
    Prefix {
        alone: Option<Action>,
    },
    Unbound,
}

/// The live bindings: defaults with the Config's remaps applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    /// In [`ACTIONS`] order, each Action's keys in the order given.
    bindings: Vec<(Action, Chord)>,
}

impl Default for Keymap {
    fn default() -> Keymap {
        Keymap::new(&[])
    }
}

impl Keymap {
    /// The defaults, with each remapped Action's keys replaced. A key a
    /// remap claims is taken away from any other Action in the same scope,
    /// so `apply_all = "r"` leaves `random` without `r`.
    pub fn new(remaps: &[Remap]) -> Keymap {
        let claimed = |scope: Scope, chord: &Chord| {
            remaps
                .iter()
                .any(|r| r.action.scope() == scope && r.keys.contains(chord))
        };
        let mut bindings = Vec::new();
        for info in ACTIONS {
            match remaps.iter().find(|r| r.action == info.action) {
                Some(remap) => {
                    bindings.extend(remap.keys.iter().map(|k| (info.action, k.clone())));
                }
                None => {
                    for text in info.defaults {
                        let chord = parse_chord(text).expect("default keys parse");
                        if !claimed(info.scope, &chord) {
                            bindings.push((info.action, chord));
                        }
                    }
                }
            }
        }
        Keymap { bindings }
    }

    /// What `keys` (the keys pressed so far) mean in `scope`.
    pub fn lookup(&self, scope: Scope, keys: &[Key]) -> Lookup {
        let mut exact = None;
        let mut longer = false;
        for (action, chord) in self.in_scope(scope) {
            if chord.as_slice() == keys {
                exact = exact.or(Some(*action));
            } else if chord.len() > keys.len() && chord.starts_with(keys) {
                longer = true;
            }
        }
        match (exact, longer) {
            (alone, true) => Lookup::Prefix { alone },
            (Some(action), false) => Lookup::Action(action),
            (None, false) => Lookup::Unbound,
        }
    }

    fn in_scope(&self, scope: Scope) -> impl Iterator<Item = &(Action, Chord)> {
        self.bindings
            .iter()
            .filter(move |(a, _)| a.scope() == scope)
    }

    /// Every chord bound to `action`, in order.
    pub fn chords(&self, action: Action) -> impl Iterator<Item = &Chord> {
        self.bindings
            .iter()
            .filter(move |(a, _)| *a == action)
            .map(|(_, c)| c)
    }

    /// The first key of `action`, as printed, or `None` if it is unbound.
    pub fn primary(&self, action: Action) -> Option<String> {
        self.chords(action).next().map(|c| display_chord(c))
    }

    /// Every key of `action`, as printed: `j, ↓`.
    pub fn all_keys(&self, action: Action) -> String {
        let keys: Vec<String> = self.chords(action).map(|c| display_chord(c)).collect();
        keys.join(", ")
    }
}

/// A footer hint, laid out: the cells it covers and what a click does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub x: u16,
    pub width: u16,
    pub keys: String,
    pub label: &'static str,
    pub action: Action,
}

/// What the footer depends on besides the mode.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Context {
    /// A search or the Favorites filter narrows the list.
    pub filtered: bool,
    /// The thumbnail grid is on.
    pub grid: bool,
    /// The terminal is narrow: the list and the preview take turns.
    pub narrow: bool,
    /// On a narrow terminal, the preview is shown instead of the list.
    pub preview_only: bool,
    /// The Daemon could not be reached.
    pub daemon_down: bool,
    /// The Rotation panel edits a Monitor's Override.
    pub overriding: bool,
}

/// What the footer offers in a mode, as (Actions joined into one hint,
/// label, priority). Lower priority numbers survive a narrow footer.
fn footer_items(mode: Mode, ctx: Context) -> Vec<(&'static [Action], &'static str, u8)> {
    let Context { filtered, grid, .. } = ctx;
    match mode {
        Mode::Normal => {
            let (moves, other): (&'static [Action], _) = match grid {
                false => (&[Action::Down, Action::Up], "grid"),
                true => (
                    &[Action::Left, Action::Down, Action::Up, Action::Right],
                    "list",
                ),
            };
            let mut items: Vec<(&'static [Action], &'static str, u8)> = vec![
                (moves, "move", 1),
                (&[Action::ToggleGrid], other, 6),
                (&[Action::Apply], "apply", 2),
                (&[Action::Search], "search", 3),
                (&[Action::Favorite], "favorite", 5),
                (&[Action::FavoritesOnly], "favorites", 9),
                (&[Action::ApplyAll], "all", 7),
                (&[Action::Random], "random", 8),
                (&[Action::Monitor], "monitor", 10),
                (&[Action::Rotation], "rotation", 8),
                (&[Action::Palette], "commands", 6),
                (&[Action::Help], "help", 0),
                (&[Action::Quit], "quit", 4),
            ];
            if ctx.narrow && !grid {
                let label = if ctx.preview_only { "list" } else { "preview" };
                items.insert(1, (&[Action::TogglePreview], label, 1));
            }
            if filtered {
                items.insert(1, (&[Action::Back], "clear", 1));
            }
            if ctx.daemon_down {
                items.insert(0, (&[Action::Reconnect], "reconnect", 0));
            }
            items
        }
        Mode::Rotation => {
            let mut items: Vec<(&'static [Action], &'static str, u8)> = vec![
                (&[Action::FieldDown, Action::FieldUp], "move", 1),
                (&[Action::Decrease, Action::Increase], "change", 0),
                (&[Action::Toggle], "toggle", 2),
                (&[Action::Close], "close", 0),
            ];
            if ctx.overriding {
                items.push((&[Action::RemoveOverride], "use global", 3));
            }
            items
        }
        Mode::Welcome => vec![
            (&[Action::Confirm], "choose", 0),
            (&[Action::Next, Action::Prev], "move", 1),
            (&[Action::Complete], "complete", 1),
            (&[Action::Cancel], "not now", 2),
        ],
        Mode::Search => vec![
            (&[Action::Confirm], "keep", 0),
            (&[Action::Cancel], "clear", 0),
            (&[Action::Next, Action::Prev], "move", 1),
        ],
        Mode::Help => vec![
            (&[Action::Cancel], "close", 0),
            (&[Action::Next, Action::Prev], "scroll", 1),
        ],
        Mode::Palette => vec![
            (&[Action::Confirm], "run", 0),
            (&[Action::Cancel], "close", 0),
            (&[Action::Next, Action::Prev], "select", 1),
        ],
    }
}

/// Gap between hints.
const GAP: u16 = 2;

/// The footer hints that fit in `width` columns, laid out from column 1.
/// Hints keep their order; when space runs out the least important go.
/// An Action with no key gets no hint.
pub fn footer(keymap: &Keymap, mode: Mode, ctx: Context, width: u16) -> Vec<Hint> {
    let items: Vec<(Action, String, &'static str, u8)> = footer_items(mode, ctx)
        .into_iter()
        .filter_map(|(actions, label, priority)| {
            let keys: Vec<String> = actions.iter().filter_map(|a| keymap.primary(*a)).collect();
            (!keys.is_empty()).then(|| (actions[0], keys.join("/"), label, priority))
        })
        .collect();
    let cost = |keys: &str, label: &str| {
        u16::try_from(keys.chars().count() + 1 + label.chars().count()).unwrap_or(u16::MAX) + GAP
    };
    let mut by_priority: Vec<usize> = (0..items.len()).collect();
    by_priority.sort_by_key(|&i| items[i].3);
    let mut keep = vec![false; items.len()];
    let mut used = 1;
    for i in by_priority {
        let c = cost(&items[i].1, items[i].2);
        if used + c <= width.saturating_add(GAP) {
            used += c;
            keep[i] = true;
        }
    }
    let mut x = 1;
    let mut hints = Vec::new();
    for (i, (action, keys, label, _)) in items.into_iter().enumerate() {
        if !keep[i] {
            continue;
        }
        let width = cost(&keys, label) - GAP;
        hints.push(Hint {
            x,
            width,
            keys,
            label,
            action,
        });
        x += width + GAP;
    }
    hints
}

/// A row of the help overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelpRow {
    Section(&'static str),
    Entry { keys: String, about: String },
}

/// Mouse gestures, listed in help.
const MOUSE: &[(&str, &str)] = &[
    ("click", "Select a wallpaper, monitor or hint"),
    ("double-click", "Apply to the targeted monitor"),
    ("wheel", "Scroll"),
];

/// The help overlay, generated from the live keymap and filtered by
/// `query` (fuzzy, over keys and text). Sections with no match are left
/// out.
pub fn help_rows(keymap: &Keymap, query: &str) -> Vec<HelpRow> {
    let mut sections: Vec<(&'static str, Vec<(String, String)>)> = vec![
        ("Browse", Vec::new()),
        ("Search, help, palette and welcome", Vec::new()),
        ("Rotation panel", Vec::new()),
        ("Mouse", Vec::new()),
    ];
    for info in ACTIONS {
        let keys = keymap.all_keys(info.action);
        let section = match info.scope {
            Scope::Browse => 0,
            Scope::Prompt => 1,
            Scope::Panel => 2,
        };
        sections[section].1.push((
            if keys.is_empty() {
                "unbound".into()
            } else {
                keys
            },
            info.about.into(),
        ));
    }
    sections[1]
        .1
        .push(("ctrl-c".into(), "Quit, from anywhere".into()));
    sections[3].1.extend(
        MOUSE
            .iter()
            .map(|(k, a)| ((*k).to_owned(), (*a).to_owned())),
    );

    let mut rows = Vec::new();
    for (title, entries) in sections {
        let texts: Vec<String> = entries.iter().map(|(k, a)| format!("{k} {a}")).collect();
        let matched = super::search::rank(query, texts.iter().map(String::as_str));
        if matched.is_empty() {
            continue;
        }
        rows.push(HelpRow::Section(title));
        // Help keeps its own order; the query only filters.
        let mut keep: Vec<usize> = matched.into_iter().map(|m| m.index).collect();
        keep.sort_unstable();
        rows.extend(keep.into_iter().map(|i| {
            let (keys, about) = entries[i].clone();
            HelpRow::Entry { keys, about }
        }));
    }
    rows
}

/// A palette row: an Action by name, with its keys and the matched
/// characters of `about`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaletteRow {
    pub action: Action,
    pub about: &'static str,
    pub keys: String,
    pub matched: Vec<u32>,
}

/// Every Action the palette can run, best match for `query` first.
pub fn palette_rows(keymap: &Keymap, query: &str) -> Vec<PaletteRow> {
    let runnable: Vec<&Info> = ACTIONS
        .iter()
        .filter(|i| i.scope == Scope::Browse && i.action != Action::Palette)
        .collect();
    super::search::rank(query, runnable.iter().map(|i| i.about))
        .into_iter()
        .map(|m| {
            let info = runnable[m.index];
            PaletteRow {
                action: info.action,
                about: info.about,
                keys: keymap.all_keys(info.action),
                matched: m.matched,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> Chord {
        parse_chord(text).unwrap()
    }

    #[test]
    fn keys_parse_and_print_back() {
        for (text, shown) in [
            ("j", "j"),
            ("G", "G"),
            ("ctrl-d", "ctrl-d"),
            ("ctrl-D", "ctrl-d"),
            ("alt-x", "alt-x"),
            ("enter", "⏎"),
            ("space", "space"),
            ("f5", "f5"),
            ("-", "-"),
            ("g g", "gg"),
        ] {
            assert_eq!(display_chord(&chord(text)), shown, "{text}");
        }
        for bad in ["", "ctrl-", "enterr", "f99", "jk"] {
            assert!(parse_chord(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_sequence_start_waits_and_knows_what_it_does_alone() {
        let keymap = Keymap::default();
        let g = chord("g");
        assert_eq!(
            keymap.lookup(Scope::Browse, &g),
            Lookup::Prefix {
                alone: Some(Action::ToggleGrid)
            }
        );
        assert_eq!(
            keymap.lookup(Scope::Browse, &chord("g g")),
            Lookup::Action(Action::Top)
        );
        let remapped = Keymap::new(&[Remap {
            action: Action::Random,
            keys: vec![g.clone()],
        }]);
        assert_eq!(
            remapped.lookup(Scope::Browse, &g),
            Lookup::Prefix {
                alone: Some(Action::Random)
            }
        );
    }

    #[test]
    fn a_remap_replaces_defaults_and_steals_its_keys() {
        let keymap = Keymap::new(&[Remap {
            action: Action::ApplyAll,
            keys: vec![chord("r")],
        }]);
        assert_eq!(
            keymap.lookup(Scope::Browse, &chord("r")),
            Lookup::Action(Action::ApplyAll)
        );
        assert_eq!(keymap.lookup(Scope::Browse, &chord("a")), Lookup::Unbound);
        assert_eq!(keymap.primary(Action::Random), None);
        // Prompt keys are untouched.
        assert_eq!(
            keymap.lookup(Scope::Prompt, &chord("enter")),
            Lookup::Action(Action::Confirm)
        );
    }

    #[test]
    fn a_narrow_footer_keeps_help_and_the_most_important_hints() {
        let keymap = Keymap::default();
        let wide = footer(&keymap, Mode::Normal, Context::default(), 200);
        assert_eq!(wide.len(), 13);
        let grid = Context {
            grid: true,
            ..Context::default()
        };
        let grid = footer(&keymap, Mode::Normal, grid, 200);
        assert_eq!((grid[0].keys.as_str(), grid[1].label), ("h/j/k/l", "list"));
        let narrow = footer(&keymap, Mode::Normal, Context::default(), 40);
        let labels: Vec<&str> = narrow.iter().map(|h| h.label).collect();
        assert_eq!(labels, ["move", "apply", "search", "help"]);
        let last = narrow.last().unwrap();
        assert!(last.x + last.width <= 40);
    }
}
