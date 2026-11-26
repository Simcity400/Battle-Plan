use dashboard_effects::ScriptedEffects;
use dashboard_text::triggers::{
    Combat, Eval, Evaluator, Scope, Scopes, World, compare, shown, trigger_boolean,
};
use content_reader_script::{trigger_grammar::effect_slot, scripted_trigger_grammar::ScriptedTriggers, trigger_keys::{CountTriggerNames, TriggerNames}, sub_units::SubUnits};
use content_reader_military::leader_traits::LeaderTraits;
use content_reader_parser::parser::{Item, Operator, c_string};
use content_reader_key_names::unit_leader::Traits;
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{
    rows::LandCombatDetailsRow, rows::LandCombatDivisionRow, rows::LexicalTokenRow, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};
use viewer_engagements::engagement_progress::sides as engagement_progress_sides;
#[cfg(test)]
use viewer_foundation::tooltip_text::NoIcons;
use super::content::Content as StaticContent;
use viewer_foundation::perspective::Vision;
use dashboard_text::table_access::rows;
use {
    super::{
        Selection, Values,
        binding::{Bindings, hide, sprite, text},
    },
    viewer_foundation::tooltip_text::{TextIcons, builtin, token},
};

mod entry_tooltips;

#[derive(Debug, Default)]
pub(super) struct Menu {
    pub values: Values,
    pub entries: Vec<Values>,
}

#[derive(Debug)]
struct Definition {
    active: bool,
    attacker: bool,
    items: Vec<Item>,
    phase: i32,
    display_phase: i32,
    effects: [i64; 6],
    picture: Result<String, String>,
}

impl Definition {
    fn blocks(&self, key: &str) -> Vec<Item> {
        self.items
            .iter()
            .filter(|item| item.keyword_is(key))
            .filter_map(|item| item.value.items())
            .flatten()
            .cloned()
            .collect()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Effects {
    pub width: i64,
    pub movement: i64,
    pub attacker: i64,
    pub attacker_org: i64,
    pub defender: i64,
    pub defender_org: i64,
}

#[derive(Debug)]
struct RegistryEntry {
    id: i32,
    token: Vec<u8>,
    definition: Result<Definition, String>,
}

#[derive(Default)]
struct Candidate {
    tactic_id: i32,
    unlock_id: u32,
    token: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct Content {
    phases: Vec<Vec<u8>>,
    registry: Vec<RegistryEntry>,
    leader_traits: Traits,
    country_leader_traits: Option<Result<Arc<LeaderTraits>, String>>,
    sub_units: SubUnits,
    sub_units_unreadable: Option<String>,
    registry_unsettled: Option<String>,
    names: TriggerNames,
    warnings: Vec<String>,
}

fn field<'a>(items: &'a [Item], name: &str) -> Option<&'a Item> {
    items.iter().rev().find(|item| item.keyword_is(name))
}

impl Content {
    pub(super) fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub(super) fn set_count_names(&mut self, counts: CountTriggerNames) {
        self.names.counts = Some(counts);
    }

    pub fn trigger_names(&self) -> TriggerNames {
        self.names.clone()
    }

    pub(super) fn trait_names(&self) -> &Traits {
        &self.leader_traits
    }

    pub(super) fn from_database(database: content_reader_military::combat_tactics::Database) -> Self {
        let content_reader_military::combat_tactics::Database {
            phases,
            tactics,
            unsupported,
            logged,
        } = database;
        let registry_unsettled = unsupported.first().cloned();
        let mut warnings: Vec<String> = unsupported
            .into_iter()
            .map(|reason| format!("Combat tactics: {reason}."))
            .chain(
                logged
                    .into_iter()
                    .map(|reason| format!("Combat tactics content error: {reason}.")),
            )
            .collect();
        let registry = tactics
            .into_iter()
            .map(|tactic| {
                let definition = definition(&tactic, &mut warnings);
                RegistryEntry {
                    id: tactic.id,
                    token: tactic.token,
                    definition,
                }
            })
            .collect();
        Self {
            phases,
            registry,
            registry_unsettled,
            warnings,
            ..Self::default()
        }
    }

    pub(super) fn with_country_leader_traits(
        mut self,
        traits: Result<Arc<LeaderTraits>, String>,
    ) -> Self {
        self.country_leader_traits = Some(traits);
        self
    }

    fn character_traits(&self) -> Result<(&Traits, &LeaderTraits), &str> {
        match &self.country_leader_traits {
            None => Err("the country-leader traits are not loaded"),
            Some(Err(why)) => Err(why),
            Some(Ok(traits)) => Ok((&self.leader_traits, traits)),
        }
    }

    pub fn trait_databases(&self) -> Option<(Traits, Result<Arc<LeaderTraits>, String>)> {
        self.country_leader_traits
            .as_ref()
            .map(|traits| (self.leader_traits.clone(), traits.clone()))
    }

    pub(super) fn with_inventories(
        mut self,
        leader_traits: Traits,
        sub_units: Result<&SubUnits, &String>,
        mut scripted: ScriptedTriggers,
    ) -> Self {
        scripted = map_content::loader::resolved_scripted(&self.phases, scripted);
        self.warnings.extend(
            scripted
                .logged
                .iter()
                .map(|reason| format!("Combat tactics: triggers: {reason}.")),
        );
        self.names.scripted = scripted;
        self.warnings.extend(
            leader_traits
                .unsupported
                .iter()
                .map(|reason| format!("Combat tactics: has_trait: {reason}.")),
        );
        self.leader_traits = leader_traits;
        match sub_units {
            Ok(sub_units) => self.sub_units = sub_units.clone(),
            Err(reason) => self.sub_units_unreadable = Some(reason.clone()),
        }
        self
    }

    fn entry(&self, id: i32, token: &[u8]) -> Evaluation<&Definition> {
        let name = String::from_utf8_lossy(token);
        let index = usize::try_from(id)
            .ok()
            .and_then(|id| id.checked_sub(1))
            .ok_or_else(|| {
                format!("recorded tactic {name}'s id {id} is not a content tactic id")
            })?;
        let entry = self.registry.get(index).ok_or_else(|| match &self.registry_unsettled {
            Some(reason) => format!(
                "recorded tactic {name}'s id {id} is past the {} combat tactics read before {reason}",
                self.registry.len()
            ),
            None => format!(
                "recorded tactic {name}'s id {id} is past the content's {} combat tactics",
                self.registry.len()
            ),
        })?;
        if entry.id != id || entry.token != token {
            return Err(format!(
                "recorded tactic id {id} is {name}, but the content's tactic {id} is {}",
                String::from_utf8_lossy(&entry.token)
            ));
        }
        entry.definition.as_ref().map_err(Clone::clone)
    }

    fn definition(&self, candidate: &Candidate) -> Evaluation<&Definition> {
        self.entry(candidate.tactic_id, &candidate.token)
    }

    fn named_id(&self, name: &[u8]) -> Evaluation<i32> {
        if let Some(why) = &self.registry_unsettled {
            return Err(format!("the tactic name registry is incomplete: {why}"));
        }
        Ok(self
            .registry
            .iter()
            .rev()
            .find(|entry| entry.token == name)
            .map_or(0, |entry| entry.id))
    }

    pub(super) fn picture(&self, id: i32, token: &[u8]) -> Evaluation<&str> {
        let definition = self.entry(id, token)?;
        definition.picture.as_deref().map_err(Clone::clone)
    }

    pub(super) fn refusals(&self) -> impl Iterator<Item = &str> {
        self.registry
            .iter()
            .filter_map(|entry| match &entry.definition {
                Err(why) => Some(why.as_str()),
                Ok(definition) => definition.picture.as_ref().err().map(String::as_str),
            })
    }

    pub(super) fn pictures(&self) -> impl Iterator<Item = (&str, String)> {
        self.registry.iter().filter_map(|entry| {
            let key = entry.definition.as_ref().ok()?.picture.as_deref().ok()?;
            let name = key.strip_prefix(PICTURE_PREFIX)?;
            Some((key, format!("gfx/interface/landcombat/tactics/{name}.dds")))
        })
    }

    pub(super) fn effects(&self, id: i32, token: &[u8]) -> Result<Effects, String> {
        let definition = self.entry(id, token)?;
        let [
            width,
            movement,
            attacker,
            attacker_org,
            defender,
            defender_org,
        ] = definition.effects;
        Ok(Effects {
            width,
            movement,
            attacker,
            attacker_org,
            defender,
            defender_org,
        })
    }
}

const PICTURE_PREFIX: &str = "combat_tactic:";

pub const NULL_PICTURE: &str = PICTURE_PREFIX;

fn definition(
    tactic: &content_reader_military::combat_tactics::Tactic,
    warnings: &mut Vec<String>,
) -> Result<Definition, String> {
    let name = String::from_utf8_lossy(&tactic.token);
    if let Some(reason) = &tactic.unproven {
        return Err(format!(
            "tactic {name}'s ID or fields cannot be read as the game reads them: {reason}"
        ));
    }
    let items = &tactic.fields;
    let active = field_boolean(&name, items, "active", true, warnings);
    let attacker = field_boolean(&name, items, "is_attacker", false, warnings);
    let picture = match field(items, "picture").map(|item| &item.value) {
        None => Err(format!("tactic {name} has no picture field")),
        Some(value) => match (value.scalar(), value.c_str()) {
            (Some(_), Ok(picture)) => Ok(format!("{PICTURE_PREFIX}{picture}")),
            (Some(_), Err(_)) => Err(format!(
                "tactic {name}'s picture {} is not UTF-8 before its first NUL, so its texture path cannot be formed",
                shown(value)
            )),
            (None, _) => Err(format!("tactic {name}'s picture is not a value")),
        },
    };
    Ok(Definition {
        active,
        attacker,
        items: items.clone(),
        phase: tactic.phase,
        display_phase: tactic.display_phase,
        effects: tactic.effects,
        picture,
    })
}

fn field_boolean(
    tactic: &str,
    items: &[Item],
    key: &str,
    default: bool,
    warnings: &mut Vec<String>,
) -> bool {
    let mut state = default;
    for item in items.iter().filter(|item| item.keyword_is(key)) {
        match item.value.scalar().and_then(content_reader_parser::parser::read_bool) {
            Some(value) => state = value,
            _ => warnings.push(format!(
                "Combat tactics: tactic {tactic}'s {key} = {} is not exactly yes or no; the game logs an error and keeps {}.",
                shown(&item.value),
                if state { "yes" } else { "no" }
            )),
        }
    }
    state
}

const NULL_TACTIC_NAME: &[u8] = b"nullCombatTactic";

fn candidates(content: &Content, lexical: &[LexicalTokenRow]) -> Evaluation<Vec<Candidate>> {
    content
        .registry
        .iter()
        .enumerate()
        .map(|(ordinal, entry)| {
            let name = String::from_utf8_lossy(&entry.token);
            let id = i32::try_from(ordinal + 1).map_err(|_| {
                format!("tactic {name} is at registry position {ordinal}, beyond the native ID range")
            })?;
            if entry.id != id {
                return Err(format!(
                    "tactic {name} has ID {} at registry position {id}",
                    entry.id
                ));
            }
            let mut matches = lexical
                .iter()
                .filter(|row| row.literal.eq_ignore_ascii_case(&entry.token));
            let token = matches.next().ok_or_else(|| {
                format!("native unlock token for tactic {name} is absent from the lexical dictionary")
            })?;
            if token.token == 0 {
                return Err(format!(
                    "native unlock token for tactic {name} is the invalid token 0"
                ));
            }
            if let Some(other) = matches.next() {
                return Err(format!(
                    "native unlock token for tactic {name} is ambiguous: tokens {} and {} both spell it",
                    token.token, other.token
                ));
            }
            Ok(Candidate {
                tactic_id: id,
                unlock_id: token.token,
                token: entry.token.clone(),
            })
        })
        .collect()
}

type Evaluation<T> = Result<T, String>;

struct Context<'a> {
    detail: &'a LandCombatDetailsRow,
    marker_flags: Option<[u32; 2]>,
    candidates: &'a [Candidate],
    side: usize,
    content: &'a Content,
    preference_factors: Result<[i64; 3], String>,
    terrain: Result<[i64; 2], String>,
    terrains: Vec<&'a [u8]>,
    aliases: &'a BTreeMap<u32, u32>,
    links: Result<(), String>,
    countries: Result<(), String>,
    first_countries: [Option<u32>; 2],
    country_unlocks: BTreeMap<u32, &'a [u32]>,
    country_tags: BTreeMap<&'a [u8], u32>,
    divisions: BTreeMap<(u8, u64), &'a LandCombatDivisionRow>,
    values: BTreeMap<(u8, u64), replay_data::rows::LandCombatMemberValues>,
    movement_sources: BTreeMap<u64, u32>,
    participants: &'a [u64],
    tokens: BTreeMap<u32, &'a [u8]>,
    tokens_complete: Result<(), String>,
    province_vp: Option<i32>,
    world: World<'a>,
}

