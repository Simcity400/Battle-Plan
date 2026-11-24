use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::{LoadContext, Product};
use dashboard_base::{Source, products::at_hour::{At, AtHour, Keys, Owner, Read, ReadsAt}};
use dashboard_base::products::{
    resources::{RESOURCE_DECIMALS, ResourceInfo, ResourceNaming},
};
use dashboard_follow::live::{Cells, Source as Cell};
use dashboard_query::series::{Ask, MissingSpan};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct ResourceBalance;

pub struct ResourceContext {
    naming: ResourceNaming,
    deals: crate::products::convoys::ConvoyIndex,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceBalanceParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " The order `shown` lists the resources in; the game's when absent."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub order: Option<ResourceOrder>,
    #[doc = " Leave out of `shown` the resources with nothing in either bar."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub hide_empty: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum ResourceOrder {
    #[default]
    Game,
    Shortfall,
    Volume,
}

pub const POOL_CELLS: [(&str, &str); 8] = [
    ("produced", "produced"),
    ("subjects", "adjustment"),
    ("imported", "imported"),
    ("toExport", "to_export"),
    ("exported", "exported"),
    ("available", "used"),
    ("afterProjects", "special_total"),
    ("production", "production_demand"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum TradeDirection {
    Import,
    Export,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResourceDeal {
    #[doc = " The deal's native handle, as a decimal string."]
    pub deal: String,
    pub resource: u32,
    pub direction: TradeDirection,
    #[doc = " The other country's tag at the hour."]
    pub partner: String,
    #[doc = " Civilian factories the importer pays."]
    pub factories: i32,
    pub convoys_assigned: i32,
    pub convoys_required: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResourceBalanceReply {
    pub tag: String,
    pub decimals: u8,
    #[doc = " Every recorded resource, in the game's order."]
    pub resources: Vec<ResourceInfo>,
    #[doc = " The resources the country has a row of, in `resources` order: each a row of live cells"]
    #[doc = " (`POOL_CELLS`)."]
    pub pools: Vec<u32>,
    #[doc = " The resources the bars show, in the asked order (those with no row last, in the game's)."]
    pub shown: Vec<u32>,
    #[doc = " Why the pools are not known at the hour."]
    pub pools_missing: Option<String>,
    #[doc = " Its deals, largest delivery first."]
    pub deals: Vec<ResourceDeal>,
    #[doc = " Why the deals are not known at the hour: unknown, which is not none."]
    pub deals_missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ResourceBalanceReply, ResourceSizesReply);

fn range(id: u32) -> Keys {
    let owner = u128::from(id) << 32;
    Keys::Range(owner, owner | u128::from(u32::MAX))
}

fn deals_of(frame: &DecodedFrame, id: u32) -> io::Result<Vec<(&replay_data::rows::ResourceExchangeRow, u32, TradeDirection)>> {
    Ok(frame
        .complete::<tables::ResourceExchanges>()?
        .iter()
        .filter_map(|row| match (row.exporter == id, row.importer == id) {
            (true, false) => Some((row, row.importer, TradeDirection::Export)),
            (false, true) => Some((row, row.exporter, TradeDirection::Import)),
            _ => None,
        })
        .collect())
}

fn recorded_resources(frame: &DecodedFrame) -> io::Result<(Vec<(u32, String)>, usize)> {
    let definitions = frame.complete::<tables::ResourceDefinitions>()?;
    let tokens = frame.complete::<tables::LexicalTokens>()?;
    let mut out = Vec::with_capacity(definitions.len());
    let mut unnamed = 0;
    for definition in definitions {
        match tokens
            .binary_search_by_key(&definition.token, |row| row.token)
            .ok()
            .and_then(|at| std::str::from_utf8(&tokens[at].literal).ok())
            .filter(|token| !token.is_empty())
        {
            Some(token) => out.push((definition.id, token.to_owned())),
            None => unnamed += 1,
        }
    }
    Ok((out, unnamed))
}

struct Bars {
    supply: i128,
    demand: i128,
    net: i128,
}

impl Bars {
    fn of(row: &replay_data::rows::CountryResourceRow) -> Self {
        let (adjustment, after_projects) = (i128::from(row.adjustment), i128::from(row.special_total));
        let production = -i128::from(row.production_demand);
        Self {
            supply: i128::from(row.produced) + adjustment.max(0) + i128::from(row.imported),
            demand: i128::from(row.used) - after_projects + production + (-adjustment).max(0) + i128::from(row.to_export),
            net: after_projects - production,
        }
    }
}

fn shown_order(order_of: &BTreeMap<u32, usize>, bars: &BTreeMap<u32, Bars>, order: ResourceOrder, hide_empty: bool) -> Vec<u32> {
    let mut ids: Vec<u32> = order_of
        .keys()
        .copied()
        .filter(|id| !hide_empty || bars.get(id).is_some_and(|bars| bars.supply != 0 || bars.demand != 0))
        .collect();
    ids.sort_by_key(|id| {
        let key = bars.get(id).map(|bars| match order {
            ResourceOrder::Game => 0,
            ResourceOrder::Shortfall => bars.net,
            ResourceOrder::Volume => -bars.supply.max(bars.demand),
        });
        (key.is_none(), key.unwrap_or(0), order_of[id])
    });
    ids
}

fn reply(frame: &DecodedFrame, params: &ResourceBalanceParams, tag: &str, id: u32, naming: &ResourceNaming, live: &Cells, cancelled: &AtomicBool) -> io::Result<ResourceBalanceReply> {
    let mut warnings = Vec::new();
    let (recorded, unnamed) = match recorded_resources(frame) {
        Ok(found) => found,
        Err(error) => {
            warnings.push(format!("The recorded resources cannot be read at this hour: {error}"));
            (Vec::new(), 0)
        }
    };
    if unnamed > 0 {
        warnings.push(format!("{unnamed} recorded resources name a token the recorded dictionary does not spell; they are left out."));
    }
    let (resources, named) = naming.infos(&recorded, cancelled)?;
    warnings.extend(named);
    let order: BTreeMap<u32, usize> = resources.iter().enumerate().map(|(at, info)| (info.id, at)).collect();
    let (mut bars, mut pools_missing) = (BTreeMap::new(), None);
    match frame.complete::<tables::CountryResources>() {
        Ok(rows) => {
            let mut columns = [0; POOL_CELLS.len()];
            for (column, (_, field)) in columns.iter_mut().zip(POOL_CELLS) {
                *column = dashboard_declare::field(TableId::CountryResources, field)?;
            }
            let mut left_out = 0;
            for row in rows.iter().filter(|row| row.country == id) {
                if !order.contains_key(&row.resource) {
                    left_out += 1;
                    continue;
                }
                let (cell, key) = (row.resource.to_string(), u128::from(id) << 32 | u128::from(row.resource));
                for ((name, _), column) in POOL_CELLS.iter().zip(columns) {
                    live.value(&cell, name, Cell { table: TableId::CountryResources, column, key }, 1.0);
                }
                bars.insert(row.resource, Bars::of(row));
            }
            if left_out > 0 {
                warnings.push(format!("{left_out} recorded rows name a resource the recorded definitions do not; they are left out."));
            }
        }
        Err(error) => pools_missing = Some(format!("Resources are not recorded at this hour: {error}")),
    }
    let mut pools: Vec<u32> = bars.keys().copied().collect();
    pools.sort_by_key(|id| order[id]);
    let shown = shown_order(&order, &bars, params.order.unwrap_or_default(), params.hide_empty.unwrap_or(false));
    let (mut deals, mut deals_missing) = (Vec::new(), None);
    let delivered = dashboard_declare::field(TableId::ResourceExchanges, "delivered")?;
    match deals_of(frame, id) {
        Ok(found) => {
            let countries = frame.complete::<tables::Countries>();
            let mut unresolved = 0;
            for (row, partner, direction) in found {
                let tag = countries.as_ref().ok().and_then(|rows| rows.binary_search_by_key(&partner, |country| country.id).ok().map(|at| &rows[at]));
                let Some(partner) = tag.and_then(|country| std::str::from_utf8(&country.tag).ok()).filter(|tag| !tag.is_empty()) else {
                    unresolved += 1;
                    continue;
                };
                if !order.contains_key(&row.resource) {
                    continue;
                }
                let deal = ResourceDeal {
                    deal: row.id.to_string(),
                    resource: row.resource,
                    direction,
                    partner: partner.to_owned(),
                    factories: row.assigned_factories,
                    convoys_assigned: row.convoys_assigned,
                    convoys_required: row.convoys_required,
                };
                live.value(&deal.deal, "delivered", Cell { table: TableId::ResourceExchanges, column: delivered, key: u128::from(row.id) }, 1.0);
                deals.push((row.delivered, deal));
            }
            deals.sort_by(|(a_raw, a), (b_raw, b)| b_raw.cmp(a_raw).then_with(|| a.partner.cmp(&b.partner)).then_with(|| a.deal.cmp(&b.deal)));
            if unresolved > 0 {
                warnings.push(format!("{unresolved} recorded trade rows name another country the recorded countries do not; they are left out."));
            }
        }
        Err(error) => deals_missing = Some(format!("Trade deals are not recorded at this hour: {error}")),
    }
    if pools_missing.is_none() && pools.is_empty() {
        warnings.push(format!("{tag}: no recorded resources at this hour"));
    }
    let deals = deals.into_iter().map(|(_, deal)| deal).collect();
    Ok(ResourceBalanceReply { tag: tag.to_owned(), decimals: RESOURCE_DECIMALS, resources, pools, shown, pools_missing, deals, deals_missing, warnings })
}

impl AtHour for ResourceBalance {
    const KEY: &'static str = "resource-balance";
    const TITLE: &'static str = "Resource balance";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::EveryHour("stage-0 reads_in queries ConvoyIndex::deals_at(owner, hour) without retained_reads expiry");
    type Params = ResourceBalanceParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, order, hideEmpty];
    type Reply = ResourceBalanceReply;
    type Context = ResourceContext;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            ResourceBalanceParams::decl(),
            ResourceBalanceReply::decl(),
            ResourceInfo::decl(),
            ResourceOrder::decl(),
            ResourceDeal::decl(),
            TradeDirection::decl(),
        ]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        vec![TableId::ResourceDefinitions, TableId::LexicalTokens, TableId::CountryResources, TableId::ResourceExchanges, TableId::Countries]
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        match step {
            0 => vec![
                Read::all(TableId::ResourceDefinitions),
                Read::all(TableId::ResourceExchanges),
                Read { table: TableId::CountryResources, keys: range(owner.id) },
            ],
            1 => {
                let tokens: BTreeSet<u32> = held.complete::<tables::ResourceDefinitions>().map(|rows| rows.iter().map(|row| row.token).collect()).unwrap_or_default();
                let partners: BTreeSet<u32> = deals_of(held, owner.id).map(|deals| deals.into_iter().map(|(_, partner, _)| partner).collect()).unwrap_or_default();
                vec![
                    Read::these(TableId::LexicalTokens, tokens.into_iter().map(u128::from)),
                    Read::these(TableId::Countries, partners.into_iter().map(u128::from)),
                ]
            }
            _ => Vec::new(),
        }
    }

    fn reads_in(params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        let (0, Some(owner)) = (step, owner) else { return Self::reads(params, owner, step, held) };
        let deals = match at.context.deals.run(at.run) {
            Ok(index) => Read::these(TableId::ResourceExchanges, index.deals_at(owner.id, at.hour)),
            Err(_) => Read::all(TableId::ResourceExchanges),
        };
        vec![Read::all(TableId::ResourceDefinitions), deals, Read { table: TableId::CountryResources, keys: range(owner.id) }]
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(ResourceContext { naming: ResourceNaming::stored(source, load), deals: crate::products::convoys::ConvoyIndex::of(source) })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Resource balance country is unavailable at this hour"))?;
        let reply = reply(frame, params, &at.holders.name(&params.country, at.hour), owner.id, &at.context.naming, at.live, at.cancelled)?;
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

pub struct ResourceSizes;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResourceSizesParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " The span; the whole timeline when absent."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from: Option<i32>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub to: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResourceSize {
    #[doc = " The resource's native index."]
    pub id: u32,
    #[doc = " Its longest bar, fixed5; null where no hour of it is recorded."]
    pub longest_raw: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ResourceSizesReply {
    pub tag: String,
    pub sizes: Vec<ResourceSize>,
    #[doc = " The span's hours no holder of the tag, or no row, is known, with why."]
    pub missing: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

impl Product for ResourceSizes {
    const KEY: &'static str = "resource-sizes";
    const CONTENT_DEPENDENT: bool = false;
    type Params = ResourceSizesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, from: Start, to: End];
    type Reply = ResourceSizesReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ResourceSizesParams::decl(), ResourceSizesReply::decl(), ResourceSize::decl(), MissingSpan::decl()]
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))?)?;
        let series = dashboard_declare::series("resource-balance").ok_or_else(|| dashboard_core::invalid("Resource balance is not declared"))?;
        let measure = series.measure("barLength").ok_or_else(|| dashboard_core::invalid("Resource balance has no bar length"))?;
        let parts = series.parts.ok_or_else(|| dashboard_core::invalid("Resource balance has no parts"))?;
        let from = params.from.unwrap_or(query.timeline.first).max(query.timeline.first);
        let to = params.to.unwrap_or(query.timeline.last).min(query.timeline.last);
        if from > to {
            return Err(dashboard_core::invalid("Invalid span"));
        }
        let line = dashboard_query::series::Line::new(&query, series, measure)?;
        let Some(last) = query.timeline.stretches.last() else {
            return Ok(ResourceSizesReply { tag: query.holders.name(&params.country, to), sizes: Vec::new(), missing: Vec::new(), warnings: Vec::new() });
        };
        let read = dashboard_query::cost::storage(|| {
            dashboard_query::read::at(query.replay.recording(), last.run, &[dashboard_query::read::Read { table: TableId::ResourceDefinitions, columns: None, keys: dashboard_query::read::Keys::All }], last.last)
        })?;
        dashboard_text::metadata::check_cancel(cancelled)?;
        let asks: Vec<(Ask, Vec<(i32, i32)>)> = read[0].rows.iter().map(|&(key, _)| (Ask { country: params.country.clone(), part: parts.key(0, key) }, vec![(from, to)])).collect();
        let scale = 10f64.powi(i32::from(measure.decimals));
        let drawn = line.summaries_within(&asks, from, to, 1)?;
        let missing = asks.first().map(|(ask, _)| line.unheld(&ask.country, from, to, &line.held(ask, from, to))).unwrap_or_default();
        let sizes = drawn
            .into_iter()
            .zip(&asks)
            .map(|((summary, _), (ask, _))| ResourceSize {
                id: parts.id(ask.part).and_then(|id| u32::try_from(id).ok()).unwrap_or_default(),
                longest_raw: summary
                    .buckets
                    .iter()
                    .flatten()
                    .map(|bucket| bucket.max_value)
                    .reduce(f64::max)
                    .map(|most| format!("{:.0}", most * scale)),
            })
            .collect();
        Ok(ResourceSizesReply { tag: query.holders.name(&params.country, to), sizes, missing, warnings: Vec::new() })
    }
}
