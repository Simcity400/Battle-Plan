//! Lane DQ stage-2 experiment: run the H4CP **v3 prototype** beside the v2 path
//! on a real capture and price both.
//!
//! # What it decides
//!
//! The design study's stage 2 keeps each byte-dominant section's fold state as
//! canonical binary with stable keys and makes the hour's mutation journal the
//! cache delta, instead of rebuilding, re-serializing, re-parsing, re-hashing
//! and re-diffing the whole world document every game hour. The go/no-go is two
//! numbers: projected full-conversion CPU must stay under 0.20 s per game hour,
//! and two resident states must stay under 1 GB.
//!
//! # Why it reports a BAND and not a number
//!
//! The prototype does not rewrite the ~68 domain reducers -- that is the next
//! stage -- so it cannot observe the mutations a reducer would make. It derives
//! them instead, from the section the v2 fold already built. That derivation is
//! not free, and whether it is charged decides the answer, so this instrument
//! charges it BOTH ways and publishes both:
//!
//! * `api_lower` -- the v3 mutation machinery alone: serialize each changed
//!   grain, hash it, install it, journal it, move the state root, then coalesce,
//!   sort and encode the hour, and hash the delta. This is what stage 3's
//!   per-hour cost would be IF its reducers handed the API its mutations for
//!   nothing. Nothing does anything for nothing, so this is a floor and is
//!   labelled one.
//! * `adapter_inclusive_upper` -- the same, plus the section's CURRENT
//!   construction cost (`build_snapshot`'s own span for that section, read from
//!   the fold's profiler) plus the full decomposition and change-set
//!   derivation. This charges v3 for every gram of work the prototype cannot yet
//!   do incrementally, including work stage 3 would delete outright. It is an
//!   over-charge, and it is the number the reject threshold is read against,
//!   because a design that passes while being over-charged has passed.
//!
//! A result where only the lower bound clears the threshold is INCONCLUSIVE and
//! says so.
//!
//! # Drift
//!
//! This machine moves ±20% between runs minutes apart, so nothing here compares
//! two runs. The v2 and v3 lanes are measured on the SAME hour, in the same
//! process, alternating which goes first so neither lane is systematically the
//! one that finds the section already in cache, and the headline is their ratio.
//!
//! # What it does not do
//!
//! It writes no artifact, seals nothing, and touches neither the production fold
//! nor the v2 cache format. `verify_every_hour`, checkpoint framing, the H4IX
//! index and the whole-cache SHA are in NEITHER lane -- v3 would still owe all
//! of them, and the report says so rather than quietly banking their absence.
//!
//! # A band names the file it opened
//!
//! Every report carries `executable_path` and `executable_sha256`: the pathname
//! `current_exe` returns, and the SHA-256 of the file opened by resolving that
//! pathname, read as the report is written. Nothing more is claimed -- not the
//! running image, not the file the process was launched from, because the
//! pathname is resolved a second time at open. `executable_identity`'s note
//! states the mechanism and the three things that do not follow from it. Lane GE's `PROVENANCE.txt` had
//! to record even this much by hand, and said in its own words that it was "NOT
//! an independent binding of a binary to a JSON"; the report now carries it.
//!
//! That matters more here than it would elsewhere, because this instrument's
//! numbers are read across sessions and the CONFIGURATION lives in the binary:
//! `COVERED_SECTIONS` is a const, so a band measured over 39 sections and one
//! measured over 41 are indistinguishable in the JSON except by the digest. Two
//! reports with different digests are two different experiments, whatever their
//! fields say.

use std::collections::BTreeMap;
use std::path::Path as FsPath;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use fold_v3::{decompose_ref, Path, RootKind, SectionSpec, SectionState, Shape};
use snapshot::canon_value::CanonValue;
use snapshot::replay_cache::measure_section_patch_bytes;
use snapshot::sha256::Sha256;

use crate::{HourCursor, IndexSourceDescriptor, StreamIdentityMode, WorldHour};
use recorder_host::world_fold::foldprof;

/// The six byte-dominant top-level sections of the world document, as measured
/// by `cz_finalize_profile`'s member byte census on capture t30's last hour:
/// special_projects 29.07%, countries 20.09%, map 18.24%, supply 12.49%,
/// divisions 8.61%, states 3.06% -- 91.56% of that document's member bytes.
///
/// `divisions` is spelled as a JSON array and is the one section with no keyed
/// children in the document. It does have them in the fold, which produces it
/// from a `BTreeMap<uid, object>` and emits that map's values in key order, so
/// the prototype addresses it by `div_uid` and reproduces the array by writing
/// the keys back in ascending order. Without that, an insertion in the middle
/// would move every later element and the whole section would re-emit.
/// Tranche 1 adds the three members the LANE EE DENSITY CENSUS
/// (`dq_v3_census`) named, in the order it named them. On t30 the uncovered
/// remainder moved 14.8k grains and wrote 273 kB an hour; `strategic_air`,
/// `naval_air` and `order_of_battle` are 87% of both, on 40% of the uncovered
/// bytes. They are converted first because the extrapolation that stands in for
/// them is wrong about them specifically: it predicts the uncovered remainder at
/// the covered set's rate per byte, which under-counts the remainder's mutations
/// by 1.88x, and essentially all of that error is these three.
///
/// `strategic_air` and `naval_air` are 175 kB and 111 kB that move 5.8k and 5.8k
/// grains an hour -- a mutation density 30x the covered set's per byte, on
/// 17-byte grains. `order_of_battle` is the largest uncovered member (1.77 MB)
/// and writes 176 kB an hour, more than the rest of the remainder combined.
/// # Tranches 2-5: the cheap tail
///
/// Lane EI censused every one of the world document's 41 top-level members at
/// grain depths 1-6 on both captures before converting anything (evidence in
/// `runs/dq_v3_lane_ei/`), because the shape that killed `strategic_air` --
/// thousands of four-byte grains an hour -- is invisible in a section's byte
/// share and only shows up in its mutation density. The remaining 32 members
/// were then converted in four tranches, densest first, each priced against the
/// unchanged seven-section base in the same machine window.
///
/// The tail is genuinely cheap: at the depths chosen below every one of the 32
/// sits BELOW the covered set's 0.57 mutations per kB, which is the opposite of
/// what tranche 1 found and is why the extrapolation these replace was
/// pessimistic rather than optimistic about them.
///
/// # Tranche 6: the two sections the default depth killed
///
/// `strategic_air` and `naval_air` were the whole of what remained uncovered --
/// 0.80% of t30's member bytes and 6.00% of t31's -- and they were removed from
/// this list on 2026-08-07 for costing 33k four-byte grains an hour at the
/// DEFAULT grain depth, which is the depth they were measured at. Two things
/// changed since, and both are the reason they are back:
///
/// * a grain depth is a per-section optimum, and these two never got one. Both
///   are objects whose members are arrays of small objects, so depth 3 lands on
///   an element's individual FIELDS -- `{"region_id":42}` is a four-byte grain
///   with a forty-byte address. The census in `runs/ge_v3_stage3/` prices every
///   depth on both captures and the coarse depth below is its argmin. This is
///   the "coarse-grain design for tiny-value sections" the removal ruling named
///   as the condition for revisiting them, and it needed no design: the knob
///   every other tranche used was simply never turned for these two.
/// * the per-mutation constant fell from 8.6 us to 1.64 us (lanes FB and FL),
///   so the same mutation count costs a fifth of what it did when the ruling
///   was made.
///
/// They are re-added at the END of the const, after the tranches whose bands are
/// already published, so no earlier section's schedule moves. What keeps this
/// from being a silent reversal is that it is measured: `runs/ge_v3_stage3/`
/// carries the interleaved A/B of the 39-section baseline against each of them
/// separately, against the kill-switch the conversion was funded under.
pub const COVERED_SECTIONS: [(&str, &str); 41] = [
    ("special_projects", "object"),
    ("countries", "object"),
    ("map", "object"),
    ("supply", "object"),
    ("divisions", "keyed_array:div_uid"),
    ("states", "object"),
    // -- tranche 1 survivor (owner ruling 2026-08-07) -----------------------
    // `order_of_battle` measured free on CPU and a size WIN (journal
    // 0.56-0.60x of v2's patch). `strategic_air` and `naval_air` were
    // converted, measured, and REMOVED from the default: 33k four-byte
    // grains/hour put them past the per-mutation constant's worst case
    // (0.157 s/h on t31 alone, journals 1.34-1.42x LARGER than the v2
    // patches they replace). Absence from this const is what made them
    // unconvertible AND unselectable -- the section filter validates
    // against it -- so re-measuring them took a deliberate re-add, which
    // is tranche 6 at the end of this list.
    ("order_of_battle", "object"),
    // -- tranche 2: the top of the tail by density and by bytes -------------
    // 2.79 MB of t30's 3.14 MB non-air uncovered remainder (89%), and the
    // only members of it that move more than 25 grains an hour.
    ("provinces", "object"),
    ("designs", "object"),
    ("world_events", "object"),
    ("raids", "object"),
    ("_r2_sunk_ship_history_live", "object"),
    // -- tranche 3: the 60-95 kB sidecars -----------------------------------
    ("world", "object"),
    ("_r2_sunk_convoy_history_live", "object"),
    ("_r2_sunk_convoy_history_live_candidate", "object"),
    ("visibility", "object"),
    ("_r2_nuke_stockpile_live", "object"),
    // -- tranche 4: the small movers ----------------------------------------
    // `combats` is the document's other array. It has no member that is a
    // unique, already-ascending string identity on every hour -- the census
    // refused every candidate -- so unlike `divisions` it can only be
    // addressed by ordinal, and an insertion re-emits every element after
    // it. That is priced, not assumed: it is 8.8 kB.
    ("combats", "array"),
    ("provenance", "object"),
    ("naval_combats", "object"),
    ("measured_countries", "object"),
    ("tech_sharing_group", "object"),
    // -- tranche 5: the residue ---------------------------------------------
    // Seventeen members totalling 2.1 kB on t30 -- 0.006% of the document.
    // They are one tranche and not four because each of them moves the
    // projection by less than the machine's own drift, so four boundaries
    // here would be four readings of the same noise. They are converted at
    // all because with them the projection stops being an extrapolation over
    // anything but the two air sections.
    //
    // Nine of them are never containers (`date` is a string, `hour_index` a
    // number, five are `null` every hour). A section is not its shape --
    // `assign_section` and the driver both route a non-container value to a
    // scalar root holding the whole member at the empty path -- so `object`
    // is the right spelling for "not an array", and the root-kind journal
    // record is what carries the difference.
    ("_r2_dynamic_modifier_lifecycle_live", "object"),
    ("_r2_peace_conference_live", "object"),
    ("ended_combats", "array"),
    ("_r2_occupation_garrison_alert_live", "object"),
    ("date", "object"),
    ("hour_index", "object"),
    ("tag_db_id_index", "object"),
    ("diplomacy_world", "object"),
    ("session_header", "object"),
    ("events", "array"),
    ("game_id", "object"),
    ("save_version", "object"),
    ("schema_version", "object"),
    ("strategic_operatives", "object"),
    ("_r2_delayed_event_lifecycle_live", "object"),
    ("_r2_market_trade_activation_live", "object"),
    ("_r2_selected_designer_identity_live", "object"),
    // -- tranche 6: the two the default depth killed ------------------------
    // Both are objects of arrays of small objects, and both are re-added at a
    // COARSE depth. They are last in the const because every section before
    // them has a published band measured with itself at that position, and the
    // order is the run order.
    ("strategic_air", "object"),
    ("naval_air", "object"),
];

