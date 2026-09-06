use super::*;
use crate::codec::{
    encode_branches, encode_bundle_metadata_entry_descriptors, encode_schemas, SchemaDefinition,
};
use crate::model::{BRANCHES_PATH, BUNDLE_PATH, PROVINCES_PATH, SCHEMAS_PATH, SYMBOLS_PATH};
use compact_contract::stream_validator::StreamIdentityMode;
use compact_contract::{
    COMPACT_DOMAIN_FRAME, COMPACT_FRAME_END_PAYLOAD_LEN, COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_GAP, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_END, COMPACT_RECORD_VERSION, COMPACT_STATUS_DELTA_DATA,
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

fn d81_v4_payload(status: u32) -> Vec<u8> {
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
    compact_contract::country_name_inputs::encode_domain_version(18, &mut payload);
    compact_contract::country_name_inputs::encode_country_link_indices(
        &compact_contract::country_name_inputs::CountryLinkIndices::Unavailable(compact_contract::country_name_inputs::NameInputRefusal::Unreadable), &mut payload);
    payload.extend_from_slice(&source_id.to_le_bytes());
    payload.extend_from_slice(&(tag.len() as u16).to_le_bytes());
    payload.extend_from_slice(tag.as_bytes());
    payload.push(1);
    compact_contract::country_name_inputs::encode_effective(&Err(compact_contract::country_name_inputs::NameInputRefusal::Unreadable), &mut payload);
    payload.extend_from_slice(&[0, 1]);
    payload.extend_from_slice(compact_contract::country_identity_membership::CIM1_MAGIC);
    payload.extend_from_slice(
        &compact_contract::country_identity_membership::CIM1_VERSION.to_le_bytes(),
    );
    payload.extend_from_slice(
        &(compact_contract::country_identity_membership::CIM1_HEADER_LEN as u16).to_le_bytes(),
    );
    payload.extend_from_slice(
        &[0; compact_contract::country_identity_membership::CIM1_HEADER_LEN - 8],
    );
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
        &d81_v4_payload(data_status),
    );
    *next_sequence += 1;
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
            d81_v4_payload(0),
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

/// A D18 DATA payload with an arbitrary outer word and arbitrary CIM1 family
/// words. `claims_excluded` / `cores_excluded` add the state row and exclusion
/// entry a BADGED family must carry, so a badged fixture is one the recorder
/// host would also admit rather than a contradiction it refuses.
fn d18_country_payload_families(
    source_id: u32,
    tag: &str,
    outer: u32,
    claims: u32,
    cores: u32,
    major: u32,
    claims_excluded: bool,
    cores_excluded: bool,
) -> Vec<u8> {
    use compact_contract::country_identity_membership as cim;
    assert_eq!(tag.len(), 3);
    let mut payload = Vec::new();
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&1u32.to_le_bytes());
    payload.extend_from_slice(&outer.to_le_bytes());
    compact_contract::country_name_inputs::encode_domain_version(18, &mut payload);
    compact_contract::country_name_inputs::encode_country_link_indices(
        &compact_contract::country_name_inputs::CountryLinkIndices::Unavailable(compact_contract::country_name_inputs::NameInputRefusal::Unreadable), &mut payload);
    payload.extend_from_slice(&source_id.to_le_bytes());
    payload.extend_from_slice(&(tag.len() as u16).to_le_bytes());
    payload.extend_from_slice(tag.as_bytes());
    payload.push(1);
    compact_contract::country_name_inputs::encode_effective(&Err(compact_contract::country_name_inputs::NameInputRefusal::Unreadable), &mut payload);
    payload.extend_from_slice(&[0, 1]);
    payload.extend_from_slice(cim::CIM1_MAGIC);
    payload.extend_from_slice(&cim::CIM1_VERSION.to_le_bytes());
    payload.extend_from_slice(&(cim::CIM1_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&claims.to_le_bytes());
    payload.extend_from_slice(&cores.to_le_bytes());
    payload.extend_from_slice(&major.to_le_bytes());
    let any_excluded = claims_excluded || cores_excluded;
    payload.extend_from_slice(&u32::from(any_excluded).to_le_bytes()); // state rows
    payload.extend_from_slice(&0u32.to_le_bytes()); // claim memberships
    payload.extend_from_slice(&0u32.to_le_bytes()); // core memberships
    // A CLEAN major family must exactly cover the country roster, which is this
    // one country. Zero rows under a clean major word is a shape both recorder
    // consumers refuse, so a fixture claiming producer-admissibility cannot use
    // it. (Codex review 01a03c12, finding 2.)
    let major_rows = u32::from(!compact_contract::compact_status_is_failure(major));
    payload.extend_from_slice(&major_rows.to_le_bytes());
    payload.extend_from_slice(&u32::from(claims_excluded).to_le_bytes());
    payload.extend_from_slice(&u32::from(cores_excluded).to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes()); // reserved
    if any_excluded {
        // One state row carrying no members, which is what an excluded state
        // publishes.
        payload.extend_from_slice(&793u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    if major_rows != 0 {
        payload.extend_from_slice(&source_id.to_le_bytes());
        payload.push(0);
        payload.extend_from_slice(&[0, 0, 0]);
    }
    // The exclusion lists, claims first, exactly as the producer orders them.
    for excluded in [claims_excluded, cores_excluded] {
        if excluded {
            payload.extend_from_slice(&793u32.to_le_bytes());
            payload.extend_from_slice(&1815u32.to_le_bytes());
        }
    }
    payload
}

fn d18_binding_stream(perf_status: u32, data: Vec<u8>) -> Vec<u8> {
    let tick = valid_hour(0);
    let mut stream = Vec::new();
    let mut sequence = 1u64;
    for (domain, record_type, payload) in [
        (
            wire_format::d81::DOMAIN as u16,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            perf_payload(0),
        ),
        (
            wire_format::d81::DOMAIN as u16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            d81_v4_payload(0),
        ),
        (18, COMPACT_RECORD_TYPE_DOMAIN_PERF, perf_payload(perf_status)),
        (18, COMPACT_RECORD_TYPE_DOMAIN_DATA, data),
    ] {
        push_h4cr_record(&mut stream, sequence, tick, domain, record_type, 0, &payload);
        sequence += 1;
    }
    push_h4cr_record(
        &mut stream,
        sequence,
        tick,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(2, 2, 0),
    );
    stream
}

fn d18_binding_result(perf_status: u32, data: Vec<u8>) -> Result<(), String> {
    let stream = d18_binding_stream(perf_status, data);
    inspect_d81_session(H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::Legacy,
        source: H4crSource::Bytes(&stream),
    })
    .map(|_| ())
    .map_err(|error| error.to_string())
}

/// The D18 authority equation, term by term:
///
/// ```text
/// PERF == outer DATA roster status | CIM1 claims | CIM1 cores | CIM1 major
/// ```
///
/// Binding PERF against the OUTER word alone REFUSED a sealed hour that excluded
/// one state of 1,081 -- the same data loss the lane closed, moved one stage
/// downstream -- and it also refused every producer-emittable hour whose failure
/// lives in a CIM1 family, while admitting payloads whose CIM1 words contradict
/// their own PERF.
///
/// EACH TERM IS PINNED SEPARATELY, which a single badged-cores fixture cannot
/// do: with one contributing family the OR survives dropping either of the other
/// two. The cases below give the three families DIFFERENT words so each term is
/// load-bearing on its own, and the negative case is what kills globalizing the
/// comparison -- globalizing both sides preserves equality, so only a
/// contradiction that must be REFUSED can catch it.
///
/// (Codex reviews 01a03ba4, 01a03bb5 and 01a03bcf, finding 1.)
#[test]
fn d18_binding_reconstructs_every_term_of_the_data_authority() {
    let badge = compact_contract::DOMAIN18_STATUS_MEMBERSHIP_STATES_UNMEASURED;
    let failed = compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE
        | compact_contract::COMPACT_STATUS_PARTIAL_DATA;
    assert!(
        !compact_contract::compact_status_is_failure(badge),
        "the premise: an excluded state is not a failed hour"
    );

    // (name, outer, claims, cores, major, cores_excluded, which term it pins)
    for (name, outer, claims, cores, major, claims_excluded, cores_excluded) in [
        // The rc_vanilla6 shape: cores badged, everything else clean. Pins the
        // CORES term -- drop it and the reconstruction is 0 against PERF badge.
        ("cores badged", 0u32, 0u32, badge, 0u32, false, true),
        // Claims badged with an independent major failure. Pins the CLAIMS term
        // (drop it: `failed` against `badge|failed`) and the MAJOR term (drop
        // it: `badge` against `badge|failed`).
        ("claims badged + major failed", 0, badge, 0, failed, true, false),
        // A roster failure. The producer still writes the CIM1 block, and
        // `unavailable()` fills all three families, so PERF is 0x300 while the
        // outer word is 0x100. Returning the outer word early -- which both
        // binders used to do on this branch -- refuses a hour the producer
        // sealed as failed, and skips the extension's validation entirely.
        (
            "roster failure carries its families",
            compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
            failed,
            failed,
            failed,
            false,
            false,
        ),
    ] {
        let data = d18_country_payload_families(
            7,
            "GER",
            outer,
            claims,
            cores,
            major,
            claims_excluded,
            cores_excluded,
        );
        let perf = outer | claims | cores | major;
        assert_eq!(
            d18_binding_result(perf, data),
            Ok(()),
            "{name}: PERF {perf:#010x} is exactly the complete data authority and must bind"
        );
    }

    // THE NEGATIVE that kills globalization. A PERF word carrying the badge over
    // a CIM1 block that names no exclusion is a contradiction -- the recorder
    // host refuses it for badge/list disagreement -- and the binder must refuse
    // it too. Globalizing BOTH sides preserves equality in every positive case
    // above, so this is the only case that catches it.
    let contradiction = d18_country_payload_families(7, "GER", 0, 0, 0, 0, false, false);
    assert!(
        d18_binding_result(badge, contradiction).is_err(),
        "a PERF word that claims an exclusion its own CIM1 block does not carry \
         must be refused; admitting it is what globalizing the comparison does"
    );
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
        &d81_v4_payload(0),
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
        &d81_v4_payload(0),
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
        &frame_end_payload(2, 2, 0),
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
fn canonical_d81_v4_data_uses_zero_h4cr_flags_and_perf_status_at_offset_32() {
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
fn an_enveloped_d81_session_inspects_and_builds_like_the_raw_one() {
    // kills: scan_h4cr treating a `PSD1` envelope as d81's RRL1 grammar --
    // `d81::split_payload` would reject every wrapped DATA record and
    // publication would fail wholesale once the collector wraps d81.
    // Unwrapping through the same consumer contract every other walker uses
    // must leave the inspection counts, the raw-stream digest and every
    // STATE-bearing bundle entry identical to the same hours riding
    // unwrapped, while the two IDENTITY artefacts (branches.bin's embedded
    // session_fingerprint and the metadata hashing it) differ exactly by
    // naming their own source stream -- see the assertions at the tail.
    let mut raw = Vec::new();
    let mut wrapped = Vec::new();
    let mut sequence = 1u64;
    let mut producer = compact_contract::payload_span_delta::PayloadSpanDeltaProducer::new();
    for hour in 0..3i32 {
        let tick = 10 + hour;
        let payload =
            compact_contract::rail_replay::encode_payload_with_hub_simulation(
                &[],
                &[],
                u64::try_from(hour).unwrap() * 1000,
                0,
                0,
                0,
            )
            .unwrap();
        let envelope = producer
            .wrap(
                compact_contract::payload_span_delta::PayloadSpanDeltaIdentity {
                    domain_id: wire_format::d81::DOMAIN as u16,
                    source_sequence: sequence + 1,
                    game_hour_raw: tick,
                },
                &payload,
                true,
            )
            .unwrap();
        for (out, data_payload, flags) in [
            (&mut raw, payload.as_slice(), 0),
            (
                &mut wrapped,
                envelope.as_slice(),
                COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
            ),
        ] {
            push_h4cr_record(
                out,
                sequence,
                tick,
                wire_format::d81::DOMAIN as u16,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                0,
                &perf_payload(0),
            );
            push_h4cr_record(
                out,
                sequence + 1,
                tick,
                wire_format::d81::DOMAIN as u16,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                flags,
                data_payload,
            );
            push_h4cr_record(
                out,
                sequence + 2,
                tick,
                COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                0,
                &frame_end_payload(1, 1, 0),
            );
        }
        sequence += 3;
        producer.commit();
    }

    let raw_inspection = inspect_d81_session(legacy_bytes_session(&raw)).unwrap();
    let wrapped_inspection = inspect_d81_session(legacy_bytes_session(&wrapped)).unwrap();
    assert_eq!(wrapped_inspection.data_records, 3);
    assert_eq!(
        wrapped_inspection.data_records,
        raw_inspection.data_records
    );
    assert_eq!(
        wrapped_inspection.publication_authoritative_records,
        raw_inspection.publication_authoritative_records
    );
    assert_eq!(wrapped_inspection.publication_authoritative_records, 3);
    assert_eq!(wrapped_inspection.unreconstructed_records, 0);
    assert_eq!(wrapped_inspection.decoded_sha256, source_digest(&wrapped));

    // The product built from enveloped hours is byte-identical to the one
    // built from raw bytes: the envelopes never reach the block encoder.
    let catalog = authenticated_catalog();
    let config = BundleBuildConfig::default();
    let raw_bundle = build_d81_bundle(
        config,
        std::slice::from_ref(&h4cr_branch(10, &raw)),
        &catalog,
        std::slice::from_ref(&legacy_bytes_session(&raw)),
    )
    .unwrap();
    let wrapped_bundle = build_d81_bundle(
        config,
        std::slice::from_ref(&h4cr_branch(10, &wrapped)),
        &catalog,
        std::slice::from_ref(&legacy_bytes_session(&wrapped)),
    )
    .unwrap();
    // THE TRANSPORT MUST NOT REACH THE DERIVED STATE. Every state-bearing
    // entry is built from reconstructed bases, so raw and enveloped hours
    // must produce byte-identical blocks, province catalog, schemas, symbol
    // manifest and gaps.
    let state_entries = |bundle: &PreparedBundle| {
        let mut rows: Vec<(String, String)> = bundle
            .entries()
            .iter()
            .filter(|entry| {
                !matches!(
                    entry.role,
                    ArtifactRole::BranchTable | ArtifactRole::BundleMetadata
                )
            })
            .map(|entry| (entry.path.clone(), hex_encode_blake3(&entry.blake3)))
            .collect();
        rows.sort();
        rows
    };
    assert_eq!(state_entries(&raw_bundle), state_entries(&wrapped_bundle));

    // ...while the two IDENTITY artefacts differ exactly by the source
    // binding they exist to carry and nothing else. branches.bin embeds each
    // branch's `session_fingerprint` (`codec::encode_branches`) — here each
    // stream's own SHA-256, because the product authenticates its exact
    // source capture — and bundle.bin hashes the descriptor list that
    // includes it. Decoding both tables proves every other field agrees and
    // both fingerprints name their own stream; decoding both metadata blobs
    // proves their descriptor lists agree except that one entry's hash.
    let entry_by_role = |bundle: &PreparedBundle, role: ArtifactRole| bundle
        .entries()
        .iter()
        .find(|entry| entry.role == role)
        .unwrap_or_else(|| panic!("bundle has no {role:?} entry"))
        .clone();
    let raw_branches = crate::codec::decode_branches(&portable_test_fs_retry::read(
        &entry_by_role(&raw_bundle, ArtifactRole::BranchTable).file_path,
    )
    .unwrap())
    .unwrap();
    let wrapped_branches = crate::codec::decode_branches(
        &portable_test_fs_retry::read(
            &entry_by_role(&wrapped_bundle, ArtifactRole::BranchTable).file_path,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(raw_branches.len(), 1);
    assert_eq!(wrapped_branches.len(), 1);
    let (raw_branch, wrapped_branch) = (&raw_branches[0], &wrapped_branches[0]);
    assert_eq!(raw_branch.branch, wrapped_branch.branch);
    assert_eq!(raw_branch.first_tick, wrapped_branch.first_tick);
    assert_eq!(raw_branch.parent, wrapped_branch.parent);
    assert_ne!(
        raw_branch.session_fingerprint,
        wrapped_branch.session_fingerprint
    );
    assert_eq!(raw_branch.session_fingerprint, source_digest(&raw));
    assert_eq!(
        wrapped_branch.session_fingerprint,
        source_digest(&wrapped)
    );

    // bundle.bin hashes the descriptor list that includes branches.bin, so it
    // differs transitively and only through that one descriptor. Decoding both
    // metadata blobs proves it: same authenticated digests, descriptor lists
    // equal once the branch-table entry is set aside, and the branch-table
    // entries themselves equal in length (the tables differ only in their
    // fingerprint bytes) while naming different hashes.
    fn read_metadata(bundle: &PreparedBundle) -> crate::codec::BundleMetadata {
        crate::codec::decode_bundle_metadata(
            &portable_test_fs_retry::read(
                &bundle
                    .entries()
                    .iter()
                    .find(|entry| entry.role == ArtifactRole::BundleMetadata)
                    .expect("bundle has no metadata entry")
                    .file_path,
            )
            .unwrap(),
        )
        .unwrap()
    }
    fn without_branches(metadata: &crate::codec::BundleMetadata) -> Vec<(String, u64, [u8; 32])> {
        let mut rows: Vec<(String, u64, [u8; 32])> = metadata
            .artifacts
            .iter()
            .filter(|descriptor| descriptor.path != BRANCHES_PATH)
            .map(|descriptor| (descriptor.path.clone(), descriptor.len, descriptor.blake3))
            .collect();
        rows.sort();
        rows
    }
    fn branch_descriptor(metadata: &crate::codec::BundleMetadata) -> &crate::codec::ArtifactDescriptor {
        metadata
            .artifacts
            .iter()
            .find(|descriptor| descriptor.path == BRANCHES_PATH)
            .expect("bundle metadata names no branches.bin")
    }
    let raw_metadata = read_metadata(&raw_bundle);
    let wrapped_metadata = read_metadata(&wrapped_bundle);
    assert_eq!(
        raw_metadata.content_manifest_sha256,
        wrapped_metadata.content_manifest_sha256
    );
    assert_eq!(
        raw_metadata.definition_csv_sha256,
        wrapped_metadata.definition_csv_sha256
    );
    assert_eq!(
        without_branches(&raw_metadata),
        without_branches(&wrapped_metadata)
    );
    let raw_descriptor = branch_descriptor(&raw_metadata);
    let wrapped_descriptor = branch_descriptor(&wrapped_metadata);
    assert_eq!(raw_descriptor.len, wrapped_descriptor.len);
    assert_ne!(raw_descriptor.blake3, wrapped_descriptor.blake3);
}

#[test]
fn a_repeated_enveloped_d81_hour_publishes_the_same_state_as_its_full_record() {
    // kills: the PSD1 x DOMAIN_DATA_REPEAT composition seam. A repeat whose
    // baseline hour rode ENVELOPED resolves to those ON-WIRE envelope bytes
    // (d81 declares no volatile prefix, so the repeat digest covers the whole
    // envelope), and scan_h4cr must unwrap them through its span-delta
    // consumer before any grammar read. Before this seam was closed the d81
    // arm took from a priming slot that repeats never fill (panic) or fed the
    // envelope straight to `d81::split_payload`. The repeated hour must count,
    // publish, and produce state byte-identical to a plain full record.
    let mut wrapped = Vec::new();
    let mut sequence = 1u64;
    let mut producer = compact_contract::payload_span_delta::PayloadSpanDeltaProducer::new();

    // Hour 10: PERF + enveloped keyframe DATA + FRAME_END.
    let tick_a = 10i32;
    let payload_a = compact_contract::rail_replay::encode_payload_with_hub_simulation(
        &[],
        &[],
        0,
        0,
        0,
        0,
    )
    .unwrap();
    let identity = |sequence: u64, tick: i32| {
        compact_contract::payload_span_delta::PayloadSpanDeltaIdentity {
            domain_id: wire_format::d81::DOMAIN as u16,
            source_sequence: sequence,
            game_hour_raw: tick,
        }
    };
    let envelope_a = producer.wrap(identity(sequence + 1, tick_a), &payload_a, true).unwrap();
    push_h4cr_record(
        &mut wrapped,
        sequence,
        tick_a,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf_payload(0),
    );
    push_h4cr_record(
        &mut wrapped,
        sequence + 1,
        tick_a,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
        &envelope_a,
    );
    push_h4cr_record(
        &mut wrapped,
        sequence + 2,
        tick_a,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(1, 1, 0),
    );
    sequence += 3;

    // Hour 11: PERF + a DOMAIN_DATA_REPEAT vouching hour 10's on-wire bytes.
    // d81 declares no volatile prefix, so the repeat payload is exactly the
    // 32-byte digest of the stored baseline (the envelope). The gate keeps the
    // flag word of the record it replaces, so this record carries the PSD1 bit.
    let tick_b = 11i32;
    let repeat_payload = {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(&envelope_a);
        digest.to_vec()
    };
    producer.commit();
    push_h4cr_record(
        &mut wrapped,
        sequence,
        tick_b,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf_payload(0),
    );
    push_h4cr_record(
        &mut wrapped,
        sequence + 1,
        tick_b,
        wire_format::d81::DOMAIN as u16,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_DATA_REPEAT,
        COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
        &repeat_payload,
    );
    push_h4cr_record(
        &mut wrapped,
        sequence + 2,
        tick_b,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(1, 1, 0),
    );

    // The composed session inspects like two full hours...
    let inspection = inspect_d81_session(legacy_bytes_session(&wrapped)).unwrap();
    assert_eq!(inspection.data_records, 2);
    assert_eq!(inspection.publication_authoritative_records, 2);
    assert_eq!(inspection.unreconstructed_records, 0);

    // ...and publishes state identical to a session where BOTH hours are full
    // records carrying the same payload (the only thing the repeat asserts).
    let mut raw = Vec::new();
    let mut raw_sequence = 1u64;
    for tick in [tick_a, tick_b] {
        push_h4cr_record(
            &mut raw,
            raw_sequence,
            tick,
            wire_format::d81::DOMAIN as u16,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        push_h4cr_record(
            &mut raw,
            raw_sequence + 1,
            tick,
            wire_format::d81::DOMAIN as u16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload_a,
        );
        push_h4cr_record(
            &mut raw,
            raw_sequence + 2,
            tick,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(1, 1, 0),
        );
        raw_sequence += 3;
    }
    let catalog = authenticated_catalog();
    let config = BundleBuildConfig::default();
    let describe_state_entries = |bundle: &PreparedBundle| {
        let mut rows: Vec<(String, String)> = bundle
            .entries()
            .iter()
            .filter(|entry| entry.role == ArtifactRole::StateBlock)
            .map(|entry| (entry.path.clone(), hex_encode_blake3(&entry.blake3)))
            .collect();
        rows.sort();
        rows
    };
    let raw_bundle = build_d81_bundle(
        config,
        std::slice::from_ref(&h4cr_branch(tick_a as u64, &raw)),
        &catalog,
        std::slice::from_ref(&legacy_bytes_session(&raw)),
    )
    .unwrap();
    let wrapped_bundle = build_d81_bundle(
        config,
        std::slice::from_ref(&h4cr_branch(tick_a as u64, &wrapped)),
        &catalog,
        std::slice::from_ref(&legacy_bytes_session(&wrapped)),
    )
    .unwrap();
    assert_eq!(
        describe_state_entries(&raw_bundle),
        describe_state_entries(&wrapped_bundle)
    );
}

/// Hex helper local to these tests; `blake3::Hasher` output rendered the way
/// [`BundleEntry`] displays it.
fn hex_encode_blake3(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn an_unreconstructable_d81_hour_degrades_to_a_named_gap_and_still_publishes() {
    // kills: propagating a PSD1 reconstruction refusal out of scan_h4cr with
    // `?`. A damaged or dropped record is the ordinary case the cadence
    // keyframe exists to recover from; refusing publication for it would be
    // the exact defect class PS's Codex rounds 7-9 closed for every other
    // walker. The lost hour must close as a coverage-shortfall gap IN the product
    // and the exclusion must be counted on the session inspection.
    let mut wrapped = Vec::new();
    let mut sequence = 1u64;
    let mut producer = compact_contract::payload_span_delta::PayloadSpanDeltaProducer::new();
    let mut damaged_at = None;
    for hour in 0..2i32 {
        let tick = 10 + hour;
        let payload =
            compact_contract::rail_replay::encode_payload_with_hub_simulation(
                &[],
                &[],
                u64::try_from(hour).unwrap() * 1000,
                0,
                0,
                0,
            )
            .unwrap();
        let mut envelope = producer
            .wrap(
                compact_contract::payload_span_delta::PayloadSpanDeltaIdentity {
                    domain_id: wire_format::d81::DOMAIN as u16,
                    source_sequence: sequence + 1,
                    game_hour_raw: tick,
                },
                &payload,
                true,
            )
            .unwrap();
        if hour == 1 {
            // Damage a byte INSIDE the delta body so the H4CR framing and the
            // PSD1 header stay valid but the reconstruction hash refuses.
            let last = envelope.len() - 1;
            envelope[last] ^= 0xff;
            damaged_at = Some(last);
        }
        push_h4cr_record(
            &mut wrapped,
            sequence,
            tick,
            wire_format::d81::DOMAIN as u16,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        push_h4cr_record(
            &mut wrapped,
            sequence + 1,
            tick,
            wire_format::d81::DOMAIN as u16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
            &envelope,
        );
        push_h4cr_record(
            &mut wrapped,
            sequence + 2,
            tick,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(1, 1, 0),
        );
        sequence += 3;
        producer.commit();
    }
    assert!(damaged_at.is_some());

    let inspection = inspect_d81_session(legacy_bytes_session(&wrapped)).unwrap();
    assert_eq!(inspection.unreconstructed_records, 1);
    assert_eq!(inspection.data_records, 1);
    assert_eq!(inspection.publication_authoritative_records, 1);

    let bundle = build_d81_bundle(
        BundleBuildConfig::default(),
        std::slice::from_ref(&h4cr_branch(10, &wrapped)),
        &authenticated_catalog(),
        std::slice::from_ref(&legacy_bytes_session(&wrapped)),
    )
    .unwrap();
    let gap_entries = bundle
        .entries()
        .iter()
        .filter(|entry| entry.role == ArtifactRole::GapRecord)
        .count();
    assert!(
        gap_entries >= 1,
        "the lost hour must appear as a gap record in the bundle"
    );
}

#[test]
fn a_branch_whose_every_d81_hour_was_lost_still_publishes_as_named_gaps() {
    // kills: the zero-authority refusal firing AFTER degradation converted a
    // short selection's every snapshot into named gaps -- publication would
    // abort exactly where the lost-record law says it must not. A branch with
    // no authoritative snapshots but NAMED exclusions publishes gap-only, and
    // the refusal stays for a branch with neither snapshots nor exclusions.
    let mut wrapped = Vec::new();
    let mut sequence = 1u64;
    let mut producer = compact_contract::payload_span_delta::PayloadSpanDeltaProducer::new();
    for hour in 0..2i32 {
        let tick = 10 + hour;
        let payload =
            compact_contract::rail_replay::encode_payload_with_hub_simulation(
                &[],
                &[],
                u64::try_from(hour).unwrap() * 1000,
                0,
                0,
                0,
            )
            .unwrap();
        let mut envelope = producer
            .wrap(
                compact_contract::payload_span_delta::PayloadSpanDeltaIdentity {
                    domain_id: wire_format::d81::DOMAIN as u16,
                    source_sequence: sequence + 1,
                    game_hour_raw: tick,
                },
                &payload,
                true,
            )
            .unwrap();
        let last = envelope.len() - 1;
        envelope[last] ^= 0xff;
        push_h4cr_record(
            &mut wrapped,
            sequence,
            tick,
            wire_format::d81::DOMAIN as u16,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        push_h4cr_record(
            &mut wrapped,
            sequence + 1,
            tick,
            wire_format::d81::DOMAIN as u16,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
            &envelope,
        );
        push_h4cr_record(
            &mut wrapped,
            sequence + 2,
            tick,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(1, 1, 0),
        );
        sequence += 3;
        producer.commit();
    }

    let inspection = inspect_d81_session(legacy_bytes_session(&wrapped)).unwrap();
    assert_eq!(inspection.unreconstructed_records, 2);
    assert_eq!(inspection.data_records, 0);
    assert_eq!(inspection.publication_authoritative_records, 0);

    let bundle = build_d81_bundle(
        BundleBuildConfig::default(),
        std::slice::from_ref(&h4cr_branch(10, &wrapped)),
        &authenticated_catalog(),
        std::slice::from_ref(&legacy_bytes_session(&wrapped)),
    )
    .unwrap();
    assert!(bundle
        .entries()
        .iter()
        .any(|entry| entry.role == ArtifactRole::GapRecord));
}

#[test]
fn a_damaged_volatile_prefix_is_a_named_exclusion_and_not_an_abort() {
    // kills: the PSD1 digest excludes the declared volatile prefix, and for
    // d81 those eight bytes are the payload's own RRL1 magic/version/length.
    // Damage confined there reconstructs faithfully and then fails
    // `d81::split_payload` — which used to propagate with `?` and abort the
    // whole publication. It must take the same excluded-and-named path as any
    // other lost record.
    let mut wrapped = Vec::new();
    let mut sequence = 1u64;
    let mut producer = compact_contract::payload_span_delta::PayloadSpanDeltaProducer::new();
    let tick = 10;
    let payload = compact_contract::rail_replay::encode_payload_with_hub_simulation(
        &[],
        &[],
        0,
        0,
        0,
        0,
    )
    .unwrap();
    let mut envelope = producer
        .wrap(
            compact_contract::payload_span_delta::PayloadSpanDeltaIdentity {
                domain_id: wire_format::d81::DOMAIN as u16,
                source_sequence: sequence + 1,
                game_hour_raw: tick,
            },
            &payload,
                true,
        )
        .unwrap();
    envelope[0] ^= 0xff; // inside the volatile prefix, outside every digest
    push_h4cr_record(
        &mut wrapped,
        sequence,
        tick,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        0,
        &perf_payload(0),
    );
    push_h4cr_record(
        &mut wrapped,
        sequence + 1,
        tick,
        wire_format::d81::DOMAIN as u16,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
        &envelope,
    );
    push_h4cr_record(
        &mut wrapped,
        sequence + 2,
        tick,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(1, 1, 0),
    );

    let inspection = inspect_d81_session(legacy_bytes_session(&wrapped)).unwrap();
    assert_eq!(inspection.unreconstructed_records, 1);
    assert_eq!(inspection.data_records, 0);

    let bundle = build_d81_bundle(
        BundleBuildConfig::default(),
        std::slice::from_ref(&h4cr_branch(10, &wrapped)),
        &authenticated_catalog(),
        std::slice::from_ref(&legacy_bytes_session(&wrapped)),
    )
    .unwrap();
    assert!(bundle
        .entries()
        .iter()
        .any(|entry| entry.role == ArtifactRole::GapRecord));
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
    let data = d81_v4_payload(0);
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
        &d81_v4_payload(COMPACT_STATUS_DELTA_DATA),
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
        &d81_v4_payload(0),
    );
    assert!(inspect_d81_session(legacy_bytes_session(&missing_end))
        .unwrap_err()
        .to_string()
        .contains("unclosed frame 10"));

    let mut unsealed_first = missing_end.clone();
    let mut sequence = 3;
    append_d81_hour(&mut unsealed_first, &mut sequence, 11, 0, 0);
    assert!(inspect_d81_session(legacy_bytes_session(&unsealed_first))
        .unwrap_err().to_string().contains("without FRAME_END"));

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
        &d81_v4_payload(0),
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
fn frame_end_outcome_is_independent_and_refuses_fatal_or_unknown_status() {
    let failure = compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE;
    for perf in [0, failure, COMPACT_STATUS_DELTA_DATA] {
        for status in [0, failure, 1 << 31] {
            let mut stream = Vec::new();
            let mut sequence = 1;
            let (_, end) = append_d81_hour(&mut stream, &mut sequence, valid_hour(0), perf, perf);
            let offset = end.start as usize + COMPACT_RECORD_HEADER_LEN + 24;
            stream[offset..offset + 4].copy_from_slice(&status.to_le_bytes());
            let result = inspect_d81_session(legacy_bytes_session(&stream));
            if status == 0 {
                let inspection = result.unwrap();
                assert_eq!(inspection.publication_authoritative_records, u64::from(perf != failure));
            } else {
                assert!(result.unwrap_err().to_string().contains("FRAME_END seal mismatch"));
            }
        }
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
        &d81_v4_payload(0),
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

/// Env-gated capture probe: runs only with BATTLE_PLAN_D81I_STREAM, BATTLE_PLAN_D81I_CONTENT_MANIFEST and BATTLE_PLAN_D81I_DEFINITION_CSV set.
#[test]
fn a_real_late_attach_capture_inspects_and_builds_its_d81_product() {
    fn input(var: &str) -> Option<PathBuf> {
        let path = PathBuf::from(std::env::var_os(var)?);
        path.is_file().then_some(path)
    }
    let (stream_path, manifest_path, definition_path) = match (
        input("BATTLE_PLAN_D81I_STREAM"),
        input("BATTLE_PLAN_D81I_CONTENT_MANIFEST"),
        input("BATTLE_PLAN_D81I_DEFINITION_CSV"),
    ) {
        (Some(stream), Some(manifest), Some(definition)) => (stream, manifest, definition),
        _ => return,
    };
    let catalog = AuthenticatedProvinceCatalog::from_content_manifest(
        &std::fs::read(&manifest_path).unwrap(),
        &std::fs::read(&definition_path).unwrap(),
    )
    .unwrap();
    let session = H4crSession {
        branch: 0,
        identity_mode: StreamIdentityMode::IdentityV1,
        source: H4crSource::File(&stream_path),
    };
    let inspection = match inspect_d81_session_with_catalog(session, &catalog) {
        Ok(inspection) => inspection,
        Err(error) => {
            println!(
                "D81I_CAPTURE06 {{\"stream_bytes\":{},\"inspected\":false,\"refusal\":\"{}\"}}",
                std::fs::metadata(&stream_path).unwrap().len(),
                error.to_string().replace('"', "'")
            );
            panic!("capture inspection refused: {error}");
        }
    };
    let branch = BranchSpec {
        branch: 0,
        parent: None,
        first_tick: inspection.first_tick,
        session_fingerprint: inspection.decoded_sha256,
    };
    let product = match build_d81_bundle(
        BundleBuildConfig::default(),
        std::slice::from_ref(&branch),
        &catalog,
        std::slice::from_ref(&session),
    ) {
        Ok(bundle) => format!(
            "{{\"built\":true,\"entries\":{},\"bytes\":{}}}",
            bundle.entries().len(),
            bundle.entries().iter().map(|entry| entry.len).sum::<u64>(),
        ),
        Err(error) => panic!("capture product build refused: {error}"),
    };
    println!(
        "D81I_CAPTURE06 {{\"stream_bytes\":{},\"inspected\":true,\"decoded_sha256\":\"{}\",\"first_tick\":{},\"last_tick\":{},\"d81_data_records\":{},\"d81_publication_authoritative_records\":{},\"d81_failure_records\":{},\"d81_unreconstructed_records\":{},\"product\":{}}}",
        std::fs::metadata(&stream_path).unwrap().len(),
        hex_encode_sha256(&inspection.decoded_sha256),
        inspection.first_tick,
        inspection.last_tick,
        inspection.data_records,
        inspection.publication_authoritative_records,
        inspection.failure_records,
        inspection.unreconstructed_records,
        product,
    );
}

fn hex_encode_sha256(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn d18_binding_parses_required_name_inputs_and_refuses_unknown_selection() {
    let payload = d18_country_payload(0);
    assert!(d18_binding_result(0, payload.clone()).is_ok());
    let mut unknown = payload.clone();
    let mut selection = 20 + compact_contract::country_name_inputs::DOMAIN_VERSION_HEADER_LEN;
        compact_contract::country_name_inputs::decode_country_link_indices(&payload, &mut selection).unwrap();
        selection += 4 + 2 + 3 + 1;
    unknown[selection] = 3;
    assert!(d18_binding_result(0, unknown).is_err());
    let mut missing = payload;
    missing.drain(selection..selection + 2);
    assert!(d18_binding_result(0, missing).is_err());
}

#[test]
fn d18_unversioned_payloads_are_refused_by_domain_version() {
    let mut payload=d18_country_payload(0);
    payload.drain(20..28);
    let error=d18_binding_result(0,payload).unwrap_err();
    assert!(error.contains("country_identity unsupported payload version"), "{error}");
}

#[test]
fn d18_wire_binding_refuses_base_id_or_tag_mismatch_with_containing_country() {
    for (source, selected_id, tag, admitted) in [
        (1u8, 47i32, "GER", true),
        (1, 2, "GER", false),
        (1, 47, "ENG", false),
        (1, 2, "ENG", false),
        (1, 47, "ger", false),
        (1, 47, "", false),
        (2, 2, "ENG", true),
        (2, 2, "", true),
    ] {
        let mut payload = d18_country_payload_for(0, 47, "GER");
        let mut selection = 20 + compact_contract::country_name_inputs::DOMAIN_VERSION_HEADER_LEN;
        compact_contract::country_name_inputs::decode_country_link_indices(&payload, &mut selection).unwrap();
        selection += 4 + 2 + 3 + 1;
        assert_eq!(&payload[selection..selection + 2], &[0, 1]);
        let mut wire = vec![source];
        wire.extend_from_slice(&selected_id.to_le_bytes());
        wire.push(2);
        wire.extend_from_slice(&(tag.len() as u16).to_le_bytes());
        wire.extend_from_slice(tag.as_bytes());
        payload.splice(selection..selection + 2, wire);
        let result = d18_binding_result(0, payload);
        if admitted {
            assert!(result.is_ok(), "{source}/{selected_id}/{tag:?}: {result:?}");
        } else {
            let error = result.expect_err("mismatched base selection must be refused");
            assert!(error.contains("country name base selection identity mismatch"), "{error}");
        }
    }
}

#[test]
fn d18_wire_product_validates_mandatory_link_block_before_country_rows() {
    use compact_contract::country_name_inputs::{CountryLinkIndices, NameInputRefusal, encode_country_link_indices};
    for links in [CountryLinkIndices::Measured(vec![]),
        CountryLinkIndices::Measured(vec![0, u32::MAX, 7, 7, 0x80000000]),
        CountryLinkIndices::Unavailable(NameInputRefusal::ClockChanged)] {
        let mut block=Vec::new(); encode_country_link_indices(&links, &mut block);
        let mut payload=d18_country_payload(0);
        payload.splice(28..36, block.clone());
        assert!(d18_binding_result(0, payload.clone()).is_ok());
        for (offset, byte) in [(0,2),(2,1),(3,1)] {
            let mut bad=payload.clone(); bad[28+offset]=byte;
            assert!(d18_binding_result(0,bad).unwrap_err().contains("country link"));
        }
        for end in 0..block.len() {
            let bad=payload[..28+end].to_vec();
            assert!(d18_binding_result(0,bad).is_err());
        }
        let mut old=payload; old[24..26].copy_from_slice(&2u16.to_le_bytes());
        assert!(d18_binding_result(0,old).unwrap_err().contains("unsupported payload version"));
    }
}
