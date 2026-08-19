use super::*;
use crate::codec::{
    encode_branches, encode_bundle_metadata_entry_descriptors, encode_schemas, SchemaDefinition,
};
use crate::model::{BRANCHES_PATH, BUNDLE_PATH, PROVINCES_PATH, SCHEMAS_PATH, SYMBOLS_PATH};
use compact_contract::stream_validator::StreamIdentityMode;
use compact_contract::{
    COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_HEADER_LEN,
    COMPACT_RECORD_MAGIC, COMPACT_RECORD_TYPE_DOMAIN_DATA, COMPACT_RECORD_TYPE_DOMAIN_GAP,
    COMPACT_RECORD_TYPE_DOMAIN_PERF, COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_VERSION,
    COMPACT_STATUS_DELTA_DATA,
};
use portable_content_manifest::symbol_manifest::SymbolManifestBuilder;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use wire_format::body::{encode_shaped_baseline, key, CollectionState, Row};
use wire_format::frame::{BlockHeader, BlockWriter, Frame, Kind};
use wire_format::gap::{encode_record, GapPart, GapReason, GapRecord, GapWitness};
use wire_format::{Record, Value};

#[derive(Clone, Copy, Debug)]
struct RecordExtent {
    sequence: u64,
    start: u64,
    end: u64,
}

#[derive(Debug)]
struct TestDir(PathBuf);

impl TestDir {
    fn create() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "wire-product-contract-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn authenticated_catalog() -> AuthenticatedProvinceCatalog {
    let definition_csv = b"1;1;2;3;land;false;plains;1\n".to_vec();
    AuthenticatedProvinceCatalog {
        content_manifest_sha256: [1; 32],
        definition_csv_sha256: Sha256::digest(&definition_csv).into(),
        bindings: vec![ProvinceBinding {
            province_id: 1,
            symbol: "province/rgb/1/2/3".into(),
        }],
        definition_csv,
    }
}

fn push_h4cr_record(
    out: &mut Vec<u8>,
    sequence: u64,
    tick: i32,
    domain: u16,
    record_type: u16,
    record_flags: u32,
    payload: &[u8],
) -> RecordExtent {
    let start = out.len() as u64;
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&tick.to_le_bytes());
    out.extend_from_slice(&domain.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&record_flags.to_le_bytes());
    out.extend_from_slice(payload);
    RecordExtent {
        sequence,
        start,
        end: out.len() as u64,
    }
}

fn perf_payload(status: u32) -> Vec<u8> {
    let mut payload = vec![0u8; 40];
    payload[16..20].copy_from_slice(&9u32.to_le_bytes());
    payload[20..24].copy_from_slice(&7u32.to_le_bytes());
    payload[32..36].copy_from_slice(&status.to_le_bytes());
    payload
}

fn d81_v3_payload(status: u32) -> Vec<u8> {
    compact_contract::rail_replay::encode_payload_with_hub_simulation(&[], &[], 0, status, 0, 0)
        .unwrap()
}

fn d18_country_payload(status: u32) -> Vec<u8> {
    d18_country_payload_for(status, 7, "GER")
}

fn d18_country_payload_for(status: u32, source_id: u32, tag: &str) -> Vec<u8> {
    assert_eq!(tag.len(), 3);
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&status.to_le_bytes());
    payload.extend_from_slice(&source_id.to_le_bytes());
    payload.extend_from_slice(&(tag.len() as u16).to_le_bytes());
    payload.extend_from_slice(tag.as_bytes());
    payload.push(1);
    payload.extend_from_slice(b"CIM1");
    payload.extend_from_slice(&1u16.to_le_bytes());
    payload.extend_from_slice(&40u16.to_le_bytes());
    payload.extend_from_slice(&[0; 32]);
    payload
}

fn frame_end_payload(emitted: u32, scheduled: u32, status: u32) -> Vec<u8> {
    let mut payload = Vec::with_capacity(COMPACT_FRAME_END_PAYLOAD_LEN);
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&emitted.to_le_bytes());
    payload.extend_from_slice(&scheduled.to_le_bytes());
    payload.extend_from_slice(&status.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload
}

fn valid_hour(offset: i32) -> i32 {
    compact_contract::calendar::DATE_EPOCH_CONST + 1936 * 365 * 24 + offset
}

fn append_d81_gap_hour(
    out: &mut Vec<u8>,
    next_sequence: &mut u64,
    carrier: compact_contract::DomainGapRecordV1,
) -> (RecordExtent, RecordExtent) {
    let tick = carrier.target_hour_raw;
    let first = push_d81_gap_record(out, next_sequence, carrier);
    let last = push_h4cr_record(
        out,
        *next_sequence,
        tick,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(1, 1, 0),
    );
    *next_sequence += 1;
    (first, last)
}

fn push_d81_gap_record(
    out: &mut Vec<u8>,
    next_sequence: &mut u64,
    carrier: compact_contract::DomainGapRecordV1,
) -> RecordExtent {
    push_domain_gap_record(out, next_sequence, wire_format::d81::DOMAIN as u16, carrier)
}

fn push_domain_gap_record(
    out: &mut Vec<u8>,
    next_sequence: &mut u64,
    domain: u16,
    carrier: compact_contract::DomainGapRecordV1,
) -> RecordExtent {
    let payload = carrier.encode().unwrap();
    let extent = push_h4cr_record(
        out,
        *next_sequence,
        carrier.target_hour_raw,
        domain,
        COMPACT_RECORD_TYPE_DOMAIN_GAP,
        0,
        &payload,
    );
    *next_sequence += 1;
    extent
}

fn append_d81_hour(
    out: &mut Vec<u8>,
    next_sequence: &mut u64,
    tick: i32,
    perf_status: u32,
    data_status: u32,
) -> (RecordExtent, RecordExtent) {
    let first = push_h4cr_record(
        out,
        *next_sequence,
        tick,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf_payload(perf_status),
    );
    *next_sequence += 1;
    push_h4cr_record(
        out,
        *next_sequence,
        tick,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &d81_v3_payload(data_status),
    );
    *next_sequence += 1;
    let last = push_h4cr_record(
        out,
        *next_sequence,
        tick,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(1, 1, perf_status),
    );
    *next_sequence += 1;
    (first, last)
}

