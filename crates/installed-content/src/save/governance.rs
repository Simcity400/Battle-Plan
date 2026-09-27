//! Literal governance state and the omissions established by the native writers.

use std::{collections::BTreeMap, io};

use super::{
    Decimal, Item, SavedDate, Value, block, boolean, bytes, decimal, field, integer, invalid,
    required_block, required_bytes, scalar,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub countries: BTreeMap<Vec<u8>, Country>,
    pub tech_sharing: Vec<TechSharing>,
    pub power_balances: Option<Vec<PowerBalance>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Country {
    pub capital: Option<i32>,
    pub original_capital: Option<i32>,
    pub politics: Option<Politics>,
    pub focus_tree: Option<Vec<u8>>,
    pub focus: Option<Focus>,
    pub research: Option<Research>,
    pub decisions: Option<Decisions>,
    pub dynamic_modifiers: Vec<DynamicModifier>,
    pub preferred_tactic: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Politics {
    pub parties: BTreeMap<Vec<u8>, Decimal>,
    /// Each party's country leaders in native vector order, keyed like
    /// `parties`. `CPoliticalParty::vf2@141191FA0` writes `country_leader`
    /// only for a nonzero leader count (+0x7c), so omission is an empty list.
    pub leaders: BTreeMap<Vec<u8>, Vec<PartyLeader>>,
    pub ideas: Vec<Vec<u8>>,
    pub ruling_party: Option<Vec<u8>>,
    pub last_election: Option<SavedDate>,
    pub election_frequency: Option<i32>,
    pub elections_allowed: Option<bool>,
}

/// CCountryLeader role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartyLeader {
    /// The role's ideology token (leader+0x140, then +0x10).
    pub ideology: Vec<u8>,
    /// The parent CCharacter's reference words (`*(leader+8)+8`): id and type.
    /// A null parent is written as the zeroed invalid reference, `{0, 0}`.
    pub character: (u32, u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Focus {
    pub current: Option<Vec<u8>>,
    pub continuous: Option<Vec<u8>>,
    pub progress: Option<Decimal>,
    /// Writer 2DC4E0 emits false only for a zero native byte.
    pub paused: Option<bool>,
    pub completed: Vec<CompletedFocus>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletedFocus {
    pub token: Vec<u8>,
    /// Joint focuses retain their separately serialized originator.
    pub originator: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Research {
    /// ED02A0 deliberately filters the complete live technology census.
    pub technologies: BTreeMap<Vec<u8>, Technology>,
    /// Saved array order and repeated idle/technology keys remain intact.
    pub slots: Vec<ResearchSlot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Technology {
    pub level: Option<i32>,
    pub progress: Option<Decimal>,
    pub ahead_reduction: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResearchSlot {
    pub technology: Vec<u8>,
    pub points: Option<Decimal>,
    pub used_saved_points: Option<Decimal>,
    pub points_factor: Option<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decisions {
    pub taken: Vec<Vec<u8>>,
    /// Source positions: re-enable, remove, timed, targeted, targeted-timed.
    pub containers: [Vec<Decision>; 5],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub token: Vec<u8>,
    pub days: i32,
    pub lifecycle: Option<i32>,
    pub target: Option<Target>,
    pub ignore: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Country(Vec<u8>),
    /// Native signed numeric target spelling; its bits remain a full u32.
    Numeric(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DynamicModifier {
    pub token: Vec<u8>,
    pub scope_country: Option<Vec<u8>>,
    pub scope_state: Option<i32>,
    pub days: Option<i32>,
    pub enabled: bool,
    pub values: Vec<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechSharing {
    pub token: Vec<u8>,
    pub countries: Vec<Vec<u8>>,
    pub bonuses: Vec<Decimal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerBalance {
    pub template: Vec<u8>,
    pub value: Decimal,
    pub left: Vec<u8>,
    pub right: Vec<u8>,
    pub trending: Vec<u8>,
    pub countries: Vec<Vec<u8>>,
    pub overrides: Vec<PowerBalanceOverride>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PowerBalanceOverride {
    pub side: Vec<u8>,
    pub gfx: Vec<u8>,
}

fn optional_integer<T: std::str::FromStr>(items: &[Item], key: &[u8]) -> io::Result<Option<T>> {
    field(items, key)?.map(|v| integer(scalar(v)?)).transpose()
}

fn required_literal(items: &[Item], key: &[u8]) -> io::Result<Vec<u8>> {
    bytes(items, key)?.ok_or_else(|| {
        invalid(format!(
            "missing governance {}",
            String::from_utf8_lossy(key)
        ))
    })
}

fn optional_block<'a>(items: &'a [Item], key: &[u8]) -> io::Result<Option<&'a [Item]>> {
    field(items, key)?.map(block).transpose()
}

fn required_bool(items: &[Item], key: &[u8]) -> io::Result<bool> {
    boolean(field(items, key)?.ok_or_else(|| invalid("missing governance boolean"))?)
}

fn list(items: &[Item], key: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    optional_block(items, key)?
        .unwrap_or_default()
        .iter()
        .map(|item| {
            if item.key.is_some() {
                return Err(invalid("governance list contains a keyed element"));
            }
            scalar(&item.value).map(<[u8]>::to_vec)
        })
        .collect()
}

fn fixed_list(items: &[Item], key: &[u8]) -> io::Result<Vec<Decimal>> {
    list(items, key)?
        .iter()
        .map(|v| Decimal::parse(v))
        .collect()
}

fn parse_politics(items: &[Item]) -> io::Result<Politics> {
    let mut parties = BTreeMap::new();
    let mut leaders = BTreeMap::new();
    // BA12D0 omits parties/ideas exactly for empty native arrays.
    for party in optional_block(items, b"parties")?.unwrap_or_default() {
        let token = party
            .key
            .clone()
            .ok_or_else(|| invalid("party lacks token"))?;
        let fields = block(&party.value)?;
        let popularity = Decimal::parse(&required_bytes(fields, b"popularity")?)?;
        leaders.insert(token.clone(), party_leaders(fields)?);
        if parties.insert(token, popularity).is_some() {
            return Err(invalid("duplicate political party token"));
        }
    }
    Ok(Politics {
        parties,
        leaders,
        ideas: list(items, b"ideas")?,
        ruling_party: bytes(items, b"ruling_party")?,
        last_election: bytes(items, b"last_election")?
            .map(|value| SavedDate::parse(&value))
            .transpose()?,
        election_frequency: optional_integer(items, b"election_frequency")?,
        elections_allowed: field(items, b"elections_allowed")?
            .map(boolean)
            .transpose()?,
    })
}

/// A party's `country_leader` block: one anonymous `{ ideology character }`
/// entry per element of the leader vector, in order. The writer emits the key
/// once, and only for a nonempty vector.
fn party_leaders(items: &[Item]) -> io::Result<Vec<PartyLeader>> {
    let Some(entries) = optional_block(items, b"country_leader")? else {
        return Ok(Vec::new());
    };
    if entries.is_empty() {
        return Err(invalid(
            "141191FA0 writes country_leader only for a nonzero leader count",
        ));
    }
    entries
        .iter()
        .map(|entry| {
            if entry.key.is_some() {
                return Err(invalid("party country leader entry must be anonymous"));
            }
            let fields = block(&entry.value)?;
            let handle = super::null_permitted_handle(
                field(fields, b"character")?
                    .ok_or_else(|| invalid("party country leader lacks character"))?,
            )?;
            Ok(PartyLeader {
                ideology: required_bytes(fields, b"ideology")?,
                character: ((handle >> 32) as u32, handle as u32),
            })
        })
        .collect()
}

fn parse_focus(items: &[Item]) -> io::Result<Focus> {
    let mut completed = Vec::new();
    for entry in items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"completed"))
    {
        let (token, originator) = match &entry.value {
            Value::Scalar(token) => (token.clone(), None),
            Value::Block(pair) if pair.len() == 2 && pair.iter().all(|v| v.key.is_none()) => (
                scalar(&pair[1].value)?.to_vec(),
                Some(scalar(&pair[0].value)?.to_vec()),
            ),
            _ => {
                return Err(invalid(
                    "completed focus must be a token or originator/token pair",
                ));
            }
        };
        completed.push(CompletedFocus { token, originator });
    }
    Ok(Focus {
        current: bytes(items, b"current")?,
        continuous: bytes(items, b"current_continuous")?,
        progress: decimal(items, b"progress")?,
        paused: field(items, b"paused")?.map(boolean).transpose()?,
        completed,
    })
}

fn parse_research(items: &[Item]) -> io::Result<Research> {
    let mut technologies = BTreeMap::new();
    for entry in optional_block(items, b"technologies")?.unwrap_or_default() {
        let token = entry
            .key
            .clone()
            .ok_or_else(|| invalid("technology lacks token"))?;
        let fields = block(&entry.value)?;
        let technology = Technology {
            level: optional_integer(fields, b"level")?,
            progress: decimal(fields, b"research_points")?,
            ahead_reduction: decimal(fields, b"ahead_reduction")?,
        };
        if technologies.insert(token, technology).is_some() {
            return Err(invalid("duplicate saved technology"));
        }
    }
    let mut slots = Vec::new();
    for entry in optional_block(items, b"slots")?.unwrap_or_default() {
        let technology = entry
            .key
            .clone()
            .ok_or_else(|| invalid("research slot lacks token"))?;
        let fields = block(&entry.value)?;
        slots.push(ResearchSlot {
            technology,
            points: decimal(fields, b"points")?,
            used_saved_points: decimal(fields, b"used_saved_points")?,
            points_factor: decimal(fields, b"points_factor")?,
        });
    }
    Ok(Research {
        technologies,
        slots,
    })
}

fn parse_decisions(items: &[Item]) -> io::Result<Decisions> {
    let mut containers: [Vec<Decision>; 5] = Default::default();
    // 72BE70 writes the five source vectors in this order. The two targeted
    // save labels are distinct from the names in the superseded old reader.
    for (kind, key) in [
        b"decision_to_re_enable".as_slice(),
        b"decision_to_remove",
        b"active_timed_decision",
        b"active_targeted_decision",
        b"active_targeted_timed_decision",
    ]
    .into_iter()
    .enumerate()
    {
        for entry in items.iter().filter(|item| item.key.as_deref() == Some(key)) {
            let fields = block(&entry.value)?;
            let targeted = kind >= 3;
            let lifecycle = if kind < 2 {
                None
            } else {
                let token = required_bytes(fields, b"state")?;
                let names = if targeted {
                    [
                        b"available".as_slice(),
                        b"completed",
                        b"re_enable_cooldown",
                        b"failed",
                        b"aborted",
                    ]
                } else {
                    [
                        b"active".as_slice(),
                        b"completed",
                        b"failed",
                        b"aborted",
                        b"re_enable_cooldown",
                    ]
                };
                Some(
                    names
                        .iter()
                        .position(|name| *name == token)
                        .ok_or_else(|| invalid("unserialized decision lifecycle token"))?
                        as i32,
                )
            };
            let target = if targeted {
                let token = required_bytes(fields, b"target")?;
                Some(
                    if token
                        .first()
                        .is_some_and(|b| b.is_ascii_digit() || *b == b'-' || *b == b'+')
                    {
                        Target::Numeric(integer::<i32>(&token)? as u32)
                    } else {
                        Target::Country(token)
                    },
                )
            } else {
                None
            };
            containers[kind].push(Decision {
                token: required_literal(fields, b"decision")?,
                days: integer(&required_bytes(fields, b"days")?)?,
                lifecycle,
                target,
                ignore: if targeted {
                    Some(required_bool(fields, b"ignore")?)
                } else {
                    None
                },
            });
        }
    }
    Ok(Decisions {
        taken: list(items, b"decisions_taken")?,
        containers,
    })
}

fn parse_dynamic_modifiers(items: &[Item]) -> io::Result<Vec<DynamicModifier>> {
    let Some(items) = optional_block(items, b"dynamic_modifier")? else {
        return Ok(Vec::new());
    };
    items
        .iter()
        .filter(|item| item.key.as_deref() == Some(b"modifier"))
        .map(|entry| {
            let fields = block(&entry.value)?;
            Ok(DynamicModifier {
                token: required_literal(fields, b"modifier")?,
                scope_country: bytes(fields, b"tag")?,
                scope_state: optional_integer(fields, b"state")?,
                days: optional_integer(fields, b"days")?,
                enabled: required_bool(fields, b"enabled")?,
                values: fixed_list(fields, b"value")?,
            })
        })
        .collect()
}

fn parse_power_balance(items: &[Item]) -> io::Result<PowerBalance> {
    let overrides = optional_block(items, b"sides")?
        .unwrap_or_default()
        .iter()
        .map(|entry| {
            if entry.key.is_some() {
                return Err(invalid("power-balance side must be anonymous"));
            }
            let fields = block(&entry.value)?;
            Ok(PowerBalanceOverride {
                side: bytes(fields, b"id")?
                    .ok_or_else(|| invalid("power-balance side lacks id"))?,
                gfx: bytes(fields, b"gfx")?
                    .ok_or_else(|| invalid("power-balance side lacks gfx"))?,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    Ok(PowerBalance {
        template: required_literal(items, b"template")?,
        value: Decimal::parse(&required_bytes(items, b"value")?)?,
        left: bytes(items, b"left_side")?.ok_or_else(|| invalid("missing left side"))?,
        right: bytes(items, b"right_side")?.ok_or_else(|| invalid("missing right side"))?,
        trending: bytes(items, b"trending_side")?
            .ok_or_else(|| invalid("missing trending side"))?,
        countries: list(items, b"countries")?,
        overrides,
    })
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    let mut countries = BTreeMap::new();
    for entry in required_block(items, b"countries")? {
        let tag = entry
            .key
            .clone()
            .ok_or_else(|| invalid("governance country lacks tag"))?;
        let fields = block(&entry.value)?;
        let country = Country {
            capital: optional_integer(fields, b"capital")?,
            original_capital: optional_integer(fields, b"original_capital")?,
            politics: optional_block(fields, b"politics")?
                .map(parse_politics)
                .transpose()?,
            focus_tree: bytes(fields, b"focus_tree")?,
            focus: optional_block(fields, b"focus")?
                .map(parse_focus)
                .transpose()?,
            research: optional_block(fields, b"technology")?
                .map(parse_research)
                .transpose()?,
            decisions: optional_block(fields, b"decision_status")?
                .map(parse_decisions)
                .transpose()?,
            dynamic_modifiers: parse_dynamic_modifiers(fields)?,
            preferred_tactic: optional_integer(fields, b"preferred_tactic")?,
        };
        if countries.insert(tag, country).is_some() {
            return Err(invalid("duplicate governance country"));
        }
    }
    let tech_sharing = items
        .iter()
        .filter(|entry| entry.key.as_deref() == Some(b"tech_sharing_group"))
        .map(|entry| {
            let fields = block(&entry.value)?;
            let group = TechSharing {
                token: required_literal(fields, b"id")?,
                countries: list(fields, b"countries")?,
                bonuses: fixed_list(fields, b"bonuses")?,
            };
            if group.countries.len() != group.bonuses.len() {
                return Err(invalid("technology-sharing member/bonus lengths differ"));
            }
            Ok(group)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let power_balances = optional_block(items, b"power_balance")?
        .map(|manager| {
            optional_block(manager, b"power_balances")?
                .unwrap_or_default()
                .iter()
                .map(|entry| {
                    if entry.key.is_some() {
                        return Err(invalid("power balance must be anonymous"));
                    }
                    parse_power_balance(block(&entry.value)?)
                })
                .collect::<io::Result<Vec<_>>>()
        })
        .transpose()?;
    Ok(Pack {
        countries,
        tech_sharing,
        power_balances,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::GameDate;

    fn parse_text(text: &[u8]) -> io::Result<Pack> {
        parse(&crate::parser::parse(text)?.items)
    }

    #[test]
    fn election_negative_year_marker_preserves_raw_date_without_extending_calendar() {
        let pack = parse_text(
            br#"countries={AAA={politics={last_election="-1.1.1.1"}}
                BBB={politics={last_election="1942.4.3.16"}}}"#,
        )
        .unwrap();
        let date = |tag: &[u8]| {
            pack.countries[tag]
                .politics
                .as_ref()
                .unwrap()
                .last_election
                .unwrap()
        };
        assert_eq!(date(b"AAA"), SavedDate::Invalid);
        let raw = date(b"AAA").engine_hour().unwrap();
        assert_eq!(raw, 43_791_240);
        let delta = raw - 43_800_000;
        assert_eq!((delta / 8760, (delta / 24) % 365, delta % 24), (-1, 0, 0));
        assert_eq!(
            date(b"BBB").engine_hour().unwrap(),
            GameDate::parse(b"1942.4.3.16")
                .unwrap()
                .engine_hour()
                .unwrap()
        );
        assert!(GameDate::parse(b"-1.1.1.1").is_err());
        for value in [b"-1.1.1.2".as_slice(), b"-2.1.1.1", b"-1.2.1.1"] {
            assert!(SavedDate::parse(value).is_err());
        }
    }

    #[test]
    fn exact_values_optional_presence_and_order_survive() {
        let pack = parse_text(
            br#"countries={AAA={
            focus={completed=x completed={BBB y} completed=x progress=-21474.83649}
            technology={technologies={long_tech={level=-3 research_points=0.00001}}
                slots={empty={} empty={points=1.00001}}}
            dynamic_modifier={modifier={modifier=x enabled=no value={1.00001 -21474.83649}}
                              modifier={modifier=x tag=BBB state=4 enabled=yes days=0}}
            preferred_tactic=-1
        }}"#,
        )
        .unwrap();
        let country = &pack.countries[b"AAA".as_slice()];
        let focus = country.focus.as_ref().unwrap();
        assert_eq!(focus.progress.as_ref().unwrap().raw, -2_147_483_649);
        assert_eq!(focus.paused, None);
        assert_eq!(
            focus
                .completed
                .iter()
                .map(|v| v.token.as_slice())
                .collect::<Vec<_>>(),
            [b"x", b"y", b"x"]
        );
        assert_eq!(
            focus.completed[1].originator.as_deref(),
            Some(b"BBB".as_slice())
        );
        assert_eq!(country.research.as_ref().unwrap().slots.len(), 2);
        assert_eq!(country.dynamic_modifiers[0].days, None);
        assert_eq!(country.dynamic_modifiers[1].days, Some(0));
        assert_eq!(country.dynamic_modifiers[0].values[1].raw, -2_147_483_649);
        assert_eq!(country.preferred_tactic, Some(-1));
    }

    #[test]
    fn party_leaders_keep_native_order_and_the_null_character_reference() {
        // The writer's own layout, including its unkeyed entry blocks.
        let pack = parse_text(
            b"countries={AAA={politics={parties={
            fascism={default=yes popularity=40 country_leader={
{
                ideology=\"nazism\"
                character={ id=1800 type=73 }
            }
            {
                ideology=\"nazism\"
                character={ id=0 type=0 }
            }
 }}
            democratic={popularity=60}
        } ruling_party=fascism}}}",
        )
        .unwrap();
        let politics = pack.countries[b"AAA".as_slice()].politics.as_ref().unwrap();
        let fascism = &politics.leaders[b"fascism".as_slice()];
        assert_eq!(fascism.len(), 2);
        assert_eq!(fascism[0].ideology, b"nazism");
        assert_eq!(fascism[0].character, (1800, 73));
        assert_eq!(fascism[1].character, (0, 0));
        assert!(politics.leaders[b"democratic".as_slice()].is_empty());
        for text in [
            b"countries={AAA={politics={parties={x={popularity=1 country_leader={}}}}}}".as_slice(),
            b"countries={AAA={politics={parties={x={popularity=1 country_leader={a={ideology=y character={id=1 type=73}}}}}}}}",
            b"countries={AAA={politics={parties={x={popularity=1 country_leader={{ideology=y}}}}}}}",
            b"countries={AAA={politics={parties={x={popularity=1 country_leader={{character={id=1 type=73}}}}}}}}",
            b"countries={AAA={politics={parties={x={popularity=1 country_leader={{ideology=y character={id=1 type=73}}} country_leader={{ideology=y character={id=1 type=73}}}}}}}}",
        ] {
            assert!(parse_text(text).is_err(), "{}", String::from_utf8_lossy(text));
        }
    }

    #[test]
    fn decision_sources_have_distinct_lifecycle_mappings_and_full_targets() {
        let pack = parse_text(
            br#"countries={AAA={decision_status={
            active_timed_decision={decision=x days=-4 state=failed}
            active_targeted_decision={decision=y days=0 state=failed target=-2147483648 ignore=yes}
            active_targeted_timed_decision={decision=z days=5 state=available target=BBB ignore=no}
        }}}"#,
        )
        .unwrap();
        let rows = &pack.countries[b"AAA".as_slice()]
            .decisions
            .as_ref()
            .unwrap()
            .containers;
        assert_eq!(rows[2][0].lifecycle, Some(2));
        assert_eq!(rows[3][0].lifecycle, Some(3));
        assert_eq!(rows[3][0].target, Some(Target::Numeric(0x8000_0000)));
        assert_eq!(rows[4][0].target, Some(Target::Country(b"BBB".to_vec())));
    }

    #[test]
    fn malformed_or_ambiguous_state_never_becomes_empty() {
        for text in [
            b"countries={A={focus={completed={A x y}}}}".as_slice(),
            b"countries={A={technology={technologies={x={} x={}}}}}",
            b"countries={A={dynamic_modifier={modifier={modifier=x enabled=yes value={0.000001}}}}}",
            b"countries={A={decision_status={active_timed_decision={decision=x days=1 state=unknown}}}}",
            b"countries={} tech_sharing_group={id=x countries={A B} bonuses={0}}",
        ] { assert!(parse_text(text).is_err(), "{}", String::from_utf8_lossy(text)); }
    }
}
