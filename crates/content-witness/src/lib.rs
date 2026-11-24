use content_reader_layers::ResolvedLayers;
use content_reader_layers::content::{ContentRead, LayerSource, ReadOutcome};
use product_codec::Product;
use sha2::{Digest, Sha256};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};
use std::{
    io,
    sync::{Arc, Weak},
};

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
    sources: Arc<[(LayerSource, Layer)]>,
    entries: Arc<Mutex<BTreeMap<Canonical, Option<Weak<EncodedRecord>>>>>,
    shards: Arc<Mutex<std::collections::BTreeMap<Vec<String>, Witness>>>,
}
impl Recorder {
    pub fn new(layers: &ResolvedLayers) -> Self {
        let roster = Self::roster(layers);
        let mut entries = BTreeMap::new();
        entries.insert(
            Canonical(Entry::Roster {
                layers: roster.clone(),
            }),
            None,
        );
        Self {
            sources: layers
                .layers
                .iter()
                .zip(&roster)
                .map(|(layer, witness)| (layer.source.clone(), witness.clone()))
                .collect(),
            roster: roster.into(),
            entries: Arc::new(Mutex::new(entries)),
            shards: Default::default(),
        }
    }
    pub fn for_layers(&self, layers: &ResolvedLayers) -> Self {
        let mut used = BTreeSet::new();
        let roster = Self::roster(layers)
            .into_iter()
            .zip(&layers.layers)
            .map(|(layer, resolved)| {
                let source =
                    self.sources
                        .iter()
                        .enumerate()
                        .find(|(index, (source, candidate))| {
                            !used.contains(index)
                                && candidate.id == layer.id
                                && candidate.kind == layer.kind
                                && candidate.replace_paths == layer.replace_paths
                                && match (source, &resolved.source) {
                                    (LayerSource::Folder(a), LayerSource::Folder(b))
                                    | (LayerSource::Archive(a), LayerSource::Archive(b)) => a == b,
                                    _ => false,
                                }
                        });
                match source {
                    Some((index, (_, candidate))) => {
                        used.insert(index);
                        candidate.clone()
                    }
                    None => layer,
                }
            })
            .collect();
        Self {
            roster,
            sources: Arc::clone(&self.sources),
            entries: Arc::clone(&self.entries),
            shards: Arc::clone(&self.shards),
        }
    }
    fn roster(layers: &ResolvedLayers) -> Vec<Layer> {
        layers
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
            .collect()
    }
    pub fn record_note(&self, purpose: &str, message: &str) {
        self.record(Entry::Result {
            purpose: purpose.into(),
            status: 0,
            kind: String::new(),
            message: message.into(),
        });
    }
    pub fn record_error(&self, purpose: &str, error: &io::Error) {
        self.record(Entry::Result {
            purpose: purpose.into(),
            status: 1,
            kind: format!("{:?}", error.kind()),
            message: error.to_string(),
        });
    }
    fn record(&self, entry: Entry) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let entry = Canonical(entry);
        let mut shards = self
            .shards
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let std::collections::btree_map::Entry::Vacant(slot) = entries.entry(entry) {
            shards.retain(|purposes, _| {
                !purposes.iter().any(|wanted| wanted == slot.key().0.location().1)
            });
            slot.insert(None);
        }
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
    pub fn snapshot_for(&self, purposes: &[&str]) -> io::Result<Witness> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut shards = self
            .shards
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((_, witness)) = shards.iter().find(|(key, _)| {
            key.len() == purposes.len()
                && key.iter().map(String::as_str).eq(purposes.iter().copied())
        }) {
            return Ok(witness.clone());
        }
        let witness = build_witness(&mut entries, purposes, false)?;
        shards.insert(
            purposes.iter().map(|value| value.to_string()).collect(),
            witness.clone(),
        );
        Ok(witness)
    }

    pub fn snapshot(&self) -> io::Result<Witness> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        build_witness(&mut entries, &[], true)
    }
}

#[derive(Debug)]
struct EncodedRecord {
    bytes: Vec<u8>,
}
impl PartialEq for EncodedRecord {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}
impl Eq for EncodedRecord {}
impl EncodedRecord {
    fn get(entry: &Entry, cached: &mut Option<Weak<Self>>) -> io::Result<Arc<Self>> {
        if let Some(record) = cached.as_ref().and_then(Weak::upgrade) {
            return Ok(record);
        }
        let mut bytes = engine_allocation::vec(entry_bytes(entry)?)?;
        entry.encode(&mut bytes)?;
        let record = Arc::new(Self { bytes });
        *cached = Some(Arc::downgrade(&record));
        Ok(record)
    }
}

