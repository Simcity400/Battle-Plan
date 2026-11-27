use content_reader_politics::faction_templates::FactionTemplateProducts as _;

use job_priority::SpawnCarried as _;
use replay_frame::frame::DecodedFrame;

use crate::scope::{RecordedScopes, table_in as rows};
use content_reader_country_names::country_names::CountryNames;
use content_reader_localisation::{localisation::Localisation, localisation::crt_case_text};
use replay_data::rows::{
    CharacterRow, CountryAppearanceRow, CountryLinkRow, CountryRow, FactionRow,
    IndustrialOrganisationRow, IntelligenceAgencyRow, LexicalTokenRow, LocalPlayerRow,
    PartyLeaderRow, PoliticalPartyRow, PoliticsRow, ProvinceNameRow, ProvinceRow, RowCodec,
    StateResistanceRow, StateRow, TableSchema, column, mask, tables,
};
use replay_frame::frame::{HourFrame, OperandStatus, Rowset};
use std::borrow::Cow;
#[cfg(any(test, feature = "test-support"))]
use replay_frame::frame::TableView;

const ORGANISATION: u64 = mask::<IndustrialOrganisationRow>(&["owner", "token", "name"]);
pub(super) const CHARACTER: u64 =
    mask::<CharacterRow>(&["country", "name_token_id", "name", "token", "gender"]);
pub(super) const POLITICS: u64 =
    mask::<PoliticsRow>(&["capital", "original_capital", "ruling_party", "last_election"]);
pub(super) const APPEARANCE: u64 = mask::<CountryAppearanceRow>(&[
    "overlord",
    "old_name",
    "old_adjective",
    "old_definite_name",
]);
pub(super) const STATE: u64 = mask::<StateRow>(&[
    "owner",
    "controller",
    "custom_name",
    "owner_name",
    "continent",
    "best_vp_index",
]);
pub(super) const LINK: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);
const PLAYER: u64 = mask::<LocalPlayerRow>(&["country", "observer_country"]);
pub(super) const PARTY: u64 = mask::<PoliticalPartyRow>(&[
    "token",
    "tag",
    "name_default",
    "name",
    "long_name",
]);
const FACTION: u64 = mask::<FactionRow>(&["name", "template_token"]);
const UNIT_LEADER: u64 = mask::<replay_data::rows::UnitLeaderRow>(&["character_id"]);
pub(super) const PARTY_LEADER: u64 = mask::<PartyLeaderRow>(&["character"]);
pub(super) const COUNTRY: u64 = mask::<CountryRow>(&["canonical_slot", "manpower_residual"]);
pub(super) const AGENCY: u64 = mask::<IntelligenceAgencyRow>(&["name", "created"]);
pub(super) const PROVINCE: u64 = mask::<ProvinceRow>(&["state", "owner", "controller"]);
pub(super) const PROVINCE_NAME: u64 = mask::<ProvinceNameRow>(&["name"]);
pub(super) const RESISTANCE: u64 = mask::<StateResistanceRow>(&["country"]);
const SEED: u64 = mask::<replay_data::rows::ClockRow>(&["seed"]);
pub(super) const LITERAL: usize = column::<LexicalTokenRow>("literal");
pub(super) const PROVINCE_NAME_COLUMN: usize = column::<ProvinceNameRow>("name");

pub(super) fn built(why: std::io::Error) -> String {
    why.to_string()
}

pub(super) fn keyed<'r, R: RowCodec + Clone>(
    rows: &Rowset<'r, R>,
    key: u128,
) -> Result<Option<Cow<'r, R>>, String> {
    rows.get(key).map_err(built)
}

pub(super) fn first<'r, R: RowCodec + Clone + 'r>(
    rows: &Rowset<'r, R>,
    keep: impl FnMut(u128) -> bool,
) -> Result<Option<Cow<'r, R>>, String> {
    rows.keyed(keep).next().transpose().map_err(built)
}

pub(super) fn each<'r, R: RowCodec + Clone + 'r>(
    rows: Rowset<'r, R>,
) -> impl DoubleEndedIterator<Item = Result<Cow<'r, R>, String>> + 'r {
    rows.iter().map(|row| row.map_err(built))
}

pub fn spelled<'r>(
    lexical: &Rowset<'r, LexicalTokenRow>,
    token: u32,
) -> Result<Option<&'r [u8]>, String> {
    lexical
        .text(u128::from(token), |row| &row.literal, LITERAL)
        .map_err(built)
}

pub const TABLES: &[replay_data::rows::TableId] = crate::triggers::RECORDED_TABLES;

mod balance;
pub use balance::{BalanceDrift, ChangeRate, balance_drift};
#[cfg(any(test, feature = "test-support"))]
pub use balance::tests as balance_test_support;
mod dependence;
mod entries;
mod numeric;
pub mod operands;
pub use entries::{
    Environment, StateContent, StateNameUsage, StateNames, live_state_warnings,
    plain_state_name_of, province_primary_name, read_state_content, state_display_name,
    victory_point_name,
};
mod properties;
pub mod random;
pub mod recorded_names;
mod state_properties;

const MAX_SCRIPTED_NESTING: usize = 10_000;

fn conditional(inner: &str) -> Option<String> {
    (inner.len() >= 2 && inner.starts_with('(') && inner.ends_with(')'))
        .then(|| format!("the conditional scripted text [{inner}] is not evaluated"))
}

