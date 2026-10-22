//! Current presentation metadata, keyed by accepted recording/content stamps.
use super::*;
use replay_data::identity::IdentityCodec;
use serde::{Deserialize, Serialize};

const SCHEMA: u32 = 2;
const SECTION: &str = "source-metadata-v1";
type IndexEntry = (u64, i32, i32, u64, u64, u64);

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Inputs {
    header: Vec<u8>,
    index: Vec<IndexEntry>,
    sessions: Vec<Vec<u8>>,
    branches: Vec<Vec<u8>>,
    gaps: Vec<Vec<u8>>,
}
impl Inputs {
    fn is_prefix_of(&self, current: &Self, replay: &Replay) -> io::Result<bool> {
        if self.header != current.header
            || self.index.len() > current.index.len()
            || self
                .index
                .iter()
                .zip(&current.index)
                .any(|(old, new)| old != new)
            || self
                .sessions
                .iter()
                .any(|old| !current.sessions.contains(old))
            || self
                .branches
                .iter()
                .any(|old| !current.branches.contains(old))
            || self.gaps.iter().any(|old| !current.gaps.contains(old))
        {
            return Ok(false);
        }
        for gap in &replay.gaps {
            let mut encoded = Vec::new();
            gap.encode(&mut encoded)?;
            if !self.gaps.contains(&encoded)
                && self
                    .index
                    .iter()
                    .any(|block| block.0 == gap.branch && gap.first_hour <= block.2)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn read(replay: &Replay) -> io::Result<Self> {
        let mut header = Vec::new();
        replay.identity.encode(&mut header)?;
        header.extend_from_slice(&replay.campaign_id);
        header.extend_from_slice(&replay.format.version().to_le_bytes());
        header.extend_from_slice(&replay.keyframe_interval.to_le_bytes());
        Ok(Self {
            header,
            index: replay
                .index
                .iter()
                .map(|index| {
                    (
                        index.branch,
                        index.first_hour,
                        index.last_hour,
                        index.offset,
                        index.length,
                        index.raw_length,
                    )
                })
                .collect(),
            sessions: replay
                .sessions
                .iter()
                .map(|value| {
                    let mut bytes = Vec::new();
                    value.encode(&mut bytes)?;
                    Ok(bytes)
                })
                .collect::<io::Result<_>>()?,
            branches: replay
                .branches
                .iter()
                .map(|value| {
                    let mut bytes = Vec::new();
                    value.encode(&mut bytes)?;
                    Ok(bytes)
                })
                .collect::<io::Result<_>>()?,
            gaps: replay
                .gaps
                .iter()
                .map(|value| {
                    let mut bytes = Vec::new();
                    value.encode(&mut bytes)?;
                    Ok(bytes)
                })
                .collect::<io::Result<_>>()?,
        })
    }
}
#[derive(Serialize, Deserialize)]
struct Cached {
    schema: u32,
    inputs: Inputs,
    metadata: ReplayMetadata,
    named: Option<(Option<ContentKey>, ReplayMetadata)>,
}

pub fn write(writer: &mut store::Writer, source: &Source) -> io::Result<()> {
    let cache = Cached {
        schema: SCHEMA,
        inputs: Inputs::read(source.replay_ref())?,
        metadata: source.metadata.clone(),
        named: source
            .named
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone(),
    };
    writer.write_at(SECTION, 0, serde_json::to_vec(&cache)?.as_slice())
}
/// Stale/corrupt derived metadata is a miss, never a refusal to open a recording.
pub(super) type CachedNames = (store::SourceKey, Option<ContentKey>, ReplayMetadata);
pub(super) struct Restored {
    pub metadata: ReplayMetadata,
    pub named: Option<CachedNames>,
    pub inputs: store::SourceKey,
}
pub(super) fn read(
    path: &Path,
    replay: &Arc<Replay>,
    cancelled: &AtomicBool,
) -> io::Result<Option<Restored>> {
    let store = crate::prepared::recording_store(path)?;
    let Some(key) = store.saved_key()? else {
        return Ok(None);
    };
    // Raw catalogue metadata depends on the recording. Named metadata is checked
    // against the complete resolved input list when the content binding is made.
    let current = key.refresh_recording()?;
    let file = if current == key {
        store.read(&key, SECTION, 0)?
    } else {
        store.read_prefix(&key, &current, SECTION, 0)?
    };
    let Some(file) = file else { return Ok(None) };
    let cached: Cached = serde_json::from_reader(io::BufReader::new(file))?;
    let inputs = Inputs::read(replay)?;
    if cached.schema != SCHEMA || !cached.inputs.is_prefix_of(&inputs, replay)? {
        return Ok(None);
    }
    let metadata = metadata::read_append(
        replay,
        path,
        cancelled,
        &cached.metadata,
        cached.inputs.index.len(),
    )?;
    Ok(Some(Restored {
        metadata,
        inputs: current.clone(),
        named: cached
            .named
            .map(|(content, metadata)| (current, content, metadata)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{CAMPAIGN, continuation_records, records, scale_frame};
    use replay_data::rows::TableId;

    struct Fixture {
        root: PathBuf,
        path: PathBuf,
        store: Arc<store::Store>,
    }
    impl Fixture {
        fn new(replay: &Replay) -> Self {
            let root =
                std::env::temp_dir().join(format!("metadata-prefix-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&root).unwrap();
            let path = root.join("synthetic.battleplan");
            std::fs::write(&path, replay.bytes()).unwrap();
            let store = crate::prepared::recording_store(&path).unwrap();
            Self { root, path, store }
        }
        fn open(&self) -> Source {
            Source::open(
                super::super::SourceKey {
                    source_id: "metadata-prefix".into(),
                    revision: crate::protocol::replay_revision(&self.path).unwrap(),
                },
                self.path.clone(),
                &AtomicBool::new(false),
            )
            .unwrap()
        }
        fn save(&self, source: &Source) {
            let key = store::SourceKey::capture(&self.path, &[]).unwrap();
            let mut writer = self.store.replace(key).unwrap();
            write(&mut writer, source).unwrap();
            writer.commit().unwrap();
        }
        fn replace_with_rejected_old_payload(&self, replay: &Replay) {
            let mut bytes = replay.bytes().to_vec();
            let old = &replay.index[0];
            bytes[old.offset as usize..(old.offset + old.length) as usize].fill(0);
            std::fs::write(&self.path, bytes).unwrap();
            let rejected = Replay::open_path(&self.path).unwrap();
            assert!(
                rejected
                    .visit_block(&rejected.index[0], None, |_| Ok(false))
                    .is_err()
            );
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.store.clear().unwrap();
            std::fs::remove_dir_all(&self.root).unwrap();
        }
    }

    #[test]
    fn growth_reopens_raw_and_named_metadata_without_reading_old_payload() {
        let initial =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![scale_frame(1, 10, 2)])])
                .unwrap();
        let fixture = Fixture::new(&initial);
        let source = fixture.open();
        source.name_countries(None);
        let expected = source.metadata();
        fixture.save(&source);
        let grown = initial
            .append(&[continuation_records(
                &initial,
                1,
                vec![scale_frame(1, 11, 2)],
            )])
            .unwrap();
        fixture.replace_with_rejected_old_payload(&grown);
        let reopened = fixture.open();
        assert!(reopened.metadata().damaged.is_none());
        assert_eq!(reopened.metadata().branches[0].last_hour, 11);
        assert_eq!(
            reopened.metadata().countries.len(),
            expected.countries.len()
        );
        reopened.name_countries(None);
        assert_eq!(
            reopened.metadata().countries[0].name,
            expected.countries[0].name
        );
    }

    #[test]
    fn raw_prefix_metadata_does_not_revalidate_content() {
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![scale_frame(1, 10, 2)])])
                .unwrap();
        let fixture = Fixture::new(&replay);
        let source = fixture.open();
        let content = fixture.root.join("synthetic-mod");
        std::fs::write(&content, b"synthetic mod").unwrap();
        let key = store::SourceKey::capture(&fixture.path, std::slice::from_ref(&content)).unwrap();
        let mut writer = fixture.store.replace(key).unwrap();
        write(&mut writer, &source).unwrap();
        writer.commit().unwrap();
        let grown = replay
            .append(&[continuation_records(
                &replay,
                1,
                vec![scale_frame(1, 11, 2)],
            )])
            .unwrap();
        std::fs::write(&fixture.path, grown.bytes()).unwrap();
        std::fs::write(&content, b"changed synthetic mod").unwrap();
        let current = Arc::new(Replay::open_path(&fixture.path).unwrap());
        assert!(
            read(&fixture.path, &current, &AtomicBool::new(false))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn appended_branch_catalogue_reuses_metadata_and_resolves_keyframe_anchors() {
        let initial = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![scale_frame(1, 10, 2), scale_frame(1, 11, 3)],
            )],
        )
        .unwrap();
        let fixture = Fixture::new(&initial);
        let source = fixture.open();
        fixture.save(&source);
        let grown = initial
            .append(&[records(2, vec![scale_frame(2, 20, 3)])])
            .unwrap();
        // An appended keyframe can use an earlier branch's keyframe as an
        // anchor. Preserve those entries while damaging old later Countries
        // entries: resolving an anchor succeeds, rereading that catalogue fails.
        let old = &grown.index[0];
        let sections = grown.block_sections(old).unwrap().unwrap();
        let later = sections
            .iter()
            .rposition(|entry| {
                matches!(
                    entry.section,
                    replay::container::Section::Later(TableId::Countries)
                )
            })
            .unwrap();
        let after: u64 = sections[later + 1..]
            .iter()
            .map(|entry| entry.compressed)
            .sum();
        let mut bytes = grown.bytes().to_vec();
        bytes[(old.offset + old.length - after - 1) as usize] ^= 0x40;
        std::fs::write(&fixture.path, &bytes).unwrap();
        let resident = Replay::open(bytes).unwrap();
        let held = Replay::open_path(&fixture.path).unwrap();
        let selected = &[
            TableId::Campaign,
            TableId::Countries,
            TableId::CountryLinks,
            TableId::LocalPlayer,
        ];
        assert!(
            resident
                .read_block_selected(&resident.index[0], selected)
                .is_err()
        );
        assert!(held.read_block_selected(&held.index[0], selected).is_err());
        assert_eq!(
            grown
                .read_block_selected(&grown.index[1], selected)
                .unwrap(),
            resident
                .read_block_selected(&resident.index[1], selected)
                .unwrap()
        );
        assert_eq!(
            resident
                .read_block_selected(&resident.index[1], selected)
                .unwrap(),
            held.read_block_selected(&held.index[1], selected).unwrap()
        );
        let reopened = fixture.open();
        assert!(reopened.metadata().damaged.is_none());
        assert_eq!(reopened.metadata().branches.len(), 2);
        assert_eq!(reopened.metadata().countries.len(), 3);
    }
}
