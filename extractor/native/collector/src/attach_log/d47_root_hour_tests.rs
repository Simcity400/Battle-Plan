use super::*;

use domain47::adapter::{
    AdvisorRoot, CharacterRoot, CountryLeaderRoleRoot, TraitDefinitionRoot, UnitLeaderRoot,
};
use domain47::extractor::{
    ResolvedTypeRoots, CADVISOR_CHARACTER_PTR_OFF, CCHARACTER_COUNTRY_TAG_DB_ID_OFF,
    CCHARACTER_ID_OFF, CCHARACTER_TYPE_ID, CCHARACTER_TYPE_OFF, CCHARACTER_UNIT_LEADER_PTR_OFF,
    CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
};
use domain47::scheduled::{read_native_scheduled_frame, NativeFrameInput, NativeRootCache};

const CHAR_VT: u64 = 0x1000_1000;
const ARMY_VT: u64 = 0x1000_2000;
const NAVY_VT: u64 = 0x1000_3000;
const COUNTRY_LEADER_VT: u64 = 0x1000_4000;
const ADVISOR_VT: u64 = 0x1000_5000;
const TRAIT_VT: u64 = 0x1000_6000;

const STALE_NAME: &str = "Stale Hour Character";
const STALE_TOKEN: &str = "GER_stale_hour_character";
const STALE_IDEA: &str = "GER_stale_hour_advisor_idea";

#[derive(Default)]
struct FakeMemory {
    bytes: BTreeMap<u64, u8>,
}

impl FakeMemory {
    fn write_u32(&mut self, addr: u64, value: u32) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn write_u64(&mut self, addr: u64, value: u64) {
        self.write_bytes(addr, &value.to_le_bytes());
    }

    fn write_bytes(&mut self, addr: u64, bytes: &[u8]) {
        for (idx, byte) in bytes.iter().enumerate() {
            self.bytes.insert(addr + idx as u64, *byte);
        }
    }
}

impl domain47::extractor::LaneMemory for FakeMemory {
    fn read_u8(&self, addr: u64) -> Option<u8> {
        self.bytes.get(&addr).copied()
    }
}

fn stale_root_world() -> (FakeMemory, NativeRootCache) {
    let mut mem = FakeMemory::default();
    let character = 0x1000u64;
    let country_leader = 0x3000u64;
    let advisor = 0x5000u64;
    let trait_def = 0x2300u64;

    mem.write_u64(character, CHAR_VT);
    mem.write_u32(character + CCHARACTER_TYPE_OFF, CCHARACTER_TYPE_ID);
    mem.write_u32(character + CCHARACTER_ID_OFF, 1458);
    mem.write_u32(character + CCHARACTER_COUNTRY_TAG_DB_ID_OFF, 2);
    mem.write_u64(character + CCHARACTER_UNIT_LEADER_PTR_OFF, 0);
    mem.write_u64(country_leader, COUNTRY_LEADER_VT);
    mem.write_u64(
        country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
        character,
    );
    mem.write_u64(advisor, ADVISOR_VT);
    mem.write_u64(advisor + CADVISOR_CHARACTER_PTR_OFF, character);
    mem.write_u64(trait_def, TRAIT_VT);

    let cache = NativeRootCache {
        known_families: characters_leaders_cabinet_live_lane::FAMILY_ALL,
        roots: Some(ResolvedTypeRoots {
            ccharacter_vtable: CHAR_VT,
            carmy_leader_vtable: ARMY_VT,
            cnavy_leader_vtable: NAVY_VT,
            ccountry_leader_vtable: COUNTRY_LEADER_VT,
            cadvisor_vtable: ADVISOR_VT,
            cunit_leader_trait_vtable: TRAIT_VT,
        }),
        roots_validated: true,
        skipped_root_rows: 0,
        characters: vec![CharacterRoot {
            character_addr: character,
            char_id: 1458,
            name: Some(STALE_NAME.to_string()),
            token: Some(STALE_TOKEN.to_string()),
        }],
        unit_leaders: Vec::new(),
        unit_leader_trait_tokens: vec![TraitDefinitionRoot {
            trait_addr: trait_def,
            token: "stale_hour_trait".to_string(),
        }],
        country_leader_roles: vec![CountryLeaderRoleRoot {
            country_leader_addr: country_leader,
            char_id: 1458,
            name: None,
            token: None,
        }],
        advisors: vec![AdvisorRoot {
            advisor_addr: advisor,
            idea_token: STALE_IDEA.to_string(),
            slot: Some("political_advisor".to_string()),
            ledger: None,
        }],
        portraits: Vec::new(),
    };
    (mem, cache)
}

