//! The game facts a recording carries: every covered file that is not art, deduplicated by
//! digest, as one byte stream cut into independently compressed chunks, with each recorded
//! layer's file list, mount order and `replace_path`s. Playback reads facts only from here;
//! art comes from the installed game ([`is_art`]).
//!
//! A pack is `H4FP`, its version, the compressed manifest's length and raw length, the
//! manifest, then the chunks. Localisation is carried for the recorded language only: a file
//! is left out when every language header it has names another language, unless a file at the
//! same path in another layer is carried, so a left-out file never uncovers one below it.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    path::Path,
    sync::{Arc, Mutex},
};

use replay_identity::Identity;
use sha2::{Digest, Sha256};

use super::{
    LayerPath, LayerRole, LayerSource, ResolvedLayer, ResolvedLayers, below, display, invalid,
    physfs_zip, record_digest,
};

const MAGIC: &[u8; 4] = b"H4FP";
const VERSION: u16 = 1;
/// The art rule [`is_art`] applies; a pack names the one it was written under.
const ART_POLICY: u32 = 1;
const CHUNK_BYTES: usize = 2 << 20;
const LEVEL: i32 = 19;
/// Decompressed chunks kept for later reads.
const KEPT_CHUNKS: usize = 24;
/// What a manifest may inflate to: its paths and digests, far above any game's.
const MAX_MANIFEST: usize = 1 << 30;

/// Whether a covered path is art under the current rule: textures, meshes, fonts, shaders,
/// sounds and cursors anywhere, everything below `gfx` but its descriptors and scripts, and
/// the terrain textures and world normal map below `map`.
pub fn is_art(path: &[u8]) -> bool {
    is_art_under(ART_POLICY, path)
}

fn is_art_under(_policy: u32, path: &[u8]) -> bool {
    let lower = path.to_ascii_lowercase();
    if below(&lower, b"gfx") {
        return !(lower.ends_with(b".gfx") || lower.ends_with(b".asset") || lower.ends_with(b".txt"));
    }
    if below(&lower, b"map/terrain") || lower.starts_with(b"map/world_normal.") {
        return true;
    }
    let extension = lower
        .rsplit(|byte| *byte == b'/')
        .next()
        .and_then(|name| name.iter().rposition(|byte| *byte == b'.').map(|at| &name[at..]))
        .unwrap_or_default();
    matches!(
        extension,
        b".dds"
            | b".tga"
            | b".png"
            | b".jpg"
            | b".jpeg"
            | b".mesh"
            | b".anim"
            | b".fnt"
            | b".ttf"
            | b".otf"
            | b".fx"
            | b".fxh"
            | b".shader"
            | b".wav"
            | b".ogg"
            | b".mp3"
            | b".cur"
            | b".ani"
            | b".ico"
            | b".psd"
            | b".pdn"
            | b".bik"
            | b".webm"
            | b".mp4"
    )
}

/// What one recorded layer's facts are in a pack.
#[derive(Debug)]
pub enum LayerFacts {
    /// Each carried path and its file; and for a folder layer, each path by its ASCII
    /// lower case, as a folder lookup ignores case.
    Carried {
        files: BTreeMap<LayerPath, u32>,
        folded: BTreeMap<LayerPath, LayerPath>,
    },
    /// No install matching the recorded layer was found when the facts were gathered.
    Withheld(String),
}

#[derive(Debug)]
pub struct PackLayer {
    pub role: LayerRole,
    pub id: String,
    /// Its recorded source was an archive: lookups are exact, never case-folded.
    pub archive: bool,
    pub hash: [u8; 32],
    pub replace_paths: Vec<LayerPath>,
    /// A DLC descriptor's `name`, which `has_dlc` tests.
    pub dlc_name: Option<String>,
    pub facts: LayerFacts,
}

impl PackLayer {
    /// The carried file a lookup of `path` finds in this layer, by its recorded spelling.
    pub(super) fn find(&self, path: &[u8]) -> io::Result<Option<(&LayerPath, u32)>> {
        match &self.facts {
            LayerFacts::Withheld(reason) => Err(withheld(&self.id, reason)),
            LayerFacts::Carried { files, folded } => {
                let real = if self.archive {
                    files.get_key_value(path).map(|(path, _)| path)
                } else {
                    folded.get(&path.to_ascii_lowercase())
                };
                Ok(real.and_then(|real| files.get_key_value(real)).map(|(path, file)| (path, *file)))
            }
        }
    }

