//! Gates for D13 `tech_live`'s keyed per-country patch.
//!
//! D13 was the one country-bundle domain that re-sent every country every hour.
//! The others suppress an unchanged country upstream, in
//! `compact_country_changed_domains`, by comparing a per-country memory
//! fingerprint. D13 runs that same machinery and it never fires, because
//! `research_slots_fingerprint` mixes the CURRENT GAME HOUR into the hash — a
//! term that is right for the job the fingerprint was built for (it is
//! `read_country_research_cached`'s key, and the row carries hour-dependent
//! live projections) and wrong for the question it was then also asked, which
//! is whether the country's WIRE BYTES changed.
//!
//! Measured over `runs/live_unlock/t26/collector_stream.bin`
//! (2,594,412,495 bytes): all 439 countries in all 285 D13 records, 6,737,040
//! payload bytes, while only 2,802 of 125,115 per-country blocks (2.25%)
//! differed from the block sent before them.
//!
//! The questions this module answers, each written so a production change turns
//! it red:
//!
//! 1. **Did the refactor move a byte?** `compact_tech_data_payload` now builds
//!    from `CompactTechCountryBlock`s. `the_block_serialiser_is_byte_identical_
//!    to_the_payload_builder_it_replaced` holds it against a FROZEN
//!    transcription of the previous whole-payload builder.
//! 2. **Does a patch carry what a whole payload would have carried?**
//!    `a_patch_reconstructs_exactly_the_blocks_the_whole_payload_carries` runs a
//!    reference implementation beside production over randomized frame
//!    sequences and compares the reconstructed block map after every frame.
//! 3. **Is absence read as deletion?** It must not be:
//!    `an_unmeasured_country_is_neither_emitted_nor_removed`, plus the two
//!    fail-closed paths beside it.
//! 4. **Is the key sound?**
//!    `a_repeated_key_forces_a_whole_payload_instead_of_an_inconsistent_patch`.
//! 5. **Does a failed read stay out of the baseline?**
//!    `a_withheld_baseline_emits_whole_and_unseeds_the_pin`.

use super::*;

// ---------------------------------------------------------------------
// The frozen reference: the payload builder as it stood before the block
// refactor, transcribed verbatim.
// ---------------------------------------------------------------------

/// `compact_tech_data_payload` exactly as it was written before
/// `CompactTechCountryBlock` existed, kept here so the refactor has something
/// to be equal to. Nothing may "fix" this copy: its whole value is that it is
/// the old code.
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

// ---------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------

fn research_slot(slot_index: u32, progress_raw: u32, named: bool) -> CountryResearchSlotRow {
    CountryResearchSlotRow {
        slot_index,
        tech_handle_raw: 1000 + slot_index,
        tech_def_index_raw: 2000 + slot_index,
        progress_raw,
        tech_name: named.then(|| format!("tech_{slot_index}")),
        progress_units: progress_raw as f64 / TECH_PROGRESS_SCALE_RAW as f64,
        // The derived `_units` fields never reach the wire. They are given
        // deliberately DIFFERENT values from their `_raw` partners in the
        // reference frames below, which is how the tests prove the diff is over
        // the emitted bytes and not over the row.
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
    }
}

