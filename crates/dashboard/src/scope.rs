//! Script scope targets resolved over one recorded hour.
//!
//! [`installed_content::scope::parse`] reads a scope as the game's event target
//! reader does (14146DB00); this resolves it against what the recording
//! holds. Whatever the recording cannot reach is refused by name with the
//! missing input, never guessed. The viewer and the dashboard both reach it
//! (the viewer depends on the dashboard), behind [`RecordedScopes::resolve`].

use installed_content::scope::{Keyword, Names, Scope, ScopeKind};
use installed_content::tag_aliases::{TagAlias, TagAliases};
use replay::{
    frame::{DecodedFrame, TableView},
    rows::{TableSchema, tables},
};
use std::{collections::BTreeMap, sync::Arc};

mod original_tags;
pub use original_tags::{Group, OriginalTagLists};

/// A resolved scope target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A country by its canonical slot.
    Country(u32),
    /// A state by its id.
    State(u32),
    /// A character by its recorded id.
    Character(u32),
}

impl Target {
    /// The scope kind the target is, for [`installed_content::scope::TriggerScopes`]
    /// (140542CA0). A scope set with the clearing flag (set_country or
    /// SetState with 1, ClearCarriedTargets 14052F3F0) carries only this
    /// kind; a caller whose native path keeps other carried targets passes
    /// them itself.
    pub fn kind(self) -> ScopeKind {
        match self {
            Self::Country(_) => ScopeKind::Country,
            Self::State(_) => ScopeKind::State,
            Self::Character(_) => ScopeKind::Character,
        }
    }
}

/// Why a scope resolves to no target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unresolved {
    /// The scope names nothing in the recorded facts, as the game's scope is
    /// null there: a state whose owner or controller link is 0, a country
    /// with no overlord or capital, a scope the context does not set, a
    /// zero number or text the event target reader does not accept. Not a
    /// refusal.
    Nothing(String),
    /// A recorded input the scope needs is missing or incomplete (a table
    /// the hour lacks, a link CountryLinks does not hold) or not recorded at
    /// all: a refusal, naming the input.
    Missing(String),
}

impl std::fmt::Display for Unresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Nothing(what) | Self::Missing(what) => f.write_str(what),
        }
    }
}

/// The scopes a script or text is evaluated in. `None` is a scope the
/// caller's context does not set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub root: Option<Target>,
    pub this: Option<Target>,
    pub prev: Option<Target>,
    pub from: Option<Target>,
}

impl Context {
    /// A country as ROOT and THIS, as a country context sets them.
    pub fn country(slot: u32) -> Self {
        Self {
            root: Some(Target::Country(slot)),
            this: Some(Target::Country(slot)),
            ..Self::default()
        }
    }
}

/// The recorded facts scopes resolve through, read once per hour.
#[derive(Clone, Debug, Default)]
pub struct RecordedScopes {
    /// Raw country link to canonical slot (`CountryLinks`).
    links: BTreeMap<u32, u32>,
    /// Canonical slot to tag, and back (`Countries`).
    tags: BTreeMap<u32, String>,
    /// Whether `Countries` is complete, so `tags` is every country.
    countries_complete: bool,
    slots: BTreeMap<String, u32>,
    /// State id to raw owner and controller links (`States`).
    states: BTreeMap<u32, (u32, u32)>,
    /// Canonical slot to capital state (`Politics.capital`).
    capitals: BTreeMap<u32, i32>,
    /// Canonical slot to raw overlord link (`CountryAppearance.overlord`).
    overlords: BTreeMap<u32, i32>,
    /// Character script token, folded as the lexer folds a token (the
    /// lookup is by token ID), to its ids with their countries' slots
    /// (`Characters.name_token_id` through `LexicalTokens`).
    characters: BTreeMap<String, Vec<(u32, Option<u32>)>>,
    /// Whether `Characters` and `LexicalTokens` are complete, so a token
    /// no row has names no live character.
    characters_complete: bool,
    /// Canonical slot to owned state count (CCountry+0x484,
    /// `CountryCounterInputs.owned_state_count`).
    owned: BTreeMap<u32, i32>,
    /// `_CountriesForOriginalTags` (CGameState+0x988) as derived.
    original_tags: OriginalTagLists,
    /// The content's tag alias database (143316AA8).
    aliases: Arc<TagAliases>,
    saved_targets: Vec<replay::rows::GlobalSavedEventTargetRow>,
    saved_targets_complete: bool,
    /// Why a table's rows may lack an entry, by table name.
    gaps: BTreeMap<&'static str, String>,
}

/// A table's rows, and why an entry it lacks is unknown.
fn table<T: TableSchema>(frame: &DecodedFrame) -> (&[T::Row], String) {
    match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) => (
            rows.rows::<T>().unwrap_or_default(),
            "the complete table has no row for it".into(),
        ),
        TableView::Truncated { rows, missing } => (
            rows.rows::<T>().unwrap_or_default(),
            format!("the table is truncated ({missing} rows missing)"),
        ),
        TableView::Unavailable { reason } => (&[], format!("the table is unavailable: {reason}")),
    }
}