    /// Every carried path, or why there are none.
    pub(super) fn paths(&self) -> io::Result<impl Iterator<Item = &LayerPath>> {
        match &self.facts {
            LayerFacts::Withheld(reason) => Err(withheld(&self.id, reason)),
            LayerFacts::Carried { files, .. } => Ok(files.keys()),
        }
    }

    pub fn withheld(&self) -> Option<&str> {
        match &self.facts {
            LayerFacts::Withheld(reason) => Some(reason),
            LayerFacts::Carried { .. } => None,
        }
    }
}

fn withheld(id: &str, reason: &str) -> io::Error {
    invalid(format!("game facts of {id} are withheld: {reason}"))
}

#[derive(Clone, Copy, Debug)]
struct FileEntry {
    digest: [u8; 32],
    offset: u64,
    length: u64,
}

#[derive(Clone, Copy, Debug)]
struct Chunk {
    /// From the pack's first byte.
    at: u64,
    compressed: u32,
    raw: u32,
    /// The stream offset of its first byte.
    start: u64,
}

/// Where a pack's bytes are read from: the recording, resident or on disk.
pub trait PackSource: Send + Sync {
    fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<()>;
}

impl PackSource for Vec<u8> {
    fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| invalid("fact pack offset"))?;
        let bytes = start
            .checked_add(out.len())
            .and_then(|end| self.get(start..end))
            .ok_or_else(|| invalid("fact pack is truncated"))?;
        out.copy_from_slice(bytes);
        Ok(())
    }
}

/// The facts a recording carries.
pub struct Pack {
    pub language: String,
    pub exe_hash: [u8; 32],
    pub vanilla_hash: [u8; 32],
    pub layers: Vec<PackLayer>,
    /// The executable's tables as their owner encodes them, or why they are withheld.
    pub executable: Result<Vec<u8>, String>,
    /// The pack's size in the recording.
    pub bytes: u64,
    art_policy: u32,
    files: Vec<FileEntry>,
    chunks: Vec<Chunk>,
    source: Arc<dyn PackSource>,
    kept: Mutex<Vec<(usize, Arc<Vec<u8>>)>>,
    tables: std::sync::OnceLock<Result<engine_executable::executable_tables::Tables, String>>,
}

