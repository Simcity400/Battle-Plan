//! Division equipment at a reached hour. Shortfalls are computed separately
//! for every division and archetype before summing; stockpile is dated context
//! only. Daily pools and hourly newly minted pools keep their own coverage.
use super::{
    army_content::{self, Refusals},
    division_needs::Needs,
};
use dashboard_base::{
    prepared::query::progressive,
    products::{Product, equipment},
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_forces::products::army_size::{ArmyAttribution, attributed};
use dashboard_text::catalogue::Catalogue;
use replay_data::rows::{DivisionEquipmentRow, DivisionRow, DivisionStatusRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct DivisionEquipment;
const TABLES: &[TableId] = &[
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Divisions,
    TableId::DivisionStatus,
    TableId::DivisionLandInputs,
    TableId::DivisionTemplates,
    TableId::DivisionTemplateIcons,
    TableId::LexicalTokens,
    TableId::CountryLocalSubunits,
    TableId::NewDivisionEquipment,
];
const POOLS: &[TableId] = &[TableId::DivisionEquipment, TableId::Divisions];
const STOCK: &[TableId] = &[
    TableId::Stockpiles,
    TableId::Countries,
    TableId::CountryLinks,
];

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DivisionEquipmentParams {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
    pub count_for: ArmyAttribution,
    pub template: Option<String>,
    pub division: Option<String>,
}

/// Actual holdings of one design, never a variant-specific requirement.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EquipmentVariant {
    pub variant: String,
    pub name: Option<String>,
    pub held_raw: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EquipmentType {
    /// None is the explicit unclassified bucket, not a content key.
    pub archetype: Option<String>,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    /// What every division whose pool was read holds, its comparison known or not.
    pub held_raw: String,
    pub needed_raw: Option<String>,
    /// Each known division's holding capped at its own need, summed: the
    /// fulfilment its need is measured by, so one division's surplus never
    /// fills another's shortfall.
    pub filled_raw: Option<String>,
    pub missing_raw: Option<String>,
    pub divisions_short: u32,
    pub unknown_divisions: u32,
    /// National stock is never used in the deployed shortfall calculation.
    pub stockpile_raw: Option<String>,
    pub stockpile_hour: Option<i32>,
    pub variants: Vec<EquipmentVariant>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EquipmentTemplate {
    pub template: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub icon: Option<String>,
    pub icon_missing: Option<String>,
    pub divisions: u32,
    pub manpower: Option<String>,
    pub manpower_required: Option<String>,
    pub hp_raw: Option<String>,
    pub hp_max_raw: Option<String>,
    /// Known divisions only, with unknown_divisions naming the remainder.
    pub needed_raw: Option<String>,
    /// Only needed archetypes, capped separately at each division's need.
    pub held_raw: Option<String>,
    pub types_short: u32,
    pub unknown_divisions: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EquipmentDivision {
    pub division: String,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub manpower: Option<String>,
    pub manpower_required: Option<String>,
    pub hp_raw: Option<String>,
    pub hp_max_raw: Option<String>,
    pub needed_raw: Option<String>,
    pub held_raw: Option<String>,
    pub types_short: u32,
    pub pool_known: bool,
    /// The previous template's name during conversion.
    pub converting_from: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionPoolType {
    pub archetype: Option<String>,
    pub name: Option<String>,
    pub held_raw: String,
    pub needed_raw: Option<String>,
    pub missing_raw: Option<String>,
    pub variants: Vec<EquipmentVariant>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionPool {
    pub division: String,
    pub name: Option<String>,
    pub template: String,
    pub template_name: Option<String>,
    pub manpower: Option<String>,
    pub manpower_required: Option<String>,
    pub hp_raw: Option<String>,
    pub hp_max_raw: Option<String>,
    pub types: Vec<DivisionPoolType>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DivisionEquipmentReply {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
    pub count_for: ArmyAttribution,
    pub absent: bool,
    pub pool_hour: Option<i32>,
    pub divisions: u32,
    /// Divisions with a known shortfall of any type; unknown ones are not among them.
    pub divisions_short: u32,
    pub unknown_divisions: u32,
    pub types: Vec<EquipmentType>,
    pub templates: Vec<EquipmentTemplate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub division_rows: Option<Vec<EquipmentDivision>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pool: Option<DivisionPool>,
    pub warnings: Vec<String>,
}
dashboard_base::reply_warnings!(DivisionEquipmentReply);

impl Product for DivisionEquipment {
    const KEY: &'static str = "division-equipment";
    const INTERACTIVE: bool = true;
    const CONTENT_DEPENDENT: bool = true;
    type Params = DivisionEquipmentParams;
    type Reply = DivisionEquipmentReply;
    fn types() -> Vec<String> {
        vec![
            DivisionEquipmentParams::decl(),
            DivisionEquipmentReply::decl(),
            EquipmentVariant::decl(),
            EquipmentType::decl(),
            EquipmentTemplate::decl(),
            EquipmentDivision::decl(),
            DivisionPoolType::decl(),
            DivisionPool::decl(),
            ArmyAttribution::decl(),
        ]
    }
    fn validate(p: &Self::Params) -> Result<(), &'static str> {
        dashboard_base::products::validate_tag(&p.tag)?;
        p.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if p.template.is_some() && p.division.is_some() {
            return Err("Choose a template or a division, not both");
        }
        for handle in [&p.template, &p.division].into_iter().flatten() {
            if handle
                .parse::<u64>()
                .ok()
                .filter(|handle| *handle != 0)
                .is_none()
            {
                return Err("Invalid full unit or template handle");
            }
        }
        Ok(())
    }
    fn load(
        source: &Source,
        p: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        Self::validate(p).map_err(io::Error::other)?;
        let branch = p.branch_id.parse::<u64>().map_err(io::Error::other)?;
        let reply = dashboard_land_battles::products::reached::cached(
            source,
            context,
            Self::KEY,
            branch,
            p.hour,
            &(&p.tag, p.count_for, &p.template, &p.division),
            cancelled,
            || {
                let Supplied {
                    frame,
                    daily,
                    stock,
                    designs,
                    stock_designs,
                } = supplied(source, branch, p.hour, cancelled)?;
                let mut refusals = Refusals::default();
                let cat = refusals.keep(army_content::content(source, context)).0;
                let names = cat.as_ref().map(|cat| {
                    let (names, notes) =
                        dashboard_land_battles::products::division_log::division_name_groups(
                            source, cat,
                        );
                    for note in notes {
                        refusals.note(note);
                    }
                    names
                });
                let needs = Needs::read(cat.as_ref(), &frame);
                let mut reply = build(
                    &frame,
                    p,
                    &Context {
                        daily: daily.as_deref(),
                        stock: stock.as_deref(),
                        cat: cat.as_ref(),
                        designs: &army_content::Designs::new(designs.as_deref(), cat.as_ref()),
                        stock_designs: &army_content::Designs::new(
                            stock_designs.as_deref(),
                            cat.as_ref(),
                        ),
                        needs: &needs,
                        names: names
                            .as_ref()
                            .map(|names| names.as_ref().map(Arc::as_ref).map_err(Clone::clone)),
                        icon: &|handle| {
                            army_content::template_icon(source, &frame, handle, cat.as_ref())
                        },
                    },
                    &mut refusals,
                    cancelled,
                )?;
                reply.warnings = refusals.lines();
                Ok(reply)
            },
        )?;
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

/// The reached hour's frame and the supplying reads dated independently of it.
pub(crate) struct Supplied {
    pub frame: Box<DecodedFrame>,
    pub daily: Option<Box<DecodedFrame>>,
    pub stock: Option<Box<DecodedFrame>>,
    pub designs: Option<Box<DecodedFrame>>,
    /// The designs as of the stockpile's read.
    pub stock_designs: Option<Box<DecodedFrame>>,
}

/// [`Supplied`] at `hour`, its reads at once where block decodes are allowed.
pub(crate) fn supplied(
    source: &Source,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<Supplied> {
    supplied_with(
        source,
        branch,
        hour,
        cancelled,
        !progressive::block_decodes_disabled(),
    )
}

/// [`Supplied`] at `hour`. The reads are independent but for the stockpile's designs,
/// so `at_once` runs them together; the first failure in their listed order is the
/// answer either way. In order, no read follows a failure, so none refuses a decode
/// the answer does not need.
pub(crate) fn supplied_with(
    source: &Source,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
    at_once: bool,
) -> io::Result<Supplied> {
    let frame = || {
        progressive::frame_at(source, branch, hour, TABLES, cancelled)?
            .ok_or_else(|| io::Error::other("This hour is not recorded"))
    };
    let latest = |hour: i32, table: TableId, tables: &[TableId]| {
        progressive::latest_read(
            source,
            branch,
            hour,
            table,
            replay_data::cadence::STAGGER_PERIOD,
            tables,
            cancelled,
        )
    };
    let daily = || latest(hour, TableId::DivisionEquipment, POOLS);
    let stock = || latest(hour, TableId::Stockpiles, STOCK);
    let designs = |hour| {
        latest(
            hour,
            TableId::VariantDefinitions,
            &equipment::VariantCatalogue::TABLES,
        )
    };
    let stock_designs = |stock: &Option<Box<DecodedFrame>>| match stock.as_deref() {
        Some(stock) => designs(stock.hour),
        None => Ok(None),
    };
    if !at_once {
        let frame = frame()?;
        let daily = daily()?;
        let stock = stock()?;
        let designs = designs(hour)?;
        let stock_designs = stock_designs(&stock)?;
        return Ok(Supplied {
            frame,
            daily,
            stock,
            designs,
            stock_designs,
        });
    }
    let ((frame, daily), ((stock, stock_designs), designs)) = progressive::join(
        || progressive::join(frame, daily),
        || {
            progressive::join(
                || {
                    let stock = stock();
                    let designs = stock.as_ref().ok().map(stock_designs);
                    (stock, designs)
                },
                || designs(hour),
            )
        },
    );
    let (frame, daily, stock, designs) = (frame?, daily?, stock?, designs?);
    Ok(Supplied {
        frame,
        daily,
        stock,
        designs,
        stock_designs: stock_designs.expect("read with the stockpile")?,
    })
}

/// Independently dated inputs; tests supply synthetic frames through this same builder.
pub struct Context<'a> {
    pub daily: Option<&'a DecodedFrame>,
    pub stock: Option<&'a DecodedFrame>,
    pub cat: Option<&'a Arc<Catalogue>>,
    pub designs: &'a army_content::Designs<'a>,
    pub stock_designs: &'a army_content::Designs<'a>,
    pub needs: &'a Needs<'a>,
    pub names: Option<Result<&'a installed_content_division_names::division_names::Database, String>>,
    pub icon: &'a dyn Fn(u64) -> Result<String, String>,
}

fn count(value: usize) -> io::Result<u32> {
    u32::try_from(value)
        .map_err(|_| io::Error::other("Division equipment count exceeds its wire width"))
}
fn sum(left: i128, right: i128) -> io::Result<i128> {
    left.checked_add(right)
        .ok_or_else(|| io::Error::other("Division equipment total overflows"))
}
fn missing(needed: i128, held: i128) -> io::Result<i128> {
    needed
        .checked_sub(held)
        .map(|amount| amount.max(0))
        .ok_or_else(|| io::Error::other("Division equipment shortfall overflows"))
}

/// Only a complete census proves that omitted daily pools were empty. A
/// complete new-division read covers divisions absent from that census too.
struct Pools<'a> {
    daily: Option<&'a [DivisionEquipmentRow]>,
    census: Option<&'a [DivisionRow]>,
    new: Option<&'a [DivisionEquipmentRow]>,
    hour: Option<i32>,
}
impl<'a> Pools<'a> {
    fn read(frame: &'a DecodedFrame, daily: Option<&'a DecodedFrame>) -> Self {
        Self {
            daily: daily.and_then(|frame| frame.complete::<tables::DivisionEquipment>().ok()),
            census: daily.and_then(|frame| frame.complete::<tables::Divisions>().ok()),
            new: frame.complete::<tables::NewDivisionEquipment>().ok(),
            hour: daily
                .filter(|frame| frame.complete::<tables::DivisionEquipment>().is_ok())
                .map(|frame| frame.hour),
        }
    }
    fn slice(rows: &'a [DivisionEquipmentRow], division: u64) -> &'a [DivisionEquipmentRow] {
        let start = rows.partition_point(|row| row.division < division);
        let end = rows[start..].partition_point(|row| row.division == division);
        &rows[start..start + end]
    }
    fn division(&self, division: u64) -> Option<&'a [DivisionEquipmentRow]> {
        if let Some(new) = self.new {
            let rows = Self::slice(new, division);
            if !rows.is_empty() {
                return Some(rows);
            }
        }
        let existed = self
            .census
            .map(|rows| rows.binary_search_by_key(&division, |row| row.id).is_ok());
        if let Some(rows) = self.daily {
            let rows = Self::slice(rows, division);
            if !rows.is_empty() || existed == Some(true) {
                return Some(rows);
            }
        }
        if existed == Some(false) && self.daily.is_some() && self.new.is_some() {
            return Some(&[]);
        }
        None
    }
}

#[derive(Default)]
struct HeldType {
    held: i128,
    variants: BTreeMap<u64, (Option<String>, i128)>,
}
impl HeldType {
    fn add(&mut self, row: &DivisionEquipmentRow, name: Option<String>) -> io::Result<()> {
        self.held = sum(self.held, i128::from(row.amount_raw))?;
        let entry = self.variants.entry(row.variant_handle).or_insert((name, 0));
        entry.1 = sum(entry.1, i128::from(row.amount_raw))?;
        Ok(())
    }
    fn variants(&self) -> Vec<EquipmentVariant> {
        self.variants
            .iter()
            .map(|(variant, (name, amount))| EquipmentVariant {
                variant: variant.to_string(),
                name: name.clone(),
                held_raw: amount.to_string(),
            })
            .collect()
    }
}

/// Holdings of every read pool, and known-only need subtotals. No contributing
/// known division means None, not zero.
#[derive(Default)]
struct TypeTotals {
    held: HeldType,
    needed: Option<i128>,
    filled: Option<i128>,
    missing: Option<i128>,
    short: u32,
    unknown: BTreeSet<u64>,
}
impl TypeTotals {
    fn hold(&mut self, held: &HeldType) -> io::Result<()> {
        self.held.held = sum(self.held.held, held.held)?;
        for (variant, (name, amount)) in &held.variants {
            let entry = self.held.variants.entry(*variant).or_insert((name.clone(), 0));
            entry.1 = sum(entry.1, *amount)?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct TemplateTotals {
    divisions: u32,
    unknown: u32,
    manpower: Option<i64>,
    required: Option<i64>,
    hp: Option<i128>,
    hp_max: Option<i128>,
    needed: Option<i128>,
    held: Option<i128>,
    short: BTreeSet<String>,
}
impl TemplateTotals {
    fn new() -> Self {
        Self {
            manpower: Some(0),
            required: Some(0),
            hp: Some(0),
            hp_max: Some(0),
            ..Self::default()
        }
    }
}
fn add_optional<T: Copy>(
    total: &mut Option<T>,
    value: Option<T>,
    add: impl FnOnce(T, T) -> Option<T>,
) {
    *total = total.and_then(|total| value.and_then(|value| add(total, value)));
}

fn division_name(
    status: Option<&DivisionStatusRow>,
    context: &Context<'_>,
) -> Result<String, String> {
    let status = status.ok_or("Division status was not read")?;
    let cat = context
        .cat
        .ok_or("Division names require the recorded content stack")?;
    dashboard_text::unit_names::army_name(status, &cat.localisation, &cat.recorded_language, || {
        context
            .names
            .clone()
            .ok_or_else(|| "Division name groups were not read".to_owned())?
    })
    .and_then(|name| dashboard_base::catalogue::plain_text(&name))
}

#[cfg(test)]
mod tests;

/// One division's own facts and its comparison of pool against need.
struct Division<'a> {
    row: &'a DivisionRow,
    status: Option<&'a DivisionStatusRow>,
    manpower: Option<i64>,
    required: Option<i64>,
    hp_max: Option<i128>,
    need: Result<Vec<(String, i64)>, String>,
    pool_known: bool,
    held: BTreeMap<Option<String>, HeldType>,
    /// The per-type comparison, present only when pool, need and every
    /// design's classification are known.
    compared: Option<Compared>,
}

/// One type of one division: its need, its holding capped at the need, and
/// its own shortfall.
struct TypeCompared {
    key: Option<String>,
    need: i128,
    filled: i128,
    shortfall: i128,
}

/// Each type's comparison, before any summing.
struct Compared {
    types: Vec<TypeCompared>,
    needed: i128,
    filled: i128,
    short: BTreeSet<String>,
}

fn compare(
    need: &[(String, i64)],
    held: &BTreeMap<Option<String>, HeldType>,
) -> io::Result<Compared> {
    let need: Vec<(Option<String>, i128)> = need
        .iter()
        .map(|(key, amount)| (Some(key.clone()), i128::from(*amount)))
        .collect();
    let mut keys: Vec<Option<String>> = need.iter().map(|(key, _)| key.clone()).collect();
    keys.extend(held.keys().filter(|key| !keys.contains(key)).cloned().collect::<Vec<_>>());
    let mut compared = Compared {
        types: Vec::new(),
        needed: 0,
        filled: 0,
        short: BTreeSet::new(),
    };
    for key in keys {
        let amount = need
            .iter()
            .find(|(need, _)| *need == key)
            .map_or(0, |(_, amount)| *amount);
        let in_pool = held.get(&key).map_or(0, |pool| pool.held);
        let shortfall = missing(amount, in_pool)?;
        let filled = in_pool.clamp(0, amount.max(0));
        compared.needed = sum(compared.needed, amount)?;
        compared.filled = sum(compared.filled, filled)?;
        if shortfall > 0
            && let Some(key) = &key
        {
            compared.short.insert(key.clone());
        }
        compared.types.push(TypeCompared {
            key,
            need: amount,
            filled,
            shortfall,
        });
    }
    Ok(compared)
}

/// The divisions counted for the country, narrowed to a template or a division.
/// A division whose country is not linked is left out and counted.
fn members<'a>(
    frame: &'a DecodedFrame,
    p: &DivisionEquipmentParams,
    slot: u32,
    countries: &army_content::Countries<'_>,
    refusals: &mut Refusals,
) -> io::Result<Vec<&'a DivisionRow>> {
    let template = p.template.as_ref().and_then(|handle| handle.parse::<u64>().ok());
    let division = p.division.as_ref().and_then(|handle| handle.parse::<u64>().ok());
    let mut members = Vec::new();
    for row in frame.complete::<tables::Divisions>()? {
        if row.reveal_skip != 0 {
            continue;
        }
        match countries.slot(attributed(row, p.count_for)) {
            None => refusals.note("A division's country has no recorded link; it is left out"),
            Some(owner) if owner == slot
                && template.is_none_or(|handle| row.template == handle)
                && division.is_none_or(|handle| row.id == handle) =>
            {
                members.push(row);
            }
            Some(_) => {}
        }
    }
    if division.is_some() && members.is_empty() {
        refusals.note("The selected division is not in this country's army at this hour");
    }
    Ok(members)
}

/// The row keyed `id` in rows sorted by that key.
fn find<R>(rows: Option<&[R]>, id: u64, key: impl FnMut(&R) -> u64) -> Option<&R> {
    let rows = rows?;
    rows.binary_search_by_key(&id, key).ok().map(|at| &rows[at])
}

/// One division read against its template's need and its witnessed pool.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::type_complexity)]
fn division<'a>(
    frame: &'a DecodedFrame,
    row: &'a DivisionRow,
    pools: &Pools<'a>,
    context: &Context<'_>,
    needs: &mut BTreeMap<u64, Result<Vec<(String, i64)>, String>>,
    designs: &mut BTreeMap<u64, army_content::Design>,
    refusals: &mut Refusals,
) -> io::Result<Division<'a>> {
    let status = find(frame.complete::<tables::DivisionStatus>().ok(), row.id, |row| row.division);
    let manpower = find(frame.complete::<tables::DivisionLandInputs>().ok(), row.id, |row| row.division)
        .map(|land| i64::from(land.manpower.iter().fold(0i32, |total, part| total.wrapping_add(part.amount))));
    if manpower.is_none() {
        refusals.note("Division manpower was not recorded");
    }
    let required = find(frame.complete::<tables::DivisionTemplateIcons>().ok(), row.template, |row| row.template)
        .and_then(|icon| icon.settings.require("template settings").ok()?.as_ref().map(|settings| i64::from(settings.total_manpower)));
    if required.is_none() {
        refusals.note("Template required manpower was not recorded");
    }
    let hp_max = status.map(|status| i128::from(status.maximum_hit_points));
    if hp_max.is_none() {
        refusals.note("Division maximum hit points were not recorded");
    }
    let need = needs
        .entry(row.template)
        .or_insert_with(|| context.needs.template(frame, row.template))
        .clone();
    if let Err(why) = &need {
        refusals.note(why.clone());
    }
    let pool = pools.division(row.id);
    if pool.is_none() {
        refusals.note("Division equipment pool was not recorded within a day of this hour");
    }
    let mut held = BTreeMap::<Option<String>, HeldType>::new();
    for entry in pool.unwrap_or_default() {
        let design = designs
            .entry(entry.variant_handle)
            .or_insert_with(|| context.designs.design(entry.variant_handle, refusals));
        if design.archetype.is_none() {
            refusals.note("Division equipment design has no resolved archetype; holdings are unclassified");
        }
        held.entry(design.archetype.clone())
            .or_default()
            .add(entry, design.name.clone())?;
    }
    let compared = match (&need, pool.is_some() && !held.contains_key(&None)) {
        (Ok(need), true) => Some(compare(need, &held)?),
        _ => None,
    };
    Ok(Division {
        row,
        status,
        manpower,
        required,
        hp_max,
        need,
        pool_known: pool.is_some(),
        held,
        compared,
    })
}

/// One country inventory, optionally narrowed to a template or a full division
/// handle. Every comparison uses current-template need and its witnessed pool;
/// a dated pool never claims an hourly reinforcement request.
pub fn build(
    frame: &DecodedFrame,
    p: &DivisionEquipmentParams,
    context: &Context<'_>,
    refusals: &mut Refusals,
    cancelled: &AtomicBool,
) -> io::Result<DivisionEquipmentReply> {
    DivisionEquipment::validate(p).map_err(io::Error::other)?;
    let pools = Pools::read(frame, context.daily);
    let mut reply = DivisionEquipmentReply {
        branch_id: p.branch_id.clone(),
        hour: frame.hour,
        tag: p.tag.clone(),
        count_for: p.count_for,
        absent: false,
        pool_hour: pools.hour,
        divisions: 0,
        divisions_short: 0,
        unknown_divisions: 0,
        types: Vec::new(),
        templates: Vec::new(),
        division_rows: p.template.as_ref().map(|_| Vec::new()),
        pool: None,
        warnings: Vec::new(),
    };
    let Some(holder) = army_content::holder(frame, &p.tag)? else {
        reply.absent = true;
        return Ok(reply);
    };
    let slot = holder.slot;
    let countries = army_content::Countries::read(frame)?;
    for warning in context.needs.warnings() {
        refusals.note(warning);
    }
    let members = members(frame, p, slot, &countries, refusals)?;
    reply.divisions = count(members.len())?;
    let mut types = BTreeMap::<Option<String>, TypeTotals>::new();
    let mut templates = BTreeMap::<u64, TemplateTotals>::new();
    let mut needs = BTreeMap::new();
    let mut designs = BTreeMap::new();
    // An unknown comparison can involve any type, including one first reached
    // later in this census, so it is counted against every type at the end.
    let mut unknown = BTreeSet::new();
    for row in members {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let division = division(frame, row, &pools, context, &mut needs, &mut designs, refusals)?;
        // What a read pool holds counts whether or not its comparison is known.
        for (key, held) in &division.held {
            types.entry(key.clone()).or_default().hold(held)?;
        }
        match &division.compared {
            Some(compared) => {
                reply.divisions_short += u32::from(!compared.short.is_empty());
                for kind in &compared.types {
                    let total = types.entry(kind.key.clone()).or_default();
                    total.needed = Some(sum(total.needed.unwrap_or(0), kind.need)?);
                    total.filled = Some(sum(total.filled.unwrap_or(0), kind.filled)?);
                    total.missing = Some(sum(total.missing.unwrap_or(0), kind.shortfall)?);
                    total.short += u32::from(kind.shortfall > 0);
                }
            }
            None => {
                reply.unknown_divisions += 1;
                unknown.insert(row.id);
                // Its needed types are listed even when no known division holds them.
                for (archetype, _) in division.need.iter().flatten() {
                    types.entry(Some(archetype.clone())).or_default();
                }
            }
        }
        // `new` starts every total at zero; `Default` leaves them unknown.
        #[allow(clippy::unwrap_or_default)]
        let total = templates.entry(row.template).or_insert_with(TemplateTotals::new);
        total.divisions += 1;
        total.unknown += u32::from(division.compared.is_none());
        add_optional(&mut total.manpower, division.manpower, i64::checked_add);
        add_optional(&mut total.required, division.required, i64::checked_add);
        add_optional(&mut total.hp, Some(i128::from(row.hit_points)), i128::checked_add);
        add_optional(&mut total.hp_max, division.hp_max, i128::checked_add);
        if let Some(compared) = &division.compared {
            total.needed = Some(sum(total.needed.unwrap_or(0), compared.needed)?);
            total.held = Some(sum(total.held.unwrap_or(0), compared.filled)?);
            total.short.extend(compared.short.iter().cloned());
        }
        if let Some(rows) = reply.division_rows.as_mut() {
            rows.push(division_row(frame, &division, context, refusals)?);
        }
        if p.division.is_some() {
            reply.pool = Some(pool(frame, &division, context, refusals)?);
        }
    }
    let stocks = stock_totals(holder, context, refusals, cancelled)?;
    for (key, mut total) in types {
        total.unknown.extend(&unknown);
        let (name, name_missing) = match &key {
            Some(key) => refusals.keep(army_content::localised(context.cat, key)),
            None => (None, Some("Equipment archetype is unclassified".into())),
        };
        let stock = stocks
            .as_ref()
            .and_then(|stocks| key.as_ref().map(|key| stocks.get(key).copied().unwrap_or(0)));
        reply.types.push(EquipmentType {
            archetype: key,
            name,
            name_missing,
            held_raw: total.held.held.to_string(),
            needed_raw: total.needed.map(|amount| amount.to_string()),
            filled_raw: total.filled.map(|amount| amount.to_string()),
            missing_raw: total.missing.map(|amount| amount.to_string()),
            divisions_short: total.short,
            unknown_divisions: count(total.unknown.len())?,
            stockpile_raw: stock.map(|amount| amount.to_string()),
            stockpile_hour: stock.and(context.stock.map(|frame| frame.hour)),
            variants: total.held.variants(),
        });
    }
    for (handle, total) in templates {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let (name, name_missing) =
            refusals.keep(army_content::template_name(frame, handle, context.cat));
        let (icon, icon_missing) = refusals.keep((context.icon)(handle));
        reply.templates.push(EquipmentTemplate {
            template: handle.to_string(),
            name,
            name_missing,
            icon,
            icon_missing,
            divisions: total.divisions,
            manpower: total.manpower.map(|value| value.to_string()),
            manpower_required: total.required.map(|value| value.to_string()),
            hp_raw: total.hp.map(|amount| amount.to_string()),
            hp_max_raw: total.hp_max.map(|amount| amount.to_string()),
            needed_raw: total.needed.map(|amount| amount.to_string()),
            held_raw: total.held.map(|amount| amount.to_string()),
            types_short: count(total.short.len())?,
            unknown_divisions: total.unknown,
        });
    }
    Ok(reply)
}

/// A row of a template's divisions.
fn division_row(
    frame: &DecodedFrame,
    division: &Division<'_>,
    context: &Context<'_>,
    refusals: &mut Refusals,
) -> io::Result<EquipmentDivision> {
    let (name, name_missing) = refusals.keep(division_name(division.status, context));
    let converting_from = match division
        .status
        .and_then(|row| row.pending_template.require("DivisionStatus.pending_template").ok())
        .copied()
        .flatten()
    {
        Some(0) => None,
        Some(previous) => refusals.keep(army_content::template_name(frame, previous, context.cat)).0,
        None => {
            refusals.note("Division conversion context was not recorded");
            None
        }
    };
    Ok(EquipmentDivision {
        division: division.row.id.to_string(),
        name,
        name_missing,
        manpower: division.manpower.map(|value| value.to_string()),
        manpower_required: division.required.map(|value| value.to_string()),
        hp_raw: Some(division.row.hit_points.to_string()),
        hp_max_raw: division.hp_max.map(|amount| amount.to_string()),
        needed_raw: division.compared.as_ref().map(|compared| compared.needed.to_string()),
        held_raw: division.compared.as_ref().map(|compared| compared.filled.to_string()),
        types_short: count(division.compared.as_ref().map_or(0, |compared| compared.short.len()))?,
        pool_known: division.pool_known,
        converting_from,
    })
}

/// One division's pool: each needed or held type, with the type's own need and
/// shortfall when the comparison is known. A pool keeps its readable holdings
/// even when its need is unknown.
fn pool(
    frame: &DecodedFrame,
    division: &Division<'_>,
    context: &Context<'_>,
    refusals: &mut Refusals,
) -> io::Result<DivisionPool> {
    let mut types = Vec::new();
    if division.pool_known {
        let mut keys: Vec<Option<String>> = division
            .need
            .as_ref()
            .map(|need| need.iter().map(|(key, _)| Some(key.clone())).collect())
            .unwrap_or_default();
        keys.extend(division.held.keys().filter(|key| !keys.contains(key)).cloned().collect::<Vec<_>>());
        for key in keys {
            let compared = division
                .compared
                .as_ref()
                .and_then(|compared| compared.types.iter().find(|kind| kind.key == key));
            let needed = division.need.as_ref().ok().and_then(|need| {
                let key = key.as_ref()?;
                Some(need.iter().find(|(archetype, _)| archetype == key).map_or(0, |(_, amount)| i128::from(*amount)))
            });
            let held = division.held.get(&key);
            types.push(DivisionPoolType {
                name: key.as_ref().and_then(|key| refusals.keep(army_content::localised(context.cat, key)).0),
                archetype: key,
                held_raw: held.map_or(0, |pool| pool.held).to_string(),
                needed_raw: needed.map(|amount| amount.to_string()),
                missing_raw: compared.map(|kind| kind.shortfall.to_string()),
                variants: held.map_or_else(Vec::new, HeldType::variants),
            });
        }
    }
    let (name, _) = refusals.keep(division_name(division.status, context));
    Ok(DivisionPool {
        division: division.row.id.to_string(),
        name,
        template: division.row.template.to_string(),
        template_name: refusals.keep(army_content::template_name(frame, division.row.template, context.cat)).0,
        manpower: division.manpower.map(|value| value.to_string()),
        manpower_required: division.required.map(|value| value.to_string()),
        hp_raw: Some(division.row.hit_points.to_string()),
        hp_max_raw: division.hp_max.map(|amount| amount.to_string()),
        types,
    })
}

/// National stock at its own read and country identity. A classification gap
/// cannot prove zero stock of an omitted archetype, so withhold that context.
fn stock_totals(
    holder: army_content::Holder,
    context: &Context<'_>,
    refusals: &mut Refusals,
    cancelled: &AtomicBool,
) -> io::Result<Option<BTreeMap<String, i128>>> {
    let Some(stock) = context.stock else {
        refusals.note("National equipment stockpile was not read within a day of this hour");
        return Ok(None);
    };
    if let Err(why) = army_content::same_holder(stock, holder) {
        refusals.note(format!("National equipment stockpile is left out: {why} at its read"));
        return Ok(None);
    }
    let slot = holder.slot;
    let (Ok(countries), Ok(rows)) = (
        army_content::Countries::read(stock),
        stock.complete::<tables::Stockpiles>(),
    ) else {
        refusals.note("National equipment stockpile or its country links were not recorded");
        return Ok(None);
    };
    let mut totals = BTreeMap::new();
    let mut designs = BTreeMap::<u64, army_content::Design>::new();
    let mut complete = true;
    for row in rows {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let Some(country) = countries.slot(row.country) else {
            refusals.note("Stockpile country has no recorded link");
            complete = false;
            continue;
        };
        if country != slot {
            continue;
        }
        let design = designs
            .entry(row.variant_handle)
            .or_insert_with(|| context.stock_designs.design(row.variant_handle, refusals));
        if let Some(key) = &design.archetype {
            let amount = totals.entry(key.clone()).or_default();
            *amount = sum(*amount, i128::from(row.amount_raw))?;
        } else {
            refusals.note("National stockpile design has no resolved archetype");
            complete = false;
        }
    }
    Ok(complete.then_some(totals))
}
