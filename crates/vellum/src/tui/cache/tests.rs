//! The caches through their public interface.

use std::thread::sleep;
use std::time::Duration;

use super::*;

/// Lets mtimes tell entries apart even on coarse filesystems.
fn tick() {
    sleep(Duration::from_millis(15));
}

/// What one entry with a 1-byte key and `n` data bytes costs on disk.
fn cost(n: u64) -> u64 {
    4 + 1 + n
}

#[test]
fn the_lru_drops_the_least_recently_used_value() {
    let mut lru = Lru::new(2);
    lru.put("a", 1);
    lru.put("b", 2);
    assert_eq!(lru.get(&"a"), Some(&1));
    lru.put("c", 3);
    assert_eq!(lru.get(&"b"), None);
    assert_eq!(lru.get(&"a"), Some(&1));
    assert_eq!(lru.get(&"c"), Some(&3));
    lru.put("a", 10);
    assert_eq!(lru.get(&"a"), Some(&10));
}

#[test]
fn a_blob_comes_back_by_its_key() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DiskCache::new(dir.path().join("thumbs"), 1 << 20);
    assert_eq!(cache.get("a"), None);
    cache.put("a", b"pixels");
    assert_eq!(cache.get("a").as_deref(), Some(&b"pixels"[..]));
    // Another cache on the same folder, as the next TUI run sees it.
    let reopened = DiskCache::new(dir.path().join("thumbs"), 1 << 20);
    assert_eq!(reopened.get("a").as_deref(), Some(&b"pixels"[..]));
    assert_eq!(reopened.get("b"), None);
}

#[test]
fn going_over_the_cap_evicts_the_least_recently_used_entries_first() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DiskCache::new(dir.path().to_owned(), 3 * cost(100));
    for key in ["a", "b", "c"] {
        cache.put(key, &[0; 100]);
        tick();
    }
    // Reading `a` makes `b` the oldest.
    assert!(cache.get("a").is_some());
    tick();
    cache.put("d", &[0; 100]);
    assert!(cache.get("b").is_none(), "the least recently used goes");
    for key in ["a", "c", "d"] {
        assert!(cache.get(key).is_some(), "{key} stays");
        tick();
    }

    // A big entry pushes out as many old ones as it needs: a and c are
    // now the oldest.
    cache.put("e", &[0; 200]);
    let left: Vec<_> = ["a", "c", "d", "e"]
        .into_iter()
        .filter(|k| cache.get(k).is_some())
        .collect();
    assert_eq!(left, ["d", "e"]);
}

#[test]
fn eviction_counts_what_an_earlier_run_left_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let first = DiskCache::new(dir.path().to_owned(), 2 * cost(100));
    first.put("a", &[0; 100]);
    tick();
    first.put("b", &[0; 100]);
    tick();
    let second = DiskCache::new(dir.path().to_owned(), 2 * cost(100));
    second.put("c", &[0; 100]);
    assert!(second.get("a").is_none());
    assert!(second.get("b").is_some());
    assert!(second.get("c").is_some());
}

#[test]
fn a_zero_cap_turns_the_disk_cache_off() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DiskCache::new(dir.path().join("thumbs"), 0);
    cache.put("a", b"pixels");
    assert_eq!(cache.get("a"), None);
    assert!(!dir.path().join("thumbs").exists());
}

#[test]
fn an_entry_bigger_than_the_cap_is_not_stored() {
    let dir = tempfile::tempdir().unwrap();
    let cache = DiskCache::new(dir.path().to_owned(), cost(10));
    cache.put("a", &[0; 10]);
    cache.put("b", &[0; 11]);
    assert!(cache.get("a").is_some());
    assert!(cache.get("b").is_none());
}
