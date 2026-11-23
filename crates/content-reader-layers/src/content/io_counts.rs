use super::{LayerSource, ResolvedLayer};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Mutex, OnceLock},
};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub struct Source {
    pub layer: String,
    pub root: String,
}

impl Source {
    pub(super) fn layer(layer: &ResolvedLayer) -> Self {
        Self {
            layer: layer.id.clone(),
            root: match &layer.source {
                LayerSource::Folder(root) => format!("folder:{}", root.display()),
                LayerSource::Archive(path) => format!("archive:{}", path.display()),
            },
        }
    }

    fn external() -> Self {
        Self {
            layer: "external metadata".into(),
            root: String::new(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    source: Source,
    path: Vec<u8>,
    digest: [u8; 32],
}

#[derive(serde::Serialize)]
pub struct ReadCount {
    pub source: Source,
    pub path: Vec<u8>,
    pub digest: [u8; 32],
    pub bytes: u64,
    pub reads: u64,
}

#[derive(serde::Serialize)]
pub struct Counts {
    pub total_reads: u64,
    pub bytes_read: u64,
    pub repeated_resolved_byte_reads: u64,
    pub repeated_bytes: u64,
    pub files: Vec<ReadCount>,
    pub parses: Vec<content_reader_parser::parse_counts::ParseCount>,
}

static READS: OnceLock<Option<Mutex<BTreeMap<Key, (u64, u64)>>>> = OnceLock::new();

fn reads() -> Option<&'static Mutex<BTreeMap<Key, (u64, u64)>>> {
    READS
        .get_or_init(|| content_reader_parser::parse_counts::enabled().then(Mutex::default))
        .as_ref()
}

pub(super) fn record(source: Source, path: &[u8], digest: [u8; 32], length: u64) {
    let Some(reads) = reads() else {
        return;
    };
    let mut reads = reads.lock().unwrap_or_else(|error| error.into_inner());
    let count = reads
        .entry(Key {
            source,
            path: path.to_vec(),
            digest,
        })
        .or_default();
    count.0 += 1;
    count.1 = length;
}

pub(super) fn bytes(source: Source, path: &[u8], bytes: &[u8]) {
    if reads().is_some() {
        record(
            source,
            path,
            Sha256::digest(bytes).into(),
            bytes.len() as u64,
        );
    }
}

pub(super) fn layer_bytes(layer: &ResolvedLayer, path: &[u8], bytes: &[u8]) {
    if reads().is_some() {
        self::bytes(Source::layer(layer), path, bytes);
    }
}

pub(super) fn layer_digest(layer: &ResolvedLayer, path: &[u8], digest: [u8; 32], length: u64) {
    if reads().is_some() {
        record(Source::layer(layer), path, digest, length);
    }
}

pub(super) fn external_digest(path: &Path, digest: [u8; 32], length: u64) {
    if reads().is_some() {
        record(
            Source::external(),
            path.to_string_lossy().as_bytes(),
            digest,
            length,
        );
    }
}

pub(super) fn external(path: &Path, bytes: &[u8]) {
    if reads().is_some() {
        external_digest(path, Sha256::digest(bytes).into(), bytes.len() as u64);
    }
}

pub fn snapshot() -> Counts {
    let files: Vec<_> = reads()
        .map(|reads| {
            reads
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .iter()
                .map(|(key, (count, bytes))| ReadCount {
                    source: key.source.clone(),
                    path: key.path.clone(),
                    digest: key.digest,
                    bytes: *bytes,
                    reads: *count,
                })
                .collect()
        })
        .unwrap_or_default();
    Counts {
        total_reads: files.iter().map(|file| file.reads).sum(),
        bytes_read: files.iter().map(|file| file.reads * file.bytes).sum(),
        repeated_resolved_byte_reads: files.iter().map(|file| file.reads.saturating_sub(1)).sum(),
        repeated_bytes: files
            .iter()
            .map(|file| file.reads.saturating_sub(1) * file.bytes)
            .sum(),
        files,
        parses: content_reader_parser::parse_counts::snapshot(),
    }
}
