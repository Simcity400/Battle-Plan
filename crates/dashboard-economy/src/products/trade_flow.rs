use dashboard_base::products::{
    Country, LoadContext,
    at_hour::{At, AtHour},
    flows::{CountryFlowReply, FlowEdge, FlowPart, FlowShape, build_flows},
    resources::{RESOURCE_DECIMALS, ResourceNaming},
    validate_country,
};
use dashboard_base::Source;
use dashboard_world::products::holder_tags::HolderTags;
use replay_data::rows::{Recorded, TableId, TagHolderRow, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const NOT_RECORDED: &str = "A deal's daily delivery was not recorded at this hour";
const UNRESOLVED: &str = "Some deals name a country or resource the recording cannot resolve at this hour; they are left out";

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

pub fn edges(rows: &[replay_data::rows::TradeFlowRow], index: &HashMap<u32, u32>, tag: impl Fn((u32, u32)) -> Option<String>, resource: Option<u32>) -> (Vec<FlowEdge>, bool) {
    let mut out = Vec::with_capacity(rows.len());
    let mut unresolved = false;
    for row in rows {
        if resource.is_some_and(|only| only != row.resource) {
            continue;
        }
        if row.seller_slot == TagHolderRow::NONE || row.buyer_slot == TagHolderRow::NONE {
            unresolved = true;
            continue;
        }
        let (Some(from), Some(to), Some(&part)) = (tag((row.seller_slot, row.seller)), tag((row.buyer_slot, row.buyer)), index.get(&row.resource)) else {
            unresolved = true;
            continue;
        };
        let amount = match row.delivered {
            Recorded::Value(value) => Some(i128::from(value)),
            Recorded::NotRecorded => None,
        };
        out.push(FlowEdge { from, to, part, amount });
    }
    (out, unresolved)
}

impl AtHour for TradeFlow {
    const KEY: &'static str = "trade-flow";
    const TITLE: &'static str = "Trade flows";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = TradeFlowParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, subject, countries, "countries[]", between, limit, resource, table];
    type Reply = CountryFlowReply;
    type Context = TradeContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        use dashboard_base::products::flows::{FlowAmount, FlowLink, FlowTableRow};
        vec![TradeFlowParams::decl(), FlowPart::decl(), FlowAmount::decl(), FlowLink::decl(), FlowTableRow::decl(), CountryFlowReply::decl()]
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
        Ok(TradeContext { parts: Arc::new(Vec::new()), index: Arc::new(HashMap::new()), warnings: Vec::new() })
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let Ok(branch) = params.branch_id.parse::<u64>() else { return Self::context(source, load) };
        let query = dashboard_base::queries::query(source, branch)?;
        let mut warnings = Vec::new();
        let tokens = super::convoys::resource_tokens(&query, None, &mut warnings)?;
        let (resources, notes) = ResourceNaming::stored(source, load).infos(&tokens, &AtomicBool::new(false))?;
        warnings.extend(notes);
        let (parts, index) = parts_of(&resources);
        Ok(TradeContext { parts: Arc::new(parts), index: Arc::new(index), warnings })
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
        let tags = HolderTags::from_holders(at.holders);
        let (edges, unresolved) = edges(rows, &at.context.index, |identity| tags.latest_at(identity, at.hour).map(str::to_owned), params.resource);
        let resolve = |country: &Country| at.holders.tag_at(country, at.hour).map(str::to_owned).or_else(|| country.tag().map(str::to_owned));
        let subject = params.subject.as_ref().and_then(resolve);
        if params.subject.is_some() && subject.is_none() {
            return Ok(CountryFlowReply::not_recorded("The chosen country holds no tag at this hour".to_owned(), RESOURCE_DECIMALS, warnings));
        }
        let chosen: Option<HashSet<String>> = (!params.countries.is_empty()).then(|| params.countries.iter().filter_map(resolve).collect());
        let shape = FlowShape {
            subject: subject.as_deref(),
            chosen: chosen.as_ref(),
            between: params.between,
            limit: params.limit.map(|limit| limit.max(1) as usize),
            table: params.table,
            not_recorded: NOT_RECORDED,
        };
        let mut reply = build_flows(&edges, at.context.parts.to_vec(), RESOURCE_DECIMALS, &shape);
        reply.partial = unresolved.then(|| UNRESOLVED.to_owned());
        reply.warnings.append(&mut warnings);
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TradeFlowRow;

    fn row(seller: (u32, u32), buyer: (u32, u32), resource: u32, delivered: Recorded<i64>) -> TradeFlowRow {
        TradeFlowRow { seller_slot: seller.0, seller: seller.1, buyer_slot: buyer.0, buyer: buyer.1, resource, delivered, ..TradeFlowRow::default() }
    }

    fn tag((slot, _): (u32, u32)) -> Option<String> {
        Some(match slot {
            1 => "USA",
            2 => "ENG",
            _ => return None,
        }.to_owned())
    }

    #[test]
    fn deals_become_edges_and_unresolved_sides_are_reported_not_dropped_silently() {
        let index: HashMap<u32, u32> = [(3, 0), (7, 1)].into_iter().collect();
        let rows = [
            row((1, 10), (2, 20), 3, Recorded::Value(500_000)),
            row((1, 10), (2, 20), 7, Recorded::NotRecorded),
            row((1, 10), (9, 90), 3, Recorded::Value(1)),
            row((TagHolderRow::NONE, 0), (2, 20), 3, Recorded::Value(1)),
        ];
        let (out, unresolved) = edges(&rows, &index, tag, None);
        assert_eq!(out, [
            FlowEdge { from: "USA".into(), to: "ENG".into(), part: 0, amount: Some(500_000) },
            FlowEdge { from: "USA".into(), to: "ENG".into(), part: 1, amount: None },
        ]);
        assert!(unresolved);
        let (only, _) = edges(&rows, &index, tag, Some(7));
        assert_eq!(only.len(), 1);
    }
}
