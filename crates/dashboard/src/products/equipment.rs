//! Equipment facts every equipment product shares: units per week as the game's
//! production view writes them, a variant's name as the game writes it, and an
//! equipment type's domain (land, air or naval) by the engine's own classifiers.
//! Products about equipment (stockpile, production lines, production) read them
//! here, never a copy.

pub(crate) mod production;

use crate::{
    Source,
    catalogue::{Catalogue, View},
    metadata::{link_slot, slot_read},
};
use replay::{
    frame::DecodedFrame,
    rows::{
        CountryAppearanceRow, CountryLinkRow, CountryRow, LexicalTokenRow, TableId,
        VariantDefinitionRow, VariantModuleRow, VariantUpgradeRow, tables,
    },
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};
use ts_rs::TS;

/// Decimal places of recorded equipment amounts (`engine::equipment_entry`) and
/// of production line cost and speed: fixed5.
pub const EQUIPMENT_DECIMALS: u8 = 5;
const _: () = assert!(engine::equipment_entry::FIXED_POINT_SCALE == 100_000);

/// Recorded/content operands used to order equipment variants.
#[derive(Clone, Copy)]
pub struct VariantOrder {
    pub archetype_ordinal: usize,
    pub parent_depth: i32,
    /// Zero with DLC-enabled module slots; otherwise wrapping upgrade-byte sum.
    pub upgrade_sum: i32,
    pub creator_reference: i32,
    pub creator_slot: i32,
    pub id: u32,
}

/// Shared by native model candidates and market archetype selection.
pub fn compare_variants(
    owner_reference: i32,
    owner_slot: i32,
    left: VariantOrder,
    right: VariantOrder,
) -> std::cmp::Ordering {
    let same_country =
        |a: i32, a_slot: i32, b: i32, b_slot: i32| a == b || (a != 0 && b != 0 && a_slot == b_slot);
    let preferred = |variant: VariantOrder| {
        same_country(
            variant.creator_reference,
            variant.creator_slot,
            owner_reference,
            owner_slot,
        )
    };
    left.archetype_ordinal
        .cmp(&right.archetype_ordinal)
        .then_with(|| right.parent_depth.cmp(&left.parent_depth))
        .then_with(|| right.upgrade_sum.cmp(&left.upgrade_sum))
        .then_with(|| preferred(right).cmp(&preferred(left)))
        .then_with(|| {
            if same_country(
                left.creator_reference,
                left.creator_slot,
                right.creator_reference,
                right.creator_slot,
            ) {
                right.id.cmp(&left.id)
            } else {
                left.creator_slot.cmp(&right.creator_slot)
            }
        })
}

/// Whether the equipment type has module slots enabled by the recorded DLC mask.
pub fn enabled_module_slots(
    database: &installed_content::equipment::Database,
    index: usize,
    frame: &DecodedFrame,
) -> Result<bool, String> {
    let kind = &database.equipment[index];
    if kind.module_slots.is_empty() {
        return Ok(false);
    }
    let dlc = frame
        .complete::<tables::SimulationFeatures>()
        .map_err(|why| why.to_string())?
        .first()
        .ok_or("SimulationFeatures lacks the DLC mask")?
        .dlc_mask;
    let mask = if dlc & (1 << 4) != 0 { 0x80004001C0 } else { 0 }
        | if dlc & (1 << 7) != 0 { 4 } else { 0 }
        | if dlc & (1 << 8) != 0 { 0x1D0026FC00 } else { 0 };
    Ok(database.category(kind) & mask != 0)
}

/// Recorded operands used to rank equipment variants; an unread operand
/// refuses the rank rather than changing the winning design.
pub fn variant_order(
    database: &installed_content::equipment::Database,
    index: usize,
    variant: &VariantDefinitionRow,
    frame: &DecodedFrame,
) -> Result<VariantOrder, String> {
    let kind = &database.equipment[index];
    let archetype = kind
        .archetype
        .ok_or("equipment variant has a null archetype")?;
    let upgrade_sum = if enabled_module_slots(database, index, frame)? {
        0
    } else {
        // Upgrade levels are unsigned; their i32 sum wraps.
        let upgrades = frame
            .complete::<tables::VariantUpgrades>()
            .map_err(|why| why.to_string())?;
        let first = upgrades.partition_point(|row| row.variant < variant.id);
        let count = upgrades[first..].partition_point(|row| row.variant == variant.id);
        upgrades[first..first + count]
            .iter()
            .fold(0_i32, |sum, row| sum.wrapping_add(i32::from(row.level)))
    };
    let creator = variant.creator_link as i32;
    let creator_slot = if creator > 0 {
        link_slot(
            frame
                .complete::<tables::CountryLinks>()
                .map_err(|why| why.to_string())?,
            variant.creator_link,
        )
        .ok_or("CountryLinks lacks variant creator")? as i32
    } else {
        creator
    };
    Ok(VariantOrder {
        archetype_ordinal: archetype,
        parent_depth: kind.depth,
        upgrade_sum,
        creator_reference: creator,
        creator_slot,
        id: (variant.id >> 32) as u32,
    })
}

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

