
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use replay_identity::Identity;
use sha2::{Digest, Sha256};

use super::{
    LayerPath, LayerRole, LayerSource, ResolveOptions, ResolvedLayer, ResolvedLayers, below,
    display, invalid, physfs_zip, record_digest,
};

pub const EQUIPMENT_VERSION: u32 = 1;

#[cfg(any(test, feature = "layer-fixtures"))]
pub const MAGIC_FOR_FIXTURES: &[u8; 4] = MAGIC;
const MAGIC: &[u8; 4] = b"H4FP";
#[cfg(any(test, feature = "layer-fixtures"))]
pub const VERSION_FOR_FIXTURES: u16 = VERSION;
const VERSION: u16 = 2;
#[cfg(any(test, feature = "layer-fixtures"))]
pub const UNEQUIPPED_FOR_FIXTURES: u16 = UNEQUIPPED;
const UNEQUIPPED: u16 = 1;
const ART_POLICY: u32 = 1;
const CHUNK_BYTES: usize = 2 << 20;
const LEVEL: i32 = 19;
const KEPT_CHUNKS: usize = 24;
const MAX_MANIFEST: usize = 1 << 30;

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

#[derive(Debug)]
pub enum LayerFacts {
    Carried {
        files: BTreeMap<LayerPath, u32>,
        folded: BTreeMap<LayerPath, LayerPath>,
    },
    Withheld(String),
}

#[derive(Debug)]
pub struct PackLayer {
    pub role: LayerRole,
    pub id: String,
    pub archive: bool,
    pub hash: [u8; 32],
    pub replace_paths: Vec<LayerPath>,
    pub dlc_name: Option<String>,
    pub facts: LayerFacts,
}

impl PackLayer {
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
    at: u64,
    compressed: u32,
    raw: u32,
    start: u64,
}

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

pub struct Pack {
    pub language: String,
    pub exe_hash: [u8; 32],
    pub vanilla_hash: [u8; 32],
    pub layers: Vec<PackLayer>,
    pub executable: Result<Vec<u8>, String>,
    equipment: Result<u32, String>,
    pub bytes: u64,
    art_policy: u32,
    files: Vec<FileEntry>,
    chunks: Vec<Chunk>,
    source: Arc<dyn PackSource>,
    kept: Mutex<Vec<(usize, Arc<Vec<u8>>)>>,
    keeps: usize,
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
    pub fn open(source: Arc<dyn PackSource>, offset: u64, length: u64) -> io::Result<Self> {
        let mut prefix = [0; 14];
        source.read_at(offset, &mut prefix)?;
        if &prefix[..4] != MAGIC {
            return Err(invalid("not a fact pack"));
        }
        let version = u16::from_le_bytes([prefix[4], prefix[5]]);
        if version != VERSION && version != UNEQUIPPED {
            return Err(invalid(format!(
                "fact pack version {version} is not one this build reads ({UNEQUIPPED} or {VERSION})"
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
        let equipment = match version {
            UNEQUIPPED => Err("the recording's game facts were packed before they carried equipment facts".to_owned()),
            _ => match input.u8()? {
                0 => {
                    let packed = input.u32()?;
                    let held = input.u32()?;
                    if held as usize >= files.len() {
                        return Err(invalid("fact pack names a file it does not hold"));
                    }
                    match packed {
                        EQUIPMENT_VERSION => Ok(held),
                        _ => Err(format!(
                            "the recording's game facts carry equipment facts of version {packed}; this build reads version {}",
                            EQUIPMENT_VERSION
                        )),
                    }
                }
                _ => Err(input.string()?),
            },
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
            equipment,
            bytes: length,
            art_policy,
            files,
            chunks,
            source,
            kept: Mutex::new(Vec::new()),
            keeps: KEPT_CHUNKS,
            tables: Default::default(),
        })
    }

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
            equipment: Err(reason.to_owned()),
            bytes: 0,
            art_policy: ART_POLICY,
            files: Vec::new(),
            chunks: Vec::new(),
            source: Arc::new(Vec::new()),
            kept: Mutex::new(Vec::new()),
            keeps: KEPT_CHUNKS,
            tables: Default::default(),
        }
    }

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

    pub fn keeping(mut self, chunks: usize) -> Self {
        self.keeps = chunks;
        self
    }

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

    pub fn is_art(&self, path: &[u8]) -> bool {
        is_art_under(self.art_policy, path)
    }

    pub fn language_name(&self) -> &str {
        self.language.strip_prefix("l_").unwrap_or(&self.language)
    }

    pub(super) fn read_payload(&self, file: u32) -> io::Result<(Vec<u8>, super::ReadPayload)> {
        let entry = *self
            .files
            .get(file as usize)
            .ok_or_else(|| invalid("fact pack file is absent"))?;
        let mut bytes = Vec::with_capacity(entry.length as usize);
        let mut whole_chunks: Option<std::ops::Range<usize>> = None;
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
            if from == 0 && to == raw.len() {
                whole_chunks.get_or_insert(index..index).end = index + 1;
            }
            index += 1;
        }
        if let Some(whole_chunks) = whole_chunks {
            self.kept.lock().unwrap_or_else(|error| error.into_inner())
                .retain(|(index, _)| !whole_chunks.contains(index));
        }
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        if digest != entry.digest {
            return Err(invalid("a recorded game fact does not match its digest; the recording is damaged"));
        }
        Ok((bytes, super::ReadPayload { length: entry.length, digest }))
    }

