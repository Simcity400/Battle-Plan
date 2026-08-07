//! Reduce a `range-tracer` sidecar into the capture-floor census table.
//!
//! ```text
//! range_census <sidecar.bin> [--span 0x10000] [--json out.json]
//! ```
//!
//! Every number printed comes from [`collector::range_census`], which is
//! covered by `cargo test -p collector --lib`. This binary parses arguments,
//! formats, and nothing else — deliberately, so that the arithmetic a decision
//! rests on cannot live somewhere a test does not reach.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Write};
use std::path::PathBuf;

use collector::range_census::{
    exact_json_number, gap_curve_report, read_trace, walk_all, window_coverage, BracketCensus, Census, DerivationSpan,
    GapCurve, GapCurveReport, WindowCoverage, DEFAULT_DERIVATION_SPANS, DEFAULT_GAP_THRESHOLDS,
};

/// Entry cap for the reduction. 128M entries is 3 GiB of sidecar; beyond that
/// the tool refuses rather than censusing a prefix.
const MAX_ENTRIES: usize = 128 * 1024 * 1024;

fn main() {
    if let Err(error) = run() {
        eprintln!("range_census: {error}");
        std::process::exit(1);
    }
}

fn usage() -> String {
    "usage: range_census <sidecar.bin> [--span <bytes>] [--json <out.json>]\n       \
     range_census <sidecar.bin> --gap-curve [--span <bytes>] [--gaps 16,32,...] \
     [--tags 42,43,...] [--json <out.json>]"
        .to_string()
}

