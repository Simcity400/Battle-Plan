//! Lane RU's proof that moving the country publish encode into the children
//! kept every recorded byte identical.
//!
//! The reference half of each test restates, statement for statement, what the
//! frame tail's `CountryPublish` segment did before lane RU: the two row walks
//! per spec, `append_compact_domain_perf_record`, then the domain's data
//! payload through `append_compact_record`. The actual half runs the new child
//! encode (`country_bundle_publication_records`) and appends its pre-built
//! records the way the tail now does. Equality of the two outputs — across ALL
//! EIGHT arms, including D13's tracker evolution frame over frame, and on REAL
//! panic substitutes from `compact_worker_panic_read` — is the whole claim.

use super::*;

fn spec_of(id: u16) -> CompactCountryDomainSpec {
    *compact_country_domain_specs()
        .iter()
        .find(|spec| spec.id == id)
        .expect("country spec")
}

fn test_dictionary() -> TechnologyIdDictionary {
    TechnologyIdDictionary {
        by_definition_index: (0..8u32)
            .map(|index| (index, (0usize, format!("tech_{index}"))))
            .collect(),
    }
}

fn fuel_row(tag_db_id: u32, current_raw: i64) -> CountryObjectRow {
    let mut row = super::domain13_tech_delta_tests::bare_country_row(tag_db_id);
    row.fuel = Some(CountryFuelRow {
        current_raw,
        max_raw: current_raw + 100,
        gain_raw: 3,
        cost_raw: 1,
        current: 0.0,
        max: 0.0,
        gain: 0.0,
        cost: 0.0,
    });
    row
}

fn research_row(tag_db_id: u32, seed: u32) -> CountryObjectRow {
    let mut row = super::domain13_tech_delta_tests::bare_country_row(tag_db_id);
    // Classifier enums are wire values whose resident range starts at 1
    // (domain13's own fixtures use 1..=6); staying inside it keeps these rows
    // on the accepted-witness path.
    let states: Vec<(u32, u8)> = (0..(1 + seed % 4))
        .map(|index| (index, 1 + ((seed + index) % 6) as u8))
        .collect();
    let completed: Vec<u32> = states
        .iter()
        .filter(|(_, state)| *state == 4)
        .map(|(def_index, _)| *def_index)
        .collect();
    row.research = Some(CountryResearchRow {
        completed_tech_count: Some(completed.len().min(u32::MAX as usize) as u32),
        research_slot_count: Some(1 + seed % 3),
        active_research: None,
        technology_witness: Ok(Arc::new(CountryTechnologyWitness { states, completed })),
    });
    row
}

/// Six mixed countries: two with fuel, three with research witnesses, one bare.
fn mixed_rows() -> Vec<CountryObjectRow> {
    vec![
        fuel_row(1, 500),
        research_row(2, 1),
        fuel_row(3, 900),
        research_row(4, 3),
        super::domain13_tech_delta_tests::bare_country_row(5),
        research_row(6, 2),
    ]
}

fn bundle_read(specs: &[u16], rows: Vec<CountryObjectRow>) -> CompactBundleRead {
    CompactBundleRead {
        specs: specs.iter().copied().map(spec_of).collect(),
        rows,
        read_time_us: 4_242,
        row_count: 6,
        status_flags: 7,
        worker_panic_reason: None,
        technology_ids: Arc::new(test_dictionary()),
        published_records: None,
    }
}

fn all_spec_ids() -> Vec<u16> {
    compact_country_domain_specs()
        .iter()
        .map(|spec| spec.id)
        .collect()
}

fn all_specs() -> Vec<CompactCountryDomainSpec> {
    compact_country_domain_specs().to_vec()
}

