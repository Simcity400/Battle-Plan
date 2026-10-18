//! One country's equipment stockpile over a branch, for the Equipment stockpile
//! widget: what it holds of each variant, what its production lines make of it
//! per day and the IC they apply, and each variant's equipment type (archetype)
//! and domain from content.
//!
//! **In stock** is the recorded `Stockpiles` pool (fixed5,
//! `engine_native::equipment_entry`), compared exactly with the save by the
//! fresh-save oracle (`recorder::oracle::production::compare_stockpiles`). A
//! variant listed twice holds their sum; one not listed holds none.
//!
//! **Output per day** is the sum, over the country's military production lines
//! making a variant, of each line's units per day by the game's per-day formula
//! ([`equipment::daily_units_raw`]), each line truncated on its own as the game's
//! production view shows it (never a summed weekly figure divided by seven). A
//! line whose rate cannot be computed (no positive cost) makes that variant's
//! daily total unknown for the read. **IC per day** sums the same lines' speed,
//! the IC each daily production tick adds; the **cost per unit** is each line's
//! recorded cost after its organisation's bonus, sent as the lowest and highest
//! among the variant's lines. A variant held but not in production has no
//! recorded cost. Naval, refit and railway-gun lines are not military lines:
//! whether and at what rate they deliver to the stockpile is not established, so
//! a variant one of them produces has an unknown daily total, IC and cost for the
//! read, never zero.
//!
//! **The surplus or shortage** is the recorded `LogisticsEquipment` balance: series
//! 16 of the logistics record, the value the Logistics screen's stock balance
//! column shows, with series 15, the stock the same daily update counted
//! (`engine_land_inputs::clogistics_status`, table read in daily slot 0, just after the update).
//! The daily update writes the balance per archetype as
//! `Avail − clamp(Demand + PoolDifference)`:
//!
//! - `Avail` is series 15 (for an archetype of category bit 0, convoys, the live
//!   pool instead), plus the supply system's truck terms for a `supply_truck`
//!   archetype or its train term for a category-bit-31 archetype, clamped at zero;
//! - `Demand` sums training lines' missing equipment, the army's reinforcement
//!   requests, garrisons, the cached air requirement, operations and trucks and
//!   trains; `PoolDifference` is outgoing
//!   minus incoming lend-lease pools; the sum saturates at ±[`NEED_BOUND_RAW`].
//!
//! The game writes the balance only for countries that are not AI-controlled,
//! so a complete table without rows for the country means it was AI-controlled
//! at that read (`ai_hours`), not that it holds nothing.
//!
//! **What is needed** is `available - balance`, using the actual available
//! operand observed at the same native series-16 write. This includes the
//! native convoy cache and truck/train adjustments, never a current-hour
//! substitution. In an older prefix, ordinary archetypes still derive it as
//! `max(0, stock) - balance`; convoy, truck and train rows require the appended
//! operand. A value at the native saturation bound is not an exact sum.
//!
//! Stockpiles and production lines are read in one daily slot and carried in
//! between (`metadata::slot_read`); both come from the same read, so their values
//! always describe one game state. The logistics balance and its stock come from
//! their own record, read in slot 0, so a balance is always shown with the stock
//! that record counted. Values start only at a read, are carried over
//! the following hours of the same country identity, and end at a gap, an
//! incomplete table or a change of identity (`read_runs`).
//!
//! **Equipment types** come from content (`installed_content_core::equipment`): a variant's type is
//! its recorded equipment token, resolved to the authored key under the lexer's
//! fold, and grouped by that type's archetype, as the Logistics screen groups
//! rows (by archetype pointer, never substituting one archetype for another).
//! Types are listed in the equipment database's order.
//! A variant's type never changes, so any read of the variant table names it.
//!
//! **Artwork** uses the transient [`artwork`] product. The Logistics screen draws
//! an archetype's row with the country's best variant for it, selected from its
//! best-variant candidates and drawn with that variant's icon, starting from its
//! icon entry. The recording holds both: the candidates are
//! `AvailableEquipmentVariants` roster kind 2 and each variant's icon entry is in
//! `VariantDefinitions`. Missing candidates or ranking
//! operands refuse the picture instead of guessing a design. Variant rows use
//! their own icon; domain aggregates have no single best archetype variant.