fn append_d81_d18_country_hour(
    out: &mut Vec<u8>,
    next_sequence: &mut u64,
    tick: i32,
    source_id: u32,
    tag: &str,
) {
    for (domain, record_type, payload) in [
        (
            wire_format::d81::DOMAIN as u16,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            perf_payload(0),
        ),
        (
            wire_format::d81::DOMAIN as u16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            d81_v3_payload(0),
        ),
        (18, COMPACT_RECORD_TYPE_DOMAIN_PERF, perf_payload(0)),
        (
            18,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            d18_country_payload_for(0, source_id, tag),
        ),
    ] {
        push_h4cr_record(out, *next_sequence, tick, domain, record_type, 0, &payload);
        *next_sequence += 1;
    }
    push_h4cr_record(
        out,
        *next_sequence,
        tick,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(2, 2, 0),
    );
    *next_sequence += 1;
}

#[test]
fn d18_country_authority_requires_status_bound_perf_data_in_the_closed_frame() {
    let tick = valid_hour(0);
    let mut missing_perf = Vec::new();
    push_h4cr_record(
        &mut missing_perf,
        1,
        tick,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf_payload(0),
    );
    push_h4cr_record(
        &mut missing_perf,
        2,
        tick,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &d81_v3_payload(0),
    );
    push_h4cr_record(
        &mut missing_perf,
        3,
        tick,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &d18_country_payload(0),
    );
    push_h4cr_record(
        &mut missing_perf,
        4,
        tick,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(1, 1, 0),
    );
    let error = inspect_d81_session(H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::Bytes(&missing_perf),
    })
    .unwrap_err();
    assert!(error.to_string().contains("D18 PERF/DATA binding differs"));

    let mut status_mismatch = Vec::new();
    push_h4cr_record(
        &mut status_mismatch,
        1,
        tick,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf_payload(0),
    );
    push_h4cr_record(
        &mut status_mismatch,
        2,
        tick,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &d81_v3_payload(0),
    );
    push_h4cr_record(
        &mut status_mismatch,
        3,
        tick,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf_payload(compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE),
    );
    push_h4cr_record(
        &mut status_mismatch,
        4,
        tick,
        18,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &d18_country_payload(0),
    );
    push_h4cr_record(
        &mut status_mismatch,
        5,
        tick,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(2, 2, compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE),
    );
    let error = inspect_d81_session(H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::Bytes(&status_mismatch),
    })
    .unwrap_err();
    assert!(error.to_string().contains("D18 PERF/DATA binding differs"));
}

#[test]
fn inspect_rejects_cross_frame_d18_inverse_aliasing() {
    let mut stream = Vec::new();
    let mut sequence = 1;
    append_d81_d18_country_hour(&mut stream, &mut sequence, valid_hour(0), 7, "GER");
    append_d81_d18_country_hour(&mut stream, &mut sequence, valid_hour(1), 8, "GER");
    assert!(inspect_d81_session(legacy_bytes_session(&stream))
        .unwrap_err()
        .to_string()
        .contains("aliases source ids 7 and 8"));
}

fn source_digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn h4cr_branch(first_tick: u64, bytes: &[u8]) -> BranchSpec {
    BranchSpec {
        branch: 0,
        parent: None,
        first_tick,
        session_fingerprint: source_digest(bytes),
    }
}

fn legacy_bytes_session(bytes: &[u8]) -> H4crSession<'_> {
    H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::Bytes(bytes),
    }
}

fn root_branch() -> BranchSpec {
    BranchSpec {
        branch: 0,
        parent: None,
        first_tick: 10,
        session_fingerprint: [7; 32],
    }
}

fn states() -> Vec<CollectionState> {
    vec![
        CollectionState::Keyed(
            [(key(&[0]), Record::new(vec![Value::Bool(true)]))]
                .into_iter()
                .collect(),
        ),
        CollectionState::Keyed(Default::default()),
        CollectionState::OrderedKeyed(Vec::<Row>::new()),
        CollectionState::Keyed(Default::default()),
    ]
}

fn block(index: u64, tick: u64) -> BundleEntry {
    block_on(0, index, tick)
}

fn block_on(branch: u32, index: u64, tick: u64) -> BundleEntry {
    let schema = wire_format::d81::schema();
    let mut body = Vec::new();
    encode_shaped_baseline(&schema, &states(), &mut body).unwrap();
    let mut writer = BlockWriter::new(
        BlockHeader {
            domain: wire_format::d81::DOMAIN,
            branch,
            schema_hash: schema.schema_hash(),
            first_tick: tick,
        },
        1,
    );
    writer
        .push(Frame::state(Kind::Baseline, tick, body))
        .unwrap();
    BundleEntry::new(
        format!("rebuilt_wire/blocks/d81-b{branch}-{index:08}.bin"),
        ArtifactRole::StateBlock,
        writer.finish().unwrap(),
    )
    .unwrap()
}

fn gap(index: u64, start: u64, end: u64) -> BundleEntry {
    gap_on(0, index, start, end)
}

fn gap_on(branch: u32, index: u64, start: u64, end: u64) -> BundleEntry {
    let gap = GapRecord {
        tick_start: start,
        tick_end: end,
        branch,
        reason: GapReason::Skipped,
        part: GapPart::State,
        domains: vec![wire_format::d81::DOMAIN],
        witness: GapWitness::TickBracket {
            start_tick: start,
            end_tick: end,
        },
    };
    BundleEntry::new(
        format!("rebuilt_wire/gaps/d81-b{branch}-{index:08}.bin"),
        ArtifactRole::GapRecord,
        encode_record(&gap).unwrap(),
    )
    .unwrap()
}

fn gap_entry(branch: u32, index: u64, gap: GapRecord) -> BundleEntry {
    BundleEntry::new(
        format!("rebuilt_wire/gaps/d81-b{branch}-{index:08}.bin"),
        ArtifactRole::GapRecord,
        encode_record(&gap).unwrap(),
    )
    .unwrap()
}

fn fixture(domain_entries: Vec<BundleEntry>) -> PreparedBundle {
    fixture_with_branches(vec![root_branch()], domain_entries)
}

fn fixture_with_branches(
    branches: Vec<BranchSpec>,
    domain_entries: Vec<BundleEntry>,
) -> PreparedBundle {
    PreparedBundle::from_entries(fixture_entries(branches, domain_entries)).unwrap()
}

fn unchecked_fixture(domain_entries: Vec<BundleEntry>) -> PreparedBundle {
    let entries = fixture_entries(vec![root_branch()], domain_entries);
    let mut bundle = PreparedBundle::create_spool(std::env::temp_dir().as_path()).unwrap();
    for entry in entries {
        bundle
            .push_bytes(entry.path, entry.role, &entry.bytes)
            .unwrap();
    }
    bundle
        .entries
        .sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    bundle
}

