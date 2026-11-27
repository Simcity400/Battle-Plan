
use base64::Engine;
use content_reader_image::Rgba8Image;
use std::{
    collections::{HashMap, HashSet},
    fmt, fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc, LazyLock, OnceLock, PoisonError, RwLock, RwLockReadGuard,
        atomic::{AtomicU64, Ordering},
    },
};

pub const UNCONFIGURED: &str = "artwork:";
const NAME_LENGTH: usize = 36;

enum Shared {
    Desktop(LazyLock<Table>),
    Configured(Table),
}

static SHARED: OnceLock<Shared> = OnceLock::new();

pub fn configure(base: &str, folder: io::Result<PathBuf>) -> io::Result<()> {
    let (table, result) = table_in(base, folder);
    configure_shared(Shared::Configured(table))?;
    result
}

fn table_in(base: &str, folder: io::Result<PathBuf>) -> (Table, io::Result<()>) {
    let kept = folder.and_then(|folder| fs::create_dir_all(&folder).map(|()| folder));
    let table = match &kept {
        Ok(folder) => Table::kept_in(base, folder),
        Err(_) => Table::new(base),
    };
    (table, kept.map(drop))
}

fn configure_shared(shared: Shared) -> io::Result<()> {
    SHARED.set(shared).map_err(|_| io::Error::other("artwork is already configured"))
}

pub const DESKTOP_BASE: &str = "http://artwork.localhost/";

pub fn prepare_desktop() -> io::Result<()> {
    configure_shared(Shared::Desktop(LazyLock::new(|| {
        let (table, result) = table_in(DESKTOP_BASE, local_folder());
        if let Err(error) = result {
            eprintln!("{error}");
        }
        table
    })))
}

pub fn local_folder() -> io::Result<PathBuf> {
    Ok(PathBuf::from(
        std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| io::Error::other("LOCALAPPDATA is not set"))?,
    )
    .join("BattlePlan")
    .join("artwork"))
}

pub fn base() -> &'static str {
    &Table::shared().base
}

pub fn png_url(image: &Rgba8Image) -> io::Result<String> {
    Ok(Table::shared().put(encode(image)?))
}

pub fn png_data_url(image: &Rgba8Image) -> io::Result<String> {
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(encode(image)?)
    ))
}

pub fn bytes(name: &str) -> Option<Arc<[u8]>> {
    Table::shared().bytes(name)
}

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

pub struct Table {
    base: String,
    images: RwLock<HashMap<Name, Arc<[u8]>>>,
    folder: Option<PathBuf>,
}

static WRITES: AtomicU64 = AtomicU64::new(0);

impl Table {
    pub fn new(base: &str) -> Self {
        Self {
            base: base.to_owned(),
            images: RwLock::default(),
            folder: None,
        }
    }

    pub fn kept_in(base: &str, folder: &Path) -> Self {
        Self {
            folder: Some(folder.to_owned()),
            ..Self::new(base)
        }
    }

    pub fn shared() -> &'static Self {
        match SHARED.get_or_init(|| Shared::Configured(Self::new(UNCONFIGURED))) {
            Shared::Desktop(table) => table,
            Shared::Configured(table) => table,
        }
    }

    pub fn put(&self, png: Vec<u8>) -> String {
        let name = Name::of(&png);
        self.keep(name, png);
        format!("{}{name}", self.base)
    }

    fn keep(&self, name: Name, png: Vec<u8>) {
        if self.read().contains_key(&name) {
            return;
        }
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

    pub fn bytes(&self, name: &str) -> Option<Arc<[u8]>> {
        self.get(&Name::parse(name)?)
    }

    pub fn url_bytes(&self, url: &str) -> Option<Arc<[u8]>> {
        self.bytes(url.strip_prefix(&self.base)?)
    }

    fn named(&self, text: &str) -> HashSet<Name> {
        text.match_indices(self.base.as_str())
            .filter_map(|(at, _)| {
                let start = at + self.base.len();
                text.get(start..start + NAME_LENGTH).and_then(Name::parse)
            })
            .collect()
    }

    pub fn images(&self, text: &str) -> Option<Vec<Arc<[u8]>>> {
        self.named(text).iter().map(|name| self.get(name)).collect()
    }

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
