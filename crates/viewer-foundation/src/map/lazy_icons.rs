//! Icon textures decoded on first use instead of at content load.
//!
//! A content set indexes every flag file and portrait texture it ships, but a
//! map draws only the flags of the countries and the portraits of the leaders
//! it shows. With a flag per country for thousands of countries, decoding the
//! whole index at load held hundreds of megabytes the session never drew.
//!
//! So the layout asks for a texture when it draws one. A texture already
//! decoded answers at once; one that is not is queued and the layout draws the
//! native fallback it would draw for an unreadable file (a flag's plain tag
//! flag when that is loaded) or nothing, for that frame only. The queue is
//! decoded on a bounded pool, off the UI thread ([`LazyTextures::schedule`]),
//! or synchronously by a headless render ([`LazyTextures::settle`]), which then
//! lays the frame out again. Every change bumps [`LazyTextures::version`], so
//! a window that laid a frame out before the load lays it out again.
//!
//! Texture ids of lazy textures start at [`DYNAMIC_BASE`], above every static
//! icon page, and are never reused, so a quad laid out against an id that has
//! since been evicted draws nothing rather than another texture.

#[cfg(test)]
use crate::map::test_support::TEST_HANG_GUARD;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc, Mutex, MutexGuard, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use installed_content_image::{Rgba8Image, dds};

use super::atlas_pages::{CellPages, page_limit};
use installed_content_core::{ResolvedLayers, flags, flags::FlagSet};

/// The first lazy texture id; static icon pages sit below it.
pub const DYNAMIC_BASE: usize = 1 << 40;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// A queue of requested textures that can be decoded on demand.
pub trait PendingLoad: Send + Sync {
    /// Decodes every queued request; whether any was queued.
    fn load_pending(&self) -> bool;
    /// Whether any request is queued.
    fn has_pending(&self) -> bool;
}

/// One lazy texture as the renderer uploads it.
#[derive(Clone)]
pub struct Resident {
    /// Bumped whenever the texture's pixels change.
    pub revision: u64,
    pub texture: Arc<dds::Texture>,
}

#[derive(Default)]
struct Registry {
    entries: BTreeMap<usize, Resident>,
    /// The frame each texture was last drawn in ([`LazyTextures::mark_drawn`]).
    drawn: HashMap<usize, u64>,
    next: usize,
}

/// Frames a texture stays pinned after it was last drawn or asked for. Every
/// rendered map view advances the frame, so this covers a few frames of each
/// of several views sharing one content set.
pub const PIN_FRAMES: u64 = 8;

type Wake = Arc<dyn Fn() + Send + Sync>;

/// Every lazy texture of one content set, and the loader that fills them.
pub struct LazyTextures {
    registry: Mutex<Registry>,
    version: AtomicU64,
    /// Rendered frames so far; the LRU clock of [`LazyImages`].
    frame: AtomicU64,
    loaders: Mutex<Vec<Weak<dyn PendingLoad>>>,
    running: AtomicBool,
    background: AtomicBool,
    wake: Mutex<Option<Wake>>,
}