impl std::fmt::Debug for Pack {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Pack")
            .field("language", &self.language)
            .field("layers", &self.layers.len())
            .field("files", &self.files.len())
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl Pack {
    /// The pack of `length` bytes at `offset` in `source`.
    pub fn open(source: Arc<dyn PackSource>, offset: u64, length: u64) -> io::Result<Self> {
        let mut prefix = [0; 14];
        source.read_at(offset, &mut prefix)?;
        if &prefix[..4] != MAGIC {
            return Err(invalid("not a fact pack"));
        }
        let version = u16::from_le_bytes([prefix[4], prefix[5]]);
        if version != VERSION {
            return Err(invalid(format!(
                "fact pack version {version} is not one this build reads (1)"
            )));
        }
        let compressed = u32::from_le_bytes(prefix[6..10].try_into().unwrap()) as usize;
        let raw = u32::from_le_bytes(prefix[10..14].try_into().unwrap()) as usize;
        if 14 + compressed as u64 > length || raw > MAX_MANIFEST {
            return Err(invalid("fact pack manifest lies outside its pack"));
        }
        let mut manifest = vec![0; compressed];
        source.read_at(offset + 14, &mut manifest)?;
        let manifest = zstd::bulk::decompress(&manifest, raw)?;
        let mut input = Input(&manifest);
        let art_policy = input.u32()?;
        if art_policy != ART_POLICY {
            return Err(invalid(format!(
                "fact pack art rule {art_policy} is not one this build reads (1)"
            )));
        }
        let exe_hash = input.digest()?;
        let vanilla_hash = input.digest()?;
        let language = input.string()?;
        let mut layers = Vec::new();
        for _ in 0..input.u32()? {
            let role = LayerRole::from_byte(input.u8()?)?;
            let id = input.string()?;
            let archive = input.u8()? != 0;
            let hash = input.digest()?;
            let mut replace_paths = Vec::new();
            for _ in 0..input.u32()? {
                replace_paths.push(input.bytes()?.to_vec());
            }
            let dlc_name = (input.u8()? != 0).then(|| input.string()).transpose()?;
            let facts = if input.u8()? == 0 {
                let mut files = BTreeMap::new();
                let mut folded = BTreeMap::new();
                for _ in 0..input.u32()? {
                    let path = input.bytes()?.to_vec();
                    let file = input.u32()?;
                    if !archive {
                        folded.insert(path.to_ascii_lowercase(), path.clone());
                    }
                    files.insert(path, file);
                }
                LayerFacts::Carried { files, folded }
            } else {
                LayerFacts::Withheld(input.string()?)
            };
            layers.push(PackLayer {
                role,
                id,
                archive,
                hash,
                replace_paths,
                dlc_name,
                facts,
            });
        }
        let mut files = Vec::new();
        for _ in 0..input.u32()? {
            files.push(FileEntry {
                digest: input.digest()?,
                offset: input.u64()?,
                length: input.u64()?,
            });
        }
        let data = offset + 14 + compressed as u64;
        let mut chunks = Vec::new();
        let mut start = 0;
        let mut at = data;
        for _ in 0..input.u32()? {
            let compressed = input.u32()?;
            let raw = input.u32()?;
            if raw as usize > CHUNK_BYTES {
                return Err(invalid("fact pack chunk is larger than any written"));
            }
            chunks.push(Chunk {
                at,
                compressed,
                raw,
                start,
            });
            at += u64::from(compressed);
            start += u64::from(raw);
        }
        let executable = match input.u8()? {
            0 => Ok(input.bytes()?.to_vec()),
            _ => Err(input.string()?),
        };
        if !input.0.is_empty() || at != offset + length {
            return Err(invalid("fact pack manifest does not match its length"));
        }
        for layer in &layers {
            if let LayerFacts::Carried { files: carried, .. } = &layer.facts
                && carried.values().any(|file| *file as usize >= files.len())
            {
                return Err(invalid("fact pack names a file it does not hold"));
            }
        }
        if files
            .iter()
            .any(|file| file.offset.checked_add(file.length).is_none_or(|end| end > start))
        {
            return Err(invalid("fact pack file lies outside its chunks"));
        }
        Ok(Self {
            language,
            exe_hash,
            vanilla_hash,
            layers,
            executable,
            bytes: length,
            art_policy,
            files,
            chunks,
            source,
            kept: Mutex::new(Vec::new()),
            tables: Default::default(),
        })
    }

    /// A pack withholding every fact of `identity`'s layers for `reason`; `builtins` are the
    /// built-in roots the installed game mounts, whose art is still shown.
    pub fn withheld(identity: &Identity, reason: &str, builtins: Vec<(LayerRole, String)>) -> Self {
        let layer = |role, id: &str, hash| PackLayer {
            role,
            id: id.to_owned(),
            archive: false,
            hash,
            replace_paths: Vec::new(),
            dlc_name: None,
            facts: LayerFacts::Withheld(reason.to_owned()),
        };
        let mut layers = vec![layer(LayerRole::Vanilla, "vanilla", identity.vanilla_hash)];
        let (below, above): (Vec<_>, Vec<_>) = builtins
            .into_iter()
            .partition(|(role, _)| *role != LayerRole::AboveMods);
        for (role, id) in below {
            layers.push(layer(role, &id, [0; 32]));
        }
        for dlc in &identity.dlc {
            layers.push(layer(LayerRole::Dlc, &dlc.id, dlc.hash));
        }
        for recorded in &identity.mods {
            layers.push(layer(LayerRole::Mod, &recorded.id, recorded.hash));
        }
        for (role, id) in above {
            layers.push(layer(role, &id, [0; 32]));
        }
        Self {
            language: identity.language.clone(),
            exe_hash: identity.exe_hash,
            vanilla_hash: identity.vanilla_hash,
            layers,
            executable: Err(reason.to_owned()),
            bytes: 0,
            art_policy: ART_POLICY,
            files: Vec::new(),
            chunks: Vec::new(),
            source: Arc::new(Vec::new()),
            kept: Mutex::new(Vec::new()),
            tables: Default::default(),
        }
    }

    /// Why this pack is not `identity`'s facts, or `None` when it is: the same executable,
    /// base game, and DLC and mods in the same order with the same hashes.
    pub fn binding_differs(&self, identity: &Identity) -> Option<String> {
        if self.exe_hash != identity.exe_hash || self.vanilla_hash != identity.vanilla_hash {
            return Some("the recording's facts belong to another game build or base game".to_owned());
        }
        let dlc = self
            .layers
            .iter()
            .filter(|layer| layer.role == LayerRole::Dlc)
            .map(|layer| (layer.id.as_str(), layer.hash));
        if !dlc.eq(identity.dlc.iter().map(|dlc| (dlc.id.as_str(), dlc.hash))) {
            return Some("the recording's facts name other DLC than it recorded".to_owned());
        }
        let mods = self
            .layers
            .iter()
            .filter(|layer| layer.role == LayerRole::Mod)
            .map(|layer| (layer.id.as_str(), layer.hash));
        if !mods.eq(identity.mods.iter().map(|layer| (layer.id.as_str(), layer.hash))) {
            return Some("the recording's facts name other mods than it recorded".to_owned());
        }
        None
    }

    /// The executable's tables this pack carries, kept with every other build's for work on
    /// its recording to read in its scope ([`engine_executable::executable_tables::scope`]).
    pub fn tables(&self) -> Result<engine_executable::executable_tables::Tables, String> {
        self.tables
            .get_or_init(|| {
                let bytes = self.executable.as_ref().map_err(Clone::clone)?;
                let tables = engine_executable::executable_tables::initialize_recorded(bytes)
                    .map_err(|error| format!("the recorded tables cannot be read: {error}"))?;
                if tables.identity().sha256 != self.exe_hash {
                    return Err("the recorded tables are another executable's".to_owned());
                }
                Ok(tables)
            })
            .clone()
    }

    /// Whether this pack's art rule calls a covered path art.
    pub fn is_art(&self, path: &[u8]) -> bool {
        is_art_under(self.art_policy, path)
    }

    /// The recorded language's name without its `l_`.
    pub fn language_name(&self) -> &str {
        self.language.strip_prefix("l_").unwrap_or(&self.language)
    }

    /// The bytes of a carried file, checked against its recorded digest.
    pub(super) fn read_payload(&self, file: u32) -> io::Result<(Vec<u8>, super::ReadPayload)> {
        let entry = *self
            .files
            .get(file as usize)
            .ok_or_else(|| invalid("fact pack file is absent"))?;
        let mut bytes = Vec::with_capacity(entry.length as usize);
        let end = entry.offset + entry.length;
        let mut index = self.chunks.partition_point(|chunk| chunk.start + u64::from(chunk.raw) <= entry.offset);
        while (bytes.len() as u64) < entry.length {
            let chunk = *self
                .chunks
                .get(index)
                .ok_or_else(|| invalid("fact pack file runs past its chunks"))?;
            let raw = self.chunk(index, chunk)?;
            let from = (entry.offset + bytes.len() as u64 - chunk.start) as usize;
            let to = (end.min(chunk.start + u64::from(chunk.raw)) - chunk.start) as usize;
            bytes.extend_from_slice(&raw[from..to]);
            index += 1;
        }
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        if digest != entry.digest {
            return Err(invalid("a recorded game fact does not match its digest; the recording is damaged"));
        }
        Ok((bytes, super::ReadPayload { length: entry.length, digest }))
    }

    fn chunk(&self, index: usize, chunk: Chunk) -> io::Result<Arc<Vec<u8>>> {
        {
            let mut kept = self.kept.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(at) = kept.iter().position(|(kept, _)| *kept == index) {
                let entry = kept.remove(at);
                let raw = Arc::clone(&entry.1);
                kept.push(entry);
                return Ok(raw);
            }
        }
        let mut compressed = vec![0; chunk.compressed as usize];
        self.source.read_at(chunk.at, &mut compressed)?;
        let raw = zstd::bulk::decompress(&compressed, chunk.raw as usize)?;
        if raw.len() != chunk.raw as usize {
            return Err(invalid("a fact pack chunk is damaged"));
        }
        let raw = Arc::new(raw);
        let mut kept = self.kept.lock().unwrap_or_else(|error| error.into_inner());
        if !kept.iter().any(|(kept, _)| *kept == index) {
            if kept.len() >= KEPT_CHUNKS {
                kept.remove(0);
            }
            kept.push((index, Arc::clone(&raw)));
        }
        Ok(raw)
    }

    /// Carried files and their raw bytes, for reporting.
    pub fn stats(&self) -> (usize, u64) {
        (
            self.files.len(),
            self.chunks.iter().map(|chunk| u64::from(chunk.raw)).sum(),
        )
    }
}

/// What [`write`] gathered.
#[derive(Clone, Debug, Default)]
pub struct WriteStats {
    pub files: usize,
    pub raw_bytes: u64,
    pub pack_bytes: u64,
    pub cached_chunks: usize,
    pub chunks: usize,
}

/// One layer of facts to write: its place in `layers`, or why its facts are withheld.
pub struct Source<'a> {
    pub layers: &'a ResolvedLayers,
    /// Layers of `layers` whose facts cannot be proven, by index, with why.
    pub withheld: BTreeMap<usize, String>,
}

