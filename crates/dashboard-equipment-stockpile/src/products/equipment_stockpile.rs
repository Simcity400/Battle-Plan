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
//! that record counted. Values start only at a read, hold over the following
//! hours of the same country identity, and end at a gap, an incomplete table or
//! a change of identity: each stream's coverage ([`emit`]).
//!
//! **Readers.** [`at`] answers one hour from each stream's latest keyframe and
//! the deltas after it; [`catalogue`] replays each holder's streams once per
//! publication for its variants, peaks, coverage, reads and chart lines.
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

use dashboard_base::{
    products::{
        Product,
        country_groups::HourSpan,
        equipment::{Domain, Texts as _},
    },
    read_runs::AmountRun,
};
use installed_content_core::equipment::Database;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod artwork;
pub mod at;
pub mod catalogue;
pub mod columnar;
pub mod content;
pub mod emit;
mod read;
pub mod served;

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

/// `Demand + PoolDifference` saturates at this magnitude during the daily
/// logistics update (`engine_land_inputs::clogistics_status`; the same bound as
/// `engine_stability::country_stability`'s ring saturation).
pub const NEED_BOUND_RAW: i64 = 0x53e2_d620_fb40;

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

/// One holder's stockpile at one hour ([`at`]).
pub struct EquipmentStockpileAt;

impl Product for EquipmentStockpileAt {
    const KEY: &'static str = "equipment-stockpile-at";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::AT);
    type Params = at::StockpileAtParams;
    type Reply = EquipmentStockpileReply;

    fn types() -> Vec<String> {
        vec![
            at::StockpileAtParams::decl(),
            AmountRun::decl(),
            Domain::decl(),
            StockVariantName::decl(),
            StockVariant::decl(),
            StockType::decl(),
            TypeLogistics::decl(),
            EquipmentStockpileReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }
}

/// The holders' variants, logistics types, coverage, reads and groups' peaks ([`catalogue`]).
pub struct EquipmentStockpileCatalogue;

impl Product for EquipmentStockpileCatalogue {
    const KEY: &'static str = "equipment-stockpile-catalogue";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> =
        Some(&served::CATALOGUE);
    type Params = catalogue::StockpileCatalogueParams;
    type Reply = catalogue::StockpileCatalogueReply;

    fn types() -> Vec<String> {
        vec![
            catalogue::StockpileHolder::decl(),
            catalogue::StockpileCatalogueParams::decl(),
            catalogue::StockpileChoicePeak::decl(),
            catalogue::StockpileCatalogueVariant::decl(),
            catalogue::StockpileCatalogueLogistics::decl(),
            catalogue::StockpileCatalogueReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }
}

/// The holders' groups as chart lines over a view ([`catalogue::chart`]).
pub struct EquipmentStockpileChart;

impl Product for EquipmentStockpileChart {
    const KEY: &'static str = "equipment-stockpile-chart";
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::CHART);
    type Params = catalogue::StockpileChartParams;
    type Reply = catalogue::StockpileChartReply;

    fn types() -> Vec<String> {
        vec![
            catalogue::StockpileMeasure::decl(),
            catalogue::StockpileLine::decl(),
            catalogue::StockpileChartParams::decl(),
            catalogue::StockpileChartLine::decl(),
            catalogue::StockpileChartReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }
}

#[cfg(test)]
mod tests;