fn fixture_entries(
    branches: Vec<BranchSpec>,
    mut domain_entries: Vec<BundleEntry>,
) -> Vec<BundleEntry> {
    let branch_table = encode_branches(&branches).unwrap();
    let schema = wire_format::d81::schema();
    let schemas = encode_schemas(&[SchemaDefinition {
        domain: schema.domain,
        schema_hash: schema.schema_hash(),
        canonical_bytes: schema.canonical_bytes(),
    }])
    .unwrap();
    let symbols = SymbolManifestBuilder::from_base(["province/rgb/1/2/3"])
        .unwrap()
        .manifest()
        .encode_json()
        .unwrap();
    let catalog = authenticated_catalog();
    let mut entries = vec![
        BundleEntry::new(BRANCHES_PATH, ArtifactRole::BranchTable, branch_table).unwrap(),
        BundleEntry::new(SCHEMAS_PATH, ArtifactRole::SchemaRegistry, schemas).unwrap(),
        BundleEntry::new(SYMBOLS_PATH, ArtifactRole::SymbolManifest, symbols).unwrap(),
        BundleEntry::new(
            PROVINCES_PATH,
            ArtifactRole::ProvinceCatalog,
            catalog.encode_artifact().unwrap(),
        )
        .unwrap(),
    ];
    entries.append(&mut domain_entries);
    let descriptors = entries
        .iter()
        .map(BundleEntryDescriptor::from)
        .collect::<Vec<_>>();
    let metadata = encode_bundle_metadata_entry_descriptors(
        [1; 32],
        catalog.definition_csv_sha256,
        &descriptors,
    )
    .unwrap();
    entries.push(BundleEntry::new(BUNDLE_PATH, ArtifactRole::BundleMetadata, metadata).unwrap());
    entries
}

#[test]
fn strict_reopen_accepts_exact_single_hour_bundle() {
    let verified = verify_prepared_bundle(&fixture(vec![block(0, 10)])).unwrap();
    assert_eq!(verified.schema_count, 1);
    assert_eq!(verified.symbol_count, 1);
    assert_eq!(verified.branches[0].state_observations, 1);
    assert_eq!(verified.branches[0].gap_hours, 0);
}

#[test]
fn exact_descriptor_set_rejects_rehashed_payload_mutation() {
    let mut fixture = fixture(vec![block(0, 10)]);
    let block = fixture
        .entries
        .iter_mut()
        .find(|entry| entry.role == ArtifactRole::StateBlock)
        .unwrap();
    let mut bytes = portable_test_fs_retry::read(&block.file_path).unwrap();
    bytes.push(0);
    std::fs::write(&block.file_path, &bytes).unwrap();
    block.len += 1;
    block.blake3 = *blake3::hash(&bytes).as_bytes();
    assert!(verify_prepared_bundle(&fixture)
        .unwrap_err()
        .to_string()
        .contains("authenticated descriptor"));
}

#[test]
fn semantic_reopen_rejects_silent_hour_between_blocks() {
    let fixture = unchecked_fixture(vec![block(0, 10), block(1, 12)]);
    assert!(verify_prepared_bundle(&fixture)
        .unwrap_err()
        .to_string()
        .contains("silent d81 coverage"));
}

#[test]
fn explicit_typed_gap_closes_the_same_hour() {
    let fixture = fixture(vec![block(0, 10), gap(0, 11, 12), block(1, 12)]);
    let verified = verify_prepared_bundle(&fixture).unwrap();
    assert_eq!(verified.branches[0].state_observations, 2);
    assert_eq!(verified.branches[0].gap_hours, 1);
    assert_eq!(verified.branches[0].last_tick, 12);
}

#[test]
fn torn_sweep_requires_an_exact_tick_bracket_and_preserves_coverage_union() {
    let torn = GapRecord {
        tick_start: 11,
        tick_end: 12,
        branch: 0,
        reason: GapReason::TornSweep,
        part: GapPart::State,
        domains: vec![wire_format::d81::DOMAIN],
        witness: GapWitness::TickBracket {
            start_tick: 100,
            end_tick: 103,
        },
    };
    let verified = verify_prepared_bundle(&fixture(vec![
        block(0, 10),
        gap_entry(0, 0, torn.clone()),
        block(1, 12),
    ]))
    .unwrap();
    assert_eq!(verified.branches[0].state_observations, 2);
    assert_eq!(verified.branches[0].gap_hours, 1);

    let wrong_witness = GapRecord {
        witness: GapWitness::TickBracket {
            start_tick: 101,
            end_tick: 101,
        },
        ..torn
    };
    let invalid = unchecked_fixture(vec![
        block(0, 10),
        gap_entry(0, 0, wrong_witness),
        block(1, 12),
    ]);
    assert!(verify_prepared_bundle(&invalid)
        .unwrap_err()
        .to_string()
        .contains("reason and witness"));
}

#[test]
fn backpressure_requires_shared_counter_identity_and_exact_hour_count() {
    let queue_counter =
        u64::from(compact_contract::gap_record::GAP_COUNTER_CODE_QUEUE_OVERRUN_HOURS);
    let sink_counter =
        u64::from(compact_contract::gap_record::GAP_COUNTER_CODE_SINK_BACKPRESSURE_HOURS);
    for counter_id in [queue_counter, sink_counter] {
        let backpressure = GapRecord {
            tick_start: 11,
            tick_end: 13,
            branch: 0,
            reason: GapReason::Backpressure,
            part: GapPart::State,
            domains: vec![wire_format::d81::DOMAIN],
            witness: GapWitness::Counter {
                counter_id,
                observed: 2,
            },
        };
        let verified = verify_prepared_bundle(&fixture(vec![
            block(0, 10),
            gap_entry(0, 0, backpressure),
            block(1, 13),
        ]))
        .unwrap();
        assert_eq!(verified.branches[0].gap_hours, 2);
        assert_eq!(verified.branches[0].state_observations, 2);
    }

    for witness in [
        GapWitness::Counter {
            counter_id: u64::from(compact_contract::gap_record::GAP_COUNTER_CODE_COALESCED_HOURS),
            observed: 2,
        },
        GapWitness::Counter {
            counter_id: queue_counter,
            observed: 1,
        },
    ] {
        let invalid = GapRecord {
            tick_start: 11,
            tick_end: 13,
            branch: 0,
            reason: GapReason::Backpressure,
            part: GapPart::State,
            domains: vec![wire_format::d81::DOMAIN],
            witness,
        };
        let fixture = unchecked_fixture(vec![block(0, 10), gap_entry(0, 0, invalid), block(1, 13)]);
        assert!(verify_prepared_bundle(&fixture)
            .unwrap_err()
            .to_string()
            .contains("reason and witness"));
    }
}

