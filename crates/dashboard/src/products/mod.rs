//! Data products: one module per widget data set, registered once below.
//!
//! A product turns a source plus typed parameters into one compact reply that
//! covers a whole branch, so playback in the frontend is a local lookup. The
//! service, transport, host and frontend never name a product; they route by
//! its key.

pub mod artwork;
pub mod country_experience;
pub mod country_flags;
pub mod country_fuel;
pub mod country_groups;
pub mod country_measure;
pub mod country_stability;
pub mod country_styles;
pub mod equipment;
pub mod equipment_stockpile;
pub mod idea_format;
pub mod idea_history;
pub mod land_owners;
pub mod national_focus;
pub mod occupied_states;
pub mod party_history;
pub mod politics_content;
pub mod research;
pub mod resource_balance;
pub mod resources;
pub mod stability_parts;
pub mod subjects;
pub mod trade_flows;

use std::{io, sync::atomic::AtomicBool};

use installed_content::ResolveOptions;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use ts_rs::TS;

use crate::{Source, transport::bounded_json};

/// Game and user folders chosen in Settings, needed by products that read
/// artwork or localisation.
#[derive(Clone)]
pub struct ContentOptions {
    pub options: ResolveOptions,
    pub allow_unverified: bool,
}

pub struct LoadContext {
    pub content: Option<ContentOptions>,
}

impl LoadContext {
    pub fn content(&self) -> io::Result<&ContentOptions> {
        self.content
            .as_ref()
            .ok_or_else(|| crate::invalid("Choose the game and user folders in Settings first"))
    }
}

pub trait Product: 'static {
    const KEY: &'static str;
    type Params: DeserializeOwned + TS + 'static;
    type Reply: Serialize + TS + 'static;

    fn validate(_params: &Self::Params) -> Result<(), &'static str> {
        Ok(())
    }

    /// TypeScript declarations this product introduces, parameters and reply first.
    fn types() -> Vec<String> {
        vec![Self::Params::decl(), Self::Reply::decl()]
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply>;
}

/// Parameters shared by the per-country products.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CountryParams {
    pub branch_id: String,
    pub tag: String,
}

impl CountryParams {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.branch_id.parse::<u64>().is_err() {
            return Err("Invalid branch");
        }
        validate_tag(&self.tag)
    }

    pub fn branch(&self) -> u64 {
        self.branch_id.parse().expect("validated branch id")
    }
}

/// Most countries one line-chart request may name.
pub const MAX_COUNTRIES: usize = 16;

/// Selection checks shared by products drawing several countries.
pub fn validate_countries(branch_id: &str, tags: &[String]) -> Result<(), &'static str> {
    if tags.is_empty() || tags.len() > MAX_COUNTRIES {
        return Err("Invalid country selection");
    }
    for tag in tags {
        validate_tag(tag)?;
    }
    if branch_id.parse::<u64>().is_err() {
        return Err("Invalid branch");
    }
    Ok(())
}

/// The one check for a requested country tag, shared by products and the map.
/// Tags are lookup keys compared with recorded tags, never paths. The engine's
/// tag loader (`CCountryDatabase` load_tags, `14070e900`) takes any non-NUL
/// bytes, so no character set is assumed; only emptiness, NUL and a generous
/// length bound are refused.
pub fn validate_tag(tag: &str) -> Result<(), &'static str> {
    if tag.is_empty() || tag.len() > 128 || tag.contains('\0') {
        return Err("Invalid country tag");
    }
    Ok(())
}

/// The rows a table holds this hour, complete or truncated; an unavailable table
/// holds none. For products that use whatever rows were recorded.
pub(crate) fn recorded_rows<T: replay::rows::TableSchema>(
    frame: &replay::frame::DecodedFrame,
) -> io::Result<&[T::Row]> {
    use replay::frame::TableView;
    match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
            rows.rows::<T>().ok_or_else(|| {
                crate::invalid(format!(
                    "hour {} {:?} contains another table's rows",
                    frame.hour,
                    T::ID
                ))
            })
        }
        TableView::Unavailable { .. } => Ok(&[]),
    }
}

/// Serialised with the lease identity the frontend used, so a reply can never
/// be attributed to another recording.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Envelope<'a, T: Serialize> {
    source_id: &'a str,
    revision: &'a str,
    product: &'static str,
    data: T,
}

