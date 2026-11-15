//! Intel network coverage (family 64): every country's intel network in every other at an hour, as
//! the game's own retained coverage and strength caches hold them (rows at an hour of the daily
//! network summaries; the caches can be stale against the state rows, and are shown as kept). The
//! reply is the structure: the countries present and which directed pairs hold a network. Each
//! pair's five summary values are live cells; a pair's history is the declared `intel-networks`
//! series, read key by key.

use crate::{
    Source,
    products::{
        LoadContext,
        at_hour::{At, AtHour},
        intelligence::{Country, IDENTITY, Tags, branch},
    },
};
use dashboard_follow::live::{Cells, Source as Cell};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::io;
use ts_rs::TS;

const TABLES: [TableId; 3] = [TableId::IntelNetworkSummaries, IDENTITY[0], IDENTITY[1]];

/// The summary columns each pair's live cells read, by the names the widget gives them.
pub const FIELDS: [(&str, &str); 5] = [
    ("coveredControlled", "covered_controlled_states"),
    ("coveredCore", "covered_core_states"),
    ("controlledStrength", "controlled_strength_sum"),
    ("coreStrength", "core_strength_sum"),
    ("maxStrength", "max_state_strength"),
];

pub struct IntelNetworkCoverage;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntelCoverageParams {
    pub branch_id: String,
}

/// A directed pair with a network at the hour. Its values are the live cells of the row
/// `<owner id>:<target id>`, columns as [`FIELDS`] names them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IntelNetwork {
    pub owner: Country,
    pub target: Country,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IntelCoverageReply {
    /// Every country present at the hour (a slot the identity names), by tag: a pair of them
    /// without a network has none, where the summaries are known.
    pub countries: Vec<Country>,
    /// By owner, then target id.
    pub networks: Vec<IntelNetwork>,
    /// The summaries' read hour the rows hold (they are taken daily); it moves daily, so a widget
    /// renders nothing for it.
    pub read_hour: Option<i32>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(IntelCoverageReply);

/// The hour's networks, each pair's summary values named as live cells.
fn networks(frame: &DecodedFrame, tags: &Tags, live: &Cells) -> io::Result<Vec<IntelNetwork>> {
    let mut columns = Vec::with_capacity(FIELDS.len());
    for (name, field) in FIELDS {
        columns.push((name, dashboard_declare::field(TableId::IntelNetworkSummaries, field)?));
    }
    let mut out = Vec::new();
    for row in frame.complete::<tables::IntelNetworkSummaries>()? {
        let id = format!("{}:{}", row.owner_country, row.target_country);
        let key = (u128::from(row.owner_country) << 32) | u128::from(row.target_country);
        for &(name, column) in &columns {
            live.value(&id, name, Cell { table: TableId::IntelNetworkSummaries, column, key }, 1.0);
        }
        out.push(IntelNetwork { owner: Country::of(tags, row.owner_country), target: Country::of(tags, row.target_country) });
    }
    Ok(out)
}

impl AtHour for IntelNetworkCoverage {
    const KEY: &'static str = "intel-network-coverage";
    const TITLE: &'static str = "Intel network coverage";
    const CONTENT_DEPENDENT: bool = false;
    type Params = IntelCoverageParams;
    type Reply = IntelCoverageReply;
    type Context = ();

    fn types() -> Vec<String> {
        vec![IntelCoverageParams::decl(), Country::decl(), IntelNetwork::decl(), IntelCoverageReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, _params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let read_hour = frame.tables[TableId::IntelNetworkSummaries.index()].sampled_at();
        let tags = match Tags::read(frame) {
            Ok(tags) => tags,
            Err(error) => return Ok(IntelCoverageReply { countries: Vec::new(), networks: Vec::new(), read_hour, missing: Some(error.to_string()), warnings: Vec::new() }),
        };
        let countries = tags.present();
        Ok(match networks(frame, &tags, at.live) {
            Ok(networks) => IntelCoverageReply { countries, networks, read_hour, missing: None, warnings: Vec::new() },
            Err(error) => IntelCoverageReply { countries, networks: Vec::new(), read_hour, missing: Some(error.to_string()), warnings: Vec::new() },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schema_numbers_every_summary_column_the_cells_and_the_series_read() {
        for (name, field) in FIELDS {
            assert!(dashboard_declare::field(TableId::IntelNetworkSummaries, field).is_ok(), "{name}");
        }
        let series = dashboard_declare::SERIES.iter().find(|series| series.kind == "intel-networks").expect("declared");
        assert_eq!(series.table, TableId::IntelNetworkSummaries);
        assert!(dashboard_declare::read_by_key(TableId::IntelNetworkSummaries));
        let declared: Vec<&str> = series.measures.iter().map(|measure| measure.name).collect();
        assert_eq!(declared, FIELDS.map(|(name, _)| name));
    }
}
