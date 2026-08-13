//! The sealed manifest's recording-gap ledger, in a shape that does not grow
//! one JSON object per game hour.
//!
//! # What this replaces, and why
//!
//! `compact_stream.gaps` and `compact_stream.domain_gap_outcomes` used to be
//! flat arrays with one JSON object per row. On the real 683-game-hour capture
//! `runs/live_unlock/t44c_shadow` that is 40,139 + 39,637 rows and **26.0 MB of
//! a 36.2 MB manifest** — 33.5 KB of JSON per game hour. A full campaign is
//! ~105,000 game hours, so the flat shape is ~3.4 GB of JSON text, held as a
//! `serde_json::Value` tree several times that size, and read whole and
//! re-parsed at pack/finalize. It was the last term in finalize whose memory and
//! IO scaled with session length; everything else there is world-size dominated
//! and flat from 129 to 602 hours (lane FR, `runs/fr_recorder_mem/RESULTS.md`).
//!
//! # The shape
//!
//! Both tables become lists of **classes**. A class is the tuple that every one
//! of its member rows shares, plus the hours at which those rows occurred, run
//! encoded. On t44c that is 40,139 rows -> 174 classes / 1,122 runs and 39,637
//! rows -> 209 classes / 2,134 runs.
//!
//! The win comes from grouping domains, not from grouping hours: a scheduler
//! stalls a whole SET of domains together, so per domain the affected hours look
//! scattered (68 classes / 15,705 runs for the 22,503 coverage rows) while the
//! set itself is stable across consecutive hours (174 classes / 1,122 runs for
//! all 40,139).
//!
//! ## The exact-set rule
//!
//! A class's `domains` list is formed INDEPENDENTLY AT EACH HOUR and two hours
//! are run-merged only when their lists are identical. Nothing here is a
//! Cartesian product of a domain set with an hour range: encoding "domain 10 at
//! hour 5, domain 11 at hour 6" as `domains:[10,11] x hours:[5,6]` would invent
//! outcomes at (11,5) and (10,6), which is exactly the hour misattribution this
//! ledger exists to make impossible.
//!
//! ## Multiplicity
//!
//! Two identical rows at the same hour stay two rows. A run is extended only by
//! the STRICTLY next hour, so a repeated hour opens a new run rather than being
//! absorbed, and a repeated domain within one hour appears twice in that hour's
//! domain list. [`expand_gap_ledger`] therefore returns the same multiset it was
//! given, which is what `round_trips_*` asserts.
//!
//! # What this is NOT
//!
//! It is not a bound. The worst case — every row carrying a different witness,
//! or affecting a different domain set every hour — is one class or one run per
//! row, i.e. the flat shape plus a little framing. What the shape removes is the
//! *observed* per-hour repetition and the per-row framing (`h4gp_hex` alone was
//! 145 of the 361 bytes per row, and it is a pure function of the record, so a
//! reader re-derives it). The honest claim is "O(distinct classes + hour runs)",
//! measured at 400x on a real capture, not "O(1) in session length".
//!
//! # One format, and a version field for the next one
//!
//! [`read_compact_stream_gap_ledger`] reads `compact_stream.gap_ledger` and
//! nothing else. There is no fallback to the flat arrays: owner ruling
//! 2026-08-09 is that every capture sealed before this is disposable test data,
//! so this owes the old shape nothing, and a second decode path would be a
//! second thing to keep right.
//!
//! The `schema` / `version` pair is kept for the NEXT format, not for the last
//! one. Both must be exactly [`GAP_LEDGER_SCHEMA`] / [`GAP_LEDGER_VERSION`] or
//! the reader REFUSES: a later version may mean the same key differently, and a
//! reader that assumed otherwise would publish hours nobody recorded. A manifest
//! with no ledger at all is an error rather than an empty ledger, because "no
//! ledger" and "no gaps" are different claims and only one of them is
//! evidence.

use std::collections::BTreeMap;

use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};
use compact_contract::gap_record::{GapCounter, GapReason, GapRecord, GapWitness};
use compact_contract::CompactDomainId;
use serde_json::{json, Map, Value};

/// The ledger object's schema tag. Checked exactly on read.
pub const GAP_LEDGER_SCHEMA: &str = "hoi4-gap-ledger-v2";
/// The ledger object's version. Checked exactly on read; anything else is
/// refused rather than interpreted.
pub const GAP_LEDGER_VERSION: u64 = 2;

/// One H4DG outcome as the manifest carries it: the affected domain plus the
/// record itself.
pub type DomainOutcome = (CompactDomainId, DomainGapRecordV1);

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GapLedgerError {
    /// The manifest carries no `gap_ledger` at all.
    MissingLedger,
    /// The ledger object carried a schema/version this build does not define.
    /// Refused rather than guessed: a later version may mean the same key
    /// differently, and a reader that assumed otherwise would publish hours it
    /// invented.
    UnknownVersion { schema: String, version: String },
    /// A field was absent or had the wrong JSON type.
    Malformed(String),
    /// A field was well typed but out of the range its target type accepts --
    /// an hour run that walks past `i32::MAX`, a tick offset that does not
    /// resolve, an ordinal that overflows.
    OutOfRange(String),
    /// The expansion produced a different number of rows than the ledger
    /// declares. The declared count is checked against the expansion rather
    /// than trusted, so a truncated class list cannot read as a short session.
    RowCountMismatch { declared: u64, expanded: u64 },
    /// A record reached [`GapClassTableBuilder`] for an hour it had already
    /// closed. Refused rather than mis-filed: the builder's whole saving is that
    /// it keeps only the current hour's tallies, and folding a late record into
    /// a closed run would move a row to an hour nobody recorded it at.
    OutOfOrder { open_hour: i32, record_hour: i32 },
}

impl std::fmt::Display for GapLedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingLedger => write!(f, "compact_stream carries no `gap_ledger`"),
            Self::UnknownVersion { schema, version } => write!(
                f,
                "gap_ledger schema/version {schema:?}/{version} is not {GAP_LEDGER_SCHEMA:?}/\
                 {GAP_LEDGER_VERSION}; refused rather than guessed"
            ),
            Self::Malformed(what) => write!(f, "gap ledger is malformed: {what}"),
            Self::OutOfRange(what) => write!(f, "gap ledger value out of range: {what}"),
            Self::RowCountMismatch { declared, expanded } => write!(
                f,
                "gap ledger declares {declared} rows but expands to {expanded}"
            ),
            Self::OutOfOrder {
                open_hour,
                record_hour,
            } => write!(
                f,
                "a gap record for hour {record_hour} reached the class builder after hour \
                 {open_hour} was closed; the builder requires non-decreasing hour_start"
            ),
        }
    }
}

impl std::error::Error for GapLedgerError {}

// ---------------------------------------------------------------------------
// Anchored numbers
// ---------------------------------------------------------------------------

/// A witness field that normally sits a fixed distance from the row's hour.
///
/// Ticks are `u64` and hours are `i32`, so the difference does not fit either
/// one: `target_hour_raw = i32::MIN` with `tick_before = u64::MAX` is a delta of
/// 18,446,744,075,857,035,263, which is a valid `i128` and neither a `u64` nor
/// an `i64` — and so not a `serde_json::Number` at all. Rather than let that
/// abort a finalize, the encoder states such a field ABSOLUTELY. Exactly one of
/// the two spellings is ever written, and the decoder rejects a field carrying
/// both or neither, so the representation can never be ambiguous.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Anchored {
    Rel(i64),
    Abs(u64),
}

impl Anchored {
    fn from_tick(tick: u64, anchor_hour: i32) -> Self {
        let delta = i128::from(tick) - i128::from(anchor_hour);
        match i64::try_from(delta) {
            Ok(rel) => Self::Rel(rel),
            Err(_) => Self::Abs(tick),
        }
    }

    fn resolve(self, anchor_hour: i32, what: &str) -> Result<u64, GapLedgerError> {
        match self {
            Self::Abs(tick) => Ok(tick),
            Self::Rel(rel) => {
                let value = i128::from(anchor_hour) + i128::from(rel);
                u64::try_from(value).map_err(|_| {
                    GapLedgerError::OutOfRange(format!(
                        "{what} resolves to {value}, which is not a u64 tick"
                    ))
                })
            }
        }
    }

    fn to_json(self) -> Value {
        match self {
            Self::Rel(rel) => json!({ "rel": rel }),
            Self::Abs(abs) => json!({ "abs": abs }),
        }
    }

