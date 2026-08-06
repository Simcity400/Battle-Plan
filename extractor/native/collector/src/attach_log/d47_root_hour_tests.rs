//! d47's retained-root cache must never publish a row under an hour it did
//! not observe.
//!
//! THE RETENTION. `CompactDomain47PrewarmCache::roots` is the ONLY d47 state
//! that survives a frame. It holds, verbatim from the last SUCCESSFUL
//! prewarm, one entry per character / unit leader / country-leader role /
//! advisor / trait definition, and each entry carries ancillary TEXT the
//! frame walk does not re-read: `CharacterRoot::{name, token}`,
//! `UnitLeaderRoot::{name, token}` (plus its `country_tag_db_id` and
//! `leader_kind`), `AdvisorRoot::{idea_token, slot, ledger}` and
//! `TraitDefinitionRoot::token`. The walk re-reads the live scalars at each
//! root ADDRESS every frame, but copies that text straight into the emitted
//! row.
//!
//! WHY A HOLD IS ALWAYS CROSS-HOUR. One extraction frame is one scheduled
//! game hour — the record's label is `request.stamp.hour_raw`, minted once
//! per hour by the read-only scheduler. Consecutive frames are therefore
//! never closer than one hour apart, and when the frame period overruns the
//! 77.73 ms hour budget they are further apart, never nearer. So roots
//! retained across a failed prewarm refresh were observed in a STRICTLY
//! EARLIER hour than the frame that would publish them; there is no
//! same-hour case to preserve, and no runtime hour comparison can find one.
//!
//! WHAT WAS WRONG. `domain47_note_refresh_hold` used to poison the retained
//! roots only from the SECOND consecutive hold, so the first hold published
//! CLEAN rows built from the previous hour's roots — no PARTIAL flag, no
//! ledger entry, nothing naming the relabel. And because the
//! type-root-resolution hold arms a 12-frame negative-cache window whose
//! frames take an early return WITHOUT calling that function, the streak
//! never advanced past 1: up to 13 consecutive hours could publish one
//! hour's roots.

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

/// Text that could only have come out of the retained cache. If any of these
/// reaches a decoded row, the previous hour's roots were published.
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