/// A bare country row. Local rather than borrowed from the sibling `tests`
/// module: its constructor is private to that module, and widening it would
/// couple this file to a file every other lane also edits.
fn bare_country_row(tag_db_id: u32) -> CountryObjectRow {
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

/// One country whose research block is a deterministic function of `seed`.
fn tech_row(tag_db_id: u32, seed: u32) -> CountryObjectRow {
    let mut row = bare_country_row(tag_db_id);
    row.research = Some(CountryResearchRow {
        completed_tech_count: (seed % 5 != 0).then_some(seed),
        research_slot_count: Some(1 + seed % 3),
        active_research: (seed % 4 != 0).then(|| {
            (0..(seed % 3))
                .map(|index| research_slot(index, seed * 17 + index, seed % 2 == 0))
                .collect()
        }),
    });
    row
}

/// A country with no research at all: it never reaches D13's payload.
fn researchless_row(tag_db_id: u32) -> CountryObjectRow {
    bare_country_row(tag_db_id)
}

// ---------------------------------------------------------------------
// The decoder the tests compare through: `tag_db_id -> that country's wire
// bytes`, read back out of a payload exactly as `fold_tech` reads it.
// ---------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Clone)]
struct DecodedBlock {
    country_row: Vec<u8>,
    active_rows: Vec<Vec<u8>>,
}

/// Decode a D13 payload into its per-country blocks.
///
/// Panics rather than returns on a malformed payload: every payload here comes
/// from production, so a malformed one is a failure and not a case to handle.
fn decode_tech_payload(payload: &[u8]) -> BTreeMap<u32, DecodedBlock> {
    let read_u32 = |at: usize| u32::from_le_bytes(payload[at..at + 4].try_into().unwrap());
    let country_count = read_u32(8) as usize;
    let active_count = read_u32(28) as usize;

    let mut live: BTreeMap<u32, DecodedBlock> = BTreeMap::new();
    let mut cursor = 32usize;
    for _ in 0..country_count {
        let tag = read_u32(cursor);
        let flags = read_u32(cursor + 4);
        assert_eq!(
            flags & !COMPACT_TECH_COUNTRY_KNOWN_FLAGS,
            0,
            "country {tag} carries a flag no consumer accepts"
        );
        let row = payload[cursor..cursor + 16].to_vec();
        cursor += 16;
        assert!(
            live.insert(
                tag,
                DecodedBlock {
                    country_row: row,
                    active_rows: Vec::new(),
                },
            )
            .is_none(),
            "a payload carries country {tag} twice"
        );
    }
    for _ in 0..active_count {
        let start = cursor;
        let tag = read_u32(cursor);
        let flags = read_u32(cursor + 20);
        cursor += 40;
        if flags & (1 << 2) != 0 {
            let len = u16::from_le_bytes(payload[cursor..cursor + 2].try_into().unwrap()) as usize;
            cursor += 2 + len;
        }
        if flags & (1 << 3) != 0 {
            cursor += 32;
        }
        if flags & (1 << 4) != 0 {
            cursor += 176;
        }
        live.get_mut(&tag)
            .unwrap_or_else(|| panic!("active row references country {tag}, absent from the header"))
            .active_rows
            .push(payload[start..cursor].to_vec());
    }
    assert_eq!(cursor, payload.len(), "the payload has trailing bytes");
    live
}

// ---------------------------------------------------------------------
// 1. The refactor moved no byte
// ---------------------------------------------------------------------

/// What production change makes this fail: any reordering, re-flagging or
/// re-sizing inside `compact_tech_country_block`,
/// `append_compact_tech_active_slot_row` or
/// `compact_tech_payload_from_blocks`. The frozen copy above is what those
/// three have to keep agreeing with.
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
        assert_eq!(
            compact_tech_data_payload(&rows, frame as u64, status),
            frozen_tech_data_payload(&rows, frame as u64, status),
            "frame {frame}: the block serialiser and the builder it replaced disagree"
        );
    }
}

// ---------------------------------------------------------------------
// 2. A patch carries what a whole payload would have carried
// ---------------------------------------------------------------------

/// The reference fold: `fold_tech`'s per-tag merge, written plainly.
#[derive(Default)]
struct ReferenceTechFold {
    world: BTreeMap<u32, DecodedBlock>,
}

impl ReferenceTechFold {
    /// Apply what production emitted, the way `fold_tech` does — merge every
    /// carried tag, touch nothing else — and return the world it produces.
    fn apply(&mut self, payload: &[u8]) -> BTreeMap<u32, DecodedBlock> {
        for (tag, block) in decode_tech_payload(payload) {
            self.world.insert(tag, block);
        }
        self.world.clone()
    }
}