/// A product erased to its key and JSON entry points.
pub struct Registered {
    pub key: &'static str,
    pub load:
        fn(&Source, &str, serde_json::Value, &LoadContext, &AtomicBool) -> Result<String, String>,
    pub types: fn() -> Vec<String>,
}

const fn register<P: Product>() -> Registered {
    Registered {
        key: P::KEY,
        load: load_json::<P>,
        types: types::<P>,
    }
}

fn load_json<P: Product>(
    source: &Source,
    lease_id: &str,
    params: serde_json::Value,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> Result<String, String> {
    let params: P::Params = serde_json::from_value(params)
        .map_err(|error| format!("Invalid {} request: {error}", P::KEY))?;
    P::validate(&params).map_err(str::to_owned)?;
    let data = P::load(source, &params, context, cancelled).map_err(|error| error.to_string())?;
    bounded_json(
        &Envelope {
            source_id: lease_id,
            revision: &source.key().revision,
            product: P::KEY,
            data,
        },
        cancelled,
    )
}

fn types<P: Product>() -> Vec<String> {
    P::types()
}

/// Every product the service answers. Add a product here and in the frontend
/// widget registry; nothing else needs to change.
pub static PRODUCTS: &[Registered] = &[
    register::<national_focus::NationalFocus>(),
    register::<research::Research>(),
    register::<country_flags::CountryFlags>(),
    register::<land_owners::LandOwnersProduct>(),
    register::<country_styles::CountryStyles>(),
    register::<country_groups::CountryGroups>(),
    register::<country_stability::CountryStability>(),
    register::<stability_parts::StabilityParts>(),
    register::<country_measure::CountryMeasure>(),
    register::<country_fuel::CountryFuel>(),
    register::<occupied_states::OccupiedStates>(),
    register::<country_experience::CountryExperience>(),
    register::<idea_history::IdeaHistory>(),
    register::<party_history::PartyHistory>(),
    register::<resource_balance::ResourceBalance>(),
    register::<equipment_stockpile::EquipmentStockpile>(),
    register::<trade_flows::TradeFlows>(),
    register::<subjects::Subjects>(),
];

/// A recorded character's script token: its lexer id (`CCharacter` +0x18,
/// the row's `name_token_id`) resolved through LexicalTokens. The row's `token`
/// field is the saved name (+0x48), not the script token. None when the id is
/// negative or not in the dictionary.
pub fn character_script_token<'a>(
    character: &replay::rows::CharacterRow,
    lexical: &'a [replay::rows::LexicalTokenRow],
) -> Option<&'a str> {
    let token = u32::try_from(character.name_token_id).ok()?;
    let index = lexical.binary_search_by_key(&token, |row| row.token).ok()?;
    std::str::from_utf8(&lexical[index].literal).ok()
}

/// The template a recorded character holds (CCharacter+0x20), from its
/// Characters row's template columns, its tag (+0x8) a lexer token spelled
/// by LexicalTokens; `Err` names what the hour does not hold.
pub(crate) fn built_template(
    character: &replay::rows::CharacterRow,
    lexical: Option<&[replay::rows::LexicalTokenRow]>,
) -> Result<installed_content::characters::BuiltTemplate, String> {
    let id = character.id;
    if character.template_present == 0 {
        return Err(format!(
            "character {id} holds no template (CCharacter+0x20 is null)"
        ));
    }
    let token = character.template_tag;
    let lexical = lexical.ok_or_else(|| {
        format!("character {id}'s template tag token {token} is not spelled: LexicalTokens is incomplete")
    })?;
    let index = lexical
        .binary_search_by_key(&token, |row| row.token)
        .map_err(|_| {
            format!("character {id}'s template tag token {token} has no LexicalTokens row")
        })?;
    let tag = std::str::from_utf8(&lexical[index].literal)
        .map_err(|_| format!("character {id}'s template tag token {token} is not UTF-8"))?;
    Ok(installed_content::characters::BuiltTemplate {
        tag: tag.to_owned(),
        file: character.template_file.clone(),
        line: character.template_line,
        generated: character.template_generated != 0,
    })
}

