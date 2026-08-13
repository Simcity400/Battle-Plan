//! What d81's observation phase would cost as a COPY PLAN, and what a copy
//! plan cannot reach.
//!
//! # Why this file exists
//!
//! S4 of the capture-floor plan converts a reader's clock bracket into bulk
//! copies only: the bracket copies bytes and decodes addresses and lengths,
//! and every validation, comparison and row assembly runs afterwards against
//! the copied image. d81 (`rail_replay_state_live`) is one of the two domains
//! that own the frame's critical lane, so it is the conversion that decides
//! whether S4 pays.
//!
//! Lane EK already moved d81's *compute* out of the bracket
//! (`d81_bracket_split.rs`). What is left inside is observation: guarded reads
//! and their witnesses. EK's own closing note said the remaining span is
//! "data-dependent discovery whose addresses come from values it reads", and
//! that copy-only "requires over-reading". This file is the arithmetic behind
//! that sentence: which of d81's read sites a copy plan can BATCH, which it
//! cannot, and what the two shapes cost against the same primitives.
//!
//! # The claim being measured, stated so it can be falsified
//!
//! d81's in-bracket cost is driven by the NUMBER of guarded calls far more
//! than by the number of bytes. The recorded evidence for that is lane BG's
//! transcription of `d81_guarded_read_cost_model`'s output, quoted in
//! `attach_log.rs` beside the rail-header coalescing it justified: `read_u64` +
//! `read_u32` = 179.74 / 174.15 ns per visit over two runs, one 16-byte
//! `read_into` = 85.34 / 80.79, and a 168-byte `read_into` = 100.04 / 110.31.
//! So ~80-90 ns is the fixed `range_confirmed_good` + `copy_selected` cost and
//! width is nearly free up to at least 168 bytes. (The bench itself prints and
//! records nothing; those numbers are its transcribed output, not an assertion
//! anything re-checks.)
//!
//! A copy plan is therefore worth roughly the calls it removes, MINUS what its
//! extra bytes cost — and it does move far more bytes, so "worth the calls"
//! is a first-order claim and the bench below is what prices the remainder.
//!
//! It removes calls in one place and one place only: **reads indexed into a
//! contiguous engine table**. Those become one copy of the whole table. Reads
//! into SCATTERED heap objects — a rail-info record, a map province, a
//! CProvince — cannot be batched by any plan, because the objects are separate
//! allocations at unrelated addresses; the plan issues the same guarded calls
//! the live reader issues, at the same widths, with ONE exception: two scalars
//! inside the SAME object can be merged into one window, which is what the plan
//! does to the CProvince vtable and its id at +0xA4.
//!
//! # Scope: the province walk, because the hub half is already a copy plan
//!
//! The census below covers `read_compact_rail_replay_state`'s per-province
//! walk and NOT `read_compact_rail_hub_simulation`. That is not an omission,
//! it is the second finding: the hub's expensive half is ALREADY copy-only.
//! `d81_hash_table_rows` computes a whole-table extent from the engine's own
//! header (`slot_count = mask + max_probe + 2`, bucket stride 0x18) and takes
//! it in ONE bulk read through `d81_read_stable_bytes`, which copies the whole
//! blob twice back to back and compares the two images before anything is
//! decoded from them; the bucket scan then runs over the copied bytes. That is
//! precisely the shape S4 asks for — a bounded whole-extent copy taken as an
//! adjacent A/B pair, decoded afterwards — hand-built inside d81 and in
//! production today. A copy plan has nothing to win there because the win is
//! already taken.
//!
//! # What this file is not
//!
//! It is NOT a conversion, and it does not claim the conversion is correct.
//! `the_copy_plan_covers_every_range_the_live_reader_reads` proves one
//! necessary condition — the plan's ranges are a superset of the live
//! reader's — and nothing about refusal vocabulary, witness provenance or
//! elision semantics, which is where the substrate gaps are.
//!
//! AND BOTH NON-IGNORED TESTS CONSTRAIN THIS FILE'S MODEL, NOT THE READER. No
//! edit to `read_compact_rail_replay_state` can fail either of them; the census
//! is a hand-derived transcription of that function's read sites and nothing
//! machine-checks the transcription. What the two tests buy is narrower and
//! worth stating exactly: the published census and the published plan cannot
//! drift apart without failing, a whole-table copy cannot silently stop
//! spanning the table it stands in for, and the file cannot quietly stop being
//! a call reduction. The transcription itself is checked by reading it against
//! the reader, and the row-by-row citations exist for that.
//!
//! It also cannot be run against the game: d81 has no end-to-end fixture (see
//! the head of `d81_frame_cost.rs` for why). Both arms therefore run over a
//! synthetic pool at a shape that is PARTLY measured and partly assumed: the
//! province and railed-province counts are off the capture wire, while the
//! neighbour count, the selector-table size, the probe depth, the table extent
//! and the self-keyed share are declared assumptions, each labelled as one at
//! its constant. Where the fixture understates and overstates is stated at
//! [`d81_copy_bracket_cost_model`].

use super::*;
use std::time::Instant;

// ---------------------------------------------------------------------------
// The shape. Every number here is measured or is declared as an assumption.
// ---------------------------------------------------------------------------

/// d81's per-frame shape, from the capture wire.
pub(crate) mod shape {
    /// `d81_phase_provinces_visited`, t31. Every province the topology names;
    /// each one costs at least the rail-table slot read.
    pub(crate) const PROVINCES: usize = 13_413;

    /// `d81_phase_provinces_railed`, t26 ("t26's 3510 railed provinces", cited
    /// at `attach_log.rs` in the rail-header coalescing comment). Everything
    /// after the `rail_info == 0` continue is paid once per RAILED province.
    pub(crate) const RAILED: usize = 3_510;

