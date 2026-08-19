use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

pub mod canon;
pub mod canon_to_json;
pub mod canon_value;
pub mod container;
pub mod design_catalog;
mod dir_store;
pub mod pack;
pub mod prefolded;
pub mod prefolded_decoder;
pub mod recording_composition;
pub mod replay_cache;
pub mod replay_frame_identity;
pub mod sha256;
pub mod stream_verify;
pub mod strict_json;
pub mod zip;

pub use canon_to_json::{to_value as canon_to_value, CanonToValueError};
pub use container::{Container, ContainerError, ExpectedIdentity, SpoolEntryReceipt};
pub use pack::{
    pack, pack_borrowed, pack_borrowed_to_writer, pack_borrowed_with, pack_borrowed_with_writer,
    pack_mixed_to_writer, pack_ordered_streaming_to_writer_with_options, pack_streaming_to_writer,
    pack_streaming_zstd_to_writer, pack_streaming_zstd_to_writer_with_profile, pack_with,
    BorrowedPackEntry, CompressionProfile, FileBackedPackEntry, OrderedStreamingPackEntry,
    PackEntry, PackError, PackIdentity, PackOptions, StreamingFileEntry, VerifiedPackEntry,
    CONTAINER_SCHEMA_VERSION,
};
pub use prefolded::{
    PrefoldedHourEntry, PrefoldedHourIndex, PrefoldedReplay, PrefoldedReplayError,
};
pub use prefolded_decoder::{
    decode_certified_prefolded, decode_certified_prefolded_spool, CertifiedPrefoldDecoder,
    CertifiedPrefoldRequest, CertifiedPrefoldSpoolDecoder, CertifiedPrefoldSpoolRequest,
    CertifiedPrefoldedReplay, CertifiedPrefoldedSpool, PrefoldDecodeError, PrefoldSourceReceipt,
    PrefoldStreamIdentityMode, PrefoldedSpool, PrefoldedSpoolEntry, COMPACT_STREAM_ROLE,
};
pub use recording_composition::{
    container_raw_stream_packaging, raw_stream_unavailable_message, read_raw_stream_packaging,
    read_raw_stream_packaging_with_legacy_default, CompositionError, RawStreamPackaging,
    RAW_STREAM_OMITTED_REASON, RAW_STREAM_PACKAGING_FIELD,
};
pub use replay_cache::{
    validate_checkpoint_cache_header, CacheAudit, CacheAuditVerdict, CacheBuildHour,
    CacheBuildHourMeta, CacheIdentityMode, CacheTruncation, CanonicalWorldHour, CertifiedCacheFile,
    CertifiedReplayCache, ConfinedHour, FrameOutcome, ReplayCacheEncoder, ReplayCacheError,
    ReplayCacheReader, ReplayExcludedHour, ReplayHourEntry, ReplayHourIndex, ReplaySeekReceipt,
    ScrubOutcome, CHECKPOINT_CACHE_FILE, CHECKPOINT_CACHE_HEADER_LEN, CHECKPOINT_CACHE_ROLE,
    DEFAULT_CHECKPOINT_INTERVAL_HOURS, HOUR_INDEX_FILE, HOUR_INDEX_ROLE, MAX_CACHE_RECORD_BYTES,
    MAX_CHECKPOINT_INTERVAL_HOURS, PUBLISHED_COVERAGE_FILE, PUBLISHED_COVERAGE_SCHEMA,
};
pub use stream_verify::{
    verify_replay_container_streaming, verify_replay_container_streaming_file,
    verify_replay_container_streaming_file_at_path,
    verify_replay_container_streaming_file_at_path_with_policy, StreamEntryPolicy,
    StreamVerifyReport,
};
pub use strict_json::from_slice as parse_json_strict;

pub(crate) fn decode_json_strict<T: DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, serde_json::Error> {
    serde_json::from_value(strict_json::from_slice(bytes)?)
}

