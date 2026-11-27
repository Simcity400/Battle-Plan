
use dashboard_base::{Source, products::{Country as ParamCountry, LoadContext, at_hour::{At, AtHour}, validate_country}};
use crate::products::intelligence::{Country, IDENTITY, Tags, branch};
use dashboard_query::series::ranked::GroupSelection;
use dashboard_world::products::group_members::GroupsAt;
use replay_data::rows::{IntelNetworkSummaryRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const TABLES: [TableId; 3] = [TableId::IntelNetworkSummaries, IDENTITY[0], IDENTITY[1]];
const FIXED_SCALE: i64 = 100_000;

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
    #[doc = " A group: its members at the hour (or every member so far), resolved in Rust; replaces `countries` and `top`."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IntelPair {
    #[doc = " The owner's and the target's index in `displayed`."]
    pub row: u32,
    pub column: u32,
    pub value: i64,
    pub covered_controlled: i32,
    pub covered_core: i32,
    pub controlled_strength: i64,
    pub core_strength: i64,
    pub max_strength: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct IntelCoverageReply {
    pub displayed: Vec<Country>,
    #[doc = " The witnessed networks between displayed countries, by row then column."]
    pub pairs: Vec<IntelPair>,
    #[doc = " Why a pair not in `pairs` may still have a network (the summaries were read only in part); null when the read was whole, so an unlisted pair has none."]
    pub unlisted: Option<String>,
    pub max_value: i64,
    pub scale: i64,
    #[doc = " The summaries' read hour the rows hold (they are taken daily); it moves daily, so a widget"]
    #[doc = " renders nothing for it."]
    pub read_hour: Option<i32>,
    pub missing: Option<String>,
    #[doc = " Why the countries to show are not known at this hour (a top over summaries read only in part, or a group whose membership is not recorded); none are shown then."]
    pub chosen_missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(IntelCoverageReply);

impl IntelCoverageReply {
    fn empty(scale: i64, read_hour: Option<i32>, missing: Option<String>, chosen_missing: Option<String>) -> Self {
        Self { displayed: Vec::new(), pairs: Vec::new(), unlisted: None, max_value: 0, scale, read_hour, missing, chosen_missing, warnings: Vec::new() }
    }
}

fn row_value(row: &IntelNetworkSummaryRow, measure: IntelCoverageMeasure, basis: IntelCoverageBasis) -> i64 {
    match (measure, basis) {
        (IntelCoverageMeasure::Strongest, _) => row.max_state_strength,
        (IntelCoverageMeasure::Strength, IntelCoverageBasis::Core) => row.core_strength_sum,
        (IntelCoverageMeasure::Strength, IntelCoverageBasis::Controlled) => row.controlled_strength_sum,
        (IntelCoverageMeasure::Covered, IntelCoverageBasis::Core) => i64::from(row.covered_core_states),
        (IntelCoverageMeasure::Covered, IntelCoverageBasis::Controlled) => i64::from(row.covered_controlled_states),
    }
}

pub(crate) struct Matrix {
    pub displayed: Vec<Country>,
    pub pairs: Vec<IntelPair>,
    pub max_value: i64,
}

pub(crate) fn build_matrix(
    countries: &[Country],
    rows: &[IntelNetworkSummaryRow],
    tags: &Tags,
    selected: Option<&HashSet<String>>,
    top: Option<u32>,
    touching: bool,
    measure: IntelCoverageMeasure,
    basis: IntelCoverageBasis,
) -> Matrix {
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
    let mut chosen: Vec<Country> = if let Some(filter) = selected {
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
    let index: HashMap<&str, u32> = chosen.iter().enumerate().filter_map(|(at, c)| Some((c.tag.as_deref()?, at as u32))).collect();
    let mut by_pair: HashMap<(u32, u32), &IntelNetworkSummaryRow> = HashMap::new();
    for (o, t, row) in &pairs_by_tag {
        if let (Some(&r), Some(&c)) = (index.get(o.as_str()), index.get(t.as_str()))
            && r != c
        {
            by_pair.insert((r, c), row);
        }
    }
    let mut max_value = 0_i64;
    let mut pairs: Vec<IntelPair> = by_pair
        .into_iter()
        .map(|((row, column), held)| {
            let value = row_value(held, measure, basis);
            max_value = max_value.max(value);
            IntelPair {
                row,
                column,
                value,
                covered_controlled: held.covered_controlled_states,
                covered_core: held.covered_core_states,
                controlled_strength: held.controlled_strength_sum,
                core_strength: held.core_strength_sum,
                max_strength: held.max_state_strength,
            }
        })
        .collect();
    pairs.sort_unstable_by_key(|pair| (pair.row, pair.column));
    Matrix { displayed: chosen, pairs, max_value }
}

impl AtHour for IntelNetworkCoverage {
    const KEY: &'static str = "intel-network-coverage";
    const TITLE: &'static str = "Intel network coverage";
    const CONTENT_DEPENDENT: bool = false;
    type Params = IntelCoverageParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, countries, "countries[]", top, touching, measure, basis, group];
    type Reply = IntelCoverageReply;
    type Context = GroupsAt;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            IntelCoverageParams::decl(),
            IntelCoverageMeasure::decl(),
            IntelCoverageBasis::decl(),
            Country::decl(),
            IntelPair::decl(),
            IntelCoverageReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        branch(&params.branch_id)?;
        if let Some(countries) = &params.countries {
            countries.iter().try_for_each(validate_country)?;
        }
        if params.group.is_some() && (params.top.is_some() || params.countries.as_ref().is_some_and(|list| !list.is_empty())) {
            return Err("A matrix names a group, a top or countries, one of them");
        }
        Ok(())
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(GroupsAt::default())
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        GroupsAt::load(source, load, &params.branch_id, params.group.as_ref())
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        at.context.valid_through(params.group.as_ref(), at.hour)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let read_hour = frame.tables[TableId::IntelNetworkSummaries.index()].sampled_at();
        let measure = params.measure.unwrap_or_default();
        let basis = params.basis.unwrap_or_default();
        let scale = match measure {
            IntelCoverageMeasure::Covered => 1,
            IntelCoverageMeasure::Strength | IntelCoverageMeasure::Strongest => FIXED_SCALE,
        };
        if params.group.is_none()
            && let Some(reason) = params.countries.iter().flatten().find(|country| at.holders.tag_at(country, at.hour).is_none()).and_then(|country| at.holders.not_known(country, at.hour))
        {
            return Ok(IntelCoverageReply::empty(scale, read_hour, None, Some(reason)));
        }
        let tags = match Tags::read(frame) {
            Ok(tags) => tags,
            Err(error) => return Ok(IntelCoverageReply::empty(scale, read_hour, Some(error.to_string()), None)),
        };
        let summaries = dashboard_query::coverage::witnessed_rows::<tables::IntelNetworkSummaries>(frame);
        let view: &TableView = &frame.tables[TableId::IntelNetworkSummaries.index()];
        if !matches!(view, TableView::Complete(_) | TableView::Truncated { .. }) {
            return Ok(IntelCoverageReply::empty(scale, read_hour, summaries.missing, None));
        }
        let unlisted = summaries.missing;
        let selected: Option<HashSet<String>> = match &params.group {
            Some(group) => {
                let members = at.context.members(group, at.hour)?;
                if let Some(refused) = members.refused {
                    return Ok(IntelCoverageReply::empty(scale, read_hour, None, Some(refused)));
                }
                Some(members.tags.into_iter().collect())
            }
            None => params.countries.as_ref().filter(|list| !list.is_empty()).map(|list| {
                list.iter().filter_map(|country| at.holders.tag_at(country, at.hour).map(str::to_owned).or_else(|| country.tag().map(str::to_owned))).collect()
            }),
        };
        if selected.is_none()
            && let Some(reason) = &unlisted
        {
            return Ok(IntelCoverageReply::empty(scale, read_hour, None, Some(format!("{reason}, so the countries with the most networks are not known"))));
        }
        let matrix = build_matrix(&tags.present(), summaries.rows, &tags, selected.as_ref(), params.top, params.touching == Some(true), measure, basis);
        Ok(IntelCoverageReply { displayed: matrix.displayed, pairs: matrix.pairs, unlisted, max_value: matrix.max_value, scale, read_hour, missing: None, chosen_missing: None, warnings: Vec::new() })
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
        let matrix = build_matrix(&countries, &rows, &tags, Some(&filter), None, false, IntelCoverageMeasure::Covered, IntelCoverageBasis::Controlled);
        assert_eq!(matrix.displayed.len(), 2);
        assert_eq!(matrix.max_value, 3, "off-matrix CCC network with 50 states never inflates displayed max_value");
        assert_eq!(matrix.pairs.iter().map(|pair| (pair.row, pair.column, pair.value)).collect::<Vec<_>>(), vec![(0, 1, 3)], "only the witnessed network is a pair: no diagonal, no cell for a pair without a network");

        let ranked = build_matrix(&countries, &rows, &tags, None, Some(2), false, IntelCoverageMeasure::Covered, IntelCoverageBasis::Controlled);
        assert_eq!(ranked.displayed.iter().filter_map(|c| c.tag.as_deref()).collect::<Vec<_>>(), vec!["AAA", "BBB"]);
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
        let matrix = build_matrix(&countries, &rows, &tags, None, Some(2), false, IntelCoverageMeasure::Covered, IntelCoverageBasis::Controlled);
        assert_eq!(matrix.displayed.iter().filter_map(|c| c.tag.as_deref()).collect::<Vec<_>>(), vec!["AAA", "BBB"]);
        assert_eq!(matrix.pairs.iter().map(|pair| (pair.row, pair.column, pair.value)).collect::<Vec<_>>(), vec![(0, 1, 4)], "the second raw id's network is drawn, not shown as absent");
    }

    #[test]
    fn a_group_is_its_members_at_the_asked_hour_and_unknown_membership_shows_no_country() {
        use dashboard_query::{identity::Holders, series::ranked::{GroupKind, GroupMembership}};
        use dashboard_world::products::groups::{CountryGroupReply, MembershipRun, UnobservedHours};
        use dashboard_base::span::HourSpan;
        use dashboard_follow::live::Cells;
        use std::sync::{Arc, atomic::AtomicBool};
        let mut frame = DecodedFrame { branch: 1, hour: 500, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
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
        let network = |owner_country, target_country| IntelNetworkSummaryRow { owner_country, target_country, covered_controlled_states: 1, covered_core_states: 1, controlled_strength_sum: 0, core_strength_sum: 0, max_state_strength: 0, covered_worth: 0, ordinal: 0 };
        *frame.tables[TableId::IntelNetworkSummaries.index()] = TableView::Complete(TableRows::IntelNetworkSummaries(vec![network(1, 2), network(2, 1)]));
        let run = |tag: &str, first_hour, last_hour| MembershipRun { tag: tag.into(), first_hour, last_hour };
        let reply = CountryGroupReply {
            branch_id: "1".into(),
            humans: vec![run("AAA", 0, 1000), run("BBB", 600, 1000)],
            factions: Vec::new(),
            wars: Vec::new(),
            war_starts: Vec::new(),
            capitulations: Vec::new(),
            identity_changes: Vec::new(),
            country_identities: Vec::new(),
            unobserved: UnobservedHours { humans: vec![HourSpan { first_hour: 300, last_hour: 320 }], ..UnobservedHours::default() },
            warnings: Vec::new(),
        };
        let groups = GroupsAt::new(Arc::new(reply), 0);
        let holders = Holders::new(Default::default(), Vec::new());
        let group = GroupSelection { kind: GroupKind::Humans, id: None, tag: None, membership: GroupMembership::Hour };
        let params = IntelCoverageParams { branch_id: "1".into(), countries: None, top: None, touching: None, measure: None, basis: None, group: Some(group) };
        let live = Cells::default();
        let cancelled = AtomicBool::new(false);
        let at = |hour: i32| At { owner: None, holders: &holders, hour, context: &groups, cancelled: &cancelled, live: &live };
        let shown = |hour: i32| {
            let reply = IntelNetworkCoverage::reply(&frame, &params, &at(hour)).unwrap();
            (reply.displayed.iter().filter_map(|country| country.tag.clone()).collect::<Vec<_>>(), reply.chosen_missing)
        };
        assert_eq!(shown(500), (vec!["AAA".to_owned()], None), "a country joining at 600 is not shown at 500");
        assert_eq!(shown(700), (vec!["AAA".to_owned(), "BBB".to_owned()], None));
        let (none, refused) = shown(310);
        assert!(none.is_empty() && refused.is_some(), "unknown membership is never a guess or a union");
        assert_eq!(IntelNetworkCoverage::valid_through(&frame, &params, &at(500)), 599, "the reply holds only until the next member joins");
    }

    #[test]
    fn a_truncated_read_shows_its_witnessed_networks_marks_the_rest_not_read_and_ranks_no_top() {
        use dashboard_follow::live::Cells;
        use dashboard_query::identity::Holders;
        use std::sync::atomic::AtomicBool;
        let mut frame = DecodedFrame { branch: 1, hour: 500, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
            CountryRow { id: 1, canonical_slot: 1, tag: b"AAA".to_vec(), ..CountryRow::default() },
            CountryRow { id: 2, canonical_slot: 2, tag: b"BBB".to_vec(), ..CountryRow::default() },
        ]));
        *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow { id: 1, canonical_slot: 1 }, CountryLinkRow { id: 2, canonical_slot: 2 }]));
        let network = IntelNetworkSummaryRow { owner_country: 1, target_country: 2, covered_controlled_states: 5, covered_core_states: 1, controlled_strength_sum: 0, core_strength_sum: 0, max_state_strength: 0, covered_worth: 0, ordinal: 0 };
        *frame.tables[TableId::IntelNetworkSummaries.index()] = TableView::Truncated { rows: TableRows::IntelNetworkSummaries(vec![network]), missing: 3 };
        let groups = GroupsAt::default();
        let holders = Holders::new(Default::default(), Vec::new());
        let live = Cells::default();
        let cancelled = AtomicBool::new(false);
        let at = At { owner: None, holders: &holders, hour: 500, context: &groups, cancelled: &cancelled, live: &live };
        let chosen = IntelCoverageParams { branch_id: "1".into(), countries: Some(vec![ParamCountry::from("AAA"), ParamCountry::from("BBB")]), top: None, touching: None, measure: None, basis: None, group: None };
        let reply = IntelNetworkCoverage::reply(&frame, &chosen, &at).unwrap();
        assert_eq!(reply.missing, None, "a truncated read is not refused whole");
        assert_eq!(reply.pairs.iter().map(|pair| (pair.row, pair.column, pair.value)).collect::<Vec<_>>(), vec![(0, 1, 5)], "the witnessed network keeps its value");
        assert!(reply.unlisted.is_some(), "a pair not listed is not read, never 'no network'");
        let top = IntelCoverageParams { countries: None, top: Some(2), ..chosen };
        let reply = IntelNetworkCoverage::reply(&frame, &top, &at).unwrap();
        assert!(reply.displayed.is_empty() && reply.chosen_missing.is_some(), "the countries with the most networks are not known from part of the read");
    }
}