    fn from_json(value: &Value, what: &str) -> Result<Self, GapLedgerError> {
        let object = value
            .as_object()
            .ok_or_else(|| GapLedgerError::Malformed(format!("{what} is not an object")))?;
        if let Some(unexpected) = object.keys().find(|key| *key != "rel" && *key != "abs") {
            return Err(GapLedgerError::Malformed(format!(
                "{what} carries {unexpected:?}, which this format does not define"
            )));
        }
        match (object.get("rel"), object.get("abs")) {
            (Some(rel), None) => rel
                .as_i64()
                .map(Self::Rel)
                .ok_or_else(|| GapLedgerError::Malformed(format!("{what}.rel is not an i64"))),
            (None, Some(abs)) => abs
                .as_u64()
                .map(Self::Abs)
                .ok_or_else(|| GapLedgerError::Malformed(format!("{what}.abs is not a u64"))),
            // Both spellings at once has no single meaning and neither has any:
            // guessing one would be inventing the witness.
            _ => Err(GapLedgerError::Malformed(format!(
                "{what} must carry exactly one of `rel` and `abs`"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Witness templates
// ---------------------------------------------------------------------------

/// A [`GapWitness`] with its hour-linked fields expressed relative to the row's
/// `hour_start`, so that rows differing only by hour share one class.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum WitnessTemplate {
    Counter {
        counter: GapCounter,
        observed: u64,
    },
    HourDiscontinuity {
        previous_rel: i64,
        next_rel: i64,
    },
    StatusBits {
        status_bits: u32,
    },
    Coverage {
        expected_perf_records: u32,
        observed_perf_records: u32,
        expected_data_records: u32,
        observed_data_records: u32,
    },
    CoverageAndStatus {
        expected_perf_records: u32,
        observed_perf_records: u32,
        expected_data_records: u32,
        observed_data_records: u32,
        status_bits: u32,
    },
    TornSweep {
        tick_start: Anchored,
        tick_end: Anchored,
    },
}

impl WitnessTemplate {
    fn from_witness(witness: &GapWitness, hour_start: i32) -> Self {
        match witness {
            GapWitness::Counter { counter, observed } => Self::Counter {
                counter: *counter,
                observed: *observed,
            },
            // Both endpoints are `i32`, so the difference always fits `i64` and
            // no absolute fallback is needed here.
            GapWitness::HourDiscontinuity {
                previous_hour,
                next_hour,
            } => Self::HourDiscontinuity {
                previous_rel: i64::from(*previous_hour) - i64::from(hour_start),
                next_rel: i64::from(*next_hour) - i64::from(hour_start),
            },
            GapWitness::StatusBits { status_bits } => Self::StatusBits {
                status_bits: *status_bits,
            },
            GapWitness::Coverage {
                expected_perf_records,
                observed_perf_records,
                expected_data_records,
                observed_data_records,
            } => Self::Coverage {
                expected_perf_records: *expected_perf_records,
                observed_perf_records: *observed_perf_records,
                expected_data_records: *expected_data_records,
                observed_data_records: *observed_data_records,
            },
            GapWitness::CoverageAndStatus {
                expected_perf_records,
                observed_perf_records,
                expected_data_records,
                observed_data_records,
                status_bits,
            } => Self::CoverageAndStatus {
                expected_perf_records: *expected_perf_records,
                observed_perf_records: *observed_perf_records,
                expected_data_records: *expected_data_records,
                observed_data_records: *observed_data_records,
                status_bits: *status_bits,
            },
            GapWitness::TornSweep {
                tick_start,
                tick_end,
            } => Self::TornSweep {
                tick_start: Anchored::from_tick(*tick_start, hour_start),
                tick_end: Anchored::from_tick(*tick_end, hour_start),
            },
        }
    }

    fn to_witness(&self, hour_start: i32) -> Result<GapWitness, GapLedgerError> {
        Ok(match self {
            Self::Counter { counter, observed } => GapWitness::Counter {
                counter: *counter,
                observed: *observed,
            },
            Self::HourDiscontinuity {
                previous_rel,
                next_rel,
            } => GapWitness::HourDiscontinuity {
                previous_hour: resolve_hour(hour_start, *previous_rel, "witness.previous_hour")?,
                next_hour: resolve_hour(hour_start, *next_rel, "witness.next_hour")?,
            },
            Self::StatusBits { status_bits } => GapWitness::StatusBits {
                status_bits: *status_bits,
            },
            Self::Coverage {
                expected_perf_records,
                observed_perf_records,
                expected_data_records,
                observed_data_records,
            } => GapWitness::Coverage {
                expected_perf_records: *expected_perf_records,
                observed_perf_records: *observed_perf_records,
                expected_data_records: *expected_data_records,
                observed_data_records: *observed_data_records,
            },
            Self::CoverageAndStatus {
                expected_perf_records,
                observed_perf_records,
                expected_data_records,
                observed_data_records,
                status_bits,
            } => GapWitness::CoverageAndStatus {
                expected_perf_records: *expected_perf_records,
                observed_perf_records: *observed_perf_records,
                expected_data_records: *expected_data_records,
                observed_data_records: *observed_data_records,
                status_bits: *status_bits,
            },
            Self::TornSweep {
                tick_start,
                tick_end,
            } => GapWitness::TornSweep {
                tick_start: tick_start.resolve(hour_start, "witness.tick_start")?,
                tick_end: tick_end.resolve(hour_start, "witness.tick_end")?,
            },
        })
    }

    fn to_json(&self) -> Value {
        match self {
            Self::Counter { counter, observed } => json!({
                "kind": "counter",
                "counter": counter.wire_name(),
                "observed": observed,
            }),
            Self::HourDiscontinuity {
                previous_rel,
                next_rel,
            } => json!({
                "kind": "hour_discontinuity",
                "previous_hour_rel": previous_rel,
                "next_hour_rel": next_rel,
            }),
            Self::StatusBits { status_bits } => json!({
                "kind": "status_bits",
                "status_bits": status_bits,
            }),
            Self::Coverage {
                expected_perf_records,
                observed_perf_records,
                expected_data_records,
                observed_data_records,
            } => json!({
                "kind": "coverage",
                "expected_perf_records": expected_perf_records,
                "observed_perf_records": observed_perf_records,
                "expected_data_records": expected_data_records,
                "observed_data_records": observed_data_records,
            }),
            Self::CoverageAndStatus {
                expected_perf_records,
                observed_perf_records,
                expected_data_records,
                observed_data_records,
                status_bits,
            } => json!({
                "kind": "coverage_and_status",
                "expected_perf_records": expected_perf_records,
                "observed_perf_records": observed_perf_records,
                "expected_data_records": expected_data_records,
                "observed_data_records": observed_data_records,
                "status_bits": status_bits,
            }),
            Self::TornSweep {
                tick_start,
                tick_end,
            } => json!({
                "kind": "torn_sweep",
                "tick_start": tick_start.to_json(),
                "tick_end": tick_end.to_json(),
            }),
        }
    }

    fn from_json(value: &Value) -> Result<Self, GapLedgerError> {
        let kind = string_field(value, "witness.kind")?;
        // Closed per kind, for the same reason a class's field set is closed: an
        // unread witness field is a second spelling of the same witness.
        let allowed: &[&str] = match kind.as_str() {
            "counter" => &["kind", "counter", "observed"],
            "hour_discontinuity" => &["kind", "previous_hour_rel", "next_hour_rel"],
            "status_bits" => &["kind", "status_bits"],
            "coverage" => &[
                "kind",
                "expected_perf_records",
                "observed_perf_records",
                "expected_data_records",
                "observed_data_records",
            ],
            "coverage_and_status" => &[
                "kind",
                "expected_perf_records",
                "observed_perf_records",
                "expected_data_records",
                "observed_data_records",
                "status_bits",
            ],
            "torn_sweep" => &["kind", "tick_start", "tick_end"],
            other => {
                return Err(GapLedgerError::Malformed(format!(
                    "unknown witness kind {other:?}"
                )))
            }
        };
        check_fields(value, allowed, &format!("witness {kind}"))?;
        Ok(match kind.as_str() {
            "counter" => {
                let name = string_field(value, "witness.counter")?;
                Self::Counter {
                    counter: GapCounter::from_wire_name(&name).ok_or_else(|| {
                        GapLedgerError::Malformed(format!("unknown gap counter {name:?}"))
                    })?,
                    observed: u64_field(value, "witness.observed")?,
                }
            }
            "hour_discontinuity" => Self::HourDiscontinuity {
                previous_rel: i64_field(value, "witness.previous_hour_rel")?,
                next_rel: i64_field(value, "witness.next_hour_rel")?,
            },
            "status_bits" => Self::StatusBits {
                status_bits: u32_field(value, "witness.status_bits")?,
            },
            "coverage" => Self::Coverage {
                expected_perf_records: u32_field(value, "witness.expected_perf_records")?,
                observed_perf_records: u32_field(value, "witness.observed_perf_records")?,
                expected_data_records: u32_field(value, "witness.expected_data_records")?,
                observed_data_records: u32_field(value, "witness.observed_data_records")?,
            },
            "coverage_and_status" => Self::CoverageAndStatus {
                expected_perf_records: u32_field(value, "witness.expected_perf_records")?,
                observed_perf_records: u32_field(value, "witness.observed_perf_records")?,
                expected_data_records: u32_field(value, "witness.expected_data_records")?,
                observed_data_records: u32_field(value, "witness.observed_data_records")?,
                status_bits: u32_field(value, "witness.status_bits")?,
            },
            "torn_sweep" => Self::TornSweep {
                tick_start: Anchored::from_json(
                    value
                        .get("tick_start")
                        .ok_or_else(|| GapLedgerError::Malformed("witness.tick_start".into()))?,
                    "witness.tick_start",
                )?,
                tick_end: Anchored::from_json(
                    value
                        .get("tick_end")
                        .ok_or_else(|| GapLedgerError::Malformed("witness.tick_end".into()))?,
                    "witness.tick_end",
                )?,
            },
            other => {
                return Err(GapLedgerError::Malformed(format!(
                    "unknown witness kind {other:?}"
                )))
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Class keys
// ---------------------------------------------------------------------------

/// How a class's `domains` list is to be read back.
///
/// `PerDomain` is the shape every record this recorder derives actually has:
/// exactly one affected domain, so several rows differing only in that domain
/// can share a class and the list expands to ONE ROW PER ENTRY (duplicates
/// included). `Shared` is the general case the contract allows — a record whose
/// own `domains` field is empty (meaning every domain) or names several at once
/// — and it expands to a SINGLE row carrying that list verbatim. Keeping the two
/// apart is what makes the encoding injective: without it, one row naming
/// domains 10 and 11 and two rows naming 10 and 11 separately would encode
/// identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DomainGrouping {
    PerDomain,
    Shared,
}

impl DomainGrouping {
    const fn wire_name(self) -> &'static str {
        match self {
            Self::PerDomain => "per_domain",
            Self::Shared => "shared",
        }
    }

    fn from_wire_name(name: &str) -> Option<Self> {
        match name {
            "per_domain" => Some(Self::PerDomain),
            "shared" => Some(Self::Shared),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GapClassKey {
    reason: GapReason,
    grouping: DomainGrouping,
    /// The record's own `domains` field, verbatim, for a [`DomainGrouping::Shared`]
    /// class; empty for [`DomainGrouping::PerDomain`], where the affected domains
    /// are gathered per hour instead.
    ///
    /// IT HAS TO BE IN THE KEY. Merging two shared records' domain lists at one
    /// hour would turn "two records, one naming domain 10 and one naming 11"
    /// into "one record naming both", which is a different claim about how many
    /// separate absences the recorder witnessed.
    shared_domains: Vec<CompactDomainId>,
    /// Inclusive width, `hour_end - hour_start + 1`. Computed in `i64` because
    /// the widest legal inclusive interval, `i32::MIN..=i32::MAX`, is 4294967296
    /// and does not fit an `i32`.
    span_hours: i64,
    witness: WitnessTemplate,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct OutcomeClassKey {
    reason: DomainGapReason,
    part: DomainGapPart,
    queue_capacity: u32,
    queue_depth: u32,
    queue_dropped: u64,
    /// Offsets from `target_hour_raw`; both endpoints are `i32`, so `i64` holds
    /// every difference.
    range_start_rel: i64,
    range_end_rel: i64,
    tick_before: Anchored,
    tick_after: Anchored,
}

// ---------------------------------------------------------------------------
// Small JSON helpers
// ---------------------------------------------------------------------------

/// Look up a field by the LAST dotted segment of `path`.
///
/// `path` is the reader-facing location (`"witness.observed"`) and is what the
/// error message names; the key looked up is its last segment (`"observed"`).
/// That convention is only sound while no key in this schema contains a dot,
/// which `no_ledger_key_contains_a_dot` holds the encoder to.
fn member<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    value.get(path.rsplit('.').next().unwrap_or(path))
}

fn string_field(value: &Value, what: &str) -> Result<String, GapLedgerError> {
    member(value, what)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| GapLedgerError::Malformed(format!("{what} is not a string")))
}

fn u64_field(value: &Value, what: &str) -> Result<u64, GapLedgerError> {
    member(value, what)
        .and_then(Value::as_u64)
        .ok_or_else(|| GapLedgerError::Malformed(format!("{what} is not a u64")))
}

fn i64_field(value: &Value, what: &str) -> Result<i64, GapLedgerError> {
    member(value, what)
        .and_then(Value::as_i64)
        .ok_or_else(|| GapLedgerError::Malformed(format!("{what} is not an i64")))
}

fn u32_field(value: &Value, what: &str) -> Result<u32, GapLedgerError> {
    let raw = u64_field(value, what)?;
    u32::try_from(raw).map_err(|_| GapLedgerError::OutOfRange(format!("{what} = {raw} is not u32")))
}

/// Resolve an hour stated as an offset from `anchor`.
///
/// `i128`, not `i64`: `rel` is a DECODED value and a ledger is free to state
/// `i64::MAX`, which overflows the addition before the checked narrowing can
/// reject it -- a panic in a release build with overflow checks, and a wrapped
/// hour without them. Both are worse than an error, and inventing an hour is the
/// one thing this ledger must never do.
/// Refuse a class carrying any field outside its exact allowed set.
///
/// THE STRUCTURAL CLOSE, not another per-field rule. Every "ignored field"
/// defect this ledger has had -- `ordinals` on a `lane_sequential` class,
/// `lane_ordinal_first` on an explicit table, `repeat` on a `per_domain` gap --
/// is one instance of the same thing: a document carrying something no reader
/// uses expands to exactly what the document without it does, so the two mean
/// one thing. Worse, an unread field still counts toward a class's IDENTITY, so
/// adding one splits an aggregation key and walks straight through the
/// same-hour check.
///
/// A closed field set ends the family: nothing the encoder does not write is
/// admitted, so no future field can be smuggled in to separate two classes that
/// should have been one.
fn check_fields(value: &Value, allowed: &[&str], what: &str) -> Result<(), GapLedgerError> {
    let map = value
        .as_object()
        .ok_or_else(|| GapLedgerError::Malformed(format!("{what} is not an object")))?;
    if let Some(unexpected) = map.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(GapLedgerError::Malformed(format!(
            "{what} carries {unexpected:?}, which this format does not define"
        )));
    }
    Ok(())
}

/// A class's identity: every field except the ones naming its hours.
///
/// Serialized rather than compared field by field, because `serde_json`'s object
/// is a `BTreeMap` here and so serializes deterministically -- and because a
/// field added to the class later is then part of the identity without anyone
/// remembering to add it.
fn class_identity(class: &Value, without: &[&str]) -> String {
    match class.as_object() {
        Some(map) => {
            let trimmed: serde_json::Map<String, Value> = map
                .iter()
                .filter(|(key, _)| !without.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            serde_json::to_string(&Value::Object(trimmed)).unwrap_or_default()
        }
        None => String::new(),
    }
}

/// `class_count` is published beside the class list and must agree with it.
///
/// A count that disagrees is a second claim about the same list, and the reader
/// that ignores it lets two documents differ in a field neither reads.
fn check_class_count(table: &Value, actual: usize, what: &str) -> Result<(), GapLedgerError> {
    let declared = u64_field(table, "class_count")
        .map_err(|_| GapLedgerError::Malformed(format!("{what}.class_count is not a u64")))?;
    if declared != actual as u64 {
        return Err(GapLedgerError::Malformed(format!(
            "{what}.class_count is {declared} but the list holds {actual}"
        )));
    }
    Ok(())
}

fn resolve_hour(anchor: i32, rel: i64, what: &str) -> Result<i32, GapLedgerError> {
    let value = i128::from(anchor) + i128::from(rel);
    i32::try_from(value)
        .map_err(|_| GapLedgerError::OutOfRange(format!("{what} resolves to {value}, not an i32")))
}

fn domains_json(domains: &[CompactDomainId]) -> Value {
    Value::Array(domains.iter().map(|id| json!(id)).collect())
}

fn domains_from_json(value: &Value, what: &str) -> Result<Vec<CompactDomainId>, GapLedgerError> {
    let array = value
        .as_array()
        .ok_or_else(|| GapLedgerError::Malformed(format!("{what} is not an array")))?;
    let domains: Vec<CompactDomainId> = array
        .iter()
        .map(|entry| {
            let raw = entry
                .as_u64()
                .ok_or_else(|| GapLedgerError::Malformed(format!("{what} entry is not a u64")))?;
            CompactDomainId::try_from(raw).map_err(|_| {
                GapLedgerError::OutOfRange(format!("{what} entry {raw} is not a domain id"))
            })
        })
        .collect::<Result<_, _>>()?;
    // NON-DECREASING, as the encoder writes it. `[4,5]` and `[5,4]` expand to
    // the same rows after the reader's own sort, so admitting both would let two
    // documents mean one thing -- the same non-injectivity a split run carries.
    // Not STRICTLY increasing: a repeated domain is legal and carries
    // multiplicity.
    if domains.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(GapLedgerError::Malformed(format!(
            "{what} is not in ascending order"
        )));
    }
    Ok(domains)
}

/// Run-encode a list of hours. A run is extended ONLY by the strictly next
/// hour, so an hour that repeats opens a new run and multiplicity survives.
fn run_encode(mut hours: Vec<i32>) -> Vec<(i32, u32)> {
    hours.sort_unstable();
    let mut runs: Vec<(i32, u32)> = Vec::new();
    for hour in hours {
        if let Some((first, count)) = runs.last_mut() {
            // `i64` because `first + count` walks off `i32` at the top of the
            // hour space, where the comparison must still be answerable.
            if i64::from(*first) + i64::from(*count) == i64::from(hour) {
                *count += 1;
                continue;
            }
        }
        runs.push((hour, 1));
    }
    runs
}

fn runs_json(runs: &[(i32, u32)]) -> Value {
    Value::Array(
        runs.iter()
            .map(|(first, count)| json!([first, count]))
            .collect(),
    )
}

fn runs_from_json(value: &Value, what: &str) -> Result<Vec<(i32, u32)>, GapLedgerError> {
    let array = value
        .as_array()
        .ok_or_else(|| GapLedgerError::Malformed(format!("{what} is not an array")))?;
    // A class that names NO hour expands to nothing, so a table could carry any
    // number of them and still mean exactly what the empty table means. The
    // encoder never writes one -- a class exists because at least one row put it
    // there -- and admitting it makes the format non-injective, which is the
    // property the zero-COUNT check next to this one also protects.
    if array.is_empty() {
        return Err(GapLedgerError::Malformed(format!(
            "{what} is empty, so the class names no row"
        )));
    }
    array
        .iter()
        .map(|entry| {
            let pair = entry.as_array().filter(|pair| pair.len() == 2).ok_or_else(|| {
                GapLedgerError::Malformed(format!("{what} entry is not a [first, count] pair"))
            })?;
            let first = pair[0]
                .as_i64()
                .ok_or_else(|| GapLedgerError::Malformed(format!("{what} first hour is not i64")))?;
            let first = i32::try_from(first).map_err(|_| {
                GapLedgerError::OutOfRange(format!("{what} first hour {first} is not an i32"))
            })?;
            let count = pair[1]
                .as_u64()
                .ok_or_else(|| GapLedgerError::Malformed(format!("{what} count is not a u64")))?;
            let count = u32::try_from(count).map_err(|_| {
                GapLedgerError::OutOfRange(format!("{what} count {count} is not a u32"))
            })?;
            // A zero-length run is not "no rows here", it is a run that should
            // not have been written; admitting it would let two different class
            // lists mean the same expansion.
            if count == 0 {
                return Err(GapLedgerError::Malformed(format!("{what} count is zero")));
            }
            // The last hour the run names has to exist. Checked here so the
            // expansion loop below cannot silently wrap.
            let last = i64::from(first) + i64::from(count) - 1;
            i32::try_from(last).map_err(|_| {
                GapLedgerError::OutOfRange(format!("{what} run ends at {last}, past i32::MAX"))
            })?;
            Ok((first, count))
        })
        .collect::<Result<Vec<_>, _>>()
        .and_then(|runs| {
            // CANONICAL AND MAXIMAL. `[[101,1],[102,2]]` expands to exactly what
            // `[[101,3]]` does, so admitting both would let two documents mean
            // one thing -- the same non-injectivity an empty class carries. The
            // encoder never splits a run and never emits them out of order, so
            // requiring both costs a legal ledger nothing.
            for pair in runs.windows(2) {
                let (first, count) = pair[0];
                let end = i64::from(first) + i64::from(count);
                if i64::from(pair[1].0) < end {
                    return Err(GapLedgerError::Malformed(format!(
                        "{what} is not ascending at hour {}", pair[1].0
                    )));
                }
                if i64::from(pair[1].0) == end {
                    return Err(GapLedgerError::Malformed(format!(
                        "{what} splits a run at hour {}; runs must be maximal", pair[1].0
                    )));
                }
            }
            Ok(runs)
        })
}

// ---------------------------------------------------------------------------
// gaps: encode / expand
// ---------------------------------------------------------------------------

/// One class of the `gaps` table, in typed form.
///
/// The JSON a class becomes is a pure function of this, and the rows it stands
/// for can be walked back out of it without ever building the flat list -- which
/// is the whole reason the typed form exists rather than only the JSON one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GapClass {
    key: GapClassKey,
    /// The hour's exact domain tally for a [`DomainGrouping::PerDomain`] class,
    /// ascending, duplicates kept. Empty for a `Shared` class, whose domains are
    /// in `key.shared_domains`.
    domains: Vec<CompactDomainId>,
    /// How many times a `Shared` class's one record occurred at each of its
    /// hours. Zero for `PerDomain`, where the tally above carries multiplicity.
    repeat: u32,
    hour_runs: Vec<(i32, u32)>,
}

impl GapClass {
    /// How many rows this class stands for at ONE of its hours.
    fn rows_per_hour(&self) -> usize {
        match self.key.grouping {
            DomainGrouping::PerDomain => self.domains.len(),
            DomainGrouping::Shared => self.repeat as usize,
        }
    }

    /// Every hour this class names, ascending.
    ///
    /// `first + step` cannot leave `i32`: a run is only ever extended by a hour
    /// that was itself an `i32` (see [`GapClassTableBuilder::close_hour`]), so
    /// its last hour is a value that existed.
    fn hours(&self) -> impl Iterator<Item = i32> + '_ {
        self.hour_runs.iter().flat_map(|(first, count)| {
            (0..*count).map(move |step| (i64::from(*first) + i64::from(step)) as i32)
        })
    }

    /// The `index`-th row this class stands for at `hour`.
    fn row_at(&self, hour: i32, index: usize) -> GapRecord {
        GapRecord {
            hour_start: hour,
            // In range for the same reason `hours()` is: `span_hours` came from
            // a record whose `hour_end` was an `i32`, and every hour in the run
            // list carried that same span.
            hour_end: (i64::from(hour) + self.key.span_hours - 1) as i32,
            reason: self.key.reason,
            domains: match self.key.grouping {
                DomainGrouping::PerDomain => vec![self.domains[index]],
                DomainGrouping::Shared => self.key.shared_domains.clone(),
            },
            // INFALLIBLE HERE, and checked rather than assumed: the builder
            // refuses any record whose witness does not survive the template
            // round trip, so a template in a table is one that resolves.
            witness: self
                .key
                .witness
                .to_witness(hour)
                .expect("the builder rejects a witness that does not round trip"),
        }
    }

    /// Every row this class stands for, hour ascending.
    fn rows(&self) -> impl Iterator<Item = GapRecord> + '_ {
        self.hours()
            .flat_map(move |hour| (0..self.rows_per_hour()).map(move |index| self.row_at(hour, index)))
    }

    /// Whether this class's rows can affect `domain`, by the same rule
    /// [`GapRecord::affects_domain`] uses: an empty domain list is every domain.
    fn affects_domain(&self, domain: CompactDomainId) -> bool {
        match self.key.grouping {
            DomainGrouping::PerDomain => self.domains.binary_search(&domain).is_ok(),
            DomainGrouping::Shared => {
                self.key.shared_domains.is_empty()
                    || self.key.shared_domains.binary_search(&domain).is_ok()
            }
        }
    }

    /// The hours this class starts at whose record still covers `hour`.
    ///
    /// A class's rows all share one span, so a row starting at `s` covers `hour`
    /// exactly when `s` is in `[hour - span + 1, hour]`. The runs are ascending
    /// and disjoint, so that window is found by walking from the first run that
    /// can reach it rather than by scanning every hour the class names.
    fn starts_covering(&self, hour: i32) -> impl Iterator<Item = i32> + '_ {
        let low = i64::from(hour) - self.key.span_hours + 1;
        let high = i64::from(hour);
        // The first run whose LAST hour is at or after `low`; everything before
        // it ends too early to reach `hour`.
        let from = self
            .hour_runs
            .partition_point(|(first, count)| i64::from(*first) + i64::from(*count) - 1 < low);
        self.hour_runs[from.min(self.hour_runs.len())..]
            .iter()
            .take_while(move |(first, _)| i64::from(*first) <= high)
            .flat_map(move |(first, count)| {
                let run_low = i64::from(*first).max(low);
                let run_high = (i64::from(*first) + i64::from(*count) - 1).min(high);
                let steps = (run_high - run_low + 1).max(0) as u64;
                (0..steps).map(move |step| (run_low + step as i64) as i32)
            })
    }
}

/// The `gaps` table, kept as classes rather than as rows.
///
/// # Why this is a type and not just a `Value`
///
/// The ledger's classes are what the manifest carries, but they are also the
/// only compact form the DERIVED rows have. Every consumer of those rows used to
/// take a `&[GapRecord]` -- one struct plus one heap `Vec` per (hour, domain)
/// absence, 211 MiB at 68,300 game hours (lane FV, `runs/fv_gap_ledger`) -- and
/// the list existed only so those consumers could walk it. Holding the classes
/// instead and expanding them on demand costs O(classes + hour runs) and gives
/// every consumer the same rows.
///
/// The claim is NOT "O(1) in session length": a class whose hours alternate
/// carries one run per two hours. It is "O(distinct classes + hour runs)", which
/// on a real capture is 174 classes / 1,122 runs for 40,139 rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GapClassTable {
    classes: Vec<GapClass>,
    row_count: usize,
}

impl GapClassTable {
    /// How many rows this table stands for. Exactly what the flat array held.
    pub fn row_count(&self) -> usize {
        self.row_count
    }

    /// How many classes the table holds. Published beside the row count so a
    /// reader can see the compaction that was achieved rather than assume it.
    pub fn class_count(&self) -> usize {
        self.classes.len()
    }

    /// How many hour runs the classes hold in total: the OTHER term this shape's
    /// size is O of, and the one that grows on an alternating class.
    pub fn hour_run_count(&self) -> usize {
        self.classes.iter().map(|class| class.hour_runs.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.row_count == 0
    }

    /// Every row, class order, hour ascending within a class.
    ///
    /// The same MULTISET the table was built from; NOT the order it was built
    /// in. Every consumer of the derived rows answers an existence, a count or a
    /// per-record question, so none of them reads the order -- and the two that
    /// need hour order ask for it explicitly ([`Self::rows_hour_ordered`]).
    pub fn rows(&self) -> impl Iterator<Item = GapRecord> + '_ {
        self.classes.iter().flat_map(GapClass::rows)
    }

    /// Every row, hour ascending across the whole table.
    ///
    /// A k-way merge over the classes, so the state is one cursor per class
    /// rather than one row per row. Ties are broken by class index, which makes
    /// the order deterministic without saying anything about the rows.
    pub fn rows_hour_ordered(&self) -> HourOrderedRows<'_> {
        HourOrderedRows::new(self)
    }

    /// Whether any two rows in this table are identical.
    ///
    /// O(classes), not O(rows), and exact. Two identical rows share a class key
    /// and an hour, and the builder folds every row with that pair into ONE
    /// class entry -- so a duplicate can only show up as a repeated domain in a
    /// per-domain tally or as a `repeat` above one on a shared class. There is
    /// nowhere else for it to hide.
    pub fn has_duplicate_rows(&self) -> bool {
        self.classes.iter().any(|class| match class.key.grouping {
            DomainGrouping::PerDomain => class.domains.windows(2).any(|pair| pair[0] == pair[1]),
            DomainGrouping::Shared => class.repeat > 1,
        })
    }

    /// Whether any row covering `(hour, domain)` satisfies `predicate`.
    ///
    /// The point query the per-(hour, domain) validators ask. Answered by
    /// walking the classes and binary searching their runs, so it costs
    /// O(classes * log runs) instead of a scan of every row.
    pub fn any_covering(
        &self,
        hour: i32,
        domain: CompactDomainId,
        mut predicate: impl FnMut(&GapRecord) -> bool,
    ) -> bool {
        for class in &self.classes {
            if !class.affects_domain(domain) {
                continue;
            }
            for start in class.starts_covering(hour) {
                for index in 0..class.rows_per_hour() {
                    let record = class.row_at(start, index);
                    // The class-level filters above are an INDEX, and an index
                    // that is wrong must not change an answer: the record's own
                    // `covers` is what decides, exactly as it did when this was
                    // a scan of a flat list.
                    if record.covers(hour, domain) && predicate(&record) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Every row that intersects `[hour_start, hour_end]` and satisfies
    /// `filter`, as a list.
    ///
    /// Materializes only what the WINDOW holds, which is why it takes one: the
    /// two callers ask about a single hole between two observed hours.
    pub fn rows_intersecting(
        &self,
        hour_start: i32,
        hour_end: i32,
        mut filter: impl FnMut(&GapRecord) -> bool,
    ) -> Vec<GapRecord> {
        let mut found = Vec::new();
        for class in &self.classes {
            // A row starting at `s` intersects the window when
            // `s <= hour_end` and `s + span - 1 >= hour_start`.
            let low = i64::from(hour_start) - class.key.span_hours + 1;
            let high = i64::from(hour_end);
            let from = class
                .hour_runs
                .partition_point(|(first, count)| i64::from(*first) + i64::from(*count) - 1 < low);
            for (first, count) in &class.hour_runs[from.min(class.hour_runs.len())..] {
                if i64::from(*first) > high {
                    break;
                }
                let run_low = i64::from(*first).max(low);
                let run_high = (i64::from(*first) + i64::from(*count) - 1).min(high);
                let mut step = run_low;
                while step <= run_high {
                    let start = step as i32;
                    for index in 0..class.rows_per_hour() {
                        let record = class.row_at(start, index);
                        if record.hour_end >= hour_start
                            && record.hour_start <= hour_end
                            && filter(&record)
                        {
                            found.push(record);
                        }
                    }
                    step += 1;
                }
            }
        }
        found
    }

    /// Resident bytes the classes hold, as a FLOOR: the class spines, their
    /// domain lists and their run lists. Understating slack is the safe
    /// direction, exactly as [`gap_records_resident_bytes`] does it.
    pub fn resident_bytes(&self) -> u64 {
        let spine = (self.classes.len() as u64) * (std::mem::size_of::<GapClass>() as u64);
        let inner: u64 = self
            .classes
            .iter()
            .map(|class| {
                let domains = (class.domains.len() + class.key.shared_domains.len()) as u64
                    * (std::mem::size_of::<CompactDomainId>() as u64);
                let runs =
                    class.hour_runs.len() as u64 * (std::mem::size_of::<(i32, u32)>() as u64);
                domains + runs
            })
            .sum();
        spine + inner
    }

    /// The `gaps` table as the manifest carries it.
    pub fn to_json(&self) -> Value {
        let table: Vec<Value> = self
            .classes
            .iter()
            .map(|class| {
                let mut object = Map::new();
                object.insert("reason".into(), json!(class.key.reason.wire_name()));
                object.insert("grouping".into(), json!(class.key.grouping.wire_name()));
                match class.key.grouping {
                    DomainGrouping::PerDomain => {
                        object.insert("domains".into(), domains_json(&class.domains));
                    }
                    DomainGrouping::Shared => {
                        object.insert("domains".into(), domains_json(&class.key.shared_domains));
                        object.insert("repeat".into(), json!(class.repeat));
                    }
                }
                object.insert("span_hours".into(), json!(class.key.span_hours));
                object.insert("witness".into(), class.key.witness.to_json());
                object.insert("hour_runs".into(), runs_json(&class.hour_runs));
                Value::Object(object)
            })
            .collect();
        json!({
            "row_count": self.row_count,
            "class_count": table.len(),
            "classes": table,
        })
    }

    /// Build from a list of records in ANY order.
    ///
    /// For the reference and test paths, where the rows are already in hand.
    /// Production never has a list: it streams its derivation into
    /// [`GapClassTableBuilder`], and `the_ordered_builder_and_from_records_agree`
    /// holds the two to the same table.
    ///
    /// Takes a SLICE and sorts four-byte indices, rather than taking an iterator
    /// and collecting. A caller that already holds the rows would otherwise pay
    /// for a second full copy of them just to hand them over, which is the exact
    /// cost this whole change exists to remove -- and it would then not be
    /// visible in any component, because nobody would charge it.
    pub fn from_records(records: &[GapRecord]) -> Self {
        // `usize`, not a narrower index type: a `u32` index would silently drop
        // every row past the four-billionth, and a helper whose contract is
        // "records in any order" must not have a length above which it quietly
        // means "some of the records".
        let mut order: Vec<usize> = (0..records.len()).collect();
        // Stable, and by hour_start only: within one hour the builder sorts the
        // tally and counts the repeats, so no finer order can change the table.
        order.sort_by_key(|index| records[*index].hour_start);
        let mut builder = GapClassTableBuilder::default();
        for index in order {
            builder
                .push(&records[index])
                .expect("records visited in hour order are in order for the builder");
        }
        builder.finish()
    }

    /// The bytes [`Self::from_records`] holds beyond the table while it builds:
    /// one index per row.
    ///
    /// Published so a caller that charges the table can charge the construction
    /// transient too. Production does not use `from_records` at all.
    pub fn from_records_transient_bytes(record_count: usize) -> u64 {
        (record_count as u64) * (std::mem::size_of::<usize>() as u64)
    }
}

/// Builds a [`GapClassTable`] from records delivered in NON-DECREASING
/// `hour_start` order.
///
/// The order is a requirement, not a convenience. What the builder retains is
/// the tallies of the hour it is currently on plus the classes it has already
/// closed; a record for an hour it has closed cannot be folded in without
/// reopening a run, so it is REFUSED rather than dropped or mis-filed. The
/// caller that guarantees the order is the derivation, which merges its four
/// sources by hour.
#[derive(Debug, Default)]
pub struct GapClassTableBuilder {
    open_hour: Option<i32>,
    open: BTreeMap<GapClassKey, (Vec<CompactDomainId>, u32)>,
    closed: BTreeMap<(GapClassKey, Vec<CompactDomainId>, u32), Vec<(i32, u32)>>,
    row_count: usize,
}

impl GapClassTableBuilder {
    /// Fold one record in. Errors if `record.hour_start` is BEFORE the hour the
    /// builder is on, or if the record's witness does not survive the template
    /// round trip.
    pub fn push(&mut self, record: &GapRecord) -> Result<(), GapLedgerError> {
        match self.open_hour {
            Some(open) if record.hour_start < open => {
                return Err(GapLedgerError::OutOfOrder {
                    open_hour: open,
                    record_hour: record.hour_start,
                })
            }
            Some(open) if record.hour_start > open => self.close_hour(open),
            Some(_) => {}
            None => {}
        }
        self.open_hour = Some(record.hour_start);

        let grouping = if record.domains.len() == 1 {
            DomainGrouping::PerDomain
        } else {
            DomainGrouping::Shared
        };
        let witness = WitnessTemplate::from_witness(&record.witness, record.hour_start);
        // CHECKED HERE so `GapClass::row_at` can be infallible. The template
        // states the witness relative to the hour, and a value that cannot be
        // resolved back is a row this table could not give again -- which is
        // information loss, and it is refused where it can still be seen.
        if witness.to_witness(record.hour_start).ok().as_ref() != Some(&record.witness) {
            return Err(GapLedgerError::Malformed(format!(
                "the witness of the gap record at hour {} does not survive the class template",
                record.hour_start
            )));
        }
        let key = GapClassKey {
            reason: record.reason,
            grouping,
            shared_domains: match grouping {
                DomainGrouping::PerDomain => Vec::new(),
                DomainGrouping::Shared => record.domains.clone(),
            },
            span_hours: i64::from(record.hour_end) - i64::from(record.hour_start) + 1,
            witness,
        };
        let slot = self.open.entry(key).or_default();
        match grouping {
            DomainGrouping::PerDomain => slot.0.extend_from_slice(&record.domains),
            DomainGrouping::Shared => slot.1 += 1,
        }
        self.row_count += 1;
        Ok(())
    }

    /// Move the tallies of `hour` into the closed classes.
    fn close_hour(&mut self, hour: i32) {
        for (key, (mut domains, repeat)) in std::mem::take(&mut self.open) {
            domains.sort_unstable();
            let runs = self.closed.entry((key, domains, repeat)).or_default();
            // Hours arrive ascending, so a run is extended in place when the
            // hour is the strictly next one and a new run opens otherwise --
            // which is exactly what `run_encode` does to a sorted list.
            match runs.last_mut() {
                Some((first, count)) if i64::from(*first) + i64::from(*count) == i64::from(hour) => {
                    *count += 1;
                }
                _ => runs.push((hour, 1)),
            }
        }
    }

    pub fn finish(mut self) -> GapClassTable {
        if let Some(hour) = self.open_hour.take() {
            self.close_hour(hour);
        }
        let classes = self
            .closed
            .into_iter()
            .map(|((key, domains, repeat), hour_runs)| GapClass {
                key,
                domains,
                repeat,
                hour_runs,
            })
            .collect();
        GapClassTable {
            classes,
            row_count: self.row_count,
        }
    }
}

/// A k-way merge over a [`GapClassTable`]'s classes, yielding rows hour
/// ascending with one cursor per class rather than one row per row.
pub struct HourOrderedRows<'a> {
    table: &'a GapClassTable,
    /// Per class, the (run index, step within that run) of the hour the heap
    /// currently holds for it. A cursor rather than a search: re-finding "the
    /// next hour after this one" by scanning the runs would make the merge cost
    /// O(rows * runs), which is the shape this whole change exists to remove.
    cursors: Vec<(usize, u32)>,
    /// (hour, class index, index within the class's rows at that hour), in a
    /// `Reverse` wrapper so `BinaryHeap`'s max-heap pops hours ascending.
    heap: std::collections::BinaryHeap<std::cmp::Reverse<(i32, usize, usize)>>,
}

impl<'a> HourOrderedRows<'a> {
    fn new(table: &'a GapClassTable) -> Self {
        let mut heap = std::collections::BinaryHeap::with_capacity(table.classes.len());
        let mut cursors = vec![(0usize, 0u32); table.classes.len()];
        for (index, class) in table.classes.iter().enumerate() {
            if class.rows_per_hour() == 0 {
                // Nothing to yield, and the cursor must not point at a run whose
                // rows will never be asked for.
                cursors[index] = (class.hour_runs.len(), 0);
                continue;
            }
            if let Some((first, _)) = class.hour_runs.first() {
                heap.push(std::cmp::Reverse((*first, index, 0)));
            } else {
                cursors[index] = (0, 0);
            }
        }
        Self {
            table,
            cursors,
            heap,
        }
    }
}

impl Iterator for HourOrderedRows<'_> {
    type Item = GapRecord;

    fn next(&mut self) -> Option<GapRecord> {
        let std::cmp::Reverse((hour, class_index, row_index)) = self.heap.pop()?;
        let class = &self.table.classes[class_index];
        let record = class.row_at(hour, row_index);
        if row_index + 1 < class.rows_per_hour() {
            self.heap
                .push(std::cmp::Reverse((hour, class_index, row_index + 1)));
            return Some(record);
        }
        let (run_index, step) = &mut self.cursors[class_index];
        *step += 1;
        if class
            .hour_runs
            .get(*run_index)
            .is_some_and(|(_, count)| *step >= *count)
        {
            *run_index += 1;
            *step = 0;
        }
        if let Some((first, _)) = class.hour_runs.get(*run_index) {
            let next_hour = (i64::from(*first) + i64::from(*step)) as i32;
            self.heap
                .push(std::cmp::Reverse((next_hour, class_index, 0)));
        }
        Some(record)
    }
}

/// Compact `records` into the ledger's `gaps` table.
///
/// The reference and test entry point; production builds a [`GapClassTable`]
/// from its streaming derivation and calls [`GapClassTable::to_json`] instead.
pub fn encode_gaps(records: &[GapRecord]) -> Value {
    GapClassTable::from_records(records).to_json()
}

fn expand_gaps(table: &Value) -> Result<Vec<GapRecord>, GapLedgerError> {
    check_fields(table, &["row_count", "class_count", "classes"], "gaps")?;
    let declared = u64_field(table, "gaps.row_count")?;
    let classes = table
        .get("classes")
        .and_then(Value::as_array)
        .ok_or_else(|| GapLedgerError::Malformed("gaps.classes is not an array".into()))?;
    check_class_count(table, classes.len(), "gaps")?;
    let mut records = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    // (identity WITHOUT domains) -> the hours it has already claimed. The
    // encoder aggregates every domain affected at one hour into ONE class, so
    // two classes that differ only in their domain list may not both name the
    // same hour: `domains:[4,5] @100` and `[4] @100` + `[5] @100` expand
    // identically, and the class-identity rule alone cannot see it because the
    // domain list is part of that identity.
    let mut claimed: BTreeMap<String, std::collections::BTreeSet<i32>> = BTreeMap::new();
    for class in classes {
        // TWO CLASSES WITH THE SAME KEY WOULD HAVE BEEN ONE CLASS. Splitting a
        // run across two otherwise identical classes expands to exactly what the
        // merged class does, so the split spelling is a second document with one
        // meaning -- which the within-class maximal-run rule alone does not
        // close. The key is everything but the hours.
        let key = class_identity(class, &["hour_runs", "ordinals"]);
        if !seen.insert(key) {
            return Err(GapLedgerError::Malformed(
                "two classes share one identity; they would have been one class".into(),
            ));
        }
        check_fields(
            class,
            &[
                "reason", "grouping", "domains", "span_hours", "witness", "hour_runs",
                "repeat",
            ],
            "gap class",
        )?;
        let reason_name = string_field(class, "class.reason")?;
        let reason = GapReason::from_wire_name(&reason_name).ok_or_else(|| {
            GapLedgerError::Malformed(format!("unknown gap reason {reason_name:?}"))
        })?;
        let grouping_name = string_field(class, "class.grouping")?;
        let grouping = DomainGrouping::from_wire_name(&grouping_name).ok_or_else(|| {
            GapLedgerError::Malformed(format!("unknown domain grouping {grouping_name:?}"))
        })?;
        let domains = domains_from_json(
            class
                .get("domains")
                .ok_or_else(|| GapLedgerError::Malformed("class.domains missing".into()))?,
            "class.domains",
        )?;
        // A per-domain class with NO domain expands to nothing, so a table could
        // carry any number of them and still mean exactly what the empty table
        // means -- the same non-injectivity an empty run list would create. An
        // empty list is meaningful ONLY for a `shared` class, where it is the
        // contract's "every domain".
        if grouping == DomainGrouping::PerDomain && domains.is_empty() {
            return Err(GapLedgerError::Malformed(
                "a per_domain class names no domain, so it names no row".into(),
            ));
        }
        // A ONE-DOMAIN record is always `per_domain` on the encoder's side, so
        // `shared` with a single domain is a second spelling of the same rows.
        if grouping == DomainGrouping::Shared && domains.len() == 1 {
            return Err(GapLedgerError::Malformed(
                "a shared class names exactly one domain; that record is per_domain".into(),
            ));
        }
        let span_hours = i64_field(class, "class.span_hours")?;
        if span_hours < 1 {
            return Err(GapLedgerError::Malformed(format!(
                "class.span_hours is {span_hours}; a gap that covers no hour is not a gap"
            )));
        }
        let witness = WitnessTemplate::from_json(
            class
                .get("witness")
                .ok_or_else(|| GapLedgerError::Malformed("class.witness missing".into()))?,
        )?;
        // A shared class states how many times its one record occurred at each
        // of its hours. Required rather than defaulted: an absent `repeat` would
        // silently mean 1, which is a claim about multiplicity nobody made.
        // A class must not carry a field its own grouping IGNORES: `repeat` is
        // read only for `shared`, `ordinals` only by the outcome table, and a
        // document carrying an ignored field expands to exactly what the
        // document without it does.
        if grouping == DomainGrouping::PerDomain && class.get("repeat").is_some() {
            return Err(GapLedgerError::Malformed(
                "a per_domain class carries `repeat`, which it ignores".into(),
            ));
        }
        if class.get("ordinals").is_some() {
            return Err(GapLedgerError::Malformed(
                "a gap class carries `ordinals`, which the gap table has no use for".into(),
            ));
        }
        let repeat = match grouping {
            DomainGrouping::PerDomain => 0,
            DomainGrouping::Shared => {
                let repeat = u64_field(class, "class.repeat")?;
                if repeat == 0 {
                    return Err(GapLedgerError::Malformed(
                        "class.repeat is zero, so the class names no row".into(),
                    ));
                }
                repeat
            }
        };
        let runs = runs_from_json(
            class
                .get("hour_runs")
                .ok_or_else(|| GapLedgerError::Malformed("class.hour_runs missing".into()))?,
            "class.hour_runs",
        )?;
        let aggregation_key = class_identity(class, &["hour_runs", "ordinals", "domains"]);
        let hours_claimed = claimed.entry(aggregation_key).or_default();
        // A class cannot name more hours than the whole table has rows: the
        // count is a `u32`, so an unbounded walk is 4.29 billion iterations on a
        // ledger nobody wrote. The row count is itself checked against the
        // expansion, so this bounds the work by a number the document already
        // has to be honest about.
        // Bounded by the ROWS the class emits, not the hours it names: a shared
        // class multiplies every hour by `repeat`, so an hour bound alone still
        // admitted a `repeat` of `u64::MAX` and the loop under it.
        let run_hours: u64 = runs.iter().map(|(_, count)| u64::from(*count)).sum();
        let per_hour = match grouping {
            DomainGrouping::PerDomain => domains.len() as u64,
            DomainGrouping::Shared => repeat,
        };
        let class_rows = run_hours.saturating_mul(per_hour);
        if class_rows > declared {
            return Err(GapLedgerError::Malformed(format!(
                "a class emits {class_rows} rows, more than the {declared} the table declares"
            )));
        }
        for (first, count) in runs {
            for step in 0..count {
                let hour_start = i32::try_from(i64::from(first) + i64::from(step)).map_err(|_| {
                    GapLedgerError::OutOfRange("hour run walks past i32::MAX".into())
                })?;
                if !hours_claimed.insert(hour_start) {
                    return Err(GapLedgerError::Malformed(format!(
                        "two classes differing only in their domains both name hour {hour_start};                          the encoder aggregates a hour's domains into one class"
                    )));
                }
                // `i128` for the same reason as `resolve_hour`: `span_hours` is
                // decoded, so `i64::MAX` is a value a ledger can state and the
                // sum must be computable before it can be rejected.
                let hour_end =
                    i32::try_from(i128::from(hour_start) + i128::from(span_hours) - 1)
                        .map_err(|_| {
                            GapLedgerError::OutOfRange(format!(
                                "hour_end for hour {hour_start} spanning {span_hours} is not an i32"
                            ))
                        })?;
                let witness = witness.to_witness(hour_start)?;
                match grouping {
                    DomainGrouping::PerDomain => {
                        // Checked in `expand_gaps`, above the run loop.
                        for domain in &domains {
                            records.push(GapRecord {
                                hour_start,
                                hour_end,
                                reason,
                                domains: vec![*domain],
                                witness: witness.clone(),
                            });
                        }
                    }
                    DomainGrouping::Shared => {
                        for _ in 0..repeat {
                            records.push(GapRecord {
                                hour_start,
                                hour_end,
                                reason,
                                domains: domains.clone(),
                                witness: witness.clone(),
                            });
                        }
                    }
                }
            }
        }
    }
    let expanded = records.len() as u64;
    if expanded != declared {
        return Err(GapLedgerError::RowCountMismatch { declared, expanded });
    }
    Ok(records)
}

// ---------------------------------------------------------------------------
// domain_gap_outcomes: encode / expand
// ---------------------------------------------------------------------------

/// A lane is the scope `outcome_ordinal` counts within: the contract defines it
/// as `(domain_id, part)`.
type LaneKey = (CompactDomainId, DomainGapPart);

fn lane_of(outcome: &DomainOutcome) -> LaneKey {
    (outcome.0, outcome.1.part)
}

// ---------------------------------------------------------------------------
// The outcome source (task #179-1)
// ---------------------------------------------------------------------------

/// A RANDOM-ACCESS source of H4DG outcomes.
///
/// # Why not an iterator any more
///
/// Lane GE made this an `IntoIterator + Clone` so the caller need not
/// materialize a second copy of a session-length list -- 104 MiB at 68,300 game
/// hours -- and that property is preserved here. What an iterator cannot do is
/// be VISITED IN A DIFFERENT ORDER, and hour order is what lets the encoder stop
/// holding per-outcome state: `summary.domain_gaps` is ordered by the frame that
/// published each observation, not by the hour it describes, so an H4DG range
/// may begin before the frame that reported it.
///
/// A source that can be indexed can be walked through a SORTED INDEX instead,
/// which is one row index -- one `usize` -- per row rather than the three
/// O(rows) maps the old encoder built. Implementors project rather than copy:
/// the production one reads `summary.domain_gaps[i]` in place.
pub trait DomainOutcomeSource {
    fn outcome_count(&self) -> usize;
    fn outcome_at(&self, index: usize) -> DomainOutcome;

    /// Just the target hour of one row, for the sort that builds the visiting
    /// order.
    ///
    /// THE DEFAULT IS THE DEFINITION, and an override is only ever an
    /// optimisation of it. `hour_at` decides which rows are grouped together
    /// and which hour a class records; `outcome_at` supplies the ordinal that
    /// goes under it. An implementor whose two readers DISAGREED would emit
    /// ordinals under the wrong hours and nothing downstream could tell -- so
    /// this may be overridden only where the override reads the SAME FIELD OF
    /// THE SAME ROW, which is checkable by inspection at the override, not by
    /// trusting a caller (Codex round 7).
    fn hour_at(&self, index: usize) -> i32 {
        self.outcome_at(index).1.target_hour_raw
    }
}

/// The slice itself, unsized, so `&outcomes[..]` is a source without a wrapper.
impl DomainOutcomeSource for [DomainOutcome] {
    fn outcome_count(&self) -> usize {
        self.len()
    }
    fn outcome_at(&self, index: usize) -> DomainOutcome {
        self[index]
    }
    /// The same field of the same row the default would have read, without
    /// copying the record around it. Inspectable in one line, which is the only
    /// basis on which this override is allowed.
    fn hour_at(&self, index: usize) -> i32 {
        self[index].1.target_hour_raw
    }
}

/// A source backed by a projection closure, for a caller whose rows are not
/// `DomainOutcome` values but can produce one on demand.
///
/// This is how production passes `summary.domain_gaps` without copying it.
pub struct ProjectedOutcomes<F> {
    len: usize,
    project: F,
}

impl<F: Fn(usize) -> DomainOutcome> ProjectedOutcomes<F> {
    /// ONE closure, deliberately.
    ///
    /// A draft took a second one that read only the target hour, so the sort
    /// would not copy a 64-byte record per comparison. Nothing could make the
    /// two agree: a caller whose hour closure returned a different row's hour
    /// would have had its ordinals emitted under the wrong hours, silently and
    /// with no downstream check able to see it (Codex round 7). The exact input
    /// was two same-lane rows at hours 10 and 20 with the hour closure returning
    /// 20 and 10.
    ///
    /// So the hour comes from the record. `sort_unstable_by_key` is the
    /// NON-CACHING key sort and obtains a key for both compared rows, so the
    /// cost is by-value projections for both -- O(rows log rows) key
    /// projections, once per finalize. (Projections rather than copies is the
    /// defensible unit: the compiler may elide the copy inside one, and
    /// counting comparisons rather than projections understated it by half --
    /// Codex round 8.) That is the price of removing a silent-corruption class
    /// no test downstream of it can catch. If a future lane measures the sort as
    /// hot, the fix is a source whose rows expose their hour without building a
    /// record -- not two closures a caller can desync.
    pub fn new(len: usize, project: F) -> Self {
        Self { len, project }
    }
}

impl<F: Fn(usize) -> DomainOutcome> DomainOutcomeSource for ProjectedOutcomes<F> {
    fn outcome_count(&self) -> usize {
        self.len
    }
    fn outcome_at(&self, index: usize) -> DomainOutcome {
        (self.project)(index)
    }
    // `hour_at` is deliberately NOT overridden: see `new`.
}

/// The hour-ordered visiting order, and the ONLY per-row structure this encoder
/// builds.
///
/// Row indices, sorted by target hour: one `usize` a row, eight bytes on a
/// 64-bit target against the forty or so the old encoder's three maps held, and
/// it dies before the JSON is built.
///
/// NO SIZE BOUND. A first draft packed `(i32, u32)` to keep the hour beside the
/// index, which imposed a `u32::MAX`-row ceiling the iterator API never had and
/// enforced it with a panic -- a production abort as the failure mode for an
/// input-size limitation, on an input a virtual source could present without
/// holding 275 GB (Codex round 6). Indices alone carry no ceiling, and the sort
/// key comes from [`DomainOutcomeSource::hour_at`], which is the record's own
/// `target_hour_raw` unless an implementor reads that same field more cheaply.
fn hour_ordered_index<S: DomainOutcomeSource + ?Sized>(outcomes: &S) -> Vec<usize> {
    let mut order: Vec<usize> = (0..outcomes.outcome_count()).collect();
    order.sort_unstable_by_key(|index| outcomes.hour_at(*index));
    order
}

/// Whether every lane's ordinals can be recovered from hour order alone.
///
/// Two conditions, both checked against the real rows rather than assumed:
/// every lane's `target_hour_raw` values are distinct (otherwise hour order does
/// not determine which row gets which ordinal, and the decoder would be
/// guessing), and sorting a lane by hour yields its ordinals contiguously from
/// the lane's own first. When either fails the encoder writes the ordinals out.
///
/// STREAMED over `order`, so the state is O(lanes) rather than one
/// `(hour, ordinal)` pair per row. Walking in hour order is what makes that
/// possible: each lane's hours arrive non-decreasing, so "distinct" is a
/// comparison against the lane's previous hour and "contiguous from first" is a
/// comparison against `first + count`. The old walk collected every lane's rows
/// and sorted them, which is the same test with the whole list held.
fn lane_ordinal_firsts<S: DomainOutcomeSource + ?Sized>(
    outcomes: &S,
    order: &[usize],
) -> Option<BTreeMap<LaneKey, u64>> {
    struct LaneState {
        first: u64,
        count: u64,
        last_hour: i32,
    }
    let mut lanes: BTreeMap<LaneKey, LaneState> = BTreeMap::new();
    for index in order {
        let outcome = outcomes.outcome_at(*index);
        let hour = &outcome.1.target_hour_raw;
        let ordinal = outcome.1.outcome_ordinal;
        match lanes.entry(lane_of(&outcome)) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(LaneState {
                    first: ordinal,
                    count: 1,
                    last_hour: *hour,
                });
            }
            std::collections::btree_map::Entry::Occupied(mut slot) => {
                let state = slot.get_mut();
                // Two rows of one lane at the same hour: hour order cannot say
                // which gets which ordinal.
                if state.last_hour == *hour {
                    return None;
                }
                if ordinal != state.first.checked_add(state.count)? {
                    return None;
                }
                state.count += 1;
                state.last_hour = *hour;
            }
        }
    }
    Some(
        lanes
            .into_iter()
            .map(|(lane, state)| (lane, state.first))
            .collect(),
    )
}

/// What one class has accumulated so far: run-encoded hours, and the ordinals
/// only when they have to be written out.
struct ClassAccumulator {
    /// `run_encode`'s output, built as the hours arrive rather than from a held
    /// list of them. Hours reach a class strictly ascending, so this is the same
    /// arithmetic `run_encode` does over a sorted vector.
    runs: Vec<(i32, u32)>,
    /// Only populated under `explicit` encoding, where the ordinals ARE the
    /// output and O(rows) is therefore irreducible. Empty under
    /// `lane_sequential`, which is what the old encoder held and then discarded.
    ordinals: Vec<u64>,
    row_count: usize,
}

impl ClassAccumulator {
    fn push_hour(&mut self, hour: i32, rows: usize) {
        if let Some((first, count)) = self.runs.last_mut() {
            if i64::from(*first) + i64::from(*count) == i64::from(hour) {
                *count += 1;
                self.row_count += rows;
                return;
            }
        }
        self.runs.push((hour, 1));
        self.row_count += rows;
    }
}

/// Compact `outcomes` into the ledger's `domain_gap_outcomes` table.
///
/// # What this used to hold, and why it stopped (task #179-1)
///
/// Lane GE removed the derived gap ROWS and named this encoder as the largest
/// remaining term in the finalize gap phase (`runs/ge_gap_stream/RESULTS.md`
/// residual 2). It built three structures, each O(outcome rows), and none of
/// them survived into the output under the encoding a real capture uses:
///
///   * `lane_ordinal_firsts` collected every lane's `(hour, ordinal)` pairs and
///     sorted them, to answer a question about ordering;
///   * a `per_hour` map held one `(domain, ordinal)` per row, to group them;
///   * the class map held one `u64` ordinal per row, which `lane_sequential`
///     encoding then DISCARDED.
///
/// The output is O(classes + hour runs). Everything above was transient state,
/// and the only reason it existed is that the input arrives in publication
/// order rather than in hour order.
///
/// So the input is visited in hour order, through a sorted index that is one
/// row index -- one `usize` -- per row (see [`hour_ordered_index`]). With the
/// rows arriving
/// hour-ascending, the lane check needs O(lanes), the per-hour grouping needs
/// only the hour it is on, and a class keeps its run-encoder state instead of
/// its rows. The ordinals survive only under `explicit` encoding, where they are
/// the output and holding them is not a choice.
///
/// The emitted JSON is byte-identical to what the previous encoder emitted;
/// `reference_encode_domain_gap_outcomes` is that encoder, kept verbatim, and
/// the tests hold the two to each other.
pub fn encode_domain_gap_outcomes<S: DomainOutcomeSource + ?Sized>(outcomes: &S) -> Value {
    let order = hour_ordered_index(outcomes);
    let firsts = lane_ordinal_firsts(outcomes, &order);
    let explicit = firsts.is_none();

    let mut classes: BTreeMap<(OutcomeClassKey, Vec<CompactDomainId>), ClassAccumulator> =
        BTreeMap::new();
    // RECONSTRUCTED each hour, with its live state bounded to one hour. The
    // binding is reused; its STORAGE is not -- `clear()` drops every map node
    // and every row `Vec` with its capacity, and the next hour reallocates them
    // (Codex round 6). What matters for this change is the bound, not the
    // allocation traffic: a pool would cut the traffic and is not what #179-1
    // was about.
    let mut hour_rows: BTreeMap<OutcomeClassKey, Vec<(CompactDomainId, u64)>> = BTreeMap::new();

    let mut at = 0usize;
    while at < order.len() {
        let hour = outcomes.hour_at(order[at]);
        hour_rows.clear();
        while at < order.len() && outcomes.hour_at(order[at]) == hour {
            let (domain_id, record) = outcomes.outcome_at(order[at]);
            let key = OutcomeClassKey {
                reason: record.reason,
                part: record.part,
                queue_capacity: record.queue_capacity,
                queue_depth: record.queue_depth,
                queue_dropped: record.queue_dropped,
                range_start_rel: i64::from(record.range_start_hour_raw)
                    - i64::from(record.target_hour_raw),
                range_end_rel: i64::from(record.range_end_hour_raw)
                    - i64::from(record.target_hour_raw),
                tick_before: Anchored::from_tick(record.tick_before, record.target_hour_raw),
                tick_after: Anchored::from_tick(record.tick_after, record.target_hour_raw),
            };
            hour_rows
                .entry(key)
                .or_default()
                .push((domain_id, record.outcome_ordinal));
            at += 1;
        }
        // Grouped exactly as `gaps` is: the domain list is formed per hour and
        // two hours join a run only when their lists are identical.
        for (key, rows) in hour_rows.iter_mut() {
            rows.sort_unstable();
            let domains: Vec<CompactDomainId> = rows.iter().map(|(domain, _)| *domain).collect();
            let accumulator = classes
                .entry((key.clone(), domains))
                .or_insert_with(|| ClassAccumulator {
                    runs: Vec::new(),
                    ordinals: Vec::new(),
                    row_count: 0,
                });
            accumulator.push_hour(hour, rows.len());
            if explicit {
                // Aligned with this class's expansion order: hours ascending,
                // and within an hour the class's `domains` order -- which is the
                // order `rows` is now in.
                accumulator
                    .ordinals
                    .extend(rows.iter().map(|(_, ordinal)| *ordinal));
            }
        }
    }

    let row_count: usize = classes.values().map(|class| class.row_count).sum();
    let mut table = Vec::with_capacity(classes.len());
    for ((key, domains), accumulator) in classes {
        let mut class = Map::new();
        class.insert("reason".into(), json!(key.reason.wire_name()));
        class.insert("part".into(), json!(key.part.wire_name()));
        class.insert("domains".into(), domains_json(&domains));
        class.insert("queue_capacity".into(), json!(key.queue_capacity));
        class.insert("queue_depth".into(), json!(key.queue_depth));
        class.insert("queue_dropped".into(), json!(key.queue_dropped));
        class.insert("range_start_rel".into(), json!(key.range_start_rel));
        class.insert("range_end_rel".into(), json!(key.range_end_rel));
        class.insert("tick_before".into(), key.tick_before.to_json());
        class.insert("tick_after".into(), key.tick_after.to_json());
        class.insert("hour_runs".into(), runs_json(&accumulator.runs));
        if explicit {
            class.insert("ordinals".into(), json!(accumulator.ordinals));
        }
        table.push(Value::Object(class));
    }

    let mut ledger = Map::new();
    ledger.insert("row_count".into(), json!(row_count));
    ledger.insert("class_count".into(), json!(table.len()));
    match &firsts {
        Some(firsts) => {
            ledger.insert("ordinal_encoding".into(), json!("lane_sequential"));
            ledger.insert(
                "lane_ordinal_first".into(),
                Value::Array(
                    firsts
                        .iter()
                        .map(|((domain_id, part), first)| {
                            json!({
                                "domain_id": domain_id,
                                "part": part.wire_name(),
                                "first": first,
                            })
                        })
                        .collect(),
                ),
            );
        }
        None => {
            ledger.insert("ordinal_encoding".into(), json!("explicit"));
        }
    }
    ledger.insert("classes".into(), Value::Array(table));
    Value::Object(ledger)
}


/// Whether every lane's ordinals can be recovered from hour order alone.
///
/// The PRE-CHANGE walk, kept verbatim (task #179-1).
///
/// Two conditions, both checked against the real rows rather than assumed:
/// every lane's `target_hour_raw` values are distinct (otherwise hour order does
/// not determine which row gets which ordinal, and the decoder would be
/// guessing), and sorting a lane by hour yields its ordinals contiguously from
/// the lane's own first. When either fails the encoder writes the ordinals out.
fn reference_lane_ordinal_firsts(
    outcomes: impl IntoIterator<Item = DomainOutcome>,
) -> Option<BTreeMap<LaneKey, u64>> {
    let mut lanes: BTreeMap<LaneKey, Vec<(i32, u64)>> = BTreeMap::new();
    for outcome in outcomes {
        lanes
            .entry(lane_of(&outcome))
            .or_default()
            .push((outcome.1.target_hour_raw, outcome.1.outcome_ordinal));
    }
    let mut firsts = BTreeMap::new();
    for (lane, mut rows) in lanes {
        rows.sort_unstable();
        if rows.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return None;
        }
        let first = rows[0].1;
        for (index, (_, ordinal)) in rows.iter().enumerate() {
            let expected = first.checked_add(index as u64)?;
            if *ordinal != expected {
                return None;
            }
        }
        firsts.insert(lane, first);
    }
    Some(firsts)
}

/// The PRE-CHANGE encoder, kept VERBATIM as the reference the streaming one
/// is held byte-identical to (task #179-1).
///
/// Not reachable from production: nothing outside this module's tests and the
/// profile arm calls it. It exists so "the new encoder emits what the old one
/// emitted" is a checked property rather than a claim.
///
/// Compact `outcomes` into the ledger's `domain_gap_outcomes` table.
///
/// Takes an ITERATOR, not a slice, and walks it twice (the `Clone` bound is the
/// second walk). The manifest's outcomes are a projection of the summary's own
/// `domain_gaps`, and collecting them into a `Vec` first was a full second copy
/// of a session-length structure -- 104 MiB at 68,300 game hours, more than the
/// entire rest of this component. The compaction's own state is
/// O(classes + runs); nothing here retains the rows.
pub fn reference_encode_domain_gap_outcomes<I>(outcomes: I) -> Value
where
    I: IntoIterator<Item = DomainOutcome> + Clone,
{
    let firsts = reference_lane_ordinal_firsts(outcomes.clone());

    let mut per_hour: BTreeMap<(OutcomeClassKey, i32), Vec<(CompactDomainId, u64)>> =
        BTreeMap::new();
    for (domain_id, record) in outcomes {
        let key = OutcomeClassKey {
            reason: record.reason,
            part: record.part,
            queue_capacity: record.queue_capacity,
            queue_depth: record.queue_depth,
            queue_dropped: record.queue_dropped,
            range_start_rel: i64::from(record.range_start_hour_raw)
                - i64::from(record.target_hour_raw),
            range_end_rel: i64::from(record.range_end_hour_raw) - i64::from(record.target_hour_raw),
            tick_before: Anchored::from_tick(record.tick_before, record.target_hour_raw),
            tick_after: Anchored::from_tick(record.tick_after, record.target_hour_raw),
        };
        per_hour
            .entry((key, record.target_hour_raw))
            .or_default()
            .push((domain_id, record.outcome_ordinal));
    }

    // Grouped exactly as `gaps` is: the domain list is formed per hour and two
    // hours join a run only when their lists are identical.
    let mut classes: BTreeMap<(OutcomeClassKey, Vec<CompactDomainId>), Vec<(i32, Vec<u64>)>> =
        BTreeMap::new();
    for ((key, hour), mut rows) in per_hour {
        rows.sort_unstable();
        let domains = rows.iter().map(|(domain, _)| *domain).collect::<Vec<_>>();
        let ordinals = rows.iter().map(|(_, ordinal)| *ordinal).collect::<Vec<_>>();
        classes
            .entry((key, domains))
            .or_default()
            .push((hour, ordinals));
    }

    // Counted from the classes rather than from the input, because the input is
    // an iterator this function has already consumed.
    let row_count: usize = classes
        .values()
        .flat_map(|hours| hours.iter().map(|(_, ordinals)| ordinals.len()))
        .sum();
    let mut table = Vec::with_capacity(classes.len());
    for ((key, domains), mut hours) in classes {
        hours.sort_by_key(|(hour, _)| *hour);
        let runs = run_encode(hours.iter().map(|(hour, _)| *hour).collect());
        let mut class = Map::new();
        class.insert("reason".into(), json!(key.reason.wire_name()));
        class.insert("part".into(), json!(key.part.wire_name()));
        class.insert("domains".into(), domains_json(&domains));
        class.insert("queue_capacity".into(), json!(key.queue_capacity));
        class.insert("queue_depth".into(), json!(key.queue_depth));
        class.insert("queue_dropped".into(), json!(key.queue_dropped));
        class.insert("range_start_rel".into(), json!(key.range_start_rel));
        class.insert("range_end_rel".into(), json!(key.range_end_rel));
        class.insert("tick_before".into(), key.tick_before.to_json());
        class.insert("tick_after".into(), key.tick_after.to_json());
        class.insert("hour_runs".into(), runs_json(&runs));
        if firsts.is_none() {
            // Aligned with this class's expansion order: hours ascending, and
            // within an hour the class's `domains` order.
            let ordinals: Vec<u64> = hours
                .iter()
                .flat_map(|(_, ordinals)| ordinals.iter().copied())
                .collect();
            class.insert("ordinals".into(), json!(ordinals));
        }
        table.push(Value::Object(class));
    }

    let mut ledger = Map::new();
    ledger.insert("row_count".into(), json!(row_count));
    ledger.insert("class_count".into(), json!(table.len()));
    match &firsts {
        Some(firsts) => {
            ledger.insert("ordinal_encoding".into(), json!("lane_sequential"));
            ledger.insert(
                "lane_ordinal_first".into(),
                Value::Array(
                    firsts
                        .iter()
                        .map(|((domain_id, part), first)| {
                            json!({
                                "domain_id": domain_id,
                                "part": part.wire_name(),
                                "first": first,
                            })
                        })
                        .collect(),
                ),
            );
        }
        None => {
            ledger.insert("ordinal_encoding".into(), json!("explicit"));
        }
    }
    ledger.insert("classes".into(), Value::Array(table));
    Value::Object(ledger)
}

fn expand_domain_gap_outcomes(table: &Value) -> Result<Vec<DomainOutcome>, GapLedgerError> {
    // THE TABLE IS CLOSED TOO. Closing the classes but not their container left
    // the same escape one level up: a field on the table is a spelling the
    // encoder cannot emit, and the container is exactly where a future field
    // would be added.
    check_fields(
        table,
        &[
            "row_count",
            "class_count",
            "classes",
            "ordinal_encoding",
            "lane_ordinal_first",
        ],
        "domain_gap_outcomes",
    )?;
    let declared = u64_field(table, "domain_gap_outcomes.row_count")?;
    let encoding = string_field(table, "domain_gap_outcomes.ordinal_encoding")?;
    let explicit = match encoding.as_str() {
        "lane_sequential" => false,
        "explicit" => true,
        other => {
            return Err(GapLedgerError::Malformed(format!(
                "unknown ordinal_encoding {other:?}"
            )))
        }
    };
    // THE TWO BRANCHES MUST NOT CARRY EACH OTHER'S FIELDS. A `lane_sequential`
    // table ignores a class's `ordinals`, and an `explicit` one ignores
    // `lane_ordinal_first` -- so a document carrying the ignored field means
    // exactly what the document without it means. Two documents, one expansion,
    // which is the non-injectivity the empty-class and duplicate-lane checks
    // also close.
    if explicit && table.get("lane_ordinal_first").is_some() {
        return Err(GapLedgerError::Malformed(
            "an explicit ordinal table also carries lane_ordinal_first, which it ignores".into(),
        ));
    }
    let mut lane_first: BTreeMap<LaneKey, u64> = BTreeMap::new();
    if !explicit {
        let entries = table
            .get("lane_ordinal_first")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                GapLedgerError::Malformed("lane_ordinal_first is not an array".into())
            })?;
        for entry in entries {
            check_fields(entry, &["domain_id", "part", "first"], "lane_ordinal_first entry")?;
            let domain_raw = u64_field(entry, "lane_ordinal_first.domain_id")?;
            let domain_id = CompactDomainId::try_from(domain_raw).map_err(|_| {
                GapLedgerError::OutOfRange(format!("lane domain id {domain_raw} is not a u16"))
            })?;
            let part_name = string_field(entry, "lane_ordinal_first.part")?;
            let part = DomainGapPart::from_wire_name(&part_name).ok_or_else(|| {
                GapLedgerError::Malformed(format!("unknown gap part {part_name:?}"))
            })?;
            // A DUPLICATE lane entry silently overwrote its predecessor, so two
            // different tables expanded identically -- the same non-injectivity
            // an empty class carries.
            if lane_first
                .insert((domain_id, part), u64_field(entry, "lane_ordinal_first.first")?)
                .is_some()
            {
                return Err(GapLedgerError::Malformed(format!(
                    "lane_ordinal_first names domain {domain_id} part {part:?} twice"
                )));
            }
        }
    }