fn text(bytes: &[u8], what: &str) -> Result<String, String> {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| format!("{what} is not UTF-8"))
}

/// A variant's naming operands at one read of the variant table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawVariant {
    pub id: u64,
    pub first_hour: i32,
    /// The equipment type's token spelling.
    pub token: String,
    /// The variant's own name; empty when it has none.
    pub display_name: String,
    /// The suffix and whether the name takes it.
    pub suffix: String,
    pub append: bool,
    /// The tag its name is localised for: its origin, else its creator's (original) tag.
    pub name_tag: Option<String>,
    /// The tag of the country in slot 0, which a production row's type line is localised for.
    pub type_tag: Option<String>,
    /// The design's fingerprint at this read ([`design_fingerprint`]); none where the
    /// module and upgrade tables were not read complete with it, or the recorded
    /// dictionary does not spell one of its modules, slots or upgrades. A reload restores the game's
    /// variant index, so a later branch can give a handle to another design; the
    /// fingerprint tells them apart.
    pub design: Option<String>,
    /// Invalid or unread operands retain the variant and its numeric data.
    pub refusals: Vec<(&'static str, String)>,
}

impl RawVariant {
    pub(crate) fn check(&self, field: &str) -> Result<(), String> {
        match self.refusals.iter().find(|(name, _)| *name == field) {
            Some((_, why)) => Err(why.clone()),
            None => Ok(()),
        }
    }

    /// The display name may hold literal or already localised text.
    /// Its original key/provenance is not recorded, so use the shared language guard.
    pub fn own_name(&self, texts: Option<&dyn Texts>) -> Result<Option<String>, String> {
        self.check("display name")?;
        if self.display_name.is_empty() {
            return Ok(None);
        }
        stored(texts, &self.display_name, "equipment display name").map(Some)
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

/// The countries' tags at one hour, by canonical slot: each tag, and the original
/// tag a dynamic country is named after (`CountryAppearance.name_tag`).
pub struct Tags<'a> {
    links: &'a [CountryLinkRow],
    tags: BTreeMap<u32, Result<String, String>>,
    original: BTreeMap<u32, Result<String, String>>,
}

impl<'a> Tags<'a> {
    pub fn new(
        countries: &[CountryRow],
        links: &'a [CountryLinkRow],
        appearance: Option<&[CountryAppearanceRow]>,
    ) -> Self {
        let tags = countries
            .iter()
            .map(|country| {
                (
                    country.canonical_slot,
                    text(&country.tag, "equipment country tag"),
                )
            })
            .collect();
        let original = appearance
            .unwrap_or_default()
            .iter()
            .filter(|row| !row.name_tag.is_empty())
            .filter_map(|row| {
                Some((
                    link_slot(links, row.country)?,
                    text(&row.name_tag, "equipment original country tag"),
                ))
            })
            .collect();
        Self {
            links,
            tags,
            original,
        }
    }

    /// The tag of the country a raw link names.
    pub fn tag_of(&self, raw: u32) -> Option<String> {
        self.tags
            .get(&link_slot(self.links, raw)?)?
            .as_ref()
            .ok()
            .cloned()
    }