/// A pack of `source`'s facts for a recording of `identity`. Every fact file is read again from
/// its layer and must still have the digest its layer was hashed with. Compressed chunks are
/// kept in `cache_dir` and reused by any later pack that holds the same bytes.
pub fn write(
    source: &Source<'_>,
    identity: &Identity,
    executable: Result<Vec<u8>, String>,
    cache_dir: &Path,
    cancelled: &(dyn Fn() -> bool + Sync),
) -> io::Result<(Vec<u8>, WriteStats)> {
    let language = identity.language.strip_prefix("l_").unwrap_or(&identity.language);
    struct Planned<'a> {
        layer: &'a ResolvedLayer,
        index: usize,
        files: Vec<(LayerPath, [u8; 32])>,
    }
    let mut planned = Vec::new();
    for (index, layer) in source.layers.layers.iter().enumerate() {
        if matches!(layer.role, LayerRole::UserDirectory | LayerRole::Visual) {
            continue;
        }
        let files = layer
            .hashes
            .iter()
            .filter(|(path, _)| super::fact_class(path) && !is_art(path))
            .map(|(path, digest)| (path.clone(), *digest))
            .collect();
        planned.push(Planned {
            layer,
            index,
            files,
        });
    }
    // Every distinct fact file's bytes, read once from the first layer holding it.
    let mut wanted: Vec<(usize, LayerPath, [u8; 32])> = Vec::new();
    let mut seen = BTreeSet::new();
    for (position, plan) in planned.iter().enumerate() {
        if source.withheld.contains_key(&plan.index) {
            continue;
        }
        for (path, digest) in &plan.files {
            if seen.insert(*digest) {
                wanted.push((position, path.clone(), *digest));
            }
        }
    }
    let read = crate::parallel::map(&wanted, |(position, path, digest)| {
        if cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
        }
        let bytes = read_layer_file(planned[*position].layer, path)?;
        let actual: [u8; 32] = Sha256::digest(&bytes).into();
        if actual != *digest {
            return Err(invalid(format!(
                "{} changed while its facts were gathered: {}",
                planned[*position].layer.id,
                display(path)
            )));
        }
        Ok(bytes)
    });
    let mut contents: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
    let mut failed: BTreeMap<usize, String> = BTreeMap::new();
    for ((position, _, digest), bytes) in wanted.iter().zip(read) {
        match bytes {
            Ok(bytes) => {
                contents.insert(*digest, bytes);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
            Err(error) => {
                failed
                    .entry(planned[*position].index)
                    .or_insert_with(|| error.to_string());
            }
        }
    }
    // A layer holding a file no layer could read is withheld with the first failure.
    for plan in &planned {
        if source.withheld.contains_key(&plan.index) || failed.contains_key(&plan.index) {
            continue;
        }
        if plan.files.iter().any(|(_, digest)| !contents.contains_key(digest)) {
            let reason = failed
                .values()
                .next()
                .cloned()
                .unwrap_or_else(|| "a file it holds could not be read".to_owned());
            failed.insert(plan.index, reason);
        }
    }
    let localisation = |path: &[u8]| {
        let lower = path.to_ascii_lowercase();
        below(&lower, b"localisation") && lower.ends_with(b".yml")
    };
    // Localisation of other languages is left out, unless a file at its path is carried.
    let dropped: BTreeSet<[u8; 32]> = planned
        .iter()
        .flat_map(|plan| &plan.files)
        .filter(|(path, digest)| {
            localisation(path)
                && contents
                    .get(digest)
                    .is_some_and(|bytes| other_language_only(bytes, language))
        })
        .map(|(_, digest)| *digest)
        .collect();
    let mut carried_paths = BTreeSet::new();
    for plan in &planned {
        for (path, digest) in &plan.files {
            if localisation(path) && !dropped.contains(digest) {
                carried_paths.insert(path.to_ascii_lowercase());
            }
        }
    }
    let carries = |path: &LayerPath, digest: &[u8; 32]| {
        !(localisation(path) && dropped.contains(digest) && !carried_paths.contains(&path.to_ascii_lowercase()))
    };
    // The stream: each carried file once, in first-use order.
    let mut index_of: HashMap<[u8; 32], u32> = HashMap::new();
    let mut files: Vec<FileEntry> = Vec::new();
    let mut stream_order: Vec<[u8; 32]> = Vec::new();
    let mut offset = 0u64;
    let mut layer_files: Vec<Result<BTreeMap<LayerPath, u32>, String>> = Vec::new();
    for plan in &planned {
        let reason = source
            .withheld
            .get(&plan.index)
            .or_else(|| failed.get(&plan.index));
        if let Some(reason) = reason {
            layer_files.push(Err(reason.clone()));
            continue;
        }
        let mut carried = BTreeMap::new();
        for (path, digest) in &plan.files {
            if !carries(path, digest) {
                continue;
            }
            let file = match index_of.get(digest) {
                Some(file) => *file,
                None => {
                    let length = contents[digest].len() as u64;
                    let file = u32::try_from(files.len()).map_err(|_| invalid("too many fact files"))?;
                    files.push(FileEntry {
                        digest: *digest,
                        offset,
                        length,
                    });
                    stream_order.push(*digest);
                    offset += length;
                    index_of.insert(*digest, file);
                    file
                }
            };
            carried.insert(path.clone(), file);
        }
        layer_files.push(Ok(carried));
    }
    // Cut the stream into chunks.
    let mut chunk_ranges: Vec<Vec<(usize, usize, usize)>> = Vec::new();
    let mut current: Vec<(usize, usize, usize)> = Vec::new();
    let mut filled = 0;
    for (position, digest) in stream_order.iter().enumerate() {
        let length = contents[digest].len();
        let mut at = 0;
        while at < length {
            let take = (CHUNK_BYTES - filled).min(length - at);
            current.push((position, at, at + take));
            at += take;
            filled += take;
            if filled == CHUNK_BYTES {
                chunk_ranges.push(std::mem::take(&mut current));
                filled = 0;
            }
        }
    }
    if !current.is_empty() {
        chunk_ranges.push(current);
    }
    let cache = cache_dir.join("fact-chunks");
    let _ = std::fs::create_dir_all(&cache);
    let compressed = crate::parallel::map(&chunk_ranges, |ranges| {
        if cancelled() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
        }
        let mut raw = Vec::with_capacity(CHUNK_BYTES);
        for (position, from, to) in ranges {
            raw.extend_from_slice(&contents[&stream_order[*position]][*from..*to]);
        }
        let key: [u8; 32] = Sha256::new()
            .chain_update(LEVEL.to_le_bytes())
            .chain_update(&raw)
            .finalize()
            .into();
        let path = cache.join(hex(&key));
        if let Ok(bytes) = std::fs::read(&path)
            && zstd::bulk::decompress(&bytes, raw.len()).is_ok_and(|back| back == raw)
        {
            return Ok((raw.len(), bytes, true));
        }
        let bytes = zstd::bulk::compress(&raw, LEVEL)?;
        let staged = path.with_extension(format!("{}.tmp", std::process::id()));
        if std::fs::write(&staged, &bytes).is_ok() && std::fs::rename(&staged, &path).is_err() {
            let _ = std::fs::remove_file(&staged);
        }
        Ok((raw.len(), bytes, false))
    });
    let compressed = compressed.into_iter().collect::<io::Result<Vec<_>>>()?;
    trim_cache(&cache);
    // The manifest.
    let mut manifest = Vec::new();
    put_u32(&mut manifest, ART_POLICY);
    manifest.extend_from_slice(&identity.exe_hash);
    manifest.extend_from_slice(&identity.vanilla_hash);
    put_bytes(&mut manifest, identity.language.as_bytes())?;
    put_u32(&mut manifest, count(planned.len())?);
    for (plan, carried) in planned.iter().zip(&layer_files) {
        let layer = plan.layer;
        manifest.push(layer.role.byte());
        put_bytes(&mut manifest, layer.id.as_bytes())?;
        manifest.push(u8::from(matches!(layer.source, LayerSource::Archive(_))));
        manifest.extend_from_slice(&layer_identity_hash(layer, identity)?);
        put_u32(&mut manifest, count(layer.replace_paths.len())?);
        for path in &layer.replace_paths {
            put_bytes(&mut manifest, path)?;
        }
        match layer.dlc_name()? {
            Some(name) => {
                manifest.push(1);
                put_bytes(&mut manifest, name.as_bytes())?;
            }
            None => manifest.push(0),
        }
        match carried {
            Ok(carried) => {
                manifest.push(0);
                put_u32(&mut manifest, count(carried.len())?);
                for (path, file) in carried {
                    put_bytes(&mut manifest, path)?;
                    put_u32(&mut manifest, *file);
                }
            }
            Err(reason) => {
                manifest.push(1);
                put_bytes(&mut manifest, reason.as_bytes())?;
            }
        }
    }
    put_u32(&mut manifest, count(files.len())?);
    for file in &files {
        manifest.extend_from_slice(&file.digest);
        manifest.extend_from_slice(&file.offset.to_le_bytes());
        manifest.extend_from_slice(&file.length.to_le_bytes());
    }
    put_u32(&mut manifest, count(compressed.len())?);
    for (raw, bytes, _) in &compressed {
        put_u32(&mut manifest, count(bytes.len())?);
        put_u32(&mut manifest, count(*raw)?);
    }
    match &executable {
        Ok(bytes) => {
            manifest.push(0);
            put_bytes(&mut manifest, bytes)?;
        }
        Err(reason) => {
            manifest.push(1);
            put_bytes(&mut manifest, reason.as_bytes())?;
        }
    }
    let packed_manifest = zstd::bulk::compress(&manifest, LEVEL)?;
    let mut pack = Vec::with_capacity(
        14 + packed_manifest.len() + compressed.iter().map(|(_, bytes, _)| bytes.len()).sum::<usize>(),
    );
    pack.extend_from_slice(MAGIC);
    pack.extend_from_slice(&VERSION.to_le_bytes());
    put_u32(&mut pack, count(packed_manifest.len())?);
    put_u32(&mut pack, count(manifest.len())?);
    pack.extend_from_slice(&packed_manifest);
    for (_, bytes, _) in &compressed {
        pack.extend_from_slice(bytes);
    }
    let stats = WriteStats {
        files: files.len(),
        raw_bytes: offset,
        pack_bytes: pack.len() as u64,
        cached_chunks: compressed.iter().filter(|(_, _, cached)| *cached).count(),
        chunks: compressed.len(),
    };
    Ok((pack, stats))
}