    /// ASSUMPTION, not a measurement. `observed_neighbors` is on the wire but
    /// no run's value is on disk in this tree, so the pair blob is priced at
    /// four rail neighbours per railed province. The blob is
    /// `pair_count * 8` bytes in two reads; at 4 that is 32 bytes, and the
    /// call count — which is what dominates — does not depend on it at all.
    /// The engine's own ceiling is
    /// `compact_contract::rail_replay::MAX_NEIGHBORS_PER_PROVINCE` = 64.
    pub(crate) const NEIGHBORS_PER_RAILED: usize = 4;

    /// ASSUMPTION, not a measurement. The anchor-selector map's mask is read
    /// from the engine at `selector_map + 0x34` and is not recoverable
    /// offline. 16,384 entries is the smallest power of two that holds
    /// `PROVINCES` keys at a load factor under 0.82; a larger table makes the
    /// copy plan's single table copy WIDER (more bytes) without changing its
    /// call count, so this assumption is the one that flatters the plan and
    /// the bench reports a doubled variant beside it.
    pub(crate) const SELECTOR_ENTRIES: usize = 16_384;

    /// `select_secondary`'s entry stride, from the reader.
    pub(crate) const SELECTOR_STRIDE: usize = 0x18;

    /// ASSUMPTION, and the one that a real conversion must NOT inherit.
    ///
    /// [`PROVINCES`] counts the roots the topology walk VISITED. The rail,
    /// active and province-array tables are indexed by `province_id`, and
    /// nothing establishes that the visited ids are dense from zero — with
    /// 13,413 distinct positive ids at least one id is >= 13,413, so a
    /// 13,413-entry zero-based extent is NOT guaranteed to span them.
    ///
    /// A copy plan therefore has to size every whole-table copy from the
    /// ENGINE's own count — `u32[container + 0x74]` for the map container
    /// (`d81_resolve_map_province_container`) and `topology.province_count`
    /// for the gamestate side — never from how many provinces it walked.
    ///
    /// This constant stands in for that count offline. It is NOT the engine's
    /// ceiling: `MAX_MAP_CONTROL_PROVINCE_COUNT` in `offsets.rs` is 65,536,
    /// four times this. 16,384 is chosen only as a value comfortably above the
    /// observed province count, so the fixture's copies cannot be short while
    /// its bytes stay representative; a plan sized at the true ceiling would
    /// move four times the table bytes for the same call count, which is the
    /// direction this understates.
    pub(crate) const TABLE_SLOTS: usize = 16_384;

    /// Entries of probe tail the anchor-selector copy must carry beyond the
    /// masked table, because `select_secondary` advances without wrapping and
    /// reads the terminating marker one slot past its last admitted step. The
    /// live extent is `mask + max_distance + 2` and [`SELECTOR_ENTRIES`] is
    /// `mask + 1`, so the tail this constant has to cover is
    /// `max_distance + 1` — at most 256, since `selector_max_distance` is the
    /// u8 read from `selector_map + 0x38`. 258 is used rather than 256 purely
    /// so the fixture keeps two entries of margin and can never be short; the
    /// two spare entries are 48 bytes and buy nothing else.
    pub(crate) const PROBE_SLACK_ENTRIES: usize = 258;

    /// ASSUMPTION, not a measurement. The number of railed provinces whose
    /// engine selector key is NOT their own id.
    ///
    /// It matters because of an elision: when the key IS the province's own id
    /// the resolver serves the province-array slot from the walk's own opening
    /// read and the closing selector read collapses into the source-slot close,
    /// so the site costs four guarded calls. When the two slots differ, the
    /// resolver takes its own opening pair and the closing selector pair runs
    /// as well — eight calls. No run publishes the split, so this is declared
    /// at zero (the fully-elided case) and named here rather than buried: it is
    /// the one row of the census that is a LOWER bound on the live arm, and it
    /// understates the live side by four calls per affected province.
    pub(crate) const NON_SELF_KEYED_RAILED: usize = 0;

    /// ASSUMPTION, not a measurement. Linear-probe steps per lookup at the
    /// load factor above. Each step is a `read_u8_stable` + a
    /// `read_u32_stable` = four guarded calls, so this is the single largest
    /// lever on the live arm's call count and the bench reports its
    /// sensitivity.
    pub(crate) const PROBE_STEPS: usize = 2;

    /// Table strides, from the reader.
    pub(crate) const RAIL_SLOT: usize = 8;
    pub(crate) const ACTIVE_STRIDE: usize = 0x20;
    pub(crate) const POINTER: usize = 8;
}

/// One family of guarded reads d81 issues inside its bracket.
///
/// `calls` is the number of guarded calls the family costs for a whole frame,
/// and `bytes` the bytes those calls move. A `_stable` scalar read is TWO
/// calls (`read_u64_stable` reads twice and compares — see `attach_log.rs`),
/// and a site that is read again at closure is counted at both positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReadFamily {
    /// The reader site, for audit.
    site: &'static str,
    /// True when the addresses are indexed into ONE contiguous engine table,
    /// which is the only case a copy plan can batch.
    contiguous: bool,
    calls: usize,
    bytes: usize,
}

