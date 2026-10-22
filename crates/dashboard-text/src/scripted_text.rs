//! Scripted localisation (`[ROOT.GetName]`) evaluated over one recorded hour.
//!
//! Text processing expands every `[` .. first `]` in a localisation
//! context. Each bracket starts from the saved root context. `[(...)]`
//! is the conditional form and `[?...]` the variable form; anything else
//! splits at every `.`: each segment but the last promotes the scope,
//! and the last is a property, whose text is processed again one level
//! deeper, up to `NGame.MAX_SCRIPTED_LOC_RECURSION` (at the limit the text
//! is kept as it
//! is). A `[` with no `]` after it ends the text, dropping it from the last
//! bracket on.
//!
//! The views this serves (idea names and descriptions, focus names and
//! the stability breakdown) build a fresh event scope with only the
//! country set. ROOT, FROM and PREV are itself, so the root context is
//! the event scope of that country.
//!
//! Names match without case at their exact length;
//! country tags and scripted localisation names match exactly. What the
//! recording cannot answer is refused by name with the missing input.

use crate::scope::RecordedScopes;
use installed_content_country_names::country_names::CountryNames;
use installed_content_localisation::{localisation::Localisation, localisation::crt_case_text};
use replay_data::{rows::TableSchema, rows::tables};
use replay_frame::{frame::DecodedFrame, frame::TableView};

/// Recorded inputs for authored country-scoped scripted text and its triggers.
/// Consumers that see a scripted `[` in resolved content decode this set once
/// in their forward presentation pass. Plain text needs none of these tables.
pub const TABLES: &[replay_data::rows::TableId] = &[
    replay_data::rows::TableId::ActiveIdeas,
    replay_data::rows::TableId::AdvisorTokens,
    replay_data::rows::TableId::ArmyOrderPlans,
    replay_data::rows::TableId::ArmyOrders,
    replay_data::rows::TableId::Autonomy,
    replay_data::rows::TableId::CharacterOperatives,
    replay_data::rows::TableId::CharacterRoleTraits,
    replay_data::rows::TableId::Characters,
    replay_data::rows::TableId::CompletedFocuses,
    replay_data::rows::TableId::Countries,
    replay_data::rows::TableId::CountryAppearance,
    replay_data::rows::TableId::CountryBuildingResources,
    replay_data::rows::TableId::CountryCharacters,
    replay_data::rows::TableId::CountryCivilWars,
    replay_data::rows::TableId::CountryCounterInputs,
    replay_data::rows::TableId::CountryFactories,
    replay_data::rows::TableId::CountryLeaderRoles,
    replay_data::rows::TableId::CountryLinks,
    replay_data::rows::TableId::CountryRetiredCharacters,
    replay_data::rows::TableId::CountryResources,
    replay_data::rows::TableId::CountryStabilityInputs,
    replay_data::rows::TableId::CountryWars,
    replay_data::rows::TableId::DynamicModifiers,
    replay_data::rows::TableId::FactionAdvisors,
    replay_data::rows::TableId::FactionMembers,
    replay_data::rows::TableId::Factions,
    replay_data::rows::TableId::Focus,
    replay_data::rows::TableId::GlobalSavedEventTargets,
    replay_data::rows::TableId::AdvisorRoles,
    replay_data::rows::TableId::HiredAdvisors,
    replay_data::rows::TableId::HumanControllers,
    replay_data::rows::TableId::IntelligenceAgencies,
    replay_data::rows::TableId::InvasionPreparations,
    replay_data::rows::TableId::LandTheatres,
    replay_data::rows::TableId::LexicalTokens,
    replay_data::rows::TableId::LocalPlayer,
    replay_data::rows::TableId::ModifierDefinitions,
    replay_data::rows::TableId::Nukes,
    replay_data::rows::TableId::OrdersGroupPlans,
    replay_data::rows::TableId::OrdersGroups,
    replay_data::rows::TableId::PartyLeaders,
    replay_data::rows::TableId::PoliticalParties,
    replay_data::rows::TableId::Politics,
    replay_data::rows::TableId::Players,
    replay_data::rows::TableId::PowerBalanceMembers,
    replay_data::rows::TableId::PowerBalanceModifierBreakdowns,
    replay_data::rows::TableId::PowerBalances,
    replay_data::rows::TableId::Projects,
    replay_data::rows::TableId::ProvinceBuildings,
    replay_data::rows::TableId::ProvinceNames,
    replay_data::rows::TableId::Provinces,
    replay_data::rows::TableId::RaidCountryInputs,
    replay_data::rows::TableId::RelationOpinions,
    replay_data::rows::TableId::ResearchTechnologies,
    replay_data::rows::TableId::ResearchModifiers,
    replay_data::rows::TableId::ReservedCountryNaming,
    replay_data::rows::TableId::ResourceDefinitions,
    replay_data::rows::TableId::ScriptFlags,
    replay_data::rows::TableId::ScriptVariables,
    replay_data::rows::TableId::SimulationFeatures,
    replay_data::rows::TableId::StateBaseResources,
    replay_data::rows::TableId::StateBuildings,
    replay_data::rows::TableId::StateCores,
    replay_data::rows::TableId::StateDynamicModifiers,
    replay_data::rows::TableId::StateResourceInputs,
    replay_data::rows::TableId::StateResources,
    replay_data::rows::TableId::StateResistance,
    replay_data::rows::TableId::States,
    replay_data::rows::TableId::Subjects,
    replay_data::rows::TableId::TechnologyDefinitions,
    replay_data::rows::TableId::TechSharingGroups,
    replay_data::rows::TableId::TechSharingMembers,
    replay_data::rows::TableId::UnitLeaderTraits,
    replay_data::rows::TableId::UnitLeaders,
    replay_data::rows::TableId::WarRelations,
];

mod balance;
mod dependence;
mod entries;
mod numeric;
pub use entries::{
    Environment, StateContent, StateNameUsage, StateNames, live_state_warnings, read_state_content,
    state_display_name,
};
mod properties;
pub mod random;
pub mod recorded_names;
mod state_properties;

// Nesting backstop: each scripted entry restarts text processing at
// depth zero, independently of the content recursion limit.
const MAX_SCRIPTED_NESTING: usize = 10_000;

fn nesting_refusal(name: &str) -> String {
    format!(
        "scripted localisation nesting the game could never finish (native stack exhaustion): {name}, beyond {MAX_SCRIPTED_NESTING} levels"
    )
}

/// A localisation context object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Object {
    /// Faction environment: copied event scope and faction.
    FactionEnvironment(Option<u64>),
    /// A faction selected from the environment.
    Faction(Option<u64>),
    /// The root event scope, whose country is the view's.
    Event,
    /// A country by canonical slot.
    Country(u32),
    /// A state, `None` for a null state object.
    State(Option<u32>),
    /// A character by ID.
    Character(u32),
}

/// An event-scope link: ROOT, FROM or PREV.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// The scope itself, as a default-constructed scope links.
    Itself,
    /// Another scope of the same [`EventScopes`], by index.
    Scope(usize),
    /// A scope the caller cannot describe; promoting through it is refused.
    Unknown,
}

/// What an event scope holds, as text reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventObject {
    Country(u32),
    /// A combat context retains only the combat object.
    /// Its side country is separate from the event scope's country.
    Combat,
    /// Country and state set without clearing, for environment localisation.
    CountryState {
        country: u32,
        state: u32,
    },
    /// A state by ID; setting it clears the scope's country.
    State(u32),
    Province(u32),
    /// A unit leader by full reference: stores its character
    /// and clears the country.
    UnitLeader(u64),
    /// Active ability tooltip: set the country with clearing, then
    /// the unit leader without clearing. Country and character both survive.
    /// Text promotions prefer the character; variable storage and RNG
    /// identity prefer the country. Neither setter changes the self links.
    CountryUnitLeader {
        country: u32,
        leader: u64,
    },
    /// A character by ID; setting it clears the country.
    Character(u32),
    /// Setting an industrial organisation clears other carried objects.
    IndustrialOrganisation(u64),
    /// The project owner and ID carried by the scope.
    Project {
        owner: u32,
        id: u32,
    },
    /// The raid instance reference carried by the scope.
    Raid(u64),
    /// The unit reference carried by the scope.
    Unit(u64),
}

/// One event scope as text reads it: its object and its links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventScope {
    pub this: EventObject,
    pub root: Link,
    pub from: Link,
    pub prev: Link,
    /// Seed used by an unseeded random_list.
    pub random_seed: u32,
    /// Counter copied independently of the seed.
    pub random_counter: u32,
    /// Missing input to these lanes; copies retain it, fresh constructors clear it.
    /// Only a random selection needs this input.
    pub random_seed_refusal: Option<&'static str>,
    /// The Clock seed, recorded or not, the lanes were derived from; a draw
    /// reads it ([`crate::triggers::inputs::Key::ClockSeed`]). None for lanes
    /// no recorded seed set.
    pub clock_seed: Option<Option<u32>>,
}

impl EventScope {
    /// A default-constructed scope set to `this`: its ROOT, FROM
    /// and PREV are itself.
    pub fn of(this: EventObject) -> Self {
        Self {
            this,
            root: Link::Itself,
            from: Link::Itself,
            prev: Link::Itself,
            random_seed: 1, // Setters preserve the seed words.
            random_counter: 0x5ea6_ba9e,
            random_seed_refusal: None,
            clock_seed: None,
        }
    }

    pub(crate) fn require_random_seed(&self) -> Result<u32, String> {
        if let Some(seed) = &self.clock_seed {
            crate::triggers::inputs::read(crate::triggers::inputs::Key::ClockSeed, seed);
        }
        match self.random_seed_refusal {
            Some(why) => Err(why.into()),
            None => Ok(self.random_seed),
        }
    }

    pub(crate) fn random_draw(&self) -> Result<u32, String> {
        Ok(random::scoped_draw(self.require_random_seed()?, self.random_counter) as u32)
    }
}