#[derive(Debug, Clone, Deserialize)]
pub struct Index {
    pub game_id: String,
    #[serde(default)]
    pub schema_version: Option<Value>,
    #[serde(default)]
    pub index_schema: Option<String>,
    #[serde(default)]
    pub segments_file: Option<String>,
    #[serde(default)]
    pub main_head: Option<Value>,
    #[serde(default)]
    pub coverage: Vec<Value>,
    #[serde(default)]
    pub snapshots: Vec<IndexEntry>,
}

impl Index {
    pub fn is_stitched(&self) -> bool {
        self.index_schema.as_deref() == Some("stitched-main-v1")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SegmentGraph {
    #[serde(default)]
    pub stitch_input_digest: Option<String>,
    #[serde(default)]
    pub play_order_ledger_digest: Option<String>,
    #[serde(default)]
    pub stream_major: Option<u64>,
    #[serde(default)]
    pub campaign: Option<Value>,
    #[serde(default)]
    pub main_head: Option<Value>,
    #[serde(default)]
    pub main_slices: Vec<Value>,
    #[serde(default)]
    pub segments: Vec<SegmentRecord>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SegmentRecord {
    pub segment_id: u32,
    #[serde(default)]
    pub play_order: Option<u64>,
    #[serde(default)]
    pub in_session_ordinal: Option<u32>,
    #[serde(default)]
    pub cutover_hour: Option<i64>,
    #[serde(default)]
    pub head_hour: Option<i64>,
    #[serde(default)]
    pub parent_segment: Option<u32>,
    #[serde(default)]
    pub empty_recovery: bool,
    #[serde(default)]
    pub index_file: Option<String>,
    #[serde(default)]
    pub visible_spans: Vec<Value>,
    #[serde(default)]
    pub superseded_spans: Vec<Value>,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IndexEntry {
    #[serde(default)]
    pub hour_index: Option<i64>,
    #[serde(default)]
    pub date: Option<String>,
    pub file: String,
    #[serde(default)]
    pub save_version: Option<String>,
    #[serde(default)]
    pub counts: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Snapshot {
    #[serde(default)]
    pub game_id: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub hour_index: Option<i64>,
    #[serde(default)]
    pub save_version: Option<String>,
    #[serde(default)]
    pub schema_version: Option<Value>,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

impl Snapshot {
    pub fn from_world_value(world: Value) -> Result<Snapshot, String> {
        let Value::Object(mut root) = world else {
            return Err("a world body is a JSON object".to_string());
        };
        fn text(root: &mut Map<String, Value>, key: &str) -> Result<Option<String>, String> {
            match root.remove(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(text)) => Ok(Some(text)),
                Some(_) => Err(format!("world member {key} is not a string")),
            }
        }
        let game_id = text(&mut root, "game_id")?;
        let date = text(&mut root, "date")?;
        let save_version = text(&mut root, "save_version")?;
        let hour_index = match root.remove("hour_index") {
            None | Some(Value::Null) => None,
            Some(Value::Number(number)) => Some(
                number
                    .as_i64()
                    .ok_or_else(|| "world member hour_index is not an integer".to_string())?,
            ),
            Some(_) => return Err("world member hour_index is not an integer".to_string()),
        };
        let schema_version = match root.remove("schema_version") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value),
        };
        Ok(Snapshot {
            game_id,
            date,
            hour_index,
            save_version,
            schema_version,
            rest: root,
        })
    }

    pub fn domain(&self, key: &str) -> Option<&Value> {
        self.rest.get(key)
    }
    pub fn countries(&self) -> Option<&Map<String, Value>> {
        self.rest.get("countries").and_then(Value::as_object)
    }
    pub fn country(&self, tag: &str) -> Option<&Value> {
        self.countries()?.get(tag)
    }
    pub fn states(&self) -> Option<&Map<String, Value>> {
        self.rest.get("states").and_then(Value::as_object)
    }
    pub fn state(&self, sid: &str) -> Option<&Value> {
        self.states()?.get(sid)
    }

    pub fn country_path<'a>(&'a self, tag: &str, dotted: &str) -> Option<&'a Value> {
        path_walk(self.country(tag)?, dotted)
    }
    pub fn state_path<'a>(&'a self, sid: &str, dotted: &str) -> Option<&'a Value> {
        path_walk(self.state(sid)?, dotted)
    }
}