/// d81's in-bracket read census, one entry per site, in walk order.
///
/// EVERY ROW IS DERIVED FROM A CODE SITE, and the site is named so the row can
/// be checked against it. The prologue (roots, selector header, map container)
/// and the four whole-snapshot closing gates are 62 guarded calls between
/// them; they get one row each, itemised in the comment above that row rather
/// than split into a row per hop.
fn d81_live_read_census() -> Vec<ReadFamily> {
    use shape::*;
    let railed = RAILED;
    let pair_bytes = NEIGHBORS_PER_RAILED * 8;
    vec![
        // Prologue, itemised: `roots()` takes five `_stable` u64 hops (10
        // calls, 80 B); `selector_header()` takes a `_stable` u64, u32 and u8
        // (6 calls, 26 B); `map_container()` takes six `_stable` u64 hops --
        // the two clock singletons, the vtable, and the three container hops --
        // plus one `_stable` u32 count (14 calls, 104 B).
        ReadFamily {
            site: "prologue roots/selector header/map container",
            contiguous: false,
            calls: 30,
            bytes: 210,
        },
        // `read_u64_stable(rail_table + id*8)`, for EVERY province including
        // the railless ones. The only site paid per province rather than per
        // railed province, and the largest single row.
        ReadFamily {
            site: "rail-table slot",
            contiguous: true,
            calls: 2 * PROVINCES,
            bytes: 2 * PROVINCES * RAIL_SLOT,
        },
        // `header()`: one 16-byte `read_into` taken twice, at the opening and
        // again at closure.
        ReadFamily {
            site: "rail-info header window (opening + closing)",
            contiguous: false,
            calls: 4 * railed,
            bytes: 4 * railed * 16,
        },
        // The doubled pair blob.
        ReadFamily {
            site: "rail pair blob",
            contiguous: false,
            calls: 2 * railed,
            bytes: 2 * railed * pair_bytes,
        },
        // `d81_read_coherent_active_entry`: a 7-byte window read until two
        // agree — two reads on a quiet frame — at the opening and at closure.
        ReadFamily {
            site: "active-table entry window (opening + closing)",
            contiguous: true,
            calls: 4 * railed,
            bytes: 4 * railed * 7,
        },
        // `d81_read_u64_pair(source_province_slot)`, opening and closing, plus
        // the resolver's own opening pair and the un-elided closing selector
        // pair for any province whose selector key is not its own id.
        ReadFamily {
            site: "gamestate province-array slot (opening + closing)",
            contiguous: true,
            calls: 4 * railed + 4 * NON_SELF_KEYED_RAILED,
            bytes: (4 * railed + 4 * NON_SELF_KEYED_RAILED) * POINTER,
        },
        // `d81_map_province_slot` + its closing re-read.
        ReadFamily {
            site: "map container province-array slot (opening + closing)",
            contiguous: true,
            calls: 4 * railed,
            bytes: 4 * railed * POINTER,
        },
        // `d81_admit_map_province_reciprocity`: the `+0x11B8` record pointer
        // read, consumed, then re-read to close.
        ReadFamily {
            site: "map province +0x11B8 record pointer (opening + closing)",
            contiguous: false,
            calls: 4 * railed,
            bytes: 4 * railed * POINTER,
        },
        // The selector key at `record + 0xC4`.
        ReadFamily {
            site: "selector-key record +0xC4",
            contiguous: false,
            calls: 2 * railed,
            bytes: 2 * railed * 4,
        },
        // `d81_resolve_anchor_selector_key`: the CProvince vtable and its id,
        // two `_stable` scalars 0xA4 apart in one object.
        ReadFamily {
            site: "CProvince vtable + id",
            contiguous: false,
            calls: 4 * railed,
            bytes: 2 * railed * 8 + 2 * railed * 4,
        },
        // The hash probe: per step a `read_u8_stable` marker and a
        // `read_u32_stable` key, plus a `read_u64_stable` value on the hit.
        ReadFamily {
            site: "anchor-selector hash probe",
            contiguous: true,
            calls: (4 * PROBE_STEPS + 2) * railed,
            bytes: (2 * PROBE_STEPS + 2 * PROBE_STEPS * 4 + 2 * 8) * railed,
        },
        // Two passes over the 12-byte anchor triplet.
        ReadFamily {
            site: "anchor triplet window",
            contiguous: false,
            calls: 2 * railed,
            bytes: 2 * railed * 12,
        },
        // The four whole-snapshot closing gates re-run `roots()` (10 calls),
        // `selector_header()` (6), `map_container()` (14) and the
        // province-array currency check, which adds an ordinary u64 and u32
        // (2 calls, 12 B).
        ReadFamily {
            site: "whole-snapshot closing gates",
            contiguous: false,
            calls: 32,
            bytes: 222,
        },
    ]
}