impl Context<'_> {
    fn preference_factor(&self, position: usize) -> Evaluation<i64> {
        self.preference_factors
            .as_ref()
            .map(|factors| factors[position])
            .map_err(|why| format!("the preferred-tactic weight defines are unavailable: {why}"))
    }

    fn country_slot(&self) -> Evaluation<u32> {
        let raw = self.first_countries[self.side].ok_or_else(|| {
            format!(
                "the {} has no first primary country in LandCombatSides",
                side_name(self.side)
            )
        })?;
        self.canonical(raw).map_err(|why| {
            format!(
                "the {}'s first primary country: {why}",
                side_name(self.side)
            )
        })
    }

    fn canonical(&self, link: u32) -> Evaluation<u32> {
        self.aliases
            .get(&link)
            .copied()
            .ok_or_else(|| match &self.links {
                Ok(()) => format!("country link {link} has no CountryLinks row"),
                Err(why) => format!("country link {link} has no CountryLinks row, and {why}"),
            })
    }

    fn division(&self, unit: u64) -> Evaluation<&LandCombatDivisionRow> {
        self.divisions
            .get(&(self.side as u8, unit))
            .copied()
            .ok_or_else(|| {
                format!(
                    "{} division {unit} has no LandCombatDivisions row",
                    side_name(self.side)
                )
            })
    }

    fn token_ids(&self, text: &[u8]) -> Vec<u32> {
        self.tokens
            .iter()
            .filter(|(_, literal)| literal.eq_ignore_ascii_case(text))
            .map(|(id, _)| *id)
            .collect()
    }

    fn unlocked(
        &self,
        candidate: &Candidate,
        definition: &Definition,
        slot: u32,
    ) -> Evaluation<bool> {
        if definition.active {
            return Ok(true);
        }
        let unlocks = self
            .country_unlocks
            .get(&slot)
            .ok_or_else(|| format!("country slot {slot} has no CountryTacticUnlocks row"))?;
        unlocks.iter().try_fold(false, |found, &id| {
            if id == 0 {
                let null = self
                    .tokens
                    .get(&candidate.unlock_id)
                    .is_some_and(|literal| literal.eq_ignore_ascii_case(NULL_TACTIC_NAME));
                return Ok(found || null);
            }
            let unlocked = usize::try_from(id)
                .ok()
                .and_then(|id| id.checked_sub(1))
                .and_then(|ordinal| self.candidates.get(ordinal))
                .filter(|unlocked| unlocked.tactic_id as u32 == id)
                .ok_or_else(|| {
                    format!(
                        "country slot {slot} records unlocked tactic ID {id}, outside the matching tactic registry"
                    )
                })?;
            Ok(found || unlocked.unlock_id == candidate.unlock_id)
        })
    }

    fn has_tactic(
        &self,
        operand: &[u8],
        slot: impl FnOnce() -> Evaluation<u32>,
    ) -> Evaluation<bool> {
        let shown = String::from_utf8_lossy(operand);
        let stored = match self.token_ids(operand).as_slice() {
            [id] => self.tokens[id],
            [] => {
                return Err(match &self.tokens_complete {
                    Ok(()) => format!(
                        "{shown} is not in the recorded lexical dictionary, though the trigger's reader interns it"
                    ),
                    Err(why) => format!(
                        "{shown} is not in the recorded lexical dictionary, which is not the whole dictionary ({why}), so the spelling the trigger looks up is unknown"
                    ),
                });
            }
            [first, second, ..] => {
                return Err(format!(
                    "{shown} is spelled by tokens {first} and {second} in the recorded lexical dictionary, so which the trigger's reader interns is unknown"
                ));
            }
        };
        let id = self.content.named_id(stored)?;
        if id == 0 {
            return Ok(true);
        }
        let candidate = self
            .candidates
            .iter()
            .find(|candidate| candidate.tactic_id == id)
            .ok_or_else(|| {
                format!(
                    "tactic {} has no unlock token",
                    String::from_utf8_lossy(stored)
                )
            })?;
        let definition = self.content.definition(candidate)?;
        if definition.active {
            return Ok(true);
        }
        self.unlocked(candidate, definition, slot()?)
    }

    fn check(&self, items: &[Item]) -> Evaluation<bool> {
        Evaluator::new(&self.world, Some(self))
            .evaluate(items, Scopes::combat())
            .decided()
    }

    fn combat_trigger(
        &self,
        name: &str,
        item: &Item,
        this: Scope,
        evaluator: &Evaluator,
    ) -> Option<Evaluation<bool>> {
        let key = String::from_utf8_lossy(item.key.as_deref().unwrap_or_default()).into_owned();
        let (comparison, value) = (item.operator, &item.value);
        let symbol = String::from_utf8_lossy(comparison.text());
        let fail = |why: &str| format!("trigger {key} {symbol} {}: {why}", shown(value));
        if name == "has_tactic" {
            let country = || match this {
                Scope::Combat => self.country_slot(),
                Scope::Country(slot) => Ok(slot),
                other => Err(format!(
                    "the trigger's scope check passed in a {} scope, which holds no country",
                    evaluator.describe(other)
                )),
            };
            let operand = value
                .scalar()
                .map(c_string)
                .ok_or_else(|| fail("the operand is a block"));
            return Some(
                operand.and_then(|operand| {
                    self.has_tactic(operand, country).map_err(|why| fail(&why))
                }),
            );
        }
        if this != Scope::Combat {
            return None;
        }
        let bytes = value.scalar().unwrap_or_default();
        let operand = || {
            value
                .scalar_str()
                .ok_or_else(|| fail("the operand is not UTF-8"))
        };
        let c_bytes = || {
            value
                .scalar()
                .map(c_string)
                .ok_or_else(|| fail("the operand is a block"))
        };
        let side = &self.detail.sides[self.side];
        let numeric = |actual: i64, fixed| {
            let required = evaluator
                .required(bytes, item.quoted(), fixed, Scopes::combat())
                .map_err(|why| fail(&why))?;
            Ok(compare(actual, required, comparison, fixed))
        };
        let skill = |side: &replay_data::rows::LandCombatSideDetails| {
            if side.commander == 0 {
                0
            } else {
                side.commander_skill
            }
        };
        Some(match name {
            "is_attacker" => trigger_boolean(item)
                .map(|desired| desired == (self.side == 0))
                .map_err(|why| fail(&why)),
            "phase" => {
                operand().and_then(|operand| {
                    operand
                        .parse::<i32>()
                        .map(|phase| phase == self.detail.phase)
                        .map_err(|_| {
                            fail("the operand is not a phase ID resolved from the content")
                        })
                })
            }
            "has_reserves" => trigger_boolean(item)
                .map(|desired| desired != side.reserves.is_empty())
                .map_err(|why| fail(&why)),
            "skill" => numeric(i64::from(skill(side)) * 100_000, false),
            "skill_advantage" => {
                let other = &self.detail.sides[1 - self.side];
                numeric(
                    i64::from(skill(side).wrapping_sub(skill(other))) * 100_000,
                    false,
                )
            }
            "has_trait" => {
                let presentation = &side.presentation;
                if side.commander == 0 || presentation.character_state <= 1 {
                    return Some(Ok(false));
                }
                if presentation.character_state != 2 {
                    return Some(Err(fail(&format!(
                        "the commander's recorded character state {} is not one whose traits the viewer reads",
                        presentation.character_state
                    ))));
                }
                let character = (presentation.character >> 32) as u32;
                evaluator
                    .character_traits(character, bytes, || {
                        Ok((presentation.character_role_leader != 0).then(|| {
                            presentation
                                .role_traits
                                .iter()
                                .map(|entry| entry.definition_ordinal)
                                .collect()
                        }))
                    })
                    .map_err(|why| fail(&why))
            }
            "is_fighting_in_terrain" => {
                let valid = self.detail.stored_terrain_valid != 0;
                let named = self
                    .terrains
                    .iter()
                    .find(|name| !item.quoted() && name.eq_ignore_ascii_case(bytes));
                match named {
                    None => Ok(!valid),
                    Some(name) if !valid || self.detail.stored_terrain_name != *name => Ok(false),
                    Some(name)
                        if self.terrains.iter().filter(|other| *other == name).count() > 1 =>
                    {
                        Err(fail(&format!(
                            "the content defines terrain {} more than once and the recording does not say which the combat holds",
                            String::from_utf8_lossy(name)
                        )))
                    }
                    Some(_) => Ok(true),
                }
            }
            "province_vp" => match self.province_vp {
                None => Err(fail(&format!(
                    "province {} has no Provinces row",
                    self.detail.province
                ))),
                Some(points) => numeric(i64::from(points) * 100_000, true),
            },
            "frontage_full" => self.frontage_full(item).map_err(|why| fail(&why)),
            "hardness" => (|| {
                if comparison == Operator::Equals {
                    return Err(fail(
                        "the game asserts on '=' instead of choosing a reduction",
                    ));
                }
                let greater = comparison == Operator::Greater;
                let mut actual = if greater { 100_000 } else { 0 };
                for unit in self.participants {
                    self.division(*unit).map_err(|why| fail(&why))?;
                    let hardness = self
                        .values
                        .get(&(self.side as u8, *unit))
                        .ok_or_else(|| {
                            fail("a participant's stats are in neither its LandCombatDivisions row nor its division's rows")
                        })?
                        .stat_values[2];
                    actual = if greater {
                        actual.min(hardness)
                    } else {
                        actual.max(hardness)
                    };
                }
                numeric(actual, true)
            })(),
            "has_combat_modifier" if item.quoted() => Err(fail(
                "the operand is quoted, so token 0xF names no combat-modifier keyword; what the trigger then holds is not reproduced",
            )),
            "has_combat_modifier" => operand().and_then(|operand| {
                if operand.eq_ignore_ascii_case("river_crossing") {
                    self.marker_flags
                        .map(|flags| flags[self.side] & (1 << 10) != 0)
                        .ok_or_else(|| {
                            fail("the combat's LandCombatSides marker flags are unavailable")
                        })
                } else {
                    Err(fail(
                        "only the river_crossing combat modifier is reproduced",
                    ))
                }
            }),
            "has_unit_type" => (|| {

                let operand = c_bytes()?;
                for unit in self.participants {
                    let row = self.division(*unit).map_err(|why| fail(&why))?;
                    for term in self.composition(*unit, row).map_err(|why| fail(&why))? {
                        if term.1.eq_ignore_ascii_case(operand) {
                            return Ok(true);
                        }
                    }
                }
                Ok(false)
            })(),
            "has_artillery_ratio" => (|| {
                if let Some(reason) = &self.content.sub_units_unreadable {
                    return Err(fail(&format!(
                        "the content's sub-unit database is unreadable: {reason}"
                    )));
                }
                let (mut total, mut artillery) = (0_i64, 0_i64);
                for unit in self.participants {
                    let row = self.division(*unit).map_err(|why| fail(&why))?;
                    let mut count = 0_i64;
                    for (units, name) in self.composition(*unit, row).map_err(|why| fail(&why))? {
                        count += units;
                        if self
                            .content
                            .sub_units
                            .artillery(name)
                            .map_err(|why| fail(&why))?
                        {
                            artillery = artillery.wrapping_add(units);
                        }
                    }
                    if count > i64::from(i16::MAX) {
                        return Err(fail(&format!(
                            "division {unit} holds {count} sub-units, beyond the proved signed16 grid-count bound"
                        )));
                    }
                    total = total.wrapping_add(count);
                }
                numeric(
                    if total == 0 {
                        0
                    } else {
                        artillery
                            .wrapping_mul(100_000)
                            .checked_div(total)
                            .ok_or_else(|| fail("the game's artillery ratio division traps"))?
                    },
                    true,
                )
            })(),
            _ => return None,
        })
    }

    fn frontage_full(&self, item: &Item) -> Evaluation<bool> {
        let side = &self.detail.sides[self.side];
        let committed = &self.detail.sides[0].committed;
        let directions = super::land_details::direction_count(
            committed,
            &self.divisions,
            self.detail.province,
            &self.movement_sources,
        )
        .map_err(|reason| format!("the attacker's directions are unavailable: {reason}"))?;
        let mut used = 0_i64;
        for unit in &side.committed {
            let row = self.division(*unit)?;
            let width = super::land_details::division_width(row)
                .ok_or_else(|| format!("division {unit}'s width modifiers are unavailable"))?;
            used = used.wrapping_add(width);
        }
        let terrain = self.terrain.as_ref().map_err(Clone::clone)?;
        let full = super::land_details::total_width(self.detail, *terrain, directions)
            .wrapping_sub(used)
            <= 0;
        trigger_boolean(item)
            .map(|desired| desired == full)
            .map_err(|why| why.to_string())
    }

    fn category_share(&self, mask: u64) -> Result<i64, String> {
        let mut matching = 0_i32;
        for unit in self.participants {
            let row = self.division(*unit)?;
            if row.categories & mask != 0 {
                matching = matching.wrapping_add(1);
            }
        }
        let units = self.participants.len() as i32;
        if units == 0 {
            return Ok(0);
        }
        let (matching, units) = (i64::from(matching), i64::from(units));
        Ok(
            if units * 100_000 != 0 && -0x7fff_ffff_ffff_ffff < matching * 100_000 {
                matching * 10_000_000_000 / (units * 100_000)
            } else {
                0xffff_ffff
            },
        )
    }

    fn composition(&self, unit: u64, row: &LandCombatDivisionRow) -> Evaluation<Vec<(i64, &[u8])>> {
        if row.context.template == 0 {
            return Ok(Vec::new());
        }
        if row.width_terms.is_empty() {
            return Err(format!(
                "division {unit}'s template has no verified composition aggregate"
            ));
        }
        row.width_terms
            .iter()
            .map(|term| {
                if term.count <= 0 {
                    return Err(format!(
                        "division {unit}'s template records {} of sub-unit token {}, not a positive count",
                        term.count, term.subunit
                    ));
                }
                let name = self.tokens.get(&term.subunit).ok_or_else(|| {
                    format!(
                        "sub-unit token {} of division {unit} is not in the lexical dictionary",
                        term.subunit
                    )
                })?;
                Ok((i64::from(term.count), *name))
            })
            .collect()
    }

    fn weight(&self, candidate: &Candidate, definition: &Definition) -> Evaluation<i64> {
        let slot = self.country_slot()?;
        if !self.unlocked(candidate, definition, slot)? {
            return Ok(0);
        }
        if !self.check(&definition.blocks("trigger"))? {
            return Ok(0);
        }
        let mut weight = Evaluator::new(&self.world, Some(self))
            .mean_time_to_happen("base weight", &definition.blocks("base"), Scopes::combat())
            .map_err(|why| why.to_string())?;
        let inputs = &self.detail.tactic_inputs;
        for preference in 0..3 {
            if inputs.preferred_tactic_ids[self.side][preference] != candidate.tactic_id {
                continue;
            }
            let factor = self.preferred_weight_factor(preference)?;
            weight = weight.wrapping_mul(100_000_i64.wrapping_add(factor)) / 100_000;
        }
        Ok(weight)
    }

    fn preferred_weight_factor(&self, preference: usize) -> Evaluation<i64> {
        let factor = self.preference_factor(preference)?;
        if preference != 0 {
            return Ok(factor);
        }
        let inputs = &self.detail.tactic_inputs;
        if inputs.country_weight_present[self.side] == 0 {
            return Err(format!(
                "the {}'s preferred country tactic modifier is not recorded",
                side_name(self.side)
            ));
        }
        let recorded = &inputs.country_weights[self.side];
        Ok(
            factor.wrapping_add(if recorded.accumulator_entry_present == 0 {
                0
            } else {
                recorded.accumulator_raw
            }),
        )
    }
}

impl Combat for Context<'_> {
    fn country(&self) -> Result<u32, dashboard_text::triggers::Failure> {
        self.country_slot().map_err(Into::into)
    }

    fn trigger(&self, name: &str, item: &Item, this: Scope, evaluator: &Evaluator) -> Option<Eval> {
        self.combat_trigger(name, item, this, evaluator)
            .map(|value| match value {
                Ok(value) => Eval::from_bool(value),
                Err(why) => Eval::unknown(why),
            })
    }

    fn category_share(&self, mask: u64) -> Result<i64, dashboard_text::triggers::Failure> {
        Context::category_share(self, mask).map_err(Into::into)
    }
}

fn side_name(side: usize) -> &'static str {
    if side == 0 { "attacker" } else { "defender" }
}

fn normalized_chances(weights: &[Option<i64>]) -> Vec<Option<i64>> {
    let total = weights.iter().try_fold(0_i64, |sum, weight| {
        Some(sum.wrapping_add((*weight)?.max(0)))
    });
    weights
        .iter()
        .map(|weight| match (weight, total) {
            (Some(0), _) => Some(0),
            (Some(weight), Some(total)) => Some(
                if (*weight as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd {
                    0xffff_ffff
                } else {
                    weight.wrapping_mul(100_000) / if total > 0 { total } else { 100_000 }
                },
            ),
            _ => None,
        })
        .collect()
}

pub(super) fn viewing_side(
    primary: &[u32],
    side: &str,
    aliases: &BTreeMap<u32, u32>,
    viewing_slot: u32,
) -> Evaluation<usize> {
    let mut member = false;
    let mut unresolved = None;
    for raw in primary {
        match aliases.get(raw) {
            Some(slot) => member |= *slot == viewing_slot,
            None => unresolved = unresolved.or(Some(*raw)),
        }
    }
    match (member, unresolved) {
        (true, _) => Ok(0),
        (false, Some(raw)) => Err(format!(
            "the viewing country's combat side is unavailable: {side} primary country {raw} has no CountryLinks row"
        )),
        (false, None) => Ok(1),
    }
}

fn localize_named(
    icons: &dyn TextIcons,
    loc: &Localisation,
    key: &str,
    argument: &str,
    name: &[u8],
) -> Evaluation<String> {
    let name = token(icons, loc, name)?;
    builtin(icons, loc, key, &[(argument, Argument::Text(&name))])
}

pub(super) fn bind(
    out: &mut Bindings,
    static_content: &StaticContent,
    frame: &DecodedFrame,
    selection: Selection,
    vision: &Vision,
    nato: bool,
) -> io::Result<()> {
    let Selection::Land(id) = selection else {
        return Ok(());
    };
    let details = rows::<tables::LandCombatDetails>(frame)?;
    let Some(detail) = details.rows().iter().find(|row| row.combat == id) else {
        out.warnings.push(format!(
            "Possible tactics: land combat {id} has no LandCombatDetails row at hour {}; the list stays empty.",
            frame.hour
        ));
        return Ok(());
    };
    let sides = engagement_progress_sides(frame)?;
    let country_sets = sides.get(&id).copied();
    let marker_flags = country_sets.map(|row| row.flags);
    let combats = rows::<tables::LandCombats>(frame)?;
    let Some(combat) = combats.rows().iter().find(|row| row.id == id) else {
        out.warnings.push(format!(
            "Possible tactics: land combat {id} has no LandCombats row at hour {}; the list stays empty.",
            frame.hour
        ));
        return Ok(());
    };
    let links = rows::<tables::CountryLinks>(frame)?;
    let aliases: BTreeMap<_, _> = links
        .rows()
        .iter()
        .map(|row| (row.id, row.canonical_slot))
        .collect();
    let whole = |table: &str, error: io::Result<()>| {
        error.map_err(|error| format!("{table} is not complete this hour: {error}"))
    };
    let first_countries = country_sets.map_or([None, None], |sets| {
        [
            sets.attacker_primary.first().copied(),
            sets.defender_primary.first().copied(),
        ]
    });
    let mut unavailable = BTreeSet::new();
    let selected_side = country_sets
        .ok_or_else(|| {
            format!("the viewing country's combat side is unavailable: combat {id} has no LandCombatSides row")
        })
        .and_then(|sets| viewing_side(&sets.attacker_primary, "attacker", &aliases, vision.ours()));
    let side_known = selected_side.is_ok();
    let side = match selected_side {
        Ok(side) => side,
        Err(reason) => {
            unavailable.insert(reason);
            1
        }
    };
    let tokens = rows::<tables::LexicalTokens>(frame)?;
    let content = &static_content.assets.tactics;
    let candidates = match candidates(content, tokens.rows()) {
        Ok(candidates) => candidates,
        Err(reason) => {
            unavailable.insert(reason);
            Vec::new()
        }
    };
    let unlock_rows = rows::<tables::CountryTacticUnlocks>(frame)?;
    let mut country_unlocks = BTreeMap::new();
    for row in unlock_rows.rows() {
        match aliases.get(&row.country) {
            Some(&slot) => {
                country_unlocks.insert(slot, row.unlocked_tactic_ids.as_slice());
            }
            None => {
                unavailable.insert(format!(
                    "the tactic unlocks recorded for country {} cannot be placed: it has no CountryLinks row",
                    row.country
                ));
            }
        }
    }
    let terrain = match static_content.terrain.combat_terrain.get(&detail.province) {
        None => Err(format!(
            "province {} has no terrain category in the map definitions",
            detail.province
        )),
        Some(terrain) => match static_content.terrain.combat_widths(terrain) {
            None => Err(format!(
                "terrain {terrain} of province {} has no combat widths in the content",
                detail.province
            )),
            Some(widths) => Ok(*widths),
        },
    };
    let division_rows = rows::<tables::LandCombatDivisions>(frame)?;
    let combat_members = viewer_engagements::land_combat_math::combat_rows(division_rows.rows(), id);
    let member_values = viewer_engagements::land_combat_math::member_values(frame, combat_members)?;
    let context = Context {
        detail,
        marker_flags,
        candidates: &candidates,
        side,
        content,
        preference_factors: static_content
            .assets
            .land_defines
            .as_ref()
            .map(|defines| defines.preference_factors)
            .map_err(Clone::clone),
        terrain,
        terrains: static_content
            .terrain
            .terrain_widths
            .iter()
            .map(|(name, _)| name.as_slice())
            .collect(),
        aliases: &aliases,
        links: whole(
            "CountryLinks",
            frame.complete::<tables::CountryLinks>().map(|_| ()),
        ),
        countries: whole(
            "Countries",
            frame.complete::<tables::Countries>().map(|_| ()),
        ),
        first_countries,
        country_unlocks,
        country_tags: rows::<tables::Countries>(frame)?
            .rows()
            .iter()
            .map(|row| (row.tag.as_slice(), row.canonical_slot))
            .collect(),
        divisions: combat_members
            .iter()
            .map(|row| ((row.side, row.division), row))
            .collect(),
        values: combat_members
            .iter()
            .zip(member_values)
            .filter_map(|(row, values)| Some(((row.side, row.division), values?)))
            .collect(),
        movement_sources: rows::<tables::DivisionMovement>(frame)?
            .rows()
            .iter()
            .map(|row| (row.division, row.source))
            .collect(),
        participants: if side == 0 {
            &combat.attackers
        } else {
            &combat.defenders
        },
        tokens: tokens
            .rows()
            .iter()
            .map(|row| (row.token, row.literal.as_slice()))
            .collect(),
        tokens_complete: whole(
            "LexicalTokens",
            frame.complete::<tables::LexicalTokens>().map(|_| ()),
        ),
        province_vp: rows::<tables::Provinces>(frame)?
            .rows()
            .iter()
            .find(|row| row.id == detail.province)
            .map(|row| row.victory_points),
        world: World::read(
            frame,
            dashboard_text::triggers::ContentFacts {
                character_traits: content.character_traits(),
                ..static_content.trigger_content.facts()
            },
        ),
    };
    let mut weights = Vec::with_capacity(candidates.len());
    let mut unlocked = Vec::with_capacity(candidates.len());
    for candidate in &candidates {
        let definition = match content.definition(candidate) {
            Ok(definition) => definition,
            Err(reason) => {
                unavailable.insert(reason);
                weights.push(None);
                unlocked.push(None);
                continue;
            }
        };
        let name = String::from_utf8_lossy(&candidate.token);
        let mut refused = |reason: String| {
            unavailable.insert(format!("tactic {name}: {reason}"));
        };
        weights.push(if side_known {
            context
                .weight(candidate, definition)
                .map_err(&mut refused)
                .ok()
        } else {
            None
        });
        unlocked.push(
            context
                .unlocked(candidate, definition, vision.ours())
                .map_err(&mut refused)
                .ok(),
        );
    }
    let chances = normalized_chances(&weights);
    let (icons, loc): (&dyn TextIcons, _) =
        (static_content.assets.as_ref(), &static_content.localisation);
    let mut menu = Menu::default();
    for name in ["filter_all", "filter_attack", "filter_defend"] {
        hide(&mut menu.values, name);
    }
    if let Some(popup) = static_content
        .assets
        .native_windows
        .gui_type(b"tactics_list_view")
    {
        preferred_description(&mut menu.values, popup, icons, loc, &mut unavailable);
    }
    match builtin(icons, loc, "TACTICS_LIST_VIEW_TITLE", &[]) {
        Ok(title) => text(&mut menu.values, "window_title", title),
        Err(reason) => withhold(
            &mut menu.values,
            "window_title",
            &mut unavailable,
            format!("the title: {reason}"),
        ),
    }
    let phase: Evaluation<&[u8]> = if detail.phase < 0 {
        Ok(b"PHASE_DEFAULT")
    } else {
        content
            .phases
            .get(detail.phase as usize)
            .map(Vec::as_slice)
            .ok_or_else(|| {
                format!(
                    "the combat's phase ID {} is beyond the content's {} phases",
                    detail.phase,
                    content.phases.len()
                )
            })
    };
    match phase.and_then(|phase| localize_named(icons, loc, "TACTICS_CURRENT_PHASE", "NAME", phase))
    {
        Ok(label) => text(&mut menu.values, "current_phase", label),
        Err(reason) => withhold(
            &mut menu.values,
            "current_phase",
            &mut unavailable,
            format!("the current phase: {reason}"),
        ),
    }
    let description = match builtin(icons, loc, "TACTICS_CURRENT_PHASE_DESC", &[]) {
        Ok(description) => Some(description),
        Err(reason) => {
            unavailable.insert(format!("the current phase's hover: {reason}"));
            None
        }
    };
    super::tooltips::set_hover(
        &mut menu.values,
        "current_phase",
        description.map(Into::into),
        None,
    );
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    if unlocked.iter().all(Option::is_some) && chances.iter().all(Option::is_some) {
        native_sort(&mut order, |a, b| {
            unlocked[*a] > unlocked[*b]
                || (unlocked[*a] == unlocked[*b] && chances[*a] > chances[*b])
        });
    } else {
        unavailable.insert("selection chances or unlocks are incomplete; candidate order remains matching content order".into());
    }
    let scripted = scripted_text(frame, loc, &static_content.scripted, vision.ours(), nato);
    for index in order {
        let candidate = &candidates[index];
        let token = String::from_utf8_lossy(&candidate.token);
        let Ok(definition) = content.definition(candidate) else {
            continue;
        };
        match listed_for(definition, &context, vision.ours()) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(reason) => {
                unavailable.insert(format!("tactic {token}'s {reason}"));
                continue;
            }
        }
        let mut row = entry_values(
            icons,
            &|name: &str| static_content.assets.has_sprite(name),
            loc,
            &content.phases,
            &token,
            candidate,
            definition,
            chances[index],
            unlocked[index],
            &mut unavailable,
        );
        let main = match unlocked[index] {
            Some(true) => {
                entry_tooltips::main(icons, loc, content, candidate, definition, &mut unavailable)
            }
            Some(false) => (|| {
                let text = scripted.as_ref().map_err(Clone::clone)?;
                let scripted_content = static_content
                    .scripted
                    .as_ref()
                    .as_ref()
                    .map_err(Clone::clone)?;
                let effects = scripted_content.effects()?;
                let effect_context = dashboard_effects::effect_description::Context::new(
                    text,
                    loc,
                    vision.ours(),
                    "common/combat_tactics.txt",
                    effects.view(None),
                );
                let mut refused = BTreeMap::new();
                let sources = dashboard_effects::effect_description::doctrine_tactic_sources(
                    &effect_context,
                    vision.ours(),
                    candidate.tactic_id,
                    |name| content.named_id(name),
                    &mut refused,
                );
                for (why, count) in refused {
                    unavailable.insert(format!(
                        "tactic {token}'s doctrine source hover ({count}): {why}"
                    ));
                }
                entry_tooltips::locked(icons, loc, &sources?)
            })(),
            None => Err("the viewing country's tactic unlock is unavailable".into()),
        };
        let requirements_body = if side_known {
            (|| {
                let text = scripted.as_ref().map_err(Clone::clone)?;
                let evaluator = Evaluator::new(&context.world, Some(&context));
                let mut refused = BTreeMap::new();
                let body = dashboard_text::triggers::description::describe(
                    &definition.blocks("trigger"),
                    &evaluator,
                    text,
                    loc,
                    Scopes::combat(),
                    &content.phases,
                    &|stored| {
                        Ok(if content.named_id(stored)? == 0 {
                            NULL_TACTIC_NAME.to_vec()
                        } else {
                            stored.to_vec()
                        })
                    },
                    &mut refused,
                );
                for (why, count) in refused {
                    unavailable.insert(format!(
                        "tactic {token}'s requirement description hover ({count}): {why}"
                    ));
                }
                body
            })()
        } else {
            Err("the viewing country's combat side is unavailable".into())
        };
        let requirements = entry_tooltips::requirements(
            icons,
            loc,
            &candidate.token,
            requirements_body,
            &mut unavailable,
        );
        let status = if side_known {
            entry_tooltips::status(
                icons,
                loc,
                &context,
                candidate,
                definition,
                &weights,
                index,
                &mut unavailable,
            )
        } else {
            Err("the viewing country's combat side is unavailable".into())
        };
        entry_tooltips::bind(
            &mut row,
            &token,
            main,
            requirements,
            status,
            &mut unavailable,
        );
        menu.entries.push(row);
    }
    for reason in unavailable {
        out.warnings.push(format!(
            "Possible tactics: {reason}; missing values remain unavailable."
        ));
    }
    out.tactics = menu;
    Ok(())
}

