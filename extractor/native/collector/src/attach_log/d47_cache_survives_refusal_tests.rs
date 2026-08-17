//! Lane MX: d47's DISCOVERY CACHE (roots + sealed byte witness + walk families)
//! SURVIVES every refusal path — its own rejected bracket, a frame that rejects
//! every domain, and the whole-frame refusal salvage.
//!
//! WHY (MT t83, code-confirmed): `reset_trackers_after_rejected_bracket`
//! default-constructed the whole cache whenever d47's bracket was refused, so
//! the next frame ran a cold 75-85 ms discovery, straddled the next ~90-136 ms
//! hour, was refused again, and wiped the cache again — 350 full frames against
//! 19 replay misses, reuse on 3 of 373 frames. The cache is never published
//! from directly: the next frame's worker replays the witness inside ITS
//! bracket at open and close and publishes only on a byte-identical replay, so
//! a refused bracket (a statement about which HOUR this frame's product belongs
//! to) has nothing to say about the cache's validity.
//!
//! What production change makes each test fail: removing the
//! `characters_leaders_cabinet_live` carve-out from
//! `reset_trackers_after_rejected_bracket` (the blanket `fresh` then wins), or
//! the move in `restore_country_trackers_after_refused_frame`.

use super::*;

use domain47::adapter::CharacterRoot;
use domain47::extractor::ResolvedTypeRoots;
use domain47::scheduled::NativeRootCache;

/// A cache as it stands after one adoption that sealed a witness and cached a
/// walk family, with distinctive values in every field a wipe would erase.
pub(super) fn adopted_cache_with_witness() -> CompactDomain47PrewarmCache {
    let mut roots = NativeRootCache::default();
    roots.roots_validated = true;
    roots.known_families = domain47::FAMILY_ALL;
    roots.roots = Some(ResolvedTypeRoots {
        ccharacter_vtable: 0x1000_1000,
        carmy_leader_vtable: 0x1000_2000,
        cnavy_leader_vtable: 0x1000_3000,
        ccountry_leader_vtable: 0x1000_4000,
        cadvisor_vtable: 0x1000_5000,
        cunit_leader_trait_vtable: 0x1000_6000,
    });
    roots.characters.push(CharacterRoot {
        character_addr: 0x1000,
        char_id: 1458,
        name: Some("mx_name".to_string()),
        token: Some("mx_token".to_string()),
    });
    let mut walk = domain47::adapter::CachedWalk::default();
    walk.families = domain47::FAMILY_CHARACTER;
    let witness = Domain47DiscoveryWitness {
        gamestate: 0x40,
        base: 0x7ff6_0000_0000,
        manager_root: 0x50,
        manager: 0x60,
        manager_vtable: 0x70,
        historical: Domain47ManagerVectorSnapshot {
            data: 0x80,
            capacity: 4,
            size: 1,
            entries: vec![0x1000],
        },
        dynamic: Domain47ManagerVectorSnapshot {
            data: 0x90,
            capacity: 0,
            size: 0,
            entries: Vec::new(),
        },
        pieces: vec![Domain47WitnessRecord {
            addr: 0x1000,
            len: 4,
            off: 0,
        }],
        bytes: vec![1, 2, 3, 4],
        spans: vec![Domain47WitnessSpan {
            start: 0x1000,
            len: 4,
            first_piece: 0,
            piece_count: 1,
        }],
        failed: vec![(0x2000, 8)],
        raw_record_count: 2,
        walk: Some(std::sync::Arc::new(walk)),
    };
    CompactDomain47PrewarmCache {
        roots,
        sweep_gen: Some(7),
        sweep_epoch: Some(11),
        negative_prewarm_frames_remaining: 3,
        refresh_hold_streak: 1,
        witness: Some(witness),
        witness_seal_refusal: 0,
        witness_last_miss: (2, 9),
        witness_last_miss_addr: 0x2070_32f5_f18,
        adopted_summary: Domain47PrewarmSummary {
            ran: true,
            stable_root_count: 1,
            ..Domain47PrewarmSummary::default()
        },
        ..CompactDomain47PrewarmCache::default()
    }
}

fn assert_cache_survived(cache: &CompactDomain47PrewarmCache, expected: &CompactDomain47PrewarmCache) {
    assert!(cache.roots.has_stable_roots(), "the adopted roots must survive");
    assert_eq!(cache.roots.characters.len(), 1);
    assert_eq!(cache.roots.characters[0].name.as_deref(), Some("mx_name"));
    assert_eq!(cache.roots.skipped_root_rows, expected.roots.skipped_root_rows);
    assert_eq!(cache.sweep_gen, expected.sweep_gen);
    assert_eq!(cache.sweep_epoch, expected.sweep_epoch);
    assert_eq!(
        cache.negative_prewarm_frames_remaining,
        expected.negative_prewarm_frames_remaining
    );
    assert_eq!(cache.refresh_hold_streak, expected.refresh_hold_streak);
    assert_eq!(cache.witness, expected.witness, "the sealed witness must survive");
    assert!(
        cache
            .witness
            .as_ref()
            .and_then(|w| w.walk.as_ref())
            .is_some_and(|w| w.families == domain47::FAMILY_CHARACTER),
        "the cached walk families must survive with the witness"
    );
    assert_eq!(cache.witness_last_miss, expected.witness_last_miss);
    assert_eq!(cache.witness_last_miss_addr, expected.witness_last_miss_addr);
    assert_eq!(cache.adopted_summary.stable_root_count, 1);
}

