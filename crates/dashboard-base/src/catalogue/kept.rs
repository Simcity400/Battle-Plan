
use crate::Source;
use content_reader_layers::content::diagnostics::Diagnostics;
use content_reader_layers::kept::{Input, put_bytes};
use content_reader_gfx::gfx::{SpriteSet, SpriteType};
use content_reader_localisation::localisation::Localisation;
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

const MAGIC: &[u8; 8] = b"H4KEPT\0\x01";
const FORMAT: u32 = 2;
const KEY: usize = 32;
const CHECKSUM: usize = 32;
const PIECE: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Keeping {
    On,
    Off,
}

pub(crate) struct Place {
    path: PathBuf,
    key: [u8; KEY],
}

pub(crate) trait Part: Sized + PartialEq {
    const NAME: &'static str;
    fn keep(&self, out: &mut Vec<u8>) -> io::Result<()>;
    fn read(bytes: &Arc<Vec<u8>>, at: usize) -> io::Result<(Self, usize)>;
}

impl Part for Localisation {
    const NAME: &'static str = "localisation";

    fn keep(&self, out: &mut Vec<u8>) -> io::Result<()> {
        Localisation::keep(self, out);
        Ok(())
    }

    fn read(bytes: &Arc<Vec<u8>>, at: usize) -> io::Result<(Self, usize)> {
        Localisation::from_kept(bytes, at)
    }
}

impl Part for SpriteSet {
    const NAME: &'static str = "sprites";

    fn keep(&self, out: &mut Vec<u8>) -> io::Result<()> {
        SpriteSet::keep(self, out, encode_sprite).map_err(io::Error::other)
    }

    fn read(bytes: &Arc<Vec<u8>>, at: usize) -> io::Result<(Self, usize)> {
        SpriteSet::from_kept(bytes, at, decode_sprite)
    }
}

fn encode_sprite(sprite: &SpriteType) -> Result<Vec<u8>, String> {
    rmp_serde::to_vec(&(&sprite.texture, sprite.frames, &sprite.source_file, &sprite.facts))
        .map_err(|error| error.to_string())
}

fn decode_sprite(name: &str, body: &[u8]) -> Result<SpriteType, String> {
    let (texture, frames, source_file, facts) =
        rmp_serde::from_slice(body).map_err(|error| error.to_string())?;
    Ok(SpriteType {
        name: name.to_owned(),
        texture,
        frames,
        source_file,
        facts,
    })
}

fn build() -> Option<&'static (String, [u8; 24])> {
    static BUILD: OnceLock<Option<(String, [u8; 24])>> = OnceLock::new();
    BUILD
        .get_or_init(|| {
            let executable = std::env::current_exe().ok()?;
            let program = executable.file_stem()?.to_str()?.to_owned();
            let metadata = executable.metadata().ok()?;
            let modified = metadata
                .modified()
                .ok()?
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos();
            let mut stamp = [0; 24];
            stamp[..8].copy_from_slice(&metadata.len().to_le_bytes());
            stamp[8..].copy_from_slice(&modified.to_le_bytes());
            Some((program, stamp))
        })
        .as_ref()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn place<P: Part>(source: &Source, cache_dir: &Path, inputs: &[u8]) -> Option<Place> {
    let replay = source.replay_ref();
    let facts = replay.fact_packs.last()?;
    let pack = source.facts().ok()?;
    if pack.binding_differs(&replay.identity).is_some()
        || pack.layers.iter().any(|layer| layer.withheld().is_some())
    {
        return None;
    }
    let (program, stamp) = build()?;
    let mut key = blake3::Hasher::new();
    key.update(MAGIC);
    key.update(&FORMAT.to_le_bytes());
    for part in [program.as_bytes(), stamp.as_slice(), P::NAME.as_bytes(), inputs] {
        key.update(&(part.len() as u64).to_le_bytes());
        key.update(part);
    }
    key.update(&facts.digest);
    key.update(&facts.raw.to_le_bytes());
    key.update(&facts.stored.to_le_bytes());
    let digest = *key.finalize().as_bytes();
    let file_name = if inputs.is_empty() {
        format!("{}-{}-{program}.bin", hex(&facts.digest), P::NAME)
    } else {
        format!(
            "{}-{}-{}-{program}.bin",
            hex(&facts.digest),
            hex(&digest[..8]),
            P::NAME
        )
    };
    Some(Place {
        path: cache_dir.join("catalogue").join(file_name),
        key: digest,
    })
}

