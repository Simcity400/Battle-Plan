#[test]
fn compact_contract_calendar_agrees_with_the_shared_admission_window() {
    assert_eq!(
        compact_contract::calendar::DATE_EPOCH_CONST,
        portable_admission::DATE_EPOCH_CONST,
        "compact_contract::calendar::DATE_EPOCH_CONST drifted from \
         portable_admission::DATE_EPOCH_CONST. The replay consumer converts \
         raw dates with the shared value; a producer that stamps hours with a \
         different epoch has every row rejected as an out-of-window date."
    );

    for probe in [
        portable_admission::RAW_DATE_MIN - 1,
        portable_admission::RAW_DATE_MIN,
        portable_admission::RAW_DATE_MIN + 1,
        portable_admission::RAW_DATE_MAX - 1,
        portable_admission::RAW_DATE_MAX,
        portable_admission::RAW_DATE_MAX + 1,
    ] {
        let producer = compact_contract::calendar::hour_index_from_raw(probe);
        let shared = portable_admission::hour_index_from_raw(probe);
        assert_eq!(
            producer, shared,
            "compact_contract::calendar::hour_index_from_raw and \
             portable_admission::hour_index_from_raw disagree at raw date \
             {probe}. One side admits an hour the other discards."
        );
    }
}

#[test]
fn d67_raw_date_window_agrees_with_the_shared_admission_window() {
    assert_eq!(
        wire_format::d67::DATE_EPOCH_CONST,
        portable_admission::DATE_EPOCH_CONST,
        "wire_format::d67::DATE_EPOCH_CONST drifted from the shared epoch"
    );
    assert_eq!(
        wire_format::d67::RAW_DATE_MIN,
        portable_admission::RAW_DATE_MIN,
        "wire_format::d67::RAW_DATE_MIN drifted from the shared window"
    );
    assert_eq!(
        wire_format::d67::RAW_DATE_MAX,
        portable_admission::RAW_DATE_MAX,
        "wire_format::d67::RAW_DATE_MAX drifted from the shared window"
    );
}

#[test]
fn compact_owner_tag_db_id_bound_agrees_with_the_shared_bound() {
    assert_eq!(
        compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID,
        portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID,
        "compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID drifted \
         from portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID. Raising it here \
         alone means the extra owners are collected and then discarded by the \
         replay consumer, which still applies the shared ceiling."
    );
}

#[test]
fn the_structural_tag_db_size_is_the_engines_signed_index_type_on_both_sides() {
    assert_eq!(
        compact_contract::country_tag_db::MAX_STRUCTURAL_TAG_DB_SIZE,
        portable_admission::MAX_STRUCTURAL_TAG_DB_SIZE,
        "the producer and the shared crate disagree about the width of the \
         engine's country-database index type"
    );
    assert_eq!(
        portable_admission::MAX_STRUCTURAL_TAG_DB_SIZE,
        i32::MAX as u32,
        "the engine's accessor compares a signed int index against a signed \
         int size"
    );
    assert_eq!(
        portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID + 1,
        portable_admission::MAX_STRUCTURAL_TAG_DB_SIZE
    );
    assert_eq!(
        portable_admission::MAX_COUNTRY_TABLE_ROWS,
        portable_admission::MAX_STRUCTURAL_TAG_DB_SIZE,
        "a country ROW COUNT is bounded by the size, not by the size minus one"
    );
    assert_eq!(
        compact_contract::build_profile::MAX_COUNTRY_TABLE_ROWS,
        portable_admission::MAX_COUNTRY_TABLE_ROWS
    );
}