fn path_walk<'a>(mut cur: &'a Value, dotted: &str) -> Option<&'a Value> {
    for seg in dotted.split('.') {
        cur = cur.as_object()?.get(seg)?;
    }
    Some(cur)
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Directory(String),
    Container(ContainerError),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "io: {e}"),
            LoadError::Json(e) => write!(f, "json: {e}"),
            LoadError::Directory(e) => write!(f, "directory store rejected: {e}"),
            LoadError::Container(e) => write!(f, "container: {e}"),
        }
    }
}
impl std::error::Error for LoadError {}
impl From<std::io::Error> for LoadError {
    fn from(e: std::io::Error) -> Self {
        LoadError::Io(e)
    }
}
impl From<serde_json::Error> for LoadError {
    fn from(e: serde_json::Error) -> Self {
        LoadError::Json(e)
    }
}
impl From<ContainerError> for LoadError {
    fn from(e: ContainerError) -> Self {
        LoadError::Container(e)
    }
}

#[derive(Clone)]
pub struct Store {
    backend: Backend,
}

#[derive(Clone, Debug)]
struct DirGame {
    dir: dir_store::BoundDir,
    index_bytes: Option<Vec<u8>>,
}

#[derive(Clone)]
enum Backend {
    Dir {
        root: Option<dir_store::AnchoredDir>,
        games: Arc<OnceLock<Result<BTreeMap<String, DirGame>, dir_store::DirAccessError>>>,
        label: PathBuf,
        open_error: Option<dir_store::DirAccessError>,
    },
    Replay {
        container: Arc<Container>,
    },
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.backend {
            Backend::Dir {
                root,
                label,
                open_error,
                ..
            } => f
                .debug_struct("Store::Dir")
                .field("root", label)
                .field("anchored", &root.is_some())
                .field("open_error", open_error)
                .finish(),
            Backend::Replay { container } => f
                .debug_struct("Store::Replay")
                .field("game_id", &container.game_id())
                .finish(),
        }
    }
}

impl Store {
    pub fn from_container(container: Container) -> Self {
        Self {
            backend: Backend::Replay {
                container: Arc::new(container),
            },
        }
    }

    pub fn open<P: Into<PathBuf>>(path: P) -> Self {
        let path = path.into();
        if path.is_file() {
            match Container::open(&path, &ExpectedIdentity::default()) {
                Ok(c) => {
                    return Store {
                        backend: Backend::Replay {
                            container: Arc::new(c),
                        },
                    }
                }
                Err(_) => {
                    return Store {
                        backend: Self::dir_backend(path),
                    };
                }
            }
        }
        Store {
            backend: Self::dir_backend(path),
        }
    }

    fn dir_backend(path: PathBuf) -> Backend {
        match dir_store::AnchoredDir::open(&path) {
            Ok(root) => Backend::Dir {
                root: Some(root),
                games: Arc::new(OnceLock::new()),
                label: path,
                open_error: None,
            },
            Err(error) => Backend::Dir {
                root: None,
                games: Arc::new(OnceLock::new()),
                label: path,
                open_error: Some(error),
            },
        }
    }

    pub fn open_directory<P: AsRef<Path>>(path: P) -> Result<Self, LoadError> {
        let path = path.as_ref().to_path_buf();
        let root = dir_store::AnchoredDir::open(&path)
            .map_err(|error| LoadError::Directory(error.message))?;
        Ok(Self {
            backend: Backend::Dir {
                root: Some(root),
                games: Arc::new(OnceLock::new()),
                label: path,
                open_error: None,
            },
        })
    }