pub(crate) fn kept<P: Part>(
    place: Option<Place>,
    build: impl FnOnce() -> (io::Result<P>, Diagnostics),
) -> (io::Result<P>, Diagnostics) {
    let Some(place) = place else {
        return build();
    };
    if let Some(read) = read(&place) {
        return read;
    }
    #[cfg(test)]
    BUILT.with(|built| built.set(built.get() + 1));
    let (built, diagnostics) = build();
    if let Ok(part) = &built
        && !diagnostics.has_uncovered()
        && let Err(why) = write(&place, part, &diagnostics)
    {
        eprintln!(
            "The content catalogue's {} is not kept in {}: {why}; each new source of the recording builds it again.",
            P::NAME,
            place.path.display()
        );
    }
    (built, diagnostics)
}

fn checksum(bytes: &[u8]) -> [u8; CHECKSUM] {
    let pieces: Vec<&[u8]> = bytes.chunks(PIECE).collect();
    let digests = content_reader_layers::parallel::map(&pieces, |piece| *blake3::hash(piece).as_bytes());
    let mut whole = blake3::Hasher::new();
    whole.update(&(bytes.len() as u64).to_le_bytes());
    for digest in &digests {
        whole.update(digest);
    }
    *whole.finalize().as_bytes()
}

fn read<P: Part>(place: &Place) -> Option<(io::Result<P>, Diagnostics)> {
    let bytes = std::fs::read(&place.path).ok()?;
    let body = bytes.len().checked_sub(CHECKSUM)?;
    if bytes.get(..MAGIC.len())? != MAGIC.as_slice()
        || bytes.get(MAGIC.len()..MAGIC.len() + KEY)? != place.key.as_slice()
        || &bytes[body..] != checksum(&bytes[..body]).as_slice()
    {
        return None;
    }
    let bytes = Arc::new(bytes);
    let mut input = Input::new(&bytes, MAGIC.len() + KEY);
    let diagnostics: Diagnostics = rmp_serde::from_slice(input.bytes().ok()?).ok()?;
    let (part, end) = P::read(&bytes, input.at()).ok()?;
    (end == body).then_some((Ok(part), diagnostics))
}

fn write<P: Part>(place: &Place, part: &P, diagnostics: &Diagnostics) -> io::Result<()> {
    let mut out = Vec::new();
    out.extend(MAGIC);
    out.extend(place.key);
    put_bytes(
        &mut out,
        &rmp_serde::to_vec(diagnostics).map_err(io::Error::other)?,
    );
    let at = out.len();
    part.keep(&mut out)?;
    let out = Arc::new(out);
    if !P::read(&out, at).is_ok_and(|(read, end)| end == out.len() && read == *part) {
        return Err(io::Error::other("it does not read back as it was built"));
    }
    let mut out = Arc::try_unwrap(out).unwrap_or_else(|shared| (*shared).clone());
    let checksum = checksum(&out);
    out.extend(checksum);
    let folder = place
        .path
        .parent()
        .ok_or_else(|| io::Error::other("the cache folder has no parent"))?;
    std::fs::create_dir_all(folder)?;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let temporary = place.path.with_extension(format!(
        "{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&temporary, &out)
        .and_then(|()| std::fs::rename(&temporary, &place.path))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
}

#[cfg(test)]
thread_local! {
    static BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests;