#[test]
fn every_country_identity_consumer_applies_the_shared_tag_db_bound() {
    for (name, value) in [
        (
            "wire_format::d46::ace::MAX_TAG_DB_ID",
            wire_format::d46::ace::MAX_TAG_DB_ID,
        ),
        (
            "wire_format::d48::MAX_COMPACT_OWNER_TAG_DB_ID",
            wire_format::d48::MAX_COMPACT_OWNER_TAG_DB_ID,
        ),
        (
            "wire_format::d50::MAX_OWNER_TAG_DB_ID",
            wire_format::d50::MAX_OWNER_TAG_DB_ID,
        ),
        (
            "wire_format::d51::MAX_COUNTRY_TAG_DB_ID",
            wire_format::d51::MAX_COUNTRY_TAG_DB_ID,
        ),
        (
            "wire_format::d56::MAX_OWNER_TAG_DB_ID",
            wire_format::d56::MAX_OWNER_TAG_DB_ID,
        ),
        (
            "wire_format::d62::MAX_COUNTRY_COUNT",
            wire_format::d62::MAX_COUNTRY_COUNT,
        ),
        (
            "wire_format::d67::MAX_OWNER_TAG_DB_ID",
            wire_format::d67::MAX_OWNER_TAG_DB_ID,
        ),
        (
            "wire_format::d68::MAX_PLAYER_TAG_DB_ID",
            wire_format::d68::MAX_PLAYER_TAG_DB_ID,
        ),
        (
            "wire_format::d11::MAX_PRODUCTION_LICENSE_TAG_DB_ID",
            wire_format::d11::MAX_PRODUCTION_LICENSE_TAG_DB_ID,
        ),
        (
            "wire_format::d94::MAX_POV_COUNTRY_ID",
            wire_format::d94::MAX_POV_COUNTRY_ID,
        ),
        (
            "wire_format::d95::MAX_POV_COUNTRY_ID",
            wire_format::d95::MAX_POV_COUNTRY_ID,
        ),
        (
            "compact_contract::pov_visibility::MAX_POV_COUNTRY_ID",
            compact_contract::pov_visibility::MAX_POV_COUNTRY_ID,
        ),
        (
            "compact_contract::spotted_units::MAX_POV_COUNTRY_ID",
            compact_contract::spotted_units::MAX_POV_COUNTRY_ID,
        ),
        (
            "compact_contract::raid_instances::MAX_TAG_DB_ID",
            compact_contract::raid_instances::MAX_TAG_DB_ID,
        ),
        (
            "compact_contract::build_profile::DOMAIN66_MAX_TAG_DB_ID",
            compact_contract::build_profile::DOMAIN66_MAX_TAG_DB_ID,
        ),
        (
            "compact_contract::build_profile::DOMAIN68_MAX_PLAYER_TAG_DB_ID",
            compact_contract::build_profile::DOMAIN68_MAX_PLAYER_TAG_DB_ID,
        ),
        (
            "compact_contract::build_profile::DOMAIN69_MAX_MEMBER_TAG_DB_ID",
            compact_contract::build_profile::DOMAIN69_MAX_MEMBER_TAG_DB_ID,
        ),
    ] {
        assert_eq!(
            value,
            portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID,
            "{name} does not apply the shared country-identity bound. A \
             consumer narrower than the producer discards rows the recorder \
             collected, and nothing on the producer side reports it."
        );
    }
    assert_eq!(
        portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID,
        2_147_483_646
    );
}

#[test]
fn the_link_vector_index_bound_is_derived_separately_from_the_database_bound() {
    assert_eq!(
        wire_format::d52::MAX_TAG_DB_ID,
        (i32::MAX as u32) - 1,
        "the link vector's own structural width: a signed 32-bit subscript"
    );
}

#[test]
fn a_consumer_admission_path_walks_the_old_bound_the_new_bound_and_one_past() {
    let admitted = |pov: u32| wire_format::d94::MAX_POV_COUNTRY_ID >= pov && pov != 0;

    assert!(admitted(2_048), "the old ceiling is still an ordinary id");
    assert!(
        admitted(2_049),
        "the first id the old ceiling discarded is an ordinary country tag"
    );
    assert!(admitted(portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID));
    assert!(
        !admitted(portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID + 1),
        "one past the ceiling must be refused, never truncated"
    );
    assert!(
        !admitted(portable_admission::MAX_STRUCTURAL_TAG_DB_SIZE),
        "no loaded database can index its own size"
    );
    assert!(
        !admitted(0x8000_0000),
        "a negative index to the engine's accessor"
    );
    assert!(!admitted(u32::MAX));

    for admitted_pov in [
        1u32,
        2_048,
        2_049,
        portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID,
    ] {
        let payload = fow1_payload(admitted_pov, &[0, 1, 2]);
        let (header, levels) = wire_format::d94::split_payload(&payload)
            .unwrap_or_else(|reason| panic!("d94 refused pov {admitted_pov}: {reason}"));
        assert_eq!(
            header.pov_country_id, admitted_pov,
            "the POV must survive the decode unchanged, never clamped"
        );
        assert_eq!(levels.len(), 3);
    }
    for refused_pov in [
        0u32,
        portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID + 1,
        portable_admission::MAX_STRUCTURAL_TAG_DB_SIZE,
        0x8000_0000,
        u32::MAX,
    ] {
        assert!(
            wire_format::d94::split_payload(&fow1_payload(refused_pov, &[0, 1, 2])).is_err(),
            "d94 admitted pov_country_id {refused_pov}"
        );
    }
}

