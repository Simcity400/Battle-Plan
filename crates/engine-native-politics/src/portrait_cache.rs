use std::{
    marker::PhantomData,
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering, fence},
    },
};

pub trait Changes {
    fn revision(&self, array: usize) -> Option<(u64, u64)>;
    fn quiet_revision(&self) -> Option<(u64, u64)> {
        None
    }
    fn any_revision(&self) -> Option<(u64, u64)> {
        self.quiet_revision()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EntryImage {
    pub head: [u8; 16],
    pub text: [usize; 2],
}

#[derive(Clone, Copy, Debug)]
pub struct HolderImage<'a> {
    pub header: [u8; 16],
    entries: &'a [EntryImage],
    text: *const u8,
    text_len: usize,
    _text: PhantomData<&'a [u8]>,
}

impl<'a> HolderImage<'a> {
    pub fn new(header: [u8; 16], entries: &'a [EntryImage], text: &'a [u8]) -> Self {
        Self {
            header,
            entries,
            text: text.as_ptr(),
            text_len: text.len(),
            _text: PhantomData,
        }
    }

    /// # Safety
    /// For `'a`, `text` points to `text_len` bytes in one allocation, and the bytes each of
    /// `entries` names there are initialized and unchanged. Other bytes in the span may change.
    pub unsafe fn published(
        header: [u8; 16],
        entries: &'a [EntryImage],
        text: *const u8,
        text_len: usize,
    ) -> Self {
        Self {
            header,
            entries,
            text,
            text_len,
            _text: PhantomData,
        }
    }

    pub fn entries(&self) -> &'a [EntryImage] {
        self.entries
    }

    pub fn identity(&self, index: usize) -> Result<&'a [u8], &'static str> {
        let [start, len] = self
            .entries
            .get(index)
            .ok_or("character portrait pre-image entry outside its holder")?
            .text;
        if start.checked_add(len).is_none_or(|end| end > self.text_len) {
            return Err("character portrait pre-image outside its text");
        }
        // SAFETY: the range lies within the text span and belongs to one of this image's
        // entries, which its constructor keeps unchanged for `'a`.
        Ok(unsafe { std::slice::from_raw_parts(self.text.add(start), len) })
    }
}

pub struct Readers(AtomicUsize);

impl Readers {
    pub const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }

    pub fn enter(&self) -> Reader<'_> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Reader(self)
    }

    pub fn idle(&self) -> bool {
        self.0.load(Ordering::SeqCst) == 0
    }
}

impl Default for Readers {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Reader<'a>(&'a Readers);

impl Drop for Reader<'_> {
    fn drop(&mut self) {
        self.0.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Reading<'a> {
    _reader: Reader<'a>,
    image: HolderImage<'a>,
}

impl<'a> Reading<'a> {
    pub fn new(reader: Reader<'a>, image: HolderImage<'a>) -> Self {
        Self {
            _reader: reader,
            image,
        }
    }

    pub fn image(&self) -> HolderImage<'_> {
        self.image
    }
}

pub trait PreImages: Sync {
    fn image(&self, array: usize) -> Option<Result<Reading<'_>, &'static str>>;
}

static PRE_IMAGES: OnceLock<&'static dyn PreImages> = OnceLock::new();

pub fn install(pre_images: &'static dyn PreImages) {
    let _ = PRE_IMAGES.set(pre_images);
}

pub fn settled<T>(
    array: usize,
    live: Result<T, &'static str>,
    image: impl FnOnce(HolderImage<'_>) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    fence(Ordering::SeqCst);
    match PRE_IMAGES.get().and_then(|images| images.image(array)) {
        None => live,
        Some(Ok(reading)) => image(reading.image()),
        Some(Err(reason)) => Err(reason),
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_images {
    use super::*;
    use std::cell::Cell;

    thread_local! {
        static IMAGE: Cell<Option<(usize, HolderImage<'static>)>> = const { Cell::new(None) };
    }

    static READERS: Readers = Readers::new();

    struct ThreadImages;
    impl PreImages for ThreadImages {
        fn image(&self, array: usize) -> Option<Result<Reading<'_>, &'static str>> {
            IMAGE
                .get()
                .filter(|(at, _)| *at == array)
                .map(|(_, image)| Ok(Reading::new(READERS.enter(), image)))
        }
    }

    pub fn set(array: usize, image: Option<HolderImage<'static>>) {
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
            keys: engine_allocation::filled(rows, Key::default())?,
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

    pub fn grow(&mut self, rows: usize) -> std::io::Result<()> {
        if self.keys.len() < rows {
            engine_allocation::resize(&mut self.keys, rows, Key::default())?;
        }
        Ok(())
    }

    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    pub fn unchanged(&self, changes: &dyn Changes) -> bool {
        self.valid
            && self.keys[..self.used].iter().all(|key| {
                key.revision.is_some()
                    && key
                        .pointer
                        .checked_add(engine_native::ccharacter::PORTRAIT_DATA)
                        .and_then(|array| changes.revision(array))
                        == key.revision
            })
    }

    pub fn all_watched(&self) -> bool {
        self.valid
            && self.keys[..self.used]
                .iter()
                .all(|key| key.revision.is_some())
    }

    pub fn matches_census(&self, rows: &[engine_native::ccharacter::manager::PortraitIdentity]) -> bool {
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
        let row = engine_native::ccharacter::manager::PortraitIdentity {
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
        let row = engine_native::ccharacter::manager::PortraitIdentity {
            pointer: key.pointer,
            character: key.character,
        };
        assert!(cache.matches_census(&[row]));
        assert!(!cache.matches_census(&[]));
        assert!(!cache.matches_census(&[row, row]));
        assert!(
            !cache.matches_census(&[engine_native::ccharacter::manager::PortraitIdentity {
                pointer: 200,
                ..row
            }])
        );
        assert!(
            !cache.matches_census(&[engine_native::ccharacter::manager::PortraitIdentity {
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
