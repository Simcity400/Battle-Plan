//! The desktop wire contract. Engine identifiers and native values are decimal strings.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReplayMetadata {
    pub path: String,
    pub name: String,
    pub format: u16,
    pub branches: Vec<BranchMetadata>,
    /// Observations from each branch's first readable country snapshot. Later
    /// successful queries add their catalogue; this is never a census of all hours.
    pub countries: Vec<CountryMetadata>,
    /// The recorded player's canonical slot, from the first readable snapshot.
    pub player_country_id: Option<String>,
    /// The recorder's identity witness, not a new local asset verification.
    pub content_verified: bool,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct BranchMetadata {
    pub id: String,
    pub session_id: String,
    /// Includes missing leading/trailing hours. This range is not a count of frames.
    pub first_hour: i32,
    pub last_hour: i32,
    /// A non-null parent is probable, never asserted.
    pub parent_id: Option<String>,
    pub parent_hour: Option<i32>,
    pub sample_boundary: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryMetadata {
    /// Canonical country slot, resolved through that hour's CountryLinks.
    pub id: String,
    pub tag: String,
    pub name: String,
    pub observed_branch_id: String,
    pub observed_hour: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountrySeriesRequest {
    pub branch_id: String,
    pub country_id: String,
    /// Defaults to 1200; requests outside 1..=1200 are rejected.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub max_points: Option<u32>,
    /// Inclusive bounds; omitted bounds select the corresponding branch edge.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub first_hour: Option<i32>,
    #[serde(default)]
    #[ts(optional = nullable)]
    pub last_hour: Option<i32>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryValues {
    pub political_power: Option<String>,
    pub command_power: Option<String>,
    pub stability: Option<String>,
    pub war_support: Option<String>,
    pub convoys: Option<String>,
}

/// Counts are bounded by the signed 32-bit hour domain (at most 2^32), and
/// therefore remain exact JavaScript numbers. Engine values never use this type.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountryMetricCounts {
    #[ts(type = "number")]
    pub political_power: u64,
    #[ts(type = "number")]
    pub command_power: u64,
    #[ts(type = "number")]
    pub stability: u64,
    #[ts(type = "number")]
    pub war_support: u64,
    #[ts(type = "number")]
    pub convoys: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountrySample {
    pub hour: i32,
    pub values: CountryValues,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum PointStatus {
    Recorded,
    Gap,
    Unavailable,
    Absent,
    Mixed,
}

/// A bounded time envelope, not an invented sample at its midpoint. For a
/// single-hour interval first/last/min/max coincide. On longer intervals the
/// extrema are exact, but their intervening path is not retained. A chart must
/// break at mixed/non-recorded points and for metrics with missingMetricHours.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountrySeriesPoint {
    pub first_hour: i32,
    pub last_hour: i32,
    pub status: PointStatus,
    #[ts(type = "number")]
    pub recorded_hours: u64,
    #[ts(type = "number")]
    pub gap_hours: u64,
    #[ts(type = "number")]
    pub unavailable_hours: u64,
    #[ts(type = "number")]
    pub absent_hours: u64,
    pub first: Option<CountrySample>,
    pub last: Option<CountrySample>,
    pub min: CountryValues,
    pub max: CountryValues,
    /// Missing optional values within otherwise recorded country samples.
    pub missing_metric_hours: CountryMetricCounts,
    /// First failure reason in the interval; counts include every failure.
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct SeriesCoverage {
    #[ts(type = "number")]
    pub recorded_hours: u64,
    #[ts(type = "number")]
    pub gap_hours: u64,
    #[ts(type = "number")]
    pub unavailable_hours: u64,
    #[ts(type = "number")]
    pub absent_hours: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CountrySeries {
    pub branch_id: String,
    pub session_id: String,
    pub country_id: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub country: Option<CountryMetadata>,
    /// Catalogue observed during this scan, scoped to this branch.
    pub countries: Vec<CountryMetadata>,
    pub points: Vec<CountrySeriesPoint>,
    pub coverage: SeriesCoverage,
    pub downsampled: bool,
    pub sample_boundary: String,
    /// All five metrics keep the save parser's fixed-point denominator.
    pub value_scale: String,
    pub warnings: Vec<String>,
}

/// One deterministic file, exported explicitly rather than as a test side effect.
pub fn typescript_contract() -> String {
    let declarations = [
        ReplayMetadata::decl(),
        BranchMetadata::decl(),
        CountryMetadata::decl(),
        CountrySeriesRequest::decl(),
        CountryValues::decl(),
        CountryMetricCounts::decl(),
        CountrySample::decl(),
        PointStatus::decl(),
        CountrySeriesPoint::decl(),
        SeriesCoverage::decl(),
        CountrySeries::decl(),
    ];
    let mut text =
        String::from("// Generated by cargo run -p analytics --bin export-types. Do not edit.\n");
    for declaration in declarations {
        text.push_str("export ");
        text.push_str(&declaration);
        text.push('\n');
    }
    text
}
