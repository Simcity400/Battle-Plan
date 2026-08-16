//! WHERE A CAPTURE'S BYTES GO, by record category, derived from the wire itself.
//!
//! # The question
//!
//! "Are replays bloated with diagnostic and safety machinery that is not replay
//! content?" That question is only answerable as a byte census over a real
//! capture, classified by the record vocabulary the producer actually writes —
//! not by reading the producer and guessing which branch is hot.
//!
//! # The categories, and why each record type lands where it does
//!
//! Every H4CR record carries a `record_type` in its 32-byte envelope, and the
//! ten legal types (`compact_contract::record_semantics`) partition cleanly:
//!
//! * `REPLAY_DATA` — `DOMAIN_DATA` (3). The domain rows the fold applies and
//!   the replay renders from. This is the product.
//! * `LOSS_ACCOUNTING` — `DOMAIN_GAP` (5), `FRAME_REFUSED` (6),
//!   `FRAME_DEGRADED` (7), `CAPTURE_TERMINATED` (8),
//!   `CLOCK_UNREADABLE_INTERVAL` (9), `UNTRUSTED_CLOCK_RECOVERY` (10). Named
//!   loss. Owner-ruled PRODUCT ("partial data still seals"): an hour that
//!   published without a domain, or never published at all, says so on the wire
//!   or the loss is silent. Never a cut candidate.
//! * `FRAMING` — `FRAME_END` (2) and `STREAM_HEADER` (4). `FRAME_END` is what
//!   closes an hour: `world_fold::ingest` dispatches on exactly two types, and
//!   this is one of them (`finish_hour`). `STREAM_HEADER` carries campaign
//!   identity. Both are consumed; neither is diagnostic.
//! * `PERF_BASE` — the canonical 40-byte `DOMAIN_PERF` (1) payload. This is the
//!   ambiguous one and it is deliberately NOT filed under diagnostics: a PERF
//!   record is a domain's TERMINAL OUTCOME
//!   (`compact_record_counts_toward_terminal_outcome`), the positive
//!   counterpart of a `DOMAIN_GAP`. Completeness accounting reads the record's
//!   existence. Its 40 bytes of counters are the price of that outcome slot,
//!   so they are reported as their own line rather than folded into either
//!   neighbour, and a reader can add them wherever they believe they belong.
//! * `PERF_DIAG_TRAILER` — the optional `PDU1` trailer past
//!   `COMPACT_DOMAIN_PERF_BASE_LEN`, present only when
//!   `COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER` is set. Per-frame timers and
//!   counters (`d81_phase_*`, `lw_*`, `domain47_manager_*`, `hub_*`).
//!
//!   THE TRAILER IS NOT UNIFORMLY DEV-ONLY, and calling it so was the first
//!   version's mistake (Codex review 019fd708-ad11-7c92-96b3-5235bb472881,
//!   finding 1). The host summarizer decodes every one of these units
//!   (`decode_compact_domain_perf_diagnostic_units`), and four of them decide
//!   `acceptance_telemetry.problems` — which
//!   `ProductionCapture.Validation.ps1` fails a capture over. So the trailer is
//!   split by CONSUMER, not by `UnitRole`: a role says how a producer MEANT a
//!   unit and says nothing about who reads it. See [`Consumer`].
//!
//!   The per-`UnitRole` and per-name breakdowns are still printed, but they are
//!   descriptive; the consumer split is the one that answers "what is safe to
//!   drop".
//!
//!   What no bucket contains is a per-frame SAMPLE that survives sealing.
//!   `world_fold::ingest` dispatches on `DOMAIN_DATA` and `FRAME_END` only, so
//!   no trailer byte reaches a checkpoint; the summariser's aggregates are the
//!   only diagnostic content in the sealed container, and they are kilobytes.
//!
//! # What this tool does NOT do
//!
//! It does not model. Every number printed is a span of bytes in the file, and
//! the spans partition the file exactly: the tool asserts that the classified
//! total plus any unparsed tail equals the file length, so a category cannot
//! silently absorb another's bytes.
//!
//! Envelope bytes are attributed to the record they frame, never pooled, so
//! "what would this category's absence save" reads directly off its line.

