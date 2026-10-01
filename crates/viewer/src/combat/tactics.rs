//! The native possible-tactics window, derived in playback.
//!
//! 141C5C1F0 chooses the combat side from the viewing country's membership,
//! evaluates 141299340, sorts by viewing-country unlock then chance, and only
//! then filters `only_show_for`. 141C5D350 binds the authored entry. The menu's
//! open/scroll/drag state belongs to Window, never to the recording.
//!
//! Keys and trigger names are keywords: readers dispatch on the key's token
//! ID ([`Item::keyword`], CTextLexer::read_token 1424A7A90), so a bare key
//! matches without case and a quoted one is no keyword. Values a reader
//! compares by their text keep their case: `yes`/`no` (read_bool_token
//! 1424B06E0), tactic, trait and country names. A value a reader dispatches
//! on by token ID (a terrain, a combat modifier, an integer operand) carries
//! its quoted flag from the content read.
use super::{
    Selection, Values,
    binding::{Bindings, TextIcons, builtin, hide, sprite, text, token},
};
use crate::{document::Document, map::perspective::Vision, table_access::rows};
use dashboard::triggers::{
    Combat, Eval, Evaluator, Scope, Scopes, World, compare, shown, trigger_boolean,
};
use installed_content::{
    combat_tactics::EFFECTS,
    leader_traits::LeaderTraits,
    localisation::{Argument, Localisation},
    parser::{Item, Operator, c_string},
    scripted_triggers::ScriptedTriggers,
    trigger_keys::{CountTriggerNames, TriggerNames},
    unit_leader::{SubUnits, Traits},
};
use replay::{
    frame::DecodedFrame,
    rows::{LandCombatDetailsRow, LandCombatDivisionRow, LexicalTokenRow, tables},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

#[derive(Default)]
pub(super) struct Menu {
    pub values: Values,
    pub entries: Vec<Values>,
}

struct Definition {
    active: bool,
    attacker: bool,
    items: Vec<Item>,
    /// The resolved `phase` and `display_phase` IDs
    /// (installed_content::combat_tactics).
    phase: i32,
    display_phase: i32,
    /// The six [`EFFECTS`] as the content was read.
    effects: [i64; 6],
    /// The picture's sprite: 1406B3CB0 stores the last `picture` read, and
    /// 141C5DD20 draws that object's, so two tactics sharing a name keep
    /// their own. An error says why the viewer has none.
    picture: Result<String, String>,
}

/// A tactic's six effects (1406B3CB0, +0x138..+0x160) in [`EFFECTS`] order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Effects {
    pub width: i64,
    pub movement: i64,
    pub attacker: i64,
    pub attacker_org: i64,
    pub defender: i64,
    pub defender_org: i64,
}

struct RegistryEntry {
    id: i32,
    token: Vec<u8>,
    /// The definition, or why the viewer cannot read it as the game does.
    definition: Result<Definition, String>,
}

#[derive(Default)]
struct Candidate {
    tactic_id: i32,
    unlock_id: u32,
    token: Vec<u8>,
}

/// All expressions and presentation constants are matching-content inputs.
/// The matching tactic file supplies registry order, identity and definitions.
/// The recorded lexical dictionary supplies the native dynamic unlock token.
#[derive(Default)]
pub(crate) struct Content {
    phases: Vec<Vec<u8>>,
    registry: Vec<RegistryEntry>,
    /// The one trait-name inventory (append_trait 140AD7890,
    /// installed_content::unit_leader::Traits::registered).
    leader_traits: Traits,
    /// The country-leader trait database (installed_content::leader_traits), which
    /// a character's `has_trait` searches by name (140712D30) after the
    /// unit-leader traits; `None` until loaded.
    country_leader_traits: Option<Result<Arc<LeaderTraits>, String>>,
    /// The one sub-unit database read (installed_content::unit_leader::sub_units):
    /// a definition is found without case (140AB4070), as the recorded
    /// sub-unit token is interned (1424A7A90).
    sub_units: SubUnits,
    /// Why the sub-unit database could not be read, if it could not.
    sub_units_unreadable: Option<String>,
    /// Where the tactic file's reading cannot be followed: no tactic after
    /// the last one read is known.
    registry_unsettled: Option<String>,
    /// The names a key reaches after the trigger registry misses: scripted
    /// triggers and content count triggers (installed_content::trigger_keys), the one
    /// read every trigger consumer shares.
    names: Arc<TriggerNames>,
    warnings: Vec<String>,
}

/// The last field under keyword `name`, as a reader that overwrites on
/// each occurrence keeps it.
fn field<'a>(items: &'a [Item], name: &str) -> Option<&'a Item> {
    items.iter().rev().find(|item| item.keyword_is(name))
}

impl Content {
    pub(super) fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// The building, ideology and resource names LoadKey's count triggers
    /// find, from the databases the document already loaded. With them the
    /// blocks the content read cannot settle are known
    /// ([`Self::settle_one_value_blocks`]).
    pub(super) fn set_count_names(&mut self, counts: CountTriggerNames) {
        Arc::make_mut(&mut self.names).counts = Some(counts);
        self.settle_one_value_blocks();
    }

    /// The scripted triggers and count names, as settled for the tactics:
    /// the one read the other trigger consumers share.
    pub(crate) fn trigger_names(&self) -> Arc<TriggerNames> {
        Arc::clone(&self.names)
    }

    /// A block under a key that is, or may be, a one-value count or
    /// scripted trigger (TriggerNames::one_value_block) leaves the content
    /// read: the tactic holding one in its trigger or weight modifiers, and
    /// every later one, are unproven by name, as a block the reader replay
    /// cannot settle leaves them (installed_content::combat_tactics); any scripted
    /// body read that holds one, a replaced one included, leaves the rest of
    /// its file unknown (ScriptedTriggers::refuse_bodies).
    fn settle_one_value_blocks(&mut self) {
        let names = Arc::clone(&self.names);
        let offender = self.registry.iter().enumerate().find_map(|(index, entry)| {
            let definition = entry.definition.as_ref().ok()?;
            definition
                .items
                .iter()
                .filter_map(|item| {
                    let items = item.value.items()?;
                    if item.keyword_is("trigger") {
                        names.one_value_block(items)
                    } else if item.keyword_is("base") {
                        items
                            .iter()
                            .filter(|entry| entry.keyword_is("modifier"))
                            .find_map(|modifier| names.one_value_block(modifier.value.items()?))
                    } else {
                        None
                    }
                })
                .next()
                .map(|why| (index, why))
        });
        let logged = self.names.scripted.logged.len();
        Arc::make_mut(&mut self.names).settle();
        self.warnings.extend(
            self.names.scripted.logged[logged..]
                .iter()
                .map(|reason| format!("Combat tactics: triggers: {reason}.")),
        );
        if let Some((index, why)) = offender {
            let first = String::from_utf8_lossy(&self.registry[index].token).into_owned();
            let reason = format!("tactic {first}: {why}");
            self.warnings.push(format!("Combat tactics: {reason}."));
            for entry in &mut self.registry[index..] {
                let name = String::from_utf8_lossy(&entry.token);
                entry.definition = Err(format!(
                    "tactic {name}'s ID or fields cannot be read as the game reads them: {reason}"
                ));
            }
            self.registry_unsettled.get_or_insert(reason);
        }
    }

    /// The one trait-name inventory, for the trait sprites.
    pub(super) fn trait_names(&self) -> &Traits {
        &self.leader_traits
    }

