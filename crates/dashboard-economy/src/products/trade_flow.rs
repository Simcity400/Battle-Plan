use dashboard_base::products::{
    Country, LoadContext,
    at_hour::{At, AtHour, Owner, Read},
    flows::{CountryFlowReply, FlowEdge, FlowEndpoint, FlowPart, FlowShape, build_flows},
    resources::{RESOURCE_DECIMALS, ResourceInfo, ResourceNaming},
    validate_country,
};
use dashboard_base::Source;
use dashboard_query::series::ranked::GroupSelection;
use dashboard_world::products::{equipment_names::spelled, group_members::GroupsAt};
use replay_data::rows::{LexicalTokenRow, Recorded, ResourceDefinitionRow, TableId, TagHolderRow, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

const NOT_RECORDED: &str = "A deal's daily delivery was not recorded at this hour";
const UNLINKED: &str = "The deal names a country the recording cannot link at this hour";

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
    naming: Option<Arc<ResourceNaming>>,
    groups: GroupsAt,
}

pub struct Resources {
    pub spelled: Vec<(u32, String)>,
    pub unresolved: BTreeMap<u32, String>,
}

pub fn resources_at(definitions: Result<&[ResourceDefinitionRow], String>, lexical: Option<&[LexicalTokenRow]>, dealt: impl IntoIterator<Item = u32>) -> Resources {
    let mut out = Resources { spelled: Vec::new(), unresolved: BTreeMap::new() };
    let defined: HashMap<u32, u32> = definitions.as_ref().map(|rows| rows.iter().map(|row| (row.id, row.token)).collect()).unwrap_or_default();
    for (&id, &token) in &defined {
        if let Some(text) = lexical.and_then(|lexical| spelled(lexical, token)) {
            out.spelled.push((id, text));
        }
    }
    out.spelled.sort_unstable();
    for id in dealt {
        if out.unresolved.contains_key(&id) || out.spelled.binary_search_by_key(&id, |(held, _)| *held).is_ok() {
            continue;
        }
        let why = match (&definitions, defined.contains_key(&id)) {
            (Err(error), _) => format!("Resource {id}: resource definitions were not recorded at this hour ({error})"),
            (Ok(_), false) => format!("Resource {id} has no recorded definition at this hour"),
            (Ok(_), true) => format!("Resource {id} has no recorded spelling at this hour"),
        };
        out.unresolved.insert(id, why);
    }
    out
}

pub fn parts_at(named: &[ResourceInfo], unresolved: &BTreeMap<u32, String>) -> (Vec<FlowPart>, HashMap<u32, u32>) {
    let mut parts: BTreeMap<u32, FlowPart> = named
        .iter()
        .map(|resource| {
            (resource.id, FlowPart {
                key: resource.token.clone(),
                name: resource.name.clone(),
                name_missing: resource.name.is_none().then(|| format!("The content names no resource {}", resource.token)),
                ordinal: 0,
                icon: resource.icon.clone(),
            })
        })
        .collect();
    for (&id, why) in unresolved {
        parts.insert(id, FlowPart { key: format!("resource:{id}"), name: None, name_missing: Some(why.clone()), ordinal: 0, icon: None });
    }
    let index = parts.keys().enumerate().map(|(at, &id)| (id, at as u32)).collect();
    let parts = parts.into_values().enumerate().map(|(at, part)| FlowPart { ordinal: at as u32, ..part }).collect();
    (parts, index)
}

