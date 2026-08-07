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
pub const COVERED_SECTIONS: [(&str, &str); 9] = [
    ("special_projects", "object"),
    ("countries", "object"),
    ("map", "object"),
    ("supply", "object"),
    ("divisions", "keyed_array:div_uid"),
    ("states", "object"),
    // -- tranche 1, density order ------------------------------------------
    ("strategic_air", "object"),
    ("naval_air", "object"),
    ("order_of_battle", "object"),
];

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
    let mut changed: Vec<(Path, &CanonValue)> = Vec::new();
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
            _ => changed.push((path.clone(), *value)),
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
    for (index, (path, value)) in changed.into_iter().enumerate() {
        if drop_one && index == 0 {
            continue;
        }
        txn.upsert(path, value)
            .map_err(|e| format!("section {}: {e}", run.name))?;
    }
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
/// * `sidecars` brackets eight members at once and cannot be attributed to any
///   one of them, so members built there carry no build term. The report names
///   every such member rather than letting a silent zero flatter the bound.
fn build_span_for(section: &str) -> Option<usize> {
    if let Some(index) = section_index(section) {
        return Some(index);
    }
    match section {
        "naval_air" => section_index("naval_and_air"),
        _ => None,
    }
}

fn json_string(body: &str) -> String {
    format!("\"{}\"", body.replace('\\', "/").replace('"', "'"))
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
        document_member_bytes_last = members
            .iter()
            .map(|(_, member)| member.to_json_bytes().len() as u64)
            .sum();
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
        // Grain payload, plus the addresses, plus what the containers cost per
        // grain: a `Grain` is a `Vec` header, a 32-byte digest, a revision and
        // a 32-byte cached state-root contribution (96 B), a `Path` is a `Vec`
        // header (24 B), and a `BTreeMap` entry carries roughly another 16 B of
        // node share. Counting only the payload would answer the memory
        // threshold with the flattering half of the number.
        const PER_GRAIN_OVERHEAD: u64 = 136;
        resident_one += run.report.retained_bytes_last
            + run.report.retained_path_bytes_last
            + run.report.grains_last * PER_GRAIN_OVERHEAD;
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
            run.report.v3_calls as f64 / section_hours,
            run.report.v3_no_ops as f64 / section_hours,
            run.report.v3_root_changes,
            run.report.v3_grain_bytes_written as f64 / section_hours,
        ));
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
    let two_states = 2 * resident_one;
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

    // `apply_domain_data` is 54-58% of the projected budget and v3 does not
    // touch it, so a cut to it survives whatever the fold state looks like. One
    // number over ~70 reducers cannot say where to cut; these rows can.
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

    Ok(format!(
        "{{\"schema\":\"dq-v3-prototype-v1\",\"grain_depth\":{grain_depth},\
         \"grain_depth_spec\":{},\"grain_depths\":[{}],\
         \"hours\":{hours},\"document_bytes_last\":{document_bytes_last},\
         \"document_member_bytes_last\":{document_member_bytes_last},\
         \"coverage_hour\":{coverage_hour},\"hours_measured\":{measured_hours},\
         \"sections_absent_at_coverage_hour\":[{}],\
         \"sections_without_build_span\":[{}],\
         \"section_filter\":{},\"sections_converted\":{},\
         \"covered_bytes_last\":{covered_bytes},\"coverage_by_bytes\":{coverage:.4},\
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
         \"projected_full\":{two_states_full},\"threshold\":1073741824}},\
         \"peak_private_bytes\":{peak_private},\"peak_working_set_bytes\":{peak_ws},\
         \"v2_patch_bytes_per_hour\":{:.1},\"v3_delta_bytes_per_hour\":{:.1},\
         \"oracle_section_hours_equal\":{oracle_section_hours},\
         \"divergences\":[{}],\"sections_missing_from_document\":[{}],\
         \"unpriced_v3_obligations\":\"checkpoint framing, H4IX index, whole-cache \
         SHA-256, verify_every_hour, container write\",\
         \"apply_domain_data_by_domain\":[{}],\
         \"verdict\":{},\"sections\":[{}]}}",
        json_string(grain_depth_spec),
        depth_rows.join(","),
        absent_rows.join(","),
        no_build_rows.join(","),
        json_string(section_filter.unwrap_or("<all>")),
        active_sections.len(),
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
    ))
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

    #[test]
    fn a_filter_naming_something_uncovered_is_refused() {
        // A typo that was accepted would silently measure a SMALLER
        // configuration than the command line asked for, and the band would be
        // filed against a configuration nobody ran.
        assert!(select_sections(Some("countries,provinces")).is_err());
        assert!(select_sections(Some("")).is_err());
        assert!(select_sections(Some(",")).is_err());
    }

    /// A grain-depth override for a section that is not covered is a typo, and
    /// accepting it would run the DEFAULT depth while the command line said
    /// otherwise -- a band filed against a grain spec that was never used.
    #[test]
    fn a_grain_depth_override_for_an_uncovered_section_is_refused() {
        assert!(parse_grain_depths("3,provinces=2").is_err());
        assert!(parse_grain_depths("3,order_of_battle=0").is_err());
        assert!(parse_grain_depths("0").is_err());
        let (default, overrides) = parse_grain_depths("3,supply=2,order_of_battle=4").unwrap();
        assert_eq!(default, 3);
        assert_eq!(overrides.get("supply"), Some(&2));
        assert_eq!(overrides.get("order_of_battle"), Some(&4));
    }
}
