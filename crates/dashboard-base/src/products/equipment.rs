//! Equipment facts every equipment product shares: units per week as the game's
//! production view writes them, each variant's naming history (named by
//! [`crate::variant_names`]), and an equipment type's domain (land, air or naval)
//! by the engine's own classifiers. Products about equipment (stockpile,
//! production lines, production) read them here, never a copy.

pub mod production;

use crate::{Source, catalogue::Catalogue, metadata::slot_read};
use replay::{
    frame::DecodedFrame,
    rows::{
        LexicalTokenRow, TableId, VariantDefinitionRow, VariantModuleRow, VariantUpgradeRow, tables,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use ts_rs::TS;

use crate::variant_names::text;
pub use crate::variant_names::{
    RawVariant, Tags, Texts, VariantOrder, compare_variants, enabled_module_slots, localise_for,
    raw_variant, variant_name, variant_names, variant_order,
};

/// Decimal places of recorded equipment amounts (`engine::equipment_entry`) and
/// of production line cost and speed: fixed5.
pub const EQUIPMENT_DECIMALS: u8 = 5;
const _: () = assert!(engine::equipment_entry::FIXED_POINT_SCALE == 100_000);

/// Units a production line makes per week, fixed5, by the game's weekly formula:
/// its production-rate formatter writes the line's speed (the IC added at each
/// daily production tick) × 7 days × 100000 ÷ its cost per unit, truncated toward
/// zero: `speed × 700000 ÷ cost`.
///
/// The formatter shows that weekly figure only when `speed < cost ≤ 7 × speed`;
/// it picks per day (`speed × 100000 ÷ cost`) when `speed ≥ cost`, and per month
/// or year below a week. Each is the same rate truncated in its own unit, so this
/// weekly figure is exact, and is the unit the widgets sum lines in. With a cost of
/// zero, `speed ≥ cost` always holds and the formatter shows its per-day
/// placeholder whatever the speed, not a rate, so there is no weekly
/// figure to show: None without a positive cost, and None on overflow (where the
/// formatter shows the same placeholder).
pub fn weekly_units_raw(speed_raw: i64, cost_raw: i64) -> Option<i64> {
    if cost_raw <= 0 {
        return None;
    }
    i64::try_from(i128::from(speed_raw) * 700_000 / i128::from(cost_raw)).ok()
}

/// Units a production line makes per day, fixed5, by the game's per-day formula:
/// the production-rate formatter's daily figure, the line's speed (the IC added at
/// each daily production tick) × 100000 ÷ its cost per unit, truncated toward
/// zero: `speed × 100000 ÷ cost`. It is its own truncation, not the weekly figure
/// divided by seven. None without a positive cost (the formatter's placeholder,
/// not a rate) and None on overflow, as for [`weekly_units_raw`].
pub fn daily_units_raw(speed_raw: i64, cost_raw: i64) -> Option<i64> {
    if cost_raw <= 0 {
        return None;
    }
    i64::try_from(i128::from(speed_raw) * 100_000 / i128::from(cost_raw)).ok()
}

/// Which arm an equipment type serves, from the engine's own classifiers of its
/// effective category flags (`installed_content::equipment::Database::category`):
/// the plane family is air, the ship family (convoys included) naval, everything
/// else land.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum Domain {
    Land,
    Air,
    Naval,
}

/// Starts the key of an equipment type content does not resolve, before the
/// recorded token's spelling (or the variant's handle without one). Settings can
/// store it (a content-set token), so such a type can be included and chosen like
/// any other; it is kept apart, never merged into another type. A content type key
/// starting with it is warned about ([`marked_keys_warning`]).
pub const UNRESOLVED_MARK: &str = ":";
/// The arm of types content does not resolve, whose arm cannot be told.
pub const UNRESOLVED_ARM: &str = "unresolved";

/// The warning for content type keys that start with [`UNRESOLVED_MARK`], which the
/// widgets would take for unresolved types; none when no key does.
pub fn marked_keys_warning<'a>(keys: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let marked: Vec<&str> = keys
        .into_iter()
        .filter(|key| key.starts_with(UNRESOLVED_MARK))
        .collect();
    (!marked.is_empty()).then(|| {
        format!(
            "Equipment types whose key starts with \"{UNRESOLVED_MARK}\" are not supported: the widgets mark unresolved types that way, so they could be mistaken for one ({}).",
            marked.join(", ")
        )
    })
}