/// The hash a layer has in the recording's identity: a DLC's or mod's own, or for the base
/// game and its built-in roots, which the identity hashes together, each root's own.
fn layer_identity_hash(layer: &ResolvedLayer, identity: &Identity) -> io::Result<[u8; 32]> {
    Ok(match layer.role {
        LayerRole::Dlc => identity
            .dlc
            .iter()
            .find(|dlc| dlc.id == layer.id)
            .map(|dlc| dlc.hash)
            .ok_or_else(|| invalid(format!("DLC {} is not in the recording's identity", layer.id)))?,
        LayerRole::Mod => identity
            .mods
            .iter()
            .find(|layer_mod| layer_mod.id == layer.id)
            .map(|layer_mod| layer_mod.hash)
            .ok_or_else(|| invalid(format!("Mod {} is not in the recording's identity", layer.id)))?,
        _ => super::layer_hash(&layer.hashes)?,
    })
}

/// A file's bytes as its layer holds them: a folder file, or an archive record's content.
fn read_layer_file(layer: &ResolvedLayer, path: &[u8]) -> io::Result<Vec<u8>> {
    match &layer.source {
        LayerSource::Folder(root) => std::fs::read(root.join(super::folder_text(path)?)),
        LayerSource::Archive(archive_path) => {
            let entry = layer
                .files
                .get(path)
                .and_then(|member| member.archive_entry.clone())
                .ok_or_else(|| invalid(format!("{} is not in its archive", display(path))))?;
            let mut archive = physfs_zip::Archive::open(archive_path)?;
            let (_, bytes, _) = record_digest(&mut archive, &entry)?;
            bytes.ok_or_else(|| {
                invalid(format!(
                    "{} is an archive record the game does not open without a password",
                    display(path)
                ))
            })
        }
    }
}