/// The VECTOR copy plan for the same observation set (lane HS, 2026-08-13).
///
/// The rule that produces it: a family whose addresses are indexed into ONE
/// contiguous engine table becomes one copy of that table, taken as an A/B
/// pair at the opening ONLY — no bulk closing image is consumed by the walk,
/// so none is taken. A family whose addresses are scattered heap objects is
/// carried across at its DEMANDED image count (the consumption table in
/// `runs/hs_d81_cheap/COST_MODEL.txt`): the header and record-pointer windows
/// keep all four images, the pair blob, anchors, record key and `CProvince`
/// windows keep the opening pair only, and the slot and active closings the
/// walk consumes become closing-only vector families over the array entries
/// the walk actually closes.
fn d81_copy_plan_census() -> Vec<ReadFamily> {
    use shape::*;
    let railed = RAILED;
    let mut plan: Vec<ReadFamily> = Vec::new();

    // The five contiguous tables, each copied whole as an opening A/B pair
    // and nothing else. Sized by [`TABLE_SLOTS`], the stand-in for the
    // ENGINE's declared count, never by how many provinces the walk visited
    // -- see that constant for why the difference is a correctness one and
    // not a rounding one.
    //
    // THE SELECTOR SLACK IS NOT PADDING. `select_secondary` starts at the
    // MASKED slot and then advances by one stride at a time WITHOUT wrapping,
    // up to `selector_max_distance + 1` steps, and reads the terminating
    // marker one slot beyond that, so the real plan sizes that extent
    // `mask + max_distance + 2`; [`PROBE_SLACK_ENTRIES`] stands in for the
    // tail offline.
    for (site, bytes) in [
        ("rail table (whole)", TABLE_SLOTS * RAIL_SLOT),
        ("active table (whole)", TABLE_SLOTS * ACTIVE_STRIDE),
        ("gamestate province array (whole)", TABLE_SLOTS * POINTER),
        ("map container province array (whole)", TABLE_SLOTS * POINTER),
        (
            "anchor-selector table (whole)",
            (SELECTOR_ENTRIES + PROBE_SLACK_ENTRIES) * SELECTOR_STRIDE,
        ),
    ] {
        plan.push(ReadFamily {
            site,
            contiguous: true,
            calls: 2,
            bytes: 2 * bytes,
        });
    }

    // The scattered families, at their demanded image counts.
    plan.push(ReadFamily {
        // O, S1, C, CS1 of the 16-byte header window: the walk consumes both
        // pairs (`header(Opening)` and `header(Closing)`).
        site: "rail-info header window (opening + closing)",
        contiguous: false,
        calls: 4 * railed,
        bytes: 4 * railed * 16,
    });
    plan.push(ReadFamily {
        // Opening pair only: the walk never reads a blob closing image.
        site: "rail pair blob",
        contiguous: false,
        calls: 2 * railed,
        bytes: 2 * railed * NEIGHBORS_PER_RAILED * 8,
    });
    plan.push(ReadFamily {
        // O, S1, C, CS1 of the 8-byte record pointer: the closing pair is
        // consumed by the reciprocity gate after the key.
        site: "map province +0x11B8 record pointer (opening + closing)",
        contiguous: false,
        calls: 4 * railed,
        bytes: 4 * railed * POINTER,
    });
    plan.push(ReadFamily {
        // Opening pair of ONE 24-byte window covering BOTH adjacent triplets
        // (0x14E4 and 0x14F0), where the live reader took two 12-byte reads.
        site: "anchor triplet window",
        contiguous: false,
        calls: 2 * railed,
        bytes: 2 * railed * 24,
    });
    plan.push(ReadFamily {
        // Opening pair only.
        site: "selector-key record +0xC4",
        contiguous: false,
        calls: 2 * railed,
        bytes: 2 * railed * 4,
    });
    plan.push(ReadFamily {
        // Opening pair of the merged 168-byte head window (vtable through
        // id), where the live reader took two `_stable` scalars.
        site: "CProvince vtable + id",
        contiguous: false,
        calls: 2 * railed,
        bytes: 2 * railed * 168,
    });

    // The closing-only vector families: the slot and active closings the walk
    // consumes, observed per RAILED entry AFTER every family reached through
    // them. Their opening pairs are the bulk arrays' — these rows are the
    // closing side only.
    plan.push(ReadFamily {
        site: "gamestate province-array slot closes (vector)",
        contiguous: true,
        calls: 2 * railed,
        bytes: 2 * railed * POINTER,
    });
    plan.push(ReadFamily {
        site: "map container province-array slot closes (vector)",
        contiguous: true,
        calls: 2 * railed,
        bytes: 2 * railed * POINTER,
    });
    plan.push(ReadFamily {
        // Three interleaved closing observations of the 7-byte window, the
        // coherence loop's own count.
        site: "active-table entry window closes (vector)",
        contiguous: true,
        calls: 3 * railed,
        bytes: 3 * railed * 7,
    });

    // The prologue and the four whole-snapshot closing gates are LIVE reads
    // in both architectures — the plan is rooted on what they resolve — so
    // they carry across from the live census verbatim.
    for family in d81_live_read_census() {
        if family.site == "prologue roots/selector header/map container"
            || family.site == "whole-snapshot closing gates"
        {
            plan.push(family);
        }
    }

    plan
}

fn totals(census: &[ReadFamily]) -> (usize, usize) {
    census
        .iter()
        .fold((0, 0), |(calls, bytes), family| {
            (calls + family.calls, bytes + family.bytes)
        })
}

// ---------------------------------------------------------------------------
// The necessary condition: the plan must reach every byte the reader reads
// ---------------------------------------------------------------------------