    pub fn open_replay<P: AsRef<Path>>(
        path: P,
        expected: &ExpectedIdentity,
    ) -> Result<Self, LoadError> {
        let c = Container::open(path, expected)?;
        Ok(Store {
            backend: Backend::Replay {
                container: Arc::new(c),
            },
        })
    }

    pub fn container(&self) -> Option<&Container> {
        match &self.backend {
            Backend::Replay { container } => Some(container),
            Backend::Dir { .. } => None,
        }
    }

    fn directory_games(&self) -> Result<&BTreeMap<String, DirGame>, LoadError> {
        let (root, cache) = match &self.backend {
            Backend::Dir {
                root: Some(root),
                games,
                ..
            } => (root, games),
            Backend::Dir {
                open_error: Some(error),
                ..
            } => return Err(LoadError::Directory(error.message.clone())),
            Backend::Dir { .. } => {
                return Err(LoadError::Directory(
                    "directory root is not anchored".to_string(),
                ))
            }
            Backend::Replay { .. } => {
                return Err(LoadError::Directory(
                    "container has no directory games".to_string(),
                ))
            }
        };
        cache
            .get_or_init(|| Self::discover_directory_games(root))
            .as_ref()
            .map_err(|error| LoadError::Directory(error.message.clone()))
    }

    fn discover_directory_games(
        root: &dir_store::AnchoredDir,
    ) -> Result<BTreeMap<String, DirGame>, dir_store::DirAccessError> {
        let mut games = BTreeMap::new();
        for entry in root.root_entries()? {
            if !entry.directory_or_link {
                continue;
            }
            let dir = root.bind_child_dir(&entry.name)?;
            let index_bytes = dir.read_optional(&["index.json"])?;
            games.insert(entry.name, DirGame { dir, index_bytes });
        }
        Ok(games)
    }

    fn directory_game(&self, game: &str) -> Result<&DirGame, LoadError> {
        self.directory_games()?.get(game).ok_or_else(|| {
            LoadError::Directory(format!(
                "game directory {game:?} was not selected from the anchored store root"
            ))
        })
    }

    pub fn games(&self) -> Result<Vec<String>, LoadError> {
        match &self.backend {
            Backend::Dir { .. } => {
                let mut out = Vec::new();
                for (name, game) in self.directory_games()? {
                    if game.index_bytes.is_some() {
                        game.dir
                            .revalidate()
                            .map_err(|error| LoadError::Directory(error.message))?;
                        out.push(name.clone());
                    }
                }
                Ok(out)
            }
            Backend::Replay { container } => Ok(vec![container.game_id().to_string()]),
        }
    }

    pub fn index(&self, game: &str) -> Result<Index, LoadError> {
        match &self.backend {
            Backend::Dir { .. } => {
                let selected = self.directory_game(game)?;
                selected
                    .dir
                    .revalidate()
                    .map_err(|error| LoadError::Directory(error.message))?;
                let bytes = selected.index_bytes.as_deref().ok_or_else(|| {
                    LoadError::Directory(format!(
                        "required index.json is missing from selected game directory {game:?}"
                    ))
                })?;
                let index: Index = decode_json_strict(bytes)?;
                if index.game_id != game {
                    return Err(LoadError::Container(ContainerError::Reject(format!(
                        "index.game_id {:?} != selected directory name {game:?}",
                        index.game_id
                    ))));
                }
                Ok(index)
            }
            Backend::Replay { container } => {
                if container.game_id() != game {
                    return Err(LoadError::Container(ContainerError::Reject(format!(
                        "container holds game {:?}, not {game:?}",
                        container.game_id()
                    ))));
                }
                Ok(container.index().clone())
            }
        }
    }

    fn read_dir_snapshot(&self, game: &str, file: &str) -> Result<Snapshot, LoadError> {
        crate::zip::validate_path(file).map_err(|e| {
            LoadError::Container(ContainerError::Reject(format!(
                "unsafe snapshot file name {file:?}: {e}"
            )))
        })?;
        let bytes = self.read_game_file_bytes_required(game, file)?;
        Ok(decode_json_strict(&bytes)?)
    }

