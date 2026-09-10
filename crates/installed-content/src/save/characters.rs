//! Character-manager ownership, role state and persistent command abilities.

use super::{
    Decimal, GameDate, block, bytes, decimal, field, integer, invalid, object_id, required_block,
    required_bytes, scalar, typed_handle,
};
use crate::parser::Item;
use std::{collections::BTreeMap, io};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    /// Missing manager is not an empty native roster.
    pub manager: Option<Manager>,
    /// Every saved country is present, including countries with no records.
    pub abilities: BTreeMap<Vec<u8>, Vec<Ability>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Manager {
    pub characters: BTreeMap<u64, Character>,
    pub unit_leaders: BTreeMap<u64, UnitLeader>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Character {
    pub id: u64,
    pub roster: u8,
    pub ordinal: u32,
    /// F913A0 resolves CCharacter +18 through the lexer; this is not +48.
    pub token: Vec<u8>,
    /// F913A0 writes native +48 as name, omitting an empty string. Native
    /// +28 is a different string and is not serialized by this writer.
    pub name: Option<Vec<u8>>,
    /// Omission proves the raw country reference is signed-nonpositive.
    pub country: Option<Vec<u8>>,
    pub country_leaders: Vec<CountryLeader>,
    pub advisors: Vec<Advisor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryLeader {
    /// FB73E0 writes the role's ideology definition token, not its map node.
    pub ideology: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Advisor {
    /// 12902F0 omits each of these strings exactly when its length is zero.
    /// Presence remains explicit because a nonempty string beginning with NUL
    /// serializes an empty literal, which differs from an omitted empty string.
    pub slot: Option<Vec<u8>>,
    pub idea: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitLeader {
    pub id: u64,
    pub character: u64,
    /// F913A0 selects field_marshal=0, corps_commander=1, navy_leader=2.
    pub category: i32,
    /// C0D920 serializes +20 as name only when nonempty in text mode.
    pub name: Option<Vec<u8>>,
    pub skill: i32,
    /// Native i64 fixed5, omitted precisely at zero by C0D920.
    pub experience: Decimal,
    /// Army attack/defense/planning/logistics; navy attack/defense/maneuvering/
    /// coordination. C18CA0/C18E90 omit each signed integer precisely at zero.
    pub components: [i32; 4],
    pub traits: Vec<Vec<u8>>,
    /// Independent native order. Missing progress entries never invent zero.
    pub trait_experience: Vec<TraitExperience>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraitExperience {
    pub token: Vec<u8>,
    pub experience: Decimal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ability {
    pub token: Vec<u8>,
    pub start_date: GameDate,
    pub country: Option<Vec<u8>>,
    pub leader: Option<u64>,
    /// 70C580 emits the signed value exactly when caster is nonnull. Native
    /// storage remains initialized and captured even when this field is omitted.
    pub num_units: Option<i32>,
    /// Native target+58, not a typed formation identity.
    pub strategic_region: Option<i32>,
    /// Omission proves raw allocated cost <= 0, not necessarily zero.
    pub cost: Option<Decimal>,
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
    })
}

fn manager(items: &[Item]) -> io::Result<Manager> {
    let mut manager = Manager::default();
    for (roster, key) in [(0, b"historical".as_slice()), (1, b"dynamic".as_slice())] {
        // 6AE170 -> 6AE080 omits only empty vectors; both vectors preserve order.
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
                    country_leaders.push(CountryLeader {
                        ideology: required_bytes(block(&role.value)?, b"ideology")?,
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
                    });
                }
            }
            let character = Character {
                id,
                roster,
                ordinal: u32::try_from(ordinal)
                    .map_err(|_| invalid("character ordinal exceeds u32"))?,
                token: bytes(fields, b"token")?.ok_or_else(|| invalid("character lacks token"))?,
                name: bytes(fields, b"name")?,
                country: bytes(fields, b"country")?,
                country_leaders,
                advisors,
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
    let manager = field(items, b"character_manager")?
        .map(|value| manager(block(value)?))
        .transpose()?;
    let mut abilities = BTreeMap::new();
    for country in required_block(items, b"countries")? {
        let tag = country
            .key
            .clone()
            .ok_or_else(|| invalid("ability country lacks a tag"))?;
        let mut rows = Vec::new();
        // 70C580 emits one direct block per value-array entry, in native order.
        for item in block(&country.value)?
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"active_ability"))
        {
            rows.push(ability(block(&item.value)?)?);
        }
        if abilities.insert(tag, rows).is_some() {
            return Err(invalid("duplicate country in ability roster"));
        }
    }
    Ok(Pack { manager, abilities })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(input: &[u8]) -> io::Result<Pack> {
        parse(&crate::parser::parse(input)?.items)
    }

    #[test]
    fn character_rosters_keep_identity_order_names_and_optional_country() {
        let pack = fixture(
            br#"countries={} character_manager={historical={
            character={id={id=4294967295 type=73} token="" name="raw name"}
            character={id={id=1 type=73} token="same" country=GER}
        } dynamic={character={id={id=2 type=73} token="same"}}}"#,
        )
        .unwrap();
        let manager = pack.manager.unwrap();
        let first = &manager.characters[&((u64::from(u32::MAX) << 32) | 73)];
        assert_eq!((first.roster, first.ordinal), (0, 0));
        assert!(first.token.is_empty());
        assert!(first.country.is_none());
        assert_eq!(first.name.as_deref(), Some(b"raw name".as_slice()));
        assert_eq!(
            (
                manager.characters[&((2_u64 << 32) | 73)].roster,
                manager.characters.len()
            ),
            (1, 3)
        );
        assert!(fixture(b"countries={} character_manager={historical={character={id={id=1 type=73} token=a}} dynamic={character={id={id=1 type=73} token=b}}}").is_err());
        assert!(fixture(b"countries={}").unwrap().manager.is_none());
    }

    #[test]
    fn leader_precision_and_independent_trait_order_are_lossless() {
        let pack = fixture(br#"countries={} character_manager={historical={character={
            id={id=8 type=73} token=x
            navy_leader={id={id=4294967295 type=4713} skill=-2147483648 experience=-90071992547.40993
                attack_skill=2147483647 coordination_skill=-1
                traits={b a b} in_progress={a=0.00001 b=-90071992547.40993 a=0}}
            advisors={advisor={slot=x idea_token=y dynamic_template={slot=ignored}} advisor={}}
        }}}"#).unwrap();
        let manager = pack.manager.unwrap();
        let leader = manager.unit_leaders.values().next().unwrap();
        assert_eq!(leader.experience.raw, -9_007_199_254_740_993);
        assert_eq!(leader.components, [i32::MAX, 0, 0, -1]);
        assert_eq!(leader.traits, [b"b".to_vec(), b"a".to_vec(), b"b".to_vec()]);
        assert_eq!(
            leader.trait_experience[1].experience.raw,
            -9_007_199_254_740_993
        );
        assert_eq!(leader.trait_experience.len(), 3);
        assert!(
            manager.characters.values().next().unwrap().advisors[1]
                .slot
                .is_none()
        );
    }

    #[test]
    fn ability_optional_operands_follow_serializer_predicates() {
        let pack = fixture(
            br#"countries={GER={active_ability={id=a start_date="1936.1.1.1"
            leader={id=4294967295 type=4713} num_units=-1 strategic_region=-2147483648 cost=0.00001}
            active_ability={id=b start_date="1936.1.1.1"}}}"#,
        )
        .unwrap();
        let rows = &pack.abilities[b"GER".as_slice()];
        assert_eq!(rows[0].leader, Some((u64::from(u32::MAX) << 32) | 4713));
        assert_eq!(rows[0].cost.as_ref().unwrap().raw, 1);
        assert_eq!(rows[0].strategic_region, Some(i32::MIN));
        assert!(rows[1].cost.is_none());
        for invalid in [
            br#"countries={GER={active_ability={id=a start_date="1936.1.1.1" num_units=1}}}"#
                .as_slice(),
            br#"countries={GER={active_ability={id=a start_date="1936.1.1.1" cost=0}}}"#,
        ] {
            assert!(fixture(invalid).is_err());
        }
    }
}