fn nesting_refusal(name: &str) -> String {
    format!(
        "scripted localisation nesting the game could never finish (native stack exhaustion): {name}, beyond {MAX_SCRIPTED_NESTING} levels"
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Object {
    FactionEnvironment(Option<u64>),
    Faction(Option<u64>),
    Event,
    Country(u32),
    State(Option<u32>),
    Character(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    Itself,
    Scope(usize),
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum EventObject {
    Country(u32),
    Combat,
    CountryState {
        country: u32,
        state: u32,
    },
    State(u32),
    Province(u32),
    UnitLeader(u64),
    CountryUnitLeader {
        country: u32,
        leader: u64,
    },
    Character(u32),
    IndustrialOrganisation(u64),
    Project {
        owner: u32,
        id: u32,
    },
    Raid(u64),
    Unit(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventScope {
    pub this: EventObject,
    pub root: Link,
    pub from: Link,
    pub prev: Link,
    pub random_seed: u32,
    pub random_counter: u32,
    pub random_seed_refusal: Option<&'static str>,
    pub clock_seed: Option<Option<u32>>,
}

impl EventScope {
    pub fn of(this: EventObject) -> Self {
        Self {
            this,
            root: Link::Itself,
            from: Link::Itself,
            prev: Link::Itself,
            random_seed: 1,
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

    pub fn random_draw(&self) -> Result<u32, String> {
        Ok(random::scoped_draw(self.require_random_seed()?, self.random_counter) as u32)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventScopes {
    pub scopes: Vec<EventScope>,
    pub variables: ScopeVariables,
    pub tooltip_targets: std::collections::BTreeMap<usize, Vec<(Vec<u8>, EventObject)>>,
    pub saved_targets: std::collections::BTreeMap<usize, Vec<(Vec<u8>, EventObject)>>,
    pub global_tooltip_targets: Vec<(Vec<u8>, EventObject)>,
}

pub const SCRIPTED_GAP: char = '\u{FFFC}';

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptedGap {
    pub key: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpandedText {
    pub text: String,
    pub gaps: Vec<ScriptedGap>,
}

fn expand_brackets(
    text: &str,
    out: &mut String,
    gaps: &mut Vec<ScriptedGap>,
    mut resolve: impl FnMut(&str, &mut String, &mut Vec<ScriptedGap>) -> Result<(), String>,
) {
    let mut rest = text;
    loop {
        let Some(open) = rest.find('[') else {
            out.push_str(rest);
            return;
        };
        let Some(length) = rest[open + 1..].find(']') else {
            out.push_str(rest);
            return;
        };
        let inner = &rest[open + 1..open + 1 + length];
        out.push_str(&rest[..open]);
        if let Err(reason) = resolve(inner, out, gaps) {
            out.push(SCRIPTED_GAP);
            gaps.push(ScriptedGap {
                key: inner.to_owned(),
                reason,
            });
        }
        rest = &rest[open + 1 + length + 1..];
    }
}

const MAX_DATE_EXPANSION_DEPTH: usize = 4;

fn expand_dates_at(
    text: &str,
    hour: i32,
    localisation: &Localisation,
    unavailable: &str,
    depth: usize,
    out: &mut String,
    gaps: &mut Vec<ScriptedGap>,
) {
    expand_brackets(text, out, gaps, |inner, out, gaps| {
        if depth >= MAX_DATE_EXPANSION_DEPTH {
            return Err("date expansion exceeded maximum nesting depth".to_owned());
        }
        if DATES.iter().any(|date| named(inner, date)) {
            let value = properties::date_property_at(hour, inner, |key| {
                localisation
                    .localise(key.as_bytes())
                    .map_err(|refusal| refusal.to_string())
            })?
            .ok_or_else(|| unavailable.to_owned())?;
            expand_dates_at(&value, hour, localisation, unavailable, depth + 1, out, gaps);
            return Ok(());
        }
        Err(unavailable.to_owned())
    });
}

pub fn expand_dates_in(
    text: &str,
    hour: i32,
    localisation: &Localisation,
    unavailable: &str,
) -> ExpandedText {
    let mut out = String::new();
    let mut gaps = Vec::new();
    expand_dates_at(text, hour, localisation, unavailable, 0, &mut out, &mut gaps);
    ExpandedText { text: out, gaps }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopeName {
    Hour,
    Recorded([Vec<u8>; 3]),
    Unproven(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScopeVariables {
    pub target_state: Option<u32>,
    pub target_province: Option<u32>,
    pub target_leader: Option<u64>,
}

impl EventScopes {
    pub fn new(scopes: Vec<EventScope>) -> Self {
        Self {
            scopes,
            variables: ScopeVariables::default(),
            tooltip_targets: Default::default(),
            saved_targets: Default::default(),
            global_tooltip_targets: Vec::new(),
        }
    }

    pub fn empty() -> Self {
        Self::new(vec![EventScope::of(EventObject::State(
            crate::triggers::NULL_STATE,
        ))])
    }

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

#[derive(Clone)]
pub struct ExpansionScopes<'e, 'a> {
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
    recording: Option<std::sync::Arc<std::sync::Mutex<operands::Recording>>>,
}

impl<'e, 'a> ExpansionScopes<'e, 'a> {
    pub fn world(&self) -> Option<&'e crate::triggers::World<'a>> {
        self.world
    }

    pub fn combat(&self) -> Option<&'e dyn crate::triggers::Combat> {
        self.combat
    }

    pub fn variable_session(&self) -> Option<crate::triggers::VariableSession> {
        self.session.borrow().clone()
    }

    pub fn replace_variable_session(&self, session: Option<crate::triggers::VariableSession>) {
        self.session.replace(session);
    }

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
            recording: None,
        }
    }

    fn scope(&self, at: usize) -> EventScope {
        self.scopes.borrow()[at]
    }

    fn append_description<'s>(
        &'s self,
        scopes: &EventScopes,
    ) -> Result<DescriptionScopeGuard<'s, 'e, 'a>, String> {
        for (kind, targets) in [
            ("tooltip target", &scopes.tooltip_targets),
            ("saved target", &scopes.saved_targets),
        ] {
            if let Some((&at, _)) = targets.last_key_value()
                && at >= scopes.scopes.len()
            {
                return Err(format!("scripted text {kind} event scope {at} is not given"));
            }
        }
        let offset = self.scopes.borrow().len();
        let rebase = |link| match link {
            Link::Scope(at) => at
                .checked_add(offset)
                .map(Link::Scope)
                .ok_or_else(|| format!("scripted text event scope link {at} overflows when rebased")),
            other => Ok(other),
        };
        let appended = scopes
            .scopes
            .iter()
            .map(|scope| {
                Ok(EventScope {
                    root: rebase(scope.root)?,
                    from: rebase(scope.from)?,
                    prev: rebase(scope.prev)?,
                    ..*scope
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let next = Self::new(scopes);
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
        Ok(DescriptionScopeGuard {
            expanded: self,
            offset,
            variables,
            valid_links,
            globals,
        })
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
            None => text.trigger_world()?,
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

pub trait EffectDescriber: std::any::Any + Send + Sync {
    fn describe<'f>(
        &self,
        text: &ScriptedText<'f>,
        source: &str,
        items: &[content_reader_parser::parser::Item],
        expansion: &ExpansionScopes<'_, 'f>,
        graph: &mut EventScopes,
        refused: &mut Vec<String>,
    ) -> Result<String, String>;

    fn prepare_static_modifiers(&self, content: &ScriptedContent) -> Result<(), String>;
}

pub type EffectDescriberFactory = fn() -> Box<dyn EffectDescriber>;

pub struct ScriptedContent {
    pub names: content_reader_scripted_localisation::scripted_localisation::Names,
    pub recursion: i32,
    pub tag_aliases: std::sync::Arc<content_reader_key_names::tag_aliases::TagAliases>,
    pub recorded_language: String,
    pub layers: std::sync::Arc<content_reader_layers::content::ResolvedContent>,
    pub(crate) environment:
        content_reader_layers::content::diagnostics::Once<Result<entries::Environment, String>>,
    pub(crate) state_content: std::sync::Arc<
        content_reader_layers::content::diagnostics::Once<Result<entries::StateContent, String>>,
    >,
    pub(crate) template_names:
        content_reader_layers::content::diagnostics::Once<Result<Localisation, String>>,
    name_indexes: crate::triggers::world::NameIndexes,
    event_index: content_reader_layers::content::diagnostics::Once<content_reader_events::events::LazyEvents>,
    effect_describer: Option<Box<dyn EffectDescriber>>,
    static_modifiers: std::sync::Arc<content_reader_politics::static_modifiers::Descriptions>,
}

impl ScriptedContent {
    pub fn read(
        content: std::sync::Arc<content_reader_layers::content::ResolvedContent>,
        recorded_language: String,
    ) -> Result<Self, String> {
        let layers = &content.layers;
        let names = content_reader_scripted_localisation::scripted_localisation::names(layers)
            .map_err(|error| format!("common/scripted_localisation is unreadable: {error}"))?;
        let environment = content_reader_layers::content::diagnostics::Once::new();
        let recursion = environment
            .get_or_init(|| {
                Environment::read(layers).map_err(|error| {
                    format!("NGame.MAX_SCRIPTED_LOC_RECURSION is unavailable: {error}")
                })
            })
            .as_ref()
            .map_err(Clone::clone)?
            .defines()
            .int32("NGame", "MAX_SCRIPTED_LOC_RECURSION")
            .map_err(|error| format!("NGame.MAX_SCRIPTED_LOC_RECURSION is unavailable: {error}"))?;
        let tag_aliases = content_reader_key_names::tag_aliases::read(layers)
            .map_err(|error| format!("common/country_tag_aliases is unreadable: {error}"))?;
        Ok(Self {
            names,
            recursion,
            tag_aliases: std::sync::Arc::new(tag_aliases),
            recorded_language,
            layers: content,
            environment,
            state_content: std::sync::Arc::new(content_reader_layers::content::diagnostics::Once::new()),
            template_names: content_reader_layers::content::diagnostics::Once::new(),
            name_indexes: Default::default(),
            event_index: content_reader_layers::content::diagnostics::Once::new(),
            effect_describer: None,
            static_modifiers: Default::default(),
        })
    }

    pub fn with_effect_describer(mut self, factory: EffectDescriberFactory) -> Self {
        debug_assert!(self.effect_describer.is_none(), "effect describer is already installed");
        self.effect_describer.get_or_insert_with(factory);
        self
    }

    pub fn effect_describer<T: EffectDescriber>(&self) -> Option<&T> {
        let describer = self.effect_describer.as_deref()?;
        (describer as &dyn std::any::Any).downcast_ref()
    }

    pub fn static_modifiers(
        &self,
    ) -> &std::sync::Arc<content_reader_politics::static_modifiers::Descriptions> {
        &self.static_modifiers
    }

    pub fn event_index(&self) -> &content_reader_events::events::LazyEvents {
        self.event_index.get_or_init(|| {
            content_reader_events::events::LazyEvents::from_content(std::sync::Arc::clone(&self.layers))
        })
    }

    pub fn environment(&self) -> Result<&Environment, String> {
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
    pub fn set_synthetic_environment_error(&self, reason: String) {
        assert!(self.environment().unwrap().inputs.set(Err(reason)).is_ok());
    }

    pub fn set_synthetic_environment(&self, state_names: std::collections::BTreeMap<u32, String>) {
        let mut environment = entries::EnvironmentInputs::synthetic().with_constants(
            content_reader_politics::script_constants::read(&self.layers.layers)
                .map_err(|why| why.to_string()),
        );
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

pub fn stored_text(
    localisation: &Localisation,
    recorded: &str,
    bytes: &[u8],
    what: &str,
) -> Result<String, String> {
    if !bytes.is_empty() && localisation.language.is_empty() {
        return Err(format!(
            "{what} is text the game stored in {recorded} without its key, and the game language \
             it would be shown in is unknown"
        ));
    }
    if !bytes.is_empty() && recorded != localisation.language {
        return Err(format!(
            "{what} is text the game stored in {recorded} without its key, and the viewer's language is {}",
            localisation.language
        ));
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| format!("{what} is not UTF-8"))
}

pub struct ScriptedText<'a> {
    localisation: &'a Localisation,
    content: &'a ScriptedContent,
    scopes: std::sync::Arc<RecordedScopes<'a>>,
    names: std::sync::Arc<std::sync::OnceLock<recorded_names::RecordedNames<'a>>>,
    lexical_names: std::sync::Arc<std::sync::OnceLock<crate::faction_names::LexicalNames<'a>>>,
    frame: HourFrame<'a>,
    reserved_tag: std::sync::OnceLock<Result<String, String>>,
    root: u32,
    viewer: Option<u32>,
    pub(crate) nato_counters: bool,
    world: std::sync::OnceLock<Result<crate::triggers::World<'a>, String>>,
    accepted_projects: Option<crate::triggers::ProjectReads<'a>>,
}

fn named(segment: &str, name: &str) -> bool {
    segment.eq_ignore_ascii_case(name)
}

fn complete<T: TableSchema>(frame: HourFrame<'_>) -> bool {
    matches!(
        frame.status(T::ID),
        OperandStatus::Complete | OperandStatus::WrongLayout
    )
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
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
    PartyLeader(u32, Option<String>),
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
                crate::triggers::inputs::fingerprint(&self.capital_unrecorded(*slot, *original))
            }
            TextQuery::Overlord(slot) => {
                crate::triggers::inputs::digest(&self.overlord_unrecorded(*slot))
            }
            TextQuery::StateRecord(id) => {
                crate::triggers::inputs::digest(&self.state_record_unrecorded(*id))
            }
            TextQuery::StateCountry(id, owner) => {
                crate::triggers::inputs::fingerprint(&self.state_country_unrecorded(*id, *owner))
            }
            TextQuery::CharacterCountry(id) => {
                crate::triggers::inputs::fingerprint(&self.character_country_unrecorded(*id))
            }
            TextQuery::EventObject(object, name) => {
                crate::triggers::inputs::fingerprint(&self.event_object_unrecorded(*object, name))
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
    pub(crate) fn countries_gap(&self) -> Option<String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::CountriesGap),
            || self.countries_gap_unrecorded(),
        )
    }

    pub(crate) fn countries_gap_unrecorded(&self) -> Option<String> {
        (!complete::<tables::Countries>(self.frame))
            .then(|| rows::<tables::Countries>(self.frame, 0).1)
    }
}

pub fn clock_seed(frame: &dyn replay_frame::frame::RecordedSource) -> Option<u32> {
    frame.clock_seed().ok().flatten().map(|seed| seed as u32)
}

pub fn clock_seed_in(frame: HourFrame<'_>) -> Option<u32> {
    match frame.decoded() {
        Some(decoded) => clock_seed(decoded),
        None => frame
            .complete::<tables::Clock>(SEED)
            .ok()?
            .iter()
            .next()?
            .ok()
            .map(|row| row.seed as u32),
    }
}

impl<'a> ScriptedText<'a> {
    pub fn new(
        frame: &'a dyn replay_frame::frame::DecodedSource,
        localisation: &'a Localisation,
        content: &'a ScriptedContent,
        root: u32,
    ) -> Self {
        Self::new_in(HourFrame::Decoded(frame), localisation, content, root)
    }

    pub fn new_in(
        frame: HourFrame<'a>,
        localisation: &'a Localisation,
        content: &'a ScriptedContent,
        root: u32,
    ) -> Self {
        Self::with_scopes_in(
            frame,
            localisation,
            content,
            root,
            RecordedScopes::read_in(frame, std::sync::Arc::clone(&content.tag_aliases)),
        )
    }

    pub fn with_scopes_in(
        frame: HourFrame<'a>,
        localisation: &'a Localisation,
        content: &'a ScriptedContent,
        root: u32,
        scopes: RecordedScopes<'a>,
    ) -> Self {
        let text = Self {
            localisation,
            content,
            scopes: std::sync::Arc::new(scopes),
            names: std::sync::Arc::default(),
            lexical_names: std::sync::Arc::default(),
            frame,
            reserved_tag: std::sync::OnceLock::new(),
            root,
            viewer: None,
            nato_counters: content_reader_settings::settings::MapOptions::DEFAULT.nato_counters,
            world: std::sync::OnceLock::new(),
            accepted_projects: None,
        };
        if crate::triggers::inputs::recording() {
            text.names
                .get_or_init(|| recorded_names::RecordedNames(CountryNames::read(&frame, localisation)));
            text.lexical_names
                .get_or_init(|| crate::faction_names::LexicalNames::from_tables(frame));
        }
        text
    }

    fn names(&self) -> &recorded_names::RecordedNames<'a> {
        self.names.get_or_init(|| {
            crate::triggers::inputs::unrecorded(|| {
                recorded_names::RecordedNames(CountryNames::read(&self.frame, self.localisation))
            })
        })
    }

    fn lexical_names(&self) -> &crate::faction_names::LexicalNames<'a> {
        self.lexical_names.get_or_init(|| {
            crate::triggers::inputs::unrecorded(|| {
                crate::faction_names::LexicalNames::from_tables(self.frame)
            })
        })
    }

    pub fn for_root(&self, root: u32) -> Self {
        Self {
            localisation: self.localisation,
            content: self.content,
            scopes: std::sync::Arc::clone(&self.scopes),
            names: std::sync::Arc::clone(&self.names),
            lexical_names: std::sync::Arc::clone(&self.lexical_names),
            frame: self.frame,
            reserved_tag: std::sync::OnceLock::new(),
            root,
            viewer: self.viewer,
            nato_counters: self.nato_counters,
            world: std::sync::OnceLock::new(),
            accepted_projects: self.accepted_projects,
        }
    }

    pub fn with_accepted_projects(mut self, accepted: crate::triggers::ProjectReads<'a>) -> Self {
        self.accepted_projects = Some(accepted);
        self
    }


    pub fn with_nato_counters(mut self, nato: bool) -> Self {
        self.nato_counters = nato;
        self
    }

    pub fn viewing(mut self, viewer: u32) -> Self {
        self.viewer = Some(viewer);
        self
    }

    pub fn expand(&self, text: &str) -> Result<String, String> {
        let scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(self.root))]);
        self.expand_in(text, &scopes)
    }

    pub fn expand_empty(&self, text: &str) -> Result<String, String> {
        self.expand_in(text, &EventScopes::empty())
    }

    pub fn expand_faction(&self, text: &str, faction: u64) -> Result<String, String> {
        let scopes = EventScopes::new(vec![EventScope::of(EventObject::Country(self.root))]);
        self.expand_in_description(text, &scopes, 0, None, None, Some(Some(faction)))
            .0
    }

    pub fn organisation_template(&self, reference: u64) -> Result<(String, EventScopes), String> {
        let organisations = self
            .frame
            .complete::<tables::IndustrialOrganisations>(ORGANISATION)
            .map_err(|why| why.to_string())?;
        let row = keyed(&organisations, u128::from(reference))?
            .ok_or_else(|| format!("industrial organisations lacks organisation {reference:#x}"))?;
        let name = if row.name.is_empty() {
            let tokens = self
                .frame
                .complete::<tables::LexicalTokens>(0)
                .map_err(|why| why.to_string())?;
            spelled(&tokens, row.token)?.ok_or("lexical tokens lacks industrial organisation name")?
        } else {
            row.name.as_slice()
        };
        let name =
            std::str::from_utf8(name).map_err(|_| "industrial organisation name is not UTF-8")?;
        let owner = self
            .scopes
            .slot(row.owner)
            .ok_or("country links lacks industrial organisation owner")?;
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

    pub fn expand_in(&self, text: &str, scopes: &EventScopes) -> Result<String, String> {
        if scopes.scopes.is_empty() {
            return Err("scripted text needs an event scope".into());
        }
        self.expand_in_at(text, scopes, 0)
    }

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

    pub fn expand_names_in(
        &self,
        text: &str,
        scopes: &EventScopes,
        names: &[ScopeName],
    ) -> ExpandedText {
        let mut out = String::new();
        let mut gaps = Vec::new();
        self.names_only(
            text,
            0,
            &ExpansionScopes::new(scopes),
            names,
            &mut out,
            &mut gaps,
        );
        ExpandedText { text: out, gaps }
    }

    fn names_only(
        &self,
        text: &str,
        depth: i32,
        scopes: &ExpansionScopes<'_, 'a>,
        names: &[ScopeName],
        out: &mut String,
        gaps: &mut Vec<ScriptedGap>,
    ) {
        if depth >= self.content.recursion {
            out.push_str(text);
            return;
        }
        expand_brackets(text, out, gaps, |inner, out, gaps| {
            let value = self.expand_name_bracket(inner, scopes, names)?;
            self.names_only(&value, depth + 1, scopes, names, out, gaps);
            Ok(())
        });
    }

    fn expand_name_bracket(
        &self,
        inner: &str,
        scopes: &ExpansionScopes<'_, 'a>,
        names: &[ScopeName],
    ) -> Result<String, String> {
        let refused = || "reads more than an event scope's country name or occurrence date".to_owned();
        let (links, property) = match inner.rsplit_once('.') {
            Some((links, property)) => (Some(links), property),
            None => (None, inner),
        };
        if links.is_none() && DATES.iter().any(|date| named(property, date)) {
            return self.date_property(property)?.ok_or_else(refused);
        }
        let Some(links) = links else {
            return Err(refused());
        };
        if !links
            .split('.')
            .all(|segment| SCOPE_LINKS.iter().any(|link| named(segment, link)))
            || !COUNTRY_NAMES.iter().any(|name| named(property, name))
        {
            return Err(refused());
        }
        if scopes.scopes.borrow().is_empty() {
            return Err("scripted text needs an event scope".to_owned());
        }
        let (mut object, mut active, mut event_active) = (Object::Event, 0, true);
        for segment in links.split('.') {
            object = self.promote(object, segment, scopes, &mut active, &mut event_active)?;
        }
        if let Some(ScopeName::Unproven(reason)) = names.get(active) {
            return Err(format!("event scope {active}: {reason}"));
        }
        let Object::Country(slot) = object else {
            return Err(refused());
        };
        match names.get(active) {
            Some(ScopeName::Recorded(recorded)) => {
                self.recorded_country_property(slot, property, recorded)
            }
            _ => self.country_property(slot, property, scopes),
        }?
        .ok_or_else(refused)
    }

    fn recorded_country_property(
        &self,
        slot: u32,
        name: &str,
        [full, definite, adjective]: &[Vec<u8>; 3],
    ) -> Result<Option<String>, String> {
        let is = |entry: &str| named(name, entry);
        let upper_first = |text: String| crt_case_text(&text, true, true);
        let text =
            |bytes: &[u8], what: &str| self.stored_text(bytes, &format!("the occurrence's {what}"));
        Ok(Some(if is("GetName") {
            text(full, "country name")?
        } else if is("GetNameDef") {
            text(definite, "definite country name")?
        } else if is("GetNameDefCap") {
            upper_first(text(definite, "definite country name")?)
        } else if is("GetAdjective") {
            text(adjective, "country adjective")?
        } else if is("GetAdjectiveCap") {
            upper_first(text(adjective, "country adjective")?)
        } else if is("GetTag") {
            self.country_tag(slot)?.to_owned()
        } else if is("GetFlag") {
            format!("\u{12}{}", self.country_tag(slot)?)
        } else if is("GetNameWithFlag") {
            let name = upper_first(text(definite, "definite country name")?);
            format!("\u{12}{} {name}", self.country_tag(slot)?)
        } else {
            return Ok(None);
        }))
    }

    pub fn expand_in_session(
        &self,
        text: &str,
        scopes: &EventScopes,
        at: usize,
        evaluator: &crate::triggers::Evaluator<'_, 'a>,
    ) -> Result<String, String> {
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

    pub fn expand_in_description(
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
        let appended = match expanded.append_description(scopes) {
            Ok(appended) => appended,
            Err(why) => return (Err(why), session),
        };
        expanded.session.replace(session);
        let object = faction_environment.map_or(Object::Event, Object::FactionEnvironment);
        let result = self.process(text, 0, expanded, at + appended.offset, object);
        let session = expanded.session.borrow().clone();
        (result, session)
    }

    pub fn recorded(&self) -> &RecordedScopes<'a> {
        &self.scopes
    }
    pub fn tables(&self) -> HourFrame<'a> {
        self.frame
    }

    pub fn localisation(&self) -> &'a Localisation {
        self.localisation
    }

    pub fn nato_counters(&self) -> bool {
        self.nato_counters
    }

    pub fn content(&self) -> &'a ScriptedContent {
        self.content
    }

    pub fn reread(&self, key: &crate::triggers::inputs::Key) -> Option<u64> {
        use crate::triggers::inputs::Key;
        match key {
            Key::Table(_) | Key::Fields(..) | Key::FocusTag(_) => None,
            Key::ClockSeed => Some(crate::triggers::inputs::digest(&clock_seed_in(self.frame))),
            Key::World(query) => self.trigger_world().ok().map(|world| world.reread(query)),
            Key::Scopes(query) => Some(self.scopes.reread(query)),
            Key::Names(query) => Some(self.names().reread(query)),
            Key::Text(query) => Some(self.reread_text(query)),
        }
    }

    pub(crate) fn country_names(&self) -> &recorded_names::RecordedNames<'_> {
        self.names()
    }

    pub fn country_name_operands(
        &self,
        slot: u32,
    ) -> Option<&Result<content_reader_country_names::country_names::Operands, String>> {
        self.country_names().operands(slot)
    }

    pub fn viewing_country(&self) -> u32 {
        self.root
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_synthetic_world(&self, world: crate::triggers::World<'a>) {
        assert!(self.world.set(Ok(world)).is_ok());
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_synthetic_world_with_template_names(&self, world: crate::triggers::World<'a>) {
        self.set_synthetic_world(world.with_template_names(
            &self.content.template_names,
            &self.content.layers.layers,
            &self.content.recorded_language,
        ));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn synthetic_world_loaded(&self) -> bool {
        self.world.get().is_some()
    }

    pub fn country_name(&self, country: u32, definite: bool) -> Result<String, String> {
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
            self.names().name_with(country, "_DEF")
        } else {
            self.names().name(country)
        }
    }

    pub fn country_tag(&self, country: u32) -> Result<&str, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::CountryTag(country)),
            || self.country_tag_unrecorded(country),
        )
    }

    pub(crate) fn country_tag_unrecorded(&self, country: u32) -> Result<&str, String> {
        if country == 0 {
            self.reserved_tag
                .get_or_init(|| {
                    content_reader_country_names::country_names::reserved_zero_tag_in(&self.frame)
                        .map(Cow::into_owned)
                })
                .as_deref()
                .map_err(Clone::clone)
        } else {
            self.scopes
                .tag(country)
                .ok_or_else(|| format!("Countries lacks tag for country {country}"))
        }
    }

    pub fn country_marked_name(&self, country: u32) -> Result<String, String> {
        let tag = self.country_tag(country)?;
        let name = crt_case_text(&self.names().name_with(country, "_DEF")?, true, true);
        Ok(format!("\u{12}{tag} {name}"))
    }

    pub fn country_adjective(&self, country: u32) -> Result<String, String> {
        self.names().name_with(country, "_ADJ")
    }

    pub fn recorded_language(&self) -> &str {
        &self.content.recorded_language
    }

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
        let reads = crate::triggers::inputs::handoff();
        let discovery = replay_frame::frame::Discovery::current();
        let memory = engine_allocation::labels::current();
        let ((((result, session), noted), carried), requested) = std::thread::scope(|thread_scope| {
            let cpu = replay_frame::cpu::Context::capture();
            std::thread::Builder::new()
                .name("scripted-localisation".into())
                .stack_size(MAX_SCRIPTED_NESTING * 64 * 1024)
                .spawn_scoped_carried(thread_scope, move || {
                    let _memory = engine_allocation::labels::resume(memory);
                    let _cpu = cpu.enter("scripted localisation");
                    caller.enter();
                    discovery.run(|| reads.run(move || {
                        crate::catalogue::capture(move || {
                            let result = expand(&expanded);
                            (result, expanded.session.into_inner())
                        })
                    }))
                })
                .map_err(|error| {
                    format!("scripted localisation expansion thread could not start: {error}")
                })?
                .join()
                .map_err(|_| "scripted localisation expansion thread failed".to_owned())
        })?;
        crate::catalogue::credit(&noted);
        carried.credit();
        replay_frame::frame::Discovery::credit(requested);
        scopes.session.replace(session);
        result
    }

    fn expand_fresh_empty(
        &self,
        text: &str,
        name: &str,
        scopes: &ExpansionScopes<'_, 'a>,
    ) -> Result<String, String> {
        self.expand_fresh_object(
            text,
            EventObject::State(crate::triggers::NULL_STATE),
            name,
            scopes,
        )
    }

    fn expand_fresh_country(
        &self,
        text: &str,
        slot: u32,
        name: &str,
        scopes: &ExpansionScopes<'_, 'a>,
    ) -> Result<String, String> {
        self.expand_fresh_object(text, EventObject::Country(slot), name, scopes)
    }

    fn expand_fresh_object(
        &self,
        text: &str,
        object: EventObject,
        name: &str,
        scopes: &ExpansionScopes<'_, 'a>,
    ) -> Result<String, String> {
        if scopes.needs_large_stack() {
            return self.on_large_stack(scopes, |scopes| {
                self.expand_fresh_object(text, object, name, scopes)
            });
        }
        let depth = scopes.fresh_depth.get();
        if scopes.active.borrow().len() + depth >= MAX_SCRIPTED_NESTING {
            return Err(nesting_refusal(name));
        }
        let at = scopes.scopes.borrow().len();
        scopes.scopes.borrow_mut().push(EventScope::of(object));
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
            let Some(length) = rest[open + 1..].find(']') else {
                return Ok(out);
            };
            let inner = &rest[open + 1..open + 1 + length];
            out.push_str(&rest[..open]);
            if let Some(refusal) = conditional(inner) {
                return Err(refusal);
            }
            if let Some(variable) = inner.strip_prefix('?') {
                let value = self.numeric_value(variable, scopes, start);
                scopes.note(inner, || match &value {
                    Ok(value) => operands::Value::Number(*value),
                    Err(reason) => operands::Value::Refused(reason.clone()),
                });
                out.push_str(&numeric::format_variable(variable, value?));
                rest = &rest[open + 1 + length + 1..];
                continue;
            }
            let value = self.bracket(inner, scopes, start, context)?;
            out.push_str(&self.process(&value, depth + 1, scopes, start, context)?);
            rest = &rest[open + 1 + length + 1..];
        }
    }

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
                .map_err(|reason| scopes.refuse(inner, format!("[{inner}]: {reason}")))?;
        }
        let within = |reason: String| format!("[{inner}]: {reason}");
        let reread = scopes
            .recording()
            .then(|| self.rereads(object, property, scopes))
            .flatten();
        let (property_of, stateful) = scopes.muted(|| self.property_of(object, property, scopes));
        match property_of {
            Err(reason) => {
                let reason = within(reason);
                scopes.note(inner, || {
                    reread.unwrap_or_else(|| operands::Value::Refused(reason.clone()))
                });
                Err(reason)
            }
            Ok(Some(text)) => {
                scopes.note(inner, || {
                    reread.unwrap_or_else(|| {
                        operands::Value::of_property(object, property, &text, stateful)
                    })
                });
                Ok(text)
            }
            Ok(None) => {
                let outer = scopes.begin_entry();
                let value = self
                    .entry(object, property, scopes, active, event_active)
                    .map_err(within);
                let selected = scopes.end_entry(outer);
                scopes.note(inner, || match (selected, &value) {
                    (Some(selected), _) => operands::Value::Entry(Some(selected)),
                    (None, Ok(_)) => operands::Value::Entry(None),
                    (None, Err(reason)) => operands::Value::Refused(reason.clone()),
                });
                value
            }
        }
    }

    fn promote(
        &self,
        object: Object,
        segment: &str,
        scopes: &ExpansionScopes<'_, 'a>,
        active: &mut usize,
        event_active: &mut bool,
    ) -> Result<Object, String> {
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
                    let country = self.scopes.raw_slot(raw).map_err(|why| why.to_string())?;
                    return Ok(Object::Country(country));
                }
            }
            Object::Country(slot) => {
                if named(segment, "Capital") || named(segment, "OriginalCapital") {
                    return self.capital(slot, named(segment, "OriginalCapital"));
                }
                if named(segment, "Overlord") {
                    return Ok(self.overlord(slot)?.map_or(object, Object::Country));
                }
                if named(segment, "FactionLeader") {
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
        if named(segment, "Player") {
            return self.local_player().map(Object::Country);
        }
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
        if self.scopes.country(segment).is_some() {
            return self
                .scopes
                .resolve_tag(segment)
                .map(Object::Country)
                .map_err(|why| why.to_string());
        }
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
                "{segment}: whether it is a country tag is unknown: countries: {gap}"
            ));
        }
        if (*event_active || matches!(object, Object::Country(_)))
            && let Some(id) = self.character_named(object, segment, scopes.scope(*active).this)?
        {
            return Ok(Object::Character(id));
        }
        let digits = segment.strip_prefix(['-', '+']).unwrap_or(segment);
        if !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()) {
            let id = content_reader_parser::fixed5::decimal_i32(segment.as_bytes());
            if id <= 0 {
                return Ok(object);
            }
            let (found, complete, gap) = self.state_record(id as u32);
            if found {
                return Ok(Object::State(Some(id as u32)));
            }
            if complete {
                return Ok(Object::State(None));
            }
            return Err(format!("state {segment} is unknown: states: {gap}"));
        }
        if object == Object::Event {
            return self.saved_promotion(segment, scopes, *active);
        }
        if named(segment, "Country") && matches!(object, Object::Country(_)) {
            return Ok(object);
        }
        Ok(object)
    }

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
                        "saved event target {name}: state {id} is unknown: states: {gap}"
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

    fn property_of(
        &self,
        object: Object,
        name: &str,
        scopes: &ExpansionScopes<'_, 'a>,
    ) -> Result<Option<String>, String> {
        Ok(match object {
            Object::FactionEnvironment(_) => Some(String::new()),
            Object::Faction(faction) => Some(if named(name, "GetName") {
                faction.map_or_else(|| Ok("NULL".into()), |id| self.faction_name_by_id(id))?
            } else {
                String::new()
            }),
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
        })
    }

    fn localise(&self, key: &str) -> Result<String, String> {
        self.localisation
            .localise(key.as_bytes())
            .map_err(|refusal| refusal.to_string())
    }

    fn stored_text(&self, bytes: &[u8], what: &str) -> Result<String, String> {
        stored_text(
            self.localisation,
            &self.content.recorded_language,
            bytes,
            what,
        )
    }

    fn country_property(
        &self,
        slot: u32,
        name: &str,
        scopes: &ExpansionScopes<'_, 'a>,
    ) -> Result<Option<String>, String> {
        let tag = self.country_tag(slot)?.to_owned();
        if let Some(text) = operands::country_text(
            name,
            &tag,
            self.localisation,
            |suffix| self.names().name_with(slot, suffix),
            || Ok(self.names().ruling_group(slot)?.map(str::to_owned)),
        )? {
            return Ok(Some(text));
        }
        let is = |entry: &str| named(name, entry);
        let upper_first = |text: String| crt_case_text(&text, true, true);
        Ok(Some(if is("GetRulingParty") || is("GetRulingPartyLong") {
            self.ruling_party(slot, &tag, is("GetRulingPartyLong"))?
        } else if let Some(&(entry, suffix, cap)) = OLD_NAMES.iter().find(|(entry, ..)| is(entry)) {
            let text = self.old_name(slot, entry, suffix)?;
            if cap { upper_first(text) } else { text }
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
            let (characters, gap) = rows::<tables::Characters>(self.frame, CHARACTER);
            let row = keyed(&characters, u128::from(id))?
                .ok_or_else(|| format!("character {id} has no Characters row: {gap}"))?;
            return self.stored_character_name(&row).map(Some);
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

    fn character_name(&self, id: u32) -> Result<String, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::CharacterName(id)),
            || self.character_name_unrecorded(id),
        )
    }

    pub(crate) fn character_name_unrecorded(&self, id: u32) -> Result<String, String> {
        let (characters, gap) = rows::<tables::Characters>(self.frame, CHARACTER);
        let row = keyed(&characters, u128::from(id))?
            .ok_or_else(|| format!("character {id} has no Characters row: {gap}"))?;
        if !row.name.is_empty() {
            return self.stored_character_name(&row);
        }
        self.localisation
            .localise(&row.token)
            .map_err(|refusal| refusal.to_string())
    }

    fn stored_character_name(
        &self,
        row: &replay_data::rows::CharacterRow,
    ) -> Result<String, String> {
        stored_text(
            self.localisation,
            &self.content.recorded_language,
            &row.name,
            &format!("character {} name", row.id),
        )
    }

    fn capital(&self, slot: u32, original: bool) -> Result<Object, String> {
        crate::triggers::inputs::query_typed(
            || crate::triggers::inputs::Key::Text(TextQuery::Capital(slot, original)),
            || self.capital_unrecorded(slot, original),
        )
    }

    pub(crate) fn capital_unrecorded(&self, slot: u32, original: bool) -> Result<Object, String> {
        let (politics, gap) = rows::<tables::Politics>(self.frame, POLITICS);
        let row = first(&politics, |key| self.scopes.slot(key as u32) == Some(slot))?
            .ok_or_else(|| {
                format!("the capital of country slot {slot} is unknown: politics: {gap}")
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
        let (appearance, gap) = rows::<tables::CountryAppearance>(self.frame, APPEARANCE);
        let row = first(&appearance, |key| self.scopes.slot(key as u32) == Some(slot))?
            .ok_or_else(|| {
                format!("the overlord of country slot {slot} is unknown: country appearance: {gap}")
            })?;
        let link = match u32::try_from(row.overlord) {
            Ok(link) if link > 0 => link,
            _ => return Ok(None),
        };
        let (_, links) = rows::<tables::CountryLinks>(self.frame, 0);
        self.scopes.slot(link).map(Some).ok_or_else(|| {
            format!(
                "the overlord link {link} of country slot {slot} resolves to no country: country links: {links}"
            )
        })
    }

    fn country_row<T: TableSchema>(
        &self,
        slot: u32,
        mask: u64,
    ) -> Result<Option<Cow<'a, T::Row>>, String> {
        let (found, gap) = rows::<T>(self.frame, mask);
        if let Some(row) = first(&found, |key| self.scopes.slot(key as u32) == Some(slot))? {
            return Ok(Some(row));
        }
        if !complete::<T>(self.frame) {
            return Err(format!("{}: {gap}", T::ID.words()));
        }
        if !complete::<tables::CountryLinks>(self.frame)
            && found.keys().any(|key| self.scopes.slot(key as u32).is_none())
        {
            let (_, links) = rows::<tables::CountryLinks>(self.frame, 0);
            return Err(format!(
                "{} names a country that no recorded country link holds: country links: {links}",
                T::ID.words(),
            ));
        }
        Ok(None)
    }

    fn state_record(&self, id: u32) -> (bool, bool, String) {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::StateRecord(id)),
            || self.state_record_unrecorded(id),
        )
    }

    pub(crate) fn state_record_unrecorded(&self, id: u32) -> (bool, bool, String) {
        let (states, gap) = rows::<tables::States>(self.frame, 0);
        (
            states.contains(u128::from(id)),
            complete::<tables::States>(self.frame),
            gap,
        )
    }

    fn state_country(&self, id: u32, owner: bool) -> Result<Object, String> {
        crate::triggers::inputs::query_typed(
            || crate::triggers::inputs::Key::Text(TextQuery::StateCountry(id, owner)),
            || self.state_country_unrecorded(id, owner),
        )
    }

    pub(crate) fn state_country_unrecorded(&self, id: u32, owner: bool) -> Result<Object, String> {
        let (states, gap) = rows::<tables::States>(self.frame, STATE);
        let state = keyed(&states, u128::from(id))?
            .ok_or_else(|| format!("state {id} has no States row: {gap}"))?;
        let link = if owner { state.owner } else { state.controller };
        self.scopes
            .slot(link)
            .map(Object::Country)
            .ok_or_else(|| format!("state {id}'s country link {link} resolves to no country"))
    }

    fn character_country(&self, id: u32) -> Result<Object, String> {
        crate::triggers::inputs::query_typed(
            || crate::triggers::inputs::Key::Text(TextQuery::CharacterCountry(id)),
            || self.character_country_unrecorded(id),
        )
    }

    pub(crate) fn character_country_unrecorded(&self, id: u32) -> Result<Object, String> {
        let (characters, gap) = rows::<tables::Characters>(self.frame, CHARACTER);
        let row = keyed(&characters, u128::from(id))?
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

    fn event_object(&self, object: EventObject, name: &str) -> Result<Object, String> {
        crate::triggers::inputs::query_typed(
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
            EventObject::State(id) => {
                Object::State((id != crate::triggers::NULL_STATE).then_some(id))
            }
            EventObject::Character(id) => Object::Character(id),
            EventObject::UnitLeader(reference)
            | EventObject::CountryUnitLeader {
                leader: reference, ..
            } => {
                let (leaders, gap) = rows::<tables::UnitLeaders>(self.frame, UNIT_LEADER);
                let row = keyed(&leaders, u128::from(reference))?
                    .ok_or_else(|| {
                        format!(
                            "{name}: unit leader {reference:#x} is not recorded: unit leaders: {gap}"
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
            | EventObject::Combat => Object::State(None),
        })
    }

    fn local_player(&self) -> Result<u32, String> {
        if let Some(viewer) = self.viewer {
            return Ok(viewer);
        }
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::LocalPlayer),
            || self.local_player_unrecorded(),
        )
    }

    pub(crate) fn local_player_unrecorded(&self) -> Result<u32, String> {
        let (players, gap) = rows::<tables::LocalPlayer>(self.frame, PLAYER);
        if !complete::<tables::LocalPlayer>(self.frame) {
            return Err(format!(
                "the local player's country is not recorded this hour: local player: {gap}"
            ));
        }
        if players.len() != 1 {
            return Err(format!(
                "local player holds {} rows where the game state holds one local player",
                players.len()
            ));
        }
        let player = each(players)
            .next()
            .transpose()?
            .ok_or("local player holds no row")?;
        let raw = if (player.country as i32) > 0 {
            player.country
        } else {
            player.observer_country
        };
        if raw == 0 {
            return Ok(0);
        }
        self.scopes
            .slot(raw)
            .ok_or_else(|| "the local player's country has no recorded country link".to_owned())
    }

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
        let (characters, gap) = rows::<tables::Characters>(self.frame, CHARACTER);
        let (lexical, lexical_gap) = rows::<tables::LexicalTokens>(self.frame, 0);
        let links_whole = complete::<tables::CountryLinks>(self.frame);
        let country_links: Option<Vec<u32>> = country
            .filter(|_| links_whole)
            .map(|slot| -> Result<Vec<u32>, String> {
                let mut links = Vec::new();
                for link in each(rows::<tables::CountryLinks>(self.frame, LINK).0) {
                    let link = link?;
                    if link.canonical_slot == slot {
                        links.push(link.id);
                    }
                }
                Ok(links)
            })
            .transpose()?;
        let mut unknown = None;
        for row in each(characters) {
            let row = row?;
            if let Some(links) = &country_links {
                if !links.contains(&row.country) {
                    continue;
                }
            } else {
                match self.scopes.slot(row.country) {
                    _ if country.is_none() => {}
                    Some(slot) if Some(slot) == country => {}
                    None if !links_whole => {
                        let (_, links) = rows::<tables::CountryLinks>(self.frame, 0);
                        unknown.get_or_insert_with(|| {
                            format!(
                                "character {}'s country link {} resolves to no country: country links: {links}",
                                row.id, row.country
                            )
                        });
                        continue;
                    }
                    _ => continue,
                }
            }
            let Ok(token) = u32::try_from(row.name_token_id) else {
                continue;
            };
            match spelled(&lexical, token)? {
                Some(literal) if literal == segment.as_bytes() => {
                    if country.is_none() {
                        return Err(format!(
                            "{segment} names character {}, and the event scope's country, which character lookup searches, is not known here",
                            row.id
                        ));
                    }
                    return Ok(Some(row.id));
                }
                Some(_) => {}
                None => {
                    unknown.get_or_insert_with(|| {
                        format!(
                            "character {}'s script token {token} is not spelled: {}: {lexical_gap}",
                            row.id,
                            tables::LexicalTokens::ID.words()
                        )
                    });
                }
            }
        }
        let unknown = if self.frame.status(tables::Characters::ID).is_table_complete() {
            unknown
        } else {
            Some(format!("{}: {gap}", tables::Characters::ID.words()))
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
        let (politics, gap) = rows::<tables::Politics>(self.frame, POLITICS);
        let politics = first(&politics, |key| self.scopes.slot(key as u32) == Some(slot))?
            .ok_or_else(|| format!("the ruling party of {tag} is unknown: politics: {gap}"))?;
        let Some(party) = politics.ruling_party else {
            return Err(format!("{tag} has no ruling party"));
        };
        let (parties, gap) = rows::<tables::PoliticalParties>(self.frame, PARTY);
        let party = keyed(&parties, u128::from(politics.country) << 32 | u128::from(party))?
            .ok_or_else(|| {
                format!("the ruling party of {tag} has no political parties row: {gap}")
            })?;
        self.party_name(&party, tag, long)
    }

    pub fn party_name(
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
        let slot = if party.tag == 0 {
            0
        } else {
            self.scopes.slot(party.tag).ok_or_else(|| {
                format!("the ruling party of {tag}: its country has no recorded country link")
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
                crate::party_names::party_key_with_selected_tag(
                    &naming,
                    &content_reader_country_names::country_names::original_or_own_tag_in(
                        &self.frame,
                        slot,
                    )?,
                    defines,
                )
                .ok_or_else(|| {
                    format!("the ruling party of {tag}: its tag or original tag is not recorded")
                })?
            }
        };
        let long_key = format!("{key}_long");
        let key = if long && key != naming.token && defines(&long_key) {
            long_key
        } else {
            key
        };
        self.localise(&key)
    }

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

    pub fn faction_name_by_id(&self, id: u64) -> Result<String, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::FactionNameById(id)),
            || self.faction_name_by_id_unrecorded(id),
        )
    }

    pub(crate) fn faction_name_by_id_unrecorded(&self, id: u64) -> Result<String, String> {
        let (factions, gap) = rows::<tables::Factions>(self.frame, FACTION);
        let faction = keyed(&factions, u128::from(id))?
            .ok_or_else(|| format!("faction {id} has no Factions row: {gap}"))?;
        let (spelling, templates) = if faction.name.is_empty() && faction.template_token.is_some() {
            crate::catalogue::used(crate::catalogue::Reader::FactionTemplates);
            (
                crate::faction_names::template_spelling_in(&self.frame, faction.template_token),
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
            |name| self.lexical_names().token_of(name),
        )
    }
}

