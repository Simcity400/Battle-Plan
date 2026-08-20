use super::*;

use domain47::adapter::CharacterRoot;
use domain47::extractor::ResolvedTypeRoots;
use domain47::scheduled::NativeRootCache;

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

fn assert_cache_survived(
    cache: &CompactDomain47PrewarmCache,
    expected: &CompactDomain47PrewarmCache,
) {
    assert!(
        cache.roots.has_stable_roots(),
        "the adopted roots must survive"
    );
    assert_eq!(cache.roots.characters.len(), 1);
    assert_eq!(cache.roots.characters[0].name.as_deref(), Some("mx_name"));
    assert_eq!(
        cache.roots.skipped_root_rows,
        expected.roots.skipped_root_rows
    );
    assert_eq!(cache.sweep_gen, expected.sweep_gen);
    assert_eq!(cache.sweep_epoch, expected.sweep_epoch);
    assert_eq!(
        cache.negative_prewarm_frames_remaining,
        expected.negative_prewarm_frames_remaining
    );
    assert_eq!(cache.refresh_hold_streak, expected.refresh_hold_streak);
    assert_eq!(
        cache.witness, expected.witness,
        "the sealed witness must survive"
    );
    assert!(
        cache
            .witness
            .as_ref()
            .and_then(|w| w.walk.as_ref())
            .is_some_and(|w| w.families == domain47::FAMILY_CHARACTER),
        "the cached walk families must survive with the witness"
    );
    assert_eq!(cache.witness_last_miss, expected.witness_last_miss);
    assert_eq!(
        cache.witness_last_miss_addr,
        expected.witness_last_miss_addr
    );
    assert_eq!(cache.adopted_summary.stable_root_count, 1);
}

#[test]
fn d47s_own_rejection_keeps_roots_witness_and_walk() {
    let expected = adopted_cache_with_witness();
    let mut trackers = CompactPerfGateTrackers::default();
    trackers.characters_leaders_cabinet_live = adopted_cache_with_witness();
    let rejected: BTreeSet<u16> = [COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE].into();
    reset_trackers_after_rejected_bracket(&mut trackers, &rejected);
    assert_cache_survived(&trackers.characters_leaders_cabinet_live, &expected);
}

#[test]
fn rejecting_every_domain_keeps_the_d47_cache() {
    let expected = adopted_cache_with_witness();
    let mut trackers = CompactPerfGateTrackers::default();
    trackers.characters_leaders_cabinet_live = adopted_cache_with_witness();
    let everything: BTreeSet<u16> = (0u16..=1000).collect();
    reset_trackers_after_rejected_bracket(&mut trackers, &everything);
    assert_cache_survived(&trackers.characters_leaders_cabinet_live, &expected);
}

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
    assert!(
        cache.roots.has_stable_roots(),
        "poisoned roots are held, not destroyed"
    );
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

#[test]
fn a_refused_frame_returns_the_d47_cache_to_the_published_slot() {
    let expected = adopted_cache_with_witness();
    let mut refused = CompactPerfGateTrackers::default();
    refused.characters_leaders_cabinet_live = adopted_cache_with_witness();
    let mut published = CompactPerfGateTrackers::default();
    restore_country_trackers_after_refused_frame(&mut refused, &mut published);
    assert_cache_survived(&published.characters_leaders_cabinet_live, &expected);
}

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

// ---------------------------------------------------------------------
// Lane MX step 2: the witness-bytes miss locator (`domain47_witness_miss_owner`)
// ---------------------------------------------------------------------