/// CSavedEventTarget::vf9@14052BE50, using captured object liveness.
pub(crate) fn valid_saved_target(r: &replay::rows::GlobalSavedEventTargetRow) -> bool {
    !r.name.is_empty()
        && (r.state != 0
            || r.country > 0
            || r.live & !replay::rows::GlobalSavedEventTargetRow::UNSAVED_NAME != 0)
}

impl RecordedScopes {
    /// Reads the hour's scope facts; a table the hour lacks leaves its
    /// scopes refused with the table's state. The original-tag lists are
    /// this frame's alone, their order unknown ([`OriginalTagLists::unordered`]);
    /// [`Self::read_with`] takes them derived from the branch. `aliases`
    /// is the content's tag alias database (143316AA8).
    pub fn read(frame: &DecodedFrame, aliases: Arc<TagAliases>) -> Self {
        Self::read_with(frame, OriginalTagLists::unordered(frame), aliases)
    }

    /// [`Self::read`] with the original-tag lists derived for this hour
    /// ([`OriginalTagLists::derive`]).
    pub fn read_with(
        frame: &DecodedFrame,
        original_tags: OriginalTagLists,
        aliases: Arc<TagAliases>,
    ) -> Self {
        let mut gaps = BTreeMap::new();
        let (links, gap) = table::<tables::CountryLinks>(frame);
        gaps.insert("CountryLinks", gap);
        let links: BTreeMap<u32, u32> = links
            .iter()
            .map(|link| (link.id, link.canonical_slot))
            .collect();
        let countries_complete = matches!(
            &*frame.tables[tables::Countries::ID.index()],
            TableView::Complete(_)
        );
        let (countries, gap) = table::<tables::Countries>(frame);
        gaps.insert("Countries", gap);
        let tags: BTreeMap<u32, String> = countries
            .iter()
            .map(|country| {
                (
                    country.canonical_slot,
                    String::from_utf8_lossy(&country.tag).into_owned(),
                )
            })
            .collect();
        let slots = tags
            .iter()
            .map(|(slot, tag)| (tag.clone(), *slot))
            .collect();
        let (states, gap) = table::<tables::States>(frame);
        gaps.insert("States", gap);
        let states = states
            .iter()
            .map(|state| (state.id, (state.owner, state.controller)))
            .collect();
        let (politics, gap) = table::<tables::Politics>(frame);
        gaps.insert("Politics", gap);
        let capitals = politics
            .iter()
            .filter_map(|row| Some((*links.get(&row.country)?, row.capital)))
            .collect();
        let (appearance, gap) = table::<tables::CountryAppearance>(frame);
        gaps.insert("CountryAppearance", gap);
        let overlords = appearance
            .iter()
            .filter_map(|row| Some((*links.get(&row.country)?, row.overlord)))
            .collect();
        let (counters, gap) = table::<tables::CountryCounterInputs>(frame);
        gaps.insert("CountryCounterInputs", gap);
        let owned = counters
            .iter()
            .filter_map(|row| Some((*links.get(&row.country)?, row.owned_state_count)))
            .collect();
        // A character's script token is its name token id spelled by the
        // hour's LexicalTokens (`Characters.token` is its saved name).
        let characters_complete = [tables::Characters::ID, tables::LexicalTokens::ID]
            .iter()
            .all(|id| matches!(&*frame.tables[id.index()], TableView::Complete(_)));
        let (characters, gap) = table::<tables::Characters>(frame);
        let (lexical, lexical_gap) = table::<tables::LexicalTokens>(frame);
        gaps.insert(
            "Characters",
            format!("{gap} (LexicalTokens: {lexical_gap})"),
        );
        let mut by_token: BTreeMap<String, Vec<(u32, Option<u32>)>> = BTreeMap::new();
        for row in characters {
            if let Some(token) = crate::products::identity::character_script_token(row, lexical) {
                by_token
                    .entry(token.to_ascii_lowercase())
                    .or_default()
                    .push((row.id, links.get(&row.country).copied()));
            }
        }
        let (saved_targets, gap) = table::<tables::GlobalSavedEventTargets>(frame);
        gaps.insert("GlobalSavedEventTargets", gap);
        let saved_targets_complete = matches!(
            &*frame.tables[tables::GlobalSavedEventTargets::ID.index()],
            TableView::Complete(_)
        );
        Self {
            saved_targets: saved_targets.to_vec(),
            saved_targets_complete,
            links,
            tags,
            countries_complete,
            slots,
            states,
            capitals,
            overlords,
            characters: by_token,
            characters_complete,
            owned,
            original_tags,
            aliases,
            gaps,
        }
    }

    /// FindGlobalSavedEventTarget@1401DB7B0 returns the first name match,
    /// even when that record is invalid (14052BE50); no later duplicate wins.
    pub(crate) fn saved_target(
        &self,
        name: &[u8],
    ) -> Result<Option<&replay::rows::GlobalSavedEventTargetRow>, String> {
        if !self.saved_targets_complete {
            return Err(format!(
                "saved event target {}: GlobalSavedEventTargets {}",
                String::from_utf8_lossy(name),
                self.gaps
                    .get("GlobalSavedEventTargets")
                    .map(String::as_str)
                    .unwrap_or("is not recorded")
            ));
        }
        Ok(self.saved_targets.iter().find(|r| r.name == name))
    }

