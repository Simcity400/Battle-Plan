use crate::triggers::Failure;

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
};

use content_reader_parser::parser::Item;
use content_reader_script::{scope::{Keyword as ScopeKeyword, Names, Scope as Parsed, ScopeKind, TriggerScopes}, scripted_trigger_grammar::Template, trigger_keys::{CountKind, Dispatch, Dispatched, ScopeStep, cycle, describe, equipment_category_mask, frame, registered_trigger, trigger_class}};
use replay_data::{rows::BuildingRow, rows::tables};

use super::{
    Eval, Scope, Scopes,
    operands::{Selected, Selector, compare, scope_selector, shown, trigger_boolean},
    saved_event_targets::Reader,
    variables::{TEMP_WRITERS, Temps, Value},
    world::{CHARACTER_FLAGS, NULL_STATE, World, table},
};
use crate::{
    character_scope::TraitNames,
    scope::{Context, Target, Unresolved, valid_saved_target},
    state_resources::{ONE, resource_comparison},
};

pub trait Combat: Sync {
    fn country(&self) -> Result<u32, Failure>;
    fn trigger(&self, name: &str, item: &Item, this: Scope, evaluator: &Evaluator) -> Option<Eval>;
    fn category_share(&self, mask: u64) -> Result<i64, Failure>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Count {
    Building {
        token: u32,
        provincial: Option<bool>,
    },
    Resource { index: u32, token: u32 },
    Ideology(Vec<u8>),
    Equipment(u64),
}

impl Count {
    fn scopes(&self) -> u32 {
        match self {
            Self::Building { .. } | Self::Resource { .. } => 6,
            Self::Ideology(_) => 4,
            Self::Equipment(_) => 0x10,
        }
    }

    fn describe(&self) -> &'static str {
        describe(match self {
            Self::Building { .. } => CountKind::Building,
            Self::Resource { .. } => CountKind::Resource,
            Self::Ideology(_) => CountKind::Ideology,
            Self::Equipment(_) => CountKind::Equipment,
        })
    }
}

enum EvaluationState {
    Idle,
    Running,
    Diverged(Failure),
}

pub(super) struct Evaluation<'e> {
    state: &'e RefCell<EvaluationState>,
    outermost: bool,
}

impl Evaluation<'_> {
    pub(super) fn divergence(&self) -> Option<Failure> {
        match &*self.state.borrow() {
            EvaluationState::Diverged(why) => Some(why.clone()),
            _ => None,
        }
    }
}

impl Drop for Evaluation<'_> {
    fn drop(&mut self) {
        if self.outermost {
            self.state.replace(EvaluationState::Idle);
        }
    }
}

pub struct Evaluator<'e, 'a> {
    pub(super) world: &'e World<'a>,
    combat: Option<&'e dyn Combat>,
    stack: RefCell<Vec<Vec<u8>>>,
    evaluation: RefCell<EvaluationState>,
    pub(super) aliases: RefCell<Vec<super::aliases::Entry>>,
    temps: Temps,
    pub(super) local_temps: RefCell<BTreeMap<Vec<u8>, Result<Value, Failure>>>,
    pub(super) checking: Cell<usize>,
    uncertain: Cell<bool>,
    callback_depth: Cell<usize>,
    written: RefCell<Option<Failure>>,
    variable_effects: Cell<u64>,
    pub(super) event_graph:
        RefCell<Option<std::rc::Rc<RefCell<crate::scripted_text::EventScopes>>>>,
    pub(super) description_depth: Cell<usize>,
}

#[derive(Clone)]
pub struct VariableSession {
    values: BTreeMap<Vec<u8>, Result<Value, Failure>>,
    written: Option<Failure>,
    active: bool,
    temps: Temps,
}

pub(super) enum Child<'t> {
    Keyless,
    Coded(u32, String),
    Scope(Parsed),
    Count(Count),
    Scripted(&'t Template),
}

struct LoadNames<'n> {
    recorded: &'n crate::scope::RecordedScopes<'n>,
    templates: &'n dyn Fn() -> Result<&'n BTreeSet<Vec<u8>>, &'n str>,
    asked: RefCell<Vec<Failure>>,
}

impl Names for LoadNames<'_> {
    fn is_country(&self, tag: &str) -> bool {
        if self.recorded.country(tag).is_some() {
            return true;
        }
        if let Err(why) = self.recorded.roster() {
            self.asked.borrow_mut().push(why.context(|why| format!(
                "whether {tag} names a country tag is unknown because Countries is incomplete: {why}"
            )));
            return false;
        }
        self.recorded.is_country(tag)
    }

    fn is_character(&self, token: &str) -> bool {
        match (self.templates)() {
            Ok(names) => names.contains(&token.as_bytes().to_ascii_lowercase()),
            Err(why) => {
                self.asked.borrow_mut().push(Failure::from(format!(
                    "whether {token} names a character template is unknown: {why}"
                )));
                false
            }
        }
    }

    fn is_keyword(&self, text: &str) -> bool {
        self.recorded.is_keyword(text)
    }
}

pub(super) fn key_text(item: &Item) -> String {
    String::from_utf8_lossy(item.key.as_deref().unwrap_or_default()).into_owned()
}

pub(super) fn fail_text(item: &Item, why: &str) -> String {
    format!(
        "trigger {} {} {}: {why}",
        key_text(item),
        String::from_utf8_lossy(item.operator.text()),
        shown(&item.value)
    )
}

impl<'e, 'a> Evaluator<'e, 'a> {
    pub fn new(world: &'e World<'a>, combat: Option<&'e dyn Combat>) -> Self {
        Self {
            world,
            combat,
            stack: RefCell::default(),
            evaluation: RefCell::new(EvaluationState::Idle),
            aliases: RefCell::default(),
            temps: Temps::Fresh,
            local_temps: RefCell::default(),
            checking: Cell::new(0),
            uncertain: Cell::new(false),
            callback_depth: Cell::new(0),
            written: RefCell::default(),
            variable_effects: Cell::new(0),
            event_graph: RefCell::default(),
            description_depth: Cell::new(0),
        }
    }

    pub fn temp_value(&self, name: &[u8]) -> Option<Result<i64, Failure>> {
        if let Some(writer) = self.temps_written() {
            writer.reached();
            return Some(Err(writer.context(|writer| {
                format!(
                    "temp variable {} is unknown after {writer}",
                    String::from_utf8_lossy(name)
                )
            })));
        }
        let value = self
            .local_temps
            .borrow()
            .get(name)
            .map(|value| match value {
                Ok(super::variables::Value::Raw(value)) => Ok(*value),
                Ok(super::variables::Value::Object(_)) => Err(format!(
                    "temp variable {} holds a scope, not a number",
                    String::from_utf8_lossy(name)
                )
                .into()),
                Err(why) => {
                    why.reached();
                    Err(why.clone())
                }
            });
        value.or_else(|| {
            (self.temps == Temps::Inherited)
                .then(|| Err(super::variables::inherited_temps_failure()))
        })
    }

    pub fn variable_session(&self) -> VariableSession {
        VariableSession {
            values: self.local_temps.borrow().clone(),
            written: self.written.borrow().clone(),
            active: self.description_depth.get() > 0 || self.checking.get() > 0,
            temps: self.temps(),
        }
    }

