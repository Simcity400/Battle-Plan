//! One country's equipment stockpile over a branch, for the Equipment stockpile
//! widget: what it holds of each variant, what its production lines make of it
//! per week, and each variant's equipment type (archetype) and domain from content.
//!
//! **In stock** is the recorded `Stockpiles` pool (`CProductionStatus` +0x200,
//! fixed5, `engine::equipment_entry`), compared exactly with the save by the
//! fresh-save oracle (`recorder::oracle::production::compare_stockpiles`). A
//! variant listed twice holds their sum; one not listed holds none.
//!
//! **Produced per week** is the sum, over the country's military production
//! lines making a variant, of each line's units per week by the game's weekly
//! formula ([`equipment::weekly_units_raw`]). A line whose rate cannot be computed
//! (no positive cost) makes that variant's weekly total unknown for the read.
//! Naval, refit and railway-gun lines are not military lines: whether and at what
//! rate they deliver to the stockpile is not established, so a variant one of them
//! produces has an unknown weekly total for the read, never zero. Naval, refit and railway-gun lines make
//! ships and guns, not stockpiled equipment, and are left out.
//!
//! **The surplus or shortage** is the recorded `LogisticsEquipment` balance: series
//! 16 of `CLogisticsStatus` (`CCountry+0xf98`), the value the Logistics screen's
//! "Stored" column shows (`LOGISTICS_IN_STOCK`, "Total amount of Equipment
//! surplus/deficit"), with series 15, the stock the same daily update counted
//! (`engine::clogistics_status`, table read in daily slot 0, just after the update).
//! `CLogisticsStatus::PostDailyUpdate@14135ffa0` writes the balance per archetype as
//! `Avail − clamp(Demand + PoolDifference)`:
//!
//! - `Avail` (`Logistics_GetAvailableEquipmentStockpile@14135dbc0`) is series 15
//!   (for an archetype of category bit 0, convoys, the live pool instead), plus the
//!   supply system's truck terms for a `supply_truck` archetype (`CEquipmentType
//!   +0x3eb`, token 0x4d63) or its train term for a category-bit-31 archetype,
//!   clamped at zero;
//! - `Demand` (`ComputeArchetypeDemandAdjustment@14135de20`) sums training lines'
//!   missing equipment, the army's reinforcement requests, garrisons, the cached air
//!   requirement, operations and trucks and trains; `PoolDifference`
//!   (`GetArchetypePoolDifference@14135c650`) is outgoing minus incoming lend-lease
//!   pools; the sum saturates at ±`0x53e2d620fb40`.
//!
//! The game writes the balance only for countries `IsCountryAIControlled@1401e1fe0`
//! calls not AI-controlled, so a complete table without rows for the country means
//! it was AI-controlled at that read (`ai_hours`), not that it holds nothing.
//!
//! **What is needed** is derived from the same record: for an archetype that is not
//! a convoy type, not a `supply_truck` and not a train type, `Avail` is exactly
//! `max(0, stock)`, so `max(0, stock) − balance` is exactly `clamp(Demand +
//! PoolDifference)`: what divisions, training, garrisons, air wings and operations
//! need plus net outgoing lend-lease. For the other archetypes `Avail` holds terms
//! the recording does not establish at the daily calculation (the live convoy
//! pool and supply adjustments), so their need is not derived
//! ([`StockType::need_note`]). Hourly TrainInventory records allocated trucks and
//! required trains; it does not bind them to the daily balance, nor record the
//! truck subtractor at supply+0xc8 (14135DBC0). A value at the saturation bound
//! is not a real sum.
//!
//! Stockpiles and production lines are read in one daily slot and carried in
//! between (`metadata::slot_read`); both come from the same read, so their values
//! always describe one game state. The logistics balance and its stock come from
//! their own record, read in slot 0, so a balance is always shown with the stock
//! that record counted. Values start only at a read, are carried over
//! the following hours of the same country identity, and end at a gap, an
//! incomplete table or a change of identity (`read_runs`).
//!
//! **Equipment types** come from content (`installed_content::equipment`): a variant's type is
//! its recorded equipment token, resolved to the authored key under the lexer's
//! fold, and grouped by that type's archetype (`CEquipmentType+0x4d8`), as the
//! Logistics screen groups rows (by archetype pointer, never substituting one
//! archetype for another). Types are listed in the equipment database's order.
//! A variant's type never changes, so any read of the variant table names it.
//!
//! **Artwork is not shown yet.** The Logistics screen draws an archetype's row with
//! the country's best variant for it (`CProductionStatus::FindBestVariantForArchetype
//! @140e54540`, which scans the best-variant candidates at `CProductionStatus+0xB8`),
//! drawn with that variant's icon (`BindEquipmentVariantIcon@140b371f0`, starting
//! from its icon entry at `CEquipmentVariant+0x450`). The recording holds both:
//! the candidates are `AvailableEquipmentVariants` roster kind 2 and each
//! variant's icon entry is in `VariantDefinitions`. What is missing is the product
//! code: the shared equipment icon rule and the best-variant ranking still need
//! to be wired in.