use dashboard_base::products::{
    CountryParams, Product,
    equipment::{self, Domain, Texts, VariantCatalogue},
};
use dashboard_base::{
    catalogue,
    products::country_groups::HourSpan,
    read_runs::{AmountRun, ReadRuns},
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_text::metadata::{IdentityTrack, slot_read};
use installed_content_core::equipment::Database;
use replay_data::{
    rows::LogisticsEquipmentRow, rows::ProductionLineRow, rows::StockpileRow, rows::TableId,
    rows::tables,
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

#[cfg(test)]
use dashboard_text::metadata::validate_campaign;
#[cfg(test)]
use replay::container::Replay;
#[cfg(test)]
use replay_frame::frame::FrameDecoder;
#[cfg(test)]
use std::{collections::VecDeque, sync::atomic::Ordering};

pub struct EquipmentStockpile;

pub mod artwork;
pub mod prepared;

#[cfg(test)]
const CACHED_COUNTRIES: usize = 8;

pub use dashboard_base::products::equipment::StockVariantName;

/// One variant the country held or produced in the branch.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockVariant {
    /// The variant's native handle (`VariantDefinitions.id`), as a decimal string.
    pub id: String,
    /// The equipment type's script key; empty when no read of the variant table
    /// named it.
    pub token: String,
    /// Its type's archetype key; null when content does not resolve its type.
    pub archetype: Option<String>,
    pub names: Vec<StockVariantName>,
    /// In stock, fixed5; no run inside a known span means none.
    pub stock: Vec<AmountRun>,
    /// Units a day from its military production lines by the game's per-day
    /// formula, each line truncated on its own and summed, fixed5; no run inside
    /// a known span means none.
    pub daily: Vec<AmountRun>,
    /// The factory IC a day those lines apply (each line's speed), fixed5.
    pub daily_ic: Vec<AmountRun>,
    /// The lowest and highest cost per unit among those lines, fixed5, after each
    /// line's organisation bonus: equal when they share one cost, unknown where a
    /// line has no positive cost, none without a line.
    pub unit_cost_low: Vec<AmountRun>,
    pub unit_cost_high: Vec<AmountRun>,
}

/// An equipment type (archetype) as content names it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockType {
    pub key: String,
    /// The localised key; null when content does not name it.
    pub name: Option<String>,
    pub domain: Domain,
    /// Why older or missing observations cannot derive this type's need.
    /// A recorded native available operand takes precedence over this note.
    pub need_note: Option<String>,
}

pub use dashboard_base::products::equipment::UNRESOLVED_MARK;