    let classes = table
        .get("classes")
        .and_then(Value::as_array)
        .ok_or_else(|| GapLedgerError::Malformed("domain_gap_outcomes.classes".into()))?;
    check_class_count(table, classes.len(), "domain_gap_outcomes")?;

    // (hour, domain, class index) -> the row, with the ordinal still to assign
    // when the encoding is `lane_sequential`.
    let mut rows: Vec<(i32, CompactDomainId, usize, DomainGapRecordV1)> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    // See `expand_gaps`: the encoder aggregates a hour's domains into one class.
    let mut claimed: BTreeMap<String, std::collections::BTreeSet<i32>> = BTreeMap::new();
    for (class_index, class) in classes.iter().enumerate() {
        // See `expand_gaps`: two classes with one identity would have been one.
        let key = class_identity(class, &["hour_runs", "ordinals"]);
        if !seen.insert(key) {
            return Err(GapLedgerError::Malformed(
                "two classes share one identity; they would have been one class".into(),
            ));
        }
        check_fields(
            class,
            &[
                "reason", "part", "domains", "queue_capacity", "queue_depth",
                "queue_dropped", "range_start_rel", "range_end_rel", "tick_before",
                "tick_after", "hour_runs", "ordinals",
            ],
            "outcome class",
        )?;
        let reason_name = string_field(class, "class.reason")?;
        let reason = DomainGapReason::from_wire_name(&reason_name).ok_or_else(|| {
            GapLedgerError::Malformed(format!("unknown domain gap reason {reason_name:?}"))
        })?;
        let part_name = string_field(class, "class.part")?;
        let part = DomainGapPart::from_wire_name(&part_name)
            .ok_or_else(|| GapLedgerError::Malformed(format!("unknown gap part {part_name:?}")))?;
        let domains = domains_from_json(
            class
                .get("domains")
                .ok_or_else(|| GapLedgerError::Malformed("class.domains missing".into()))?,
            "class.domains",
        )?;
        // An outcome class ALWAYS names its domains -- there is no "every
        // domain" spelling on this side -- so an empty list expands to nothing
        // and makes the format non-injective.
        if domains.is_empty() {
            return Err(GapLedgerError::Malformed(
                "an outcome class names no domain, so it names no row".into(),
            ));
        }
        let queue_capacity = u32_field(class, "class.queue_capacity")?;
        let queue_depth = u32_field(class, "class.queue_depth")?;
        let queue_dropped = u64_field(class, "class.queue_dropped")?;
        let range_start_rel = i64_field(class, "class.range_start_rel")?;
        let range_end_rel = i64_field(class, "class.range_end_rel")?;
        let tick_before = Anchored::from_json(
            class
                .get("tick_before")
                .ok_or_else(|| GapLedgerError::Malformed("class.tick_before missing".into()))?,
            "class.tick_before",
        )?;
        let tick_after = Anchored::from_json(
            class
                .get("tick_after")
                .ok_or_else(|| GapLedgerError::Malformed("class.tick_after missing".into()))?,
            "class.tick_after",
        )?;
        let runs = runs_from_json(
            class
                .get("hour_runs")
                .ok_or_else(|| GapLedgerError::Malformed("class.hour_runs missing".into()))?,
            "class.hour_runs",
        )?;
        if !explicit && class.get("ordinals").is_some() {
            return Err(GapLedgerError::Malformed(
                "a lane_sequential class also carries `ordinals`, which it ignores".into(),
            ));
        }
        let explicit_ordinals: Option<Vec<u64>> = if explicit {
            let raw = class
                .get("ordinals")
                .and_then(Value::as_array)
                .ok_or_else(|| GapLedgerError::Malformed("class.ordinals missing".into()))?;
            Some(
                raw.iter()
                    .map(|entry| {
                        entry.as_u64().ok_or_else(|| {
                            GapLedgerError::Malformed("class.ordinals entry is not a u64".into())
                        })
                    })
                    .collect::<Result<_, _>>()?,
            )
        } else {
            None
        };

        let aggregation_key = class_identity(class, &["hour_runs", "ordinals", "domains"]);
        let hours_claimed = claimed.entry(aggregation_key).or_default();
        let run_hours: u64 = runs.iter().map(|(_, count)| u64::from(*count)).sum();
        let class_rows = run_hours.saturating_mul(domains.len() as u64);
        if class_rows > declared {
            return Err(GapLedgerError::Malformed(format!(
                "a class emits {class_rows} rows, more than the {declared} the table declares"
            )));
        }
        let mut position = 0usize;
        for (first, count) in runs {
            for step in 0..count {
                let target_hour_raw = i32::try_from(i64::from(first) + i64::from(step))
                    .map_err(|_| GapLedgerError::OutOfRange("hour run past i32::MAX".into()))?;
                if !hours_claimed.insert(target_hour_raw) {
                    return Err(GapLedgerError::Malformed(format!(
                        "two classes differing only in their domains both name hour                          {target_hour_raw}; the encoder aggregates a hour's domains into one class"
                    )));
                }
                for domain_id in &domains {
                    let outcome_ordinal = match &explicit_ordinals {
                        Some(ordinals) => *ordinals.get(position).ok_or_else(|| {
                            GapLedgerError::Malformed(
                                "class.ordinals is shorter than the class expansion".into(),
                            )
                        })?,
                        // Filled in below, once every class has contributed its
                        // rows and the lane can be walked in hour order.
                        None => 0,
                    };
                    rows.push((
                        target_hour_raw,
                        *domain_id,
                        class_index,
                        DomainGapRecordV1 {
                            part,
                            reason,
                            target_hour_raw,
                            range_start_hour_raw: resolve_hour(
                                target_hour_raw,
                                range_start_rel,
                                "range_start_hour_raw",
                            )?,
                            range_end_hour_raw: resolve_hour(
                                target_hour_raw,
                                range_end_rel,
                                "range_end_hour_raw",
                            )?,
                            tick_before: tick_before.resolve(target_hour_raw, "tick_before")?,
                            tick_after: tick_after.resolve(target_hour_raw, "tick_after")?,
                            queue_capacity,
                            queue_depth,
                            queue_dropped,
                            outcome_ordinal,
                        },
                    ));
                    position += 1;
                }
            }
        }
        if let Some(ordinals) = &explicit_ordinals {
            if ordinals.len() != position {
                return Err(GapLedgerError::Malformed(format!(
                    "class.ordinals has {} entries for {position} expanded rows",
                    ordinals.len()
                )));
            }
        }
    }

