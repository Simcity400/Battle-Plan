//! An equipment variant's name as the game writes it, and the operands that order
//! variants, shared by the equipment products and effect descriptions.

use crate::{
    catalogue::{Catalogue, View},
    metadata::link_slot,
};
use replay_data::{
    rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow, rows::LexicalTokenRow,
    rows::VariantDefinitionRow, rows::tables,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    database: &installed_content_core::equipment::Database,
    index: usize,
    frame: &dyn replay_frame::frame::DecodedSource,
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
    database: &installed_content_core::equipment::Database,
    index: usize,
    variant: &VariantDefinitionRow,
    frame: &dyn replay_frame::frame::DecodedSource,
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

pub fn text(bytes: &[u8], what: &str) -> Result<String, String> {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| format!("{what} is not UTF-8"))
}

/// A variant's naming operands at one read of the variant table.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    #[serde(deserialize_with = "variant_refusals")]
    pub refusals: Vec<(String, String)>,
}

fn variant_refusals<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<(String, String)>, D::Error> {
    let refusals = Vec::<(String, String)>::deserialize(deserializer)?;
    for (label, _) in &refusals {
        if !matches!(
            label.as_str(),
            "token" | "display name" | "suffix" | "name tag" | "type tag" | "design"
        ) {
            return Err(<D::Error as serde::de::Error>::custom(format!(
                "Unknown equipment variant operand {label:?}"
            )));
        }
    }
    Ok(refusals)
}

impl RawVariant {
    pub fn check(&self, field: &str) -> Result<(), String> {
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
        let slot = link_slot(self.links, raw)?;
        crate::triggers::refusal::reference(
            self.tags
                .get(&slot)
                .and_then(|tag| tag.as_ref().ok())
                .cloned(),
            raw != 0,
            || {
                crate::triggers::refusal::missing_reference(
                    replay_data::rows::TableId::Countries,
                    Some(crate::triggers::Store::Country(slot)),
                )
            },
        )
    }

    /// Its original tag when it is a dynamic country, else its tag.
    fn naming_tag(&self, raw: u32, original: bool) -> Result<Option<String>, String> {
        let Some(slot) = link_slot(self.links, raw) else {
            return Ok(None);
        };
        crate::triggers::refusal::reference(
            if original {
                self.original.get(&slot).or_else(|| self.tags.get(&slot))
            } else {
                self.tags.get(&slot)
            },
            raw != 0,
            || {
                crate::triggers::refusal::missing_reference(
                    replay_data::rows::TableId::Countries,
                    Some(crate::triggers::Store::Country(slot)),
                )
            },
        )
        .cloned()
        .transpose()
    }
}

/// A variant's naming operands at a read.
pub fn raw_variant<'a>(
    definition: &VariantDefinitionRow,
    hour: i32,
    tags: &Tags,
    lexical: impl Into<replay_frame::frame::Rows<'a, LexicalTokenRow>>,
) -> RawVariant {
    let lexical = lexical.into();
    // The origin when set (a signed id of at least 1), else the creator, by its
    // original tag when it is a dynamic country.
    let mut refusals = Vec::new();
    let mut checked = |field: &str, bytes: &[u8], what: &str| match text(bytes, what) {
        Ok(text) => text,
        Err(why) => {
            refusals.push((String::from(field), why));
            String::new()
        }
    };
    let token = crate::triggers::refusal::reference(
        lexical
            .binary_search_by_key(&definition.equipment_token, |row| row.token)
            .ok(),
        definition.equipment_token != 0,
        || {
            crate::triggers::refusal::missing_reference(
                replay_data::rows::TableId::LexicalTokens,
                None,
            )
        },
    )
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
            "token".into(),
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
            refusals.push(("name tag".into(), why));
            None
        }
    };
    let type_tag = match tags.tags.get(&0).cloned().transpose() {
        Ok(tag) => tag,
        Err(why) => {
            refusals.push(("type tag".into(), why));
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
#[cfg(any(test, feature = "test-support"))]
impl Texts for installed_content_localisation::localisation::Localisation {
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