    pub fn load(&self, game: &str, hour_index: i64) -> Result<Snapshot, LoadError> {
        let file = match &self.backend {
            Backend::Dir { .. } => {
                let index = self.index(game)?;
                let file = index
                    .snapshots
                    .iter()
                    .find(|e| e.hour_index == Some(hour_index))
                    .map(|e| e.file.clone())
                    .ok_or_else(|| {
                        LoadError::Container(ContainerError::Reject(format!(
                            "no snapshot for hour_index {hour_index}"
                        )))
                    })?;
                file
            }
            Backend::Replay { container } => container
                .index()
                .snapshots
                .iter()
                .find(|entry| entry.hour_index == Some(hour_index))
                .map(|entry| entry.file.clone())
                .ok_or_else(|| {
                    LoadError::Container(ContainerError::Reject(format!(
                        "no snapshot for hour_index {hour_index}"
                    )))
                })?,
        };
        self.load_file(game, &file)
    }

    pub fn load_file(&self, game: &str, file: &str) -> Result<Snapshot, LoadError> {
        let mut snapshot = match &self.backend {
            Backend::Dir { .. } => self.read_dir_snapshot(game, file),
            Backend::Replay { container } => Ok(container.load_file(file)?),
        }?;
        self.hydrate_external_production_render_blobs(game, &mut snapshot)?;
        Ok(snapshot)
    }

    fn hydrate_external_production_render_blobs(
        &self,
        game: &str,
        snapshot: &mut Snapshot,
    ) -> Result<(), LoadError> {
        let Some(render) = snapshot.rest.get_mut("production_render") else {
            return Ok(());
        };
        let mut cache = BTreeMap::<String, String>::new();
        let mut reference_count = 0usize;
        self.hydrate_external_base64_value(game, render, &mut cache, &mut reference_count, 0)
    }