impl LazyTextures {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            registry: Mutex::default(),
            version: AtomicU64::new(0),
            frame: AtomicU64::new(0),
            loaders: Mutex::default(),
            running: AtomicBool::new(false),
            background: AtomicBool::new(true),
            wake: Mutex::default(),
        })
    }

    /// Changes so far; any new texture, pixel change or eviction bumps it.
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    fn bump(&self) {
        self.version.fetch_add(1, Ordering::AcqRel);
    }

    fn insert(&self, texture: dds::Texture) -> usize {
        let mut registry = lock(&self.registry);
        let id = DYNAMIC_BASE + registry.next;
        registry.next += 1;
        registry.entries.insert(
            id,
            Resident {
                revision: 0,
                texture: Arc::new(texture),
            },
        );
        drop(registry);
        self.bump();
        id
    }

    fn modify(&self, id: usize, change: impl FnOnce(&mut dds::Texture)) {
        let mut registry = lock(&self.registry);
        if let Some(entry) = registry.entries.get_mut(&id) {
            change(Arc::make_mut(&mut entry.texture));
            entry.revision += 1;
        }
        drop(registry);
        self.bump();
    }

    fn remove(&self, id: usize) {
        let mut registry = lock(&self.registry);
        registry.entries.remove(&id);
        registry.drawn.remove(&id);
        drop(registry);
        self.bump();
    }

    /// The current frame.
    pub fn frame(&self) -> u64 {
        self.frame.load(Ordering::Acquire)
    }

    /// Records the lazy textures one rendered frame drew and ends the frame.
    /// A texture drawn in the last [`PIN_FRAMES`] frames is never evicted, so
    /// what is on screen does not thrash however many portraits it shows.
    pub fn mark_drawn(&self, ids: impl IntoIterator<Item = usize>) {
        let frame = self.frame();
        let mut registry = lock(&self.registry);
        for id in ids {
            if registry.entries.contains_key(&id) {
                registry.drawn.insert(id, frame);
            }
        }
        drop(registry);
        self.frame.fetch_add(1, Ordering::AcqRel);
    }

    /// The frame `id` was last drawn in.
    fn drawn_at(&self, id: usize) -> Option<u64> {
        lock(&self.registry).drawn.get(&id).copied()
    }

    /// Every resident texture, for the renderer to mirror.
    pub fn snapshot(&self) -> Vec<(usize, Resident)> {
        lock(&self.registry)
            .entries
            .iter()
            .map(|(id, entry)| (*id, entry.clone()))
            .collect()
    }

    /// Bytes of every resident texture's pixels.
    pub fn resident_bytes(&self) -> usize {
        lock(&self.registry)
            .entries
            .values()
            .map(|entry| {
                entry
                    .texture
                    .mips
                    .iter()
                    .flatten()
                    .map(|mip| mip.data.len())
                    .sum::<usize>()
            })
            .sum()
    }

    fn register(&self, loader: Weak<dyn PendingLoad>) {
        lock(&self.loaders).push(loader);
    }

    fn loaders(&self) -> Vec<Arc<dyn PendingLoad>> {
        lock(&self.loaders)
            .iter()
            .filter_map(Weak::upgrade)
            .collect()
    }

    /// What a background load calls once it has decoded something, so a
    /// still window repaints and lays its frame out again.
    pub fn set_wake(&self, wake: impl Fn() + Send + Sync + 'static) {
        *lock(&self.wake) = Some(Arc::new(wake));
    }

    pub fn has_wake(&self) -> bool {
        lock(&self.wake).is_some()
    }

    /// Whether any request is queued.
    pub fn has_pending(&self) -> bool {
        self.loaders().iter().any(|loader| loader.has_pending())
    }

    /// Whether any request is queued or a background load is decoding one: a frame drawn
    /// meanwhile may still lack what it asked for.
    pub fn busy(&self) -> bool {
        self.has_pending() || self.running.load(Ordering::Acquire)
    }

    /// Decodes every queued request on this thread, waiting for a background
    /// load already under way. Returns whether anything was decoded here.
    pub fn settle(&self) -> bool {
        let mut any = false;
        for loader in self.loaders() {
            any |= loader.load_pending();
        }
        any
    }

    /// Whether a request starts a background load. A headless render turns
    /// it off and settles each frame itself, so its cells land in the same
    /// order on every run.
    pub fn set_background(&self, on: bool) {
        self.background.store(on, Ordering::Release);
    }

    /// Starts a background load of the queued requests unless one is running.
    pub fn schedule(this: &Arc<Self>) {
        if !this.background.load(Ordering::Acquire) || this.running.swap(true, Ordering::AcqRel) {
            return;
        }
        let textures = Arc::clone(this);
        let spawned = std::thread::Builder::new()
            .name("lazy icon loader".into())
            .spawn(move || {
                loop {
                    while textures.settle() {}
                    textures.running.store(false, Ordering::Release);
                    // A request queued after the last pass and before the flag
                    // cleared would otherwise wait for the next one.
                    if !textures.has_pending() || textures.running.swap(true, Ordering::AcqRel) {
                        break;
                    }
                }
                let wake = lock(&textures.wake).clone();
                if let Some(wake) = wake {
                    wake();
                }
            });
        if spawned.is_err() {
            // No thread to spare: the next settle or schedule picks it up.
            this.running.store(false, Ordering::Release);
        }
    }
}