/// One archetype's logistics record: the balance (surplus positive, deficit
/// negative) and the stock the same daily update counted, fixed5. No run inside
/// a known span (`balance_hours`) means zero.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TypeLogistics {
    /// The archetype key (a `types` key), or [`UNRESOLVED_MARK`] and the recorded
    /// token's spelling when content does not resolve it.
    pub key: String,
    pub balance: Vec<AmountRun>,
    pub stock: Vec<AmountRun>,
    /// Actual native availability at the balance write; null runs are refused
    /// or unrecorded observations. It is independent of the stock series.
    pub available: Vec<AmountRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct EquipmentStockpileReply {
    pub branch_id: String,
    pub tag: String,
    /// The recorded campaign identity (`ReplayMetadata::campaign_key`). A variant's
    /// handle is an id only within its campaign, so saved variant choices carry it.
    pub campaign_key: String,
    pub decimals: u8,
    /// Every archetype a listed variant belongs to, in the equipment database's
    /// order (the game's order); empty without content.
    pub types: Vec<StockType>,
    pub variants: Vec<StockVariant>,
    /// Hours the stockpile was read, ascending: a value shown later is as of the
    /// latest read before it.
    pub reads: Vec<i32>,
    /// Hours whose stock is known: each read and the hours carried from it.
    pub stock_hours: Vec<HourSpan>,
    /// Hours whose production lines are known, likewise. Lines are captured apart
    /// from the stockpile, so stock can be known where production is not.
    pub line_hours: Vec<HourSpan>,
    /// Hours the production lines were read, ascending, for dating a daily rate
    /// apart from the stockpile read.
    pub line_reads: Vec<i32>,
    /// Each archetype's logistics record over the branch.
    pub logistics: Vec<TypeLogistics>,
    /// Hours the logistics record was read for the country, ascending.
    pub logistics_reads: Vec<i32>,
    /// Hours whose balance is known: reads that held the country's rows and the
    /// hours carried from them.
    pub balance_hours: Vec<HourSpan>,
    /// Hours the country was AI-controlled, so the game computed no balance: reads
    /// of a complete table without its rows and the hours carried from them.
    pub ai_hours: Vec<HourSpan>,
    /// The saturation bound of `Demand + PoolDifference`, fixed5: a derived need
    /// at it is not a real sum.
    pub need_bound_raw: String,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScannedStockpile {
    pub stock: ReadRuns<u64>,
    /// Units a day, IC a day and the lowest and highest cost per unit by variant.
    pub daily: ReadRuns<u64>,
    pub daily_ic: ReadRuns<u64>,
    pub cost_low: ReadRuns<u64>,
    pub cost_high: ReadRuns<u64>,
    pub reads: Vec<i32>,
    /// Each variant's naming operands, with where they are known.
    #[serde(skip)]
    pub variants: VariantCatalogue,
    /// Lines of the country whose cost gives no daily output, per read.
    pub costless: usize,
    /// Naval, refit and railway-gun line reads of the country, whose daily
    /// output is unknown.
    pub other_kinds: usize,
    /// Logistics balance, stock and same-write availability by archetype token.
    pub balance: ReadRuns<u32>,
    pub logistics_stock: ReadRuns<u32>,
    pub available: ReadRuns<u32>,
    pub missing_available: usize,
    /// Spans the country was AI-controlled (a key-less run set).
    pub ai: ReadRuns<u32>,
    pub logistics_reads: Vec<i32>,
    /// Hours the country's production lines were read.
    pub line_reads: Vec<i32>,
    /// Each archetype token's spelling, from the reads that named it.
    pub logistics_tokens: BTreeMap<u32, String>,
}

/// `Demand + PoolDifference` saturates at this magnitude during the daily
/// logistics update (`engine_land_inputs::clogistics_status`; the same bound as
/// `engine_stability::country_stability`'s ring saturation).
pub const NEED_BOUND_RAW: i64 = 0x53e2_d620_fb40;

impl ScannedStockpile {
    /// The production lines broke: nothing of them is carried until their next read.
    fn close_lines(&mut self) {
        for runs in [
            &mut self.daily,
            &mut self.daily_ic,
            &mut self.cost_low,
            &mut self.cost_high,
        ] {
            runs.close();
        }
    }

    /// An hour carried from the lines' last read.
    fn carry_lines(&mut self, hour: i32) {
        for runs in [
            &mut self.daily,
            &mut self.daily_ic,
            &mut self.cost_low,
            &mut self.cost_high,
        ] {
            runs.carry(hour);
        }
    }
}

/// Tables are joined and grouped once for all countries in one accepted frame.
pub(crate) struct FrameRows<'a> {
    frame: &'a DecodedFrame,
    pub(crate) identities: BTreeMap<String, (u32, u32)>,
    stock: Option<BTreeMap<u32, Vec<&'a StockpileRow>>>,
    lines: Option<BTreeMap<u32, Vec<&'a ProductionLineRow>>>,
    logistics: Option<BTreeMap<u32, Vec<&'a LogisticsEquipmentRow>>>,
}
impl<'a> FrameRows<'a> {
    #[cfg(test)]
    pub(crate) fn new(frame: &'a DecodedFrame) -> Self {
        let reads = frame.complete::<tables::Countries>().ok().map(|countries| {
            dashboard_base::products::territory::CountryReads::new(frame, countries)
        });
        Self::resolved(frame, reads.as_ref(), true, true)
    }
    pub(crate) fn resolved(
        frame: &'a DecodedFrame,
        reads: Option<&dashboard_base::products::territory::CountryReads>,
        stock_read: bool,
        logistics_read: bool,
    ) -> Self {
        let reads = reads.filter(|_| frame.complete::<tables::CountryLinks>().is_ok());
        let identities = reads
            .map(|reads| {
                reads
                    .holders()
                    .iter()
                    .filter_map(|(tag, identity)| identity.map(|identity| (tag.clone(), identity)))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            frame,
            identities,
            stock: frame
                .complete::<tables::Stockpiles>()
                .ok()
                .zip(reads)
                .map(|(rows, reads)| {
                    if stock_read {
                        owners(rows, reads.slots(), |row| row.country)
                    } else {
                        BTreeMap::new()
                    }
                }),
            lines: frame
                .complete::<tables::ProductionLines>()
                .ok()
                .zip(reads)
                .map(|(rows, reads)| {
                    if stock_read {
                        owners(rows, reads.slots(), |row| row.country)
                    } else {
                        BTreeMap::new()
                    }
                }),
            logistics: frame
                .complete::<tables::LogisticsEquipment>()
                .ok()
                .zip(reads)
                .map(|(rows, reads)| {
                    if logistics_read {
                        owners(rows, reads.slots(), |row| row.country)
                    } else {
                        BTreeMap::new()
                    }
                }),
        }
    }
}
fn owners<'a, T>(
    rows: &'a [T],
    slots: &std::collections::HashMap<u32, u32>,
    country: impl Fn(&T) -> u32,
) -> BTreeMap<u32, Vec<&'a T>> {
    let mut result = BTreeMap::<u32, Vec<&T>>::new();
    for row in rows {
        #[cfg(test)]
        ROW_VISITS.with(|count| count.set(count.get() + 1));
        if let Some(&slot) = slots.get(&country(row)) {
            result.entry(slot).or_default().push(row);
        }
    }
    result
}
#[cfg(test)]
thread_local! { static ROW_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

/// The legacy frame step; preparation retains one instance per selected holder.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct ScanState {
    pub(crate) result: ScannedStockpile,
    track: IdentityTrack,
    /// The last hour this holder was stepped through while it held its
    /// identity; none when it is current (restored or new) or has none.
    #[serde(skip)]
    advanced: Option<i32>,
    #[cfg(test)]
    #[serde(skip)]
    advances: usize,
}
impl ScanState {
    /// Step the accepted hours after the last one this holder was stepped
    /// through, up to `through`, as if each had been advanced in turn: hours
    /// whose rows left the holder unchanged are not visited when they pass.
    pub(crate) fn catch_up(&mut self, through: Option<i32>) {
        if let (Some(from), Some(through)) = (self.advanced, through)
            && from < through
        {
            let first = from + 1;
            self.track.advance(through);
            for rows in [
                &mut self.result.stock,
                &mut self.result.daily,
                &mut self.result.daily_ic,
                &mut self.result.cost_low,
                &mut self.result.cost_high,
            ] {
                rows.advance_span(first, through);
            }
            for rows in [
                &mut self.result.balance,
                &mut self.result.logistics_stock,
                &mut self.result.available,
            ] {
                rows.advance_span(first, through);
            }
            self.result.ai.advance_span(first, through);
            self.advanced = Some(through);
            #[cfg(test)]
            {
                self.advances += 1;
            }
        }
    }
    pub(crate) fn advance(&mut self, hour: i32) {
        #[cfg(test)]
        {
            self.advances += 1;
        }
        self.advanced = Some(hour);
        self.track.advance(hour);
        for rows in [
            &mut self.result.stock,
            &mut self.result.daily,
            &mut self.result.daily_ic,
            &mut self.result.cost_low,
            &mut self.result.cost_high,
        ] {
            rows.advance(hour);
        }
        for rows in [
            &mut self.result.balance,
            &mut self.result.logistics_stock,
            &mut self.result.available,
        ] {
            rows.advance(hour);
        }
        self.result.ai.advance(hour);
    }
    pub(crate) fn seal(&mut self) {
        for rows in [
            &mut self.result.stock,
            &mut self.result.daily,
            &mut self.result.daily_ic,
            &mut self.result.cost_low,
            &mut self.result.cost_high,
        ] {
            rows.seal();
        }
        for rows in [
            &mut self.result.balance,
            &mut self.result.logistics_stock,
            &mut self.result.available,
        ] {
            rows.seal();
        }
        self.result.ai.seal();
    }
    pub(crate) fn observe(
        &mut self,
        format: replay_data::format::FormatVersion,
        input: &FrameRows<'_>,
        identity: Option<(u32, u32)>,
    ) {
        let decoded = input.frame;
        let hour = decoded.hour;
        // A holder without an identity is not stepped again until it has one.
        self.advanced = identity.map(|_| hour);
        let result = &mut self.result;
        let continues = self.track.step(identity, hour);
        let stock = input.stock.as_ref();
        let lines = input.lines.as_ref();
        // Each table ends its own values where it is missing; the other goes on.
        if !continues || stock.is_none() {
            result.stock.close();
        }
        if !continues || lines.is_none() {
            result.close_lines();
        }
        let logistics = input.logistics.as_ref();
        if !continues || logistics.is_none() {
            result.balance.close();
            result.logistics_stock.close();
            result.available.close();
            result.ai.close();
        }
        let Some((slot, _)) = identity else {
            return;
        };
        match logistics {
            Some(rows)
                if slot_read(
                    format,
                    &decoded.metrics,
                    TableId::LogisticsEquipment,
                    decoded.hour,
                ) =>
            {
                let own = rows.get(&slot).map(Vec::as_slice).unwrap_or_default();
                if own.is_empty() {
                    // The game computed no balance: the country was AI-controlled.
                    result.ai.read(hour, []);
                    result.balance.close();
                    result.logistics_stock.close();
                    result.available.close();
                } else {
                    result.logistics_reads.push(hour);
                    if let Ok(lexical) = decoded.complete::<tables::LexicalTokens>() {
                        for row in own {
                            if let Ok(index) =
                                lexical.binary_search_by_key(&row.token, |token| token.token)
                            {
                                result.logistics_tokens.entry(row.token).or_insert_with(|| {
                                    String::from_utf8_lossy(&lexical[index].literal).into_owned()
                                });
                            }
                        }
                    }
                    result.balance.read(
                        hour,
                        own.iter().map(|row| (row.token, Some(row.balance_raw))),
                    );
                    result
                        .logistics_stock
                        .read(hour, own.iter().map(|row| (row.token, Some(row.stock_raw))));
                    result.missing_available += own
                        .iter()
                        .filter(|row| {
                            !matches!(
                                row.available_raw,
                                replay_data::rows::Recorded::Value(Some(_))
                            )
                        })
                        .count();
                    result.available.read(
                        hour,
                        own.iter().map(|row| {
                            let value = match row.available_raw {
                                replay_data::rows::Recorded::Value(value) => value,
                                replay_data::rows::Recorded::NotRecorded => None,
                            };
                            (row.token, value)
                        }),
                    );
                    result.ai.close();
                }
            }
            _ => {
                result.balance.carry(hour);
                result.logistics_stock.carry(hour);
                result.available.carry(hour);
                result.ai.carry(hour);
            }
        }
        if !slot_read(format, &decoded.metrics, TableId::Stockpiles, decoded.hour) {
            // Carried: the last read still holds, but only after a read of this identity.
            result.stock.carry(hour);
            result.carry_lines(hour);
            return;
        }
        if let Some(stock) = stock {
            result.reads.push(hour);
            result.stock.read(
                hour,
                stock
                    .get(&slot)
                    .into_iter()
                    .flatten()
                    .map(|row| (row.variant_handle, Some(row.amount_raw))),
            );
        }
        if let Some(lines) = lines {
            result.line_reads.push(hour);
            let (mut daily, mut daily_ic) = (Vec::new(), Vec::new());
            // Each variant's lowest and highest cost per unit among its
            // military lines; unknown once any of them has no positive cost.
            let mut costs: BTreeMap<u64, Option<(i64, i64)>> = BTreeMap::new();
            for line in lines.get(&slot).into_iter().flatten() {
                // A rate that cannot be computed makes the variant's daily
                // total unknown at this read, never zero or a partial sum. Only a
                // military line's rate is the military formula; what a naval,
                // refit or railway-gun line delivers is not established.
                let military =
                    line.kind == engine_native::cequipment_production_line::KIND_MILITARY;
                let units = if military {
                    let units = equipment::daily_units_raw(line.speed_raw, line.cost_raw);
                    if units.is_none() {
                        result.costless += 1;
                    }
                    units
                } else {
                    result.other_kinds += 1;
                    None
                };
                daily.push((line.variant, units));
                daily_ic.push((line.variant, military.then_some(line.speed_raw)));
                let cost = (military && line.cost_raw > 0).then_some(line.cost_raw);
                costs
                    .entry(line.variant)
                    .and_modify(|range| {
                        *range = range
                            .zip(cost)
                            .map(|((low, high), cost)| (low.min(cost), high.max(cost)))
                    })
                    .or_insert(cost.map(|cost| (cost, cost)));
            }
            result.daily.read(hour, daily);
            result.daily_ic.read(hour, daily_ic);
            result.cost_low.read(
                hour,
                costs
                    .iter()
                    .map(|(variant, range)| (*variant, range.map(|range| range.0))),
            );
            result.cost_high.read(
                hour,
                costs
                    .iter()
                    .map(|(variant, range)| (*variant, range.map(|range| range.1))),
            );
        }
    }
}