/// The grain depth each section was MEASURED to be cheapest at, as a spec string
/// for the driver's fifth argument.
///
/// A grain depth is an optimum and not a preference, so every entry here is a
/// census number: the depth minimizing `written_bytes_per_hour + mutations *
/// per-record overhead` over depths 1-6 on both captures, ties broken towards
/// the depth with fewer grains (a grain costs [`PER_GRAIN_OVERHEAD`] bytes of
/// resident state whether or not it ever moves). Depth 3 is the default and the
/// sections at it carry no override.
///
/// The first three overrides are the six-section base's own, unchanged since the
/// bands that admitted them were measured. The rest name sections the base run
/// does not activate, where an override is inert -- which is what lets ONE spec
/// string drive every tranche's configuration, so that no two tranches can
/// differ in a depth by accident.
pub const MEASURED_GRAIN_DEPTHS: &str = "3,supply=2,divisions=2,\
     provinces=1,designs=2,world_events=2,raids=2,_r2_sunk_ship_history_live=2,\
     world=2,_r2_sunk_convoy_history_live=2,\
     _r2_sunk_convoy_history_live_candidate=2,_r2_nuke_stockpile_live=1,\
     combats=2,provenance=2,measured_countries=1,tech_sharing_group=1,\
     _r2_dynamic_modifier_lifecycle_live=1,_r2_peace_conference_live=1,\
     ended_combats=1,_r2_occupation_garrison_alert_live=1,date=1,hour_index=1,\
     tag_db_id_index=1,diplomacy_world=1,session_header=1,events=1,game_id=1,\
     save_version=1,schema_version=1,strategic_operatives=1,\
     _r2_delayed_event_lifecycle_live=1,_r2_market_trade_activation_live=1,\
     _r2_selected_designer_identity_live=1,strategic_air=2,naval_air=2";

/// What one grain costs in resident state BEYOND its own payload.
///
/// This is the grain store's slot row, which is where a grain now lives: a
/// `Grain` is a `Vec` header, a 32-byte digest, a revision and a 32-byte cached
/// state-root contribution (96 B); a `Path` is a `Vec` header (24 B); and the
/// slot caches the address's hash (8 B). Counting only the payload would answer
/// the memory threshold with the flattering half of the number -- and it would
/// also make every grain-depth choice look free, when a section cut three
/// levels deep instead of one pays this constant on ten times as many grains
/// for the same bytes.
///
/// It is the OCCUPIED row and nothing else. The store's hash index, its two
/// order arrays, its free list and every reserved row no grain is standing in
/// are reported by `SectionState::retained_scratch_bytes`, so charging a share
/// of them here as well would invent memory. The number it replaced was 136 --
/// the same `Grain` and `Path` plus a `BTreeMap` node's share -- so the change
/// is 8 bytes a grain of accounting, not of behaviour.
pub const PER_GRAIN_OVERHEAD: u64 = 128;

fn shape_for(spelling: &str) -> Result<Shape, String> {
    match spelling {
        "object" => Ok(Shape::Object),
        "array" => Ok(Shape::Array),
        other => match other.strip_prefix("keyed_array:") {
            Some(key) => Ok(Shape::KeyedArray {
                key_member: key.to_string(),
            }),
            None => Err(format!("unknown section shape spelling {other}")),
        },
    }
}

fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

#[derive(Default, Clone, Copy)]
struct Lane {
    /// v2: serialize the section from its value model.
    v2_serialize: Duration,
    /// v2: parse those bytes back, which is what `cache_push` does to the
    /// document it is handed.
    v2_parse: Duration,
    /// v2: SHA-256 over those bytes.
    v2_sha: Duration,
    /// v2: the production `diff_value` + `encode_patch`, on this section.
    v2_diff_encode: Duration,
    /// v3: the mutation API, commit and delta hash. The lower bound.
    v3_api: Duration,
    /// v3: decomposition and change-set derivation -- work a real reducer would
    /// not do, charged into the upper bound anyway.
    v3_adapter: Duration,
    /// The fold's own `build_snapshot` span for this section, charged into the
    /// upper bound.
    build: Duration,
}

#[derive(Default)]
struct SectionReport {
    lane: Lane,
    v2_patch_bytes: u64,
    v3_delta_bytes: u64,
    v3_mutations: u64,
    /// Mutations the `DQ_DROP_MUTATION` negative control actually skipped. Zero
    /// on every ordinary run, and the difference between a control that proved
    /// the oracle and a control that did nothing.
    mutations_dropped: u64,
    v3_calls: u64,
    v3_no_ops: u64,
    v3_root_changes: u64,
    v3_grain_bytes_written: u64,
    section_bytes_last: u64,
    /// The hour `section_bytes_last` was taken on. Coverage sums sizes across
    /// sections, so a size carried over from an hour the section was last seen
    /// would be measured against a different hour's document.
    section_bytes_hour: i64,
    grains_last: u64,
    retained_bytes_last: u64,
    retained_path_bytes_last: u64,
    /// The section's cross-hour scratch buffers, at capacity. A state that keeps
    /// its journal and its commit buffer between hours -- which is deliberate,
    /// and is what keeps the allocator off the mutation path -- is holding that
    /// memory whether or not the buffers are empty at the moment they are read.
    retained_scratch_bytes_last: u64,
    hours_present: u64,
    /// Hours folded into the lane totals: every hour but the section's first.
    hours_measured: u64,
    hours_scalar: u64,
    /// Worst single hour of `v3 delta bytes / v2 patch bytes`, with the hour it
    /// happened on. Grain that is too coarse shows up here before it shows up in
    /// the mean.
    worst_amplification: f64,
    worst_amplification_hour: i64,
}

struct SectionRun {
    name: String,
    spec: SectionSpec,
    state: SectionState,
    replica: SectionState,
    previous: Option<CanonValue>,
    report: SectionReport,
}

/// One hour's v2 terms for one section.
struct V2Hour {
    bytes: Vec<u8>,
    patch_bytes: usize,
}

/// Price the five whole-document terms v2 pays, restricted to one section:
/// serialize it, parse it back, hash it, and diff and encode it against the
/// previous hour with the production encoder's own functions.
fn price_v2(
    run: &mut SectionRun,
    section: &CanonValue,
    measure: bool,
) -> Result<V2Hour, String> {
    let started = Instant::now();
    let bytes = section.to_json_bytes();
    if measure {
        run.report.lane.v2_serialize += started.elapsed();
    }

    let started = Instant::now();
    let reparsed = CanonValue::parse(&bytes)
        .map_err(|e| format!("section {} did not reparse: {e}", run.name))?;
    if measure {
        run.report.lane.v2_parse += started.elapsed();
    }
    std::hint::black_box(&reparsed);

    let started = Instant::now();
    let digest = Sha256::digest(&bytes);
    if measure {
        run.report.lane.v2_sha += started.elapsed();
    }
    std::hint::black_box(digest);

    let mut patch_bytes = 0usize;
    if let Some(previous) = &run.previous {
        let started = Instant::now();
        patch_bytes = measure_section_patch_bytes(previous, section)
            .map_err(|e| format!("section {} patch: {e}", run.name))?;
        if measure {
        run.report.lane.v2_diff_encode += started.elapsed();
    }
    }
    Ok(V2Hour { bytes, patch_bytes })
}

struct V3Hour {
    delta_bytes: usize,
    mutations: u64,
    /// Time spent on the oracle, which is never charged to either lane.
    instrument: Duration,
}

