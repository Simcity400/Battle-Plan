
use dashboard_base::{Source, products::{LoadContext, at_hour::{At, AtHour}}};
use crate::products::intelligence::{Country, IDENTITY, Tags, branch};
use dashboard_follow::live::{Cells, Source as Cell};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::io;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const TABLES: [TableId; 3] = [TableId::IntelNetworkSummaries, IDENTITY[0], IDENTITY[1]];

pub const FIELDS: [(&str, &str); 5] = [
    ("coveredControlled", "covered_controlled_states"),
    ("coveredCore", "covered_core_states"),
    ("controlledStrength", "controlled_strength_sum"),
    ("coreStrength", "core_strength_sum"),
    ("maxStrength", "max_state_strength"),
];

pub struct IntelNetworkCoverage;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntelCoverageParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IntelNetwork {
    pub owner: Country,
    pub target: Country,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IntelCoverageReply {
    #[doc = " Every country present at the hour (a slot the identity names), by tag: a pair of them"]
    #[doc = " without a network has none, where the summaries are known."]
    pub countries: Vec<Country>,
    #[doc = " By owner, then target id."]
    pub networks: Vec<IntelNetwork>,
    #[doc = " The summaries' read hour the rows hold (they are taken daily); it moves daily, so a widget"]
    #[doc = " renders nothing for it."]
    pub read_hour: Option<i32>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(IntelCoverageReply);

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
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId];
    type Reply = IntelCoverageReply;
    type Context = ();

    #[cfg(any(test, feature = "export-types"))]
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