/// THE REFERENCE: the old inline drain body, statement for statement.
fn reference_inline_drain(
    read: &CompactBundleRead,
    elapsed_wall_ms: u64,
    tech_delta: &mut CompactTechCountryDeltaTracker,
    out: &mut Vec<u8>,
    sequence: &mut u64,
    game_hour_raw: i32,
) -> u32 {
    let mut emitted_domains = 0u32;
    for spec in &read.specs {
        let changed_rows = read
            .rows
            .iter()
            .filter(|row| row_has_compact_domain(row, spec.kind))
            .count();
        let unit_count = compact_domain_unit_count(&read.rows, spec.kind);
        append_compact_domain_perf_record(
            out,
            *sequence,
            game_hour_raw,
            CompactDomainPerfRecord {
                domain_id: spec.id,
                read_time_us: read.read_time_us,
                row_count: read.row_count,
                changed_rows: changed_rows.min(u32::MAX as usize) as u32,
                unit_count,
                changed_unit_count: unit_count,
                status_flags: read.status_flags,
            },
            elapsed_wall_ms,
        );
        *sequence += 1;
        let (domain_id, payload) = match spec.kind {
            CompactCountryDomainKind::Fuel => (
                COMPACT_DOMAIN_FUEL,
                compact_fuel_data_payload(&read.rows, elapsed_wall_ms, read.status_flags),
            ),
            CompactCountryDomainKind::Production => (
                COMPACT_DOMAIN_PRODUCTION,
                compact_production_data_payload(&read.rows, elapsed_wall_ms, read.status_flags),
            ),
            CompactCountryDomainKind::Focus => (
                COMPACT_DOMAIN_FOCUS,
                compact_focus_data_payload(&read.rows, elapsed_wall_ms, read.status_flags),
            ),
            CompactCountryDomainKind::Tech => (
                COMPACT_DOMAIN_TECH,
                compact_tech_data_payload_delta(
                    &read.rows,
                    elapsed_wall_ms,
                    read.status_flags,
                    tech_delta,
                    &read.technology_ids,
                ),
            ),
            CompactCountryDomainKind::Politics => (
                COMPACT_DOMAIN_POLITICS,
                compact_politics_data_payload(&read.rows, elapsed_wall_ms, read.status_flags),
            ),
            CompactCountryDomainKind::PoliticsResiduals => (
                COMPACT_DOMAIN_POLITICS_RESIDUALS,
                compact_politics_residuals_data_payload(
                    &read.rows,
                    elapsed_wall_ms,
                    read.status_flags,
                ),
            ),
            CompactCountryDomainKind::Decisions => (
                COMPACT_DOMAIN_DECISIONS,
                compact_decisions_data_payload(&read.rows, elapsed_wall_ms, read.status_flags),
            ),
            CompactCountryDomainKind::Diplomacy => (
                COMPACT_DOMAIN_DIPLOMACY,
                compact_diplomacy_data_payload(&read.rows, elapsed_wall_ms, read.status_flags),
            ),
        };
        append_compact_record(
            out,
            *sequence,
            game_hour_raw,
            domain_id,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &payload,
        );
        *sequence += 1;
        emitted_domains += 1;
    }
    emitted_domains
}

/// The tail's new publication loop: headers and memcpys over pre-built bytes.
fn append_prebuilt(
    records: Vec<PrebuiltCountryRecord>,
    out: &mut Vec<u8>,
    sequence: &mut u64,
    game_hour_raw: i32,
) -> u32 {
    let mut emitted_domains = 0u32;
    for record in records {
        append_compact_record(
            out,
            *sequence,
            game_hour_raw,
            record.domain_id,
            record.record_type,
            record.flags,
            &record.payload,
        );
        *sequence += 1;
        if record.is_data_record {
            emitted_domains += 1;
        }
    }
    emitted_domains
}

