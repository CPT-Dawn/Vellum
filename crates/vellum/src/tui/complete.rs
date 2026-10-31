//! Tab completion of folder names on the welcome screen.

use std::path::{Path, PathBuf};

/// The folders `input` could complete to, spelled the way it was typed
/// (`~/` kept) with a trailing `/`, sorted. Hidden folders only show when
/// the typed name starts with a dot.
pub fn folders(input: &str, home: Option<&Path>) -> Vec<String> {
    let input = if input.is_empty() { "~/" } else { input };
    let (typed_dir, prefix) = match input.rfind('/') {
        Some(i) => (&input[..=i], &input[i + 1..]),
        // A bare `~` means the home folder.
        None if input == "~" => return vec!["~/".into()],
        None => return Vec::new(),
    };
    let Some(dir) = expand(typed_dir, home) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| std::fs::metadata(e.path()).is_ok_and(|m| m.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| name.starts_with(prefix))
        .filter(|name| !name.starts_with('.') || prefix.starts_with('.'))
        .map(|name| format!("{typed_dir}{name}/"))
        .collect();
    out.sort();
    out
}

/// The longest start every candidate shares.
pub fn common_prefix(candidates: &[String]) -> String {
    let Some(first) = candidates.first() else {
        return String::new();
    };
    let mut end = first.len();
    for other in &candidates[1..] {
        end = first
            .char_indices()
            .zip(other.chars())
            .take_while(|((_, a), b)| a == b)
            .last()
            .map_or(0, |((i, a), _)| i + a.len_utf8())
            .min(end);
    }
    first[..end].to_owned()
}

/// A typed folder as a path: `~` and `~/...` against `home`, else it must
/// be absolute.
pub fn expand(typed: &str, home: Option<&Path>) -> Option<PathBuf> {
    let typed = typed.trim();
    if typed == "~" {
        return home.map(Path::to_path_buf);
    }
    if let Some(rest) = typed.strip_prefix("~/") {
        return home.map(|h| h.join(rest));
    }
    Some(PathBuf::from(typed)).filter(|p| p.is_absolute())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_lists_matching_folders_spelled_as_typed() {
        let home = tempfile::tempdir().unwrap();
        for dir in ["Pictures", "Pics-old", "Music", ".hidden"] {
            std::fs::create_dir(home.path().join(dir)).unwrap();
        }
        std::fs::write(home.path().join("Picture.png"), b"").unwrap();
        let home = Some(home.path());
        assert_eq!(folders("~/Pi", home), ["~/Pics-old/", "~/Pictures/"]);
        assert_eq!(folders("~/Mu", home), ["~/Music/"]);
        assert_eq!(folders("~/.h", home), ["~/.hidden/"]);
        assert_eq!(folders("", home).len(), 3, "no hidden, no files");
        assert!(folders("~/nope/x", home).is_empty());
        assert!(folders("relative", home).is_empty());
    }

    #[test]
    fn the_common_prefix_is_what_tab_can_fill_in() {
        let c = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert_eq!(common_prefix(&c(&["~/Pics-old/", "~/Pictures/"])), "~/Pic");
        assert_eq!(common_prefix(&c(&["~/Music/"])), "~/Music/");
        assert_eq!(common_prefix(&[]), "");
    }
}
