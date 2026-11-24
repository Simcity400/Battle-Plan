
use dashboard_base::{Source, products::{Country as ParamCountry, LoadContext, at_hour::{At, AtHour}, validate_country}};
use crate::products::intelligence::{Country, IDENTITY, Tags, branch};
use dashboard_follow::live::{Cells, Source as Cell};
use replay_data::rows::{IntelNetworkSummaryRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const TABLES: [TableId; 3] = [TableId::IntelNetworkSummaries, IDENTITY[0], IDENTITY[1]];
const FIXED_SCALE: i64 = 100_000;

pub const FIELDS: [(&str, &str); 5] = [
    ("coveredControlled", "covered_controlled_states"),
    ("coveredCore", "covered_core_states"),
    ("controlledStrength", "controlled_strength_sum"),
    ("coreStrength", "core_strength_sum"),
    ("maxStrength", "max_state_strength"),
];

pub struct IntelNetworkCoverage;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum IntelCoverageMeasure {
    #[default]
    Covered,
    Strength,
    Strongest,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "lowercase")]
pub enum IntelCoverageBasis {
    #[default]
    Controlled,
    Core,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntelCoverageParams {
    pub branch_id: String,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub countries: Option<Vec<ParamCountry>>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub touching: Option<bool>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub measure: Option<IntelCoverageMeasure>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub basis: Option<IntelCoverageBasis>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IntelNetwork {
    pub owner: Country,
    pub target: Country,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum IntelCoverageCell {
    Present {
        value: i64,
        covered_controlled: i32,
        covered_core: i32,
        controlled_strength: i64,
        core_strength: i64,
        max_strength: i64,
    },
    NoNetwork,
    Diagonal,
    Missing {
        reason: String,
    },
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
    pub displayed: Vec<Country>,
    pub cells: Vec<IntelCoverageCell>,
    pub max_value: i64,
    pub scale: i64,
    #[doc = " The summaries' read hour the rows hold (they are taken daily); it moves daily, so a widget"]
    #[doc = " renders nothing for it."]
    pub read_hour: Option<i32>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(IntelCoverageReply);

fn row_value(row: &IntelNetworkSummaryRow, measure: IntelCoverageMeasure, basis: IntelCoverageBasis) -> i64 {
    match (measure, basis) {
        (IntelCoverageMeasure::Strongest, _) => row.max_state_strength,
        (IntelCoverageMeasure::Strength, IntelCoverageBasis::Core) => row.core_strength_sum,
        (IntelCoverageMeasure::Strength, IntelCoverageBasis::Controlled) => row.controlled_strength_sum,
        (IntelCoverageMeasure::Covered, IntelCoverageBasis::Core) => i64::from(row.covered_core_states),
        (IntelCoverageMeasure::Covered, IntelCoverageBasis::Controlled) => i64::from(row.covered_controlled_states),
    }
}

pub(crate) fn build_matrix(
    countries: &[Country],
    rows: &[IntelNetworkSummaryRow],
    tags: &Tags,
    selected_tags: Option<&HashSet<String>>,
    top: Option<u32>,
    touching: bool,
    measure: IntelCoverageMeasure,
    basis: IntelCoverageBasis,
) -> (Vec<Country>, Vec<IntelCoverageCell>, i64, i64) {
    let mut counts: HashMap<&str, usize> = countries.iter().filter_map(|c| Some((c.tag.as_deref()?, 0))).collect();
    let mut pairs_by_tag: Vec<(String, String, &IntelNetworkSummaryRow)> = Vec::new();
    for row in rows {
        let owner = Country::of(tags, row.owner_country);
        let target = Country::of(tags, row.target_country);
        if let (Some(o), Some(t)) = (owner.tag, target.tag) {
            if let Some(entry) = counts.get_mut(o.as_str()) {
                *entry += 1;
            }
            if let Some(entry) = counts.get_mut(t.as_str()) {
                *entry += 1;
            }
            pairs_by_tag.push((o, t, row));
        }
    }
    let mut chosen: Vec<Country> = if let Some(filter) = selected_tags {
        countries.iter().filter(|c| c.tag.as_deref().is_some_and(|tag| filter.contains(tag))).cloned().collect()
    } else {
        let limit = top.unwrap_or(12) as usize;
        let mut ranked: Vec<&Country> = countries.iter().collect();
        ranked.sort_by(|a, b| {
            let ca = a.tag.as_deref().and_then(|t| counts.get(t)).copied().unwrap_or(0);
            let cb = b.tag.as_deref().and_then(|t| counts.get(t)).copied().unwrap_or(0);
            cb.cmp(&ca).then(a.tag.cmp(&b.tag))
        });
        ranked.truncate(limit);
        ranked.into_iter().cloned().collect()
    };
    if touching && !chosen.is_empty() {
        let kept: HashSet<String> = chosen.iter().filter_map(|c| c.tag.clone()).collect();
        let mut linked: HashSet<&str> = HashSet::new();
        for (o, t, _) in &pairs_by_tag {
            if kept.contains(o.as_str()) {
                linked.insert(t.as_str());
            }
            if kept.contains(t.as_str()) {
                linked.insert(o.as_str());
            }
        }
        for c in countries {
            if let Some(tag) = c.tag.as_deref()
                && !kept.contains(tag)
                && linked.contains(tag)
            {
                chosen.push(c.clone());
            }
        }
    }
    let displayed_tags: HashSet<&str> = chosen.iter().filter_map(|c| c.tag.as_deref()).collect();
    let mut by_pair: HashMap<(&str, &str), &IntelNetworkSummaryRow> = HashMap::new();
    for (o, t, row) in &pairs_by_tag {
        if displayed_tags.contains(o.as_str()) && displayed_tags.contains(t.as_str()) {
            by_pair.insert((o.as_str(), t.as_str()), row);
        }
    }
    let scale = match measure {
        IntelCoverageMeasure::Covered => 1,
        IntelCoverageMeasure::Strength | IntelCoverageMeasure::Strongest => FIXED_SCALE,
    };
    let mut cells = Vec::with_capacity(chosen.len() * chosen.len());
    let mut max_value = 0_i64;
    for (r, owner) in chosen.iter().enumerate() {
        for (c, target) in chosen.iter().enumerate() {
            if r == c {
                cells.push(IntelCoverageCell::Diagonal);
                continue;
            }
            match owner.tag.as_deref().zip(target.tag.as_deref()).and_then(|pair| by_pair.get(&pair).copied()) {
                Some(row) => {
                    let value = row_value(row, measure, basis);
                    max_value = max_value.max(value);
                    cells.push(IntelCoverageCell::Present {
                        value,
                        covered_controlled: row.covered_controlled_states,
                        covered_core: row.covered_core_states,
                        controlled_strength: row.controlled_strength_sum,
                        core_strength: row.core_strength_sum,
                        max_strength: row.max_state_strength,
                    });
                }
                None => cells.push(IntelCoverageCell::NoNetwork),
            }
        }
    }
    (chosen, cells, max_value, scale)
}

fn networks(frame: &DecodedFrame, tags: &Tags, live: &Cells) -> io::Result<(Vec<IntelNetwork>, Vec<IntelNetworkSummaryRow>)> {
    let mut columns = Vec::with_capacity(FIELDS.len());
    for (name, field) in FIELDS {
        columns.push((name, dashboard_declare::field(TableId::IntelNetworkSummaries, field)?));
    }
    let rows = frame.complete::<tables::IntelNetworkSummaries>()?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let id = format!("{}:{}", row.owner_country, row.target_country);
        let key = (u128::from(row.owner_country) << 32) | u128::from(row.target_country);
        for &(name, column) in &columns {
            live.value(&id, name, Cell { table: TableId::IntelNetworkSummaries, column, key }, 1.0);
        }
        out.push(IntelNetwork { owner: Country::of(tags, row.owner_country), target: Country::of(tags, row.target_country) });
    }
    Ok((out, rows.to_vec()))
}

impl AtHour for IntelNetworkCoverage {
    const KEY: &'static str = "intel-network-coverage";
    const TITLE: &'static str = "Intel network coverage";
    const CONTENT_DEPENDENT: bool = false;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = IntelCoverageParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, "countries[]", top, touching, measure, basis];
    type Reply = IntelCoverageReply;
    type Context = ();

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            IntelCoverageParams::decl(),
            IntelCoverageMeasure::decl(),
            IntelCoverageBasis::decl(),
            IntelCoverageCell::decl(),
            Country::decl(),
            IntelNetwork::decl(),
            IntelCoverageReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        if let Some(countries) = &params.countries {
            countries.iter().try_for_each(validate_country)?;
        }
        Ok(())
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(())
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let read_hour = frame.tables[TableId::IntelNetworkSummaries.index()].sampled_at();
        let measure = params.measure.unwrap_or_default();
        let basis = params.basis.unwrap_or_default();
        let scale = match measure {
            IntelCoverageMeasure::Covered => 1,
            IntelCoverageMeasure::Strength | IntelCoverageMeasure::Strongest => FIXED_SCALE,
        };
        let tags = match Tags::read(frame) {
            Ok(tags) => tags,
            Err(error) => {
                return Ok(IntelCoverageReply {
                    countries: Vec::new(),
                    networks: Vec::new(),
                    displayed: Vec::new(),
                    cells: Vec::new(),
                    max_value: 0,
                    scale,
                    read_hour,
                    missing: Some(error.to_string()),
                    warnings: Vec::new(),
                });
            }
        };
        let countries = tags.present();
        let selected: Option<HashSet<String>> = params.countries.as_ref().filter(|list| !list.is_empty()).map(|list| {
            list.iter().filter_map(|country| at.holders.tag_at(country, at.hour).map(str::to_owned).or_else(|| country.tag().map(str::to_owned))).collect()
        });
        Ok(match networks(frame, &tags, at.live) {
            Ok((networks, rows)) => {
                let (displayed, cells, max_value, scale) = build_matrix(&countries, &rows, &tags, selected.as_ref(), params.top, params.touching == Some(true), measure, basis);
                IntelCoverageReply { countries, networks, displayed, cells, max_value, scale, read_hour, missing: None, warnings: Vec::new() }
            }
            Err(error) => IntelCoverageReply {
                countries,
                networks: Vec::new(),
                displayed: Vec::new(),
                cells: Vec::new(),
                max_value: 0,
                scale,
                read_hour,
                missing: Some(error.to_string()),
                warnings: Vec::new(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::{CountryLinkRow, CountryRow, TableRows};
    use replay_frame::frame::{Metrics, TableView, not_selected};

    #[test]
    fn intel_matrix_ranks_top_countries_and_scopes_max_value_to_displayed_cells_only() {
        let mut frame = DecodedFrame { branch: 1, hour: 100, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
            CountryRow { id: 1, canonical_slot: 1, tag: b"AAA".to_vec(), ..CountryRow::default() },
            CountryRow { id: 2, canonical_slot: 2, tag: b"BBB".to_vec(), ..CountryRow::default() },
            CountryRow { id: 3, canonical_slot: 3, tag: b"CCC".to_vec(), ..CountryRow::default() },
        ]));
        *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow { id: 1, canonical_slot: 1 },
            CountryLinkRow { id: 2, canonical_slot: 2 },
            CountryLinkRow { id: 3, canonical_slot: 3 },
        ]));
        let tags = Tags::read(&frame).unwrap();
        let countries = tags.present();
        let rows = vec![
            IntelNetworkSummaryRow { owner_country: 1, target_country: 2, covered_controlled_states: 3, covered_core_states: 2, controlled_strength_sum: 250_000, core_strength_sum: 180_000, max_state_strength: 100_000, covered_worth: 0, ordinal: 0 },
            IntelNetworkSummaryRow { owner_country: 3, target_country: 1, covered_controlled_states: 50, covered_core_states: 40, controlled_strength_sum: 9_000_000, core_strength_sum: 8_000_000, max_state_strength: 1_000_000, covered_worth: 0, ordinal: 0 },
        ];
        let filter = HashSet::from(["AAA".to_owned(), "BBB".to_owned()]);
        let (displayed, cells, max_value, scale) = build_matrix(&countries, &rows, &tags, Some(&filter), None, false, IntelCoverageMeasure::Covered, IntelCoverageBasis::Controlled);
        assert_eq!(displayed.len(), 2);
        assert_eq!(scale, 1);
        assert_eq!(max_value, 3, "off-matrix CCC network with 50 states never inflates displayed max_value");
        assert_eq!(cells[0], IntelCoverageCell::Diagonal);
        assert!(matches!(cells[1], IntelCoverageCell::Present { value: 3, .. }));
        assert_eq!(cells[2], IntelCoverageCell::NoNetwork);

        let (ranked, _, _, _) = build_matrix(&countries, &rows, &tags, None, Some(2), false, IntelCoverageMeasure::Covered, IntelCoverageBasis::Controlled);
        assert_eq!(ranked.iter().filter_map(|c| c.tag.as_deref()).collect::<Vec<_>>(), vec!["AAA", "BBB"]);
    }

    #[test]
    fn a_network_counted_for_the_ranking_is_the_one_the_matrix_draws_when_two_raw_ids_share_a_slot() {
        let mut frame = DecodedFrame { branch: 1, hour: 100, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
            CountryRow { id: 1, canonical_slot: 1, tag: b"AAA".to_vec(), ..CountryRow::default() },
            CountryRow { id: 2, canonical_slot: 2, tag: b"BBB".to_vec(), ..CountryRow::default() },
        ]));
        *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow { id: 1, canonical_slot: 1 },
            CountryLinkRow { id: 2, canonical_slot: 2 },
            CountryLinkRow { id: 10, canonical_slot: 1 },
        ]));
        let tags = Tags::read(&frame).unwrap();
        let countries = tags.present();
        let rows = vec![IntelNetworkSummaryRow { owner_country: 10, target_country: 2, covered_controlled_states: 4, covered_core_states: 4, controlled_strength_sum: 0, core_strength_sum: 0, max_state_strength: 0, covered_worth: 0, ordinal: 0 }];
        let (displayed, cells, _, _) = build_matrix(&countries, &rows, &tags, None, Some(2), false, IntelCoverageMeasure::Covered, IntelCoverageBasis::Controlled);
        assert_eq!(displayed.iter().filter_map(|c| c.tag.as_deref()).collect::<Vec<_>>(), vec!["AAA", "BBB"]);
        assert!(matches!(cells[1], IntelCoverageCell::Present { value: 4, .. }), "the second raw id's network is drawn, not shown as absent: {:?}", cells[1]);
    }
}