    /// Its original tag when it is a dynamic country, else its tag.
    fn naming_tag(&self, raw: u32, original: bool) -> Result<Option<String>, String> {
        let Some(slot) = link_slot(self.links, raw) else {
            return Ok(None);
        };
        (if original {
            self.original.get(&slot).or_else(|| self.tags.get(&slot))
        } else {
            self.tags.get(&slot)
        })
        .cloned()
        .transpose()
    }
}

/// A variant's token spelling through the hour's lexical tokens; empty when unspelled.
pub fn variant_token(definition: &VariantDefinitionRow, lexical: &[LexicalTokenRow]) -> String {
    lexical
        .binary_search_by_key(&definition.equipment_token, |row| row.token)
        .ok()
        .and_then(|index| text(&lexical[index].literal, "equipment type token").ok())
        .unwrap_or_default()
}

/// A variant's naming operands at a read.
pub fn raw_variant(
    definition: &VariantDefinitionRow,
    hour: i32,
    tags: &Tags,
    lexical: &[LexicalTokenRow],
) -> RawVariant {
    // The origin when set (a signed id of at least 1), else the creator, by its
    // original tag when it is a dynamic country.
    let mut refusals = Vec::new();
    let mut checked = |field, bytes: &[u8], what: &str| match text(bytes, what) {
        Ok(text) => text,
        Err(why) => {
            refusals.push((field, why));
            String::new()
        }
    };
    let token = lexical
        .binary_search_by_key(&definition.equipment_token, |row| row.token)
        .ok()
        .map(|index| checked("token", &lexical[index].literal, "equipment type token"))
        .unwrap_or_default();
    let display_name = checked(
        "display name",
        &definition.display_name,
        "equipment display name",
    );
    let suffix = checked(
        "suffix",
        &definition.variant_name,
        "equipment variant suffix",
    );
    if token.is_empty() && !refusals.iter().any(|(field, _)| *field == "token") {
        refusals.push((
            "token",
            format!(
                "LexicalTokens lacks equipment type token {}",
                definition.equipment_token
            ),
        ));
    }
    let name_tag = if (definition.origin_link as i32) >= 1 {
        tags.naming_tag(definition.origin_link, false)
    } else {
        tags.naming_tag(definition.creator_link, true)
    };
    let name_tag = match name_tag {
        Ok(tag) => tag,
        Err(why) => {
            refusals.push(("name tag", why));
            None
        }
    };
    let type_tag = match tags.tags.get(&0).cloned().transpose() {
        Ok(tag) => tag,
        Err(why) => {
            refusals.push(("type tag", why));
            None
        }
    };
    RawVariant {
        id: definition.id,
        first_hour: hour,
        token,
        display_name,
        suffix,
        append: definition.append_variant_name != 0,
        name_tag,
        // A production row's type line is localised for the country in slot 0.
        type_tag,
        // Set from the module and upgrade tables by the catalogue's read.
        design: None,
        refusals,
    }
}

/// One entry of a variant's naming history: its operands from a read of the variant
/// table, or `None` from an hour its names stopped being known.
#[derive(Clone, Debug, PartialEq, Eq)]
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
#[derive(Clone, Debug, Default)]
pub struct VariantCatalogue {
    pub versions: BTreeMap<u64, Vec<VariantVersion>>,
    last_hour: Option<i32>,
}

impl VariantCatalogue {
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
    pub fn observe(&mut self, frame: &DecodedFrame) {
        let hour = frame.hour;
        // After a gap, what was known ends at the first hour not recorded.
        if let Some(last) = self.last_hour
            && last.checked_add(1) != Some(hour)
        {
            self.forget(last.saturating_add(1));
        }
        self.last_hour = Some(hour);
        if !slot_read(&frame.metrics, TableId::VariantDefinitions, frame.hour) {
            // Carried only while the table stays complete.
            if frame.complete::<tables::VariantDefinitions>().is_err() {
                self.forget(hour);
            }
            return;
        }
        let (Ok(countries), Ok(links), Ok(definitions), Ok(lexical)) = (
            frame.complete::<tables::Countries>(),
            frame.complete::<tables::CountryLinks>(),
            frame.complete::<tables::VariantDefinitions>(),
            frame.complete::<tables::LexicalTokens>(),
        ) else {
            // A due read that did not land: names are not known from here.
            self.forget(hour);
            return;
        };
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
                    by_variant.entry(row.variant).or_default().push(row);
                }
                for rows in by_variant.values_mut() {
                    rows.sort_by_key(|row| row.source_ordinal);
                }
                by_variant
            });
        for definition in definitions {
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
                            raw.refusals.push(("design", why));
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
                push(
                    versions,
                    VariantVersion {
                        first_hour: hour,
                        raw: Some(raw),
                    },
                );
            }
        }
    }

    /// Names are not known from `hour` until the next complete read.
    pub(crate) fn forget(&mut self, hour: i32) {
        for versions in self.versions.values_mut() {
            if versions.last().is_some_and(|last| last.raw.is_some()) {
                push(
                    versions,
                    VariantVersion {
                        first_hour: hour,
                        raw: None,
                    },
                );
            }
        }
    }

    /// A variant's operands as last observed: none where they are not known now.
    pub fn current(&self, id: u64) -> Option<&RawVariant> {
        self.versions.get(&id)?.last()?.raw.as_ref()
    }

    /// A variant's equipment token spelling from its latest known read; empty when no
    /// read named it (a variant's type never changes).
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

/// Localised texts a name is looked up in: the recorded content's localisation.
pub trait Texts {
    /// Whether the localisation defines the key.
    fn has(&self, key: &str) -> bool;
    /// The key's resolved text; none when missing or needing runtime scope.
    fn text(&self, key: &str) -> Option<String>;
    /// A stored native string needs its recording's language, not a key lookup.
    fn stored_text(&self, _text: &str, what: &str) -> Result<String, String> {
        Err(format!("{what} has no recorded/viewer language context"))
    }
    fn refuse(&self, _reason: &str, _identity: &str) {}
}