/// Turns a flag file's bytes into its finished cell image, or `None` when the
/// engine's reader would refuse it.
pub type FlagReader = Arc<dyn Fn(&str) -> Option<Rgba8Image> + Send + Sync>;

#[derive(Default)]
struct FlagState {
    /// Decided keys: their cell, or `None` for a file the reader refused.
    cells: HashMap<String, Option<u32>>,
    /// The key of every cell, in cell order.
    keys: Vec<String>,
    pending: BTreeSet<String>,
    /// Texture id of each page.
    pages: Vec<usize>,
    rejected: usize,
}

/// One flag set: its file index, and the cells decoded so far on fixed pages.
pub struct LazyFlags {
    /// `<TAG>_<group>` or `<TAG>` to its file.
    index: BTreeMap<String, String>,
    pages: CellPages,
    read: FlagReader,
    state: Mutex<FlagState>,
    loading: Mutex<()>,
    textures: Arc<LazyTextures>,
}

impl LazyFlags {
    pub fn new(
        index: BTreeMap<String, String>,
        pages: CellPages,
        read: FlagReader,
        textures: &Arc<LazyTextures>,
    ) -> Arc<Self> {
        let flags = Arc::new(Self {
            index,
            pages,
            read,
            state: Mutex::default(),
            loading: Mutex::default(),
            textures: Arc::clone(textures),
        });
        let weak: Weak<dyn PendingLoad> = Arc::downgrade(&flags) as Weak<dyn PendingLoad>;
        textures.register(weak);
        flags
    }

    /// Files indexed.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// The set's cell size.
    pub fn cell_size(&self) -> [u32; 2] {
        self.pages.cell
    }

    /// Files the reader has refused so far.
    pub fn rejected(&self) -> usize {
        lock(&self.state).rejected
    }

    /// Cells decoded so far.
    pub fn loaded(&self) -> usize {
        lock(&self.state).keys.len()
    }

    /// Whether `key` names a file that is not known to be unreadable.
    pub fn contains(&self, key: &str) -> bool {
        self.index.contains_key(key) && !matches!(lock(&self.state).cells.get(key), Some(None))
    }

    /// Longest indexed key at the start of producer text, without decoding its artwork.
    pub fn key_prefix(&self, text: &[char]) -> Option<usize> {
        use std::ops::Bound::{Included, Unbounded};
        let mut prefix = String::new();
        let mut found = None;
        for (index, ch) in text.iter().enumerate() {
            prefix.push(*ch);
            let Some((key, _)) = self
                .index
                .range::<str, _>((Included(prefix.as_str()), Unbounded))
                .next()
                .filter(|(key, _)| key.starts_with(&prefix))
            else {
                break;
            };
            if key == &prefix {
                found = Some(index + 1);
            }
        }
        found
    }

    /// The first readable candidate's page texture and rectangle.
    ///
    /// Candidates run in the engine's order (`<TAG>_<group>`, then `<TAG>`);
    /// one it could not read is skipped, as the engine skips to its next
    /// candidate. The first candidate not decoded yet is queued, and until it
    /// is, a later candidate that is already decoded stands in for it.
    pub fn resolve(&self, candidates: &[&str]) -> Option<(usize, [f32; 4])> {
        let mut state = lock(&self.state);
        let mut queued = false;
        let mut found = None;
        for key in candidates {
            if !self.index.contains_key(*key) {
                continue;
            }
            match state.cells.get(*key) {
                Some(None) => continue,
                Some(Some(cell)) => {
                    let (page, uv) = self.pages.uv(*cell);
                    found = Some((state.pages[page as usize], uv));
                    break;
                }
                None if !queued => {
                    state.pending.insert((*key).to_owned());
                    queued = true;
                }
                None => {}
            }
        }
        drop(state);
        if queued {
            LazyTextures::schedule(&self.textures);
        }
        found
    }

    /// [`Self::resolve`] with every candidate decided first, on this thread: the flag the
    /// candidates resolve to as it would be had the set been built whole at load, as the
    /// engine builds its flag atlases, so a frame
    /// never draws a stand-in for a flag that is not decoded yet.
    pub fn acquire(&self, candidates: &[&str]) -> Option<(usize, [f32; 4])> {
        let queued = {
            let mut state = lock(&self.state);
            let mut queued = false;
            for key in candidates {
                if self.index.contains_key(*key) && !state.cells.contains_key(*key) {
                    state.pending.insert((*key).to_owned());
                    queued = true;
                }
            }
            queued
        };
        if queued {
            self.load_pending();
        }
        self.resolve(candidates)
    }

