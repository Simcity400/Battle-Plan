//! Character-manager ownership, role state and persistent command abilities.

use super::{
    Decimal, GameDate, block, boolean, bytes, decimal, field, integer, invalid, object_id,
    required_block, required_bytes, scalar, typed_handle,
};
use installed_content_core::parser::Item;
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    /// Every serialized unit-leader object, including leaders outside characters.
    pub portraits: BTreeMap<u64, [Vec<u8>; 3]>,
    /// Missing manager is not an empty native roster.
    pub manager: Option<Manager>,
    /// Every saved country is present, including countries with no records.
    pub abilities: BTreeMap<Vec<u8>, Vec<Ability>>,
    /// Every saved country's appointed advisors, in written array order.
    /// The country writer includes `characters` only for a non-null manager;
    /// the manager writes `appointed_advisors` only for a nonzero count, so
    /// either omission is an empty list.
    pub appointed_advisors: BTreeMap<Vec<u8>, Vec<AppointedAdvisor>>,
    /// Every saved country's `character_status` entries, in written order.
    /// The manager writer emits one entry per distinct character in the
    /// active-character records, ordered by the character's reference
    /// through a map, not in array order; a missing `characters` block
    /// (a null manager) is an empty list.
    pub character_status: BTreeMap<Vec<u8>, Vec<CharacterStatus>>,
    /// Every saved country's `retired_character_status` entries, with the
    /// same record shape as active statuses, in written order; a missing
    /// `characters` block is an empty list.
    pub retired_character_status: BTreeMap<Vec<u8>, Vec<CharacterStatus>>,
}

/// One `character_status` entry: the writer saves the record's character
/// reference and its four status bytes as booleans.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterStatus {
    /// Typed handle of the record's character.
    pub character: u64,
    /// The `advisor` status byte, set when the character's advisor roles
    /// are registered; `None` when omitted.
    pub advisor: Option<bool>,
    /// The writer saves the stored unit-leader status byte under `unit_leader`.
    pub unit_leader: Option<bool>,
}

/// The entries of one status list (`character_status` or
/// `retired_character_status`) of a country's `characters` block.
fn character_status(country: &[Item], key: &[u8]) -> io::Result<Vec<CharacterStatus>> {
    let Some(characters) = field(country, b"characters")? else {
        return Ok(Vec::new());
    };
    block(characters)?
        .iter()
        .filter(|item| item.key.as_deref() == Some(key))
        .map(|item| {
            let fields = block(&item.value)?;
            Ok(CharacterStatus {
                character: typed_handle(
                    field(fields, b"character")?
                        .ok_or_else(|| invalid("character_status lacks its character"))?,
                )?,
                advisor: field(fields, b"advisor")?.map(boolean).transpose()?,
                unit_leader: field(fields, b"unit_leader")?.map(boolean).transpose()?,
            })
        })
        .collect()
}

/// One `appointed_advisors` entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppointedAdvisor {
    /// The advisor's slot string, written even when empty.
    pub slot: Vec<u8>,
    /// Typed handle of the advisor's character.
    pub character: u64,
}