/// A witness whose bytes carry pointers the classifier must FOLLOW without
/// touching the engine: manager (name-index header -> bucket array), a
/// character body (name string header -> heap payload; portrait map header ->
/// entry array -> gfx payload), an image-side trait database root -> db object
/// -> data array. Every address is synthetic; nothing here is read live.
fn witness_with_pointers() -> CompactDomain47PrewarmCache {
    const BASE: usize = 0x7ff6_0000_0000;
    const MANAGER: usize = 0x2000_0000;
    const BUCKETS: usize = 0x1000_0000; // below the manager, as t84's ordinal-1 span was
    const CHARACTER: usize = 0x3000_0000;
    const NAME_PAYLOAD: usize = 0x3100_0000;
    const PORTRAITS: usize = 0x3200_0000;
    const GFX_PAYLOAD: usize = 0x3300_0000;
    const DB: usize = 0x4000_0000;
    const DB_DATA: usize = 0x4100_0000;
    let mut pieces = Vec::new();
    let mut bytes = Vec::new();
    let mut put = |addr: usize, data: &[u8]| {
        pieces.push(Domain47WitnessRecord {
            addr,
            len: data.len() as u32,
            off: bytes.len() as u32,
        });
        bytes.extend_from_slice(data);
    };
    // manager window [0, 0x60): name index header at +0x40 (buckets ptr +0x48, mask +0x54 = 3)
    let mut m = vec![0u8; 0x60];
    m[0x48..0x50].copy_from_slice(&(BUCKETS as u64).to_le_bytes());
    m[0x54..0x58].copy_from_slice(&3u32.to_le_bytes());
    // character body: name header at +0x28 (ptr, size 20, cap 31 -> heap), portrait map data +0x80, count +0x8C = 2
    let mut c = vec![0u8; domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN];
    c[0x28..0x30].copy_from_slice(&(NAME_PAYLOAD as u64).to_le_bytes());
    c[0x38..0x40].copy_from_slice(&20u64.to_le_bytes());
    c[0x40..0x48].copy_from_slice(&31u64.to_le_bytes());
    c[0x80..0x88].copy_from_slice(&(PORTRAITS as u64).to_le_bytes());
    c[0x8C..0x90].copy_from_slice(&2u32.to_le_bytes());
    // two portrait entries of 0x38; entry 1's gfx string (+0x10) on the heap
    let mut p = vec![0u8; 2 * domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_LEN];
    let e1 = domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_LEN + domain47::extractor::CCHARACTER_PORTRAIT_ENTRY_GFX_OFF;
    p[e1..e1 + 8].copy_from_slice(&(GFX_PAYLOAD as u64).to_le_bytes());
    p[e1 + 0x10..e1 + 0x18].copy_from_slice(&40u64.to_le_bytes());
    p[e1 + 0x18..e1 + 0x20].copy_from_slice(&47u64.to_le_bytes());
    // trait database: image root -> DB; DB data ptr +0x58, size +0x64 = 5
    let mut d = vec![0u8; DOMAIN47_UNIT_LEADER_DATABASE_SIZE_OFF + 8];
    d[DOMAIN47_UNIT_LEADER_DATABASE_DATA_OFF..DOMAIN47_UNIT_LEADER_DATABASE_DATA_OFF + 8]
        .copy_from_slice(&(DB_DATA as u64).to_le_bytes());
    d[DOMAIN47_UNIT_LEADER_DATABASE_SIZE_OFF..DOMAIN47_UNIT_LEADER_DATABASE_SIZE_OFF + 4]
        .copy_from_slice(&5u32.to_le_bytes());
    // address-sorted, as the seal leaves them
    put(BUCKETS, &[0u8; 4 * DOMAIN47_MANAGER_NAME_INDEX_BUCKET_STRIDE]);
    put(MANAGER, &m);
    put(CHARACTER, &c);
    put(NAME_PAYLOAD, &[b'x'; 21]);
    put(PORTRAITS, &p);
    put(GFX_PAYLOAD, &[b'g'; 41]);
    put(DB, &d);
    put(DB_DATA, &[0u8; 40]);
    put(BASE + DOMAIN47_UNIT_LEADER_DATABASE_PTR_RVA, &(DB as u64).to_le_bytes());
    let mut cache = adopted_cache_with_witness();
    cache.roots.characters[0].character_addr = CHARACTER as u64;
    let witness = cache.witness.as_mut().expect("fixture witness");
    witness.base = BASE;
    witness.manager = MANAGER;
    witness.pieces = pieces;
    witness.bytes = bytes;
    witness.spans = Vec::new();
    cache
}

