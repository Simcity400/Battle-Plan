use std::collections::BTreeMap;

use compact_contract::domain_gap::{DomainGapPart, DomainGapReason, DomainGapRecordV1};
use compact_contract::gap_record::{GapCounter, GapReason, GapRecord, GapWitness};
use compact_contract::CompactDomainId;
use serde_json::{json, Map, Value};

pub const GAP_LEDGER_SCHEMA: &str = "hoi4-gap-ledger-v2";
pub const GAP_LEDGER_VERSION: u64 = 2;

pub type DomainOutcome = (CompactDomainId, DomainGapRecordV1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GapLedgerError {
    MissingLedger,
    UnknownVersion { schema: String, version: String },
    Malformed(String),
    OutOfRange(String),
    RowCountMismatch { declared: u64, expanded: u64 },
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
            _ => Err(GapLedgerError::Malformed(format!(
                "{what} must carry exactly one of `rel` and `abs`"
            ))),
        }
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DomainGrouping {
    PerDomain,
    Shared,
}

/// The two directions of one name, from one declaration.
///
/// They were two matches, so renaming a grouping in the writer alone compiled
/// and serialised a per-domain grouping as shared (Codex round 8). The name
/// must spell the variant, and the reader is the writer read backwards.
macro_rules! domain_grouping_wire_names {
    ($( $variant:ident => $name:literal, )+) => {
        impl DomainGrouping {
            const fn wire_name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)+
                }
            }

            fn from_wire_name(name: &str) -> Option<Self> {
                match name {
                    $($name => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }

        const _: () = {
            $(assert!(
                compact_contract::diagnostic_channel::const_str_is_snake_of(
                    $name,
                    stringify!($variant),
                ),
                "a grouping's wire name does not spell the grouping",
            );)+
            assert!(
                compact_contract::diagnostic_channel::const_names_are_distinct(&[$($name,)+]),
                "two groupings share a wire name",
            );
        };
    };
}

domain_grouping_wire_names! {
    PerDomain => "per_domain",
    Shared => "shared",
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GapClassKey {
    reason: GapReason,
    grouping: DomainGrouping,
    shared_domains: Vec<CompactDomainId>,
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
    range_start_rel: i64,
    range_end_rel: i64,
    tick_before: Anchored,
    tick_after: Anchored,
}

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

/// reject it -- a panic in a release build with overflow checks, and a wrapped
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
    if domains.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(GapLedgerError::Malformed(format!(
            "{what} is not in ascending order"
        )));
    }
    Ok(domains)
}