/// ALL EIGHT arms over the mixed roster in ONE read, encoded both ways, then a
/// second frame with CHANGED rows against seeded pins so the tech delta branch
/// runs against a baseline instead of repeating the whole-payload branch.
#[test]
fn a_child_encoded_publication_is_byte_identical_to_the_old_inline_drain() {
    let hour = -77;
    let wall = 31;

    let frame_one_rows = mixed_rows();
    // Frame two changes one country's fuel level and one country's witness.
    let mut frame_two_rows = frame_one_rows.clone();
    frame_two_rows[0].fuel.as_mut().unwrap().current_raw = 501;
    if let Some(research) = frame_two_rows[1].research.as_mut() {
        let witness = research.technology_witness.as_ref().unwrap();
        let mut states = witness.states.clone();
        // Advance one classifier state, wrapping 6 -> 1 to stay in range.
        states[0].1 = if states[0].1 >= 6 { 1 } else { states[0].1 + 1 };
        let completed: Vec<u32> = states
            .iter()
            .filter(|(_, state)| *state == 4)
            .map(|(def_index, _)| *def_index)
            .collect();
        research.completed_tech_count = Some(completed.len().min(u32::MAX as usize) as u32);
        research.technology_witness =
            Ok(Arc::new(CountryTechnologyWitness { states, completed }));
    }

    let specs = all_spec_ids();
    let mut reference_out = Vec::new();
    let mut prebuilt_out = Vec::new();
    let mut reference_sequence = 9_000u64;
    let mut prebuilt_sequence = 9_000u64;
    let mut reference_tracker = CompactTechCountryDeltaTracker::default();
    let mut prebuilt_tracker = CompactTechCountryDeltaTracker::default();

    for rows in [frame_one_rows, frame_two_rows] {
        let read = bundle_read(&specs, rows);

        let reference_emitted = reference_inline_drain(
            &read,
            wall,
            &mut reference_tracker,
            &mut reference_out,
            &mut reference_sequence,
            hour,
        );

        let records =
            country_bundle_publication_records(&read, wall, Some(&mut prebuilt_tracker));
        let prebuilt_emitted =
            append_prebuilt(records, &mut prebuilt_out, &mut prebuilt_sequence, hour);

        assert_eq!(reference_emitted, prebuilt_emitted);
        assert_eq!(
            reference_out, prebuilt_out,
            "child-encoded publication bytes diverged from the old inline drain"
        );
    }
    assert_eq!(reference_sequence, prebuilt_sequence);
    // Two frames x eight specs x one PERF+DATA pair per spec.
    assert_eq!(reference_emitted_frame_count(&reference_out), 16);
}

/// Count PERF+DATA PAIRS in an encoded buffer by walking its record headers.
fn reference_emitted_frame_count(buffer: &[u8]) -> usize {
    let header_len = usize::from(compact_contract::COMPACT_RECORD_HEADER_LEN);
    let mut records = 0usize;
    let mut cursor = 0usize;
    while cursor < buffer.len() {
        let len = u32::from_le_bytes(
            buffer[cursor + 24..cursor + 28].try_into().expect("len"),
        ) as usize;
        cursor += header_len + len;
        records += 1;
    }
    assert_eq!(cursor, buffer.len(), "record walk must land on the boundary");
    records / 2
}

/// The panic-substitute fallback encodes to exactly the bytes the OLD INLINE
/// drain published for the same substitute. The substitute is the real thing:
/// `compact_worker_panic_read`'s zero scalars, WORKER_PANIC badge, and the
/// war-segment-absent bit its all-specs shape carries.
#[test]
fn the_tail_fallback_republishes_a_real_panic_substitute_byte_identically() {
    let hour = -5;
    let wall = 12;
    let substitute_specs = compact_worker_panic_read(all_specs(), 7, None);

    let mut fallback_out = Vec::new();
    let mut sequence = 40u64;
    let fallback_records = {
        let mut read = substitute_specs;
        match read.published_records.take() {
            Some(records) => records,
            None => country_bundle_publication_records(
                &read,
                wall,
                Some(&mut CompactTechCountryDeltaTracker::default()),
            ),
        }
    };
    let fallback_emitted =
        append_prebuilt(fallback_records, &mut fallback_out, &mut sequence, hour);

    // The same substitute through the old inline drain body.
    let twin = compact_worker_panic_read(all_specs(), 7, None);
    let mut reference_out = Vec::new();
    let mut reference_sequence = 40u64;
    let reference_emitted = reference_inline_drain(
        &twin,
        wall,
        &mut CompactTechCountryDeltaTracker::default(),
        &mut reference_out,
        &mut reference_sequence,
        hour,
    );

    assert_eq!(fallback_emitted, reference_emitted);
    assert_eq!(fallback_out, reference_out);
    assert_ne!(
        fallback_out.len(),
        0,
        "a substitute still publishes sixteen records"
    );
}