    /// `database` is the one read of the tactic file
    /// (installed_content::combat_tactics); pictures, weights and hovers all read it.
    pub(super) fn from_database(database: installed_content::combat_tactics::Database) -> Self {
        let installed_content::combat_tactics::Database {
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
                // A field the viewer cannot read cannot remove a native
                // registry position or shift every later database ID.
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

    /// The country-leader trait database, or why the content cannot give
    /// it (installed_content::leader_traits::read).
    pub(super) fn with_country_leader_traits(
        mut self,
        traits: Result<Arc<LeaderTraits>, String>,
    ) -> Self {
        self.country_leader_traits = Some(traits);
        self
    }

    /// The trait databases a character's `has_trait` reads, or why they
    /// are not loaded.
    fn character_traits(&self) -> Result<(&Traits, &LeaderTraits), &str> {
        match &self.country_leader_traits {
            None => Err("the country-leader traits are not loaded"),
            Some(Err(why)) => Err(why),
            Some(Ok(traits)) => Ok((&self.leader_traits, traits)),
        }
    }

    /// The unit-leader trait names and the sub-unit database, each from its
    /// one reader in installed_content::unit_leader, and the scripted triggers
    /// (installed_content::scripted_triggers), with their refusals. Scripted bodies load
    /// after the combat tactics (14018B610: LoadScriptedTriggers 140AA8DC0
    /// after 1406B3690), so their `phase` operands resolve against every
    /// phase (140473850 -> 1406B35C0).
    pub(super) fn with_inventories(
        mut self,
        leader_traits: Traits,
        sub_units: Result<&SubUnits, &String>,
        mut scripted: ScriptedTriggers,
    ) -> Self {
        for template in scripted.templates.values_mut() {
            if let Ok(body) = &mut template.body {
                installed_content::combat_tactics::resolve_phase_triggers(&self.phases, body);
            }
        }
        self.warnings.extend(
            scripted
                .logged
                .iter()
                .map(|reason| format!("Combat tactics: triggers: {reason}.")),
        );
        Arc::make_mut(&mut self.names).scripted = scripted;
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

    /// The registry entry a recording names: the entry at its recorded ID
    /// (1406B0A70 stores the database ID at +0x98), which must carry the
    /// recorded name, and its definition when the content read proves it.
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

    /// The picture sprite of the tactic a recording names (141C5DD20 draws
    /// the tactic object's own), through the registry's identity and
    /// refusal checks.
    pub(super) fn picture(&self, id: i32, token: &[u8]) -> Evaluation<&str> {
        let definition = self.entry(id, token)?;
        definition.picture.as_deref().map_err(Clone::clone)
    }

    /// Every picture sprite a proven tactic names, with its texture path,
    /// for the sprite declarations.
    /// Every registry entry the viewer refuses, and every proven tactic's
    /// picture it cannot form: what each tactic's widgets refuse.
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

    /// The effects of the tactic a recording names, for the land hovers
    /// (1406B12D0, 141299190); the error names why the registry entry is
    /// not proven.
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

/// The sprite name a tactic's picture declares: its texture is
/// `gfx/interface/landcombat/tactics/<picture>.dds` (141C5DD20).
const PICTURE_PREFIX: &str = "combat_tactic:";

/// A tactic's definition as 1406B3CB0 reads it, or why the viewer cannot.
fn definition(
    tactic: &installed_content::combat_tactics::Tactic,
    warnings: &mut Vec<String>,
) -> Result<Definition, String> {
    let name = String::from_utf8_lossy(&tactic.token);
    if let Some(reason) = &tactic.unproven {
        return Err(format!(
            "tactic {name}'s ID or fields cannot be read as the game reads them: {reason}"
        ));
    }
    let items = &tactic.fields;
    // 1406B0A70 initializes +170 (active) to 1 and +171 (is_attacker) to 0.
    let active = field_boolean(&name, items, "active", true, warnings);
    let attacker = field_boolean(&name, items, "is_attacker", false, warnings);
    // What the game draws for a tactic without a picture is unverified.
    // 1406B3CB0 stores the value slot's text as a C string (+0x70): to its
    // first NUL.
    let picture = match field(items, "picture").map(|item| &item.value) {
        None => Err(format!("tactic {name} has no picture field")),
        Some(value) => match (value.scalar(), value.c_str()) {
            (Some(_), Ok(picture)) => Ok(format!("{PICTURE_PREFIX}{picture}")),
            (Some(_), Err(_)) => Err(format!(
                "tactic {name}'s picture {} is not UTF-8 before its first NUL, so its texture path cannot be formed",
                shown(value)
            )),
            // A block here leaves the text: the tactic is unproven.
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

/// A yes/no field as read_bool_field (1424ABCA0) reads each occurrence in
/// order: read_bool_token (1424B06E0) accepts only `yes` and `no`, each
/// compared with its terminating NUL, so the text to its first NUL; any
/// other value is reported (1424ACEE0) and leaves the value as it was,
/// since the tactic loader (1406B3690) never checks the reader's errors.
/// A block here marks the tactic unproven as the content is read
/// (installed_content::combat_tactics), so it never reaches this.
fn field_boolean(
    tactic: &str,
    items: &[Item],
    key: &str,
    default: bool,
    warnings: &mut Vec<String>,
) -> bool {
    let mut state = default;
    for item in items.iter().filter(|item| item.keyword_is(key)) {
        match item.value.scalar().and_then(installed_content::parser::read_bool) {
            Some(value) => state = value,
            _ => warnings.push(format!(
                "Combat tactics: tactic {tactic}'s {key} = {} is not exactly yes or no; the game logs an error and keeps {} (1424B06E0).",
                shown(&item.value),
                if state { "yes" } else { "no" }
            )),
        }
    }
    state
}

/// The name 1406B3BD0 gives the null tactic (database entry 0) before
/// 1406B0A70 interns it; an engine constant, not content.
const NULL_TACTIC_NAME: &[u8] = b"nullCombatTactic";

/// 1406B0A70 interns each authored name into the global lexer and stores its
/// token ID at +94. The recording's lexical table is that global dictionary;
/// source-order names from matching content preserve the tactic database IDs.
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
    /// The preferred-tactic weight defines, or why they are unavailable.
    preference_factors: Result<[i64; 3], String>,
    /// The province terrain's combat and support widths, or why they are
    /// unavailable.
    terrain: Result<[i64; 2], String>,
    /// The content's terrain names in database order.
    terrains: Vec<&'a [u8]>,
    aliases: &'a BTreeMap<u32, u32>,
    /// Whether the hour's CountryLinks is whole, or why not: a link it
    /// lacks is then unknown, not another country.
    links: Result<(), String>,
    /// Whether the hour's Countries is whole, or why not: a tag it lacks is
    /// then in no tag list.
    countries: Result<(), String>,
    first_countries: [Option<u32>; 2],
    country_unlocks: BTreeMap<u32, &'a [u32]>,
    country_tags: BTreeMap<&'a [u8], u32>,
    divisions: BTreeMap<(u8, u64), &'a LandCombatDivisionRow>,
    movement_sources: BTreeMap<u64, u32>,
    participants: &'a [u64],
    tokens: BTreeMap<u32, &'a [u8]>,
    /// Whether the hour's LexicalTokens is the whole global dictionary, or
    /// why not.
    tokens_complete: Result<(), String>,
    province_vp: Option<i32>,
    /// The hour's recorded facts the shared trigger evaluator reads
    /// (dashboard::triggers).
    world: World<'a>,
}

impl Context<'_> {
    fn preference_factor(&self, position: usize) -> Evaluation<i64> {
        self.preference_factors
            .as_ref()
            .map(|factors| factors[position])
            .map_err(|why| format!("the preferred-tactic weight defines are unavailable: {why}"))
    }

    /// 141299340 scopes a side to its first primary country.
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

    /// A raw country link's canonical slot (ccountry_tag::canonical_slot
    /// 140BA6240, CGameState+0x340), or why the hour cannot give it.
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

    /// The recorded token IDs spelling `text` without ASCII case: the lexer
    /// looks a bare token up through a lowercased copy (1424A7A90).
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
        // 1413B8CF0 compares the unlocked tactics' +94 tokens with the
        // candidate's. The census records each unlocked tactic's +98
        // database ID, whose token the matching registry supplies. ID 0 is
        // the CNullCombatTactic an unknown `unlock_tactic` name appends
        // (1406B11F0 -> 1406B3BD0); its token interns the engine's own name.
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

    /// NDoctrines::CHasCombatTacticTrigger. Its reader (1404A8120) interns
    /// the operand (add_dynamic_token 1424A6500: the token of that text in
    /// any case, else a new one of this spelling), and Evaluate (14051F250)
    /// looks the token's stored spelling (get_token_string 1424A7300) up
    /// byte-for-byte among the tactic names (1406B11F0). A miss is the null
    /// tactic (GetNullCombatTactic 1406B3BD0), which 1406B0A70 builds
    /// active (+0x170), so the trigger holds; an active tactic holds, and
    /// another is looked up among the scope country's unlocks (1413B8CF0).
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
                        "{shown} is not in the recorded lexical dictionary, though the trigger's reader interns it (1424A6500)"
                    ),
                    Err(why) => format!(
                        "{shown} is not in the recorded lexical dictionary, which is not the whole dictionary ({why}), so the spelling the trigger looks up (1424A7300) is unknown"
                    ),
                });
            }
            [first, second, ..] => {
                return Err(format!(
                    "{shown} is spelled by tokens {first} and {second} in the recorded lexical dictionary, so which the trigger's reader interns (1424A6500) is unknown"
                ));
            }
        };
        let mut named = self
            .content
            .registry
            .iter()
            .filter(|entry| entry.token.as_slice() == stored);
        let Some(entry) = named.next() else {
            if let Some(reason) = &self.content.registry_unsettled {
                return Err(format!(
                    "no tactic read is named {}, and the tactics after {reason} are unknown",
                    String::from_utf8_lossy(stored)
                ));
            }
            return Ok(true);
        };
        if named.next().is_some() {
            return Err(format!(
                "more than one tactic is named {}, and which the name lookup (1406B11F0) holds is not reproduced",
                String::from_utf8_lossy(stored)
            ));
        }
        let candidate = self
            .candidates
            .iter()
            .find(|candidate| candidate.tactic_id == entry.id)
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

    /// A trigger body checked in the tactic scope by the shared evaluator
    /// (dashboard::triggers), this combat answering its combat triggers.
    fn check(&self, items: &[Item]) -> Evaluation<bool> {
        Evaluator::new(&self.world, Some(self))
            .evaluate(items, Scopes::combat())
            .decided()
    }

    /// The combat triggers this combat decides: `has_tactic` in any country
    /// scope, the others only in the combat scope, past their scope mask
    /// (0x10 for the combat triggers; `has_trait` and `skill` 0x18, which a
    /// character scope also passes and the shared evaluator decides there).
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
        // 1404A8120 interns the value token as a C string.
        if name == "has_tactic" {
            let country = || match this {
                Scope::Combat => self.country_slot(),
                Scope::Country(slot) => Ok(slot),
                other => Err(format!(
                    "the trigger's scope check (140542CA0) passed in a {} scope, which holds no country (14052F3F0)",
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
        // The operand's whole text, for a reader of the token itself.
        let operand = || {
            value
                .scalar_str()
                .ok_or_else(|| fail("the operand is not UTF-8"))
        };
        // The operand as a reader that makes a C string of the value token
        // reads it: its bytes to the first NUL, compared as bytes.
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
        let skill = |side: &replay::rows::LandCombatSideDetails| {
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
                // 140472EB0 asks vtable+58 for kind1. Both authenticated
                // land classes, including border combat, return1 there;
                // GetCombatTypeToken at+50 is a different discriminator.
                // The phase ID resolved as the content was read
                // (installed_content::combat_tactics: 140473850 -> 1406B35C0);
                // Evaluate tests equality whatever the operator.
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
                // The tactic scope holds no character (140529310), so the
                // combat's unit leader's (+0x60, vf+0xE8) character
                // (LookupCharacter 140C0EEF0): none is false, before any
                // trait lookup. HasRetiredCharacter (14052BC80) reads the
                // scope's character, so no retired gate applies.
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
                // The character reference's id word (+0x54 of the pair).
                let character = (presentation.character >> 32) as u32;
                // GetUnitLeader (140F8A840): the recorded role leader and
                // its assigned traits (CUnitLeader+0xDC8).
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
                // 140473CB0 stores the first terrain, in database order,
                // whose lexer token is the operand's (names are interned
                // without case, 1424A7A90), else database entry 0, the null
                // object (only logged, 141395A40). Evaluate (1404733A0)
                // compares that pointer with the combat's terrain (+0x48).
                // Only the null object has +0x10 clear (CTerrainDatabase::
                // construct 140AC6860 clears it; 1414297F0 sets it on every
                // CTerrainType).
                // A quoted operand is token 0xF, no terrain's token.
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
                // 1404749D0 uses every participant, min for '>' and max for
                // '<'. Its '=' path asserts rather than selecting a reduction.
                if comparison == Operator::Equals {
                    return Err(fail(
                        "1404749D0 asserts on '=' instead of choosing a reduction",
                    ));
                }
                let greater = comparison == Operator::Greater;
                let mut actual = if greater { 100_000 } else { 0 };
                for unit in self.participants {
                    let hardness = self.division(*unit).map_err(|why| fail(&why))?.stat_values[2];
                    actual = if greater {
                        actual.min(hardness)
                    } else {
                        actual.max(hardness)
                    };
                }
                numeric(actual, true)
            })(),
            // 140473990 maps the operand's keyword ID (141440AE0), which the
            // lexer finds without case; a quoted operand is token 0xF.
            "has_combat_modifier" if item.quoted() => Err(fail(
                "the operand is quoted, so it is token 0xF (1424A7A90) and no combat-modifier keyword 140473990 maps (141440AE0); what the trigger then holds is not reproduced",
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
                // 1404739B0 hashes the value token as a C string
                // (140BBA1B0).
                let operand = c_bytes()?;
                for unit in self.participants {
                    let row = self.division(*unit).map_err(|why| fail(&why))?;
                    for term in self.composition(*unit, row).map_err(|why| fail(&why))? {
                        // 140472610's hash/string match folds ASCII case.
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
                    // 140FAA360 counts each grid with signed16 arithmetic.
                    // This bound proves no grid can overflow despite the
                    // aggregate omitting the original grid partition.
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
                        artillery.wrapping_mul(100_000) / total
                    },
                    true,
                )
            })(),
            _ => return None,
        })
    }

    /// CFrontageFullTrigger: whether the attacker's committed divisions fill
    /// the combat's width (141298E00).
    fn frontage_full(&self, item: &Item) -> Evaluation<bool> {
        let side = &self.detail.sides[self.side];
        let committed = &self.detail.sides[0].committed;
        let directions = super::land_details::direction_count(
            committed,
            &self.divisions,
            self.detail.province,
            &self.movement_sources,
        )
        // 141298E00 needs every row, and the movement source of a unit in
        // the combat province that it does not fold into an earlier
        // direction.
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
        trigger_boolean(item).map(|desired| desired == full)
    }

    /// CEquipmentTypeTrigger::vf23 (141390370) on the side's CLandCombatant
    /// (vf+0x48 true): of its full unit array (+0x20, LandCombats'
    /// attackers or defenders) the share, fixed5, whose stat block category
    /// mask (`*(CUnit+0x138)+0x5A8`, LandCombatDivisions.categories) meets
    /// the trigger's (+0x130): `matching * 10^10 / (units * 100000)`, 0 with
    /// no units, 0xFFFFFFFF past its range check.
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