#[test]
fn schema_registry_rejects_hash_collision_before_publication() {
    let schema = wire_format::d81::schema();
    let mut other = schema.canonical_bytes();
    other.push(0);
    let error = encode_schemas(&[
        SchemaDefinition {
            domain: 81,
            schema_hash: schema.schema_hash(),
            canonical_bytes: schema.canonical_bytes(),
        },
        SchemaDefinition {
            domain: 82,
            schema_hash: schema.schema_hash(),
            canonical_bytes: other,
        },
    ])
    .unwrap_err();
    assert!(error.to_string().contains("collides"));
}

#[test]
fn branch_parent_is_inclusive_and_child_begins_next_tick() {
    let branches = [
        root_branch(),
        BranchSpec {
            branch: 1,
            parent: Some(BranchParent {
                branch: 0,
                through_tick: 19,
            }),
            first_tick: 20,
            session_fingerprint: [8; 32],
        },
    ];
    assert!(encode_branches(&branches).is_ok());
    let mut wrong = branches;
    wrong[1].first_tick = 21;
    assert!(encode_branches(&wrong).is_err());
}

#[test]
fn child_may_inherit_through_an_authenticated_parent_gap() {
    let branches = vec![
        root_branch(),
        BranchSpec {
            branch: 1,
            parent: Some(BranchParent {
                branch: 0,
                through_tick: 10,
            }),
            first_tick: 11,
            session_fingerprint: [8; 32],
        },
    ];
    let verified = verify_prepared_bundle(&fixture_with_branches(
        branches,
        vec![gap_on(0, 0, 10, 11), block_on(1, 0, 11)],
    ))
    .unwrap();
    assert_eq!(verified.branches[0].gap_hours, 1);
    assert_eq!(verified.branches[1].first_tick, 11);
}

#[test]
fn materialization_is_file_backed_and_raii_cleaned() {
    let fixture = fixture(vec![block(0, 10)]);
    let parent = std::env::temp_dir();
    let root = {
        let materialized = fixture.materialize(&parent).unwrap();
        assert_eq!(materialized.entries.len(), fixture.entries.len());
        assert!(materialized
            .entries
            .iter()
            .all(|entry| entry.file_path.is_file()));
        materialized.root().to_path_buf()
    };
    assert!(!root.exists());
}

#[test]
fn reconstructed_container_entry_constructor_recomputes_integrity() {
    let entry = BundleEntry::from_bytes(
        "rebuilt_wire/test.bin",
        ArtifactRole::StateBlock,
        vec![1, 2, 3],
    )
    .unwrap();
    assert_eq!(entry.len, 3);
    assert_eq!(entry.blake3, *blake3::hash(&[1, 2, 3]).as_bytes());
    assert_eq!(
        ArtifactRole::from_str(ArtifactRole::StateBlock.as_str()).unwrap(),
        ArtifactRole::StateBlock
    );
}

#[test]
fn canonical_d81_v3_data_uses_zero_h4cr_flags_and_perf_status_at_offset_32() {
    let mut stream = Vec::new();
    let mut sequence = 1;
    append_d81_hour(&mut stream, &mut sequence, 10, 0, 0);

    let inspection = inspect_d81_session(legacy_bytes_session(&stream)).unwrap();
    assert_eq!(inspection.first_tick, 10);
    assert_eq!(inspection.last_tick, 10);
    assert_eq!(inspection.data_records, 1);
    assert_eq!(inspection.publication_authoritative_records, 1);
    assert_eq!(inspection.failure_records, 0);
    assert_eq!(inspection.decoded_sha256, source_digest(&stream));
}

#[test]
fn raw_path_retained_raw_and_retained_zstd_reopen_the_same_decoded_source() {
    let mut stream = Vec::new();
    let mut sequence = 1;
    append_d81_hour(&mut stream, &mut sequence, 10, 0, 0);
    let temp = TestDir::create();
    let raw_path = temp.path("source.h4cr");
    let zstd_path = temp.path("source.h4cr.zst");
    std::fs::write(&raw_path, &stream).unwrap();
    std::fs::write(
        &zstd_path,
        zstd::stream::encode_all(stream.as_slice(), 3).unwrap(),
    )
    .unwrap();
    let raw_file = portable_test_fs_retry::open(&raw_path).unwrap();
    let zstd_file = portable_test_fs_retry::open(&zstd_path).unwrap();

    let from_bytes = inspect_d81_session(legacy_bytes_session(&stream)).unwrap();
    let from_path = inspect_d81_session(H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::File(&raw_path),
    })
    .unwrap();
    let from_retained_raw = inspect_d81_session(H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::RetainedFile(&raw_file),
    })
    .unwrap();
    let from_retained_zstd = inspect_d81_session(H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::RetainedFile(&zstd_file),
    })
    .unwrap();

    assert_eq!(from_path, from_bytes);
    assert_eq!(from_retained_raw, from_bytes);
    assert_eq!(from_retained_zstd, from_bytes);
    assert_eq!(from_bytes.decoded_sha256, source_digest(&stream));
}

#[test]
fn public_entry_source_parity_gate_refolds_the_authenticated_h4cr() {
    let mut stream = Vec::new();
    let mut sequence = 1;
    append_d81_hour(&mut stream, &mut sequence, 10, 0, 0);
    let catalog = authenticated_catalog();
    let branch = h4cr_branch(10, &stream);
    let session = legacy_bytes_session(&stream);
    let bundle = build_d81_bundle(
        BundleBuildConfig::default(),
        std::slice::from_ref(&branch),
        &catalog,
        std::slice::from_ref(&session),
    )
    .unwrap();
    let mut descriptors = bundle
        .entries()
        .iter()
        .map(BundleEntryDescriptor::from)
        .collect::<Vec<_>>();
    descriptors.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    let retained = bundle
        .entries()
        .iter()
        .map(|entry| (entry.path.clone(), entry.file_path.clone()))
        .collect::<BTreeMap<_, _>>();

    let verified = verify_entry_source_against_h4cr(
        &descriptors,
        |path| {
            let file = retained
                .get(path)
                .ok_or_else(|| format!("missing retained artifact {path}"))?;
            portable_test_fs_retry::read(file).map_err(|error| error.to_string())
        },
        BundleBuildConfig::default(),
        std::slice::from_ref(&branch),
        &catalog,
        std::slice::from_ref(&session),
        &[],
    )
    .unwrap();
    assert_eq!(verified.branches[0].state_observations, 1);
    assert_eq!(verified.branches[0].gap_hours, 0);
}