    if !explicit {
        // One pass per lane, in hour order, exactly mirroring the check the
        // encoder ran before it chose this encoding.
        let mut used_lanes: std::collections::BTreeSet<LaneKey> = Default::default();
        let mut by_lane: BTreeMap<LaneKey, Vec<usize>> = BTreeMap::new();
        for (index, (_, domain_id, _, record)) in rows.iter().enumerate() {
            by_lane
                .entry((*domain_id, record.part))
                .or_default()
                .push(index);
        }
        for (lane, mut indices) in by_lane {
            indices.sort_by_key(|index| rows[*index].0);
            if indices
                .windows(2)
                .any(|pair| rows[pair[0]].0 == rows[pair[1]].0)
            {
                // The encoder refuses to choose `lane_sequential` in this case,
                // so a ledger that says it did is one this reader must not
                // interpret: which row holds which ordinal is unknowable.
                return Err(GapLedgerError::Malformed(format!(
                    "lane {lane:?} has two outcomes at one hour under lane_sequential ordinals"
                )));
            }
            let first = *lane_first.get(&lane).ok_or_else(|| {
                GapLedgerError::Malformed(format!("lane {lane:?} has no lane_ordinal_first"))
            })?;
            for (offset, index) in indices.into_iter().enumerate() {
                rows[index].3.outcome_ordinal =
                    first.checked_add(offset as u64).ok_or_else(|| {
                        GapLedgerError::OutOfRange("outcome ordinal overflows u64".into())
                    })?;
            }
            used_lanes.insert(lane);
        }
        // An UNUSED lane entry changes nothing about the expansion, so a table
        // carrying one means exactly what the table without it means. Refused
        // for the same reason a class that names no row is.
        if let Some(unused) = lane_first.keys().find(|lane| !used_lanes.contains(lane)) {
            return Err(GapLedgerError::Malformed(format!(
                "lane_ordinal_first names lane {unused:?}, which no class expands to"
            )));
        }
    }