/// A copy plan that misses a byte the reader decodes turns a published frame
/// into a refusal, so coverage is the plan's first correctness obligation.
///
/// This checks it structurally at the level the census is written in: every
/// live read family is either carried across verbatim, or is subsumed by a
/// whole-table copy that spans the table it indexes into.
///
/// WHAT THIS DOES NOT PROVE: that the whole-table copies are wide enough at
/// the LIVE mask (`shape::SELECTOR_ENTRIES` is an assumption), that the plan
/// preserves refusal vocabulary, or that the closing comparison can still be
/// made field-granular. Those are the substrate gaps, and no test in this file
/// speaks to them.
///
/// MUTATION RUN: `("rail table (whole)", TABLE_SLOTS * RAIL_SLOT)` changed to
/// `TABLE_SLOTS * RAIL_SLOT / 2`. Observed in `--release`:
///
/// ```text
/// thread '...::the_copy_plan_covers_every_range_the_live_reader_reads'
///   panicked at collector\src\attach_log\tests\d81_copy_bracket.rs:477:9:
/// the plan's "rail table (whole)" copy spans 65536 bytes, short of the
///   131072 the live reader indexes into
/// ```
///
/// Restored. The transcript is in
/// `extractor/native/runs/fx_d81_copy_bracket/mutation1.log`.
#[test]
fn the_copy_plan_covers_every_range_the_live_reader_reads() {
    use shape::*;
    let live = d81_live_read_census();
    let plan = d81_copy_plan_census();

    // Every scattered family survives into the plan unchanged, except the one
    // documented in-object merge.
    for family in live.iter().filter(|family| !family.contiguous) {
        let carried = plan
            .iter()
            .find(|entry| entry.site == family.site)
            .unwrap_or_else(|| panic!("the plan drops the scattered site {:?}", family.site));
        assert!(
            carried.calls <= family.calls,
            "the plan must not issue MORE calls than the live reader at {:?}: \
             plan {}, live {}",
            family.site,
            carried.calls,
            family.calls
        );
        assert!(
            carried.bytes >= family.bytes,
            "a merged window must not read FEWER bytes than the scalars it \
             replaces at {:?}",
            family.site
        );
    }

    // Every contiguous family is spanned by a whole-table copy of the table it
    // indexes into. The tables and their extents, from the reader's own
    // strides.
    for (plan_site, extent, live_site) in [
        (
            "rail table (whole)",
            TABLE_SLOTS * RAIL_SLOT,
            "rail-table slot",
        ),
        (
            "active table (whole)",
            TABLE_SLOTS * ACTIVE_STRIDE,
            "active-table entry window (opening + closing)",
        ),
        (
            "gamestate province array (whole)",
            TABLE_SLOTS * POINTER,
            "gamestate province-array slot (opening + closing)",
        ),
        (
            "map container province array (whole)",
            TABLE_SLOTS * POINTER,
            "map container province-array slot (opening + closing)",
        ),
        (
            // The probe tail is part of the required span, not slack the plan
            // may drop: `select_secondary` walks past the masked slot without
            // wrapping. See `PROBE_SLACK_ENTRIES`.
            "anchor-selector table (whole)",
            (SELECTOR_ENTRIES + PROBE_SLACK_ENTRIES) * SELECTOR_STRIDE,
            "anchor-selector hash probe",
        ),
    ] {
        let copy = plan
            .iter()
            .find(|entry| entry.site == plan_site)
            .unwrap_or_else(|| panic!("the plan has no {plan_site} copy"));
        // The census records the whole family's bytes (all passes), so the
        // single-pass span is what has to cover the table. Every bulk table
        // is an opening pair and nothing else: no bulk closing image is
        // consumed by the walk (the vector families own the closings it does
        // consume, asserted below).
        let passes = 2;
        let span = copy.bytes / passes;
        assert!(
            span >= extent,
            "the plan's {plan_site:?} copy spans {span} bytes, short of the \
             {extent} the live reader indexes into",
        );
        assert!(
            live.iter().any(|family| family.site == live_site),
            "the live census must still contain {live_site:?}, or this row \
             covers nothing"
        );
    }

    // The closing side the walk consumes exists as vector families, at the
    // walk's own observation counts: an adjacent pair for the two slot
    // families and the coherence loop's three for the active window. These
    // rows are the demand table's closing half made executable.
    //
    // kills: dropping a slot-close or active-close family from
    // `d81_copy_plan_census` (or changing its per-entry observation count)
    // without this model noticing — which is the drift that would let the
    // real pipeline lose a closing family while the census still claimed
    // coverage.
    for (site, calls) in [
        ("gamestate province-array slot closes (vector)", 2 * shape::RAILED),
        (
            "map container province-array slot closes (vector)",
            2 * shape::RAILED,
        ),
        ("active-table entry window closes (vector)", 3 * shape::RAILED),
    ] {
        let close = plan
            .iter()
            .find(|entry| entry.site == site)
            .unwrap_or_else(|| panic!("the plan has no {site} family"));
        assert_eq!(
            close.calls, calls,
            "{site} must observe at the walk's own closing count"
        );
    }

    // And the plan must be a strict call reduction, or there is no S4 case for
    // d81 at all.
    let (live_calls, _) = totals(&live);
    let (plan_calls, _) = totals(&plan);
    assert!(
        plan_calls < live_calls,
        "the copy plan issues {plan_calls} guarded calls against the live \
         reader's {live_calls}: it is not a reduction"
    );
}

// ---------------------------------------------------------------------------
// The price
// ---------------------------------------------------------------------------