/// Sixteen records for the eight-spec read, PERF before DATA per spec, in
/// contract order — and the PERF payload scalars sit at the positions the
/// contract fixes, holding each spec's own walks.
#[test]
fn the_prebuilt_records_carry_the_contract_order_and_perf_scalars() {
    let wall = 44u64;
    let read = bundle_read(&all_spec_ids(), mixed_rows());
    let records = country_bundle_publication_records(
        &read,
        wall,
        Some(&mut CompactTechCountryDeltaTracker::default()),
    );

    let mut expected: Vec<(u16, bool)> = Vec::new();
    for spec in &read.specs {
        expected.push((spec.id, false));
        expected.push((spec.id, true));
    }
    let observed: Vec<(u16, bool)> = records
        .iter()
        .map(|record| (record.domain_id, record.is_data_record))
        .collect();
    assert_eq!(observed, expected);

    for record in &records {
        let expected_type = if record.is_data_record {
            COMPACT_RECORD_TYPE_DOMAIN_DATA
        } else {
            COMPACT_RECORD_TYPE_DOMAIN_PERF
        };
        assert_eq!(record.record_type, expected_type);
        assert!(!record.payload.is_empty());
    }

    // Decode every PERF payload's fixed scalar block and require each spec's
    // own walk results at the contract offsets. A mis-mapped arm shows up here
    // as another domain's counts.
    let mut perfect_pairs = records.chunks(2);
    for spec in &read.specs {
        let pair = perfect_pairs.next().expect("perf/data pair");
        let perf = &pair[0];
        assert!(!perf.is_data_record);
        let changed_expected = read
            .rows
            .iter()
            .filter(|row| row_has_compact_domain(row, spec.kind))
            .count()
            .min(u32::MAX as usize) as u32;
        let units_expected = compact_domain_unit_count(&read.rows, spec.kind);
        assert_eq!(u64::from_le_bytes(perf.payload[0..8].try_into().unwrap()), wall);
        assert_eq!(
            u64::from_le_bytes(perf.payload[8..16].try_into().unwrap()),
            read.read_time_us
        );
        assert_eq!(u32::from_le_bytes(perf.payload[16..20].try_into().unwrap()), 6);
        assert_eq!(
            u32::from_le_bytes(perf.payload[20..24].try_into().unwrap()),
            changed_expected
        );
        assert_eq!(
            u32::from_le_bytes(perf.payload[24..28].try_into().unwrap()),
            units_expected
        );
        assert_eq!(
            u32::from_le_bytes(perf.payload[28..32].try_into().unwrap()),
            units_expected
        );
        assert_eq!(
            u32::from_le_bytes(perf.payload[32..36].try_into().unwrap()),
            read.status_flags
        );
        assert_eq!(u32::from_le_bytes(perf.payload[36..40].try_into().unwrap()), 0);
    }

    // Spot-check one DATA header against its own format: the fuel payload —
    // the first spec's data record — carries the fuel row count twice behind
    // the wall time.
    let fuel_data = &records[1];
    assert!(fuel_data.is_data_record);
    assert_eq!(fuel_data.domain_id, COMPACT_DOMAIN_FUEL);
    let fuel_rows = read
        .rows
        .iter()
        .filter(|row| row.fuel.is_some())
        .count()
        .min(u32::MAX as usize) as u32;
    assert_eq!(
        u32::from_le_bytes(fuel_data.payload[8..12].try_into().unwrap()),
        fuel_rows
    );
    assert_eq!(
        u32::from_le_bytes(fuel_data.payload[12..16].try_into().unwrap()),
        fuel_rows
    );
}