#[test]
fn d81_perf_and_data_must_be_singletons_with_identical_status() {
    let data = d81_v3_payload(0);
    let perf = perf_payload(0);
    let end = frame_end_payload(1, 1, 0);

    let mut duplicate_perf = Vec::new();
    push_h4cr_record(
        &mut duplicate_perf,
        1,
        10,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf,
    );
    push_h4cr_record(
        &mut duplicate_perf,
        2,
        10,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf,
    );
    push_h4cr_record(
        &mut duplicate_perf,
        3,
        10,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &data,
    );
    push_h4cr_record(
        &mut duplicate_perf,
        4,
        10,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &end,
    );
    assert!(inspect_d81_session(legacy_bytes_session(&duplicate_perf))
        .unwrap_err()
        .to_string()
        .contains("repeats d81 PERF"));

    let mut duplicate_data = Vec::new();
    push_h4cr_record(
        &mut duplicate_data,
        1,
        10,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf,
    );
    push_h4cr_record(
        &mut duplicate_data,
        2,
        10,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &data,
    );
    push_h4cr_record(
        &mut duplicate_data,
        3,
        10,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &data,
    );
    push_h4cr_record(
        &mut duplicate_data,
        4,
        10,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &end,
    );
    assert!(inspect_d81_session(legacy_bytes_session(&duplicate_data))
        .unwrap_err()
        .to_string()
        .contains("repeats d81 DATA"));

    let mut mismatched = Vec::new();
    push_h4cr_record(
        &mut mismatched,
        1,
        10,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf,
    );
    push_h4cr_record(
        &mut mismatched,
        2,
        10,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &d81_v3_payload(COMPACT_STATUS_DELTA_DATA),
    );
    push_h4cr_record(
        &mut mismatched,
        3,
        10,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(1, 1, 0),
    );
    assert!(inspect_d81_session(legacy_bytes_session(&mismatched))
        .unwrap_err()
        .to_string()
        .contains("PERF/DATA binding differs"));
}

#[test]
fn frame_end_is_mandatory_and_forward_discontinuity_requires_h4dg() {
    let mut missing_end = Vec::new();
    push_h4cr_record(
        &mut missing_end,
        1,
        10,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf_payload(0),
    );
    push_h4cr_record(
        &mut missing_end,
        2,
        10,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &d81_v3_payload(0),
    );
    assert!(inspect_d81_session(legacy_bytes_session(&missing_end))
        .unwrap_err()
        .to_string()
        .contains("unclosed frame 10"));

    let mut discontinuous = Vec::new();
    let mut sequence = 1;
    append_d81_hour(&mut discontinuous, &mut sequence, 10, 0, 0);
    append_d81_hour(&mut discontinuous, &mut sequence, 12, 0, 0);
    assert!(inspect_d81_session(legacy_bytes_session(&discontinuous))
        .unwrap_err()
        .to_string()
        .contains("exact H4DG range is required"));
}

#[test]
fn multi_hour_forward_jump_without_h4dg_is_rejected() {
    let mut stream = Vec::new();
    let mut sequence = 1;
    append_d81_hour(&mut stream, &mut sequence, 10, 0, 0);
    append_d81_hour(&mut stream, &mut sequence, 13, 0, 0);
    append_d81_hour(&mut stream, &mut sequence, 14, 0, 0);

    assert!(inspect_d81_session(legacy_bytes_session(&stream))
        .unwrap_err()
        .to_string()
        .contains("exact H4DG range is required"));
}

#[test]
fn authenticated_h4dg_torn_gap_preserves_tick_witness_and_rebaselines() {
    let h0 = valid_hour(0);
    let h1 = valid_hour(1);
    let h2 = valid_hour(2);
    let mut stream = Vec::new();
    let mut sequence = 1;
    append_d81_hour(&mut stream, &mut sequence, h0, 0, 0);
    append_d81_gap_hour(
        &mut stream,
        &mut sequence,
        compact_contract::DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::State,
            reason: compact_contract::DomainGapReason::Torn,
            target_hour_raw: h1,
            range_start_hour_raw: h1,
            range_end_hour_raw: h1,
            tick_before: 100,
            tick_after: 103,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        },
    );
    append_d81_hour(&mut stream, &mut sequence, h2, 0, 0);

    let catalog = authenticated_catalog();
    let branch = h4cr_branch(u64::try_from(h0).unwrap(), &stream);
    let session = legacy_bytes_session(&stream);
    let bundle = build_d81_bundle(
        BundleBuildConfig::default(),
        std::slice::from_ref(&branch),
        &catalog,
        std::slice::from_ref(&session),
    )
    .unwrap();
    let verified = verify_prepared_bundle(&bundle).unwrap();
    assert_eq!(verified.branches[0].state_observations, 2);
    assert_eq!(verified.branches[0].gap_hours, 1);
    assert_eq!(verified.branches[0].blocks, 2);
    let gap = bundle
        .entries()
        .iter()
        .find(|entry| entry.role == ArtifactRole::GapRecord)
        .map(|entry| {
            wire_format::gap::decode_record(
                &portable_test_fs_retry::read(&entry.file_path).unwrap(),
            )
            .unwrap()
        })
        .unwrap();
    assert_eq!(gap.reason, GapReason::TornSweep);
    assert_eq!(
        gap.witness,
        GapWitness::TickBracket {
            start_tick: 100,
            end_tick: 103,
        }
    );
}

#[test]
fn authenticated_h4dg_overrun_and_backpressure_use_distinct_shared_counters() {
    use compact_contract::{DomainGapReason, DomainGapRecordV1};

    let queue_counter =
        u64::from(compact_contract::gap_record::GAP_COUNTER_CODE_QUEUE_OVERRUN_HOURS);
    let sink_counter =
        u64::from(compact_contract::gap_record::GAP_COUNTER_CODE_SINK_BACKPRESSURE_HOURS);
    for (reason, expected_counter) in [
        (DomainGapReason::Overrun, queue_counter),
        (DomainGapReason::Backpressure, sink_counter),
    ] {
        let h0 = valid_hour(0);
        let h2 = valid_hour(2);
        let h3 = valid_hour(3);
        let mut stream = Vec::new();
        let mut sequence = 1;
        append_d81_hour(&mut stream, &mut sequence, h0, 0, 0);
        let (tick_before, tick_after, queue_capacity, queue_depth, queue_dropped) = match reason {
            DomainGapReason::Overrun => (100, 102, 0, 0, 0),
            DomainGapReason::Backpressure => (100, 100, 8, 8, 2),
            _ => unreachable!(),
        };
        append_d81_gap_hour(
            &mut stream,
            &mut sequence,
            DomainGapRecordV1 {
                part: compact_contract::DomainGapPart::State,
                reason,
                target_hour_raw: h2,
                range_start_hour_raw: valid_hour(1),
                range_end_hour_raw: h2,
                tick_before,
                tick_after,
                queue_capacity,
                queue_depth,
                queue_dropped,
                outcome_ordinal: 0,
            },
        );
        append_d81_hour(&mut stream, &mut sequence, h3, 0, 0);

        let catalog = authenticated_catalog();
        let branch = h4cr_branch(u64::try_from(h0).unwrap(), &stream);
        let session = legacy_bytes_session(&stream);
        let bundle = build_d81_bundle(
            BundleBuildConfig::default(),
            std::slice::from_ref(&branch),
            &catalog,
            std::slice::from_ref(&session),
        )
        .unwrap();
        let verified = verify_prepared_bundle(&bundle).unwrap();
        assert_eq!(verified.branches[0].state_observations, 2);
        assert_eq!(verified.branches[0].gap_hours, 2);
        assert_eq!(verified.branches[0].blocks, 2);
        let gap = bundle
            .entries()
            .iter()
            .find(|entry| entry.role == ArtifactRole::GapRecord)
            .map(|entry| {
                wire_format::gap::decode_record(
                    &portable_test_fs_retry::read(&entry.file_path).unwrap(),
                )
                .unwrap()
            })
            .unwrap();
        assert_eq!(gap.reason, GapReason::Backpressure);
        assert_eq!(
            gap.witness,
            GapWitness::Counter {
                counter_id: expected_counter,
                observed: 2,
            }
        );
    }
}