    let expanded = rows.len() as u64;
    if expanded != declared {
        return Err(GapLedgerError::RowCountMismatch { declared, expanded });
    }
    // Stable, hour-major, domain-minor: the order the flat array carried, and
    // the order every consumer that walks the rows expects.
    rows.sort_by_key(|(hour, domain_id, class_index, _)| (*hour, *domain_id, *class_index));
    Ok(rows
        .into_iter()
        .map(|(_, domain_id, _, record)| (domain_id, record))
        .collect())
}

// ---------------------------------------------------------------------------
// The ledger object
// ---------------------------------------------------------------------------

/// Build the whole `compact_stream.gap_ledger` object.
///
/// `outcomes` is a random-access SOURCE, not a list: the caller need not
/// materialize a second copy of a session-length list, and the encoder needs to
/// visit it in hour order. See [`DomainOutcomeSource`] and
/// [`encode_domain_gap_outcomes`].
pub fn encode_gap_ledger<S: DomainOutcomeSource + ?Sized>(
    gaps: &[GapRecord],
    outcomes: &S,
) -> Value {
    encode_gap_ledger_from_table(&GapClassTable::from_records(gaps), outcomes)
}

/// Build the whole `compact_stream.gap_ledger` object from an already-compacted
/// [`GapClassTable`].
///
/// The production entry point. The flat row list never exists on this path: the
/// derivation streams into the table and the table becomes the JSON.
pub fn encode_gap_ledger_from_table<S: DomainOutcomeSource + ?Sized>(
    gaps: &GapClassTable,
    outcomes: &S,
) -> Value {
    json!({
        "schema": GAP_LEDGER_SCHEMA,
        "version": GAP_LEDGER_VERSION,
        // NO SLASHES IN THIS STRING. The portable-manifest projection rejects
        // anything that looks like a path before a container is sealed, and a
        // prose reference such as "scripts/gap_ledger.py" trips it -- which is
        // the gate working, since a note is not worth failing a seal over.
        "note": "run-encoded gap ledger: one class per distinct (reason, domain set, \
                 witness) tuple, with the hours it occurred at. Replaces the flat \
                 `gaps` and `domain_gap_outcomes` arrays, which cost 33.5 KB of \
                 manifest JSON per game hour. Expand it with the recorder_host \
                 gap_ledger module or the gap_ledger.py helper.",
        "gaps": gaps.to_json(),
        "domain_gap_outcomes": encode_domain_gap_outcomes(outcomes),
    })
}