/// Tags the curve report breaks out in the text table. Every tag is in the
/// JSON regardless; these are the families the floor re-pricing named.
const DEFAULT_FOCUS_TAGS: [u16; 5] = [42, 43, 29, 81, u16::MAX];

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let input = args.next().map(PathBuf::from).ok_or_else(usage)?;
    let mut span = DerivationSpan(0x1_0000);
    let mut json: Option<PathBuf> = None;
    let mut gap_curve_mode = false;
    let mut gaps: Vec<u64> = DEFAULT_GAP_THRESHOLDS.to_vec();
    let mut focus_tags: Vec<u16> = DEFAULT_FOCUS_TAGS.to_vec();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--span" => {
                let raw = args.next().ok_or_else(|| format!("--span needs a value\n{}", usage()))?;
                let value = parse_u64(&raw)?;
                span = DerivationSpan(value);
            }
            "--gap-curve" => gap_curve_mode = true,
            "--gaps" => {
                let raw = args.next().ok_or_else(|| format!("--gaps needs a list\n{}", usage()))?;
                gaps = raw
                    .split(',')
                    .filter(|piece| !piece.trim().is_empty())
                    .map(parse_u64)
                    .collect::<Result<Vec<u64>, String>>()?;
                if gaps.is_empty() {
                    return Err("--gaps listed no thresholds".to_string());
                }
            }
            "--tags" => {
                let raw = args.next().ok_or_else(|| format!("--tags needs a list\n{}", usage()))?;
                focus_tags = raw
                    .split(',')
                    .filter(|piece| !piece.trim().is_empty())
                    .map(|piece| {
                        parse_u64(piece).and_then(|value| {
                            u16::try_from(value).map_err(|_| format!("tag {value} is not a u16"))
                        })
                    })
                    .collect::<Result<Vec<u16>, String>>()?;
            }
            "--json" => {
                json = Some(
                    args.next()
                        .map(PathBuf::from)
                        .ok_or_else(|| format!("--json needs a path\n{}", usage()))?,
                );
            }
            other => return Err(format!("unknown argument {other}\n{}", usage())),
        }
    }

    let file = File::open(&input)
        .map_err(|error| format!("open {}: {error}", input.display()))?;
    let (header, entries) = read_trace(&mut BufReader::new(file), MAX_ENTRIES)?;

    if gap_curve_mode {
        // `gaps` must be ascending for the tables to read as a curve, and the
        // gap-0 row is the baseline every other row is measured against.
        gaps.sort_unstable();
        gaps.dedup();
        if gaps.first() != Some(&0) {
            gaps.insert(0, 0);
        }
        return run_gap_curve(&input, header, &entries, span, &gaps, &focus_tags, json);
    }

    let census = Census::build(header, &entries, span);

    println!("RANGE / BYTE / DEPTH CENSUS  {}", input.display());
    println!("  frame ordinal      {}", header.frame_ordinal);
    println!(
        "  trace entries      {}{}",
        entries.len(),
        if header.truncated() {
            "   *** TRUNCATED: every figure below is a LOWER BOUND ***"
        } else {
            ""
        }
    );
    println!("  derivation span    {:#x}", span.0);
    println!();
    println!(
        "  ROWS ARE BRACKET TAGS, NOT DOMAINS. A tag is usually a domain id, but\n  \
         tag 200 is the country bundle (one worker covering domains 10..=17) and\n  \
         a few workers cover a neighbour. Tag 65535 is everything read outside\n  \
         any bracket. The FULL SET totals below are unaffected by this grouping;\n  \
         `shared bytes` is a LOWER bound because sharing inside one bundled\n  \
         bracket is already deduplicated within its row."
    );
    println!();
    println!(
        "  {:>6}  {:>12}  {:>14}  {:>12}  {:>12}  {:>14}  {:>5}  {:>10}",
        "bracket", "reads", "bytes moved", "distinct", "coalesced", "unique bytes", "depth", "roots"
    );
    let mut rows: Vec<&BracketCensus> = census.brackets.iter().collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.unique_bytes));
    for row in &rows {
        println!(
            "  {:>6}  {:>12}  {:>14}  {:>12}  {:>12}  {:>14}  {:>5}  {:>10}",
            row.bracket_tag,
            row.read_count,
            row.bytes_moved,
            row.distinct_ranges,
            row.coalesced_ranges,
            row.unique_bytes,
            row.max_depth,
            row.root_reads,
        );
    }

    let per_bracket_sum: u64 = census.brackets.iter().map(|row| row.unique_bytes).sum();
    println!();
    println!("  FULL SET (every bracket, deduplicated)");
    println!("    distinct ranges      {}", census.full_set_distinct_ranges);
    println!("    coalesced ranges     {}", census.full_set_coalesced_ranges);
    println!(
        "    unique bytes         {} ({:.3} MiB)",
        census.full_set_unique_bytes,
        mib(census.full_set_unique_bytes)
    );
    println!(
        "    sum of per-bracket   {} ({:.3} MiB)   <- NOT the full set",
        per_bracket_sum,
        mib(per_bracket_sum)
    );
    println!(
        "    shared bytes         {} ({:.3} MiB)   = the double-count that sum commits",
        census.shared_bytes,
        mib(census.shared_bytes)
    );
    println!();
    println!("  DEPTH SENSITIVITY (max over all brackets, by derivation span)");
    for (span, depth) in Census::depth_by_span(header, &entries, &DEFAULT_DERIVATION_SPANS) {
        println!("    span {:#010x}   depth {depth}", span.0);
    }
    println!();
    println!(
        "  EXCLUDED BY CONSTRUCTION: the full-heap discovery sweep\n    \
         (`read_bytes_scan_reporting`) is not traced — it searches address\n    \
         space rather than observing a domain's data. Domains that discover\n    \
         by sweeping therefore report their POST-discovery reads only."
    );

    if let Some(path) = json {
        let mut object: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        object.insert("frame_ordinal".into(), header.frame_ordinal.into());
        object.insert("truncated".into(), header.truncated().into());
        object.insert("derivation_span".into(), span.0.into());
        object.insert(
            "full_set_distinct_ranges".into(),
            census.full_set_distinct_ranges.into(),
        );
        object.insert(
            "full_set_coalesced_ranges".into(),
            census.full_set_coalesced_ranges.into(),
        );
        object.insert(
            "full_set_unique_bytes".into(),
            census.full_set_unique_bytes.into(),
        );
        object.insert("shared_bytes".into(), census.shared_bytes.into());
        object.insert(
            "brackets".into(),
            serde_json::Value::Array(
                census
                    .brackets
                    .iter()
                    .map(|row| {
                        serde_json::json!({
                            "bracket_tag": row.bracket_tag,
                            "read_count": row.read_count,
                            "bytes_moved": row.bytes_moved,
                            "distinct_ranges": row.distinct_ranges,
                            "coalesced_ranges": row.coalesced_ranges,
                            "unique_bytes": row.unique_bytes,
                            "max_depth": row.max_depth,
                            "root_reads": row.root_reads,
                        })
                    })
                    .collect(),
            ),
        );
        let mut out = File::create(&path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        serde_json::to_writer_pretty(&mut out, &object)
            .map_err(|error| format!("encode {}: {error}", path.display()))?;
        out.write_all(b"\n")
            .map_err(|error| format!("write {}: {error}", path.display()))?;
    }
    Ok(())
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn mib128(bytes: u128) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// The rejection rule the floor re-pricing set for 64-byte safe merging.
const VERDICT_GAP: u64 = 64;
const VERDICT_MAX_RANGES: u64 = 180_000;
const VERDICT_MAX_BYTES: u128 = 100 * 1024 * 1024;

/// Window sizes the per-family recipe check sweeps. 2048/4096 are the d42
/// candidates and 64/128 the d43 ones; the rest bracket them.
const WINDOW_SIZES: [u64; 8] = [64, 128, 256, 512, 1024, 2048, 4096, 8192];

fn print_curve(curve: &GapCurve) {
    println!(
        "  {} — {} distinct (address,len) pairs, {} gap-0 spans, {} B ({:.3} MiB) genuinely read",
        curve.label,
        curve.distinct_ranges,
        curve.baseline_ranges,
        curve.baseline_bytes,
        mib128(curve.baseline_bytes)
    );
    println!(
        "  {:>8}  {:>12}  {:>14}  {:>10}  {:>14}  {:>10}  {:>14}  {:>10}",
        "gap B", "ranges", "bytes", "MiB", "over-copy B", "MiB", "worst-case B", "MiB"
    );
    for point in &curve.points {
        println!(
            "  {:>8}  {:>12}  {:>14}  {:>10.3}  {:>14}  {:>10.3}  {:>14}  {:>10.3}",
            point.gap,
            point.ranges,
            point.bytes,
            mib128(point.bytes),
            point.over_copy_bytes,
            mib128(point.over_copy_bytes),
            point.worst_case_envelope_bytes,
            mib128(point.worst_case_envelope_bytes),
        );
    }
    println!();
}

fn curve_json(curve: &GapCurve) -> serde_json::Value {
    serde_json::json!({
        "label": curve.label,
        "distinct_ranges": curve.distinct_ranges,
        "baseline_ranges": curve.baseline_ranges,
        "baseline_bytes": exact_json_number(curve.baseline_bytes),
        "points": curve.points.iter().map(|point| serde_json::json!({
            "gap": point.gap,
            "ranges": point.ranges,
            "bytes": exact_json_number(point.bytes),
            "over_copy_bytes": exact_json_number(point.over_copy_bytes),
            "worst_case_envelope_bytes": exact_json_number(point.worst_case_envelope_bytes),
        })).collect::<Vec<_>>(),
    })
}

fn coverage_json(tag: u16, anchor_kind: &str, coverage: &WindowCoverage) -> serde_json::Value {
    serde_json::json!({
        "bracket_tag": tag,
        "anchor_kind": anchor_kind,
        "anchors": coverage.anchors,
        "window_len": coverage.window_len,
        "window_ranges": coverage.window_ranges,
        "window_bytes": exact_json_number(coverage.window_bytes),
        "data_bytes": exact_json_number(coverage.data_bytes),
        "covered_bytes": exact_json_number(coverage.covered_bytes),
        "data_ranges": coverage.data_ranges,
        "fully_covered_ranges": coverage.fully_covered_ranges,
        "partially_covered_ranges": coverage.partially_covered_ranges,
        "uncovered_ranges": coverage.uncovered_ranges,
        "byte_coverage_pct": coverage.byte_coverage_pct(),
        "range_coverage_pct": coverage.range_coverage_pct(),
    })
}

#[allow(clippy::too_many_arguments)]
fn run_gap_curve(
    input: &PathBuf,
    header: collector::range_census::TraceHeader,
    entries: &[collector::range_census::TraceEntry],
    span: DerivationSpan,
    gaps: &[u64],
    focus_tags: &[u16],
    json: Option<PathBuf>,
) -> Result<(), String> {
    let walks = walk_all(entries, span);
    let report: GapCurveReport = gap_curve_report(&walks, gaps);

    println!("GAP-MERGE CURVE  {}", input.display());
    println!("  frame ordinal      {}", header.frame_ordinal);
    println!(
        "  trace entries      {}{}",
        entries.len(),
        if header.truncated() {
            "   *** TRUNCATED: every figure below is a LOWER BOUND ***"
        } else {
            ""
        }
    );
    println!("  derivation span    {:#x}", span.0);
    println!("  bracket tags       {}", walks.len());
    println!();
    println!(
        "  A `gap B` row merges two spans when the UNREAD hole between them is at\n  \
         most that many bytes (`<=`), and the resulting `bytes` INCLUDE the bridged\n  \
         holes — that is what a copier issuing one descriptor would move.\n  \
         `over-copy B` is `bytes` minus the gap-0 union: the padding measured.\n  \
         `worst-case B` is the no-trace bound `baseline + gap*(baseline_ranges-1)`,\n  \
         i.e. what a reader must assume with only the census table in hand."
    );
    println!();
    println!(
        "  SCOPES. GLOBAL deduplicates ACROSS bracket tags before merging: one\n  \
         central witness image, a shared byte copied once. SUM OF BRACKETS gives\n  \
         every bracket its own image, so shared bytes are paid for repeatedly.\n  \
         THE COPY PLAN NEEDS THE GLOBAL SCOPE — the per-bracket sum is printed\n  \
         only to price the alternative that was rejected."
    );
    println!();
    print_curve(&report.global);
    print_curve(&report.summed_per_bracket);

    let by_tag: BTreeMap<u16, &GapCurve> = report
        .per_bracket
        .iter()
        .filter_map(|curve| {
            curve
                .label
                .strip_prefix("tag ")
                .and_then(|tag| tag.parse::<u16>().ok())
                .map(|tag| (tag, curve))
        })
        .collect();
    for tag in focus_tags {
        match by_tag.get(tag) {
            Some(curve) => print_curve(curve),
            None => println!("  tag {tag} — NOT PRESENT in this trace\n"),
        }
    }

    // ---- the rejection rule -------------------------------------------------
    let verdict_point = report
        .global
        .points
        .iter()
        .find(|point| point.gap == VERDICT_GAP);
    println!("  REJECTION RULE: does {VERDICT_GAP} B merging reach <= {VERDICT_MAX_RANGES} ranges and <= {} MiB per image?", VERDICT_MAX_BYTES / (1024 * 1024));
    match verdict_point {
        Some(point) => {
            let ranges_ok = point.ranges <= VERDICT_MAX_RANGES;
            let bytes_ok = point.bytes <= VERDICT_MAX_BYTES;
            println!(
                "    GLOBAL at {} B: {} ranges ({}), {} bytes / {:.3} MiB ({})",
                point.gap,
                point.ranges,
                if ranges_ok { "PASS" } else { "FAIL" },
                point.bytes,
                mib128(point.bytes),
                if bytes_ok { "PASS" } else { "FAIL" },
            );
            println!(
                "    VERDICT: {}",
                if ranges_ok && bytes_ok {
                    "PASS — 64 B merging alone meets both limits"
                } else {
                    "FAIL — 64 B merging alone does NOT meet both limits"
                }
            );
            // The smallest listed threshold that would meet both, if any.
            let first_pass = report
                .global
                .points
                .iter()
                .find(|p| p.ranges <= VERDICT_MAX_RANGES && p.bytes <= VERDICT_MAX_BYTES);
            match first_pass {
                Some(p) => println!(
                    "    smallest listed threshold meeting both: {} B ({} ranges, {:.3} MiB)",
                    p.gap,
                    p.ranges,
                    mib128(p.bytes)
                ),
                None => println!(
                    "    NO listed threshold up to {} B meets both limits",
                    gaps.last().copied().unwrap_or(0)
                ),
            }
        }
        None => println!("    {VERDICT_GAP} B was not among the thresholds computed"),
    }
    println!();

    // ---- per-family window recipes -----------------------------------------
    println!(
        "  PER-FAMILY WINDOW RECIPES. A window recipe places [anchor, anchor+W)\n  \
         on every discovered anchor and copies the union. `roots` anchors on the\n  \
         DISTINCT addresses of depth-0 reads (the `roots` column of the census\n  \
         counts the EVENTS, not the addresses); `pointers` anchors on every\n  \
         heap-shaped value the bracket's reads returned, which is the set of\n  \
         object bases the reader actually dereferenced. A span that a window\n  \
         only CLIPS is not covered: the reader still needs the rest of it."
    );
    println!();
    let mut coverage_rows: Vec<serde_json::Value> = Vec::new();
    for tag in focus_tags {
        let Some(walk) = walks.get(tag) else { continue };
        println!(
            "  tag {tag} — {} gap-0 spans, {} B read, {} root addresses ({} root reads), {} observed pointers",
            walk.row.coalesced_ranges,
            walk.row.unique_bytes,
            walk.root_addresses.len(),
            walk.row.root_reads,
            walk.pointer_values.len(),
        );
        println!(
            "  {:>10}  {:>9}  {:>12}  {:>14}  {:>10}  {:>9}  {:>9}  {:>9}  {:>9}",
            "anchors",
            "window B",
            "win ranges",
            "win bytes",
            "win MiB",
            "byte cov%",
            "span cov%",
            "clipped",
            "missed"
        );
        for (kind, anchors) in [
            ("roots", &walk.root_addresses),
            ("pointers", &walk.pointer_values),
        ] {
            for window in WINDOW_SIZES {
                let coverage = window_coverage(&walk.ranges, anchors, window);
                println!(
                    "  {:>10}  {:>9}  {:>12}  {:>14}  {:>10.3}  {:>9.2}  {:>9.2}  {:>9}  {:>9}",
                    kind,
                    window,
                    coverage.window_ranges,
                    coverage.window_bytes,
                    mib128(coverage.window_bytes),
                    coverage.byte_coverage_pct(),
                    coverage.range_coverage_pct(),
                    coverage.partially_covered_ranges,
                    coverage.uncovered_ranges,
                );
                coverage_rows.push(coverage_json(*tag, kind, &coverage));
            }
        }
        println!();
    }

    if let Some(path) = json {
        let mut object: BTreeMap<String, serde_json::Value> = BTreeMap::new();
        object.insert("source".into(), input.display().to_string().into());
        object.insert("frame_ordinal".into(), header.frame_ordinal.into());
        object.insert("truncated".into(), header.truncated().into());
        object.insert("derivation_span".into(), span.0.into());
        object.insert("trace_entries".into(), (entries.len() as u64).into());
        object.insert("gap_thresholds".into(), gaps.to_vec().into());
        object.insert("global".into(), curve_json(&report.global));
        object.insert(
            "summed_per_bracket".into(),
            curve_json(&report.summed_per_bracket),
        );
        object.insert(
            "per_bracket".into(),
            serde_json::Value::Array(report.per_bracket.iter().map(curve_json).collect()),
        );
        object.insert(
            "window_coverage".into(),
            serde_json::Value::Array(coverage_rows),
        );
        let mut out = File::create(&path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        serde_json::to_writer_pretty(&mut out, &object)
            .map_err(|error| format!("encode {}: {error}", path.display()))?;
        out.write_all(b"\n")
            .map_err(|error| format!("write {}: {error}", path.display()))?;
    }
    Ok(())
}

fn parse_u64(raw: &str) -> Result<u64, String> {
    let trimmed = raw.trim();
    let parsed = if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16)
    } else {
        trimmed.parse::<u64>()
    };
    parsed.map_err(|error| format!("cannot parse {raw:?} as a number: {error}"))
}
