use super::*;
use compact_contract::domain_gap::{DomainGapReason, DomainGapRecordV1};

const HOUR: i32 = 60_832_037;

fn bracket(tag: u16, opening: i32, closing: i32) -> DomainClockBracket {
    DomainClockBracket {
        tag,
        opening: Some(ClockObservation { hour: opening, ordinal: 1, at_us: 1 }),
        closing: Some(ClockObservation { hour: closing, ordinal: 2, at_us: 2 }),
        worker_panicked: false,
    }
}

fn record() -> Vec<u8> {
    let mut records = Vec::new();
    append_compact_record(
        &mut records, 1, HOUR, COMPACT_DOMAIN_DIVISIONS,
        COMPACT_RECORD_TYPE_DOMAIN_DATA, 0, &[7; 8],
    );
    records
}

#[test]
fn rejected_root_acquisition_replaces_clean_consumers_actual_record() {
    for (source, expected) in [
        (Some(bracket(28, HOUR, HOUR + 1)), DomainGapReason::Torn),
        (Some(bracket(28, HOUR + 1, HOUR + 1)), DomainGapReason::HourLabelMismatch),
        (None, DomainGapReason::ClockUnreadable),
    ] {
        let mut brackets = vec![bracket(COMPACT_DOMAIN_DIVISIONS, HOUR, HOUR)];
        brackets.extend(source);
        let mut records = record();
        let (_, rejected) = replace_rejected_domain_records_with_root_dependencies(
            &mut records, 1, HOUR, &brackets, &mut BTreeMap::new(),
            HourLabelMismatchPolicy::Refuse, &[(COMPACT_DOMAIN_DIVISIONS, Some(28))],
        ).unwrap();
        assert!(rejected.contains(&COMPACT_DOMAIN_DIVISIONS));
        let header = compact_contract::parse_compact_record_header(&records).unwrap();
        assert_eq!(header.record_type, compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP);
        let gap = DomainGapRecordV1::decode(
            &records[compact_contract::COMPACT_RECORD_HEADER_LEN..header.record_len()],
        ).unwrap();
        assert_eq!(gap.reason, expected);
    }
}

#[test]
fn clean_acquisition_preserves_bytes_and_unrelated_failed_reader_does_not_taint_roots() {
    let mut records = record();
    let original = records.clone();
    let (_, rejected) = replace_rejected_domain_records_with_root_dependencies(
        &mut records, 1, HOUR,
        &[bracket(20, HOUR, HOUR), bracket(28, HOUR, HOUR), bracket(29, HOUR, HOUR + 1)],
        &mut BTreeMap::new(), HourLabelMismatchPolicy::Refuse, &[(20, Some(28))],
    ).unwrap();
    assert!(!rejected.contains(&20));
    assert_eq!(records, original);
}

#[test]
fn root_dependency_uses_first_acquirer_and_each_actual_consumer() {
    let acquisition = RootAcquisition::default();
    let saved = ACTIVE_READER_BRACKET_DOMAIN.with(|slot| slot.replace(Some(29)));
    let first = acquisition.consumer();
    acquisition.begin(first);
    ACTIVE_READER_BRACKET_DOMAIN.with(|slot| slot.set(Some(28)));
    assert_eq!(acquisition.consumer(), Some(28));
    ACTIVE_READER_BRACKET_DOMAIN.with(|slot| slot.set(saved));
    assert_eq!(acquisition.dependencies(), vec![(28, Some(29)), (29, Some(29))]);
}

#[test]
fn unnamed_acquisition_is_not_manufactured_from_later_consumer() {
    let acquisition = RootAcquisition::default();
    acquisition.begin(None);
    let saved = ACTIVE_READER_BRACKET_DOMAIN.with(|slot| slot.replace(Some(20)));
    acquisition.consumer();
    ACTIVE_READER_BRACKET_DOMAIN.with(|slot| slot.set(saved));
    let dependencies = acquisition.dependencies();
    assert_eq!(dependencies, vec![(20, None)]);
    let mut records = record();
    let (_, rejected) = replace_rejected_domain_records_with_root_dependencies(
        &mut records, 1, HOUR, &[bracket(20, HOUR, HOUR)],
        &mut BTreeMap::new(), HourLabelMismatchPolicy::Refuse, &dependencies,
    ).unwrap();
    assert!(rejected.contains(&20));
    let header = compact_contract::parse_compact_record_header(&records).unwrap();
    let gap = DomainGapRecordV1::decode(
        &records[compact_contract::COMPACT_RECORD_HEADER_LEN..header.record_len()],
    ).unwrap();
    assert_eq!(gap.reason, DomainGapReason::ClockUnreadable);
}