/// The event scopes a text is expanded in. The first is the caller's,
/// made active again at every bracket; ROOT, FROM and PREV promotions
/// make the linked scope active, so later segments follow its links.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventScopes {
    pub scopes: Vec<EventScope>,
    /// The thread-local script variables set while the text is expanded.
    pub variables: ScopeVariables,
    /// Description-only stores; save follows ROOT, lookup follows FROM.
    pub tooltip_targets: std::collections::BTreeMap<usize, Vec<(Vec<u8>, EventObject)>>,
    pub saved_targets: std::collections::BTreeMap<usize, Vec<(Vec<u8>, EventObject)>>,
    pub global_tooltip_targets: Vec<(Vec<u8>, EventObject)>,
}

/// Thread-local script variables a caller has set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScopeVariables {
    pub target_state: Option<u32>,
    pub target_province: Option<u32>,
    pub target_leader: Option<u64>,
}

impl EventScopes {
    /// `scopes`, with no script variables set.
    /// Fresh scopes; description completion clears temporary globals.
    /// Event-chain consumers need their own target arrays.
    pub fn new(scopes: Vec<EventScope>) -> Self {
        Self {
            scopes,
            variables: ScopeVariables::default(),
            tooltip_targets: Default::default(),
            saved_targets: Default::default(),
            global_tooltip_targets: Vec::new(),
        }
    }

    /// The scopes the game evaluates an organisation's conditions and text
    /// in: the organisation, whose ROOT and PREV are itself and whose FROM is
    /// its owner, then the owner country (canonical slot), whose links are
    /// all itself. An unknown owner leaves FROM unknown.
    pub fn organisation(organisation: u64, owner: Option<u32>) -> Self {
        let this = EventScope::of(EventObject::IndustrialOrganisation(organisation));
        Self::new(match owner {
            Some(owner) => vec![
                EventScope {
                    from: Link::Scope(1),
                    ..this
                },
                EventScope::of(EventObject::Country(owner)),
            ],
            None => vec![EventScope {
                from: Link::Unknown,
                ..this
            }],
        })
    }
}

/// The index of the scope that `link` of scope `at` names.
fn linked_scope(length: usize, at: usize, link: Link, name: &str) -> Result<usize, String> {
    match link {
        Link::Itself => Ok(at),
        Link::Scope(to) if to < length => Ok(to),
        Link::Scope(to) => Err(format!("{name}: event scope {to} is not given")),
        Link::Unknown => Err(format!(
            "{name}: the event scope's {name} link is not known here"
        )),
    }
}

/// One expansion's scopes, shared by nested entries without copying their history.
#[derive(Clone)]
pub(crate) struct ExpansionScopes<'e, 'a> {
    scopes: std::cell::RefCell<Vec<EventScope>>,
    variables: std::cell::Cell<ScopeVariables>,
    targets: std::cell::RefCell<EventScopes>,
    session: std::cell::RefCell<Option<crate::triggers::VariableSession>>,
    active: std::cell::RefCell<Vec<entries::ActiveEntry>>,
    prev_distance: std::cell::RefCell<Vec<Option<usize>>>,
    valid_links: std::cell::Cell<bool>,
    fresh_depth: std::cell::Cell<usize>,
    on_large_stack: bool,
    pub(crate) world: Option<&'e crate::triggers::World<'a>>,
    pub(crate) combat: Option<&'e dyn crate::triggers::Combat>,
}

impl<'e, 'a> ExpansionScopes<'e, 'a> {
    fn new(scopes: &EventScopes) -> Self {
        let prev_distance = (0..scopes.scopes.len())
            .map(|start| {
                let this = scopes.scopes[start].this;
                let mut at = start;
                let mut distance = 0;
                let mut seen = std::collections::BTreeSet::new();
                loop {
                    if !seen.insert(at) {
                        return None;
                    }
                    distance += 1;
                    let Ok(next) =
                        linked_scope(scopes.scopes.len(), at, scopes.scopes[at].prev, "")
                    else {
                        return Some(distance);
                    };
                    at = next;
                    if scopes.scopes[at].this != this {
                        return Some(distance);
                    }
                }
            })
            .collect();
        Self {
            scopes: std::cell::RefCell::new(scopes.scopes.clone()),
            variables: std::cell::Cell::new(scopes.variables),
            targets: std::cell::RefCell::new(scopes.clone()),
            session: std::cell::RefCell::new(None),
            active: std::cell::RefCell::new(Vec::new()),
            prev_distance: std::cell::RefCell::new(prev_distance),
            valid_links: std::cell::Cell::new(scopes.scopes.iter().all(|scope| {
                [scope.root, scope.from, scope.prev]
                    .iter()
                    .all(|link| !matches!(link, Link::Scope(to) if *to >= scopes.scopes.len()))
            })),
            fresh_depth: std::cell::Cell::new(0),
            on_large_stack: false,
            world: None,
            combat: None,
        }
    }

    fn scope(&self, at: usize) -> EventScope {
        self.scopes.borrow()[at]
    }

    /// Keep entry history and the fresh-context guard across effect descriptions.
    /// Append its graph so active entries retain their existing cursor identities.
    fn append_description<'s>(&'s self, scopes: &EventScopes) -> DescriptionScopeGuard<'s, 'e, 'a> {
        let next = Self::new(scopes);
        let offset = self.scopes.borrow().len();
        let rebase = |link| match link {
            Link::Scope(at) => Link::Scope(at + offset),
            other => other,
        };
        let appended: Vec<_> = scopes
            .scopes
            .iter()
            .map(|scope| EventScope {
                root: rebase(scope.root),
                from: rebase(scope.from),
                prev: rebase(scope.prev),
                ..*scope
            })
            .collect();
        self.scopes.borrow_mut().extend(appended);
        self.prev_distance
            .borrow_mut()
            .extend(next.prev_distance.into_inner());
        let variables = self.variables.replace(scopes.variables);
        let valid_links = self
            .valid_links
            .replace(self.valid_links.get() && next.valid_links.get());
        let mut targets = self.targets.borrow_mut();
        for (at, values) in &scopes.tooltip_targets {
            targets.tooltip_targets.insert(at + offset, values.clone());
        }
        for (at, values) in &scopes.saved_targets {
            targets.saved_targets.insert(at + offset, values.clone());
        }
        let globals = std::mem::replace(
            &mut targets.global_tooltip_targets,
            scopes.global_tooltip_targets.clone(),
        );
        DescriptionScopeGuard {
            expanded: self,
            offset,
            variables,
            valid_links,
            globals,
        }
    }

    fn linked(&self, at: usize, link: Link, name: &str) -> Result<usize, String> {
        linked_scope(self.scopes.borrow().len(), at, link, name)
    }

    fn evaluator<'s>(
        &'s self,
        text: &'s ScriptedText<'a>,
        temps: crate::triggers::Temps,
    ) -> Result<crate::triggers::Evaluator<'s, 'a>, String> {
        let world = match self.world {
            Some(world) => world,
            None => text.world()?,
        };
        let mut graph = self.targets.borrow().clone();
        graph.scopes = self.scopes.borrow().clone();
        graph.variables = self.variables.get();
        let evaluator = crate::triggers::Evaluator::new(world, self.combat)
            .with_temps(temps)
            .with_event_scopes(std::rc::Rc::new(std::cell::RefCell::new(graph)));
        Ok(match self.session.borrow().as_ref() {
            Some(session) => evaluator.with_variable_session(session),
            None => evaluator,
        })
    }

    fn needs_large_stack(&self) -> bool {
        // Handoff early enough for ordinary caller stacks; shallow expansions stay local.
        !self.on_large_stack && self.active.borrow().len() + self.fresh_depth.get() >= 8
    }
}

struct DescriptionScopeGuard<'s, 'e, 'a> {
    expanded: &'s ExpansionScopes<'e, 'a>,
    offset: usize,
    variables: ScopeVariables,
    valid_links: bool,
    globals: Vec<(Vec<u8>, EventObject)>,
}

impl Drop for DescriptionScopeGuard<'_, '_, '_> {
    fn drop(&mut self) {
        self.expanded.scopes.borrow_mut().truncate(self.offset);
        self.expanded
            .prev_distance
            .borrow_mut()
            .truncate(self.offset);
        self.expanded.variables.set(self.variables);
        self.expanded.valid_links.set(self.valid_links);
        let mut targets = self.expanded.targets.borrow_mut();
        targets.tooltip_targets.retain(|at, _| *at < self.offset);
        targets.saved_targets.retain(|at, _| *at < self.offset);
        targets.global_tooltip_targets = std::mem::take(&mut self.globals);
    }
}

/// Content facts the evaluator needs besides localisation.
pub struct ScriptedContent {
    /// `common/scripted_localisation` names ([`installed_content_scripted_localisation::scripted_localisation`]).
    pub names: installed_content_scripted_localisation::scripted_localisation::Names,
    /// `NGame.MAX_SCRIPTED_LOC_RECURSION`, compared with text processing depth.
    pub recursion: i32,
    /// `common/country_tag_aliases` names, which promotion step 3 tries
    /// after the country tags.
    pub tag_aliases: std::sync::Arc<installed_content_core::tag_aliases::TagAliases>,
    /// The recording client's game language ([`crate::catalogue::Catalogue`]):
    /// text the game stored already localised is in it.
    pub recorded_language: String,
    /// The content stack the triggers of scripted localisation entries and
    /// state names read ([`entries`]).
    pub layers: std::sync::Arc<installed_content_core::content::ResolvedContent>,
    pub(crate) environment: std::sync::OnceLock<Result<entries::Environment, String>>,
    pub(crate) state_content:
        std::sync::Arc<std::sync::OnceLock<Result<entries::StateContent, String>>>,
    pub(crate) template_names: std::sync::OnceLock<Result<Localisation, String>>,
    event_index: std::sync::OnceLock<installed_content_events::events::LazyEvents>,
    pub(crate) effects:
        std::sync::OnceLock<Result<crate::effect_description::CachedContent, String>>,
}

