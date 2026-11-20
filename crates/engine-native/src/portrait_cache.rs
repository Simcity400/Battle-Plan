//! Reuse of owned portrait entries certified by an observed native mutation revision.
//! The caller still authenticates every roster occurrence and current array header.
//!
//! Portrait holder writers may run during a capture, held or beside the game. The first to change
//! a holder during one logs the holder's array as it was ([`PreImages`]), and every reader of a
//! holder's array reads it live and then takes that image instead ([`settled`]), so a frame holds
//! every portrait as it was when its capture began.
use std::sync::{
    OnceLock,
    atomic::{Ordering, fence},
};

/// A revision is usable only while all relevant native writers are observed and quiet.
pub trait Changes {
    fn revision(&self, array: usize) -> Option<(u64, u64)>;
    /// Optional quiet revision covering every writer of a watched holder.
    /// Callers still validate fresh owner/header dependencies before reuse.
    fn quiet_revision(&self) -> Option<(u64, u64)> {
        None
    }
    /// A revision every writer moves, including writers of holders no
    /// revision watches. A walk that reads unwatched holders afresh compares
    /// it across the walk; by default the quiet revision.
    fn any_revision(&self) -> Option<(u64, u64)> {
        self.quiet_revision()
    }
}

/// One portrait entry as a writer found it: its first 16 bytes (role, size, set flag, kind)
/// and where its identity bytes lie in the image's text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntryImage {
    pub head: [u8; 16],
    pub text: [usize; 2],
}

/// A holder's entry array as it was when the running capture began.
#[derive(Clone, Copy, Debug)]
pub struct HolderImage<'a> {
    /// The array's data pointer, capacity and count.
    pub header: [u8; 16],
    pub entries: &'a [EntryImage],
    pub text: &'a [u8],
}

impl HolderImage<'_> {
    pub fn identity(&self, entry: &EntryImage) -> Result<&[u8], &'static str> {
        let [start, len] = entry.text;
        start
            .checked_add(len)
            .and_then(|end| self.text.get(start..end))
            .ok_or("character portrait pre-image outside its text")
    }
}

/// The holders changed during the running capture.
pub trait PreImages: Sync {
    /// The array at `array` as it was when the running capture began, when a writer has
    /// changed it since; `Err` when that state could not be read or may have been changed
    /// unlogged.
    fn image(&self, array: usize) -> Option<Result<HolderImage<'_>, &'static str>>;
}

static PRE_IMAGES: OnceLock<&'static dyn PreImages> = OnceLock::new();

/// The collector's store; the first installed stays.
pub fn install(pre_images: &'static dyn PreImages) {
    let _ = PRE_IMAGES.set(pre_images);
}

/// Every reader of a portrait holder's entry array goes through here. `live` is what it read of
/// the array at `array` before this call; when a writer has changed that array since the
/// running capture began, `image` builds the reader's result from the array as it was then; the
/// store's refusal is the result when it cannot know that.
pub fn settled<T>(
    array: usize,
    live: Result<T, &'static str>,
    image: impl FnOnce(HolderImage<'_>) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    // The live reads come first: a writer whose log this misses had not begun to write.
    fence(Ordering::SeqCst);
    match PRE_IMAGES.get().and_then(|images| images.image(array)) {
        None => live,
        Some(Ok(holder)) => image(holder),
        Some(Err(reason)) => Err(reason),
    }
}

#[cfg(test)]
pub(crate) mod test_images {
    use super::*;
    use std::cell::Cell;

    thread_local! {
        static IMAGE: Cell<Option<(usize, HolderImage<'static>)>> = const { Cell::new(None) };
    }

    struct ThreadImages;
    impl PreImages for ThreadImages {
        fn image(&self, array: usize) -> Option<Result<HolderImage<'_>, &'static str>> {
            IMAGE
                .get()
                .filter(|(at, _)| *at == array)
                .map(|(_, image)| Ok(image))
        }
    }

    /// Readers on this thread take `image` for the array at `array` until it is cleared.
    pub(crate) fn set(array: usize, image: Option<HolderImage<'static>>) {
        install(&ThreadImages);
        IMAGE.set(image.map(|image| (array, image)));
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused_characters: u64,
    pub reused_entries: u64,
    pub read_characters: u64,
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
    pub fn new(rows: usize) -> std::io::Result<Self> {
        Ok(Self {
            keys: crate::allocation::filled(rows, Key::default())?,
            scope: (0, 0, 0, 0),
            valid: false,
            used: 0,
            counts: Counts::default(),
        })
    }

    pub fn empty() -> Self {
        Self {
            keys: Vec::new(),
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
    /// Growth only appends default keys: the certified ones keep naming the
    /// native holders and the arena offsets the owner's growth preserves, so
    /// the cache stays valid.
    pub fn grow(&mut self, rows: usize) -> std::io::Result<()> {
        if self.keys.len() < rows {
            crate::allocation::resize(&mut self.keys, rows, Key::default())?;
        }
        Ok(())
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

    /// Every key of the latest walk holds a revision: no entry was left to
    /// be read afresh every hour for want of a watch.
    pub fn all_watched(&self) -> bool {
        self.valid
            && self.keys[..self.used]
                .iter()
                .all(|key| key.revision.is_some())
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
    fn allocation_growth_preserves_the_previous_capacity_trace() {
        let mut cache = Cache::new(1).unwrap();
        let mut previous = vec![Key::default(); 1];
        for rows in [2, 5, 4, 9] {
            if previous.len() < rows {
                previous.resize(rows, Key::default());
            }
            cache.grow(rows).unwrap();
            assert_eq!(cache.keys.capacity(), previous.capacity());
            assert_eq!(
                cache.allocated_bytes(),
                previous.capacity() * std::mem::size_of::<Key>()
            );
        }
    }

    /// Expanding the keys keeps a certified walk: its keys, extent, scope and
    /// validity, so the next walk over the same census still reuses them.
    #[test]
    fn growth_keeps_the_certified_keys() {
        let mut cache = Cache::new(1).unwrap();
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
        cache.finish(1);
        cache.grow(8).unwrap();
        assert_eq!(cache.keys.len(), 8);
        let row = crate::ccharacter_manager::PortraitIdentity {
            pointer: key.pointer,
            character: key.character,
        };
        assert!(cache.matches_census(&[row]));
        assert!(cache.all_watched());
        cache.begin(scope);
        assert!(cache.matches(0, key));
    }

    #[test]
    fn failed_growth_keeps_the_existing_cache() {
        let mut cache = Cache::new(1).unwrap();
        cache.valid = true;
        let pointer = cache.keys.as_ptr();
        assert_eq!(
            cache.grow(usize::MAX).unwrap_err().kind(),
            std::io::ErrorKind::OutOfMemory
        );
        assert_eq!(cache.keys.as_ptr(), pointer);
        assert!(cache.valid);
    }

    #[test]
    fn ownership_arena_position_revision_and_scope_are_required() {
        let mut cache = Cache::new(1).unwrap();
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