    /// One exact key's cell, queued when not decoded yet.
    pub fn get(&self, key: &str) -> Option<(usize, [f32; 4])> {
        self.resolve(&[key])
    }

    /// The key a quad of this set draws, if it is one of its cells.
    pub fn key_at(&self, texture: usize, uv: [f32; 4]) -> Option<String> {
        let state = lock(&self.state);
        let page = state.pages.iter().position(|page| *page == texture)?;
        let cell = self.pages.cell_at(u32::try_from(page).ok()?, uv)?;
        state.keys.get(cell as usize).cloned()
    }

    /// Whether `texture` is one of this set's pages.
    pub fn owns(&self, texture: usize) -> bool {
        lock(&self.state).pages.contains(&texture)
    }
}

impl PendingLoad for LazyFlags {
    fn has_pending(&self) -> bool {
        !lock(&self.state).pending.is_empty()
    }

    fn load_pending(&self) -> bool {
        let _serial = lock(&self.loading);
        let keys: Vec<String> = {
            let mut state = lock(&self.state);
            let pending = std::mem::take(&mut state.pending);
            pending
                .into_iter()
                .filter(|key| !state.cells.contains_key(key))
                .collect()
        };
        if keys.is_empty() {
            return false;
        }
        // Requests decode on the bounded pool and are packed in key order, so
        // one set of requests always lands in the same cells.
        let images = crate::parallel::map(&keys, |key| {
            self.index.get(key).and_then(|path| (self.read)(path))
        });
        let mut state = lock(&self.state);
        for (key, image) in keys.into_iter().zip(images) {
            let Some(image) = image else {
                eprintln!(
                    "Warning: flag {} is not an uncompressed 24/32-bit {} x {} Targa; the next candidate flag is drawn instead.",
                    self.index.get(&key).map_or(key.as_str(), String::as_str),
                    self.pages.cell[0],
                    self.pages.cell[1]
                );
                state.cells.insert(key, None);
                state.rejected += 1;
                continue;
            };
            let cell = state.keys.len() as u32;
            let (page, at) = self.pages.locate(cell);
            while state.pages.len() <= page as usize {
                let [width, height] = self.pages.page_size();
                let id = self.textures.insert(dds::Texture::from_rgba8(Rgba8Image {
                    width,
                    height,
                    pixels: vec![0; self.pages.page_bytes()],
                }));
                state.pages.push(id);
            }
            let pages = self.pages;
            self.textures.modify(state.pages[page as usize], |texture| {
                pages.blit(&mut texture.mips[0][0].data, at, &image.pixels);
            });
            state.cells.insert(key.clone(), Some(cell));
            state.keys.push(key);
        }
        drop(state);
        // A refused file changes no pixels but does change what is drawn: its
        // fallback candidate. Lay the frame out again either way.
        self.textures.bump();
        true
    }
}

/// Decodes one texture file into an uploadable texture.
pub type ImageReader = Arc<dyn Fn(&str) -> io::Result<dds::Texture> + Send + Sync>;

enum ImageSlot {
    Loaded {
        id: usize,
        size: [u32; 2],
        /// The frame this texture was last asked for in.
        used: u64,
    },
    Failed,
}

#[derive(Default)]
struct ImageState {
    slots: HashMap<String, ImageSlot>,
    pending: BTreeSet<String>,
    failures: Vec<String>,
}

/// What [`LazyImages::get`] knows about a texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup {
    Loaded {
        texture: usize,
        size: [u32; 2],
    },
    /// Queued; ask again once [`LazyTextures::version`] moves.
    Pending,
    /// The file cannot be read or decoded.
    Failed,
}

/// Whole textures (portraits) decoded on first use and kept in a bounded
/// least-recently-used set: past `capacity` resident textures, the least
/// recently used are evicted, except that a texture drawn or asked for in the
/// last [`PIN_FRAMES`] frames is pinned. With more than `capacity` on screen,
/// the set grows to what is on screen instead of evicting it.
pub struct LazyImages {
    /// What the textures are, for warnings.
    what: &'static str,
    read: ImageReader,
    capacity: usize,
    state: Mutex<ImageState>,
    loading: Mutex<()>,
    textures: Arc<LazyTextures>,
}