impl ScriptedContent {
    /// The scripted localisation names, with the files the reader cannot read
    /// to their end, `NGame.MAX_SCRIPTED_LOC_RECURSION` and the tag aliases
    /// of `content`, or why they cannot be read. `recorded_language` is the
    /// recording client's language, without its `l_`.
    pub fn read(
        content: std::sync::Arc<installed_content_core::content::ResolvedContent>,
        recorded_language: String,
    ) -> Result<Self, String> {
        let layers = &content.layers;
        let names = installed_content_scripted_localisation::scripted_localisation::names(layers)
            .map_err(|error| format!("common/scripted_localisation is unreadable: {error}"))?;
        let environment = Environment::read(layers)
            .map_err(|error| format!("NGame.MAX_SCRIPTED_LOC_RECURSION is unavailable: {error}"))?;
        let recursion = environment
            .defines()
            .int32("NGame", "MAX_SCRIPTED_LOC_RECURSION")
            .map_err(|error| format!("NGame.MAX_SCRIPTED_LOC_RECURSION is unavailable: {error}"))?;
        let tag_aliases = installed_content_core::tag_aliases::read(layers)
            .map_err(|error| format!("common/country_tag_aliases is unreadable: {error}"))?;
        Ok(Self {
            names,
            recursion,
            tag_aliases: std::sync::Arc::new(tag_aliases),
            recorded_language,
            layers: content,
            environment: std::sync::OnceLock::from(Ok(environment)),
            state_content: std::sync::Arc::new(std::sync::OnceLock::new()),
            template_names: std::sync::OnceLock::new(),
            event_index: std::sync::OnceLock::new(),
            effects: std::sync::OnceLock::new(),
        })
    }

    pub(crate) fn event_index(&self) -> &installed_content_events::events::LazyEvents {
        self.event_index.get_or_init(|| {
            installed_content_events::events::LazyEvents::from_content(std::sync::Arc::clone(&self.layers))
        })
    }

    pub(crate) fn environment(&self) -> Result<&Environment, String> {
        self.environment
            .get_or_init(|| {
                Environment::read(&self.layers.layers)
                    .map_err(|why| format!("scripted localisation content: {why}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ScriptedContent {
    pub fn set_synthetic_environment(&self, state_names: std::collections::BTreeMap<u32, String>) {
        let mut environment = entries::EnvironmentInputs::synthetic();
        environment.state_names = state_names
            .into_iter()
            .map(|(id, name)| (id, Ok(name)))
            .collect();
        assert!(
            self.environment()
                .unwrap()
                .inputs
                .set(Ok(environment))
                .is_ok()
        );
    }
}

/// Text the game stored already localised, or as a literal, without its key:
/// `what` names the field. The game copies these bytes, so they are exact for
/// a viewer in the recording client's language (`recorded`, without its
/// `l_`). A game in another language would have stored its own translation of
/// a key that was not kept, so there they are refused rather than shown in the
/// recording's language; empty text is the same in every language.
pub fn stored_text(
    localisation: &Localisation,
    recorded: &str,
    bytes: &[u8],
    what: &str,
) -> Result<String, String> {
    if !bytes.is_empty() && recorded != localisation.language {
        return Err(format!(
            "{what} is text the game stored in {recorded} without its key, and the viewer's language is {}",
            localisation.language
        ));
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| format!("{what} is not UTF-8"))
}

/// One hour's scripted text context for a country.
pub struct ScriptedText<'a> {
    localisation: &'a Localisation,
    content: &'a ScriptedContent,
    scopes: RecordedScopes,
    names: recorded_names::RecordedNames<'a>,
    lexical_names: crate::faction_names::LexicalNames,
    frame: &'a DecodedFrame,
    root: u32,
    pub(crate) nato_counters: bool,
    world: std::sync::OnceLock<Result<crate::triggers::World<'a>, String>>,
}

/// Case-insensitive match at the exact length.
fn named(segment: &str, name: &str) -> bool {
    segment.eq_ignore_ascii_case(name)
}

/// Whether the frame holds the whole table.
fn complete<T: TableSchema>(frame: &DecodedFrame) -> bool {
    matches!(&*frame.tables[T::ID.index()], TableView::Complete(_))
}

fn rows<T: TableSchema>(frame: &DecodedFrame) -> (&[T::Row], String) {
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

/// A recorded query of a scripted text's own hour reads ([`crate::triggers::inputs`]).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextQuery {
    CountryName(u32, bool),
    CountryTag(u32),
    CharacterProperty(u32, String),
    CharacterName(u32),
    Capital(u32, bool),
    Overlord(u32),
    StateRecord(u32),
    StateCountry(u32, bool),
    CharacterCountry(u32),
    EventObject(EventObject, String),
    LocalPlayer,
    RulingParty(u32, String, bool),
    FactionName(u32),
    FactionNameById(u64),
    CharacterNamed(Object, String, EventObject),
    StateNameFor(u32, bool),
    ProvinceDisplayNamePrimary(u32),
    ProvinceDisplayName(u32),
    ProvinceNameOverride(u32),
    ProvinceVictoryPointName(u32),
    PartySupport(u32),
    /// A country's ruling party's (`None`) or a group's first party's leader.
    PartyLeader(u32, Option<String>),
    /// The long name of a country's first party of an exact group, as `tag`.
    NamedPartyName(u32, String, String),
    LastElection(u32),
    Manpower(u32),
    Agency(u32),
    OldName(u32, String, String),
    CharacterGender(u32),
    CountryPronoun(u32, String),
    DateProperty(String),
    CountriesGap,
}

impl ScriptedText<'_> {
    /// The digest `query` reads at this text's hour.
    pub fn reread_text(&self, query: &TextQuery) -> u64 {
        match query {
            TextQuery::CountryName(country, definite) => {
                crate::triggers::inputs::digest(&self.country_name_unrecorded(*country, *definite))
            }
            TextQuery::CountryTag(country) => {
                crate::triggers::inputs::digest(&self.country_tag_unrecorded(*country))
            }
            TextQuery::CharacterProperty(id, name) => {
                crate::triggers::inputs::digest(&self.character_property_unrecorded(*id, name))
            }
            TextQuery::CharacterName(id) => {
                crate::triggers::inputs::digest(&self.character_name_unrecorded(*id))
            }
            TextQuery::Capital(slot, original) => {
                crate::triggers::inputs::digest_debug(&self.capital_unrecorded(*slot, *original))
            }
            TextQuery::Overlord(slot) => {
                crate::triggers::inputs::digest(&self.overlord_unrecorded(*slot))
            }
            TextQuery::StateRecord(id) => {
                crate::triggers::inputs::digest(&self.state_record_unrecorded(*id))
            }
            TextQuery::StateCountry(id, owner) => {
                crate::triggers::inputs::digest_debug(&self.state_country_unrecorded(*id, *owner))
            }
            TextQuery::CharacterCountry(id) => {
                crate::triggers::inputs::digest_debug(&self.character_country_unrecorded(*id))
            }
            TextQuery::EventObject(object, name) => {
                crate::triggers::inputs::digest_debug(&self.event_object_unrecorded(*object, name))
            }
            TextQuery::LocalPlayer => {
                crate::triggers::inputs::digest(&self.local_player_unrecorded())
            }
            TextQuery::RulingParty(slot, tag, long) => {
                crate::triggers::inputs::digest(&self.ruling_party_unrecorded(*slot, tag, *long))
            }
            TextQuery::FactionName(slot) => {
                crate::triggers::inputs::digest(&self.faction_name_unrecorded(*slot))
            }
            TextQuery::FactionNameById(id) => {
                crate::triggers::inputs::digest(&self.faction_name_by_id_unrecorded(*id))
            }
            TextQuery::CharacterNamed(object, segment, event) => crate::triggers::inputs::digest(
                &self.character_named_unrecorded(*object, segment, *event),
            ),
            TextQuery::StateNameFor(id, plain) => {
                crate::triggers::inputs::digest(&self.state_name_for_unrecorded(*id, *plain))
            }
            TextQuery::ProvinceDisplayNamePrimary(id) => {
                crate::triggers::inputs::digest(&self.province_display_name_primary_unrecorded(*id))
            }
            TextQuery::ProvinceDisplayName(id) => {
                crate::triggers::inputs::digest(&self.province_display_name_unrecorded(*id))
            }
            TextQuery::ProvinceNameOverride(id) => {
                crate::triggers::inputs::digest(&self.province_name_override_unrecorded(*id))
            }
            TextQuery::ProvinceVictoryPointName(id) => {
                crate::triggers::inputs::digest(&self.province_victory_point_name_unrecorded(*id))
            }
            TextQuery::PartySupport(slot) => {
                crate::triggers::inputs::digest(&self.party_support_unrecorded(*slot))
            }
            TextQuery::PartyLeader(slot, group) => crate::triggers::inputs::digest(
                &self.party_leader_unrecorded(*slot, group.as_deref()),
            ),
            TextQuery::NamedPartyName(slot, group, tag) => crate::triggers::inputs::digest(
                &self.named_party_name_unrecorded(*slot, group, tag),
            ),
            TextQuery::LastElection(slot) => {
                crate::triggers::inputs::digest(&self.last_election_unrecorded(*slot))
            }
            TextQuery::Manpower(slot) => {
                crate::triggers::inputs::digest(&self.manpower_unrecorded(*slot))
            }
            TextQuery::Agency(slot) => {
                crate::triggers::inputs::digest(&self.agency_unrecorded(*slot))
            }
            TextQuery::OldName(slot, entry, suffix) => {
                crate::triggers::inputs::digest(&self.old_name_unrecorded(*slot, entry, suffix))
            }
            TextQuery::CharacterGender(id) => {
                crate::triggers::inputs::digest(&self.character_gender_unrecorded(*id))
            }
            TextQuery::CountryPronoun(slot, name) => {
                crate::triggers::inputs::digest(&self.country_pronoun_unrecorded(*slot, name))
            }
            TextQuery::DateProperty(name) => {
                crate::triggers::inputs::digest(&self.date_property_unrecorded(name))
            }
            TextQuery::CountriesGap => {
                crate::triggers::inputs::digest(&self.countries_gap_unrecorded())
            }
        }
    }
}

impl ScriptedText<'_> {
    /// Why Countries is not complete this hour, None when it is.
    pub(crate) fn countries_gap(&self) -> Option<String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::CountriesGap),
            || self.countries_gap_unrecorded(),
        )
    }

    pub(crate) fn countries_gap_unrecorded(&self) -> Option<String> {
        (!complete::<tables::Countries>(self.frame))
            .then(|| rows::<tables::Countries>(self.frame).1)
    }
}