/// Read the gap ledger out of a sealed manifest's `compact_stream` object,
/// whether it was sealed before or after this change.
pub fn read_compact_stream_gap_ledger(
    compact_stream: &Value,
) -> Result<(Vec<GapRecord>, Vec<DomainOutcome>), GapLedgerError> {
    if let Some(ledger) = compact_stream.get("gap_ledger") {
        let schema = ledger.get("schema").and_then(Value::as_str).unwrap_or("");
        let version = ledger.get("version").and_then(Value::as_u64);
        // BOTH, exactly. A ledger tagged with this version but another schema is
        // a different document that happens to share a number.
        if schema != GAP_LEDGER_SCHEMA || version != Some(GAP_LEDGER_VERSION) {
            return Err(GapLedgerError::UnknownVersion {
                schema: schema.to_string(),
                version: version.map(|v| v.to_string()).unwrap_or_else(|| "none".into()),
            });
        }
        // THE LEDGER ITSELF IS THE LAST CONTAINER, and it was the last one
        // open. Closing the classes and then their tables still left this escape
        // one level farther out -- the same defect three times, each a level up
        // from the last. `compact_stream` above it is not this format's to close.
        check_fields(
            ledger,
            &["schema", "version", "note", "gaps", "domain_gap_outcomes"],
            "gap_ledger",
        )?;
        // AND `note` IS TYPED, not merely named. Allowlisting the name left an
        // OBJECT-SHAPED HOLE: `note: {"novel_nested_field": 1}` passed every
        // reader, because a name allowlist says nothing about the value under it.
        //
        // A STRING is what this format defines `note` to be, and that is what is
        // required. The exact prose is deliberately NOT pinned: the note is
        // documentation, read by nobody, and part of no class identity, so a
        // differing string cannot change a row or merge two classes -- while an
        // object could later carry structure. Pinning the text would make prose
        // load-bearing across three readers and a vector for no information gain.
        if !ledger
            .get("note")
            .map(|note| note.is_string())
            .unwrap_or(true)
        {
            return Err(GapLedgerError::Malformed(
                "gap_ledger.note is not a string".into(),
            ));
        }
        let gaps = expand_gaps(
            ledger
                .get("gaps")
                .ok_or_else(|| GapLedgerError::Malformed("gap_ledger.gaps missing".into()))?,
        )?;
        let outcomes = expand_domain_gap_outcomes(ledger.get("domain_gap_outcomes").ok_or_else(
            || GapLedgerError::Malformed("gap_ledger.domain_gap_outcomes missing".into()),
        )?)?;
        return Ok((gaps, outcomes));
    }
    // ONE format. There is deliberately no fallback to the flat arrays this
    // replaced (owner ruling 2026-08-09: every capture sealed before this is
    // disposable test data). A manifest without a `gap_ledger` is not a capture
    // with no gaps -- it is a document this reader has no ledger for, and saying
    // so is the difference between reporting nothing and reporting zero.
    Err(GapLedgerError::MissingLedger)
}