    /// A participant's sub-units as (count, recorded name), empty for a
    /// division without a template.
    fn composition(&self, unit: u64, row: &LandCombatDivisionRow) -> Evaluation<Vec<(i64, &[u8])>> {
        if row.context.template == 0 {
            return Ok(Vec::new());
        }
        // SpawnArmyConsoleCommand140289780 can populate grids without
        // rebuilding this aggregate. Empty terms are therefore not proof
        // that a present template is empty.
        if row.width_terms.is_empty() {
            return Err(format!(
                "division {unit}'s template has no verified composition aggregate"
            ));
        }
        // 140B94FA0 rebuilds template+C0 from every active slot in all three
        // grids via140B88BA0. These hourly width terms are also the exact
        // composition multiset; no daily template census or additional
        // capture is needed.
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
        // Every `trigger` block loads into the one CAndTrigger at +0xE0 and
        // every `base` block into the one CMeanTimeToHappen at +0xA8
        // (1406B0A70, 1406B3CB0). The content read keeps each as a block: a
        // scalar trigger reads nothing (1405431C0) and is dropped, a scalar
        // base leaves the text and refuses the tactic, and a scalar modifier
        // is an empty one (installed_content::combat_tactics).
        let blocks = |key: &str| -> Vec<Item> {
            definition
                .items
                .iter()
                .filter(|item| item.keyword_is(key))
                .filter_map(|item| item.value.items())
                .flatten()
                .cloned()
                .collect()
        };
        if !self.check(&blocks("trigger"))? {
            return Ok(0);
        }
        // The weight's CMeanTimeToHappen, its modifiers checked in the
        // tactic scope (1405455A0).
        let mut weight = Evaluator::new(&self.world, Some(self)).mean_time_to_happen(
            "base weight",
            &blocks("base"),
            Scopes::combat(),
        )?;
        let inputs = &self.detail.tactic_inputs;
        for preference in 0..3 {
            if inputs.preferred_tactic_ids[self.side][preference] != candidate.tactic_id {
                continue;
            }
            let factor = if preference == 0 {
                let recorded = &inputs.country_weights[self.side];
                if inputs.country_weight_present[self.side] == 0 {
                    return Err(format!(
                        "the {}'s preferred country tactic modifier is not recorded",
                        side_name(self.side)
                    ));
                }
                let modifier = if recorded.accumulator_entry_present == 0 {
                    0
                } else {
                    recorded.accumulator_raw
                };
                self.preference_factor(0)?.wrapping_add(modifier)
            } else {
                self.preference_factor(preference)?
            };
            weight = weight.wrapping_mul(100_000_i64.wrapping_add(factor)) / 100_000;
        }
        Ok(weight)
    }
}

/// The combat side the tactic scope is (1412A77A0), for the shared
/// evaluator.
impl Combat for Context<'_> {
    fn country(&self) -> Result<u32, String> {
        self.country_slot()
    }

    fn trigger(&self, name: &str, item: &Item, this: Scope, evaluator: &Evaluator) -> Option<Eval> {
        self.combat_trigger(name, item, this, evaluator)
            .map(|value| match value {
                Ok(value) => Eval::from_bool(value),
                Err(why) => Eval::unknown(why),
            })
    }

    fn category_share(&self, mask: u64) -> Result<i64, String> {
        Context::category_share(self, mask)
    }
}

fn side_name(side: usize) -> &'static str {
    if side == 0 { "attacker" } else { "defender" }
}