/// The hour's Clock random seed, if recorded.
pub fn clock_seed(frame: &DecodedFrame) -> Option<u32> {
    frame
        .complete::<tables::Clock>()
        .ok()
        .and_then(|rows| rows.first())
        .map(|clock| clock.seed as u32)
}

impl<'a> ScriptedText<'a> {
    /// The context of `root` (a canonical country slot) at `frame`'s hour,
    /// the original-tag lists read from this frame alone
    /// ([`RecordedScopes::read`]).
    pub fn new(
        frame: &'a DecodedFrame,
        localisation: &'a Localisation,
        content: &'a ScriptedContent,
        root: u32,
    ) -> Self {
        Self::with_scopes(
            frame,
            localisation,
            content,
            root,
            RecordedScopes::read(frame, std::sync::Arc::clone(&content.tag_aliases)),
        )
    }

    /// [`Self::new`] with the hour's scopes already read, for example with
    /// the original-tag lists derived from the branch
    /// ([`RecordedScopes::read_with`]).
    pub fn with_scopes(
        frame: &'a DecodedFrame,
        localisation: &'a Localisation,
        content: &'a ScriptedContent,
        root: u32,
        scopes: RecordedScopes,
    ) -> Self {
        Self {
            localisation,
            content,
            scopes,
            names: recorded_names::RecordedNames(CountryNames::read(frame, localisation)),
            lexical_names: crate::faction_names::LexicalNames::new(
                frame.tables[replay_data::rows::TableId::LexicalTokens.index()].clone(),
            ),
            frame,
            root,
            nato_counters: installed_content_settings::settings::Counters::default().nato == 1,
            world: std::sync::OnceLock::new(),
        }
    }

    /// Unit-bonus icons, including nested effect descriptions, follow the view.
    pub fn with_nato_counters(mut self, nato: bool) -> Self {
        self.nato_counters = nato;
        self
    }