impl LazyImages {
    pub fn new(
        what: &'static str,
        read: ImageReader,
        capacity: usize,
        textures: &Arc<LazyTextures>,
    ) -> Arc<Self> {
        let images = Arc::new(Self {
            what,
            read,
            capacity: capacity.max(1),
            state: Mutex::default(),
            loading: Mutex::default(),
            textures: Arc::clone(textures),
        });
        let weak: Weak<dyn PendingLoad> = Arc::downgrade(&images) as Weak<dyn PendingLoad>;
        textures.register(weak);
        images
    }

    /// The texture of `path`, queued when not decoded yet.
    pub fn get(&self, path: &str) -> Lookup {
        let frame = self.textures.frame();
        let mut state = lock(&self.state);
        match state.slots.get_mut(path) {
            Some(ImageSlot::Loaded { id, size, used }) => {
                *used = frame;
                Lookup::Loaded {
                    texture: *id,
                    size: *size,
                }
            }
            Some(ImageSlot::Failed) => Lookup::Failed,
            None => {
                state.pending.insert(path.to_owned());
                drop(state);
                LazyTextures::schedule(&self.textures);
                Lookup::Pending
            }
        }
    }

    /// What is known of `path`, without queuing it or counting it as used.
    pub fn peek(&self, path: &str) -> Lookup {
        match lock(&self.state).slots.get(path) {
            Some(ImageSlot::Loaded { id, size, .. }) => Lookup::Loaded {
                texture: *id,
                size: *size,
            },
            Some(ImageSlot::Failed) => Lookup::Failed,
            None => Lookup::Pending,
        }
    }

    /// The path a resident texture was decoded from.
    pub fn path_of(&self, texture: usize) -> Option<String> {
        lock(&self.state)
            .slots
            .iter()
            .find_map(|(path, slot)| match slot {
                ImageSlot::Loaded { id, .. } if *id == texture => Some(path.clone()),
                _ => None,
            })
    }

    /// Resident textures.
    #[cfg(any(test, feature = "test-support"))]
    pub fn resident(&self) -> usize {
        lock(&self.state)
            .slots
            .values()
            .filter(|slot| matches!(slot, ImageSlot::Loaded { .. }))
            .count()
    }

    /// Files that could not be decoded so far, with their errors.
    #[cfg(any(test, feature = "test-support"))]
    pub fn failures(&self) -> Vec<String> {
        lock(&self.state).failures.clone()
    }
}

impl PendingLoad for LazyImages {
    fn has_pending(&self) -> bool {
        !lock(&self.state).pending.is_empty()
    }

    fn load_pending(&self) -> bool {
        let _serial = lock(&self.loading);
        let paths: Vec<String> = {
            let mut state = lock(&self.state);
            let pending = std::mem::take(&mut state.pending);
            pending
                .into_iter()
                .filter(|path| !state.slots.contains_key(path))
                .collect()
        };
        if paths.is_empty() {
            return false;
        }
        let decoded = crate::parallel::map(&paths, |path| (self.read)(path));
        let frame = self.textures.frame();
        let mut state = lock(&self.state);
        for (path, texture) in paths.into_iter().zip(decoded) {
            let slot = match texture {
                Ok(texture) => ImageSlot::Loaded {
                    size: [texture.width, texture.height],
                    id: self.textures.insert(texture),
                    used: frame,
                },
                Err(error) => {
                    eprintln!(
                        "Warning: {} {path} cannot be decoded and is not drawn: {error}",
                        self.what
                    );
                    state.failures.push(format!("{path}: {error}"));
                    ImageSlot::Failed
                }
            };
            state.slots.insert(path, slot);
        }
        let evicted = evictions(
            state.slots.iter().filter_map(|(path, slot)| match slot {
                ImageSlot::Loaded { id, used, .. } => Some((
                    path.clone(),
                    (*used).max(self.textures.drawn_at(*id).unwrap_or(0)),
                )),
                ImageSlot::Failed => None,
            }),
            self.capacity,
            frame,
        );
        for path in evicted {
            if let Some(ImageSlot::Loaded { id, .. }) = state.slots.remove(&path) {
                self.textures.remove(id);
            }
        }
        drop(state);
        // A failed texture changes what is drawn (a fallback, or nothing).
        self.textures.bump();
        true
    }
}