fn price_v3(
    run: &mut SectionRun,
    section: &CanonValue,
    hour_index: i64,
    measure: bool,
) -> Result<V3Hour, String> {
    // ---- adapter: the work a real reducer would never do -------------------
    let started = Instant::now();
    let scalar_root = !matches!(
        (&run.spec.shape, section),
        (Shape::Object, CanonValue::Object(_))
            | (Shape::Array | Shape::KeyedArray { .. }, CanonValue::Array(_))
    );
    let incoming: Vec<(Path, &CanonValue)> = if scalar_root {
        vec![(Vec::new(), section)]
    } else {
        decompose_ref(section, run.spec.grain_depth, &run.spec.shape)
            .map_err(|e| format!("section {}: {e}", run.name))?
    };
    // Which grains actually moved. A candidate's digest is compared against the
    // one the state already holds; a reducer knows this from the record it is
    // applying, so it is charged to the adapter and not to the API.
    // The third element says whether the address ALREADY STANDS in the state --
    // a value moving at an address that exists, as opposed to an address
    // arriving. Only the negative control reads it; see where `drop_one` is
    // applied for why the distinction is load-bearing.
    let mut changed: Vec<(Path, &CanonValue, bool)> = Vec::new();
    let mut present: BTreeMap<Path, ()> = BTreeMap::new();
    for (path, value) in &incoming {
        present.insert(path.clone(), ());
        let candidate = value.to_json_bytes();
        let digest = Sha256::digest(&candidate);
        match run.state.grain_entry(path) {
            // Unchanged only if the VALUE and the address's SPELLING both stand.
            // An address looked up under a different escaped spelling matches by
            // digest while the projection keeps writing the old spelling, so
            // comparing the digest alone would classify a real byte change as no
            // change at all.
            Some((stored, grain))
                if grain.digest() == &digest && fold_v3::same_spelling(stored, path) => {}
            standing => changed.push((path.clone(), *value, standing.is_some())),
        }
    }
    let removed: Vec<Path> = run
        .state
        .grain_paths()
        .filter(|path| !present.contains_key(*path))
        .cloned()
        .collect();
    let target_kind = if scalar_root {
        RootKind::Scalar
    } else {
        run.spec.shape.declared_root_kind()
    };
    let root_change = run.state.root_kind() != target_kind;
    if measure {
        run.report.lane.v3_adapter += started.elapsed();
    }

    // A negative control for the oracle. `DQ_DROP_MUTATION=<section>` makes the
    // prototype skip one mutation per hour of that section -- the exact defect
    // the equivalence oracle exists to catch. If a run with this set still
    // reports zero divergences, the oracle is vacuous and no number from it means
    // anything.
    //
    // The skip is COUNTED, and the count is published as
    // `mutations_dropped_by_negative_control`. It only ever skips one upsert an
    // hour, after the section's first hour, at an address that already stands,
    // so a section that is absent, that never changes, or that changes only by
    // arrivals and removals would run the control without dropping anything --
    // and a control that dropped nothing reports no divergence for the same
    // reason a correct build does. The two readings are indistinguishable in
    // the verdict and were indistinguishable in the report; the counter is what
    // separates "the oracle caught it" from "there was nothing to catch", and
    // the run is refused outright when the count comes out zero.
    let drop_one = std::env::var("DQ_DROP_MUTATION")
        .map(|name| name == run.name)
        .unwrap_or(false)
        && run.state.revision() > 0;

    // ---- the API itself: the lower bound -----------------------------------
    let started = Instant::now();
    let mut txn = run
        .state
        .begin_hour()
        .map_err(|e| format!("section {}: {e}", run.name))?;
    // Removals, then the root kind, then upserts. The order is forced twice
    // over: a member that turns from a value into a container needs its old
    // grain gone before the new ones can stand, and a root kind can only move
    // once the grains that contradict it are gone and before the ones that need
    // it arrive.
    for path in &removed {
        txn.remove(path)
            .map_err(|e| format!("section {}: {e}", run.name))?;
    }
    if root_change {
        txn.set_root_kind(target_kind)
            .map_err(|e| format!("section {}: {e}", run.name))?;
    }
    let mut dropped = 0u64;
    for (path, value, standing) in changed.into_iter() {
        // Drop the first upsert of the hour that lands on an address ALREADY
        // STANDING, not simply the first upsert.
        //
        // The difference is what the control ends up testing. On an
        // ordinal-addressed section, dropping an ARRIVING address can leave the
        // hour's delta installing ordinal 1 with no ordinal 0, which the
        // replica's decoder refuses outright: the run dies with an error and
        // writes no report, so the control proves the decoder rejects a
        // malformed delta -- which it already has tests for -- and proves
        // nothing about the equivalence oracle. Dropping a value at an address
        // that stands leaves every structural invariant intact and leaves
        // exactly one grain holding last hour's bytes, which is the defect the
        // oracle exists to catch, on any section shape.
        if drop_one && dropped == 0 && standing {
            dropped += 1;
            continue;
        }
        txn.upsert(path, value)
            .map_err(|e| format!("section {}: {e}", run.name))?;
    }
    run.report.mutations_dropped += dropped;
    let delta = txn.commit();
    // The bytes a v3 writer would hash on their way to the file.
    let delta_digest = Sha256::digest(&delta.bytes);
    if measure {
        run.report.lane.v3_api += started.elapsed();
    }
    std::hint::black_box(delta_digest);

    // ---- oracle: never timed into either lane ------------------------------
    let started = Instant::now();
    let applied = run
        .replica
        .apply_delta(&delta.bytes)
        .map_err(|e| format!("section {} replica: {e}", run.name))?;
    if applied != delta.mutations {
        return Err(format!(
            "section {} replica applied {applied} of {} records at hour {hour_index}",
            run.name, delta.mutations
        ));
    }
    if run.replica.state_root() != delta.state_root {
        return Err(format!(
            "section {} replica root diverged at hour {hour_index}",
            run.name
        ));
    }
    if run.state.state_root() != run.state.recomputed_state_root() {
        return Err(format!(
            "section {} incremental root disagrees with a full recomputation at hour \
             {hour_index}",
            run.name
        ));
    }
    // An ordinal-addressed container whose ordinals are not `0..k` projects its
    // elements at positions they are not addressed by, silently. The replica's
    // decoder refuses such a delta; this says the PRODUCER never builds one out
    // of real capture data, which is the claim the decoder's cheap incremental
    // check rests on.
    if let Some(complaint) = run.state.ordinal_density_error() {
        return Err(format!(
            "section {} holds a sparse ordinal container at hour {hour_index}: {complaint}",
            run.name
        ));
    }
    // A scalar root must hold EXACTLY the grain at the empty path.
    //
    // Every other check above is a comparison of BYTES, and for a member that is
    // `null` on every hour the bytes cannot see this: `project` renders a scalar
    // root with no grain as `null`, so a state that lost the grain projects the
    // same four bytes as a state that holds it, its incremental root agrees with
    // a full recomputation of the same wrong grain set, and a replica driven by
    // the same missing upsert agrees with the producer. Nine of the members this
    // driver now converts are never containers and five of those are `null`
    // every hour, which is precisely the shape the byte oracle is blind to.
    //
    // Checked on the replica as well as the producer: the two agreeing is the
    // whole of what the replica check proves, and they can agree on a state that
    // is missing the same grain.
    for (side, state) in [("producer", &run.state), ("replica", &run.replica)] {
        let scalar = state.root_kind() == RootKind::Scalar;
        let at_root = state.grain(&[]).is_some();
        if scalar && !(at_root && state.grain_count() == 1) {
            return Err(format!(
                "section {} {side} has a scalar root holding {} grains at hour {hour_index}, \
                 which projects as `null` whatever it lost",
                run.name,
                state.grain_count()
            ));
        }
        if !scalar && at_root {
            return Err(format!(
                "section {} {side} has a container root with a whole-section grain under it \
                 at hour {hour_index}",
                run.name
            ));
        }
    }
    let instrument = started.elapsed();

    Ok(V3Hour {
        delta_bytes: delta.bytes.len(),
        mutations: delta.mutations,
        instrument,
    })
}

/// Read and reset one `build_snapshot` section span.
fn take_section_ns(index: usize) -> Duration {
    Duration::from_nanos(foldprof::SECTION_NS[index].swap(0, Ordering::Relaxed))
}

fn section_index(name: &str) -> Option<usize> {
    foldprof::SECTIONS.iter().position(|entry| *entry == name)
}

/// Which `build_snapshot` span a document member's construction is charged to.
///
/// Most members are named after the span that builds them, and for those this is
/// [`section_index`] unchanged -- the six original sections keep exactly the
/// build term they were measured with. The fold's span table is coarser than the
/// document in two places, and the upper bound is only an over-charge if that is
/// handled deliberately:
///
/// * `naval_and_air` brackets `build_naval_combats`, `build_naval_air` AND
///   `build_strategic_air`, accumulating all three into one slot. It is charged
///   in full to `naval_air` and not at all to `strategic_air`, because
///   [`take_section_ns`] swaps the slot to zero and a second reader would get
///   nothing anyway -- naming the owner makes that a decision instead of an
///   artefact of iteration order. `naval_air` is therefore over-charged and
///   `strategic_air` under-charged; only their SUM is a build term, and the
///   upper bound is a sum.
///
///   With tranche 6 both air sections are converted again, so the slot is
///   claimed by `naval_air` once more and that is the arrangement described
///   above: `naval_air` carries all three builds, `strategic_air` and
///   `naval_combats` carry none, and only the sum of the three is a build
///   term. It matters less than it reads: the build span enters the UPPER
///   bound only, and the kill-switch this lane converts against is read on the
///   API lane, which no build span touches.
/// * `combats` brackets `build_combats`, which returns `combats` AND
///   `ended_combats` from one call. The span is charged to `combats`, the
///   member that is three orders of magnitude the larger of the two, and
///   `ended_combats` carries none. Same rule as the air pair: over-charge the
///   named owner, under-charge its partner, and let only their sum be read as a
///   build term.
/// * `diplomacy_world` brackets `project_market_trade_lend_lease`, which writes
///   lend-lease members into `countries` AFTER the countries span has closed.
///   The span is therefore not purely its own member's construction, and by
///   name it is charged entirely to a 293-byte member. It is left that way for
///   the same reason `naval_and_air` is: `countries` is one of the seven
///   sections whose bands are already published, and re-pointing a span at it
///   would re-measure the base configuration rather than this tranche. The
///   whole span is 3 microseconds per game hour against `countries`'s 22.5
///   milliseconds of build -- 0.013% -- so the mis-attribution is real and
///   moves no bound. Measured on t30 and t31, in `runs/dq_v3_lane_ei/`.
/// * `sidecars` brackets nine members at once and cannot be attributed to any
///   one of them, so members built there carry no build term. The report names
///   every such member rather than letting a silent zero flatter the bound.
/// * `map_mirrors`, `late_resolution` and `assembly` are spans no document
///   member is named after, so nothing claims them and their nanoseconds enter
///   no bound. They are whole-document work v3 would still owe, and they are
///   named here so their absence from the upper bound is on the record rather
///   than in it.
fn build_span_for(section: &str) -> Option<usize> {
    if let Some(index) = section_index(section) {
        return Some(index);
    }
    match section {
        "naval_air" => section_index("naval_and_air"),
        _ => None,
    }
}