use compact_contract::campaign_identity::COMPACT_RECORD_TYPE_STREAM_HEADER;
use compact_contract::diagnostic_channel::{decode_units, UnitRole};
use compact_contract::{
    parse_compact_record_header, COMPACT_DOMAIN_PERF_BASE_LEN,
    COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC, COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_TYPE_CAPTURE_TERMINATED,
    COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL, COMPACT_RECORD_TYPE_DOMAIN_DATA,
    COMPACT_RECORD_TYPE_DOMAIN_GAP, COMPACT_RECORD_TYPE_DOMAIN_PERF,
    COMPACT_RECORD_TYPE_FRAME_DEGRADED, COMPACT_RECORD_TYPE_FRAME_END,
    COMPACT_RECORD_TYPE_FRAME_REFUSED, COMPACT_RECORD_TYPE_FREEZE_CENSUS,
    COMPACT_RECORD_TYPE_UNTRUSTED_CLOCK_RECOVERY,
};
use std::collections::BTreeMap;
use std::env;
use std::io::{BufWriter, Read, Write};
use std::path::PathBuf;

/// The census buckets. Ordering is report ordering.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Category {
    ReplayData,
    LossAccounting,
    Framing,
    PerfBase,
    PerfDiagTrailer,
    FreezeCensus,
    Unknown,
}

impl Category {
    fn label(self) -> &'static str {
        match self {
            Self::ReplayData => "(a) REPLAY DATA        DOMAIN_DATA",
            Self::LossAccounting => "(b) LOSS ACCOUNTING   GAP/REFUSED/DEGRADED/TERM/CLOCK",
            Self::Framing => "(d) FRAMING           FRAME_END + STREAM_HEADER",
            Self::PerfBase => "(d) PERF BASE         DOMAIN_PERF outcome + 40B counters",
            Self::PerfDiagTrailer => "(c) DEV PERF DIAG     PDU1 trailer units",
            Self::FreezeCensus => "(e) FREEZE CENSUS     H4FC stopped-window telemetry",
            Self::Unknown => "(e) UNKNOWN           unclassified record type",
        }
    }
}

/// Classify by record type alone. PERF splits further at the payload boundary.
fn category_of(record_type: u16) -> Category {
    match record_type {
        COMPACT_RECORD_TYPE_DOMAIN_DATA => Category::ReplayData,
        COMPACT_RECORD_TYPE_DOMAIN_GAP
        | COMPACT_RECORD_TYPE_FRAME_REFUSED
        | COMPACT_RECORD_TYPE_FRAME_DEGRADED
        | COMPACT_RECORD_TYPE_CAPTURE_TERMINATED
        | COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL
        | COMPACT_RECORD_TYPE_UNTRUSTED_CLOCK_RECOVERY => Category::LossAccounting,
        COMPACT_RECORD_TYPE_FRAME_END | COMPACT_RECORD_TYPE_STREAM_HEADER => Category::Framing,
        COMPACT_RECORD_TYPE_DOMAIN_PERF => Category::PerfBase,
        COMPACT_RECORD_TYPE_FREEZE_CENSUS => Category::FreezeCensus,
        _ => Category::Unknown,
    }
}

#[derive(Default, Clone, Copy)]
struct Bucket {
    records: u64,
    bytes: u64,
}

impl Bucket {
    fn add(&mut self, bytes: u64) {
        self.records += 1;
        self.bytes += bytes;
    }
}

#[derive(Default)]
struct Census {
    buckets: BTreeMap<Category, Bucket>,
    /// Per record type, so the (b) bucket can be shown itemised.
    per_type: BTreeMap<u16, Bucket>,
    /// PDU1 unit bytes by role.
    diag_by_role: BTreeMap<&'static str, Bucket>,
    /// PDU1 unit bytes by who actually reads the unit. THE answer table.
    diag_by_consumer: BTreeMap<Consumer, Bucket>,
    /// PDU1 unit bytes by unit name.
    diag_by_name: BTreeMap<String, Bucket>,
    /// PDU1 framing overhead: the `PDU1` magic, version and unit count that are
    /// not attributable to any single unit.
    diag_framing_bytes: u64,
    /// Records that carried the trailer flag.
    perf_with_trailer: u64,
    perf_total: u64,
    total_bytes: u64,
    records: u64,
    /// Bytes at the tail that did not parse as a record.
    unparsed_tail: u64,
    /// Non-fatal decode complaints, deduplicated.
    trailer_errors: BTreeMap<String, u64>,
}

