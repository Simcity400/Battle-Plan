//! Actual native acquisition -> production publisher -> frame settlement.
use super::*;
use crate::attach_log::province_occupants::tests::stage;
use compact_contract::domain_gap::{DomainGapReason, DomainGapRecordV1};
use compact_contract::domain_outcome::{failure_diagnosis, DomainOutcomeLedger};
use division_output_storage::{FrameCopyReservation, StorageGapReservation};

const CHANGED_COUNTRIES: [i32; 4] = [COUNTRY, 31, COUNTRY, i32::MIN];
const FIRST_SEQUENCE: u64 = 19;
const PRIOR_ORDINAL: u64 = 7;

fn gap_key() -> (u16, u16) {
    (
        COMPACT_DOMAIN_DIVISIONS,
        domain_gap_part(COMPACT_DOMAIN_DIVISIONS).code(),
    )
}

fn roots(fixture: &Fixture) -> CompactDivisionRootCache {
    use compact_contract::country_roster::{
        Acquisition, Country, Kind, MemberRef, Observation, Slot,
    };
    let mut roots = CompactDivisionRootCache::default();
    roots.rows.push(fixture.entry());
    roots.complete = true;
    roots.observed_candidate_rows = 1;
    // This fixture supplies the already-acquired caller root cache. Its
    // explicit census matches the native row's owner/type/id; root discovery
    // itself is outside this reader/publication test's acquisition boundary.
    roots.country_roster = Some(Observation {
        kind: Kind::Division,
        country_count: Some(1),
        failure: None,
        countries: vec![Country {
            physical_slot: 0,
            owner_tag_db_id: Some(3),
            acquisition: Acquisition::Bracketed,
            observed_count: Some(1),
            failure: None,
            slots: vec![Slot {
                ordinal: 0,
                reference: Ok(MemberRef {
                    native_type: DIVISION_TYPE,
                    id: 7,
                }),
            }],
        }],
    });
    roots.railway_roster = Some(Observation {
        kind: Kind::Railway,
        country_count: Some(1),
        failure: None,
        countries: vec![Country {
            physical_slot: 0,
            owner_tag_db_id: Some(3),
            acquisition: Acquisition::Bracketed,
            observed_count: Some(0),
            failure: None,
            slots: Vec::new(),
        }],
    });
    roots
}

fn current_hour(fixture: &Fixture) -> i32 {
    i32::from_le_bytes(
        fixture.game_state.bytes[GS_CURRENT_DATE_OFFSET..GS_CURRENT_DATE_OFFSET + 4]
            .try_into()
            .unwrap(),
    )
}

fn next_hour(fixture: &mut Fixture) {
    let hour = current_hour(fixture) + 1;
    fixture.game_state.write_i32(GS_CURRENT_DATE_OFFSET, hour);
}

fn read_actual(
    fixture: &Fixture,
    trackers: &mut CompactPerfGateTrackers,
    budget: &country_roster_readset::Budget,
) -> CompactDivisionRead {
    page_cache_new_frame();
    let mut state = empty_collector_state();
    state.base = fixture.module;
    state.date_ptr = fixture.game_state.addr() as u64;
    read_compact_divisions(
        &state,
        &raw_test_control(),
        0,
        &mut trackers.division_movement,
        &roots(fixture),
        budget,
        current_hour(fixture),
        Some(&campaign()),
    )
}

fn membership(words: &[i32]) -> FirstCombatCountriesObservation {
    FirstCombatCountriesObservation::Present {
        combat: combat_identity(),
        countries: CountryWordsObservation::Measured(words.to_vec()),
    }
}

fn assert_pin(trackers: &CompactPerfGateTrackers, words: &[i32]) {
    assert_eq!(trackers.division_movement.len(), 1);
    let row = trackers.division_movement.get(&7).unwrap();
    assert_eq!(row.first_combat_countries, membership(words));
    assert_eq!(
        row.training_scalars.country_operand,
        TrainingObservation::Measured(COUNTRY)
    );
}