#[test]
fn partial_h4dg_that_leaves_silent_coverage_is_rejected() {
    let h0 = valid_hour(0);
    let h2 = valid_hour(2);
    let h3 = valid_hour(3);
    let mut stream = Vec::new();
    let mut sequence = 1;
    append_d81_hour(&mut stream, &mut sequence, h0, 0, 0);
    append_d81_gap_hour(
        &mut stream,
        &mut sequence,
        compact_contract::DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::State,
            reason: compact_contract::DomainGapReason::Torn,
            target_hour_raw: h2,
            range_start_hour_raw: h2,
            range_end_hour_raw: h2,
            tick_before: 200,
            tick_after: 201,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        },
    );
    append_d81_hour(&mut stream, &mut sequence, h3, 0, 0);

    assert!(inspect_d81_session(legacy_bytes_session(&stream))
        .unwrap_err()
        .to_string()
        .contains("does not exactly authenticate closed-frame coverage"));
}

#[test]
fn initial_coalesced_h4dg_range_start_is_the_session_and_branch_origin() {
    let h0 = valid_hour(0);
    let h2 = valid_hour(2);
    let h3 = valid_hour(3);
    let mut stream = Vec::new();
    let mut sequence = 1;
    append_d81_gap_hour(
        &mut stream,
        &mut sequence,
        compact_contract::DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::State,
            reason: compact_contract::DomainGapReason::Backpressure,
            target_hour_raw: h2,
            range_start_hour_raw: h0,
            range_end_hour_raw: h2,
            tick_before: 300,
            tick_after: 300,
            queue_capacity: 8,
            queue_depth: 8,
            queue_dropped: 3,
            outcome_ordinal: 0,
        },
    );
    append_d81_hour(&mut stream, &mut sequence, h3, 0, 0);

    let inspection = inspect_d81_session(legacy_bytes_session(&stream)).unwrap();
    assert_eq!(
        inspection.first_tick, h0 as u64,
        "inspection must expose authenticated H4DG coverage, not only its closing frame"
    );
    assert_eq!(inspection.last_tick, h3 as u64);

    let catalog = authenticated_catalog();
    let branch = h4cr_branch(h0 as u64, &stream);
    let session = legacy_bytes_session(&stream);
    let bundle = build_d81_bundle(
        BundleBuildConfig::default(),
        std::slice::from_ref(&branch),
        &catalog,
        std::slice::from_ref(&session),
    )
    .unwrap();
    let verified = verify_prepared_bundle(&bundle).unwrap();
    assert_eq!(verified.branches[0].first_tick, h0 as u64);
    assert_eq!(verified.branches[0].gap_hours, 3);
    assert_eq!(verified.branches[0].state_observations, 1);
}

#[test]
fn authenticated_h4dg_overlap_non_state_and_malformed_payload_fail_closed() {
    use compact_contract::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

    let tick = valid_hour(1);
    let torn = DomainGapRecordV1 {
        part: DomainGapPart::State,
        reason: DomainGapReason::Torn,
        target_hour_raw: tick,
        range_start_hour_raw: tick,
        range_end_hour_raw: tick,
        tick_before: 300,
        tick_after: 301,
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal: 0,
    };

    let mut overlapping = Vec::new();
    let mut sequence = 1;
    append_d81_gap_hour(
        &mut overlapping,
        &mut sequence,
        DomainGapRecordV1 {
            reason: DomainGapReason::Backpressure,
            target_hour_raw: tick,
            range_start_hour_raw: valid_hour(0),
            range_end_hour_raw: tick,
            tick_before: 301,
            tick_after: 301,
            queue_capacity: 8,
            queue_depth: 8,
            queue_dropped: 2,
            outcome_ordinal: 0,
            ..torn
        },
    );
    append_d81_gap_hour(
        &mut overlapping,
        &mut sequence,
        DomainGapRecordV1 {
            reason: DomainGapReason::Backpressure,
            target_hour_raw: valid_hour(2),
            range_start_hour_raw: tick,
            range_end_hour_raw: valid_hour(2),
            tick_before: 302,
            tick_after: 302,
            queue_capacity: 8,
            queue_depth: 8,
            queue_dropped: 2,
            outcome_ordinal: 1,
            ..torn
        },
    );
    assert!(inspect_d81_session(legacy_bytes_session(&overlapping))
        .unwrap_err()
        .to_string()
        .contains("overlap"));

    let mut event_only = Vec::new();
    let mut sequence = 1;
    append_d81_gap_hour(
        &mut event_only,
        &mut sequence,
        DomainGapRecordV1 {
            part: DomainGapPart::Event,
            ..torn
        },
    );
    assert!(inspect_d81_session(legacy_bytes_session(&event_only))
        .unwrap_err()
        .to_string()
        .contains("is not state"));

    let mut malformed = Vec::new();
    let mut payload = torn.encode().unwrap();
    payload[12] = 1;
    push_h4cr_record(
        &mut malformed,
        1,
        tick,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_GAP,
        0,
        &payload,
    );
    push_h4cr_record(
        &mut malformed,
        2,
        tick,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(0, 0, 0),
    );
    assert!(inspect_d81_session(legacy_bytes_session(&malformed))
        .unwrap_err()
        .to_string()
        .contains("reserved"));
}