/// A JSON string literal, by SUBSTITUTION rather than escaping.
///
/// Backslashes become forward slashes and quotes become apostrophes, which is
/// lossy and deliberately so: every value that reaches here is a name, a path or
/// a diagnostic, and a report that is readable in a terminal is worth more than
/// one that round-trips a quote.
///
/// Control characters are substituted too, and that is not cosmetic. Two of the
/// strings reaching this function are outside this program's control — the
/// `section_filter` comes from `argv`, and the executable-identity refusals
/// carry an OS error message — and a raw control character inside a JSON string
/// makes the WHOLE report unparseable, so one stray byte would cost the run
/// rather than the field.
fn json_string(body: &str) -> String {
    let sanitized: String = body
        .chars()
        .map(|c| match c {
            '\\' => '/',
            '"' => '\'',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    format!("\"{sanitized}\"")
}

/// The pathname `current_exe` returns, and a best-effort digest of the file
/// opened by resolving it, as `(path, sha256)`.
///
/// The name is deliberately neither "the identity of the binary writing this
/// report" nor "the file the process was launched from". Both are claims this
/// function cannot make: the pathname is resolved once to obtain it and again
/// to open it, so what is hashed is whatever that pathname resolved to at OPEN
/// time. Four review rounds cut this sentence down; the section below is what
/// survived.
///
/// # Why the report carries it
///
/// Nothing else does. Lane GE's `PROVENANCE.txt` says so in its own words: it is
/// "the lane's own record of the runs" and "NOT an independent binding of a
/// binary to a JSON", because "nothing in a run JSON names or hashes the binary
/// that wrote it, so a reader is trusting this record for the mapping". A band
/// is only as good as the code that produced it, and this instrument's numbers
/// have already been read across builds that differed by an uncommitted hunk.
/// Hashing the file that pathname resolves to, at the moment we report, is
/// enough to tell two builds apart in the artifact rather than in a note beside
/// it — which is the whole job. It is not, and does not claim to be, an
/// attestation.
///
/// # Why a failure is reported and not fatal
///
/// A run that has already spent its measurement window must not be thrown away
/// because `current_exe` was unavailable — but it must not silently look bound
/// either. A refusal string is neither a path nor a 64-hex digest, so no reader
/// can mistake one for the other.
///
/// The encoding is PER FIELD, and the asymmetry is deliberate. If `current_exe`
/// itself fails there is no path to report, so both fields carry the refusal. If
/// the pathname resolves and the file it names cannot be opened or read, the
/// path is KEPT and only `executable_sha256` carries the refusal — a reader
/// learns which pathname was reported and that the file it resolved to could
/// not be read, which is strictly more than blanking both.
///
/// # Exactly what this is, stated as narrowly as it is true
///
/// It is the SHA-256 of the bytes read from one file object, obtained by
/// resolving the pathname `current_exe` returned, on an unsynchronised
/// filesystem. That sentence is the whole guarantee. Three things follow from
/// it that a reader must not assume away:
///
/// * **Not atomic with the naming.** `current_exe` resolves a pathname and
///   `File::open` resolves that same pathname again. A replacement, a rename,
///   or a redirected reparse point between the two calls means the digest is of
///   whatever the path named at OPEN time.
/// * **Not a content snapshot.** The read is a sequence of 64 KiB reads from
///   one handle. Holding a handle fixes the file OBJECT across later renames;
///   it does not freeze the object's contents. A writer truncating or
///   rewriting the file mid-read can produce a digest spliced from two states,
///   or a short read that ends the loop early. Nothing here detects that.
/// * **Not the loaded module.** Proving the bytes are the ones the loader
///   mapped needs the loader's mapping, not a file read; and nothing covers a
///   replacement between process start and this call.
///
/// Two earlier revisions of this comment overstated it — first claiming
/// `File::open` closed the naming gap that `fs::read` left (it does not; both
/// resolve the pathname), then claiming one handle gave "one consistent view of
/// one file" (it does not; a handle is not a snapshot). Both were caught in
/// review, and the second is why this section now states the mechanism rather
/// than a property.
///
/// # Why that is enough here, and what would be needed if it were not
///
/// The job is to tell two builds of an off-by-default measurement instrument
/// apart inside its own report, on a developer machine, where the executable is
/// not being rewritten while it runs. For that, a best-effort digest is exactly
/// right and a fail-closed would throw away a spent measurement window for a
/// condition that does not occur.
///
/// If this ever needs to be an attestation, the closure is not in the standard
/// library: bring the path back OUT of the handle
/// (`GetFinalPathNameByHandleW`), and take the content from a stable source —
/// the loader's own mapping, or a file opened with sharing denied so no writer
/// can move under the read. Both are platform-specific unsafe calls, which is
/// the dependency this instrument declines to carry, not an oversight.
fn executable_identity() -> (String, String) {
    let path = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            let refusal = format!("unavailable: current_exe failed: {error}");
            return (refusal.clone(), refusal);
        }
    };
    let display = path.to_string_lossy().into_owned();
    // Opened once and read through the handle. That is not a snapshot and not
    // atomic with the naming above; the function's own note states both rather
    // than implying them away.
    let mut file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) => {
            return (
                display,
                format!("unavailable: opening the file at that path failed: {error}"),
            )
        }
    };
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        match std::io::Read::read(&mut file, &mut buffer) {
            Ok(0) => break,
            Ok(read) => hasher.update(&buffer[..read]),
            Err(error) => {
                return (
                    display,
                    format!("unavailable: reading the file at that path failed: {error}"),
                )
            }
        }
    }
    (display, snapshot::sha256::to_hex(&hasher.finalize()))
}

/// THE ONE PLACE a report becomes a report.
///
/// `run` builds the body and hands it here; this adds the schema and the
/// executable-file identity and closes the object. It exists so those two are not
/// optional: a report cannot be emitted without going through this function, so
/// a test that drives this function covers what every report carries. The
/// alternative — splicing the identity into `run`'s own `format!` — was
/// untestable in this crate, because `run` takes a capture spool and a test
/// that deleted the splice stayed green.
fn report_envelope(body: &str) -> String {
    let (path, sha) = executable_identity();
    format!(
        "{{\"schema\":\"dq-v3-prototype-v1\",\
         \"executable_path\":{},\"executable_sha256\":{},{body}}}",
        json_string(&path),
        json_string(&sha)
    )
}

/// Parse a grain-depth spec: a default depth, optionally followed by
/// `,<section>=<depth>` overrides.
///
/// One depth for every section is the wrong shape and the sweep says so: at
/// depth 2 `divisions` already emits FEWER bytes than v2's patch while `map`
/// emits eleven times more, because the same depth lands on a 44-byte child in
/// one section and a 594-byte one in the other. The knob has to be per section
/// or the experiment measures the compromise instead of the design.
pub fn parse_grain_depths(spec: &str) -> Result<(usize, BTreeMap<String, usize>), String> {
    let mut parts = spec.split(',');
    let default = parts
        .next()
        .ok_or("empty grain-depth spec")?
        .trim()
        .parse::<usize>()
        .map_err(|_| format!("grain-depth spec must start with a depth, got {spec}"))?;
    let mut overrides = BTreeMap::new();
    for part in parts {
        let (name, depth) = part
            .split_once('=')
            .ok_or_else(|| format!("grain-depth override {part} is not <section>=<depth>"))?;
        if !COVERED_SECTIONS.iter().any(|(covered, _)| *covered == name) {
            return Err(format!("{name} is not one of the covered sections"));
        }
        let depth = depth
            .trim()
            .parse::<usize>()
            .map_err(|_| format!("grain-depth override {part} has no depth"))?;
        if depth == 0 {
            return Err("grain depth 0 would make the whole section one grain".to_string());
        }
        overrides.insert(name.to_string(), depth);
    }
    if default == 0 {
        return Err("grain depth 0 would make the whole section one grain".to_string());
    }
    Ok((default, overrides))
}

/// Which of [`COVERED_SECTIONS`] this run converts.
///
/// A tranche's band can only be read against the band it grew from, and this
/// machine moves 35-55% between one session and the next -- the A/B that
/// produced tranche 1's first numbers ran `apply_domain_data`, which neither
/// lane touches, 37% slower than the session before it. Comparing a tranche's
/// JSON against a previous session's JSON therefore measures the machine.
///
/// The subset is a parameter so both configurations can be run by ONE binary,
/// alternating, inside one window. It defaults to the whole list, so the
/// established six-argument invocation is unchanged and means what it always
/// meant.
fn select_sections(filter: Option<&str>) -> Result<Vec<(&'static str, &'static str)>, String> {
    let Some(filter) = filter else {
        return Ok(COVERED_SECTIONS.to_vec());
    };
    let wanted: Vec<&str> = filter.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    if wanted.is_empty() {
        return Err("the section filter selects nothing".to_string());
    }
    for name in &wanted {
        if !COVERED_SECTIONS.iter().any(|(covered, _)| covered == name) {
            return Err(format!("{name} is not one of the covered sections"));
        }
    }
    // Filtered in the CONST's order, not the filter's. The order decides which
    // section's `build_snapshot` span is harvested first and which lane runs
    // first on a given hour, so letting the command line reorder it would make
    // two spellings of one configuration measure differently.
    Ok(COVERED_SECTIONS
        .iter()
        .filter(|(name, _)| wanted.contains(name))
        .copied()
        .collect())
}

