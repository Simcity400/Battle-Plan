
#[cfg(test)]
use viewer_runtime::test_support::TEST_HANG_GUARD;
use job_priority::SpawnCarried as _;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::{
        Arc, Mutex, MutexGuard, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use content_reader_image::{Rgba8Image, dds};

use viewer_gpu::atlas_pages::{CellPages, page_limit};
use content_reader_layers::ResolvedLayers;
use content_reader_gui::flags::{self, FlagSet};

pub const DYNAMIC_BASE: usize = 1 << 40;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

pub trait PendingLoad: Send + Sync {
    fn background_only(&self) -> bool { false }

    fn load_pending(&self) -> bool;
    fn has_pending(&self) -> bool;
}

#[derive(Clone)]
pub struct Resident {
    pub revision: u64,
    pub texture: Arc<dds::Texture>,
    pub cells: Arc<BTreeMap<[u32; 4], u64>>,
}

#[derive(Default)]
struct Registry {
    entries: BTreeMap<usize, Resident>,
    drawn: HashMap<usize, u64>,
    next: usize,
}

pub const PIN_FRAMES: u64 = 8;

type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct LazyTextures {
    registry: Mutex<Registry>,
    version: AtomicU64,
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

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    pub(crate) fn bump(&self) {
        self.version.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn insert(&self, texture: dds::Texture) -> usize {
        let mut registry = lock(&self.registry);
        let id = DYNAMIC_BASE + registry.next;
        registry.next += 1;
        registry.entries.insert(
            id,
            Resident {
                revision: 0,
                texture: Arc::new(texture),
                cells: Default::default(),
            },
        );
        drop(registry);
        self.bump();
        id
    }

    pub(crate) fn image(&self, id: usize) -> Option<Arc<dds::Texture>> {
        lock(&self.registry).entries.get(&id).map(|entry| Arc::clone(&entry.texture))
    }

    fn modify(&self, id: usize, cell: [u32; 4], change: impl FnOnce(&mut dds::Texture)) {
        let mut registry = lock(&self.registry);
        if let Some(entry) = registry.entries.get_mut(&id) {
            change(Arc::make_mut(&mut entry.texture));
            entry.revision += 1;
            Arc::make_mut(&mut entry.cells).insert(cell, entry.revision);
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

    pub fn frame(&self) -> u64 {
        self.frame.load(Ordering::Acquire)
    }

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

    fn drawn_at(&self, id: usize) -> Option<u64> {
        lock(&self.registry).drawn.get(&id).copied()
    }

    pub fn snapshot(&self) -> Vec<(usize, Resident)> {
        lock(&self.registry)
            .entries
            .iter()
            .map(|(id, entry)| (*id, entry.clone()))
            .collect()
    }

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

    pub(crate) fn register(&self, loader: Weak<dyn PendingLoad>) {
        lock(&self.loaders).push(loader);
    }

    fn loaders(&self) -> Vec<Arc<dyn PendingLoad>> {
        lock(&self.loaders)
            .iter()
            .filter_map(Weak::upgrade)
            .collect()
    }

    pub fn set_wake(&self, wake: impl Fn() + Send + Sync + 'static) {
        *lock(&self.wake) = Some(Arc::new(wake));
    }

    pub fn has_wake(&self) -> bool {
        lock(&self.wake).is_some()
    }

    pub fn has_pending(&self) -> bool {
        self.loaders().iter().any(|loader| loader.has_pending())
    }

    pub fn busy(&self) -> bool {
        self.has_pending() || self.running.load(Ordering::Acquire)
    }

    pub fn settle_on_frame(&self) {
        for loader in self.loaders() { if !loader.background_only() { loader.load_pending(); } }
    }

    pub fn settle(&self) -> bool {
        let mut any = false;
        for loader in self.loaders() {
            any |= loader.load_pending();
        }
        any
    }

    pub fn set_background(&self, on: bool) {
        self.background.store(on, Ordering::Release);
    }

    pub fn schedule(this: &Arc<Self>) {
        if !this.background.load(Ordering::Acquire) || this.running.swap(true, Ordering::AcqRel) {
            return;
        }
        let textures = Arc::clone(this);
        let spawned = std::thread::Builder::new()
            .name("lazy icon loader".into())
            .spawn_carried(move || {
                viewer_runtime::worker_priority::enter(viewer_runtime::worker_priority::Class::Normal);
                loop {
                    while textures.settle() {}
                    textures.running.store(false, Ordering::Release);
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
            this.running.store(false, Ordering::Release);
        }
    }
}

pub type FlagReader = Arc<dyn Fn(&str) -> Option<Rgba8Image> + Send + Sync>;

fn fold(ch: char) -> char {
    let mut upper = ch.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(upper), None) => upper,
        _ => ch,
    }
}

fn folded_cmp(folded: &str, key: &str) -> std::cmp::Ordering {
    folded.chars().cmp(key.chars().map(fold))
}

#[derive(Default)]
struct FlagState {
    cells: HashMap<String, Option<u32>>,
    decoded: HashMap<String, Option<Rgba8Image>>,
    keys: Vec<String>,
    pending: BTreeSet<String>,
    pages: Vec<usize>,
    rejected: usize,
}

pub struct LazyFlags {
    index: Vec<(String, String)>,
    pages: CellPages,
    read: FlagReader,
    state: Mutex<FlagState>,
    loading: Mutex<()>,
    warming: Mutex<()>,
    textures: Arc<LazyTextures>,
}

impl LazyFlags {
    pub fn new(
        index: BTreeMap<String, String>,
        pages: CellPages,
        read: FlagReader,
        textures: &Arc<LazyTextures>,
    ) -> Arc<Self> {
        let mut folded: BTreeMap<String, String> = BTreeMap::new();
        for (key, path) in index {
            folded
                .entry(key.chars().map(fold).collect())
                .or_insert(path);
        }
        let flags = Arc::new(Self {
            index: folded.into_iter().collect(),
            pages,
            read,
            state: Mutex::default(),
            loading: Mutex::default(),
            warming: Mutex::default(),
            textures: Arc::clone(textures),
        });
        let weak: Weak<dyn PendingLoad> = Arc::downgrade(&flags) as Weak<dyn PendingLoad>;
        textures.register(weak);
        flags
    }

    pub fn version(&self) -> u64 {
        self.textures.version()
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn cell_size(&self) -> [u32; 2] {
        self.pages.cell
    }

    pub fn rejected(&self) -> usize {
        lock(&self.state).rejected
    }

    pub fn loaded(&self) -> usize {
        lock(&self.state).keys.len()
    }

    fn file(&self, key: &str) -> Option<&str> {
        let at = self
            .index
            .binary_search_by(|(folded, _)| folded_cmp(folded, key))
            .ok()?;
        Some(self.index[at].1.as_str())
    }

    pub fn contains(&self, key: &str) -> bool {
        self.file(key).is_some() && !matches!(lock(&self.state).cells.get(key), Some(None))
    }

    pub fn key_prefix(&self, text: &[char]) -> Option<usize> {
        let mut prefix = String::new();
        let mut found = None;
        for (index, ch) in text.iter().enumerate() {
            prefix.push(fold(*ch));
            let at = self
                .index
                .partition_point(|(folded, _)| folded.as_str() < prefix.as_str());
            let Some((key, _)) = self
                .index
                .get(at)
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

    pub fn resolve(&self, candidates: &[&str]) -> Option<(usize, [f32; 4])> {
        let mut state = lock(&self.state);
        let mut queued = false;
        let mut found = None;
        for key in candidates {
            if self.file(key).is_none() {
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

    pub fn warm<'a>(&self, keys: impl IntoIterator<Item = &'a str>) {
        let _serial = lock(&self.warming);
        let keys: Vec<String> = {
            let state = lock(&self.state);
            keys.into_iter()
                .filter(|key| {
                    self.file(key).is_some()
                        && !state.cells.contains_key(*key)
                        && !state.decoded.contains_key(*key)
                })
                .map(str::to_owned)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        };
        let images = viewer_runtime::parallel::map(&keys, |key| (self.read)(key));
        let mut state = lock(&self.state);
        for (key, image) in keys.into_iter().zip(images) {
            if !state.cells.contains_key(&key) {
                state.decoded.insert(key, image);
            }
        }
    }

    pub fn acquire(&self, candidates: &[&str]) -> Option<(usize, [f32; 4])> {
        let queued = {
            let mut state = lock(&self.state);
            let mut queued = false;
            for key in candidates {
                if self.file(key).is_some() && !state.cells.contains_key(*key) {
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

    pub fn get(&self, key: &str) -> Option<(usize, [f32; 4])> {
        self.resolve(&[key])
    }

    pub fn key_at(&self, texture: usize, uv: [f32; 4]) -> Option<String> {
        let state = lock(&self.state);
        let page = state.pages.iter().position(|page| *page == texture)?;
        let cell = self.pages.cell_at(u32::try_from(page).ok()?, uv)?;
        state.keys.get(cell as usize).cloned()
    }

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
        let (mut images, keys) = {
            let mut state = lock(&self.state);
            let pending = std::mem::take(&mut state.pending);
            let mut images = BTreeMap::new();
            let mut keys = Vec::new();
            for key in pending {
                if state.cells.contains_key(&key) {
                    continue;
                }
                if let Some(image) = state.decoded.remove(&key) {
                    images.insert(key, image);
                } else {
                    keys.push(key);
                }
            }
            (images, keys)
        };
        if images.is_empty() && keys.is_empty() {
            return false;
        }
        let decoded = viewer_runtime::parallel::map(&keys, |key| (self.read)(key));
        images.extend(keys.into_iter().zip(decoded));
        let mut state = lock(&self.state);
        for (key, image) in images {
            state.decoded.remove(&key);
            let Some(image) = image else {
                eprintln!(
                    "Warning: flag {key} (indexed as {}) does not open under that name or is not an uncompressed 24/32-bit {} x {} Targa; the next candidate flag is drawn instead.",
                    self.file(&key).unwrap_or(key.as_str()),
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
            self.textures.modify(
                state.pages[page as usize],
                [at[0], at[1], pages.cell[0], pages.cell[1]],
                |texture| {
                    pages.blit(texture.mips[0][0].data.make_mut(), at, &image.pixels);
                },
            );
            state.cells.insert(key.clone(), Some(cell));
            state.keys.push(key);
        }
        drop(state);
        self.textures.bump();
        true
    }
}

pub type ImageReader = Arc<dyn Fn(&str) -> io::Result<dds::Texture> + Send + Sync>;

enum ImageSlot {
    Loaded {
        id: usize,
        size: [u32; 2],
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup {
    Loaded {
        texture: usize,
        size: [u32; 2],
    },
    Pending,
    Failed,
}

pub struct LazyImages {
    what: &'static str,
    read: ImageReader,
    state: Mutex<ImageState>,
    loading: Mutex<()>,
    textures: Arc<LazyTextures>,
}

impl LazyImages {
    pub fn new(what: &'static str, read: ImageReader, textures: &Arc<LazyTextures>) -> Arc<Self> {
        let images = Arc::new(Self {
            what,
            read,
            state: Mutex::default(),
            loading: Mutex::default(),
            textures: Arc::clone(textures),
        });
        let weak: Weak<dyn PendingLoad> = Arc::downgrade(&images) as Weak<dyn PendingLoad>;
        textures.register(weak);
        images
    }

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

    pub fn acquire(&self, path: &str) -> Lookup {
        let frame = self.textures.frame();
        {
            let mut state = lock(&self.state);
            match state.slots.get_mut(path) {
                Some(ImageSlot::Loaded { id, size, used }) => {
                    *used = frame;
                    return Lookup::Loaded {
                        texture: *id,
                        size: *size,
                    };
                }
                Some(ImageSlot::Failed) => return Lookup::Failed,
                None => {
                    state.pending.insert(path.to_owned());
                }
            }
        }
        self.load_pending();
        self.peek(path)
    }

    pub fn path_of(&self, texture: usize) -> Option<String> {
        lock(&self.state)
            .slots
            .iter()
            .find_map(|(path, slot)| match slot {
                ImageSlot::Loaded { id, .. } if *id == texture => Some(path.clone()),
                _ => None,
            })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn resident(&self) -> usize {
        lock(&self.state)
            .slots
            .values()
            .filter(|slot| matches!(slot, ImageSlot::Loaded { .. }))
            .count()
    }

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
        let decoded = viewer_runtime::parallel::map(&paths, |path| (self.read)(path));
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
            frame,
        );
        for path in evicted {
            if let Some(ImageSlot::Loaded { id, .. }) = state.slots.remove(&path) {
                self.textures.remove(id);
            }
        }
        drop(state);
        self.textures.bump();
        true
    }
}

fn evictions(resident: impl Iterator<Item = (String, u64)>, frame: u64) -> Vec<String> {
    resident
        .filter(|(_, used)| used + PIN_FRAMES <= frame)
        .map(|(key, _)| key)
        .collect()
}

pub fn flag_set(
    layers: &Arc<ResolvedLayers>,
    set: FlagSet,
    cell: [u32; 2],
    finish: impl Fn(&mut content_reader_image::Rgba8Image) + Send + Sync + 'static,
    textures: &Arc<LazyTextures>,
) -> io::Result<Arc<LazyFlags>> {
    let directory = set.directory();
    let mut index = BTreeMap::new();
    for path in layers.art_paths() {
        let Some(name) = path
            .get(..directory.len())
            .filter(|prefix| prefix.eq_ignore_ascii_case(directory.as_bytes()))
            .map(|_| &path[directory.len()..])
        else {
            continue;
        };
        let Some(stem) = name
            .len()
            .checked_sub(4)
            .filter(|&end| name[end..].eq_ignore_ascii_case(b".tga"))
            .map(|end| &name[..end])
        else {
            continue;
        };
        if stem.contains(&b'/') {
            continue;
        }
        let (Ok(key), Ok(path)) = (std::str::from_utf8(stem), std::str::from_utf8(path)) else {
            continue;
        };
        index.insert(key.to_owned(), path.to_owned());
    }
    let pages = CellPages::plan(cell, page_limit(), directory)?;
    let layers = Arc::clone(layers);
    let recorded = layers.without_overlays();
    let read: FlagReader = Arc::new(move |key: &str| {
        let path = format!("{directory}{key}.tga");
        let decoded = |stack: &ResolvedLayers| flags::decode(&stack.read(&path).ok()?, cell);
        let mut image = match decoded(&layers) {
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
        let read: FlagReader = Arc::new(move |key: &str| {
            (!key.contains("T0013")).then(|| cell_image(cell, key.len() as u8))
        });
        LazyFlags::new(index, pages, read, textures)
    }

    fn foreground() -> Arc<LazyTextures> {
        let textures = LazyTextures::new();
        textures.set_background(false);
        textures
    }

    #[test]
    fn cell_revisions_cover_missed_updates_without_changing_old_snapshots() {
        let textures = foreground();
        let id = textures.insert(dds::Texture::from_rgba8(Rgba8Image {
            width: 2,
            height: 1,
            pixels: vec![0; 8],
        }));
        textures.modify(id, [0, 0, 1, 1], |image| image.mips[0][0].data.make_mut()[0] = 1);
        let first = textures.snapshot().pop().unwrap().1;
        textures.modify(id, [1, 0, 1, 1], |image| image.mips[0][0].data.make_mut()[4] = 2);
        textures.modify(id, [0, 0, 1, 1], |image| image.mips[0][0].data.make_mut()[0] = 3);
        let latest = textures.snapshot().pop().unwrap().1;
        let changed: Vec<_> = latest
            .cells
            .iter()
            .filter_map(|(cell, revision)| (*revision > first.revision).then_some(*cell))
            .collect();
        assert_eq!(changed, [[0, 0, 1, 1], [1, 0, 1, 1]]);
        assert_eq!(first.cells.len(), 1);
        assert_eq!(first.texture.base().data[0], 1);
        assert_eq!(latest.texture.base().data[0], 3);
        assert_eq!(latest.texture.base().data[4], 2);
    }

    #[test]
    fn flags_load_only_the_tags_asked_for_on_as_few_pages_as_they_need() {
        let textures = foreground();
        let cell = [82, 52];
        let flags = flag_set(&textures, cell);
        assert_eq!(flags.len(), 25_000);
        let wanted: Vec<String> = (0..300).map(|tag| format!("T{:04}", tag * 7)).collect();
        for tag in &wanted {
            let group = format!("{tag}_fascism");
            assert_eq!(flags.resolve(&[&group, tag]), None);
        }
        assert!(textures.settle());
        assert_eq!(flags.loaded(), 300);
        let page_bytes = CellPages::plan(cell, 8192, "flags").unwrap().page_bytes();
        assert_eq!(textures.snapshot().len(), 1);
        assert_eq!(textures.resident_bytes(), page_bytes);
        let mut seen = BTreeSet::new();
        for tag in &wanted {
            let group = format!("{tag}_fascism");
            let (texture, uv) = flags.resolve(&[&group, tag]).expect("decoded");
            assert!(texture >= DYNAMIC_BASE);
            assert!(seen.insert(uv.map(f32::to_bits)));
            assert_eq!(flags.key_at(texture, uv), Some(group));
        }
        for tag in 1_000..2_100 {
            flags.get(&format!("T{tag:04}"));
        }
        textures.settle();
        assert_eq!(flags.loaded(), 1_400);
        assert_eq!(textures.snapshot().len(), 2);
    }

    #[test]
    fn warming_decides_candidates_without_changing_request_cells_or_reading_again() {
        for cell in [[10, 7], [41, 26], [82, 52]] {
            let textures = foreground();
            let reads = Arc::new(Mutex::new(Vec::new()));
            let calls = Arc::clone(&reads);
            let caller = std::thread::current().id();
            let read: FlagReader = Arc::new(move |key| {
                assert_ne!(std::thread::current().id(), caller);
                lock(&calls).push(key.to_owned());
                (!key.contains("T0013")).then(|| cell_image(cell, key.len() as u8))
            });
            let cold_textures = foreground();
            let cold = flag_set(&cold_textures, cell);
            let flags = LazyFlags::new(
                cold.index.iter().cloned().collect(),
                cold.pages,
                read,
                &textures,
            );
            let candidates = [
                vec!["T0002_fascism", "T0002"],
                vec!["not indexed", "T0001_fascism", "T0001"],
                vec!["T0013_fascism", "T0004"],
                vec!["T0013_fascism", "T0013"],
            ];
            std::thread::scope(|scope| {
                scope.spawn(|| {
                    flags.warm(candidates.iter().rev().flatten().copied());
                    flags.warm(candidates.iter().flatten().copied());
                });
            });
            let before = lock(&reads).clone();
            assert_eq!(before.len(), 7);
            assert_eq!(flags.loaded(), 0);
            assert_eq!(textures.version(), 0);
            assert!(!textures.has_pending());
            assert!(textures.snapshot().is_empty());
            for keys in candidates {
                assert_eq!(flags.acquire(&keys), cold.acquire(&keys));
                assert_eq!(*lock(&reads), before);
                assert_eq!(flags.rejected(), cold.rejected());
                assert_eq!(flags.loaded(), cold.loaded());
                let warmed = textures.snapshot();
                let reference = cold_textures.snapshot();
                assert_eq!(warmed.len(), reference.len());
                for ((id, image), (cold_id, cold_image)) in warmed.iter().zip(&reference) {
                    assert_eq!(id, cold_id);
                    assert_eq!(image.revision, cold_image.revision);
                    assert_eq!(
                        image.texture.mips[0][0].data,
                        cold_image.texture.mips[0][0].data
                    );
                }
            }
            assert!(lock(&flags.state).decoded.is_empty());
            assert!(cold.acquire(&["T0005"]).is_some());
        }
    }

    #[test]
    fn acquisition_does_not_wait_for_warming_and_still_decides_cold_candidates() {
        let textures = foreground();
        let (started, waiting) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let resume = Mutex::new(resume);
        let later_reads = std::sync::atomic::AtomicUsize::new(0);
        let read: FlagReader = Arc::new(move |key| {
            if key == "later" && later_reads.fetch_add(1, Ordering::Relaxed) == 0 {
                started.send(()).unwrap();
                lock(&resume).recv_timeout(TEST_HANG_GUARD).unwrap();
            }
            Some(cell_image([2, 2], 7))
        });
        let flags = LazyFlags::new(
            ["ready", "later"]
                .map(|key| (key.to_owned(), key.to_owned()))
                .into(),
            CellPages::plan([2, 2], 8192, "flags").unwrap(),
            read,
            &textures,
        );
        flags.warm(["ready"]);
        std::thread::scope(|scope| {
            scope.spawn(|| flags.warm(["later"]));
            waiting.recv_timeout(TEST_HANG_GUARD).unwrap();
            assert!(flags.acquire(&["ready"]).is_some());
            assert!(flags.acquire(&["later"]).is_some());
            release.send(()).unwrap();
        });
        assert_eq!(flags.loaded(), 2);
        assert!(lock(&flags.state).decoded.is_empty());
    }

    #[test]
    fn an_unreadable_flag_falls_back_to_the_next_candidate() {
        let textures = foreground();
        let flags = flag_set(&textures, [10, 7]);
        flags.get("T0001");
        textures.settle();
        let plain = flags.get("T0001").unwrap();
        assert_eq!(flags.resolve(&["T0001_fascism", "T0001"]), Some(plain));
        textures.settle();
        assert_ne!(flags.resolve(&["T0001_fascism", "T0001"]), Some(plain));
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

    fn image_set(textures: &Arc<LazyTextures>) -> Arc<LazyImages> {
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
        LazyImages::new("test portrait", read, textures)
    }

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
    fn portraits_stay_resident_only_while_shown() {
        let textures = foreground();
        let images = image_set(&textures);
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
            assert!(
                images.resident() <= (PIN_FRAMES as usize + 1) * 10,
                "frame {frame}"
            );
            assert_eq!(textures.snapshot().len(), images.resident());
        }
        render(&textures, &[], PIN_FRAMES);
        images.get("gfx/leaders/next.dds");
        textures.settle();
        assert_eq!(images.resident(), 1);
        assert_eq!(images.get("gfx/leaders/broken.dds"), Lookup::Pending);
        let before = textures.version();
        textures.settle();
        assert!(textures.version() > before);
        assert_eq!(images.get("gfx/leaders/broken.dds"), Lookup::Failed);
        assert_eq!(images.failures().len(), 1);
    }

    #[test]
    fn portraits_on_screen_are_never_evicted() {
        let textures = foreground();
        let images = image_set(&textures);
        let visible: Vec<String> = (0..10).map(|i| format!("gfx/leaders/v{i}.dds")).collect();
        for path in &visible {
            images.get(path);
        }
        textures.settle();
        let ids: Vec<usize> = visible.iter().map(|path| loaded(&images, path)).collect();
        for frame in 0..50 {
            render(&textures, &ids, 1);
            images.get(&format!("gfx/leaders/other{frame}.dds"));
            textures.settle();
            let now: Vec<usize> = visible.iter().map(|path| loaded(&images, path)).collect();
            assert_eq!(now, ids, "frame {frame}: an on-screen portrait was evicted");
        }
        assert!(images.resident() <= 10 + PIN_FRAMES as usize + 1);
    }

    #[test]
    fn eviction_takes_what_the_pinned_frames_did_not_show() {
        let resident =
            [("a", 1), ("b", 5), ("c", 2), ("d", 20)].map(|(key, used)| (key.to_owned(), used));
        assert_eq!(evictions(resident.clone().into_iter(), 20), ["a", "b", "c"]);
        assert_eq!(evictions(resident.clone().into_iter(), 12), ["a", "c"]);
        assert!(evictions(resident.into_iter(), 2).is_empty());
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
        let images = image_set(&textures);
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

    #[test]
    fn acquire_decodes_immediately_and_caches_loaded_and_failed_images() {
        let textures = foreground();
        let reads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let calls = Arc::clone(&reads);
        let read: ImageReader = Arc::new(move |path: &str| {
            calls.fetch_add(1, Ordering::Relaxed);
            if path.ends_with("broken.dds") {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "broken"));
            }
            Ok(dds::Texture::from_rgba8(Rgba8Image {
                width: 6,
                height: 4,
                pixels: vec![1; 6 * 4 * 4],
            }))
        });
        let images = LazyImages::new("test sprite", read, &textures);
        let first = images.acquire("gfx/good.dds");
        assert!(matches!(first, Lookup::Loaded { size: [6, 4], .. }));
        assert_eq!(images.acquire("gfx/good.dds"), first);
        assert_eq!(images.acquire("gfx/broken.dds"), Lookup::Failed);
        assert_eq!(images.acquire("gfx/broken.dds"), Lookup::Failed);
        assert_eq!(reads.load(Ordering::Relaxed), 2);
    }

    fn tga(cell: [u32; 2], shade: u8) -> Vec<u8> {
        let mut bytes = vec![0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        bytes.extend((cell[0] as u16).to_le_bytes());
        bytes.extend((cell[1] as u16).to_le_bytes());
        bytes.extend([32, 8]);
        bytes.extend(std::iter::repeat_n(shade, (cell[0] * cell[1] * 4) as usize));
        bytes
    }

    #[test]
    fn flags_open_as_the_engine_requests_them_beside_unrelated_unicode_paths() {
        use content_reader_layers::test_support::{Record, zip_records};
        let root = std::env::temp_dir().join(format!(
            "hoi4_viewer_flag_index_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let (base, archive) = (root.join("base"), root.join("flags.zip"));
        let cell = [2, 2];
        for (path, bytes) in [
            ("common/abé.txt", b"x".to_vec()),
            ("gfx/flags/GER.TGA", tga(cell, 1)),
            ("gfx/flags/sov_communism.tga", tga(cell, 2)),
            ("gfx/flags/nested/ITA.tga", tga(cell, 3)),
        ] {
            let file = base.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, bytes).unwrap();
        }
        let ita = tga(cell, 4);
        let record = Record {
            name: b"gfx/flags/ita.tga",
            host: 0,
            flags: 0,
            deflate: false,
            data: &ita,
        };
        std::fs::write(&archive, zip_records(b"", &[record])).unwrap();
        let layers = Arc::new(
            ResolvedLayers::from_folders(&[(&base, &[][..]), (&archive, &[][..])]).unwrap(),
        );
        assert!(
            layers
                .paths()
                .unwrap()
                .any(|path| path == "common/abé.txt".as_bytes())
        );
        let textures = foreground();
        let flags = super::flag_set(&layers, FlagSet::Normal, cell, |_| {}, &textures).unwrap();
        assert_eq!(flags.len(), 3);
        let (texture, uv) = flags.acquire(&["GER"]).expect("folder flag GER.TGA");
        assert_eq!(flags.key_at(texture, uv).as_deref(), Some("GER"));
        let (texture, uv) = flags
            .acquire(&["SOV_communism", "SOV"])
            .expect("folder flag sov_communism.tga");
        assert_eq!(flags.key_at(texture, uv).as_deref(), Some("SOV_communism"));
        let text: Vec<char> = "SOV_communism army".chars().collect();
        assert_eq!(flags.key_prefix(&text), Some(13));
        assert!(flags.contains("ITA"));
        assert_eq!(flags.acquire(&["ITA"]), None);
        assert!(!flags.contains("ITA"));
        assert_eq!(flags.rejected(), 1);
        assert!(flags.acquire(&["ita"]).is_some());
        assert_eq!(flags.loaded(), 3);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
