
use crate::{
    catalogue::{Catalogue, View},
    metadata::link_slot_in,
};
use replay_data::{
    rows::CountryAppearanceRow, rows::CountryLinkRow, rows::CountryRow, rows::LexicalTokenRow,
    rows::SimulationFeatureRow, rows::VariantDefinitionRow, rows::VariantModuleRow, rows::VariantUpgradeRow,
    rows::column, rows::mask, rows::tables,
};
use replay_frame::frame::{HourFrame, Rowset};

const DLC: u64 = mask::<SimulationFeatureRow>(&["dlc_mask"]);
const LEVEL: u64 = mask::<VariantUpgradeRow>(&["level"]);
const LINK_SLOT: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);
const COUNTRY_SLOT: u64 = mask::<CountryRow>(&["canonical_slot"]);
const COUNTRY_TAG: usize = column::<CountryRow>("tag");
const NAME_TAG: usize = column::<CountryAppearanceRow>("name_tag");
const LITERAL: usize = column::<LexicalTokenRow>("literal");
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy)]
pub struct VariantOrder {
    pub archetype_ordinal: usize,
    pub parent_depth: i32,
    pub upgrade_sum: i32,
    pub creator_reference: i32,
    pub creator_slot: i32,
    pub id: u32,
}

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

pub fn enabled_module_slots<'f>(
    database: &content_reader_key_names::equipment::Database,
    index: usize,
    frame: impl Into<HourFrame<'f>>,
) -> Result<bool, String> {
    let frame = frame.into();
    let kind = &database.equipment[index];
    if kind.module_slots.is_empty() {
        return Ok(false);
    }
    let dlc = frame
        .complete::<tables::SimulationFeatures>(DLC)
        .map_err(|why| why.to_string())?
        .iter()
        .next()
        .transpose()
        .map_err(|why| why.to_string())?
        .ok_or("simulation features lacks the DLC mask")?
        .dlc_mask;
    let mask = if dlc & (1 << 4) != 0 { 0x80004001C0 } else { 0 }
        | if dlc & (1 << 7) != 0 { 4 } else { 0 }
        | if dlc & (1 << 8) != 0 { 0x1D0026FC00 } else { 0 };
    Ok(database.category(kind) & mask != 0)
}