/// Scans one branch for one country's stockpile, production and variant names.
#[cfg(test)]
pub fn scan(
    replay: &Replay,
    branch: u64,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<ScannedStockpile> {
    let selected: Vec<_> = [
        TableId::Campaign,
        TableId::Stockpiles,
        TableId::ProductionLines,
        TableId::LogisticsEquipment,
    ]
    .into_iter()
    .chain(VariantCatalogue::TABLES)
    .collect();
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Stockpile request cancelled");
    let mut state = ScanState::default();
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(cancel());
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    let input = FrameRows::new(decoded);
                    state.result.variants.observe(replay.format, decoded);
                    state.observe(replay.format, &input, input.identities.get(tag).copied());
                }
            }
            Ok(())
        },
    )?;
    Ok(state.result)
}

/// A scan as cached: only the variants its reply lists, with the variant
/// warnings of every variant it read.
#[cfg(test)]
struct Cached {
    scanned: ScannedStockpile,
    variant_warnings: Vec<String>,
}

#[cfg(test)]
#[derive(Default)]
struct Cache {
    entries: VecDeque<((u64, String), Arc<Cached>)>,
}

use dashboard_base::products::equipment::{archetype_of, equipment_warnings};

/// Why an archetype's need cannot be derived from its logistics record, when its
/// available stock holds terms not established at the daily calculation; none
/// when `Avail` is exactly `max(0, stock)`.
fn need_note(types: &Database, archetype: &str) -> Result<Option<String>, String> {
    let kind = types
        .named(archetype)
        .ok_or_else(|| format!("{archetype} is not authored equipment"))?;
    let category = types.category(kind);
    Ok(if category & 1 != 0 {
        Some("This convoy archetype needs the native available-stock operand at the same balance write; it is unknown when that observation is missing.".to_owned())
    } else if kind.supply_truck {
        Some("This supply-truck archetype needs the native available-stock operand at the same balance write; it is unknown when that observation is missing.".to_owned())
    } else if category & (1 << 31) != 0 {
        Some("This train archetype needs the native available-stock operand at the same balance write; hourly train inventory cannot replace a missing observation.".to_owned())
    } else {
        None
    })
}