/// **The differential gate.** Over randomized frame sequences — countries
/// changing, staying still, and going UNMEASURED — the world the patch stream
/// folds to must equal the world the whole-payload stream folds to, after EVERY
/// frame.
///
/// The whole-payload arm is the real `compact_tech_data_payload` rather than a
/// hand-written shape, so neither arm can drift into agreeing about something
/// the producer cannot emit.
///
/// What production change makes this fail: dropping the carry-forward in
/// `compact_tech_delta_blocks` — an unmeasured country would be handed to the
/// shared engine as absent, its `deleted_row_fn` would fire, and the arms part
/// immediately — emitting an unchanged country as changed and then losing it
/// from the pin, advancing the pin to a set the payload did not carry, or any
/// disagreement between the diff's key and the payload's own `tag_db_id`.
#[test]
fn a_patch_folds_to_the_same_world_as_the_whole_payload_every_frame() {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let mut patched_fold = ReferenceTechFold::default();
    let mut whole_fold = ReferenceTechFold::default();
    // A cheap deterministic PRNG: the sequence has to be reproducible from the
    // test's own text, so a failure is re-runnable.
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    let mut patches = 0usize;
    for frame in 0..300u32 {
        // The ROSTER is fixed — for D13 countries do not come and go, because
        // `read_country_research` walks the country DB whose membership D18
        // replaces. What varies is whether each country's research could be
        // SAMPLED this frame.
        let mut rows = Vec::new();
        for tag in 1..=12u32 {
            let roll = next() % 100;
            if roll < 15 {
                // Unmeasured: in the roster, no research. This is the case the
                // encoder must not read as a deletion.
                rows.push(bare_country_row(tag));
                continue;
            }
            let seed = if roll < 60 { tag } else { tag + frame };
            rows.push(tech_row(tag, seed));
        }
        let patched = compact_tech_data_payload_delta(&rows, frame as u64, 0, &mut tracker);
        let whole = compact_tech_data_payload(&rows, frame as u64, 0);
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

// ---------------------------------------------------------------------
// 3. Absence is not deletion
// ---------------------------------------------------------------------

/// The load-bearing property: a country the frame did not MEASURE is neither
/// emitted nor removed, and the pin keeps the block the consumer already holds.
///
/// What production change makes this fail: dropping the `pin.get` carry-forward
/// in `compact_tech_delta_blocks`. The shared engine would then find country 2
/// missing from the frame and call `deleted_row_fn`, whose `debug_assert!(false)`
/// fires in a debug build and whose empty block corrupts the payload in a
/// release one.
#[test]
fn an_unmeasured_country_is_neither_emitted_nor_removed() {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let seeded = compact_tech_data_payload_delta(
        &[tech_row(1, 1), tech_row(2, 2), tech_row(3, 3)],
        0,
        0,
        &mut tracker,
    );
    assert_eq!(
        decode_tech_payload(&seeded).len(),
        3,
        "the first payload seeds the pin whole"
    );

    // Country 2 is still in the roster but its research did not read.
    let patched = compact_tech_data_payload_delta(
        &[tech_row(1, 1), bare_country_row(2), tech_row(3, 3)],
        1,
        0,
        &mut tracker,
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

    // The pin still holds country 2's block, so re-measuring it at the same
    // value must still emit nothing.
    let again = compact_tech_data_payload_delta(
        &[tech_row(1, 1), tech_row(2, 2), tech_row(3, 3)],
        2,
        0,
        &mut tracker,
    );
    assert!(
        decode_tech_payload(&again).is_empty(),
        "the pin lost the unmeasured country, so it was re-sent when it reappeared"
    );
}

/// A key that leaves the ROSTER is a real removal, and D13 has no grammar for
/// one — so the encoder falls back to a whole payload and re-seeds rather than
/// inventing a tombstone or silently dropping the key.
///
/// What production change makes this fail: removing the `pin.keys` containment
/// check from `compact_tech_delta_blocks`. The engine would then tombstone the
/// departed key through a `deleted_row_fn` that exists only to assert it is
/// never called.
#[test]
fn a_key_leaving_the_roster_forces_a_whole_payload_and_re_seeds() {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let _ = compact_tech_data_payload_delta(
        &[tech_row(1, 1), tech_row(2, 2), tech_row(3, 3)],
        0,
        0,
        &mut tracker,
    );

    // Country 3 is gone from the roster entirely — not present-and-unmeasured.
    let payload =
        compact_tech_data_payload_delta(&[tech_row(1, 1), tech_row(2, 2)], 1, 0, &mut tracker);
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

/// A repeated key makes the record's meaning depend on which occurrence wins,
/// and the shared engine's answer is not self-consistent for D13: it compares
/// EVERY occurrence against the old baseline but pins the LAST, so a changed
/// first occurrence followed by an unchanged second emits the first and pins
/// the second. The encoder refuses to patch such a frame at all.
///
/// (Measured on t26: 439 distinct `tag_db_id`s, unique within every one of the
/// 285 records, and the set neither grew nor shrank across 555 game-hours. This
/// is the fail-closed path for a reader that ever breaks that.)
///
/// What production change makes this fail: dropping the `seen` insert in
/// `compact_tech_delta_blocks`. The frame below then emits seed 5's block while
/// pinning seed 2's, and the next unchanged frame re-emits it.
#[test]
fn a_repeated_key_forces_a_whole_payload_instead_of_an_inconsistent_patch() {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let _ = compact_tech_data_payload_delta(&[tech_row(1, 1), tech_row(2, 2)], 0, 0, &mut tracker);

    // The exact ordering the shared engine cannot represent: the FIRST
    // occurrence differs from the pin, the LAST equals it.
    let payload = compact_tech_data_payload_delta(
        &[tech_row(1, 1), tech_row(2, 5), tech_row(2, 2)],
        1,
        0,
        &mut tracker,
    );
    assert_eq!(
        u32::from_le_bytes(payload[16..20].try_into().unwrap()) & COMPACT_STATUS_DELTA_DATA,
        0,
        "a repeated key must force a whole payload, not a patch whose pin and payload disagree"
    );
    assert!(!tracker.baseline_seeded, "the pin is dropped, not advanced");
}

// ---------------------------------------------------------------------
// 5. A failed read stays out of the baseline
// ---------------------------------------------------------------------

/// A read that did not cover its whole extent may not become the reference: a
/// later patch against it would speak for the countries it merely failed to
/// reach. `fold_tech` applies the record anyway, so the pin is dropped outright
/// rather than advanced, and the next frame is a keyframe — the only emission
/// that needs no agreement about what came before.
///
/// What production change makes this fail: removing the
/// `compact_delta_baseline_withheld` arm, or making it advance the pin instead
/// of clearing it. Either way the frame after the failure would be a patch
/// against a baseline the fold does not hold.
#[test]
fn a_withheld_baseline_emits_whole_and_unseeds_the_pin() {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let _ = compact_tech_data_payload_delta(&[tech_row(1, 1), tech_row(2, 2)], 0, 0, &mut tracker);
    assert!(tracker.baseline_seeded, "the first clean read seeds the pin");

    // A torn read that reached only one country.
    let torn = compact_tech_data_payload_delta(
        &[tech_row(1, 1)],
        1,
        COMPACT_STATUS_PARTIAL_DATA,
        &mut tracker,
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

    // The next clean frame must therefore be a whole payload again.
    let after = compact_tech_data_payload_delta(&[tech_row(1, 1), tech_row(2, 2)], 2, 0, &mut tracker);
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

// ---------------------------------------------------------------------
// 6. The instrument: what the patch costs the sweep
// ---------------------------------------------------------------------

/// t26's country count, which is the per-frame row count that decides D13's
/// verdict under §4.5.
const D13_T26_COUNTRY_COUNT: u32 = 439;

/// Best-of-5, because this is a shared machine and the interesting quantity is
/// the floor, not the mean of whatever else was running.
const D13_BENCH_TRIALS: usize = 5;

/// An INSTRUMENT, not a gate — no assertion here is about wall clock, because a
/// timing assertion on a fixture this small measures the machine and not the
/// code. Run it with
/// `cargo test -p collector --release -- --ignored --nocapture d13_tech_delta_bench`.
///
/// It reports the whole cost of D13's emission on both arms — serialise-only
/// versus serialise-and-diff — at t26's 439 countries, with 2.25% of them
/// changed, which is t26's measured rate.
#[test]
#[ignore = "instrument, not a gate; run with --ignored --nocapture"]
fn d13_tech_delta_bench() {
    let seed_rows: Vec<CountryObjectRow> = (1..=D13_T26_COUNTRY_COUNT)
        .map(|tag| tech_row(tag, tag))
        .collect();
    // 2.25% changed, which is what t26 measured.
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
    let _ = compact_tech_data_payload_delta(&seed_rows, 0, 0, &mut tracker);

    let mut whole_best = Duration::from_secs(3600);
    let mut patch_best = Duration::from_secs(3600);
    let mut whole_bytes = 0usize;
    let mut patch_bytes = 0usize;
    for _ in 0..D13_BENCH_TRIALS {
        let started = Instant::now();
        let whole = compact_tech_data_payload(&frame_rows, 0, 0);
        whole_best = whole_best.min(started.elapsed());
        whole_bytes = whole.len();

        // A fresh pin each trial, so every trial times the same work.
        let mut trial_tracker = CompactTechCountryDeltaTracker::default();
        let _ = compact_tech_data_payload_delta(&seed_rows, 0, 0, &mut trial_tracker);
        let started = Instant::now();
        let patch = compact_tech_data_payload_delta(&frame_rows, 0, 0, &mut trial_tracker);
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

/// The seed is whole and unflagged; every later clean frame is a patch and says
/// so. Without this, an encoder that never set `DELTA_DATA` would pass every
/// test above while the fold's `reject_delta_before_baseline` protected
/// nothing.
#[test]
fn the_seed_is_whole_and_every_later_clean_frame_announces_itself_as_a_patch() {
    let mut tracker = CompactTechCountryDeltaTracker::default();
    let seed = compact_tech_data_payload_delta(&[tech_row(1, 1)], 0, 0, &mut tracker);
    assert_eq!(
        u32::from_le_bytes(seed[16..20].try_into().unwrap()) & COMPACT_STATUS_DELTA_DATA,
        0,
        "the seeding keyframe must not carry DELTA_DATA"
    );
    for frame in 1..5u64 {
        let patch = compact_tech_data_payload_delta(&[tech_row(1, 1)], frame, 0, &mut tracker);
        assert_ne!(
            u32::from_le_bytes(patch[16..20].try_into().unwrap()) & COMPACT_STATUS_DELTA_DATA,
            0,
            "frame {frame} is a patch and must say so, or the fold cannot refuse one that \
             arrives before it has a baseline"
        );
    }
}
