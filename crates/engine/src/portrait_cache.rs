//! Reuse of owned portrait entries certified by an observed native mutation revision.
//! The caller still authenticates every roster occurrence and current array header.

/// A revision is usable only while all relevant native writers are observed and quiet.
pub trait Changes {
    fn revision(&self, array: usize) -> Option<(u64, u64)>;
    fn verify(&self) -> bool;
    /// Optional quiet revision covering every writer in this observation domain.
    /// Callers still validate fresh owner/header dependencies before reuse.
    fn quiet_revision(&self) -> Option<(u64, u64)> {
        None
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused_characters: u64,
    pub reused_entries: u64,
    pub read_characters: u64,
    pub verified_entries: u64,
    pub mismatches: u64,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Key {
    pub pointer: usize,
    pub character: u32,
    pub header: [u8; 16],
    pub start: usize,
    pub revision: Option<(u64, u64)>,
}

/// Metadata only: descriptors remain in the owning acquisition's existing arena.
/// A changed arena position misses, so earlier writes cannot corrupt a later hit.
pub struct Cache {
    keys: Vec<Key>,
    scope: (usize, usize, usize, usize),
    valid: bool,
    used: usize,
    pub counts: Counts,
}

impl Cache {
    pub fn new(rows: usize) -> Self {
        Self {
            keys: vec![Key::default(); rows],
            scope: (0, 0, 0, 0),
            valid: false,
            used: 0,
            counts: Counts::default(),
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.keys.capacity() * std::mem::size_of::<Key>()
    }

    /// Called with ordinary off-thread buffer growth, never during acquisition.
    pub fn grow(&mut self, rows: usize) {
        if self.keys.len() < rows {
            self.keys.resize(rows, Key::default());
            self.invalidate();
        }
    }

    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    /// A mutation elsewhere need not invalidate this share's owned descriptors.
    /// Header/roster/identity dependencies are checked separately, using fresh
    /// reads, before the caller may publish the unchanged rows.
    pub fn unchanged(&self, changes: &dyn Changes) -> bool {
        self.valid
            && self.keys[..self.used].iter().all(|key| {
                key.revision.is_some()
                    && key
                        .pointer
                        .checked_add(crate::ccharacter::PORTRAIT_DATA)
                        .and_then(|array| changes.revision(array))
                        == key.revision
            })
    }

    /// Current owned occurrences must match the exact old share before its
    /// header-only prepared program may run. Never deduplicate aliases.
    pub fn matches_census(&self, rows: &[crate::ccharacter_manager::PortraitIdentity]) -> bool {
        self.valid
            && self.used == rows.len()
            && self.keys[..self.used]
                .iter()
                .zip(rows)
                .all(|(key, row)| key.pointer == row.pointer && key.character == row.character)
    }

    pub(crate) fn begin(&mut self, scope: (usize, usize, usize, usize)) {
        if self.scope != scope || !self.valid {
            self.keys[..self.used].fill(Key::default());
            self.used = 0;
        }
        self.scope = scope;
        self.valid = true;
        self.counts = Counts::default();
    }

    pub(crate) fn matches(&self, index: usize, key: Key) -> bool {
        key.revision.is_some() && self.keys.get(index) == Some(&key)
    }

    pub(crate) fn remember(&mut self, index: usize, key: Key) {
        if let Some(slot) = self.keys.get_mut(index) {
            *slot = key;
            self.used = self.used.max(index + 1);
        }
    }

    pub(crate) fn finish(&mut self, rows: usize) {
        // Slots outside a shrinking roster were not refreshed. Their old arena
        // ranges may have been overwritten by other characters this time.
        if rows < self.used {
            self.keys[rows..self.used].fill(Key::default());
            self.used = rows;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_arena_position_revision_and_scope_are_required() {
        let mut cache = Cache::new(1);
        let scope = (1, 2, 0, 4);
        let key = Key {
            pointer: 100,
            character: 7,
            header: [3; 16],
            start: 4,
            revision: Some((2, 9)),
        };
        cache.begin(scope);
        cache.remember(0, key);
        let row = crate::ccharacter_manager::PortraitIdentity {
            pointer: key.pointer,
            character: key.character,
        };
        assert!(cache.matches_census(&[row]));
        assert!(!cache.matches_census(&[]));
        assert!(!cache.matches_census(&[row, row]));
        assert!(
            !cache.matches_census(&[crate::ccharacter_manager::PortraitIdentity {
                pointer: 200,
                ..row
            }])
        );
        assert!(
            !cache.matches_census(&[crate::ccharacter_manager::PortraitIdentity {
                character: 99,
                ..row
            }])
        );
        cache.begin(scope);
        assert!(cache.matches(0, key));
        for different in [
            Key {
                pointer: 200,
                ..key
            },
            Key {
                character: 8,
                ..key
            },
            Key {
                header: [4; 16],
                ..key
            },
            Key { start: 5, ..key },
            Key {
                revision: Some((2, 10)),
                ..key
            },
            Key {
                revision: Some((3, 9)),
                ..key
            },
            Key {
                revision: None,
                ..key
            },
        ] {
            assert!(!cache.matches(0, different));
        }
        assert!(!cache.matches(1, key));
        cache.invalidate();
        cache.begin(scope);
        assert!(!cache.matches(0, key));
        cache.remember(0, key);
        cache.begin((1, 3, 0, 4));
        assert!(!cache.matches(0, key));
        cache.remember(0, key);
        cache.finish(0);
        assert!(!cache.matches(0, key));
    }
}
