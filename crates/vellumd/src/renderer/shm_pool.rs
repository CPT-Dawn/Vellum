use anyhow::{bail, Result};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
struct BufferEntry {
    bytes: usize,
    in_use: bool,
    last_touched_generation: u64,
}

#[derive(Debug, Default)]
pub(crate) struct ShmPool {
    entries: BTreeMap<u64, BufferEntry>,
    next_id: u64,
    generation: u64,
}

impl ShmPool {
    pub(crate) fn acquire(&mut self, required_bytes: usize) -> Result<u64> {
        if required_bytes == 0 {
            bail!("buffer size must be greater than zero");
        }

        self.generation = self.generation.saturating_add(1);

        let mut candidate: Option<(u64, usize)> = None;
        for (&id, entry) in &self.entries {
            if entry.in_use || entry.bytes < required_bytes {
                continue;
            }

            match candidate {
                Some((_, best_bytes)) if entry.bytes >= best_bytes => {}
                _ => candidate = Some((id, entry.bytes)),
            }
        }

        if let Some((id, _)) = candidate {
            if let Some(entry) = self.entries.get_mut(&id) {
                entry.in_use = true;
                entry.last_touched_generation = self.generation;
                return Ok(id);
            }
        }

        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.entries.insert(
            id,
            BufferEntry {
                bytes: required_bytes,
                in_use: true,
                last_touched_generation: self.generation,
            },
        );
        Ok(id)
    }

    pub(crate) fn release(&mut self, id: u64) {
        self.generation = self.generation.saturating_add(1);
        if let Some(entry) = self.entries.get_mut(&id) {
            entry.in_use = false;
            entry.last_touched_generation = self.generation;
        }
    }

    pub(crate) fn reclaim_unused(&mut self, max_idle_generations: u64) {
        self.generation = self.generation.saturating_add(1);
        let now = self.generation;
        self.entries.retain(|_, entry| {
            if entry.in_use {
                true
            } else {
                now.saturating_sub(entry.last_touched_generation) <= max_idle_generations
            }
        });
    }

    pub(crate) fn leased_count(&self) -> usize {
        self.entries.values().filter(|entry| entry.in_use).count()
    }

    #[cfg(test)]
    pub(crate) fn total_bytes(&self) -> usize {
        self.entries.values().map(|entry| entry.bytes).sum()
    }

    pub(crate) fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::ShmPool;

    #[test]
    fn acquire_release_and_reuse_buffer() {
        let mut pool = ShmPool::default();

        let first = pool.acquire(4096).expect("initial acquire should succeed");
        assert_eq!(pool.leased_count(), 1);

        pool.release(first);
        assert_eq!(pool.leased_count(), 0);

        let reused = pool.acquire(2048).expect("reuse should succeed");
        assert_eq!(first, reused);
        assert_eq!(pool.leased_count(), 1);
    }

    #[test]
    fn reclaim_unused_drops_old_free_buffers() {
        let mut pool = ShmPool::default();

        let first = pool.acquire(1024).expect("acquire should succeed");
        let second = pool.acquire(2048).expect("acquire should succeed");

        pool.release(first);
        pool.release(second);
        assert_eq!(pool.entry_count(), 2);

        pool.reclaim_unused(0);
        assert_eq!(pool.entry_count(), 0);
    }
}
