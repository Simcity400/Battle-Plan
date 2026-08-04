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
            country_tag_db_id: 2,
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

    let summary = domain47_prewarm_hold(&mut cache, true, false, true);

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