#[test]
fn the_country_row_ceilings_admit_an_overhaul_and_still_refuse_an_impossible_count() {
    assert!(wire_format::d92::MAX_ROWS > 100_000);
    assert!(wire_format::d96::MAX_COUNTRIES > 100_000);
    assert_eq!(
        wire_format::d92::MAX_ROWS,
        compact_contract::preferred_tactic::MAX_ROWS,
        "the wire mirror and the shipping contract must agree"
    );
    assert_eq!(
        wire_format::d96::MAX_COUNTRIES,
        compact_contract::tactic_weight::MAX_COUNTRIES
    );

    assert_eq!(wire_format::d92::max_rows_in(0), 0);
    assert_eq!(
        wire_format::d92::max_rows_in(wire_format::d92::FIXED_ROW_LEN - 1),
        0
    );
    assert_eq!(
        wire_format::d92::max_rows_in(wire_format::d92::FIXED_ROW_LEN * 1_025),
        1_025,
        "an overhaul-sized roster fits when the bytes are actually there"
    );
    assert_eq!(
        wire_format::d96::max_rows_in((wire_format::d96::FIXED_ROW_LEN + 1) * 1_025),
        1_025
    );
    assert_eq!(
        wire_format::d92::max_rows_in(12_000),
        compact_contract::preferred_tactic::max_rows_in(12_000)
    );
    assert_eq!(
        wire_format::d96::max_rows_in(12_000),
        compact_contract::tactic_weight::max_rows_in(12_000)
    );

    let d92_refusal = wire_format::d92::split_payload(&ptc1_header_declaring(1))
        .expect_err("a PTC1 header declaring a row it has no bytes for must be refused");
    assert!(
        d92_refusal.contains("exceeds the payload's capacity"),
        "d92 refused for the wrong reason: {d92_refusal}"
    );
    assert!(
        wire_format::d92::split_payload(&ptc1_header_declaring(0)).is_ok(),
        "a PTC1 header declaring no rows is ordinary"
    );

    let d96_refusal = wire_format::d96::split_payload(&ptw1_header_declaring(1))
        .expect_err("a PTW1 header declaring a row it has no bytes for must be refused");
    assert!(
        d96_refusal.contains("exceeds the payload's capacity"),
        "d96 refused for the wrong reason: {d96_refusal}"
    );
    if let Err(reason) = wire_format::d96::split_payload(&ptw1_header_declaring(0)) {
        panic!("a PTW1 header declaring no rows is ordinary, got: {reason}");
    }
}

