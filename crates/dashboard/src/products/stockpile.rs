//! A country's equipment stockpile (family 15), answered from the recording's own tables: what it
//! holds of each design, what its military production lines make of it a day, and the game's
//! Logistics balance per equipment archetype.
//!
//! **In stock** is `StockByVariant.amount` (the recorder's sum of the slot's `Stockpiles` positions
//! holding a design, fixed5); a design listed at zero is held at zero, one not listed holds none.
//! **Output a day** sums, over the lines making a design, each line's `speed × 100000 ÷ cost`
//! truncated on its own ([`daily_units_raw`]); a line without a positive cost leaves its design's
//! output unknown, and so does a naval, refit or railway-gun line, whose delivery to the stockpile
//! is not established. **The surplus or shortage** is `LogisticsEquipment.balance_raw`, with the
//! stock that update counted and the available operand of the same write: a need is
//! `available − balance`, unknown where the operand was not observed. The game writes the balance
//! only for countries that are not AI-controlled: a complete table without the country's rows is
//! an AI-controlled read, never a country holding nothing.
//!
//! Products: [`at`] (one holder at an hour, its moving numbers live cells), [`catalogue`] (the
//! holders' designs, types, reads and peaks over the branch, and each group's line over a view,
//! from the stock and logistics histories read once per holder and run, [`history`]) and
//! [`artwork`] (an archetype's best design and a design's own picture). Names and types come from
//! the recorded content: an archetype is the design's equipment token resolved by the content's
//! equipment database, refused by name where it does not resolve.

pub mod artwork;
pub mod at;
pub mod catalogue;
mod history;
#[cfg(test)]
mod tests;

pub use artwork::StockpileArtwork;
pub use at::EquipmentStockpileAt;
pub use catalogue::{EquipmentStockpileCatalogue, EquipmentStockpileChart};

use dashboard_base::products::LoadContext;
use crate::{
    Source,
    catalogue::{self as loaded, Catalogue},
};
use super::at_hour::Read;
use dashboard_query::Query;
use dashboard_text::variant_names::{self, Tags, Texts, raw_variant, variant_names};
use installed_content_core::equipment::Database;
use replay_data::rows::{TableId, VariantModuleRow, VariantUpgradeRow, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use ts_rs::TS;

/// Decimal places of recorded equipment amounts and line cost and speed: fixed5.
pub const DECIMALS: u8 = 5;
const _: () = assert!(engine_native::equipment_entry::FIXED_POINT_SCALE == 100_000);

/// `Demand + PoolDifference` saturates at this magnitude in the daily logistics update: a derived
/// need at it is not a real sum.
pub const NEED_BOUND_RAW: i64 = 0x53e2_d620_fb40;

/// Starts the key of an equipment type the content does not resolve, before the recorded token's
/// spelling (or the design's handle without one): kept apart, never merged into another type.
pub const UNRESOLVED_MARK: &str = ":";
/// The arm of types the content does not resolve.
pub const UNRESOLVED_ARM: &str = "unresolved";

/// Units a line makes a day, fixed5, by the game's per-day formula: `speed × 100000 ÷ cost`,
/// truncated; none without a positive cost (the game shows a placeholder, not a rate) or past i64.
pub fn daily_units_raw(speed_raw: i64, cost_raw: i64) -> Option<i64> {
    if cost_raw <= 0 {
        return None;
    }
    i64::try_from(i128::from(speed_raw) * 100_000 / i128::from(cost_raw)).ok()
}

/// Which arm an equipment type serves: production spending's, one shape in the contract.
pub use super::production_spending::Domain;

/// An equipment type (archetype) as the content names it.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockType {
    pub key: String,
    /// The localised key; null when the content does not name it.
    pub name: Option<String>,
    pub domain: Domain,
    /// Why a need cannot be derived for this type without the available operand of its balance
    /// write; none for an ordinary type.
    pub need_note: Option<String>,
}

/// A design's names from an hour on: production spending's, one shape in the contract.
pub use super::production_spending::StockVariantName;

/// What the family reads of the recorded content: the reader's catalogue and its equipment
/// database, each or why not. Without them stock still shows, by design under unnamed types.
#[derive(Clone)]
pub struct Content {
    pub catalogue: Result<Arc<Catalogue>, String>,
    pub units: Result<Arc<Database>, String>,
    pub warnings: Vec<String>,
}

impl Content {
    pub fn load(source: &Source, load: &LoadContext) -> io::Result<Self> {
        let Some(options) = load.content.as_ref() else {
            return Ok(Self::none("Equipment names and types need the game folders."));
        };
        let catalogue = loaded::catalogue(source, options)?;
        let mut warnings = Vec::new();
        let units = loaded::equipment_units(&catalogue).map_err(|error| format!("Equipment types are unavailable: {error}"));
        match &units {
            Ok(units) => warnings.extend(units.misreads.iter().cloned()),
            Err(why) => warnings.push(why.clone()),
        }
        Ok(Self { catalogue: Ok(catalogue), units, warnings })
    }