    pub(super) fn read_pieces(&self, file: u32, piece: &mut dyn FnMut(&[u8]) -> io::Result<()>) -> io::Result<()> {
        let entry = *self
            .files
            .get(file as usize)
            .ok_or_else(|| invalid("fact pack file is absent"))?;
        let end = entry.offset + entry.length;
        let mut at = entry.offset;
        let mut digest = Sha256::new();
        let mut index = self.chunks.partition_point(|chunk| chunk.start + u64::from(chunk.raw) <= entry.offset);
        while at < end {
            let chunk = *self
                .chunks
                .get(index)
                .ok_or_else(|| invalid("fact pack file runs past its chunks"))?;
            let raw = self.decompressed(index, chunk, false)?;
            let from = (at - chunk.start) as usize;
            let to = (end.min(chunk.start + u64::from(chunk.raw)) - chunk.start) as usize;
            digest.update(&raw[from..to]);
            piece(&raw[from..to])?;
            at += (to - from) as u64;
            index += 1;
        }
        let digest: [u8; 32] = digest.finalize().into();
        if digest != entry.digest {
            return Err(invalid("a recorded game fact does not match its digest; the recording is damaged"));
        }
        Ok(())
    }

    fn chunk(&self, index: usize, chunk: Chunk) -> io::Result<Arc<Vec<u8>>> {
        self.decompressed(index, chunk, true)
    }

    fn decompressed(&self, index: usize, chunk: Chunk, keep: bool) -> io::Result<Arc<Vec<u8>>> {
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
        if !keep || self.keeps == 0 {
            return Ok(raw);
        }
        let mut kept = self.kept.lock().unwrap_or_else(|error| error.into_inner());
        if !kept.iter().any(|(kept, _)| *kept == index) {
            while kept.len() >= self.keeps {
                kept.remove(0);
            }
            kept.push((index, Arc::clone(&raw)));
        }
        Ok(raw)
    }

    pub fn equipment_section(&self) -> Result<Vec<u8>, String> {
        let file = *self.equipment.as_ref().map_err(Clone::clone)?;
        let length = self.files.get(file as usize).map_or(0, |entry| entry.length as usize);
        let mut bytes = Vec::with_capacity(length);
        self.read_pieces(file, &mut |piece| {
            bytes.extend_from_slice(piece);
            Ok(())
        })
        .map_err(|error| format!("the recording's sub-unit needs cannot be read: {error}"))?;
        Ok(bytes)
    }

    pub fn stats(&self) -> (usize, u64) {
        (
            self.files.len(),
            self.chunks.iter().map(|chunk| u64::from(chunk.raw)).sum(),
        )
    }
}

#[derive(Clone, Debug, Default)]
pub struct WriteStats {
    pub files: usize,
    pub raw_bytes: u64,
    pub pack_bytes: u64,
    pub cached_chunks: usize,
    pub chunks: usize,
}