fn adopted_cache(roots: NativeRootCache) -> CompactDomain47PrewarmCache {
    CompactDomain47PrewarmCache {
        roots,
        sweep_gen: Some(7),
        sweep_epoch: Some(11),
        ..CompactDomain47PrewarmCache::default()
    }
}

fn emitted_row_text(mem: &FakeMemory, cache: &NativeRootCache) -> (u32, u32, String) {
    let read = read_native_scheduled_frame(mem, cache, NativeFrameInput::default());
    let decoded = domain47::decode_payload_rows(0, &read.payload).expect("payload decodes");
    (
        read.row_count,
        read.status_flags,
        format!("{:?}", decoded.rows),
    )
}

#[test]
fn unheld_roots_do_publish_the_retained_text() {
    let (mem, cache) = stale_root_world();
    let (row_count, status_flags, rows) = emitted_row_text(&mem, &cache);
    assert!(row_count > 0, "control fixture emitted no rows");
    assert_eq!(
        status_flags & COMPACT_STATUS_PARTIAL_DATA,
        0,
        "control fixture must be a CLEAN frame"
    );
    assert!(
        rows.contains(STALE_NAME) && rows.contains(STALE_TOKEN) && rows.contains(STALE_IDEA),
        "control fixture did not carry the retained text into its rows: {rows}"
    );
}

#[test]
fn first_refresh_hold_stops_the_previous_hours_roots_from_publishing() {
    let (mem, roots) = stale_root_world();
    let mut cache = adopted_cache(roots);
    assert!(cache.roots.has_stable_roots(), "fixture must have roots");

    let summary = domain47_prewarm_hold(
        &mut cache,
        true,
        false,
        true,
        Domain47PrewarmHoldReason::SweepCoverageIncomplete,
    );

    assert!(summary.refresh_held_roots, "this is a refresh hold");
    assert_ne!(
        cache.roots.skipped_root_rows, 0,
        "the FIRST hold must poison the retained roots"
    );
    assert_ne!(
        summary.skipped_root_rows, 0,
        "the hold summary must report the poison it applied"
    );

    let (row_count, status_flags, rows) = emitted_row_text(&mem, &cache.roots);
    assert_eq!(row_count, 0, "a held cache published rows: {rows}");
    assert_ne!(
        status_flags & COMPACT_STATUS_PARTIAL_DATA,
        0,
        "a held cache must emit PARTIAL"
    );
    assert!(
        !rows.contains(STALE_NAME) && !rows.contains(STALE_TOKEN) && !rows.contains(STALE_IDEA),
        "the previous hour's text reached a row: {rows}"
    );
}

