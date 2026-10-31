//! Writing the Config from the TUI without trampling the user's file.
//!
//! The Config is hand-edited and often lives in dotfiles, so a write
//! changes only the keys it names: comments, blank lines, key order and
//! formatting elsewhere survive ([`toml_edit`] keeps them). A write that
//! would leave the Config invalid is refused, and a symlinked Config is
//! written through to its target.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value as TomlValue};

use super::{Config, home, path, read_text};

/// One change to the Config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// `key = value` in the table at `table` (empty: the top level),
    /// creating the table if needed.
    Set {
        table: Vec<String>,
        key: String,
        value: Value,
    },
    /// Removes the table at `table` and everything in it.
    Remove { table: Vec<String> },
}

/// A Config value the TUI writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Str(String),
    List(Vec<String>),
}

impl Edit {
    /// `key = "value"` in `table`.
    pub fn set(table: &[&str], key: &str, value: impl Into<String>) -> Edit {
        Edit::Set {
            table: table.iter().map(|s| (*s).to_owned()).collect(),
            key: key.to_owned(),
            value: Value::Str(value.into()),
        }
    }

    /// The table this edit is about and, for a Set, its key: two edits
    /// with the same target replace each other.
    pub fn target(&self) -> (&[String], Option<&str>) {
        match self {
            Edit::Set { table, key, .. } => (table, Some(key)),
            Edit::Remove { table } => (table, None),
        }
    }
}

/// `text` with `edits` applied in order. Everything the edits do not name
/// is kept byte for byte.
pub fn apply(text: &str, edits: &[Edit]) -> Result<String, String> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|err| format!("cannot edit the Config, it is not valid TOML: {err}"))?;
    for edit in edits {
        match edit {
            Edit::Set { table, key, value } => {
                // In a file of only comments they are the document's
                // trailing text; keep them above the first key.
                let comments = (doc.as_table().is_empty() && table.is_empty())
                    .then(|| doc.trailing().as_str().unwrap_or_default().to_owned())
                    .filter(|c| !c.is_empty());
                let parent = table_at(doc.as_table_mut(), table)?;
                set(parent, key, value);
                if let Some(comments) = comments {
                    if let Some(mut k) = doc.as_table_mut().key_mut(key) {
                        k.leaf_decor_mut().set_prefix(comments);
                    }
                    doc.set_trailing("");
                }
            }
            Edit::Remove { table } => {
                let Some((last, parents)) = table.split_last() else {
                    continue;
                };
                if let Some(parent) = existing_table(doc.as_table_mut(), parents) {
                    parent.remove(last);
                }
            }
        }
    }
    Ok(doc.to_string())
}

/// The table at `path` under `root`, created (implicit, so no empty
/// `[a]` header appears for `[a.b]`) where missing.
fn table_at<'a>(root: &'a mut Table, path: &[String]) -> Result<&'a mut dyn TableLike, String> {
    let mut current: &mut dyn TableLike = root;
    for (i, name) in path.iter().enumerate() {
        let last = i + 1 == path.len();
        let item = current.entry(name).or_insert_with(|| {
            let mut table = Table::new();
            table.set_implicit(!last);
            Item::Table(table)
        });
        current = item.as_table_like_mut().ok_or_else(|| {
            format!(
                "cannot edit the Config: `{}` is not a table",
                path[..=i].join(".")
            )
        })?;
    }
    Ok(current)
}

/// The table at `path`, if it exists.
fn existing_table<'a>(root: &'a mut Table, path: &[String]) -> Option<&'a mut dyn TableLike> {
    let mut current: &mut dyn TableLike = root;
    for name in path {
        current = current.get_mut(name)?.as_table_like_mut()?;
    }
    Some(current)
}

/// Sets `key`, keeping the comments and spacing around an existing value.
fn set(table: &mut dyn TableLike, key: &str, value: &Value) {
    let mut new = match value {
        Value::Str(s) => TomlValue::from(s.as_str()),
        Value::List(items) => {
            let mut array: Array = items.iter().map(String::as_str).collect();
            if let Some(TomlValue::Array(old)) = table.get(key).and_then(Item::as_value) {
                array.set_trailing(old.trailing().clone());
                array.set_trailing_comma(old.trailing_comma());
            }
            TomlValue::Array(array)
        }
    };
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(old) => {
            *new.decor_mut() = old.decor().clone();
            *old = new;
        }
        None => {
            table.insert(key, Item::Value(new));
        }
    }
}

/// Applies `edits` to the Config file (creating it if there is none),
/// checks the result is a valid Config, and writes it. Returns the new
/// Config. The Daemon's watch picks the change up like any other edit.
pub fn save(edits: &[Edit]) -> Result<Config, String> {
    let file = path().ok_or("cannot find the Config folder: set $HOME or $XDG_CONFIG_HOME")?;
    save_to(&file, edits, home().as_deref())
}

/// [`save`] for the Config at `file`, `~` expanding to `home`.
pub fn save_to(file: &Path, edits: &[Edit], home: Option<&Path>) -> Result<Config, String> {
    // Write through a symlink (dotfiles), never over it.
    let target = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_owned());
    let text = read_text(&target)?.unwrap_or_default();
    let new = apply(&text, edits)?;
    let config = Config::parse(&new, home).map_err(|err| {
        format!(
            "not saved: {} has a problem to fix first: {err}",
            file.display()
        )
    })?;
    write(&target, &new).map_err(|err| format!("cannot save {}: {err}", target.display()))?;
    Ok(config)
}

/// Writes `text` to `path` through a temp file of its own and a rename,
/// so a reader (the Daemon's sync) never sees half a file and two writers
/// never share a temp file.
fn write(path: &Path, text: &str) -> std::io::Result<()> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .map_or_else(|| "config".into(), |n| n.to_string_lossy().into_owned());
    let tmp: PathBuf = dir.join(format!(
        ".{name}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// `path` as the Config spells it: `~/...` under `home`.
pub fn tilde(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests;