    /// Expands `text` from depth zero. The view's event scope is its
    /// country, as ROOT, THIS, FROM and PREV.
    pub fn expand(&self, text: &str) -> Result<String, String> {
        let scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(self.root))]);
        self.expand_in(text, &scopes)
    }

    /// A faction screen's localisation environment, with the selected country
    /// as its event country and the explicit faction as its faction property.
    pub fn expand_faction(&self, text: &str, faction: u64) -> Result<String, String> {
        let scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(self.root))]);
        self.expand_in_description(text, &scopes, 0, None, None, Some(Some(faction)))
            .0
    }

    /// Organisation name before expansion: the localised text and the fresh
    /// organisation/owner-FROM scope pair. Neither depends on the root, and
    /// text without `[` expands to itself.
    pub fn organisation_template(&self, reference: u64) -> Result<(String, EventScopes), String> {
        let organisations = self
            .frame
            .complete::<tables::IndustrialOrganisations>()
            .map_err(|why| why.to_string())?;
        let row = organisations
            .iter()
            .find(|row| row.organisation == reference)
            .ok_or_else(|| format!("IndustrialOrganisations lacks organisation {reference:#x}"))?;
        let name = if row.name.is_empty() {
            let tokens = self
                .frame
                .complete::<tables::LexicalTokens>()
                .map_err(|why| why.to_string())?;
            let at = tokens
                .binary_search_by_key(&row.token, |token| token.token)
                .map_err(|_| "LexicalTokens lacks industrial organisation name")?;
            tokens[at].literal.as_slice()
        } else {
            row.name.as_slice()
        };
        let name =
            std::str::from_utf8(name).map_err(|_| "industrial organisation name is not UTF-8")?;
        let owner = self
            .scopes
            .slot(row.owner)
            .ok_or("CountryLinks lacks industrial organisation owner")?;
        let tagged = format!("{}_{name}", self.country_tag(owner)?);
        let key = if owner > 0 && self.localisation.get(&tagged).is_some() {
            tagged.as_str()
        } else {
            name
        };
        Ok((
            self.localise(key)?,
            EventScopes::organisation(reference, Some(owner)),
        ))
    }

    /// Expand in the caller's event scopes, the first being the caller's own.
    pub fn expand_in(&self, text: &str, scopes: &EventScopes) -> Result<String, String> {
        if scopes.scopes.is_empty() {
            return Err("scripted text needs an event scope".into());
        }
        self.expand_in_at(text, scopes, 0)
    }

    /// Expand from a caller-owned scope cursor without flattening its links.
    pub fn expand_in_at(
        &self,
        text: &str,
        scopes: &EventScopes,
        at: usize,
    ) -> Result<String, String> {
        if at >= scopes.scopes.len() {
            return Err(format!("scripted text event scope {at} is not given"));
        }
        let expanded = ExpansionScopes::new(scopes);
        self.process(text, 0, &expanded, at, Object::Event)
    }

    /// Scripted entries inside effect descriptions share their active variable context.
    pub fn expand_in_session(
        &self,
        text: &str,
        scopes: &EventScopes,
        at: usize,
        evaluator: &crate::triggers::Evaluator<'_, 'a>,
    ) -> Result<String, String> {
        // Only immutable World/Combat references cross on_large_stack;
        // RefCells and the caller evaluator remain on their calling thread.
        let mut expanded = ExpansionScopes::new(&EventScopes::new(Vec::new()));
        expanded.world = Some(evaluator.world());
        expanded.combat = evaluator.combat_context();
        let (result, session) = self.expand_in_description(
            text,
            scopes,
            at,
            Some(evaluator.variable_session()),
            Some(&expanded),
            None,
        );
        if let Some(session) = session.as_ref() {
            evaluator.restore_variable_session(session);
        }
        result
    }

    pub(crate) fn expand_in_description(
        &self,
        text: &str,
        scopes: &EventScopes,
        at: usize,
        session: Option<crate::triggers::VariableSession>,
        parent: Option<&ExpansionScopes<'_, 'a>>,
        faction_environment: Option<Option<u64>>,
    ) -> (
        Result<String, String>,
        Option<crate::triggers::VariableSession>,
    ) {
        if at >= scopes.scopes.len() {
            return (
                Err("scripted text event scope is not given".into()),
                session,
            );
        }
        let fresh;
        let expanded = match parent {
            Some(parent) => parent,
            None => {
                fresh = ExpansionScopes::new(&EventScopes::new(Vec::new()));
                &fresh
            }
        };
        let appended = expanded.append_description(scopes);
        expanded.session.replace(session);
        let object = faction_environment.map_or(Object::Event, Object::FactionEnvironment);
        let result = self.process(text, 0, expanded, at + appended.offset, object);
        let session = expanded.session.borrow().clone();
        (result, session)
    }

    pub(crate) fn recorded(&self) -> &RecordedScopes {
        &self.scopes
    }
    pub(crate) fn frame(&self) -> &'a DecodedFrame {
        self.frame
    }

    pub(crate) fn content(&self) -> &'a ScriptedContent {
        self.content
    }

    /// The digest a recorded query reads in this hour's world, scopes or
    /// names; None for a whole table ([`crate::triggers::inputs`]).
    pub fn reread(&self, key: &crate::triggers::inputs::Key) -> Option<u64> {
        use crate::triggers::inputs::Key;
        match key {
            Key::Table(_) | Key::Fields(..) | Key::FocusTag(_) => None,
            Key::ClockSeed => Some(crate::triggers::inputs::digest(&clock_seed(self.frame))),
            Key::World(query) => self.trigger_world().ok().map(|world| world.reread(query)),
            Key::Scopes(query) => Some(self.scopes.reread(query)),
            Key::Names(query) => Some(self.names.reread(query)),
            Key::Text(query) => Some(self.reread_text(query)),
        }
    }

    pub(crate) fn country_names(&self) -> &recorded_names::RecordedNames<'_> {
        &self.names
    }

    pub(crate) fn viewing_country(&self) -> u32 {
        self.root
    }

    #[cfg(test)]
    pub(crate) fn set_synthetic_world(&self, world: crate::triggers::World<'a>) {
        assert!(self.world.set(Ok(world)).is_ok());
    }

    pub(crate) fn country_name(&self, country: u32, definite: bool) -> Result<String, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::CountryName(country, definite)),
            || self.country_name_unrecorded(country, definite),
        )
    }

    pub(crate) fn country_name_unrecorded(
        &self,
        country: u32,
        definite: bool,
    ) -> Result<String, String> {
        if definite {
            self.names.name_with(country, "_DEF")
        } else {
            self.names.name(country)
        }
    }

    /// Tag lookup uses database slot zero for raw zero.
    pub(crate) fn country_tag(&self, country: u32) -> Result<&str, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::CountryTag(country)),
            || self.country_tag_unrecorded(country),
        )
    }

    pub(crate) fn country_tag_unrecorded(&self, country: u32) -> Result<&str, String> {
        if country == 0 {
            installed_content_country_names::country_names::reserved_zero_tag(self.frame)
        } else {
            self.scopes
                .tag(country)
                .ok_or_else(|| format!("Countries lacks tag for country {country}"))
        }
    }

    /// Flag-marked definite country name with its first byte capitalised.
    pub(crate) fn country_marked_name(&self, country: u32) -> Result<String, String> {
        let tag = self.country_tag(country)?;
        let name = crt_case_text(&self.names.name_with(country, "_DEF")?, true, true);
        Ok(format!("\u{12}{tag} {name}"))
    }

    /// The country adjective resolved from its recorded naming inputs.
    pub(crate) fn country_adjective(&self, country: u32) -> Result<String, String> {
        self.names.name_with(country, "_ADJ")
    }

    pub(crate) fn recorded_language(&self) -> &str {
        &self.content.recorded_language
    }

    /// Expand a caller scope whose random word is not recorded. Ordinary
    /// expressions remain usable; an unseeded multi-member random_list refuses
    /// only when it reads that word. ROOT/FROM/PREV select a concrete context
    /// whose entry fallback constructs the separate known 424242 scope;
    /// bare entries keep the caller scope.
    pub fn expand_in_without_random_seed(
        &self,
        text: &str,
        scopes: &EventScopes,
        reason: &'static str,
    ) -> Result<String, String> {
        if scopes.scopes.is_empty() {
            return Err("scripted text needs an event scope".into());
        }
        if !text.contains('[') {
            return Ok(text.to_owned());
        }
        let expansion = ExpansionScopes::new(scopes);
        expansion.scopes.borrow_mut()[0].random_seed_refusal = Some(reason);
        self.process(text, 0, &expansion, 0, Object::Event)
    }

    fn on_large_stack<'e>(
        &self,
        scopes: &ExpansionScopes<'e, 'a>,
        expand: impl FnOnce(&ExpansionScopes<'e, 'a>) -> Result<String, String> + Send,
    ) -> Result<String, String> {
        let mut expanded = scopes.clone();
        expanded.on_large_stack = true;
        let caller = job_priority::Caller::current();
        let ((result, session), noted) = std::thread::scope(|thread_scope| {
            std::thread::Builder::new()
                .name("scripted-localisation".into())
                // Reserve 64 KiB per backstop level for the entry's debug call frames.
                .stack_size(MAX_SCRIPTED_NESTING * 64 * 1024)
                .spawn_scoped(thread_scope, move || {
                    caller.enter();
                    // The expansion's catalogue notes belong to the calling consumer.
                    crate::catalogue::capture(move || {
                        let result = expand(&expanded);
                        (result, expanded.session.into_inner())
                    })
                })
                .map_err(|error| {
                    format!("scripted localisation expansion thread could not start: {error}")
                })?
                .join()
                .map_err(|_| "scripted localisation expansion thread failed".to_owned())
        })?;
        crate::catalogue::credit(&noted);
        scopes.session.replace(session);
        result
    }

    /// Expands in a fresh country event, within the same game world.
    fn expand_fresh_country(
        &self,
        text: &str,
        slot: u32,
        name: &str,
        scopes: &ExpansionScopes<'_, 'a>,
    ) -> Result<String, String> {
        if scopes.needs_large_stack() {
            return self.on_large_stack(scopes, |scopes| {
                self.expand_fresh_country(text, slot, name, scopes)
            });
        }
        let depth = scopes.fresh_depth.get();
        if scopes.active.borrow().len() + depth >= MAX_SCRIPTED_NESTING {
            return Err(nesting_refusal(name));
        }
        let at = scopes.scopes.borrow().len();
        scopes
            .scopes
            .borrow_mut()
            .push(EventScope::of(EventObject::Country(slot)));
        scopes.prev_distance.borrow_mut().push(None);
        let variables = scopes.variables.replace(ScopeVariables::default());
        scopes.fresh_depth.set(depth + 1);
        let expanded = self.process(text, 0, scopes, at, Object::Event);
        scopes.fresh_depth.set(depth);
        scopes.variables.set(variables);
        scopes.scopes.borrow_mut().truncate(at);
        scopes.prev_distance.borrow_mut().truncate(at);
        expanded
    }

    /// Process text with the context's saved event scope `start`.
    fn process(
        &self,
        text: &str,
        depth: i32,
        scopes: &ExpansionScopes<'_, 'a>,
        start: usize,
        context: Object,
    ) -> Result<String, String> {
        if depth >= self.content.recursion {
            return Ok(text.to_owned());
        }
        let mut out = String::new();
        let mut rest = text;
        loop {
            let Some(open) = rest.find('[') else {
                out.push_str(rest);
                return Ok(out);
            };
            // No `]` after it: return before appending the text
            // before the `[`.
            let Some(length) = rest[open + 1..].find(']') else {
                return Ok(out);
            };
            let inner = &rest[open + 1..open + 1 + length];
            out.push_str(&rest[..open]);
            if inner.len() >= 2 && inner.starts_with('(') && inner.ends_with(')') {
                return Err(format!(
                    "the conditional scripted text [{inner}] is not evaluated"
                ));
            }
            if let Some(variable) = inner.strip_prefix('?') {
                out.push_str(&self.numeric_variable(variable, scopes, start)?);
                rest = &rest[open + 1 + length + 1..];
                continue;
            }
            let value = self.bracket(inner, scopes, start, context)?;
            out.push_str(&self.process(&value, depth + 1, scopes, start, context)?);
            rest = &rest[open + 1 + length + 1..];
        }
    }

    /// One `[A.B.Property]`, from the context's saved event scope `start`,
    /// restored before each bracket.
    fn bracket(
        &self,
        inner: &str,
        scopes: &ExpansionScopes<'_, 'a>,
        start: usize,
        context: Object,
    ) -> Result<String, String> {
        let segments: Vec<&str> = inner.split('.').collect();
        let (property, promotions) = segments.split_last().unwrap_or((&"", &[]));
        let mut object = context;
        let mut active = start;
        let mut event_active = !matches!(context, Object::FactionEnvironment(_));
        for segment in promotions {
            object = self
                .promote(object, segment, scopes, &mut active, &mut event_active)
                .map_err(|reason| format!("[{inner}]: {reason}"))?;
        }
        self.property(object, property, scopes, active, event_active)
            .map_err(|reason| format!("[{inner}]: {reason}"))
    }

    /// Promote one segment; `active` is the context's event-scope index.
    fn promote(
        &self,
        object: Object,
        segment: &str,
        scopes: &ExpansionScopes<'_, 'a>,
        active: &mut usize,
        event_active: &mut bool,
    ) -> Result<Object, String> {
        // Environment selectors retain both carried fields.
        if object == Object::Event
            && let EventObject::CountryState { country, state } = scopes.scope(*active).this
        {
            if named(segment, "Country") {
                return Ok(Object::Country(country));
            }
            if named(segment, "State") {
                return Ok(Object::State(Some(state)));
            }
        }
        // 1. The current kind's table.
        match object {
            Object::FactionEnvironment(faction) => {
                if named(segment, "Faction") {
                    return Ok(Object::Faction(faction));
                }
                if ["Building", "Province", "Combat"]
                    .iter()
                    .any(|name| named(segment, name))
                {
                    return Err(format!(
                        "{segment}: null faction-environment promotion is not implemented"
                    ));
                }
            }
            Object::Faction(faction) => {
                if named(segment, "Leader") {
                    let Some(faction) = faction else {
                        return Ok(object);
                    };
                    let raw = self
                        .scopes
                        .faction_first_raw(faction)
                        .map_err(|why| why.to_string())?;
                    // Text keeps the actual Country0 object; event-scope clearing
                    // is a separate operation and is not applied here.
                    let country = self.scopes.raw_slot(raw).map_err(|why| why.to_string())?;
                    return Ok(Object::Country(country));
                }
            }
            Object::Country(slot) => {
                if named(segment, "Capital") || named(segment, "OriginalCapital") {
                    return self.capital(slot, named(segment, "OriginalCapital"));
                }
                if named(segment, "Overlord") {
                    // No overlord: logged, the context is unchanged.
                    return Ok(self.overlord(slot)?.map_or(object, Object::Country));
                }
                if named(segment, "FactionLeader") {
                    // Promotion keeps the context when the
                    // country has no faction.
                    return Ok(
                        match self
                            .scopes
                            .faction_leader_raw(slot)
                            .map_err(|why| why.to_string())?
                        {
                            Some(raw) => Object::Country(
                                self.scopes.raw_slot(raw).map_err(|why| why.to_string())?,
                            ),
                            None => object,
                        },
                    );
                }
            }
            Object::State(state) => {
                if named(segment, "Owner") || named(segment, "Controller") {
                    let Some(id) = state else {
                        // The handler runs on a null object: nothing changes.
                        return Ok(object);
                    };
                    return self.state_country(id, named(segment, "Owner"));
                }
                if named(segment, "Occupied") {
                    return state.map_or(Ok(object), |id| self.state_occupied(id));
                }
            }
            Object::Character(id) => {
                if named(segment, "Owner") {
                    return self.character_country(id);
                }
            }
            Object::Event => {}
        }
        // 1 in kind 1, and 2 from every kind: the event-scope table.
        if named(segment, "Player") {
            return self.local_player().map(Object::Country);
        }
        // On the active scope, THIS keeps it; ROOT, FROM and PREV follow
        // its links. The chosen scope becomes active and its object
        // becomes the context.
        let current = scopes.scope(*active);
        for (name, link) in [
            ("THIS", Link::Itself),
            ("Root", current.root),
            ("From", current.from),
            ("Prev", current.prev),
        ] {
            if named(segment, name) {
                if !*event_active && !matches!(object, Object::FactionEnvironment(_)) {
                    return Err(format!(
                        "{segment}: null active event pointer in faction environment is not implemented"
                    ));
                }
                let chosen = scopes.linked(*active, link, name)?;
                *active = chosen;
                *event_active = true;
                return self.event_object(scopes.scope(chosen).this, name);
            }
        }
        // 3. A country tag (exact), a character of the current country, a
        //    state ID. Each is found, proven absent, or refused.
        if self.scopes.country(segment).is_some() {
            // Resolve a country tag as a script scope.
            return self
                .scopes
                .resolve_tag(segment)
                .map(Object::Country)
                .map_err(|why| why.to_string());
        }
        // Then the tag alias database: resolve in a default scope, with
        // ROOT, FROM and PREV itself, set with clearing to the local
        // player's country.
        if self.scopes.alias_entry(segment).is_some() {
            let local = self
                .local_player()
                .map_err(|why| format!("tag alias {segment}: {why}"))?;
            let root = crate::triggers::Scope::Country(local);
            let scopes = crate::triggers::Scopes {
                prev: Some(root),
                from: Some(root),
                bare: true,
                ..crate::triggers::Scopes::of(root)
            };
            return self
                .world()
                .and_then(|world| world.resolve_tag(segment, scopes))
                .map_err(|why| format!("tag alias {segment}: {why}"))?
                .map(Object::Country)
                .ok_or_else(|| format!("tag alias {segment} names no country this hour"));
        }
        if let Some(refusal) = self.scopes.alias(segment) {
            return Err(refusal);
        }
        if let Some(gap) = self.countries_gap() {
            return Err(format!(
                "{segment}: whether it is a country tag is unknown: Countries {gap}"
            ));
        }
        if (*event_active || matches!(object, Object::Country(_)))
            && let Some(id) = self.character_named(object, segment, scopes.scope(*active).this)?
        {
            return Ok(Object::Character(id));
        }
        let digits = segment.strip_prefix(['-', '+']).unwrap_or(segment);
        if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
            let id = installed_content_core::fixed5::decimal_i32(segment.as_bytes());
            if id <= 0 {
                // Consumed, the context unchanged.
                return Ok(object);
            }
            let (found, complete, gap) = self.state_record(id as u32);
            if found {
                return Ok(Object::State(Some(id as u32)));
            }
            if complete {
                // A positive id no state holds is a null state object.
                return Ok(Object::State(None));
            }
            return Err(format!("state {segment} is unknown: States {gap}"));
        }
        // 4. Event context only: saved event targets use their own object
        // selector, including text kinds unsupported by the trigger evaluator.
        if object == Object::Event {
            return self.saved_promotion(segment, scopes, *active);
        }
        // 5. The kind-2 names, on the current object.
        if named(segment, "Country") && matches!(object, Object::Country(_)) {
            return Ok(object);
        }
        // 6. Unknown: logged, the context unchanged.
        Ok(object)
    }

    /// Local tooltip/saved, then global tooltip/saved targets; a missing
    /// or invalid target leaves the promoted object alone.
    fn saved_promotion(
        &self,
        name: &str,
        scopes: &ExpansionScopes<'_, 'a>,
        active: usize,
    ) -> Result<Object, String> {
        let mut graph = scopes.targets.borrow().clone();
        graph.scopes = scopes.scopes.borrow().clone();
        if let Some(target) = crate::triggers::event_tooltip_target(&graph, active, name.as_bytes())
            .map_err(|why| why.to_string())?
        {
            return self.event_object(target, name);
        }
        use crate::triggers::saved_event_targets::{Reader, Selected};
        let selected = self
            .scopes
            .saved_selection(name.as_bytes(), Reader::Text)
            .map_err(|why| why.to_string())?;
        match selected {
            Selected::Missing | Selected::Caller => Ok(Object::Event),
            Selected::State(id) => {
                if id <= 0 {
                    return Ok(Object::State(None));
                }
                let (found, complete, gap) = self.state_record(id as u32);
                if !found && !complete {
                    return Err(format!(
                        "saved event target {name}: state {id} is unknown: States {gap}"
                    ));
                }
                Ok(Object::State(found.then_some(id as u32)))
            }
            Selected::Country(raw) => self
                .scopes
                .saved_country(raw, name.as_bytes())
                .map_err(|why| why.to_string())?
                .map(Object::Country)
                .ok_or_else(|| {
                    format!("saved event target {name}: country {raw} has no recorded object")
                }),
            Selected::Character(id) => Ok(Object::Character(id)),
            Selected::Leader(_) => Err(Reader::Text.refusal(name.as_bytes(), "unit leader")),
            Selected::Unsupported(kind) => Err(Reader::Text.refusal(name.as_bytes(), kind)),
        }
    }

    /// Property text in the promoted context.
    fn property(
        &self,
        object: Object,
        name: &str,
        scopes: &ExpansionScopes<'_, 'a>,
        active: usize,
        event_active: bool,
    ) -> Result<String, String> {
        let property = match object {
            // Faction contexts have no scripted-entry fallback.
            Object::FactionEnvironment(_) => return Ok(String::new()),
            Object::Faction(faction) => {
                return if named(name, "GetName") {
                    faction.map_or_else(|| Ok("NULL".into()), |id| self.faction_name_by_id(id))
                } else {
                    Ok(String::new())
                };
            }
            Object::Event => self.date_property(name)?,
            Object::Country(slot) => self.country_property(slot, name, scopes)?,
            Object::State(state) => {
                const STATE: [&str; 4] = [
                    "GetName",
                    "GetID",
                    "GetCapitalVictoryPointName",
                    "GetContinent",
                ];
                match (STATE.iter().find(|entry| named(name, entry)), state) {
                    (Some(_), None) => Some("NULL".to_owned()),
                    (Some(&"GetID"), Some(id)) => Some(id.to_string()),
                    (Some(&"GetName"), Some(id)) => Some(self.state_name(id)?),
                    (Some(&"GetContinent"), Some(id)) => Some(self.state_continent(id)?),
                    (Some(&"GetCapitalVictoryPointName"), Some(id)) => {
                        Some(self.state_capital_victory_point_name(id)?)
                    }
                    (Some(entry), Some(id)) => {
                        return Err(format!(
                            "{entry} of state {id}: the victory-point/continent text selector is not implemented"
                        ));
                    }
                    (None, _) => None,
                }
            }
            Object::Character(id) => self.character_property(id, name)?,
        };
        let Some(text) = property else {
            // A miss: a scripted localisation of that name, else
            // empty text.
            return self.entry(object, name, scopes, active, event_active);
        };
        Ok(text)
    }

    fn localise(&self, key: &str) -> Result<String, String> {
        self.localisation
            .localise(key.as_bytes())
            .map_err(|refusal| refusal.to_string())
    }

    /// [`stored_text`] in the recording client's language.
    fn stored_text(&self, bytes: &[u8], what: &str) -> Result<String, String> {
        stored_text(
            self.localisation,
            &self.content.recorded_language,
            bytes,
            what,
        )
    }

    /// The country table's properties, `None` for a
    /// name the table lacks.
    fn country_property(
        &self,
        slot: u32,
        name: &str,
        scopes: &ExpansionScopes<'_, 'a>,
    ) -> Result<Option<String>, String> {
        let tag = self.country_tag(slot)?.to_owned();
        let names = |suffix: &str| self.names.name_with(slot, suffix);
        // Non-ideology properties probe the current tag without ideology.
        let non_ideology = |suffix: &str| -> Result<String, String> {
            let key = format!("{tag}{suffix}");
            if self.localisation.get(&key).is_some() {
                self.localise(&key)
            } else {
                names(suffix)
            }
        };
        let is = |entry: &str| named(name, entry);
        // The first byte's toupper in the "C" locale ([`crt_case_text`]).
        let upper_first = |text: String| crt_case_text(&text, true, true);
        Ok(Some(if is("GetName") {
            names("")?
        } else if is("GetNameDef") {
            names("_DEF")?
        } else if is("GetNameDefCap") {
            upper_first(names("_DEF")?)
        } else if is("GetAdjective") {
            self.country_adjective(slot)?
        } else if is("GetAdjectiveCap") {
            upper_first(self.country_adjective(slot)?)
        } else if is("GetTag") {
            tag
        } else if is("GetFlag") {
            format!("\u{12}{tag}")
        } else if is("GetNameWithFlag") {
            self.country_marked_name(slot)?
        } else if let Some(&(entry, suffix, cap)) = OLD_NAMES.iter().find(|(entry, ..)| is(entry)) {
            let text = self.old_name(slot, entry, suffix)?;
            if cap { upper_first(text) } else { text }
        } else if is("GetNonIdeologyName") {
            non_ideology("")?
        } else if is("GetNonIdeologyNameDef") {
            non_ideology("_DEF")?
        } else if is("GetNonIdeologyNameDefCap") {
            upper_first(non_ideology("_DEF")?)
        } else if is("GetNonIdeologyAdjective") {
            non_ideology("_ADJ")?
        } else if is("GetNonIdeologyAdjectiveCap") {
            upper_first(non_ideology("_ADJ")?)
        } else if is("GetRulingIdeology") || is("GetRulingIdeologyNoun") {
            let Some(group) = self.names.ruling_group(slot)? else {
                return Err(format!("{name}: {tag} has no ruling party"));
            };
            let key = if is("GetRulingIdeologyNoun") {
                format!("{group}_noun")
            } else {
                group.to_owned()
            };
            // Lowercase with the "C" locale's tolower.
            crt_case_text(&self.localise(&key)?, false, false)
        } else if is("GetRulingParty") || is("GetRulingPartyLong") {
            self.ruling_party(slot, &tag, is("GetRulingPartyLong"))?
        } else if is("GetLeader") {
            self.party_leader(slot, None)?
        } else if let Some(group) = properties::leader_group(name) {
            self.party_leader(slot, Some(group))?
        } else if let Some(group) = properties::party_group(name) {
            self.named_party_name(slot, group, &tag)?
        } else if is("GetPartySupport") {
            self.party_support(slot)?
        } else if is("GetLastElection") {
            self.last_election(slot)?
        } else if PRONOUNS.iter().any(|entry| is(entry)) {
            self.country_pronoun(slot, name)?
        } else if is("GetManpower") {
            self.manpower(slot)?.to_string()
        } else if is("GetAgency") {
            self.agency(slot)?
        } else if let Some(text) = self.balance(slot, name, scopes)? {
            text
        } else if is("GetFactionName") {
            self.faction_name(slot)?
        } else {
            return Ok(None);
        }))
    }

    fn character_property(&self, id: u32, name: &str) -> Result<Option<String>, String> {
        crate::triggers::inputs::query(
            || {
                crate::triggers::inputs::Key::Text(TextQuery::CharacterProperty(
                    id,
                    name.to_owned(),
                ))
            },
            || self.character_property_unrecorded(id, name),
        )
    }

    pub(crate) fn character_property_unrecorded(
        &self,
        id: u32,
        name: &str,
    ) -> Result<Option<String>, String> {
        let is = |entry: &str| named(name, entry);
        if is("GetName") {
            // Character GetName copies the stored name directly;
            // party leaders instead use the token fallback.
            let (characters, gap) = rows::<tables::Characters>(self.frame);
            let row = characters
                .iter()
                .find(|row| row.id == id)
                .ok_or_else(|| format!("character {id} has no Characters row: {gap}"))?;
            return String::from_utf8(row.name.clone())
                .map(Some)
                .map_err(|_| format!("character {id}'s name is not UTF-8"));
        }
        if is("GetIdeology") || is("GetIdeologyGroup") {
            return Err(format!(
                "{name}: a character's country-leader ideology is not recorded per character"
            ));
        }
        if PRONOUNS.iter().any(|entry| is(entry)) {
            return self
                .pronoun(name, self.character_gender(id)? == 2)
                .map(Some);
        }
        Ok(None)
    }

    /// The stored character name, else the saved name key through the localiser.
    fn character_name(&self, id: u32) -> Result<String, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::CharacterName(id)),
            || self.character_name_unrecorded(id),
        )
    }

    pub(crate) fn character_name_unrecorded(&self, id: u32) -> Result<String, String> {
        let (characters, gap) = rows::<tables::Characters>(self.frame);
        let row = characters
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| format!("character {id} has no Characters row: {gap}"))?;
        if !row.name.is_empty() {
            return String::from_utf8(row.name.clone())
                .map_err(|_| format!("character {id}'s name is not UTF-8"));
        }
        self.localisation
            .localise(&row.token)
            .map_err(|refusal| refusal.to_string())
    }

    /// Capital or original capital, falling back to capital: a null state
    /// when invalid, so a property then reads `NULL`.
    fn capital(&self, slot: u32, original: bool) -> Result<Object, String> {
        crate::triggers::inputs::query_debug(
            || crate::triggers::inputs::Key::Text(TextQuery::Capital(slot, original)),
            || self.capital_unrecorded(slot, original),
        )
    }

    pub(crate) fn capital_unrecorded(&self, slot: u32, original: bool) -> Result<Object, String> {
        let (politics, gap) = rows::<tables::Politics>(self.frame);
        let row = politics
            .iter()
            .find(|row| self.scopes.slot(row.country) == Some(slot))
            .ok_or_else(|| {
                format!("the capital of country slot {slot} is unknown: Politics {gap}")
            })?;
        let capital = if original && row.original_capital != 0 {
            row.original_capital
        } else {
            row.capital
        };
        Ok(Object::State(
            u32::try_from(capital).ok().filter(|id| *id > 0),
        ))
    }

    fn overlord(&self, slot: u32) -> Result<Option<u32>, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::Overlord(slot)),
            || self.overlord_unrecorded(slot),
        )
    }

    pub(crate) fn overlord_unrecorded(&self, slot: u32) -> Result<Option<u32>, String> {
        let (appearance, gap) = rows::<tables::CountryAppearance>(self.frame);
        let row = appearance
            .iter()
            .find(|row| self.scopes.slot(row.country) == Some(slot))
            .ok_or_else(|| {
                format!("the overlord of country slot {slot} is unknown: CountryAppearance {gap}")
            })?;
        // A positive overlord becomes the context; none
        // is logged and leaves it unchanged.
        let link = match u32::try_from(row.overlord) {
            Ok(link) if link > 0 => link,
            _ => return Ok(None),
        };
        let (_, links) = rows::<tables::CountryLinks>(self.frame);
        self.scopes.slot(link).map(Some).ok_or_else(|| {
            format!(
                "the overlord link {link} of country slot {slot} resolves to no country: CountryLinks {links}"
            )
        })
    }

    /// The row of country `slot` in table `T`, whose rows name their country
    /// by `link`; `None` only where the frame proves there is none: the table
    /// is whole, and CountryLinks is whole or every row's link resolves.
    fn country_row<T: TableSchema>(
        &self,
        slot: u32,
        link: impl Fn(&T::Row) -> u32,
    ) -> Result<Option<&'a T::Row>, String> {
        let (found, gap) = rows::<T>(self.frame);
        if let Some(row) = found
            .iter()
            .find(|row| self.scopes.slot(link(row)) == Some(slot))
        {
            return Ok(Some(row));
        }
        if !complete::<T>(self.frame) {
            return Err(format!("{:?} {gap}", T::ID));
        }
        if !complete::<tables::CountryLinks>(self.frame)
            && let Some(row) = found
                .iter()
                .find(|row| self.scopes.slot(link(row)).is_none())
        {
            let (_, links) = rows::<tables::CountryLinks>(self.frame);
            return Err(format!(
                "a {:?} row's country link {} resolves to no country: CountryLinks {links}",
                T::ID,
                link(row)
            ));
        }
        Ok(None)
    }

    /// Whether States records a state `id`, whether States is complete, and
    /// why not.
    fn state_record(&self, id: u32) -> (bool, bool, String) {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::StateRecord(id)),
            || self.state_record_unrecorded(id),
        )
    }

    pub(crate) fn state_record_unrecorded(&self, id: u32) -> (bool, bool, String) {
        let (states, gap) = rows::<tables::States>(self.frame);
        (
            states.iter().any(|state| state.id == id),
            complete::<tables::States>(self.frame),
            gap,
        )
    }

    fn state_country(&self, id: u32, owner: bool) -> Result<Object, String> {
        crate::triggers::inputs::query_debug(
            || crate::triggers::inputs::Key::Text(TextQuery::StateCountry(id, owner)),
            || self.state_country_unrecorded(id, owner),
        )
    }

    pub(crate) fn state_country_unrecorded(&self, id: u32, owner: bool) -> Result<Object, String> {
        let (states, gap) = rows::<tables::States>(self.frame);
        let state = states
            .iter()
            .find(|state| state.id == id)
            .ok_or_else(|| format!("state {id} has no States row: {gap}"))?;
        let link = if owner { state.owner } else { state.controller };
        self.scopes
            .slot(link)
            .map(Object::Country)
            .ok_or_else(|| format!("state {id}'s country link {link} resolves to no country"))
    }

    fn character_country(&self, id: u32) -> Result<Object, String> {
        crate::triggers::inputs::query_debug(
            || crate::triggers::inputs::Key::Text(TextQuery::CharacterCountry(id)),
            || self.character_country_unrecorded(id),
        )
    }

    pub(crate) fn character_country_unrecorded(&self, id: u32) -> Result<Object, String> {
        let (characters, gap) = rows::<tables::Characters>(self.frame);
        let row = characters
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| format!("character {id} has no Characters row: {gap}"))?;
        self.scopes
            .slot(row.country)
            .map(Object::Country)
            .ok_or_else(|| {
                format!(
                    "character {id}'s country link {} resolves to no country",
                    row.country
                )
            })
    }

    /// Select the context from an event scope: its character first,
    /// then its country, then its state.
    fn event_object(&self, object: EventObject, name: &str) -> Result<Object, String> {
        crate::triggers::inputs::query_debug(
            || crate::triggers::inputs::Key::Text(TextQuery::EventObject(object, name.to_owned())),
            || self.event_object_unrecorded(object, name),
        )
    }

    pub(crate) fn event_object_unrecorded(
        &self,
        object: EventObject,
        name: &str,
    ) -> Result<Object, String> {
        Ok(match object {
            EventObject::Country(slot) | EventObject::CountryState { country: slot, .. } => {
                Object::Country(slot)
            }
            EventObject::State(id) => Object::State(Some(id)),
            EventObject::Character(id) => Object::Character(id),
            EventObject::UnitLeader(reference)
            | EventObject::CountryUnitLeader {
                leader: reference, ..
            } => {
                let (leaders, gap) = rows::<tables::UnitLeaders>(self.frame);
                let row = leaders
                    .iter()
                    .find(|row| {
                        (u64::from(row.id) << 32 | u64::from(row.reference_type)) == reference
                    })
                    .ok_or_else(|| {
                        format!(
                            "{name}: unit leader {reference:#x} is not recorded: UnitLeaders {gap}"
                        )
                    })?;
                Object::Character(row.character_id)
            }
            EventObject::Province(id) => {
                return Err(format!(
                    "{name}: province {id} is a text object, not an event-scope target"
                ));
            }
            EventObject::IndustrialOrganisation(_)
            | EventObject::Project { .. }
            | EventObject::Raid(_)
            | EventObject::Unit(_)
            | EventObject::Combat => Object::State(None), // No country, character or state.
        })
    }

    /// The local player's country when signed-positive, else the observer
    /// country, through CountryLinks; refused by name where the hour
    /// does not say.
    fn local_player(&self) -> Result<u32, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::LocalPlayer),
            || self.local_player_unrecorded(),
        )
    }

    pub(crate) fn local_player_unrecorded(&self) -> Result<u32, String> {
        let (players, gap) = rows::<tables::LocalPlayer>(self.frame);
        if !complete::<tables::LocalPlayer>(self.frame) {
            return Err(format!(
                "the local player's country is not recorded this hour: LocalPlayer {gap}"
            ));
        }
        let [player] = players else {
            return Err(format!(
                "LocalPlayer holds {} rows where the game state holds one local player",
                players.len()
            ));
        };
        let raw = if (player.country as i32) > 0 {
            player.country
        } else {
            player.observer_country
        };
        // Country-reference resolution maps zero to slot zero.
        if raw == 0 {
            return Ok(0);
        }
        self.scopes
            .slot(raw)
            .ok_or_else(|| format!("the local player's country link {raw} has no CountryLinks row"))
    }

    /// A character of the current country, else of the active event scope's
    /// country when it has one, whose script
    /// token is `segment`; `None` only where the frame proves there is none.
    /// A state, character or unit leader scope holds no country: default
    /// construction leaves it zero and setting those objects clears it, so
    /// the search is skipped. The active ability's nonclearing leader setter keeps
    /// the country, including after a character text promotion. For a
    /// province scope the country is not known, so only a
    /// token no character has is proven no character.
    fn character_named(
        &self,
        object: Object,
        segment: &str,
        event: EventObject,
    ) -> Result<Option<u32>, String> {
        crate::triggers::inputs::query(
            || {
                crate::triggers::inputs::Key::Text(TextQuery::CharacterNamed(
                    object,
                    segment.to_owned(),
                    event,
                ))
            },
            || self.character_named_unrecorded(object, segment, event),
        )
    }

    pub(crate) fn character_named_unrecorded(
        &self,
        object: Object,
        segment: &str,
        event: EventObject,
    ) -> Result<Option<u32>, String> {
        let country = match (object, event) {
            (Object::Country(slot), _)
            | (
                _,
                EventObject::Country(slot)
                | EventObject::CountryState { country: slot, .. }
                | EventObject::CountryUnitLeader { country: slot, .. },
            ) => Some(slot),
            (
                _,
                EventObject::State(_)
                | EventObject::Character(_)
                | EventObject::UnitLeader(_)
                | EventObject::IndustrialOrganisation(_)
                | EventObject::Project { .. }
                | EventObject::Raid(_)
                | EventObject::Unit(_)
                | EventObject::Combat,
            ) => {
                return Ok(None);
            }
            (_, EventObject::Province(_)) => None,
        };
        let (characters, gap) = rows::<tables::Characters>(self.frame);
        let (lexical, lexical_gap) = rows::<tables::LexicalTokens>(self.frame);
        let links_whole = complete::<tables::CountryLinks>(self.frame);
        // With every link recorded, a character is the country's exactly when
        // its link is one of the country's links: other rows are skipped
        // without resolving them.
        let country_links: Option<Vec<u32>> = country.filter(|_| links_whole).map(|slot| {
            rows::<tables::CountryLinks>(self.frame)
                .0
                .iter()
                .filter(|link| link.canonical_slot == slot)
                .map(|link| link.id)
                .collect()
        });
        let mut unknown = None;
        for row in characters {
            if let Some(links) = &country_links {
                if !links.contains(&row.country) {
                    continue;
                }
            } else {
                match self.scopes.slot(row.country) {
                    _ if country.is_none() => {}
                    Some(slot) if Some(slot) == country => {}
                    None if !links_whole => {
                        let (_, links) = rows::<tables::CountryLinks>(self.frame);
                        unknown.get_or_insert_with(|| {
                            format!(
                                "character {}'s country link {} resolves to no country: CountryLinks {links}",
                                row.id, row.country
                            )
                        });
                        continue;
                    }
                    _ => continue,
                }
            }
            // A negative token id is no token.
            let Ok(token) = u32::try_from(row.name_token_id) else {
                continue;
            };
            match lexical.binary_search_by_key(&token, |entry| entry.token) {
                Ok(index) if lexical[index].literal == segment.as_bytes() => {
                    if country.is_none() {
                        return Err(format!(
                            "{segment} names character {}, and the event scope's country, which character lookup searches, is not known here",
                            row.id
                        ));
                    }
                    return Ok(Some(row.id));
                }
                Ok(_) => {}
                Err(_) => {
                    unknown.get_or_insert_with(|| {
                        format!(
                            "character {}'s script token {token} is not spelled: LexicalTokens {lexical_gap}",
                            row.id
                        )
                    });
                }
            }
        }
        let unknown = if complete::<tables::Characters>(self.frame) {
            unknown
        } else {
            Some(format!("Characters {gap}"))
        };
        match unknown {
            Some(reason) => Err(match country {
                Some(country) => format!(
                    "{segment}: whether it names a character of country slot {country} is unknown: {reason}"
                ),
                None => format!("{segment}: whether it names a character is unknown: {reason}"),
            }),
            None => Ok(None),
        }
    }

    /// The ruling party's name or its long form.
    fn ruling_party(&self, slot: u32, tag: &str, long: bool) -> Result<String, String> {
        crate::triggers::inputs::query(
            || {
                crate::triggers::inputs::Key::Text(TextQuery::RulingParty(
                    slot,
                    tag.to_owned(),
                    long,
                ))
            },
            || self.ruling_party_unrecorded(slot, tag, long),
        )
    }

    pub(crate) fn ruling_party_unrecorded(
        &self,
        slot: u32,
        tag: &str,
        long: bool,
    ) -> Result<String, String> {
        let (politics, gap) = rows::<tables::Politics>(self.frame);
        let politics = politics
            .iter()
            .find(|row| self.scopes.slot(row.country) == Some(slot))
            .ok_or_else(|| format!("the ruling party of {tag} is unknown: Politics {gap}"))?;
        let Some(party) = politics.ruling_party else {
            return Err(format!("{tag} has no ruling party"));
        };
        let (parties, gap) = rows::<tables::PoliticalParties>(self.frame);
        let party = parties
            .iter()
            .find(|row| row.country == politics.country && row.id == party)
            .ok_or_else(|| {
                format!("the ruling party of {tag} has no PoliticalParties row: {gap}")
            })?;
        self.party_name(party, tag, long)
    }

    pub(crate) fn party_name(
        &self,
        party: &replay_data::rows::PoliticalPartyRow,
        tag: &str,
        long: bool,
    ) -> Result<String, String> {
        if party.name_default == 0 {
            let key = if long { &party.long_name } else { &party.name };
            return self
                .localisation
                .localise(key)
                .map_err(|refusal| refusal.to_string());
        }
        // Party naming uses the party's tag reference, not its roster owner.
        let slot = if party.tag == 0 {
            0
        } else {
            self.scopes.slot(party.tag).ok_or_else(|| {
                format!(
                    "the ruling party of {tag}: tag link {} has no CountryLinks row",
                    party.tag
                )
            })?
        };
        let naming = crate::party_names::PartyNaming {
            token: String::from_utf8_lossy(&party.token).into_owned(),
            tag: Some(self.country_tag(slot)?.to_owned()),
            original_tag: None,
            key: None,
        };
        let defines = |key: &str| self.localisation.get(key).is_some();
        let key = match crate::party_names::party_key(&naming, defines) {
            Some(key) => key,
            None => {
                // Read the signed original-tag reference only after the own
                // party key misses, without the name generator's dynamic gate.
                crate::party_names::party_key_with_selected_tag(
                    &naming,
                    installed_content_country_names::country_names::original_or_own_tag(self.frame, slot)?,
                    defines,
                )
                .ok_or_else(|| {
                    format!("the ruling party of {tag}: its tag or original tag is not recorded")
                })?
            }
        };
        // Probe `_long` only on a `_party` key found, not on
        // the group token it falls back to.
        let long_key = format!("{key}_long");
        let key = if long && key != naming.token && defines(&long_key) {
            long_key
        } else {
            key
        };
        self.localise(&key)
    }

    /// The faction's name, else `NO_FACTION` through the localiser,
    /// which copies the key as its bytes when the content lacks it.
    fn faction_name(&self, slot: u32) -> Result<String, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::FactionName(slot)),
            || self.faction_name_unrecorded(slot),
        )
    }

    pub(crate) fn faction_name_unrecorded(&self, slot: u32) -> Result<String, String> {
        let Some(faction) = self.scopes.faction(slot).map_err(|why| why.to_string())? else {
            return self.localise(NO_FACTION);
        };
        self.faction_name_by_id(faction)
    }

    /// Effect descriptions name the same faction without a country lookup.
    pub(crate) fn faction_name_by_id(&self, id: u64) -> Result<String, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::FactionNameById(id)),
            || self.faction_name_by_id_unrecorded(id),
        )
    }

    pub(crate) fn faction_name_by_id_unrecorded(&self, id: u64) -> Result<String, String> {
        let (factions, gap) = rows::<tables::Factions>(self.frame);
        let faction = factions
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| format!("faction {id} has no Factions row: {gap}"))?;
        // Only a nonnull fallback template needs content or a lexer dictionary.
        let (spelling, templates) = if faction.name.is_empty() && faction.template_token.is_some() {
            crate::catalogue::used(crate::catalogue::Reader::FactionTemplates);
            (
                crate::faction_names::template_spelling(self.frame, faction.template_token),
                self.content.layers.layers.faction_templates(),
            )
        } else {
            let unused = "Faction templates were not needed";
            (Ok(None), Err(unused.to_owned()))
        };
        crate::faction_names::name(
            &faction.name,
            spelling
                .as_ref()
                .map(|value| value.as_deref())
                .map_err(Clone::clone),
            templates.as_deref().map_err(Clone::clone),
            self.localisation,
            &self.content.recorded_language,
            |name| self.lexical_names.token_of(name),
        )
    }
}

/// The key GetFactionName localises without a faction.
const NO_FACTION: &str = "NO_FACTION";

/// The ten pronoun properties shared by countries and characters.
const PRONOUNS: [&str; 10] = [
    "GetSheHe",
    "GetSheHeCap",
    "GetHerHim",
    "GetHerHimCap",
    "GetHerHis",
    "GetHerHisCap",
    "GetHersHis",
    "GetHersHisCap",
    "GetHerselfHimself",
    "GetHerselfHimselfCap",
];

/// GetOld*: the property, the suffix of the current
/// name it falls back to, and whether the first byte is uppercased.
const OLD_NAMES: [(&str, &str, bool); 5] = [
    ("GetOldName", "", false),
    ("GetOldNameDef", "_DEF", false),
    ("GetOldNameDefCap", "_DEF", true),
    ("GetOldAdjective", "_ADJ", false),
    ("GetOldAdjectiveCap", "_ADJ", true),
];

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_wiring;

#[cfg(test)]
#[path = "scripted_text/effect_scopes/tests.rs"]
mod tests_effect_scopes;

#[cfg(test)]
mod organisation_tests;

#[cfg(test)]
mod dependence_tests;