#[test]
fn blocked_adoption_fails_closed_on_both_arms() {
    let (mem, roots) = stale_root_world();
    let expected_roots = roots.stable_root_count();
    let mut held = adopted_cache(roots.clone());
    assert!(
        domain47_prewarm_adoption_blocked(1, held.roots.stable_root_count()),
        "a torn read must block adoption"
    );

    let refresh_held_roots = domain47_apply_blocked_adoption(&mut held, true, 1, 0, 0);

    assert!(refresh_held_roots, "the held arm must report itself");
    assert_eq!(
        held.roots.stable_root_count(),
        expected_roots,
        "the held arm must not destroy the retained world"
    );
    assert_ne!(
        held.roots.skipped_root_rows, 0,
        "the held arm must refuse the retained roots"
    );
    assert_eq!(
        held.negative_prewarm_frames_remaining, DOMAIN47_NEGATIVE_PREWARM_RESCAN_FRAMES,
        "the held arm must arm the backoff window"
    );
    let (row_count, status_flags, rows) = emitted_row_text(&mem, &held.roots);
    assert_eq!(row_count, 0, "a held cache published rows: {rows}");
    assert_ne!(status_flags & COMPACT_STATUS_PARTIAL_DATA, 0);
    assert!(
        !rows.contains(STALE_NAME) && !rows.contains(STALE_TOKEN) && !rows.contains(STALE_IDEA),
        "the previous hour's text reached a row: {rows}"
    );

    let mut cold = CompactDomain47PrewarmCache {
        roots: domain47::scheduled::NativeRootCache {
            roots: None,
            roots_validated: false,
            characters: roots.characters.clone(),
            advisors: roots.advisors.clone(),
            skipped_root_rows: 77,
            ..domain47::scheduled::NativeRootCache::default()
        },
        sweep_gen: Some(7),
        sweep_epoch: Some(11),
        excluded_scan_hits: 99,
        ambiguous_root_rows: 98,
        negative_prewarm_frames_remaining: 0,
        ..CompactDomain47PrewarmCache::default()
    };
    assert!(!cold.roots.has_stable_roots(), "this must be the cold arm");
    assert_ne!(cold.roots.stable_root_count(), 0, "seeded with leftovers");

    let refresh_held_roots = domain47_apply_blocked_adoption(&mut cold, false, 4, 5, 6);

    assert!(!refresh_held_roots);
    assert_eq!(
        cold.roots.stable_root_count(),
        0,
        "the cold arm must REPLACE the cache, not patch it"
    );
    assert_eq!(cold.roots.skipped_root_rows, 4);
    assert_eq!(cold.sweep_gen, None);
    assert_eq!(cold.sweep_epoch, None);
    assert_eq!(cold.excluded_scan_hits, 5);
    assert_eq!(cold.ambiguous_root_rows, 6);
    assert_eq!(
        cold.negative_prewarm_frames_remaining,
        DOMAIN47_NEGATIVE_PREWARM_RESCAN_FRAMES
    );
    let cold_read = read_native_scheduled_frame(&mem, &cold.roots, NativeFrameInput::default());
    assert_eq!(cold_read.row_count, 0);
    assert_ne!(
        cold_read.status_flags & COMPACT_STATUS_SOURCE_UNAVAILABLE,
        0,
        "a rootless cache must report SOURCE_UNAVAILABLE"
    );
}

#[test]
fn a_hold_refuses_the_retained_roots_without_destroying_them() {
    let (mem, roots) = stale_root_world();
    let expected_roots = roots.stable_root_count();
    let expected_characters = roots.characters.clone();
    let mut cache = adopted_cache(roots);

    domain47_note_refresh_hold(&mut cache);

    assert_ne!(
        cache.roots.skipped_root_rows, 0,
        "the hold must refuse the retained roots"
    );
    assert_eq!(
        cache.roots.stable_root_count(),
        expected_roots,
        "the hold must not destroy the retained character world"
    );
    assert_eq!(
        cache.roots.characters, expected_characters,
        "the hold must leave every root byte-identical for the next refresh"
    );
    assert!(
        cache.roots.has_stable_roots(),
        "a held cache must still count as having roots, or the next prewarm is a COLD rebuild"
    );

    cache.roots.skipped_root_rows = 0;
    cache.refresh_hold_streak = 0;
    let (row_count, status_flags, rows) = emitted_row_text(&mem, &cache.roots);
    assert!(row_count > 0, "a recovered cache must publish again");
    assert_eq!(status_flags & COMPACT_STATUS_PARTIAL_DATA, 0);
    assert!(rows.contains(STALE_NAME), "recovered rows must carry text");
}

