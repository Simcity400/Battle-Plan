//! National focus, prepared as the `focus` series family on the segment driver ([`facts`],
//! [`emit`]) and answered from its publication ([`query`]): each country's focus order (its
//! completed focuses with their dates, the active focus and pause), its progress and daily gain
//! at every hour it was observed, the four tables' refusals, and the completions the event feed
//! lists. Focus titles, icons and descriptions are not prepared: [`art`] evaluates them at the
//! hour a reply asks for.
use crate::products::{ContentOptions, LoadContext, artwork};
use crate::{Artwork, FocusEntry, Source, metadata::check_cancel};
use replay_data::rows::TableId;
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;

/// Row-page size limits a read, never the number of countries or completed focuses.
const PAGE_ROWS: usize = 128;

/// The schema replies name.
const STATE_SCHEMA: u32 = 6;

#[derive(Default, Clone, Debug, PartialEq, Eq)]
struct StateSchema;
impl Serialize for StateSchema {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(STATE_SCHEMA)
    }
}
impl<'de> Deserialize<'de> for StateSchema {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let schema = u32::deserialize(deserializer)?;
        if schema != STATE_SCHEMA {
            return Err(serde::de::Error::custom(
                "Unsupported prepared focus table/schema; rebuild focus pages",
            ));
        }
        Ok(Self)
    }
}

pub use artwork::FocusProjection as Presentation;

/// Why a country has no focus observation at an hour (counted per country and hour), or a
/// completion has no event date (counted per completion).
pub(crate) const UNRESOLVED: &str = "Focus completion observation: unresolved country identity";
pub(crate) const NO_FOCUS_ROW: &str = "Focus completion observation: country focus row missing";
pub(crate) const NO_TAG: &str = "Focus completion observation: country tag is empty or not UTF-8";
pub(crate) const INCOMPLETE_ORDER: &str = "Focus completion observation: completed order incomplete";
pub(crate) const INCOMPLETE_TABLES: &str =
    "Focus completion observation: required tables incomplete (counted per hour)";
pub(crate) const UNDATED: &str = "Focus completion observation: completions first seen after the country was not observed have no event dates";
pub(crate) const RESET: &str =
    "Focus completion observation: reset or reordered completion list has no event dates";

/// One table's refusal at an hour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusTableRefusal")]
pub struct TableRefusal {
    pub table: String,
    pub reason: Option<String>,
    pub missing_rows: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusArt")]
pub struct PresentedFocus {
    pub artwork: Artwork,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusPages")]
pub struct Pages {
    #[serde(with = "decimal_u64")]
    #[ts(type = "string")]
    pub first: u64,
    #[serde(with = "decimal_u64")]
    #[ts(type = "string")]
    pub count: u64,
    #[serde(with = "decimal_u64")]
    #[ts(type = "string")]
    pub rows: u64,
}

mod decimal_u64 {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

/// A country's focus at an hour: its order fields hold from `hour` through
/// `lastHour`; progress and its daily gain are those at `at`, the hour asked.
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusHour")]
pub struct Hour {
    #[ts(type = "number")]
    schema: StateSchema,
    pub slot: u32,
    pub country_id: u32,
    pub hour: i32,
    pub last_hour: i32,
    pub order_first_hour: Option<i32>,
    pub current: Option<String>,
    pub continuous: Option<String>,
    pub paused: bool,
    pub completed: Pages,
    pub at: i32,
    pub progress_raw: String,
    pub daily_gain_raw: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename = "PreparedFocusUnavailableHour")]
pub struct UnavailableHour {
    #[ts(type = "number")]
    schema: StateSchema,
    pub first_hour: i32,
    pub last_hour: i32,
    pub reasons: Vec<TableRefusal>,
}

pub mod art;
pub mod emit;
pub mod facts;
pub mod query;
pub mod served;
pub use art::{FocusArt, FocusArtFailure, FocusArtMode, FocusArtParams, FocusArtReply};

/// The hourly row census the publication is held to.
#[cfg(test)]
pub(crate) mod row;
#[cfg(test)]
mod art_tests;
#[cfg(test)]
mod columnar_tests;
