use dashboard_base::products::{
    Country, LoadContext,
    at_hour::{At, AtHour, Edits, Held, Owner, Read, ReadsAt, before, field_columns, warnings_through},
    flows::{CountryFlowReply, FlowEdge, FlowEnd, FlowEndpoint, FlowGap, FlowPart, FlowShape, build_flows},
    resources::{RESOURCE_DECIMALS, ResourceInfo, ResourceNaming},
    validate_country,
};
use dashboard_base::Source;
use dashboard_follow::live::Live;
use dashboard_query::coverage::{Witness, witnessed, witnessed_rows};
use dashboard_query::identity::Holders;
use dashboard_query::series::ranked::GroupSelection;
use dashboard_world::products::group_members::GroupsAt;
use replay_data::rows::{Recorded, ResourceDefinitionRow, RowCodec, TableId, TableRows, TagHolderRow, TradeFlowRow, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
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
    #[doc = " An explicit top partners per side: exactly this many named and the rest folded, where the order is proven at the hour."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub side_totals_only: Option<bool>,
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

pub fn resources_at(definitions: &[ResourceDefinitionRow], definitions_missing: Option<&str>, spell: impl Fn(u32) -> Result<Option<String>, String>, dealt: impl IntoIterator<Item = u32>) -> Resources {
    let mut out = Resources { spelled: Vec::new(), unresolved: BTreeMap::new() };
    let mut unspelled: HashMap<u32, String> = HashMap::new();
    let defined: HashMap<u32, u32> = definitions.iter().map(|row| (row.id, row.token)).collect();
    for (&id, &token) in &defined {
        match spell(token) {
            Ok(Some(text)) => out.spelled.push((id, text)),
            Ok(None) => {}
            Err(reason) => {
                unspelled.insert(id, reason);
            }
        }
    }
    out.spelled.sort_unstable();
    for id in dealt {
        if out.unresolved.contains_key(&id) || out.spelled.binary_search_by_key(&id, |(held, _)| *held).is_ok() {
            continue;
        }
        let why = match (defined.contains_key(&id), definitions_missing, unspelled.get(&id)) {
            (true, _, Some(reason)) => format!("Resource {id}: its spelling was not recorded at this hour ({reason})"),
            (true, ..) => format!("Resource {id} has no recorded spelling at this hour"),
            (false, Some(reason), _) => format!("Resource {id}: resource definitions were not recorded at this hour ({reason})"),
            (false, None, _) => format!("Resource {id} has no recorded definition at this hour"),
        };
        out.unresolved.insert(id, why);
    }
    out
}

pub fn spelling(frame: &DecodedFrame, token: u32) -> Result<Option<String>, String> {
    match witnessed::<tables::LexicalTokens>(frame, u128::from(token)) {
        Witness::Row(row) => Ok(std::str::from_utf8(&row.literal).ok().map(str::to_owned)),
        Witness::Absent => Ok(None),
        Witness::NotRecorded(reason) => Err(reason),
    }
}

pub fn witnessed_deals(frame: &DecodedFrame) -> Result<(&[TradeFlowRow], Vec<FlowGap>), String> {
    let read = witnessed_rows::<tables::TradeFlows>(frame);
    match read.missing {
        Some(reason) if read.rows.is_empty() => Err(reason),
        Some(reason) => Ok((read.rows, vec![FlowGap::partial(reason)])),
        None => Ok((read.rows, Vec::new())),
    }
}

fn side(slot: u32, holder: u32) -> FlowEnd {
    match (slot != TagHolderRow::NONE, holder != TagHolderRow::NONE) {
        (true, _) => FlowEnd::Slot(slot),
        (false, true) => FlowEnd::Raw(holder),
        (false, false) => FlowEnd::Collective,
    }
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
    let parts = parts.into_iter().map(|(id, part)| FlowPart { ordinal: id, ..part }).collect();
    (parts, index)
}

pub fn edges(rows: &[TradeFlowRow], index: &HashMap<u32, u32>, resource: Option<u32>) -> Vec<FlowEdge> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if resource.is_some_and(|only| only != row.resource) {
            continue;
        }
        let Some(&part) = index.get(&row.resource) else { continue };
        let amount = match row.delivered {
            Recorded::Value(value) => Some(i128::from(value)),
            Recorded::NotRecorded => None,
        };
        out.push(FlowEdge { from: side(row.seller_slot, row.seller), to: side(row.buyer_slot, row.buyer), part, amount });
    }
    out
}