/// The least recently used of `resident` (key, last frame drawn or asked
/// for) to evict so at most `capacity` remain, never one used in the last
/// [`PIN_FRAMES`] frames before `frame`.
fn evictions(
    resident: impl Iterator<Item = (String, u64)>,
    capacity: usize,
    frame: u64,
) -> Vec<String> {
    let mut resident: Vec<(u64, String)> = resident.map(|(key, used)| (used, key)).collect();
    if resident.len() <= capacity {
        return Vec::new();
    }
    resident.sort();
    let excess = resident.len() - capacity;
    resident
        .into_iter()
        .take(excess)
        .take_while(|(used, _)| used + PIN_FRAMES <= frame)
        .map(|(_, key)| key)
        .collect()
}

/// Indexes one flag set's files for lazy decoding.
///
/// `finish` runs on every decoded cell (the medium set's cloth overlay, the
/// combat set's shield mask). A file the engine's reader refuses stays out of
/// the atlas, and lookups fall back past it as the engine does.
pub fn flag_set(
    layers: &Arc<ResolvedLayers>,
    set: FlagSet,
    cell: [u32; 2],
    finish: impl Fn(&mut installed_content_image::Rgba8Image) + Send + Sync + 'static,
    textures: &Arc<LazyTextures>,
) -> io::Result<Arc<LazyFlags>> {
    let directory = set.directory();
    let mut names: Vec<&str> = layers
        .paths()
        .filter(|path| {
            path.len() > directory.len()
                && path[..directory.len()].eq_ignore_ascii_case(directory)
                && path.to_ascii_lowercase().ends_with(".tga")
                && !path[directory.len()..].contains('/')
        })
        .collect();
    names.sort_unstable();
    let index = names
        .into_iter()
        .filter_map(|path| {
            let key = path[directory.len()..].strip_suffix(".tga")?;
            Some((key.to_owned(), path.to_owned()))
        })
        .collect();
    let pages = CellPages::plan(cell, page_limit(), directory)?;
    let layers = Arc::clone(layers);
    // A flag the visual mods break is the recording's own, named.
    let recorded = layers.without_overlays();
    let read: FlagReader = Arc::new(move |path: &str| {
        let decoded = |stack: &ResolvedLayers| flags::decode(&stack.read(path).ok()?, cell);
        let mut image = match decoded(&*layers) {
            Some(image) => image,
            None => {
                let image = decoded(recorded.as_ref()?)?;
                layers.note_overlay_refusal(format!(
                    "flag {path} (not an uncompressed Targa of the flag cell's size)"
                ));
                image
            }
        };
        finish(&mut image);
        Some(image)
    });
    Ok(LazyFlags::new(index, pages, read, textures))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell_image(cell: [u32; 2], shade: u8) -> Rgba8Image {
        Rgba8Image {
            width: cell[0],
            height: cell[1],
            pixels: vec![shade; (cell[0] * cell[1] * 4) as usize],
        }
    }

    /// 5,000 synthetic tags with five flag files each.
    fn flag_set(textures: &Arc<LazyTextures>, cell: [u32; 2]) -> Arc<LazyFlags> {
        let groups = ["", "_fascism", "_communism", "_democratic", "_neutrality"];
        let index = (0..5_000)
            .flat_map(|tag| {
                groups.map(|group| {
                    let key = format!("T{tag:04}{group}");
                    (key.clone(), format!("gfx/flags/{key}.tga"))
                })
            })
            .collect();
        let pages = CellPages::plan(cell, 8192, "flags").unwrap();
        let read: FlagReader = Arc::new(move |path: &str| {
            // Every file of tag 13 is unreadable.
            (!path.contains("T0013")).then(|| cell_image(cell, path.len() as u8))
        });
        LazyFlags::new(index, pages, read, textures)
    }

    fn foreground() -> Arc<LazyTextures> {
        let textures = LazyTextures::new();
        textures.set_background(false);
        textures
    }

    #[test]
    fn flags_load_only_the_tags_asked_for_on_as_few_pages_as_they_need() {
        let textures = foreground();
        let cell = [82, 52];
        let flags = flag_set(&textures, cell);
        assert_eq!(flags.len(), 25_000);
        // 300 of 5,000 tags are drawn, each under its ruling group.
        let wanted: Vec<String> = (0..300).map(|tag| format!("T{:04}", tag * 7)).collect();
        for tag in &wanted {
            let group = format!("{tag}_fascism");
            // Nothing is decoded yet: no cell, and the request is queued.
            assert_eq!(flags.resolve(&[&group, tag]), None);
        }
        assert!(textures.settle());
        assert_eq!(flags.loaded(), 300);
        let page_bytes = CellPages::plan(cell, 8192, "flags").unwrap().page_bytes();
        assert_eq!(textures.snapshot().len(), 1);
        assert_eq!(textures.resident_bytes(), page_bytes);
        // Every drawn tag now has its own cell on the one page.
        let mut seen = BTreeSet::new();
        for tag in &wanted {
            let group = format!("{tag}_fascism");
            let (texture, uv) = flags.resolve(&[&group, tag]).expect("decoded");
            assert!(texture >= DYNAMIC_BASE);
            assert!(seen.insert(uv.map(f32::to_bits)));
            assert_eq!(flags.key_at(texture, uv), Some(group));
        }
        // More tags than one page holds take a second page, and no more.
        for tag in 1_000..2_100 {
            flags.get(&format!("T{tag:04}"));
        }
        textures.settle();
        assert_eq!(flags.loaded(), 1_400);
        assert_eq!(textures.snapshot().len(), 2);
    }

    #[test]
    fn an_unreadable_flag_falls_back_to_the_next_candidate() {
        let textures = foreground();
        let flags = flag_set(&textures, [10, 7]);
        // The plain flag is loaded first, then the group flag is asked for.
        flags.get("T0001");
        textures.settle();
        let plain = flags.get("T0001").unwrap();
        // Until the group flag decodes, the plain flag stands in for it.
        assert_eq!(flags.resolve(&["T0001_fascism", "T0001"]), Some(plain));
        textures.settle();
        assert_ne!(flags.resolve(&["T0001_fascism", "T0001"]), Some(plain));
        // Tag 13's files cannot be read: its group flag is skipped, then its plain one.
        assert_eq!(flags.resolve(&["T0013_fascism", "T0013"]), None);
        textures.settle();
        assert!(!flags.contains("T0013_fascism"));
        assert_eq!(flags.resolve(&["T0013_fascism", "T0013"]), None);
        textures.settle();
        assert_eq!(flags.resolve(&["T0013_fascism", "T0013"]), None);
        assert_eq!(flags.rejected(), 2);
        assert!(!textures.has_pending());
    }

    #[test]
    fn a_background_load_wakes_the_window() {
        let textures = LazyTextures::new();
        let flags = flag_set(&textures, [10, 7]);
        let (send, woken) = std::sync::mpsc::channel();
        let send = Mutex::new(send);
        textures.set_wake(move || {
            let _ = lock(&send).send(());
        });
        let before = textures.version();
        assert_eq!(flags.get("T0002"), None);
        woken
            .recv_timeout(TEST_HANG_GUARD)
            .expect("the loader woke the window");
        assert!(textures.version() > before);
        assert!(flags.get("T0002").is_some());
    }

    fn image_set(textures: &Arc<LazyTextures>, capacity: usize) -> Arc<LazyImages> {
        let read: ImageReader = Arc::new(|path: &str| {
            if path.ends_with("broken.dds") {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "broken"));
            }
            Ok(dds::Texture::from_rgba8(Rgba8Image {
                width: 4,
                height: 8,
                pixels: vec![1; 4 * 8 * 4],
            }))
        });
        LazyImages::new("test portrait", read, capacity, textures)
    }

    /// Ends `frames` rendered frames that drew `ids`.
    fn render(textures: &LazyTextures, ids: &[usize], frames: u64) {
        for _ in 0..frames {
            textures.mark_drawn(ids.iter().copied());
        }
    }

    fn loaded(images: &LazyImages, path: &str) -> usize {
        match images.peek(path) {
            Lookup::Loaded { texture, .. } => texture,
            other => panic!("{path} is {other:?}"),
        }
    }

    #[test]
    fn portraits_stay_within_the_lru_bound_outside_the_working_set() {
        let textures = foreground();
        let images = image_set(&textures, 16);
        // 100 frames, each drawing 10 leaders of a 50,000-character roster.
        for frame in 0..100u32 {
            let drawn: Vec<String> = (0..10)
                .map(|slot| format!("gfx/leaders/{}.dds", (frame * 10 + slot) * 37 % 50_000))
                .collect();
            for path in &drawn {
                images.get(path);
            }
            textures.settle();
            let ids: Vec<usize> = drawn.iter().map(|path| loaded(&images, path)).collect();
            render(&textures, &ids, 1);
            // The bound, plus what the pinned frames drew.
            assert!(
                images.resident() <= 16 + PIN_FRAMES as usize * 10,
                "frame {frame}"
            );
            assert_eq!(textures.snapshot().len(), images.resident());
        }
        // Once the older frames unpin, a load trims the set to its bound.
        render(&textures, &[], PIN_FRAMES);
        images.get("gfx/leaders/next.dds");
        textures.settle();
        assert_eq!(images.resident(), 16);
        // A broken file is decided once, never retried, and still moves the
        // version so the frame lays its fallback out at once.
        assert_eq!(images.get("gfx/leaders/broken.dds"), Lookup::Pending);
        let before = textures.version();
        textures.settle();
        assert!(textures.version() > before);
        assert_eq!(images.get("gfx/leaders/broken.dds"), Lookup::Failed);
        assert_eq!(images.failures().len(), 1);
    }

    #[test]
    fn portraits_on_screen_never_thrash_past_the_bound() {
        let textures = foreground();
        let images = image_set(&textures, 4);
        // Ten portraits on screen, more than the bound, laid out once.
        let visible: Vec<String> = (0..10).map(|i| format!("gfx/leaders/v{i}.dds")).collect();
        for path in &visible {
            images.get(path);
        }
        textures.settle();
        let ids: Vec<usize> = visible.iter().map(|path| loaded(&images, path)).collect();
        // Every later frame draws them from its cached layout, without asking
        // again, while other portraits come and go.
        for frame in 0..50 {
            render(&textures, &ids, 1);
            images.get(&format!("gfx/leaders/other{frame}.dds"));
            textures.settle();
            let now: Vec<usize> = visible.iter().map(|path| loaded(&images, path)).collect();
            assert_eq!(now, ids, "frame {frame}: an on-screen portrait was evicted");
        }
        // The set grew to what is on screen, plus the pinned newcomers.
        assert!(images.resident() <= 10 + PIN_FRAMES as usize + 1);
    }

    #[test]
    fn eviction_takes_the_least_recent_outside_the_pinned_frames() {
        let resident =
            [("a", 1), ("b", 5), ("c", 2), ("d", 20)].map(|(key, used)| (key.to_owned(), used));
        assert_eq!(evictions(resident.clone().into_iter(), 2, 20), ["a", "c"]);
        // `b` was used within the last PIN_FRAMES frames of 12: pinned.
        assert_eq!(evictions(resident.clone().into_iter(), 1, 12), ["a", "c"]);
        assert!(evictions(resident.into_iter(), 4, 100).is_empty());
    }

    #[test]
    fn a_refused_flag_moves_the_version() {
        let textures = foreground();
        let flags = flag_set(&textures, [10, 7]);
        assert_eq!(flags.get("T0013"), None);
        let before = textures.version();
        textures.settle();
        assert!(textures.version() > before);
        assert_eq!(flags.rejected(), 1);
    }

    #[test]
    fn an_evicted_id_is_never_reused() {
        let textures = foreground();
        let images = image_set(&textures, 1);
        let Lookup::Pending = images.get("a") else {
            panic!("queued")
        };
        textures.settle();
        let first = loaded(&images, "a");
        render(&textures, &[], PIN_FRAMES);
        images.get("b");
        textures.settle();
        let second = loaded(&images, "b");
        assert_ne!(first, second);
        assert!(textures.snapshot().iter().all(|(id, _)| *id != first));
    }
}