#[test]
fn authenticated_h4dg_ordinals_are_exact_zero_based_per_domain_part() {
    use compact_contract::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

    for ordinals in [vec![0, 0], vec![0, 2], vec![0, 1, 0]] {
        let mut stream = Vec::new();
        let mut sequence = 1;
        for (offset, ordinal) in ordinals.iter().copied().enumerate() {
            let tick = valid_hour(i32::try_from(offset).unwrap());
            append_d81_gap_hour(
                &mut stream,
                &mut sequence,
                DomainGapRecordV1 {
                    part: DomainGapPart::State,
                    reason: DomainGapReason::Torn,
                    target_hour_raw: tick,
                    range_start_hour_raw: tick,
                    range_end_hour_raw: tick,
                    tick_before: 400 + offset as u64,
                    tick_after: 401 + offset as u64,
                    queue_capacity: 0,
                    queue_depth: 0,
                    queue_dropped: 0,
                    outcome_ordinal: ordinal,
                },
            );
        }
        let error = inspect_d81_session(legacy_bytes_session(&stream))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("outcome ordinal"),
            "ordinal sequence {ordinals:?} was not rejected exactly: {error}"
        );
    }
}

#[test]
fn authenticated_h4dg_parts_have_independent_per_domain_outcome_lanes() {
    use compact_contract::{DomainGapPart, DomainGapReason, DomainGapRecordV1};

    let mut stream = Vec::new();
    let mut sequence = 1;
    for (offset, part) in [
        DomainGapPart::State,
        DomainGapPart::Event,
        DomainGapPart::Both,
    ]
    .into_iter()
    .enumerate()
    {
        let tick = valid_hour(i32::try_from(offset).unwrap());
        push_domain_gap_record(
            &mut stream,
            &mut sequence,
            18,
            DomainGapRecordV1 {
                part,
                reason: DomainGapReason::Torn,
                target_hour_raw: tick,
                range_start_hour_raw: tick,
                range_end_hour_raw: tick,
                tick_before: 500,
                tick_after: 501,
                queue_capacity: 0,
                queue_depth: 0,
                queue_dropped: 0,
                outcome_ordinal: 0,
            },
        );
        push_d81_gap_record(
            &mut stream,
            &mut sequence,
            DomainGapRecordV1 {
                part: DomainGapPart::State,
                reason: DomainGapReason::Torn,
                target_hour_raw: tick,
                range_start_hour_raw: tick,
                range_end_hour_raw: tick,
                tick_before: 500,
                tick_after: 501,
                queue_capacity: 0,
                queue_depth: 0,
                queue_dropped: 0,
                outcome_ordinal: offset as u64,
            },
        );
        push_h4cr_record(
            &mut stream,
            sequence,
            tick,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(2, 2, 0),
        );
        sequence += 1;
    }

    let inspection = inspect_d81_session(legacy_bytes_session(&stream)).unwrap();
    assert_eq!(inspection.first_tick, valid_hour(0) as u64);
    assert_eq!(inspection.last_tick, valid_hour(2) as u64);
    assert_eq!(inspection.publication_authoritative_records, 0);
}

#[test]
fn frame_terminal_outcome_is_exactly_one_perf_or_h4dg_per_domain() {
    let tick = valid_hour(0);
    let mut stream = Vec::new();
    let mut sequence = 1;
    push_d81_gap_record(
        &mut stream,
        &mut sequence,
        compact_contract::DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::State,
            reason: compact_contract::DomainGapReason::Torn,
            target_hour_raw: tick,
            range_start_hour_raw: tick,
            range_end_hour_raw: tick,
            tick_before: 600,
            tick_after: 601,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        },
    );
    push_h4cr_record(
        &mut stream,
        sequence,
        tick,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf_payload(0),
    );
    sequence += 1;
    push_h4cr_record(
        &mut stream,
        sequence,
        tick,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &d81_v3_payload(0),
    );
    sequence += 1;
    push_h4cr_record(
        &mut stream,
        sequence,
        tick,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(1, 1, 0),
    );
    assert!(inspect_d81_session(legacy_bytes_session(&stream))
        .unwrap_err()
        .to_string()
        .contains("repeats terminal outcome"));
}

#[test]
fn duplicate_and_regressing_closed_hours_remain_hard_errors() {
    for later in [10, 9] {
        let mut stream = Vec::new();
        let mut sequence = 1;
        append_d81_hour(&mut stream, &mut sequence, 10, 0, 0);
        append_d81_hour(&mut stream, &mut sequence, later, 0, 0);
        let error = inspect_d81_session(legacy_bytes_session(&stream))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("regresses or duplicates"),
            "tick {later} was not rejected as a closed-hour regression: {error}"
        );
    }
}

#[test]
fn frame_end_emitted_and_scheduled_domain_counts_must_match() {
    let mut stream = Vec::new();
    push_h4cr_record(
        &mut stream,
        1,
        10,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf_payload(0),
    );
    push_h4cr_record(
        &mut stream,
        2,
        10,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &d81_v3_payload(0),
    );
    push_h4cr_record(
        &mut stream,
        3,
        10,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(1, 2, 0),
    );
    let error = inspect_d81_session(legacy_bytes_session(&stream))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("emitted") && error.contains("scheduled"),
        "{error}"
    );
}

