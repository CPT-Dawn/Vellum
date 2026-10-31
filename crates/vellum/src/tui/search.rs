//! Fuzzy matching with nucleo, over an in-memory index of the Library.
//!
//! [`Index`] holds every wallpaper's name (its path inside its Library
//! folder) pre-converted for the matcher, so filtering on each keystroke
//! only scores. [`rank`] is the same matching for small one-off lists
//! (help, palette).

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32String};

/// One match: which item, and which of its characters matched (sorted
/// char indices, for highlighting).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub index: usize,
    pub matched: Vec<u32>,
}

/// The Library's names, ready to match.
pub struct Index {
    names: Vec<Utf32String>,
    matcher: Box<Matcher>,
}

impl std::fmt::Debug for Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Index({} names)", self.names.len())
    }
}

impl Default for Index {
    fn default() -> Index {
        Index::new(std::iter::empty::<&str>())
    }
}

impl Index {
    pub fn new<'a>(names: impl IntoIterator<Item = &'a str>) -> Index {
        Index {
            names: names.into_iter().map(Utf32String::from).collect(),
            matcher: Box::new(Matcher::new(Config::DEFAULT.match_paths())),
        }
    }

    /// The items among `candidates` that match `query`, best first (ties
    /// keep Library order). An empty query matches everything, in order,
    /// with nothing highlighted.
    pub fn filter(&mut self, query: &str, candidates: impl Iterator<Item = usize>) -> Vec<Match> {
        if query.trim().is_empty() {
            return candidates
                .map(|index| Match {
                    index,
                    matched: Vec::new(),
                })
                .collect();
        }
        let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
        let mut scored: Vec<(u32, Match)> = Vec::new();
        let mut matched = Vec::new();
        for index in candidates {
            let Some(name) = self.names.get(index) else {
                continue;
            };
            matched.clear();
            if let Some(score) = pattern.indices(name.slice(..), &mut self.matcher, &mut matched) {
                matched.sort_unstable();
                matched.dedup();
                scored.push((
                    score,
                    Match {
                        index,
                        matched: matched.clone(),
                    },
                ));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.index.cmp(&b.1.index)));
        scored.into_iter().map(|(_, m)| m).collect()
    }
}

/// [`Index::filter`] over a short list of strings.
pub fn rank<'a>(query: &str, items: impl Iterator<Item = &'a str>) -> Vec<Match> {
    let mut index = Index::new(items);
    let all = 0..index.names.len();
    index.filter(query, all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_matches_come_first_with_their_characters() {
        let names = [
            "aurora.png",
            "beach/dunes.jpg",
            "beach/tide.webp",
            "city/neon-rain.jpg",
        ];
        let mut index = Index::new(names);
        let found = index.filter("dun", 0..names.len());
        assert_eq!(found[0].index, 1);
        assert_eq!(found[0].matched, [6, 7, 8]);
        assert!(index.filter("zzz", 0..names.len()).is_empty());
        assert_eq!(index.filter("", [3, 0].into_iter()).len(), 2);
    }
}
