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

/// Compact `records` into the ledger's `gaps` table.
///
/// Cost is O(n log n) in the number of records and O(classes + runs) in
/// retained bytes: the flat `serde_json::Value` array is never built.
pub fn encode_gaps(records: &[GapRecord]) -> Value {
    // (class key, hour) -> what happened AT THAT HOUR: for a per-domain class,
    // the affected domains, one entry per record; for a shared class, how many
    // times that exact record occurred. The per-hour tally is what makes the run
    // merge exact -- two hours join a run only when their tallies are identical.
    let mut per_hour: BTreeMap<(GapClassKey, i32), (Vec<CompactDomainId>, u32)> = BTreeMap::new();
    for record in records {
        let grouping = if record.domains.len() == 1 {
            DomainGrouping::PerDomain
        } else {
            DomainGrouping::Shared
        };
        let key = GapClassKey {
            reason: record.reason,
            grouping,
            shared_domains: match grouping {
                DomainGrouping::PerDomain => Vec::new(),
                DomainGrouping::Shared => record.domains.clone(),
            },
            span_hours: i64::from(record.hour_end) - i64::from(record.hour_start) + 1,
            witness: WitnessTemplate::from_witness(&record.witness, record.hour_start),
        };
        let slot = per_hour.entry((key, record.hour_start)).or_default();
        match grouping {
            DomainGrouping::PerDomain => slot.0.extend_from_slice(&record.domains),
            DomainGrouping::Shared => slot.1 += 1,
        }
    }

    // (class key, that hour's exact tally) -> the hours carrying it.
    let mut classes: BTreeMap<(GapClassKey, Vec<CompactDomainId>, u32), Vec<i32>> = BTreeMap::new();
    for ((key, hour), (mut domains, repeat)) in per_hour {
        domains.sort_unstable();
        classes
            .entry((key, domains, repeat))
            .or_default()
            .push(hour);
    }

    let table: Vec<Value> = classes
        .into_iter()
        .map(|((key, domains, repeat), hours)| {
            let mut class = Map::new();
            class.insert("reason".into(), json!(key.reason.wire_name()));
            class.insert("grouping".into(), json!(key.grouping.wire_name()));
            match key.grouping {
                DomainGrouping::PerDomain => {
                    class.insert("domains".into(), domains_json(&domains));
                }
                DomainGrouping::Shared => {
                    class.insert("domains".into(), domains_json(&key.shared_domains));
                    class.insert("repeat".into(), json!(repeat));
                }
            }
            class.insert("span_hours".into(), json!(key.span_hours));
            class.insert("witness".into(), key.witness.to_json());
            class.insert("hour_runs".into(), runs_json(&run_encode(hours)));
            Value::Object(class)
        })
        .collect();

    json!({
        "row_count": records.len(),
        "class_count": table.len(),
        "classes": table,
    })
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

/// Whether every lane's ordinals can be recovered from hour order alone.
///
/// Two conditions, both checked against the real rows rather than assumed:
/// every lane's `target_hour_raw` values are distinct (otherwise hour order does
/// not determine which row gets which ordinal, and the decoder would be
/// guessing), and sorting a lane by hour yields its ordinals contiguously from
/// the lane's own first. When either fails the encoder writes the ordinals out.
fn lane_ordinal_firsts(
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

/// Compact `outcomes` into the ledger's `domain_gap_outcomes` table.
///
/// Takes an ITERATOR, not a slice, and walks it twice (the `Clone` bound is the
/// second walk). The manifest's outcomes are a projection of the summary's own
/// `domain_gaps`, and collecting them into a `Vec` first was a full second copy
/// of a session-length structure -- 104 MiB at 68,300 game hours, more than the
/// entire rest of this component. The compaction's own state is
/// O(classes + runs); nothing here retains the rows.
pub fn encode_domain_gap_outcomes<I>(outcomes: I) -> Value
where
    I: IntoIterator<Item = DomainOutcome> + Clone,
{
    let firsts = lane_ordinal_firsts(outcomes.clone());

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
/// `outcomes` is an iterator so the caller need not materialize a second copy of
/// a session-length list; see [`encode_domain_gap_outcomes`].
pub fn encode_gap_ledger<I>(gaps: &[GapRecord], outcomes: I) -> Value
where
    I: IntoIterator<Item = DomainOutcome> + Clone,
{
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
        "gaps": encode_gaps(gaps),
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
pub fn ledger_manifest_bytes(ledger: &Value, records: &[GapRecord], outcome_count: usize) -> u64 {
    let json = serde_json::to_vec(ledger)
        .map(|bytes| bytes.len() as u64)
        .unwrap_or(0);
    json + gap_records_resident_bytes(records) + domain_outcomes_resident_bytes(outcome_count)
}

#[cfg(test)]
mod tests;
