
use super::{
    Decimal, GameDate, block, boolean, bytes, decimal, field, integer, invalid, object_id,
    required_block, required_bytes, scalar, typed_handle,
};
use content_reader_parser::parser::Item;
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub portraits: BTreeMap<u64, [Vec<u8>; 3]>,
    pub manager: Option<Manager>,
    pub abilities: BTreeMap<Vec<u8>, Vec<Ability>>,
    pub appointed_advisors: BTreeMap<Vec<u8>, Vec<AppointedAdvisor>>,
    pub character_status: BTreeMap<Vec<u8>, Vec<CharacterStatus>>,
    pub retired_character_status: BTreeMap<Vec<u8>, Vec<CharacterStatus>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterStatus {
    pub character: u64,
    pub advisor: Option<bool>,
    pub unit_leader: Option<bool>,
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppointedAdvisor {
    pub slot: Vec<u8>,
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

impl Manager {
    pub fn loaded(&self) -> Self {
        let mut order: Vec<&Character> = self.characters.values().collect();
        order.sort_unstable_by_key(|character| (character.roster, character.ordinal));
        let mut next = [0_u32; 2];
        let mut characters = BTreeMap::new();
        for character in order.into_iter().filter(|c| c.survives_load()) {
            let ordinal = &mut next[usize::from(character.roster)];
            characters.insert(
                character.id,
                Character {
                    ordinal: *ordinal,
                    ..character.clone()
                },
            );
            *ordinal += 1;
        }
        let unit_leaders = self
            .unit_leaders
            .iter()
            .filter(|(_, leader)| characters.contains_key(&leader.character))
            .map(|(&id, leader)| (id, leader.clone()))
            .collect();
        Self {
            characters,
            unit_leaders,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Character {
    pub gender: Option<Vec<u8>>,
    pub portraits: Option<Vec<Portrait>>,
    pub id: u64,
    pub roster: u8,
    pub ordinal: u32,
    pub token: Vec<u8>,
    pub template: Option<Vec<u8>>,
    pub flags: Option<Vec<super::intel::Flag>>,
    pub variables: Option<super::intel::Variables>,
    pub name: Option<Vec<u8>>,
    pub country: Option<Vec<u8>>,
    pub country_leaders: Vec<CountryLeader>,
    pub advisors: Vec<Advisor>,
    pub scientist: Option<Scientist>,
    pub operative: Option<u64>,
}

impl Character {
    pub fn survives_load(&self) -> bool {
        self.name.as_deref().is_some_and(|name| !name.is_empty())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scientist {
    pub active: bool,
    pub injury_days: Option<i32>,
    pub traits: Option<Vec<Vec<u8>>>,
    pub skills: Vec<ScientistSkill>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScientistSkill {
    pub name: Vec<u8>,
    pub level: Option<i32>,
    pub experience: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryLeader {
    pub ideology: Vec<u8>,
    pub id: Option<i32>,
    pub traits: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advisor {
    pub slot: Option<Vec<u8>>,
    pub idea: Option<Vec<u8>>,
    pub traits: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitLeader {
    pub id: u64,
    pub character: u64,
    pub category: i32,
    pub name: Option<Vec<u8>>,
    pub skill: i32,
    pub experience: Decimal,
    pub components: [i32; 4],
    pub traits: Vec<Vec<u8>>,
    pub trait_experience: Vec<TraitExperience>,
    pub trait_factors: Vec<TraitExperience>,
    pub timed_traits: Vec<TimedTrait>,
    pub captured: bool,
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
    pub num_units: Option<i32>,
    pub strategic_region: Option<i32>,
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
fn leader_portraits(items: &[Item], out: &mut BTreeMap<u64, [Vec<u8>; 3]>) -> io::Result<()> {
    for item in items {
        if let content_reader_parser::parser::Value::Block(children) = &item.value {
            if matches!(
                item.key.as_deref(),
                Some(b"field_marshal" | b"corps_commander" | b"navy_leader" | b"operative")
            ) && field(children, b"id")?
                .is_some_and(|value| matches!(value, content_reader_parser::parser::Value::Block(_)))
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
        injury_days: field(items, b"is_scientist_injured")?
            .map(|value| integer(scalar(value)?))
            .transpose()?,
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
        trait_factors: field(items, b"trait_xp_factor")?
            .map(|value| {
                block(value)?
                    .iter()
                    .map(|entry| {
                        Ok(TraitExperience {
                            token: entry
                                .key
                                .clone()
                                .ok_or_else(|| invalid("trait XP factor lacks its token"))?,
                            experience: Decimal::parse(scalar(&entry.value)?)?,
                        })
                    })
                    .collect::<io::Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or_default(),
        timed_traits,
        captured: bytes(items, b"captured")?.as_deref() == Some(b"yes"),
        government_in_exile_tag: bytes(items, b"government_in_exile_tag")?,
    })
}

fn manager(items: &[Item]) -> io::Result<Manager> {
    let mut manager = Manager::default();
    for (roster, key) in [(0, b"historical".as_slice()), (1, b"dynamic".as_slice())] {
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
        let doc = content_reader_parser::parser::parse(
            br#"countries={X={}}
                wrapper={countries={operative={id={id=7 type=73} portrait_path="nested.dds"}}}
                navy_leader={id={id=8 type=73} portrait_path="root.dds"}"#,
        )
        .unwrap();
        let pack = parse(&doc.items).unwrap();
        assert_eq!(pack.portraits.len(), 2);
        assert_eq!(pack.portraits[&((7 << 32) | 73)][0], b"nested.dds");
        assert_eq!(pack.portraits[&((8 << 32) | 73)][0], b"root.dds");

        let doc = content_reader_parser::parser::parse(
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