pub struct Source<'a> {
    pub layers: &'a ResolvedLayers,
    pub withheld: BTreeMap<usize, String>,
}

struct Planned<'a> {
    layer: &'a ResolvedLayer,
    index: usize,
    files: Vec<(LayerPath, [u8; 32])>,
}

fn visible_files(planned: &[Planned<'_>], excluded: impl Fn(usize) -> bool) -> Vec<Vec<usize>> {
    let mut visible = vec![Vec::new(); planned.len()];
    let mut higher_folder_exact: HashSet<&[u8]> = HashSet::new();
    let mut higher_archive_exact: HashSet<&[u8]> = HashSet::new();
    let mut higher_replaces: Vec<Vec<u8>> = Vec::new();
    for (position, plan) in planned.iter().enumerate().rev() {
        if excluded(plan.index) {
            continue;
        }
        let is_archive = matches!(plan.layer.source, LayerSource::Archive(_));
        let kept = &mut visible[position];
        kept.reserve(plan.files.len());
        for (file_index, (path, _)) in plan.files.iter().enumerate() {
            let exact = path.as_slice();
            if higher_folder_exact.contains(exact)
                || (is_archive && higher_archive_exact.contains(exact))
            {
                continue;
            }
            if !higher_replaces.is_empty() {
                let lower = path.to_ascii_lowercase();
                if higher_replaces.iter().any(|root| below(&lower, root)) {
                    continue;
                }
            }
            kept.push(file_index);
        }
        if is_archive {
            for (path, _) in &plan.files {
                higher_archive_exact.insert(path.as_slice());
            }
        } else {
            for (path, _) in &plan.files {
                higher_folder_exact.insert(path.as_slice());
            }
        }
        for replace in &plan.layer.replace_paths {
            higher_replaces.push(replace.to_ascii_lowercase());
        }
    }
    visible
}

pub fn write(
    source: &Source<'_>,
    identity: &Identity,
    executable: Result<Vec<u8>, String>,
    cache_dir: &Path,
    cancelled: &(dyn Fn() -> bool + Sync),
    gather_equipment_section: impl FnOnce(&ResolvedLayers) -> Result<Vec<u8>, String>,
) -> io::Result<(Vec<u8>, WriteStats)> {
    let language = identity.language.strip_prefix("l_").unwrap_or(&identity.language);
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
    let mut contents: HashMap<[u8; 32], Vec<u8>> = HashMap::new();
    let mut failed: BTreeMap<usize, String> = BTreeMap::new();
    let visible = loop {
        let visible = visible_files(&planned, |index| {
            source.withheld.contains_key(&index) || failed.contains_key(&index)
        });
        let mut wanted: Vec<(usize, LayerPath, [u8; 32])> = Vec::new();
        let mut seen = BTreeSet::new();
        for (position, indices) in visible.iter().enumerate() {
            let plan = &planned[position];
            if source.withheld.contains_key(&plan.index) || failed.contains_key(&plan.index) {
                continue;
            }
            for &file_index in indices {
                let (path, digest) = &plan.files[file_index];
                if !contents.contains_key(digest) && seen.insert(*digest) {
                    wanted.push((position, path.clone(), *digest));
                }
            }
        }
        if wanted.is_empty() {
            break visible;
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
        let before_failed = failed.len();
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
        for (position, indices) in visible.iter().enumerate() {
            let plan = &planned[position];
            if source.withheld.contains_key(&plan.index) || failed.contains_key(&plan.index) {
                continue;
            }
            if indices
                .iter()
                .any(|&file_index| !contents.contains_key(&plan.files[file_index].1))
            {
                let reason = failed
                    .values()
                    .next()
                    .cloned()
                    .unwrap_or_else(|| "a file it holds could not be read".to_owned());
                failed.insert(plan.index, reason);
            }
        }
        if failed.len() == before_failed {
            break visible;
        }
    };
    let localisation = |path: &[u8]| {
        let lower = path.to_ascii_lowercase();
        below(&lower, b"localisation") && lower.ends_with(b".yml")
    };
    let dropped: BTreeSet<[u8; 32]> = planned
        .iter()
        .zip(&visible)
        .flat_map(|(plan, indices)| indices.iter().map(|&i| &plan.files[i]))
        .filter(|(path, digest)| {
            localisation(path)
                && contents
                    .get(digest)
                    .is_some_and(|bytes| other_language_only(bytes, language))
        })
        .map(|(_, digest)| *digest)
        .collect();
    let mut carried_paths = BTreeSet::new();
    for (plan, indices) in planned.iter().zip(&visible) {
        for &file_index in indices {
            let (path, digest) = &plan.files[file_index];
            if localisation(path) && !dropped.contains(digest) {
                carried_paths.insert(path.to_ascii_lowercase());
            }
        }
    }
    let carries = |path: &LayerPath, digest: &[u8; 32]| {
        !(localisation(path) && dropped.contains(digest) && !carried_paths.contains(&path.to_ascii_lowercase()))
    };
    let mut index_of: HashMap<[u8; 32], u32> = HashMap::new();
    let mut files: Vec<FileEntry> = Vec::new();
    let mut stream_order: Vec<[u8; 32]> = Vec::new();
    let mut offset = 0u64;
    let mut layer_files: Vec<Result<BTreeMap<LayerPath, u32>, String>> = Vec::new();
    for (plan, indices) in planned.iter().zip(&visible) {
        let reason = source
            .withheld
            .get(&plan.index)
            .or_else(|| failed.get(&plan.index));
        if let Some(reason) = reason {
            layer_files.push(Err(reason.clone()));
            continue;
        }
        let mut carried = BTreeMap::new();
        for &file_index in indices {
            let (path, digest) = &plan.files[file_index];
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
    let cached_chunks = compressed.iter().filter(|(_, _, cached)| *cached).count();
    let mut chunks: Vec<(usize, Vec<u8>)> = compressed.into_iter().map(|(raw, bytes, _)| (raw, bytes)).collect();
    drop(contents);
    let mut head = Vec::new();
    put_u32(&mut head, ART_POLICY);
    head.extend_from_slice(&identity.exe_hash);
    head.extend_from_slice(&identity.vanilla_hash);
    put_bytes(&mut head, identity.language.as_bytes())?;
    put_u32(&mut head, count(planned.len())?);
    for (plan, carried) in planned.iter().zip(&layer_files) {
        let layer = plan.layer;
        head.push(layer.role.byte());
        put_bytes(&mut head, layer.id.as_bytes())?;
        head.push(u8::from(matches!(layer.source, LayerSource::Archive(_))));
        head.extend_from_slice(&layer_identity_hash(layer, identity)?);
        put_u32(&mut head, count(layer.replace_paths.len())?);
        for path in &layer.replace_paths {
            put_bytes(&mut head, path)?;
        }
        match layer.dlc_name()? {
            Some(name) => {
                head.push(1);
                put_bytes(&mut head, name.as_bytes())?;
            }
            None => head.push(0),
        }
        match carried {
            Ok(carried) => {
                head.push(0);
                put_u32(&mut head, count(carried.len())?);
                for (path, file) in carried {
                    put_bytes(&mut head, path)?;
                    put_u32(&mut head, *file);
                }
            }
            Err(reason) => {
                head.push(1);
                put_bytes(&mut head, reason.as_bytes())?;
            }
        }
    }
    let unequipped = assemble(&head, &files, &chunks, &executable, &Err("they are being gathered".to_owned()), 1)?;
    let equipment = match gather_equipment(unequipped, identity, gather_equipment_section) {
        Ok(gathered) => {
            if cancelled() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "stopped"));
            }
            let file = count(files.len())?;
            files.push(FileEntry {
                digest: Sha256::digest(&gathered).into(),
                offset,
                length: gathered.len() as u64,
            });
            offset += gathered.len() as u64;
            for piece in gathered.chunks(CHUNK_BYTES) {
                chunks.push((piece.len(), zstd::bulk::compress(piece, LEVEL)?));
            }
            Ok(file)
        }
        Err(why) => Err(why),
    };
    let pack = assemble(&head, &files, &chunks, &executable, &equipment, LEVEL)?;
    let stats = WriteStats {
        files: files.len(),
        raw_bytes: offset,
        pack_bytes: pack.len() as u64,
        cached_chunks,
        chunks: chunks.len(),
    };
    Ok((pack, stats))
}

fn assemble(
    head: &[u8],
    files: &[FileEntry],
    chunks: &[(usize, Vec<u8>)],
    executable: &Result<Vec<u8>, String>,
    equipment: &Result<u32, String>,
    level: i32,
) -> io::Result<Vec<u8>> {
    let mut manifest = head.to_vec();
    put_u32(&mut manifest, count(files.len())?);
    for file in files {
        manifest.extend_from_slice(&file.digest);
        manifest.extend_from_slice(&file.offset.to_le_bytes());
        manifest.extend_from_slice(&file.length.to_le_bytes());
    }
    put_u32(&mut manifest, count(chunks.len())?);
    for (raw, bytes) in chunks {
        put_u32(&mut manifest, count(bytes.len())?);
        put_u32(&mut manifest, count(*raw)?);
    }
    match executable {
        Ok(bytes) => {
            manifest.push(0);
            put_bytes(&mut manifest, bytes)?;
        }
        Err(reason) => {
            manifest.push(1);
            put_bytes(&mut manifest, reason.as_bytes())?;
        }
    }
    match equipment {
        Ok(needs) => {
            manifest.push(0);
            put_u32(&mut manifest, EQUIPMENT_VERSION);
            put_u32(&mut manifest, *needs);
        }
        Err(reason) => {
            manifest.push(1);
            put_bytes(&mut manifest, reason.as_bytes())?;
        }
    }
    let packed_manifest = zstd::bulk::compress(&manifest, level)?;
    let mut pack = Vec::with_capacity(14 + packed_manifest.len() + chunks.iter().map(|(_, bytes)| bytes.len()).sum::<usize>());
    pack.extend_from_slice(MAGIC);
    pack.extend_from_slice(&VERSION.to_le_bytes());
    put_u32(&mut pack, count(packed_manifest.len())?);
    put_u32(&mut pack, count(manifest.len())?);
    pack.extend_from_slice(&packed_manifest);
    for (_, bytes) in chunks {
        pack.extend_from_slice(bytes);
    }
    Ok(pack)
}

fn gather_equipment(
    unequipped: Vec<u8>, identity: &Identity,
    gather: impl FnOnce(&ResolvedLayers) -> Result<Vec<u8>, String>,
) -> Result<Vec<u8>, String> {
    let length = unequipped.len() as u64;
    let pack = Pack::open(Arc::new(unequipped), 0, length).map_err(|error| format!("the pack being written cannot be read back: {error}"))?;
    let tables = pack.tables().map_err(|why| format!("the equipment database needs the executable's tables: {why}"))?;
    let options = ResolveOptions { game_dir: PathBuf::new(), user_dir: PathBuf::new(), cache_dir: PathBuf::new() };
    let content = super::playback(identity, Ok(Arc::new(pack)), &options).map_err(|error| format!("the pack being written cannot be mounted: {error}"))?;
    engine_executable::executable_tables::scope(Some(tables), || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gather(&content.layers))).unwrap_or_else(|panic| {
            let text = panic.downcast_ref::<&str>().map(|text| (*text).to_owned()).or_else(|| panic.downcast_ref::<String>().cloned()).unwrap_or_default();
            Err(format!("the sub-unit needs' reader stopped: {text}"))
        })
    })
}

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