impl Census {
    fn bucket(&mut self, category: Category, bytes: u64) {
        self.buckets.entry(category).or_default().add(bytes);
    }

    /// Attribute one record's bytes. `envelope` is charged to the record's own
    /// category; a PERF record additionally splits its payload.
    fn ingest(&mut self, domain_id: u16, record_type: u16, record_flags: u32, payload: &[u8]) {
        let record_len = (COMPACT_RECORD_HEADER_LEN + payload.len()) as u64;
        self.records += 1;
        self.total_bytes += record_len;
        self.per_type
            .entry(record_type)
            .or_default()
            .add(record_len);

        if record_type != COMPACT_RECORD_TYPE_DOMAIN_PERF {
            self.bucket(category_of(record_type), record_len);
            return;
        }

        self.perf_total += 1;
        let has_trailer = record_flags & COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER != 0;
        let trailer = if has_trailer {
            payload.get(COMPACT_DOMAIN_PERF_BASE_LEN..).unwrap_or(&[])
        } else {
            &[]
        };
        // Envelope + the canonical base ride with the outcome slot; only the
        // trailer past the base is the diagnostic candidate.
        let base_bytes = record_len - trailer.len() as u64;
        self.bucket(Category::PerfBase, base_bytes);
        if trailer.is_empty() {
            return;
        }
        self.perf_with_trailer += 1;
        self.buckets
            .entry(Category::PerfDiagTrailer)
            .or_default()
            .add(trailer.len() as u64);
        self.split_trailer(domain_id, trailer);
    }

    /// Break the PDU1 blob into per-unit spans. A trailer that will not decode
    /// is charged entirely to framing overhead and the reason is recorded, so
    /// its bytes are never lost from the total.
    fn split_trailer(&mut self, domain_id: u16, trailer: &[u8]) {
        if trailer.len() < 8 || &trailer[..4] != COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC {
            self.diag_framing_bytes += trailer.len() as u64;
            *self
                .trailer_errors
                .entry("PDU1 trailer missing or misframed".to_string())
                .or_default() += 1;
            return;
        }
        // magic(4) + version(2) is unattributable framing; decode_units adds the
        // 2-byte count itself, charged below.
        match decode_units(&trailer[6..]) {
            Ok((units, consumed)) => {
                // magic + version + unit count, plus anything decode_units did
                // not consume. A canonical trailer has NO remainder --
                // `validate_compact_record_semantics` rejects trailing bytes --
                // so this is defensive rather than expected, and it is here
                // because a census that silently drops bytes it did not
                // understand is worse than one that names them (Codex review
                // 019fd708, finding 4).
                let unconsumed = trailer.len().saturating_sub(6 + consumed);
                self.diag_framing_bytes += 8 + unconsumed as u64;
                for unit in units {
                    let bytes = unit.encoded_len() as u64;
                    self.diag_by_role
                        .entry(role_label(unit.role))
                        .or_default()
                        .add(bytes);
                    self.diag_by_consumer
                        .entry(Consumer::of(domain_id, &unit.name))
                        .or_default()
                        .add(bytes);
                    self.diag_by_name.entry(unit.name).or_default().add(bytes);
                }
            }
            Err(error) => {
                self.diag_framing_bytes += trailer.len() as u64;
                *self.trailer_errors.entry(error).or_default() += 1;
            }
        }
    }
}

fn role_label(role: UnitRole) -> &'static str {
    role.label()
}