#[test]
fn cached_army_consumer_retains_direct_division_acquisition_dependency() {
    let state = super::tests::empty_collector_state();
    let divisions = CompactFrameDivisionRootsCache::new(&state, true);
    let armies = CompactFrameArmyRootsCache::new(state.base, &divisions);
    assert!(divisions.roots.get().is_none());
    assert!(armies.roots.get().is_none());

    let saved = ACTIVE_READER_BRACKET_DOMAIN.with(|slot| slot.replace(Some(26)));
    assert!(divisions.get().rows.is_empty());
    ACTIVE_READER_BRACKET_DOMAIN.with(|slot| slot.set(Some(28)));
    assert!(armies.get().armies.is_empty());
    let first_armies = armies.roots.get().unwrap() as *const CompactArmyOrderRoots;
    ACTIVE_READER_BRACKET_DOMAIN.with(|slot| slot.set(Some(20)));
    assert!(std::ptr::eq(armies.get(), first_armies));
    ACTIVE_READER_BRACKET_DOMAIN.with(|slot| slot.set(saved));

    assert_eq!(divisions.acquisition.dependencies(),
        vec![(20, Some(26)), (26, Some(26)), (28, Some(26))]);
    assert_eq!(armies.acquisition.dependencies(), vec![(20, Some(28)), (28, Some(28))]);
    let mut dependencies = divisions.acquisition.dependencies();
    dependencies.extend(armies.acquisition.dependencies());
    let mut records = record();
    let (_, rejected) = replace_rejected_domain_records_with_root_dependencies(
        &mut records, 1, HOUR,
        &[bracket(20, HOUR, HOUR), bracket(28, HOUR, HOUR), bracket(26, HOUR, HOUR + 1)],
        &mut BTreeMap::new(), HourLabelMismatchPolicy::Refuse, &dependencies,
    ).unwrap();
    assert!(rejected.contains(&20));
    let header = compact_contract::parse_compact_record_header(&records).unwrap();
    let gap = DomainGapRecordV1::decode(
        &records[compact_contract::COMPACT_RECORD_HEADER_LEN..header.record_len()],
    ).unwrap();
    assert_eq!(gap.reason, DomainGapReason::Torn);
}