fn appointed_advisors(country: &[Item]) -> io::Result<Vec<AppointedAdvisor>> {
    let Some(characters) = field(country, b"characters")? else {
        return Ok(Vec::new());
    };
    let Some(list) = field(block(characters)?, b"appointed_advisors")? else {
        return Ok(Vec::new());
    };
    let advisors = block(list)?
        .iter()
        .map(|entry| {
            if entry.key.is_some() {
                return Err(invalid("appointed advisor entries are anonymous blocks"));
            }
            let fields = block(&entry.value)?;
            Ok(AppointedAdvisor {
                slot: bytes(fields, b"slot")?
                    .ok_or_else(|| invalid("appointed advisor lacks its slot"))?,
                character: typed_handle(
                    field(fields, b"character")?
                        .ok_or_else(|| invalid("appointed advisor lacks its character"))?,
                )?,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    if advisors.is_empty() {
        return Err(invalid(
            "character writer emits appointed_advisors only for a nonzero count",
        ));
    }
    Ok(advisors)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manager {
    pub characters: BTreeMap<u64, Character>,
    pub unit_leaders: BTreeMap<u64, UnitLeader>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Character {
    /// The writer saves the stored gender enum as a token.
    pub gender: Option<Vec<u8>>,
    pub portraits: Option<Vec<Portrait>>,
    pub id: u64,
    pub roster: u8,
    pub ordinal: u32,
    /// The character's lexer token, distinct from its name string.
    pub token: Vec<u8>,
    /// The writer emits `template` only with a template, as the lexer literal
    /// of the template's tag.
    pub template: Option<Vec<u8>>,
    /// The writer saves the character's own flag manager as `flags` only
    /// when its count is positive; omission is no flags.
    pub flags: Option<Vec<super::intel::Flag>>,
    /// The writer saves the character's variables unless they hold no entry
    /// and their random state is the initial value of one, so omission is
    /// no variables.
    pub variables: Option<super::intel::Variables>,
    /// The writer saves the name, omitting an empty string. The separate
    /// stored string is not serialized by this writer.
    pub name: Option<Vec<u8>>,
    /// Omission proves the raw country reference is signed-nonpositive.
    pub country: Option<Vec<u8>>,
    pub country_leaders: Vec<CountryLeader>,
    pub advisors: Vec<Advisor>,
    /// Present exactly when the character owns a scientist role.
    pub scientist: Option<Scientist>,
    /// The writer saves the unit-leader reference as `operative` (`id` then
    /// `type`) only when either word is nonzero, so omission is none; the
    /// handle is the stored eight bytes (type low, id high).
    pub operative: Option<u64>,
}

/// A character's serialized scientist role.
///
/// The character writer emits `scientist` exactly when the role pointer is
/// non-null. The role keeps separate trait and specialization vectors. Its
/// initial skill state contains one zero-level record per specialization
/// definition, which is why every saved scientist lists the same keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scientist {
    /// The writer emits `is_assigned` only when the assigned byte is nonzero.
    pub active: bool,
    /// The scientist writer emits `traits` only when the trait count is
    /// nonzero, so omission proves an empty trait vector.
    pub traits: Option<Vec<Vec<u8>>>,
    /// The scientist writer emits its embedded `skills` unconditionally.
    pub skills: Vec<ScientistSkill>,
}

/// One `specialization_*` block. The loader matches the saved key against
/// each record's own token, so the key identifies the record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScientistSkill {
    pub name: Vec<u8>,
    /// The skill writer emits `level` only while the stored signed
    /// i32 is strictly positive, so omission proves a nonpositive level and
    /// cannot distinguish zero from a negative raw value.
    pub level: Option<i32>,
    /// The same writer emits the record's i64 experience under the same strictly
    /// positive guard; omission likewise proves a nonpositive raw value.
    pub experience: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryLeader {
    /// The writer saves the role's ideology definition token, not its map node.
    pub ideology: Vec<u8>,
    /// The writer saves the role's id only when it is not -1,
    /// so omission is -1.
    pub id: Option<i32>,
    /// The writer saves each trait's token in array order as
    /// `traits` only for a nonzero count, so omission is no traits.
    pub traits: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advisor {
    /// The advisor writer omits each of these strings exactly when its length is zero.
    /// Presence remains explicit because a nonempty string beginning with NUL
    /// serializes an empty literal, which differs from an omitted empty string.
    pub slot: Option<Vec<u8>>,
    pub idea: Option<Vec<u8>>,
    /// The advisor writer saves each trait's name in order as `traits` only
    /// for a nonzero count, so omission is no traits.
    pub traits: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitLeader {
    pub id: u64,
    pub character: u64,
    /// Role categories are field_marshal=0, corps_commander=1, navy_leader=2.
    pub category: i32,
    /// The leader writer saves the name only when nonempty in text mode.
    pub name: Option<Vec<u8>>,
    pub skill: i32,
    /// Native i64 fixed5, omitted precisely at zero by the leader writer.
    pub experience: Decimal,
    /// Army attack/defense/planning/logistics; navy attack/defense/maneuvering/
    /// coordination. The writers omit each signed integer precisely at zero.
    pub components: [i32; 4],
    pub traits: Vec<Vec<u8>>,
    /// Independent native order. Missing progress entries never invent zero.
    pub trait_experience: Vec<TraitExperience>,
    /// `traits_to_remove`: the writer saves the timed-trait vector in native
    /// order as each trait's token and signed remaining days,
    /// and omits the block for an empty vector.
    pub timed_traits: Vec<TimedTrait>,
    /// The writer emits `captured=yes`, with the captor and location, only
    /// when the captured byte is set.
    pub captured: bool,
    /// The writer emits the government-in-exile tag only when positive.
    pub government_in_exile_tag: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitExperience {
    pub token: Vec<u8>,
    pub experience: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedTrait {
    pub token: Vec<u8>,
    pub days: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ability {
    pub token: Vec<u8>,
    pub start_date: GameDate,
    pub country: Option<Vec<u8>>,
    pub leader: Option<u64>,
    /// The writer emits the signed value exactly when the caster is nonnull. Native
    /// storage remains initialized and captured even when this field is omitted.
    pub num_units: Option<i32>,
    /// The target's region id, not a typed formation identity.
    pub strategic_region: Option<i32>,
    /// Omission proves raw allocated cost <= 0, not necessarily zero.
    pub cost: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Portrait {
    pub category: Vec<u8>,
    pub size: Vec<u8>,
    pub identity: Vec<u8>,
}
fn portraits(items: &[Item]) -> io::Result<Option<Vec<Portrait>>> {
    field(items, b"portraits")?
        .map(|v| {
            let mut result = Vec::new();
            for category in block(v)? {
                let name = category
                    .key
                    .clone()
                    .ok_or_else(|| invalid("portrait category missing"))?;
                for size in block(&category.value)? {
                    result.push(Portrait {
                        category: name.clone(),
                        size: size
                            .key
                            .clone()
                            .ok_or_else(|| invalid("portrait size missing"))?,
                        identity: scalar(&size.value)?.to_vec(),
                    });
                }
            }
            Ok(result)
        })
        .transpose()
}
// The leader writer saves exactly these nonempty strings. Walk all role wrappers rather
// than assuming every leader is owned by the character manager.
fn leader_portraits(items: &[Item], out: &mut BTreeMap<u64, [Vec<u8>; 3]>) -> io::Result<()> {
    for item in items {
        if let installed_content_core::parser::Value::Block(children) = &item.value {
            if matches!(
                item.key.as_deref(),
                Some(b"field_marshal" | b"corps_commander" | b"navy_leader" | b"operative")
            ) && field(children, b"id")?
                .is_some_and(|value| matches!(value, installed_content_core::parser::Value::Block(_)))
            {
                let id = object_id(children)?;
                let mut values: [Vec<u8>; 3] = Default::default();
                for (value, key) in
                    values
                        .iter_mut()
                        .zip([b"portrait_path".as_slice(), b"picture", b"gfx"])
                {
                    *value = bytes(children, key)?.unwrap_or_default();
                }
                if out.insert(id, values).is_some() {
                    return Err(invalid("duplicate serialized leader portrait identity"));
                }
            }
            leader_portraits(children, out)?;
        }
    }
    Ok(())
}

fn optional_i32(items: &[Item], key: &[u8]) -> io::Result<Option<i32>> {
    bytes(items, key)?.map(|value| integer(&value)).transpose()
}

fn list(items: &[Item], key: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    field(items, key)?
        .map(|value| {
            block(value)?
                .iter()
                .map(|entry| {
                    if entry.key.is_some() {
                        return Err(invalid(
                            "leader traits must be an ordinary ordered token list",
                        ));
                    }
                    Ok(scalar(&entry.value)?.to_vec())
                })
                .collect()
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

fn scientist(items: &[Item]) -> io::Result<Scientist> {
    let skills = required_block(items, b"skills")?
        .iter()
        .map(|entry| {
            let name = entry.key.clone().ok_or_else(|| {
                invalid("saved scientist specialization lacks its definition name")
            })?;
            let fields = block(&entry.value)?;
            let level = optional_i32(fields, b"level")?;
            let experience = decimal(fields, b"experience")?;
            if level.is_some_and(|level| level <= 0)
                || experience.as_ref().is_some_and(|value| value.raw <= 0)
            {
                return Err(invalid(
                    "skill writer omits every nonpositive specialization level and experience",
                ));
            }
            Ok(ScientistSkill {
                name,
                level,
                experience,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok(Scientist {
        active: field(items, b"is_assigned")?
            .map(boolean)
            .transpose()?
            .unwrap_or(false),
        traits: field(items, b"traits")?
            .map(|value| {
                block(value)?
                    .iter()
                    .map(|entry| {
                        if entry.key.is_some() {
                            return Err(invalid(
                                "scientist traits must be an ordinary ordered token list",
                            ));
                        }
                        Ok(scalar(&entry.value)?.to_vec())
                    })
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?,
        skills,
    })
}

fn unit_leader(character: u64, category: i32, items: &[Item]) -> io::Result<UnitLeader> {
    let component_keys: [&[u8]; 4] = if category == 2 {
        [
            b"attack_skill",
            b"defense_skill",
            b"maneuvering_skill",
            b"coordination_skill",
        ]
    } else {
        [
            b"attack_skill",
            b"defense_skill",
            b"planning_skill",
            b"logistics_skill",
        ]
    };
    let mut components = [0; 4];
    for (component, key) in components.iter_mut().zip(component_keys) {
        *component = optional_i32(items, key)?.unwrap_or(0);
    }
    let mut trait_experience = Vec::new();
    if let Some(progress) = field(items, b"in_progress")? {
        for entry in block(progress)? {
            let token = entry
                .key
                .clone()
                .ok_or_else(|| invalid("trait progress lacks its token"))?;
            trait_experience.push(TraitExperience {
                token,
                experience: Decimal::parse(scalar(&entry.value)?)?,
            });
        }
    }
    let mut timed_traits = Vec::new();
    if let Some(timed) = field(items, b"traits_to_remove")? {
        for entry in block(timed)? {
            let token = entry
                .key
                .clone()
                .ok_or_else(|| invalid("timed trait lacks its token"))?;
            timed_traits.push(TimedTrait {
                token,
                days: integer(scalar(&entry.value)?)?,
            });
        }
    }
    Ok(UnitLeader {
        id: object_id(items)?,
        character,
        category,
        name: bytes(items, b"name")?,
        skill: integer(&required_bytes(items, b"skill")?)?,
        experience: decimal(items, b"experience")?.unwrap_or(Decimal::parse(b"0")?),
        components,
        traits: list(items, b"traits")?,
        trait_experience,
        timed_traits,
        captured: bytes(items, b"captured")?.as_deref() == Some(b"yes"),
        government_in_exile_tag: bytes(items, b"government_in_exile_tag")?,
    })
}

fn manager(items: &[Item]) -> io::Result<Manager> {
    let mut manager = Manager::default();
    for (roster, key) in [(0, b"historical".as_slice()), (1, b"dynamic".as_slice())] {
        // The portrait writer omits only empty vectors; both vectors preserve order.
        let Some(value) = field(items, key)? else {
            continue;
        };
        for (ordinal, entry) in block(value)?.iter().enumerate() {
            if entry.key.as_deref() != Some(b"character") {
                return Err(invalid("character roster contains an unsupported entry"));
            }
            let fields = block(&entry.value)?;
            let id = object_id(fields)?;
            let mut country_leaders = Vec::new();
            if let Some(roles) = field(fields, b"country_leaders")? {
                for role in block(roles)? {
                    if role.key.as_deref() != Some(b"country_leader") {
                        return Err(invalid(
                            "country leader roster contains an unsupported entry",
                        ));
                    }
                    let fields = block(&role.value)?;
                    let id = optional_i32(fields, b"id")?;
                    if id == Some(-1) {
                        return Err(invalid("country leader writer omits an id of -1"));
                    }
                    country_leaders.push(CountryLeader {
                        ideology: required_bytes(fields, b"ideology")?,
                        id,
                        traits: list(fields, b"traits")?,
                    });
                }
            }
            let mut advisors = Vec::new();
            if let Some(roles) = field(fields, b"advisors")? {
                for role in block(roles)? {
                    if role.key.as_deref() != Some(b"advisor") {
                        return Err(invalid("advisor roster contains an unsupported entry"));
                    }
                    let fields = block(&role.value)?;
                    advisors.push(Advisor {
                        slot: bytes(fields, b"slot")?,
                        idea: bytes(fields, b"idea_token")?,
                        traits: list(fields, b"traits")?,
                    });
                }
            }
            let character = Character {
                portraits: portraits(fields)?,
                id,
                roster,
                ordinal: u32::try_from(ordinal)
                    .map_err(|_| invalid("character ordinal exceeds u32"))?,
                token: bytes(fields, b"token")?.ok_or_else(|| invalid("character lacks token"))?,
                template: bytes(fields, b"template")?,
                gender: bytes(fields, b"gender")?,
                flags: field(fields, b"flags")?
                    .map(|value| {
                        let flags = super::intel::parse_flags(block(value)?)?;
                        if flags.is_empty() {
                            return Err(invalid(
                                "character writer emits flags only for a positive count",
                            ));
                        }
                        Ok(flags)
                    })
                    .transpose()?,
                variables: field(fields, b"variables")?
                    .map(|value| super::intel::parse_variables(block(value)?))
                    .transpose()?,
                name: bytes(fields, b"name")?,
                country: bytes(fields, b"country")?,
                country_leaders,
                advisors,
                scientist: field(fields, b"scientist")?
                    .map(|value| scientist(block(value)?))
                    .transpose()?,
                operative: field(fields, b"operative")?.map(typed_handle).transpose()?,
            };
            if manager.characters.insert(id, character).is_some() {
                return Err(invalid(
                    "duplicate full character handle across native rosters",
                ));
            }
            let mut has_leader = false;
            for (category, key) in [
                (0, b"field_marshal".as_slice()),
                (1, b"corps_commander"),
                (2, b"navy_leader"),
            ] {
                if let Some(value) = field(fields, key)? {
                    if has_leader {
                        return Err(invalid(
                            "character serializes more than one unit leader role",
                        ));
                    }
                    has_leader = true;
                    let leader = unit_leader(id, category, block(value)?)?;
                    if manager.unit_leaders.insert(leader.id, leader).is_some() {
                        return Err(invalid("duplicate full unit leader handle"));
                    }
                }
            }
        }
    }
    Ok(manager)
}

fn ability(items: &[Item]) -> io::Result<Ability> {
    let leader = field(items, b"leader")?.map(typed_handle).transpose()?;
    let num_units = optional_i32(items, b"num_units")?;
    if leader.is_some() != num_units.is_some() {
        return Err(invalid(
            "active ability must save num_units exactly when its caster is nonnull",
        ));
    }
    let cost = decimal(items, b"cost")?;
    if cost.as_ref().is_some_and(|cost| cost.raw <= 0) {
        return Err(invalid(
            "active ability saves allocated cost only when positive",
        ));
    }
    Ok(Ability {
        token: bytes(items, b"id")?.ok_or_else(|| invalid("active ability lacks id"))?,
        start_date: GameDate::parse(&required_bytes(items, b"start_date")?)?,
        country: bytes(items, b"country")?,
        leader,
        num_units,
        strategic_region: optional_i32(items, b"strategic_region")?,
        cost,
    })
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    parse_with_countries(items, super::CountriesView::from_root(items)?)
}

pub(super) fn parse_with_countries(
    items: &[Item],
    countries: super::CountriesView<'_>,
) -> io::Result<Pack> {
    let mut portraits = BTreeMap::new();
    for item in items {
        if countries.is_root_item(item) {
            for country in countries.iter() {
                leader_portraits(std::slice::from_ref(country), &mut portraits)?;
            }
        } else {
            leader_portraits(std::slice::from_ref(item), &mut portraits)?;
        }
    }
    let manager = field(items, b"character_manager")?
        .map(|value| manager(block(value)?))
        .transpose()?;
    let mut abilities = BTreeMap::new();
    let mut appointed = BTreeMap::new();
    let mut status = BTreeMap::new();
    let mut retired = BTreeMap::new();
    for country in countries.required()? {
        let tag = country
            .key
            .clone()
            .ok_or_else(|| invalid("ability country lacks a tag"))?;
        let mut rows = Vec::new();
        // The country writer emits one direct block per value-array entry, in native order.
        for item in block(&country.value)?
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"active_ability"))
        {
            rows.push(ability(block(&item.value)?)?);
        }
        appointed.insert(tag.clone(), appointed_advisors(block(&country.value)?)?);
        status.insert(
            tag.clone(),
            character_status(block(&country.value)?, b"character_status")?,
        );
        retired.insert(
            tag.clone(),
            character_status(block(&country.value)?, b"retired_character_status")?,
        );
        if abilities.insert(tag, rows).is_some() {
            return Err(invalid("duplicate country in ability roster"));
        }
    }
    Ok(Pack {
        portraits,
        manager,
        abilities,
        appointed_advisors: appointed,
        character_status: status,
        retired_character_status: retired,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod country_view_tests {
    use super::*;

    #[test]
    fn portrait_scan_keeps_unrelated_nested_countries_and_duplicate_checks() {
        let doc = installed_content_core::parser::parse(
            br#"countries={X={}}
                wrapper={countries={operative={id={id=7 type=73} portrait_path="nested.dds"}}}
                navy_leader={id={id=8 type=73} portrait_path="root.dds"}"#,
        )
        .unwrap();
        let pack = parse(&doc.items).unwrap();
        assert_eq!(pack.portraits.len(), 2);
        assert_eq!(pack.portraits[&((7 << 32) | 73)][0], b"nested.dds");
        assert_eq!(pack.portraits[&((8 << 32) | 73)][0], b"root.dds");

        let doc = installed_content_core::parser::parse(
            br#"countries={X={}}
                wrapper={countries={operative={id={id=7 type=73} portrait_path="nested.dds"}}}
                operative={id={id=7 type=73} portrait_path="duplicate.dds"}"#,
        )
        .unwrap();
        assert_eq!(
            parse(&doc.items).unwrap_err().to_string(),
            "duplicate serialized leader portrait identity"
        );
    }
}