const FRAME_HOUR: i32 = 60_832_014;

fn clock(hour: i32, ordinal: u64) -> Option<ClockObservation> {
    Some(clock_obs(hour, ordinal))
}

#[test]
fn the_precheck_separates_the_two_clock_failures() {
    #[allow(clippy::type_complexity)]
    let cases: [(
        &str,
        Domain47PrewarmClockVerdict,
        Option<ClockObservation>,
        bool,
    ); 3] = [
        (
            "clean",
            Domain47PrewarmClockVerdict::InFrameHour,
            clock(FRAME_HOUR, 1),
            false,
        ),
        (
            "wrong hour",
            Domain47PrewarmClockVerdict::WrongHour,
            clock(FRAME_HOUR + 1, 1),
            true,
        ),
        (
            "opening unreadable",
            Domain47PrewarmClockVerdict::Unreadable,
            None,
            true,
        ),
    ];
    for (name, expected, opening, unpublishable) in cases {
        let verdict = domain47_worker_clock_precheck(opening, FRAME_HOUR);
        assert_eq!(
            verdict, expected,
            "{name}: a worker that opened in another hour is not this frame's observation"
        );
        assert_eq!(
            verdict.roots_are_unpublishable(),
            unpublishable,
            "{name}: publishability"
        );
    }
    assert_eq!(
        Domain47PrewarmSummary::default().clock_verdict,
        Domain47PrewarmClockVerdict::NotScheduled,
    );
    assert!(!Domain47PrewarmClockVerdict::NotScheduled.roots_are_unpublishable());
    let codes: Vec<u64> = [
        Domain47PrewarmClockVerdict::NotScheduled,
        Domain47PrewarmClockVerdict::InFrameHour,
        Domain47PrewarmClockVerdict::WrongHour,
        Domain47PrewarmClockVerdict::Unreadable,
    ]
    .iter()
    .map(|verdict| verdict.code())
    .collect();
    let mut distinct = codes.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        codes.len(),
        "verdict codes must be distinct"
    );
    assert!(
        !codes.contains(&2),
        "code 2 (Straddled) is retired, not reassigned"
    );
}

#[test]
fn a_straddled_prewarm_bracket_rejects_d47_as_torn() {
    let brackets = [DomainClockBracket {
        tag: COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE,
        opening: clock(FRAME_HOUR, 1),
        closing: clock(FRAME_HOUR + 1, 2),
        worker_panicked: false,
    }];
    let rejected = rejected_bracket_domains(&brackets, FRAME_HOUR, HourLabelMismatchPolicy::Refuse);
    let d47 = rejected
        .get(&COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE)
        .copied()
        .expect("a straddled prewarm must reject d47");
    assert_eq!(d47.0, compact_contract::domain_gap::DomainGapReason::Torn);
    assert_eq!(
        (d47.1, d47.2),
        (FRAME_HOUR as u64, (FRAME_HOUR + 1) as u64),
        "the gap must witness the prewarm's OWN observed hours"
    );
    assert_eq!(rejected.len(), 1, "only d47 is covered by d47's bracket");

    let clean = [DomainClockBracket {
        tag: COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE,
        opening: clock(FRAME_HOUR, 1),
        closing: clock(FRAME_HOUR, 2),
        worker_panicked: false,
    }];
    assert!(
        rejected_bracket_domains(&clean, FRAME_HOUR, HourLabelMismatchPolicy::Refuse).is_empty(),
        "an in-hour prewarm must reject nothing"
    );
}