pub fn count(value: usize) -> io::Result<u32> {
    u32::try_from(value).map_err(|_| invalid("fact pack field is too large"))
}

pub fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

pub fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> io::Result<()> {
    put_u32(out, count(bytes.len())?);
    out.extend_from_slice(bytes);
    Ok(())
}

pub struct Input<'a>(pub &'a [u8]);

impl<'a> Input<'a> {
    pub fn take(&mut self, count: usize) -> io::Result<&'a [u8]> {
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

    pub fn u32(&mut self) -> io::Result<u32> {
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

    pub fn string(&mut self) -> io::Result<String> {
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

    fn pack(raws: &[&[u8]], from: u64, to: u64, digest: Option<[u8; 32]>) -> Pack {
        let (mut bytes, mut chunks, mut stream) = (Vec::new(), Vec::new(), Vec::new());
        for raw in raws {
            let compressed = zstd::bulk::compress(raw, 1).unwrap();
            chunks.push(Chunk { at: bytes.len() as u64, compressed: compressed.len() as u32, raw: raw.len() as u32, start: stream.len() as u64 });
            bytes.extend_from_slice(&compressed);
            stream.extend_from_slice(raw);
        }
        let own: [u8; 32] = Sha256::digest(&stream[from as usize..to as usize]).into();
        Pack {
            language: "l_english".into(),
            exe_hash: [0; 32],
            vanilla_hash: [0; 32],
            layers: Vec::new(),
            executable: Err("none".into()),
            equipment: Err("none".into()),
            bytes: bytes.len() as u64,
            art_policy: ART_POLICY,
            files: vec![FileEntry { digest: digest.unwrap_or(own), offset: from, length: to - from }],
            chunks,
            source: Arc::new(bytes),
            kept: Mutex::new(Vec::new()),
            keeps: KEPT_CHUNKS,
            tables: Default::default(),
        }
    }

    #[test]
    fn a_fact_an_older_policy_never_hashed_is_never_read_from_the_installed_game() {
        let root = std::env::temp_dir().join(format!("hoi4_facts_unhashed_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (path, bytes) in [("game/events/installed.txt", b"installed = yes".as_slice()), ("game/gfx/world/art.dds", b"art")] {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        std::fs::create_dir_all(root.join("user")).unwrap();
        let identity = Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 2,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [0; 32],
            verification: replay_identity::Verification::Unchecked,
        };
        let options = ResolveOptions { game_dir: root.join("game"), user_dir: root.join("user"), cache_dir: root.join("cache") };
        let pack = Pack::withheld(&identity, "none gathered", Vec::new());
        let content = super::super::playback(&identity, Ok(Arc::new(pack)), &options).unwrap();
        let read = content.layers.read_uncovered("events/installed.txt").unwrap_err().to_string();
        assert!(read.contains("never hashed"), "{read}");
        let probed = content.layers.has_file("events/installed.txt").unwrap_err().to_string();
        assert!(probed.contains("never hashed"), "{probed}");
        assert_eq!(content.layers.read_uncovered("gfx/world/art.dds").unwrap(), b"art");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_fact_read_in_pieces_is_the_fact_read_whole_and_keeps_no_chunk() {
        let raws: [&[u8]; 3] = [b"0123456789", b"abcdefghij", b"KLMNOPQRST"];
        let read = pack(&raws, 4, 26, None);
        let mut pieces = Vec::new();
        read.read_pieces(0, &mut |piece| {
            pieces.push(piece.to_vec());
            Ok(())
        })
        .unwrap();
        assert_eq!(pieces, [b"456789".to_vec(), b"abcdefghij".to_vec(), b"KLMNOP".to_vec()]);
        assert!(read.kept.lock().unwrap().is_empty(), "a piecewise read keeps no chunk");
        assert_eq!(read.read_payload(0).unwrap().0, pieces.concat());
        assert_eq!(read.kept.lock().unwrap().iter().map(|(index, _)| *index).collect::<Vec<_>>(), [0, 2]);
        assert_eq!(read.read_payload(0).unwrap().0, pieces.concat());
        let aligned = pack(&raws, 10, 20, None);
        assert_eq!(aligned.read_payload(0).unwrap().0, raws[1]);
        assert!(aligned.kept.lock().unwrap().is_empty());
        let unkept = pack(&raws, 4, 26, None).keeping(0);
        assert_eq!(unkept.read_payload(0).unwrap().0, pieces.concat());
        assert!(unkept.kept.lock().unwrap().is_empty());
        let damaged = pack(&raws, 4, 26, Some([7; 32]));
        let refused = damaged.read_pieces(0, &mut |_| Ok(())).unwrap_err().to_string();
        assert!(refused.contains("does not match its digest"), "{refused}");
    }

    #[test]
    fn shadowed_lower_layer_facts_are_omitted_without_changing_playback() {
        let root = std::env::temp_dir().join(format!("hoi4_facts_shadowed_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let base = root.join("base");
        let mod_dir = root.join("mod");
        let cache = root.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let mut vanilla = super::super::fixtures::folder_of(
            "vanilla",
            &base,
            &[
                ("common/kept.txt", b"vanilla kept"),
                ("common/overridden.txt", b"vanilla overridden payload"),
                ("events/replaced_out.txt", b"vanilla replaced payload"),
            ],
        );
        vanilla.role = LayerRole::Vanilla;
        let mut overhaul = super::super::fixtures::folder_of(
            "overhaul",
            &mod_dir,
            &[
                ("common/overridden.txt", b"mod override"),
                ("events/mod_only.txt", b"mod event"),
            ],
        );
        overhaul.role = LayerRole::Mod;
        overhaul.replace_paths = vec![b"events".to_vec()];
        let vanilla_hash = super::super::layer_hash(&vanilla.hashes).unwrap();
        let mod_hash = super::super::layer_hash(&overhaul.hashes).unwrap();
        let layers = ResolvedLayers::from_test_layers(vec![vanilla, overhaul]).unwrap();
        let identity = Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash,
            dlc: Vec::new(),
            mods: vec![replay_identity::Mod {
                id: "overhaul".into(),
                display_name: "Overhaul".into(),
                declared_version: "1.0".into(),
                hash: mod_hash,
            }],
            coverage_policy: 5,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [0; 32],
            verification: replay_identity::Verification::Unchecked,
        };
        let source = Source {
            layers: &layers,
            withheld: BTreeMap::new(),
        };
        let (bytes, stats) = write(
            &source,
            &identity,
            Err("none".to_owned()),
            &cache,
            &|| false,
            |_| Err("none".to_owned()),
        )
        .unwrap();
        assert_eq!(stats.files, 3);
        let opened = Arc::new(Pack::open(Arc::new(bytes), 0, stats.pack_bytes).unwrap());
        assert_eq!(opened.stats().0, 3);
        let options = ResolveOptions {
            game_dir: root.join("empty_game"),
            user_dir: root.join("empty_user"),
            cache_dir: cache.clone(),
        };
        std::fs::create_dir_all(&options.game_dir).unwrap();
        std::fs::create_dir_all(&options.user_dir).unwrap();
        let playback = super::super::playback(&identity, Ok(opened), &options).unwrap();
        assert_eq!(playback.layers.read("common/kept.txt").unwrap(), b"vanilla kept");
        assert_eq!(playback.layers.read("common/overridden.txt").unwrap(), b"mod override");
        assert_eq!(playback.layers.read("events/mod_only.txt").unwrap(), b"mod event");
        assert_eq!(
            playback.layers.read("events/replaced_out.txt").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        let withheld_source = Source {
            layers: &layers,
            withheld: BTreeMap::from([(1, "mod withheld".to_owned())]),
        };
        let (_, withheld_stats) = write(
            &withheld_source,
            &identity,
            Err("none".to_owned()),
            &cache,
            &|| false,
            |_| Err("none".to_owned()),
        )
        .unwrap();
        assert_eq!(withheld_stats.files, 3);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn archive_over_folder_keeps_lower_folder_file_for_case_folded_lookups() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_facts_archive_case_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let base = root.join("base");
        let zip_path = root.join("mod.zip");
        let cache = root.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let mut vanilla = super::super::fixtures::folder_of(
            "vanilla",
            &base,
            &[("common/foo.txt", b"vanilla folder payload")],
        );
        vanilla.role = LayerRole::Vanilla;
        let zip = super::super::fixtures::raw_zip(&[(b"common/foo.txt", 0, b"mod archive payload")]);
        std::fs::write(&zip_path, zip).unwrap();
        let mut archive = super::super::fixtures::archive_layer("archive_mod", &zip_path, &[]);
        archive.role = LayerRole::Mod;
        let vanilla_hash = super::super::layer_hash(&vanilla.hashes).unwrap();
        let mod_hash = super::super::layer_hash(&archive.hashes).unwrap();
        let layers = ResolvedLayers::from_test_layers(vec![vanilla, archive]).unwrap();
        let identity = Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash,
            dlc: Vec::new(),
            mods: vec![replay_identity::Mod {
                id: "archive_mod".into(),
                display_name: "Archive Mod".into(),
                declared_version: "1.0".into(),
                hash: mod_hash,
            }],
            coverage_policy: 5,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [0; 32],
            verification: replay_identity::Verification::Unchecked,
        };
        let source = Source {
            layers: &layers,
            withheld: BTreeMap::new(),
        };
        let (bytes, stats) = write(
            &source,
            &identity,
            Err("none".to_owned()),
            &cache,
            &|| false,
            |_| Err("none".to_owned()),
        )
        .unwrap();
        assert_eq!(stats.files, 2);
        let opened = Arc::new(Pack::open(Arc::new(bytes), 0, stats.pack_bytes).unwrap());
        let options = ResolveOptions {
            game_dir: root.join("empty_game"),
            user_dir: root.join("empty_user"),
            cache_dir: cache,
        };
        std::fs::create_dir_all(&options.game_dir).unwrap();
        std::fs::create_dir_all(&options.user_dir).unwrap();
        let playback = super::super::playback(&identity, Ok(opened), &options).unwrap();
        assert_eq!(
            playback.layers.read("common/foo.txt").unwrap(),
            b"mod archive payload"
        );
        assert_eq!(
            playback.layers.read("common/FOO.txt").unwrap(),
            b"vanilla folder payload"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn folder_over_folder_keeps_case_variant_for_descriptor_listings() {
        let root = std::env::temp_dir().join(format!(
            "hoi4_facts_folder_case_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let base = root.join("base");
        let mod_dir = root.join("mod");
        let cache = root.join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        let mut vanilla = super::super::fixtures::folder_of(
            "vanilla",
            &base,
            &[("common/Foo.txt", b"vanilla payload")],
        );
        vanilla.role = LayerRole::Vanilla;
        let mut addon = super::super::fixtures::folder_of(
            "addon",
            &mod_dir,
            &[("common/foo.txt", b"mod payload")],
        );
        addon.role = LayerRole::Mod;
        let vanilla_hash = super::super::layer_hash(&vanilla.hashes).unwrap();
        let mod_hash = super::super::layer_hash(&addon.hashes).unwrap();
        let layers = ResolvedLayers::from_test_layers(vec![vanilla, addon]).unwrap();
        let expected_descriptors = layers.descriptor_paths("common", &[".txt"]).unwrap();
        let expected_listed = layers.listed_paths("common");
        let identity = Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash,
            dlc: Vec::new(),
            mods: vec![replay_identity::Mod {
                id: "addon".into(),
                display_name: "Addon".into(),
                declared_version: "1.0".into(),
                hash: mod_hash,
            }],
            coverage_policy: 5,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [0; 32],
            verification: replay_identity::Verification::Unchecked,
        };
        let source = Source {
            layers: &layers,
            withheld: BTreeMap::new(),
        };
        let (bytes, stats) = write(
            &source,
            &identity,
            Err("none".to_owned()),
            &cache,
            &|| false,
            |_| Err("none".to_owned()),
        )
        .unwrap();
        assert_eq!(stats.files, 2);
        let opened = Arc::new(Pack::open(Arc::new(bytes), 0, stats.pack_bytes).unwrap());
        let options = ResolveOptions {
            game_dir: root.join("empty_game"),
            user_dir: root.join("empty_user"),
            cache_dir: cache,
        };
        std::fs::create_dir_all(&options.game_dir).unwrap();
        std::fs::create_dir_all(&options.user_dir).unwrap();
        let playback = super::super::playback(&identity, Ok(opened), &options).unwrap();
        assert_eq!(
            playback.layers.descriptor_paths("common", &[".txt"]).unwrap(),
            expected_descriptors
        );
        assert_eq!(playback.layers.listed_paths("common"), expected_listed);
        assert_eq!(playback.layers.read("common/foo.txt").unwrap(), b"mod payload");
        assert_eq!(playback.layers.read("common/Foo.txt").unwrap(), b"mod payload");
        let _ = std::fs::remove_dir_all(&root);
    }
}