/// Equipment and variant names resolve keys with the plain localiser: brackets
/// are glyphs.
#[cfg(test)]
impl Texts for installed_content::localisation::Localisation {
    fn has(&self, key: &str) -> bool {
        self.get(key).is_some()
    }
    fn text(&self, key: &str) -> Option<String> {
        self.has(key)
            .then(|| crate::catalogue::localised(self, key, View::Plain).ok())
            .flatten()
    }
    fn stored_text(&self, text: &str, what: &str) -> Result<String, String> {
        crate::scripted_text::stored_text(self, &self.language, text.as_bytes(), what)
    }
}

impl Texts for Catalogue {
    fn has(&self, key: &str) -> bool {
        self.localisation.get(key).is_some()
    }
    fn text(&self, key: &str) -> Option<String> {
        self.has(key)
            .then(|| self.localised(key, View::Plain))
            .flatten()
    }
    fn stored_text(&self, text: &str, what: &str) -> Result<String, String> {
        crate::scripted_text::stored_text(
            &self.localisation,
            &self.recorded_language,
            text.as_bytes(),
            what,
        )
    }
    fn refuse(&self, reason: &str, identity: &str) {
        Catalogue::refuse(self, reason, identity);
    }
}

fn stored(texts: Option<&dyn Texts>, text: &str, what: &str) -> Result<String, String> {
    texts
        .ok_or_else(|| format!("{what} has no recorded/viewer language context"))?
        .stored_text(text, what)
}

/// Resolves `<TAG>_<key>` when the localisation has it, else `<key>`.
pub fn localise_for(texts: &dyn Texts, tag: Option<&str>, key: &str) -> Option<String> {
    let prefixed = tag.map(|tag| format!("{tag}_{key}"));
    let key = prefixed
        .as_deref()
        .filter(|prefixed| texts.has(prefixed))
        .unwrap_or(key);
    texts.text(key)
}

/// A variant's name (its own name, else its type key localised for its country,
/// then its suffix when it asks for one) and its type line (the type key localised
/// for the country in slot 0). Stored own names/suffixes have
/// no recorded key/provenance and must pass the shared stored-text language guard.
pub fn variant_name(raw: &RawVariant, texts: Option<&dyn Texts>) -> Result<Option<String>, String> {
    let base = match raw.own_name(texts)? {
        Some(name) => Some(name),
        None => {
            raw.check("token")?;
            raw.check("name tag")?;
            texts.and_then(|texts| localise_for(texts, raw.name_tag.as_deref(), &raw.token))
        }
    };
    if raw.append {
        raw.check("suffix")?;
        if !raw.suffix.is_empty() {
            let suffix = stored(texts, &raw.suffix, "equipment variant suffix")?;
            return Ok(base.map(|base| format!("{base} {suffix}")));
        }
    }
    Ok(base)
}

pub fn variant_names(
    raw: &RawVariant,
    texts: Option<&dyn Texts>,
) -> (Option<String>, Option<String>) {
    if let Some(texts) = texts {
        for (_, why) in &raw.refusals {
            texts.refuse(why, &raw.id.to_string());
        }
    }
    let name = match variant_name(raw, texts) {
        Ok(name) => name,
        Err(why) => {
            if let Some(texts) = texts {
                texts.refuse(&why, &raw.id.to_string());
            }
            None
        }
    };
    let type_name = raw
        .check("token")
        .and_then(|()| raw.check("type tag"))
        .ok()
        .and_then(|()| {
            texts.and_then(|texts| localise_for(texts, raw.type_tag.as_deref(), &raw.token))
        });
    (name, type_name)
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

/// The authored equipment types, read once per catalogue for every equipment
/// product of a source (the source's state is keyed by this type).
#[derive(Default)]
struct UnitsCache {
    content: Option<(Arc<Catalogue>, Units)>,
}

/// The recorded content stack's equipment types for a catalogue, read on first use.
pub fn units(source: &Source, catalogue: &Arc<Catalogue>) -> Units {
    let state = source.state::<UnitsCache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if !cache
        .content
        .as_ref()
        .is_some_and(|(cached, _)| Arc::ptr_eq(cached, catalogue))
    {
        let units = installed_content::equipment::Database::load_complete(&catalogue.content.layers)
            .map(Arc::new)
            .map_err(|error| error.to_string());
        cache.content = Some((Arc::clone(catalogue), units));
    }
    cache.content.as_ref().expect("units cached").1.clone()
}

#[cfg(test)]
mod tests;