fn run_encode(mut hours: Vec<i32>) -> Vec<(i32, u32)> {
    hours.sort_unstable();
    let mut runs: Vec<(i32, u32)> = Vec::new();
    for hour in hours {
        if let Some((first, count)) = runs.last_mut() {
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
    if array.is_empty() {
        return Err(GapLedgerError::Malformed(format!(
            "{what} is empty, so the class names no row"
        )));
    }
    array
        .iter()
        .map(|entry| {
            let pair = entry
                .as_array()
                .filter(|pair| pair.len() == 2)
                .ok_or_else(|| {
                    GapLedgerError::Malformed(format!("{what} entry is not a [first, count] pair"))
                })?;
            let first = pair[0].as_i64().ok_or_else(|| {
                GapLedgerError::Malformed(format!("{what} first hour is not i64"))
            })?;
            let first = i32::try_from(first).map_err(|_| {
                GapLedgerError::OutOfRange(format!("{what} first hour {first} is not an i32"))
            })?;
            let count = pair[1]
                .as_u64()
                .ok_or_else(|| GapLedgerError::Malformed(format!("{what} count is not a u64")))?;
            let count = u32::try_from(count).map_err(|_| {
                GapLedgerError::OutOfRange(format!("{what} count {count} is not a u32"))
            })?;
            if count == 0 {
                return Err(GapLedgerError::Malformed(format!("{what} count is zero")));
            }
            let last = i64::from(first) + i64::from(count) - 1;
            i32::try_from(last).map_err(|_| {
                GapLedgerError::OutOfRange(format!("{what} run ends at {last}, past i32::MAX"))
            })?;
            Ok((first, count))
        })
        .collect::<Result<Vec<_>, _>>()
        .and_then(|runs| {
            for pair in runs.windows(2) {
                let (first, count) = pair[0];
                let end = i64::from(first) + i64::from(count);
                if i64::from(pair[1].0) < end {
                    return Err(GapLedgerError::Malformed(format!(
                        "{what} is not ascending at hour {}",
                        pair[1].0
                    )));
                }
                if i64::from(pair[1].0) == end {
                    return Err(GapLedgerError::Malformed(format!(
                        "{what} splits a run at hour {}; runs must be maximal",
                        pair[1].0
                    )));
                }
            }
            Ok(runs)
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GapClass {
    key: GapClassKey,
    domains: Vec<CompactDomainId>,
    repeat: u32,
    hour_runs: Vec<(i32, u32)>,
}

impl GapClass {
    fn rows_per_hour(&self) -> usize {
        match self.key.grouping {
            DomainGrouping::PerDomain => self.domains.len(),
            DomainGrouping::Shared => self.repeat as usize,
        }
    }

    fn hours(&self) -> impl Iterator<Item = i32> + '_ {
        self.hour_runs.iter().flat_map(|(first, count)| {
            (0..*count).map(move |step| (i64::from(*first) + i64::from(step)) as i32)
        })
    }

    fn row_at(&self, hour: i32, index: usize) -> GapRecord {
        GapRecord {
            hour_start: hour,
            hour_end: (i64::from(hour) + self.key.span_hours - 1) as i32,
            reason: self.key.reason,
            domains: match self.key.grouping {
                DomainGrouping::PerDomain => vec![self.domains[index]],
                DomainGrouping::Shared => self.key.shared_domains.clone(),
            },
            witness: self
                .key
                .witness
                .to_witness(hour)
                .expect("the builder rejects a witness that does not round trip"),
        }
    }

    fn rows(&self) -> impl Iterator<Item = GapRecord> + '_ {
        self.hours().flat_map(move |hour| {
            (0..self.rows_per_hour()).map(move |index| self.row_at(hour, index))
        })
    }

    fn affects_domain(&self, domain: CompactDomainId) -> bool {
        match self.key.grouping {
            DomainGrouping::PerDomain => self.domains.binary_search(&domain).is_ok(),
            DomainGrouping::Shared => {
                self.key.shared_domains.is_empty()
                    || self.key.shared_domains.binary_search(&domain).is_ok()
            }
        }
    }

    fn starts_covering(&self, hour: i32) -> impl Iterator<Item = i32> + '_ {
        let low = i64::from(hour) - self.key.span_hours + 1;
        let high = i64::from(hour);
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GapClassTable {
    classes: Vec<GapClass>,
    row_count: usize,
}

impl GapClassTable {
    pub fn row_count(&self) -> usize {
        self.row_count
    }

    pub fn class_count(&self) -> usize {
        self.classes.len()
    }

    pub fn hour_run_count(&self) -> usize {
        self.classes.iter().map(|class| class.hour_runs.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.row_count == 0
    }

    pub fn rows(&self) -> impl Iterator<Item = GapRecord> + '_ {
        self.classes.iter().flat_map(GapClass::rows)
    }

    pub fn rows_hour_ordered(&self) -> HourOrderedRows<'_> {
        HourOrderedRows::new(self)
    }

    pub fn has_duplicate_rows(&self) -> bool {
        self.classes.iter().any(|class| match class.key.grouping {
            DomainGrouping::PerDomain => class.domains.windows(2).any(|pair| pair[0] == pair[1]),
            DomainGrouping::Shared => class.repeat > 1,
        })
    }

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
                    if record.covers(hour, domain) && predicate(&record) {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn rows_intersecting(
        &self,
        hour_start: i32,
        hour_end: i32,
        mut filter: impl FnMut(&GapRecord) -> bool,
    ) -> Vec<GapRecord> {
        let mut found = Vec::new();
        for class in &self.classes {
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

    pub fn from_records(records: &[GapRecord]) -> Self {
        let mut order: Vec<usize> = (0..records.len()).collect();
        order.sort_by_key(|index| records[*index].hour_start);
        let mut builder = GapClassTableBuilder::default();
        for index in order {
            builder
                .push(&records[index])
                .expect("records visited in hour order are in order for the builder");
        }
        builder.finish()
    }

    pub fn from_records_transient_bytes(record_count: usize) -> u64 {
        (record_count as u64) * (std::mem::size_of::<usize>() as u64)
    }
}

#[derive(Debug, Default)]
pub struct GapClassTableBuilder {
    open_hour: Option<i32>,
    open: BTreeMap<GapClassKey, (Vec<CompactDomainId>, u32)>,
    closed: BTreeMap<(GapClassKey, Vec<CompactDomainId>, u32), Vec<(i32, u32)>>,
    row_count: usize,
}

impl GapClassTableBuilder {
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

    fn close_hour(&mut self, hour: i32) {
        for (key, (mut domains, repeat)) in std::mem::take(&mut self.open) {
            domains.sort_unstable();
            let runs = self.closed.entry((key, domains, repeat)).or_default();
            match runs.last_mut() {
                Some((first, count))
                    if i64::from(*first) + i64::from(*count) == i64::from(hour) =>
                {
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

pub struct HourOrderedRows<'a> {
    table: &'a GapClassTable,
    cursors: Vec<(usize, u32)>,
    heap: std::collections::BinaryHeap<std::cmp::Reverse<(i32, usize, usize)>>,
}

impl<'a> HourOrderedRows<'a> {
    fn new(table: &'a GapClassTable) -> Self {
        let mut heap = std::collections::BinaryHeap::with_capacity(table.classes.len());
        let mut cursors = vec![(0usize, 0u32); table.classes.len()];
        for (index, class) in table.classes.iter().enumerate() {
            if class.rows_per_hour() == 0 {
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
    let mut claimed: BTreeMap<String, std::collections::BTreeSet<i32>> = BTreeMap::new();
    for class in classes {
        let key = class_identity(class, &["hour_runs", "ordinals"]);
        if !seen.insert(key) {
            return Err(GapLedgerError::Malformed(
                "two classes share one identity; they would have been one class".into(),
            ));
        }
        check_fields(
            class,
            &[
                "reason",
                "grouping",
                "domains",
                "span_hours",
                "witness",
                "hour_runs",
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
        if grouping == DomainGrouping::PerDomain && domains.is_empty() {
            return Err(GapLedgerError::Malformed(
                "a per_domain class names no domain, so it names no row".into(),
            ));
        }
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
                let hour_start =
                    i32::try_from(i64::from(first) + i64::from(step)).map_err(|_| {
                        GapLedgerError::OutOfRange("hour run walks past i32::MAX".into())
                    })?;
                if !hours_claimed.insert(hour_start) {
                    return Err(GapLedgerError::Malformed(format!(
                        "two classes differing only in their domains both name hour {hour_start};                          the encoder aggregates a hour's domains into one class"
                    )));
                }
                let hour_end = i32::try_from(i128::from(hour_start) + i128::from(span_hours) - 1)
                    .map_err(|_| {
                    GapLedgerError::OutOfRange(format!(
                        "hour_end for hour {hour_start} spanning {span_hours} is not an i32"
                    ))
                })?;
                let witness = witness.to_witness(hour_start)?;
                match grouping {
                    DomainGrouping::PerDomain => {
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

type LaneKey = (CompactDomainId, DomainGapPart);

fn lane_of(outcome: &DomainOutcome) -> LaneKey {
    (outcome.0, outcome.1.part)
}

pub trait DomainOutcomeSource {
    fn outcome_count(&self) -> usize;
    fn outcome_at(&self, index: usize) -> DomainOutcome;

    fn hour_at(&self, index: usize) -> i32 {
        self.outcome_at(index).1.target_hour_raw
    }
}

impl DomainOutcomeSource for [DomainOutcome] {
    fn outcome_count(&self) -> usize {
        self.len()
    }
    fn outcome_at(&self, index: usize) -> DomainOutcome {
        self[index]
    }
    fn hour_at(&self, index: usize) -> i32 {
        self[index].1.target_hour_raw
    }
}

pub struct ProjectedOutcomes<F> {
    len: usize,
    project: F,
}

impl<F: Fn(usize) -> DomainOutcome> ProjectedOutcomes<F> {
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
}

fn hour_ordered_index<S: DomainOutcomeSource + ?Sized>(outcomes: &S) -> Vec<usize> {
    let mut order: Vec<usize> = (0..outcomes.outcome_count()).collect();
    order.sort_unstable_by_key(|index| outcomes.hour_at(*index));
    order
}

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

struct ClassAccumulator {
    runs: Vec<(i32, u32)>,
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

pub fn encode_domain_gap_outcomes<S: DomainOutcomeSource + ?Sized>(outcomes: &S) -> Value {
    let order = hour_ordered_index(outcomes);
    let firsts = lane_ordinal_firsts(outcomes, &order);
    let explicit = firsts.is_none();

    let mut classes: BTreeMap<(OutcomeClassKey, Vec<CompactDomainId>), ClassAccumulator> =
        BTreeMap::new();
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
        for (key, rows) in hour_rows.iter_mut() {
            rows.sort_unstable();
            let domains: Vec<CompactDomainId> = rows.iter().map(|(domain, _)| *domain).collect();
            let accumulator =
                classes
                    .entry((key.clone(), domains))
                    .or_insert_with(|| ClassAccumulator {
                        runs: Vec::new(),
                        ordinals: Vec::new(),
                        row_count: 0,
                    });
            accumulator.push_hour(hour, rows.len());
            if explicit {
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
            check_fields(
                entry,
                &["domain_id", "part", "first"],
                "lane_ordinal_first entry",
            )?;
            let domain_raw = u64_field(entry, "lane_ordinal_first.domain_id")?;
            let domain_id = CompactDomainId::try_from(domain_raw).map_err(|_| {
                GapLedgerError::OutOfRange(format!("lane domain id {domain_raw} is not a u16"))
            })?;
            let part_name = string_field(entry, "lane_ordinal_first.part")?;
            let part = DomainGapPart::from_wire_name(&part_name).ok_or_else(|| {
                GapLedgerError::Malformed(format!("unknown gap part {part_name:?}"))
            })?;
            if lane_first
                .insert(
                    (domain_id, part),
                    u64_field(entry, "lane_ordinal_first.first")?,
                )
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

    let mut rows: Vec<(i32, CompactDomainId, usize, DomainGapRecordV1)> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    let mut claimed: BTreeMap<String, std::collections::BTreeSet<i32>> = BTreeMap::new();
    for (class_index, class) in classes.iter().enumerate() {
        let key = class_identity(class, &["hour_runs", "ordinals"]);
        if !seen.insert(key) {
            return Err(GapLedgerError::Malformed(
                "two classes share one identity; they would have been one class".into(),
            ));
        }
        check_fields(
            class,
            &[
                "reason",
                "part",
                "domains",
                "queue_capacity",
                "queue_depth",
                "queue_dropped",
                "range_start_rel",
                "range_end_rel",
                "tick_before",
                "tick_after",
                "hour_runs",
                "ordinals",
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
    rows.sort_by_key(|(hour, domain_id, class_index, _)| (*hour, *domain_id, *class_index));
    Ok(rows
        .into_iter()
        .map(|(_, domain_id, _, record)| (domain_id, record))
        .collect())
}

pub fn encode_gap_ledger<S: DomainOutcomeSource + ?Sized>(
    gaps: &[GapRecord],
    outcomes: &S,
) -> Value {
    encode_gap_ledger_from_table(&GapClassTable::from_records(gaps), outcomes)
}

pub fn encode_gap_ledger_from_table<S: DomainOutcomeSource + ?Sized>(
    gaps: &GapClassTable,
    outcomes: &S,
) -> Value {
    json!({
        "schema": GAP_LEDGER_SCHEMA,
        "version": GAP_LEDGER_VERSION,
        "note": "run-encoded gap ledger: one class per distinct (reason, domain set, \
                 witness) tuple, with the hours it occurred at. Replaces the flat \
                 `gaps` and `domain_gap_outcomes` arrays, which cost 33.5 KB of \
                 manifest JSON per game hour. Expand it with the recorder_host \
                 gap_ledger module or the gap_ledger.py helper.",
        "gaps": gaps.to_json(),
        "domain_gap_outcomes": encode_domain_gap_outcomes(outcomes),
    })
}

pub fn read_compact_stream_gap_ledger(
    compact_stream: &Value,
) -> Result<(Vec<GapRecord>, Vec<DomainOutcome>), GapLedgerError> {
    if let Some(ledger) = compact_stream.get("gap_ledger") {
        let schema = ledger.get("schema").and_then(Value::as_str).unwrap_or("");
        let version = ledger.get("version").and_then(Value::as_u64);
        if schema != GAP_LEDGER_SCHEMA || version != Some(GAP_LEDGER_VERSION) {
            return Err(GapLedgerError::UnknownVersion {
                schema: schema.to_string(),
                version: version
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "none".into()),
            });
        }
        check_fields(
            ledger,
            &["schema", "version", "note", "gaps", "domain_gap_outcomes"],
            "gap_ledger",
        )?;
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
        let outcomes =
            expand_domain_gap_outcomes(ledger.get("domain_gap_outcomes").ok_or_else(|| {
                GapLedgerError::Malformed("gap_ledger.domain_gap_outcomes missing".into())
            })?)?;
        return Ok((gaps, outcomes));
    }
    Err(GapLedgerError::MissingLedger)
}

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

pub fn domain_outcomes_resident_bytes(count: usize) -> u64 {
    (count as u64) * (std::mem::size_of::<DomainOutcome>() as u64)
}

pub fn ledger_manifest_bytes(ledger: &Value, outcome_count: usize) -> u64 {
    let json = serde_json::to_vec(ledger)
        .map(|bytes| bytes.len() as u64)
        .unwrap_or(0);
    json + domain_outcomes_resident_bytes(outcome_count)
}

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