use super::{
    CountryParams, LoadContext, Product,
    equipment::{self, Domain, Texts, VariantCatalogue},
};
use crate::{
    Source, catalogue,
    metadata::{link_slot, slot_read, tagged_country, validate_campaign},
    products::country_groups::HourSpan,
    read_runs::{AmountRun, ReadRuns},
};
use installed_content::equipment::Database;
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct EquipmentStockpile;

const CACHED_COUNTRIES: usize = 8;

pub use super::equipment::StockVariantName;

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
    /// Units a week from its military production lines, fixed5; no run inside a
    /// known span means none.
    pub weekly: Vec<AmountRun>,
}

/// An equipment type (archetype) as content names it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockType {
    pub key: String,
    /// The localised key; null when content does not name it.
    pub name: Option<String>,
    pub domain: Domain,
    /// Why its need cannot be derived from the logistics record; null when
    /// `max(0, stock) − balance` is exactly what it needs.
    pub need_note: Option<String>,
}

pub use super::equipment::UNRESOLVED_MARK;

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
    /// Hours the production lines were read, ascending, for dating a weekly rate
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

#[derive(Clone, Debug)]
pub struct ScannedStockpile {
    pub stock: ReadRuns<u64>,
    pub weekly: ReadRuns<u64>,
    pub reads: Vec<i32>,
    /// Each variant's naming operands, with where they are known.
    pub variants: VariantCatalogue,
    /// Lines of the country whose cost gives no weekly output, per read.
    pub costless: usize,
    /// Naval, refit and railway-gun line reads of the country, whose weekly
    /// output is unknown.
    pub other_kinds: usize,
    /// Logistics balance and stock by archetype token.
    pub balance: ReadRuns<u32>,
    pub logistics_stock: ReadRuns<u32>,
    /// Spans the country was AI-controlled (a key-less run set).
    pub ai: ReadRuns<u32>,
    pub logistics_reads: Vec<i32>,
    /// Hours the country's production lines were read.
    pub line_reads: Vec<i32>,
    /// Each archetype token's spelling, from the reads that named it.
    pub logistics_tokens: BTreeMap<u32, String>,
}

/// `Demand + PoolDifference` saturates at this magnitude in
/// `CLogisticsStatus::PostDailyUpdate@14135ffa0` (`engine::clogistics_status`; the
/// same bound as `engine::country_stability`'s ring saturation).
pub const NEED_BOUND_RAW: i64 = 0x53e2_d620_fb40;