fn scripted_text<'a>(
    frame: &'a DecodedFrame,
    loc: &'a Localisation,
    content: &'a Result<dashboard_text::scripted_text::ScriptedContent, String>,
    ours: u32,
    nato: bool,
) -> Result<dashboard_text::scripted_text::ScriptedText<'a>, String> {
    content
        .as_ref()
        .map(|content| {
            dashboard_text::scripted_text::ScriptedText::new(frame, loc, content, ours)
                .viewing(ours)
                .with_nato_counters(nato)
        })
        .map_err(Clone::clone)
}

fn listed_for(definition: &Definition, context: &Context, ours: u32) -> Evaluation<bool> {
    let Some(only_show_for) = field(&definition.items, "only_show_for") else {
        return Ok(true);
    };
    let tag = c_string(only_show_for.value.scalar().unwrap_or_default());
    match (context.country_tags.get(tag), &context.countries) {
        (Some(slot), _) => Ok(*slot == ours),
        (None, Ok(())) => Ok(true),
        (None, Err(why)) => Err(format!(
            "only_show_for = {} names no recorded country, and {why}",
            shown(&only_show_for.value)
        )),
    }
}

#[expect(clippy::too_many_arguments, reason = "the text resources and each recorded fact of one tactic entry")]
fn entry_values(
    icons: &dyn TextIcons,
    has_sprite: &dyn Fn(&str) -> bool,
    loc: &Localisation,
    phases: &[Vec<u8>],
    token: &str,
    candidate: &Candidate,
    definition: &Definition,
    chance: Option<i64>,
    unlocked: Option<bool>,
    unavailable: &mut BTreeSet<String>,
) -> Values {
    let mut row = Values::new();
    hide(&mut row, "button_background");
    hide(&mut row, "command_power_cost");
    if let Some(unlocked) = unlocked {
        row.entry("Background".into()).or_default().frame = Some(if !unlocked {
            2
        } else {
            u32::from(definition.attacker)
        });
        row.entry("locked_icon".into()).or_default().hidden = unlocked;
    } else {
        hide(&mut row, "Background");
        hide(&mut row, "locked_icon");
    }
    match localize_named(icons, loc, "HEADER", "KEY", &candidate.token) {
        Ok(name) => text(&mut row, "tactics_name", name),
        Err(reason) => withhold(
            &mut row,
            "tactics_name",
            unavailable,
            format!("tactic {token}'s name: {reason}"),
        ),
    }
    match &definition.picture {
        Ok(picture) if has_sprite(picture) => sprite(&mut row, "tactics_icon", picture.clone()),
        Ok(picture) => withhold(
            &mut row,
            "tactics_icon",
            unavailable,
            format!("tactic {token}'s picture: the content lacks sprite {picture}"),
        ),
        Err(reason) => withhold(
            &mut row,
            "tactics_icon",
            unavailable,
            format!("tactic {token}'s picture: {reason}"),
        ),
    }
    if chance.is_none() {
        hide(&mut row, "tactics_chance");
    }
    if let Some(chance) = chance {
        let scaled = chance.wrapping_mul(100);
        let rounded = if scaled < 0 {
            scaled.wrapping_sub(50_000) / 100_000
        } else {
            scaled.wrapping_add(50_000) / 100_000
        };
        match builtin(
            icons,
            loc,
            "TACTIC_SELECTION_CHANCE",
            &[("CHANCE", Argument::Int32(rounded as i32))],
        ) {
            Ok(label) => text(&mut row, "tactics_chance", label),
            Err(reason) => withhold(
                &mut row,
                "tactics_chance",
                unavailable,
                format!("tactic {token}'s chance: {reason}"),
            ),
        }
    }
    let shown = [
        (definition.display_phase, "PHASE_DISPLAY"),
        (definition.phase, "PHASE_START"),
    ]
    .into_iter()
    .find(|(id, _)| *id >= 0);
    match shown {
        None => hide(&mut row, "phase"),
        Some((id, key)) => {
            let label = usize::try_from(id)
                .ok()
                .and_then(|index| phases.get(index))
                .ok_or_else(|| {
                    format!(
                        "phase ID {id} is beyond the content's {} phases",
                        phases.len()
                    )
                })
                .and_then(|phase| localize_named(icons, loc, key, "NAME", phase));
            match label {
                Ok(label) => text(&mut row, "phase", label),
                Err(reason) => withhold(
                    &mut row,
                    "phase",
                    unavailable,
                    format!("tactic {token}'s phase: {reason}"),
                ),
            }
        }
    }
    for (key, source, favorable) in [
        (
            "attacker_damage_value",
            "attacker",
            Some(definition.attacker),
        ),
        (
            "defender_damage_value",
            "defender",
            Some(!definition.attacker),
        ),
        ("combat_width_change_value", "combat_width", None),
        (
            "attacker_speed_value",
            "attacker_movement_speed",
            Some(definition.attacker),
        ),
        (
            "attacker_org_modifier_value",
            "attacker_org_damage_modifier",
            Some(definition.attacker),
        ),
        (
            "defender_org_modifier_value",
            "defender_org_damage_modifier",
            Some(definition.attacker),
        ),
    ] {
        let format = match favorable {
            None => "TACTIC_LIST_VALUE_NEUTRAL",
            Some(true) => "TACTIC_LIST_VALUE_GOOD",
            Some(false) => "TACTIC_LIST_VALUE_BAD",
        };
        let slot =
            effect_slot(source.as_bytes()).expect("every popup value is one of the six effects");
        let raw = definition.effects[slot];
        match builtin(icons, loc, format, &[("VAL", Argument::Fixed5(raw))]) {
            Ok(label) => text(&mut row, key, label),
            Err(reason) => withhold(
                &mut row,
                key,
                unavailable,
                format!("tactic {token}'s {source}: {reason}"),
            ),
        }
    }
    row
}

fn preferred_description(
    values: &mut Values,
    popup: &content_reader_gui::gui::native::Element,
    icons: &dyn TextIcons,
    loc: &Localisation,
    unavailable: &mut BTreeSet<String>,
) {
    let name = "preferred_tactic_desc_text";
    let Some(element) = popup.path_named(name) else {
        return;
    };
    let Some(authored) = element.text_box() else {
        return;
    };
    let label = if authored.context_aware && !authored.bound_text.nodes.is_empty() {
        Err("uses scoped bound_text, whose battle-popup localizer is unavailable".to_owned())
    } else {
        token(icons, loc, &authored.text)
    };
    match label {
        Ok(label) => {
            text(values, name, label);
            values.get_mut(name).expect("bound above").hidden = element.hide;
        }
        Err(reason) => withhold(
            values,
            name,
            unavailable,
            format!("the preferred tactic explanation: {reason}"),
        ),
    }
}

fn withhold(values: &mut Values, key: &str, unavailable: &mut BTreeSet<String>, reason: String) {
    let widget = values.entry(key.into()).or_default();
    widget.text = None;
    widget.sprite = None;
    widget.hidden = true;
    unavailable.insert(reason);
}

fn native_sort<T: Clone, F: Fn(&T, &T) -> bool>(values: &mut [T], less: F) {
    let len = values.len();
    sort_range(values, 0, len, len, &less);
}

fn sort_range<T: Clone, F: Fn(&T, &T) -> bool>(
    values: &mut [T],
    mut first: usize,
    mut last: usize,
    mut ideal: usize,
    less: &F,
) {
    loop {
        if last - first <= 32 {
            insertion_sort(&mut values[first..last], less);
            return;
        }
        if ideal == 0 {
            heap_sort(&mut values[first..last], less);
            return;
        }
        let (pivot_first, pivot_last) = partition(values, first, last, less);
        ideal = (ideal >> 1) + (ideal >> 2);
        if pivot_first - first < last - pivot_last {
            sort_range(values, first, pivot_first, ideal, less);
            first = pivot_last;
        } else {
            sort_range(values, pivot_last, last, ideal, less);
            last = pivot_first;
        }
    }
}

fn insertion_sort<T: Clone, F: Fn(&T, &T) -> bool>(values: &mut [T], less: &F) {
    for next in 1..values.len() {
        let value = values[next].clone();
        let mut hole = next;
        if less(&value, &values[0]) {
            while hole > 0 {
                values[hole] = values[hole - 1].clone();
                hole -= 1;
            }
        } else {
            while less(&value, &values[hole - 1]) {
                values[hole] = values[hole - 1].clone();
                hole -= 1;
            }
        }
        values[hole] = value;
    }
}

fn median_three<T, F: Fn(&T, &T) -> bool>(
    values: &mut [T],
    first: usize,
    middle: usize,
    last: usize,
    less: &F,
) {
    if less(&values[middle], &values[first]) {
        values.swap(middle, first);
    }
    if less(&values[last], &values[middle]) {
        values.swap(last, middle);
        if less(&values[middle], &values[first]) {
            values.swap(middle, first);
        }
    }
}

fn partition<T, F: Fn(&T, &T) -> bool>(
    values: &mut [T],
    first: usize,
    last: usize,
    less: &F,
) -> (usize, usize) {
    let middle = first + (last - first) / 2;
    let back = last - 1;
    if back - first > 40 {
        let step = (back - first + 1) / 8;
        median_three(values, first, first + step, first + 2 * step, less);
        median_three(values, middle - step, middle, middle + step, less);
        median_three(values, back - 2 * step, back - step, back, less);
        median_three(values, first + step, middle, back - step, less);
    } else {
        median_three(values, first, middle, back, less);
    }

    let mut pivot_first = middle;
    let mut pivot_last = middle + 1;
    while first < pivot_first
        && !less(&values[pivot_first - 1], &values[pivot_first])
        && !less(&values[pivot_first], &values[pivot_first - 1])
    {
        pivot_first -= 1;
    }
    while pivot_last < last
        && !less(&values[pivot_last], &values[pivot_first])
        && !less(&values[pivot_first], &values[pivot_last])
    {
        pivot_last += 1;
    }

    let mut scan_first = pivot_last;
    let mut scan_last = pivot_first;
    loop {
        while scan_first < last {
            if less(&values[pivot_first], &values[scan_first]) {
            } else if less(&values[scan_first], &values[pivot_first]) {
                break;
            } else {
                if pivot_last != scan_first {
                    values.swap(pivot_last, scan_first);
                }
                pivot_last += 1;
            }
            scan_first += 1;
        }
        while first < scan_last {
            let previous = scan_last - 1;
            if less(&values[previous], &values[pivot_first]) {
            } else if less(&values[pivot_first], &values[previous]) {
                break;
            } else {
                pivot_first -= 1;
                if pivot_first != previous {
                    values.swap(pivot_first, previous);
                }
            }
            scan_last -= 1;
        }
        if scan_last == first && scan_first == last {
            return (pivot_first, pivot_last);
        }
        if scan_last == first {
            if pivot_last != scan_first {
                values.swap(pivot_first, pivot_last);
            }
            pivot_last += 1;
            values.swap(pivot_first, scan_first);
            pivot_first += 1;
            scan_first += 1;
        } else if scan_first == last {
            scan_last -= 1;
            pivot_first -= 1;
            if scan_last != pivot_first {
                values.swap(scan_last, pivot_first);
            }
            pivot_last -= 1;
            values.swap(pivot_first, pivot_last);
        } else {
            scan_last -= 1;
            values.swap(scan_first, scan_last);
            scan_first += 1;
        }
    }
}

fn heap_sort<T: Clone, F: Fn(&T, &T) -> bool>(values: &mut [T], less: &F) {
    let len = values.len();
    for root in (0..len / 2).rev() {
        let value = values[root].clone();
        repair_heap(values, root, len, value, less);
    }
    for end in (1..len).rev() {
        let value = values[end].clone();
        values[end] = values[0].clone();
        repair_heap(values, 0, end, value, less);
    }
}