impl Domain {
    pub fn of(category: u64) -> Self {
        if category & engine::cequipment_type::PLANE_MASK != 0 {
            Self::Air
        } else if category & engine::cequipment_type::SHIP_MASK != 0 {
            Self::Naval
        } else {
            Self::Land
        }
    }
}

/// A design's fingerprint: 64-bit FNV-1a, as hex, over its equipment token, own name,
/// suffix and modules (each slot, module and whether it is active, in the recorded
/// order), and upgrades (each definition and level, in recorded order), every text
/// as its lexer spelling. The same design reads the same in every
/// branch; a design renamed reads as another.
pub fn design_fingerprint(
    raw: &RawVariant,
    modules: &[(String, String, bool)],
    upgrades: &[(String, u8)],
) -> Option<String> {
    if ["token", "display name", "suffix", "design"]
        .iter()
        .any(|field| raw.check(field).is_err())
    {
        return None;
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut add = |bytes: &[u8]| {
        for &byte in bytes.iter().chain(&[0xff]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    };
    add(raw.token.as_bytes());
    add(raw.display_name.as_bytes());
    add(raw.suffix.as_bytes());
    add(&[u8::from(raw.append)]);
    for (slot, module, active) in modules {
        add(slot.as_bytes());
        add(module.as_bytes());
        add(&[u8::from(*active)]);
    }
    for (definition, level) in upgrades {
        // A non-UTF-8 tag keeps upgrades apart from module text fields.
        add(&[0xfe]);
        add(definition.as_bytes());
        add(&[*level]);
    }
    Some(format!("{hash:016x}"))
}

/// A token's exact spelling: empty for token 0 (none), refusal when unspelled.
fn known_spelling(token: u32, lexical: &[LexicalTokenRow]) -> Result<String, String> {
    if token == 0 {
        return Ok(String::new());
    }
    let index = lexical
        .binary_search_by_key(&token, |row| row.token)
        .map_err(|_| format!("LexicalTokens lacks equipment token {token}"))?;
    text(
        &lexical[index].literal,
        &format!("equipment lexer token {token}"),
    )
}

/// A variant's token spelling through the hour's lexical tokens; empty when unspelled.
pub fn variant_token(definition: &VariantDefinitionRow, lexical: &[LexicalTokenRow]) -> String {
    lexical
        .binary_search_by_key(&definition.equipment_token, |row| row.token)
        .ok()
        .and_then(|index| text(&lexical[index].literal, "equipment type token").ok())
        .unwrap_or_default()
}

/// One entry of a variant's naming history: its operands from a read of the variant
/// table, or `None` from an hour its names stopped being known.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantVersion {
    pub first_hour: i32,
    pub raw: Option<RawVariant>,
}

/// Every variant's naming operands over a branch, with where they are known. The
/// variant table is read in its own daily slot and names hold until the next read,
/// but only while the recording is unbroken: a gap, a due read of the table that is
/// not complete (with the country and token tables its names need), or an hour in
/// between whose variant table is not complete (the recording marked what it carries
/// unavailable) ends what is known, so a variant renamed meanwhile never keeps its old
/// name. Every product that names variants over time builds them here.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VariantCatalogue {
    pub versions: BTreeMap<u64, Vec<VariantVersion>>,
    last_hour: Option<i32>,
    known: BTreeSet<u64>,
    #[serde(skip)]
    country_keys: Option<crate::metadata::CountryKeys>,
    #[cfg(any(test, feature = "test-support"))]
    #[serde(skip)]
    pub work: crate::prepared::observer_tests::Work,
}

