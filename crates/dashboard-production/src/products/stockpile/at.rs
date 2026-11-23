
use dashboard_base::products::{Country, validate_country};
use super::{Content, DECIMALS, NAME_TABLES, NEED_BOUND_RAW, StockType, Types, named_in};
use dashboard_base::{Source, products::{LoadContext, at_hour::{At, AtHour, Owner, Read}}};
use crate::products::production::LineKind;
use dashboard_follow::live::Source as Cell;
use replay_data::{
    cadence::STAGGER_PERIOD,
    rows::{TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StockpileAtParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum StockBalance {
    Player,
    Ai,
    Unknown,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockAtVariant {
    pub id: String,
    #[doc = " Its equipment token; empty where it has no definition at the hour."]
    pub token: String,
    pub archetype: Option<String>,
    pub name: Option<String>,
    #[doc = " False where it has no definition at the hour."]
    pub known: bool,
    pub design: Option<String>,
    #[doc = " It has a stock row (its `stock` cell); a design not held holds none."]
    pub held: bool,
    #[doc = " The military lines making it (each its `l<line>` cells)."]
    pub lines: Vec<String>,
    #[doc = " Why its output a day is not known whatever its lines' cells say; null where they decide."]
    pub output_missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockAtLogistics {
    #[doc = " The archetype key, or the unresolved mark and the recorded token's spelling."]
    pub key: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockpileAtReply {
    pub tag: String,
    pub campaign_key: String,
    pub decimals: u8,
    #[doc = " Every archetype a listed design or logistics row belongs to, in the equipment database's order."]
    pub types: Vec<StockType>,
    pub variants: Vec<StockAtVariant>,
    pub logistics: Vec<StockAtLogistics>,
    #[doc = " Why the stock is not known at the hour; null where it is."]
    pub stock_missing: Option<String>,
    #[doc = " Why the production lines are not known at the hour; null where they are."]
    pub lines_missing: Option<String>,
    pub balance: StockBalance,
    pub balance_missing: Option<String>,
    pub need_bound_raw: String,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(StockpileAtReply);

pub struct AtContext {
    content: Content,
    campaign_key: String,
}

fn stale(frame: &DecodedFrame, table: TableId, what: &str) -> Option<String> {
    let read = frame.tables[table.index()].sampled_at()?;
    (i64::from(frame.hour) - i64::from(read) >= i64::from(STAGGER_PERIOD)).then(|| format!("The {what} was not read within a day of this hour."))
}

fn owner_range(slot: u32) -> (u128, u128) {
    (u128::from(slot) << 64, u128::from(slot) << 64 | u128::from(u64::MAX))
}

fn raw_range(id: u32) -> (u128, u128) {
    (u128::from(id) << 32, u128::from(id) << 32 | u128::from(u32::MAX))
}

fn column(table: TableId, name: &str) -> io::Result<usize> {
    dashboard_declare::field(table, name)
}

fn raws(frame: &DecodedFrame, slot: u32) -> BTreeSet<u32> {
    frame.complete::<tables::CountryLinks>().map(|links| links.iter().filter(|link| link.canonical_slot == slot).map(|link| link.id).collect()).unwrap_or_default()
}

fn designs(frame: &DecodedFrame, owner: Owner) -> BTreeSet<u64> {
    let mut ids: BTreeSet<u64> = frame.complete::<tables::StockByVariant>().map(|rows| rows.iter().filter(|row| row.slot == owner.slot).map(|row| row.variant).collect()).unwrap_or_default();
    let raws = raws(frame, owner.slot);
    if let Ok(lines) = frame.complete::<tables::ProductionLines>() {
        ids.extend(lines.iter().filter(|line| raws.contains(&line.country)).map(|line| line.variant));
    }
    ids
}

pub(super) fn reply(frame: &DecodedFrame, params: &StockpileAtParams, owner: Owner, at: &At<'_, AtContext>) -> io::Result<StockpileAtReply> {
    let context = at.context;
    let content = &context.content;
    let mut warnings = content.warnings.clone();
    let raws = raws(frame, owner.slot);
    let stock_missing = match frame.complete::<tables::StockByVariant>() {
        Err(error) => Some(format!("The stockpile is not known at this hour: {error}")),
        Ok(_) => stale(frame, TableId::Stockpiles, "stockpile"),
    };
    let stock_rows: Vec<_> = frame.complete::<tables::StockByVariant>().map(|rows| rows.iter().filter(|row| row.slot == owner.slot).cloned().collect()).unwrap_or_default();
    let (lines_missing, lines) = match frame.complete::<tables::ProductionLines>() {
        Err(error) => (Some(format!("Production lines are not known at this hour: {error}")), Vec::new()),
        Ok(rows) => (stale(frame, TableId::ProductionLines, "production lines"), rows.iter().filter(|line| raws.contains(&line.country)).cloned().collect()),
    };
    let (balance, balance_missing, logistics_rows) = match frame.complete::<tables::LogisticsEquipment>() {
        Err(error) => (StockBalance::Unknown, Some(format!("The logistics record is not known at this hour: {error}")), Vec::new()),
        Ok(rows) => match stale(frame, TableId::LogisticsEquipment, "logistics record") {
            Some(why) => (StockBalance::Unknown, Some(why), Vec::new()),
            None => {
                let held: Vec<_> = rows.iter().filter(|row| row.country == owner.id).cloned().collect();
                (if held.is_empty() { StockBalance::Ai } else { StockBalance::Player }, None, held)
            }
        },
    };
    let mut ids: BTreeSet<u64> = stock_rows.iter().map(|row| row.variant).collect();
    ids.extend(lines.iter().map(|line| line.variant));
    let ids: Vec<u64> = ids.into_iter().collect();
    let tokens: BTreeSet<u32> = logistics_rows.iter().map(|row| row.token).collect();
    let mut refused = BTreeMap::new();
    let (mut named, spelled) = named_in(frame, &ids, &tokens, content, &mut refused);
    let (amount, speed, cost) = (column(TableId::StockByVariant, "amount")?, column(TableId::ProductionLines, "speed_raw")?, column(TableId::ProductionLines, "cost_raw")?);
    let mut types = Types::default();
    let held: BTreeSet<u64> = stock_rows.iter().map(|row| row.variant).collect();
    let (mut costless, mut other) = (0, 0);
    let mut variants = Vec::with_capacity(ids.len());
    let mut unnamed = 0;
    for id in ids {
        let name = named.remove(&id).filter(|name| name.known);
        unnamed += usize::from(name.is_none());
        let token = name.as_ref().map(|name| name.token.clone()).unwrap_or_default();
        let mut made = Vec::new();
        let mut output_missing = None;
        for line in lines.iter().filter(|line| line.variant == id) {
            if LineKind::of(i64::from(line.kind)) == Some(LineKind::Military) {
                let row = format!("l{}", line.handle);
                at.live.value(&row, "speed", Cell { table: TableId::ProductionLines, column: speed, key: u128::from(line.handle) }, 1.0);
                at.live.value(&row, "cost", Cell { table: TableId::ProductionLines, column: cost, key: u128::from(line.handle) }, 1.0);
                costless += usize::from(line.cost_raw <= 0);
                made.push(line.handle.to_string());
            } else {
                other += 1;
                output_missing = Some("A naval, refit or railway-gun line makes it: its daily output is not established.".to_owned());
            }
        }
        if held.contains(&id) {
            at.live.value(&format!("v{id}"), "stock", Cell { table: TableId::StockByVariant, column: amount, key: u128::from(owner.slot) << 64 | u128::from(id) }, 1.0);
        }
        variants.push(StockAtVariant {
            id: id.to_string(),
            archetype: types.variant(content, &token),
            token,
            known: name.is_some(),
            name: name.as_ref().and_then(|name| name.name.clone()),
            design: name.and_then(|name| name.design),
            held: held.contains(&id),
            lines: made,
            output_missing,
        });
    }
    let mut logistics: BTreeSet<String> = BTreeSet::new();
    let columns = [("balance", column(TableId::LogisticsEquipment, "balance_raw")?), ("stock", column(TableId::LogisticsEquipment, "stock_raw")?), ("available", column(TableId::LogisticsEquipment, "available_raw")?)];
    for row in &logistics_rows {
        let spelling = spelled.get(&row.token).cloned().unwrap_or_else(|| format!("token {}", row.token));
        let key = types.logistics(content, &spelling);
        let id = format!("t{key}");
        for (name, column) in columns {
            at.live.value(&id, name, Cell { table: TableId::LogisticsEquipment, column, key: u128::from(row.country) << 32 | u128::from(row.archetype) }, 1.0);
        }
        logistics.insert(key);
    }
    if unnamed > 0 {
        warnings.push(format!("{unnamed} designs in stock or production have no recorded definition by this hour; their type is unknown."));
    }
    for (why, count) in refused {
        warnings.push(format!("{count} equipment designs with unknown text or design operands: {why}"));
    }
    if costless > 0 {
        warnings.push(format!("{costless} production lines at this read have no positive cost, so their design's daily output and cost per unit are unknown (the game shows only a placeholder rate for them)."));
    }
    if other > 0 {
        warnings.push(format!("{other} naval, refit or railway-gun lines at this read make equipment whose daily output is not established, so it, its IC and its cost are unknown."));
    }
    let types = types.finish(content, &mut warnings);
    Ok(StockpileAtReply {
        tag: at.holders.name(&params.country, at.hour),
        campaign_key: context.campaign_key.clone(),
        decimals: DECIMALS,
        types,
        variants,
        logistics: logistics.into_iter().map(|key| StockAtLogistics { key }).collect(),
        stock_missing,
        lines_missing,
        balance,
        balance_missing,
        need_bound_raw: NEED_BOUND_RAW.to_string(),
        warnings,
    })
}

pub struct EquipmentStockpileAt;

impl AtHour for EquipmentStockpileAt {
    const KEY: &'static str = "equipment-stockpile-at";
    const TITLE: &'static str = "Equipment stockpile";
    const CONTENT_DEPENDENT: bool = true;
    type Params = StockpileAtParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = StockpileAtReply;
    type Context = AtContext;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![StockpileAtParams::decl(), super::Domain::decl(), StockType::decl(), StockBalance::decl(), StockAtVariant::decl(), StockAtLogistics::decl(), StockpileAtReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables = vec![TableId::StockByVariant, TableId::Stockpiles, TableId::ProductionLines, TableId::LogisticsEquipment, TableId::VariantDefinitions,
            TableId::VariantModules, TableId::VariantUpgrades];
        tables.extend(NAME_TABLES);
        tables
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        let Some(owner) = owner else { return Vec::new() };
        let range = |table, (low, high)| Read { table, keys: dashboard_base::products::at_hour::Keys::Range(low, high) };
        match step {
            0 => {
                let mut reads = vec![range(TableId::StockByVariant, owner_range(owner.slot)), range(TableId::Stockpiles, raw_range(owner.id)), range(TableId::LogisticsEquipment, raw_range(owner.id)), Read::all(TableId::ProductionLines)];
                reads.extend(NAME_TABLES.map(Read::all));
                reads
            }
            1 => super::design_reads(designs(held, owner)).into(),
            _ => Vec::new(),
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(AtContext { content: Content::load(source, load)?, campaign_key: source.campaign_key().to_owned() })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Equipment stockpile country is unavailable at this hour"))?;
        reply(frame, params, owner, at)
    }
}