fn trade_unavailable(params: &TradeFlowParams, holders: &Holders, groups: &GroupsAt, hour: i32) -> io::Result<bool> {
    if params.subject.as_ref().is_some_and(|country| holders.slot_at(country, hour).is_none()) {
        return Ok(true);
    }
    Ok(groups.chosen_slots(params.group.as_ref(), &params.countries, holders, hour)?.is_err())
}

fn trade_reaches(from: FlowEnd, to: FlowEnd, subject: Option<u32>, chosen: Option<&HashSet<u32>>, between: bool) -> bool {
    if from.slot().is_some() && from == to {
        return false;
    }
    if let Some(subject) = subject {
        return from == FlowEnd::Slot(subject)
            || to == FlowEnd::Slot(subject)
            || from.slot().is_none()
            || to.slot().is_none();
    }
    let Some(chosen) = chosen else {
        return true;
    };
    let ends = [from.slot(), to.slot()];
    let unknown = ends.iter().any(Option::is_none);
    let mut known = ends.iter().flatten();
    let decided = if between { known.any(|slot| !chosen.contains(slot)) } else { known.any(|slot| chosen.contains(slot)) };
    !matches!((decided, between, unknown), (true, true, _) | (false, false, false))
}

impl AtHour for TradeFlow {
    const KEY: &'static str = "trade-flow";
    const TITLE: &'static str = "Trade flows";
    const CONTENT_DEPENDENT: bool = true;
    type Params = TradeFlowParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, subject, countries, "countries[]", between, group, limit, top, sideTotalsOnly, resource, table];
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
        params.subject.as_ref().map_or(Ok(()), validate_country)?;
        dashboard_base::products::flows::validate_chosen(params.subject.as_ref(), &params.countries, params.group.is_some())
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::TradeFlows, TableId::ResourceDefinitions, TableId::LexicalTokens]
    }

    fn reads(_params: &Self::Params, _owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match step {
            0 => [TableId::TradeFlows, TableId::ResourceDefinitions].into_iter().map(Read::all).collect(),
            1 => {
                let tokens = witnessed_rows::<tables::ResourceDefinitions>(held).rows.iter().map(|row| u128::from(row.token)).collect::<Vec<_>>();
                vec![Read::these(TableId::LexicalTokens, tokens)]
            }
            _ => Vec::new(),
        }
    }

    fn reads_in(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        if trade_unavailable(params, at.holders, &at.context.groups, at.hour).unwrap_or(false) {
            return Vec::new();
        }
        Self::reads(params, owner, step, held)
    }

    fn retained_reads(params: &Self::Params, owner: Option<Owner>, stage: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> (Vec<Read>, i32) {
        let through = if stage == 0 {
            before(at.holders.next_change(at.hour)).min(at.context.groups.valid_through(params.group.as_ref(), at.hour))
        } else {
            i32::MAX
        };
        (Self::reads_in(params, owner, stage, held, at), through)
    }

    fn columns(table: TableId) -> Option<Vec<usize>> {
        match table {
            TableId::TradeFlows => Some(field_columns(table, &["delivered"])),
            TableId::ResourceDefinitions => Some(field_columns(table, &["token"])),
            _ => None,
        }
    }

    fn plan_changes(stage: usize, edits: &Edits, _held: &DecodedFrame, _at: &ReadsAt<'_, Self::Context>) -> bool {
        stage == 1 && edits.fields(TableId::ResourceDefinitions, u64::MAX)
    }

    fn compares_previous(table: TableId) -> bool {
        table == TableId::TradeFlows
    }

    fn context(_source: &Source, _load: &LoadContext) -> io::Result<Self::Context> {
        Ok(TradeContext { naming: None, groups: GroupsAt::default() })
    }

    fn context_for(source: &Source, load: &LoadContext, params: &Self::Params) -> io::Result<Self::Context> {
        let groups = GroupsAt::load(source, load, &params.branch_id, params.group.as_ref())?;
        Ok(TradeContext { naming: Some(Arc::new(ResourceNaming::stored(source, load))), groups })
    }

    fn valid_through(_frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> i32 {
        before(at.holders.next_change(at.hour)).min(at.context.groups.valid_through(params.group.as_ref(), at.hour))
    }

    fn valid_reply_through(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>, reply: &Self::Reply, partial: bool) -> i32 {
        if partial {
            return frame.hour;
        }
        Self::valid_through(frame, params, at).min(warnings_through(
            frame.hour,
            reply.missing.iter()
                .chain(&reply.warnings)
                .map(String::as_str)
                .chain(reply.parts.iter().filter_map(|part| part.name_missing.as_deref())),
        ))
    }

    fn advance(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>, edits: &Edits, _live: &Live, _data: &mut Held<'_>) -> io::Result<bool> {
        if !edits.replaced.is_empty() || edits.tables.keys().any(|&table| table != TableId::TradeFlows) {
            return Ok(false);
        }
        let Some(keys) = edits.tables.get(&TableId::TradeFlows) else {
            return Ok(true);
        };
        let subject = params.subject.as_ref().map(|country| at.holders.slot_at(country, at.hour));
        if let Some(None) = subject {
            return Ok(true);
        }
        let Ok(chosen) = at.context.groups.chosen_slots(params.group.as_ref(), &params.countries, at.holders, at.hour)? else {
            return Ok(true);
        };
        let Some(prior) = edits.previous.get(&TableId::TradeFlows).and_then(|view| match view.held() {
            TableView::Complete(TableRows::TradeFlows(rows)) => Some(rows.as_slice()),
            _ => None,
        }) else {
            return Ok(false);
        };
        let now = frame.complete::<tables::TradeFlows>()?;
        let definitions = witnessed_rows::<tables::ResourceDefinitions>(frame).rows;
        for &key in keys.keys() {
            let old = prior.binary_search_by_key(&key, RowCodec::key).ok().map(|i| &prior[i]);
            let new = now.binary_search_by_key(&key, RowCodec::key).ok().map(|i| &now[i]);
            if old.map(|row| row.delivered) == new.map(|row| row.delivered) && old.is_some() == new.is_some() {
                continue;
            }
            let Some(row) = new.or(old) else {
                continue;
            };
            if old.is_some() != new.is_some() {
                let spelled = definitions
                    .binary_search_by_key(&row.resource, |def| def.id)
                    .ok()
                    .is_some_and(|i| matches!(spelling(frame, definitions[i].token), Ok(Some(_))));
                if !spelled {
                    return Ok(false);
                }
            }
            if params.resource.is_none_or(|only| only == row.resource)
                && trade_reaches(side(row.seller_slot, row.seller), side(row.buyer_slot, row.buyer), subject.flatten(), chosen.as_ref(), params.between)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let mut warnings = Vec::new();
        let resolve = |country: &Country| at.holders.slot_at(country, at.hour);
        let subject = params.subject.as_ref().map(resolve);
        if let Some(None) = subject {
            return Ok(CountryFlowReply::not_recorded(params.subject.as_ref().and_then(|country| at.holders.not_known(country, at.hour)).unwrap_or_else(|| "The chosen country holds no tag at this hour".to_owned()), RESOURCE_DECIMALS, warnings));
        }
        let chosen = match at.context.groups.chosen_slots(params.group.as_ref(), &params.countries, at.holders, at.hour)? {
            Ok(chosen) => chosen,
            Err(refused) => return Ok(CountryFlowReply::not_recorded(refused, RESOURCE_DECIMALS, warnings)),
        };
        let (rows, gaps) = match witnessed_deals(frame) {
            Ok(read) => read,
            Err(reason) => return Ok(CountryFlowReply::not_recorded(reason, RESOURCE_DECIMALS, warnings)),
        };
        let definitions = witnessed_rows::<tables::ResourceDefinitions>(frame);
        let resources = resources_at(definitions.rows, definitions.missing.as_deref(), |token| spelling(frame, token), rows.iter().map(|row| row.resource));
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
        let shape = FlowShape {
            subject: subject.flatten(),
            chosen: chosen.as_ref(),
            between: params.between,
            limit: params.limit.map(|limit| limit.max(1) as usize),
            top: params.top.map(|top| top.max(1) as usize),
            side_totals_only: params.side_totals_only.unwrap_or_default(),
            table: params.table,
            not_recorded: NOT_RECORDED,
            unlinked: UNLINKED,
        };
        let mut reply = build_flows(&edges, &gaps, parts, RESOURCE_DECIMALS, &shape, &|end| FlowEndpoint::at(at.holders, end, at.hour, UNLINKED));
        reply.warnings.append(&mut warnings);
        Ok(reply)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::products::flows::FlowAmount;
    use replay_data::rows::{LexicalTokenRow, TableRows};
    use replay_frame::frame::TableView;

    fn row(seller: u32, buyer: u32, resource: u32, delivered: Recorded<i64>) -> TradeFlowRow {
        let holder = |slot: u32| if slot == TagHolderRow::NONE { slot } else { slot * 10 };
        TradeFlowRow { seller_slot: seller, seller: holder(seller), buyer_slot: buyer, buyer: holder(buyer), resource, delivered, ..TradeFlowRow::default() }
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
            FlowEdge { from: FlowEnd::Slot(1), to: FlowEnd::Slot(2), part: 0, amount: Some(500_000) },
            FlowEdge { from: FlowEnd::Slot(1), to: FlowEnd::Slot(2), part: 1, amount: None },
            FlowEdge { from: FlowEnd::Collective, to: FlowEnd::Slot(2), part: 0, amount: Some(1) },
        ], "the writer keys every unidentified seller of a buyer and resource as one row: a combined amount");
        assert_eq!(edges(&rows, &index, Some(7)).len(), 1);
    }

    #[test]
    fn resources_are_the_definitions_recorded_at_the_hour_and_an_unresolved_one_keeps_its_deal_its_id_and_why() {
        let definitions = [definition(3, 30), definition(7, 70), definition(9, 90)];
        let lexical = [word(30, "oil"), word(90, "steel")];
        let spell = |token: u32| Ok(lexical.iter().find(|row| row.token == token).map(|row| String::from_utf8(row.literal.clone()).unwrap()));
        let at = resources_at(&definitions, None, spell, [3, 7, 12, 3]);
        assert_eq!(at.spelled, [(3, "oil".to_owned()), (9, "steel".to_owned())]);
        assert_eq!(at.unresolved.get(&7).map(String::as_str), Some("Resource 7 has no recorded spelling at this hour"));
        assert_eq!(at.unresolved.get(&12).map(String::as_str), Some("Resource 12 has no recorded definition at this hour"), "a mod's resource missing here is named by its id, never dropped");
        let (parts, index) = parts_at(&[info(3, "oil"), info(9, "steel")], &at.unresolved);
        assert_eq!(parts.iter().map(|part| (part.key.as_str(), part.ordinal, part.name_missing.is_some())).collect::<Vec<_>>(), [
            ("oil", 3, false),
            ("resource:7", 7, true),
            ("steel", 9, false),
            ("resource:12", 12, true),
        ], "a part's colour follows its recorded id: an undefined resource between two named ones moves neither");
        let (without, _) = parts_at(&[info(3, "oil"), info(9, "steel")], &BTreeMap::new());
        assert_eq!(without.iter().map(|part| part.ordinal).collect::<Vec<_>>(), [3, 9]);
        let deals = edges(&[row(1, 2, 12, Recorded::Value(4))], &index, None);
        assert_eq!(deals, [FlowEdge { from: FlowEnd::Slot(1), to: FlowEnd::Slot(2), part: 3, amount: Some(4) }], "the deal stays, in its own part");
        let unread = resources_at(&[], Some("not read"), spell, [3]);
        assert!(unread.unresolved[&3].contains("resource definitions were not recorded at this hour"));
        let partly = resources_at(&definitions[..1], Some("read only in part"), |_| Err("lexical tokens read only in part".to_owned()), [3, 9]);
        assert!(partly.unresolved[&3].contains("its spelling was not recorded at this hour (lexical tokens read only in part)"), "a witnessed definition keeps its own reason");
        assert!(partly.unresolved[&9].contains("resource definitions were not recorded at this hour (read only in part)"));
    }

    fn frame(view: TableView) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            tables: std::array::from_fn(|_| TableView::Unavailable { reason: "outside the test".into() }.into()),
            metrics: replay_frame::frame::Metrics { total_ns: 0, table_ns: [0; replay_data::rows::TABLE_COUNT], pool_bytes: 0, dropped_frames: 0 },
        };
        *frame.tables[TableId::TradeFlows.index()] = view;
        frame
    }

    #[test]
    fn a_truncated_deal_table_keeps_the_deals_it_witnessed_and_withholds_their_amounts() {
        let witnessed = vec![row(1, 2, 3, Recorded::Value(500_000)), row(2, 4, 3, Recorded::Value(200_000))];
        let truncated = frame(TableView::Truncated { rows: TableRows::TradeFlows(witnessed.clone()), missing: 3 });
        let (rows, gaps) = witnessed_deals(&truncated).unwrap();
        assert_eq!(rows.len(), 2);
        let index: HashMap<u32, u32> = [(3, 0)].into_iter().collect();
        let shape = FlowShape { subject: Some(2), chosen: None, between: false, limit: None, top: None, side_totals_only: false, table: false, not_recorded: NOT_RECORDED, unlinked: UNLINKED };
        let reply = build_flows(&edges(rows, &index, None), &gaps, vec![FlowPart { key: "oil".into(), name: None, name_missing: None, ordinal: 0, icon: None }], 0, &shape, &|end| FlowEndpoint::unidentified(end, String::new()));
        let partners = |side: &dashboard_base::products::flows::FlowSide, from: bool| -> Vec<Option<u32>> {
            side.links.iter().map(|link| if from { link.from } else { link.to }.and_then(|at| reply.endpoints[at as usize].slot)).collect()
        };
        assert_eq!((partners(&reply.incoming, true), partners(&reply.outgoing, false)), (vec![Some(1)], vec![Some(4)]), "both witnessed deals are kept");
        assert_eq!(reply.incoming.links[0].amounts, [FlowAmount { part: 0, amount: None }], "a pair's sum over a partial read may miss its other rows");
        assert!(reply.incoming.links[0].missing.as_deref().is_some_and(|why| why.ends_with("read only in part at this hour (3 rows not read)")));
        assert!(reply.incoming.total.is_none() && reply.outgoing.total.is_none());
        let whole = frame(TableView::Complete(TableRows::TradeFlows(witnessed)));
        assert!(witnessed_deals(&whole).unwrap().1.is_empty(), "a whole read has no gap");
        assert!(witnessed_deals(&frame(TableView::Unavailable { reason: "reader fault".into() })).is_err(), "nothing witnessed is not recorded, never an empty trade");
    }
}