fn ptw1_header_declaring(row_count: u32) -> Vec<u8> {
    use wire_format::d96;
    let mut out = Vec::with_capacity(d96::HEADER_LEN);
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&row_count.max(1).to_le_bytes());
    out.extend_from_slice(&row_count.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&d96::MAGIC);
    out.extend_from_slice(&d96::VERSION.to_le_bytes());
    out.extend_from_slice(&(d96::HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&1u64.to_le_bytes());
    out.extend_from_slice(&(d96::FIXED_SCALE as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0i64.to_le_bytes());
    out.extend_from_slice(&d96::EXECUTABLE_SHA256);
    out.extend_from_slice(&d96::CONSUMER_EVIDENCE_SHA256);
    out.extend_from_slice(&d96::DEFINE_EVIDENCE_SHA256);
    out.extend_from_slice(&d96::LOOKUP_EVIDENCE_SHA256);
    out.extend_from_slice(&d96::RUNTIME_OBSERVATION_SHA256);
    out.extend_from_slice(&d96::TACTIC_KEY_CENSUS_SHA256);
    out.extend_from_slice(&d96::NONZERO_OVERRIDE_OBSERVATION_SHA256);
    assert_eq!(
        out.len(),
        d96::HEADER_LEN,
        "the fixture must build a real header"
    );
    out
}

fn ptc1_header_declaring(row_count: u32) -> Vec<u8> {
    use wire_format::d92;
    let mut out = Vec::with_capacity(d92::HEADER_LEN);
    out.extend_from_slice(&0u64.to_le_bytes());
    out.extend_from_slice(&row_count.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&d92::MAGIC);
    out.extend_from_slice(&d92::VERSION.to_le_bytes());
    out.extend_from_slice(&(d92::HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&1u64.to_le_bytes());
    out.extend_from_slice(&d92::EXECUTABLE_SHA256);
    out.extend_from_slice(&d92::CONSUMER_EVIDENCE_SHA256);
    out.extend_from_slice(&d92::WRITER_EVIDENCE_SHA256);
    out.extend_from_slice(&d92::IDENTITY_EVIDENCE_SHA256);
    assert_eq!(
        out.len(),
        d92::HEADER_LEN,
        "the fixture must build a real header"
    );
    out
}

fn fow1_payload(pov_country_id: u32, levels: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(compact_contract::pov_visibility::HEADER_LEN + levels.len());
    out.extend_from_slice(&compact_contract::pov_visibility::CARRIER_MAGIC);
    out.extend_from_slice(&compact_contract::pov_visibility::CARRIER_VERSION.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u64.to_le_bytes());
    out.extend_from_slice(&pov_country_id.to_le_bytes());
    out.extend_from_slice(&(levels.len() as u32).to_le_bytes());
    out.extend_from_slice(levels);
    out
}

#[test]
fn the_shared_admission_bounds_hold_their_exact_documented_values() {
    assert_eq!(portable_admission::DATE_EPOCH_CONST, 43_799_999);
    assert_eq!(portable_admission::RAW_DATE_MIN, 60_005_999);
    assert_eq!(portable_admission::RAW_DATE_MAX, 63_071_999);
    assert_eq!(
        portable_admission::MAX_STRUCTURAL_TAG_DB_SIZE,
        2_147_483_647
    );
    assert_eq!(
        portable_admission::MAX_COMPACT_OWNER_TAG_DB_ID,
        2_147_483_646
    );
    assert_eq!(portable_admission::MAX_COUNTRY_TABLE_ROWS, 2_147_483_647);
    assert_eq!(
        portable_admission::MAX_STRUCTURAL_PROVINCE_VECTOR_SIZE,
        2_147_483_647
    );
    assert_eq!(portable_admission::SUPPLY_MAX_PROVINCE_ID, 2_147_483_646);
    assert_eq!(
        portable_admission::MAX_STRUCTURAL_STATE_TABLE_SIZE,
        2_147_483_647
    );
    assert_eq!(portable_admission::MAX_STRUCTURAL_STATE_ID, 2_147_483_646);
    assert_eq!(
        portable_admission::MAX_STRUCTURAL_COUNTRY_LINK_INDEX,
        2_147_483_646
    );
    assert_eq!(
        portable_admission::MAX_DELAYED_EVENT_LIFECYCLE_TAG_DB_ID,
        2_147_483_646
    );
    assert_eq!(
        portable_admission::MAX_DELAYED_EVENT_LIFECYCLE_EVENTS,
        65_536
    );
    assert_eq!(portable_admission::MAX_FRD_DIRECT_TRANSITIONS, 1_000_000);
}

#[test]
fn the_supply_province_ceiling_is_the_shared_province_envelope() {
    assert_eq!(
        portable_admission::SUPPLY_MAX_PROVINCE_ID,
        compact_contract::build_profile::MAX_PROVINCE_ID,
        "one build cannot have two largest province ids"
    );
    assert_eq!(
        compact_contract::build_profile::MAX_PROVINCE_ID,
        2_147_483_646
    );
}

#[test]
fn every_province_and_state_consumer_applies_the_shared_envelope() {
    for (name, value) in [
        (
            "wire_format::d34::MAX_PROVINCE_ID",
            wire_format::d34::MAX_PROVINCE_ID,
        ),
        (
            "wire_format::d48::MAX_PROVINCE_ID",
            wire_format::d48::MAX_PROVINCE_ID,
        ),
        (
            "wire_format::d54::MAX_PROVINCE_ID",
            wire_format::d54::MAX_PROVINCE_ID,
        ),
        (
            "wire_format::d56::MAX_PROVINCE_ID",
            wire_format::d56::MAX_PROVINCE_ID,
        ),
        (
            "wire_format::d62::MAX_PROVINCE_ID",
            wire_format::d62::MAX_PROVINCE_ID,
        ),
        (
            "compact_contract::build_profile::MAX_PROVINCE_ID",
            compact_contract::build_profile::MAX_PROVINCE_ID,
        ),
    ] {
        assert_eq!(
            value,
            portable_admission::SUPPLY_MAX_PROVINCE_ID,
            "{name} does not apply the shared province envelope. A consumer \
             narrower than the producer discards rows the recorder collected, \
             and nothing on the producer side reports it."
        );
    }
    for (name, value) in [
        (
            "wire_format::d48::MAX_STATE_ID",
            wire_format::d48::MAX_STATE_ID,
        ),
        (
            "wire_format::d51::MAX_STATE_ID",
            wire_format::d51::MAX_STATE_ID,
        ),
        (
            "wire_format::d52::MAX_STATE_ID",
            wire_format::d52::MAX_STATE_ID,
        ),
        (
            "wire_format::d54::MAX_STATE_ID",
            wire_format::d54::MAX_STATE_ID,
        ),
        (
            "wire_format::d62::MAX_STATE_ID",
            wire_format::d62::MAX_STATE_ID,
        ),
        (
            "compact_contract::build_profile::DOMAIN66_MAX_STATE_ID",
            compact_contract::build_profile::DOMAIN66_MAX_STATE_ID,
        ),
    ] {
        assert_eq!(
            value,
            compact_contract::build_profile::MAX_STATE_ID,
            "{name} does not apply the shared state envelope"
        );
    }
    assert_eq!(compact_contract::build_profile::MAX_STATE_ID, 2_147_483_646);
}

#[test]
fn the_shared_calendar_window_is_inclusive_at_both_ends() {
    assert_eq!(
        portable_admission::hour_index_from_raw(portable_admission::RAW_DATE_MIN - 1),
        None
    );
    assert!(portable_admission::hour_index_from_raw(portable_admission::RAW_DATE_MIN).is_some());
    assert!(portable_admission::hour_index_from_raw(portable_admission::RAW_DATE_MAX).is_some());
    assert_eq!(
        portable_admission::hour_index_from_raw(portable_admission::RAW_DATE_MAX + 1),
        None
    );
    assert_eq!(portable_admission::hour_index_from_raw(0), None);
    assert_eq!(portable_admission::hour_index_from_raw(i32::MIN), None);
    assert_eq!(portable_admission::hour_index_from_raw(i32::MAX), None);
}

#[test]
fn the_province_and_state_id_spaces_walk_their_boundary() {
    let province_max = portable_admission::SUPPLY_MAX_PROVINCE_ID;
    let state_max = portable_admission::MAX_STRUCTURAL_STATE_ID;

    for admitted in [
        1u32, 13_413, 13_414, 65_535, 65_536, 65_537, 70_000, 70_001, 1_000_000, 1_000_001,
    ] {
        assert!(
            admitted <= province_max,
            "province id {admitted} must be admitted: it sits below the id \
             space's structural width and every ceiling that refused it was a \
             guess"
        );
        assert!(
            admitted <= state_max,
            "state id {admitted} must be admitted for the same reason"
        );
    }

    assert_eq!(province_max, 2_147_483_646);
    assert_eq!(state_max, 2_147_483_646);
    assert_eq!(
        province_max + 1,
        i32::MAX as u32,
        "one past the ceiling must be exactly the first unaddressable word"
    );
    assert_eq!(state_max + 1, i32::MAX as u32);

    assert_eq!(
        portable_admission::MAX_STRUCTURAL_PROVINCE_VECTOR_SIZE,
        province_max + 1
    );
    assert_eq!(
        portable_admission::MAX_STRUCTURAL_STATE_TABLE_SIZE,
        state_max + 1
    );

    for province_id in [1u32, 13_414, 65_536, 1_000_001, province_max] {
        let row = d34_row_with_province(province_id);
        assert!(
            wire_format::d34::parse_row(&row).is_ok(),
            "d34::parse_row must admit province {province_id}"
        );
    }
    for province_id in [province_max + 1, u32::MAX] {
        let row = d34_row_with_province(province_id);
        assert!(
            wire_format::d34::parse_row(&row).is_err(),
            "d34::parse_row must refuse province {province_id}"
        );
    }
}

fn d34_row_with_province(province_id: u32) -> Vec<u8> {
    let mut row = vec![0u8; wire_format::d34::ROW_LEN];
    row[0..4].copy_from_slice(&1u32.to_le_bytes());
    row[4..8].copy_from_slice(&1u32.to_le_bytes());
    row[28..32].copy_from_slice(&province_id.to_le_bytes());
    row
}

#[test]
fn the_d94_province_slot_count_is_a_budget_not_the_id_space() {
    assert_eq!(compact_contract::pov_visibility::MAX_PROVINCE_SLOTS, 65_535);
    assert_eq!(
        wire_format::d94::MAX_PROVINCE_SLOTS,
        compact_contract::pov_visibility::MAX_PROVINCE_SLOTS,
        "the codec and the contract must budget the same vector"
    );
    assert!(
        (compact_contract::pov_visibility::MAX_PROVINCE_SLOTS as u64)
            < u64::from(compact_contract::build_profile::MAX_PROVINCE_ID),
        "a dense per-province buffer budget must stay far below the id space's \
         structural width -- it sizes an allocation from a stream word"
    );
    assert_eq!(
        compact_contract::pov_visibility::MAX_PROVINCE_SLOT_INDEX as usize,
        compact_contract::pov_visibility::MAX_PROVINCE_SLOTS - 1
    );
    assert_eq!(
        wire_format::d94::MAX_PROVINCE_SLOT_INDEX,
        compact_contract::pov_visibility::MAX_PROVINCE_SLOT_INDEX
    );
}

#[test]
fn d67_partial_trailer_producer_and_reader_agree_for_every_cause() {
    use compact_contract::dynamic_modifiers as producer;
    use wire_format::d67 as reader;
    for cause in 0..18 {
        let frame = producer::DynamicModifierFrame {
            elapsed_wall_ms: 7, status_flags: compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            authority: producer::DynamicModifierAuthority::exact_a729(),
            instances: vec![producer::DynamicModifierInstance {
                owner_tag_db_id: 1, modifier_id: "complete".into(), enabled: true,
                values_raw: vec![i64::MIN, 0, i64::MAX], remaining_days: Some(3),
            }],
            zero_removals: vec![],
            partiality: vec![producer::CountryPartiality {
                owner_tag_db_id: 2, causes: 1 << cause, first_tick: 100, last_tick: 101,
            }],
        };
        let bytes = producer::encode_payload(&frame).unwrap();
        let decoded = reader::decode_payload(&bytes, true).unwrap();
        assert_eq!(decoded.instances[0].values_raw, frame.instances[0].values_raw);
        assert_eq!(decoded.partiality[0].owner_tag_db_id, 2);
        assert_eq!(decoded.partiality[0].causes, 1 << cause);
        assert_eq!((decoded.partiality[0].first_tick, decoded.partiality[0].last_tick), (100, 101));
        let mut malformed = bytes.clone();
        let start = malformed.len() - producer::PARTIALITY_ROW_LEN;
        malformed[start + 4..start + 8].copy_from_slice(&(1u32 << 18).to_le_bytes());
        assert!(producer::decode_payload(&malformed).is_err());
        assert!(reader::decode_payload(&malformed, true).is_err());
    }
}
