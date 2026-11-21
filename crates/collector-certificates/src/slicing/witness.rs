use engine_allocation::{advisory, reserve};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Default)]
pub struct Witnesses {
    held: Vec<Entry>,
    held_words: Vec<u64>,
    next: Vec<Entry>,
    next_words: Vec<u64>,
}

#[derive(Clone, Copy)]
struct Entry {
    owner: u32,
    stamp: u64,
    first: usize,
    len: usize,
}

impl Witnesses {
    pub fn observe(&mut self, owner: u32, words: &[u64]) -> u64 {
        let held = self
            .held
            .binary_search_by_key(&owner, |entry| entry.owner)
            .ok()
            .map(|at| self.held[at])
            .filter(|entry| &self.held_words[entry.first..entry.first + entry.len] == words);
        let stamp = held.map_or_else(|| NEXT.fetch_add(1, Ordering::Relaxed), |entry| entry.stamp);
        if advisory(|| reserve(&mut self.next, 1).and_then(|()| reserve(&mut self.next_words, words.len())))
            .is_ok()
        {
            self.next.push(Entry {
                owner,
                stamp,
                first: self.next_words.len(),
                len: words.len(),
            });
            self.next_words.extend_from_slice(words);
        }
        stamp
    }

    pub fn finish(&mut self) {
        self.next.sort_unstable_by_key(|entry| entry.owner);
        self.next.dedup_by_key(|entry| entry.owner);
        std::mem::swap(&mut self.held, &mut self.next);
        std::mem::swap(&mut self.held_words, &mut self.next_words);
        self.next.clear();
        self.next_words.clear();
    }

    pub fn forget(&mut self) {
        self.held.clear();
        self.held_words.clear();
        self.next.clear();
        self.next_words.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::Witnesses;

    #[test]
    fn only_an_identical_witness_keeps_its_stamp() {
        let mut witnesses = Witnesses::default();
        let first = witnesses.observe(7, &[1, 2, 3]);
        let other = witnesses.observe(8, &[1, 2, 3]);
        assert_ne!(first, other);
        witnesses.finish();

        assert_eq!(witnesses.observe(7, &[1, 2, 3]), first);
        assert_ne!(witnesses.observe(8, &[1, 2, 4]), other);
        witnesses.finish();

        let changed = witnesses.observe(8, &[1, 2, 3]);
        assert_ne!(changed, other);
        assert_ne!(witnesses.observe(7, &[1, 2]), first);
        witnesses.finish();

        witnesses.finish();
        assert_ne!(witnesses.observe(8, &[1, 2, 3]), changed);
    }

    #[test]
    fn rebuilt_stores_never_reissue_a_stamp() {
        let first = Witnesses::default().observe(7, &[5]);
        let rebuilt = Witnesses::default().observe(7, &[5]);
        assert_ne!(first, rebuilt);
    }
}