// ---------------------------------------------------------------------------
// The flat shape, as a reference only
// ---------------------------------------------------------------------------

/// The pre-ledger `compact_stream.gaps` array.
///
/// NOTHING READS THIS BACK and no manifest is written with it. It exists as the
/// reference the size measurement compares against (`fv_gap_ledger_profile`'s
/// `flat` arm is the "before" number) and as the expected rows in the
/// cross-language conformance vector. Keeping the one function that renders it
/// is what makes those two comparisons a measurement of this change rather than
/// of a remembered number.
pub fn reference_flat_gaps_json(records: &[GapRecord]) -> Value {
    Value::Array(
        records
            .iter()
            .map(|record| {
                let witness = match &record.witness {
                    GapWitness::Counter { counter, observed } => json!({
                        "kind": "counter",
                        "counter": counter.wire_name(),
                        "observed": observed,
                    }),
                    GapWitness::HourDiscontinuity {
                        previous_hour,
                        next_hour,
                    } => json!({
                        "kind": "hour_discontinuity",
                        "previous_hour": previous_hour,
                        "next_hour": next_hour,
                    }),
                    GapWitness::StatusBits { status_bits } => json!({
                        "kind": "status_bits",
                        "status_bits": status_bits,
                    }),
                    GapWitness::Coverage {
                        expected_perf_records,
                        observed_perf_records,
                        expected_data_records,
                        observed_data_records,
                    } => json!({
                        "kind": "coverage",
                        "expected_perf_records": expected_perf_records,
                        "observed_perf_records": observed_perf_records,
                        "expected_data_records": expected_data_records,
                        "observed_data_records": observed_data_records,
                    }),
                    GapWitness::CoverageAndStatus {
                        expected_perf_records,
                        observed_perf_records,
                        expected_data_records,
                        observed_data_records,
                        status_bits,
                    } => json!({
                        "kind": "coverage_and_status",
                        "expected_perf_records": expected_perf_records,
                        "observed_perf_records": observed_perf_records,
                        "expected_data_records": expected_data_records,
                        "observed_data_records": observed_data_records,
                        "status_bits": status_bits,
                    }),
                    GapWitness::TornSweep {
                        tick_start,
                        tick_end,
                    } => json!({
                        "kind": "torn_sweep",
                        "tick_start": tick_start,
                        "tick_end": tick_end,
                    }),
                };
                json!({
                    "hour_start": record.hour_start,
                    "hour_end": record.hour_end,
                    "reason": record.reason.wire_name(),
                    "domains": &record.domains,
                    "witness": witness,
                    "h4gp_hex": compact_contract::encode_gap_record(record)
                        .ok()
                        .map(|encoded| {
                            encoded.iter().fold(String::new(), |mut out, byte| {
                                use std::fmt::Write as _;
                                let _ = write!(out, "{byte:02x}");
                                out
                            })
                        }),
                })
            })
            .collect(),
    )
}