pub fn edges(rows: &[replay_data::rows::TradeFlowRow], index: &HashMap<u32, u32>, resource: Option<u32>) -> Vec<FlowEdge> {
    let mut out = Vec::with_capacity(rows.len());
    let slot = |slot: u32| (slot != TagHolderRow::NONE).then_some(slot);
    for row in rows {
        if resource.is_some_and(|only| only != row.resource) {
            continue;
        }
        let Some(&part) = index.get(&row.resource) else { continue };
        let amount = match row.delivered {
            Recorded::Value(value) => Some(i128::from(value)),
            Recorded::NotRecorded => None,
        };
        out.push(FlowEdge { from: slot(row.seller_slot), to: slot(row.buyer_slot), part, amount });
    }
    out
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
        vec![TableId::TradeFlows, TableId::ResourceDefinitions, TableId::LexicalTokens]
    }

    fn reads(_params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match step {
            0 => [TableId::TradeFlows, TableId::ResourceDefinitions].into_iter().map(Read::all).collect(),
            1 => {
                let tokens = held.complete::<tables::ResourceDefinitions>().map(|rows| rows.iter().map(|row| u128::from(row.token)).collect::<Vec<_>>()).unwrap_or_default();
                vec![Read::these(TableId::LexicalTokens, tokens)]
            }
            _ => Vec::new(),
        }
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(TradeContext { naming: None, groups: GroupsAt::default() })
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let groups = GroupsAt::load(source, load, &params.branch_id, params.group.as_ref())?;
        Ok(TradeContext { naming: Some(Arc::new(ResourceNaming::stored(source, load))), groups })
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        if params.subject.is_some() { i32::MAX } else { at.context.groups.valid_through(params.group.as_ref(), at.hour) }
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut warnings = Vec::new();
        if let Some(reason) = dashboard_text::scope::unread(TableId::TradeFlows, &frame.tables[TableId::TradeFlows.index()]) {
            return Ok(CountryFlowReply::not_recorded(reason, RESOURCE_DECIMALS, warnings));
        }
        let rows = match frame.complete::<tables::TradeFlows>() {
            Ok(rows) => rows,
            Err(error) => return Ok(CountryFlowReply::not_recorded(error.to_string(), RESOURCE_DECIMALS, warnings)),
        };
        let definitions = frame.complete::<tables::ResourceDefinitions>().map_err(|error| error.to_string());
        let resources = resources_at(definitions, frame.complete::<tables::LexicalTokens>().ok(), rows.iter().map(|row| row.resource));
        let named = match &at.context.naming {
            Some(naming) => {
                let (named, notes) = naming.infos(&resources.spelled, at.cancelled)?;
                warnings.extend(notes);
                named
            }
            None => Vec::new(),
        };
        let (parts, index) = parts_at(&named, &resources.unresolved);
        let edges = edges(rows, &index, params.resource);
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
            incomplete: None,
        };
        let mut reply = build_flows(&edges, parts, RESOURCE_DECIMALS, &shape, &|slot| FlowEndpoint::at(at.holders, slot, at.hour, UNLINKED));
        reply.warnings.append(&mut warnings);
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TradeFlowRow;

    fn row(seller: u32, buyer: u32, resource: u32, delivered: Recorded<i64>) -> TradeFlowRow {
        TradeFlowRow { seller_slot: seller, seller: seller.wrapping_mul(10), buyer_slot: buyer, buyer: buyer.wrapping_mul(10), resource, delivered, ..TradeFlowRow::default() }
    }

    fn definition(id: u32, token: u32) -> ResourceDefinitionRow {
        ResourceDefinitionRow { id, token, ..ResourceDefinitionRow::default() }
    }

    fn word(token: u32, text: &str) -> LexicalTokenRow {
        LexicalTokenRow { token, literal: text.as_bytes().to_vec() }
    }

    fn info(id: u32, token: &str) -> ResourceInfo {
        ResourceInfo { id, token: token.to_owned(), ordinal: 1, name: Some(token.to_owned()), icon: None }
    }

    #[test]
    fn deals_keep_their_recorded_slots_and_an_unlinked_side_stays_a_deal_with_an_unknown_endpoint() {
        let index: HashMap<u32, u32> = [(3, 0), (7, 1)].into_iter().collect();
        let none = TagHolderRow::NONE;
        let rows = [row(1, 2, 3, Recorded::Value(500_000)), row(1, 2, 7, Recorded::NotRecorded), row(none, 2, 3, Recorded::Value(1))];
        assert_eq!(edges(&rows, &index, None), [
            FlowEdge { from: Some(1), to: Some(2), part: 0, amount: Some(500_000) },
            FlowEdge { from: Some(1), to: Some(2), part: 1, amount: None },
            FlowEdge { from: None, to: Some(2), part: 0, amount: Some(1) },
        ]);
        assert_eq!(edges(&rows, &index, Some(7)).len(), 1);
    }

    #[test]
    fn resources_are_the_definitions_recorded_at_the_hour_and_an_unresolved_one_keeps_its_deal_its_id_and_why() {
        let definitions = [definition(3, 30), definition(7, 70), definition(9, 90)];
        let lexical = [word(30, "oil"), word(90, "steel")];
        let at = resources_at(Ok(&definitions), Some(&lexical), [3, 7, 12, 3]);
        assert_eq!(at.spelled, [(3, "oil".to_owned()), (9, "steel".to_owned())]);
        assert_eq!(at.unresolved.get(&7).map(String::as_str), Some("Resource 7 has no recorded spelling at this hour"));
        assert_eq!(at.unresolved.get(&12).map(String::as_str), Some("Resource 12 has no recorded definition at this hour"), "a mod's resource missing here is named by its id, never dropped");
        let (parts, index) = parts_at(&[info(3, "oil"), info(9, "steel")], &at.unresolved);
        assert_eq!(parts.iter().map(|part| (part.key.as_str(), part.ordinal, part.name_missing.is_some())).collect::<Vec<_>>(), [
            ("oil", 0, false),
            ("resource:7", 1, true),
            ("steel", 2, false),
            ("resource:12", 3, true),
        ]);
        let deals = edges(&[row(1, 2, 12, Recorded::Value(4))], &index, None);
        assert_eq!(deals, [FlowEdge { from: Some(1), to: Some(2), part: 3, amount: Some(4) }], "the deal stays, in its own part");
        let unread = resources_at(Err("not read".to_owned()), None, [3]);
        assert!(unread.unresolved[&3].contains("resource definitions were not recorded at this hour"));
    }
}