struct Prepared {
    read: CompactDivisionRead,
    out: Vec<u8>,
    budget: Arc<country_roster_readset::Budget>,
    gap: StorageGapReservation,
    pin_retained: bool,
    first_sequence: u64,
}

fn prepare(
    fixture: &Fixture,
    trackers: &mut CompactPerfGateTrackers,
    budget: Arc<country_roster_readset::Budget>,
) -> Prepared {
    let mut out = Vec::new();
    let gap =
        division_output_storage::reserve_storage_gap(&mut out, &budget, current_hour(fixture))
            .unwrap();
    // This is the production prelude: a failed old-pin charge poisons the
    // same quota passed to the reader; it does not replace the old baseline.
    let pin_retained =
        division_output_storage::retain_baseline_membership(&trackers.division_movement, &budget)
            .is_ok();
    let read = read_actual(fixture, trackers, &budget);
    Prepared {
        read,
        out,
        budget,
        gap,
        pin_retained,
        first_sequence: FIRST_SEQUENCE,
    }
}

struct Publication {
    out: Vec<u8>,
    sequence: u64,
    first_sequence: u64,
    hour: i32,
    outcomes: DomainOutcomeLedger,
    budget: Arc<country_roster_readset::Budget>,
    _copies: FrameCopyReservation,
}

fn publish(
    fixture: &Fixture,
    trackers: &mut CompactPerfGateTrackers,
    prepared: Prepared,
) -> Publication {
    let Prepared {
        read,
        mut out,
        budget,
        gap,
        first_sequence,
        ..
    } = prepared;
    let mut sequence = first_sequence;
    let mut outcomes = DomainOutcomeLedger::default();
    let copies = division_output_storage::publish_read(
        read,
        &mut out,
        &mut sequence,
        0,
        current_hour(fixture),
        &roots(fixture),
        &budget,
        &mut trackers.division_movement,
        &mut trackers.domain_gap_ordinals,
        &mut outcomes,
        gap,
    );
    // The surviving DATA or GAP copy receipt retains this exact frame quota.
    assert_eq!(Arc::strong_count(&budget), 2);
    Publication {
        out,
        sequence,
        first_sequence,
        hour: current_hour(fixture),
        outcomes,
        budget,
        _copies: copies,
    }
}

fn records(publication: &Publication) -> Vec<(compact_contract::CompactRecordHeader, &[u8])> {
    let out = &publication.out;
    let mut records = Vec::new();
    let mut offset = 0;
    while offset < out.len() {
        let header = compact_contract::parse_compact_record_header(&out[offset..]).unwrap();
        assert_eq!(header.domain_id, COMPACT_DOMAIN_DIVISIONS);
        assert_eq!(header.game_hour_raw, publication.hour);
        assert_eq!(
            header.sequence,
            publication.first_sequence + records.len() as u64
        );
        let end = offset + header.record_len();
        assert!(end <= out.len());
        records.push((
            header,
            &out[offset + usize::from(COMPACT_RECORD_HEADER_LEN)..end],
        ));
        offset = end;
    }
    records
}

fn word(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap())
}

fn assert_data(publication: &Publication, words: Option<&[i32]>, delta: bool) {
    let records = records(publication);
    assert_eq!(records.len(), 2);
    assert_eq!(publication.sequence, publication.first_sequence + 2);
    assert_eq!(records[0].0.record_type, COMPACT_RECORD_TYPE_DOMAIN_PERF);
    assert_eq!(records[1].0.record_type, COMPACT_RECORD_TYPE_DOMAIN_DATA);
    assert!(failure_diagnosis(&publication.outcomes, COMPACT_DOMAIN_DIVISIONS).is_none());
    let payload = records[1].1;
    let flags = word(payload, 16);
    assert!(!compact_contract::compact_status_is_failure(flags));
    assert_eq!(flags & COMPACT_STATUS_DELTA_DATA != 0, delta);
    assert_eq!(word(payload, 8), u32::from(words.is_some()));
    assert_eq!(word(payload, 12), u32::from(words.is_some()));
    assert_eq!(word(payload, 24), 1);
    let parsed = compact_contract::division_routes::DivisionRoutes::parse(payload).unwrap();
    if let Some(words) = words {
        assert_eq!(parsed.first_combat_countries(0).unwrap(), membership(words));
    }
}