#[test]
fn grouped_real_country_vectors_keep_order_empty_and_shared_exact_read_budget() {
    use compact_contract::country_roster::{Country, Kind, Observation};
    fn u32_at(words: &mut [u64], at: usize, value: u32) {
        let shift = (at % 8) * 8;
        words[at / 8] = (words[at / 8] & !(u64::from(u32::MAX) << shift)) | (u64::from(value) << shift);
    }
    let base = 0x1_4000_0000usize;
    let mut country = vec![0u64; 0x400 / 8];
    let mut game = vec![0u64; 0x800 / 8];
    let mut division_a = vec![0u64; 0x80 / 8];
    let mut division_b = vec![0u64; 0x80 / 8];
    let mut fleet_a = vec![0u64; 0x80 / 8];
    let mut fleet_b = vec![0u64; 0x80 / 8];
    for (words, id) in [(&mut division_a, 9), (&mut division_b, 2)] {
        words[0] = (base + DIVISION_WRAPPER_VTABLE_RVA) as u64;
        words[DIVISION_WRAPPER_TO_BODY / 8] = (base + DIVISION_VTABLE_RVA) as u64;
        u32_at(words, DIVISION_WRAPPER_TO_BODY + DIVISION_OFF_ID_TYPE, DIVISION_TYPE);
        u32_at(words, DIVISION_WRAPPER_TO_BODY + DIVISION_OFF_ID_ID, id);
    }
    for (words, id) in [(&mut fleet_a, 7), (&mut fleet_b, 3)] {
        words[0] = (base + FLEET_VTABLE_RVA) as u64;
        u32_at(words, 8, OOB_TYPE_FLEET);
        u32_at(words, 12, id);
    }
    let mut division_vector = vec![division_a.as_ptr() as u64, division_b.as_ptr() as u64];
    let mut fleet_vector = vec![fleet_a.as_ptr() as u64, fleet_b.as_ptr() as u64];
    country[0] = (base + COUNTRY_VTABLE_RVA) as u64;
    u32_at(&mut country, OBJ_TAG_DB_ID, 7);
    country[COUNTRY_DIVISION_WRAPPER_DATA / 8] = division_vector.as_ptr() as u64;
    u32_at(&mut country, COUNTRY_DIVISION_WRAPPER_DATA + 8, 2);
    u32_at(&mut country, COUNTRY_DIVISION_WRAPPER_DATA + 12, 2);
    country[COUNTRY_FLEET_VECTOR_DATA / 8] = fleet_vector.as_ptr() as u64;
    u32_at(&mut country, COUNTRY_FLEET_VECTOR_CAPACITY, 2);
    u32_at(&mut country, COUNTRY_FLEET_VECTOR_COUNT, 2);
    let table = vec![country.as_ptr() as u64];
    game[CGAMESTATE_COUNTRY_TABLE / 8] = table.as_ptr() as u64;
    u32_at(&mut game, CGAMESTATE_COUNTRY_COUNT, 1);
    let mut state = super::tests::empty_collector_state();
    state.base = base; state.daily_match = true; state.long_string_match = true;
    state.date_ptr = game.as_ptr() as u64;
    let saved = ACTIVE_READER_BRACKET_DOMAIN.with(|slot| slot.replace(Some(28)));
    let ids = |value: &Observation| value.countries[0].slots.iter()
        .map(|slot| slot.reference.unwrap().id).collect::<Vec<_>>();
    for reverse in [false, true] {
        if reverse { division_vector.swap(0, 1); fleet_vector.swap(0, 1); }
        page_cache_new_frame();
        let budget = country_roster_readset::Budget::default();
        let divisions = discover_compact_division_roots_from_countries_budgeted(&state, &budget, false).unwrap();
        let (fleets, roster) = discover_compact_fleet_roots_budgeted(&state, false, &budget).unwrap();
        let division_roster = divisions.country_roster.as_ref().unwrap();
        assert!(division_roster.countries.iter().all(Country::complete));
        assert!(roster.countries.iter().all(Country::complete));
        assert_eq!(ids(division_roster), if reverse { vec![2,9] } else { vec![9,2] });
        assert_eq!(ids(&roster), if reverse { vec![3,7] } else { vec![7,3] });
        assert_eq!(divisions.rows.iter().map(|row|row.div_id).collect::<Vec<_>>(), [2,9]);
        assert_eq!(fleets.iter().map(|row|row.fleet_id).collect::<Vec<_>>(), [3,7]);
        assert_eq!(budget.verification_bytes(), 164);
        assert_eq!(Observation::parse(&roster.encode().unwrap(), Kind::Fleet).unwrap(), roster);
    }
    u32_at(&mut country, COUNTRY_DIVISION_WRAPPER_DATA + 12, 0);
    u32_at(&mut country, COUNTRY_FLEET_VECTOR_COUNT, 0);
    page_cache_new_frame();
    let budget = country_roster_readset::Budget::default();
    let divisions = discover_compact_division_roots_from_countries_budgeted(&state, &budget, false).unwrap();
    let (_, fleet) = discover_compact_fleet_roots_budgeted(&state, false, &budget).unwrap();
    assert!(divisions.country_roster.unwrap().countries[0].complete());
    assert!(fleet.countries[0].complete());
    assert_eq!(fleet.countries[0].observed_count, Some(0));
    assert_eq!(budget.verification_bytes(), 68);
    ACTIVE_READER_BRACKET_DOMAIN.with(|slot| slot.set(saved));
}