impl VariantCatalogue {
    /// Authenticate carried naming without rebuilding any retained versions.
    pub fn advance(&mut self, hour: i32) {
        self.last_hour = Some(hour);
    }
    /// A catalogue carrying `versions` as they stand: each variant whose latest
    /// version names it is known, so the next forget ends its names as it would
    /// for the catalogue that read them.
    pub fn seeded(versions: BTreeMap<u64, Vec<VariantVersion>>) -> Self {
        let known = versions
            .iter()
            .filter(|(_, versions)| versions.last().is_some_and(|version| version.raw.is_some()))
            .map(|(&id, _)| id)
            .collect();
        Self {
            versions,
            known,
            ..Self::default()
        }
    }
    /// The frames' tables it reads: select them with the product's own.
    pub const TABLES: [TableId; 7] = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryAppearance,
        TableId::LexicalTokens,
        TableId::VariantDefinitions,
        TableId::VariantModules,
        TableId::VariantUpgrades,
    ];

    /// One decoded frame, in hour order.
    pub fn observe(&mut self, format: replay::format::FormatVersion, frame: &DecodedFrame) {
        let _ = self.observe_versions(format, frame, None);
    }

    pub fn observe_changed(
        &mut self,
        format: replay::format::FormatVersion,
        frame: &DecodedFrame,
        changes: &crate::prepared::frame_changes::FrameChanges,
    ) -> Vec<(u64, VariantVersion)> {
        self.observe_versions(format, frame, Some(changes))
    }

    fn observe_versions(
        &mut self,
        format: replay::format::FormatVersion,
        frame: &DecodedFrame,
        changes: Option<&crate::prepared::frame_changes::FrameChanges>,
    ) -> Vec<(u64, VariantVersion)> {
        let mut changed = Vec::new();
        let hour = frame.hour;
        // After a gap, what was known ends at the first hour not recorded.
        if let Some(last) = self.last_hour
            && last.checked_add(1) != Some(hour)
        {
            changed.extend(self.forget_versions(last.saturating_add(1)));
        }
        self.last_hour = Some(hour);
        if !changes.map_or_else(
            || {
                slot_read(
                    format,
                    &frame.metrics,
                    TableId::VariantDefinitions,
                    frame.hour,
                )
            },
            |changes| changes.slot_read[TableId::VariantDefinitions.index()],
        ) {
            // Carried only while the table stays complete.
            if frame.complete::<tables::VariantDefinitions>().is_err() {
                changed.extend(self.forget_versions(hour));
            }
            return changed;
        }
        let (Ok(countries), Ok(links), Ok(definitions), Ok(lexical)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
            frame.complete::<tables::VariantDefinitions>(),
            frame.complete::<tables::LexicalTokens>(),
        ) else {
            // A due read that did not land: names are not known from here.
            changed.extend(self.forget_versions(hour));
            return changed;
        };
        if let Some(changes) = changes {
            let country_changed = changes.rows_changed[TableId::Countries.index()]
                && self.country_keys.as_ref().is_none_or(|keys| {
                    #[cfg(any(test, feature = "test-support"))]
                    let before = keys.row_visits();
                    let changed = !keys.same(countries);
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.row_visits += keys.row_visits() - before;
                    }
                    changed
                });
            if !country_changed
                && Self::TABLES
                    .iter()
                    .filter(|table| **table != TableId::Countries)
                    .all(|table| !changes.rows_changed[table.index()])
            {
                return changed;
            }
        }
        self.country_keys = Some(crate::metadata::CountryKeys::read(countries));
        #[cfg(any(test, feature = "test-support"))]
        {
            self.work.row_visits += countries.len()
                + links.len()
                + lexical.len()
                + frame
                    .complete::<tables::CountryAppearance>()
                    .map_or(0, |rows| rows.len());
        }
        let tags = Tags::new(
            countries,
            links,
            frame.complete::<tables::CountryAppearance>().ok(),
        );
        // Modules and upgrades share the variant read's slot; a complete table
        // without a variant's rows means it has none, an unavailable one is unknown.
        let modules = frame.complete::<tables::VariantModules>().ok().map(|rows| {
            let mut by_variant: BTreeMap<u64, Vec<&VariantModuleRow>> = BTreeMap::new();
            for row in rows {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.row_visits += 1;
                }
                by_variant.entry(row.variant).or_default().push(row);
            }
            for rows in by_variant.values_mut() {
                rows.sort_by_key(|row| row.source_ordinal);
            }
            by_variant
        });
        let upgrades = frame
            .complete::<tables::VariantUpgrades>()
            .ok()
            .map(|rows| {
                let mut by_variant: BTreeMap<u64, Vec<&VariantUpgradeRow>> = BTreeMap::new();
                for row in rows {
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.work.row_visits += 1;
                    }
                    by_variant.entry(row.variant).or_default().push(row);
                }
                for rows in by_variant.values_mut() {
                    rows.sort_by_key(|row| row.source_ordinal);
                }
                by_variant
            });
        for definition in definitions {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.work.row_visits += 1;
            }
            let mut raw = raw_variant(definition, hour, &tags, lexical);
            raw.design = modules
                .as_ref()
                .zip(upgrades.as_ref())
                .and_then(|(modules, upgrades)| {
                    let own: Result<Vec<(String, String, bool)>, String> = modules
                        .get(&definition.id)
                        .into_iter()
                        .flatten()
                        .map(|row| {
                            Ok((
                                known_spelling(row.slot, lexical)?,
                                known_spelling(row.definition_token, lexical)?,
                                row.definition_active != 0,
                            ))
                        })
                        .collect();
                    let own_upgrades: Result<Vec<(String, u8)>, String> = upgrades
                        .get(&definition.id)
                        .into_iter()
                        .flatten()
                        .map(|row| {
                            let definition = known_spelling(row.definition_token as u32, lexical)?;
                            if definition.is_empty() {
                                return Err(
                                    "equipment upgrade definition has no token spelling".into()
                                );
                            }
                            Ok((definition, row.level))
                        })
                        .collect();
                    match own.and_then(|own| own_upgrades.map(|upgrades| (own, upgrades))) {
                        Ok((own, upgrades)) => design_fingerprint(&raw, &own, &upgrades),
                        Err(why) => {
                            raw.refusals.push(("design".into(), why));
                            None
                        }
                    }
                });
            let versions = self.versions.entry(definition.id).or_default();
            let same = versions.last().is_some_and(|last| {
                last.raw.as_ref().is_some_and(|last| {
                    RawVariant {
                        first_hour: hour,
                        ..last.clone()
                    } == raw
                })
            });
            if !same {
                let version = VariantVersion {
                    first_hour: hour,
                    raw: Some(raw),
                };
                push(versions, version.clone());
                self.known.insert(definition.id);
                changed.push((definition.id, version));
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.emitted += 1;
                }
            }
        }
        changed
    }

    /// Names are not known from `hour` until the next complete read.
    pub fn forget(&mut self, hour: i32) {
        let _ = self.forget_versions(hour);
    }

    fn forget_versions(&mut self, hour: i32) -> Vec<(u64, VariantVersion)> {
        std::mem::take(&mut self.known)
            .into_iter()
            .map(|id| {
                #[cfg(any(test, feature = "test-support"))]
                {
                    self.work.history_visits += 1;
                    self.work.emitted += 1;
                }
                let version = VariantVersion {
                    first_hour: hour,
                    raw: None,
                };
                push(
                    self.versions.get_mut(&id).expect("known variant"),
                    version.clone(),
                );
                (id, version)
            })
            .collect()
    }

    /// A variant's operands as last observed: none where they are not known now.
    pub fn current(&self, id: u64) -> Option<&RawVariant> {
        self.versions.get(&id)?.last()?.raw.as_ref()
    }

    /// A variant's equipment token spelling from its latest known read; empty when no
    /// read named it. The latest nonempty token also classifies earlier numeric history.
    pub fn token(&self, id: u64) -> &str {
        self.versions
            .get(&id)
            .into_iter()
            .flatten()
            .rev()
            .filter_map(|version| version.raw.as_ref())
            .map(|raw| raw.token.as_str())
            .find(|token| !token.is_empty())
            .unwrap_or_default()
    }

    /// Invalid strings refuse only names/design identity, never numeric rows.
    pub fn warnings(&self) -> Vec<String> {
        let mut refusals = crate::catalogue::Refusals::default();
        for (id, versions) in &self.versions {
            for raw in versions.iter().filter_map(|version| version.raw.as_ref()) {
                for (_, why) in &raw.refusals {
                    refusals.note(why.as_str(), id.to_string());
                }
            }
        }
        refusals.diagnostic_lines("equipment variants with unknown text/design operands")
    }
}