fn assert_gap(publication: &Publication, ordinal: u64, reason: DomainGapReason) {
    let records = records(publication);
    assert_eq!(
        records.len(),
        1,
        "no successful PERF or DATA prefix may survive"
    );
    assert_eq!(publication.sequence, publication.first_sequence + 1);
    assert_eq!(
        records[0].0.record_type,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP
    );
    let gap = DomainGapRecordV1::decode(records[0].1).unwrap();
    assert_eq!(gap.reason, reason);
    assert_eq!(gap.target_hour_raw, publication.hour);
    assert_eq!(gap.outcome_ordinal, ordinal);
    assert_eq!(gap.part, domain_gap_part(COMPACT_DOMAIN_DIVISIONS));
}

struct FoldStream {
    bytes: Vec<u8>,
    sequence: u64,
}

impl FoldStream {
    fn accept(&mut self, publication: &Publication) -> serde_json::Value {
        assert_eq!(publication.first_sequence, self.sequence);
        // Preserve the real publisher envelopes and payloads byte-for-byte.
        // stage compares ordinary WorldFold with its actual V3 cache seam.
        self.bytes.extend_from_slice(&publication.out);
        let snapshot = stage(&self.bytes, publication.sequence, publication.hour);
        append_compact_record(
            &mut self.bytes,
            publication.sequence,
            publication.hour,
            compact_contract::COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &[0; compact_contract::record_semantics::COMPACT_FRAME_END_PAYLOAD_LEN],
        );
        self.sequence = publication.sequence + 1;
        snapshot
    }
}

fn assert_fold_current(value: &serde_json::Value, words: &[i32]) {
    let snapshot = &value["snapshot"];
    let rows = snapshot["divisions"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["div_id"], 7);
    assert_eq!(
        rows[0]["first_combat_countries"],
        serde_json::json!({
            "schema": 1, "status": "present",
            "combat": {"class": "land", "type": 62, "id": 79},
            "countries": {"status": "measured", "raw_i32": words},
        })
    );
    assert_eq!(
        rows[0]["training_scalars"]["country_operand"],
        serde_json::json!({"status":"measured", "raw_i32":COUNTRY})
    );
    assert!(snapshot["provenance"]["collector"]["domain_problems"]
        .as_array()
        .unwrap()
        .iter()
        .all(|p| p["domain_id"] != COMPACT_DOMAIN_DIVISIONS));
}

fn assert_fold_gap(value: &serde_json::Value) {
    assert!(value["snapshot"]["divisions"].is_null());
    assert!(
        value["snapshot"]["provenance"]["collector"]["domain_problems"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["domain_id"] == COMPACT_DOMAIN_DIVISIONS && p["problem"] == "domain_gap")
    );
}

fn seed(fixture: &Fixture) -> (CompactPerfGateTrackers, FoldStream) {
    let mut trackers = CompactPerfGateTrackers::default();
    trackers
        .domain_gap_ordinals
        .insert(gap_key(), PRIOR_ORDINAL);
    let prepared = prepare(fixture, &mut trackers, Arc::new(Default::default()));
    let publication = publish(fixture, &mut trackers, prepared);
    assert_data(&publication, Some(&COUNTRIES), false);
    commit_country_staged_advances(&mut trackers, &BTreeSet::new());
    assert_pin(&trackers, &COUNTRIES);
    assert!(!trackers.division_movement.has_frame_entry());
    let mut stream = FoldStream {
        bytes: Vec::new(),
        sequence: FIRST_SEQUENCE,
    };
    assert_fold_current(&stream.accept(&publication), &COUNTRIES);
    (trackers, stream)
}

