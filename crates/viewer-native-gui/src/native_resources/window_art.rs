use super::*;
use crate::lazy_icons::PendingLoad;
use std::cell::Cell;
use std::sync::{
    Mutex, Once, OnceLock, Weak,
    atomic::{AtomicUsize, Ordering},
};

thread_local! {
    static MISSES: Cell<u64> = const { Cell::new(0) };
    static SYNCHRONOUS: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn window_art_misses() -> u64 {
    MISSES.get()
}

#[must_use]
pub struct SynchronousWindowArt(bool, std::marker::PhantomData<std::rc::Rc<()>>);

impl SynchronousWindowArt {
    pub fn enter() -> Self {
        Self(SYNCHRONOUS.replace(true), std::marker::PhantomData)
    }
}

impl Drop for SynchronousWindowArt {
    fn drop(&mut self) {
        SYNCHRONOUS.set(self.0);
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Sprite,
    Font,
    Texture,
}

struct Slot {
    requested: Once,
    ready: OnceLock<(Option<Box<Item>>, Diagnostics)>,
}

enum Item {
    Sprite(Prepared, Option<AlphaMask>),
    Font(native_text::Font),
    Texture(TextureFile),
}

pub(super) struct WindowArt {
    items: [HashMap<Vec<u8>, Slot>; 3],
    queue: Mutex<Vec<(Kind, Vec<u8>)>>,
    loading: AtomicUsize,
    warnings: Mutex<BTreeSet<String>>,
    reader: Mutex<GuiResources>,
    layers: ResolvedLayers,
    textures: Arc<LazyTextures>,
}

impl WindowArt {
    pub(super) fn new(
        reader: GuiResources,
        layers: ResolvedLayers,
        assets: &Assets,
        textures: &Arc<LazyTextures>,
    ) -> Arc<Self> {
        let items = [&assets.sprites, &assets.fonts, &assets.textures].map(|names| {
            names
                .iter()
                .map(|name| {
                    (
                        name.clone(),
                        Slot {
                            requested: Once::new(),
                            ready: OnceLock::new(),
                        },
                    )
                })
                .collect()
        });
        let art = Arc::new(Self {
            items,
            queue: Mutex::default(),
            loading: AtomicUsize::new(0),
            warnings: Mutex::default(),
            reader: Mutex::new(reader),
            layers,
            textures: Arc::clone(textures),
        });
        textures.register(Arc::downgrade(&art) as Weak<dyn PendingLoad>);
        art
    }

    pub(super) fn loading(&self) -> bool {
        self.loading.load(Ordering::Acquire) != 0
    }

    pub(super) fn warnings(&self) -> Vec<String> {
        self.warnings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    fn get(&self, kind: Kind, name: &[u8], request: bool) -> Option<&Item> {
        let slot = self.items[kind as usize].get(name)?;
        if request {
            slot.requested.call_once(|| {
                self.loading.fetch_add(1, Ordering::Release);
                self.queue
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .push((kind, name.to_vec()));
                if !SYNCHRONOUS.get() {
                    LazyTextures::schedule(&self.textures);
                }
            });
            if slot.ready.get().is_none() && SYNCHRONOUS.get() {
                self.load_pending();
            }
        }
        let Some(ready) = slot.ready.get() else {
            if request {
                MISSES.set(MISSES.get().wrapping_add(1));
            }
            return None;
        };
        let (item, notes) = ready;
        notes.credit();
        item.as_deref()
    }

    pub(super) fn sprite(
        &self,
        name: &[u8],
        request: bool,
    ) -> Option<(&Prepared, Option<&AlphaMask>)> {
        match self.get(Kind::Sprite, name, request)? {
            Item::Sprite(sprite, alpha) => Some((sprite, alpha.as_ref())),
            _ => None,
        }
    }

    pub(super) fn font(&self, name: &[u8], request: bool) -> Option<&native_text::Font> {
        match self.get(Kind::Font, name, request)? {
            Item::Font(font) => Some(font),
            _ => None,
        }
    }

    pub(super) fn texture(&self, name: &[u8]) -> Option<&TextureFile> {
        match self.get(Kind::Texture, name, true)? {
            Item::Texture(texture) => Some(texture),
            _ => None,
        }
    }
}

impl Kind {
    fn load(
        self,
        reader: &mut GuiResources,
        layers: &ResolvedLayers,
        name: &[u8],
    ) -> io::Result<Option<Item>> {
        let mut images = Vec::new();
        Ok(match self {
            Self::Sprite => {
                reader.prepare(layers, name, &mut images)?;
                reader.sprites.remove(name).map(|prepared| {
                    let alpha = reader.alpha.get(&prepared.sheet.texture).cloned();
                    Item::Sprite(prepared, alpha)
                })
            }
            Self::Font => {
                reader.load_font(layers, name, &mut images)?;
                reader.font(name).cloned().map(Item::Font)
            }
            Self::Texture => {
                reader.texture(Stack::Retried, layers, name, &mut images)?;
                reader.texture_file(name).map(Item::Texture)
            }
        })
    }
}

impl PendingLoad for WindowArt {
    fn background_only(&self) -> bool {
        true
    }

    fn has_pending(&self) -> bool {
        self.loading()
    }

    fn load_pending(&self) -> bool {
        if !self.loading() {
            return false;
        }
        let mut reader = self.reader.lock().unwrap_or_else(|err| err.into_inner());
        let batch = std::mem::take(&mut *self.queue.lock().unwrap_or_else(|err| err.into_inner()));
        let count = batch.len();
        if count == 0 {
            return false;
        }
        let mut warnings = Vec::new();
        for (kind, name) in batch {
            let (result, notes) = diagnostics::capture(|| {
                self.layers.artwork("map window art", |layers| {
                    kind.load(&mut reader, layers, &name)
                })
            });
            let item = match result {
                Ok(item) => item,
                Err(error) => {
                    warnings.push(format!(
                        "Map window art {} could not be read: {error}",
                        lossy(&name)
                    ));
                    None
                }
            };
            if matches!(kind, Kind::Font) {
                font_warnings(
                    &name,
                    item.as_ref().and_then(|item| match item {
                        Item::Font(font) => Some(font),
                        _ => None,
                    }),
                    &mut warnings,
                );
            }
            let _ = self.items[kind as usize][&name]
                .ready
                .set((item.map(Box::new), notes));
        }
        warnings.extend(
            reader
                .missing_textures()
                .into_iter()
                .map(|(name, why)| format!("Map window art: {name}: {why}")),
        );
        warnings.extend(std::mem::take(&mut reader.uncovered).iter().map(|path| format!("Map window art: {} is outside the content coverage policy and is not hash-verified.", lossy(path))));
        self.warnings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(warnings);
        self.textures.bump();
        self.loading.fetch_sub(count, Ordering::Release);
        true
    }
}