/// d47's OWN rejection (Torn / HourLabelMismatch on its bracket) keeps the
/// cache whole. This is the death-spiral case.
#[test]
fn d47s_own_rejection_keeps_roots_witness_and_walk() {
    let expected = adopted_cache_with_witness();
    let mut trackers = CompactPerfGateTrackers::default();
    trackers.characters_leaders_cabinet_live = adopted_cache_with_witness();
    let rejected: BTreeSet<u16> = [COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE].into();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    assert_cache_survived(&trackers.characters_leaders_cabinet_live, &expected);
}

/// Every domain rejected: the cache still stands (the blanket reset is not its
/// floor).
#[test]
fn rejecting_every_domain_keeps_the_d47_cache() {
    let expected = adopted_cache_with_witness();
    let mut trackers = CompactPerfGateTrackers::default();
    trackers.characters_leaders_cabinet_live = adopted_cache_with_witness();
    let everything: BTreeSet<u16> = (0u16..=1000).collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &everything);
    assert_cache_survived(&trackers.characters_leaders_cabinet_live, &expected);
}

/// POISON set inside the refused frame (a torn root read, a refresh hold, a
/// dead sweep epoch — since step 1b a clock verdict no longer poisons) is KEPT
/// — it refuses until a successful adoption clears it. The wipe used to clear
/// the poison, which was weaker.
///
/// kills: a carve-out that reset `skipped_root_rows` to 0 "because the frame
/// was refused anyway" would let the next frame's NEGATIVE-BACKOFF early
/// return (type roots unresolved, backoff window still armed — it returns
/// before witness eligibility and before any renewed poisoning) publish the
/// retained roots as CLEAN (the poison, not the dropped witness, is what the
/// scheduled reader's rowless gate reads). The hold arm is not that path: it
/// re-poisons through `domain47_note_refresh_hold` before publication.
#[test]
fn poison_set_in_the_refused_frame_is_kept_not_cleared() {
    let mut poisoned = adopted_cache_with_witness();
    domain47_poison_prewarm_roots(&mut poisoned);
    assert_ne!(poisoned.roots.skipped_root_rows, 0);
    assert!(poisoned.witness.is_none(), "poison drops the witness");
    let mut trackers = CompactPerfGateTrackers::default();
    trackers.characters_leaders_cabinet_live = poisoned;
    let rejected: BTreeSet<u16> = [COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE].into();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    let cache = &trackers.characters_leaders_cabinet_live;
    assert_ne!(
        cache.roots.skipped_root_rows, 0,
        "the poison must survive the reset; only a successful adoption clears it"
    );
    assert!(cache.roots.has_stable_roots(), "poisoned roots are held, not destroyed");
    assert!(cache.witness.is_none());
    assert!(
        !domain47_witness_reuse_eligible(
            cache.roots.has_stable_roots(),
            cache.roots.skipped_root_rows,
            cache.sweep_gen.is_some(),
            cache.witness.is_some(),
        ),
        "a poisoned cache is never replayed"
    );
}

/// The whole-frame refusal salvage moves the cache to the published slot
/// beside the country baselines.
#[test]
fn a_refused_frame_returns_the_d47_cache_to_the_published_slot() {
    let expected = adopted_cache_with_witness();
    let mut refused = CompactPerfGateTrackers::default();
    refused.characters_leaders_cabinet_live = adopted_cache_with_witness();
    let mut published = CompactPerfGateTrackers::default();
    restore_country_trackers_after_refused_frame(&mut refused, &mut published);
    assert_cache_survived(&published.characters_leaders_cabinet_live, &expected);
}

/// CONTROL: the fixture is a cache a wipe WOULD change, so the assertions above
/// cannot pass against `Default`.
#[test]
fn the_fixture_is_not_the_default_cache() {
    let cache = adopted_cache_with_witness();
    let fresh = CompactDomain47PrewarmCache::default();
    assert!(cache.roots.has_stable_roots() && !fresh.roots.has_stable_roots());
    assert!(cache.witness.is_some() && fresh.witness.is_none());
    assert_ne!(cache.sweep_gen, fresh.sweep_gen);
    assert!(domain47_witness_reuse_eligible(
        cache.roots.has_stable_roots(),
        cache.roots.skipped_root_rows,
        cache.sweep_gen.is_some(),
        cache.witness.is_some(),
    ));
}