struct Costs {
    pin: usize,
    read: usize,
    payload: usize,
    copies: usize,
    publication: usize,
}

// Measure exact requested layouts through production functions in a separate
// unpublished calibration frame. Every phase shares one quota, as does each
// actual exhaustion attempt below. No guessed row/list size funds a frame.
fn costs(fixture: &Fixture, published: &mut CompactPerfGateTrackers) -> Costs {
    let mut staged = stage_compact_perf_gate_trackers(published);
    let budget = Arc::new(country_roster_readset::Budget::default());
    let mut out = Vec::new();
    let _gap =
        division_output_storage::reserve_storage_gap(&mut out, &budget, current_hour(fixture))
            .unwrap();
    division_output_storage::retain_baseline_membership(&staged.division_movement, &budget)
        .unwrap();
    let pin = budget.retained_bytes();
    let read = read_actual(fixture, &mut staged, &budget);
    assert!(!read.output_storage_unavailable);
    assert!(!compact_contract::compact_status_is_failure(
        read.status_flags
    ));
    assert_eq!(read.rows.len(), 1);
    let read_cost = budget.retained_bytes();
    let roots = roots(fixture);
    let payload = compact_divisions_data_payload(
        &read.rows,
        0,
        read.status_flags,
        read.changed_rows,
        read.root_rows,
        roots.country_roster.as_ref(),
        roots.railway_roster.as_ref(),
        &budget,
    )
    .unwrap();
    let payload_cost = budget.retained_bytes();
    let (units, count) = division_output_storage::diagnostics(&read);
    let perf = division_output_storage::encode_perf(&read, 0, &units[..count]).unwrap();
    let _copies = division_output_storage::reserve_frame_copies(
        payload.capacity(),
        &perf,
        current_hour(fixture),
        &budget,
    )
    .unwrap();
    let copies = budget.retained_bytes();
    division_output_storage::reserve_publication(&mut out, payload.len(), &units[..count], &budget)
        .unwrap();
    let publication = budget.retained_bytes();
    assert!(
        pin < read_cost
            && read_cost < payload_cost
            && payload_cost < copies
            && copies < publication
    );
    restore_country_trackers_after_refused_frame(&mut staged, published);
    assert_pin(published, &COUNTRIES);
    Costs {
        pin,
        read: read_cost,
        payload: payload_cost,
        copies,
        publication,
    }
}

fn assert_recovery(
    fixture: &mut Fixture,
    trackers: &mut CompactPerfGateTrackers,
    stream: &mut FoldStream,
) {
    next_hour(fixture);
    let mut prepared = prepare(fixture, trackers, Arc::new(Default::default()));
    prepared.first_sequence = stream.sequence;
    let recovered = publish(fixture, trackers, prepared);
    assert_data(&recovered, Some(&CHANGED_COUNTRIES), false);
    assert_fold_current(&stream.accept(&recovered), &CHANGED_COUNTRIES);
    assert_pin(trackers, &CHANGED_COUNTRIES);
    commit_country_staged_advances(trackers, &BTreeSet::new());
    next_hour(fixture);
    let mut prepared = prepare(fixture, trackers, Arc::new(Default::default()));
    prepared.first_sequence = stream.sequence;
    let unchanged = publish(fixture, trackers, prepared);
    assert_data(&unchanged, None, true);
    assert_fold_current(&stream.accept(&unchanged), &CHANGED_COUNTRIES);
    assert_pin(trackers, &CHANGED_COUNTRIES);
    commit_country_staged_advances(trackers, &BTreeSet::new());
}