fn build_witness(
    entries: &mut BTreeMap<Canonical, Option<Weak<EncodedRecord>>>,
    purposes: &[&str],
    all: bool,
) -> io::Result<Witness> {
    let selected = |entry: &Entry| {
        all || matches!(entry, Entry::Roster { .. }) || purposes.contains(&entry.location().1)
    };
    let count = entries.keys().filter(|entry| selected(&entry.0)).count();
    let mut records = engine_allocation::vec(count)?;
    let mut hash = witness_hash();
    for (entry, cached) in entries.iter_mut().filter(|(entry, _)| selected(&entry.0)) {
        let record = EncodedRecord::get(&entry.0, cached)?;
        hash_entry(&mut hash, &record.bytes)?;
        records.push(record);
    }
    let records = records.into();
    Ok(Witness {
        digest: hash.finalize().into(),
        entries: u64::try_from(count).map_err(|_| invalid("content witness count overflows"))?,
        records,
    })
}
fn layer_bytes(layer: &Layer) -> io::Result<usize> {
    layer.replace_paths.iter().try_fold(
        engine_allocation::add(25, layer.id.len())?,
        |bytes, path| engine_allocation::add(bytes, engine_allocation::add(8, path.len())?),
    )
}
fn entry_bytes(entry: &Entry) -> io::Result<usize> {
    use engine_allocation::add;
    match entry {
        Entry::Roster { layers } => layers
            .iter()
            .try_fold(9, |bytes, layer| add(bytes, layer_bytes(layer)?)),
        Entry::Read {
            path,
            purpose,
            layer,
            outcome,
        } => {
            let base = add(
                add(add(18, path.len())?, purpose.len())?,
                layer.as_ref().map_or(Ok(0), layer_bytes)?,
            )?;
            let outcome = match outcome {
                Outcome::Bytes { .. } => 41,
                Outcome::Probe { .. } => 2,
                Outcome::Error {
                    kind,
                    message,
                    payload,
                } => add(
                    add(18 + 40 * usize::from(payload.is_some()), kind.len())?,
                    message.len(),
                )?,
            };
            add(base, outcome)
        }
        Entry::Result {
            purpose,
            kind,
            message,
            ..
        } => add(add(add(26, purpose.len())?, kind.len())?, message.len()),
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

#[derive(Clone, Debug)]
pub struct Witness {
    pub digest: [u8; 32],
    pub entries: u64,
    records: Arc<[Arc<EncodedRecord>]>,
}
impl PartialEq for Witness {
    fn eq(&self, other: &Self) -> bool {
        self.digest == other.digest
            && self.entries == other.entries
            && self.records == other.records
    }
}
impl Eq for Witness {}
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
        for (ordinal, record) in self.records.iter().enumerate() {
            visit(ordinal as u64, &record.bytes)?;
        }
        Ok(())
    }
}

