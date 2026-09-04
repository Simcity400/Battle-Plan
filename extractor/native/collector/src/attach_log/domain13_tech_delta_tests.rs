use super::*;

fn frozen_tech_data_payload(
    rows: &[CountryObjectRow],
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Vec<u8> {
    let mut country_count = 0usize;
    let mut active_count = 0usize;
    for row in rows.iter().filter(|row| row.research.is_some()) {
        country_count += 1;
        active_count += row
            .research
            .as_ref()
            .and_then(|research| research.active_research.as_ref())
            .map(Vec::len)
            .unwrap_or(0);
    }
    let mut payload = Vec::new();
    let packed_country_count = country_count.min(u32::MAX as usize) as u32;
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&packed_country_count.to_le_bytes());
    payload.extend_from_slice(&packed_country_count.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&TECH_PROGRESS_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&TECH_LIVE_PROGRESS_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&(active_count.min(u32::MAX as usize) as u32).to_le_bytes());
    for row in rows.iter().filter(|row| row.research.is_some()) {
        let research = row.research.as_ref().expect("filtered research row");
        let mut flags = 0u32;
        if research.completed_tech_count.is_some() {
            flags |= 1;
        }
        if research.research_slot_count.is_some() {
            flags |= 1 << 1;
        }
        if research.active_research.is_some() {
            flags |= 1 << 2;
        }
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(
            &research
                .completed_tech_count
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
        payload.extend_from_slice(
            &research
                .research_slot_count
                .unwrap_or(u32::MAX)
                .to_le_bytes(),
        );
    }
    for row in rows.iter().filter(|row| row.research.is_some()) {
        let research = row.research.as_ref().expect("filtered research row");
        if let Some(active) = research.active_research.as_ref() {
            for slot in active {
                let mut flags = 0u32;
                if slot.live_progress_raw.is_some() {
                    flags |= 1;
                }
                if slot.base_required_points_raw.is_some() {
                    flags |= 1 << 1;
                }
                if slot.tech_name.as_ref().is_some_and(|name| !name.is_empty()) {
                    flags |= 1 << 2;
                }
                payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
                payload.extend_from_slice(&slot.slot_index.to_le_bytes());
                payload.extend_from_slice(&slot.tech_handle_raw.to_le_bytes());
                payload.extend_from_slice(&slot.tech_def_index_raw.to_le_bytes());
                payload.extend_from_slice(&slot.progress_raw.to_le_bytes());
                payload.extend_from_slice(&flags.to_le_bytes());
                payload.extend_from_slice(&slot.live_progress_raw.unwrap_or(0).to_le_bytes());
                payload
                    .extend_from_slice(&slot.base_required_points_raw.unwrap_or(0).to_le_bytes());
                if flags & (1 << 2) != 0 {
                    append_compact_string(&mut payload, slot.tech_name.as_deref());
                }
            }
        }
    }
    payload
}

fn research_slot(slot_index: u32, progress_raw: u32, named: bool) -> CountryResearchSlotRow {
    CountryResearchSlotRow {
        slot_index,
        tech_handle_raw: 1000 + slot_index,
        tech_def_index_raw: 2000 + slot_index,
        progress_raw,
        tech_name: named.then(|| format!("tech_{slot_index}")),
        progress_units: progress_raw as f64 / TECH_PROGRESS_SCALE_RAW as f64,
        live_progress_raw: Some(progress_raw as i64 * 2),
        live_progress_units: Some(0.0),
        base_required_points_raw: Some(50_000),
        base_required_points_units: Some(0.0),
        ahead_of_time_years_raw: None,
        ahead_of_time_years: None,
        ahead_of_time_penalty_raw: None,
        ahead_of_time_penalty_pct: None,
        ahead_of_time_reduction_years_raw: None,
        ahead_of_time_reduction_years: None,
        ahead_of_time_reduced_penalty_raw: None,
        ahead_of_time_reduced_penalty_pct: None,
        ahead_of_time_provenance: None,
        ahead_of_time_derived: None,
    }
}

pub(crate) fn bare_country_row(tag_db_id: u32) -> CountryObjectRow {
    CountryObjectRow {
        tag: format!("C{tag_db_id}"),
        tag_db_id,
        name: None,
        object_ptr: 0x1000,
        record_ptr: None,
        original_capital: None,
        original_tag: None,
        templates_locked: None,
        reserved_dynamic_country: None,
        stability: None,
        war_support: None,
        political_power: None,
        command_power: None,
        num_ships: None,
        manpower_pool: None,
        convoys: None,
        misc: None,
        fuel: None,
        production: None,
        focus: None,
        research: None,
        politics: None,
        politics_residuals: None,
        decisions: None,
        diplomacy: None,
    }
}

fn tech_witness(seed: u32) -> Result<Arc<CountryTechnologyWitness>, Xui09RefusalReason> {
    if seed % 7 == 0 {
        return Err(Xui09RefusalReason::ClassifierEnumOutOfRange);
    }
    let count = 1 + seed % 5;
    let states: Vec<(u32, u8)> = (0..count)
        .map(|index| (index, 1 + ((seed + index) % 6) as u8))
        .collect();
    let completed: Vec<u32> = states
        .iter()
        .filter(|(_, state)| *state == 4)
        .map(|(def_index, _)| *def_index)
        .collect();
    Ok(Arc::new(CountryTechnologyWitness { states, completed }))
}

fn tech_witness_completed_count(seed: u32) -> Option<u32> {
    match tech_witness(seed) {
        Ok(witness) => Some(witness.completed.len() as u32),
        Err(_) => (seed % 5 != 0).then_some(seed),
    }
}

fn test_technology_dictionary() -> TechnologyIdDictionary {
    TechnologyIdDictionary {
        by_definition_index: (0..8u32)
            .map(|index| (index, (0usize, format!("tech_{index}"))))
            .collect(),
    }
}

fn tech_row(tag_db_id: u32, seed: u32) -> CountryObjectRow {
    let mut row = bare_country_row(tag_db_id);
    row.research = Some(CountryResearchRow {
        completed_tech_count: tech_witness_completed_count(seed),
        research_slot_count: Some(1 + seed % 3),
        active_research: (seed % 4 != 0).then(|| {
            (0..(seed % 3))
                .map(|index| research_slot(index, seed * 17 + index, seed % 2 == 0))
                .collect()
        }),
        technology_witness: tech_witness(seed),
    });
    row
}

fn researchless_row(tag_db_id: u32) -> CountryObjectRow {
    bare_country_row(tag_db_id)
}

#[derive(Debug, PartialEq, Eq, Clone)]
struct DecodedBlock {
    country_row: Vec<u8>,
    active_rows: Vec<Vec<u8>>,
}

fn decode_tech_payload(payload: &[u8]) -> BTreeMap<u32, DecodedBlock> {
    let header = d13_grammar::read_header(payload).expect("a production payload has a header");
    let layout =
        d13_grammar::body_layout(payload, &header).expect("a production payload meets its minimum");

    let mut live: BTreeMap<u32, DecodedBlock> = BTreeMap::new();
    let mut cursor = layout.countries.start;
    for _ in 0..header.country_count {
        let row = d13_grammar::read_country_row(payload, cursor).expect("a country row");
        assert_eq!(
            row.unknown_flags(),
            0,
            "country {} carries a flag no consumer accepts",
            row.tag_db_id
        );
        assert!(
            live.insert(
                row.tag_db_id,
                DecodedBlock {
                    country_row: payload[row.span.clone()].to_vec(),
                    active_rows: Vec::new(),
                },
            )
            .is_none(),
            "a payload carries country {} twice",
            row.tag_db_id
        );
        cursor = row.span.end;
    }
    for _ in 0..header.active_count {
        let start = cursor;
        let (mut row, unknown) =
            d13_grammar::ActiveRowCursor::begin(payload, cursor).expect("an active row");
        assert_eq!(unknown, None, "an active row carries an unassigned flag");
        let tag = row.fixed().tag_db_id;
        cursor = loop {
            match row.next_step().expect("an active row is well formed") {
                d13_grammar::ActiveStep::Section(_) => {}
                d13_grammar::ActiveStep::Recoverable(violation) => {
                    panic!("an active row breaks a combination rule: {violation:?}")
                }
                d13_grammar::ActiveStep::End(end) => break end,
            }
        };
        live.get_mut(&tag)
            .unwrap_or_else(|| {
                panic!("active row references country {tag}, absent from the header")
            })
            .active_rows
            .push(payload[start..cursor].to_vec());
    }
    cursor = skip_xui09_tail(payload, cursor);
    assert_eq!(cursor, payload.len(), "the payload has trailing bytes");
    live
}

fn skip_xui09_tail(payload: &[u8], cursor: usize) -> usize {
    let Some((mut tail, unknown)) =
        d13_grammar::Xui09Cursor::begin(payload, cursor).expect("a well-formed tail or none")
    else {
        return cursor;
    };
    assert_eq!(unknown, None, "the XUI09 tail carries an unassigned flag");
    loop {
        match tail.next_step().expect("a well-formed tail") {
            d13_grammar::Xui09Step::DictionaryEntry { id_bytes, .. } => {
                assert!(
                    !id_bytes.is_empty(),
                    "an XUI09 dictionary entry carries an empty id"
                );
            }
            d13_grammar::Xui09Step::MeasuredTag { .. }
            | d13_grammar::Xui09Step::Refusal { .. }
            | d13_grammar::Xui09Step::CountryVectorHeader { .. }
            | d13_grammar::Xui09Step::CountryVectorRows { .. } => {}
            d13_grammar::Xui09Step::Recoverable(violation) => {
                panic!("an XUI09 country vector carries an unassigned flag: {violation:?}")
            }
            d13_grammar::Xui09Step::End(end) => return end,
        }
    }
}

#[test]
fn the_block_serialiser_is_byte_identical_to_the_payload_builder_it_replaced() {
    for frame in 0..40u32 {
        let rows: Vec<CountryObjectRow> = (1..=9u32)
            .map(|tag| {
                if tag % 4 == 0 {
                    researchless_row(tag)
                } else {
                    tech_row(tag, frame * 7 + tag)
                }
            })
            .collect();
        let status = if frame % 3 == 0 { 0 } else { 1 << 20 };
        let produced =
            compact_tech_data_payload(&rows, frame as u64, status, &test_technology_dictionary());
        let frozen = frozen_tech_data_payload(&rows, frame as u64, status);
        assert_eq!(
            &produced[..frozen.len()],
            &frozen[..],
            "frame {frame}: the block serialiser and the builder it replaced disagree"
        );
        assert_eq!(
            skip_xui09_tail(&produced, frozen.len()),
            produced.len(),
            "frame {frame}: the bytes after the frozen sections are not exactly an XUI09 tail"
        );
    }
}

#[derive(Default)]
struct ReferenceTechFold {
    world: BTreeMap<u32, DecodedBlock>,
}

impl ReferenceTechFold {
    fn apply(&mut self, payload: &[u8]) -> BTreeMap<u32, DecodedBlock> {
        for (tag, block) in decode_tech_payload(payload) {
            self.world.insert(tag, block);
        }
        self.world.clone()
    }
}

#[test]
fn a_patch_folds_to_the_same_world_as_the_whole_payload_every_frame() {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let mut patched_fold = ReferenceTechFold::default();
    let mut whole_fold = ReferenceTechFold::default();
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    let mut patches = 0usize;
    for frame in 0..300u32 {
        let mut rows = Vec::new();
        for tag in 1..=12u32 {
            let roll = next() % 100;
            if roll < 15 {
                rows.push(bare_country_row(tag));
                continue;
            }
            let seed = if roll < 60 { tag } else { tag + frame };
            rows.push(tech_row(tag, seed));
        }
        let patched = compact_tech_data_payload_delta(
            &rows,
            frame as u64,
            0,
            &mut tracker,
            &test_technology_dictionary(),
        );
        let whole =
            compact_tech_data_payload(&rows, frame as u64, 0, &test_technology_dictionary());
        if u32::from_le_bytes(patched[16..20].try_into().unwrap()) & COMPACT_STATUS_DELTA_DATA != 0
        {
            patches += 1;
        }

        assert_eq!(
            patched_fold.apply(&patched),
            whole_fold.apply(&whole),
            "frame {frame}: the patch stream folds to a different world than the whole payload"
        );
    }
    assert!(
        patches > 200,
        "only {patches} of 300 frames were patches, so this gate barely exercised the patch \
         path it exists for"
    );
}

/// release one.
#[test]
fn an_unmeasured_country_is_neither_emitted_nor_removed() {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let seeded = compact_tech_data_payload_delta(
        &[tech_row(1, 1), tech_row(2, 2), tech_row(3, 3)],
        0,
        0,
        &mut tracker,
        &test_technology_dictionary(),
    );
    assert_eq!(
        decode_tech_payload(&seeded).len(),
        3,
        "the first payload seeds the pin whole"
    );

    let patched = compact_tech_data_payload_delta(
        &[tech_row(1, 1), bare_country_row(2), tech_row(3, 3)],
        1,
        0,
        &mut tracker,
        &test_technology_dictionary(),
    );
    assert_ne!(
        u32::from_le_bytes(patched[16..20].try_into().unwrap()) & COMPACT_STATUS_DELTA_DATA,
        0,
        "an unmeasured country must not cost the whole payload"
    );
    assert!(
        decode_tech_payload(&patched).is_empty(),
        "nothing changed on the wire, so the patch must be empty: {:?}",
        decode_tech_payload(&patched)
    );

    let again = compact_tech_data_payload_delta(
        &[tech_row(1, 1), tech_row(2, 2), tech_row(3, 3)],
        2,
        0,
        &mut tracker,
        &test_technology_dictionary(),
    );
    assert!(
        decode_tech_payload(&again).is_empty(),
        "the pin lost the unmeasured country, so it was re-sent when it reappeared"
    );
}

#[test]
fn a_key_leaving_the_roster_forces_a_whole_payload_and_re_seeds() {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let _ = compact_tech_data_payload_delta(
        &[tech_row(1, 1), tech_row(2, 2), tech_row(3, 3)],
        0,
        0,
        &mut tracker,
        &test_technology_dictionary(),
    );

    let payload = compact_tech_data_payload_delta(
        &[tech_row(1, 1), tech_row(2, 2)],
        1,
        0,
        &mut tracker,
        &test_technology_dictionary(),
    );
    assert_eq!(
        u32::from_le_bytes(payload[16..20].try_into().unwrap()) & COMPACT_STATUS_DELTA_DATA,
        0,
        "a departed key must force a whole payload, not a patch"
    );
    assert_eq!(
        decode_tech_payload(&payload).len(),
        2,
        "the whole payload carries what the frame measured"
    );
    assert!(!tracker.baseline_seeded, "the pin is dropped, not advanced");
}

#[test]
fn a_repeated_key_forces_a_whole_payload_instead_of_an_inconsistent_patch() {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let _ = compact_tech_data_payload_delta(
        &[tech_row(1, 1), tech_row(2, 2)],
        0,
        0,
        &mut tracker,
        &test_technology_dictionary(),
    );

    let payload = compact_tech_data_payload_delta(
        &[tech_row(1, 1), tech_row(2, 5), tech_row(2, 2)],
        1,
        0,
        &mut tracker,
        &test_technology_dictionary(),
    );
    assert_eq!(
        u32::from_le_bytes(payload[16..20].try_into().unwrap()) & COMPACT_STATUS_DELTA_DATA,
        0,
        "a repeated key must force a whole payload, not a patch whose pin and payload disagree"
    );
    assert!(!tracker.baseline_seeded, "the pin is dropped, not advanced");
}

#[test]
fn a_withheld_baseline_emits_whole_and_unseeds_the_pin() {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let _ = compact_tech_data_payload_delta(
        &[tech_row(1, 1), tech_row(2, 2)],
        0,
        0,
        &mut tracker,
        &test_technology_dictionary(),
    );
    assert!(
        tracker.baseline_seeded,
        "the first clean read seeds the pin"
    );

    let torn = compact_tech_data_payload_delta(
        &[tech_row(1, 1)],
        1,
        COMPACT_STATUS_PARTIAL_DATA,
        &mut tracker,
        &test_technology_dictionary(),
    );
    assert_eq!(
        decode_tech_payload(&torn).len(),
        1,
        "a withheld-baseline frame emits what it read, whole"
    );
    assert_eq!(
        u32::from_le_bytes(torn[16..20].try_into().unwrap()) & COMPACT_STATUS_DELTA_DATA,
        0,
        "a withheld-baseline frame is not a patch"
    );
    assert!(!tracker.baseline_seeded, "the pin is dropped, not advanced");

    let after = compact_tech_data_payload_delta(
        &[tech_row(1, 1), tech_row(2, 2)],
        2,
        0,
        &mut tracker,
        &test_technology_dictionary(),
    );
    assert_eq!(
        decode_tech_payload(&after).len(),
        2,
        "the frame after a withheld baseline is whole"
    );
    assert_eq!(
        u32::from_le_bytes(after[16..20].try_into().unwrap()) & COMPACT_STATUS_DELTA_DATA,
        0,
        "the re-seeding frame must not claim to be a patch"
    );
}

const D13_T26_COUNTRY_COUNT: u32 = 439;

const D13_BENCH_TRIALS: usize = 5;

/// `cargo test -p collector --release -- --ignored --nocapture d13_tech_delta_bench`.
#[test]
#[ignore = "instrument, not a gate; run with --ignored --nocapture"]
fn d13_tech_delta_bench() {
    let seed_rows: Vec<CountryObjectRow> = (1..=D13_T26_COUNTRY_COUNT)
        .map(|tag| tech_row(tag, tag))
        .collect();
    let frame_rows: Vec<CountryObjectRow> = (1..=D13_T26_COUNTRY_COUNT)
        .map(|tag| {
            if tag % 44 == 0 {
                tech_row(tag, tag + 1)
            } else {
                tech_row(tag, tag)
            }
        })
        .collect();

    let mut tracker = CompactTechCountryDeltaTracker::default();
    let _ = compact_tech_data_payload_delta(
        &seed_rows,
        0,
        0,
        &mut tracker,
        &test_technology_dictionary(),
    );

    let mut whole_best = Duration::from_secs(3600);
    let mut patch_best = Duration::from_secs(3600);
    let mut whole_bytes = 0usize;
    let mut patch_bytes = 0usize;
    for _ in 0..D13_BENCH_TRIALS {
        let started = Instant::now();
        let whole = compact_tech_data_payload(&frame_rows, 0, 0, &test_technology_dictionary());
        whole_best = whole_best.min(started.elapsed());
        whole_bytes = whole.len();

        let mut trial_tracker = CompactTechCountryDeltaTracker::default();
        let _ = compact_tech_data_payload_delta(
            &seed_rows,
            0,
            0,
            &mut trial_tracker,
            &test_technology_dictionary(),
        );
        let started = Instant::now();
        let patch = compact_tech_data_payload_delta(
            &frame_rows,
            0,
            0,
            &mut trial_tracker,
            &test_technology_dictionary(),
        );
        patch_best = patch_best.min(started.elapsed());
        patch_bytes = patch.len();
    }
    println!(
        "d13 rows={D13_T26_COUNTRY_COUNT} whole_us={:.3} whole_bytes={whole_bytes} \
         patch_us={:.3} patch_bytes={patch_bytes} added_us={:.3}",
        whole_best.as_secs_f64() * 1e6,
        patch_best.as_secs_f64() * 1e6,
        (patch_best.as_secs_f64() - whole_best.as_secs_f64()) * 1e6,
    );
}

#[test]
fn the_seed_is_whole_and_every_later_clean_frame_announces_itself_as_a_patch() {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let seed = compact_tech_data_payload_delta(
        &[tech_row(1, 1)],
        0,
        0,
        &mut tracker,
        &test_technology_dictionary(),
    );
    assert_eq!(
        u32::from_le_bytes(seed[16..20].try_into().unwrap()) & COMPACT_STATUS_DELTA_DATA,
        0,
        "the seeding keyframe must not carry DELTA_DATA"
    );
    for frame in 1..5u64 {
        let patch = compact_tech_data_payload_delta(
            &[tech_row(1, 1)],
            frame,
            0,
            &mut tracker,
            &test_technology_dictionary(),
        );
        assert_ne!(
            u32::from_le_bytes(patch[16..20].try_into().unwrap()) & COMPACT_STATUS_DELTA_DATA,
            0,
            "frame {frame} is a patch and must say so, or the fold cannot refuse one that \
             arrives before it has a baseline"
        );
    }
}

fn raw_hour(year: i64, day_of_year_zero_based: i64, hour: i64) -> i32 {
    i32::try_from((year + 5000) * 8760 + day_of_year_zero_based * 24 + hour)
        .expect("raw hour fits i32")
}

#[test]
fn derived_years_reproduce_the_observed_1944_engineer_company_iv_reading() {
    let date_raw = raw_hour(1944, 41, 18);
    let days = super::a29_days_ahead_of_time(1945, date_raw, 0).expect("days are derivable");
    assert_eq!(days, 324, "1945 authored against 1944.2.11.18");

    let years_raw = super::a29_years_from_days(days).expect("years are derivable");
    assert_eq!(years_raw, 88_767, "fixed-5, i.e. 0.88767 years");

    assert!(
        (88_000..89_000).contains(&years_raw),
        "derived {years_raw} must agree with the observed 0.88 at display precision"
    );
}

#[test]
fn derived_days_clamp_at_zero_and_round_the_reduction_half_away_from_zero() {
    let date_raw = raw_hour(1944, 0, 0);
    assert_eq!(
        super::a29_days_ahead_of_time(1936, date_raw, 0).expect("clamped"),
        0
    );

    let ahead = super::a29_days_ahead_of_time(1945, raw_hour(1944, 41, 18), 0).expect("base");
    let reduced =
        super::a29_days_ahead_of_time(1945, raw_hour(1944, 41, 18), 50_000).expect("reduced");
    assert_eq!(ahead - reduced, 183);
}

#[test]
fn saturating_penalty_refuses_instead_of_emitting_the_engine_sentinel() {
    assert_eq!(super::a29_penalty_from_days(324, i64::MAX), None);
    assert_eq!(super::a29_penalty_from_days(i32::MAX, 10_000_000_000), None);
    assert_eq!(
        super::a29_penalty_from_days(365, 100_000).expect("ordinary"),
        100_000
    );
}

#[test]
fn derived_penalty_scales_with_the_live_define() {
    let days = 365;
    assert_eq!(
        super::a29_penalty_from_days(days, 100_000).expect("unit"),
        100_000
    );
    assert_eq!(
        super::a29_penalty_from_days(days, 50_000).expect("half"),
        50_000
    );
    assert_eq!(super::a29_penalty_from_days(days, 0).expect("disabled"), 0);
}

#[test]
fn the_derivation_uses_the_live_factor_and_reports_the_one_it_used() {
    let date_raw = raw_hour(1944, 41, 18);
    let inputs = |factor| super::a29_ahead_values_from_inputs(1945, date_raw, 50_000, factor);

    let (unit_quartet, unit_factor) = inputs(100_000).expect("factor 1.0 derives");
    let (half_quartet, half_factor) = inputs(50_000).expect("factor 0.5 derives");

    assert_eq!(unit_factor, 100_000, "the tail names the factor used");
    assert_eq!(half_factor, 50_000, "and it is not the other one");

    assert_eq!(unit_quartet.0, half_quartet.0, "years do not scale");
    assert_eq!(unit_quartet.2, half_quartet.2, "nor does the reduction");
    let days = super::a29_days_ahead_of_time(1945, date_raw, 0).expect("days");
    let reduced_days = super::a29_days_ahead_of_time(1945, date_raw, 50_000).expect("reduced days");
    assert!(
        days > 0 && reduced_days > 0,
        "or the penalties are both zero and prove nothing"
    );
    for (quartet, factor) in [(unit_quartet, 100_000), (half_quartet, 50_000)] {
        assert_eq!(
            quartet.1,
            super::a29_penalty_from_days(days, factor).expect("penalty")
        );
        assert_eq!(
            quartet.3,
            super::a29_penalty_from_days(reduced_days, factor).expect("reduced penalty")
        );
    }
    assert_ne!(
        unit_quartet.1, half_quartet.1,
        "a derivation that ignored the factor would return the same penalty twice"
    );
    assert_ne!(unit_quartet.3, half_quartet.3);

    let (off_quartet, off_factor) = inputs(0).expect("a disabled penalty still derives");
    assert_eq!((off_quartet.1, off_quartet.3, off_factor), (0, 0, 0));
    assert_eq!(off_quartet.2, 50_000, "the reduction is not a penalty");

    assert_eq!(
        super::a29_ahead_values_from_inputs(1945, date_raw, -1, 100_000),
        None
    );
}