#[test]
fn actual_reader_and_publisher_exhaustion_emit_one_gap_and_force_full_recovery() {
    let _counters = lock_rc_counters();
    for phase in 0..6 {
        let mut fixture = Fixture::new();
        let (mut published, mut stream) = seed(&fixture);
        fixture.nodes.write_i32(0x20, CHANGED_COUNTRIES[1]);
        next_hour(&mut fixture);
        let costs = costs(&fixture, &mut published);
        let (limit, expected_reason) = [
            (costs.pin - 1, "D20 sampled row output storage unavailable"),
            (costs.pin, "D20 sampled row output storage unavailable"),
            (costs.read - 1, "D20 sampled row output storage unavailable"),
            (costs.payload - 1, "D20 output storage unavailable"),
            (
                costs.copies - 1,
                "D20 downstream frame copy storage budget exhausted",
            ),
            (costs.publication - 1, "D20 publication storage unavailable"),
        ][phase];
        let mut staged = stage_compact_perf_gate_trackers(&mut published);
        let mut prepared = prepare(
            &fixture,
            &mut staged,
            Arc::new(country_roster_readset::Budget::test_limits(
                country_roster_readset::VERIFICATION_LIMIT,
                limit,
            )),
        );
        prepared.first_sequence = stream.sequence;
        assert_eq!(prepared.pin_retained, phase != 0);
        assert_eq!(
            prepared.read.output_storage_unavailable,
            phase < 3,
            "phase {phase}"
        );
        assert_eq!(prepared.read.root_rows, 1);
        if phase < 3 {
            assert!(prepared.read.rows.is_empty());
            assert_eq!(prepared.read.changed_rows, 0);
            assert_pin(&staged, &COUNTRIES);
            assert!(!staged.division_movement.has_frame_entry());
        } else {
            assert_eq!(prepared.read.rows.len(), 1);
            assert!(!compact_contract::compact_status_is_failure(
                prepared.read.status_flags
            ));
            assert_pin(&staged, &CHANGED_COUNTRIES);
        }
        let publication = publish(&fixture, &mut staged, prepared);
        assert_gap(
            &publication,
            PRIOR_ORDINAL,
            DomainGapReason::SourceUnavailable,
        );
        assert_eq!(
            failure_diagnosis(&publication.outcomes, COMPACT_DOMAIN_DIVISIONS)
                .unwrap()
                .reason,
            expected_reason
        );
        assert_eq!(publication.budget.retained_bytes(), limit + 1);
        assert_fold_gap(&stream.accept(&publication));
        assert_eq!(
            staged.domain_gap_ordinals.get(&gap_key()),
            Some(&(PRIOR_ORDINAL + 1))
        );
        assert!(staged.division_movement.is_empty());
        assert_eq!(
            staged
                .division_movement
                .frame_entry
                .as_ref()
                .unwrap()
                .get(&7)
                .unwrap()
                .first_combat_countries,
            membership(&COUNTRIES)
        );
        commit_country_staged_advances(&mut staged, &BTreeSet::new());
        assert!(staged.division_movement.is_empty());
        assert!(!staged.division_movement.has_frame_entry());
        assert_recovery(&mut fixture, &mut staged, &mut stream);
    }
}