/// Probe-domain PDU1 unit names a promotion gate decides on.
///
/// Each is read by `acceptance_telemetry_problems_for_run` in
/// `recorder_host/src/main.rs`, and `ProductionCapture.Validation.ps1` refuses a
/// capture whose `acceptance_telemetry.problems` list is not empty. The two
/// halves fail for OPPOSITE reasons, and saying "absence fails" of all four
/// would be wrong (Codex review 019fd708, round 2):
///
/// * `lw_prewarm_us` and `lw_hour_label_skew_abs` fail when their SAMPLE COUNT
///   does not equal `frame_end_record_count` — so dropping them fails the gate.
/// * `lw_hour_label_skew_missing` and `lw_hour_label_unusable` fail when
///   NON-ZERO. Absence defaults them to zero and passes; they are gate-deciding
///   because a real value is what fails, not because the gate demands them.
///
/// Certification input either way, and never a cut candidate.
const PDU1_GATE_DECIDING: &[&str] = &[
    "lw_prewarm_us",
    "lw_hour_label_skew_abs",
    "lw_hour_label_skew_missing",
    "lw_hour_label_unusable",
];

/// PDU1 unit names on ORDINARY domains that a production verification gate
/// names directly.
///
/// `compact_replay_verification_gates`' `DomainContract` table lists these
/// under `required_units`, `zero_units` and `work_implies_rows_units`; the gate
/// decodes the units and compares them against the manifest, so a capture
/// missing one fails. Without this set the census would call every
/// ordinary-domain unit a cut candidate, which is how the first two versions of
/// this tool were wrong (Codex review 019fd708, round 2).
///
/// DRIFT WARNING: this is a COPY of another crate's table, and a copy of a
/// table is exactly the thing that rots silently. The authority is
/// `lanes/compact_replay_verification_gates/src/lib.rs`; if that table gains a
/// unit and this list does not, the census will understate what is consumed.
/// Regenerate rather than hand-edit.
const PDU1_GATE_REQUIRED_DOMAIN_UNITS: &[&str] = &[
    "active_research_slot_rows",
    "air_ace_roster_assignment_blocked_frames",
    "air_ace_roster_observed_ace_rows",
    "air_ace_roster_partial_frames",
    "air_ace_roster_rows",
    "air_ace_roster_source_unavailable_frames",
    "air_base_status_observed_air_base_rows",
    "air_base_status_rows",
    "air_region_activity_observed_bucket_rows",
    "air_region_activity_rows",
    "air_strength_delta_rows",
    "air_strength_observed_wing_rows",
    "air_superiority_input_observed_wing_rows",
    "air_superiority_input_rows",
    "air_variant_archetype_rows",
    "air_variant_def_module_token_rows",
    "air_variant_def_rows",
    "air_variant_reserve_observed_stockpile_rows",
    "air_variant_reserve_rows",
    "air_variant_stats_observed_variant_rows",
    "air_variant_stats_rows",
    "air_variant_upgrade_observed_variant_rows",
    "air_variant_upgrade_rows",
    "air_wing_loadout_equipment_rows",
    "air_wing_loadout_rows",
    "air_wing_observed_rows",
    "air_wing_rows",
    "army_order_root_rows",
    "army_order_rows",
    "construction_queue_rows",
    "decision_country_rows",
    "diplomacy_country_rows",
    "division_equipment_root_rows",
    "division_equipment_rows",
    "division_owner_rows",
    "division_root_rows",
    "division_rows",
    "domain47_adapter_active_ruler_rows",
    "domain47_adapter_advisor_roster_rows",
    "domain47_adapter_character_rows",
    "domain47_adapter_country_leader_role_rows",
    "domain47_adapter_unit_leader_rows",
    "domain47_adapter_unit_leader_trait_rows",
    "focus_country_rows",
    "fuel_country_rows",
    "land_combat_rows",
    "map_control_province_rows",
    "map_control_state_rows",
    "map_detail_province_rows",
    "map_detail_state_rows",
    "military_line_rows",
    "naval_combat_lifecycle_rows",
    "naval_combat_member_rows",
    "naval_line_rows",
    "oob_command_root_rows",
    "oob_command_rows",
    "politics_country_rows",
    "politics_residual_country_rows",
    "popularity_rows",
    "production_country_rows",
    "project_rows",
    "ship_observed_ship_rows",
    "ship_rows",
    "stockpile_rows",
    "stockpile_variant_def_module_token_rows",
    "stockpile_variant_def_rows",
    "stockpile_variant_upgrade_entry_rows",
    "stockpile_variant_upgrade_rows",
    "supply_csupply_efficiency_rows",
    "supply_csupply_input_rows",
    "supply_division_supply_rows",
    "supply_province_logistics_rows",
    "tech_country_rows",
    "weather_observed_region_rows",
    "weather_region_rows",
];