/// The same archetype name for the recorded reply and offline preparation.
pub fn type_name(catalogue: &dashboard_text::catalogue::Catalogue, key: &str) -> Option<String> {
    let name = catalogue.text(key);
    if name.is_none() {
        catalogue.note(dashboard_core::findings::Finding::new(
            dashboard_core::findings::Kind::Fallback,
            key,
            "equipment type title is unavailable",
        ));
    }
    name
}

impl Product for EquipmentStockpile {
    const KEY: &'static str = "equipment-stockpile";
    const CONTENT_DEPENDENT: bool = true;
    type Params = CountryParams;
    type Reply = EquipmentStockpileReply;

    fn types() -> Vec<String> {
        vec![
            AmountRun::decl(),
            Domain::decl(),
            StockVariantName::decl(),
            StockVariant::decl(),
            StockType::decl(),
            TypeLogistics::decl(),
            prepared::choices::StockpileChoicePeak::decl(),
            prepared::StockpileOperandSummary::decl(),
            prepared::StockpileGroupSummary::decl(),
            prepared::StockpileNamesPage::decl(),
            prepared::StockpileDisplay::decl(),
            EquipmentStockpileReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    #[cfg(test)]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let key = (params.branch(), params.tag.clone());
        let state = source.state::<Cache>();
        let cached = {
            let cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache
                .entries
                .iter()
                .find(|(cached, _)| *cached == key)
                .map(|(_, cached)| Arc::clone(cached))
        };
        let cached = match cached {
            Some(cached) => cached,
            None => {
                let mut scanned = scan(&source.replay(), params.branch(), &params.tag, cancelled)?;
                let variant_warnings = scanned.variants.warnings();
                let (stock, daily) = (&scanned.stock.runs, &scanned.daily.runs);
                scanned
                    .variants
                    .versions
                    .retain(|id, _| stock.contains_key(id) || daily.contains_key(id));
                let cached = Arc::new(Cached {
                    scanned,
                    variant_warnings,
                });
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != key);
                cache.entries.push_back((key, Arc::clone(&cached)));
                while cache.entries.len() > CACHED_COUNTRIES {
                    cache.entries.pop_front();
                }
                cached
            }
        };
        reply(
            source,
            params,
            context,
            &cached.scanned,
            cached.variant_warnings.clone(),
            cancelled,
        )
    }
}