    fn hydrate_external_base64_value(
        &self,
        game: &str,
        value: &mut Value,
        cache: &mut BTreeMap<String, String>,
        reference_count: &mut usize,
        depth: usize,
    ) -> Result<(), LoadError> {
        const MAX_DEPTH: usize = 96;
        const MAX_REFERENCES: usize = 16_384;
        if depth > MAX_DEPTH {
            return Err(LoadError::Container(ContainerError::Reject(
                "production_render external-blob graph exceeds depth limit".to_string(),
            )));
        }

        if let Some(object) = value.as_object() {
            if object.len() == 1 {
                if let Some(descriptor) = object.get("$hoi4replay_external_base64") {
                    let descriptor = descriptor.as_object().ok_or_else(|| {
                        LoadError::Container(ContainerError::Reject(
                            "production_render external-base64 descriptor is not an object"
                                .to_string(),
                        ))
                    })?;
                    if descriptor.len() != 3 {
                        return Err(LoadError::Container(ContainerError::Reject(
                            "production_render external-base64 descriptor has unexpected fields"
                                .to_string(),
                        )));
                    }
                    let path = descriptor
                        .get("path")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            LoadError::Container(ContainerError::Reject(
                                "production_render external-base64 descriptor lacks path"
                                    .to_string(),
                            ))
                        })?;
                    let expected_sha256 = descriptor
                        .get("sha256")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            LoadError::Container(ContainerError::Reject(
                                "production_render external-base64 descriptor lacks sha256"
                                    .to_string(),
                            ))
                        })?;
                    let expected_len = descriptor
                        .get("byte_len")
                        .and_then(Value::as_u64)
                        .and_then(|value| usize::try_from(value).ok())
                        .ok_or_else(|| {
                            LoadError::Container(ContainerError::Reject(
                                "production_render external-base64 descriptor has invalid byte_len"
                                    .to_string(),
                            ))
                        })?;
                    if expected_len > 256 * 1024 * 1024 {
                        return Err(LoadError::Container(ContainerError::Reject(
                            "production_render external-base64 blob exceeds 256 MiB".to_string(),
                        )));
                    }
                    let expected_path = format!("production_render_blobs/{expected_sha256}.base64");
                    if path != expected_path
                        || expected_sha256.len() != 64
                        || !expected_sha256
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                    {
                        return Err(LoadError::Container(ContainerError::Reject(
                            "production_render external-base64 path/digest is not canonical"
                                .to_string(),
                        )));
                    }
                    *reference_count += 1;
                    if *reference_count > MAX_REFERENCES {
                        return Err(LoadError::Container(ContainerError::Reject(
                            "production_render external-base64 reference count exceeds limit"
                                .to_string(),
                        )));
                    }
                    let encoded = if let Some(encoded) = cache.get(path) {
                        encoded.clone()
                    } else {
                        let bytes = self.read_game_file_bytes_required(game, path)?;
                        if bytes.len() != expected_len {
                            return Err(LoadError::Container(ContainerError::Reject(format!(
                                "production_render external-base64 {path:?} length {} != declared {expected_len}",
                                bytes.len()
                            ))));
                        }
                        let actual_sha256 = sha256::Sha256::hex(&bytes);
                        if actual_sha256 != expected_sha256 {
                            return Err(LoadError::Container(ContainerError::Reject(format!(
                                "production_render external-base64 {path:?} hashes to {actual_sha256}, expected {expected_sha256}"
                            ))));
                        }
                        let encoded = String::from_utf8(bytes).map_err(|_| {
                            LoadError::Container(ContainerError::Reject(format!(
                                "production_render external-base64 {path:?} is not UTF-8"
                            )))
                        })?;
                        cache.insert(path.to_string(), encoded.clone());
                        encoded
                    };
                    *value = Value::String(encoded);
                    return Ok(());
                }
            }
        }

        match value {
            Value::Array(values) => {
                for child in values {
                    self.hydrate_external_base64_value(
                        game,
                        child,
                        cache,
                        reference_count,
                        depth + 1,
                    )?;
                }
            }
            Value::Object(values) => {
                for child in values.values_mut() {
                    self.hydrate_external_base64_value(
                        game,
                        child,
                        cache,
                        reference_count,
                        depth + 1,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    pub fn read_game_file_bytes(
        &self,
        game: &str,
        relative: &str,
    ) -> Result<Option<Vec<u8>>, LoadError> {
        let logical = format!("{game}/{relative}");
        crate::zip::validate_path(&logical).map_err(|error| {
            LoadError::Container(ContainerError::Reject(format!(
                "unsafe game-relative path {logical:?}: {error}"
            )))
        })?;
        match &self.backend {
            Backend::Dir { .. } => {
                let components: Vec<&str> = relative.split('/').collect();
                self.directory_game(game)?
                    .dir
                    .read_optional(&components)
                    .map_err(|error| LoadError::Directory(error.message))
            }
            Backend::Replay { container } => {
                if container.game_id() != game {
                    return Err(LoadError::Container(ContainerError::Reject(format!(
                        "container holds game {:?}, not {game:?}",
                        container.game_id()
                    ))));
                }
                let path = format!("payload/{game}/{relative}");
                if !container.has_entry(&path)? {
                    return Ok(None);
                }
                Ok(Some(container.read_entry_bytes(&path)?))
            }
        }
    }

    fn read_game_file_bytes_required(
        &self,
        game: &str,
        relative: &str,
    ) -> Result<Vec<u8>, LoadError> {
        self.read_game_file_bytes(game, relative)?.ok_or_else(|| {
            LoadError::Directory(format!(
                "required game-relative file {game:?}/{relative} is missing"
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn the_direct_body_matches_serde_flatten() {
        for document in [
            r#"{}"#,
            r#"{"game_id":"g","date":"1936.1.1.1","hour_index":17032314,"save_version":"1.19","schema_version":"3","countries":{"GER":{"n":1}}}"#,
            r#"{"game_id":null,"date":null,"hour_index":null,"save_version":null,"schema_version":null,"states":[1,2]}"#,
            r#"{"schema_version":{"world":4},"hour_index":-1}"#,
            r#"{"hour_index":9223372036854775807}"#,
            r#"{"hour_index":1.5}"#,
            r#"{"hour_index":9223372036854775808}"#,
            r#"{"hour_index":"12"}"#,
            r#"{"date":12}"#,
            r#"{"game_id":[]}"#,
            r#"{"save_version":{}}"#,
            r#"[1,2,3]"#,
        ] {
            let value: Value = serde_json::from_str(document).expect("fixture parses");
            let serde_body: Result<Snapshot, _> = serde_json::from_value(value.clone());
            let direct = Snapshot::from_world_value(value);
            match (serde_body, direct) {
                (Ok(want), Ok(got)) => {
                    assert_eq!(want.game_id, got.game_id, "{document}: game_id");
                    assert_eq!(want.date, got.date, "{document}: date");
                    assert_eq!(want.hour_index, got.hour_index, "{document}: hour_index");
                    assert_eq!(
                        want.save_version, got.save_version,
                        "{document}: save_version"
                    );
                    assert_eq!(
                        want.schema_version, got.schema_version,
                        "{document}: schema_version"
                    );
                    assert_eq!(want.rest, got.rest, "{document}: rest");
                }
                (Err(_), Err(_)) => {}
                (want, got) => panic!(
                    "{document}: serde and the direct body disagree about acceptance: \
                     serde ok={} direct ok={}",
                    want.is_ok(),
                    got.is_ok()
                ),
            }
        }
    }

    fn temp_root(tag: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "hoi4_snapshot_lib_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn dir_store_load_resolves_segment_prefixed_files_through_the_index() {
        let root = temp_root("dir_load_index");
        let game = root.join("g");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            game.join("index.json"),
            br#"{"game_id":"g","schema_version":"1","snapshots":[{"hour_index":17000461,"file":"s00000001.000017000461.snapshot.json"}]}"#,
        )
        .unwrap();
        fs::write(
            game.join("s00000001.000017000461.snapshot.json"),
            br#"{"game_id":"g","hour_index":17000461}"#,
        )
        .unwrap();

        let store = Store::open_directory(&root).expect("anchor dir store");
        let snapshot = store
            .load("g", 17000461)
            .expect("segment-prefixed rollback snapshot loads by hour_index");
        assert_eq!(snapshot.hour_index, Some(17000461));

        let err = store.load("g", 5).expect_err("missing hour is refused");
        match err {
            LoadError::Container(ContainerError::Reject(message)) => assert!(
                message.contains("no snapshot for hour_index 5"),
                "unexpected message: {message}"
            ),
            other => panic!("expected typed Reject, got {other:?}"),
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn replay_store_poisoned_entry_lock_refuses_instead_of_absent() {
        let bytes = crate::pack::pack(
            "g",
            &[crate::pack::PackEntry {
                path: "payload/g/index.json".into(),
                bytes: br#"{"game_id":"g","schema_version":"1","snapshots":[]}"#.to_vec(),
                role: "index",
                hour_index: None,
            }],
            &crate::pack::PackIdentity::default(),
            false,
        )
        .unwrap();
        let container =
            Container::from_bytes(bytes, &ExpectedIdentity::default()).expect("container opens");
        let store = Store {
            backend: Backend::Replay {
                container: Arc::new(container),
            },
        };
        assert!(store
            .read_game_file_bytes("g", "index.json")
            .expect("healthy read succeeds")
            .is_some());
        store.container().unwrap().poison_entry_lock_for_test();
        let err = store
            .read_game_file_bytes("g", "index.json")
            .expect_err("a poisoned lock must refuse, never report absence");
        match err {
            LoadError::Container(ContainerError::Reject(message)) => assert!(
                message.contains("entry-verification lock poisoned"),
                "unexpected message: {message}"
            ),
            other => panic!("expected typed container Reject, got {other:?}"),
        }
    }
}