/// kills: dropping the bucket-array / payload / entry-array / db arms of
/// `domain47_witness_miss_owner` turns the named owners back into
/// "unknown" or "near_*" (the pre-MX locator's answer for t82-t84's misses).
#[test]
fn the_miss_locator_names_owners_from_the_witness_bytes_alone() {
    let cache = witness_with_pointers();
    let cases: [(u64, &str, u64, u64); 12] = [
        (0x1000_0000 + 0x18 + 0x10, "name_index_bucket_array", 0x1000_0000, 0x28),
        (0x2000_0000 + 0x50, "manager", 0x2000_0000, 0x50),
        (0x3000_0000 + 0x0C, "character", 0x3000_0000, 0x0C),
        (0x3100_0000 + 5, "character_name_payload", 0x3100_0000, 5),
        (0x3200_0000 + 0x38 + 4, "portrait_entry_array", 0x3200_0000, 0x3C),
        (0x3300_0000 + 7, "portrait_gfx_payload", 0x3300_0000, 7),
        (0x4000_0000 + 0x64, "trait_db_object", 0x4000_0000, 0x64),
        (0x4100_0000 + 16, "trait_db_data_array", 0x4100_0000, 16),
        (0x7ff6_0000_0000 + DOMAIN47_UNIT_LEADER_DATABASE_PTR_RVA as u64, "image", 0x7ff6_0000_0000, DOMAIN47_UNIT_LEADER_DATABASE_PTR_RVA as u64),
        // beyond the 4-bucket array: nothing owns it and nothing is within 0x2000 below
        (0x1000_0000 + 4 * DOMAIN47_MANAGER_NAME_INDEX_BUCKET_STRIDE as u64 + 0x4000, "unknown", 0, 0),
        // just past the character body: the nearest-root fallback names it
        (0x3000_0000 + domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN as u64 + 8, "near_character", 0x3000_0000, domain47::extractor::CCHARACTER_RAW_FIELDS_SPAN as u64 + 8),
        (0x3300_0000 + 41, "unknown", 0, 0), // one past the gfx payload's NUL
    ];
    for (addr, owner, owner_addr, off) in cases {
        assert_eq!(
            domain47_witness_miss_owner(&cache, addr),
            (owner, owner_addr, off),
            "addr 0x{addr:x}"
        );
    }
    let mut no_witness = witness_with_pointers();
    no_witness.witness = None;
    assert_eq!(domain47_witness_miss_owner(&no_witness, 0x2000_0000), ("no_witness", 0, 0));
    // A heap object INSIDE the 256 MiB above the image base is still named by
    // its precise range, not as "image" (Codex batch 2 finding 5).
    let mut low_image = witness_with_pointers();
    low_image.witness.as_mut().unwrap().base = 0x2000_0000 - 0x10;
    assert_eq!(
        domain47_witness_miss_owner(&low_image, 0x2000_0000 + 0x50),
        ("manager", 0x2000_0000, 0x50)
    );
    assert_eq!(
        domain47_witness_miss_owner(&low_image, 0x2000_0000 - 0x10 + 0x0800_0000),
        ("image", 0x2000_0000 - 0x10, 0x0800_0000),
        "an address the image window covers and no object owns is image"
    );
}

