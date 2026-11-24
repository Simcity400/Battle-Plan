
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
use dashboard_base::{Source, catalogue::{self as loaded, Catalogue}};
use dashboard_base::products::at_hour::Read;
use dashboard_query::Query;
use dashboard_text::variant_names::{self, Tags, Texts, raw_variant, variant_names};
use content_reader_key_names::equipment::Database;
use replay_data::rows::{TableId, VariantModuleRow, VariantUpgradeRow, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const DECIMALS: u8 = 5;
const _: () = assert!(engine_native_parts::equipment_entry::FIXED_POINT_SCALE == 100_000);

pub const NEED_BOUND_RAW: i64 = 0x53e2_d620_fb40;

pub const UNRESOLVED_MARK: &str = ":";
pub const UNRESOLVED_ARM: &str = "unresolved";

pub fn daily_units_raw(speed_raw: i64, cost_raw: i64) -> Option<i64> {
    if cost_raw <= 0 {
        return None;
    }
    i64::try_from(i128::from(speed_raw) * 100_000 / i128::from(cost_raw)).ok()
}

pub use super::production_spending::Domain;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct StockType {
    pub key: String,
    #[doc = " The localised key; null when the content does not name it."]
    pub name: Option<String>,
    pub domain: Domain,
    #[doc = " Why a need cannot be derived for this type without the available operand of its balance"]
    #[doc = " write; none for an ordinary type."]
    pub need_note: Option<String>,
}

pub use super::production_spending::StockVariantName;

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

pub fn archetype_of(units: &Database, token: &str) -> Result<(String, usize, Domain), String> {
    let kind = units.named(token).ok_or_else(|| format!("{token} is not authored equipment"))?;
    let ordinal = kind.archetype.ok_or_else(|| format!("equipment {token} has no known archetype"))?;
    let archetype = &units.equipment[ordinal];
    Ok((archetype.name_str().map_err(|error| error.to_string())?.to_owned(), ordinal, Domain::of(units.category(archetype))))
}

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

    pub fn variant(&mut self, content: &Content, token: &str) -> Option<String> {
        if token.is_empty() {
            return None;
        }
        self.resolve(content, token)
    }

    pub fn logistics(&mut self, content: &Content, spelling: &str) -> String {
        self.resolve(content, spelling).unwrap_or_else(|| format!("{UNRESOLVED_MARK}{spelling}"))
    }

    pub fn domain(&self, archetype: &str) -> Option<Domain> {
        self.found.get(archetype).map(|(_, domain)| *domain)
    }

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    pub known: bool,
    pub token: String,
    pub name: Option<String>,
    pub design: Option<String>,
}

pub const NAME_TABLES: [TableId; 4] = [TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::LexicalTokens];

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

pub fn design_reads(variants: impl IntoIterator<Item = u64>) -> [Read; 3] {
    let mut ids: Vec<u128> = variants.into_iter().map(u128::from).collect();
    ids.sort_unstable();
    ids.dedup();
    let ranges: Vec<(u128, u128)> = ids.iter().map(|&id| (id << 32, id << 32 | u128::from(u32::MAX))).collect();
    [
        Read::these(TableId::VariantDefinitions, ids),
        Read { table: TableId::VariantModules, keys: dashboard_base::products::at_hour::Keys::Ranges(ranges.clone()) },
        Read { table: TableId::VariantUpgrades, keys: dashboard_base::products::at_hour::Keys::Ranges(ranges) },
    ]
}

fn of_variant<T>(rows: &[T], variant: u64, key: impl Fn(&T) -> u64) -> &[T] {
    &rows[rows.partition_point(|row| key(row) < variant)..rows.partition_point(|row| key(row) <= variant)]
}

pub fn name_at(query: &Query<'_>, hour: i32, variants: &[u64], tokens: &BTreeSet<u32>, content: &Content, refused: &mut BTreeMap<String, usize>) -> io::Result<(BTreeMap<u64, Named>, BTreeMap<u32, String>)> {
    let mut reads: Vec<Read> = NAME_TABLES.into_iter().map(Read::all).collect();
    reads.extend(design_reads(variants.iter().copied()));
    let frame = dashboard_base::products::at_hour::frame(query, hour, &reads, "Stockpile")?;
    Ok(named_in(&frame, variants, tokens, content, refused))
}