    pub fn restore_variable_session(&self, session: &VariableSession) {
        self.local_temps.replace(session.values.clone());
        self.written.replace(session.written.clone());
    }

    pub fn variable_effects(&self) -> u64 {
        self.variable_effects.get()
    }

    pub(super) fn note_variable_effect(&self) {
        self.variable_effects
            .set(self.variable_effects.get().wrapping_add(1));
    }

    pub fn with_variable_session(self, session: &VariableSession) -> Self {
        self.restore_variable_session(session);
        self.description_depth.set(usize::from(session.active));
        self.with_temps(session.temps)
    }

    pub fn with_temps(self, temps: Temps) -> Self {
        Self { temps, ..self }
    }

    pub(super) fn temps(&self) -> Temps {
        if self.temps == Temps::Inherited
            || (self.checking.get() == 0 && self.description_depth.get() == 0)
        {
            self.temps
        } else {
            Temps::Fresh
        }
    }

    pub(super) fn temps_written(&self) -> Option<Failure> {
        self.written.borrow().clone()
    }

    pub(super) fn clear_temp_writer(&self) {
        self.written.replace(None);
    }

    pub(super) fn taint_temps(&self, writer: Failure) {
        self.note_variable_effect();
        self.local_temps.borrow_mut().clear();
        self.written.borrow_mut().get_or_insert(writer);
    }

    pub fn evaluate_colour(&self, items: &[Item], scopes: Scopes, seeds: &[(&[u8], i64)]) -> Eval {
        let (result, failures) =
            self.capture_colour_inputs(|| self.evaluate_seeded(items, scopes, seeds));
        if failures.is_empty() {
            result
        } else {
            Eval::Unknown(failures)
        }
    }