/// Whether a localisation file holds language tables and none of them is `language`'s.
fn other_language_only(bytes: &[u8], language: &str) -> bool {
    let text = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let mut headers = 0;
    for line in text.split(|byte| *byte == b'\n') {
        let line = line.trim_ascii();
        let Some(rest) = line.strip_prefix(b"l_") else {
            continue;
        };
        let name_end = rest
            .iter()
            .position(|byte| !(byte.is_ascii_lowercase() || *byte == b'_'))
            .unwrap_or(rest.len());
        let (name, after) = rest.split_at(name_end);
        let Some(after) = after.strip_prefix(b":") else {
            continue;
        };
        let after = after.trim_ascii_start();
        let after = &after[after.iter().take_while(|byte| byte.is_ascii_digit()).count()..];
        let after = after.trim_ascii();
        if !(after.is_empty() || after.starts_with(b"#")) {
            continue;
        }
        if name == language.as_bytes() {
            return false;
        }
        headers += 1;
    }
    headers > 0
}

/// Keeps the chunk cache to its most recently written gigabyte.
fn trim_cache(cache: &Path) {
    const KEEP: u64 = 1 << 30;
    let Ok(entries) = std::fs::read_dir(cache) else {
        return;
    };
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            Some((metadata.modified().ok()?, metadata.len(), entry.path()))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let mut total = 0;
    for (_, length, path) in files {
        total += length;
        if total > KEEP {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn count(value: usize) -> io::Result<u32> {
    u32::try_from(value).map_err(|_| invalid("fact pack field is too large"))
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> io::Result<()> {
    put_u32(out, count(bytes.len())?);
    out.extend_from_slice(bytes);
    Ok(())
}

struct Input<'a>(&'a [u8]);

impl<'a> Input<'a> {
    fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
        if self.0.len() < count {
            return Err(invalid("fact pack manifest is truncated"));
        }
        let (taken, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(taken)
    }

    fn u8(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn digest(&mut self) -> io::Result<[u8; 32]> {
        Ok(self.take(32)?.try_into().unwrap())
    }

    fn bytes(&mut self) -> io::Result<&'a [u8]> {
        let length = self.u32()? as usize;
        self.take(length)
    }

    fn string(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| invalid("fact pack text is not UTF-8"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn art_is_told_from_facts() {
        assert!(is_art(b"gfx/interface/goals/a.dds"));
        assert!(is_art(b"gfx/fonts/hoi_18.fnt"));
        assert!(is_art(b"map/terrain/colormap_rgb_cityemissivemask_a.dds"));
        assert!(is_art(b"map/world_normal.bmp"));
        assert!(!is_art(b"gfx/interface/goals.gfx"));
        assert!(!is_art(b"gfx/entities/units.asset"));
        assert!(!is_art(b"gfx/posteffect_volumes.txt"));
        assert!(!is_art(b"map/provinces.bmp"));
        assert!(!is_art(b"common/units/infantry.txt"));
        assert!(!is_art(b"interface/topbar.gui"));
        assert!(is_art(b"interface/images/a.dds"));
    }

    #[test]
    fn other_languages_are_told_by_their_headers() {
        assert!(other_language_only(b"\xef\xbb\xbfl_german:\n A:0 \"x\"\n", "english"));
        assert!(!other_language_only(b"l_english:\n A:0 \"x\"\n", "english"));
        assert!(!other_language_only(b"l_german:\n A:0 \"x\"\nl_english:\n B:0 \"y\"\n", "english"));
        assert!(!other_language_only(b"# no header\n", "english"));
        assert!(!other_language_only(b"l_english:0\n A:0 \"x\"\nl_german:\n", "english"));
        assert!(other_language_only(b"l_german:0 # note\n A:0 \"x\"\n", "english"));
    }
}