pub fn run(
    raw_h4cr_path: &FsPath,
    game_id: &str,
    identity_mode: StreamIdentityMode,
    expected_source_len: u64,
    grain_depth_spec: &str,
    max_hours: u64,
    section_filter: Option<&str>,
) -> Result<String, String> {
    let (grain_depth, depth_overrides) = parse_grain_depths(grain_depth_spec)?;
    let active_sections = select_sections(section_filter)?;
    if !foldprof::enabled() {
        return Err("set FOLD_PROFILE=1: the adapter-inclusive bound needs the fold's own \
                    per-section build spans, and without them it would silently report the \
                    lower bound twice"
            .to_string());
    }
    let raw_h4cr_path = std::fs::canonicalize(raw_h4cr_path).map_err(|e| e.to_string())?;
    let source_file = std::fs::File::open(&raw_h4cr_path).map_err(|e| e.to_string())?;
    let mut cursor = HourCursor::from_reader_with_source(
        Box::new(source_file),
        expected_source_len,
        game_id.to_string(),
        identity_mode,
        Some(IndexSourceDescriptor {
            kind: "stream_file".to_string(),
            canonical_path: raw_h4cr_path.to_string_lossy().into_owned(),
        }),
    );

    // Two sections mapped to one span would not double-charge -- the second
    // reader would find the slot already swapped to zero -- so the failure is
    // silent and shows up only as a build term that quietly moved between
    // sections when the table was reordered. It is checked instead of trusted.
    //
    // Checked over the WHOLE const and not merely the active subset: a
    // collision the subset happens to avoid is still a collision, and a filter
    // that hid one would let it reappear on the next tranche's run.
    let mut spans_seen: BTreeMap<usize, &str> = BTreeMap::new();
    for (name, _) in COVERED_SECTIONS {
        if let Some(index) = build_span_for(name) {
            if let Some(other) = spans_seen.insert(index, name) {
                return Err(format!(
                    "sections {other} and {name} both claim build span {}, and the \
                     second would silently be charged nothing",
                    foldprof::SECTIONS[index]
                ));
            }
        }
    }
    let sections_without_build_span: Vec<&str> = active_sections
        .iter()
        .filter(|(name, _)| build_span_for(name).is_none())
        .map(|(name, _)| *name)
        .collect();

    let mut runs: Vec<SectionRun> = Vec::new();
    for (name, spelling) in active_sections.iter().copied() {
        let spec = SectionSpec {
            name: name.to_string(),
            shape: shape_for(spelling)?,
            grain_depth: depth_overrides.get(name).copied().unwrap_or(grain_depth),
        };
        runs.push(SectionRun {
            name: name.to_string(),
            spec: spec.clone(),
            state: SectionState::new(spec.clone()),
            replica: SectionState::new(spec.clone()),
            previous: None,
            report: SectionReport::default(),
        });
    }

    let mut decode = Duration::ZERO;
    let mut doc_parse = Duration::ZERO;
    let mut instrument = Duration::ZERO;
    let mut hours = 0u64;
    let mut document_bytes_last = 0u64;
    // Sum of every top-level member's own canonical bytes, on the same hour the
    // covered sections are sized from. The raw snapshot length is not a
    // commensurable denominator: it also carries the members' names and the
    // document's punctuation, which no section's byte count includes, so a
    // coverage taken against it understates -- and with a stale section size it
    // could even exceed one.
    let mut document_member_bytes_last = 0u64;
    // Members of the coverage hour's document that this run did NOT convert,
    // largest first. See where it is filled for why the ratio alone is not
    // enough.
    let mut uncovered_members_last: Vec<(String, u64)> = Vec::new();
    let mut coverage_hour = 0i64;
    let mut apply_ns_first_hour = 0u64;
    let mut apply_whole_state_ns_first_hour = 0u64;
    let mut oracle_section_hours = 0u64;
    let mut divergences: Vec<String> = Vec::new();
    let mut missing_sections: BTreeMap<String, u64> = BTreeMap::new();

    // Clear whatever the fold accumulated before the first hour we price.
    for slot in foldprof::SECTION_NS.iter() {
        slot.store(0, Ordering::Relaxed);
    }
    foldprof::APPLY_NS.store(0, Ordering::Relaxed);
    foldprof::APPLY_WHOLE_STATE_NS.store(0, Ordering::Relaxed);
    foldprof::reset_domains();
    foldprof::BUILD_NS.store(0, Ordering::Relaxed);
    foldprof::SERIALIZE_NS.store(0, Ordering::Relaxed);

    let wall_started = Instant::now();
    while hours < max_hours {
        let started = Instant::now();
        let next = cursor.next_hour();
        decode += started.elapsed();
        let Some(hour) = next else { break };
        let hour: WorldHour = hour.map_err(|e| format!("decode: {e:?}"))?;
        hours += 1;
        document_bytes_last = hour.snapshot.len() as u64;

        // Harvest the fold's own section spans for THIS hour before anything
        // else can add to them.
        let build_ns: Vec<Duration> = active_sections
            .iter()
            .map(|(name, _)| build_span_for(name).map(take_section_ns).unwrap_or_default())
            .collect();

        let started = Instant::now();
        let document = CanonValue::parse(&hour.snapshot)
            .map_err(|e| format!("hour {} is not canonical: {e}", hour.hour_index))?;
        doc_parse += started.elapsed();
        let CanonValue::Object(members) = &document else {
            return Err(format!("hour {} is not an object", hour.hour_index));
        };
        if hours == 1 {
            // Everything up to here is a section's initialization, which no lane
            // is charged for; the apply term has to skip it too. The per-domain
            // rows are simply restarted, so they price the same window the
            // headline apply number does rather than a superset of it.
            apply_ns_first_hour = foldprof::APPLY_NS.load(Ordering::Relaxed);
            apply_whole_state_ns_first_hour =
                foldprof::APPLY_WHOLE_STATE_NS.load(Ordering::Relaxed);
            foldprof::reset_domains();
        }
        let started = Instant::now();
        // ONE serialization pass for both the denominator and the list of what
        // is being extrapolated over. Two passes would render every member of a
        // 35 MB document twice an hour to answer two questions about the same
        // bytes -- and this is the instrument's own wall time, which is reported
        // and should not be inflated by the instrument.
        document_member_bytes_last = 0;
        uncovered_members_last.clear();
        for (name, member) in members {
            let bytes = member.to_json_bytes().len() as u64;
            document_member_bytes_last += bytes;
            if !active_sections.iter().any(|(active, _)| active == name) {
                uncovered_members_last.push((name.clone(), bytes));
            }
        }
        // WHICH members the projection is extrapolating over, by name and by
        // bytes.
        //
        // `projected_full_conversion_s_per_hour` divides the measured v3 term by
        // `coverage_by_bytes`, which charges every unconverted member the
        // CONVERTED set's cost per byte. That is only conservative while the
        // remainder is no denser than the covered set, and the remainder is not
        // a fixed quantity -- it shrinks as tranches land, and what survives is
        // whatever was left out ON PURPOSE, which is exactly the population most
        // likely to have been left out for being expensive. A reader given only
        // a coverage ratio cannot tell a 6% remainder of ordinary members from a
        // 6% remainder of known outliers, and the two give the same number here.
        // Naming them is what makes the extrapolation auditable instead of
        // merely disclosed.
        uncovered_members_last.sort_by(|left, right| right.1.cmp(&left.1));
        coverage_hour = hour.hour_index;
        instrument += started.elapsed();

        // Alternate which lane runs first.
        let v2_first = hours % 2 == 1;

        for (index, run) in runs.iter_mut().enumerate() {
            let Some((_, section)) = members.iter().find(|(name, _)| *name == run.name) else {
                *missing_sections.entry(run.name.clone()).or_default() += 1;
                continue;
            };
            run.report.hours_present += 1;
            if run.previous.is_some() {
                run.report.lane.build += build_ns[index];
            }

            // A section's FIRST hour is not a comparison. v2 has no previous
            // tree to diff against, so it pays no delta at all, while v3 installs
            // the whole section from nothing. Folding that hour into the totals
            // would compare "no v2 delta" against a full v3 initialization and
            // book the difference as v3 overhead.
            let measure = run.previous.is_some();
            if measure {
                run.report.hours_measured += 1;
            }
            let (v2, v3) = if v2_first {
                let v2 = price_v2(run, section, measure)?;
                let v3 = price_v3(run, section, hour.hour_index, measure)?;
                (v2, v3)
            } else {
                let v3 = price_v3(run, section, hour.hour_index, measure)?;
                let v2 = price_v2(run, section, measure)?;
                (v2, v3)
            };
            instrument += v3.instrument;

            // ---- the equivalence oracle, on this hour's published bytes -----
            let started = Instant::now();
            let projected = run.state.project();
            if projected == v2.bytes {
                oracle_section_hours += 1;
            } else if divergences.len() < 8 {
                divergences.push(format!(
                    "{} hour {}: v3 projected {} bytes, v2 section is {} bytes",
                    run.name,
                    hour.hour_index,
                    projected.len(),
                    v2.bytes.len()
                ));
            }
            if run.replica.project() != projected && divergences.len() < 8 {
                divergences.push(format!(
                    "{} hour {}: the replica's projection differs from the producer's",
                    run.name, hour.hour_index
                ));
            }
            instrument += started.elapsed();

            let stats = run.state.take_stats();
            run.replica.take_stats();
            if !measure {
                run.report.section_bytes_last = v2.bytes.len() as u64;
                run.report.section_bytes_hour = hour.hour_index;
                run.report.grains_last = run.state.grain_count() as u64;
                run.report.retained_bytes_last = run.state.retained_grain_bytes();
                run.report.retained_path_bytes_last = run.state.retained_path_bytes();
                run.report.retained_scratch_bytes_last =
                    run.state.retained_scratch_bytes() + run.replica.retained_scratch_bytes();
                run.previous = Some(section.clone());
                continue;
            }
            run.report.v3_calls += stats.calls;
            run.report.v3_no_ops += stats.no_ops;
            run.report.v3_root_changes += stats.root_changes;
            run.report.v3_grain_bytes_written += stats.grain_bytes_written;
            run.report.v3_delta_bytes += v3.delta_bytes as u64;
            run.report.v3_mutations += v3.mutations;
            run.report.v2_patch_bytes += v2.patch_bytes as u64;
            if v2.patch_bytes > 0 {
                let amplification = v3.delta_bytes as f64 / v2.patch_bytes as f64;
                if amplification > run.report.worst_amplification {
                    run.report.worst_amplification = amplification;
                    run.report.worst_amplification_hour = hour.hour_index;
                }
            }
            if run.state.root_kind() == RootKind::Scalar {
                run.report.hours_scalar += 1;
            }
            run.report.section_bytes_last = v2.bytes.len() as u64;
            run.report.section_bytes_hour = hour.hour_index;
            run.report.grains_last = run.state.grain_count() as u64;
            run.report.retained_bytes_last = run.state.retained_grain_bytes();
            run.report.retained_path_bytes_last = run.state.retained_path_bytes();
                run.report.retained_scratch_bytes_last =
                    run.state.retained_scratch_bytes() + run.replica.retained_scratch_bytes();
            run.previous = Some(section.clone());
        }
    }

    let wall = wall_started.elapsed();
    let peak = recorder_host::proc_mem::peak_memory();
    let apply_ns = foldprof::APPLY_NS.load(Ordering::Relaxed);

    if hours == 0 {
        return Err("stream emitted no hours".to_string());
    }

    // Every lane's totals skip each section's first hour, so the denominator
    // has to skip it as well.
    let measured_hours = runs
        .iter()
        .map(|run| run.report.hours_measured)
        .max()
        .unwrap_or(0);
    if measured_hours == 0 {
        return Err("no covered section was present for two hours, so nothing was compared"
            .to_string());
    }
    let hours_f = measured_hours as f64;
    let mut rows: Vec<String> = Vec::new();
    let mut total = Lane::default();
    let mut covered_bytes = 0u64;
    let mut v2_patch_total = 0u64;
    let mut v3_delta_total = 0u64;
    let mut resident_one = 0u64;
    // The cross-hour scratch of BOTH states, which is why it is accumulated
    // apart from resident_one and added once rather than doubled.
    let mut resident_scratch_both = 0u64;
    let mut absent_at_coverage_hour: Vec<String> = Vec::new();
    for run in &runs {
        let lane = run.report.lane;
        // Each section's rates use ITS OWN measured hours. Dividing every
        // section by the run's maximum would understate any section that was
        // absent for part of the capture -- and on t31 `special_projects` is
        // exactly that kind of section.
        let section_hours = if run.report.hours_measured > 0 {
            run.report.hours_measured as f64
        } else {
            1.0
        };
        total.v2_serialize += lane.v2_serialize;
        total.v2_parse += lane.v2_parse;
        total.v2_sha += lane.v2_sha;
        total.v2_diff_encode += lane.v2_diff_encode;
        total.v3_api += lane.v3_api;
        total.v3_adapter += lane.v3_adapter;
        total.build += lane.build;
        if run.report.section_bytes_hour == coverage_hour {
            covered_bytes += run.report.section_bytes_last;
        } else {
            absent_at_coverage_hour.push(run.name.clone());
        }
        v2_patch_total += run.report.v2_patch_bytes;
        v3_delta_total += run.report.v3_delta_bytes;
        resident_one += run.report.retained_bytes_last
            + run.report.retained_path_bytes_last
            + run.report.grains_last * PER_GRAIN_OVERHEAD;
        resident_scratch_both += run.report.retained_scratch_bytes_last;
        let v2_total =
            lane.build + lane.v2_serialize + lane.v2_parse + lane.v2_sha + lane.v2_diff_encode;
        rows.push(format!(
            "{{\"section\":{},\"hours_present\":{},\"hours_measured\":{},\
             \"hours_null_rooted\":{},\
             \"section_bytes_last\":{},\"grains_last\":{},\
             \"retained_grain_bytes_last\":{},\"retained_path_bytes_last\":{},\
             \"v2_s_per_hour\":{{\"build\":{:.6},\"serialize\":{:.6},\"parse\":{:.6},\
             \"sha256\":{:.6},\"diff_encode\":{:.6},\"total\":{:.6}}},\
             \"v3_s_per_hour\":{{\"api_lower\":{:.6},\"adapter\":{:.6},\
             \"adapter_inclusive_upper\":{:.6}}},\
             \"v2_patch_bytes_per_hour\":{:.1},\"v3_delta_bytes_per_hour\":{:.1},\
             \"delta_byte_ratio\":{:.4},\"worst_hour_delta_byte_ratio\":{:.4},\
             \"worst_hour\":{},\"mutations_per_hour\":{:.1},\
             \"mutations_total\":{},\
             \"mutations_dropped_by_negative_control\":{},\
             \"api_calls_per_hour\":{:.1},\"no_ops_per_hour\":{:.1},\
             \"root_changes\":{},\"grain_bytes_written_per_hour\":{:.1}}}",
            json_string(&run.name),
            run.report.hours_present,
            run.report.hours_measured,
            run.report.hours_scalar,
            run.report.section_bytes_last,
            run.report.grains_last,
            run.report.retained_bytes_last,
            run.report.retained_path_bytes_last,
            secs(lane.build) / section_hours,
            secs(lane.v2_serialize) / section_hours,
            secs(lane.v2_parse) / section_hours,
            secs(lane.v2_sha) / section_hours,
            secs(lane.v2_diff_encode) / section_hours,
            secs(v2_total) / section_hours,
            secs(lane.v3_api) / section_hours,
            secs(lane.v3_adapter) / section_hours,
            secs(lane.v3_api + lane.v3_adapter + lane.build) / section_hours,
            run.report.v2_patch_bytes as f64 / section_hours,
            run.report.v3_delta_bytes as f64 / section_hours,
            if run.report.v2_patch_bytes > 0 {
                run.report.v3_delta_bytes as f64 / run.report.v2_patch_bytes as f64
            } else {
                0.0
            },
            run.report.worst_amplification,
            run.report.worst_amplification_hour,
            run.report.v3_mutations as f64 / section_hours,
            run.report.v3_mutations,
            run.report.mutations_dropped,
            run.report.v3_calls as f64 / section_hours,
            run.report.v3_no_ops as f64 / section_hours,
            run.report.v3_root_changes,
            run.report.v3_grain_bytes_written as f64 / section_hours,
        ));
    }

    // A negative control that dropped nothing is not a negative control, and it
    // reports exactly what a passing build reports: no divergence. Refusing the
    // run is the only way that distinction survives into a file somebody reads
    // later.
    //
    // Two ways it can drop nothing, and both are refused here: the named
    // section is not in the active set (a typo, or a filter that excluded it),
    // or it is active but never presented a changed upsert after its first hour
    // (absent, unchanging, or changing only by removal).
    let dropped_total: u64 = runs.iter().map(|run| run.report.mutations_dropped).sum();
    if let Ok(name) = std::env::var("DQ_DROP_MUTATION") {
        if !name.is_empty() {
            if !runs.iter().any(|run| run.name == name) {
                return Err(format!(
                    "DQ_DROP_MUTATION={name} names a section this run does not convert, so \
                     the negative control dropped nothing and its zero divergences mean \
                     nothing"
                ));
            }
            if dropped_total == 0 {
                return Err(format!(
                    "DQ_DROP_MUTATION={name} dropped no mutation in {measured_hours} measured \
                     hours -- after its first commit the section never presented an upsert at \
                     an address that already stood, so there was nothing to make stale -- and \
                     the control therefore proves nothing about the oracle"
                ));
            }
        }
    }

    let v2_covered = secs(
        total.build + total.v2_serialize + total.v2_parse + total.v2_sha + total.v2_diff_encode,
    ) / hours_f;
    let v3_lower = secs(total.v3_api) / hours_f;
    let v3_upper = secs(total.v3_api + total.v3_adapter + total.build) / hours_f;
    let coverage = if document_member_bytes_last > 0 {
        covered_bytes as f64 / document_member_bytes_last as f64
    } else {
        0.0
    };
    if !(0.0..=1.0).contains(&coverage) || coverage == 0.0 {
        return Err(format!(
            "covered sections measure {covered_bytes} bytes against a \
             {document_member_bytes_last} byte document at hour {coverage_hour}, which is \
             not a coverage"
        ));
    }
    let apply_per_hour = apply_ns.saturating_sub(apply_ns_first_hour) as f64 / 1e9 / hours_f;
    // The measured floor on how much of `apply_domain_data` is proportional to
    // the accumulated state rather than to the record -- whole-accumulator deep
    // copies and whole-section `serde_json` renders done inside the per-record
    // path. Reported beside the headline, never subtracted from it: the
    // threshold is read against `projected_lower`, which charges v3 the whole
    // apply term. The alternative below says what the same measurement gives if
    // that named work is credited to v3's design instead, and it is labelled an
    // alternative because deciding it is the owner's, not the instrument's.
    let apply_whole_state_per_hour = foldprof::APPLY_WHOLE_STATE_NS
        .load(Ordering::Relaxed)
        .saturating_sub(apply_whole_state_ns_first_hour) as f64
        / 1e9
        / hours_f;
    // Full conversion, extrapolated by the covered byte share. Both bounds add
    // back `apply_domain_data`, which v3 still owes: reducers must still ingest
    // the wire records whatever the state looks like.
    let projected_lower = v3_lower / coverage + apply_per_hour;
    let projected_upper = v3_upper / coverage + apply_per_hour;
    let projected_lower_less_whole_state =
        v3_lower / coverage + (apply_per_hour - apply_whole_state_per_hour).max(0.0);

    let verdict = if !divergences.is_empty() {
        "oracle_failed"
    } else if projected_upper <= 0.20 {
        "pass"
    } else if projected_lower <= 0.20 {
        "inconclusive"
    } else {
        "reject"
    };

    let (peak_private, peak_ws) = peak
        .map(|p| (p.peak_private_bytes, p.peak_working_set_bytes))
        .unwrap_or((0, 0));
    // Member names moved OUT of the paths and into one shared interner, so
    // `retained_path_bytes` is now the paths' own segment arrays and nothing
    // else. The names have to be added back or the memory threshold would be
    // answered with a number that got smaller because the accounting moved.
    //
    // Added ONCE, not doubled: the producer and the replica address their
    // grains with the same interned names, so a second resident state does not
    // bring a second vocabulary. It is also published on its own line, because
    // "two states" scaling it would be the wrong model for it.
    //
    // `resident_scratch_both` is added once for the same reason and a different
    // one: it is already the sum over BOTH states. A section keeps its hour
    // journal and its commit buffer across hours on purpose -- that is what
    // keeps the allocator off the mutation path -- so it holds their CAPACITY
    // whether or not they are empty when the report reads them, and an hour that
    // wrote a multi-megabyte upsert phase leaves that behind it.
    let interner_bytes = fold_v3::intern::retained_bytes();
    let interned_names = fold_v3::intern::interned_names();
    let two_states = 2 * resident_one + resident_scratch_both + interner_bytes;
    let two_states_full = (two_states as f64 / coverage) as u64;

    let divergence_rows: Vec<String> = divergences.iter().map(|row| json_string(row)).collect();
    let missing_rows: Vec<String> = missing_sections
        .iter()
        .map(|(name, count)| format!("[{},{count}]", json_string(name)))
        .collect();

    let absent_rows: Vec<String> = absent_at_coverage_hour
        .iter()
        .map(|name| json_string(name))
        .collect();
    let uncovered_rows: Vec<String> = uncovered_members_last
        .iter()
        .map(|(name, bytes)| {
            format!(
                "{{\"member\":{},\"bytes\":{bytes},\"share\":{:.6}}}",
                json_string(name),
                if document_member_bytes_last > 0 {
                    *bytes as f64 / document_member_bytes_last as f64
                } else {
                    0.0
                }
            )
        })
        .collect();
    let depth_rows: Vec<String> = runs
        .iter()
        .map(|run| format!("[{},{}]", json_string(&run.name), run.spec.grain_depth))
        .collect();

    // A section whose construction the fold does not bracket separately carries
    // no build term, which makes the upper bound less of an over-charge than it
    // reads as. Published rather than absorbed.
    let no_build_rows: Vec<String> = sections_without_build_span
        .iter()
        .map(|name| json_string(name))
        .collect();

    // `apply_domain_data` is a large share of the projected budget and v3 does
    // not touch it, so a cut to it survives whatever the fold state looks like.
    // One number over ~70 reducers cannot say where to cut; these rows can.
    //
    // The share was MEASURED at 54-58% by lane GE on the t30/t31 streams under
    // the 39-section configuration (`runs/ge_v3_stage3/MEASUREMENT_TABLE.txt`).
    // That is a reading of a run, not a property of this code, and it is stated
    // as history here because two things have moved under it since: the covered
    // set is now 41 sections including the re-added air sections, and the fold
    // no longer writes five air members into `strategic_air` as well as
    // `naval_air`. Both change the denominator. `projected_full_conversion` is
    // computed per run from that run's own spans, so the report is not affected
    // — only a reader carrying the old percentage forward would be.
    let domain_rows: Vec<String> = foldprof::domain_rows()
        .into_iter()
        .map(|(domain, ns, records, payload)| {
            format!(
                "{{\"domain\":{domain},\"s_per_hour\":{:.6},\"records_per_hour\":{:.1},\
                 \"payload_bytes_per_hour\":{:.1},\"ns_per_record\":{:.1}}}",
                ns as f64 / 1e9 / hours_f,
                records as f64 / hours_f,
                payload as f64 / hours_f,
                if records > 0 {
                    ns as f64 / records as f64
                } else {
                    0.0
                },
            )
        })
        .collect();

    Ok(report_envelope(&format!(
        "\"grain_depth\":{grain_depth},\
         \"grain_depth_spec\":{},\"grain_depths\":[{}],\
         \"hours\":{hours},\"document_bytes_last\":{document_bytes_last},\
         \"document_member_bytes_last\":{document_member_bytes_last},\
         \"coverage_hour\":{coverage_hour},\"hours_measured\":{measured_hours},\
         \"sections_absent_at_coverage_hour\":[{}],\
         \"sections_without_build_span\":[{}],\
         \"section_filter\":{},\"sections_converted\":{},\
         \"covered_bytes_last\":{covered_bytes},\"coverage_by_bytes\":{coverage:.4},\
         \"extrapolated_over\":[{}],\
         \"extrapolation_note\":\"projected_full_conversion charges every member \
         in extrapolated_over the CONVERTED set's seconds per byte; that is a \
         floor for any of them measured to be denser, and the members left out \
         last are the ones most likely to be\",\
         \"wall_s\":{:.3},\"decode_fold_s\":{:.3},\"document_parse_s\":{:.3},\
         \"instrument_only_s\":{:.3},\
         \"apply_domain_data_s_per_hour\":{apply_per_hour:.6},\
         \"apply_whole_state_s_per_hour\":{apply_whole_state_per_hour:.6},\
         \"projected_lower_if_whole_state_apply_work_is_credited_to_v3\":\
         {projected_lower_less_whole_state:.6},\
         \"covered_s_per_hour\":{{\"v2\":{v2_covered:.6},\
         \"v3_api_lower\":{v3_lower:.6},\"v3_adapter_inclusive_upper\":{v3_upper:.6},\
         \"ratio_lower\":{:.4},\"ratio_upper\":{:.4}}},\
         \"projected_full_conversion_s_per_hour\":{{\"lower\":{projected_lower:.6},\
         \"upper\":{projected_upper:.6},\"threshold\":0.20}},\
         \"two_resident_states_bytes\":{{\"covered\":{two_states},\
         \"projected_full\":{two_states_full},\"threshold\":1073741824,\
         \"interned_name_bytes\":{interner_bytes},\
         \"interned_names\":{interned_names},\
         \"cross_hour_scratch_bytes_both_states\":{resident_scratch_both}}},\
         \"peak_private_bytes\":{peak_private},\"peak_working_set_bytes\":{peak_ws},\
         \"v2_patch_bytes_per_hour\":{:.1},\"v3_delta_bytes_per_hour\":{:.1},\
         \"oracle_section_hours_equal\":{oracle_section_hours},\
         \"mutations_dropped_by_negative_control\":{dropped_total},\
         \"divergences\":[{}],\"sections_missing_from_document\":[{}],\
         \"unpriced_v3_obligations\":\"checkpoint framing, H4IX index, whole-cache \
         SHA-256, verify_every_hour, container write\",\
         \"apply_domain_data_by_domain\":[{}],\
         \"verdict\":{},\"sections\":[{}]",
        json_string(grain_depth_spec),
        depth_rows.join(","),
        absent_rows.join(","),
        no_build_rows.join(","),
        json_string(section_filter.unwrap_or("<all>")),
        active_sections.len(),
        uncovered_rows.join(","),
        secs(wall),
        secs(decode),
        secs(doc_parse),
        secs(instrument),
        if v2_covered > 0.0 {
            v3_lower / v2_covered
        } else {
            0.0
        },
        if v2_covered > 0.0 {
            v3_upper / v2_covered
        } else {
            0.0
        },
        v2_patch_total as f64 / hours_f,
        v3_delta_total as f64 / hours_f,
        divergence_rows.join(","),
        missing_rows.join(","),
        domain_rows.join(","),
        json_string(verdict),
        rows.join(","),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The six sections the prototype's published bands were measured with must
    /// keep charging the build span they were measured against. `build_span_for`
    /// was introduced to give the tranche-1 sections a span at all, and the way
    /// that goes wrong is quietly, by moving somebody else's.
    #[test]
    fn the_original_six_still_charge_their_own_build_span() {
        for name in [
            "special_projects",
            "countries",
            "map",
            "supply",
            "divisions",
            "states",
        ] {
            // The concrete index first. `assert_eq!(build_span_for, section_index)`
            // alone passes when BOTH are `None` -- which is exactly what a
            // renamed or deleted profiler span produces, and it would drop that
            // section's build seconds out of the upper bound while the test went
            // on agreeing.
            let index = section_index(name)
                .unwrap_or_else(|| panic!("{name} has no build span in the fold profiler"));
            assert_eq!(
                build_span_for(name),
                Some(index),
                "{name} no longer charges the build span it was measured with"
            );
        }
    }

    /// Two sections mapped to one span is not a double charge -- it is a SILENT
    /// zero, because `take_section_ns` swaps the slot and the second reader
    /// finds nothing. The run-time check in `run` catches it on a capture; this
    /// catches it on the table, which is where it would be introduced.
    #[test]
    fn no_two_sections_claim_one_build_span() {
        let mut seen: BTreeMap<usize, &str> = BTreeMap::new();
        for (name, _) in COVERED_SECTIONS {
            if let Some(index) = build_span_for(name) {
                let clash = seen.insert(index, name);
                assert!(
                    clash.is_none(),
                    "{name} shares build span {} with {}",
                    foldprof::SECTIONS[index],
                    clash.unwrap_or("")
                );
            }
        }
    }

    /// Every covered section must be spelled with a shape the prototype can
    /// build, or the run dies on a capture instead of here.
    #[test]
    fn every_covered_section_has_a_buildable_shape() {
        for (name, spelling) in COVERED_SECTIONS {
            shape_for(spelling).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }

    /// A filter selects WHICH sections run, never in what order they run.
    ///
    /// The order decides which build span is harvested first and the order the
    /// sections are priced in, so a filter that reordered the run would make two
    /// spellings of one configuration measure differently -- and the two
    /// spellings would be a command line typed twice.
    #[test]
    fn a_filter_selects_sections_without_reordering_them() {
        let forward = select_sections(Some("countries,states")).unwrap();
        let backward = select_sections(Some("states,countries")).unwrap();
        assert_eq!(forward, backward);
        let names: Vec<&str> = forward.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, ["countries", "states"]);
    }

    #[test]
    fn no_filter_converts_everything() {
        assert_eq!(select_sections(None).unwrap(), COVERED_SECTIONS.to_vec());
    }

    /// A name appearing twice would be run twice and, worse, would have its
    /// bytes added to `covered_bytes` twice -- inflating `coverage_by_bytes`,
    /// which every projection divides by. The band would come out LOW, which is
    /// the direction that gets a design admitted.
    #[test]
    fn no_section_is_listed_twice() {
        let mut seen: BTreeMap<&str, ()> = BTreeMap::new();
        for (name, _) in COVERED_SECTIONS {
            assert!(
                seen.insert(name, ()).is_none(),
                "{name} appears twice in COVERED_SECTIONS"
            );
        }
    }

    /// The seven sections whose bands are already on the record must stay at the
    /// FRONT of the const, in their order.
    ///
    /// The const's order is the run order, and the run order decides which
    /// section's build span is harvested first and which lane runs first on a
    /// given hour. A tranche that appended its sections anywhere but the end
    /// would re-measure the base configuration under a different schedule, and
    /// every band read against it would be reading that instead.
    #[test]
    fn the_base_seven_still_lead_the_const_in_order() {
        let head: Vec<&str> = COVERED_SECTIONS.iter().take(7).map(|(n, _)| *n).collect();
        assert_eq!(
            head,
            [
                "special_projects",
                "countries",
                "map",
                "supply",
                "divisions",
                "states",
                "order_of_battle"
            ]
        );
    }

    /// The two sections removed on 2026-08-07 are back, and they are back at a
    /// COARSE grain depth.
    ///
    /// The removal was not about the sections; it was about the depth they were
    /// cut at. At the default depth 3 both decompose to an array element's
    /// individual FIELDS -- a four-byte value under a forty-byte address --
    /// which is what produced 33k mutations an hour between them. Re-adding
    /// them WITHOUT the depth override would silently rebuild exactly the
    /// configuration that was ruled out, and every band in `runs/ge_v3_stage3/`
    /// would be filed against a configuration nobody measured.
    ///
    /// Depth 2 is the census argmin on both captures and it is checked here
    /// rather than trusted, because the override is a string in another
    /// constant and nothing else connects the two.
    ///
    /// The SHAPE is pinned for a harder reason. Both members are JSON objects,
    /// and a section spelled `"array"` here would not fail: `price_v3` routes a
    /// value that does not match its configured shape to a SCALAR root holding
    /// the whole member at the empty path, which projects the same bytes,
    /// agrees with its replica, holds an agreeing state root and presents no
    /// ordinals for the density scan to complain about. Every check in the
    /// oracle would pass while the run measured one whole-section grain an hour
    /// instead of the 715-3,675 element grains it is supposed to. Nothing else
    /// in the file distinguishes those two runs.
    #[test]
    fn the_readded_air_sections_are_cut_coarse() {
        let (_, overrides) = parse_grain_depths(MEASURED_GRAIN_DEPTHS)
            .unwrap_or_else(|e| panic!("MEASURED_GRAIN_DEPTHS does not parse: {e}"));
        for name in ["strategic_air", "naval_air"] {
            let spelling = COVERED_SECTIONS
                .iter()
                .find(|(covered, _)| *covered == name)
                .map(|(_, spelling)| *spelling)
                .unwrap_or_else(|| {
                    panic!("{name} is not converted, so the section census is not full")
                });
            assert_eq!(
                shape_for(spelling).unwrap_or_else(|e| panic!("{name}: {e}")),
                Shape::Object,
                "{name} is not cut as the object it is, so its whole section is one grain"
            );
            assert_eq!(
                overrides.get(name),
                Some(&2),
                "{name} is converted at a depth other than the one it was measured at"
            );
        }
    }

    /// Tranche 6 sits at the END of the const, after every section whose band
    /// is already published.
    ///
    /// The const's order is the run order, and the run order decides which
    /// section's build span is harvested first and which lane runs first on a
    /// given hour. Inserting the two anywhere but the end would re-schedule
    /// every section after them.
    #[test]
    fn the_readded_air_sections_are_last() {
        let tail: Vec<&str> = COVERED_SECTIONS
            .iter()
            .skip(COVERED_SECTIONS.len() - 2)
            .map(|(name, _)| *name)
            .collect();
        assert_eq!(tail, ["strategic_air", "naval_air"]);
    }

    /// [`MEASURED_GRAIN_DEPTHS`] must parse, and the base configuration's three
    /// depths must still be the ones its published bands were measured with.
    ///
    /// `parse_grain_depths` refuses an override naming a section that is not
    /// covered, so this also catches a misspelt member name -- which would
    /// otherwise be accepted silently by nothing at all and leave that section
    /// running at the default depth while the spec said otherwise.
    #[test]
    fn the_measured_depth_spec_parses_and_preserves_the_base_depths() {
        let (default, overrides) = parse_grain_depths(MEASURED_GRAIN_DEPTHS)
            .unwrap_or_else(|e| panic!("MEASURED_GRAIN_DEPTHS does not parse: {e}"));
        assert_eq!(default, 3, "the default depth the base was measured at moved");
        assert_eq!(overrides.get("supply"), Some(&2));
        assert_eq!(overrides.get("divisions"), Some(&2));
        for name in ["special_projects", "countries", "map", "states", "order_of_battle"] {
            assert!(
                !overrides.contains_key(name),
                "{name} gained a depth override it was not measured with"
            );
        }
        // Spot-checks on the tail, so a truncated or reflowed literal is caught:
        // the two ends of the tranche list and the one section cut at depth 1
        // for being 13k grains of 104 bytes.
        assert_eq!(overrides.get("provinces"), Some(&1));
        assert_eq!(overrides.get("_r2_selected_designer_identity_live"), Some(&1));
    }

    /// Every section this lane converts must be reachable by the filter under
    /// the exact name the document spells it with, because that name is also
    /// what `run` matches the document member by. A section named here but
    /// spelled differently in the document is simply never converted, and the
    /// only symptom is a coverage a few tenths lower than expected.
    #[test]
    fn every_covered_section_is_selectable_by_its_own_name() {
        for (name, _) in COVERED_SECTIONS {
            let selected = select_sections(Some(name))
                .unwrap_or_else(|e| panic!("{name} is not selectable: {e}"));
            assert_eq!(selected.len(), 1, "{name} selected {} sections", selected.len());
            assert_eq!(selected[0].0, name);
        }
    }

    /// The name both of these use as "not covered" is a MISSPELLING of a real
    /// member -- `province` for `provinces` -- and not an invented word.
    ///
    /// The sentinel has to be something a conversion tranche can never turn
    /// into a real section, or the test decays into a pass the day that section
    /// is converted, which is exactly what happened to `provinces` here. A
    /// misspelling stays wrong forever, and it is also the failure the check
    /// exists for.
    const NOT_A_SECTION: &str = "province";

    #[test]
    fn a_filter_naming_something_uncovered_is_refused() {
        // A typo that was accepted would silently measure a SMALLER
        // configuration than the command line asked for, and the band would be
        // filed against a configuration nobody ran.
        assert!(!COVERED_SECTIONS.iter().any(|(n, _)| *n == NOT_A_SECTION));
        assert!(select_sections(Some(&format!("countries,{NOT_A_SECTION}"))).is_err());
        assert!(select_sections(Some("")).is_err());
        assert!(select_sections(Some(",")).is_err());
    }

    /// A grain-depth override for a section that is not covered is a typo, and
    /// accepting it would run the DEFAULT depth while the command line said
    /// otherwise -- a band filed against a grain spec that was never used.
    #[test]
    fn a_grain_depth_override_for_an_uncovered_section_is_refused() {
        assert!(!COVERED_SECTIONS.iter().any(|(n, _)| *n == NOT_A_SECTION));
        assert!(parse_grain_depths(&format!("3,{NOT_A_SECTION}=2")).is_err());
        assert!(parse_grain_depths("3,order_of_battle=0").is_err());
        assert!(parse_grain_depths("0").is_err());
        let (default, overrides) = parse_grain_depths("3,supply=2,order_of_battle=4").unwrap();
        assert_eq!(default, 3);
        assert_eq!(overrides.get("supply"), Some(&2));
        assert_eq!(overrides.get("order_of_battle"), Some(&4));
    }

    /// Every report carries the digest of the file `current_exe` names, under
    /// the two field names, and the whole report parses as JSON.
    ///
    /// Named for what it checks. An earlier name, "binds itself to the running
    /// binary", was a claim the mechanism does not support (Codex round 10).
    ///
    /// kills: hashing anything but that file's bytes — a constant, the source
    /// path, `argv[0]` unresolved, or a digest of the empty slice. Each of
    /// those still produces 64 hex characters, so only comparing against an
    /// independently computed digest of the same file can tell them apart.
    /// Also kills renaming or dropping either field from [`report_envelope`],
    /// dropping the schema, and breaking the chunked read (a truncated or
    /// double-counted buffer changes the digest).
    ///
    /// What it CANNOT kill, so the contract is not read as more than it is:
    /// this test resolves the pathname a second time itself and reads the file
    /// the same unsynchronised way, so it detects neither the non-atomicity
    /// between `current_exe`'s naming and `File::open`'s resolution nor a
    /// writer moving the file's contents under either read. Both are properties
    /// of the environment rather than of this code; [`executable_identity`]'s
    /// note states them and names what closing them would take.
    ///
    /// It drives `report_envelope` rather than a fragment, and that is the
    /// point: `run` cannot produce a report except through it, so there is no
    /// longer an emission path this test does not cover. An earlier version
    /// spliced the fields into `run`'s own `format!` and asserted on a helper —
    /// deleting the splice left every report without them and the test green.
    #[test]
    fn the_report_carries_the_digest_of_the_file_current_exe_names() {
        let (path, sha) = executable_identity();

        // THE EXPECTATION IS OBTAINED INDEPENDENTLY. Hashing the path the
        // production function returned would pass for ANY readable file it
        // chose to name -- including the source path its `kills:` contract
        // claims to rule out. So the test asks the OS itself, asserts the
        // production path is that path, and hashes the file the test resolved.
        let named = std::env::current_exe().expect("the test process has a path");
        assert_eq!(
            std::path::PathBuf::from(&path),
            named,
            "the reported path must be the one `current_exe` names, not some other              readable file"
        );
        assert!(named.is_file(), "and it must be a real file: {named:?}");
        let expected = snapshot::sha256::to_hex(&Sha256::digest(
            &std::fs::read(&named).expect("the file at that path is readable"),
        ));
        assert_eq!(
            sha, expected,
            "the reported digest must be the digest of the file at that path"
        );
        assert_eq!(sha.len(), 64);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));

        // A WHOLE REPORT, parsed the way a reader parses one. The body is a
        // stand-in for the ~40 measured fields; what is under test is the
        // envelope every report is wrapped in.
        let report = report_envelope("\"hours\":3,\"verdict\":\"inconclusive\"");
        let object: serde_json::Value =
            serde_json::from_str(&report).expect("a report must be valid JSON");
        assert_eq!(
            object["schema"].as_str(),
            Some("dq-v3-prototype-v1"),
            "the envelope carries the schema"
        );
        assert_eq!(
            object["executable_sha256"].as_str(),
            Some(expected.as_str()),
            "and the digest, under that exact name"
        );
        assert_eq!(
            object["executable_path"].as_str(),
            Some(json_string(&named.to_string_lossy()).trim_matches('"')),
            "and the path, through the report's own substitution"
        );
        // And the body survives the wrapping, so the envelope cannot be
        // satisfied by dropping what it wraps.
        assert_eq!(object["hours"].as_u64(), Some(3));
        assert_eq!(object["verdict"].as_str(), Some("inconclusive"));
    }

    /// A control character in a value this program does not control must not be
    /// able to make the whole report unparseable.
    ///
    /// kills: removing the `is_control` arm from `json_string`. Both strings
    /// exercised here reach it in production — `section_filter` straight from
    /// `argv`, and the executable-identity refusal, which carries an OS error
    /// message.
    #[test]
    fn json_string_cannot_emit_a_raw_control_character() {
        let hostile = "a\u{0}b\nc\td\u{1b}e\"f\\g";
        let emitted = json_string(hostile);
        assert!(
            !emitted.chars().any(char::is_control),
            "emitted {emitted:?}"
        );
        assert_eq!(emitted, "\"a b c d e'f/g\"");
        assert!(serde_json::from_str::<serde_json::Value>(&emitted).is_ok());
    }
}