/// The witness readers never leave a piece: a word straddling a piece end is
/// `None`, and an SSO string (capacity < 16) has no payload.
#[test]
fn witness_readers_stay_inside_pieces() {
    let cache = witness_with_pointers();
    let w = cache.witness.as_ref().unwrap();
    assert_eq!(domain47_witness_u64(w, 0x2000_0000 + 0x48), Some(0x1000_0000));
    assert_eq!(domain47_witness_u64(w, 0x2000_0000 + 0x5C), None, "straddles the manager piece end");
    assert_eq!(domain47_witness_u64(w, 0x1234), None);
    // the token string at character+0x48 is all zero: capacity 0 => SSO => no payload
    assert_eq!(domain47_witness_string_payload(w, 0x3000_0000 + 0x48), None);
    assert_eq!(domain47_witness_string_payload(w, 0x3000_0000 + 0x28), Some((0x3100_0000, 21)));
}

/// The miss-note's piece containment is strict: a piece's final byte is owned,
/// the byte at `piece.addr + piece.len` (one past the end) belongs to NO piece,
/// and an address before the first piece resolves to none (Codex 01a00f9e b2
/// R3 finding 3, pinned per b2 R4 finding 2 on the extracted pure helper the
/// miss note calls).
///
/// kills: reverting the containment to `<=` (the one-past-the-end byte would
/// claim the piece with `piece_off == piece_len` — exactly the R3 defect), or
/// `partition_point` over `<` (a piece's own first byte would stop resolving).
#[test]
fn miss_note_piece_containment_excludes_one_past_the_end() {
    let pieces = [
        Domain47WitnessRecord { addr: 0x1000, len: 0x20, off: 0 },
        Domain47WitnessRecord { addr: 0x2000, len: 0x10, off: 0x20 },
    ];
    assert_eq!(
        domain47_witness_miss_piece(&pieces, 0x1000),
        Some((0x1000, 0x20, 0)),
        "a piece's first byte resolves to it"
    );
    assert_eq!(
        domain47_witness_miss_piece(&pieces, 0x101F),
        Some((0x1000, 0x20, 0x1F)),
        "a piece's final byte is owned"
    );
    assert_eq!(
        domain47_witness_miss_piece(&pieces, 0x1020),
        None,
        "one past the piece end is NOT contained"
    );
    assert_eq!(domain47_witness_miss_piece(&pieces, 0x0FFF), None, "before every piece");
    assert_eq!(domain47_witness_miss_piece(&pieces, 0x200F), Some((0x2000, 0x10, 0xF)));
    assert_eq!(
        domain47_witness_miss_piece(&pieces, 0x2010),
        None,
        "one past the LAST piece end is NOT contained"
    );
    assert_eq!(domain47_witness_miss_piece(&[], 0x1000), None, "no witnessed pieces");
}