/// Appends a version, never two at one hour: a later one at the same hour replaces it.
fn push(versions: &mut Vec<VariantVersion>, version: VariantVersion) {
    match versions.last_mut() {
        Some(last) if last.first_hour >= version.first_hour => *last = version,
        _ => versions.push(version),
    }
}

/// A variant's names as recorded at a read of the variant table; a new entry
/// wherever a read changes them.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockVariantName {
    /// The read these names are from; they hold until the next entry.
    pub first_hour: i32,
    /// The variant's display name; null when content does not name it, or when
    /// not known.
    pub name: Option<String>,
    /// False from an hour the variant's names stopped being known (a gap, or a due
    /// read of the variant table that did not land) until the next entry.
    pub known: bool,
    /// The design's fingerprint as read then (`equipment::design_fingerprint`: its
    /// name, modules and upgrade levels); null where not known. A saved variant
    /// choice carries it, so a later branch's other design under the same handle
    /// is never taken for it.
    pub design: Option<String>,
}

impl VariantCatalogue {
    /// A variant's names as each read recorded them, dated by the read, with where
    /// they stopped being known: a name is never carried back before the read that
    /// recorded it, nor over hours its names were not known.
    pub fn names(&self, id: u64, texts: Option<&dyn Texts>) -> Vec<StockVariantName> {
        let mut names: Vec<StockVariantName> = Vec::new();
        for version in self.versions.get(&id).into_iter().flatten() {
            let (name, known, design) = match &version.raw {
                Some(raw) => (variant_names(raw, texts).0, true, raw.design.clone()),
                None => (None, false, None),
            };
            if names.last().is_some_and(|last| {
                last.name == name && last.known == known && last.design == design
            }) {
                continue;
            }
            names.push(StockVariantName {
                first_hour: version.first_hour,
                name,
                known,
                design,
            });
        }
        names
    }
}