pub fn verify<T>(
    digest: [u8; 32],
    entries: u64,
    mut read: impl FnMut(u64) -> io::Result<Option<T>>,
    mut bytes: impl FnMut(T) -> io::Result<Vec<u8>>,
    mut check: impl FnMut() -> io::Result<()>,
) -> io::Result<()> {
    let mut hash = witness_hash();
    let mut previous: Option<Entry> = None;
    for ordinal in 0..entries {
        check()?;
        let record = read(ordinal)?.ok_or_else(|| invalid("content witness entry is dangling"))?;
        let bytes = bytes(record)?;
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
    if read(entries)?.is_some() {
        return Err(invalid("content witness has trailing entries"));
    }
    if <[u8; 32]>::from(hash.finalize()) != digest {
        return Err(invalid("content witness digest differs"));
    }
    check()
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn read(purpose: &str, outcome: Outcome) -> Entry {
        Entry::Read {
            path: b"gfx/test".to_vec(),
            purpose: purpose.into(),
            layer: None,
            outcome,
        }
    }
    #[test]
    fn overlapping_shards_share_records_and_free_them_with_their_last_shard() {
        let recorder = Recorder::new(&ResolvedLayers::default());
        recorder.record(read("map", Outcome::Probe { present: true }));
        recorder.record(read("audio", Outcome::Probe { present: false }));
        let map = recorder.snapshot_for(&["map"]).unwrap();
        let all = recorder.snapshot().unwrap();
        for record in map.records.iter() {
            let shared = all
                .records
                .iter()
                .find(|other| other.bytes == record.bytes)
                .unwrap();
            assert!(Arc::ptr_eq(record, shared));
        }
        let record = Arc::downgrade(&map.records[1]);
        drop(map);
        drop(all);
        recorder.shards.lock().unwrap().clear();
        assert!(record.upgrade().is_none());
    }
    #[test]
    fn purpose_shards_are_immutable_and_late_negative_and_error_reads_invalidate_only_dependents() {
        let recorder = Recorder::new(&ResolvedLayers::default());
        recorder.record(read(
            "map",
            Outcome::Bytes {
                length: 4,
                digest: [1; 32],
            },
        ));
        let first = recorder.snapshot_for(&["map"]).unwrap();
        let copy = recorder.snapshot_for(&["map"]).unwrap();
        assert!(Arc::ptr_eq(&first.records, &copy.records));
        recorder.record(read("audio", Outcome::Probe { present: false }));
        assert!(Arc::ptr_eq(
            &first.records,
            &recorder.snapshot_for(&["map"]).unwrap().records
        ));
        recorder.record(read("map", Outcome::Probe { present: false }));
        let negative = recorder.snapshot_for(&["map"]).unwrap();
        assert_ne!(negative.digest, first.digest);
        assert_eq!(first.records.len() + 1, negative.records.len());
        recorder.record(read(
            "map",
            Outcome::Error {
                kind: "InvalidData".into(),
                message: "named refusal".into(),
                payload: Some(Payload {
                    length: 3,
                    digest: [7; 32],
                }),
            },
        ));
        let failed = recorder.snapshot_for(&["map"]).unwrap();
        assert_ne!(failed.digest, negative.digest);
        assert_eq!(first, copy);
        let mut records = Vec::new();
        failed
            .visit(|_, bytes| {
                records.push(bytes.to_vec());
                Ok(())
            })
            .unwrap();
        verify(
            failed.digest,
            failed.entries,
            |ordinal| Ok(records.get(ordinal as usize).cloned()),
            Ok,
            || Ok(()),
        )
        .unwrap();
    }
    #[test]
    fn encoded_size_census_covers_every_entry_variant() {
        let layer = Layer {
            index: 123,
            id: "test-λ".into(),
            kind: 1,
            replace_paths: vec![b"common/test".to_vec(), vec![]],
        };
        for entry in [
            Entry::Roster {
                layers: vec![layer.clone()],
            },
            read("map", Outcome::Probe { present: true }),
            Entry::Read {
                path: vec![255, 0, 9],
                purpose: "heightmap".into(),
                layer: Some(layer),
                outcome: Outcome::Bytes {
                    length: 900,
                    digest: [1; 32],
                },
            },
            read(
                "trigger",
                Outcome::Error {
                    kind: "kind".into(),
                    message: "some error".into(),
                    payload: None,
                },
            ),
            read(
                "trigger",
                Outcome::Error {
                    kind: "kind".into(),
                    message: "some error".into(),
                    payload: Some(Payload {
                        length: 123,
                        digest: [2; 32],
                    }),
                },
            ),
            Entry::Result {
                purpose: "source".into(),
                status: 1,
                kind: "error".into(),
                message: "λ".into(),
            },
        ] {
            assert_eq!(
                entry_bytes(&entry).unwrap(),
                product_codec::encode(&entry).unwrap().len()
            );
        }
    }
    #[test]
    fn canonical_order_and_ordered_mod_roster_are_independent_of_read_order() {
        fn recorder(layers: Vec<Layer>) -> Recorder {
            let mut entries = BTreeMap::new();
            entries.insert(
                Canonical(Entry::Roster {
                    layers: layers.clone(),
                }),
                None,
            );
            Recorder {
                roster: layers.into(),
                sources: Arc::from([]),
                entries: Arc::new(Mutex::new(entries)),
                shards: Default::default(),
            }
        }
        let a = Layer {
            index: 0,
            id: "a".into(),
            kind: 0,
            replace_paths: vec![b"gfx".to_vec()],
        };
        let b = Layer {
            index: 1,
            id: "b".into(),
            kind: 1,
            replace_paths: vec![],
        };
        let first = recorder(vec![a.clone(), b.clone()]);
        let second = recorder(vec![a.clone(), b.clone()]);
        first.record(read("map", Outcome::Probe { present: false }));
        first.record(read("map", Outcome::Probe { present: true }));
        second.record(read("map", Outcome::Probe { present: true }));
        second.record(read("map", Outcome::Probe { present: false }));
        assert_eq!(
            first.snapshot_for(&["map"]).unwrap(),
            second.snapshot_for(&["map"]).unwrap()
        );
        let reordered = recorder(vec![Layer { index: 0, ..b }, Layer { index: 1, ..a }]);
        assert_ne!(
            first.snapshot_for(&["unused"]).unwrap().digest,
            reordered.snapshot_for(&["unused"]).unwrap().digest
        );
    }
}
