//! Current presentation metadata, keyed by accepted recording/content stamps.
use super::*;
use replay_data::identity::IdentityCodec;
use serde::{Deserialize, Serialize};

mod stream;

const SCHEMA: u32 = 3;
const SECTION: &str = "source-metadata-v1";
const REFUSAL_SCHEMA: &str = "source-metadata-refusals-v1";
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
#[derive(Default, Serialize, Deserialize)]
struct CachedRefusals<R = MetadataRefusals> {
    raw: R,
    named: Option<R>,
}

#[derive(Serialize, Deserialize)]
struct Cached<M = ReplayMetadata, R = MetadataRefusals> {
    schema: u32,
    inputs: Inputs,
    metadata: M,
    named: Option<(NamesKey, M)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refusals: Option<CachedRefusals<R>>,
}

pub fn write(writer: &mut store::Writer, source: &Source) -> io::Result<()> {
    let named = source
        .named
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let cache = Cached {
        schema: SCHEMA,
        inputs: Inputs::read(source.replay_ref())?,
        metadata: &source.metadata.value,
        named: named
            .as_ref()
            .map(|(content, metadata)| (content.clone(), &metadata.value)),
        refusals: (!source.metadata.refusals.is_empty()
            || named
                .as_ref()
                .is_some_and(|(_, metadata)| !metadata.refusals.is_empty()))
        .then(|| CachedRefusals {
            raw: &source.metadata.refusals,
            named: named.as_ref().map(|(_, metadata)| &metadata.refusals),
        }),
    };
    stream::write(writer, SECTION, &cache)?;
    writer.write_at(REFUSAL_SCHEMA, 0, &[1u8][..])
}
/// Stale/corrupt derived metadata is a miss, never a refusal to open a recording.
pub(super) type CachedNames = (store::SourceKey, NamesKey, ResidentMetadata);
pub(super) struct Restored {
    pub metadata: ResidentMetadata,
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
    let current_path = std::fs::canonicalize(path)?;
    let current = key.refresh_recording(&current_path)?;
    let file = if current == key {
        store.read(&key, SECTION, 0)?
    } else {
        store.read_prefix(&key, &current, SECTION, 0)?
    };
    let Some(file) = file else { return Ok(None) };
    // Earlier caches stored formatted refusal summaries instead of structured identities.
    let stamp = if current == key {
        store.read(&key, REFUSAL_SCHEMA, 0)?
    } else {
        store.read_prefix(&key, &current, REFUSAL_SCHEMA, 0)?
    };
    let Some(mut stamp) = stamp else {
        return Ok(None);
    };
    let mut version = [0; 1];
    std::io::Read::read_exact(&mut stamp, &mut version)?;
    if version != [1] {
        return Ok(None);
    }
    let cached: Cached = serde_json::from_reader(io::BufReader::new(file))?;
    let inputs = Inputs::read(replay)?;
    if cached.schema != SCHEMA || !cached.inputs.is_prefix_of(&inputs, replay)? {
        return Ok(None);
    }
    let mut refusals = cached.refusals.unwrap_or_default();
    let previous = ResidentMetadata {
        value: cached.metadata,
        refusals: refusals.raw,
    };
    let metadata = metadata::read_append(
        replay,
        path,
        cancelled,
        &previous,
        cached.inputs.index.len(),
    )?;
    Ok(Some(Restored {
        metadata,
        inputs: current.clone(),
        named: cached.named.map(|(content, metadata)| {
            (
                current,
                content,
                ResidentMetadata {
                    value: metadata,
                    refusals: refusals.named.take().unwrap_or_default(),
                },
            )
        }),
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
    fn admitted_borrowed_cache_keeps_the_owned_wire_schema_and_named_warnings() {
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![scale_frame(1, 10, 2)])])
                .unwrap();
        let fixture = Fixture::new(&replay);
        let mut source = fixture.open();
        source
            .metadata
            .value
            .warnings
            .extend(["second warning".into(), "first warning".into()]);
        source.name_countries(None).unwrap();
        fixture.save(&source);
        let key = fixture.store.saved_key().unwrap().unwrap();
        let cached: Cached =
            serde_json::from_reader(fixture.store.read(&key, SECTION, 0).unwrap().unwrap())
                .unwrap();
        assert_eq!(cached.metadata, source.metadata.value);
        assert_eq!(
            cached.named.unwrap().1,
            source.named.lock().unwrap().as_ref().unwrap().1.value
        );
    }