    pub fn capture_colour_inputs<T>(&self, run: impl FnOnce() -> T) -> (T, BTreeSet<Failure>) {
        struct Restore<'c>(&'c Cell<usize>, usize);
        impl Drop for Restore<'_> {
            fn drop(&mut self) {
                self.0.set(self.1);
            }
        }
        let _restore = Restore(
            &self.callback_depth,
            self.callback_depth.replace(self.callback_depth.get() + 1),
        );
        super::refusal::capture(run)
    }

    pub(super) fn in_colour_callback(&self) -> bool {
        self.callback_depth.get() > 0
    }

    pub(super) fn maybe(&self, uncertain: bool, run: impl FnOnce() -> Eval) -> Eval {
        let previous = self.uncertain.replace(self.uncertain.get() || uncertain);
        let result = run();
        self.uncertain.set(previous);
        result
    }

    pub fn world(&self) -> &'e World<'a> {
        self.world
    }

    pub(crate) fn combat_context(&self) -> Option<&'e dyn Combat> {
        self.combat
    }

    pub(super) fn child(&self, item: &Item) -> Result<Option<Child<'a>>, Failure> {
        if item.key.is_none() {
            return Ok(Some(Child::Keyless));
        }
        self.dispatched(item)
    }

    pub(super) fn child_value(&self, item: &Item, child: &Child<'_>, scopes: Scopes) -> Eval {
        let name = key_text(item);
        let fail = |why: &str| {
            format!(
                "trigger {name} {} {}: {why}",
                String::from_utf8_lossy(item.operator.text()),
                shown(&item.value)
            )
        };
        match child {
            Child::Keyless => Eval::unknown(format!(
                "the value {} without a trigger name is not a condition",
                shown(&item.value)
            )),
            Child::Coded(token, name) => self.coded(*token, name, item, scopes),
            Child::Scope(parsed) => match self.enter_scope(parsed, scopes) {
                Ok(Some(entered)) if entered.this == Scope::State(NULL_STATE) => Eval::False,
                Ok(Some(entered)) => self.evaluate(item.value.items().unwrap_or_default(), entered),
                Ok(None) => Eval::False,
                Err(why) => Eval::unknown(why.context(|why| fail(why))),
            },
            Child::Count(count) if item.value.items().is_some() => Eval::unknown(fail(&format!(
                "it names {}, whose reader takes one value token and skips no block, so the game's parse leaves the text",
                count.describe()
            ))),
            Child::Count(count) => self.count(count, item, scopes),
            Child::Scripted(template) => self.scripted(template, item, scopes),
        }
    }

    pub fn enter_scope(&self, parsed: &Parsed, scopes: Scopes) -> Result<Option<Scopes>, Failure> {
        if scopes.cursor.is_some() {
            return self.enter_graph(parsed, scopes);
        }
        let Parsed::EventTarget { name, then } = parsed else {
            return self
                .enter(parsed, scopes)
                .map(|this| this.map(|this| scopes.entered(parsed, this)));
        };
        let Some(entered) = self.saved_scope(name.as_bytes(), Reader::Context, scopes)? else {
            return Ok(None);
        };
        let Some(then) = then else {
            return Ok(Some(entered));
        };
        let Some(this) = self.enter(then, entered)? else {
            return Ok(None);
        };
        let mut result = scopes.entered(parsed, this);
        if matches!(**then, Parsed::Keyword(ScopeKeyword::This)) {
            result.bare = entered.bare;
        }
        Ok(Some(result))
    }

    fn carries(&self, scope: Scope, mask: u32) -> Result<&'static [ScopeKind], Failure> {
        if mask == 0 {
            return Ok(&[]);
        }
        Ok(match scope {
            Scope::Country(_) => &[ScopeKind::Country],
            Scope::CountryLeader { country, .. } => {
                if mask & ScopeKind::Country.bit() == 0 {
                    &[ScopeKind::Character]
                } else {
                    match self.variable_scope_has_country(country) {
                        Ok(true) => &[ScopeKind::Country],
                        Ok(false) | Err(_) if mask & ScopeKind::Character.bit() != 0 => {
                            &[ScopeKind::Character]
                        }
                        Ok(false) => &[],
                        Err(why) => return Err(why),
                    }
                }
            }
            Scope::Combat => &[ScopeKind::Combat],
            Scope::State(id) if id as i32 <= 0 => &[],
            Scope::State(_) => &[ScopeKind::State],
            Scope::Character(_) | Scope::Leader(_) => &[ScopeKind::Character],
            Scope::IndustrialOrganisation(_) => &[ScopeKind::IndustrialOrganisation],
            Scope::Project { .. } => &[ScopeKind::Project],
            Scope::Raid(_) => &[ScopeKind::RaidInstance],
            Scope::Unit(_) => &[ScopeKind::Unit],
            Scope::Province(province) => {
                return Err(format!(
                    "the scope kinds a province scope (province {province}) carries are not reproduced"
                ).into());
            }
        })
    }

    fn coded(&self, token: u32, name: &str, item: &Item, scopes: Scopes) -> Eval {
        if let Some((mask, _)) = trigger_class(token) {
            match self.carries(scopes.this, mask) {
                Ok(kinds) => {
                    if !TriggerScopes(mask).accepts(|kind| kinds.contains(&kind)) {
                        return self.rejected(mask, item, scopes);
                    }
                }
                Err(why) if mask != 0 => {
                    return Eval::unknown(
                        why.context(|why| format!("trigger {}: {why}", key_text(item))),
                    );
                }
                Err(_) => {}
            }
        }
        if let Some(result) = self.temp_operation(name, item, scopes, self.uncertain.get()) {
            return result.map_or_else(
                |why| Eval::unknown(why.context(|why| fail_text(item, why))),
                |_| Eval::True,
            );
        }
        if TEMP_WRITERS.contains(&name) {
            let why = "this temp writer is not reproduced; its reached use is refused";
            if !self.in_colour_callback() {
                self.taint_temps(
                    Failure::from(why).context(|why| format!("trigger {}: {why}", key_text(item))),
                );
            }
            return Eval::unknown(crate::triggers::refusal::refused(fail_text(item, why)));
        }
        if let Some(combat) = self.combat
            && let Some(value) = combat.trigger(name, item, scopes.this, self)
        {
            return value;
        }
        let block = || item.value.items();
        let scalar = item.value.scalar_str();
        let unsupported = || {
            Eval::unknown(format!(
                "trigger {} {} {}: the viewer does not reproduce this trigger",
                key_text(item),
                String::from_utf8_lossy(item.operator.text()),
                shown(&item.value)
            ))
        };
        match name {
            "and" => return block().map_or_else(unsupported, |b| self.evaluate(b, scopes)),
            "or" => return block().map_or_else(unsupported, |b| self.any(b, scopes)),
            "not" => return block().map_or_else(unsupported, |b| self.any(b, scopes).negate()),
            "always" => {
                return match trigger_boolean(item) {
                    Ok(true) => Eval::True,
                    Ok(false) => Eval::False,
                    Err(reason) => Eval::unknown(reason),
                };
            }
            "hidden_trigger" => {
                return block().map_or_else(unsupported, |b| self.evaluate(b, scopes));
            }
            "has_global_flag" => {
                return self
                    .flag_trigger(item, 0, Some(0), scopes)
                    .unwrap_or_else(Eval::unknown);
            }
            "custom_override_tooltip" | "custom_trigger_tooltip" => {
                let Some(block) = block() else {
                    return unsupported();
                };
                let children: Vec<Item> = block
                    .iter()
                    .filter(|child| {
                        !child.keyword_is("tooltip") && !child.keyword_is("not_tooltip")
                    })
                    .cloned()
                    .collect();
                return self.evaluate(&children, scopes);
            }
            "has_dlc" => {
                return scalar.map_or_else(unsupported, |dlc| {
                    Eval::from_bool(self.world.dlcs.contains(dlc))
                });
            }
            "any_owned_state"
            | "any_controlled_state"
            | "all_owned_state"
            | "all_controlled_state" => {
                return block()
                    .map_or_else(unsupported, |b| self.any_country_state(name, b, scopes));
            }
            "any_country_with_core" | "any_neighbor_country" | "all_neighbor_country" => {
                return block().map_or_else(unsupported, |b| {
                    self.map_country_iterator(name, b, scopes)
                        .unwrap_or_else(Eval::unknown)
                });
            }
            "is_in_array" => {
                return block().map_or_else(unsupported, |b| {
                    self.is_in_array(b, scopes).map_or_else(
                        |why| Eval::unknown(why.context(|why| fail_text(item, why))),
                        Eval::from_bool,
                    )
                });
            }
            "any_state_in" => {
                return block().map_or_else(unsupported, |b| self.any_state_in(b, scopes));
            }
            "any_of_scopes" | "all_of_scopes" => {
                return block().map_or_else(unsupported, |b| {
                    self.array_scope_iterator(name, b, scopes)
                        .unwrap_or_else(Eval::unknown)
                });
            }
            "any_country_with_original_tag" | "all_country_with_original_tag" => {
                return block().map_or_else(unsupported, |b| {
                    self.countries_with_original_tag(name, b, scopes)
                        .unwrap_or_else(Eval::unknown)
                });
            }
            "any_neighbor_state" | "all_neighbor_state" => {
                return block().map_or_else(unsupported, |b| self.neighbor_state(name, b, scopes));
            }
            "any_scientist" => {
                return block().map_or_else(unsupported, |b| {
                    self.country_here(scopes.this)
                        .and_then(|country| self.any_scientist(b, country, scopes))
                        .unwrap_or_else(Eval::unknown)
                });
            }
            "has_event_target" => {
                let name = match item.value.c_str() {
                    Ok(n) => n,
                    Err(e) => return Eval::unknown(e.to_string()),
                };
                match self.local_saved_exists(name.as_bytes(), scopes) {
                    Ok(true) => return Eval::True,
                    Err(why) => return Eval::unknown(why),
                    Ok(false) => {}
                }
                return match self.world.recorded().saved_target(name.as_bytes()) {
                    Ok(target) => Eval::from_bool(target.as_deref().is_some_and(valid_saved_target)),
                    Err(why) => Eval::unknown(why),
                };
            }
            "tag" => return self.tag(item, scopes).unwrap_or_else(Eval::unknown),
            "check_variable" => {
                return self.check_variable(item, scopes).map_or_else(
                    |why| Eval::unknown(why.context(|why| fail_text(item, why))),
                    Eval::from_bool,
                );
            }
            "has_variable" => {
                return self.has_variable(item, scopes).map_or_else(
                    |why| Eval::unknown(why.context(|why| fail_text(item, why))),
                    Eval::from_bool,
                );
            }
            _ => {}
        }
        let scoped = match scopes.this {
            Scope::Country(slot) => self.country_trigger(name, item, slot, scopes),
            Scope::CountryLeader { country, .. } => {
                if trigger_class(token).is_some_and(|(mask, _)| {
                    mask & ScopeKind::Character.bit() != 0 && mask & ScopeKind::Country.bit() == 0
                }) {
                    self.character_trigger(name, item, scopes)
                } else {
                    self.country_trigger(name, item, country, scopes)
                }
            }
            Scope::Combat => match self.combat_country() {
                Ok(slot) => self.country_trigger(name, item, slot, scopes),
                Err(why) => Err(why),
            },
            Scope::State(state) => self.state_trigger(name, item, state, scopes),
            Scope::Character(_) | Scope::Leader(_) => self.character_trigger(name, item, scopes),
            Scope::IndustrialOrganisation(reference) => {
                self.organisation_trigger(name, item, reference, scopes)
            }
            Scope::Province(_) | Scope::Project { .. } | Scope::Raid(_) | Scope::Unit(_) => {
                Ok(None)
            }
        };
        match scoped {
            Ok(Some(value)) => value,
            Ok(None) => unsupported(),
            Err(why) => Eval::unknown(why.context(|why| fail_text(item, why))),
        }
    }

    pub(super) fn enter_frame(&self, frame: Vec<u8>) -> Option<String> {
        let mut stack = self.stack.borrow_mut();
        if stack.contains(&frame) {
            return Some(format!(
                "{} is evaluated inside itself, and the game guards no depth",
                String::from_utf8_lossy(&frame)
            ));
        }
        stack.push(frame);
        None
    }

    pub(super) fn leave_frame(&self) {
        self.stack.borrow_mut().pop();
    }

    pub(super) fn begin_evaluation(&self) -> Evaluation<'_> {
        let mut state = self.evaluation.borrow_mut();
        let outermost = matches!(*state, EvaluationState::Idle);
        if outermost {
            *state = EvaluationState::Running;
        }
        Evaluation {
            state: &self.evaluation,
            outermost,
        }
    }

    pub(super) fn record_divergence(&self, why: &Failure) {
        let mut state = self.evaluation.borrow_mut();
        if !matches!(*state, EvaluationState::Idle) {
            *state = EvaluationState::Diverged(why.clone());
        }
    }

    fn rejected(&self, mask: u32, item: &Item, scopes: Scopes) -> Eval {
        if mask & ScopeKind::Character.bit() == 0 || scopes.bare {
            return Eval::False;
        }
        if scopes.unknown.root || scopes.unknown.prev {
            return Eval::unknown(format!(
                "{}: whether this scope was entered from a unit leader's is not known here (its ROOT or PREV is not)",
                key_text(item)
            ));
        }
        if [Some(scopes.root), scopes.prev]
            .into_iter()
            .flatten()
            .any(|scope| matches!(scope, Scope::Leader(_) | Scope::CountryLeader { .. }))
        {
            return Eval::unknown(format!(
                "trigger {} in a scope entered from a unit leader's is not reproduced",
                key_text(item)
            ));
        }
        Eval::False
    }

    pub(super) fn combat_country(&self) -> Result<u32, Failure> {
        match self.combat {
            Some(combat) => combat.country(),
            None => Err("the combat scope has no combat here".into()),
        }
    }

    fn country_here(&self, scope: Scope) -> Result<u32, Failure> {
        match scope {
            Scope::Country(slot) | Scope::CountryLeader { country: slot, .. } => Ok(slot),
            Scope::Combat => self.combat_country(),
            other => Err(format!(
                "the trigger's scope check passed in a {} scope, which holds no country",
                self.describe(other)
            )
            .into()),
        }
    }

    pub fn describe(&self, scope: Scope) -> String {
        match scope {
            Scope::Country(slot) => self.world.recorded().describe(Target::Country(slot)),
            Scope::State(NULL_STATE) => "the empty scope".into(),
            Scope::State(id) => format!("state {id}"),
            Scope::Province(id) => format!("province {id}"),
            Scope::Character(id) => format!("character {id}"),
            Scope::Leader(reference) => format!("unit leader {reference:#x}"),
            Scope::CountryLeader { country, leader } => {
                format!("country {country} and unit leader {leader:#x}")
            }
            Scope::IndustrialOrganisation(reference) => {
                format!("industrial organisation {reference:#x}")
            }
            Scope::Project { owner, id } => format!("project {id} of country {owner}"),
            Scope::Raid(reference) => format!("raid {reference:#x}"),
            Scope::Unit(reference) => format!("unit {reference:#x}"),
            Scope::Combat => "combat".into(),
        }
    }

    pub(super) fn core(&self) -> Result<(), Failure> {
        match self.world.core_gap() {
            Some(gap) => {
                Err(gap.context(|gap| format!("a core table is not complete this hour: {gap}")))
            }
            None => Ok(()),
        }
    }

    fn dispatched(&self, item: &Item) -> Result<Option<Child<'a>>, Failure> {
        let name = key_text(item);
        let templates = || self.world.character_templates();
        let names = LoadNames {
            recorded: self.world.recorded(),
            templates: &templates,
            asked: RefCell::default(),
        };
        let scope = |text: &[u8], quoted| match std::str::from_utf8(text) {
            Err(_) => ScopeStep::Unknown(
                format!("trigger {name} is not UTF-8, so its text cannot be read as a scope")
                    .into(),
            ),
            Ok(text) => {
                let parsed =
                    super::refusal::deferred(|| content_reader_script::scope::parse(text, quoted, &names));
                let asked = names.asked.take();
                let why = (!asked.is_empty()).then(|| Failure::merge(asked));
                if let Some(why) = &why {
                    why.reached();
                }
                match parsed {
                    Parsed::Unknown(_) => why.map_or(ScopeStep::NotScope, ScopeStep::Unknown),
                    scope => ScopeStep::Scope(scope),
                }
            }
        };
        let count = |keyword: &[u8]| self.count_kind(&name, keyword);
        let keyword = item.keyword();
        let Dispatched {
            dispatch,
            unless_scope,
        } = content_reader_script::trigger_keys::dispatch_with_error(
            content_reader_script::trigger_keys::Key {
                keyword: keyword.as_ref().map(|keyword| keyword.text()),
                text: item.key.as_deref().unwrap_or_default(),
                quoted: item.key_quoted,
                opens: item.value.items().is_some(),
            },
            scope,
            count,
            |text| {
                self.world
                    .triggers
                    .ok()
                    .and_then(|triggers| match triggers.scripted.lookup(text) {
                        content_reader_script::scripted_trigger_grammar::Lookup::Template(template) => {
                            Some(template)
                        }
                        content_reader_script::scripted_trigger_grammar::Lookup::Missing => None,
                    })
            },
        )?;
        if let Some(why) = unless_scope {
            return Err(why);
        }
        Ok(match dispatch {
            Dispatch::Coded => item.keyword().and_then(|keyword| {
                let name = String::from_utf8_lossy(&keyword.folded()).into_owned();
                Some(Child::Coded(registered_trigger(keyword.text())?, name))
            }),
            Dispatch::Scope(parsed) => Some(Child::Scope(parsed)),
            Dispatch::Count(count) => Some(Child::Count(count)),
            Dispatch::Scripted(template) => Some(Child::Scripted(template)),
            Dispatch::Absent => {
                self.world.triggers.map_err(|why| {
                    Failure::from(why).context(|why| format!("trigger {name}: {why}"))
                })?;
                None
            }
        })
    }

    fn count_kind(&self, name: &str, keyword: &[u8]) -> Result<Option<Count>, Failure> {
        let Some(id) = self.world.token_of(keyword).map_err(|why| why.context(|why| format!(
                "whether trigger {name} is a token (count lookups are keyed by token ID) is unknown: {why}"
            )))?
        else {
            return Ok(None);
        };
        let counts = self.world.triggers.map_err(Failure::from)?.counts.as_ref().ok_or_else(|| -> Failure {
            format!(
                "whether trigger {name} names a building or an ideology is unknown: the content's building and ideology databases are not loaded"
            ).into()
        })?;
        let text = keyword;
        Ok(if counts.is_building(text) {
            Some(Count::Building {
                token: id,
                provincial: counts.is_provincial(text),
            })
        } else if let Some(index) = self.resource_index(id).map_err(|why| {
            why.context(|why| format!("whether trigger {name} names a resource is unknown: {why}"))
        })? {
            Some(Count::Resource { index, token: id })
        } else if counts
            .is_ideology(text)
            .map_err(|why| Failure::from(why).context(|why| format!("trigger {name}: {why}")))?
        {
            Some(Count::Ideology(text.to_ascii_lowercase()))
        } else {
            equipment_category_mask(id).map(Count::Equipment)
        })
    }

    fn resource_index(&self, token: u32) -> Result<Option<u32>, Failure> {
        let definitions = counted::<tables::ResourceDefinitions>(
            self.world,
            replay_data::rows::mask::<replay_data::rows::ResourceDefinitionRow>(&[
                "token", "active",
            ]),
        )?;
        Ok(definitions
            .iter()
            .find(|row| row.token == token && row.active != 0)
            .map(|row| row.id))
    }

    pub fn enter(&self, parsed: &Parsed, scopes: Scopes) -> Result<Option<Scope>, Failure> {
        if scopes.cursor.is_some() && self.event_graph.borrow().is_some() {
            return self
                .enter_scope(parsed, scopes)
                .map(|entered| entered.map(|s| s.this));
        }
        self.enter_flat(parsed, scopes)
    }

    pub(super) fn enter_flat(
        &self,
        parsed: &Parsed,
        scopes: Scopes,
    ) -> Result<Option<Scope>, Failure> {
        let world = self.world;
        if let Parsed::Prefixed { prefix, value } = parsed
            && matches!(prefix.as_str(), "mio" | "sp")
        {
            let country = match scopes.this {
                Scope::Country(slot) | Scope::CountryLeader { country: slot, .. } => slot,
                Scope::Combat => self.combat_country()?,
                Scope::Province(_) => {
                    return Err(format!("{parsed}: province scope country is not known").into());
                }
                _ => return Ok(Some(scopes.this)),
            };
            let raw = world.raw_country(country).ok_or_else(|| -> Failure {
                format!("{parsed}: Countries lacks THIS country {country}").into()
            })?;
            if (raw as i32) < 1 {
                return Ok(Some(scopes.this));
            }
            return self
                .named_carrier(prefix, value, country)
                .map(|target| Some(target.unwrap_or(Scope::State(NULL_STATE))));
        }
        if matches!(parsed, Parsed::Keyword(ScopeKeyword::Owner)) {
            let raw = match scopes.this {
                Scope::IndustrialOrganisation(reference) => {
                    Some(world.industrial_organisation(reference)?.owner)
                }
                Scope::Project { owner, id } => Some(world.project(owner, id)?.owner),
                Scope::Unit(reference) => Some(world.unit(reference)?.logical_owner),
                _ => None,
            };
            if let Some(raw) = raw {
                if raw == 0 {
                    return Ok(Some(Scope::Country(0)));
                }
                return world
                    .slot(raw)
                    .map(|slot| Some(Scope::Country(slot)))
                    .ok_or_else(|| -> Failure {
                        format!("country links lacks scope owner {raw}").into()
                    });
            }
        }
        match parsed {
            Parsed::Keyword(ScopeKeyword::This) => Ok(Some(scopes.this)),
            Parsed::Keyword(
                keyword @ (ScopeKeyword::Root | ScopeKeyword::Prev | ScopeKeyword::From),
            ) => scopes.link(*keyword),
            Parsed::Keyword(ScopeKeyword::Owner) if scopes.this == Scope::Combat => {
                self.combat_country().map(|slot| Some(Scope::Country(slot)))
            }
            Parsed::Keyword(keyword @ (ScopeKeyword::Owner | ScopeKeyword::Controller))
                if matches!(scopes.this, Scope::State(_) | Scope::Province(_)) =>
            {
                self.core()?;
                let row = match scopes.this {
                    Scope::State(state) => world.state_row(state),
                    Scope::Province(province) => world.province_row(province),
                    _ => None,
                };
                let raw = row.map(|(owner, controller)| {
                    if *keyword == ScopeKeyword::Owner {
                        owner
                    } else {
                        controller
                    }
                });
                Ok(raw.and_then(|raw| world.slot(raw)).map(Scope::Country))
            }
            Parsed::Chain { head, then } => {
                let head_target = Parsed::Keyword(*head);
                let Some(head) = self.enter(&head_target, scopes)? else {
                    return Ok(None);
                };
                self.enter(then, scopes.entered(&head_target, head))
            }
            Parsed::EventTarget { .. } => self
                .enter_scope(parsed, scopes)
                .map(|entered| entered.map(|s| s.this)),
            Parsed::Character(_) => self.character_target(parsed, scopes),
            Parsed::State(id) => {
                let state = u32::try_from(*id)
                    .ok()
                    .filter(|id| world.state_row(*id).is_some());
                match (state, world.table_gap("States")) {
                    (None, Some(gap)) => {
                        Err(gap
                            .context(|gap| format!("state {id} has no recorded States row: {gap}")))
                    }
                    (state, _) => Ok(state.map(Scope::State)),
                }
            }
            Parsed::Country(tag) => self
                .resolve_tag(tag, scopes)
                .map(|slot| slot.map(Scope::Country)),
            Parsed::Variable(name) => self.scoped_scope(format!("var:{name}").as_bytes(), scopes),
            _ => {
                let context = match parsed {
                    Parsed::Keyword(ScopeKeyword::FactionLeader) => {
                        let this = match scopes.this {
                            Scope::Country(slot) | Scope::CountryLeader { country: slot, .. } => {
                                Target::Country(slot)
                            }
                            Scope::State(_)
                            | Scope::Character(_)
                            | Scope::Combat
                            | Scope::Leader(_)
                            | Scope::IndustrialOrganisation(_)
                            | Scope::Project { .. }
                            | Scope::Raid(_)
                            | Scope::Unit(_) => {
                                Target::State(NULL_STATE)
                            }
                            Scope::Province(province) => {
                                return Err(format!(
                                    "faction_leader: province {province}'s carried country/faction is not reproduced"
                                ).into());
                            }
                        };
                        Context {
                            this: Some(this),
                            ..Context::default()
                        }
                    }
                    Parsed::Keyword(_) | Parsed::Character(_) => self.context(scopes)?,
                    _ => Context::default(),
                };
                match world.recorded().resolve(parsed, &context) {
                    Ok(target) => Ok(Some(match target {
                        Target::Country(slot) => Scope::Country(slot),
                        Target::State(id) => Scope::State(id),
                        Target::Character(id) => Scope::Character(id),
                    })),
                    Err(Unresolved::Nothing(_)) => Ok(None),
                    Err(Unresolved::Missing(why)) => Err(why),
                }
            }
        }
    }

    fn character_target(&self, parsed: &Parsed, scopes: Scopes) -> Result<Option<Scope>, Failure> {
        match scopes.this {
            Scope::Country(_) | Scope::CountryLeader { .. } | Scope::Combat => {}
            Scope::State(_)
            | Scope::Character(_)
            | Scope::Leader(_)
            | Scope::IndustrialOrganisation(_)
            | Scope::Project { .. }
            | Scope::Raid(_)
            | Scope::Unit(_) => {
                return Ok(Some(scopes.this));
            }
            Scope::Province(province) => {
                return Err(format!(
                    "{parsed}: whether province {province}'s scope holds a country, which the character lookup needs, is not reproduced"
                ).into());
            }
        }
        match self
            .world
            .recorded()
            .resolve(parsed, &self.context(scopes)?)
        {
            Ok(Target::Character(id)) => Ok(Some(Scope::Character(id))),
            Ok(other) => Err(format!(
                "{parsed} resolved to {}, not a character",
                self.world.recorded().describe(other)
            )
            .into()),
            Err(Unresolved::Nothing(_)) => Ok(Some(Scope::State(NULL_STATE))),
            Err(Unresolved::Missing(why)) => Err(why),
        }
    }

    fn context(&self, scopes: Scopes) -> Result<Context, Failure> {
        let this = match scopes.this {
            Scope::Country(slot) | Scope::CountryLeader { country: slot, .. } => {
                Target::Country(slot)
            }
            Scope::Combat => Target::Country(self.combat_country()?),
            Scope::State(id) => Target::State(id),
            Scope::Character(id) => Target::Character(id),
            other @ (Scope::Province(_)
            | Scope::Leader(_)
            | Scope::IndustrialOrganisation(_)
            | Scope::Project { .. }
            | Scope::Raid(_)
            | Scope::Unit(_)) => {
                return Err(format!(
                    "a scope keyword read from a {} is not reproduced",
                    self.describe(other)
                )
                .into());
            }
        };
        Ok(Context {
            this: Some(this),
            ..Context::default()
        })
    }

    pub fn mean_time_to_happen(
        &self,
        what: &str,
        fields: &[Item],
        scopes: Scopes,
    ) -> Result<i64, Failure> {
        if !fields
            .iter()
            .any(|item| item.keyword_is("modifier") && item.value.items().is_some())
        {
            return Ok(super::operands::mtth_base(fields));
        }
        let depth = self.checking.get();
        if depth == 0 && self.description_depth.get() == 0 {
            self.local_temps.borrow_mut().clear();
            self.clear_temp_writer();
            self.initialize_variable_random(scopes);
        }
        self.checking.set(depth + 1);
        let result = (|| {
            let mut value = super::operands::mtth_base(fields);
            for items in fields
                .iter()
                .filter(|item| item.keyword_is("modifier"))
                .filter_map(|modifier| modifier.value.items())
            {
                let conditions: Vec<_> = items
                    .iter()
                    .filter(|item| !item.keyword_is("factor") && !item.keyword_is("add"))
                    .cloned()
                    .collect();
                if self.evaluate(&conditions, scopes).decided_input()? {
                    let factor = self.mtth_operand(what, items, "factor", ONE, scopes)?;
                    let add = self.mtth_operand(what, items, "add", 0, scopes)?;
                    value = (value.wrapping_mul(factor) / ONE).wrapping_add(add);
                    if value == 0 {
                        break;
                    }
                }
            }
            Ok(value)
        })();
        self.checking.set(depth);
        result
    }

    fn count(&self, count: &Count, item: &Item, scopes: Scopes) -> Eval {
        match self.carries(scopes.this, count.scopes()) {
            Ok(kinds) => {
                if !TriggerScopes(count.scopes()).accepts(|kind| kinds.contains(&kind)) {
                    return Eval::False;
                }
            }
            Err(why) => return Eval::unknown(why),
        }
        let fail = |why: Failure| {
            Eval::unknown(why.context(|why| {
                format!(
                    "trigger {} {} {}: {}: {why}",
                    key_text(item),
                    String::from_utf8_lossy(item.operator.text()),
                    shown(&item.value),
                    count.describe()
                )
            }))
        };
        let bytes = item.value.scalar().unwrap_or_default();
        let target = || match scopes.this {
            Scope::Combat => self.combat_country().map(Target::Country),
            Scope::Country(slot) | Scope::CountryLeader { country: slot, .. } => {
                Ok(Target::Country(slot))
            }
            Scope::State(id) => Ok(Target::State(id)),
            Scope::Character(id) => Ok(Target::Character(id)),
            other => Err(format!("reads no {} scope", self.describe(other)).into()),
        };
        let (actual, fixed) = match count {
            Count::Building { token, provincial } => {
                match target().and_then(|target| self.building_level(*token, *provincial, target)) {
                    Ok(level) => (i64::from(level) * ONE, false),
                    Err(why) => return fail(why),
                }
            }
            Count::Resource { index, token } => {
                let amount = match target()
                    .and_then(|target| self.resource_amount(*index, *token, target))
                {
                    Ok(amount) => amount,
                    Err(why) => return fail(why),
                };
                let wanted = match self.required(bytes, item.quoted(), false, scopes) {
                    Ok(wanted) => wanted / ONE,
                    Err(why) => return fail(why),
                };
                return Eval::from_bool(resource_comparison(amount, item.operator, wanted as i32));
            }
            Count::Ideology(name) => {
                match target().and_then(|target| self.ideology_support(name, target)) {
                    Ok(support) => (support, true),
                    Err(why) => return fail(why),
                }
            }
            Count::Equipment(mask) => {
                match self.combat.map(|combat| combat.category_share(*mask)) {
                    Some(Ok(share)) => (share, true),
                    Some(Err(why)) => return fail(why),
                    None => return fail("the combat scope has no combat here".into()),
                }
            }
        };
        match self.required(bytes, item.quoted(), fixed, scopes) {
            Ok(required) => Eval::from_bool(compare(actual, required, item.operator, fixed)),
            Err(why) => fail(why),
        }
    }

    fn owned_by(&self, link: u32, slot: u32) -> Result<bool, Failure> {
        if link == 0 {
            return Ok(false);
        }
        match self.world.slot(link) {
            Some(owner) => Ok(owner == slot),
            None => Err(match self.world.table_gap("CountryLinks") {
                Some(gap) => gap.context(|gap| {
                    format!("country link {link} has no country links row, and {gap}")
                }),
                None => Failure::input(
                    replay_data::rows::TableId::CountryLinks,
                    "canonical_slot",
                    None,
                    format!("country link {link} has no country links row"),
                ),
            }),
        }
    }

    fn building_level(
        &self,
        token: u32,
        provincial: Option<bool>,
        target: Target,
    ) -> Result<i32, Failure> {
        let world = self.world;
        let (in_states, in_provinces) = (provincial != Some(true), provincial != Some(false));
        let province_rows = || {
            super::world::province_values(world.tables())
                .map_err(|error| {
                    error.context(|error| format!("Provinces is not complete this hour: {error}"))
                })?
                .collect::<Result<Vec<_>, Failure>>()
        };
        let (states, provinces): (BTreeSet<u32>, BTreeSet<u32>) = match target {
            Target::Country(slot) => {
                let mut states = BTreeSet::new();
                if in_states {
                    for state in counted::<tables::States>(
                        world,
                        replay_data::rows::mask::<replay_data::rows::StateRow>(&["owner"]),
                    )? {
                        if self.owned_by(state.owner, slot).map_err(|why| {
                            why.context(|why| format!("the owner of state {}: {why}", state.id))
                        })? {
                            states.insert(state.id);
                        }
                    }
                }
                let mut owned = BTreeSet::new();
                if in_provinces {
                    for province in province_rows()? {
                        if self.owned_by(province.owner, slot).map_err(|why| {
                            why.context(|why| {
                                format!("the owner of province {}: {why}", province.id)
                            })
                        })? {
                            owned.insert(province.id);
                        }
                    }
                }
                (states, owned)
            }
            Target::State(id) => (
                BTreeSet::from([id]),
                if in_provinces {
                    province_rows()?
                        .iter()
                        .filter(|province| province.state == id)
                        .map(|province| province.id)
                        .collect()
                } else {
                    BTreeSet::new()
                },
            ),
            Target::Character(_) => {
                return Err("reads no character scope (mask 6: state, country)".into());
            }
        };
        const LEVEL: u64 = replay_data::rows::mask::<BuildingRow>(&["level"]);
        let level = |rows: &[std::borrow::Cow<'_, BuildingRow>], at: &BTreeSet<u32>| {
            rows.iter()
                .filter(|row| row.building_type == token && at.contains(&row.location))
                .fold(0_i32, |sum, row| sum.wrapping_add(i32::from(row.level)))
        };
        let mut sum = 0_i32;
        if in_states {
            let rows = counted::<tables::StateBuildings>(world, LEVEL)?;
            sum = sum.wrapping_add(level(&rows, &states));
        }
        if in_provinces {
            let rows = counted::<tables::ProvinceBuildings>(world, LEVEL)?;
            sum = sum.wrapping_add(level(&rows, &provinces));
        }
        Ok(sum)
    }

    fn resource_amount(&self, index: u32, token: u32, target: Target) -> Result<i64, Failure> {
        let slot = match target {
            Target::Country(slot) => slot,
            Target::State(state) => return self.state_resource_amount(state, token),
            Target::Character(_) => {
                return Err("reads no character scope (mask 6: state, country)".into());
            }
        };
        let rows = counted::<tables::CountryResources>(
            self.world,
            replay_data::rows::mask::<replay_data::rows::CountryResourceRow>(&[
                "produced",
                "adjustment",
            ]),
        )?;
        let mut found = None;
        for row in rows.iter().filter(|row| row.resource == index) {
            if self.owned_by(row.country, slot).map_err(|why| {
                why.context(|why| format!("a country resources row of resource {index}: {why}"))
            })? {
                found = Some(row);
                break;
            }
        }
        let row = found.ok_or_else(|| -> Failure {
            format!("country resources has no row for resource {index} of country slot {slot}")
                .into()
        })?;
        Ok(row.produced.wrapping_add(row.adjustment))
    }

    fn state_resource_amount(&self, state: u32, token: u32) -> Result<i64, Failure> {
        self.core().map_err(|why| why.context(|why| format!("the controllers' building overrides (country building resources) join through country links: {why}")))?;
        let world = self.world;
        let (_, controller) = world
            .state_row(state)
            .ok_or_else(|| -> Failure { format!("state {state} has no States row").into() })?;
        let controller = match controller {
            0 => None,
            link => Some(world.slot(link).ok_or_else(|| -> Failure {
                format!(
                    "the controller of state {state}: country link {link} has no country links row"
                )
                .into()
            })?),
        };
        world.state_resource_amount(state, token, controller)
    }

    fn ideology_support(&self, group: &[u8], target: Target) -> Result<i64, Failure> {
        let Target::Country(slot) = target else {
            return Err("reads only a country scope (mask 4)".into());
        };
        let parties = counted::<tables::PoliticalParties>(
            self.world,
            replay_data::rows::mask::<replay_data::rows::PoliticalPartyRow>(&[
                "token",
                "popularity",
            ]),
        )?;
        let (mut total, mut matching) = (0_i64, 0_i64);
        for party in parties {
            if !self.owned_by(party.country, slot).map_err(|why| {
                why.context(|why| format!("the country of a political parties row: {why}"))
            })? {
                continue;
            }
            if party.token.eq_ignore_ascii_case(group) {
                matching = party.popularity;
            }
            total = total.wrapping_add(party.popularity);
        }
        if total == 0 {
            return Ok(0);
        }
        Ok(
            if (matching as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
                matching.wrapping_mul(ONE).wrapping_div(total)
            } else {
                0xffff_ffff
            },
        )
    }

    fn scripted(&self, template: &Template, item: &Item, scopes: Scopes) -> Eval {
        let name = String::from_utf8_lossy(&template.name).into_owned();
        let desired = if item.value.items().is_some() {
            false
        } else {
            match trigger_boolean(item) {
                Ok(desired) => desired,
                Err(why) => {
                    return Eval::unknown(
                        why.context(|why| format!("scripted trigger {name}: {why}")),
                    );
                }
            }
        };
        let checked = if scopes.this == Scope::State(NULL_STATE) {
            Eval::False
        } else {
            let body = match &template.body {
                Ok(body) => body,
                Err(why) => return Eval::unknown(format!("scripted trigger {name}: {why}")),
            };
            if let Some(why) = cycle(&self.stack.borrow(), template) {
                return Eval::unknown(why);
            }
            self.stack.borrow_mut().push(frame(template));
            let checked = self.evaluate(body, scopes);
            self.stack.borrow_mut().pop();
            checked
        };
        match checked {
            Eval::Unknown(reasons) => Eval::Unknown(
                reasons
                    .into_iter()
                    .map(|why| why.context(|why| format!("scripted trigger {name}: {why}")))
                    .collect(),
            ),
            value if desired => value,
            value => value.negate(),
        }
    }

    pub fn country_operand(&self, item: &Item, scopes: Scopes) -> Result<Option<u32>, Failure> {
        let shown = String::from_utf8_lossy(item.value.scalar().unwrap_or_default()).into_owned();
        match scope_selector(item) {
            Some(Selected::Keyword(selector)) => {
                return self.selected_country(selector, &shown, scopes);
            }
            Some(Selected::EventTarget(name)) => {
                return match self
                    .saved_scope(name, Reader::CountrySelector, scopes)?
                    .map(|s| s.this)
                {
                    Some(Scope::Country(slot) | Scope::CountryLeader { country: slot, .. }) => {
                        Ok(Some(slot))
                    }
                    _ => Ok(None),
                };
            }
            None => {}
        }
        let recorded = self.world.recorded();
        if let Ok(tag) = item.value.c_str()
            && recorded.is_country(tag)
        {
            return self.resolve_tag(tag, scopes);
        }
        if let Some(gap) = self.world.table_gap("Countries") {
            return Err(
                gap.context(|gap| format!("whether {shown} is a country tag is unknown: {gap}"))
            );
        }
        self.scoped_country(item.value.scalar().unwrap_or_default(), scopes)
    }

    fn selected_country(
        &self,
        selector: Selector,
        name: &str,
        scopes: Scopes,
    ) -> Result<Option<u32>, Failure> {
        if scopes.cursor.is_some() {
            let keyword = match selector {
                Selector::This => Some(ScopeKeyword::This),
                Selector::Root => Some(ScopeKeyword::Root),
                Selector::Prev => Some(ScopeKeyword::Prev),
                Selector::From => Some(ScopeKeyword::From),
                _ => None,
            };
            if let Some(keyword) = keyword {
                let Some(entered) = self.enter_scope(&Parsed::Keyword(keyword), scopes)? else {
                    return Ok(None);
                };
                return match entered.this {
                    Scope::Country(slot) | Scope::CountryLeader { country: slot, .. } => {
                        Ok(Some(slot))
                    }
                    Scope::State(id) if id != NULL_STATE => {
                        self.core()?;
                        Ok(self
                            .world
                            .state_row(id)
                            .and_then(|(owner, _)| self.world.slot(owner)))
                    }
                    Scope::Combat => self.combat_country().map(Some),
                    _ => Ok(None),
                };
            }
        }
        let here = match selector {
            Selector::This => Some(scopes.this),
            Selector::Root => scopes.link(ScopeKeyword::Root)?,
            Selector::Prev => scopes.link(ScopeKeyword::Prev)?,
            Selector::From => scopes.link(ScopeKeyword::From)?,
            Selector::StateOwner | Selector::StateController | Selector::StateOccupier => {
                return self.state_country(selector, name, scopes.this);
            }
            Selector::None => return Ok(None),
        };
        let Some(here) = here else {
            return Ok(None);
        };
        match here {
            Scope::Combat => self.combat_country().map(Some),
            Scope::Country(slot) | Scope::CountryLeader { country: slot, .. } => Ok(Some(slot)),
            Scope::Character(_)
            | Scope::Leader(_)
            | Scope::IndustrialOrganisation(_)
            | Scope::Project { .. }
            | Scope::Raid(_)
            | Scope::Unit(_) => Ok(None),
            Scope::State(NULL_STATE) => Ok(None),
            Scope::State(id) => {
                self.core()?;
                let Some((owner, _)) = self.world.state_row(id) else {
                    return Ok(None);
                };
                match owner {
                    0 => Ok(None),
                    link => self.world.slot(link).map(Some).ok_or_else(|| -> Failure {
                        format!(
                            "the operand is the scope selector {name}, and the owner link {link} of state {id} has no country links row"
                        ).into()
                    }),
                }
            }
            Scope::Province(province) => Err(format!(
                "the operand is the scope selector {name} in province {province}'s scope, which is not reproduced"
            ).into()),
        }
    }

    fn state_country(
        &self,
        selector: Selector,
        name: &str,
        this: Scope,
    ) -> Result<Option<u32>, Failure> {
        let state = match this {
            Scope::State(NULL_STATE) => return Ok(None),
            Scope::State(id) => id,
            Scope::Province(province) => {
                return Err(format!(
                    "the operand is the scope selector {name} in province {province}'s scope, which is not reproduced"
                ).into());
            }
            _ => return Ok(None),
        };
        if selector == Selector::StateOccupier {
            return Err(format!(
                "the operand is the scope selector {name}: the occupier of state {state} is not read"
            ).into());
        }
        self.core()?;
        let Some((owner, controller)) = self.world.state_row(state) else {
            return Ok(None);
        };
        let link = if selector == Selector::StateOwner {
            owner
        } else {
            controller
        };
        match link {
            0 => Ok(None),
            link => self.world.slot(link).map(Some).ok_or_else(|| -> Failure {
                format!(
                    "the operand is the scope selector {name}, and state {state}'s link {link} has no country links row"
                ).into()
            }),
        }
    }

    fn tag(&self, item: &Item, scopes: Scopes) -> Result<Eval, Failure> {
        let fail = |why: Failure| {
            why.context(|why| {
                format!(
                    "trigger {} {} {}: {why}",
                    key_text(item),
                    String::from_utf8_lossy(item.operator.text()),
                    shown(&item.value)
                )
            })
        };
        let Some(tagged) = self.country_operand(item, scopes).map_err(fail)? else {
            return Ok(Eval::False);
        };
        let here = match scopes.this {
            scope @ (Scope::Character(_) | Scope::Leader(_) | Scope::CountryLeader { .. }) => {
                let character = self.character_of(scope).map_err(fail)?;
                return self
                    .world
                    .character_tag(character, tagged)
                    .map(Eval::from_bool)
                    .map_err(fail);
            }
            other => Some(self.country_here(other).map_err(fail)?),
        };
        Ok(Eval::from_bool(here == Some(tagged)))
    }

    fn canonical(&self, raw: u32) -> Result<u32, Failure> {
        self.world.canonical(raw)
    }

    fn trait_names(&self) -> Result<TraitNames<'_>, Failure> {
        self.world.trait_names()
    }

    fn character_of(&self, scope: Scope) -> Result<u32, Failure> {
        match scope {
            Scope::Character(id) => Ok(id),
            Scope::Leader(reference)
            | Scope::CountryLeader {
                leader: reference, ..
            } => self
                .world
                .unit_leader(reference)?
                .map(|row| row.character_id)
                .ok_or_else(|| -> Failure {
                    format!("unit leader {reference:#x} is not recorded").into()
                }),
            other => Err(format!("{} holds no character", self.describe(other)).into()),
        }
    }

    fn character_trigger(
        &self,
        name: &str,
        item: &Item,
        scopes: Scopes,
    ) -> Result<Option<Eval>, Failure> {
        let character = self.character_of(scopes.this)?;
        let bytes = item.value.scalar().unwrap_or_default();
        Ok(Some(match name {
            "has_character_flag" => {
                self.flag_trigger(item, CHARACTER_FLAGS, Some(character), scopes)?
            }
            "is_country_leader" => {
                let expected = item
                    .value
                    .scalar()
                    .and_then(content_reader_parser::parser::read_bool)
                    .ok_or("is_country_leader needs yes or no")?;
                if character == 0 || self.world.character_retired(character)? {
                    Eval::False
                } else {
                    let row = self.world.character(character)?.ok_or_else(|| -> Failure {
                        format!("character {character} is not recorded").into()
                    })?;
                    let country = self.canonical(row.country)?;
                    if self.world.ruling_party(country)?.is_none() {
                        Eval::False
                    } else {
                        let parties = self.world.party_leaders(country)?;
                        let ruling = parties
                            .iter()
                            .find(|party| party.ruling)
                            .and_then(|party| party.leaders.first());
                        Eval::from_bool(ruling.is_some_and(|leader| {
                            (leader.character == Some(character)) == expected
                        }))
                    }
                }
            }
            "skill" => {
                let skill = self.world.character_skill(character)?;
                let required = self.required(bytes, item.quoted(), false, scopes)?;
                Eval::from_bool(compare(
                    i64::from(skill) * ONE,
                    required,
                    item.operator,
                    false,
                ))
            }
            "has_ideology" => {
                if character == 0 || self.world.character_retired(character)? {
                    Eval::False
                } else {
                    let counts = self
                        .world
                        .triggers
                        .map_err(Failure::from)?
                        .counts
                        .as_ref()
                        .ok_or("has_ideology content ideology database is unavailable")?;
                    if !counts.is_ideology_type(bytes)? {
                        Eval::False
                    } else {
                        let token = self
                            .world
                            .token_of(bytes)?
                            .ok_or("has_ideology content token is absent from lexical tokens")?;
                        let mut found = false;
                        for role in counted::<tables::CountryLeaderRoles>(
                            self.world,
                            replay_data::rows::mask::<replay_data::rows::CountryLeaderRoleRow>(
                                &["current_ideology"],
                            ),
                        )?
                        .iter()
                        .filter(|role| role.character == character)
                        {
                            let ideology = *super::refusal::optional(
                                &role.current_ideology,
                                replay_data::rows::TableId::CountryLeaderRoles,
                                "current_ideology",
                                Some(super::Store::Character(character)),
                            )?;
                            if ideology == Some(token) {
                                found = true;
                                break;
                            }
                        }
                        Eval::from_bool(found)
                    }
                }
            }
            "has_trait" => {
                self.trait_names()?;
                Eval::from_bool(self.world.character_has_trait(character, bytes)?)
            }
            "has_scientist_level" => self.has_scientist_level(item, character, scopes)?,
            "is_active_scientist" => self.is_active_scientist(item, character)?,
            _ => return Ok(None),
        }))
    }

    pub fn character_traits(
        &self,
        character: u32,
        operand: &[u8],
        assigned: impl FnOnce() -> Result<Option<Vec<i32>>, Failure>,
    ) -> Result<bool, Failure> {
        self.world.read_role_traits();
        self.world.character_tables().character_traits(
            character,
            operand,
            &self.trait_names()?,
            assigned,
        )
    }
}

fn counted<'a, T: replay_data::rows::TableSchema>(
    world: &World<'a>,
    mask: u64,
) -> Result<Vec<std::borrow::Cow<'a, T::Row>>, Failure> {
    table::<T>(world.tables(), mask)
        .map_err(|error| {
            error.context(|error| format!("{} is not complete this hour: {error}", T::ID.words()))
        })?
        .iter()
        .map(|row| row.map_err(|why| super::refusal::input_error(T::ID, why)))
        .collect()
}