/// Who reads a diagnostic unit, decided by the SAME rule the host uses.
///
/// Routing is by the record's domain, not by the unit's name alone, and a
/// name-list alone gets it wrong: `add_compact_lane_wall_unit` claims
/// `lw_map_topo_*` by PREFIX and `lw_o<N>_us` / `lw_d<N>_crit_us` /
/// `lw_l<N>_wall_us` / `lw_l<N>_busy_us` by PATTERN, and anything on the probe
/// domain it declines still lands in `read_ab_probe`. So every probe-domain
/// unit reaches the sealed manifest in some aggregated form, and the split
/// that matters is between the four that DECIDE a gate and the rest.
///
/// Units on ordinary domains take the other branch entirely: they fold into
/// `per_domain[d].data_units_total`, which the manifest publishes as a single
/// SCALAR per domain. Their names and per-frame values reach nothing.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Consumer {
    GateDeciding,
    HostSummarized,
    DomainScalarOnly,
}

impl Consumer {
    fn of(domain_id: u16, name: &str) -> Self {
        if domain_id != RC_PROBE_DOMAIN_ID {
            return if PDU1_GATE_REQUIRED_DOMAIN_UNITS.contains(&name) {
                Self::GateDeciding
            } else {
                Self::DomainScalarOnly
            };
        }
        if PDU1_GATE_DECIDING.contains(&name) {
            Self::GateDeciding
        } else {
            Self::HostSummarized
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::GateDeciding => "gate-deciding   (d) fails promotion if absent",
            Self::HostSummarized => "host-summarized (d) aggregated into sealed manifest",
            Self::DomainScalarOnly => "domain-scalar   (c) collapses to one int; samples die",
        }
    }
}

/// The reserved probe domain the lane-wall record rides on.
const RC_PROBE_DOMAIN_ID: u16 = 250;

fn pct(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("wire_byte_census: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .ok_or_else(|| "usage: wire_byte_census <stream.bin|stream.zst> [--names N]".to_string())?;
    let mut name_limit = 40usize;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--names" => {
                name_limit = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| "--names needs a count".to_string())?;
            }
            other => return Err(format!("unrecognised argument {other}")),
        }
    }

    let mut reader = compact_stream::open(&path).map_err(|e| format!("open {path:?}: {e}"))?;
    let encoded_len = reader.encoded_len();
    let compression = reader.compression();

    let mut census = Census::default();
    // Streaming record reader: read the fixed header, then exactly its payload.
    let mut header = [0u8; COMPACT_RECORD_HEADER_LEN];
    let mut payload = Vec::new();
    loop {
        let filled = fill(&mut reader, &mut header)?;
        if filled == 0 {
            break;
        }
        if filled < COMPACT_RECORD_HEADER_LEN {
            census.unparsed_tail += filled as u64;
            break;
        }
        let parsed = match parse_compact_record_header(&header) {
            Ok(h) => h,
            Err(error) => {
                // Count the WHOLE residual, not just this header. Stopping here
                // and asserting only against parsed bytes would let an
                // arbitrarily large unread suffix vanish from the census while
                // the partition check still passed (Codex review 019fd708,
                // finding 4).
                census.unparsed_tail += filled as u64 + drain(&mut reader)?;
                eprintln!("wire_byte_census: framing stopped: {error:?}");
                break;
            }
        };
        payload.clear();
        payload.resize(parsed.payload_len as usize, 0);
        let got = fill(&mut reader, &mut payload)?;
        if got < payload.len() {
            census.unparsed_tail += (COMPACT_RECORD_HEADER_LEN + got) as u64 + drain(&mut reader)?;
            eprintln!("wire_byte_census: truncated final record");
            break;
        }
        census.ingest(
            parsed.domain_id,
            parsed.record_type,
            parsed.record_flags,
            &payload,
        );
    }

    // Only a verbatim source has a decoded length known independently of the
    // walk; for a zstd source the walk itself is the only measure, so the
    // whole-file check is declared absent rather than faked against a number
    // derived from the same pass it is meant to check.
    let decoded_len = match compression {
        compact_stream::StreamCompression::None => Some(encoded_len),
        compact_stream::StreamCompression::Zstd => None,
    };
    report(
        &census,
        &path,
        encoded_len,
        compression,
        decoded_len,
        name_limit,
    )
}