/// A world whose LIVE memory is entirely readable and consistent, so the only
/// thing that can make the reader refuse is the retained-root poison itself.
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
    mem.write_u64(country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF, character);
    mem.write_u64(advisor, ADVISOR_VT);
    mem.write_u64(advisor + CADVISOR_CHARACTER_PTR_OFF, character);
    mem.write_u64(trait_def, TRAIT_VT);

    let cache = NativeRootCache {
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

/// A prewarm cache holding `roots`, as it stands after one successful
/// adoption and before any hold.
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

/// CONTROL — the fixture is capable of publishing the retained text.
///
/// Without this, every "no stale text was published" assertion below could
/// pass because the fixture never produced any rows at all.
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

/// THE FIX. One failed refresh must poison the retained roots, so the very
/// next emission is a rowless PARTIAL instead of a CLEAN frame carrying the
/// previous hour's text.
///
/// MUTATION: in `domain47_note_refresh_hold`, restore the tolerance —
/// `if cache.refresh_hold_streak >= 2 { cache.roots.skipped_root_rows = ... }`
/// This test then observes `row_count > 0`, no PARTIAL flag, and the stale
/// name/token/idea text in the decoded rows, and fails on all three.
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

/// The adoption-blocked arm — the other way a refresh fails — takes the same
/// poison, and its COLD twin still fails closed the way it always did.
///
/// This drives `domain47_apply_blocked_adoption`, which is the whole of what
/// the prewarm does on that branch. The previous version of this test called
/// `domain47_note_refresh_hold` itself, so deleting the production call would
/// have left it green (Codex round 1, finding 2).
///
/// SCOPE, stated honestly: this pins the POLICY, not the call site. It cannot
/// notice `prewarm_compact_characters_leaders_cabinet_roots` ceasing to call
/// `domain47_apply_blocked_adoption` at all — reaching that branch needs a
/// live engine, a warm heap sweep and a real manager roster, none of which
/// exist offline. What it does cover is every way the policy itself could be
/// weakened.
///
/// MUTATION (held arm): drop the `domain47_note_refresh_hold(cache)` call from
/// `domain47_apply_blocked_adoption`, or restore the `>= 2` tolerance inside
/// it — the held-arm assertions fail.
/// MUTATION (cold arm): make the cold arm retain `cache.roots` instead of
/// replacing it with a rootless cache — the cold-arm assertions fail.
#[test]
fn blocked_adoption_fails_closed_on_both_arms() {
    // HELD: stable roots exist, so they are retained AND refused.
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

    // COLD: no adopted world to protect, so the cache is REPLACED with a
    // rootless one carrying this pass's counters, and the reader then reports
    // SOURCE_UNAVAILABLE rather than an empty resolved world.
    //
    // Every field below starts at a value the cold arm must overwrite. Seeded
    // from `default()` instead, each assertion would hold whether or not the
    // arm did anything, and the whole half would be decoration.
    let mut cold = CompactDomain47PrewarmCache {
        roots: domain47::scheduled::NativeRootCache {
            // Populated vectors with `roots_validated` false: `has_stable_roots`
            // is false, so this IS the cold arm, and a mutation that retained
            // the cache instead of replacing it would leave them behind.
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

/// A hold must REFUSE the retained roots without DESTROYING them.
///
/// These are two different things and the obvious over-correction conflates
/// them: "a held root view must not publish" is easiest to implement by
/// emptying `cache.roots`, and that is the 2026-07-10 d47 data-loss defect —
/// the fold full-replaces the live character world with nothing, and the
/// domain then has to rediscover every character before it can emit again.
/// The hold must leave the world intact so the next successful prewarm is a
/// refresh and not a cold rebuild, and the ONLY thing standing between the
/// retained roots and the wire must be `skipped_root_rows`, which a
/// successful adoption clears by assigning a freshly built cache.
///
/// MUTATION: make `domain47_note_refresh_hold` empty the roots —
/// `cache.roots = domain47::scheduled::NativeRootCache::default();` — which is
/// exactly that over-correction. The root-count and root-identity assertions
/// below fail, and so does the republish.
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

    // Clearing the marker is all a successful adoption does to the emit
    // decision: it assigns a freshly built cache whose `skipped_root_rows` is
    // zero by construction. One transient outage therefore costs one rowless
    // hour, not the domain.
    cache.roots.skipped_root_rows = 0;
    cache.refresh_hold_streak = 0;
    let (row_count, status_flags, rows) = emitted_row_text(&mem, &cache.roots);
    assert!(row_count > 0, "a recovered cache must publish again");
    assert_eq!(status_flags & COMPACT_STATUS_PARTIAL_DATA, 0);
    assert!(rows.contains(STALE_NAME), "recovered rows must carry text");
}

// ---------------------------------------------------------------------------
// The prewarm's OWN hour, not just the hour it was retained from.
//
// The tests above are about roots that OUTLIVE their prewarm. These are about
// the prewarm itself: it is a serial main-thread phase that ran inside no
// clock bracket at all, and it is the phase that reads the TEXT the frame walk
// copies into the row. t17 (`runs/live_unlock/t17`, 490 frames whose prewarm
// ran) carries 24 frames — 4.90% — whose wire witnesses put the label -> label+1
// tick at or before that phase: 21 frames where a worker's bracket opened at
// label+1 while no worker witnessed the label -> label+1 tick at all (so it fell
// in the one unbracketed window OR in the frame preamble that precedes it), and
// 3 where the independent stable clock read taken earlier in the same frame —
// at the top of the heavy sweep, before this phase, not adjacent to it — was
// already past the label. That is a LOWER bound: a tick inside the prewarm
// leaves no wire witness unless a SECOND tick then lands inside some worker.
// Scaling the fan-out's own fully-witnessed rate (32.45% of heavy frames over a
// 65.2 ms window) to the 25.2 ms prewarm gives 12.55%, and 62.65% of heavy
// frames witnessed no tick anywhere, so the true rate is bounded only between
// 4.90% and 62.65%. `domain47_prewarm_clock_verdict` on the wire is what will
// resolve it exactly in the next capture.
// ---------------------------------------------------------------------------

const FRAME_HOUR: i32 = 60_832_014;

/// An authenticated clock observation, as `observe_frame_clock` returns it: the
/// game hour, its global observation ordinal, and its position in the frame.
fn clock(hour: i32, ordinal: u64) -> Option<ClockObservation> {
    Some(clock_obs(hour, ordinal))
}

/// Each way the prewarm's bracket can land, and what it proves.
///
/// MUTATION RUN: in `domain47_prewarm_clock_verdict`, delete the label check
///   `if opening.0 != game_hour_raw { return WrongHour; }`
/// OBSERVED: this test fails at the "wrong hour" case —
///   `assertion `left == right` failed: wrong hour: a prewarm that ran wholly
///    in another hour is not this frame's observation
///      left: InFrameHour
///     right: WrongHour`
/// and `a_wrong_hour_prewarm_refuses_its_roots_although_nothing_tore` fails
/// with it (7 passed, 2 failed). Because the loop stops at the first bad case,
/// this run does NOT separately re-prove the straddle and unreadable arms; the
/// straddle arm is proven by `a_straddled_prewarm_bracket_rejects_d47_as_torn`.
#[test]
fn the_prewarm_clock_verdict_separates_the_three_failures() {
    #[allow(clippy::type_complexity)]
    let cases: [(
        &str,
        Domain47PrewarmClockVerdict,
        bool,
        Option<ClockObservation>,
        Option<ClockObservation>,
        bool,
    ); 7] = [
        // (name, expected, scheduled, opening, closing, roots_unpublishable)
        (
            "not scheduled",
            Domain47PrewarmClockVerdict::NotScheduled,
            false,
            None,
            None,
            false,
        ),
        (
            "clean",
            Domain47PrewarmClockVerdict::InFrameHour,
            true,
            clock(FRAME_HOUR, 1),
            clock(FRAME_HOUR, 2),
            false,
        ),
        (
            "straddle",
            Domain47PrewarmClockVerdict::Straddled,
            true,
            clock(FRAME_HOUR, 1),
            clock(FRAME_HOUR + 1, 2),
            true,
        ),
        (
            "wrong hour",
            Domain47PrewarmClockVerdict::WrongHour,
            true,
            clock(FRAME_HOUR + 1, 1),
            clock(FRAME_HOUR + 1, 2),
            true,
        ),
        (
            "opening unreadable",
            Domain47PrewarmClockVerdict::Unreadable,
            true,
            None,
            clock(FRAME_HOUR, 2),
            true,
        ),
        (
            "closing unreadable",
            Domain47PrewarmClockVerdict::Unreadable,
            true,
            clock(FRAME_HOUR, 1),
            None,
            true,
        ),
        (
            "scheduled but never read",
            Domain47PrewarmClockVerdict::Unreadable,
            true,
            None,
            None,
            true,
        ),
    ];
    for (name, expected, scheduled, opening, closing, unpublishable) in cases {
        let verdict = domain47_prewarm_clock_verdict(scheduled, opening, closing, FRAME_HOUR);
        assert_eq!(
            verdict, expected,
            "{name}: a prewarm that ran wholly in another hour is not this frame's observation"
        );
        assert_eq!(
            verdict.roots_are_unpublishable(),
            unpublishable,
            "{name}: publishability"
        );
    }
    // Every arm reports a DISTINCT code, or a capture cannot name which clock
    // failure refused its hour.
    let codes: Vec<u64> = [
        Domain47PrewarmClockVerdict::NotScheduled,
        Domain47PrewarmClockVerdict::InFrameHour,
        Domain47PrewarmClockVerdict::Straddled,
        Domain47PrewarmClockVerdict::WrongHour,
        Domain47PrewarmClockVerdict::Unreadable,
    ]
    .iter()
    .map(|verdict| verdict.code())
    .collect();
    let mut distinct = codes.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), codes.len(), "verdict codes must be distinct");
}

/// A straddled prewarm must reach the wire as a Torn DOMAIN_GAP for d47 and
/// for nothing else. This is the shape the production path records — the
/// honest (opening, closing) pair, under d47's tag, with `worker_panicked`
/// false — fed to the same `rejected_bracket_domains` every worker's bracket
/// goes through.
///
/// LIMIT: this proves the recorded SHAPE rejects d47. It cannot prove the
/// production site records it, because that site is inside
/// `build_compact_extraction_gate_records` and has no test seam.
///
/// MUTATION RUN: change the fixture's tag from
/// `COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE` to
/// `COMPACT_DOMAIN_DIVISIONS` — the shape a mis-tagged production record would
/// produce.
/// OBSERVED: `panicked at ...: a straddled prewarm must reject d47` (8 passed,
/// 1 failed) — a mis-tagged bracket silently rejects an unrelated domain while
/// d47 publishes the straddled text.
#[test]
fn a_straddled_prewarm_bracket_rejects_d47_as_torn() {
    let brackets = [DomainClockBracket {
        tag: COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE,
        opening: clock(FRAME_HOUR, 1),
        closing: clock(FRAME_HOUR + 1, 2),
        worker_panicked: false,
    }];
    let rejected =
        rejected_bracket_domains(&brackets, FRAME_HOUR, HourLabelMismatchPolicy::Refuse);
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

    // CONTROL: the same bracket that did not straddle rejects nothing, so the
    // assertion above is not passing because every bracket is rejected.
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

/// The `WrongHour` arm is the one a bracket CANNOT catch: opening == closing,
/// so `rejected_bracket_domains` accepts it, and without the direct refusal
/// the frame publishes the other hour's text CLEAN. This drives the real
/// production function over the real root cache and the real frame builder.
///
/// MUTATION RUN: in `domain47_apply_prewarm_clock_verdict`, narrow the refusal
/// to straddles only —
///   `if verdict == Domain47PrewarmClockVerdict::Straddled { poison }`
/// OBSERVED: `a prewarm that ran in another hour must not publish its text:
/// row_count 3, status 128` (left 3, right 0) — three rows carrying the
/// fixture's text published under a label the phase never observed, and status
/// 128 does NOT include PARTIAL (512), so the hour went out CLEAN.
#[test]
fn a_wrong_hour_prewarm_refuses_its_roots_although_nothing_tore() {
    let (mem, roots) = stale_root_world();
    let mut cache = adopted_cache(roots);
    let mut summary = Domain47PrewarmSummary::default();

    // Nothing tore: both reads agree, and both disagree with the label.
    let verdict = domain47_prewarm_clock_verdict(
        true,
        clock(FRAME_HOUR + 1, 1),
        clock(FRAME_HOUR + 1, 2),
        FRAME_HOUR,
    );
    // PREMISE, and the reason d47's own refusal is not redundant. This frame's
    // bracket does not tear, so the TEAR machinery -- the only thing guarding
    // the other 67 domains when this test was written -- cannot see it.
    // `CountOnly` is that machinery with the generalised hour-label rule
    // switched off, and it still accepts the frame.
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
        "premise: nothing tore, so the tear machinery accepts this frame and \
         the refusal below cannot have come from it"
    );
    // With the rule ON, the frame-wide check reaches the same verdict d47
    // reaches for itself below. Two independent statements of one rule that
    // AGREE, which is the point of generalising it -- and d47 keeps its own,
    // because the roots it refuses are built in a phase that runs before any
    // worker bracket exists.
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

    domain47_apply_prewarm_clock_verdict(verdict, &mut cache, &mut summary);

    assert_eq!(summary.clock_verdict, Domain47PrewarmClockVerdict::WrongHour);
    let (row_count, status_flags, rows) = emitted_row_text(&mem, &cache.roots);
    assert_eq!(
        row_count, 0,
        "a prewarm that ran in another hour must not publish its text: \
         row_count {row_count}, status {status_flags}"
    );
    assert_ne!(
        status_flags & COMPACT_STATUS_PARTIAL_DATA,
        0,
        "the refused hour must be VISIBLE as PARTIAL, not silently empty"
    );
    assert!(
        !rows.contains(STALE_NAME) && !rows.contains(STALE_TOKEN) && !rows.contains(STALE_IDEA),
        "no text read in another hour may reach a row: {rows}"
    );
    // The world survives: the next in-hour prewarm is a refresh, not a cold
    // rebuild, so one wrong-hour frame costs one hour and not the domain.
    assert!(cache.roots.has_stable_roots());
}

/// An in-hour prewarm must change NOTHING. Without this, every assertion above
/// would still pass if the verdict refused unconditionally — and d47 would
/// lose every hour rather than the ones it cannot attribute.
///
/// MUTATION RUN: in `roots_are_unpublishable`, move `InFrameHour` into the
/// `true` arm.
/// OBSERVED: 7 passed, 2 failed. This test stops one assertion EARLIER than
/// the row check — `an in-hour prewarm must not poison its own roots`, left 1,
/// right 0 — and `the_prewarm_clock_verdict_separates_the_three_failures` fails
/// at `clean: publishability`, left true / right false. The blunt fix costs the
/// whole domain, which is exactly the trade this lane was told not to make
/// silently.
#[test]
fn an_in_hour_prewarm_publishes_unchanged() {
    let (mem, roots) = stale_root_world();
    let mut cache = adopted_cache(roots);
    let mut summary = Domain47PrewarmSummary::default();

    domain47_apply_prewarm_clock_verdict(
        domain47_prewarm_clock_verdict(
            true,
            clock(FRAME_HOUR, 1),
            clock(FRAME_HOUR, 2),
            FRAME_HOUR,
        ),
        &mut cache,
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

/// A frame that did not schedule d47 must not be charged a clock failure. The
/// prewarm did not run, so there is nothing to attribute and nothing to
/// refuse; recording a bracket for it would mint a ClockUnreadable rejection
/// for a domain that was never read.
///
/// MUTATION RUN: drop the `scheduled` early return from
/// `domain47_prewarm_clock_verdict` (both clock reads are `None` when the
/// phase is skipped, so it falls through to the unreadable arm).
/// OBSERVED: 7 passed, 2 failed — this test with `an unscheduled d47 is not a
/// clock failure` (left `Unreadable`, right `NotScheduled`), and
/// `the_prewarm_clock_verdict_separates_the_three_failures` at its own "not
/// scheduled" case. The follow-on poison assertion is never reached, because
/// the verdict assertion fires first; the poison it would have observed is the
/// one `roots_are_unpublishable` returns for `Unreadable`.
#[test]
fn an_unscheduled_prewarm_is_not_a_clock_failure() {
    let (_mem, roots) = stale_root_world();
    let mut cache = adopted_cache(roots);
    let mut summary = Domain47PrewarmSummary::default();

    let verdict = domain47_prewarm_clock_verdict(false, None, None, FRAME_HOUR);
    assert_eq!(
        verdict,
        Domain47PrewarmClockVerdict::NotScheduled,
        "an unscheduled d47 is not a clock failure"
    );
    domain47_apply_prewarm_clock_verdict(verdict, &mut cache, &mut summary);
    assert_eq!(
        cache.roots.skipped_root_rows, 0,
        "a domain that was never read must not be refused"
    );
}
