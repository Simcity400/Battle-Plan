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
    read_trace, Census, DerivationSpan, BracketCensus, DEFAULT_DERIVATION_SPANS,
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
    "usage: range_census <sidecar.bin> [--span <bytes>] [--json <out.json>]".to_string()
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let input = args.next().map(PathBuf::from).ok_or_else(usage)?;
    let mut span = DerivationSpan(0x1_0000);
    let mut json: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--span" => {
                let raw = args.next().ok_or_else(|| format!("--span needs a value\n{}", usage()))?;
                let value = parse_u64(&raw)?;
                span = DerivationSpan(value);
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
