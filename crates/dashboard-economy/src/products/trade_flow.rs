use dashboard_base::products::{
    Country, LoadContext,
    at_hour::{At, AtHour},
    flows::{CountryFlowReply, FlowEdge, FlowEndpoint, FlowPart, FlowShape, build_flows},
    resources::{RESOURCE_DECIMALS, ResourceNaming},
    validate_country,
};
use dashboard_base::Source;
use dashboard_query::series::ranked::GroupSelection;
use dashboard_world::products::group_members::GroupsAt;
use replay_data::rows::{Recorded, TableId, TagHolderRow, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const NOT_RECORDED: &str = "A deal's daily delivery was not recorded at this hour";
const UNLINKED: &str = "The deal names a country the recording cannot link at this hour";
const UNRESOLVED: &str = "Some deals name a resource the recording cannot resolve at this hour; they are left out";

pub struct TradeFlow;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TradeFlowParams {
    pub branch_id: String,
    #[doc = " One country: its imports and exports by partner. Otherwise the links between `countries` (every country when empty)."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub subject: Option<Country>,
    #[serde(default)]
    pub countries: Vec<Country>,
    #[serde(default)]
    pub between: bool,
    #[doc = " A group: its members at the hour (or every member so far), resolved in Rust; replaces `countries`."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub group: Option<GroupSelection>,
    #[doc = " Partners shown per side before the rest are folded into one."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub limit: Option<u32>,
    #[doc = " One resource by its native index; every resource when absent."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub resource: Option<u32>,
    #[doc = " The partner by resource table of signed daily amounts."]
    #[serde(default)]
    pub table: bool,
}

pub struct TradeContext {
    parts: Arc<Vec<FlowPart>>,
    index: Arc<HashMap<u32, u32>>,
    warnings: Vec<String>,
    groups: GroupsAt,
}

pub fn parts_of(resources: &[dashboard_base::products::resources::ResourceInfo]) -> (Vec<FlowPart>, HashMap<u32, u32>) {
    let mut ordered: Vec<&dashboard_base::products::resources::ResourceInfo> = resources.iter().collect();
    ordered.sort_by_key(|resource| resource.id);
    let index = ordered.iter().enumerate().map(|(at, resource)| (resource.id, at as u32)).collect();
    let parts = ordered
        .iter()
        .enumerate()
        .map(|(at, resource)| FlowPart {
            key: resource.token.clone(),
            name: resource.name.clone(),
            name_missing: resource.name.is_none().then(|| format!("The content names no resource {}", resource.token)),
            ordinal: at as u32,
            icon: resource.icon.clone(),
        })
        .collect();
    (parts, index)
}

pub fn edges(rows: &[replay_data::rows::TradeFlowRow], index: &HashMap<u32, u32>, resource: Option<u32>) -> (Vec<FlowEdge>, bool) {
    let mut out = Vec::with_capacity(rows.len());
    let mut unresolved = false;
    let slot = |slot: u32| (slot != TagHolderRow::NONE).then_some(slot);
    for row in rows {
        if resource.is_some_and(|only| only != row.resource) {
            continue;
        }
        let Some(&part) = index.get(&row.resource) else {
            unresolved = true;
            continue;
        };
        let amount = match row.delivered {
            Recorded::Value(value) => Some(i128::from(value)),
            Recorded::NotRecorded => None,
        };
        out.push(FlowEdge { from: slot(row.seller_slot), to: slot(row.buyer_slot), part, amount });
    }
    (out, unresolved)
}

impl AtHour for TradeFlow {
    const KEY: &'static str = "trade-flow";
    const TITLE: &'static str = "Trade flows";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::Boundaries;
    type Params = TradeFlowParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, subject, countries, "countries[]", between, group, limit, resource, table];
    type Reply = CountryFlowReply;
    type Context = TradeContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        use dashboard_base::products::flows::{FlowAmount, FlowLink, FlowSide, FlowTableRow};
        vec![TradeFlowParams::decl(), FlowPart::decl(), FlowAmount::decl(), FlowEndpoint::decl(), FlowLink::decl(), FlowSide::decl(), FlowTableRow::decl(), CountryFlowReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        params.countries.iter().try_for_each(validate_country)?;
        params.subject.as_ref().map_or(Ok(()), validate_country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::TradeFlows, TableId::ResourceExchanges]
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(TradeContext { parts: Arc::new(Vec::new()), index: Arc::new(HashMap::new()), warnings: Vec::new(), groups: GroupsAt::default() })
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let Ok(branch) = params.branch_id.parse::<u64>() else { return Self::context(source, load) };
        let query = dashboard_base::queries::query(source, branch)?;
        let mut warnings = Vec::new();
        let tokens = super::convoys::resource_tokens(&query, None, &mut warnings)?;
        let (resources, notes) = ResourceNaming::stored(source, load).infos(&tokens, &AtomicBool::new(false))?;
        warnings.extend(notes);
        let (parts, index) = parts_of(&resources);
        let groups = GroupsAt::load(source, load, &params.branch_id, params.group.as_ref())?;
        Ok(TradeContext { parts: Arc::new(parts), index: Arc::new(index), warnings, groups })
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        if params.subject.is_some() { i32::MAX } else { at.context.groups.valid_through(params.group.as_ref(), at.hour) }
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut warnings = at.context.warnings.clone();
        for table in [TableId::TradeFlows, TableId::ResourceExchanges] {
            if let Some(reason) = dashboard_text::scope::unread(table, &frame.tables[table.index()]) {
                return Ok(CountryFlowReply::not_recorded(reason, RESOURCE_DECIMALS, warnings));
            }
        }
        let rows = match frame.complete::<tables::TradeFlows>() {
            Ok(rows) => rows,
            Err(error) => return Ok(CountryFlowReply::not_recorded(error.to_string(), RESOURCE_DECIMALS, warnings)),
        };
        let (edges, unresolved) = edges(rows, &at.context.index, params.resource);
        let resolve = |country: &Country| at.holders.slot_at(country, at.hour);
        let subject = params.subject.as_ref().map(resolve);
        if let Some(None) = subject {
            return Ok(CountryFlowReply::not_recorded("The chosen country holds no tag at this hour".to_owned(), RESOURCE_DECIMALS, warnings));
        }
        let chosen = match at.context.groups.chosen_slots(params.group.as_ref(), &params.countries, at.holders, at.hour)? {
            Ok(chosen) => chosen,
            Err(refused) => return Ok(CountryFlowReply::not_recorded(refused, RESOURCE_DECIMALS, warnings)),
        };
        let shape = FlowShape {
            subject: subject.flatten(),
            chosen: chosen.as_ref(),
            between: params.between,
            limit: params.limit.map(|limit| limit.max(1) as usize),
            table: params.table,
            not_recorded: NOT_RECORDED,
            unlinked: UNLINKED,
            incomplete: unresolved.then_some(UNRESOLVED),
        };
        let mut reply = build_flows(&edges, at.context.parts.to_vec(), RESOURCE_DECIMALS, &shape, &|slot| FlowEndpoint::at(at.holders, slot, at.hour, UNLINKED));
        reply.warnings.append(&mut warnings);
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TradeFlowRow;

    fn row(seller: u32, buyer: u32, resource: u32, delivered: Recorded<i64>) -> TradeFlowRow {
        TradeFlowRow { seller_slot: seller, seller: seller * 10, buyer_slot: buyer, buyer: buyer * 10, resource, delivered, ..TradeFlowRow::default() }
    }

    #[test]
    fn deals_keep_their_recorded_slots_and_an_unlinked_side_stays_a_deal_with_an_unknown_endpoint() {
        let index: HashMap<u32, u32> = [(3, 0), (7, 1)].into_iter().collect();
        let none = TagHolderRow::NONE;
        let rows = [
            row(1, 2, 3, Recorded::Value(500_000)),
            row(1, 2, 7, Recorded::NotRecorded),
            row(none, 2, 3, Recorded::Value(1)),
            row(1, 2, 99, Recorded::Value(1)),
        ];
        let (out, unresolved) = edges(&rows, &index, None);
        assert_eq!(out, [
            FlowEdge { from: Some(1), to: Some(2), part: 0, amount: Some(500_000) },
            FlowEdge { from: Some(1), to: Some(2), part: 1, amount: None },
            FlowEdge { from: None, to: Some(2), part: 0, amount: Some(1) },
        ]);
        assert!(unresolved);
        let (only, _) = edges(&rows, &index, Some(7));
        assert_eq!(only.len(), 1);
    }
}