    pub fn none(why: &str) -> Self {
        Self { catalogue: Err(why.to_owned()), units: Err(why.to_owned()), warnings: vec![why.to_owned()] }
    }

    pub fn texts(&self) -> Option<&dyn Texts> {
        self.catalogue.as_ref().ok().map(|catalogue| catalogue.as_ref() as &dyn Texts)
    }

    fn type_name(&self, key: &str) -> Option<String> {
        self.texts().and_then(|texts| texts.text(key))
    }
}

/// A token's type through the equipment database: its archetype key, database place and arm.
pub fn archetype_of(units: &Database, token: &str) -> Result<(String, usize, Domain), String> {
    let kind = units.named(token).ok_or_else(|| format!("{token} is not authored equipment"))?;
    let ordinal = kind.archetype.ok_or_else(|| format!("equipment {token} has no known archetype"))?;
    let archetype = &units.equipment[ordinal];
    Ok((archetype.name_str().map_err(|error| error.to_string())?.to_owned(), ordinal, Domain::of(units.category(archetype))))
}

/// Why an archetype's need cannot be derived without its write's available operand: a convoy,
/// supply-truck or train archetype counts terms beyond its stock; none for an ordinary one.
pub fn need_note(units: &Database, archetype: &str) -> Result<Option<String>, String> {
    let kind = units.named(archetype).ok_or_else(|| format!("{archetype} is not authored equipment"))?;
    let category = units.category(kind);
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

/// The archetypes a reply meets, with the types the content does not resolve.
#[derive(Default)]
pub struct Types {
    found: BTreeMap<String, (usize, Domain)>,
    resolved: BTreeMap<String, Result<(String, usize, Domain), String>>,
    untyped: BTreeSet<String>,
    untyped_count: usize,
}

impl Types {
    fn resolve(&mut self, content: &Content, token: &str) -> Option<String> {
        let units = content.units.as_ref().ok()?;
        let resolved = self.resolved.entry(token.to_owned()).or_insert_with(|| archetype_of(units, token));
        match resolved {
            Ok((archetype, ordinal, domain)) => {
                self.found.insert(archetype.clone(), (*ordinal, *domain));
                Some(archetype.clone())
            }
            Err(reason) => {
                self.untyped.insert(reason.clone());
                self.untyped_count += 1;
                None
            }
        }
    }

    /// A design's archetype from its token; none where the content does not resolve it or no read
    /// spelled its token.
    pub fn variant(&mut self, content: &Content, token: &str) -> Option<String> {
        if token.is_empty() {
            return None;
        }
        self.resolve(content, token)
    }

    /// A logistics token's key: its archetype, else the unresolved mark and its spelling.
    pub fn logistics(&mut self, content: &Content, spelling: &str) -> String {
        self.resolve(content, spelling).unwrap_or_else(|| format!("{UNRESOLVED_MARK}{spelling}"))
    }

    pub fn domain(&self, archetype: &str) -> Option<Domain> {
        self.found.get(archetype).map(|(_, domain)| *domain)
    }

    /// Every archetype met, in the equipment database's order, with the warnings of the types the
    /// content does not resolve.
    pub fn finish(self, content: &Content, warnings: &mut Vec<String>) -> Vec<StockType> {
        let mut types: Vec<_> = self.found.into_iter().collect();
        types.sort_by_key(|(_, (ordinal, _))| *ordinal);
        let marked: Vec<&str> = types.iter().map(|(key, _)| key.as_str()).filter(|key| key.starts_with(UNRESOLVED_MARK)).collect();
        if !marked.is_empty() {
            warnings.push(format!(
                "Equipment types whose key starts with \"{UNRESOLVED_MARK}\" are not supported: the widget marks unresolved types that way, so they could be mistaken for one ({}).",
                marked.join(", ")
            ));
        }
        if self.untyped_count > 0 {
            warnings.push(format!(
                "{} recorded designs or archetypes have an equipment type the content does not resolve; their type titles and every total they could belong to are unavailable ({}).",
                self.untyped_count,
                self.untyped.into_iter().collect::<Vec<_>>().join("; ")
            ));
        }
        types
            .into_iter()
            .map(|(key, (_, domain))| StockType {
                name: content.type_name(&key),
                need_note: match &content.units {
                    Ok(units) => need_note(units, &key).unwrap_or_else(Some),
                    Err(_) => Some("Content is unavailable, so what is needed cannot be derived.".to_owned()),
                },
                key,
                domain,
            })
            .collect()
    }
}

/// A design as one hour's tables name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    /// False where it has no definition at the hour; the rest is then empty.
    pub known: bool,
    pub token: String,
    pub name: Option<String>,
    pub design: Option<String>,
}

/// The tables naming reads whole; `VariantDefinitions`, `VariantModules` and `VariantUpgrades` are
/// read by the designs asked ([`design_reads`]).
pub const NAME_TABLES: [TableId; 4] = [TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::LexicalTokens];