/// The existing stockpile projection, shared by the legacy scan and prepared reads.
pub(crate) fn reply(
    source: &Source,
    params: &CountryParams,
    context: &LoadContext,
    scanned: &ScannedStockpile,
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<EquipmentStockpileReply> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    // Without the game folders the stock still shows, by variant and unnamed types.
    let content = match context
        .content()
        .and_then(|content| catalogue::catalogue(source, content))
    {
        Ok(catalogue) => {
            warnings.extend(catalogue.verification.iter().cloned());
            warnings.extend(catalogue.warnings.iter().cloned());
            match equipment::units(source, &catalogue) {
                Ok(types) => {
                    warnings.extend(equipment_warnings(&types));
                    Some((catalogue, types))
                }
                Err(error) => {
                    warnings.push(format!("Equipment types are unavailable: {error}"));
                    None
                }
            }
        }
        Err(error) => {
            warnings.push(format!(
                "Equipment names and types are unavailable: {error}"
            ));
            None
        }
    };
    let used: BTreeSet<u64> = scanned
        .stock
        .runs
        .keys()
        .chain(scanned.daily.runs.keys())
        .copied()
        .collect();
    let mut types: BTreeMap<String, (usize, Domain)> = BTreeMap::new();
    let mut variants = Vec::new();
    // Types content does not resolve: refused by name and counted, never assumed.
    let (mut unnamed, mut untyped, mut untyped_count) = (0, BTreeSet::new(), 0usize);
    for id in &used {
        if !scanned.variants.versions.contains_key(id) {
            unnamed += 1;
        }
        // The final nonempty spelling also classifies earlier recorded stock.
        let token = scanned.variants.token(*id);
        let archetype = match (&content, token.is_empty()) {
            (Some((_, known)), false) => match archetype_of(known, token) {
                Ok((archetype, ordinal, domain)) => {
                    types.insert(archetype.clone(), (ordinal, domain));
                    Some(archetype)
                }
                Err(reason) => {
                    untyped.insert(reason);
                    untyped_count += 1;
                    None
                }
            },
            _ => None,
        };
        let texts = content
            .as_ref()
            .map(|(catalogue, _)| &**catalogue as &dyn Texts);
        variants.push(StockVariant {
            id: id.to_string(),
            token: token.to_owned(),
            archetype,
            names: scanned.variants.names(*id, texts),
            stock: scanned.stock.runs.get(id).cloned().unwrap_or_default(),
            daily: scanned.daily.runs.get(id).cloned().unwrap_or_default(),
            daily_ic: scanned.daily_ic.runs.get(id).cloned().unwrap_or_default(),
            unit_cost_low: scanned.cost_low.runs.get(id).cloned().unwrap_or_default(),
            unit_cost_high: scanned.cost_high.runs.get(id).cloned().unwrap_or_default(),
        });
    }
    // Archetypes of the logistics record: every one with a value in the branch,
    // including shortages of types the country holds none of.
    let logistics_tokens: BTreeSet<u32> = scanned
        .balance
        .runs
        .keys()
        .chain(scanned.logistics_stock.runs.keys())
        .copied()
        .collect();
    let mut logistics = Vec::new();
    for token in logistics_tokens {
        let spelling = scanned
            .logistics_tokens
            .get(&token)
            .cloned()
            .unwrap_or_else(|| format!("token {token}"));
        let key = match &content {
            Some((_, known)) => match archetype_of(known, &spelling) {
                Ok((archetype, ordinal, domain)) => {
                    types.insert(archetype.clone(), (ordinal, domain));
                    archetype
                }
                Err(reason) => {
                    untyped.insert(reason);
                    untyped_count += 1;
                    format!("{UNRESOLVED_MARK}{spelling}")
                }
            },
            None => format!("{UNRESOLVED_MARK}{spelling}"),
        };
        logistics.push(TypeLogistics {
            key,
            balance: scanned
                .balance
                .runs
                .get(&token)
                .cloned()
                .unwrap_or_default(),
            available: scanned
                .available
                .runs
                .get(&token)
                .cloned()
                .unwrap_or_default(),
            stock: scanned
                .logistics_stock
                .runs
                .get(&token)
                .cloned()
                .unwrap_or_default(),
        });
    }
    let mut types: Vec<(String, usize, Domain)> = types
        .into_iter()
        .map(|(key, (ordinal, domain))| (key, ordinal, domain))
        .collect();
    types.sort_by_key(|(_, ordinal, _)| *ordinal);
    let marked: Vec<&str> = types
        .iter()
        .map(|(key, _, _)| key.as_str())
        .filter(|key| key.starts_with(UNRESOLVED_MARK))
        .collect();
    if !marked.is_empty() {
        warnings.push(format!(
            "Equipment types whose key starts with \"{UNRESOLVED_MARK}\" are not supported: the widget marks unresolved types that way, so they could be mistaken for one ({}).",
            marked.join(", ")
        ));
    }
    let types = types
        .into_iter()
        .map(|(key, _, domain)| StockType {
            name: content
                .as_ref()
                .and_then(|(catalogue, _)| type_name(catalogue, &key)),
            need_note: match &content {
                Some((_, known)) => need_note(known, &key).unwrap_or_else(Some),
                None => {
                    Some("Content is unavailable, so what is needed cannot be derived.".to_owned())
                }
            },
            key,
            domain,
        })
        .collect();
    if unnamed > 0 {
        warnings.push(format!(
            "{unnamed} variants in stock or production have no recorded definition in this branch; their type is unknown."
        ));
    }
    if untyped_count > 0 {
        warnings.push(format!(
            "{untyped_count} recorded variants or archetypes have an equipment type the content does not resolve; their type titles and every total they could belong to are unavailable ({}).",
            untyped.into_iter().collect::<Vec<_>>().join("; ")
        ));
    }
    if scanned.costless > 0 {
        warnings.push(format!(
            "{} production line reads have no positive cost, so their variant's daily output and cost per unit are unknown at those reads (the game shows only a placeholder rate for them).",
            scanned.costless
        ));
    }
    if scanned.logistics_reads.is_empty() && scanned.ai.spans.is_empty() {
        warnings.push("The surplus or shortage is not in this recording: it has no logistics record (LogisticsEquipment).".to_owned());
    }
    if scanned.missing_available > 0 {
        warnings.push(format!("{} logistics archetype reads have no matching recorded available-stock operand; convoy, truck and train need remains unknown at those reads.", scanned.missing_available));
    }
    let overflowed = scanned.stock.overflowed
        + scanned.daily.overflowed
        + scanned.daily_ic.overflowed
        + scanned.balance.overflowed
        + scanned.logistics_stock.overflowed
        + scanned.available.overflowed;
    if overflowed > 0 {
        warnings.push(format!(
            "{overflowed} recorded totals do not fit a 64-bit amount; they are shown as unknown, never clamped."
        ));
    }
    if scanned.other_kinds > 0 {
        warnings.push(format!(
            "{} naval, refit or railway-gun line reads make equipment whose daily output is not established, so it, its IC and its cost are unknown at those reads.",
            scanned.other_kinds
        ));
    }
    if scanned.reads.is_empty() {
        warnings.push(format!(
            "{}: no recorded stockpile in this branch",
            params.tag
        ));
    }
    if let Some((catalogue, _)) = &content {
        warnings.extend(catalogue.refusal_lines());
    }
    Ok(EquipmentStockpileReply {
        branch_id: params.branch_id.clone(),
        tag: params.tag.clone(),
        campaign_key: engine_allocation::string(source.campaign_key())?,
        decimals: equipment::EQUIPMENT_DECIMALS,
        types,
        variants,
        reads: scanned.reads.clone(),
        stock_hours: scanned.stock.spans.clone(),
        line_hours: scanned.daily.spans.clone(),
        line_reads: scanned.line_reads.clone(),
        logistics,
        logistics_reads: scanned.logistics_reads.clone(),
        balance_hours: scanned.balance.spans.clone(),
        ai_hours: scanned.ai.spans.clone(),
        need_bound_raw: NEED_BOUND_RAW.to_string(),
        warnings,
    })
}

#[cfg(test)]
mod tests;