/// Read the rest of the source, returning how many bytes were there. Used only
/// after framing stops, so the census can state the residual instead of
/// silently ending its own accounting at the stop.
fn drain(reader: &mut impl Read) -> Result<u64, String> {
    let mut scratch = vec![0u8; 1 << 20];
    let mut total = 0u64;
    loop {
        match reader.read(&mut scratch) {
            Ok(0) => return Ok(total),
            Ok(n) => total += n as u64,
            Err(e) => return Err(format!("drain: {e}")),
        }
    }
}

fn fill(reader: &mut impl Read, buf: &mut [u8]) -> Result<usize, String> {
    let mut filled = 0usize;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) => return Err(format!("read: {e}")),
        }
    }
    Ok(filled)
}

fn report(
    census: &Census,
    path: &PathBuf,
    encoded_len: u64,
    compression: compact_stream::StreamCompression,
    decoded_len: Option<u64>,
    name_limit: usize,
) -> Result<(), String> {
    let out = std::io::stdout();
    let mut w = BufWriter::new(out.lock());
    let total = census.total_bytes;

    writeln!(w, "== wire byte census ==").ok();
    writeln!(w, "file            {}", path.display()).ok();
    writeln!(w, "storage         {}", compression.as_str()).ok();
    writeln!(w, "encoded bytes   {encoded_len}").ok();
    writeln!(w, "decoded bytes   {}", total + census.unparsed_tail).ok();
    writeln!(w, "records         {}", census.records).ok();
    if census.unparsed_tail > 0 {
        writeln!(w, "unparsed tail   {} bytes", census.unparsed_tail).ok();
    }
    writeln!(w).ok();

    writeln!(
        w,
        "{:<56} {:>12} {:>14} {:>8}",
        "category", "records", "bytes", "%"
    )
    .ok();
    for (category, bucket) in &census.buckets {
        writeln!(
            w,
            "{:<56} {:>12} {:>14} {:>7.3}%",
            category.label(),
            bucket.records,
            bucket.bytes,
            pct(bucket.bytes, total)
        )
        .ok();
    }
    let classified: u64 = census.buckets.values().map(|b| b.bytes).sum();
    writeln!(
        w,
        "{:<56} {:>12} {:>14} {:>7.3}%",
        "TOTAL",
        census.records,
        classified,
        pct(classified, total)
    )
    .ok();
    // The partition must be exact against the WHOLE source, not merely against
    // the bytes that happened to parse: categories + residual == every byte the
    // decoder produced. Anything weaker lets a suffix disappear.
    if classified != total {
        return Err(format!(
            "census partition is not exact: categories sum to {classified}, records total {total}"
        ));
    }
    if let Some(expected) = decoded_len {
        if classified + census.unparsed_tail != expected {
            return Err(format!(
                "census does not cover the source: classified {classified} + residual {} != \
                 decoded length {expected}",
                census.unparsed_tail
            ));
        }
    }
    writeln!(w).ok();

    writeln!(w, "-- per record type --").ok();
    writeln!(
        w,
        "{:<10} {:<34} {:>12} {:>14} {:>8}",
        "type", "name", "records", "bytes", "%"
    )
    .ok();
    for (record_type, bucket) in &census.per_type {
        writeln!(
            w,
            "{:<10} {:<34} {:>12} {:>14} {:>7.3}%",
            record_type,
            type_name(*record_type),
            bucket.records,
            bucket.bytes,
            pct(bucket.bytes, total)
        )
        .ok();
    }
    writeln!(w).ok();

    let diag = census
        .buckets
        .get(&Category::PerfDiagTrailer)
        .copied()
        .unwrap_or_default();
    writeln!(w, "-- (c) dev perf diagnostics, inside the PDU1 trailer --").ok();
    writeln!(
        w,
        "PERF records            {} total, {} carrying a trailer",
        census.perf_total, census.perf_with_trailer
    )
    .ok();
    writeln!(
        w,
        "trailer bytes           {} ({:.3}% of stream)",
        diag.bytes,
        pct(diag.bytes, total)
    )
    .ok();
    writeln!(
        w,
        "  PDU1 framing          {} ({:.3}% of trailer)",
        census.diag_framing_bytes,
        pct(census.diag_framing_bytes, diag.bytes)
    )
    .ok();
    for (role, bucket) in &census.diag_by_role {
        writeln!(
            w,
            "  role {:<16} {:>14} units {:>12} bytes ({:.3}% of trailer)",
            role,
            bucket.records,
            bucket.bytes,
            pct(bucket.bytes, diag.bytes)
        )
        .ok();
    }
    writeln!(w).ok();
    writeln!(
        w,
        "  BY CONSUMER -- who actually reads the unit (this is the answer table):"
    )
    .ok();
    for (consumer, bucket) in &census.diag_by_consumer {
        writeln!(
            w,
            "  {:<42} {:>12} units {:>12} bytes  {:>7.3}% of trailer  {:>7.4}% of stream",
            consumer.label(),
            bucket.records,
            bucket.bytes,
            pct(bucket.bytes, diag.bytes),
            pct(bucket.bytes, total)
        )
        .ok();
    }
    if !census.trailer_errors.is_empty() {
        writeln!(w, "  trailer decode complaints:").ok();
        for (reason, count) in &census.trailer_errors {
            writeln!(w, "    {count:>10} x {reason}").ok();
        }
    }
    writeln!(w).ok();

    writeln!(w, "-- top {name_limit} diagnostic units by bytes --").ok();
    let mut names: Vec<_> = census.diag_by_name.iter().collect();
    names.sort_by(|a, b| b.1.bytes.cmp(&a.1.bytes).then(a.0.cmp(b.0)));
    writeln!(
        w,
        "{:<44} {:>14} {:>14} {:>10}",
        "unit", "occurrences", "bytes", "% stream"
    )
    .ok();
    for (name, bucket) in names.iter().take(name_limit) {
        writeln!(
            w,
            "{:<44} {:>14} {:>14} {:>9.4}%",
            name,
            bucket.records,
            bucket.bytes,
            pct(bucket.bytes, total)
        )
        .ok();
    }
    writeln!(w, "distinct unit names     {}", census.diag_by_name.len()).ok();
    w.flush().map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

fn type_name(record_type: u16) -> &'static str {
    match record_type {
        COMPACT_RECORD_TYPE_DOMAIN_PERF => "DOMAIN_PERF",
        COMPACT_RECORD_TYPE_FRAME_END => "FRAME_END",
        COMPACT_RECORD_TYPE_DOMAIN_DATA => "DOMAIN_DATA",
        COMPACT_RECORD_TYPE_STREAM_HEADER => "STREAM_HEADER",
        COMPACT_RECORD_TYPE_DOMAIN_GAP => "DOMAIN_GAP",
        COMPACT_RECORD_TYPE_FRAME_REFUSED => "FRAME_REFUSED",
        COMPACT_RECORD_TYPE_FRAME_DEGRADED => "FRAME_DEGRADED",
        COMPACT_RECORD_TYPE_CAPTURE_TERMINATED => "CAPTURE_TERMINATED",
        COMPACT_RECORD_TYPE_CLOCK_UNREADABLE_INTERVAL => "CLOCK_UNREADABLE_INTERVAL",
        COMPACT_RECORD_TYPE_UNTRUSTED_CLOCK_RECOVERY => "UNTRUSTED_CLOCK_RECOVERY",
        COMPACT_RECORD_TYPE_FREEZE_CENSUS => "FREEZE_CENSUS",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod freeze_census_tests {
    use super::*;

    /// kills: folding type 11 back into the tolerant unknown bucket loses the
    /// named telemetry category while also hiding a future raw type 12.
    #[test]
    fn freeze_census_is_named_but_unassigned_type_remains_unknown() {
        assert_eq!(
            category_of(COMPACT_RECORD_TYPE_FREEZE_CENSUS),
            Category::FreezeCensus
        );
        assert_eq!(
            type_name(COMPACT_RECORD_TYPE_FREEZE_CENSUS),
            "FREEZE_CENSUS"
        );
        assert_eq!(category_of(12), Category::Unknown);
        assert_eq!(type_name(12), "UNKNOWN");
    }
}