#[test]
fn exact_selection_uses_sequence_and_decoded_byte_endpoints_and_emits_tail_gap() {
    let h9 = valid_hour(-1);
    let h10 = valid_hour(0);
    let h11 = valid_hour(1);
    let h12 = valid_hour(2);
    let mut stream = Vec::new();
    let mut sequence = 1;
    append_d81_hour(&mut stream, &mut sequence, h9, 0, 0);
    let (first, _) = append_d81_hour(&mut stream, &mut sequence, h10, 0, 0);
    append_d81_gap_hour(
        &mut stream,
        &mut sequence,
        compact_contract::DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::State,
            reason: compact_contract::DomainGapReason::Torn,
            target_hour_raw: h11,
            range_start_hour_raw: h11,
            range_end_hour_raw: h11,
            tick_before: 20,
            tick_after: 21,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        },
    );
    let (_, last) = append_d81_gap_hour(
        &mut stream,
        &mut sequence,
        compact_contract::DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::State,
            reason: compact_contract::DomainGapReason::Torn,
            target_hour_raw: h12,
            range_start_hour_raw: h12,
            range_end_hour_raw: h12,
            tick_before: 21,
            tick_after: 22,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 1,
        },
    );
    let selection = H4crSelection {
        first_sequence: first.sequence,
        last_sequence: last.sequence,
        start_byte: first.start,
        end_byte: last.end,
    };
    let session = H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::BytesSelection(&stream, selection),
    };
    let inspection = inspect_d81_session(session).unwrap();
    assert_eq!(inspection.first_tick, h10 as u64);
    assert_eq!(inspection.last_tick, h12 as u64);
    assert_eq!(inspection.data_records, 1);
    assert_eq!(inspection.decoded_sha256, source_digest(&stream));

    let branch = h4cr_branch(h10 as u64, &stream);
    let catalog = authenticated_catalog();
    let bundle = build_d81_bundle(
        BundleBuildConfig::default(),
        std::slice::from_ref(&branch),
        &catalog,
        std::slice::from_ref(&session),
    )
    .unwrap();
    let verified = verify_prepared_bundle(&bundle).unwrap();
    assert_eq!(verified.branches[0].state_observations, 1);
    assert_eq!(verified.branches[0].gap_hours, 2);
    assert_eq!(verified.branches[0].last_tick, h12 as u64);

    let retained_paths = bundle
        .entries()
        .iter()
        .map(|entry| (entry.path.clone(), entry.file_path.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut descriptors = bundle
        .entries()
        .iter()
        .map(BundleEntryDescriptor::from)
        .collect::<Vec<_>>();
    descriptors.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    let temp = TestDir::create();
    let source_path = temp.path("selected-source.h4cr");
    std::fs::write(&source_path, &stream).unwrap();
    let retained_source = portable_test_fs_retry::open(source_path).unwrap();
    let retained_session = H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::RetainedFileSelection(&retained_source, selection),
    };
    let parity = verify_entry_source_against_h4cr(
        &descriptors,
        |path| {
            let file = retained_paths
                .get(path)
                .ok_or_else(|| format!("missing retained artifact {path}"))?;
            portable_test_fs_retry::read(file).map_err(|error| error.to_string())
        },
        BundleBuildConfig::default(),
        std::slice::from_ref(&branch),
        &catalog,
        std::slice::from_ref(&retained_session),
        &[],
    )
    .unwrap();
    assert_eq!(parity, verified);

    let wrong_sequence = H4crSelection {
        first_sequence: first.sequence + 1,
        ..selection
    };
    assert!(inspect_d81_session(H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::BytesSelection(&stream, wrong_sequence),
    })
    .unwrap_err()
    .to_string()
    .contains("byte start disagrees with first sequence"));

    let wrong_last_sequence = H4crSelection {
        last_sequence: last.sequence - 1,
        ..selection
    };
    assert!(inspect_d81_session(H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::BytesSelection(&stream, wrong_last_sequence),
    })
    .unwrap_err()
    .to_string()
    .contains("byte end disagrees with last sequence"));

    let inside_record = H4crSelection {
        start_byte: first.start + 1,
        ..selection
    };
    assert!(inspect_d81_session(H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::BytesSelection(&stream, inside_record),
    })
    .unwrap_err()
    .to_string()
    .contains("selection start byte falls inside a record"));

    let truncated_last_record = H4crSelection {
        end_byte: last.end - 1,
        ..selection
    };
    assert!(inspect_d81_session(H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::BytesSelection(&stream, truncated_last_record),
    })
    .unwrap_err()
    .to_string()
    .contains("selection end byte falls inside a record"));
}

#[test]
fn retained_selection_preserves_internal_gap_and_post_gap_baseline() {
    let h9 = valid_hour(-1);
    let h10 = valid_hour(0);
    let h11 = valid_hour(1);
    let h12 = valid_hour(2);
    let h13 = valid_hour(3);
    let h14 = valid_hour(4);
    let h15 = valid_hour(5);
    let mut stream = Vec::new();
    let mut sequence = 1;
    append_d81_hour(&mut stream, &mut sequence, h9, 0, 0);
    let (first, _) = append_d81_hour(&mut stream, &mut sequence, h10, 0, 0);
    append_d81_gap_hour(
        &mut stream,
        &mut sequence,
        compact_contract::DomainGapRecordV1 {
            part: compact_contract::DomainGapPart::State,
            reason: compact_contract::DomainGapReason::Overrun,
            target_hour_raw: h12,
            range_start_hour_raw: h11,
            range_end_hour_raw: h12,
            tick_before: 400,
            tick_after: 402,
            queue_capacity: 0,
            queue_depth: 0,
            queue_dropped: 0,
            outcome_ordinal: 0,
        },
    );
    append_d81_hour(&mut stream, &mut sequence, h13, 0, 0);
    let (_, last) = append_d81_hour(&mut stream, &mut sequence, h14, 0, 0);
    append_d81_hour(&mut stream, &mut sequence, h15, 0, 0);
    let selection = H4crSelection {
        first_sequence: first.sequence,
        last_sequence: last.sequence,
        start_byte: first.start,
        end_byte: last.end,
    };
    let selected_session = H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::BytesSelection(&stream, selection),
    };
    let branch = h4cr_branch(u64::try_from(h10).unwrap(), &stream);
    let catalog = authenticated_catalog();
    let bundle = build_d81_bundle(
        BundleBuildConfig::default(),
        std::slice::from_ref(&branch),
        &catalog,
        std::slice::from_ref(&selected_session),
    )
    .unwrap();
    let verified = verify_prepared_bundle(&bundle).unwrap();
    assert_eq!(verified.branches[0].first_tick, h10 as u64);
    assert_eq!(verified.branches[0].last_tick, h14 as u64);
    assert_eq!(verified.branches[0].state_observations, 3);
    assert_eq!(verified.branches[0].gap_hours, 2);
    assert_eq!(verified.branches[0].blocks, 2);

    let retained_paths = bundle
        .entries()
        .iter()
        .map(|entry| (entry.path.clone(), entry.file_path.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut descriptors = bundle
        .entries()
        .iter()
        .map(BundleEntryDescriptor::from)
        .collect::<Vec<_>>();
    descriptors.sort_by(|left, right| left.path.as_bytes().cmp(right.path.as_bytes()));
    let temp = TestDir::create();
    let source_path = temp.path("selected-internal-gap.h4cr");
    std::fs::write(&source_path, &stream).unwrap();
    let retained_source = portable_test_fs_retry::open(source_path).unwrap();
    let retained_session = H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::RetainedFileSelection(&retained_source, selection),
    };
    let reopened = verify_entry_source_against_h4cr(
        &descriptors,
        |path| {
            let file = retained_paths
                .get(path)
                .ok_or_else(|| format!("missing retained artifact {path}"))?;
            portable_test_fs_retry::read(file).map_err(|error| error.to_string())
        },
        BundleBuildConfig::default(),
        std::slice::from_ref(&branch),
        &catalog,
        std::slice::from_ref(&retained_session),
        &[],
    )
    .unwrap();
    assert_eq!(reopened, verified);
}