/// Step 2b: sweep-only trait hits are not read only when the skip is licensed
/// (readable database AND every admitted leader's trait pointers are database
/// slots); otherwise every hit is read.
///
/// kills: `domain47_leader_trait_refs_covered` treating `None` refs as
/// covered, or dropping the `db_readable` term, or the loop calling
/// `domain47_trait_hit_is_read` with `trait_db_readable` instead of the
/// joined licence (the structural half below pins the call).
#[test]
fn sweep_only_trait_hits_are_skipped_only_behind_a_proven_leader_join() {
    let slots = [0x1000u64, 0x2000, 0x3000];
    // the predicate
    assert!(domain47_trait_hit_is_read(true, &slots, 0x2000), "a database hit is read");
    assert!(!domain47_trait_hit_is_read(true, &slots, 0x2500), "a sweep-only hit is not read");
    assert!(domain47_trait_hit_is_read(false, &slots, 0x2500), "no licence: every hit is read");
    assert!(domain47_trait_hit_is_read(false, &[], 0x1), "no licence, no slots: every hit is read");
    // the licence
    assert!(domain47_leader_trait_refs_covered(true, &slots, Some(&[0x1000, 0x3000])));
    assert!(domain47_leader_trait_refs_covered(true, &slots, Some(&[])), "leaders holding no traits: covered");
    assert!(
        !domain47_leader_trait_refs_covered(true, &slots, Some(&[0x1000, 0x2500])),
        "a leader referencing a non-database trait: NOT covered (Codex batch 2 finding 2's frame)"
    );
    assert!(!domain47_leader_trait_refs_covered(true, &slots, None), "a torn vector: not covered");
    assert!(!domain47_leader_trait_refs_covered(false, &slots, Some(&[])), "unreadable database: not covered");
    assert!(!domain47_leader_trait_refs_covered(true, &[], Some(&[0x1000])), "empty database: not covered");
    assert!(
        !domain47_leader_trait_refs_covered(true, &[], Some(&[])),
        "empty database, leaders holding nothing: NOT covered (the vacuous subset would skip every          hit and publish trait_count 0 clean; Codex batch 2 R2 finding 2)"
    );
    // finding-2 arm (Codex b2 R3): the summary flag records that a hit WAS
    // skipped, never merely that skipping was licensed — a licensed pass that
    // skipped nothing reads exactly what an unlicensed one reads, so a missing
    // token there must not trigger the retry. Pinned structurally: the flag is
    // assigned from the skipped-hit COUNT.
    // the structural half: the loop consults the JOINED licence, and the join
    // is built from this frame's leader reads
    // NORMALISE THE LINE ENDINGS BEFORE SCANNING. The two `find("\n}\n")` end
    // markers below are multi-line string literals, and rustc normalises CRLF
    // to LF inside a literal when it reads this source file -- so the needle is
    // always LF-only. `read_to_string` does NOT normalise: on a CRLF checkout
    // (which this repo is -- attach_log.rs is 147,606 CRLF lines) the haystack
    // holds "\r\n}\r\n", the needle never matches, and the scan panicked on
    // `.expect("closed")`. Normalising here makes the scan agnostic to how the
    // file landed on disk instead of passing only on an LF checkout.
    let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/attach_log.rs"))
        .expect("attach_log.rs readable")
        .replace("\r\n", "\n");
    let start = source
        .find("fn prewarm_compact_characters_leaders_cabinet_roots(")
        .expect("prewarm declared");
    let body = &source[start..start + source[start..].find("
}
").expect("closed")];
    assert!(
        body.contains("let sweep_only_hits_skippable = trait_db_readable && leader_trait_refs_covered && !skip_suspended;")
            && body.contains("domain47_trait_hit_is_read(sweep_only_hits_skippable, &trait_db_slots, addr as u64)")
            && body.contains("domain47_leader_trait_refs_covered(")
            && body.contains("domain47_read_leader_trait_refs(leader_addr_usize)")
            && body.contains("trait_sweep_only_skipped: trait_sweep_only_skipped_hits != 0,"),
        "the sweep-only skip must be licensed by the leader join built from this discovery's reads,          and the retry flag must record an ACTUAL skip (count != 0), not the licence"
    );
    // The RETRY binds the licence to the walk's bytes: a skipping pass whose
    // walk reports a missing trait token, or whose skip left the token map
    // empty (the adapter then reads no vector at all), is discarded unread and
    // rerun with the skip suspended (kills: deleting the retry arm, or either
    // of its two triggers, in `observe_compact_characters_leaders_cabinet_live`).
    let observe_start = source
        .find("fn observe_compact_characters_leaders_cabinet_live(")
        .expect("observe half declared");
    let observe = &source[observe_start..observe_start + source[observe_start..].find("
}
").expect("closed")];
    assert!(
        observe.contains("&& prewarm.trait_sweep_only_skipped")
            && observe.contains("(observation.missing_trait_tokens() != 0 || prewarm.trait_skip_left_empty_map)")
            && observe.contains("cache.trait_skip_suspended = true;")
            && observe.contains("cache.witness = None;"),
        "a skipping pass that cannot prove its rows is discarded and rerun without the skip"
    );
}