pub fn probe_unselected_tables(
    frame: &DecodedFrame,
    run: impl FnOnce(),
) -> Vec<replay_data::rows::TableId> {
    let ((_, failures), inputs) = crate::triggers::inputs::capture(frame, || {
        crate::triggers::refusal::capture(run)
    });
    let mut tables = std::collections::BTreeSet::new();
    let mut keep = |table: replay_data::rows::TableId| {
        if frame.tables[table.index()].held().is_not_selected() {
            tables.insert(table);
        }
    };
    for key in inputs.keys() {
        match key {
            crate::triggers::inputs::Key::Table(table)
            | crate::triggers::inputs::Key::Fields(table, _) => keep(*table),
            crate::triggers::inputs::Key::ClockSeed => keep(replay_data::rows::TableId::Clock),
            crate::triggers::inputs::Key::FocusTag(_) => {
                keep(replay_data::rows::TableId::Countries);
                keep(replay_data::rows::TableId::CountryLinks);
            }
            crate::triggers::inputs::Key::Names(query) => {
                for read in query.dependence() {
                    keep(read.table);
                }
            }
            crate::triggers::inputs::Key::Text(query) => {
                if let Some(reads) = query.dependence() {
                    for read in reads {
                        keep(read.table);
                    }
                }
            }
            crate::triggers::inputs::Key::Scopes(query) => {
                if let Some(reads) = query.dependence() {
                    for read in reads {
                        keep(read.table);
                    }
                }
            }
            crate::triggers::inputs::Key::World(query) => {
                if let Some(reads) = query.dependence() {
                    for read in reads {
                        keep(read.table);
                    }
                }
            }
        }
    }
    for failure in &failures {
        for table in failure.input_tables() {
            keep(table);
        }
    }
    tables.into_iter().collect()
}