/// A country's CountryAppearance fields idea naming reads: its original
/// tag (`name_tag`, set exactly for a country
/// ccountry_tag::name_substitution_predicate 140BA61F0 holds for: canonical
/// slot at or past CCountryDatabase+0x88) and its cosmetic tag. None when the
/// frame's table does not hold the row.
pub(crate) fn appearance(frame: &replay::frame::DecodedFrame, link: u32) -> Option<(&str, &str)> {
    let rows = frame
        .complete::<replay::rows::tables::CountryAppearance>()
        .ok()?;
    let row = rows.iter().find(|row| row.country == link)?;
    Some((
        std::str::from_utf8(&row.name_tag).ok()?,
        std::str::from_utf8(&row.cosmetic_tag).ok()?,
    ))
}

/// Whether GetCountryIdeologyGroupIndex (140D2BD20) holds for a country at
/// the frame's hour, as CIdea::GetDisplayNameForCountry (140FBCF90) and
/// `has_civil_war` (1403D3670) call it on the country's CDiplomacyStatus
/// (CCountry+0xF88, whose +0x288 is the country itself): a country of its
/// civil-war list (CCountry+0x12F0, CountryCivilWars) exists (+0x484 > 0,
/// CountryCounterInputs.owned_state_count), or an existing enemy
/// (CDiplomacyStatus+0x98) holds it in its own civil-war list. Each loop
/// only asks whether one holds; the enemy's list is searched by canonical
/// slot (a lower bound over the sorted list, then equality). `Err` names the
/// input the hour lacks.
pub fn civil_war(frame: &replay::frame::DecodedFrame, slot: u32) -> Result<bool, String> {
    use replay::rows::tables;
    let hour = frame.hour;
    let countries = frame
        .complete::<tables::Countries>()
        .map_err(|error| error.to_string())?;
    let links = frame
        .complete::<tables::CountryLinks>()
        .map_err(|error| error.to_string())?;
    let lists = frame
        .complete::<tables::CountryCivilWars>()
        .map_err(|error| error.to_string())?;
    let counters = frame
        .complete::<tables::CountryCounterInputs>()
        .map_err(|error| error.to_string())?;
    let raw_of = |slot: u32| {
        countries
            .iter()
            .find(|country| country.canonical_slot == slot)
            .map(|country| country.id)
            .ok_or_else(|| format!("country slot {slot} has no Countries row at hour {hour}"))
    };
    let exists = |slot: u32| -> Result<bool, String> {
        let raw = raw_of(slot)?;
        counters
            .iter()
            .find(|row| row.country == raw)
            .map(|row| row.owned_state_count > 0)
            .ok_or_else(|| {
                format!(
                    "the owned state count (CountryCounterInputs.owned_state_count, CCountry+0x484) of country {raw} is not recorded at hour {hour}"
                )
            })
    };
    // The country's own list (+0x288 +0x12F0), its raw owner as recorded.
    let own = raw_of(slot)?;
    for row in lists.iter().filter(|row| row.country == own) {
        if exists(linked(links, row.target, "CountryCivilWars", hour)?)? {
            return Ok(true);
        }
    }
    for enemy in enemies(frame, links, slot)? {
        if !exists(enemy)? {
            continue;
        }
        let raw = raw_of(enemy)?;
        for row in lists.iter().filter(|row| row.country == raw) {
            if linked(links, row.target, "CountryCivilWars", hour)? == slot {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// A raw country reference's canonical slot, or why the hour cannot say.
fn linked(
    links: &[replay::rows::CountryLinkRow],
    raw: u32,
    table: &str,
    hour: i32,
) -> Result<u32, String> {
    crate::metadata::link_slot(links, raw).ok_or_else(|| {
        format!("{table} names country {raw}, which CountryLinks does not hold at hour {hour}")
    })
}

/// A country's enemy vector (CDiplomacyStatus+0x98) as a set: CountryWars'
/// where the recorder read it, else the other country of each ongoing war
/// relation, which RebuildWartimeAllies (140D31E20), its only runtime
/// writer, keeps it to.
fn enemies(
    frame: &replay::frame::DecodedFrame,
    links: &[replay::rows::CountryLinkRow],
    slot: u32,
) -> Result<Vec<u32>, String> {
    use replay::rows::tables;
    let hour = frame.hour;
    let wars = frame
        .complete::<tables::CountryWars>()
        .map_err(|error| error.to_string())?;
    for row in wars {
        if row.enemies_read != 0 && linked(links, row.country, "CountryWars", hour)? == slot {
            return row
                .enemies
                .iter()
                .map(|&raw| linked(links, raw, "CountryWars", hour))
                .collect();
        }
    }
    let relations = frame
        .complete::<tables::WarRelations>()
        .map_err(|error| error.to_string())?;
    let mut out = Vec::new();
    for row in relations
        .iter()
        .filter(|row| country_groups::war_ongoing(row.start_hour, row.end_hour, hour))
    {
        let first = linked(links, row.first_country, "WarRelations", hour)?;
        let second = linked(links, row.second_country, "WarRelations", hour)?;
        if first == slot {
            out.push(second);
        } else if second == slot {
            out.push(first);
        }
    }
    Ok(out)
}

/// A country as idea naming needs it at one hour, owned for the scans that
/// keep it: its canonical slot and its appearance ([`Look`]), `None` when
/// the hour lacks its CountryAppearance row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct NamingCountry {
    pub slot: u32,
    pub look: Option<Look>,
}

/// A country's CountryAppearance `name_tag` and `cosmetic_tag`
/// ([`appearance`]), and whether 140D2BD20 holds for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Look {
    pub name_tag: String,
    pub cosmetic_tag: String,
    /// [`civil_war`] for a dynamic country (`name_tag` set, so 140BA61F0
    /// holds); `Ok(false)` for any other, which 140FBCF90 never asks.
    pub civil_war: Result<bool, String>,
}

impl Look {
    /// The country's appearance at the frame's hour, `None` without its
    /// CountryAppearance row.
    pub fn read(
        frame: &replay::frame::DecodedFrame,
        row: &replay::rows::CountryRow,
    ) -> Option<Self> {
        let (name_tag, cosmetic_tag) = appearance(frame, row.id)?;
        Some(Self {
            name_tag: name_tag.to_owned(),
            cosmetic_tag: cosmetic_tag.to_owned(),
            civil_war: if name_tag.is_empty() {
                Ok(false)
            } else {
                civil_war(frame, row.canonical_slot)
            },
        })
    }

    pub fn country_look(&self) -> politics_content::CountryLook<'_> {
        politics_content::CountryLook {
            name_tag: &self.name_tag,
            cosmetic_tag: &self.cosmetic_tag,
            civil_war: self.civil_war.as_ref().copied().map_err(String::as_str),
        }
    }
}

impl NamingCountry {
    pub fn read(frame: &replay::frame::DecodedFrame, row: &replay::rows::CountryRow) -> Self {
        Self {
            slot: row.canonical_slot,
            look: Look::read(frame, row),
        }
    }

    pub fn for_tag<'a>(&'a self, tag: &'a str) -> politics_content::IdeaCountry<'a> {
        politics_content::IdeaCountry {
            tag,
            slot: self.slot,
            look: self.look.as_ref().map(Look::country_look),
        }
    }
}

pub fn find(key: &str) -> Option<&'static Registered> {
    PRODUCTS.iter().find(|product| product.key == key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_keys_are_unique_and_exported_types_are_named_once() {
        let mut keys: Vec<_> = PRODUCTS.iter().map(|product| product.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), PRODUCTS.len());
        let contract = crate::typescript_contract();
        let names: Vec<_> = contract
            .lines()
            .filter_map(|line| line.strip_prefix("export type "))
            .map(|rest| rest.split_whitespace().next().unwrap_or_default())
            .collect();
        for name in [
            "CountryParams",
            "CountryFlagSize",
            "CountryFlagImage",
            "LandOwners",
            "FocusReply",
            "ResearchReply",
            "CountryStyleReply",
            "CountryGroupReply",
            "CountryStabilityReply",
            "StabilityPartsReply",
            "FixedRun",
            "MeasureReply",
            "FuelReply",
            "ExperienceReply",
            "IdeaHistoryReply",
            "PartyHistoryReply",
            "ResourceBalanceReply",
            "EquipmentStockpileReply",
            "AmountRun",
            "ResourceInfo",
            "TradeFlowsReply",
            "SubjectReply",
        ] {
            assert_eq!(
                names.iter().filter(|n| **n == name).count(),
                1,
                "{name} must be declared exactly once"
            );
        }
    }
}