/// The pre-ledger `compact_stream.domain_gap_outcomes` array. Reference only,
/// exactly as [`reference_flat_gaps_json`].
pub fn reference_flat_domain_gap_outcomes_json(outcomes: &[DomainOutcome]) -> Value {
    Value::Array(
        outcomes
            .iter()
            .map(|(domain_id, record)| {
                json!({
                    "domain_id": domain_id,
                    "part": record.part.wire_name(),
                    "reason": record.reason.wire_name(),
                    "target_hour_raw": record.target_hour_raw,
                    "range_start_hour_raw": record.range_start_hour_raw,
                    "range_end_hour_raw": record.range_end_hour_raw,
                    "tick_before": record.tick_before,
                    "tick_after": record.tick_after,
                    "queue_capacity": record.queue_capacity,
                    "queue_depth": record.queue_depth,
                    "queue_dropped": record.queue_dropped,
                    "outcome_ordinal": record.outcome_ordinal,
                })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Memory accounting
// ---------------------------------------------------------------------------

/// Resident bytes the derived gap records hold.
///
/// Charged where the capacity is decided rather than per allocation, per the
/// [`crate::mem_account`] honesty rule. `domains` is a heap `Vec` per record, so
/// its capacity is counted too: at one `u16` per record it is small per row and
/// it is one allocation per row, which is the shape that matters at 105,000
/// game hours.
/// The bytes the derived gap rows occupy, as a FLOOR.
///
/// A slice cannot report the backing `Vec`'s capacity, so this counts `len`.
/// Understating slack is the safe direction here — the number is used to say how
/// large the ledger's inputs are, and an over-count would flatter nothing.
/// `domains` is a heap `Vec` per record, so its own bytes are counted too: one
/// `u16` per row is small, but it is one allocation per row, which is the shape
/// that matters at 105,000 game hours.
pub fn gap_records_resident_bytes(records: &[GapRecord]) -> u64 {
    let spine = (records.len() as u64) * (std::mem::size_of::<GapRecord>() as u64);
    let domains: u64 = records
        .iter()
        .map(|record| {
            (record.domains.len() as u64) * (std::mem::size_of::<CompactDomainId>() as u64)
        })
        .sum();
    spine + domains
}

/// Resident bytes `count` outcome rows occupy. `DomainGapRecordV1` is `Copy`
/// with no interior allocation, so this is exactly the spine.
///
/// Takes a COUNT rather than a slice because the encoder no longer wants a
/// slice: the outcomes are streamed straight out of the summary and never
/// collected. This prices what the summary already holds, so the component still
/// names the rows the ledger was built from.
pub fn domain_outcomes_resident_bytes(count: usize) -> u64 {
    (count as u64) * (std::mem::size_of::<DomainOutcome>() as u64)
}

/// What the ledger costs the manifest: the rows it was built from plus the exact
/// serialized length of the JSON it becomes.
///
/// The JSON half is measured by serializing rather than by walking the
/// `serde_json::Value`, because a `Value` walk can only ESTIMATE its map nodes
/// and allocator slack, and an estimate published beside exact counters is the
/// kind of number [`crate::mem_account`] exists to refuse. The serialized length
/// is exactly what the manifest carries and exactly what pack time reads back.
/// Affordable only because the ledger is run encoded — the flat shape priced
/// this way would have cost a second full copy of itself.
/// The CLASS TABLE IS NOT PRICED HERE. The derived rows are no longer
/// materialized on the production path (lane GE) and the classes that replaced
/// them are charged where they are built, for as long as they live -- charging
/// them again here would double count them into the same component, which is an
/// over-charge and exactly what `accounting_consistent` exists to catch.
///
/// What is left is the JSON and the H4DG outcomes the ledger was built from.
pub fn ledger_manifest_bytes(ledger: &Value, outcome_count: usize) -> u64 {
    let json = serde_json::to_vec(ledger)
        .map(|bytes| bytes.len() as u64)
        .unwrap_or(0);
    json + domain_outcomes_resident_bytes(outcome_count)
}

/// What a MATERIALIZED expansion of the same ledger would have cost.
///
/// Kept, and public, because it is the "before" arm of the measurement lane GE
/// is stated in: the profiler builds the flat list, charges this, and the
/// difference against [`ledger_manifest_bytes`] over the class table is the
/// change. Production never calls it.
pub fn flat_ledger_manifest_bytes(
    ledger: &Value,
    records: &[GapRecord],
    outcome_count: usize,
) -> u64 {
    let json = serde_json::to_vec(ledger)
        .map(|bytes| bytes.len() as u64)
        .unwrap_or(0);
    json + gap_records_resident_bytes(records) + domain_outcomes_resident_bytes(outcome_count)
}

#[cfg(test)]
mod tests;
