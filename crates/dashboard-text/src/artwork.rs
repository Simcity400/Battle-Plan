//! The images replies name. Each PNG is kept once, in this process's memory, under its content
//! address, and a reply carries only its URL, which the host serves from [`bytes`]. An image is
//! never dropped, so every URL this process handed out stays served for its life; a disk reply
//! entry carries the images its reply names ([`Table::images`], [`Table::restore`]).

use base64::Engine;
use installed_content_image::Rgba8Image;
use std::{
    collections::{HashMap, HashSet},
    fmt, io,
    sync::{Arc, OnceLock, PoisonError, RwLock, RwLockReadGuard},
};

/// The base of an unconfigured table's URLs (tests, command-line tools).
pub const UNCONFIGURED: &str = "artwork:";
/// 32 lowercase hex digits and `.png`.
const NAME_LENGTH: usize = 36;

static SHARED: OnceLock<Table> = OnceLock::new();

/// Name every image under `base`. Called once by a host, before any image is drawn.
pub fn configure(base: &str) -> io::Result<()> {
    SHARED
        .set(Table::new(base))
        .map_err(|_| io::Error::other("artwork is already configured"))
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

/// Images by content address, kept for the table's life.
pub struct Table {
    base: String,
    images: RwLock<HashMap<Name, Arc<[u8]>>>,
}

impl Table {
    /// A table naming its images under `base`.
    pub fn new(base: &str) -> Self {
        Self {
            base: base.to_owned(),
            images: RwLock::default(),
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
        if !self.read().contains_key(&name) {
            self.images
                .write()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(name)
                .or_insert_with(|| png.into());
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, HashMap<Name, Arc<[u8]>>> {
        self.images.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// The PNG named `name`.
    pub fn bytes(&self, name: &str) -> Option<Arc<[u8]>> {
        self.read().get(&Name::parse(name)?).cloned()
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
        let named = self.named(text);
        let images = self.read();
        named.iter().map(|name| images.get(name).cloned()).collect()
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
        let images = self.read();
        named.iter().all(|name| images.contains_key(name))
    }
}

#[cfg(test)]
mod tests;