fn normalized_chances(weights: &[Option<i64>]) -> Vec<Option<i64>> {
    // 141299340 sums positive weights before dividing every candidate,
    // including locked/ineligible zeroes. Hidden-country tactics still count.
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

/// 141C5C1F0 tests the attacker's ordered primary country set. Its members
/// may be seeded or retained after the current division roster changes.
/// An unresolved raw country cannot prove that the viewer is on the defender.
fn viewing_side(
    attacker_primary: &[u32],
    aliases: &BTreeMap<u32, u32>,
    viewing_slot: u32,
) -> Evaluation<usize> {
    let mut attacker = false;
    let mut unresolved = None;
    for raw in attacker_primary {
        match aliases.get(raw) {
            Some(slot) => attacker |= *slot == viewing_slot,
            None => unresolved = unresolved.or(Some(*raw)),
        }
    }
    match (attacker, unresolved) {
        (true, _) => Ok(0),
        (false, Some(raw)) => Err(format!(
            "the viewing country's combat side is unavailable: attacker primary country {raw} has no CountryLinks row"
        )),
        (false, None) => Ok(1),
    }
}

/// A recorded or authored name localised with no arguments (142230940: a
/// key the content lacks is its own bytes), then placed into `key`'s
/// `$NAME$`-style argument. Each refusal is the localiser's, by name.
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
    document: &Document,
    frame: &DecodedFrame,
    selection: Selection,
    vision: &Vision,
) -> io::Result<()> {
    let Selection::Land(id) = selection else {
        // Only the land view (1415C7D20) has a possible-tactics popup.
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
    let sides = super::progress::sides(frame)?;
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
        .and_then(|sets| viewing_side(&sets.attacker_primary, &aliases, vision.ours()));
    let side_known = selected_side.is_ok();
    let side = match selected_side {
        Ok(side) => side,
        Err(reason) => {
            unavailable.insert(reason);
            1
        }
    };
    let tokens = rows::<tables::LexicalTokens>(frame)?;
    let content = &document.icons.combat.tactics;
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
    let terrain = match document.counters.combat_terrain.get(&detail.province) {
        None => Err(format!(
            "province {} has no terrain category in the map definitions",
            detail.province
        )),
        Some(terrain) => match document.counters.combat_widths(terrain) {
            None => Err(format!(
                "terrain {terrain} of province {} has no combat widths in the content",
                detail.province
            )),
            Some(widths) => Ok(*widths),
        },
    };
    let context = Context {
        detail,
        marker_flags,
        candidates: &candidates,
        side,
        content,
        preference_factors: document
            .icons
            .combat
            .land_defines
            .as_ref()
            .map(|defines| defines.preference_factors)
            .map_err(Clone::clone),
        terrain,
        terrains: document
            .counters
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
        divisions: rows::<tables::LandCombatDivisions>(frame)?
            .rows()
            .iter()
            .filter(|row| row.combat == id)
            .map(|row| ((row.side, row.division), row))
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
        // The raid content's facts are the triggers' content (their
        // scripted triggers and count names are this content's,
        // Document::load), with the trait databases.
        world: World::read(
            frame,
            dashboard::triggers::ContentFacts {
                character_traits: content.character_traits(),
                ..document.icons.raids.facts()
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
        // Every reason names its tactic, so one tactic's refusal is never
        // merged with another's.
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
    let (icons, loc): (&dyn TextIcons, _) = (&document.icons.combat, &document.names.localisation);
    let mut menu = Menu::default();
    for name in [
        "filter_all",
        "filter_attack",
        "filter_defend",
        "preferred_tactic_desc_text",
    ] {
        hide(&mut menu.values, name);
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
    menu.values
        .entry("current_phase".into())
        .or_default()
        .tooltip = description.map(Into::into);
    let mut order: Vec<usize> = (0..candidates.len()).collect();
    if unlocked.iter().all(Option::is_some) && chances.iter().all(Option::is_some) {
        native_sort(&mut order, |a, b| {
            unlocked[*a] > unlocked[*b]
                || (unlocked[*a] == unlocked[*b] && chances[*a] > chances[*b])
        });
    } else {
        unavailable.insert("selection chances or unlocks are incomplete; candidate order remains matching content order".into());
    }
    for index in order {
        let candidate = &candidates[index];
        let token = String::from_utf8_lossy(&candidate.token);
        // A definition the viewer cannot read was reported above.
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
        menu.entries.push(entry_values(
            icons,
            &|name: &str| document.icons.combat.has_sprite(name),
            loc,
            &content.phases,
            &token,
            candidate,
            definition,
            chances[index],
            unlocked[index],
            &mut unavailable,
        ));
    }
    for reason in unavailable {
        out.warnings.push(format!(
            "Possible tactics: {reason}; missing values remain unavailable."
        ));
    }
    out.warnings.push("Possible-tactics condition and unlock tooltips are not reproduced; the list does not substitute the selected-tactic tooltip.".into());
    out.tactics = menu;
    Ok(())
}

/// Whether 141C5C1F0 lists a tactic for the viewing country `ours`.
/// ReadCountryTag (140BA6310 -> 140BA4CA0) stores `only_show_for`'s tag as
/// its link index, not positive for a text no tag list holds; the popup
/// lists a tactic whose restriction is not positive for every country, and
/// another only for its own (or the same country, 140BA60A0). A tag the
/// hour's partial Countries lacks is unknown.
fn listed_for(definition: &Definition, context: &Context, ours: u32) -> Evaluation<bool> {
    let Some(only_show_for) = field(&definition.items, "only_show_for") else {
        return Ok(true);
    };
    // CCountryTag::ConstructFromCString (140BA4CA0) takes the strlen.
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

/// One popup entry as 141C5C1F0 fills it: the unlock frame and lock, name,
/// picture, rounded selection chance, phase and the six effect values. A
/// value the localisation or the authored field cannot give is named with
/// the tactic, key or field in `unavailable`, never dropped silently.
#[allow(clippy::too_many_arguments)]
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
    // A name without an entry shows as its key, as the game shows it
    // (142230940 copies a missing top-level key).
    match localize_named(icons, loc, "HEADER", "KEY", &candidate.token) {
        Ok(name) => text(&mut row, "tactics_name", name),
        Err(reason) => withhold(
            &mut row,
            "tactics_name",
            unavailable,
            format!("tactic {token}'s name: {reason}"),
        ),
    }
    // The registry entry's own picture (Content::definition checked its
    // identity), as the combat widget reads it (Content::picture), when the
    // content has its sprite.
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
    // An unknown chance was reported with the weights' reasons.
    if chance.is_none() {
        hide(&mut row, "tactics_chance");
    }
    if let Some(chance) = chance {
        // 141C5C1F0 multiplies the fixed5 chance by 100 (141C5C510), rounds
        // half away from zero (1424D8760) and passes the low 32 bits
        // (141C5C521) as the int argument; the six entry modifiers below use
        // ordinary localization truncation.
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
    // The popup (141C5D350) shows the display phase when its ID is not
    // negative, else the start phase, and hides the widget when both are
    // negative (unknown, or the `no` sentinel); IDs were resolved as the
    // content was read (installed_content::combat_tactics) against the phases read
    // before the tactic, so a non-negative ID names one of them.
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
        // Read as the content was read (installed_content::combat_tactics), each
        // field through its native reader.
        let slot = EFFECTS
            .iter()
            .position(|name| *name == source)
            .expect("every popup value is one of the six effects");
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

/// A refused popup field, as a refused window binding (binding::Bindings::
/// bind): its text and sprite cleared and the widget hidden, so neither the
/// authored .gui sprite nor the painter's placeholder shows, and the reason
/// reported by name.
fn withhold(values: &mut Values, key: &str, unavailable: &mut BTreeSet<String>, reason: String) {
    let widget = values.entry(key.into()).or_default();
    widget.text = None;
    widget.sprite = None;
    widget.hidden = true;
    unavailable.insert(reason);
}

/// Match the supported executable's tactic-list sort, including equivalent-item order.
///
/// Native evidence: 0x141c59a10 (introsort), 0x141c58d50 (partition),
/// 0x141c58c90 (median), 0x141c58600 (insertion), 0x141c58900 (heap build),
/// and 0x141c596c0 (heap repair). The caller at 0x141c5c1f0 supplies len
/// as the initial partition budget. This is deliberately not Rust's stable
/// or unstable sort: equal-chance tactics must undergo the native rotations.
/// `less` must be a pure strict weak ordering; for tactics this means unlocked
/// first, then descending raw signed fixed-point chance, with no ID tie-break.
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
        // Native tests the first element separately. This branch also makes
        // the number/order of comparisons match its sentinel insertion path.
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
    // The native threshold is back - first > 40, i.e. at least 42 entries.
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
                // This item already belongs after the equivalent pivot band.
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
                // This item already belongs before the equivalent pivot band.
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
            // Rotate the pivot band upward to make room before it.
            if pivot_last != scan_first {
                values.swap(pivot_first, pivot_last);
            }
            pivot_last += 1;
            values.swap(pivot_first, scan_first);
            pivot_first += 1;
            scan_first += 1;
        } else if scan_first == last {
            // Rotate the pivot band downward to make room after it.
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
    // Native Floyd repair descends all the way, choosing the right child on
    // equivalence, then pushes the held item back up. An early-exit sift-down
    // gives a different order to equivalent tactics.
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
    use dashboard::triggers::ContentFacts;
    use installed_content::fixed5::{decimal_fixed5, script_fixed5};
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            BuildingRow, CountryResourceRow, PoliticalPartyRow, ProvinceRow, ResourceDefinitionRow,
            StateRow, TableId, TableRows,
        },
    };

    /// Six absent effects, each 0 (1406B0A70).
    fn no_effects() -> [i64; 6] {
        [0; 6]
    }

    /// A definition with no fields, as 1406B0A70 constructs one.
    fn plain(active: bool, attacker: bool) -> Definition {
        Definition {
            active,
            attacker,
            items: Vec::new(),
            phase: installed_content::combat_tactics::UNKNOWN_PHASE,
            display_phase: installed_content::combat_tactics::UNKNOWN_PHASE,
            effects: no_effects(),
            picture: Err("tactic t has no picture field".into()),
        }
    }

    /// A trigger block read through the real tactic loader
    /// (installed_content::combat_tactics): definitions consumed, comparisons
    /// normalised and operands resolved with their token kinds.
    fn trigger(text: &str) -> Vec<Item> {
        let database = installed_content::combat_tactics::parse(
            format!("t = {{ trigger = {{ {text} }} }}").as_bytes(),
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

    /// A frame holding `tables`, every other table unavailable, for the
    /// life of the test.
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

    /// The shared evaluator's content facts: `content`'s trigger names and
    /// trait inventory, the character templates of the GER characters the
    /// tests record, every other content fact empty.
    fn facts(content: &Content) -> ContentFacts<'_> {
        ContentFacts {
            state_provinces: Box::leak(Box::default()),
            impassable: Box::leak(Box::default()),
            neighbor_states: Err("the test has no state neighbours"),
            terrain: None,
            dlcs: Box::leak(Box::default()),
            triggers: &content.names,
            // Without the country-leader traits, an empty database.
            character_traits: content
                .character_traits()
                .or(Ok((&content.leader_traits, Box::leak(Box::default())))),
            tag_aliases: Box::leak(Box::default()),
            buildings: Box::leak(Box::default()),
            resource_bonuses: Err("the test has no resource bonuses"),
            leader_roles: Ok(Box::leak(Box::new(dashboard::triggers::LeaderRoles::new(
                [
                    "GER_leader",
                    "GER_politician",
                    "GER_scientist",
                    "GER_returned",
                    "GER_operative",
                    "GER_lost_agent",
                    "GER_miscounted",
                ]
                .map(|tag| installed_content::characters::LeaderTemplate {
                    path: "common/characters/GER.txt".into(),
                    tag: tag.into(),
                    line: None,
                    role: Ok(None),
                })
                .into(),
            )))),
            invasion_prepare_days: Err("the test has no invasion define"),
            script_constants: Err("the test has no script constants"),
        }
    }

    fn context<'a>(
        detail: &'a LandCombatDetailsRow,
        content: &'a Content,
        aliases: &'a BTreeMap<u32, u32>,
    ) -> Context<'a> {
        context_in(detail, content, aliases, frame(Vec::new()))
    }

    /// [`context`] over the recorded tables of `frame`.
    fn context_in<'a>(
        detail: &'a LandCombatDetailsRow,
        content: &'a Content,
        aliases: &'a BTreeMap<u32, u32>,
        frame: &'a DecodedFrame,
    ) -> Context<'a> {
        context_with(detail, content, aliases, frame, facts(content))
    }

    /// [`context_in`] with the given content facts.
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

    /// Content read through the real tactic parser.
    fn parsed(text: &[u8]) -> Content {
        Content::from_database(installed_content::combat_tactics::parse(text))
    }

    /// read_bool_token (1424B06E0) accepts only the exact texts; another
    /// value keeps what the field held (the constructor default at first),
    /// with a warning naming it, and a block is refused by name.
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
        // `e`'s `active` reads `{` (not yes or no: it keeps yes) and the
        // block's `condition = yes` as its own field; the block's `}` ends
        // `e`, and `e`'s own `}` is the sixth tactic's key, which consumes
        // `f` as its operator.
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
                "Combat tactics: tactic c's active = NO is not exactly yes or no; the game logs an error and keeps yes (1424B06E0).",
                "Combat tactics: tactic c's is_attacker = YES is not exactly yes or no; the game logs an error and keeps no (1424B06E0).",
                "Combat tactics: tactic d's active = maybe is not exactly yes or no; the game logs an error and keeps no (1424B06E0).",
                "Combat tactics: tactic e's active = { is not exactly yes or no; the game logs an error and keeps yes (1424B06E0).",
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
        // Registry IDs 1..=3; IDs 2 and 3 share a case-folded unlock token.
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
        // A recorded ID outside the matching registry is never "locked".
        assert!(context.unlocked(&candidate, &definition, 4).is_err());
        // ID 0, the null tactic, is a recorded unlock that matches only a
        // candidate whose token interns the null tactic's own name.
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
        assert_eq!(viewing_side(&[10], &aliases, 1), Ok(0));
        assert_eq!(viewing_side(&[10], &aliases, 2), Ok(1));
        assert_eq!(viewing_side(&[10, 20], &aliases, 2), Ok(0));
        assert_eq!(viewing_side(&[99, 10], &aliases, 1), Ok(0));
        assert_eq!(viewing_side(&[10, 99], &aliases, 1), Ok(0));
        assert!(viewing_side(&[99, 20], &aliases, 1).is_err());
        assert!(viewing_side(&[20, 99], &aliases, 1).is_err());
        assert_eq!(viewing_side(&[], &aliases, 2), Ok(1));
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

    /// Every popup value the localisation or the authored fields cannot give
    /// is named with the tactic and the key or field; given values render.
    /// Phases resolve against the registry: an unknown display phase falls
    /// back to the start phase, and neither resolving hides the widget.
    #[test]
    fn popup_phase_follows_native_resolution() {
        let loc: installed_content::localisation::Localisation = [
            ("opening", "Opening"),
            ("PHASE_DISPLAY", "Shown in $NAME$"),
            ("PHASE_START", "Starts in $NAME$"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let phases = [b"opening".to_vec()];
        use installed_content::combat_tactics::{NO_PHASE, UNKNOWN_PHASE};
        let run = |(display_phase, phase): (i32, i32)| {
            let definition = Definition {
                phase,
                display_phase,
                ..plain(true, true)
            };
            entry_values(
                &crate::combat::binding::NoIcons,
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

    /// A tactic's picture is its registry entry's (1406B3CB0 stores the last
    /// `picture` read; 141C5DD20 draws that object's): two tactics sharing a
    /// name keep their own, a recorded token that disagrees with the entry
    /// is refused, and a tactic whose parse left the text has no picture,
    /// whatever `picture` the tree holds after the block. The popup reads
    /// the same definition.
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
        // `active` reads `{`: the block's `picture = second` is t's last,
        // and its `}` ends t, so `picture = third` is tactic 4, whose reader
        // takes the tokens after it (`}` and `u`) as its fields.
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
        // The registry's availability and reasons hold for pictures too: a
        // tactic without one, one that is not UTF-8, the tactic whose reading
        // is lost (Read::unsettled) and every tactic after it.
        let lost = parsed(
            b"none = { }\n\
              bytes = { picture = \xe9 }\n\
              lost = { picture = here trigger = { OR = { x @v = 1 } } }\n\
              after = { picture = there }\n",
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
        // 1406B3CB0 stores the picture as a C string (+0x70).
        assert_eq!(
            parsed(b"nul = { picture = abc\x00\xe9 }").picture(1, b"nul"),
            Ok("combat_tactic:abc")
        );
        let unproven = lost.picture(3, b"lost").unwrap_err();
        assert!(
            unproven
                .starts_with("tactic lost's ID or fields cannot be read as the game reads them")
                && unproven.contains(
                    "key `x` has no operator and its reader is handed the previous value `{`"
                ),
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
            &crate::combat::binding::NoIcons,
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
        // A picture whose sprite the content lacks is hidden and named, so
        // the authored .gui sprite never shows in its place.
        let mut unavailable = BTreeSet::new();
        let row = entry_values(
            &crate::combat::binding::NoIcons,
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

    /// The movement effect reads ReadFixed5 `%lli` (so `010` is eight whole
    /// points), while the other effects read decimal.
    #[test]
    fn numeric_fields_use_their_native_readers() {
        assert_eq!(script_fixed5(b"010"), Some(800_000));
        assert_eq!(decimal_fixed5(b"010"), 1_000_000);
        let loc: installed_content::localisation::Localisation = [
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
            &crate::combat::binding::NoIcons,
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
        let loc: installed_content::localisation::Localisation = [
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
            &crate::combat::binding::NoIcons,
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
        // A missing top-level key shows its own bytes (142230940).
        assert!(
            row["tactics_chance"]
                .text
                .as_deref()
                .is_some_and(|text| text.contains("TACTIC_SELECTION_CHANCE"))
        );
        // A refused field is hidden with its value cleared, as a refused
        // window binding is: the authored sprite does not show.
        let icon = &row["tactics_icon"];
        assert!(icon.hidden && icon.sprite.is_none() && icon.text.is_none());
        assert_eq!(
            unavailable.into_iter().collect::<Vec<_>>(),
            ["tactic tactic_a's picture: tactic tactic_a has no picture field",]
        );
    }

    /// A tactic's name is localised in the game's stored bytes before HEADER
    /// takes it (142230F20's 0x1800-byte buffer, 142230FF5): a doubled `@@`
    /// is one stored `@` and fits the buffer's last byte, `é` is two and is
    /// cut, which the localiser refuses by name.
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
                &crate::combat::binding::NoIcons,
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
            // Only the name's reasons; the effect values' keys are absent.
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

    /// 141C5C1F0 passes the low 32 bits of the rounded percentage
    /// (141C5C521); an out-of-range chance is shown as the game shows it,
    /// not refused as a missing key.
    #[test]
    fn selection_chance_keeps_the_low_32_bits_of_the_rounded_percentage() {
        let loc: Localisation = [("TACTIC_SELECTION_CHANCE", "$CHANCE$%")]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        let definition = plain(true, true);
        let chance = |chance: i64| {
            let row = entry_values(
                &crate::combat::binding::NoIcons,
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
        // 2^31 percent does not fit the int argument: its low 32 bits are
        // i32::MIN.
        assert_eq!(chance(2_147_483_648_000), "-2147483648%");
    }

    /// Mixed-case keywords parse (installed_content::combat_tactics) and evaluate
    /// exactly as their lowercase spelling: trigger names, logical groups,
    /// `tooltip`, `base`, `modifier`, `factor` and `add`. A `FACTOR` inside
    /// a modifier is its factor, not an unknown trigger.
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
            // Keywords in mixed case, the combat modifier's name in capitals;
            // the tactic name, the phase names and yes/no keep their case.
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
        // 4 * 1.5 + 1; the second modifier's condition is false.
        assert_eq!(weights, [Ok(700_000), Ok(700_000)]);
    }

    /// 140473CB0 resolves the operand to the first terrain in database order
    /// whose name folds to it, else the null object; the combat's terrain is
    /// the null object exactly when its recorded +0x10 is clear.
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
        // The first case-folded match is `plains`, a different object from
        // a combat held on `Plains`.
        assert_eq!(
            run(1, b"Plains", terrains(), "is_fighting_in_terrain = PLAINS"),
            Ok(false)
        );
        assert_eq!(
            run(1, b"plains", terrains(), "is_fighting_in_terrain = PLAINS"),
            Ok(true)
        );
        // An unknown name stores the null object: it matches only a combat
        // whose terrain is the null object.
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

    /// A bool trigger's operand (14138D3D0): a scope selector by its token
    /// (14053E330) is refused by name; any other text is yes only when it
    /// is `yes` to its first NUL (1424B06E0), and a failed read is `no`: a
    /// mis-cased yes, an empty or quoted operand, a quoted selector name (token
    /// 0xF) and any other word.
    #[test]
    fn bool_trigger_operands_match_their_exact_text() {
        let detail = LandCombatDetailsRow::default();
        let content = content();
        let aliases = BTreeMap::new();
        let context = context(&detail, &content, &aliases);
        let evaluate = |text: &str| context.check(&trigger(text));
        // Side 0 is the attacker; the default side has no reserves.
        assert_eq!(evaluate("is_attacker = yes"), Ok(true));
        assert_eq!(evaluate("is_attacker = yes\0junk"), Ok(true));
        assert_eq!(evaluate("is_attacker = \"yes\""), Ok(true));
        assert_eq!(evaluate("IS_ATTACKER = YES"), Ok(false));
        assert_eq!(evaluate("is_attacker = No"), Ok(false));
        assert_eq!(evaluate("is_attacker = \"\""), Ok(false));
        assert_eq!(evaluate("is_attacker = \"ROOT\""), Ok(false));
        assert_eq!(evaluate("has_reserves = garbage"), Ok(true));
        assert_eq!(
            evaluate("is_attacker = Root"),
            Err("trigger is_attacker = Root: the operand Root is a scope selector keyword (14053E330), which the trigger's reader (14138D3D0) takes instead of yes or no; what the trigger then tests is not reproduced".into())
        );
        assert_eq!(
            evaluate("is_attacker = \"event_target:x\""),
            Err("trigger is_attacker = event_target:x: the operand event_target:x is an event target (14053E330), which the trigger's reader (14138D3D0) takes instead of yes or no; what the trigger then tests is not reproduced".into())
        );
    }

    /// Terrain names are lexer tokens (140473CB0) and sub-unit names are
    /// matched as has_unit_type matches them (140472610): both without case.
    #[test]
    fn terrain_and_sub_unit_names_match_without_case() {
        // The sub-unit database as installed_content::unit_leader::sub_units reads
        // it: `YES` fails read_bool_token (1424B06E0), so `odd` keeps `no`.
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
            context: replay::rows::LandDivisionContext {
                template: 9,
                ..Default::default()
            },
            width_terms: vec![
                replay::rows::LandDivisionWidthTerm {
                    subunit: 5,
                    count: 1,
                    width: 0,
                },
                replay::rows::LandDivisionWidthTerm {
                    subunit: 6,
                    count: 3,
                    width: 0,
                },
            ],
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
        // A quoted operand is token 0xF, no terrain's (140473CB0): the null
        // object, which a combat on a real terrain does not hold.
        assert_eq!(
            evaluate(&context, "is_fighting_in_terrain = \"plains\""),
            Ok(false)
        );
        assert_eq!(evaluate(&context, "has_unit_type = artillery"), Ok(true));
        // 1404739B0 hashes the operand as a C string (140BBA1B0).
        assert_eq!(evaluate(&context, "has_unit_type = ARTILLERY\0x"), Ok(true));
        // One artillery of four sub-units: 0.25.
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
        // A partial dictionary: an unported key's meaning is unknown.
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
        assert_eq!(evaluate("unported_trigger=yes is_attacker=no"), Ok(false));
        assert_eq!(
            evaluate("OR={ unported_trigger=yes is_attacker=yes }"),
            Ok(true)
        );
        assert!(evaluate("unported_trigger=yes is_attacker=yes").is_err());
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
        // A quoted operand is token 0xF, no combat-modifier keyword
        // (140473990 -> 141440AE0), even through a variable defined quoted.
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

    /// A tactic's weight through the real load (installed_content::combat_tactics)
    /// and evaluation paths: the attacker's side, country slot 1, and a
    /// tactic ID no side prefers. `file` is the whole tactic file; its first
    /// tactic is weighed.
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

    /// `has_tactic` interns its operand (1404A8120 -> add_dynamic_token
    /// 1424A6500, found in any case), then looks the token's stored spelling
    /// up byte-for-byte among the tactic names (14051F250 -> 1406B11F0): a
    /// miss is the active null tactic (1406B3BD0), so the trigger holds; an
    /// inactive tactic needs the country's unlock (1413B8CF0); a name two
    /// tactics share is refused by name.
    #[test]
    fn has_tactic_looks_up_the_interned_spelling() {
        let file = b"t = { trigger = { has_tactic = @operand } }\n\
                     Tactic_Foo = { active = no }\n\
                     other = { }\n\
                     twin = { } twin = { }\n";
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
            context.weight(&derived[0], definition)
        };
        let dictionary: [(u32, &[u8]); 5] = [
            (41, b"t"),
            (42, b"Tactic_Foo"),
            (43, b"other"),
            (44, b"twin"),
            (45, b"Missing_Tactic"),
        ];
        // The stored spelling is the first interned, found in any case.
        assert_eq!(weigh("TACTIC_FOO", &dictionary, &[2]), Ok(100_000));
        assert_eq!(weigh("TACTIC_FOO", &dictionary, &[]), Ok(0));
        // 1404A8120 interns the operand as a C string: to its first NUL.
        assert_eq!(weigh("TACTIC_FOO\0x", &dictionary, &[]), Ok(0));
        assert_eq!(weigh("OTHER", &dictionary, &[]), Ok(100_000));
        // No tactic of that spelling: the active null tactic.
        assert_eq!(weigh("missing_tactic", &dictionary, &[]), Ok(100_000));
        // An operand interned first as `tactic_foo` keeps that spelling,
        // which no tactic name matches byte-for-byte.
        let mut lowered = dictionary;
        lowered[1] = (42, b"tactic_foo");
        assert_eq!(weigh("Tactic_Foo", &lowered, &[]), Ok(100_000));
        let refused = |operand: &str, why: &str| {
            let reason = weigh(operand, &dictionary, &[]).unwrap_err();
            assert!(reason.contains(why), "{operand}: {reason}");
        };
        refused("twin", "more than one tactic is named twin");
        refused("absent", "absent is not in the recorded lexical dictionary");
    }

    /// `only_show_for` (141C5C1F0): the recorded country of the tag sees
    /// the tactic, another does not; a tag no tag list holds stores a link
    /// index that is not positive (140BA4CA0), so every country sees it;
    /// with a partial Countries a tag it lacks is refused by name.
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
        // The reader takes the block's `{` as the tag, which names none.
        assert_eq!(listed(&context, 2, 1), Ok(true));
        assert_eq!(listed(&context, 3, 1), Ok(true));
        context.countries = Err("Countries is not complete this hour".into());
        assert_eq!(listed(&context, 0, 1), Ok(false));
        assert_eq!(
            listed(&context, 1, 1),
            Err("only_show_for = XYZ names no recorded country, and Countries is not complete this hour".into())
        );
    }

    /// A block under a key that may make a content count trigger or a
    /// scripted trigger, which read one value token and skip no block
    /// (141394DF0, 141395A40, 1424ACD40), leaves the content read: the
    /// tactic holding it, in its trigger or a weight modifier, and every
    /// later one are unproven by name, and a scripted trigger's body holding
    /// one refuses the templates its file may have given a body.
    #[test]
    fn blocks_under_one_value_names_leave_the_content_read() {
        let mut buildings = installed_content::buildings::Buildings::default();
        buildings.types.insert(
            "arms_factory".into(),
            installed_content::buildings::BuildingType::default(),
        );
        let settled = |file: &[u8], scripted: ScriptedTriggers| {
            let mut content = parsed(file).with_inventories(
                Traits::default(),
                Ok(&SubUnits::default()),
                scripted,
            );
            content.set_count_names(CountTriggerNames::new(
                &buildings,
                &installed_content::ideologies::Ideologies::default(),
                &[],
            ));
            content
        };
        let content = settled(
            b"first = { trigger = { arms_factory = 2 } }\n\
              second = { base = { modifier = { OR = { ARMS_FACTORY = { x = 1 } } } } }\n\
              third = { }\n",
            ScriptedTriggers::default(),
        );
        assert!(content.registry[0].definition.is_ok());
        for index in [1, 2] {
            let reason = content.registry[index].definition.as_ref().err().unwrap();
            assert!(
                reason.contains(
                    "tactic second: trigger ARMS_FACTORY = { ... } names a building count trigger"
                ),
                "{reason}"
            );
        }
        // Equipment categories by their static spelling, unless the trigger
        // registry holds the token (`armor`, 0x2F85): that coded trigger's
        // class reads one value (ReadTriggerComparison 141394DF0), so its
        // block leaves the text as the game reads it, noted by the content
        // read, and the tactic is read as the game reads it.
        let content = settled(
            b"a = { trigger = { armor = { x = 1 } } }\n",
            ScriptedTriggers::default(),
        );
        assert!(content.registry[0].definition.is_ok());
        assert!(
            content.warnings().iter().any(|warning| {
                warning.contains("field `armor` is a block where its reader takes one value")
            }),
            "{:?}",
            content.warnings()
        );
        let content = settled(
            b"b = { trigger = { infantry = { x = 1 } } }\n",
            ScriptedTriggers::default(),
        );
        assert!(content.registry[0].definition.is_err());
    }

    /// A commander of skill 3 on the attacker's side, in phase 0.
    fn skilled() -> LandCombatDetailsRow {
        let mut detail = LandCombatDetailsRow::default();
        detail.sides[0].commander = 1;
        detail.sides[0].commander_skill = 3;
        detail
    }

    /// A modifier's `factor` and `add` are scoped variables
    /// (CScopedVariable::vf3 140539440 -> 140539B50), evaluated when the
    /// modifier holds (1405455A0): a constant of `-+0123456789.` text
    /// (ParseFixed5String 1424B7050), empty text setting nothing, and any
    /// other text a variable or scope read from the tactic scope's stores,
    /// which this fixture does not sample, so they are refused by name; each
    /// occurrence in order replaces the last. A block is one value token
    /// (`{`) with no block skipped: its contents are the modifier's
    /// conditions and its `}` ends the modifier.
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
        // ParseFixed5String, not `%i`: `010` is ten.
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
        // The runtime operand is read only when the modifier holds.
        assert_eq!(
            weight("modifier = { factor = var:weight is_attacker = no }"),
            Ok(100_000)
        );
    }

    /// The weight's CMeanTimeToHappen starts at 1 (140544B60); `factor` reads
    /// `%lli` (so `010` is eight), `base` a decimal, and days, months and
    /// years through ParseTokenInt32 (1424B0080, so `1.5` is one); the last
    /// field read stays, and every `base` block adds to the one object.
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
        // A quoted key is no keyword (token 0xF): the weight's reader logs
        // it as unknown (140545850).
        assert_eq!(weight("base = { \"factor\" = 3 }"), Ok(100_000));
    }

    /// Each nested reader consumes as the game's does, through weights: a
    /// scalar trigger reads nothing (1405431C0) and a scalar modifier is an
    /// empty one; a block where the weight's reader takes one value is read
    /// as `{` (ReadFixed5 fails and keeps the value), its contents as the
    /// weight's fields and its `}` ends the weight.
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

    /// 1424AC5E0 consumes definitions before the trigger's reader sees a key
    /// (1405431C0), so none is a condition; a comparison, spaced or compact,
    /// resolves its operand through the file's variables and, for `phase`,
    /// the phases read so far (140473850). CIntTrigger (14138D4E0) reads a
    /// bare number with `%i` (`010` is eight) and a quoted one as a
    /// ParseFixed5String constant (`"010"` is ten).
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
        // `phase > 99` names the phase `99`, ID 0: the combat's phase.
        assert_eq!(weight("phase > 99"), Ok(100_000));
        assert_eq!(weight("phase = close_combat"), Ok(0));
        // CIntTrigger::Evaluate (14138CC70) compares a scoped constant as
        // `(int)(value / 100000)`: `"3.5"` is 3 and `"2.9"` is 2.
        assert_eq!(weight("skill < \"3.5\""), Ok(0));
        assert_eq!(weight("skill > \"2.9\""), Ok(100_000));
    }

    /// Every count-trigger table recorded and empty.
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

    /// Country link 10 as slot 1 (tag TST), and the recorded lexical
    /// dictionary `tokens` (literal, token ID).
    fn countries(tokens: &[(&[u8], u32)]) -> Vec<(TableId, TableRows)> {
        vec![
            (
                TableId::CountryLinks,
                TableRows::from_rows::<tables::CountryLinks>(vec![replay::rows::CountryLinkRow {
                    id: 10,
                    canonical_slot: 1,
                }]),
            ),
            (
                TableId::Countries,
                TableRows::from_rows::<tables::Countries>(vec![replay::rows::CountryRow {
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

    /// Weighs the first tactic of `file` with the given inventories, the
    /// recorded lexical dictionary `tokens` (literal, token ID) and the
    /// recorded `tables`.
    fn weigh_with(
        file: &str,
        scripted: ScriptedTriggers,
        tokens: &[(&'static [u8], u32)],
        tables: Vec<(TableId, TableRows)>,
    ) -> Evaluation<i64> {
        let detail = skilled();
        let aliases = BTreeMap::from([(10, 1)]);
        let mut content = parsed(file.as_bytes()).with_inventories(
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

    /// A key the evaluator does not reproduce goes on as CTrigger::LoadKey
    /// (1405434F0) goes on. A quoted key (token 0xF, 1424A7A90) misses every
    /// lookup by token ID; a bare one may be a coded trigger (the registry,
    /// 140543E30) or a content count trigger. A scalar key naming no
    /// scripted trigger is "Invalid trigger": the game adds no child, so the
    /// condition is absent. With a block, the text is first a scope.
    #[test]
    fn keys_the_evaluator_does_not_reproduce_follow_the_trigger_name_fallbacks() {
        let none = || ScriptedTriggers::default();
        // Steel is resource 3 (ResourceDefinitions, 140AB2EC0); country 10
        // has 7 produced less 1 adjusted away (141390640).
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
        // Not the keyword: `is_attacker = no` would zero the attacker.
        assert_eq!(weigh("\"is_attacker\" = no"), Ok(100_000));
        assert_eq!(weigh("\"SKILL\" > 5"), Ok(100_000));
        assert_eq!(weigh("\"is_attacker\" = no is_attacker = no"), Ok(0));
        // A bare key the registry misses and no content names.
        assert_eq!(weigh("NOTHING = yes"), Ok(100_000));
        assert_eq!(
            weigh("if = yes"),
            Err("trigger if = yes: the conditional is not a block (CIfTrigger 141395B90)".into())
        );
        assert_eq!(weigh("Steel > 5"), Ok(100_000));
        assert_eq!(weigh("Steel > 6"), Ok(0));
        // A bare key a whole dictionary lacks is token 0xF (1424A7A90): no
        // lookup by token finds it, and naming no scripted trigger it is
        // "Invalid trigger", no child.
        assert_eq!(weigh("absent_word = yes"), Ok(100_000));
        assert_eq!(weigh("absent_word = { is_attacker = no }"), Ok(100_000));
        // No recorded country is GER here: no scope, no trigger, no child.
        assert_eq!(weigh("\"GER\" = { is_attacker = no }"), Ok(100_000));
        // Without the whole dictionary the miss proves nothing.
        let content = content();
        let (detail, aliases) = (LandCombatDetailsRow::default(), BTreeMap::new());
        let context = context(&detail, &content, &aliases);
        assert_eq!(
            context.check(&trigger("absent_word = yes")),
            Err("whether trigger absent_word is a token (the count lookups, 1405434F0, are keyed by token ID) is unknown: hour 1 LexicalTokens is unavailable: not sampled".into())
        );
    }

    /// Scripted triggers read through the real content stack
    /// (installed_content::scripted_triggers::read): a mod layer's file replaces the
    /// vanilla file of the same path; the name is found without case and
    /// whatever its quoting (140AA78C0); `= no` negates (14138D030); the
    /// body's variables and phase resolve; a cycle, which the game does not
    /// guard, is refused by name. A scripted trigger takes no parameters in
    /// this version: its value reader takes one yes/no token (14138D3D0).
    #[test]
    fn scripted_triggers_evaluate_their_body_in_the_current_scope() {
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
        let layers = installed_content::ResolvedLayers::from_folders(&[
            (&root.join("vanilla"), &[][..]),
            (&root.join("mod"), &[][..]),
        ])
        .unwrap();
        let scripted = installed_content::scripted_triggers::read(&layers).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        // `in_opening` is no token: a bare key the dictionary lacks reaches
        // the scripted triggers by its text (1405434F0 -> 140AA78C0).
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
        assert_eq!(
            weigh("cycle_a = yes"),
            Err("scripted trigger cycle_a: scripted trigger cycle_b: scripted trigger cycle_a calls itself (cycle_a -> cycle_b -> cycle_a): CScriptedTrigger::vf22 (14138D030) and CTrigger::Check (140540570) guard no depth, so the game recurses without end".into())
        );
        // A block is no scripted trigger's value: its reader takes one token
        // and skips no block, so the game's parse leaves the text, and the
        // content read refuses the tactic (and every later one).
        let unproven = weigh("my_check = { x = 1 }").unwrap_err();
        assert!(
            unproven.starts_with("tactic t's ID or fields cannot be read as the game reads them: tactic t: trigger my_check = { ... } names a scripted trigger"),
            "{unproven}"
        );
    }

    /// The content read classifies a block's key in CTrigger::LoadKey's
    /// order (1405434F0), as the evaluation does: a coded trigger by its
    /// static keyword, then a scope of its text, before the scripted
    /// triggers. So a scripted trigger named `AND` or `root` leaves
    /// `AND = { .. }` the coded group and `ROOT = { .. }` the scope. Every
    /// scripted body read is checked (140AA8DC0), a replaced one included:
    /// the other templates its file gave are refused, while the replaced
    /// template, read again in a later file, stands. A scalar group in a
    /// body is the empty group LoadKey keeps.
    #[test]
    fn content_blocks_follow_load_keys_order_in_every_body_read() {
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
            installed_content::ResolvedLayers::from_folders(&[(&root.join("vanilla"), &[][..])]).unwrap();
        let scripted = installed_content::scripted_triggers::read(&layers).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let weigh = |trigger: &str| {
            weigh_with(
                &format!("t = {{ trigger = {{ {trigger} }} }}"),
                scripted.clone(),
                &[],
                empty_tables(),
            )
        };
        // Side 0, the attacker, weighs.
        assert_eq!(weigh("AND = { is_attacker = yes }"), Ok(100_000));
        assert_eq!(weigh("ROOT = { is_attacker = yes }"), Ok(100_000));
        assert_eq!(weigh("bad = yes"), Ok(100_000));
        // An empty NOT is true (14053E070), an empty OR false (14053E0F0).
        assert_eq!(weigh("empty = yes"), Ok(100_000));
        assert_eq!(weigh("OR = no"), Ok(0));
        assert_eq!(weigh("custom_trigger_tooltip = x AND = yes"), Ok(100_000));
        // 141395B30 owns both binding keywords before LoadKey. The body
        // reader handles nested localisation arguments, never conditions;
        // neither the hazard pass nor evaluation may resolve these names
        // as the scripted triggers the content also defines.
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
        let refused = weigh("other = yes").unwrap_err();
        assert!(
            refused.starts_with(
                "scripted trigger other: scripted trigger bad's body in common/scripted_triggers/a.txt leaves the text: trigger other = { ... } names a scripted trigger"
            ) && refused.contains("unless its text is a scope, which LoadKey tries first"),
            "{refused}"
        );
    }

    /// `has_trait`, `skill` and `tag` in a character scope (CHasTraitTrigger
    /// 1403D7D20, CSkillTrigger 140475170, CTagTrigger 1403DCDF0): the
    /// scope's character, found through the recorded Characters rows
    /// (14146BB30), its unit leader (GetUnitLeader 140F8A840: CCharacter+0xA8
    /// in UnitLeaders, else the reference at +0xC8 in CharacterOperatives),
    /// its retired records (CountryRetiredCharacters) and its roles' traits
    /// (CharacterRoleTraits).
    #[test]
    fn character_scopes_read_the_recorded_character() {
        use replay::{
            frame::{Metrics, TableView},
            rows::{
                CharacterOperativeRow, CharacterRoleTraitRow, CharacterRow, CountryCharacterRow,
                CountryLinkRow, CountryRow, TableId, TableRows, UnitLeaderRow, UnitLeaderTraitRow,
            },
        };
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
        let layers = installed_content::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let traits = Traits::registered(&layers).unwrap();
        let country_leader_traits = Arc::new(installed_content::leader_traits::read(&layers).unwrap().0);
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
        // Name and trait tokens, in token order (dashboard's
        // character_script_token).
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
        // Definition ordinals (+0x8A4) follow append_trait's order after
        // the null object: the repeated `Trickster` is definition 3.
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
        // Country-leader definitions are ordinals 1 (popular_figurehead),
        // 2 (captain_of_industry) and 3 (the repeated Popular_Figurehead),
        // after the null object (1407112F0 -> 140710C80, +0x1A0).
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
                // Only the later definition of the name, with its token.
                role(3, CharacterRoleTraitRow::COUNTRY_LEADER, 1, 0x9001, Some(3)),
                role(3, CharacterRoleTraitRow::SCIENTIST, 0, 0x9003, None),
                // The name's token at an ordinal the content gives another
                // name (captain_of_industry).
                role(8, CharacterRoleTraitRow::COUNTRY_LEADER, 1, 0x9001, Some(2)),
            ]),
        );
        // Character 6 is an operative whose reference resolves; character
        // 7's does not, so it has no unit leader.
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
                },
                CharacterOperativeRow {
                    character: 7,
                    reference: (62 << 32) | 7,
                    ..Default::default()
                },
            ]),
        );
        // GER retired character 5, which has character 4's name token.
        set(
            TableId::CountryRetiredCharacters,
            TableRows::from_rows::<tables::CountryRetiredCharacters>(vec![CountryCharacterRow {
                country: 100,
                ordinal: 0,
                character: Some(5),
            }]),
        );

        // The attacker's commander is character 1's unit leader, holding
        // brilliant_strategist (definition 1) and the repeated Trickster
        // (definition 3), as LandCombatSides records them.
        let mut detail = skilled();
        let presentation = &mut detail.sides[0].presentation;
        presentation.character = (1 << 32) | 9;
        presentation.character_state = 2;
        presentation.character_role_leader = (50 << 32) | 7;
        presentation.role_traits = [(1, b"brilliant_strategist".as_slice()), (3, b"Trickster")]
            .map(
                |(definition_ordinal, token)| replay::rows::LandCombatCommanderTrait {
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
        // In the tactic scope, the combat's commander's character
        // (1403D7D20): the first definition of the name, by identity, among
        // the recorded role leader's traits; no retired gate (14052BC80
        // reads the scope's character, which the tactic scope lacks).
        assert_eq!(weigh("has_trait = Brilliant_Strategist"), Ok(100_000));
        assert_eq!(weigh("has_trait = trickster"), Ok(0));
        // A country-leader trait: none of character 1's roles holds it.
        assert_eq!(weigh("has_trait = popular_figurehead"), Ok(0));
        // No commander character: false before any trait lookup.
        let mut plain = skilled();
        assert_eq!(weigh_in(&plain, "has_trait = popular_figurehead"), Ok(0));
        plain.sides[0].presentation.character_state = 1;
        assert_eq!(weigh_in(&plain, "has_trait = trickster"), Ok(0));
        // The unit leader's skill 4, compared as CIntTrigger (14138CC70).
        assert_eq!(weigh("GER_leader = { skill > 3 }"), Ok(100_000));
        assert_eq!(weigh("GER_leader = { SKILL > 4 }"), Ok(0));
        assert_eq!(weigh("GER_leader = { skill = 4 }"), Ok(100_000));
        // Its assigned traits: the first definition of the name in any case
        // (140ADB590 -> 140AD3E30), compared by identity (1402EF030). The
        // leader holds the repeated `Trickster`, a later definition.
        assert_eq!(
            weigh("GER_leader = { has_trait = BRILLIANT_strategist }"),
            Ok(100_000)
        );
        assert_eq!(weigh("GER_leader = { has_trait = trickster }"), Ok(0));
        // No unit leader at either link: skill is 0, and a unit-leader trait
        // falls through to the other databases.
        assert_eq!(weigh("GER_politician = { skill > 0 }"), Ok(0));
        assert_eq!(weigh("GER_politician = { skill = 0 }"), Ok(100_000));
        assert_eq!(weigh("GER_politician = { has_trait = trickster }"), Ok(0));
        // A country-leader trait (140712D30, without case): a country-leader
        // or an advisor role holding it.
        assert_eq!(
            weigh("GER_politician = { has_trait = Popular_Figurehead }"),
            Ok(100_000)
        );
        assert_eq!(
            weigh("GER_politician = { has_trait = captain_of_industry }"),
            Ok(100_000)
        );
        // Character 3's leader role holds only the later definition of the
        // name, which the lookup (the first definition) does not match.
        assert_eq!(
            weigh("GER_scientist = { has_trait = popular_figurehead }"),
            Ok(0)
        );
        // Content that disagrees with the recorded ordinal is refused.
        let refused = weigh("GER_miscounted = { has_trait = popular_figurehead }").unwrap_err();
        assert!(refused.contains("do not give that name"), "{refused}");
        // Any other name: the scientist's traits, by token (14022F320),
        // whether or not a roster holds the scientist.
        assert_eq!(
            weigh("GER_scientist = { has_trait = brilliant_scientist }"),
            Ok(100_000)
        );
        assert_eq!(
            weigh("GER_politician = { has_trait = brilliant_scientist }"),
            Ok(0)
        );
        // GER's retired record has character 4's name token: retired, so
        // false although its unit leader holds the trait; skill has no
        // retirement gate.
        assert_eq!(weigh("GER_returned = { has_trait = trickster }"), Ok(0));
        assert_eq!(weigh("GER_returned = { skill > 3 }"), Ok(100_000));
        // An operative (category 3) is not gated, and its referenced unit
        // leader's traits and skill decide.
        assert_eq!(
            weigh("GER_operative = { has_trait = trickster }"),
            Ok(100_000)
        );
        assert_eq!(weigh("GER_operative = { skill > 1 }"), Ok(100_000));
        // A character scope holds no country tag (SetCharacter 14052DAF0
        // clears +8), so `tag = THIS` there is country link 0: false.
        assert_eq!(weigh("GER_leader = { tag = THIS }"), Ok(0));
        // `tag = ROOT` compares the unit leader's country (CUnitLeader+0x120)
        // first, else the character's own (+0x68).
        assert_eq!(weigh("GER_leader = { tag = ROOT }"), Ok(100_000));
        assert_eq!(weigh("GER_operative = { tag = ROOT }"), Ok(100_000));
        assert_eq!(weigh("GER_politician = { tag = ROOT }"), Ok(100_000));
        // A reference the registry does not hold is no unit leader.
        assert_eq!(weigh("GER_lost_agent = { tag = ROOT }"), Ok(100_000));
        assert_eq!(weigh("GER_lost_agent = { skill = 0 }"), Ok(100_000));
    }

    /// Scope keys (CContextTrigger, 14138C3A0): the key's text parsed as the
    /// event target reader parses it (installed_content::scope::parse, 14146DB00),
    /// resolved over the recorded hour (dashboard::scope, 14146BB30), and
    /// the block checked there (14138F160). ROOT, FROM and PREV of a tactic
    /// trigger are the tactic scope itself (1405286D0), the combat included;
    /// a country target carries no combat (set_country 14052DD00 ->
    /// 14052F3F0). A tag whose country owns no state is substituted from
    /// the original-tag lists (1401DA520), which dashboard::scope derives
    /// from CountryAppearance; without it the tag is refused by name.
    #[test]
    fn scope_keys_check_their_block_in_the_resolved_scope() {
        use replay::{
            frame::{Metrics, TableView},
            rows::{CountryCounterInputRow, CountryLinkRow, CountryRow, TableId, TableRows},
        };
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|_| TableView::Unavailable {
                reason: "not sampled".into(),
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        let countries = [(100, 0, "GER", 5), (200, 1, "SOV", 3), (300, 2, "ENG", 0)];
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
                            country,
                            owned_state_count,
                            ..Default::default()
                        },
                    )
                    .into(),
            ),
        );
        // Every walked store has a row; GER's `x` holds GER's own scope
        set(
            TableId::ScriptVariables,
            TableRows::ScriptVariables(
                std::iter::once((0, 0))
                    .chain(countries.map(|(country, ..)| (1, country)))
                    .map(|(scope_kind, scope_id)| replay::rows::ScriptVariablesRow {
                        scope_kind,
                        scope_id,
                        entries: if scope_id == 100 {
                            vec![replay::rows::ScriptVariableEntry {
                                name: b"x".to_vec(),
                                value_raw: 0x4052_4547,
                            }]
                        } else {
                            Vec::new()
                        },
                    })
                    .collect(),
            ),
        );
        let detail = skilled();
        let aliases = BTreeMap::from([(100, 0), (200, 1), (300, 2)]);
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
        let weigh = |trigger: &str| {
            let content = parsed(format!("t = {{ trigger = {{ {trigger} }} }}").as_bytes());
            let mut context = context_in(&detail, &content, &aliases, &frame);
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
        assert_eq!(weigh("GER = { tag = GER }"), Ok(100_000));
        assert_eq!(weigh("SOV = { tag = GER }"), Ok(0));
        // A quoted key's text is still a tag (steps 1-3 read the text).
        assert_eq!(weigh("\"GER\" = { tag = GER }"), Ok(100_000));
        // OWNER of the tactic scope is the side's first country.
        assert_eq!(weigh("owner = { tag = GER }"), Ok(100_000));
        // ROOT is the tactic scope, the combat included; a dotted chain
        // resolves its head, then the rest from there.
        assert_eq!(weigh("ROOT = { is_attacker = yes }"), Ok(100_000));
        assert_eq!(weigh("ROOT.owner = { tag = GER }"), Ok(100_000));
        // PREV inside a tag scope is the calling scope: the tactic scope.
        assert_eq!(weigh("SOV = { PREV = { is_attacker = yes } }"), Ok(100_000));
        // A country target carries no combat, so the combat trigger's scope
        // check (mask 0x10, 140542CA0) fails and it is false.
        assert_eq!(weigh("GER = { is_attacker = yes }"), Ok(0));
        assert_eq!(weigh("GER = { NOT = { is_attacker = yes } }"), Ok(100_000));
        assert_eq!(
            weigh("ENG = { tag = ENG }"),
            Err("trigger ENG = { ... }: ENG owns no state, and the countries with it as original tag are unknown: CountryAppearance is incomplete since hour 1".into())
        );
        // A `var:` key decodes the stored value (1405382B0 -> 140537A10).
        assert_eq!(weigh("var:x = { tag = GER }"), Ok(100_000));
        assert_eq!(weigh("var:missing = { tag = GER }"), Ok(0));
        let state = weigh("64 = { tag = GER }").unwrap_err();
        assert!(
            state.starts_with("trigger 64 = { ... }: state 64 has no recorded States row"),
            "{state}"
        );
        // `tag`'s operand (141394CA0, 1413928A0): a scope selector by token
        // (14053E330) reads that scope's country; a tag resolves as a scope
        // key's does (140BA5D10); anything else is a scoped variable decoded
        // to its country (EvaluateCountryTag 140536ED0).
        assert_eq!(weigh("tag = ROOT"), Ok(100_000));
        // The tag lookup takes the C string (141394CA0); bytes after its
        // first NUL cannot turn an existing tag into a variable.
        assert_eq!(weigh("tag = \"GER\0suffix\""), Ok(100_000));
        assert_eq!(weigh("tag = \"SOV\0suffix\""), Ok(0));
        assert_eq!(weigh("SOV = { tag = THIS }"), Ok(100_000));
        assert_eq!(weigh("SOV = { tag = prev }"), Ok(0));
        let refused = |trigger: &str, why: &str| {
            let reason = weigh(trigger).unwrap_err();
            assert!(reason.contains(why), "{trigger}: {reason}");
        };
        // owner, controller and occupied read THIS's state, which a scope
        // `tag` accepts never holds, and 140542000 tests no bit for capital
        // or random: each is country link 0, false (1403DCDF0).
        for selector in ["owner", "CONTROLLER", "occupied", "capital", "random"] {
            assert_eq!(weigh(&format!("tag = {selector}")), Ok(0), "{selector}");
        }
        refused("tag = event_target:foe", "an event target (14053E330)");
        // A quoted ROOT is no selector: as a variable it names ROOT's scope.
        assert_eq!(weigh("tag = \"ROOT\""), Ok(100_000));
        assert_eq!(weigh("tag = x"), Ok(100_000));
        // XYZ is no tag: the variable `xyz`, which GER does not hold, is 0.
        assert_eq!(weigh("tag = XYZ"), Ok(0));
        refused("tag = ENG", "ENG owns no state");
    }

    /// Content count triggers (CTrigger::LoadKey 1405434F0, steps 3 to 6):
    /// building and ideology names from the databases read through the
    /// content stack (a mod adds a provincial building and an ideology),
    /// the resource from ResourceDefinitions (a state's amount with the
    /// building's content grant, crate::state_resources), each evaluated over the
    /// recorded hour as its class's vf23 does and compared as CIntTrigger
    /// (14138CC70) or CValueTrigger (14138D120). Each class's scope mask
    /// (vtable +0x70) is checked first (140542CA0): a scope it does not
    /// accept makes it false.
    #[test]
    fn count_triggers_read_the_recorded_hour_in_the_scope_they_accept() {
        use replay::{
            frame::{Metrics, TableView},
            rows::{
                CountryBuildingResourceRow, CountryCounterInputRow, CountryLinkRow, CountryRow,
                StateBaseResourceRow, StateResourceInputRow, StateResourceRow, TableId, TableRows,
            },
        };
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
        // CBuildingTemplate reads `local_resources_<name>` only for a
        // registered resource.
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
        let layers = installed_content::ResolvedLayers::from_folders(&[
            (&root.join("vanilla"), &[][..]),
            (&root.join("mod"), &[][..]),
        ])
        .unwrap();
        let buildings = installed_content::buildings::read(&layers).unwrap();
        let (ideologies, _) = installed_content::ideologies::read(&layers).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let armor = installed_content::trigger_keys::EQUIPMENT_CATEGORIES[0].0;
        let tokens: &[(&[u8], u32)] = &[
            (b"GER", 0x7001),
            (b"mod_mill", 0x7101),
            (b"arms_factory", 0x7102),
            (b"steel", 0x7103),
            (b"technocracy", 0x7104),
            (b"fascism", 0x7105),
            (b"land_cruiser", armor),
            (b"nothing", 0x7106),
        ];
        for (name, _) in &tokens[..tokens.len() - 1] {
            assert_eq!(installed_content::trigger_keys::registered_trigger(name), None);
        }

        // GER (raw link 100) owns states 64 and 65 and provinces 1 and 3;
        // SOV (200) owns state 66 and province 2, which lies in state 65.
        // CountryResources is hourly; ResourceDefinitions is carried from
        // its daily slot, so hour 5 reads the census it carries.
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
            let countries = [(100, 0, "GER", 2), (200, 1, "SOV", 1)];
            set(
                TableId::CountryLinks,
                TableRows::CountryLinks(
                    countries
                        .map(|(id, canonical_slot, _, _)| CountryLinkRow { id, canonical_slot })
                        .into(),
                ),
            );
            // At hour 9 the walk missed link 999, which owns state 67 and a
            // party.
            let lost = hour == 9;
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
                    // State 68 has no owner (link 0).
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
                // Province 4 has no owner (link 0).
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
            // State 64 (GER's, controlled by it): template 10 plus pool 2.5,
            // an ungated additive, minus cost 0.5, plus its three arms
            // factories' grants of 2 steel each, which GER's override of -1
            // makes 1 each: 15 whole units. State 65 has no inputs row.
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
            // Technocracy holds a fifth of GER's popularity.
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
        let aliases = BTreeMap::from([(100, 0), (200, 1)]);
        // The attacking side's four units: two of the category, one of
        // another, one of none (LandCombatDivisions.categories).
        let (armor_mask, other) = (installed_content::trigger_keys::EQUIPMENT_CATEGORIES[0].1, 0x20);
        let units = [(1, armor_mask), (2, armor_mask | other), (3, other), (4, 0)].map(
            |(division, categories)| LandCombatDivisionRow {
                division,
                categories,
                ..Default::default()
            },
        );
        let participants = [1, 2, 3, 4];
        let weigh_with_units = |hour, participants: &[u64], trigger: &str| {
            let frame = frame_at(hour);
            let mut content = parsed(format!("t = {{ trigger = {{ {trigger} }} }}").as_bytes());
            content.set_count_names(CountTriggerNames::new(&buildings, &ideologies, &[]));
            let facts = ContentFacts {
                buildings: &buildings,
                resource_bonuses: Ok([15_000, 20_000]),
                ..facts(&content)
            };
            let mut context = context_with(&detail, &content, &aliases, &frame, facts);
            context.first_countries[0] = Some(100);
            context.tokens = tokens.iter().map(|(literal, id)| (*id, *literal)).collect();
            context.country_tags = BTreeMap::from([(b"GER".as_slice(), 0)]);
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

        // Buildings (14138FEB0): the tactic scope's country sums its owned
        // states' state building (3 + 2) and its owned provinces' province
        // building (1 + 2); a state sums its own and its provinces'.
        assert_eq!(weigh("arms_factory > 4"), Ok(100_000));
        assert_eq!(weigh("arms_factory > 5"), Ok(0));
        assert_eq!(weigh("ARMS_FACTORY = 5"), Ok(100_000));
        assert_eq!(weigh("mod_mill = 3"), Ok(100_000));
        assert_eq!(weigh("GER = { Mod_Mill < 4 }"), Ok(100_000));
        assert_eq!(weigh("64 = { arms_factory = 3 }"), Ok(100_000));
        assert_eq!(weigh("65 = { mod_mill = 4 }"), Ok(100_000));
        // A scoped constant operand compares by its integer part.
        assert_eq!(weigh("arms_factory = \"5.9\""), Ok(100_000));

        // Resources (141390640): produced plus adjustment, 13 whole units,
        // at any hour (CountryResources is hourly).
        assert_eq!(weigh("steel > 12"), Ok(100_000));
        assert_eq!(weigh("steel > 13"), Ok(0));
        assert_eq!(weigh_at(hour + 1, "steel > 12"), Ok(100_000));
        // A state scope (1409CC730): 15 whole units, compared by
        // CIntTrigger::Evaluate (14138CC70).
        assert_eq!(weigh("64 = { steel > 14 }"), Ok(100_000));
        assert_eq!(weigh("64 = { STEEL > 15 }"), Ok(0));
        assert_eq!(weigh("64 = { Steel = 15 }"), Ok(100_000));
        assert_eq!(weigh("64 = { steel < 16 }"), Ok(100_000));
        assert_eq!(
            weigh("65 = { steel > 0 }"),
            Err("trigger steel > 0: a resource count trigger (CResourceCountTrigger 14138C730): state 65 has no recorded resource inputs".into())
        );

        // Ideology support (141390530): 20000 * 100000 / 100000, fixed5,
        // `=` compared as `>` (14138D120); a state scope is outside mask 4.
        assert_eq!(weigh("technocracy > 0.1"), Ok(100_000));
        assert_eq!(weigh("Technocracy > 0.2"), Ok(0));
        assert_eq!(weigh("technocracy = 0.1"), Ok(100_000));
        assert_eq!(weigh("technocracy < \"0.25\""), Ok(100_000));
        assert_eq!(weigh("fascism > 0.59"), Ok(100_000));
        assert_eq!(weigh("64 = { technocracy > 0.1 }"), Ok(0));

        // Equipment categories (141390370): the share of the side's units
        // whose stat block mask meets the category, 2 of 4, fixed5 and
        // compared as CValueTrigger (`=` as `>`); they need the combat (mask
        // 0x10), so a country scope makes them false.
        assert_eq!(weigh("land_cruiser > 0.4"), Ok(100_000));
        assert_eq!(weigh("LAND_CRUISER > 0.5"), Ok(0));
        // `armor` is a category too, but the trigger registry holds its
        // static keyword (0x2F85), which LoadKey finds first.
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
        assert_eq!(
            weigh_with_units(hour, &[1, 9], "land_cruiser > 0"),
            Err("trigger land_cruiser > 0: an equipment category trigger (CEquipmentTypeTrigger 14138C550): attacker division 9 has no LandCombatDivisions row".into())
        );

        // The other scope masks: `tag` (0x1C) is false in a state scope.
        assert_eq!(weigh("64 = { tag = GER }"), Ok(0));
        assert_eq!(weigh("GER = { tag = GER }"), Ok(100_000));
        // A bare key naming nothing is "Invalid trigger": no child.
        assert_eq!(weigh("nothing = yes"), Ok(100_000));
        // `tag = PREV` where PREV is a state: its owner
        // (GetCountryTagOrStateOwner 14052B6A0), through the shared
        // resolver; an ownerless state is link 0, false.
        assert_eq!(weigh("64 = { GER = { tag = PREV } }"), Ok(100_000));
        assert_eq!(weigh("66 = { GER = { tag = prev } }"), Ok(0));
        assert_eq!(weigh("66 = { SOV = { tag = PREV } }"), Ok(100_000));
        assert_eq!(weigh("68 = { GER = { tag = PREV } }"), Ok(0));
        assert_eq!(weigh("GER = { tag = ROOT }"), Ok(100_000));
        assert_eq!(weigh("SOV = { tag = FROM }"), Ok(0));

        // An owner link the hour's CountryLinks lacks is unknown, not
        // another country (canonical_slot 140BA6240): the sums refuse it by
        // name instead of shrinking.
        for (trigger, why) in [
            (
                "arms_factory > 4",
                "the owner of state 67: country link 999",
            ),
            (
                "technocracy > 0.1",
                "PoliticalParties row: country link 999",
            ),
            (
                "64 = { steel > 0 }",
                "building overrides (CountryBuildingResources) join through CountryLinks",
            ),
        ] {
            let reason = weigh_at(9, trigger).unwrap_err();
            assert!(
                reason.contains(why) && reason.contains("CountryLinks is truncated"),
                "{trigger}: {reason}"
            );
        }
    }

    /// A block under a single-value trigger is taken as its value token and
    /// not skipped (141395A40 / 141394DF0, 1424ACD40): the trigger reads
    /// `{` (no yes, so `no`), the block's contents are read as the
    /// enclosing trigger's conditions and its `}` ends that trigger
    /// (installed_content::combat_tactics). Later tactics are read as the game reads
    /// them.
    #[test]
    fn a_block_under_a_single_value_trigger_refuses_the_rest_of_the_file() {
        let detail = skilled();
        // The attacker is no `no`: the AND is false.
        assert_eq!(
            weight_of(
                "t = { trigger = { Is_Attacker = { skill > 2 } } } u = { }",
                &detail
            ),
            Ok(0)
        );
        // `skill > {` reads `{` through `%i`, which matches nothing: the
        // comparison's operand is no number, and `2` is its sibling.
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
        // A quoted key is no keyword, so its block is no single value: its
        // text names no scope, trigger or scripted trigger here, so the game
        // logs "Invalid trigger", skips the block (1424ACF90 -> 1424AD270)
        // and adds no child.
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

    /// The tactic readers ask [`Item::keyword`] whether a key is a keyword
    /// (1424A7A90: a quoted key is token 0xF, a bare one folds). A raw key
    /// comparison outside the tests fails here; each value compare allowed
    /// below names the address that folds it.
    #[test]
    fn tactic_readers_ask_the_keyword_helper() {
        const RAW: [&str; 6] = [
            "key.as_deref() ==",
            "key.as_deref() !=",
            "eq_ignore_ascii_case(",
            "to_ascii_lowercase(",
            ".get(b\"",
            "get_all(",
        ];
        const ALLOWED: [&str; 9] = [
            // Sub-unit names: the recorded token is interned without case
            // (1424A7A90) and has_unit_type compares with `_stricmp`
            // (140472610).
            "match self.artillery.entry(name.to_ascii_lowercase()) {",
            ".get(&name.to_ascii_lowercase())",
            "if term.1.eq_ignore_ascii_case(operand) {",
            // Tactic names interned into the lexer without case (1406B0A70
            // -> 1424A7A90).
            ".filter(|row| row.literal.eq_ignore_ascii_case(&entry.token));",
            ".is_some_and(|literal| literal.eq_ignore_ascii_case(NULL_TACTIC_NAME));",
            // A bool operand's other spellings of yes or no (14138D3D0).
            "_ if operand.eq_ignore_ascii_case(\"yes\") || operand.eq_ignore_ascii_case(\"no\") => Ok(false),",
            // A terrain operand's lexer token (140473CB0).
            ".find(|name| !item.quoted() && name.eq_ignore_ascii_case(bytes));",
            // The lexer's lookup of a bare token through a lowercased copy
            // (1424A7A90), over the recorded dictionary.
            ".filter(|(_, literal)| literal.eq_ignore_ascii_case(text))",
            // A combat-modifier operand's keyword ID (140473990 -> 141440AE0).
            "if operand.eq_ignore_ascii_case(\"river_crossing\") {",
        ];
        let code = include_str!("tactics.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default();
        let raw: Vec<_> = code
            .lines()
            .map(str::trim)
            .filter(|line| {
                !line.starts_with("//")
                    && RAW.iter().any(|pattern| line.contains(pattern))
                    && !ALLOWED.contains(line)
            })
            .collect();
        assert!(raw.is_empty(), "raw key comparisons: {raw:#?}");
    }
}