/// Scans one branch for one country's stockpile, production and variant names.
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
    let mut result = ScannedStockpile {
        stock: ReadRuns::new(),
        weekly: ReadRuns::new(),
        reads: Vec::new(),
        variants: VariantCatalogue::default(),
        costless: 0,
        other_kinds: 0,
        balance: ReadRuns::new(),
        logistics_stock: ReadRuns::new(),
        ai: ReadRuns::new(),
        logistics_reads: Vec::new(),
        line_reads: Vec::new(),
        logistics_tokens: BTreeMap::new(),
    };
    let mut last: Option<((u32, u32), i32)> = None;
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancel());
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
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
            let hour = frame.hour;
            let tables = decoded
                .complete::<tables::Countries>()
                .ok()
                .zip(decoded.complete::<tables::CountryLinks>().ok());
            let identity = tables.and_then(|(countries, links)| {
                tagged_country(countries, links, tag)
                    .found()
                    .map(|(country, slot)| (slot, country.id))
            });
            let continues = identity.is_some_and(|identity| {
                last.is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour))
            });
            last = identity.map(|identity| (identity, hour));
            // Variant names and types, with where they are known (the shared catalogue).
            result.variants.observe(decoded);
            let stock = decoded.complete::<tables::Stockpiles>().ok();
            let lines = decoded.complete::<tables::ProductionLines>().ok();
            // Each table ends its own values where it is missing; the other goes on.
            if !continues || stock.is_none() {
                result.stock.close();
            }
            if !continues || lines.is_none() {
                result.weekly.close();
            }
            let logistics = decoded.complete::<tables::LogisticsEquipment>().ok();
            if !continues || logistics.is_none() {
                result.balance.close();
                result.logistics_stock.close();
                result.ai.close();
            }
            let (Some((slot, _)), Some((_, links))) = (identity, tables) else {
                continue;
            };
            match logistics {
                Some(rows)
                    if slot_read(&decoded.metrics, TableId::LogisticsEquipment, decoded.hour) =>
                {
                    let own: Vec<_> = rows
                        .iter()
                        .filter(|row| link_slot(links, row.country) == Some(slot))
                        .collect();
                    if own.is_empty() {
                        // The game computed no balance: the country was AI-controlled.
                        result.ai.read(hour, []);
                        result.balance.close();
                        result.logistics_stock.close();
                    } else {
                        result.logistics_reads.push(hour);
                        if let Ok(lexical) = decoded.complete::<tables::LexicalTokens>() {
                            for row in &own {
                                if let Ok(index) =
                                    lexical.binary_search_by_key(&row.token, |token| token.token)
                                {
                                    result.logistics_tokens.entry(row.token).or_insert_with(|| {
                                        String::from_utf8_lossy(&lexical[index].literal)
                                            .into_owned()
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
                        result.ai.close();
                    }
                }
                _ => {
                    result.balance.carry(hour);
                    result.logistics_stock.carry(hour);
                    result.ai.carry(hour);
                }
            }
            if !slot_read(&decoded.metrics, TableId::Stockpiles, decoded.hour) {
                // Carried: the last read still holds, but only after a read of this identity.
                result.stock.carry(hour);
                result.weekly.carry(hour);
                continue;
            }
            if let Some(stock) = stock {
                result.reads.push(hour);
                result.stock.read(
                    hour,
                    stock
                        .iter()
                        .filter(|row| link_slot(links, row.country) == Some(slot))
                        .map(|row| (row.variant_handle, Some(row.amount_raw))),
                );
            }
            if let Some(lines) = lines {
                result.line_reads.push(hour);
                let mut weekly = Vec::new();
                for line in lines
                    .iter()
                    .filter(|line| link_slot(links, line.country) == Some(slot))
                {
                    // A rate that cannot be computed makes the variant's weekly
                    // total unknown at this read, never zero or a partial sum. Only a
                    // military line's rate is the military formula; what a naval,
                    // refit or railway-gun line delivers is not established.
                    let units = if line.kind == engine::cequipment_production_line::KIND_MILITARY {
                        let units = equipment::weekly_units_raw(line.speed_raw, line.cost_raw);
                        if units.is_none() {
                            result.costless += 1;
                        }
                        units
                    } else {
                        result.other_kinds += 1;
                        None
                    };
                    weekly.push((line.variant, units));
                }
                result.weekly.read(hour, weekly);
            }
        }
    }
    Ok(result)
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<((u64, String), Arc<ScannedStockpile>)>,
}

use super::equipment::{archetype_of, equipment_warnings};

/// Why an archetype's need cannot be derived from its logistics record, when its
/// available stock holds terms not established at the daily calculation
/// (`Logistics_GetAvailableEquipmentStockpile@14135dbc0`); none when `Avail`
/// is exactly `max(0, stock)`.
fn need_note(types: &Database, archetype: &str) -> Result<Option<String>, String> {
    let kind = types
        .named(archetype)
        .ok_or_else(|| format!("{archetype} is not authored equipment"))?;
    let category = types.category(kind);
    Ok(if category & 1 != 0 {
        Some("The game counts its live convoy pool here, not the recorded stock, so what is needed cannot be derived.".to_owned())
    } else if kind.supply_truck {
        Some("The game's daily balance uses truck adjustments that the recording does not establish at that update, so what is needed cannot be derived.".to_owned())
    } else if category & (1 << 31) != 0 {
        Some("The hourly train requirement does not establish the trains used by the game's daily balance update, so what is needed cannot be derived.".to_owned())
    } else {
        None
    })
}

/// The same archetype name for the recorded reply and offline preparation.
pub(crate) fn type_name(catalogue: &crate::catalogue::Catalogue, key: &str) -> Option<String> {
    let name = catalogue.text(key).filter(|name| !name.is_empty());
    if name.is_none() {
        catalogue.note(crate::content_check::Finding::new(
            crate::content_check::Kind::Fallback,
            key,
            "equipment type name is unavailable; its key is shown",
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
            EquipmentStockpileReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

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
                .map(|(_, scanned)| Arc::clone(scanned))
        };
        let scanned = match cached {
            Some(scanned) => scanned,
            None => {
                let scanned = Arc::new(scan(
                    &source.replay(),
                    params.branch(),
                    &params.tag,
                    cancelled,
                )?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != key);
                cache.entries.push_back((key, Arc::clone(&scanned)));
                while cache.entries.len() > CACHED_COUNTRIES {
                    cache.entries.pop_front();
                }
                scanned
            }
        };
        let mut warnings = Vec::new();
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
            .chain(scanned.weekly.runs.keys())
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
            // A variant's type never changes; its latest spelling names it.
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
                weekly: scanned.weekly.runs.get(id).cloned().unwrap_or_default(),
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
                    None => Some(
                        "Content is unavailable, so what is needed cannot be derived.".to_owned(),
                    ),
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
                "{untyped_count} recorded variants or archetypes have an equipment type the content does not resolve; they are shown by their token and every total they could belong to is unknown ({}).",
                untyped.into_iter().collect::<Vec<_>>().join("; ")
            ));
        }
        if scanned.costless > 0 {
            warnings.push(format!(
                "{} production line reads have no positive cost, so their variant's weekly output is unknown at those reads (the game shows only a placeholder rate for them).",
                scanned.costless
            ));
        }
        if scanned.logistics_reads.is_empty() && scanned.ai.spans.is_empty() {
            warnings.push("The surplus or shortage is not in this recording: it has no logistics record (LogisticsEquipment).".to_owned());
        }
        warnings.push("Equipment art is not shown in this widget yet.".to_owned());
        let overflowed = scanned.stock.overflowed
            + scanned.weekly.overflowed
            + scanned.balance.overflowed
            + scanned.logistics_stock.overflowed;
        if overflowed > 0 {
            warnings.push(format!(
                "{overflowed} recorded totals do not fit a 64-bit amount; they are shown as unknown, never clamped."
            ));
        }
        if scanned.other_kinds > 0 {
            warnings.push(format!(
                "{} naval, refit or railway-gun line reads make equipment whose weekly output is not established, so it is unknown at those reads.",
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
            campaign_key: crate::map_settings::campaign_key(&source.replay()),
            decimals: equipment::EQUIPMENT_DECIMALS,
            types,
            variants,
            reads: scanned.reads.clone(),
            stock_hours: scanned.stock.spans.clone(),
            line_hours: scanned.weekly.spans.clone(),
            line_reads: scanned.line_reads.clone(),
            logistics,
            logistics_reads: scanned.logistics_reads.clone(),
            balance_hours: scanned.balance.spans.clone(),
            ai_hours: scanned.ai.spans.clone(),
            need_bound_raw: NEED_BOUND_RAW.to_string(),
            warnings,
        })
    }
}

#[cfg(test)]
mod tests;