#[test]
fn actual_prepared_gap_and_data_both_restore_the_old_pin_when_whole_frame_is_refused() {
    let _counters = lock_rc_counters();
    for exhaust in [false, true] {
        let mut fixture = Fixture::new();
        let (mut published, mut stream) = seed(&fixture);
        fixture.nodes.write_i32(0x20, CHANGED_COUNTRIES[1]);
        next_hour(&mut fixture);
        let costs = costs(&fixture, &mut published);
        let mut staged = stage_compact_perf_gate_trackers(&mut published);
        let limit = if exhaust {
            costs.payload - 1
        } else {
            country_roster_readset::RETAINED_LIMIT
        };
        let mut prepared = prepare(
            &fixture,
            &mut staged,
            Arc::new(country_roster_readset::Budget::test_limits(
                country_roster_readset::VERIFICATION_LIMIT,
                limit,
            )),
        );
        prepared.first_sequence = stream.sequence;
        let publication = publish(&fixture, &mut staged, prepared);
        if exhaust {
            assert_gap(
                &publication,
                PRIOR_ORDINAL,
                DomainGapReason::SourceUnavailable,
            );
            assert_eq!(
                staged.domain_gap_ordinals.get(&gap_key()),
                Some(&(PRIOR_ORDINAL + 1))
            );
        } else {
            assert_data(&publication, Some(&CHANGED_COUNTRIES), true);
        }
        // These bytes never reach the sink. Use the actual whole-frame salvage
        // path, including its separately held published gap ordinal map.
        restore_country_trackers_after_refused_frame(&mut staged, &mut published);
        assert_pin(&published, &COUNTRIES);
        assert!(!published.division_movement.has_frame_entry());
        assert_eq!(
            published.domain_gap_ordinals.get(&gap_key()),
            Some(&PRIOR_ORDINAL)
        );
        let mut prepared = prepare(&fixture, &mut published, Arc::new(Default::default()));
        prepared.first_sequence = stream.sequence;
        let retry = publish(&fixture, &mut published, prepared);
        assert_data(&retry, Some(&CHANGED_COUNTRIES), true);
        assert_fold_current(&stream.accept(&retry), &CHANGED_COUNTRIES);
        assert_pin(&published, &CHANGED_COUNTRIES);
        commit_country_staged_advances(&mut published, &BTreeSet::new());
    }
}

#[test]
fn actual_late_bracket_replacement_commits_gap_and_requires_complete_recovery() {
    let _counters = lock_rc_counters();
    let mut fixture = Fixture::new();
    let (mut published, mut stream) = seed(&fixture);
    fixture.nodes.write_i32(0x20, CHANGED_COUNTRIES[1]);
    next_hour(&mut fixture);
    let mut staged = stage_compact_perf_gate_trackers(&mut published);
    let mut prepared = prepare(&fixture, &mut staged, Arc::new(Default::default()));
    prepared.first_sequence = stream.sequence;
    let mut publication = publish(&fixture, &mut staged, prepared);
    assert_data(&publication, Some(&CHANGED_COUNTRIES), true);
    let (next, withheld) = replace_rejected_domain_records_with_root_dependencies(
        &mut publication.out,
        publication.first_sequence,
        publication.hour,
        &[DomainClockBracket {
            tag: COMPACT_DOMAIN_DIVISIONS,
            opening: Some(clock_obs(publication.hour, 1)),
            closing: Some(clock_obs(publication.hour + 1, 2)),
            worker_panicked: false,
        }],
        &mut staged.domain_gap_ordinals,
        HourLabelMismatchPolicy::Refuse,
        &[],
    )
    .unwrap();
    publication.sequence = next;
    assert_gap(&publication, PRIOR_ORDINAL, DomainGapReason::Torn);
    assert_fold_gap(&stream.accept(&publication));
    assert!(withheld.contains(&COMPACT_DOMAIN_DIVISIONS));
    assert_eq!(
        staged.domain_gap_ordinals.get(&gap_key()),
        Some(&(PRIOR_ORDINAL + 1))
    );
    reset_trackers_after_rejected_bracket(&mut staged, &withheld);
    assert!(staged.division_movement.is_empty());
    assert_eq!(
        staged
            .division_movement
            .frame_entry
            .as_ref()
            .unwrap()
            .get(&7)
            .unwrap()
            .first_combat_countries,
        membership(&COUNTRIES)
    );
    commit_country_staged_advances(&mut staged, &withheld);
    assert!(staged.division_movement.is_empty());
    assert!(!staged.division_movement.has_frame_entry());
    assert_recovery(&mut fixture, &mut staged, &mut stream);
}