fn repair_heap<T: Clone, F: Fn(&T, &T) -> bool>(
    values: &mut [T],
    top: usize,
    len: usize,
    value: T,
    less: &F,
) {
    let last_parent = (len - 1) / 2;
    let mut hole = top;
    while hole < last_parent {
        let mut child = 2 * hole + 2;
        if less(&values[child], &values[child - 1]) {
            child -= 1;
        }
        values[hole] = values[child].clone();
        hole = child;
    }
    if hole == last_parent && len.is_multiple_of(2) {
        values[hole] = values[len - 1].clone();
        hole = len - 1;
    }
    while top < hole {
        let parent = (hole - 1) / 2;
        if !less(&values[parent], &value) {
            break;
        }
        values[hole] = values[parent].clone();
        hole = parent;
    }
    values[hole] = value;
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_text::triggers::ContentFacts;
    use content_reader_script::key_names::KeyNames;
    use content_reader_parser::fixed5::{decimal_fixed5, script_fixed5};
    use replay_data::{
        rows::BuildingRow, rows::CountryResourceRow, rows::PoliticalPartyRow, rows::ProvinceRow,
        rows::ResourceDefinitionRow, rows::StateRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};

    #[test]
    fn tactic_scripted_caller_keeps_viewer_nato_through_nested_effect_unit_icons() {
        content_reader_script::test_game::enter_viewer();
        let root = std::env::temp_dir().join(format!(
            "hoi4_viewer_tactic_nato_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        for (relative, contents) in [
            (
                "common/defines/test.lua",
                "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=23}}",
            ),
            ("common/units/test.txt", "sub_units={syn_unit={}}"),
            (
                "common/bop/test.txt",
                "balance={range={id=active on_activate={add_unit_bonus={syn_unit={}}}}}",
            ),
            (
                "common/scripted_localisation/test.txt",
                "entry={name=IconEntry text={localization_key=ICON_BODY}}",
            ),
        ] {
            let path = root.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let content = Ok(dashboard_base::catalogue::read_scripted_content(
            Arc::new(content_reader_layers::content::ResolvedContent {
                layers,
                stats: Default::default(),
                changed: Vec::new(),
                notes: Vec::new(),
            }),
            String::new(),
        )
        .unwrap());
        let mut frame = DecodedFrame {
            branch: 0,
            hour: 10,
            tables: std::array::from_fn(|at| {
                TableView::Complete(TableRows::empty(TableId::ALL[at])).into()
            }),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(vec![replay_data::rows::CountryRow {
                id: 10,
                canonical_slot: 2,
                tag: b"SYN".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                replay_data::rows::CountryLinkRow {
                    id: 10,
                    canonical_slot: 2,
                },
            ]));
        *frame.tables[TableId::PowerBalances.index()] =
            TableView::Complete(TableRows::PowerBalances(vec![
                replay_data::rows::PowerBalanceRow {
                    balance: b"balance".to_vec(),
                    active_range: b"active".to_vec(),
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::PowerBalanceMembers.index()] =
            TableView::Complete(TableRows::PowerBalanceMembers(vec![
                replay_data::rows::PowerBalanceMemberRow {
                    country: 10,
                    ..Default::default()
                },
            ]));
        let loc: Localisation = [
            ("ICON_BODY", "[ROOT.GetActiveRangeActivationEffect]"),
            ("PROGRESS_SECTION_ACTIVATION_EFFECT", "Activate:\n$DESC$"),
            ("SUBUNIT_BONUS_HEADER", "$ICON$$NAME$"),
            ("syn_unit", "Unit"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
        for nato in [false, true, false] {
            let text = scripted_text(&frame, &loc, &content, 2, nato).unwrap();
            let icon_frame = if nato { 2 } else { 1 };
            assert_eq!(
                text.expand("[IconEntry]"),
                Ok(format!(
                    "Activate:\n\u{13}GFX_syn_unit_icon_small|{icon_frame} Unit\n"
                ))
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    fn no_effects() -> [i64; 6] {
        [0; 6]
    }

    fn plain(active: bool, attacker: bool) -> Definition {
        Definition {
            active,
            attacker,
            items: Vec::new(),
            phase: content_reader_military::combat_tactics::UNKNOWN_PHASE,
            display_phase: content_reader_military::combat_tactics::UNKNOWN_PHASE,
            effects: no_effects(),
            picture: Err("tactic t has no picture field".into()),
        }
    }

    fn trigger(text: &str) -> Vec<Item> {
        let database = content_reader_military::combat_tactics::parse(
            format!("t = {{ trigger = {{ {text} }} }}").as_bytes(),
            &names(&ScriptedTriggers::default()),
        );
        let tactic = &database.tactics[0];
        assert!(tactic.unproven.is_none(), "{:?}", tactic.unproven);
        tactic
            .fields
            .iter()
            .filter_map(|field| field.value.items())
            .flatten()
            .cloned()
            .collect()
    }

    fn content() -> Content {
        Content {
            phases: vec![b"close_combat".to_vec()],
            ..Content::default()
        }
    }

    fn frame(tables: Vec<(TableId, TableRows)>) -> &'static DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "not sampled".into(),
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        for (id, rows) in tables {
            *frame.tables[id.index()] = TableView::Complete(rows);
        }
        Box::leak(Box::new(frame))
    }

    fn facts(content: &Content) -> ContentFacts<'_> {
        ContentFacts {
            state_provinces: Box::leak(Box::default()),
            state_content: Ok(()),
            continents: Err("the test has no continent content"),
            ideologies: Err("the test has no independent ideology database"),
            impassable: Box::leak(Box::default()),
            neighbor_states: Err("the test has no state neighbours"),
            terrain: None,
            dlcs: Box::leak(Box::default()),
            triggers: Ok(&content.names),
            character_traits: content
                .character_traits()
                .or(Ok((&content.leader_traits, Box::leak(Box::default())))),
            tag_aliases: Box::leak(Box::default()),
            buildings: Ok(Box::leak(Box::default())),
            resource_bonuses: Err("the test has no resource bonuses"),
            leader_roles: Ok(Box::leak(Box::new(
                dashboard_text::triggers::LeaderRoles::new(
                    [
                        "GER_leader",
                        "GER_politician",
                        "GER_scientist",
                        "GER_returned",
                        "GER_operative",
                        "GER_lost_agent",
                        "GER_miscounted",
                    ]
                    .map(|tag| content_reader_key_names::characters::LeaderTemplate {
                        path: "common/characters/GER.txt".into(),
                        tag: tag.into(),
                        line: None,
                        role: Ok(None),
                    })
                    .into(),
                ),
            ))),
            surrender_limits: Err("the test has no surrender defines"),
            invasion_prepare_days: Err("the test has no invasion define"),
            script_constants: Err("the test has no script constants"),
        }
    }

    pub(super) fn context<'a>(
        detail: &'a LandCombatDetailsRow,
        content: &'a Content,
        aliases: &'a BTreeMap<u32, u32>,
    ) -> Context<'a> {
        context_in(detail, content, aliases, frame(Vec::new()))
    }

    fn context_in<'a>(
        detail: &'a LandCombatDetailsRow,
        content: &'a Content,
        aliases: &'a BTreeMap<u32, u32>,
        frame: &'a DecodedFrame,
    ) -> Context<'a> {
        context_with(detail, content, aliases, frame, facts(content))
    }

    fn context_with<'a>(
        detail: &'a LandCombatDetailsRow,
        content: &'a Content,
        aliases: &'a BTreeMap<u32, u32>,
        frame: &'a DecodedFrame,
        facts: ContentFacts<'a>,
    ) -> Context<'a> {
        Context {
            detail,
            marker_flags: None,
            candidates: &[],
            content,
            preference_factors: Ok([0, 50_000, 25_000]),
            terrain: Err("the test has no terrain".into()),
            terrains: Vec::new(),
            aliases,
            links: Ok(()),
            countries: Ok(()),
            first_countries: [None, None],
            country_unlocks: BTreeMap::new(),
            side: 0,
            country_tags: BTreeMap::new(),
            divisions: BTreeMap::new(),
            values: BTreeMap::new(),
            movement_sources: BTreeMap::new(),
            participants: &[],
            tokens: BTreeMap::new(),
            tokens_complete: Ok(()),
            province_vp: Some(5),
            world: World::read(frame, facts),
        }
    }

    #[test]
    fn authored_registry_ids_join_unique_casefolded_native_unlock_tokens() {
        let mut content = content();
        content.registry = vec![
            RegistryEntry {
                id: 1,
                token: b"Tactic_Foo".to_vec(),
                definition: Ok(plain(true, true)),
            },
            RegistryEntry {
                id: 2,
                token: b"Tactic_Foo".to_vec(),
                definition: Ok(plain(false, false)),
            },
        ];
        let lexical = [LexicalTokenRow {
            token: 42,
            literal: b"tactic_foo".to_vec(),
        }];
        let derived = candidates(&content, &lexical).unwrap();
        assert_eq!(
            derived.iter().map(|row| row.tactic_id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(derived.iter().all(|row| row.unlock_id == 42));
        assert!(content.definition(&derived[0]).unwrap().active);
        assert!(!content.definition(&derived[1]).unwrap().active);

        assert!(candidates(&content, &[]).is_err());
        assert!(
            candidates(
                &content,
                &[LexicalTokenRow {
                    token: 0,
                    ..lexical[0].clone()
                }]
            )
            .is_err()
        );
        assert!(
            candidates(
                &content,
                &[
                    lexical[0].clone(),
                    LexicalTokenRow {
                        token: 43,
                        literal: b"TACTIC_FOO".to_vec()
                    }
                ]
            )
            .is_err()
        );
    }

    fn names(scripted: &ScriptedTriggers) -> KeyNames {
        content_reader_script::test_game::enter_viewer();
        KeyNames {
            scripted: scripted.templates.keys().cloned().collect(),
            countries: [&b"GER"[..], b"SOV", b"ENG"].map(<[u8]>::to_vec).into(),
            characters: [
                "ger_leader",
                "ger_lost_agent",
                "ger_miscounted",
                "ger_operative",
                "ger_politician",
                "ger_returned",
                "ger_scientist",
            ]
            .map(|token| token.as_bytes().to_vec())
            .into(),
            ..KeyNames::default()
        }
    }

    pub(super) fn parsed(text: &[u8]) -> Content {
        parsed_with(text, &names(&ScriptedTriggers::default()))
    }

    fn parsed_with(text: &[u8], names: &KeyNames) -> Content {
        content_reader_script::test_game::enter_viewer();
        Content::from_database(content_reader_military::combat_tactics::parse(text, names))
    }

    #[test]
    fn tactic_yes_no_fields_match_their_exact_text() {
        let content = parsed(
            b"a = { other = yes }\n\
              b = { ACTIVE = no\x00x IS_ATTACKER = yes }\n\
              c = { active = NO is_attacker = YES }\n\
              d = { active = no active = maybe }\n\
              e = { active = { condition = yes } }\n\
              f = { }\n",
        );
        let fields: Vec<_> = content
            .registry
            .iter()
            .map(|entry| {
                entry
                    .definition
                    .as_ref()
                    .map(|definition| (definition.active, definition.attacker))
                    .map_err(String::as_str)
            })
            .collect();
        assert_eq!(
            fields,
            [
                Ok((true, false)),
                Ok((false, true)),
                Ok((true, false)),
                Ok((false, false)),
                Ok((true, false)),
                Ok((true, false)),
            ]
        );
        assert_eq!(
            content.warnings[4..],
            [
                "Combat tactics: tactic c's active = NO is not exactly yes or no; the game logs an error and keeps yes.",
                "Combat tactics: tactic c's is_attacker = YES is not exactly yes or no; the game logs an error and keeps no.",
                "Combat tactics: tactic d's active = maybe is not exactly yes or no; the game logs an error and keeps no.",
                "Combat tactics: tactic e's active = { is not exactly yes or no; the game logs an error and keeps yes.",
            ]
        );
        assert!(
            content.warnings[0]
                .contains("field `active` is a block where its reader takes one value"),
            "{:?}",
            content.warnings
        );
    }

    #[test]
    fn unlocks_follow_any_viewing_country_and_distinguish_empty_from_missing() {
        let detail = LandCombatDetailsRow::default();
        let content = content();
        let aliases = BTreeMap::from([(10, 1), (20, 2), (30, 3)]);
        let mut context = context(&detail, &content, &aliases);
        let registry = [(1, 9), (2, 42), (3, 42)].map(|(tactic_id, unlock_id)| Candidate {
            tactic_id,
            unlock_id,
            token: Vec::new(),
        });
        context.candidates = &registry;
        let empty: [u32; 0] = [];
        let third_country_unlocks = [1, 3, 1];
        let unknown = [1, 4];
        context.country_unlocks = BTreeMap::from([
            (1, empty.as_slice()),
            (3, third_country_unlocks.as_slice()),
            (4, unknown.as_slice()),
        ]);
        let candidate = Candidate {
            tactic_id: 2,
            unlock_id: 42,
            token: b"tactic_test".to_vec(),
        };
        let mut definition = plain(false, false);
        assert_eq!(context.unlocked(&candidate, &definition, 3), Ok(true));
        assert_eq!(context.unlocked(&candidate, &definition, 1), Ok(false));
        assert!(context.unlocked(&candidate, &definition, 2).is_err());
        assert!(context.unlocked(&candidate, &definition, 4).is_err());
        let null_only = [0];
        context.country_unlocks.insert(5, null_only.as_slice());
        assert_eq!(context.unlocked(&candidate, &definition, 5), Ok(false));
        context.tokens = BTreeMap::from([(42, b"NullCombatTactic".as_slice())]);
        assert_eq!(context.unlocked(&candidate, &definition, 5), Ok(true));
        definition.active = true;
        assert_eq!(context.unlocked(&candidate, &definition, 2), Ok(true));
    }

    #[test]
    fn unknown_denominator_never_becomes_a_displayed_positive_chance() {
        assert_eq!(
            normalized_chances(&[Some(400_000), None, Some(0)]),
            [None, None, Some(0)]
        );
        assert_eq!(
            normalized_chances(&[Some(400_000), Some(400_000), Some(0)]),
            [Some(50_000), Some(50_000), Some(0)]
        );
        assert_eq!(normalized_chances(&[Some(0), Some(0)]), [Some(0), Some(0)]);
    }

    #[test]
    fn missing_attacker_set_member_cannot_be_treated_as_defender_perspective() {
        let aliases = BTreeMap::from([(10, 1), (20, 2)]);
        assert_eq!(viewing_side(&[10], "attacker", &aliases, 1), Ok(0));
        assert_eq!(viewing_side(&[10], "attacker", &aliases, 2), Ok(1));
        assert_eq!(viewing_side(&[10, 20], "attacker", &aliases, 2), Ok(0));
        assert_eq!(viewing_side(&[99, 10], "attacker", &aliases, 1), Ok(0));
        assert_eq!(viewing_side(&[10, 99], "attacker", &aliases, 1), Ok(0));
        assert!(viewing_side(&[99, 20], "attacker", &aliases, 1).is_err());
        assert!(viewing_side(&[20, 99], "attacker", &aliases, 1).is_err());
        assert_eq!(viewing_side(&[], "attacker", &aliases, 2), Ok(1));
    }

    #[test]
    fn integer_and_fixed_trigger_equality_are_different_native_branches() {
        assert!(compare(200_000, 200_000, Operator::Equals, false));
        assert!(!compare(200_000, 200_000, Operator::Equals, true));
        assert!(compare(200_001, 200_000, Operator::Equals, true));
        assert!(!compare(200_000, 200_000, Operator::Greater, false));
    }

    #[test]
    fn equivalent_tactics_keep_native_order_before_and_after_the_partition_threshold() {
        for count in [0, 1, 31, 32, 33, 41, 42, 54, 128] {
            let mut values: Vec<_> = (0..count).collect();
            native_sort(&mut values, |_, _| false);
            assert_eq!(values, (0..count).collect::<Vec<_>>());
        }
        for count in [31, 32, 33, 41, 42, 54, 128] {
            let mut values: Vec<_> = (0..count).map(|index| (index * 13 % 7, index)).collect();
            native_sort(&mut values, |a, b| a.0 > b.0);
            assert!(values.windows(2).all(|pair| pair[0].0 >= pair[1].0));
            let mut identities: Vec<_> = values.iter().map(|row| row.1).collect();
            identities.sort_unstable();
            assert_eq!(identities, (0..count).collect::<Vec<_>>());
        }
    }

    #[test]
    fn popup_preferred_description_retains_authored_text_without_selector_arguments() {
        content_reader_script::test_game::enter_viewer();
        let gui = content_reader_gui::gui::native::read(
            br#"guiTypes = { containerWindowType = {
                name = tactics_list_view
                instantTextboxType = {
                    name = preferred_tactic_desc_text text = MOD_PREFERRED_DESC
                    position = { x = 20 y = 66 } maxWidth = 340 maxHeight = 20
                    alwaystransparent = yes
                }
            } }"#,
        );
        let popup = gui.template(b"tactics_list_view").unwrap();
        let loc: Localisation = [(
            "MOD_PREFERRED_DESC".to_owned(),
            "Preferred fixture: $CHANCE|H$ chance marker.".to_owned(),
        )]
        .into_iter()
        .collect();
        let mut values = Values::new();
        let mut unavailable = BTreeSet::new();
        text(&mut values, "current_phase", "Known phase");
        preferred_description(&mut values, popup, &NoIcons, &loc, &mut unavailable);
        let explanation = &values["preferred_tactic_desc_text"];
        assert!(!explanation.hidden);
        assert_eq!(
            explanation.text.as_deref(),
            Some("Preferred fixture:  chance marker.")
        );
        assert!(unavailable.is_empty(), "{unavailable:?}");

        let refused: Localisation = [("MOD_PREFERRED_DESC".to_owned(), "\u{14}".to_owned())]
            .into_iter()
            .collect();
        preferred_description(&mut values, popup, &NoIcons, &refused, &mut unavailable);
        let explanation = &values["preferred_tactic_desc_text"];
        assert!(explanation.hidden && explanation.text.is_none());
        assert_eq!(values["current_phase"].text.as_deref(), Some("Known phase"));
        assert_eq!(
            unavailable,
            BTreeSet::from([
                "the preferred tactic explanation: localisation MOD_PREFERRED_DESC holds the control byte 0x14 (an authored `¤`), which the viewer does not draw".to_owned()
            ])
        );

        let scoped = content_reader_gui::gui::native::read(
            br#"guiTypes = { containerWindowType = {
                name = tactics_list_view instantTextboxType = {
                    name = preferred_tactic_desc_text text = MOD_PREFERRED_DESC
                    context_aware_text = { localization_key = MOD_SCOPED_DESC }
                }
            } }"#,
        );
        unavailable.clear();
        preferred_description(
            &mut values,
            scoped.template(b"tactics_list_view").unwrap(),
            &NoIcons,
            &loc,
            &mut unavailable,
        );
        assert!(values["preferred_tactic_desc_text"].text.is_none());
        assert_eq!(
            unavailable,
            BTreeSet::from([
                "the preferred tactic explanation: uses scoped bound_text, whose battle-popup localizer is unavailable".to_owned()
            ])
        );
    }

    #[test]
    fn popup_phase_follows_native_resolution() {
        let loc: content_reader_localisation::localisation::Localisation = [
            ("opening", "Opening"),
            ("PHASE_DISPLAY", "Shown in $NAME$"),
            ("PHASE_START", "Starts in $NAME$"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let phases = [b"opening".to_vec()];
        use content_reader_military::combat_tactics::{NO_PHASE, UNKNOWN_PHASE};
        let run = |(display_phase, phase): (i32, i32)| {
            let definition = Definition {
                phase,
                display_phase,
                ..plain(true, true)
            };
            entry_values(
                &NoIcons,
                &|_: &str| true,
                &loc,
                &phases,
                "same_name",
                &Candidate::default(),
                &definition,
                None,
                None,
                &mut BTreeSet::new(),
            )
        };
        let text = |row: &Values| row.get("phase").and_then(|v| v.text.clone());
        let hidden = |row: &Values| row.get("phase").is_some_and(|v| v.hidden);
        let shown = run((0, UNKNOWN_PHASE));
        assert_eq!(text(&shown).as_deref(), Some("Shown in Opening"));
        let fallback = run((UNKNOWN_PHASE, 0));
        assert_eq!(text(&fallback).as_deref(), Some("Starts in Opening"));
        for ids in [
            (UNKNOWN_PHASE, UNKNOWN_PHASE),
            (NO_PHASE, UNKNOWN_PHASE),
            (UNKNOWN_PHASE, NO_PHASE),
        ] {
            let row = run(ids);
            assert!(hidden(&row) && text(&row).is_none(), "{ids:?}");
        }
    }

    #[test]
    fn pictures_share_the_registrys_identity_and_refusals() {
        let content = parsed(
            b"same = { picture = first }\n\
              same = { PICTURE = second \"picture\" = quoted }\n\
              t = { picture = first active = { picture = second } picture = third }\n\
              u = { }",
        );
        assert_eq!(content.picture(1, b"same"), Ok("combat_tactic:first"));
        assert_eq!(content.picture(2, b"same"), Ok("combat_tactic:second"));
        assert_eq!(
            content.picture(2, b"t"),
            Err("recorded tactic id 2 is t, but the content's tactic 2 is same".into())
        );
        assert_eq!(content.picture(3, b"t"), Ok("combat_tactic:second"));
        assert_eq!(
            content.picture(4, b"u"),
            Err("recorded tactic id 4 is u, but the content's tactic 4 is picture".into())
        );
        assert_eq!(
            content.pictures().collect::<Vec<_>>(),
            [
                (
                    "combat_tactic:first",
                    "gfx/interface/landcombat/tactics/first.dds".to_owned()
                ),
                (
                    "combat_tactic:second",
                    "gfx/interface/landcombat/tactics/second.dds".to_owned()
                ),
                (
                    "combat_tactic:second",
                    "gfx/interface/landcombat/tactics/second.dds".to_owned()
                ),
            ]
        );
        let mut unknown = names(&ScriptedTriggers::default());
        unknown.counts.ideologies_gap =
            Some("common/ideologies/a.txt is unknown from line 1".into());
        let lost = parsed_with(
            b"none = { }\n\
              bytes = { picture = \xe9 }\n\
              lost = { picture = here trigger = { OR = { x = { } } } }\n\
              after = { picture = there }\n",
            &unknown,
        );
        assert_eq!(
            lost.picture(1, b"none"),
            Err("tactic none has no picture field".into())
        );
        assert!(
            lost.picture(2, b"bytes").unwrap_err().contains(
                "is not UTF-8 before its first NUL, so its texture path cannot be formed"
            )
        );
        assert_eq!(
            parsed(b"nul = { picture = abc\x00\xe9 }").picture(1, b"nul"),
            Ok("combat_tactic:abc")
        );
        let unproven = lost.picture(3, b"lost").unwrap_err();
        assert!(
            unproven
                .starts_with("tactic lost's ID or fields cannot be read as the game reads them")
                && unproven.contains("field `x`"),
            "{unproven}"
        );
        assert!(
            lost.picture(4, b"after").unwrap_err().starts_with(
                "recorded tactic after's id 4 is past the 3 combat tactics read before"
            ),
        );
        assert_eq!(lost.pictures().count(), 0);
        let definition = content.registry[1].definition.as_ref().unwrap();
        let mut unavailable = BTreeSet::new();
        let row = entry_values(
            &NoIcons,
            &|_: &str| true,
            &Localisation::default(),
            &[],
            "same",
            &Candidate::default(),
            definition,
            None,
            None,
            &mut unavailable,
        );
        assert_eq!(
            row["tactics_icon"].sprite.as_deref(),
            Some("combat_tactic:second")
        );
        assert!(!row["tactics_icon"].hidden && row["tactics_chance"].hidden);
        let mut unavailable = BTreeSet::new();
        let row = entry_values(
            &NoIcons,
            &|name: &str| name != "combat_tactic:second",
            &Localisation::default(),
            &[],
            "same",
            &Candidate::default(),
            definition,
            None,
            None,
            &mut unavailable,
        );
        assert!(row["tactics_icon"].hidden && row["tactics_icon"].sprite.is_none());
        assert!(
            unavailable
                .contains("tactic same's picture: the content lacks sprite combat_tactic:second"),
            "{unavailable:?}"
        );
    }

    #[test]
    fn numeric_fields_use_their_native_readers() {
        assert_eq!(script_fixed5(b"010"), Some(800_000));
        assert_eq!(decimal_fixed5(b"010"), 1_000_000);
        let loc: content_reader_localisation::localisation::Localisation = [
            ("TACTIC_LIST_VALUE_GOOD", "$VAL$"),
            ("TACTIC_LIST_VALUE_BAD", "$VAL$"),
            ("TACTIC_LIST_VALUE_NEUTRAL", "$VAL$"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let content = parsed(b"t = { attacker_movement_speed=010 attacker=010 }");
        let definition = content.registry[0].definition.as_ref().unwrap();
        let candidate = Candidate::default();
        let row = entry_values(
            &NoIcons,
            &|_: &str| true,
            &loc,
            &[],
            "t",
            &candidate,
            definition,
            None,
            None,
            &mut BTreeSet::new(),
        );
        assert_eq!(row["attacker_speed_value"].text.as_deref(), Some("8.00"));
        assert_eq!(row["attacker_damage_value"].text.as_deref(), Some("10.00"));
    }

    #[test]
    fn popup_entry_refusals_name_the_tactic_and_key() {
        let loc: content_reader_localisation::localisation::Localisation = [
            ("tactic_a", "Tactic A"),
            ("HEADER", "$KEY$"),
            ("TACTIC_LIST_VALUE_GOOD", "+$VAL$"),
            ("TACTIC_LIST_VALUE_BAD", "$VAL$"),
            ("TACTIC_LIST_VALUE_NEUTRAL", "$VAL$"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let content = parsed(b"tactic_a = { attacker = 1.5e3 combat_width = 0.1 }");
        let definition = content.registry[0].definition.as_ref().unwrap();
        let candidate = Candidate {
            tactic_id: 1,
            unlock_id: 0,
            token: b"tactic_a".to_vec(),
        };
        let mut unavailable = BTreeSet::new();
        let row = entry_values(
            &NoIcons,
            &|_: &str| true,
            &loc,
            &[b"opening".to_vec()],
            "tactic_a",
            &candidate,
            definition,
            Some(50_000),
            Some(true),
            &mut unavailable,
        );
        assert!(row["tactics_name"].text.is_some());
        assert!(row["combat_width_change_value"].text.is_some());
        assert!(
            row["tactics_chance"]
                .text
                .as_deref()
                .is_some_and(|text| text.contains("TACTIC_SELECTION_CHANCE"))
        );
        let icon = &row["tactics_icon"];
        assert!(icon.hidden && icon.sprite.is_none() && icon.text.is_none());
        assert_eq!(
            unavailable.into_iter().collect::<Vec<_>>(),
            ["tactic tactic_a's picture: tactic tactic_a has no picture field",]
        );
    }

    #[test]
    fn tactic_names_are_bounded_in_native_bytes_before_header() {
        let fill = "a".repeat(0x17FF);
        let loc: Localisation = [
            ("HEADER", "$KEY$".to_owned()),
            ("fits", format!("{fill}@@")),
            ("cut", format!("{fill}é")),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let definition = plain(true, true);
        let name = |token: &str| {
            let mut unavailable = BTreeSet::new();
            let candidate = Candidate {
                token: token.as_bytes().to_vec(),
                ..Candidate::default()
            };
            let row = entry_values(
                &NoIcons,
                &|_: &str| true,
                &loc,
                &[],
                token,
                &candidate,
                &definition,
                None,
                None,
                &mut unavailable,
            );
            let named: Vec<_> = unavailable
                .into_iter()
                .filter(|reason| reason.contains("'s name"))
                .collect();
            (row.get("tactics_name").and_then(|v| v.text.clone()), named)
        };
        let (fits, refused) = name("fits");
        assert_eq!(fits, Some(format!("{fill}@")));
        assert!(refused.is_empty(), "{refused:?}");
        let (cut, refused) = name("cut");
        assert_eq!(cut, None);
        assert_eq!(
            refused,
            [
                "tactic cut's name: localisation cut: the game's 0x1800-byte buffer ends inside a multi-byte character"
            ]
        );
    }

    #[test]
    fn selection_chance_keeps_the_low_32_bits_of_the_rounded_percentage() {
        let loc: Localisation = [("TACTIC_SELECTION_CHANCE", "$CHANCE$%")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let definition = plain(true, true);
        let chance = |chance: i64| {
            let row = entry_values(
                &NoIcons,
                &|_: &str| true,
                &loc,
                &[],
                "t",
                &Candidate::default(),
                &definition,
                Some(chance),
                None,
                &mut BTreeSet::new(),
            );
            row["tactics_chance"].text.clone().unwrap()
        };
        assert_eq!(chance(12_345), "12%");
        assert_eq!(chance(-12_500), "-13%");
        assert_eq!(chance(0xffff_ffff), "4294967%");
        assert_eq!(chance(2_147_483_648_000), "-2147483648%");
    }

    #[test]
    fn mixed_case_keywords_parse_and_evaluate_like_lowercase() {
        let body = |name: &str, case: fn(&str) -> String| {
            format!(
                "{} = {{ {} = {{ {} = opening {} = yes {} = {{ {} = yes {} > 2 }} \
                 {} = {{ {} = later }} {} = {{ {} = LABEL {} = {} }} }} \
                 {} = {{ {} = 4 {} = {{ {} = 1.5 {} = 1 {} > 2 }} {} = {{ {} = 0 {} = no }} }} }}\n",
                name,
                case("trigger"),
                case("phase"),
                case("is_attacker"),
                case("or"),
                case("has_reserves"),
                case("skill"),
                case("not"),
                case("phase"),
                case("custom_trigger_tooltip"),
                case("tooltip"),
                case("has_combat_modifier"),
                case("river_crossing"),
                case("base"),
                case("factor"),
                case("modifier"),
                case("factor"),
                case("add"),
                case("skill"),
                case("modifier"),
                case("factor"),
                case("is_attacker"),
            )
        };
        let text = format!(
            "phases = {{ opening }}\n{}{}",
            body("lower", |word| word.to_owned()),
            body("Mixed", |word| {
                if word == "river_crossing" {
                    word.to_ascii_uppercase()
                } else {
                    word.char_indices()
                        .map(|(at, c)| {
                            if at % 2 == 0 {
                                c.to_ascii_uppercase()
                            } else {
                                c
                            }
                        })
                        .collect()
                }
            }),
        );
        let content = parsed(text.as_bytes());
        let lexical = [
            LexicalTokenRow {
                token: 41,
                literal: b"lower".to_vec(),
            },
            LexicalTokenRow {
                token: 42,
                literal: b"mixed".to_vec(),
            },
        ];
        let derived = candidates(&content, &lexical).unwrap();
        let mut detail = LandCombatDetailsRow {
            phase: 0,
            ..Default::default()
        };
        detail.sides[0].commander = 1;
        detail.sides[0].commander_skill = 3;
        let aliases = BTreeMap::from([(10, 1)]);
        let mut context = context(&detail, &content, &aliases);
        context.candidates = &derived;
        context.first_countries[0] = Some(10);
        context.marker_flags = Some([1 << 10, 0]);
        let weights: Vec<_> = derived
            .iter()
            .map(|candidate| context.weight(candidate, content.definition(candidate).unwrap()))
            .collect();
        assert_eq!(weights, [Ok(700_000), Ok(700_000)]);
    }

    #[test]
    fn fighting_in_terrain_compares_the_resolved_terrain_object() {
        let content = content();
        let aliases = BTreeMap::new();
        let run = |valid: u8, stored: &[u8], terrains: Vec<&[u8]>, text: &str| {
            let detail = LandCombatDetailsRow {
                stored_terrain_valid: valid,
                stored_terrain_name: stored.to_vec(),
                ..Default::default()
            };
            let mut context = context(&detail, &content, &aliases);
            context.terrains = terrains;
            context.check(&trigger(text))
        };
        let terrains = || vec![b"plains".as_slice(), b"Plains", b"forest"];
        assert_eq!(
            run(1, b"Plains", terrains(), "is_fighting_in_terrain = PLAINS"),
            Ok(false)
        );
        assert_eq!(
            run(1, b"plains", terrains(), "is_fighting_in_terrain = PLAINS"),
            Ok(true)
        );
        assert_eq!(
            run(0, b"", terrains(), "is_fighting_in_terrain = swamp"),
            Ok(true)
        );
        assert_eq!(
            run(0, b"", terrains(), "is_fighting_in_terrain = plains"),
            Ok(false)
        );
        assert_eq!(
            run(1, b"forest", vec![b"forest", b"forest"], "is_fighting_in_terrain = forest"),
            Err("trigger is_fighting_in_terrain = forest: the content defines terrain forest more than once and the recording does not say which the combat holds".into())
        );
    }

    #[test]
    fn bool_trigger_operands_match_their_exact_text() {
        let detail = LandCombatDetailsRow::default();
        let content = content();
        let aliases = BTreeMap::new();
        let context = context(&detail, &content, &aliases);
        let evaluate = |text: &str| context.check(&trigger(text));
        assert_eq!(evaluate("is_attacker = yes"), Ok(true));
        assert_eq!(evaluate("is_attacker = yes\0junk"), Ok(true));
        assert_eq!(evaluate("is_attacker = \"yes\""), Ok(true));
        assert_eq!(evaluate("IS_ATTACKER = YES"), Ok(false));
        assert_eq!(evaluate("is_attacker = No"), Ok(false));
        assert_eq!(evaluate("is_attacker = \"\""), Ok(false));
        assert_eq!(evaluate("is_attacker = \"ROOT\""), Ok(false));
        assert_eq!(evaluate("has_reserves = garbage"), Ok(true));
        let reason = evaluate("is_attacker = Root").unwrap_err();
        assert!(
            reason.contains("trigger is_attacker = Root")
                && reason.contains("operand Root")
                && reason.contains("scope selector keyword")
                && reason.contains("not reproduced"),
            "{reason}"
        );
        let reason = evaluate("is_attacker = \"event_target:x\"").unwrap_err();
        assert!(
            reason.contains("trigger is_attacker = event_target:x")
                && reason.contains("operand event_target:x")
                && reason.contains("event target")
                && reason.contains("not reproduced"),
            "{reason}"
        );
    }

    #[test]
    fn terrain_and_sub_unit_names_match_without_case() {
        content_reader_script::test_game::enter_viewer();
        let mut units = SubUnits::default();
        units.read(
            b"sub_units = { artillery = { is_artillery_brigade = yes } infantry = { } \
              odd = { is_artillery_brigade = YES } }",
            "units.txt",
        );
        let content =
            content().with_inventories(Traits::default(), Ok(&units), ScriptedTriggers::default());
        let mut detail = LandCombatDetailsRow {
            stored_terrain_valid: 1,
            stored_terrain_name: b"plains".to_vec(),
            ..Default::default()
        };
        detail.sides[0].committed = vec![1];
        let division = LandCombatDivisionRow {
            division: 1,
            context: replay_data::rows::LandDivisionContext {
                template: 9,
                ..Default::default()
            },
            width_terms: vec![
                replay_data::rows::LandDivisionWidthTerm {
                    subunit: 5,
                    count: 1,
                    width: 0,
                },
                replay_data::rows::LandDivisionWidthTerm {
                    subunit: 6,
                    count: 3,
                    width: 0,
                },
            ],
            own: Some(replay_data::rows::LandCombatMemberValues {
                ..Default::default()
            }),
            ..Default::default()
        };
        let aliases = BTreeMap::new();
        let mut context = context(&detail, &content, &aliases);
        context.participants = &detail.sides[0].committed;
        context.divisions = BTreeMap::from([((0, 1), &division)]);
        context.tokens =
            BTreeMap::from([(5, b"ARTILLERY".as_slice()), (6, b"Infantry".as_slice())]);
        context.terrains = vec![b"plains".as_slice(), b"forest"];
        let evaluate = |context: &Context, text: &str| context.check(&trigger(text));
        assert_eq!(
            evaluate(&context, "is_fighting_in_terrain = PLAINS"),
            Ok(true)
        );
        assert_eq!(
            evaluate(&context, "is_fighting_in_terrain = forest"),
            Ok(false)
        );
        assert_eq!(
            evaluate(&context, "is_fighting_in_terrain = \"plains\""),
            Ok(false)
        );
        assert_eq!(evaluate(&context, "has_unit_type = artillery"), Ok(true));
        assert_eq!(evaluate(&context, "has_unit_type = ARTILLERY\0x"), Ok(true));
        assert_eq!(evaluate(&context, "has_artillery_ratio > 0.2"), Ok(true));
        assert_eq!(evaluate(&context, "has_artillery_ratio > 0.3"), Ok(false));
        context.tokens.insert(6, b"odd".as_slice());
        assert_eq!(evaluate(&context, "has_artillery_ratio > 0.2"), Ok(true));
        context.tokens.insert(6, b"absent".as_slice());
        assert_eq!(
            evaluate(&context, "has_artillery_ratio > 0.2"),
            Err("trigger has_artillery_ratio > 0.2: sub-unit absent is not in the content's sub_units".into())
        );
    }

    #[test]
    fn native_boolean_groups_preserve_decisive_results_and_missing_operands() {
        let detail = LandCombatDetailsRow::default();
        let content = content();
        let aliases = BTreeMap::new();
        let mut context = context(&detail, &content, &aliases);
        context.tokens_complete = Err("LexicalTokens is not complete this hour".into());
        let evaluate = |text: &str| context.check(&trigger(text));
        assert_eq!(
            evaluate("NOT={ is_attacker=no has_reserves=yes }"),
            Ok(true)
        );
        assert_eq!(
            evaluate("NOT={ is_attacker=yes has_reserves=yes }"),
            Ok(false)
        );
        assert_eq!(evaluate("synthetic_unported_trigger=yes is_attacker=no"), Ok(false));
        assert_eq!(
            evaluate("OR={ synthetic_unported_trigger=yes is_attacker=yes }"),
            Ok(true)
        );
        assert!(evaluate("synthetic_unported_trigger=yes is_attacker=yes").is_err());
        assert!(evaluate("frontage_full=yes").is_err());
    }

    #[test]
    fn river_crossing_requires_the_selected_sides_marker_flags() {
        let detail = LandCombatDetailsRow::default();
        let content = content();
        let aliases = BTreeMap::new();
        let mut context = context(&detail, &content, &aliases);
        let plain = trigger("has_combat_modifier=river_crossing");
        assert!(context.check(&plain).is_err());
        context.marker_flags = Some([1 << 10, 0]);
        assert_eq!(context.check(&plain), Ok(true));
        let shouted = trigger("HAS_COMBAT_MODIFIER=RIVER_CROSSING");
        assert_eq!(context.check(&shouted), Ok(true));
        for quoted in [
            "has_combat_modifier = \"river_crossing\"",
            "@m = \"RIVER_CROSSING\" has_combat_modifier = @m",
        ] {
            let error = context.check(&trigger(quoted)).unwrap_err();
            assert!(error.contains("the operand is quoted"), "{error}");
        }
        context.side = 1;
        assert_eq!(context.check(&plain), Ok(false));
    }

    fn weight_of(file: &str, detail: &LandCombatDetailsRow) -> Evaluation<i64> {
        let content = parsed(file.as_bytes());
        let aliases = BTreeMap::from([(10, 1)]);
        let mut context = context(detail, &content, &aliases);
        context.first_countries[0] = Some(10);
        let definition = content.registry[0]
            .definition
            .as_ref()
            .map_err(Clone::clone)?;
        let candidate = Candidate {
            tactic_id: 1,
            ..Candidate::default()
        };
        context.weight(&candidate, definition)
    }

    #[test]
    fn has_tactic_looks_up_the_interned_spelling() {
        let file = b"t = { trigger = { has_tactic = @operand } }\n\
                     Tactic_Foo = { active = no }\n\
                     other = { }\n\
                     twin = { active = no } twin = { }\n";
        let detail = LandCombatDetailsRow::default();
        let aliases = BTreeMap::from([(10, 1)]);
        let weigh = |operand: &str, literals: &[(u32, &[u8])], unlocked: &[u32]| {
            let text = String::from_utf8_lossy(file).replace("@operand", operand);
            let content = parsed(text.as_bytes());
            let lexical: Vec<_> = literals
                .iter()
                .map(|(token, literal)| LexicalTokenRow {
                    token: *token,
                    literal: literal.to_vec(),
                })
                .collect();
            let derived = candidates(&content, &lexical)?;
            let mut context = context(&detail, &content, &aliases);
            context.first_countries[0] = Some(10);
            context.candidates = &derived;
            context.tokens = literals.iter().copied().collect();
            context.country_unlocks = BTreeMap::from([(1, unlocked)]);
            let definition = content.definition(&derived[0])?;
            Evaluator::new(&context.world, Some(&context))
                .evaluate(&definition.blocks("trigger"), Scopes::of(Scope::Country(1)))
                .decided()
                .map(|holds| if holds { 100_000 } else { 0 })
        };
        let dictionary: [(u32, &[u8]); 5] = [
            (41, b"t"),
            (42, b"Tactic_Foo"),
            (43, b"other"),
            (44, b"twin"),
            (45, b"Missing_Tactic"),
        ];
        assert_eq!(weigh("TACTIC_FOO", &dictionary, &[2]), Ok(100_000));
        assert_eq!(weigh("TACTIC_FOO", &dictionary, &[]), Ok(0));
        assert_eq!(weigh("TACTIC_FOO\0x", &dictionary, &[]), Ok(0));
        assert_eq!(weigh("OTHER", &dictionary, &[]), Ok(100_000));
        assert_eq!(weigh("missing_tactic", &dictionary, &[]), Ok(100_000));
        let mut lowered = dictionary;
        lowered[1] = (42, b"tactic_foo");
        assert_eq!(weigh("Tactic_Foo", &lowered, &[]), Ok(100_000));
        let refused = |operand: &str, why: &str| {
            let reason = weigh(operand, &dictionary, &[]).unwrap_err();
            assert!(reason.contains(why), "{operand}: {reason}");
        };
        assert_eq!(weigh("twin", &dictionary, &[]), Ok(100_000));
        refused("absent", "absent is not in the recorded lexical dictionary");
        assert_eq!(
            weight_of("t={trigger={has_tactic=missing_tactic}}", &detail),
            Ok(0)
        );
    }

    #[test]
    fn only_show_for_follows_the_stored_link_index() {
        let content = parsed(
            b"mine = { only_show_for = GER }\n\
              invalid = { only_show_for = XYZ }\n\
              block = { only_show_for = { GER } }\n\
              open = { }\n",
        );
        let detail = LandCombatDetailsRow::default();
        let aliases = BTreeMap::new();
        let mut context = context(&detail, &content, &aliases);
        context.country_tags = BTreeMap::from([(b"GER".as_slice(), 0), (b"SOV".as_slice(), 1)]);
        let listed = |context: &Context, index: usize, ours: u32| {
            let definition = content.registry[index].definition.as_ref().unwrap();
            listed_for(definition, context, ours)
        };
        assert_eq!(listed(&context, 0, 0), Ok(true));
        assert_eq!(listed(&context, 0, 1), Ok(false));
        assert_eq!(listed(&context, 1, 1), Ok(true));
        assert_eq!(listed(&context, 2, 1), Ok(true));
        assert_eq!(listed(&context, 3, 1), Ok(true));
        context.countries = Err("Countries is not complete this hour".into());
        assert_eq!(listed(&context, 0, 1), Ok(false));
        assert_eq!(
            listed(&context, 1, 1),
            Err("only_show_for = XYZ names no recorded country, and Countries is not complete this hour".into())
        );
    }

    #[test]
    fn blocks_under_one_value_names_are_read_as_one_value() {
        let mut buildings = content_reader_key_names::buildings::Buildings::default();
        buildings.types.insert(
            "arms_factory".into(),
            content_reader_key_names::buildings::BuildingType::default(),
        );
        let mut names = names(&ScriptedTriggers::default());
        names.counts = content_reader_key_names::key_name_products::count_trigger_names(
            &buildings,
            &content_reader_key_names::ideologies::Ideologies::default(),
            &Default::default(),
        );
        let content = parsed_with(
            b"first = { trigger = { arms_factory = 2 } }\n\
              second = { base = { modifier = { OR = { ARMS_FACTORY = { is_attacker = yes } always = no } } } }\n",
            &names,
        );
        let second = content.registry[1].definition.as_ref().unwrap();
        let modifier = field(&second.items, "base").unwrap().value.items().unwrap()[0]
            .value
            .items()
            .unwrap();
        let group = modifier[0].value.items().unwrap();
        assert_eq!(group.len(), 2, "{group:?}");
        assert_eq!(group[0].value.scalar(), Some(b"{".as_slice()));
        assert!(group[1].keyword_is("is_attacker"));
        assert!(modifier[1].keyword_is("always"));
        assert!(
            content.warnings().iter().any(|warning| {
                warning.contains("field `ARMS_FACTORY` is a block where its reader takes one value")
            }),
            "{:?}",
            content.warnings()
        );
        for text in [
            &b"a = { trigger = { armor = { x = 1 } } }\n"[..],
            b"b = { trigger = { infantry = { x = 1 } } }\n",
        ] {
            let content = parsed_with(text, &names);
            assert!(content.registry[0].definition.is_ok());
            assert!(
                content
                    .warnings()
                    .iter()
                    .any(|warning| warning.contains("is a block where its reader takes one value")),
                "{:?}",
                content.warnings()
            );
        }
    }

    fn skilled() -> LandCombatDetailsRow {
        let mut detail = LandCombatDetailsRow::default();
        detail.sides[0].commander = 1;
        detail.sides[0].commander_skill = 3;
        detail
    }

    #[test]
    fn modifier_factors_are_scoped_variables() {
        let detail = skilled();
        let weight = |body: &str| weight_of(&format!("t = {{ base = {{ {body} }} }}"), &detail);
        let refused = |body: &str, operand: &str| {
            let reason = weight(body).unwrap_err();
            assert!(
                reason.contains(&format!("base weight modifier {operand}:")),
                "{body}: {reason}"
            );
        };
        assert_eq!(weight("modifier = { ADD = 0.5 }"), Ok(150_000));
        assert_eq!(weight("modifier = { factor = 1.5 }"), Ok(150_000));
        assert_eq!(weight("modifier = { factor = 010 }"), Ok(1_000_000));
        assert_eq!(
            weight("modifier = { factor = 2 factor = \"\" }"),
            Ok(200_000)
        );
        assert_eq!(
            weight("modifier = { factor = var:x factor = 3 }"),
            Ok(300_000)
        );
        refused("modifier = { factor = var:weight }", "factor = var:weight");
        refused("modifier = { factor = 3 factor = @x }", "factor = @x");
        refused("modifier = { add = 1e3 }", "add = 1e3");
        refused(
            "modifier = { factor = { is_attacker = yes } }",
            "factor = {",
        );
        assert_eq!(
            weight("modifier = { factor = var:weight is_attacker = no }"),
            Ok(100_000)
        );
    }

    #[test]
    fn weight_base_values_follow_the_mean_time_to_happen_reader() {
        let detail = LandCombatDetailsRow::default();
        let weight = |body: &str| weight_of(&format!("t = {{ {body} }}"), &detail);
        assert_eq!(weight(""), Ok(100_000));
        assert_eq!(weight("base = { }"), Ok(100_000));
        assert_eq!(weight("BASE = { FACTOR = 010 }"), Ok(800_000));
        assert_eq!(weight("base = { base = 0.5 }"), Ok(50_000));
        assert_eq!(
            weight("base = { months = 2 factor = 3 days = 4 }"),
            Ok(400_000)
        );
        assert_eq!(weight("base = { years = 1 }"), Ok(36_500_000));
        assert_eq!(
            weight("base = { factor = 2 } base = { modifier = { add = 1 } }"),
            Ok(300_000)
        );
        assert_eq!(weight("base = { days = 1.5 }"), Ok(100_000));
        assert_eq!(weight("base = { \"factor\" = 3 }"), Ok(100_000));
    }

    #[test]
    fn malformed_blocks_weigh_as_their_readers_read_them() {
        let detail = skilled();
        let weight = |file: &str| weight_of(file, &detail);
        assert_eq!(
            weight("t = { trigger = yes TRIGGER = { is_attacker = yes } base = { factor = 2 } }"),
            Ok(200_000)
        );
        assert_eq!(
            weight("t = { base = { modifier = yes factor = 2 } }"),
            Ok(200_000)
        );
        assert_eq!(
            weight("t = { base = { factor = { factor = 2 } } }"),
            Ok(200_000)
        );
    }

    #[test]
    fn triggers_consume_definitions_and_resolve_comparisons() {
        let detail = skilled();
        let weight = |trigger: &str| {
            weight_of(
                &format!("phases = {{ 99 close_combat }} t = {{ trigger = {{ {trigger} }} }}"),
                &detail,
            )
        };
        assert_eq!(weight("@x = yes is_attacker = @x"), Ok(100_000));
        assert_eq!(
            weight("OR = { @y = yes is_attacker = @y } is_attacker = yes"),
            Ok(100_000)
        );
        assert_eq!(weight("@t = 2 skill>@t"), Ok(100_000));
        assert_eq!(weight("skill > 02"), Ok(100_000));
        assert_eq!(weight("skill > 010"), Ok(0));
        assert_eq!(weight("skill>\"010\""), Ok(0));
        assert_eq!(weight("skill < \"010\""), Ok(100_000));
        assert_eq!(weight("SKILL = 3"), Ok(100_000));
        assert_eq!(weight("phase > 99"), Ok(100_000));
        assert_eq!(weight("phase = close_combat"), Ok(0));
        assert_eq!(weight("skill < \"3.5\""), Ok(0));
        assert_eq!(weight("skill > \"2.9\""), Ok(100_000));
    }

    fn empty_tables() -> Vec<(TableId, TableRows)> {
        vec![
            (
                TableId::StateBuildings,
                TableRows::from_rows::<tables::StateBuildings>(Vec::new()),
            ),
            (
                TableId::ProvinceBuildings,
                TableRows::from_rows::<tables::ProvinceBuildings>(Vec::new()),
            ),
            (
                TableId::States,
                TableRows::from_rows::<tables::States>(Vec::new()),
            ),
            (
                TableId::Provinces,
                TableRows::from_rows::<tables::Provinces>(Vec::new()),
            ),
            (
                TableId::ResourceDefinitions,
                TableRows::from_rows::<tables::ResourceDefinitions>(Vec::new()),
            ),
            (
                TableId::CountryResources,
                TableRows::from_rows::<tables::CountryResources>(Vec::new()),
            ),
            (
                TableId::PoliticalParties,
                TableRows::from_rows::<tables::PoliticalParties>(Vec::new()),
            ),
        ]
    }

    fn countries(tokens: &[(&[u8], u32)]) -> Vec<(TableId, TableRows)> {
        vec![
            (
                TableId::CountryLinks,
                TableRows::from_rows::<tables::CountryLinks>(vec![
                    replay_data::rows::CountryLinkRow {
                        id: 10,
                        canonical_slot: 1,
                    },
                ]),
            ),
            (
                TableId::Countries,
                TableRows::from_rows::<tables::Countries>(vec![replay_data::rows::CountryRow {
                    id: 10,
                    canonical_slot: 1,
                    tag: b"TST".to_vec(),
                    ..Default::default()
                }]),
            ),
            (
                TableId::LexicalTokens,
                TableRows::from_rows::<tables::LexicalTokens>(
                    tokens
                        .iter()
                        .map(|(literal, token)| LexicalTokenRow {
                            token: *token,
                            literal: literal.to_vec(),
                        })
                        .collect(),
                ),
            ),
        ]
    }

    fn weigh_with(
        file: &str,
        scripted: ScriptedTriggers,
        tokens: &[(&'static [u8], u32)],
        tables: Vec<(TableId, TableRows)>,
    ) -> Evaluation<i64> {
        let detail = skilled();
        let aliases = BTreeMap::from([(10, 1)]);
        let mut content = parsed_with(file.as_bytes(), &names(&scripted)).with_inventories(
            Traits::default(),
            Ok(&SubUnits::default()),
            scripted,
        );
        content.set_count_names(CountTriggerNames::default());
        let mut recorded = countries(tokens);
        recorded.extend(tables);
        let mut context = context_in(&detail, &content, &aliases, frame(recorded));
        context.first_countries[0] = Some(10);
        context.tokens = tokens.iter().map(|(literal, id)| (*id, *literal)).collect();
        let definition = content.registry[0]
            .definition
            .as_ref()
            .map_err(Clone::clone)?;
        let candidate = Candidate {
            tactic_id: 1,
            ..Candidate::default()
        };
        context.weight(&candidate, definition)
    }

    #[test]
    fn keys_the_evaluator_does_not_reproduce_follow_the_trigger_name_fallbacks() {
        let none = || ScriptedTriggers::default();
        let tables = || {
            let mut tables = empty_tables();
            tables.push((
                TableId::ResourceDefinitions,
                TableRows::from_rows::<tables::ResourceDefinitions>(vec![ResourceDefinitionRow {
                    id: 3,
                    token: 0x6001,
                    active: 1,
                    ..Default::default()
                }]),
            ));
            tables.push((
                TableId::CountryResources,
                TableRows::from_rows::<tables::CountryResources>(vec![CountryResourceRow {
                    country: 10,
                    resource: 3,
                    produced: 700_000,
                    adjustment: -100_000,
                    ..Default::default()
                }]),
            ));
            tables
        };
        let tokens: &[(&[u8], u32)] = &[(b"if", 0x27C3), (b"steel", 0x6001), (b"nothing", 0x6002)];
        let weigh = |trigger: &str| {
            weigh_with(
                &format!("t = {{ trigger = {{ {trigger} }} }}"),
                none(),
                tokens,
                tables(),
            )
        };
        assert_eq!(weigh("\"is_attacker\" = no"), Ok(100_000));
        assert_eq!(weigh("\"SKILL\" > 5"), Ok(100_000));
        assert_eq!(weigh("\"is_attacker\" = no is_attacker = no"), Ok(0));
        assert_eq!(weigh("NOTHING = yes"), Ok(100_000));
        let reason = weigh("if = yes").unwrap_err();
        assert!(
            reason.contains("trigger if = yes") && reason.contains("conditional is not a block"),
            "{reason}"
        );
        assert_eq!(weigh("Steel > 5"), Ok(0));
        assert_eq!(weigh("Steel > 6"), Ok(0));
        assert_eq!(weigh("OWNER = { Steel > 5 }"), Ok(100_000));
        assert_eq!(weigh("OWNER = { Steel > 6 }"), Ok(0));
        assert_eq!(weigh("absent_word = yes"), Ok(100_000));
        assert_eq!(weigh("absent_word = { is_attacker = no }"), Ok(100_000));
        assert_eq!(weigh("\"GER\" = { is_attacker = no }"), Ok(100_000));
        let content = content();
        let (detail, aliases) = (LandCombatDetailsRow::default(), BTreeMap::new());
        let context = context(&detail, &content, &aliases);
        let reason = context.check(&trigger("absent_word = yes")).unwrap_err();
        assert!(
            reason.contains("trigger absent_word")
                && reason.contains("token")
                && reason.contains("unknown")
                && reason.contains("hour 1 LexicalTokens is unavailable: not sampled"),
            "{reason}"
        );
    }

    #[test]
    fn scripted_triggers_evaluate_their_body_in_the_current_scope() {
        content_reader_script::test_game::enter_viewer();
        let root = std::env::temp_dir().join(format!(
            "hoi4_viewer_scripted_triggers_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let write = |layer: &str, path: &str, text: &str| {
            let path = root.join(layer).join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "vanilla",
            "common/scripted_triggers/checks.txt",
            "My_Check = { is_attacker = no }",
        );
        write(
            "mod",
            "common/scripted_triggers/checks.txt",
            "@side = yes\nMy_Check = { is_attacker = @side }\nin_opening = { phase = opening }",
        );
        write(
            "mod",
            "common/scripted_triggers/cycles.txt",
            "cycle_a = { CYCLE_B = yes }\ncycle_b = { cycle_a = yes }",
        );
        let layers = content_reader_layers::ResolvedLayers::from_folders(&[
            (&root.join("vanilla"), &[][..]),
            (&root.join("mod"), &[][..]),
        ])
        .unwrap();
        let scripted = content_reader_key_names::scripted_triggers::read(&layers).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let tokens: &[(&[u8], u32)] = &[
            (b"my_check", 0x6001),
            (b"cycle_a", 0x6002),
            (b"cycle_b", 0x6003),
        ];
        let weigh = |trigger: &str| {
            weigh_with(
                &format!("phases = {{ opening }} t = {{ trigger = {{ {trigger} }} }}"),
                scripted.clone(),
                tokens,
                empty_tables(),
            )
        };
        assert_eq!(weigh("my_check = yes"), Ok(100_000));
        assert_eq!(weigh("\"MY_CHECK\" = yes"), Ok(100_000));
        assert_eq!(weigh("My_Check = no"), Ok(0));
        assert_eq!(weigh("in_opening = yes"), Ok(100_000));
        let cycle = weigh("cycle_a = yes").unwrap_err();
        assert!(cycle.contains("cycle_a -> cycle_b -> cycle_a"), "{cycle}");
        assert!(cycle.contains("recurs"), "{cycle}");
        assert_eq!(weigh("my_check = { x = 1 }"), Ok(0));
    }

    #[test]
    fn content_blocks_follow_load_keys_order_in_every_body_read() {
        content_reader_script::test_game::enter_viewer();
        let root = std::env::temp_dir().join(format!(
            "hoi4_viewer_scripted_order_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let write = |path: &str, text: &str| {
            let path = root.join("vanilla").join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "common/scripted_triggers/a.txt",
            "AND = { is_attacker = no }\nroot = { is_attacker = no }\n\
             bad = { other = { x = 1 } }\nother = { is_attacker = yes }\nempty = { OR = yes }",
        );
        write(
            "common/scripted_triggers/b.txt",
            "BAD = { is_attacker = yes }\nempty = { NOT = yes }\n\
             tooltip = { is_attacker = no }\nnot_tooltip = { is_attacker = no }",
        );
        let layers =
            content_reader_layers::ResolvedLayers::from_folders(&[(&root.join("vanilla"), &[][..])])
                .unwrap();
        let scripted = content_reader_key_names::scripted_triggers::read(&layers).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let weigh = |trigger: &str| {
            weigh_with(
                &format!("t = {{ trigger = {{ {trigger} }} }}"),
                scripted.clone(),
                &[],
                empty_tables(),
            )
        };
        assert_eq!(weigh("AND = { is_attacker = yes }"), Ok(100_000));
        assert_eq!(weigh("ROOT = { is_attacker = yes }"), Ok(100_000));
        assert_eq!(weigh("bad = yes"), Ok(100_000));
        assert_eq!(weigh("empty = yes"), Ok(100_000));
        assert_eq!(weigh("OR = no"), Ok(0));
        assert_eq!(weigh("custom_trigger_tooltip = x AND = yes"), Ok(100_000));
        assert_eq!(
            weigh("custom_trigger_tooltip = { tooltip = yes not_tooltip = yes }"),
            Ok(100_000)
        );
        assert_eq!(
            weigh(
                "custom_trigger_tooltip = { \
                tooltip = { localization_key = POSITIVE other = { localization_key = ARG } } \
                NOT_TOOLTIP = { localization_key = NEGATIVE } is_attacker = yes }"
            ),
            Ok(100_000)
        );
        assert_eq!(
            weigh("custom_trigger_tooltip = { \"not_tooltip\" = yes }"),
            Ok(0)
        );
        assert_eq!(weigh("other = yes"), Ok(100_000));
    }

    #[test]
    fn character_scopes_read_the_recorded_character() {
        content_reader_script::test_game::enter_viewer();
        use replay_data::{
            rows::CharacterOperativeRow, rows::CharacterRoleTraitRow, rows::CharacterRow,
            rows::CountryCharacterRow, rows::CountryLinkRow, rows::CountryRow, rows::TableId,
            rows::TableRows, rows::UnitLeaderRow, rows::UnitLeaderTraitRow,
        };
        use replay_frame::{frame::Metrics, frame::TableView};
        let root = std::env::temp_dir().join(format!(
            "hoi4_viewer_character_scope_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        for (file, text) in [
            (
                "common/unit_leader/00_traits.txt",
                "leader_traits = { brilliant_strategist = { } trickster = { } Trickster = { } }",
            ),
            (
                "common/country_leader/00_traits.txt",
                "leader_traits = { popular_figurehead = { } captain_of_industry = { } Popular_Figurehead = { } }",
            ),
        ] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
        }
        let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let traits = Traits::registered(&layers).unwrap();
        let country_leader_traits =
            Arc::new(content_reader_military::leader_traits::read(&layers).unwrap().0);
        std::fs::remove_dir_all(&root).unwrap();

        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "not sampled".into(),
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        let mut set = |id: TableId, rows: TableRows| {
            *frame.tables[id.index()] = TableView::Complete(rows);
        };
        set(
            TableId::CountryLinks,
            TableRows::CountryLinks(vec![CountryLinkRow {
                id: 100,
                canonical_slot: 0,
            }]),
        );
        set(
            TableId::Countries,
            TableRows::Countries(vec![CountryRow {
                id: 100,
                canonical_slot: 0,
                tag: b"GER".to_vec(),
                ..Default::default()
            }]),
        );
        let names = [
            (0x8001, "GER_leader"),
            (0x8002, "GER_politician"),
            (0x8003, "GER_scientist"),
            (0x8004, "GER_returned"),
            (0x8005, "GER_operative"),
            (0x8006, "GER_lost_agent"),
            (0x8007, "GER_miscounted"),
            (0x9001, "popular_figurehead"),
            (0x9002, "captain_of_industry"),
            (0x9003, "brilliant_scientist"),
        ];
        set(
            TableId::LexicalTokens,
            TableRows::from_rows::<tables::LexicalTokens>(
                names
                    .iter()
                    .map(|(token, literal)| LexicalTokenRow {
                        token: *token,
                        literal: literal.as_bytes().to_vec(),
                    })
                    .collect(),
            ),
        );
        let character = |id: u32, country: u32, name_token_id: i32| CharacterRow {
            id,
            country,
            name_token_id,
            ..Default::default()
        };
        set(
            TableId::Characters,
            TableRows::from_rows::<tables::Characters>(vec![
                character(1, 100, 0x8001),
                character(2, 100, 0x8002),
                character(3, 100, 0x8003),
                character(4, 100, 0x8004),
                character(5, 100, 0x8004),
                character(6, 100, 0x8005),
                character(7, 100, 0x8006),
                character(8, 100, 0x8007),
            ]),
        );
        let leader = |id: u32, character_id: u32| UnitLeaderRow {
            reference_type: 7,
            id,
            character_id,
            skill_status: 1,
            skill_valid: 1,
            skill_level: 4,
            owner: 100,
            ..Default::default()
        };
        set(
            TableId::UnitLeaders,
            TableRows::from_rows::<tables::UnitLeaders>(vec![leader(50, 1), leader(51, 4)]),
        );
        let assigned =
            |id: u64, ordinal: u32, definition_ordinal: i32, token: &[u8]| UnitLeaderTraitRow {
                leader_reference: (id << 32) | 7,
                ordinal,
                definition_ordinal,
                token: token.to_vec(),
                ..Default::default()
            };
        set(
            TableId::UnitLeaderTraits,
            TableRows::from_rows::<tables::UnitLeaderTraits>(vec![
                assigned(50, 0, 1, b"brilliant_strategist"),
                assigned(50, 1, 3, b"Trickster"),
                assigned(51, 0, 2, b"trickster"),
            ]),
        );
        let role = |character: u32, kind: u8, role: u32, token: u32, definition: Option<u32>| {
            CharacterRoleTraitRow {
                character,
                kind,
                role,
                tokens: vec![token],
                definitions: definition.into_iter().collect(),
            }
        };
        set(
            TableId::CharacterRoleTraits,
            TableRows::from_rows::<tables::CharacterRoleTraits>(vec![
                role(2, CharacterRoleTraitRow::COUNTRY_LEADER, 1, 0x9001, Some(1)),
                role(2, CharacterRoleTraitRow::ADVISOR, 0, 0x9002, Some(2)),
                role(3, CharacterRoleTraitRow::COUNTRY_LEADER, 1, 0x9001, Some(3)),
                role(3, CharacterRoleTraitRow::SCIENTIST, 0, 0x9003, None),
                role(8, CharacterRoleTraitRow::COUNTRY_LEADER, 1, 0x9001, Some(2)),
            ]),
        );
        set(
            TableId::CharacterOperatives,
            TableRows::from_rows::<tables::CharacterOperatives>(vec![
                CharacterOperativeRow {
                    character: 6,
                    reference: (61 << 32) | 7,
                    resolved: true,
                    category: 3,
                    owner: 100,
                    skill_level: Some(2),
                    trait_definitions: vec![2],
                    codename: replay_data::rows::Recorded::Value(None),
                    name: replay_data::rows::Recorded::Value(Some(Vec::new())),
                },
                CharacterOperativeRow {
                    character: 7,
                    reference: (62 << 32) | 7,
                    ..Default::default()
                },
            ]),
        );
        set(
            TableId::CountryRetiredCharacters,
            TableRows::from_rows::<tables::CountryRetiredCharacters>(vec![CountryCharacterRow {
                country: 100,
                ordinal: 0,
                character: Some(5),
            }]),
        );

        let mut detail = skilled();
        let presentation = &mut detail.sides[0].presentation;
        presentation.character = (1 << 32) | 9;
        presentation.character_state = 2;
        presentation.character_role_leader = (50 << 32) | 7;
        presentation.role_traits = [(1, b"brilliant_strategist".as_slice()), (3, b"Trickster")]
            .map(
                |(definition_ordinal, token)| replay_data::rows::LandCombatCommanderTrait {
                    definition_ordinal,
                    token: token.to_vec(),
                    ..Default::default()
                },
            )
            .into();
        let aliases = BTreeMap::from([(100, 0)]);
        let weigh_in = |detail: &LandCombatDetailsRow, trigger: &str| {
            let content = parsed(format!("t = {{ trigger = {{ {trigger} }} }}").as_bytes())
                .with_inventories(
                    traits.clone(),
                    Ok(&SubUnits::default()),
                    ScriptedTriggers::default(),
                )
                .with_country_leader_traits(Ok(Arc::clone(&country_leader_traits)));
            let mut context = context_in(detail, &content, &aliases, &frame);
            context.first_countries[0] = Some(100);
            context.tokens = names
                .iter()
                .map(|(token, literal)| (*token, literal.as_bytes()))
                .collect();
            let definition = content.registry[0]
                .definition
                .as_ref()
                .map_err(Clone::clone)?;
            let candidate = Candidate {
                tactic_id: 1,
                ..Candidate::default()
            };
            context.weight(&candidate, definition)
        };
        let weigh = |trigger: &str| weigh_in(&detail, trigger);
        assert_eq!(weigh("has_trait = Brilliant_Strategist"), Ok(100_000));
        assert_eq!(weigh("has_trait = trickster"), Ok(0));
        assert_eq!(weigh("has_trait = popular_figurehead"), Ok(0));
        let mut plain = skilled();
        assert_eq!(weigh_in(&plain, "has_trait = popular_figurehead"), Ok(0));
        plain.sides[0].presentation.character_state = 1;
        assert_eq!(weigh_in(&plain, "has_trait = trickster"), Ok(0));
        assert_eq!(weigh("GER_leader = { skill > 3 }"), Ok(100_000));
        assert_eq!(weigh("GER_leader = { SKILL > 4 }"), Ok(0));
        assert_eq!(weigh("GER_leader = { skill = 4 }"), Ok(100_000));
        assert_eq!(
            weigh("GER_leader = { has_trait = BRILLIANT_strategist }"),
            Ok(100_000)
        );
        assert_eq!(weigh("GER_leader = { has_trait = trickster }"), Ok(0));
        assert_eq!(weigh("GER_politician = { skill > 0 }"), Ok(0));
        assert_eq!(weigh("GER_politician = { skill = 0 }"), Ok(100_000));
        assert_eq!(weigh("GER_politician = { has_trait = trickster }"), Ok(0));
        assert_eq!(
            weigh("GER_politician = { has_trait = Popular_Figurehead }"),
            Ok(100_000)
        );
        assert_eq!(
            weigh("GER_politician = { has_trait = captain_of_industry }"),
            Ok(100_000)
        );
        assert_eq!(
            weigh("GER_scientist = { has_trait = popular_figurehead }"),
            Ok(0)
        );
        let refused = weigh("GER_miscounted = { has_trait = popular_figurehead }").unwrap_err();
        assert!(refused.contains("do not give that name"), "{refused}");
        assert_eq!(
            weigh("GER_scientist = { has_trait = brilliant_scientist }"),
            Ok(100_000)
        );
        assert_eq!(
            weigh("GER_politician = { has_trait = brilliant_scientist }"),
            Ok(0)
        );
        assert_eq!(weigh("GER_returned = { has_trait = trickster }"), Ok(0));
        assert_eq!(weigh("GER_returned = { skill > 3 }"), Ok(100_000));
        assert_eq!(
            weigh("GER_operative = { has_trait = trickster }"),
            Ok(100_000)
        );
        assert_eq!(weigh("GER_operative = { skill > 1 }"), Ok(100_000));
        assert_eq!(weigh("GER_leader = { tag = THIS }"), Ok(0));
        assert_eq!(weigh("GER_leader = { tag = ROOT }"), Ok(100_000));
        assert_eq!(weigh("GER_operative = { tag = ROOT }"), Ok(100_000));
        assert_eq!(weigh("GER_politician = { tag = ROOT }"), Ok(100_000));
        assert_eq!(weigh("GER_lost_agent = { tag = ROOT }"), Ok(100_000));
        assert_eq!(weigh("GER_lost_agent = { skill = 0 }"), Ok(100_000));
    }

    #[test]
    fn scope_keys_check_their_block_in_the_resolved_scope() {
        use replay_data::{
            rows::CountryCounterInputRow, rows::CountryLinkRow, rows::CountryRow, rows::Recorded,
            rows::ReservedCountryNamingRow, rows::TableId, rows::TableRows,
        };
        use replay_frame::{frame::Metrics, frame::TableView};
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "not sampled".into(),
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        let countries = [(100, 1, "GER", 5), (200, 2, "SOV", 3), (300, 3, "ENG", 0)];
        let mut set = |id: TableId, rows: TableRows| {
            *frame.tables[id.index()] = TableView::Complete(rows);
        };
        set(
            TableId::CountryLinks,
            TableRows::CountryLinks(
                countries
                    .map(|(id, canonical_slot, _, _)| CountryLinkRow { id, canonical_slot })
                    .into(),
            ),
        );
        set(
            TableId::Countries,
            TableRows::Countries(
                countries
                    .map(|(id, canonical_slot, tag, _)| CountryRow {
                        id,
                        canonical_slot,
                        tag: tag.into(),
                        ..Default::default()
                    })
                    .into(),
            ),
        );
        set(
            TableId::CountryCounterInputs,
            TableRows::CountryCounterInputs(
                countries
                    .map(
                        |(country, _, _, owned_state_count)| CountryCounterInputRow {
                            forward_visibility: replay_data::rows::Recorded::NotRecorded,
                            country,
                            owned_state_count,
                            ..Default::default()
                        },
                    )
                    .into(),
            ),
        );
        set(
            TableId::ScriptVariables,
            TableRows::ScriptVariables(
                std::iter::once((0, 0))
                    .chain(countries.map(|(country, ..)| (1, country)))
                    .map(
                        |(scope_kind, scope_id)| replay_data::rows::ScriptVariablesRow {
                            scope_kind,
                            scope_id,
                            seed: replay_data::rows::Recorded::Value(None),
                            entries: if scope_id == 100 {
                                vec![replay_data::rows::ScriptVariableEntry {
                                    name: b"x".to_vec(),
                                    value_raw: 0x4052_4547,
                                }]
                            } else {
                                Vec::new()
                            },
                        },
                    )
                    .collect(),
            ),
        );
        let detail = skilled();
        let aliases = BTreeMap::from([(100, 1), (200, 2), (300, 3)]);
        let tokens: &[(&[u8], u32)] = &[
            (b"GER", 0x7001),
            (b"SOV", 0x7002),
            (b"ENG", 0x7003),
            (b"root", 0x2C94),
            (b"prev", 0x2C95),
            (b"owner", 0x283E),
            (b"root.owner", 0x7004),
            (b"var:x", 0x7005),
        ];
        let weigh_in = |frame: &DecodedFrame, trigger: &str| {
            let content = parsed(format!("t = {{ trigger = {{ {trigger} }} }}").as_bytes());
            let mut context = context_in(&detail, &content, &aliases, frame);
            context.first_countries[0] = Some(100);
            context.tokens = tokens.iter().map(|(literal, id)| (*id, *literal)).collect();
            context.country_tags = countries
                .iter()
                .map(|(_, slot, tag, _)| (tag.as_bytes(), *slot))
                .collect();
            let definition = content.registry[0]
                .definition
                .as_ref()
                .map_err(Clone::clone)?;
            let candidate = Candidate {
                tactic_id: 1,
                ..Candidate::default()
            };
            context.weight(&candidate, definition)
        };
        let weigh = |trigger: &str| weigh_in(&frame, trigger);
        assert_eq!(weigh("GER = { tag = GER }"), Ok(100_000));
        assert_eq!(weigh("SOV = { tag = GER }"), Ok(0));
        assert_eq!(weigh("\"GER\" = { tag = GER }"), Ok(100_000));
        assert_eq!(weigh("owner = { tag = GER }"), Ok(100_000));
        assert_eq!(weigh("ROOT = { is_attacker = yes }"), Ok(100_000));
        assert_eq!(weigh("ROOT.owner = { tag = GER }"), Ok(100_000));
        assert_eq!(weigh("SOV = { PREV = { is_attacker = yes } }"), Ok(100_000));
        assert_eq!(weigh("GER = { is_attacker = yes }"), Ok(0));
        assert_eq!(weigh("GER = { NOT = { is_attacker = yes } }"), Ok(100_000));
        assert_eq!(
            weigh("ENG = { tag = ENG }"),
            Err("trigger ENG = { ... }: reserved country naming native lists are unavailable this hour".into())
        );
        let mut empty_buckets = frame.clone();
        *empty_buckets.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    original_tag_buckets: Recorded::Value(Some(vec![
                        Vec::new();
                        countries.len() + 1
                    ])),
                    ..Default::default()
                },
            ]));
        assert_eq!(weigh_in(&empty_buckets, "ENG = { tag = ENG }"), Ok(100_000));
        assert_eq!(weigh_in(&empty_buckets, "tag = ENG"), Ok(0));
        assert_eq!(weigh("var:x = { tag = GER }"), Ok(100_000));
        assert_eq!(weigh("var:missing = { tag = GER }"), Ok(0));
        let state = weigh("64 = { tag = GER }").unwrap_err();
        assert!(
            state.starts_with("trigger 64 = { ... }: state 64 has no recorded States row"),
            "{state}"
        );
        assert_eq!(weigh("tag = ROOT"), Ok(100_000));
        assert_eq!(weigh("tag = \"GER\0suffix\""), Ok(100_000));
        assert_eq!(weigh("tag = \"SOV\0suffix\""), Ok(0));
        assert_eq!(weigh("SOV = { tag = THIS }"), Ok(100_000));
        assert_eq!(weigh("SOV = { tag = prev }"), Ok(0));
        let refused = |trigger: &str, why: &str| {
            let reason = weigh(trigger).unwrap_err();
            assert!(reason.contains(why), "{trigger}: {reason}");
        };
        for selector in ["owner", "CONTROLLER", "occupied", "capital", "random"] {
            assert_eq!(weigh(&format!("tag = {selector}")), Ok(0), "{selector}");
        }
        refused("tag = event_target:foe", "saved event target foe");
        assert_eq!(weigh("tag = \"ROOT\""), Ok(100_000));
        assert_eq!(weigh("tag = x"), Ok(100_000));
        assert_eq!(weigh("tag = XYZ"), Ok(0));
        refused("tag = ENG", "reserved country naming native lists");
    }

    #[test]
    fn count_triggers_read_the_recorded_hour_in_the_scope_they_accept() {
        content_reader_script::test_game::enter_viewer();
        use replay_data::{
            rows::CountryBuildingResourceRow, rows::CountryCounterInputRow, rows::CountryLinkRow,
            rows::CountryRow, rows::StateBaseResourceRow, rows::StateResourceInputRow,
            rows::StateResourceRow, rows::TableId, rows::TableRows,
        };
        use replay_frame::{frame::Metrics, frame::TableView};
        let root = std::env::temp_dir().join(format!(
            "hoi4_viewer_count_triggers_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let write = |layer: &str, path: &str, text: &str| {
            let path = root.join(layer).join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(
            "vanilla",
            "common/buildings/00_buildings.txt",
            "buildings = { arms_factory = { base_cost = 7200 local_resources_steel = 2 } }",
        );
        write(
            "vanilla",
            "common/resources/00_resources.txt",
            "resources = { steel = { icon_frame = 2 } }",
        );
        write(
            "vanilla",
            "common/ideologies/00_ideologies.txt",
            "ideologies = { fascism = { types = { nazism = { } } } }",
        );
        write(
            "mod",
            "common/buildings/mod_buildings.txt",
            "buildings = { Mod_Mill = { level_cap = { province_max = 3 } } }",
        );
        write(
            "mod",
            "common/ideologies/mod_ideologies.txt",
            "ideologies = { Technocracy = { types = { planners = { } } } }",
        );
        write(
            "mod",
            "common/defines/surrender.lua",
            "NDefines = { NCountry = { BASE_SURRENDER_LIMIT = 0.5, MIN_SURRENDER_LIMIT = 0.1,
            SURRENDER_LIMIT_REDUCTION_PER_COLLABORATION = 0.25, SURRENDER_LIMIT_MULT_FOR_COUNTRIES_WITH_NO_CORES = 1 } }",
        );
        let layers = content_reader_layers::ResolvedLayers::from_folders(&[
            (&root.join("vanilla"), &[][..]),
            (&root.join("mod"), &[][..]),
        ])
        .unwrap();
        let defines = content_reader_defines::defines::read(&layers).unwrap();
        let surrender_limits = dashboard_text::triggers::SurrenderLimits::read(&defines).unwrap();
        let buildings = content_reader_key_names::buildings::read(&layers).unwrap();
        let (ideologies, _) = content_reader_key_names::ideologies::read(&layers).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let &(land_cruiser, land_cruiser_mask, _) =
            content_reader_script::trigger_keys::EQUIPMENT_CATEGORIES
                .iter()
                .find(|&&(_, _, name)| name == "land_cruiser")
                .unwrap();
        let tokens: &[(&[u8], u32)] = &[
            (b"GER", 0x7001),
            (b"mod_mill", 0x7101),
            (b"arms_factory", 0x7102),
            (b"steel", 0x7103),
            (b"technocracy", 0x7104),
            (b"fascism", 0x7105),
            (b"land_cruiser", land_cruiser),
            (b"nothing", 0x7106),
        ];

        let hour = 5;
        let frame_at = |hour| {
            let mut frame = DecodedFrame {
                branch: 1,
                hour,
                tables: std::array::from_fn(|_| TableView::Unavailable {
                    reason: "not sampled".into(),
                })
                .map(Into::into),
                metrics: Metrics::default(),
            };
            let mut set = |id: TableId, rows: TableRows| {
                *frame.tables[id.index()] = TableView::Complete(rows);
            };
            set(
                TableId::LexicalTokens,
                TableRows::from_rows::<tables::LexicalTokens>(
                    tokens
                        .iter()
                        .map(|(literal, token)| LexicalTokenRow {
                            token: *token,
                            literal: literal.to_vec(),
                        })
                        .collect(),
                ),
            );
            let countries = [(100, 1, "GER", 2), (200, 2, "SOV", 1)];
            set(
                TableId::CountryLinks,
                TableRows::CountryLinks(
                    countries
                        .map(|(id, canonical_slot, _, _)| CountryLinkRow { id, canonical_slot })
                        .into(),
                ),
            );
            let lost = hour == 9;
            set(
                TableId::Capitulation,
                TableRows::Capitulation(vec![replay_data::rows::CapitulationRow {
                    country: 100,
                    cached_held_core_weight: Some(6).into(),
                    cached_total_core_weight: Some(8).into(),
                    cached_held_territory_weight: Some(6).into(),
                    cached_total_territory_weight: Some(8).into(),
                    surrender_limit_modifier: Some(0).into(),
                    forced_surrender_limit_modifier: Some(0).into(),
                    max_surrender_limit_offset: Some(0).into(),
                    selected_collaboration: (hour != 6).then_some(0).into(),
                    ..Default::default()
                }]),
            );
            set(
                TableId::CountryGameScoreInputs,
                TableRows::CountryGameScoreInputs(vec![replay_data::rows::CountryGameScoreInputRow {
                    country: 100,
                    land_count: Some(1),
                    ..Default::default()
                }]),
            );
            set(
                TableId::Countries,
                TableRows::Countries(
                    countries
                        .map(|(id, canonical_slot, tag, _)| CountryRow {
                            id,
                            canonical_slot,
                            tag: tag.into(),
                            ..Default::default()
                        })
                        .into(),
                ),
            );
            set(
                TableId::CountryCounterInputs,
                TableRows::CountryCounterInputs(
                    countries
                        .map(
                            |(country, _, _, owned_state_count)| CountryCounterInputRow {
                                forward_visibility: replay_data::rows::Recorded::NotRecorded,
                                country,
                                owned_state_count,
                                ..Default::default()
                            },
                        )
                        .into(),
                ),
            );
            set(
                TableId::States,
                TableRows::States(
                    [(64, 100), (65, 100), (66, 200), (68, 0), (67, 999)]
                        [..if lost { 5 } else { 4 }]
                        .iter()
                        .map(|&(id, owner)| StateRow {
                            id,
                            owner,
                            controller: owner,
                            ..Default::default()
                        })
                        .collect(),
                ),
            );
            set(
                TableId::Provinces,
                TableRows::Provinces(
                    [(1, 64, 100), (2, 65, 200), (3, 66, 100), (4, 66, 0)]
                        .map(|(id, state, owner)| ProvinceRow {
                            id,
                            state,
                            owner,
                            controller: owner,
                            ..Default::default()
                        })
                        .into(),
                ),
            );
            let building = |building_type, (location, level)| BuildingRow {
                location,
                building_type,
                level,
                healthy_level: level,
                ..Default::default()
            };
            set(
                TableId::StateBuildings,
                TableRows::StateBuildings(
                    [(64, 3), (65, 2), (66, 7)]
                        .map(|row| building(0x7102, row))
                        .into(),
                ),
            );
            set(
                TableId::ProvinceBuildings,
                TableRows::ProvinceBuildings(
                    [(1, 1), (2, 4), (3, 2)]
                        .map(|row| building(0x7101, row))
                        .into(),
                ),
            );
            set(
                TableId::ResourceDefinitions,
                TableRows::ResourceDefinitions(vec![ResourceDefinitionRow {
                    id: 2,
                    token: 0x7103,
                    active: 1,
                    ..Default::default()
                }]),
            );
            set(
                TableId::CountryResources,
                TableRows::CountryResources(vec![CountryResourceRow {
                    country: 100,
                    resource: 2,
                    produced: 1_250_000,
                    adjustment: 50_000,
                    ..Default::default()
                }]),
            );
            set(
                TableId::StateResourceInputs,
                TableRows::StateResourceInputs(vec![StateResourceInputRow {
                    state: 64,
                    ..Default::default()
                }]),
            );
            set(
                TableId::StateBaseResources,
                TableRows::StateBaseResources(vec![StateBaseResourceRow {
                    state: 64,
                    resource: 0x7103,
                    amount: 1_000_000,
                    ..Default::default()
                }]),
            );
            set(
                TableId::StateResources,
                TableRows::StateResources(vec![StateResourceRow {
                    state: 64,
                    resource: 0x7103,
                    pool: 250_000,
                    additive: 700_000,
                    cost: 50_000,
                    ..Default::default()
                }]),
            );
            set(
                TableId::CountryBuildingResources,
                TableRows::CountryBuildingResources(vec![CountryBuildingResourceRow {
                    country: 100,
                    building: 0x7102,
                    count: 1,
                    resource: 0x7103,
                    amount: -100_000,
                    ..Default::default()
                }]),
            );
            set(
                TableId::PoliticalParties,
                TableRows::PoliticalParties(
                    [
                        (0, b"fascism".as_slice(), 60_000),
                        (1, b"technocracy", 20_000),
                        (2, b"democratic", 20_000),
                    ]
                    .map(|(id, token, popularity)| PoliticalPartyRow {
                        country: if lost && id == 2 { 999 } else { 100 },
                        id,
                        popularity,
                        token: token.to_vec(),
                        ..Default::default()
                    })
                    .into(),
                ),
            );
            if lost {
                let links = std::mem::replace(
                    &mut *frame.tables[TableId::CountryLinks.index()],
                    TableView::Unavailable {
                        reason: String::new(),
                    },
                );
                if let TableView::Complete(rows) = links {
                    *frame.tables[TableId::CountryLinks.index()] =
                        TableView::Truncated { rows, missing: 1 };
                }
            }
            frame
        };
        let detail = skilled();
        let aliases = BTreeMap::from([(100, 1), (200, 2)]);
        let other = content_reader_script::test_game::category("infantry");
        let units = [
            (1, land_cruiser_mask),
            (2, land_cruiser_mask | other),
            (3, other),
            (4, 0),
        ]
        .map(|(division, categories)| LandCombatDivisionRow {
            division,
            categories,
            own: Some(replay_data::rows::LandCombatMemberValues {
                ..Default::default()
            }),
            ..Default::default()
        });
        let participants = [1, 2, 3, 4];
        let weigh_with_units = |hour, participants: &[u64], trigger: &str| {
            let frame = frame_at(hour);
            let mut content = parsed(format!("t = {{ trigger = {{ {trigger} }} }}").as_bytes());
            content.set_count_names(content_reader_key_names::key_name_products::count_trigger_names(
                &buildings,
                &ideologies,
                &Default::default(),
            ));
            let facts = ContentFacts {
                buildings: Ok(&buildings),
                ideologies: Ok(&ideologies),
                resource_bonuses: Ok([15_000, 20_000]),
                surrender_limits: Ok(surrender_limits),
                ..facts(&content)
            };
            let mut context = context_with(&detail, &content, &aliases, &frame, facts);
            context.first_countries[0] = Some(100);
            context.tokens = tokens.iter().map(|(literal, id)| (*id, *literal)).collect();
            context.country_tags = BTreeMap::from([(b"GER".as_slice(), 1)]);
            context.links = frame
                .complete::<tables::CountryLinks>()
                .map(|_| ())
                .map_err(|error| error.to_string());
            context.participants = participants;
            context.divisions = units.iter().map(|row| ((0, row.division), row)).collect();
            let definition = content.registry[0]
                .definition
                .as_ref()
                .map_err(Clone::clone)?;
            let candidate = Candidate {
                tactic_id: 1,
                ..Candidate::default()
            };
            context.weight(&candidate, definition)
        };
        let weigh_at = |hour, trigger: &str| weigh_with_units(hour, &participants, trigger);
        let weigh = |trigger: &str| weigh_at(hour, trigger);
        let weigh_country_at =
            |hour, trigger: &str| weigh_at(hour, &format!("OWNER = {{ {trigger} }}"));
        let weigh_country = |trigger: &str| weigh_country_at(hour, trigger);

        for trigger in [
            "arms_factory > 4",
            "mod_mill = 3",
            "steel > 12",
            "technocracy > 0.1",
        ] {
            assert_eq!(weigh(trigger), Ok(0), "{trigger}");
            assert_eq!(weigh_at(9, trigger), Ok(0), "{trigger}");
        }

        assert_eq!(weigh_country("arms_factory > 4"), Ok(100_000));
        assert_eq!(weigh_country("arms_factory > 5"), Ok(0));
        assert_eq!(weigh_country("ARMS_FACTORY = 5"), Ok(100_000));
        assert_eq!(weigh_country("mod_mill = 3"), Ok(100_000));
        assert_eq!(weigh("GER = { Mod_Mill < 4 }"), Ok(100_000));
        assert_eq!(weigh("GER = { surrender_progress > 0.49 }"), Ok(100_000));
        assert_eq!(weigh("GER = { surrender_progress > 0.50 }"), Ok(0));
        let missing = weigh_at(6, "GER = { surrender_progress > 0.49 }").unwrap_err();
        assert!(missing.contains("selected_collaboration"), "{missing}");
        assert_eq!(weigh("64 = { arms_factory = 3 }"), Ok(100_000));
        assert_eq!(weigh("65 = { mod_mill = 4 }"), Ok(100_000));
        assert_eq!(weigh_country("arms_factory = \"5.9\""), Ok(100_000));

        assert_eq!(weigh_country("steel > 12"), Ok(100_000));
        assert_eq!(weigh_country("steel > 13"), Ok(0));
        assert_eq!(weigh_country_at(hour + 1, "steel > 12"), Ok(100_000));
        assert_eq!(weigh("64 = { steel > 14 }"), Ok(100_000));
        assert_eq!(weigh("64 = { STEEL > 15 }"), Ok(0));
        assert_eq!(weigh("64 = { Steel = 15 }"), Ok(100_000));
        assert_eq!(weigh("64 = { steel < 16 }"), Ok(100_000));
        let reason = weigh("65 = { steel > 0 }").unwrap_err();
        assert!(
            reason.contains("trigger steel > 0")
                && reason.contains("resource count trigger")
                && reason.contains("state 65 has no recorded resource inputs"),
            "{reason}"
        );

        assert_eq!(weigh_country("technocracy > 0.1"), Ok(100_000));
        assert_eq!(weigh_country("Technocracy > 0.2"), Ok(0));
        assert_eq!(weigh_country("technocracy = 0.1"), Ok(100_000));
        assert_eq!(weigh_country("technocracy < \"0.25\""), Ok(100_000));
        assert_eq!(weigh_country("fascism > 0.59"), Ok(100_000));
        assert_eq!(weigh("64 = { technocracy > 0.1 }"), Ok(0));

        assert_eq!(weigh("land_cruiser > 0.4"), Ok(100_000));
        assert_eq!(weigh("LAND_CRUISER > 0.5"), Ok(0));
        assert_eq!(
            weigh("Armor > 0.5"),
            Err("trigger Armor > 0.5: the viewer does not reproduce this trigger".into())
        );
        assert_eq!(weigh("land_cruiser = 0.49"), Ok(100_000));
        assert_eq!(weigh("land_cruiser < 0.51"), Ok(100_000));
        assert_eq!(weigh("GER = { land_cruiser > 0 }"), Ok(0));
        assert_eq!(
            weigh_with_units(hour, &[], "land_cruiser < 0.01"),
            Ok(100_000)
        );
        let reason = weigh_with_units(hour, &[1, 9], "land_cruiser > 0").unwrap_err();
        assert!(
            reason.contains("trigger land_cruiser > 0")
                && reason.contains("equipment category trigger")
                && reason.contains("attacker division 9 has no LandCombatDivisions row"),
            "{reason}"
        );

        assert_eq!(weigh("64 = { tag = GER }"), Ok(0));
        assert_eq!(weigh("GER = { tag = GER }"), Ok(100_000));
        assert_eq!(weigh("nothing = yes"), Ok(100_000));
        assert_eq!(weigh("64 = { GER = { tag = PREV } }"), Ok(100_000));
        assert_eq!(weigh("66 = { GER = { tag = prev } }"), Ok(0));
        assert_eq!(weigh("66 = { SOV = { tag = PREV } }"), Ok(100_000));
        assert_eq!(weigh("68 = { GER = { tag = PREV } }"), Ok(0));
        assert_eq!(weigh("GER = { tag = ROOT }"), Ok(100_000));
        assert_eq!(weigh("SOV = { tag = FROM }"), Ok(0));

        for (trigger, why) in [
            (
                "OWNER = { arms_factory > 4 }",
                "the owner of state 67: country link 999",
            ),
            (
                "OWNER = { technocracy > 0.1 }",
                "the country of a political parties row: country link 999",
            ),
            (
                "64 = { steel > 0 }",
                "building overrides (country building resources) join through country links",
            ),
        ] {
            let reason = weigh_at(9, trigger).unwrap_err();
            assert!(
                reason.contains(why) && reason.contains("country links: read only in part"),
                "{trigger}: {reason}"
            );
        }
    }

    #[test]
    fn a_block_under_a_single_value_trigger_refuses_the_rest_of_the_file() {
        let detail = skilled();
        assert_eq!(
            weight_of(
                "t = { trigger = { Is_Attacker = { skill > 2 } } } u = { }",
                &detail
            ),
            Ok(0)
        );
        let content = parsed(b"t = { trigger = { skill > { 2 } } } u = { }");
        assert!(
            content
                .registry
                .iter()
                .all(|entry| entry.definition.is_ok()),
            "{:?}",
            content
                .registry
                .iter()
                .filter_map(|entry| entry.definition.as_ref().err())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            weight_of(
                "t = { trigger = { \"is_attacker\" = { is_attacker = no } } }",
                &detail
            ),
            Ok(100_000)
        );
    }

    #[test]
    fn authored_weights_apply_factor_then_add_and_sequential_commander_preferences() {
        let mut detail = LandCombatDetailsRow {
            phase: -1,
            ..Default::default()
        };
        detail.tactic_inputs.preferred_tactic_ids[0] = [0, 7, 7];
        detail.sides[0].commander = 1;
        detail.sides[0].commander_skill = 3;
        let content = parsed(
            b"t = { trigger = { is_attacker = yes phase = -1 } \
                    base = { factor = 4 modifier = { factor = 1.5 add = 1 skill > 2 } } }",
        );
        let definition = content.registry[0].definition.as_ref().unwrap();
        let candidate = Candidate {
            tactic_id: 7,
            ..Default::default()
        };
        let aliases = BTreeMap::from([(10, 1)]);
        let mut context = context(&detail, &content, &aliases);
        context.first_countries[0] = Some(10);
        assert_eq!(context.weight(&candidate, definition), Ok(1_312_500));
        context.preference_factors = Err("NDefines.NMilitary.X is missing".into());
        assert_eq!(
            context.weight(&candidate, definition),
            Err("the preferred-tactic weight defines are unavailable: NDefines.NMilitary.X is missing".into())
        );
    }

}
