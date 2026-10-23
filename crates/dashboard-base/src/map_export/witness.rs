use super::*;
use installed_content_core::content::{ContentRead, LayerSource, ReadOutcome};
use sha2::{Digest, Sha256};
use std::{cmp::Ordering, collections::BTreeSet, sync::Mutex};

pub const WITNESS_CONTRACT: &[u8] = b"map-export/content-witness/canonical-resolution-set/v1";

product_codec::product! {
    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Layer { pub index: usize, pub id: String, pub kind: u8, pub replace_paths: Vec<Vec<u8>> }
    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Payload { pub length: u64, pub digest: [u8; 32] }
    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub enum Outcome {
        0 => Bytes { length: u64, digest: [u8; 32] },
        1 => Probe { present: bool },
        2 => Error { kind: String, message: String, payload: Option<Payload> },
    }
    #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
    pub enum Entry {
        0 => Roster { layers: Vec<Layer> },
        1 => Read { path: Vec<u8>, purpose: String, layer: Option<Layer>, outcome: Outcome },
        2 => Result { purpose: String, status: u8, kind: String, message: String },
    }
}
impl Entry {
    fn location(&self) -> (&[u8], &str) {
        match self {
            Self::Roster { .. } => (&[], ""),
            Self::Read { path, purpose, .. } => (path, purpose),
            Self::Result { purpose, .. } => (&[], purpose),
        }
    }
    fn canonical_cmp(&self, other: &Self) -> Ordering {
        self.location()
            .cmp(&other.location())
            .then_with(|| self.cmp(other))
    }
}

#[derive(PartialEq, Eq)]
struct Canonical(Entry);
impl PartialOrd for Canonical {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Canonical {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.canonical_cmp(&other.0)
    }
}

#[derive(Clone)]
pub struct Recorder {
    roster: Arc<[Layer]>,
    entries: Arc<Mutex<BTreeSet<Canonical>>>,
}
impl Recorder {
    pub fn new(layers: &ResolvedLayers) -> Self {
        let roster: Vec<_> = layers
            .layers
            .iter()
            .enumerate()
            .map(|(index, layer)| Layer {
                index,
                id: layer.id.clone(),
                kind: match &layer.source {
                    LayerSource::Folder(_) => 0,
                    LayerSource::Archive(_) => 1,
                },
                replace_paths: layer.replace_paths.clone(),
            })
            .collect();
        let mut entries = BTreeSet::new();
        entries.insert(Canonical(Entry::Roster {
            layers: roster.clone(),
        }));
        Self {
            roster: roster.into(),
            entries: Arc::new(Mutex::new(entries)),
        }
    }
    fn record(&self, entry: Entry) {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(Canonical(entry));
    }
    pub fn record_read(&self, read: ContentRead) {
        let outcome = match read.outcome {
            ReadOutcome::Bytes { length, digest } => Outcome::Bytes { length, digest },
            ReadOutcome::Probe(present) => Outcome::Probe { present },
            ReadOutcome::Error {
                kind,
                message,
                payload,
            } => Outcome::Error {
                kind,
                message,
                payload: payload.map(|payload| Payload {
                    length: payload.length,
                    digest: payload.digest,
                }),
            },
        };
        self.record(Entry::Read {
            path: read.path,
            purpose: read.purpose,
            layer: read.layer.and_then(|index| self.roster.get(index).cloned()),
            outcome,
        });
    }
    pub fn record_result<T>(&self, purpose: &str, result: &io::Result<T>) {
        self.record(Entry::Result {
            purpose: purpose.into(),
            status: u8::from(result.is_err()),
            kind: result
                .as_ref()
                .err()
                .map(|error| format!("{:?}", error.kind()))
                .unwrap_or_default(),
            message: result
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_default(),
        });
    }
    /// Snapshots the content-bounded set; retained product readers may keep using this recorder.
    pub fn snapshot(&self) -> io::Result<Witness> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut records = engine_allocation::vec(entries.len())?;
        let mut hash = witness_hash();
        for entry in entries.iter() {
            let bytes = product_codec::encode(&entry.0)?;
            hash_entry(&mut hash, &bytes)?;
            records.push(bytes);
        }
        Ok(Witness {
            digest: hash.finalize().into(),
            entries: u64::try_from(records.len())
                .map_err(|_| invalid("content witness count overflows"))?,
            records: records.into(),
        })
    }
}

fn witness_hash() -> Sha256 {
    let mut hash = Sha256::new();
    hash.update(WITNESS_CONTRACT);
    hash
}
fn hash_entry(hash: &mut Sha256, bytes: &[u8]) -> io::Result<()> {
    let length =
        u64::try_from(bytes.len()).map_err(|_| invalid("content witness entry overflows"))?;
    hash.update(length.to_le_bytes());
    hash.update(bytes);
    Ok(())
}

#[derive(Clone)]
pub struct Witness {
    pub digest: [u8; 32],
    pub entries: u64,
    records: Arc<[Vec<u8>]>,
}
impl Default for Witness {
    fn default() -> Self {
        Self {
            digest: witness_hash().finalize().into(),
            entries: 0,
            records: Arc::from([]),
        }
    }
}
impl Witness {
    pub fn capture<T>(
        layers: &ResolvedLayers,
        work: impl FnOnce(&ResolvedLayers) -> io::Result<T>,
    ) -> io::Result<(io::Result<T>, Self)> {
        let recorder = Recorder::new(layers);
        let log = recorder.clone();
        let traced = layers.with_read_trace(Arc::new(move |read| log.record_read(read)));
        let result = work(&traced);
        recorder.record_result("", &result);
        Ok((result, recorder.snapshot()?))
    }
    pub fn visit(&self, mut visit: impl FnMut(u64, &[u8]) -> io::Result<()>) -> io::Result<()> {
        for (ordinal, bytes) in self.records.iter().enumerate() {
            visit(ordinal as u64, bytes)?;
        }
        Ok(())
    }
    pub fn publish(&self, writer: &mut Writer, key: &Key) -> io::Result<()> {
        self.visit(|ordinal, bytes| writer.write_at(&key.section("witness"), ordinal, bytes))
    }
}

pub(super) fn verify(
    key: &Key,
    mut read: impl FnMut(u64) -> io::Result<Option<store::Range>>,
    mut check: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    let mut hash = witness_hash();
    let mut previous: Option<Entry> = None;
    for ordinal in 0..key.content_entries {
        check()?;
        let range = read(ordinal)?.ok_or_else(|| invalid("content witness entry is dangling"))?;
        let length = usize::try_from(range.len())
            .map_err(|_| invalid("content witness exceeds address space"))?;
        let bytes = codec::read_exact_range(range, length)?;
        let entry: Entry = product_codec::decode(&bytes)?;
        if previous
            .as_ref()
            .is_some_and(|previous| previous.canonical_cmp(&entry) != Ordering::Less)
        {
            return Err(invalid("content witness entries are not a canonical set"));
        }
        hash_entry(&mut hash, &bytes)?;
        previous = Some(entry);
    }
    if read(key.content_entries)?.is_some() {
        return Err(invalid("content witness has trailing entries"));
    }
    if <[u8; 32]>::from(hash.finalize()) != key.content {
        return Err(invalid("content witness digest differs"));
    }
    check()
}