    pub(crate) fn saved_selection(
        &self,
        name: &[u8],
        reader: crate::triggers::saved_event_targets::Reader,
    ) -> Result<crate::triggers::saved_event_targets::Selected, String> {
        use crate::triggers::saved_event_targets::{Selected, select};
        Ok(self
            .saved_target(name)?
            .map_or(Selected::Missing, |r| select(r, reader)))
    }

    pub(crate) fn saved_country(&self, raw: i32, name: &[u8]) -> Result<Option<u32>, String> {
        if raw <= 0 {
            return Ok(None);
        }
        self.slot(raw as u32).map(Some).ok_or_else(|| {
            format!(
                "saved event target {}: country reference {raw} is not in CountryLinks",
                String::from_utf8_lossy(name)
            )
        })
    }

    /// The country a tag scope names, as ResolveScriptCountryTag
    /// (140BA5D10) resolves it: the tag's own country when it owns a state
    /// (CCountry+0x484); otherwise the first country of
    /// `_CountriesForOriginalTags` (CGameState+0x988, 1401DA520) for that
    /// tag that owns one, else the tag's own country. The list is derived
    /// ([`OriginalTagLists`]); where its order is unknown and two
    /// state-owning candidates share the first such group, it is refused
    /// by name with the tag, the interval and the candidates.
    pub fn resolve_tag(&self, tag: &str) -> Result<u32, String> {
        if let Some(refusal) = self.alias(tag) {
            return Err(refusal);
        }
        let gap = |table: &str| self.gaps.get(table).cloned().unwrap_or_default();
        let slot = self.country(tag).ok_or_else(|| {
            format!(
                "country {tag} has no recorded Countries row: {}",
                gap("Countries")
            )
        })?;
        let owned = |slot: u32| {
            self.owned.get(&slot).copied().ok_or_else(|| {
                format!(
                    "the owned state count (CountryCounterInputs.owned_state_count, CCountry+0x484) of {} is not recorded: {}",
                    self.describe(Target::Country(slot)),
                    gap("CountryCounterInputs")
                )
            })
        };
        if owned(slot)? >= 1 {
            return Ok(slot);
        }
        if let Some(since) = self.original_tags.pending() {
            return Err(format!(
                "{tag} owns no state, and the countries with it as original tag are unknown: CountryAppearance is incomplete since hour {since}"
            ));
        }
        for group in self.original_tags.list(tag) {
            let mut owning = Vec::new();
            for member in &group.members {
                if owned(*member)? >= 1 {
                    owning.push(*member);
                }
            }
            match owning.as_slice() {
                [] => continue,
                [only] => return Ok(*only),
                several => {
                    return Err(format!(
                        "{tag} owns no state, and {} countries with it as original tag that own states were added to _CountriesForOriginalTags (CGameState+0x988) in one recorded interval, {}, so which one 1401DA520 takes first is not known: {}",
                        several.len(),
                        group.when,
                        several
                            .iter()
                            .map(|slot| self.describe(Target::Country(*slot)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }
        Ok(slot)
    }

    /// Why a three-byte name that is no recorded country tag is refused
    /// when it is, or may be, a tag alias: 140BA53F0 looks it up in the
    /// tag alias database (143316AA8) after the country tags, and the
    /// country an alias names is chosen at run time by its targets and
    /// triggers, which are not evaluated yet.
    pub fn alias(&self, tag: &str) -> Option<String> {
        if self.slots.contains_key(tag) {
            return None;
        }
        if self.aliases.get(tag).is_some() {
            return Some(format!(
                "tag alias {tag}: AliasResolve (1407141F0) runs its triggers in the trigger evaluator (crate::triggers), not in a scope read alone"
            ));
        }
        (tag.len() == 3 && !self.aliases.unknown.is_empty()).then(|| {
            format!(
                "{tag}: whether it is a tag alias (143316AA8) is unknown: {}",
                self.aliases.unknown.join("; ")
            )
        })
    }

    /// The body of the tag alias a three-byte name is, when it is no
    /// recorded country tag: 140BA53F0 looks the alias database (143316AA8)
    /// up after the country tags.
    pub fn alias_entry(&self, tag: &str) -> Option<&TagAlias> {
        if self.slots.contains_key(tag) {
            return None;
        }
        self.aliases.get(tag)
    }

    /// Every country's canonical slot, in slot order (the game state's
    /// country array, CGameState+0x310), or why the recording does not hold
    /// them all.
    pub fn roster(&self) -> Result<impl Iterator<Item = u32> + '_, String> {
        if !self.countries_complete {
            return Err(format!(
                "the country roster (CGameState+0x310) is not complete: {}",
                self.gaps.get("Countries").cloned().unwrap_or_default()
            ));
        }
        Ok(self.tags.keys().copied())
    }

    /// A country's owned state count (CCountry+0x484), or why it is not
    /// recorded.
    pub fn owned_states(&self, slot: u32) -> Result<i32, String> {
        self.owned.get(&slot).copied().ok_or_else(|| {
            format!(
                "the owned state count (CountryCounterInputs.owned_state_count, CCountry+0x484) of {} is not recorded: {}",
                self.describe(Target::Country(slot)),
                self.gaps.get("CountryCounterInputs").cloned().unwrap_or_default()
            )
        })
    }

    /// `_CountriesForOriginalTags` (CGameState+0x988) as derived.
    pub fn original_tags(&self) -> &OriginalTagLists {
        &self.original_tags
    }

    /// A country's tag.
    pub fn tag(&self, slot: u32) -> Option<&str> {
        self.tags.get(&slot).map(String::as_str)
    }

    /// A country's canonical slot by tag.
    pub fn country(&self, tag: &str) -> Option<u32> {
        self.slots.get(tag).copied()
    }

    /// A raw country link's canonical slot.
    pub fn slot(&self, link: u32) -> Option<u32> {
        self.links.get(&link).copied()
    }

    /// Parses `text` as the game's event target reader does
    /// ([`installed_content::scope::parse`]) with this hour's countries and
    /// characters, and resolves it in `context`.
    pub fn resolve_text(
        &self,
        text: &str,
        quoted: bool,
        context: &Context,
    ) -> Result<Target, Unresolved> {
        self.resolve(&installed_content::scope::parse(text, quoted, self), context)
    }

    /// Resolves a parsed scope in `context`: the target, a scope that names
    /// nothing ([`Unresolved::Nothing`]), or a refusal naming the recorded
    /// input it would need ([`Unresolved::Missing`]).
    ///
    /// Not yet implemented: the `occupied` promotion, which
    /// `StateResistance.country` (the occupied country's original tag,
    /// StartResistance 140F85ED0 writes CResistance+0x50) would
    /// answer; it is refused as unrecorded until a consumer needs it.
    pub fn resolve(&self, scope: &Scope, context: &Context) -> Result<Target, Unresolved> {
        use Unresolved::{Missing, Nothing};
        let gap = |table: &str| self.gaps.get(table).cloned().unwrap_or_default();
        match scope {
            Scope::State(id) => {
                let id = u32::try_from(*id)
                    .map_err(|_| Nothing(format!("{scope}: no state has a negative id")))?;
                if self.states.contains_key(&id) {
                    Ok(Target::State(id))
                } else {
                    Err(Missing(format!(
                        "state {id} has no recorded States row: {}",
                        gap("States")
                    )))
                }
            }
            Scope::Country(tag) => self.resolve_tag(tag).map(Target::Country).map_err(Missing),
            // 14146BB30 looks the token up among the scope country's
            // characters (CCountry::GetCharacters 1406E03C0, 1410D6BF0) and
            // logs "Country is mandatory" without a country; a character it
            // does not find is logged ("could not find this character in
            // country") and the scope's character is the invalid reference.
            Scope::Character(token) => {
                let Some(Target::Country(country)) = context.this else {
                    return Err(Nothing(format!(
                        "character {token}: the scope has no country, which the game requires (14146BB30)"
                    )));
                };
                let found = self
                    .characters
                    .get(&token.to_ascii_lowercase())
                    .and_then(|found| {
                        found
                            .iter()
                            .find(|(_, owner)| *owner == Some(country))
                            .map(|(id, _)| Target::Character(*id))
                    });
                found.ok_or_else(|| {
                    let why = format!(
                        "character {token} is not a character of {}",
                        self.describe(Target::Country(country))
                    );
                    if self.characters_complete {
                        Nothing(why)
                    } else {
                        Missing(format!("{why} in the recording: {}", gap("Characters")))
                    }
                })
            }
            Scope::Keyword(keyword) => self.keyword(*keyword, context),
            Scope::Chain { head, then } => {
                let head = self.keyword(*head, context)?;
                let context = Context {
                    this: Some(head),
                    prev: context.this,
                    ..*context
                };
                self.resolve(then, &context)
            }
            Scope::Variable(name) => Err(Missing(format!(
                "var:{name}: runtime script variables are not recorded"
            ))),
            Scope::EventTarget { name, .. } => Err(Missing(format!(
                "event_target:{name}: saved targets resolve in the trigger evaluator (14146BB30)"
            ))),
            // Generic prefix accepted by the parser, with no matching resolver
            // case: only CScopedVariable handles global_event_target: (14146BB30).
            Scope::Prefixed { prefix, value }
                if prefix.eq_ignore_ascii_case("global_event_target") =>
            {
                Err(Nothing(format!(
                    "{prefix}:{value}: invalid context scope (14146BB30)"
                )))
            }
            Scope::Prefixed { prefix, value } => Err(Missing(format!(
                "{prefix}:{value}: {prefix} scopes are not recorded"
            ))),
            Scope::ZeroNumber(text) => Err(Nothing(format!(
                "{text}: the event target reader keeps a zero number as a scope that names nothing (14146DA20 accepts id 0xC)"
            ))),
            Scope::Unknown(text) => Err(Nothing(format!(
                "{text} is no scope the event target reader (14146DB00) accepts"
            ))),
        }
    }

    fn keyword(&self, keyword: Keyword, context: &Context) -> Result<Target, Unresolved> {
        use Unresolved::{Missing, Nothing};
        let name = keyword.name();
        let unset = || Nothing(format!("{name} is not set in this context"));
        let this = || context.this.ok_or_else(unset);
        // A link of 0 or below is no country; a positive one CountryLinks
        // does not hold is a missing input, not an absence.
        let country = |link: i64, what: &str, of: &str| -> Result<Target, Unresolved> {
            let Some(link) = u32::try_from(link).ok().filter(|link| *link > 0) else {
                return Err(Nothing(format!("{name}: {of} has no {what}")));
            };
            self.slot(link).map(Target::Country).ok_or_else(|| {
                Missing(format!(
                    "{name}: {of}'s {what} link {link} has no CountryLinks row: {}",
                    self.gaps.get("CountryLinks").cloned().unwrap_or_default()
                ))
            })
        };
        match keyword {
            Keyword::Root => context.root.ok_or_else(unset),
            Keyword::This => this(),
            Keyword::Prev => context.prev.ok_or_else(unset),
            Keyword::From => context.from.ok_or_else(unset),
            Keyword::Owner | Keyword::Controller => match this()? {
                Target::State(id) => {
                    let (owner, controller) = self.states.get(&id).copied().ok_or_else(|| {
                        Missing(format!(
                            "{name}: state {id} has no recorded States row: {}",
                            self.gaps["States"]
                        ))
                    })?;
                    let link = if keyword == Keyword::Owner {
                        owner
                    } else {
                        controller
                    };
                    country(link.into(), "country in States", &format!("state {id}"))
                }
                other => Err(Missing(format!(
                    "{name} of {}: only a state's owner and controller are recorded (States)",
                    self.describe(other)
                ))),
            },
            Keyword::CapitalScope => match this()? {
                Target::Country(slot) => {
                    let capital = self.capitals.get(&slot).copied().ok_or_else(|| {
                        Missing(format!(
                            "{name}: {} has no recorded Politics row: {}",
                            self.describe(Target::Country(slot)),
                            self.gaps["Politics"]
                        ))
                    })?;
                    u32::try_from(capital)
                        .ok()
                        .filter(|capital| *capital > 0)
                        .map(Target::State)
                        .ok_or_else(|| {
                            Nothing(format!(
                                "{name}: {} has no capital",
                                self.describe(Target::Country(slot))
                            ))
                        })
                }
                other => Err(Missing(format!(
                    "{name} of {}: only a country's capital is recorded (Politics.capital)",
                    self.describe(other)
                ))),
            },
            Keyword::Overlord => match this()? {
                Target::Country(slot) => {
                    let overlord = self.overlords.get(&slot).copied().ok_or_else(|| {
                        Missing(format!(
                            "{name}: {} has no recorded CountryAppearance row: {}",
                            self.describe(Target::Country(slot)),
                            self.gaps["CountryAppearance"]
                        ))
                    })?;
                    country(
                        overlord.into(),
                        "overlord",
                        &self.describe(Target::Country(slot)),
                    )
                }
                other => Err(Missing(format!(
                    "{name} of {}: only a country's overlord is recorded (CountryAppearance.overlord)",
                    self.describe(other)
                ))),
            },
            Keyword::FactionLeader => Err(Missing(format!(
                "{name}: the faction leader is not recorded (FactionMembers holds membership and order only)"
            ))),
            Keyword::Occupied | Keyword::Character | Keyword::Operation | Keyword::Opponent => {
                Err(Missing(format!(
                    "{name}: this scope is set by the running script and is not recorded"
                )))
            }
        }
    }

    /// A target as a message names it.
    pub fn describe(&self, target: Target) -> String {
        match target {
            Target::Country(slot) => match self.tag(slot) {
                Some(tag) => format!("country {tag}"),
                None => format!("country slot {slot}"),
            },
            Target::State(id) => format!("state {id}"),
            Target::Character(id) => format!("character {id}"),
        }
    }
}

impl Names for RecordedScopes {
    /// A recorded country's tag, or a tag alias (140BA53F0), which then
    /// resolves to a named refusal ([`Self::alias`]).
    fn is_country(&self, tag: &str) -> bool {
        self.slots.contains_key(tag) || self.alias(tag).is_some()
    }

    /// A live character's token; the load-time classification
    /// (14146DB00 -> 1406B0030) reads the content's templates instead.
    fn is_character(&self, token: &str) -> bool {
        self.characters.contains_key(&token.to_ascii_lowercase())
    }

    /// The prefixes 14146E4E0 compares (`mio` 0x4D37, `sp` 0x4D38, `scope`
    /// 0x2996); any other keyword prefix parses as well but resolves to the
    /// same refusal.
    fn is_keyword(&self, text: &str) -> bool {
        // global_event_target is native lexer token 0x3D94 (14146DB00).
        ["mio", "sp", "scope", "global_event_target"]
            .iter()
            .any(|prefix| text.eq_ignore_ascii_case(prefix))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use replay::{
        frame::Metrics,
        rows::{
            CharacterRow, CountryAppearanceRow, CountryCounterInputRow, CountryLinkRow, CountryRow,
            LexicalTokenRow, PoliticsRow, StateRow, TableId, TableRows,
        },
    };

    /// GER (slot 0, link 100) with capital state 64, SOV (slot 1, link 200)
    /// a subject of GER, state 64 owned by GER and controlled by SOV.
    pub(crate) fn frame() -> DecodedFrame {
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
            TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 100,
                    canonical_slot: 0,
                },
                CountryLinkRow {
                    id: 200,
                    canonical_slot: 1,
                },
            ]),
        );
        set(
            TableId::Countries,
            TableRows::Countries(
                [(100, 0, "GER"), (200, 1, "SOV")]
                    .map(|(id, canonical_slot, tag)| CountryRow {
                        id,
                        canonical_slot,
                        tag: tag.into(),
                        ..Default::default()
                    })
                    .into(),
            ),
        );
        set(
            TableId::States,
            TableRows::States(vec![StateRow {
                id: 64,
                owner: 100,
                controller: 200,
                ..Default::default()
            }]),
        );
        set(
            TableId::Politics,
            TableRows::Politics(vec![PoliticsRow {
                country: 100,
                capital: 64,
                ..Default::default()
            }]),
        );
        set(
            TableId::CountryAppearance,
            TableRows::CountryAppearance(vec![
                CountryAppearanceRow {
                    country: 100,
                    ..Default::default()
                },
                CountryAppearanceRow {
                    country: 200,
                    overlord: 100,
                    ..Default::default()
                },
            ]),
        );
        set(
            TableId::CountryCounterInputs,
            TableRows::CountryCounterInputs(vec![
                CountryCounterInputRow {
                    country: 100,
                    owned_state_count: 1,
                    ..Default::default()
                },
                CountryCounterInputRow {
                    country: 200,
                    owned_state_count: 3,
                    ..Default::default()
                },
            ]),
        );
        set(
            TableId::Characters,
            TableRows::Characters(vec![CharacterRow {
                id: 7,
                country: 100,
                name_token_id: 9000,
                token: b"Erwin Rommel".to_vec(),
                ..Default::default()
            }]),
        );
        set(
            TableId::LexicalTokens,
            TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 9000,
                literal: b"GER_erwin_rommel".to_vec(),
            }]),
        );
        frame
    }

    /// ResolveScriptCountryTag (140BA5D10): a tag owning no state stands
    /// for the first country of its original-tag list that owns one.
    #[test]
    fn a_tag_without_states_resolves_as_the_game_substitutes_it() {
        let counts = |ger: i32, sov: i32| {
            let mut frame = frame();
            *frame.tables[TableId::CountryCounterInputs.index()] =
                TableView::Complete(TableRows::CountryCounterInputs(vec![
                    CountryCounterInputRow {
                        country: 100,
                        owned_state_count: ger,
                        ..Default::default()
                    },
                    CountryCounterInputRow {
                        country: 200,
                        owned_state_count: sov,
                        ..Default::default()
                    },
                ]));
            if let TableView::Complete(TableRows::CountryAppearance(rows)) =
                &mut *frame.tables[TableId::CountryAppearance.index()]
            {
                // SOV is a dynamic country named after GER.
                rows[1].name_tag = b"GER".to_vec();
            }
            RecordedScopes::read(&frame, Arc::default())
        };
        assert_eq!(counts(1, 3).resolve_tag("GER"), Ok(0));
        assert_eq!(counts(0, 3).resolve_tag("GER"), Ok(1));
        // No candidate owns a state: the tag's own country.
        assert_eq!(counts(0, 0).resolve_tag("GER"), Ok(0));
        let mut frame = frame();
        *frame.tables[TableId::CountryCounterInputs.index()] = TableView::Unavailable {
            reason: "not sampled".into(),
        };
        assert!(
            RecordedScopes::read(&frame, Arc::default())
                .resolve_tag("GER")
                .unwrap_err()
                .contains("CountryCounterInputs.owned_state_count")
        );
    }

    /// GER (link 100, slot 0) and three dynamic countries D01..D03 (links
    /// 201..203, slots 1..3), with each dynamic country's `name_tag` and
    /// every country's owned state count.
    pub(crate) fn dynamic_frame(hour: i32, names: [&str; 3], owned: [i32; 4]) -> DecodedFrame {
        let mut frame = frame();
        frame.hour = hour;
        let links = [(100, 0), (201, 1), (202, 2), (203, 3)];
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                links
                    .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                    .into(),
            ));
        *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
            [
                (100, 0, "GER"),
                (201, 1, "D01"),
                (202, 2, "D02"),
                (203, 3, "D03"),
            ]
            .map(|(id, canonical_slot, tag)| CountryRow {
                id,
                canonical_slot,
                tag: tag.into(),
                ..Default::default()
            })
            .into(),
        ));
        let mut appearance = vec![CountryAppearanceRow {
            country: 100,
            ..Default::default()
        }];
        for (index, name) in names.iter().enumerate() {
            appearance.push(CountryAppearanceRow {
                country: 201 + index as u32,
                name_tag: name.as_bytes().to_vec(),
                ..Default::default()
            });
        }
        *frame.tables[TableId::CountryAppearance.index()] =
            TableView::Complete(TableRows::CountryAppearance(appearance));
        *frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(
                links
                    .iter()
                    .zip(owned)
                    .map(|((country, _), owned_state_count)| CountryCounterInputRow {
                        country: *country,
                        owned_state_count,
                        ..Default::default()
                    })
                    .collect(),
            ));
        frame
    }

    /// _CountriesForOriginalTags derived: slot order at a load (1401DA010),
    /// then appends in hour order (1401EE1C0) with removals keeping order
    /// (1401D1BC0); one interval's appends are unordered, which matters
    /// only when two of them own states.
    #[test]
    fn the_original_tag_list_is_derived_from_the_load_and_the_appends() {
        let resolve = |lists: &OriginalTagLists, frame: &DecodedFrame| {
            RecordedScopes::read_with(frame, lists.clone(), Arc::default()).resolve_tag("GER")
        };
        // A load: slot order, D01 before D02.
        let load = dynamic_frame(10, ["GER", "GER", ""], [0, 1, 1, 1]);
        let mut lists = OriginalTagLists::start(&load, true);
        assert_eq!(resolve(&lists, &load), Ok(1));
        // Appends across hours: D01 leaves, D03 joins after D02.
        let hour = dynamic_frame(11, ["", "GER", ""], [0, 1, 1, 1]);
        lists.advance(&hour);
        let hour = dynamic_frame(12, ["", "GER", "GER"], [0, 0, 0, 1]);
        lists.advance(&hour);
        assert_eq!(resolve(&lists, &hour), Ok(3));
        let hour = dynamic_frame(13, ["GER", "GER", "GER"], [0, 1, 1, 1]);
        lists.advance(&hour);
        // D02 (load), D03 (hour 12), D01 (hour 13).
        assert_eq!(resolve(&lists, &hour), Ok(2));
        let owners: Vec<Vec<u32>> = lists
            .list("GER")
            .iter()
            .map(|g| g.members.clone())
            .collect();
        assert_eq!(owners, [vec![2], vec![3], vec![1]]);

        // One hour's appends are unordered: harmless with one state owner.
        let start = dynamic_frame(20, ["", "", ""], [0, 0, 0, 0]);
        let mut lists = OriginalTagLists::start(&start, true);
        let both = dynamic_frame(21, ["GER", "GER", ""], [0, 0, 1, 0]);
        lists.advance(&both);
        assert_eq!(resolve(&lists, &both), Ok(2));
        // ... and refused, naming the tag, the hour and the candidates,
        // when two of them own states.
        let owning = dynamic_frame(21, ["GER", "GER", ""], [0, 1, 1, 0]);
        assert_eq!(
            resolve(&lists, &owning),
            Err("GER owns no state, and 2 countries with it as original tag that own states were added to _CountriesForOriginalTags (CGameState+0x988) in one recorded interval, hour 21, so which one 1401DA520 takes first is not known: country D01, country D02".into())
        );
        // Without the branch's history one frame's members are unordered.
        assert!(
            RecordedScopes::read(&owning, Arc::default())
                .resolve_tag("GER")
                .unwrap_err()
                .contains("at hour 21, read without the branch's earlier hours")
        );
        // A branch that does not start at a load starts unordered.
        let lists = OriginalTagLists::start(&load, false);
        assert!(
            resolve(&lists, &load)
                .unwrap_err()
                .contains("before hour 10, where the branch starts without a load")
        );
    }

    #[test]
    fn a_target_is_its_scope_kind_for_the_trigger_check() {
        use installed_content::scope::TriggerScopes;
        let country = Target::Country(0);
        assert!(TriggerScopes(0x4).accepts(|kind| kind == country.kind()));
        assert!(!TriggerScopes(0x2).accepts(|kind| kind == country.kind()));
        assert!(TriggerScopes(0x2).accepts(|kind| kind == Target::State(64).kind()));
    }

    #[test]
    fn scopes_resolve_through_the_recorded_tables_and_refuse_the_rest() {
        let scopes = RecordedScopes::read(&frame(), Arc::default());
        let sov = Context::country(1);
        let resolve = |text: &str, context: &Context| {
            scopes
                .resolve_text(text, false, context)
                .map_err(|unresolved| unresolved.to_string())
        };
        assert_eq!(resolve("ROOT", &sov), Ok(Target::Country(1)));
        assert_eq!(resolve("GER", &sov), Ok(Target::Country(0)));
        assert_eq!(resolve("64", &sov), Ok(Target::State(64)));
        assert_eq!(resolve("overlord", &sov), Ok(Target::Country(0)));
        assert_eq!(
            resolve("ROOT.overlord.capital_scope", &sov),
            Ok(Target::State(64))
        );
        // A character of the scope country only (14146BB30), found by its
        // token, so without case; another country's names nothing there.
        for token in ["GER_erwin_rommel", "ger_ERWIN_rommel"] {
            assert_eq!(
                resolve(token, &Context::country(0)),
                Ok(Target::Character(7))
            );
        }
        assert!(matches!(
            scopes.resolve_text("GER_erwin_rommel", false, &sov),
            Err(Unresolved::Nothing(why))
                if why == "character GER_erwin_rommel is not a character of country SOV"
        ));
        assert!(
            resolve("64.GER_erwin_rommel", &sov).is_ok_and(|target| target == Target::State(64))
        );
        assert!(resolve("ROOT.capital_scope", &sov).is_err());
        assert_eq!(
            resolve("ROOT.overlord.capital_scope.controller", &sov),
            Ok(Target::Country(1))
        );
        // PREV inside a chain is the scope before the step.
        assert_eq!(resolve("overlord.PREV", &sov), Ok(Target::Country(1)));
        for (text, reason) in [
            ("FROM", "from is not set in this context"),
            ("var:x", "var:x: runtime script variables are not recorded"),
            (
                "event_target:hq",
                "event_target:hq: saved targets resolve in the trigger evaluator",
            ),
            ("mio:GER_tank", "mio:GER_tank: mio scopes are not recorded"),
            (
                "faction_leader",
                "faction_leader: the faction leader is not recorded",
            ),
            (
                "65",
                "state 65 has no recorded States row: the complete table has no row for it",
            ),
            (
                "GER.owner",
                "GER.owner is no scope the event target reader (14146DB00) accepts",
            ),
            (
                "owner",
                "owner of country SOV: only a state's owner and controller are recorded (States)",
            ),
            ("overlord.overlord", "overlord: country GER has no overlord"),
        ] {
            let error = resolve(text, &sov).unwrap_err();
            assert!(error.starts_with(reason), "{text}: {error}");
        }
        // A table the hour lacks is named with its state.
        let mut frame = frame();
        *frame.tables[TableId::Politics.index()] = TableView::Unavailable {
            reason: "not sampled".into(),
        };
        let scopes = RecordedScopes::read(&frame, Arc::default());
        assert_eq!(
            scopes.resolve_text("capital_scope", false, &Context::country(0)),
            Err(Unresolved::Missing("capital_scope: country GER has no recorded Politics row: the table is unavailable: not sampled".into()))
        );
    }

    /// A three-byte name that is no country tag but a tag alias
    /// (140BA53F0 -> 143316AA8) parses as a country scope and is refused by
    /// name; while an alias file stops short, any unknown three-byte name is.
    #[test]
    fn a_tag_alias_scope_is_refused_by_name() {
        let aliases = installed_content::tag_aliases::parse(
            &[("a.txt", b"SPA = { }")],
            &installed_content::key_names::KeyNames::default(),
        );
        let scopes = RecordedScopes::read(&frame(), Arc::new(aliases));
        let sov = Context::country(1);
        assert_eq!(
            scopes.resolve_text("SPA", false, &sov),
            Err(Unresolved::Missing(
                "tag alias SPA: AliasResolve (1407141F0) runs its triggers in the trigger evaluator (crate::triggers), not in a scope read alone"
                    .into()
            ))
        );
        assert_eq!(
            scopes.resolve_text("GER", false, &sov),
            Ok(Target::Country(0))
        );
        let short = TagAliases {
            unknown: vec!["common/country_tag_aliases/a.txt: line 2: x".into()],
            ..Default::default()
        };
        let scopes = RecordedScopes::read(&frame(), Arc::new(short));
        assert!(matches!(
            scopes.resolve_text("XYZ", false, &sov),
            Err(Unresolved::Missing(reason)) if reason.contains("whether it is a tag alias")
        ));
    }

    /// A state's owner or controller link of 0 names no country (the game's
    /// scope is null); a positive link CountryLinks does not hold is a
    /// missing input, refused by name.
    #[test]
    fn an_owner_link_of_zero_is_nothing_and_an_unlinked_one_is_missing() {
        let mut frame = frame();
        *frame.tables[TableId::States.index()] =
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 64,
                owner: 0,
                controller: 300,
                ..Default::default()
            }]));
        let scopes = RecordedScopes::read(&frame, Arc::default());
        let state = Context {
            this: Some(Target::State(64)),
            ..Context::default()
        };
        assert_eq!(
            scopes.resolve_text("owner", false, &state),
            Err(Unresolved::Nothing(
                "owner: state 64 has no country in States".into()
            ))
        );
        assert!(matches!(
            scopes.resolve_text("controller", false, &state),
            Err(Unresolved::Missing(reason))
                if reason.starts_with("controller: state 64's country in States link 300 has no CountryLinks row")
        ));
    }
}