const NO_FACTION: &str = "NO_FACTION";

const SCOPE_LINKS: [&str; 4] = ["THIS", "Root", "From", "Prev"];
const DATES: [&str; 4] = ["GetDateText", "GetDate", "GetYear", "GetMonth"];
const COUNTRY_NAMES: [&str; 8] = [
    "GetName",
    "GetNameDef",
    "GetNameDefCap",
    "GetAdjective",
    "GetAdjectiveCap",
    "GetTag",
    "GetFlag",
    "GetNameWithFlag",
];

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

const OLD_NAMES: [(&str, &str, bool); 5] = [
    ("GetOldName", "", false),
    ("GetOldNameDef", "_DEF", false),
    ("GetOldNameDefCap", "_DEF", true),
    ("GetOldAdjective", "_ADJ", false),
    ("GetOldAdjectiveCap", "_ADJ", true),
];

#[cfg(any(test, feature = "test-support"))]
pub fn complete_test_frame() -> DecodedFrame {
    let mut frame = crate::scope::tests::positive_frame();
    use replay_data::rows::{TableId, TableRows};
    for id in TableId::ALL {
        if matches!(&*frame.tables[id.index()], TableView::Unavailable { .. }) {
            *frame.tables[id.index()] = TableView::Complete(TableRows::empty(id));
        }
    }
    frame
}

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

#[cfg(test)]
mod root_tests;