/// Each of `variants` as `frame` names it, and each of `tokens` as its LexicalTokens spells it;
/// the refusals met are counted by reason. The one function the family names designs through.
pub fn named_in(frame: &DecodedFrame, variants: &[u64], tokens: &BTreeSet<u32>, content: &Content, refused: &mut BTreeMap<String, usize>) -> (BTreeMap<u64, Named>, BTreeMap<u32, String>) {
    let unknown = || Named { known: false, token: String::new(), name: None, design: None };
    let tables = (frame.complete::<tables::Countries>(), frame.complete::<tables::CountryLinks>(), frame.complete::<tables::LexicalTokens>(), frame.complete::<tables::VariantDefinitions>());
    let (Ok(countries), Ok(links), Ok(lexical), Ok(definitions)) = tables else {
        *refused.entry("Design definitions or their names were not read at this hour".to_owned()).or_default() += variants.len();
        return (variants.iter().map(|&id| (id, unknown())).collect(), BTreeMap::new());
    };
    let spelled = tokens
        .iter()
        .filter_map(|&token| {
            let at = lexical.binary_search_by_key(&token, |row| row.token).ok()?;
            Some((token, std::str::from_utf8(&lexical[at].literal).ok()?.to_owned()))
        })
        .collect();
    let tags = Tags::new(countries, links, frame.complete::<tables::CountryAppearance>().ok());
    let texts = content.texts();
    // A design's fingerprint needs its modules and upgrades read complete with it; else it has none.
    let fitted = match (frame.complete::<tables::VariantModules>(), frame.complete::<tables::VariantUpgrades>()) {
        (Ok(modules), Ok(upgrades)) => Some((modules, upgrades)),
        _ => None,
    };
    let spell = |token: u32| -> Result<String, String> {
        if token == 0 {
            return Ok(String::new());
        }
        let at = lexical.binary_search_by_key(&token, |row| row.token).map_err(|_| format!("lexical tokens lacks equipment token {token}"))?;
        variant_names::text(&lexical[at].literal, &format!("equipment lexer token {token}"))
    };
    let named = variants
        .iter()
        .map(|&id| {
            let Ok(at) = definitions.binary_search_by_key(&id, |row| row.id) else { return (id, unknown()) };
            let mut raw = raw_variant(&definitions[at], frame.hour, &tags, lexical);
            for (_, why) in &raw.refusals {
                *refused.entry(why.clone()).or_default() += 1;
            }
            if let Some((modules, upgrades)) = fitted {
                let own: Vec<&VariantModuleRow> = of_variant(modules, id, |row: &VariantModuleRow| row.variant).iter().collect();
                let upgraded: Vec<&VariantUpgradeRow> = of_variant(upgrades, id, |row: &VariantUpgradeRow| row.variant).iter().collect();
                variant_names::with_design(&mut raw, &own, &upgraded, &spell);
            }
            let name = variant_names(&raw, texts).0;
            (id, Named { known: true, design: raw.design, name, token: raw.token })
        })
        .collect();
    (named, spelled)
}

/// The reads naming `variants` needs beside [`NAME_TABLES`]: their definitions, and their modules
/// and upgrades (keyed by design above ordinal) for their fingerprints.
pub fn design_reads(variants: impl IntoIterator<Item = u64>) -> [Read; 3] {
    let mut ids: Vec<u128> = variants.into_iter().map(u128::from).collect();
    ids.sort_unstable();
    ids.dedup();
    let ranges: Vec<(u128, u128)> = ids.iter().map(|&id| (id << 32, id << 32 | u128::from(u32::MAX))).collect();
    [
        Read::these(TableId::VariantDefinitions, ids),
        Read { table: TableId::VariantModules, keys: super::at_hour::Keys::Ranges(ranges.clone()) },
        Read { table: TableId::VariantUpgrades, keys: super::at_hour::Keys::Ranges(ranges) },
    ]
}

/// The rows of one design among `rows` (ascending by design).
fn of_variant<T>(rows: &[T], variant: u64, key: impl Fn(&T) -> u64) -> &[T] {
    &rows[rows.partition_point(|row| key(row) < variant)..rows.partition_point(|row| key(row) <= variant)]
}

/// [`named_in`] at `hour` of `query`'s timeline: the naming tables and the designs' definitions.
pub fn name_at(query: &Query<'_>, hour: i32, variants: &[u64], tokens: &BTreeSet<u32>, content: &Content, refused: &mut BTreeMap<String, usize>) -> io::Result<(BTreeMap<u64, Named>, BTreeMap<u32, String>)> {
    let mut reads: Vec<Read> = NAME_TABLES.into_iter().map(Read::all).collect();
    reads.extend(design_reads(variants.iter().copied()));
    let frame = super::at_hour::frame(query, hour, &reads, "Stockpile")?;
    Ok(named_in(&frame, variants, tokens, content, refused))
}