/// Price the two shapes against the real guarded primitives.
///
/// WHY A BENCH AND NOT AN ASSERTION. `#[ignore]` for the reason
/// `d81_guarded_read_cost_model` and `d81_bracket_split_cost_model` give: this
/// is a measurement, and a wall-clock gate in this tree flakes under lane
/// load. Run it deliberately:
///
/// ```text
/// cargo test -p collector --release d81_copy_bracket_cost_model -- --ignored --nocapture
/// ```
///
/// WHAT THE ARMS ARE. Arm LIVE issues, over a 256 MiB pool, the guarded calls
/// of [`d81_live_read_census`] at their real widths: the table-indexed
/// families walk contiguous regions laid out at the reader's own strides, and
/// the scattered families visit randomised 64-byte-aligned object addresses.
/// Arm PLAN issues [`d81_copy_plan_census`] over the SAME pool and the SAME
/// scattered object addresses. The two arms are run nine times each and ALL
/// EIGHTEEN invocations are shuffled once, globally, so the two arms are
/// interleaved through the run rather than measured one after the other —
/// machine drift over a run exceeds most of what is being measured. The
/// reported number is the per-arm median of whole-frame walls.
///
/// WHAT IT IS PRICING, precisely: the FIXED per-call component of a guarded
/// read, plus the bytes each shape moves, at d81's call counts. It is not
/// pricing d81's live memory behaviour and does not claim to. Both arms share
/// one pool, so by the time any given invocation runs the other arm may
/// already have touched the same tables and objects; no invocation is known to
/// be cold. All samples are printed rather than only the median for that
/// reason — but the spread is just a spread, and this fixture cannot attribute
/// its top to a colder cache rather than to scheduling or any other noise.
/// Nothing here bounds the live-game ratio; the live in-bracket span is ~17 ms
/// and only a capture can price it.
///
/// WHERE IT UNDERSTATES THE PLAN'S SAVING: the pool is ONE allocation, so
/// `range_confirmed_good` resolves on its MRU fast path on essentially every
/// call. Live d81 chases pointers across many VirtualQuery regions, so the
/// fixed per-call component — the only thing the plan removes — is LARGER live
/// than here.
///
/// WHERE IT OVERSTATES IT: the whole-table copies are sequential reads inside
/// one warm allocation, which is the friendliest shape a bulk copy can have;
/// and the plan is priced at `shape::SELECTOR_ENTRIES` and
/// `shape::TABLE_SLOTS`, both assumptions, with a doubled selector variant
/// reported beside it.
///
/// WHAT IT DOES NOT MEASURE AT ALL: correctness. The plan over-reads by
/// construction, and whether that over-read changes which frames publish is
/// not a question a bench can answer.
///
/// ONE ROW IS A LOWER BOUND, not an equality: the active-entry window is read
/// until two samples agree, up to `D81_ACTIVE_COHERENCE_READS` = 3 per
/// position. The census and the arm both take the quiet-frame value of two per
/// position; a frame that has to re-sample costs more on the LIVE arm and the
/// same on the PLAN arm, so this understates the plan.
#[test]
#[ignore]
fn d81_copy_bracket_cost_model() {
    const POOL_LEN: usize = 256 << 20;
    const TRIALS: usize = 9;

    let pool = vec![0x5Au8; POOL_LEN];
    let base = pool.as_ptr() as usize;
    let mut seed = 0xD8_1C_0B_1Au64;
    let mut splitmix = move || {
        seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };

    // The contiguous tables, laid out at the reader's strides.
    let rail_table = base;
    let active_table = rail_table + shape::TABLE_SLOTS * shape::RAIL_SLOT;
    let gs_array = active_table + shape::TABLE_SLOTS * shape::ACTIVE_STRIDE;
    let container_array = gs_array + shape::TABLE_SLOTS * shape::POINTER;
    let selector_table = container_array + shape::TABLE_SLOTS * shape::POINTER;
    // The selector extent includes the probe tail the plan must copy, so the
    // scattered objects start past it and the arm never reads outside the pool.
    let tables_end = selector_table
        + (shape::SELECTOR_ENTRIES + shape::PROBE_SLACK_ENTRIES) * shape::SELECTOR_STRIDE;

    // The scattered heap objects, randomised inside the rest of the pool and
    // held far enough from the end that the widest window never runs off it.
    let scatter_lo = tables_end - base + 4096;
    let scatter_span = POOL_LEN - scatter_lo - 8192;
    let mut scatter = |count: usize| -> Vec<usize> {
        (0..count)
            .map(|_| base + scatter_lo + ((splitmix() as usize) % (scatter_span / 64)) * 64)
            .collect()
    };
    let rail_infos = scatter(shape::RAILED);
    let pair_blobs = scatter(shape::RAILED);
    let map_provinces = scatter(shape::RAILED);
    let records = scatter(shape::RAILED);
    let cprovinces = scatter(shape::RAILED);

    // The region cache is epoch-stamped; stamp a fresh frame so the first
    // visit pays the same VirtualQuery a live frame's first visit does.
    page_cache_new_frame();

    let pair_bytes = shape::NEIGHBORS_PER_RAILED * 8;

    /// The scattered half of the census: the read families whose addresses are
    /// separate heap allocations. IDENTICAL in both arms — that is the finding
    /// this bench exists to price — except for the CProvince head, which the
    /// plan merges and which each arm therefore issues for itself.
    fn scattered_half(
        rail_infos: &[usize],
        pair_blobs: &[usize],
        map_provinces: &[usize],
        records: &[usize],
        pair_bytes: usize,
    ) -> u64 {
        let mut acc = 0u64;
        let mut window = [0u8; 16];
        let mut blob = vec![0u8; pair_bytes];
        let mut anchor = [0u8; 12];
        for index in 0..rail_infos.len() {
            // rail-info header window: two passes at the opening, two at close
            for _ in 0..4 {
                acc = acc.wrapping_add(u64::from(read_into(rail_infos[index], &mut window)));
            }
            // the doubled pair blob
            for _ in 0..2 {
                acc = acc.wrapping_add(u64::from(read_into(pair_blobs[index], &mut blob)));
            }
            // map province +0x11B8 record pointer, opening and closing
            for _ in 0..2 {
                acc = acc.wrapping_add(read_u64_stable(map_provinces[index]).unwrap_or(0));
            }
            // the selector key at record +0xC4
            acc = acc.wrapping_add(u64::from(read_u32_stable(records[index]).unwrap_or(0)));
            // the 12-byte anchor triplet, two passes, off the same map object
            for _ in 0..2 {
                acc = acc.wrapping_add(u64::from(read_into(map_provinces[index] + 0x400, &mut anchor)));
            }
        }
        acc
    }

    /// The prologue and the four whole-snapshot closing gates: 62 guarded
    /// calls between them, IDENTICAL in both shapes because a copy plan
    /// changes nothing about resolving five roots and re-checking them.
    ///
    /// They are issued by both arms rather than left out of both, so that each
    /// arm's call count is exactly its census row total and the published ratio
    /// is a ratio of things that were actually run.
    fn prologue_and_closing_gates(roots: &[usize]) -> u64 {
        let mut acc = 0u64;
        // Per pass -- once for the opening prologue, once for the closing
        // gates -- TWELVE `_stable` u64 hops: five in `roots()`, the selector
        // table pointer, and six in `map_container()` (both clock singletons,
        // the vtable, and the container's three). TWO `_stable` u32 fields:
        // the selector mask and the container's province count. ONE `_stable`
        // u8: the selector's max distance. That is 12*2 + 2*2 + 1*2 = 30
        // guarded calls per pass, the census's prologue row exactly.
        for _ in 0..2 {
            for slot in roots.iter().take(12) {
                acc = acc.wrapping_add(read_u64_stable(*slot).unwrap_or(0));
            }
            acc = acc.wrapping_add(u64::from(read_u32_stable(roots[12]).unwrap_or(0)));
            acc = acc.wrapping_add(u64::from(read_u32_stable(roots[13]).unwrap_or(0)));
            acc = acc.wrapping_add(u64::from(read_u8_stable(roots[14]).unwrap_or(0)));
        }
        // The closing gates' province-array currency check, which is the two
        // plain reads that make the closing row 32 rather than 30.
        acc = acc.wrapping_add(read_u64(roots[15]).unwrap_or(0));
        acc = acc.wrapping_add(u64::from(read_u32(roots[16]).unwrap_or(0)));
        acc
    }
    let roots = scatter(17);
    let live_roots = roots.clone();

    let live_rail_infos = rail_infos.clone();
    let live_pair_blobs = pair_blobs.clone();
    let live_map_provinces = map_provinces.clone();
    let live_records = records.clone();
    let live_cprovinces = cprovinces.clone();

    #[allow(clippy::type_complexity)]
    let mut arms: Vec<(&'static str, Box<dyn FnMut() -> u64>)> = vec![
        (
            "LIVE (today's in-bracket read sequence)",
            Box::new(move || {
                let mut acc = 0u64;
                // rail-table slot, every province including the railless ones
                for id in 0..shape::PROVINCES {
                    acc = acc.wrapping_add(read_u64_stable(rail_table + id * shape::RAIL_SLOT).unwrap_or(0));
                }
                let stride = shape::PROVINCES / shape::RAILED;
                for index in 0..shape::RAILED {
                    let id = index * stride;
                    // active-table entry window, opening and closing
                    let window = active_table + id * shape::ACTIVE_STRIDE + 0x12;
                    for _ in 0..4 {
                        acc = acc.wrapping_add(read_scalar::<7>(window).map_or(0, |w| u64::from(w[0])));
                    }
                    // gamestate province-array slot, opening and closing. TWO
                    // iterations, not four: `read_u64_stable` is itself two
                    // guarded calls, so this is the census's 4.
                    for _ in 0..2 {
                        acc = acc.wrapping_add(read_u64_stable(gs_array + id * shape::POINTER).unwrap_or(0));
                    }
                    // The un-elided share: for a province whose selector key is
                    // not its own id, the resolver takes its own opening pair
                    // and the closing selector pair runs as well. Driven from
                    // the declared shape so that raising
                    // `NON_SELF_KEYED_RAILED` moves the TIMED arm and not only
                    // the census.
                    if index < shape::NON_SELF_KEYED_RAILED {
                        for _ in 0..2 {
                            acc = acc
                                .wrapping_add(read_u64_stable(gs_array + id * shape::POINTER).unwrap_or(0));
                        }
                    }
                    // map container province-array slot, opening and closing,
                    // on the same two-calls-per-iteration reckoning.
                    for _ in 0..2 {
                        acc = acc.wrapping_add(read_u64_stable(container_array + id * shape::POINTER).unwrap_or(0));
                    }
                    // CProvince vtable and id, two `_stable` scalars 0xA4 apart
                    acc = acc.wrapping_add(read_u64_stable(live_cprovinces[index]).unwrap_or(0));
                    acc = acc.wrapping_add(u64::from(read_u32_stable(live_cprovinces[index] + 0xA4).unwrap_or(0)));
                    // the bounded linear hash probe
                    let mut entry = selector_table
                        + (index * 7919 % shape::SELECTOR_ENTRIES) * shape::SELECTOR_STRIDE;
                    for _ in 0..shape::PROBE_STEPS {
                        acc = acc.wrapping_add(u64::from(read_u8_stable(entry + 4).unwrap_or(0)));
                        acc = acc.wrapping_add(u64::from(read_u32_stable(entry + 8).unwrap_or(0)));
                        entry += shape::SELECTOR_STRIDE;
                    }
                    acc = acc.wrapping_add(read_u64_stable(entry + 0x10).unwrap_or(0));
                }
                acc = acc.wrapping_add(prologue_and_closing_gates(&live_roots));
                acc.wrapping_add(scattered_half(
                    &live_rail_infos,
                    &live_pair_blobs,
                    &live_map_provinces,
                    &live_records,
                    pair_bytes,
                ))
            }),
        ),
        (
            "PLAN (vector families, demand-driven images)",
            Box::new(move || {
                let mut acc = 0u64;
                // The five whole-table copies: an opening A/B pair apiece and
                // no closing copy — no bulk closing image is consumed by the
                // walk.
                let mut rail = vec![0u8; shape::TABLE_SLOTS * shape::RAIL_SLOT];
                let mut active = vec![0u8; shape::TABLE_SLOTS * shape::ACTIVE_STRIDE];
                let mut gs = vec![0u8; shape::TABLE_SLOTS * shape::POINTER];
                let mut container = vec![0u8; shape::TABLE_SLOTS * shape::POINTER];
                let mut selector = vec![
                    0u8;
                    (shape::SELECTOR_ENTRIES + shape::PROBE_SLACK_ENTRIES)
                        * shape::SELECTOR_STRIDE
                ];
                for _ in 0..2 {
                    acc = acc.wrapping_add(u64::from(read_into(rail_table, &mut rail)));
                    acc = acc.wrapping_add(u64::from(read_into(active_table, &mut active)));
                    acc = acc.wrapping_add(u64::from(read_into(gs_array, &mut gs)));
                    acc = acc.wrapping_add(u64::from(read_into(container_array, &mut container)));
                    acc = acc.wrapping_add(u64::from(read_into(selector_table, &mut selector)));
                }
                // The scattered families at their DEMANDED image counts, one
                // family at a time in pipeline order, each window's images
                // interleaved back to back — the vector pipeline's own shape.
                let mut window = [0u8; 16];
                let mut blob = vec![0u8; pair_bytes];
                let mut anchors = [0u8; 24];
                let mut slot = [0u8; 8];
                let mut key = [0u8; 4];
                let mut head = [0u8; 168];
                let mut active_window = [0u8; 7];
                // rail headers: opening pair, then (after decode) closing pair
                for pass in 0..2 {
                    for &rail_info in rail_infos.iter() {
                        for _ in 0..2 {
                            acc = acc.wrapping_add(u64::from(read_into(rail_info, &mut window)));
                        }
                    }
                    let _ = pass;
                }
                // pair blobs: opening pair only
                for &pair_blob in pair_blobs.iter() {
                    for _ in 0..2 {
                        acc = acc.wrapping_add(u64::from(read_into(pair_blob, &mut blob)));
                    }
                }
                // record pointers: opening pair, then (after the keys) the
                // closing pair
                for _ in 0..2 {
                    for &map_province in map_provinces.iter() {
                        for _ in 0..2 {
                            acc = acc.wrapping_add(u64::from(read_into(map_province, &mut slot)));
                        }
                    }
                }
                // anchors: one 24-byte window, opening pair only
                for &map_province in map_provinces.iter() {
                    for _ in 0..2 {
                        acc = acc
                            .wrapping_add(u64::from(read_into(map_province + 0x400, &mut anchors)));
                    }
                }
                // record keys: opening pair only
                for &record in records.iter() {
                    for _ in 0..2 {
                        acc = acc.wrapping_add(u64::from(read_into(record, &mut key)));
                    }
                }
                // CProvince heads: opening pair only
                for &cprovince in cprovinces.iter() {
                    for _ in 0..2 {
                        acc = acc.wrapping_add(u64::from(read_into(cprovince, &mut head)));
                    }
                }
                // the closing-only families, last: both slot sets and the
                // active window's coherence triple
                let stride = shape::PROVINCES / shape::RAILED;
                for index in 0..shape::RAILED {
                    let id = index * stride;
                    for _ in 0..2 {
                        acc = acc.wrapping_add(u64::from(read_into(
                            gs_array + id * shape::POINTER,
                            &mut slot,
                        )));
                    }
                }
                for index in 0..shape::RAILED {
                    let id = index * stride;
                    for _ in 0..2 {
                        acc = acc.wrapping_add(u64::from(read_into(
                            container_array + id * shape::POINTER,
                            &mut slot,
                        )));
                    }
                }
                for index in 0..shape::RAILED {
                    let id = index * stride;
                    for _ in 0..3 {
                        acc = acc.wrapping_add(u64::from(read_into(
                            active_table + id * shape::ACTIVE_STRIDE + 0x12,
                            &mut active_window,
                        )));
                    }
                }
                acc.wrapping_add(prologue_and_closing_gates(&roots))
            }),
        ),
    ];

    let mut samples: Vec<Vec<f64>> = vec![Vec::with_capacity(TRIALS); arms.len()];
    let mut order: Vec<usize> = (0..arms.len() * TRIALS).map(|k| k % arms.len()).collect();
    let mut shuffle_seed = 0x51_ED_C0_DEu64;
    let mut shuffle = move || {
        shuffle_seed = shuffle_seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = shuffle_seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for k in (1..order.len()).rev() {
        let j = (shuffle() as usize) % (k + 1);
        order.swap(k, j);
    }
    for &index in &order {
        let started = Instant::now();
        let acc = (arms[index].1)();
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        std::hint::black_box(acc);
        samples[index].push(ms);
    }

    let median = |values: &[f64]| {
        let mut sorted = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        sorted[sorted.len() / 2]
    };

    let live = d81_live_read_census();
    let plan = d81_copy_plan_census();
    let (live_calls, live_bytes) = totals(&live);
    let (plan_calls, plan_bytes) = totals(&plan);

    println!(
        "\n# d81 copy-only bracket cost model ({TRIALS} interleaved trials, \
         whole-frame medians, {} provinces / {} railed)",
        shape::PROVINCES,
        shape::RAILED
    );
    println!("\n## census (derived from the reader's call sites)\n");
    println!("| arm | guarded calls | bytes moved | measured wall |");
    println!("|---|---:|---:|---:|");
    for (index, (label, _)) in arms.iter().enumerate() {
        let (calls, bytes) = if index == 0 {
            (live_calls, live_bytes)
        } else {
            (plan_calls, plan_bytes)
        };
        println!(
            "| {label} | {calls} | {:.2} MiB | {:.2} ms |",
            bytes as f64 / (1024.0 * 1024.0),
            median(&samples[index])
        );
    }
    let live_ms = median(&samples[0]);
    let plan_ms = median(&samples[1]);
    println!(
        "\ncall reduction {:.2}x, wall {:.2}x ({live_ms:.2} ms -> {plan_ms:.2} ms)",
        live_calls as f64 / plan_calls as f64,
        live_ms / plan_ms.max(f64::MIN_POSITIVE),
    );

    // EVERY SAMPLE, IN THE ORDER IT RAN, because the median hides the one thing
    // this fixture gets wrong about the live frame. d81's ~3,510 scattered
    // objects are a few MiB of working set, and both arms share one pool, so
    // most invocations find it already warm and measure the FIXED per-call
    // component almost alone. Live d81 touches those objects once per frame
    // with the game evicting in between, so its per-call cost carries a memory
    // stall this pool mostly stops paying. The samples are printed in full
    // rather than averaged away so the spread is visible; what CAUSES the
    // spread — cache, scheduling, or anything else on the box — is not
    // something this fixture separates, and no line here claims otherwise.
    println!("\n## every trial, ms (shuffled order; the spread is the output)\n");
    for (index, (label, _)) in arms.iter().enumerate() {
        let rendered: Vec<String> = samples[index]
            .iter()
            .map(|value| format!("{value:.2}"))
            .collect();
        println!("| {label} | {} |", rendered.join(" | "));
    }

    // The live arm's call count turns on PROBE_STEPS more than on anything
    // else declared, and that constant is an assumption. Publish the whole
    // band rather than the one point the median was taken at.
    println!("\n## sensitivity: probe steps per selector lookup\n");
    println!("| probe steps | live calls | plan calls | call reduction |");
    println!("|---:|---:|---:|---:|");
    for steps in [1usize, 2, 4, 8] {
        let delta = (4 * steps + 2) as isize - (4 * shape::PROBE_STEPS + 2) as isize;
        let live_at = (live_calls as isize + delta * shape::RAILED as isize) as usize;
        println!(
            "| {steps} | {live_at} | {plan_calls} | {:.2}x |",
            live_at as f64 / plan_calls as f64
        );
    }
    println!("\n## per-site census\n");
    println!("| site | batchable | calls | bytes |");
    println!("|---|---|---:|---:|");
    for family in &live {
        println!(
            "| {} | {} | {} | {} |",
            family.site,
            if family.contiguous { "table" } else { "scattered" },
            family.calls,
            family.bytes
        );
    }
    println!(
        "\nSELECTOR_ENTRIES is an assumption ({}); at twice that the plan moves \
         {:.2} MiB more and issues no more calls.",
        shape::SELECTOR_ENTRIES,
        (2 * shape::SELECTOR_ENTRIES * shape::SELECTOR_STRIDE) as f64 / (1024.0 * 1024.0)
    );
}