/// A variant's type resolved through content: its archetype key, database place and domain.
pub fn archetype_of(
    types: &installed_content::equipment::Database,
    token: &str,
) -> Result<(String, usize, Domain), String> {
    let kind = types
        .named(token)
        .ok_or_else(|| format!("{token} is not authored equipment"))?;
    let ordinal = kind
        .archetype
        .ok_or_else(|| format!("equipment {token} has no known archetype"))?;
    let archetype = &types.equipment[ordinal];
    Ok((
        archetype
            .name_str()
            .map_err(|error| error.to_string())?
            .to_owned(),
        ordinal,
        Domain::of(types.category(archetype)),
    ))
}

/// Native-reader diagnostics, with their file and definition names.
pub fn equipment_warnings(types: &installed_content::equipment::Database) -> Vec<String> {
    types.misreads.clone()
}

/// The authored equipment types as read, or why they could not be.
type Units = Result<Arc<installed_content::equipment::Database>, String>;

/// The recorded content stack's equipment types for a catalogue, read on first use.
pub fn units(_source: &Source, catalogue: &Arc<Catalogue>) -> Units {
    catalogue
        .content
        .layers
        .derived::<crate::catalogue::CachedResult<installed_content::equipment::Database>>()
        .get_or_init(|| installed_content::equipment::Database::load_complete(&catalogue.content.layers))
        .map_err(|error| error.to_string())
}

/// How many times the catalogue's content stack read its equipment types.
#[cfg(any(test, feature = "test-support"))]
pub fn unit_loads(catalogue: &Catalogue) -> usize {
    catalogue
        .content
        .layers
        .derived::<crate::catalogue::CachedResult<installed_content::equipment::Database>>()
        .loads()
}

#[cfg(test)]
mod tests;