pub fn variant_order<'f>(
    database: &content_reader_key_names::equipment::Database,
    index: usize,
    variant: &VariantDefinitionRow,
    frame: impl Into<HourFrame<'f>>,
) -> Result<VariantOrder, String> {
    let frame = frame.into();
    let kind = &database.equipment[index];
    let archetype = kind
        .archetype
        .ok_or("equipment variant has a null archetype")?;
    let upgrade_sum = if enabled_module_slots(database, index, frame)? {
        0
    } else {
        let upgrades = frame
            .complete::<tables::VariantUpgrades>(LEVEL)
            .map_err(|why| why.to_string())?;
        let low = u128::from(variant.id) << 32;
        let mut sum = 0_i32;
        for row in upgrades.span(low, low | u128::from(u32::MAX)).iter() {
            sum = sum.wrapping_add(i32::from(row.map_err(|why| why.to_string())?.level));
        }
        sum
    };
    let creator = variant.creator_link as i32;
    let creator_slot = if creator > 0 {
        link_slot_in(
            &frame
                .complete::<tables::CountryLinks>(LINK_SLOT)
                .map_err(|why| why.to_string())?,
            variant.creator_link,
        )
        .ok_or("country links lacks variant creator")? as i32
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

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub struct RawVariant {
    pub id: u64,
    pub first_hour: i32,
    pub token: String,
    pub display_name: String,
    pub suffix: String,
    pub append: bool,
    pub name_tag: Option<String>,
    pub type_tag: Option<String>,
    pub design: Option<String>,
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
    pub fn heap(&self) -> usize {
        [&self.token, &self.display_name, &self.suffix]
            .into_iter()
            .chain(&self.name_tag)
            .chain(&self.type_tag)
            .chain(&self.design)
            .map(String::capacity)
            .sum::<usize>()
            + self.refusals.capacity() * std::mem::size_of::<(String, String)>()
            + self
                .refusals
                .iter()
                .map(|(label, reason)| label.capacity() + reason.capacity())
                .sum::<usize>()
    }

    pub fn check(&self, field: &str) -> Result<(), String> {
        match self.refusals.iter().find(|(name, _)| *name == field) {
            Some((_, why)) => Err(why.clone()),
            None => Ok(()),
        }
    }

    pub fn own_name(&self, texts: Option<&dyn Texts>) -> Result<Option<String>, String> {
        self.check("display name")?;
        if self.display_name.is_empty() {
            return Ok(None);
        }
        stored(texts, &self.display_name, "equipment display name").map(Some)
    }
}

pub struct Tags<'a> {
    links: Rowset<'a, CountryLinkRow>,
    tags: BTreeMap<u32, Result<String, String>>,
    original: Option<BTreeMap<u32, Result<String, String>>>,
}

impl<'a> Tags<'a> {
    pub fn new(
        countries: &[CountryRow],
        links: &'a [CountryLinkRow],
        appearance: Option<&[CountryAppearanceRow]>,
    ) -> Self {
        Self::of(
            countries
                .iter()
                .map(|country| (country.canonical_slot, country.tag.as_slice())),
            Rowset::Decoded(links),
            appearance.map(|rows| rows.iter().map(|row| (row.country, row.name_tag.as_slice()))),
        )
    }

    pub fn from_tables(frame: HourFrame<'a>, appearance: bool) -> Result<Self, String> {
        let failed = |why: std::io::Error| why.to_string();
        let countries = frame
            .complete::<tables::Countries>(COUNTRY_SLOT)
            .map_err(failed)?;
        let links = frame
            .complete::<tables::CountryLinks>(LINK_SLOT)
            .map_err(failed)?;
        let looks = appearance
            .then(|| frame.complete::<tables::CountryAppearance>(0))
            .transpose()
            .map_err(failed)?;
        let countries = countries
            .with_text(|row| &row.tag, COUNTRY_TAG)
            .map(|pair| pair.map(|(row, tag)| (row.canonical_slot, tag)))
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(failed)?;
        let looks = looks
            .map(|rows| {
                rows.keys()
                    .zip(rows.texts(|row| &row.name_tag, NAME_TAG))
                    .map(|(key, tag)| tag.map(|tag| (key as u32, tag)))
                    .collect::<std::io::Result<Vec<_>>>()
            })
            .transpose()
            .map_err(failed)?;
        Ok(Self::of(
            countries.into_iter(),
            links,
            looks.map(Vec::into_iter),
        ))
    }

    fn of<'t>(
        countries: impl Iterator<Item = (u32, &'t [u8])>,
        links: Rowset<'a, CountryLinkRow>,
        appearance: Option<impl Iterator<Item = (u32, &'t [u8])>>,
    ) -> Self {
        let tags = countries
            .map(|(slot, tag)| (slot, text(tag, "equipment country tag")))
            .collect();
        let original = appearance.map(|rows| {
            rows.filter_map(|(country, name_tag)| {
                Some((
                    link_slot_in(&links, country)?,
                    text(name_tag, "equipment original country tag"),
                ))
            })
            .collect()
        });
        Self {
            links,
            tags,
            original,
        }
    }

    pub fn tag_of(&self, raw: u32) -> Option<String> {
        let slot = link_slot_in(&self.links, raw)?;
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

    fn naming_tag(&self, raw: u32, original: bool) -> Result<Option<String>, String> {
        let Some(slot) = link_slot_in(&self.links, raw) else {
            return if raw == 0 {
                Ok(None)
            } else {
                Err(format!("country links lacks the equipment's naming country {raw}"))
            };
        };
        if original {
            let rows = self.original.as_ref().ok_or_else(|| {
                "country appearance is unavailable, so the equipment creator's original tag is unknown"
                    .to_owned()
            })?;
            match rows.get(&slot) {
                Some(Ok(tag)) if !tag.is_empty() => return Ok(Some(tag.clone())),
                Some(Ok(_)) => {}
                Some(Err(why)) => return Err(why.clone()),
                None => {
                    return Err(format!(
                        "country appearance lacks the equipment creator's row (country slot {slot})"
                    ));
                }
            }
        }
        match crate::triggers::refusal::reference(
            self.tags.get(&slot),
            raw != 0,
            || {
                crate::triggers::refusal::missing_reference(
                    replay_data::rows::TableId::Countries,
                    Some(crate::triggers::Store::Country(slot)),
                )
            },
        ) {
            Some(tag) => tag.clone().map(Some),
            None if raw == 0 => Ok(None),
            None => Err(format!("Countries lacks the equipment's naming country (country slot {slot})")),
        }
    }
}

pub fn raw_variant<'a>(
    definition: &VariantDefinitionRow,
    hour: i32,
    tags: &Tags,
    lexical: impl Into<replay_frame::frame::Rows<'a, LexicalTokenRow>>,
) -> RawVariant {
    let lexical = lexical.into();
    let literal = lexical
        .binary_search_by_key(&definition.equipment_token, |row| row.token)
        .ok()
        .map(|index| &lexical[index].literal[..]);
    raw_variant_spelled(definition, hour, tags, literal)
}

pub fn raw_variant_in(
    definition: &VariantDefinitionRow,
    hour: i32,
    tags: &Tags,
    lexical: &Rowset<'_, LexicalTokenRow>,
) -> RawVariant {
    let literal = lexical
        .text(u128::from(definition.equipment_token), |row| &row.literal, LITERAL)
        .ok()
        .flatten();
    raw_variant_spelled(definition, hour, tags, literal)
}

pub fn raw_variant_spelled(
    definition: &VariantDefinitionRow,
    hour: i32,
    tags: &Tags,
    literal: Option<&[u8]>,
) -> RawVariant {
    let mut refusals = Vec::new();
    let mut checked = |field: &str, bytes: &[u8], what: &str| match text(bytes, what) {
        Ok(text) => text,
        Err(why) => {
            refusals.push((String::from(field), why));
            String::new()
        }
    };
    let token = crate::triggers::refusal::reference(
        literal,
        definition.equipment_token != 0,
        || {
            crate::triggers::refusal::missing_reference(
                replay_data::rows::TableId::LexicalTokens,
                None,
            )
        },
    )
    .map(|literal| checked("token", literal, "equipment type token"))
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
                "lexical tokens lacks equipment type token {}",
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
            if definition.display_name.is_empty() {
                refusals.push(("name tag".into(), why));
            }
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
        type_tag,
        design: None,
        refusals,
    }
}

pub fn design_fingerprint(raw: &RawVariant, modules: &[(String, String, bool)], upgrades: &[(String, u8)]) -> Option<String> {
    if ["token", "display name", "suffix", "design"].iter().any(|field| raw.check(field).is_err()) {
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
        add(&[0xfe]);
        add(definition.as_bytes());
        add(&[*level]);
    }
    Some(format!("{hash:016x}"))
}

pub fn with_design(raw: &mut RawVariant, modules: &[&VariantModuleRow], upgrades: &[&VariantUpgradeRow], spell: &dyn Fn(u32) -> Result<String, String>) {
    let own: Result<Vec<(String, String, bool)>, String> = modules.iter().map(|row| Ok((spell(row.slot)?, spell(row.definition_token)?, row.definition_active != 0))).collect();
    let own_upgrades: Result<Vec<(String, u8)>, String> = upgrades
        .iter()
        .map(|row| {
            let definition = spell(row.definition_token as u32)?;
            if definition.is_empty() {
                return Err("equipment upgrade definition has no token spelling".into());
            }
            Ok((definition, row.level))
        })
        .collect();
    raw.design = match own.and_then(|own| own_upgrades.map(|upgrades| (own, upgrades))) {
        Ok((own, upgrades)) => design_fingerprint(raw, &own, &upgrades),
        Err(why) => {
            raw.refusals.push(("design".into(), why));
            None
        }
    };
}

pub trait Texts {
    fn has(&self, key: &str) -> bool;
    fn text(&self, key: &str) -> Option<String>;
    fn stored_text(&self, _text: &str, what: &str) -> Result<String, String> {
        Err(format!("{what} has no recorded/viewer language context"))
    }
    fn refuse(&self, _reason: &str, _identity: &str) {}
}

#[cfg(any(test, feature = "test-support"))]
impl Texts for content_reader_localisation::localisation::Localisation {
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

pub fn localise_for(texts: &dyn Texts, tag: Option<&str>, key: &str) -> Option<String> {
    let prefixed = tag.map(|tag| format!("{tag}_{key}"));
    let key = prefixed
        .as_deref()
        .filter(|prefixed| texts.has(prefixed))
        .unwrap_or(key);
    texts.text(key)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    struct Keys(&'static [(&'static str, &'static str)]);
    impl Texts for Keys {
        fn has(&self, key: &str) -> bool {
            self.0.iter().any(|(held, _)| *held == key)
        }
        fn text(&self, key: &str) -> Option<String> {
            self.0
                .iter()
                .find(|(held, _)| *held == key)
                .map(|(_, text)| (*text).to_owned())
        }
    }

    const TEXTS: Keys = Keys(&[
        ("mod_rifle", "Generic rifle"),
        ("SYN_mod_rifle", "Original rifle"),
        ("DYN_mod_rifle", "Dynamic rifle"),
    ]);

    fn countries() -> (Vec<CountryRow>, Vec<CountryLinkRow>) {
        let countries = [(4, 1, "SYN"), (5, 2, "DYN")]
            .map(|(id, canonical_slot, tag)| CountryRow {
                id,
                canonical_slot,
                tag: tag.as_bytes().to_vec(),
                ..Default::default()
            })
            .to_vec();
        let links = [(4, 1), (5, 2)]
            .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
            .to_vec();
        (countries, links)
    }

    fn appearance(country: u32, name_tag: &str) -> CountryAppearanceRow {
        CountryAppearanceRow {
            country,
            name_tag: name_tag.as_bytes().to_vec(),
            ..Default::default()
        }
    }

    fn design(creator_link: u32, origin_link: u32, display_name: &str) -> VariantDefinitionRow {
        VariantDefinitionRow {
            id: 70,
            equipment_token: 9,
            creator_link,
            origin_link,
            display_name: display_name.as_bytes().to_vec(),
            ..Default::default()
        }
    }

    const LEXICAL: &[LexicalTokenRow] = &[];

    fn lexical() -> Vec<LexicalTokenRow> {
        vec![LexicalTokenRow {
            token: 9,
            literal: b"mod_rifle".to_vec(),
        }]
    }

    #[test]
    fn a_creator_named_design_needs_its_original_tag_observation() {
        let (countries, links) = countries();
        let lexical = lexical();
        let rows = [appearance(4, ""), appearance(5, "SYN")];
        let name = |appearance: Option<&[CountryAppearanceRow]>, creator| {
            let tags = Tags::new(&countries, &links, appearance);
            let raw = raw_variant(&design(creator, 0, ""), 1, &tags, &lexical[..]);
            (raw.name_tag.clone(), variant_name(&raw, Some(&TEXTS)), raw)
        };
        let (tag, named, _) = name(Some(&rows[..]), 5);
        assert_eq!(tag.as_deref(), Some("SYN"));
        assert_eq!(named, Ok(Some("Original rifle".into())));
        let (tag, _, _) = name(Some(&rows[..]), 4);
        assert_eq!(tag.as_deref(), Some("SYN"));
        for (appearance, reason) in [
            (None, "country appearance is unavailable"),
            (Some(&rows[..1]), "country appearance lacks the equipment creator's row"),
        ] {
            let (tag, named, raw) = name(appearance, 5);
            assert_eq!(tag, None);
            assert!(raw.check("name tag").unwrap_err().contains(reason), "{reason}");
            assert!(named.unwrap_err().contains(reason), "{reason}");
        }
        let (tag, named, raw) = name(None, 0);
        assert_eq!(tag, None);
        assert!(raw.refusals.is_empty(), "{:?}", raw.refusals);
        assert_eq!(named, Ok(Some("Generic rifle".into())));
    }

    #[test]
    fn origin_and_own_names_do_not_need_the_creators_appearance() {
        let (countries, links) = countries();
        let lexical = lexical();
        let tags = Tags::new(&countries, &links, None);
        let origin = raw_variant(&design(5, 5, ""), 1, &tags, &lexical[..]);
        assert!(origin.refusals.is_empty(), "{:?}", origin.refusals);
        assert_eq!(origin.name_tag.as_deref(), Some("DYN"));
        assert_eq!(
            variant_name(&origin, Some(&TEXTS)),
            Ok(Some("Dynamic rifle".into()))
        );
        let own = raw_variant(&design(5, 0, "Stored design"), 1, &tags, LEXICAL);
        assert_eq!(own.name_tag, None);
        assert!(own.check("name tag").is_ok());
        assert_eq!(own.display_name, "Stored design");
    }

    #[test]
    fn an_unplaced_naming_country_refuses_the_name() {
        let (countries, links) = countries();
        let lexical = lexical();
        let rows = [appearance(4, ""), appearance(5, "SYN")];
        let tags = Tags::new(&countries, &links, Some(&rows[..]));
        let mut linked = links.clone();
        linked.push(CountryLinkRow { id: 6, canonical_slot: 3 });
        let unrowed = Tags::new(&countries, &linked, Some(&rows[..]));
        for (tags, creator, origin, reason) in [
            (&tags, 9, 0, "country links lacks"),
            (&tags, 4, 9, "country links lacks"),
            (&unrowed, 4, 6, "Countries lacks"),
        ] {
            let raw = raw_variant(&design(creator, origin, ""), 1, tags, &lexical[..]);
            assert_eq!(raw.name_tag, None);
            assert!(raw.check("name tag").unwrap_err().contains(reason), "{reason}");
            assert!(variant_name(&raw, Some(&TEXTS)).is_err(), "never the generic name");
        }
    }

    #[test]
    fn a_designs_fingerprint_is_its_modules_and_upgrades_and_refused_when_one_is_unspelled() {
        let (countries, links) = countries();
        let tags = Tags::new(&countries, &links, None);
        let spell = |token: u32| match token {
            0 => Ok(String::new()),
            1 => Ok("slot_a".to_owned()),
            2 => Ok("module_a".to_owned()),
            3 => Ok("upgrade_a".to_owned()),
            other => Err(format!("lexical tokens lacks equipment token {other}")),
        };
        let module = |definition_token, definition_active| VariantModuleRow { variant: 70, source_ordinal: 0, slot: 1, definition_token, definition_active };
        let upgrade = |level| VariantUpgradeRow { variant: 70, source_ordinal: 0, definition_token: 3, level };
        let print = |modules: &[&VariantModuleRow], upgrades: &[&VariantUpgradeRow]| {
            let mut raw = raw_variant(&design(4, 0, "Own"), 1, &tags, &lexical()[..]);
            with_design(&mut raw, modules, upgrades, &spell);
            raw
        };
        let (on, off, one, two) = (module(2, 1), module(2, 0), upgrade(1), upgrade(2));
        let base = print(&[&on], &[&one]).design.expect("spelled design");
        assert_eq!(base.len(), 16);
        assert_eq!(print(&[&on], &[&one]).design.as_deref(), Some(base.as_str()));
        assert_ne!(print(&[&off], &[&one]).design.as_deref(), Some(base.as_str()));
        assert_ne!(print(&[&on], &[&two]).design.as_deref(), Some(base.as_str()));
        assert_ne!(print(&[&on, &on], &[&one]).design.as_deref(), Some(base.as_str()));
        let unspelled = print(&[&module(9, 1)], &[&one]);
        assert_eq!(unspelled.design, None);
        assert!(unspelled.check("design").unwrap_err().contains("token 9"));
    }
}
