use super::*;
use crate::lazy_icons::PendingLoad;
use std::sync::{Mutex, Once, OnceLock, Weak, atomic::{AtomicUsize, AtomicU64, Ordering}};

#[derive(Clone, Copy)]
enum Kind { Sprite, Font, Texture }

struct Slot {
    requested: Once,
    ready: OnceLock<Option<(Box<Item>, Diagnostics)>>,
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
    missed: AtomicU64,
    reader: Mutex<GuiResources>,
    layers: ResolvedLayers,
    textures: Arc<LazyTextures>,
}

impl WindowArt {
    pub(super) fn new(reader: GuiResources, layers: ResolvedLayers, assets: &Assets, textures: &Arc<LazyTextures>) -> Arc<Self> {
        let items = [&assets.sprites, &assets.fonts, &assets.textures].map(|names| {
            names.iter().map(|name| (name.clone(), Slot { requested: Once::new(), ready: OnceLock::new() })).collect()
        });
        let art = Arc::new(Self {
            items, queue: Mutex::default(), loading: AtomicUsize::new(0), missed: AtomicU64::new(0), reader: Mutex::new(reader), layers, textures: Arc::clone(textures),
        });
        textures.register(Arc::downgrade(&art) as Weak<dyn PendingLoad>);
        art
    }

    pub(super) fn loading(&self) -> bool { self.loading.load(Ordering::Acquire) != 0 }

    pub(super) fn resolve<T>(&self, mut read: impl FnMut() -> T) -> T {
        loop {
            let before = self.missed.load(Ordering::Relaxed);
            let value = read();
            if before == self.missed.load(Ordering::Relaxed) { return value; }
            self.load_pending();
        }
    }

    fn get(&self, kind: Kind, name: &[u8]) -> Option<&Item> {
        let slot = self.items[kind as usize].get(name)?;
        slot.requested.call_once(|| {
            self.loading.fetch_add(1, Ordering::Release);
            self.queue.lock().unwrap_or_else(|err| err.into_inner()).push((kind, name.to_vec()));
            LazyTextures::schedule(&self.textures);
        });
        let Some(ready) = slot.ready.get() else {
            self.missed.fetch_add(1, Ordering::Relaxed);
            return None;
        };
        let (item, notes) = ready.as_ref()?;
        notes.credit();
        Some(item)
    }

    pub(super) fn sprite(&self, name: &[u8]) -> Option<(&Prepared, Option<&AlphaMask>)> {
        match self.get(Kind::Sprite, name)? {
            Item::Sprite(sprite, alpha) => Some((sprite, alpha.as_ref())),
            _ => None,
        }
    }

    pub(super) fn font(&self, name: &[u8]) -> Option<&native_text::Font> {
        match self.get(Kind::Font, name)? {
            Item::Font(font) => Some(font),
            _ => None,
        }
    }

    pub(super) fn texture(&self, name: &[u8]) -> Option<&TextureFile> {
        match self.get(Kind::Texture, name)? {
            Item::Texture(texture) => Some(texture),
            _ => None,
        }
    }
}

impl Kind {
    fn load(self, reader: &mut GuiResources, layers: &ResolvedLayers, name: &[u8]) -> io::Result<Option<Item>> {
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
    fn has_pending(&self) -> bool { self.loading() }

    fn load_pending(&self) -> bool {
        if !self.loading() { return false; }
        let mut reader = self.reader.lock().unwrap_or_else(|err| err.into_inner());
        let batch = std::mem::take(&mut *self.queue.lock().unwrap_or_else(|err| err.into_inner()));
        let changed = !batch.is_empty();
        for (kind, name) in batch {
            let (result, notes) = diagnostics::capture(|| self.layers.artwork("map window art", |layers| kind.load(&mut reader, layers, &name)));
            let item = match result {
                Ok(item) => item,
                Err(error) => {
                    eprintln!("Map window art {} could not be read: {error}", lossy(&name));
                    None
                }
            };
            let _ = self.items[kind as usize][&name].ready.set(item.map(|item| (Box::new(item), notes)));
            self.textures.bump();
            self.loading.fetch_sub(1, Ordering::Release);
        }
        changed
    }
}
