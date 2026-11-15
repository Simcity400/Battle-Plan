//! The images replies name. Each PNG is kept once under its content address, and a reply carries
//! only its URL, which the host serves from [`bytes`]. An image is never dropped. A host's table
//! also keeps every image in a folder ([`configure`]): prepared stores keep the URLs their art was
//! drawn as, and a later process serves those names. A disk reply entry carries the images its
//! reply names ([`Table::images`], [`Table::restore`]).

use base64::Engine;
use installed_content_image::Rgba8Image;
use std::{
    collections::{HashMap, HashSet},
    fmt, fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock, PoisonError, RwLock, RwLockReadGuard,
        atomic::{AtomicU64, Ordering},
    },
};

/// The base of an unconfigured table's URLs (tests, command-line tools).
pub const UNCONFIGURED: &str = "artwork:";
/// 32 lowercase hex digits and `.png`.
const NAME_LENGTH: usize = 36;

static SHARED: OnceLock<Table> = OnceLock::new();

/// Name every image under `base` and keep each in `folder` too. Called once by a host, before
/// any image is drawn. Without a usable folder the images stay in memory, served by this process
/// alone, and the folder's error is returned.
pub fn configure(base: &str, folder: io::Result<PathBuf>) -> io::Result<()> {
    let kept = folder.and_then(|folder| fs::create_dir_all(&folder).map(|()| folder));
    let table = match &kept {
        Ok(folder) => Table::kept_in(base, folder),
        Err(_) => Table::new(base),
    };
    SHARED
        .set(table)
        .map_err(|_| io::Error::other("artwork is already configured"))?;
    kept.map(drop)
}

/// The base the desktop app's webview loads images from (its `artwork` scheme).
pub const DESKTOP_BASE: &str = "http://artwork.localhost/";

/// [`configure`] for the desktop app and each viewer process it embeds: the viewer draws its
/// followed panels' images, so it names them as the desktop does and keeps them in the folder
/// the desktop serves from.
pub fn configure_desktop() -> io::Result<()> {
    configure(DESKTOP_BASE, local_folder())
}

/// The folder a host keeps its images in, beside the app's other local data.
pub fn local_folder() -> io::Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| io::Error::other("LOCALAPPDATA is not set"))?,
    )
    .join("HOI4Replay")
    .join("artwork"))
}

/// What every image URL starts with.
pub fn base() -> &'static str {
    &Table::shared().base
}

/// Straight-alpha RGBA as a PNG, by the URL every image reply carries.
pub fn png_url(image: &Rgba8Image) -> io::Result<String> {
    Ok(Table::shared().put(encode(image)?))
}

/// Straight-alpha RGBA as a PNG data URL, for an image drawn once and never shown again (a game
/// window's scene), which the table would keep for the process's life.
pub fn png_data_url(image: &Rgba8Image) -> io::Result<String> {
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(encode(image)?)
    ))
}

/// The PNG named `name` (`<32 lowercase hex digits>.png`), or none.
pub fn bytes(name: &str) -> Option<Arc<[u8]>> {
    Table::shared().bytes(name)
}

/// The PNG a URL [`png_url`] returned names, or none.
pub fn url_bytes(url: &str) -> Option<Arc<[u8]>> {
    Table::shared().url_bytes(url)
}

fn encode(image: &Rgba8Image) -> io::Result<Vec<u8>> {
    let mut encoded = Vec::new();
    let mut encoder = png::Encoder::new(&mut encoded, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&image.pixels)?;
    writer.finish()?;
    Ok(encoded)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Name([u8; 16]);

impl Name {
    fn of(png: &[u8]) -> Self {
        let mut name = [0; 16];
        name.copy_from_slice(&blake3::hash(png).as_bytes()[..16]);
        Self(name)
    }

    fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_suffix(".png")?.as_bytes();
        if hex.len() != 32 {
            return None;
        }
        let digit = |byte: u8| match byte {
            b'0'..=b'9' => Some(byte - b'0'),
            b'a'..=b'f' => Some(byte - b'a' + 10),
            _ => None,
        };
        let mut name = [0; 16];
        for (byte, pair) in name.iter_mut().zip(hex.chunks_exact(2)) {
            *byte = digit(pair[0])? << 4 | digit(pair[1])?;
        }
        Some(Self(name))
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        f.write_str(".png")
    }
}