    #[derive(Serialize)]
    struct MainCache {
        schema: u32,
        inputs: Inputs,
        metadata: ReplayMetadata,
        named: Option<(NamesKey, ReplayMetadata)>,
    }

    fn main_serializer(source: &Source) -> Vec<u8> {
        let named = source.named.lock().unwrap();
        let owned = MainCache {
            schema: SCHEMA,
            inputs: Inputs::read(source.replay_ref()).unwrap(),
            metadata: source.metadata.value.clone(),
            named: named
                .as_ref()
                .map(|(key, metadata)| (key.clone(), metadata.value.clone())),
        };
        serde_json::to_vec(&owned).unwrap()
    }

    fn cache_bytes(fixture: &Fixture) -> Vec<u8> {
        use std::io::Read;
        let key = fixture.store.saved_key().unwrap().unwrap();
        let mut bytes = Vec::new();
        fixture
            .store
            .read(&key, SECTION, 0)
            .unwrap()
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        bytes
    }

    #[test]
    fn streamed_cache_is_byte_identical_to_mains_independent_owned_serializer() {
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![scale_frame(1, 10, 2)])])
                .unwrap();
        let fixture = Fixture::new(&replay);
        let mut source = fixture.open();
        source.metadata.value.warnings.extend([
            "quote: \" newline: \n backslash: \\".into(),
            "UTF-8: \u{e9} \u{1f642}".into(),
        ]);
        fixture.save(&source);
        assert_eq!(cache_bytes(&fixture), main_serializer(&source));
        source.name_countries(None).unwrap();
        fixture.save(&source);
        assert_eq!(cache_bytes(&fixture), main_serializer(&source));
    }

    #[test]
    fn combined_raw_and_named_cache_larger_than_transport_has_no_size_limit() {
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![scale_frame(1, 10, 2)])])
                .unwrap();
        let fixture = Fixture::new(&replay);
        let mut source = fixture.open();
        let suffix = "x".repeat(1000);
        source
            .metadata
            .value
            .warnings
            .extend((0..20_000).map(|i| format!("{i:08}:{suffix}")));
        source.name_countries(None).unwrap();
        assert!(
            serde_json::to_vec(&source.metadata.value).unwrap().len()
                < crate::transport::MAX_REPLY_BYTES
        );
        let expected = main_serializer(&source);
        assert!(expected.len() > crate::transport::MAX_REPLY_BYTES);
        fixture.save(&source);
        assert_eq!(cache_bytes(&fixture), expected);
        let reopened = fixture.open();
        assert_eq!(reopened.metadata.warnings, source.metadata.warnings);
        assert_eq!(
            reopened.cached_named.as_ref().unwrap().2.warnings,
            source.named.lock().unwrap().as_ref().unwrap().1.warnings
        );
    }

    #[test]
    fn caches_with_pre_structured_refusal_summaries_are_misses() {
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![scale_frame(1, 10, 2)])])
                .unwrap();
        let fixture = Fixture::new(&replay);
        let mut source = fixture.open();
        source.metadata.value.warnings.push("Metadata warning storage refused: 9 further distinct warnings were not stored because the complete-response byte budget was exhausted.".into());
        let old_bytes = main_serializer(&source);
        let key = store::SourceKey::capture(&fixture.path, &[]).unwrap();
        let mut writer = fixture.store.replace(key).unwrap();
        writer.write_at(SECTION, 0, old_bytes.as_slice()).unwrap();
        writer.commit().unwrap();
        assert!(
            read(&fixture.path, &source.replay(), &AtomicBool::new(false))
                .unwrap()
                .is_none()
        );
        let reopened = fixture.open();
        assert!(
            !reopened
                .metadata()
                .unwrap()
                .warnings
                .iter()
                .any(|w| w.starts_with("Metadata warning storage refused:"))
        );
    }

    #[test]
    fn two_branch_append_save_and_second_reopen_rebuild_public_damage_bytes_in_branch_order() {
        fn unreadable(recording: &Replay) -> Replay {
            let mut blocks = recording
                .index
                .iter()
                .map(|block| recording.read_block(block).unwrap())
                .collect::<Vec<_>>();
            for frames in &mut blocks {
                frames[0].tables[TableId::Campaign.index()] =
                    replay_frame::frame::TableUpdate::Snapshot(
                        replay_data::rows::TableRows::Campaign(vec![
                            replay_data::rows::CampaignRow {
                                id: b"different synthetic campaign".to_vec(),
                            },
                        ]),
                    );
            }
            replay::container::test_work::with_independent_block_frames(
                recording,
                &blocks.iter().map(Vec::as_slice).collect::<Vec<_>>(),
            )
            .unwrap()
        }
        let initial = unreadable(
            &Replay::from_records(
                CAMPAIGN.into(),
                &[
                    records(1, vec![scale_frame(1, 10, 2)]),
                    records(2, vec![scale_frame(2, 20, 2)]),
                ],
            )
            .unwrap(),
        );
        let fixture = Fixture::new(&initial);
        let first = fixture.open();
        assert!(
            first
                .metadata
                .branches
                .iter()
                .all(|branch| branch.catalogue_hour.is_none())
        );
        fixture.save(&first);
        let appended = unreadable(
            &initial
                .append(&[continuation_records(
                    &initial,
                    1,
                    vec![scale_frame(1, 11, 2)],
                )])
                .unwrap(),
        );
        for (old, new) in initial.index.iter().zip(&appended.index) {
            assert_eq!(
                (old.offset, old.length, old.raw_length),
                (new.offset, new.length, new.raw_length)
            );
        }
        std::fs::write(&fixture.path, appended.bytes()).unwrap();
        let restored = fixture.open();
        assert!(restored.cached_inputs.is_some());
        let appended_bytes = concat!(
            r#"["branch blocks are not contiguous","session blocks are not contiguous","Branch 1 block 10 – 10 is unreadable: frame campaign identity differs from replay header","#,
            r#""Branch 2 block 20 – 20 is unreadable: frame campaign identity differs from replay header","#,
            r#""Branch 1 block 11 – 11 is unreadable: frame campaign identity differs from replay header"]"#,
        );
        assert_eq!(
            serde_json::to_vec(&restored.metadata().unwrap().damaged.unwrap().reasons).unwrap(),
            appended_bytes.as_bytes()
        );
        fixture.save(&restored);
        let reopened = fixture.open();
        assert!(reopened.cached_inputs.is_some());
        let expected_damage = concat!(
            r#"{"parts":3,"firstHour":10,"lastHour":20,"reasons":["branch blocks are not contiguous","session blocks are not contiguous","Branch 1 block 10 – 10 is unreadable: frame campaign identity differs from replay header","#,
            r#""Branch 1 block 11 – 11 is unreadable: frame campaign identity differs from replay header","#,
            r#""Branch 2 block 20 – 20 is unreadable: frame campaign identity differs from replay header"]}"#,
        );
        let wire = reopened.metadata().unwrap();
        assert_eq!(
            serde_json::to_vec(&wire.damaged).unwrap(),
            expected_damage.as_bytes()
        );
        let mut expected_warnings = appended.compatibility_gaps();
        expected_warnings.extend(serde_json::from_str::<Vec<String>>(appended_bytes).unwrap());
        assert_eq!(
            serde_json::to_vec(&wire.warnings).unwrap(),
            serde_json::to_vec(&expected_warnings).unwrap()
        );
        fixture.save(&reopened);
        let cached: Cached = serde_json::from_slice(&cache_bytes(&fixture)).unwrap();
        assert_eq!(
            serde_json::to_vec(&cached.metadata.damaged).unwrap(),
            expected_damage.as_bytes()
        );
        let again = fixture.open();
        assert_eq!(
            serde_json::to_vec(&again.metadata().unwrap().damaged).unwrap(),
            expected_damage.as_bytes()
        );
    }

    #[test]
    fn exhausted_allocation_cache_reopen_and_append_do_not_import_wire_summaries() {
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![scale_frame(1, 10, 2)])])
                .unwrap();
        let fixture = Fixture::new(&replay);
        let mut source = fixture.open();
        let mut warnings = Warnings::new().unwrap();
        warnings
            .import(
                &source.metadata.warnings,
                &source.metadata.refusals.warnings,
            )
            .unwrap();
        warnings.warn("retained diagnostic").unwrap();
        let mut reasons = Warnings::new().unwrap();
        let damage = "synthetic refused block diagnostic";
        metadata::with_storage_failure_after(0, || {
            warnings.warn(damage).unwrap();
            reasons.warn(damage).unwrap();
        });
        let (values, refused) = warnings.finish();
        source.metadata.value.warnings = values;
        source.metadata.refusals.warnings = refused;
        let (values, refused) = reasons.finish();
        source.metadata.value.damaged = Some(crate::DamagedRecording {
            parts: 1,
            first_hour: Some(10),
            last_hour: Some(10),
            reasons: values,
        });
        source.metadata.refusals.reasons = refused;
        source.metadata.value.branches[0]
            .gaps
            .push(crate::RecordingGap {
                first_hour: 10,
                last_hour: 10,
                reason: damage.into(),
            });
        source.name_countries(None).unwrap();
        fixture.save(&source);
        let cached: serde_json::Value = serde_json::from_slice(&cache_bytes(&fixture)).unwrap();
        assert_eq!(
            cached["refusals"]["raw"]["warnings"],
            serde_json::json!([damage])
        );
        assert!(
            !cached["metadata"]["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|w| w
                    .as_str()
                    .unwrap()
                    .starts_with("Metadata warning storage refused:"))
        );
        let mut named_count = source
            .named
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .1
            .refusals
            .warnings
            .count();
        for grown in [false, true] {
            if grown {
                let appended = replay
                    .append(&[continuation_records(
                        &replay,
                        1,
                        vec![scale_frame(1, 11, 2)],
                    )])
                    .unwrap();
                std::fs::write(&fixture.path, appended.bytes()).unwrap();
            }
            let reopened = fixture.open();
            assert!(
                reopened.cached_inputs.is_some(),
                "must restore the cache rather than reread"
            );
            assert_eq!(reopened.metadata.refusals.warnings.count(), 1);
            assert_eq!(reopened.metadata.refusals.reasons.count(), 1);
            assert_eq!(
                reopened
                    .cached_named
                    .as_ref()
                    .unwrap()
                    .2
                    .refusals
                    .warnings
                    .count(),
                named_count
            );
            metadata::with_storage_failure_after(0, || reopened.name_countries(None).unwrap());
            named_count = reopened
                .named
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .1
                .refusals
                .warnings
                .count();
            let wire = reopened.metadata().unwrap();
            assert_eq!(
                wire.warnings
                    .iter()
                    .filter(|w| w.starts_with("Metadata warning storage refused:"))
                    .count(),
                1
            );
            assert_eq!(
                wire.damaged
                    .unwrap()
                    .reasons
                    .iter()
                    .filter(|w| w.starts_with("Metadata warning storage refused:"))
                    .count(),
                1
            );
            fixture.save(&reopened);
        }
    }

    #[test]
    fn ordinary_absolute_names_restore_the_canonical_preparation_key_unchanged() {
        let replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![scale_frame(1, 10, 2)])])
                .unwrap();
        let mut fixture = Fixture::new(&replay);
        std::fs::create_dir(fixture.root.join("alias")).unwrap();
        fixture.path = fixture.root.join("alias/../synthetic.battleplan");
        assert!(fixture.path.is_absolute());
        assert_ne!(fixture.path, std::fs::canonicalize(&fixture.path).unwrap());
        let source = fixture.open();
        let (store, key) = crate::prepared::files(&source, None).unwrap();
        let mut writer = store.replace(key.clone()).unwrap();
        write(&mut writer, &source).unwrap();
        writer.commit().unwrap();
        let restored = read(&fixture.path, &source.replay(), &AtomicBool::new(false))
            .unwrap()
            .unwrap();
        assert_eq!(restored.inputs, key);
        assert_eq!(
            restored.metadata.countries.len(),
            source.metadata().unwrap().countries.len()
        );
    }

    #[test]
    fn growth_reopens_raw_and_named_metadata_without_reading_old_payload() {
        let initial =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![scale_frame(1, 10, 2)])])
                .unwrap();
        let fixture = Fixture::new(&initial);
        let source = fixture.open();
        source.name_countries(None).unwrap();
        let expected = source.metadata().unwrap();
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
        assert!(reopened.metadata().unwrap().damaged.is_none());
        assert_eq!(reopened.metadata().unwrap().branches[0].last_hour, 11);
        assert_eq!(
            reopened.metadata().unwrap().countries.len(),
            expected.countries.len()
        );
        reopened.name_countries(None).unwrap();
        assert_eq!(
            reopened.metadata().unwrap().countries[0].name,
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
        assert!(reopened.metadata().unwrap().damaged.is_none());
        assert_eq!(reopened.metadata().unwrap().branches.len(), 2);
        assert_eq!(reopened.metadata().unwrap().countries.len(), 3);
    }
}
