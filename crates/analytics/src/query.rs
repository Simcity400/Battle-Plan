//! Transport-independent dashboard requests. The wire keeps identifiers decimal.

use std::{fmt, io, mem::size_of};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{CountryMetadata, CountrySeries, CountryValues, MAX_POINTS};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceKey {
    pub source_id: String,
    pub revision: String,
}

impl SourceKey {
    pub(crate) fn validate(&self) -> Result<(), QueryError> {
        bounded_identifier(&self.source_id, "sourceId")?;
        bounded_identifier(&self.revision, "revision")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryTicket {
    pub scope_id: String,
    pub generation: String,
}

impl QueryTicket {
    pub(crate) fn generation_number(&self) -> Result<u64, QueryError> {
        bounded_identifier(&self.scope_id, "scopeId")?;
        decimal(&self.generation, "generation")
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum QueryKind {
    ExactWindow,
    History,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum QueryPriority {
    Current,
    Prefetch,
    History,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QueryRequest {
    pub scope_id: String,
    pub generation: String,
    pub source_id: String,
    pub revision: String,
    pub branch_id: String,
    /// Canonical country slots in this source/branch, not raw country references.
    pub country_ids: Vec<String>,
    pub first_hour: i32,
    pub last_hour: i32,
    pub kind: QueryKind,
    /// History defaults to MAX_POINTS. ExactWindow accepts its span or omission.
    #[serde(default)]
    #[ts(optional = nullable)]
    pub max_points: Option<u32>,
    pub priority: QueryPriority,
}

impl QueryRequest {
    pub fn ticket(&self) -> QueryTicket {
        QueryTicket {
            scope_id: self.scope_id.clone(),
            generation: self.generation.clone(),
        }
    }

    pub fn source_key(&self) -> SourceKey {
        SourceKey {
            source_id: self.source_id.clone(),
            revision: self.revision.clone(),
        }
    }
}

/// The supported decoder does not expose table acquisition timestamps. An
/// enclosing hour or an Unchanged baseline must not be used to manufacture them.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum SampleProvenance {
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    pub source_id: String,
    pub revision: String,
    pub branch_id: String,
    pub country_ids: Vec<String>,
    pub first_hour: i32,
    pub last_hour: i32,
    pub kind: QueryKind,
    pub max_points: u32,
    pub series: Vec<CountrySeries>,
    pub sample_provenance: SampleProvenance,
}

impl QueryResult {
    /// Owned Rust payload, including vector/string capacity. This is not a
    /// measurement of allocator bookkeeping, decoded blocks, or process RSS.
    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            .saturating_add(self.source_id.capacity())
            .saturating_add(self.revision.capacity())
            .saturating_add(self.branch_id.capacity())
            .saturating_add(strings_bytes(&self.country_ids))
            .saturating_add(
                self.series
                    .capacity()
                    .saturating_mul(size_of::<CountrySeries>()),
            )
            .saturating_add(self.series.iter().map(series_heap_bytes).sum::<usize>())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueryReply {
    pub scope_id: String,
    pub generation: String,
    pub result: QueryResult,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum QueryErrorCode {
    InvalidRequest,
    UnknownSource,
    SourceChanged,
    ResourceLimit,
    Cancelled,
    Closed,
    Decode,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct QueryError {
    pub code: QueryErrorCode,
    pub message: String,
}

impl QueryError {
    pub(crate) fn new(code: QueryErrorCode, message: impl AsRef<str>) -> Self {
        Self {
            code,
            message: crate::reduce::short_text(message.as_ref(), 1024),
        }
    }

    pub(crate) fn invalid(message: impl AsRef<str>) -> Self {
        Self::new(QueryErrorCode::InvalidRequest, message)
    }

    pub(crate) fn limit(message: impl AsRef<str>) -> Self {
        Self::new(QueryErrorCode::ResourceLimit, message)
    }

    pub(crate) fn cancelled() -> Self {
        Self::new(
            QueryErrorCode::Cancelled,
            "Analytics subscription was cancelled",
        )
    }

    pub(crate) fn closed() -> Self {
        Self::new(QueryErrorCode::Closed, "Analytics runtime is closed")
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for QueryError {}

impl From<io::Error> for QueryError {
    fn from(error: io::Error) -> Self {
        if let Some(query) = error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<Self>())
        {
            return query.clone();
        }
        let code = match error.kind() {
            io::ErrorKind::Interrupted => QueryErrorCode::Cancelled,
            io::ErrorKind::InvalidInput => QueryErrorCode::InvalidRequest,
            _ => QueryErrorCode::Decode,
        };
        Self::new(code, error.to_string())
    }
}

impl From<QueryError> for io::Error {
    fn from(error: QueryError) -> Self {
        let kind = match error.code {
            QueryErrorCode::Cancelled => io::ErrorKind::Interrupted,
            QueryErrorCode::InvalidRequest => io::ErrorKind::InvalidInput,
            _ => io::ErrorKind::Other,
        };
        Self::new(kind, error)
    }
}

#[derive(Clone, Debug)]
pub struct QueryLimits {
    pub max_countries: usize,
    pub max_country_hours: u64,
    pub max_result_bytes: usize,
    /// Admission limit on the format's raw block length, not its inflated heap.
    pub max_block_raw_bytes: usize,
}

impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            max_countries: 128,
            max_country_hours: u64::from(MAX_POINTS) * 128,
            max_result_bytes: 16 * 1024 * 1024,
            // Long recording blocks can exceed 256 MiB before typed decoding.
            // This admission ceiling is not a total decoder-memory estimate.
            max_block_raw_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct QueryKey {
    pub source: SourceKey,
    pub branch: u64,
    pub countries: Vec<u32>,
    pub first: i32,
    pub last: i32,
    pub kind: QueryKind,
    pub max_points: u32,
}

impl QueryKey {
    pub(crate) fn parse(request: &QueryRequest, limits: &QueryLimits) -> Result<Self, QueryError> {
        request.ticket().generation_number()?;
        let source = request.source_key();
        source.validate()?;
        let branch = decimal(&request.branch_id, "branchId")?;
        if request.country_ids.is_empty() || request.country_ids.len() > limits.max_countries {
            return Err(QueryError::limit(
                "countryIds must fit the nonempty country budget",
            ));
        }
        let mut countries = request
            .country_ids
            .iter()
            .map(|id| decimal(id, "countryId"))
            .collect::<Result<Vec<u32>, _>>()?;
        countries.sort_unstable();
        countries.dedup();
        if request.first_hour > request.last_hour {
            return Err(QueryError::invalid(
                "firstHour/lastHour must be an ordered inclusive range",
            ));
        }
        let span = (i64::from(request.last_hour) - i64::from(request.first_hour) + 1) as u64;
        let max_points = match request.kind {
            QueryKind::ExactWindow => {
                if span > u64::from(MAX_POINTS)
                    || span.saturating_mul(countries.len() as u64) > limits.max_country_hours
                {
                    return Err(QueryError::limit(
                        "Exact window exceeds its country-hour budget",
                    ));
                }
                let points = span as u32;
                if request.max_points.is_some_and(|value| value != points) {
                    return Err(QueryError::invalid(
                        "An exact window's maxPoints must equal its hour span",
                    ));
                }
                points
            }
            QueryKind::History => request.max_points.unwrap_or(MAX_POINTS),
        };
        if !(1..=MAX_POINTS).contains(&max_points) {
            return Err(QueryError::invalid("maxPoints must be between 1 and 1200"));
        }
        Ok(Self {
            source,
            branch,
            countries,
            first: request.first_hour,
            last: request.last_hour,
            kind: request.kind,
            max_points,
        })
    }
}

pub(crate) fn bounded_identifier(text: &str, name: &str) -> Result<(), QueryError> {
    if text.is_empty() || text.len() > 128 || text.contains('\0') {
        return Err(QueryError::invalid(format!(
            "{name} must be a nonempty identifier of at most 128 bytes"
        )));
    }
    Ok(())
}

pub(crate) fn decimal<T: std::str::FromStr + ToString>(
    text: &str,
    name: &str,
) -> Result<T, QueryError> {
    bounded_identifier(text, name)?;
    let value: T = text
        .parse()
        .map_err(|_| QueryError::invalid(format!("{name} must be a decimal identifier")))?;
    if value.to_string() != text {
        return Err(QueryError::invalid(format!(
            "{name} must use canonical decimal notation"
        )));
    }
    Ok(value)
}

pub(crate) fn metadata_heap_bytes(country: &CountryMetadata) -> usize {
    country.id.capacity()
        + country.tag.capacity()
        + country.name.capacity()
        + country.observed_branch_id.capacity()
}

fn strings_bytes(values: &Vec<String>) -> usize {
    values.capacity() * size_of::<String>() + values.iter().map(String::capacity).sum::<usize>()
}

fn values_bytes(values: &CountryValues) -> usize {
    [
        &values.political_power,
        &values.command_power,
        &values.stability,
        &values.war_support,
        &values.convoys,
    ]
    .into_iter()
    .filter_map(Option::as_ref)
    .map(String::capacity)
    .sum()
}

fn series_heap_bytes(series: &CountrySeries) -> usize {
    series.branch_id.capacity()
        + series.session_id.capacity()
        + series.country_id.capacity()
        + series.sample_boundary.capacity()
        + series.value_scale.capacity()
        + series.country.as_ref().map_or(0, metadata_heap_bytes)
        + series.countries.capacity() * size_of::<CountryMetadata>()
        + series
            .countries
            .iter()
            .map(metadata_heap_bytes)
            .sum::<usize>()
        + strings_bytes(&series.warnings)
        + series.points.capacity() * size_of::<crate::CountrySeriesPoint>()
        + series
            .points
            .iter()
            .map(|point| {
                values_bytes(&point.min)
                    + values_bytes(&point.max)
                    + point
                        .first
                        .as_ref()
                        .map_or(0, |sample| values_bytes(&sample.values))
                    + point
                        .last
                        .as_ref()
                        .map_or(0, |sample| values_bytes(&sample.values))
                    + point.reason.as_ref().map_or(0, String::capacity)
            })
            .sum::<usize>()
}