#[test]
fn a_wrong_hour_verdict_is_recorded_and_the_cache_is_untouched() {
    let (_mem, roots) = stale_root_world();
    let mut cache = adopted_cache(roots);
    let mut summary = Domain47PrewarmSummary::default();

    let verdict = domain47_worker_clock_precheck(clock(FRAME_HOUR + 1, 1), FRAME_HOUR);
    let wrong_hour_bracket = [DomainClockBracket {
        tag: COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE,
        opening: clock(FRAME_HOUR + 1, 1),
        closing: clock(FRAME_HOUR + 1, 2),
        worker_panicked: false,
    }];
    assert!(
        rejected_bracket_domains(
            &wrong_hour_bracket,
            FRAME_HOUR,
            HourLabelMismatchPolicy::CountOnly,
        )
        .is_empty(),
        "premise: nothing tore, so the tear machinery accepts this frame and          the refusal below cannot have come from it"
    );
    assert_eq!(
        rejected_bracket_domains(
            &wrong_hour_bracket,
            FRAME_HOUR,
            HourLabelMismatchPolicy::Refuse,
        )
        .get(&COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE)
        .map(|outcome| outcome.0),
        Some(compact_contract::domain_gap::DomainGapReason::HourLabelMismatch),
    );

    let before_gen = cache.sweep_gen;
    let before_epoch = cache.sweep_epoch;
    domain47_apply_prewarm_clock_verdict(verdict, &mut summary);

    assert_eq!(
        summary.clock_verdict,
        Domain47PrewarmClockVerdict::WrongHour
    );
    assert!(summary.clock_verdict.roots_are_unpublishable());
    assert_eq!(
        cache.roots.skipped_root_rows, 0,
        "the cache must not be poisoned by a clock verdict"
    );
    assert!(cache.roots.has_stable_roots());
    assert_eq!(cache.sweep_gen, before_gen);
    assert_eq!(cache.sweep_epoch, before_epoch);
}

#[test]
fn an_in_hour_prewarm_publishes_unchanged() {
    let (mem, roots) = stale_root_world();
    let mut cache = adopted_cache(roots);
    let mut summary = Domain47PrewarmSummary::default();

    domain47_apply_prewarm_clock_verdict(
        domain47_worker_clock_precheck(clock(FRAME_HOUR, 1), FRAME_HOUR),
        &mut summary,
    );

    assert_eq!(
        summary.clock_verdict,
        Domain47PrewarmClockVerdict::InFrameHour
    );
    assert_eq!(
        cache.roots.skipped_root_rows, 0,
        "an in-hour prewarm must not poison its own roots"
    );
    let (row_count, status_flags, rows) = emitted_row_text(&mem, &cache.roots);
    assert!(row_count > 0, "an in-hour prewarm must publish");
    assert_eq!(status_flags & COMPACT_STATUS_PARTIAL_DATA, 0);
    assert!(rows.contains(STALE_NAME));
}

#[test]
fn an_unscheduled_prewarm_is_not_a_clock_failure() {
    let (_mem, roots) = stale_root_world();
    let mut cache = adopted_cache(roots);
    let mut summary = Domain47PrewarmSummary::default();

    let verdict = Domain47PrewarmClockVerdict::NotScheduled;
    assert_eq!(
        summary.clock_verdict, verdict,
        "the default summary's verdict is NotScheduled"
    );
    domain47_apply_prewarm_clock_verdict(verdict, &mut summary);
    assert_eq!(
        cache.roots.skipped_root_rows, 0,
        "a domain that was never read must not be refused"
    );
}

#[test]
fn nothing_is_held_through_a_cold_sweep() {
    assert!(!domain47_prewarm_holds_instead_of_recensusing(
        false,
        domain47::FAMILY_ALL
    ));
    assert!(!domain47_prewarm_holds_instead_of_recensusing(
        true,
        domain47::FAMILY_ALL
    ));
    assert!(!domain47_prewarm_holds_instead_of_recensusing(
        true,
        domain47::FAMILY_WALK_WITHOUT_TRAITS
    ));
    assert!(!domain47_prewarm_holds_instead_of_recensusing(
        true,
        domain47::FAMILY_CENSUS_ONLY
    ));
}