/// Images by content address, kept for the table's life and, with a folder, for later processes.
pub struct Table {
    base: String,
    images: RwLock<HashMap<Name, Arc<[u8]>>>,
    folder: Option<PathBuf>,
}

/// Distinguishes this process's partly written files.
static WRITES: AtomicU64 = AtomicU64::new(0);

impl Table {
    /// A table naming its images under `base`, in memory alone.
    pub fn new(base: &str) -> Self {
        Self {
            base: base.to_owned(),
            images: RwLock::default(),
            folder: None,
        }
    }

    /// A table naming its images under `base` and keeping each in `folder`.
    pub fn kept_in(base: &str, folder: &Path) -> Self {
        Self {
            folder: Some(folder.to_owned()),
            ..Self::new(base)
        }
    }

    /// This process's table, unconfigured unless a host [`configure`]d it first.
    pub fn shared() -> &'static Self {
        SHARED.get_or_init(|| Self::new(UNCONFIGURED))
    }

    /// The URL of `png`, kept.
    pub fn put(&self, png: Vec<u8>) -> String {
        let name = Name::of(&png);
        self.keep(name, png);
        format!("{}{name}", self.base)
    }

    fn keep(&self, name: Name, png: Vec<u8>) {
        if self.read().contains_key(&name) {
            return;
        }
        // Written whole before it is named, so a reader never finds part of one; a failed write
        // leaves the image served by this process alone.
        if let Some(folder) = &self.folder {
            let path = folder.join(name.to_string());
            if !path.exists() {
                let partial = folder.join(format!(
                    "{name}.{}-{}.part",
                    std::process::id(),
                    WRITES.fetch_add(1, Ordering::Relaxed)
                ));
                if fs::write(&partial, &png).is_err() || fs::rename(&partial, &path).is_err() {
                    let _ = fs::remove_file(&partial);
                }
            }
        }
        self.images
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(name)
            .or_insert_with(|| png.into());
    }

    /// The image named `name`, from memory or, kept by an earlier process, from the folder.
    fn get(&self, name: &Name) -> Option<Arc<[u8]>> {
        if let Some(png) = self.read().get(name) {
            return Some(Arc::clone(png));
        }
        let png = fs::read(self.folder.as_ref()?.join(name.to_string())).ok()?;
        if Name::of(&png) != *name {
            return None;
        }
        Some(Arc::clone(
            self.images
                .write()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(*name)
                .or_insert_with(|| png.into()),
        ))
    }

    fn read(&self) -> RwLockReadGuard<'_, HashMap<Name, Arc<[u8]>>> {
        self.images.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// The PNG named `name`.
    pub fn bytes(&self, name: &str) -> Option<Arc<[u8]>> {
        self.get(&Name::parse(name)?)
    }

    /// The PNG a URL this table returned names.
    pub fn url_bytes(&self, url: &str) -> Option<Arc<[u8]>> {
        self.bytes(url.strip_prefix(&self.base)?)
    }

    /// Each image `text` names under this table's base, once.
    fn named(&self, text: &str) -> HashSet<Name> {
        text.match_indices(self.base.as_str())
            .filter_map(|(at, _)| {
                let start = at + self.base.len();
                text.get(start..start + NAME_LENGTH).and_then(Name::parse)
            })
            .collect()
    }

    /// The PNG of every image `text` names, each once, or none when one is not kept.
    pub fn images(&self, text: &str) -> Option<Vec<Arc<[u8]>>> {
        self.named(text).iter().map(|name| self.get(name)).collect()
    }

    /// Keep each of `pngs` that `text` names, under its content address, and say whether every
    /// image `text` names is then kept.
    pub fn restore(&self, pngs: impl IntoIterator<Item = Vec<u8>>, text: &str) -> bool {
        let named = self.named(text);
        for png in pngs {
            let name = Name::of(&png);
            if named.contains(&name) {
                self.keep(name, png);
            }
        }
        named.iter().all(|name| self.get(name).is_some())
    }
}

#[cfg(test)]
mod tests;
