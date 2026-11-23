
use std::{collections::BTreeMap, io};

use super::{
    Decimal, Item, SavedDate, Value, block, boolean, bytes, decimal, field, integer, invalid,
    required_bytes, scalar,
};

mod doctrine;
pub use doctrine::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub countries: BTreeMap<Vec<u8>, Country>,
    pub tech_sharing: Vec<TechSharing>,
    pub power_balances: Option<Vec<PowerBalance>>,
    pub doctrine: Option<Vec<DoctrineStatus>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DoctrineStatus {
    pub country: Option<Vec<u8>>,
    pub enable_tactic: Vec<i32>,
    pub folders: Vec<DoctrineFolder>,
    pub cost_reductions: Vec<Option<DoctrineCostReduction>>,
    pub daily_mastery: Vec<Option<DoctrineMasteryGrant>>,
    pub equipment_bonuses: Vec<DoctrineEquipmentBonus>,
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
    pub leaders: BTreeMap<Vec<u8>, Vec<PartyLeader>>,
    pub names: BTreeMap<Vec<u8>, PartyName>,
    pub ideas: Vec<Vec<u8>>,
    pub timed_ideas: Vec<TimedIdea>,
    pub ruling_party: Option<Vec<u8>>,
    pub last_election: Option<SavedDate>,
    pub election_frequency: Option<i32>,
    pub elections_allowed: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimedIdea {
    pub token: Vec<u8>,
    pub days: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartyName {
    pub default: Option<bool>,
    pub keys: Option<(Vec<u8>, Vec<u8>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartyLeader {
    pub ideology: Vec<u8>,
    pub character: (u32, u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Focus {
    pub current: Option<Vec<u8>>,
    pub continuous: Option<Vec<u8>>,
    pub progress: Option<Decimal>,
    pub paused: Option<bool>,
    pub completed: Vec<CompletedFocus>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletedFocus {
    pub token: Vec<u8>,
    pub originator: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Research {
    pub technologies: Vec<(Vec<u8>, Technology)>,
    pub slots: Vec<ResearchSlot>,
    pub next_bonus_id: Option<i32>,
    pub grants: Vec<ResearchGrant>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResearchGrant {
    pub cost: bool,
    pub id: i32,
    pub name: Vec<u8>,
    pub uses: i32,
    pub claims: i32,
    pub speed_raw: i64,
    pub ahead_raw: i64,
    pub cost_raw: i64,
    pub technologies: Vec<Vec<u8>>,
    pub categories: Vec<Vec<u8>>,
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
    pub containers: [Vec<Decision>; 5],
    pub random_items: Vec<DecisionRandomItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionRandomItem {
    pub token: Vec<u8>,
    pub count: i32,
    pub target: i32,
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
    pub modifiers: Vec<Vec<u8>>,
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
    let mut names = BTreeMap::new();
    for party in optional_block(items, b"parties")?.unwrap_or_default() {
        let token = party
            .key
            .clone()
            .ok_or_else(|| invalid("party lacks token"))?;
        let fields = block(&party.value)?;
        let popularity = Decimal::parse(&required_bytes(fields, b"popularity")?)?;
        leaders.insert(token.clone(), party_leaders(fields)?);
        names.insert(token.clone(), party_name(fields)?);
        if parties.insert(token, popularity).is_some() {
            return Err(invalid("duplicate political party token"));
        }
    }
    Ok(Politics {
        parties,
        leaders,
        names,
        ideas: list(items, b"ideas")?,
        timed_ideas: items
            .iter()
            .filter(|item| item.keyword_is("timed_idea"))
            .map(|item| {
                let fields = block(&item.value)?;
                Ok(TimedIdea {
                    token: required_bytes(fields, b"idea")?,
                    days: integer(&required_bytes(fields, b"days")?)?,
                })
            })
            .collect::<io::Result<Vec<_>>>()?,
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

fn party_name(items: &[Item]) -> io::Result<PartyName> {
    let default = field(items, b"default")?.map(boolean).transpose()?;
    let name = bytes(items, b"name")?;
    let long_name = bytes(items, b"long_name")?;
    let keys = match (default, name, long_name) {
        (Some(false), Some(name), Some(long_name)) => Some((name, long_name)),
        (Some(false), _, _) => {
            return Err(invalid(
                "party writer emits name and long_name whenever default is false",
            ));
        }
        (_, None, None) => None,
        _ => {
            return Err(invalid(
                "party writer emits name and long_name only when default is false",
            ));
        }
    };
    Ok(PartyName { default, keys })
}

fn party_leaders(items: &[Item]) -> io::Result<Vec<PartyLeader>> {
    let Some(entries) = optional_block(items, b"country_leader")? else {
        return Ok(Vec::new());
    };
    if entries.is_empty() {
        return Err(invalid(
            "party writer emits country_leader only for a nonzero leader count",
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
    let mut technologies = Vec::new();
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
        technologies.push((token, technology));
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
    let mut grants = Vec::new();
    for cost in [false, true] {
        for item in items.iter().filter(|item| {
            item.key.as_deref()
                == Some(if cost {
                    b"limited_use_cost_reduction_bonus".as_slice()
                } else {
                    b"limited_use_bonus".as_slice()
                })
        }) {
            let fields = block(&item.value)?;
            let repeated = |key: &[u8]| {
                fields
                    .iter()
                    .filter(|field| field.key.as_deref() == Some(key))
                    .map(|field| scalar(&field.value).map(|value| value.to_vec()))
                    .collect::<io::Result<Vec<_>>>()
            };
            grants.push(ResearchGrant {
                cost,
                id: optional_integer(fields, b"id")?
                    .ok_or_else(|| invalid("research grant lacks ID"))?,
                name: required_literal(fields, b"name")?,
                uses: optional_integer(fields, b"uses")?.unwrap_or(0),
                claims: optional_integer(fields, b"claim")?.unwrap_or(0),
                speed_raw: decimal(fields, b"bonus")?.map_or(0, |value| value.raw),
                ahead_raw: decimal(fields, b"ahead_reduction")?.map_or(0, |value| value.raw),
                cost_raw: decimal(fields, b"cost_reduction")?.map_or(0, |value| value.raw),
                technologies: repeated(b"technology")?,
                categories: repeated(b"category")?,
            });
        }
    }
    Ok(Research {
        technologies,
        slots,
        next_bonus_id: optional_integer(items, b"next_bonus_id")?,
        grants,
    })
}

pub fn parse_decisions(items: &[Item]) -> io::Result<Decisions> {
    let mut containers: [Vec<Decision>; 5] = Default::default();
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
        random_items: items
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"random_item"))
            .map(|entry| {
                let fields = block(&entry.value)?;
                Ok(DecisionRandomItem {
                    token: required_literal(fields, b"decision")?,
                    count: integer(&required_bytes(fields, b"count")?)?,
                    target: integer(&required_bytes(fields, b"target")?)?,
                })
            })
            .collect::<io::Result<_>>()?,
    })
}

#[cfg(test)]
#[path = "governance/border_effects/tests.rs"]
mod border_effect_tests;

pub(crate) fn parse_dynamic_modifiers(items: &[Item]) -> io::Result<Vec<DynamicModifier>> {
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
        modifiers: items
            .iter()
            .filter(|item| item.key.as_deref() == Some(b"modifier"))
            .map(|item| scalar(&item.value).map(<[u8]>::to_vec))
            .collect::<io::Result<_>>()?,
    })
}

pub fn parse(items: &[Item]) -> io::Result<Pack> {
    parse_with_countries(items, super::CountriesView::from_root(items)?)
}

pub(super) fn parse_with_countries(
    items: &[Item],
    countries: super::CountriesView<'_>,
) -> io::Result<Pack> {
    let country_items = countries.required()?;
    let mut countries = BTreeMap::new();
    for entry in country_items {
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
    let doctrine = optional_block(items, b"doctrine")?
        .map(parse_doctrine)
        .transpose()?;
    Ok(Pack {
        countries,
        tech_sharing,
        power_balances,
        doctrine,
    })
}

fn parse_doctrine(items: &[Item]) -> io::Result<Vec<DoctrineStatus>> {
    optional_block(items, b"countries")?
        .unwrap_or_default()
        .iter()
        .map(|entry| {
            if entry.key.is_some() {
                return Err(invalid("doctrine country status must be anonymous"));
            }
            let fields = block(&entry.value)?;
            let enable_tactic = optional_block(fields, b"enable_tactic")?
                .unwrap_or_default()
                .iter()
                .map(|item| {
                    if item.key.is_some() {
                        return Err(invalid("enable_tactic contains a keyed element"));
                    }
                    integer(scalar(&item.value)?)
                })
                .collect::<io::Result<Vec<i32>>>()?;
            Ok(DoctrineStatus {
                country: bytes(fields, b"country")?,
                enable_tactic,
                folders: doctrine::folders(fields)?,
                cost_reductions: doctrine::cost_reductions(fields)?,
                daily_mastery: doctrine::mastery_grants(fields)?,
                equipment_bonuses: doctrine::equipment_bonuses(fields)?,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GameDate;

    fn parse_text(text: &[u8]) -> io::Result<Pack> {
        parse(&content_reader_parser::parser::parse(text)?.items)
    }

    #[test]
    fn gap_research_grants_research_grant_inventory_keeps_unused_grants_duplicates_restrictions_and_counter()
     {
        let research = parse_research(&content_reader_parser::parser::parse(br#"next_bonus_id=-2147483648
            limited_use_bonus={id=7 name=mod_grant uses=3 claim=2 bonus=0.25 ahead_reduction=-1.5 technology=mod_tech technology=mod_tech category=mod_category}
            limited_use_bonus={id=7 name=mod_repeated uses=-1}
            limited_use_cost_reduction_bonus={id=8 name=mod_cost uses=1 cost_reduction=0.4 category=mod_category}"#).unwrap().items).unwrap();
        assert_eq!(research.next_bonus_id, Some(i32::MIN));
        assert_eq!(research.grants.len(), 3);
        assert_eq!(
            research.grants[0].technologies,
            [b"mod_tech".to_vec(), b"mod_tech".to_vec()]
        );
        assert_eq!(research.grants[0].claims, 2);
        assert_eq!(research.grants[0].speed_raw, 25_000);
        assert_eq!(research.grants[0].ahead_raw, -150_000);
        assert_eq!(research.grants[1].id, 7);
        assert_eq!(research.grants[1].uses, -1);
        assert!(research.grants[2].cost);
        assert_eq!(research.grants[2].cost_raw, 40_000);
    }

    #[test]
    fn repeated_saved_technology_keys_retain_order_and_independent_state() {
        let pack = parse_text(b"countries={AAA={technology={technologies={repeat={level=1} other={research_points=0.25} repeat={level=2 research_points=0.5}}}}}").unwrap();
        let technologies = &pack.countries[b"AAA".as_slice()]
            .research
            .as_ref()
            .unwrap()
            .technologies;
        assert_eq!(
            technologies
                .iter()
                .map(|(token, _)| token.as_slice())
                .collect::<Vec<_>>(),
            [b"repeat".as_slice(), b"other", b"repeat"]
        );
        assert_eq!(technologies[0].1.level, Some(1));
        assert_eq!(technologies[2].1.level, Some(2));
        assert_eq!(technologies[2].1.progress.as_ref().unwrap().raw, 50_000);
    }

    #[test]
    fn timed_ideas_preserve_order_repeats_and_stored_signed_days() {
        let pack = parse_text(
            br#"countries={AAA={politics={
            timed_idea={idea="second" days=-2147483648}
            timed_idea={idea="first" days=0}
            timed_idea={idea="second" days=2147483647}
        }} BBB={politics={}}}"#,
        )
        .unwrap();
        let ideas = &pack.countries[b"AAA".as_slice()]
            .politics
            .as_ref()
            .unwrap()
            .timed_ideas;
        assert_eq!(
            ideas
                .iter()
                .map(|idea| (idea.token.as_slice(), idea.days))
                .collect::<Vec<_>>(),
            [
                (b"second".as_slice(), i32::MIN),
                (b"first".as_slice(), 0),
                (b"second".as_slice(), i32::MAX),
            ]
        );
        assert!(
            pack.countries[b"BBB".as_slice()]
                .politics
                .as_ref()
                .unwrap()
                .timed_ideas
                .is_empty()
        );
        for entry in ["idea=x", "days=1", "idea=x days=1 days=2"] {
            let text = format!("countries={{AAA={{politics={{timed_idea={{{entry}}}}}}}}}");
            assert!(parse_text(text.as_bytes()).is_err());
        }
    }

    #[test]
    fn balance_modifier_sources_keep_order_duplicates_and_omission() {
        let pack = parse_text(
            br#"countries={} power_balance={power_balances={
            {template=x value=0 left_side=l right_side=r trending_side=r
             modifier=second modifier=first modifier=second}
            {template=y value=0 left_side=l right_side=r trending_side=r}
        }}"#,
        )
        .unwrap();
        let rows = pack.power_balances.unwrap();
        assert_eq!(
            rows[0].modifiers,
            [b"second".to_vec(), b"first".to_vec(), b"second".to_vec()]
        );
        assert!(rows[1].modifiers.is_empty());
        assert!(parse_text(b"countries={} power_balance={power_balances={{template=x value=0 left_side=l right_side=r trending_side=r modifier={x}}}}").is_err());
    }

    #[test]
    fn doctrine_rows_keep_array_order_omitted_tags_and_saved_tactic_ids() {
        let pack = parse_text(
            b"countries={} doctrine={ countries={
                { folder={ { folder=\"land\" } } }
                { country=\"GER\" enable_tactic={ 43 7 43 } }
                { country=\"ITA\" }
            } }",
        )
        .unwrap();
        assert_eq!(
            pack.doctrine.unwrap(),
            [
                DoctrineStatus {
                    folders: vec![DoctrineFolder {
                        folder: b"land".to_vec(),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                DoctrineStatus {
                    country: Some(b"GER".to_vec()),
                    enable_tactic: vec![43, 7, 43],
                    ..Default::default()
                },
                DoctrineStatus {
                    country: Some(b"ITA".to_vec()),
                    enable_tactic: Vec::new(),
                    ..Default::default()
                },
            ]
        );
        assert_eq!(parse_text(b"countries={}").unwrap().doctrine, None);
        assert!(
            parse_text(b"countries={} doctrine={ countries={ { enable_tactic={ a=1 } } } }")
                .is_err()
        );
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
    fn party_names_follow_the_writer_default_rule() {
        let pack = parse_text(
            b"countries={AAA={politics={parties={
            communism={default=no name=\"AAA_x_party\" long_name=\"\" popularity=100}
            democratic={default=yes popularity=0}
            fascism={popularity=0}
        }}}}",
        )
        .unwrap();
        let names = &pack.countries[b"AAA".as_slice()]
            .politics
            .as_ref()
            .unwrap()
            .names;
        assert_eq!(
            names[b"communism".as_slice()],
            PartyName {
                default: Some(false),
                keys: Some((b"AAA_x_party".to_vec(), Vec::new())),
            }
        );
        assert_eq!(
            names[b"democratic".as_slice()],
            PartyName {
                default: Some(true),
                keys: None,
            }
        );
        assert_eq!(names[b"fascism".as_slice()], PartyName::default());
        for text in [
            b"countries={AAA={politics={parties={x={default=no name=a popularity=1}}}}}".as_slice(),
            b"countries={AAA={politics={parties={x={default=yes name=a long_name=b popularity=1}}}}}",
            b"countries={AAA={politics={parties={x={long_name=b popularity=1}}}}}",
        ] {
            assert!(parse_text(text).is_err());
        }
    }

    #[test]
    fn party_leaders_keep_saved_order_and_the_null_character_reference() {
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
            b"countries={A={dynamic_modifier={modifier={modifier=x enabled=yes value={0.000001}}}}}",
            b"countries={A={decision_status={active_timed_decision={decision=x days=1 state=unknown}}}}",
            b"countries={} tech_sharing_group={id=x countries={A B} bonuses={0}}",
        ] { assert!(parse_text(text).is_err(), "{}", String::from_utf8_lossy(text)); }
    }
}
