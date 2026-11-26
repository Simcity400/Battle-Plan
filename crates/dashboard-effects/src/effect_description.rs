use content_reader_key_names::key_name_products::KeyNamesExt as _;


use std::{
    cell::{OnceCell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    sync::Arc,
};

use crate::triggers::{NULL_STATE, Scope, Selected, Selector, scope_selector};
use crate::{
    scripted_text::{EffectDescriber, EventObject, EventScopes, ExpansionScopes, Link, ScriptedContent, ScriptedText},
    triggers::{Evaluator, Scopes, World},
};
use content_reader_parser::parser::Item;
use content_reader_script::scope::{Keyword, Scope as Parsed};
use content_reader_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::rows::{
    GlobalSavedEventTargetRow, PoliticalPartyRow, RowCodec, StateResistanceRow, StateRow,
    TableSchema, mask, tables,
};
use engine_executable::executable_tables::effect_identity;
use replay_frame::frame::{HourFrame, Rowset};
use std::borrow::Cow;

const STATE_COUNTRIES: u64 = mask::<StateRow>(&["owner", "controller"]);
const PARTY_SELECTION: u64 = mask::<PoliticalPartyRow>(&[
    "token",
    "tag",
    "name_default",
    "name",
    "long_name",
]);
const RESISTANCE_COUNTRY: u64 = mask::<StateResistanceRow>(&["country"]);
pub(crate) const DLC_MASK: u64 = mask::<replay_data::rows::SimulationFeatureRow>(&["dlc_mask"]);

pub(crate) fn built(why: std::io::Error) -> String {
    why.to_string()
}

pub(crate) fn each<'r, R: RowCodec + Clone + 'r>(
    rows: Rowset<'r, R>,
) -> impl DoubleEndedIterator<Item = Result<Cow<'r, R>, String>> + 'r {
    rows.iter().map(|row| row.map_err(built))
}

pub(crate) fn keyed<'r, R: RowCodec + Clone>(
    rows: &Rowset<'r, R>,
    key: u128,
) -> Result<Option<Cow<'r, R>>, String> {
    rows.get(key).map_err(built)
}

pub(crate) fn find<'r, R: RowCodec + Clone + 'r>(
    rows: Rowset<'r, R>,
    mut keep: impl FnMut(&R) -> bool,
) -> Result<Option<Cow<'r, R>>, String> {
    for row in each(rows) {
        let row = row?;
        if keep(&row) {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

mod ability;
mod characters;
mod concise;
pub use ability::ability_description;
mod diplomacy;
mod doctrine_sources;
pub use doctrine_sources::{doctrine_benefits, doctrine_tactic_sources};
pub use static_lists::scaled_static_modifiers;
mod dispatch;
mod empty;
mod entity_links;
pub use entity_links::{EntityKind, Span, spans};
pub(crate) const DEPENDENT_OPTION_TOOLTIP_TOKEN: u32 = 0x3FF4;
mod events;
pub use events::decision_name_key_at;
mod lists;
mod modifiers;
mod numeric;
mod occupation;
pub use occupation::occupation_modifier_description;
mod politics;
mod scope_names;
mod scopes;
mod states;
mod static_lists;
mod technology;
#[cfg(test)]
mod tests;
mod units;
mod variables;

const OPERAND_TABLES: &[replay_data::rows::TableId] = &[
    replay_data::rows::TableId::AirWings,
    replay_data::rows::TableId::AvailableAdvisors,
    replay_data::rows::TableId::AvailableEquipmentVariants,
    replay_data::rows::TableId::CapturedOperativeReferences,
    replay_data::rows::TableId::Clock,
    replay_data::rows::TableId::DecisionRandomItems,
    replay_data::rows::TableId::Decisions,
    replay_data::rows::TableId::OccupationSettings,
    replay_data::rows::TableId::OperativeMemberships,
    replay_data::rows::TableId::PowerBalanceRanges,
    replay_data::rows::TableId::ProjectPrograms,
    replay_data::rows::TableId::ProvinceUnitOrder,
    replay_data::rows::TableId::RaidFacilities,
    replay_data::rows::TableId::RaidTargets,
    replay_data::rows::TableId::RailwayGunCounterInputs,
    replay_data::rows::TableId::RelationLinks,
    replay_data::rows::TableId::TaskForces,
    replay_data::rows::TableId::TimedIdeas,
    replay_data::rows::TableId::TrainInventory,
    replay_data::rows::TableId::VariantDefinitions,
    replay_data::rows::TableId::VariantUpgrades,
];

pub fn recorded_tables() -> impl Iterator<Item = replay_data::rows::TableId> {
    crate::triggers::RECORDED_TABLES
        .iter()
        .chain(OPERAND_TABLES)
        .copied()
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ModifierListOptions<'a> {
    pub(super) precision: i32,
    pub(super) outer_colour: u8,
    pub(super) category_mask: u32,
    pub(super) excluded_category_mask: u32,
    pub(super) separator: &'a str,
    pub(super) scale: i64,
    pub(super) force_plus: bool,
    pub(super) invert: bool,
}

impl Default for ModifierListOptions<'_> {
    fn default() -> Self {
        Self {
            precision: 2,
            outer_colour: 0,
            category_mask: 0xff_ffff,
            excluded_category_mask: 0,
            separator: "\n",
            scale: 100_000,
            force_plus: false,
            invert: false,
        }
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct Description {
    pub text: String,
    pub refusals: BTreeMap<String, usize>,
}

pub type EventLookup<'a> =
    dyn Fn(&[u8]) -> Result<Arc<content_reader_events::events::Event>, String> + 'a;

#[derive(Default, Clone, Copy)]
pub struct Content<'a> {
    pub doctrines: Option<&'a content_reader_military::doctrines::Database>,
    pub decisions: Option<&'a content_reader_politics::decisions::Database>,
    pub ideas: Option<&'a content_reader_politics::ideas::IdeaContent>,
    pub static_modifiers: Option<&'a content_reader_politics::static_modifiers::Descriptions>,
    pub dynamic_modifiers: Option<&'a content_reader_politics::dynamic_modifiers::Database>,
    pub opinion_modifiers: Option<&'a content_reader_politics::opinion_modifiers::Database>,
    pub power_balances: Option<&'a content_reader_politics::power_balance::Database>,
    pub name_lists: Option<&'a content_reader_politics::names::NameLists>,
    pub events: Option<&'a EventLookup<'a>>,
    pub key_names: Option<&'a content_reader_script::key_names::KeyNames>,
    pub scripted_effects: Option<&'a content_reader_key_names::scripted_effects::ScriptedEffects>,
    pub industrial_organisations: Option<&'a content_reader_military::industrial_organisations::Database>,
    pub collections: Option<&'a content_reader_politics::collections::Database>,
    pub country_scorers: Option<&'a content_reader_politics::country_scorers::Database>,
    pub buildings: Option<&'a content_reader_key_names::buildings::Buildings>,
    pub resources: Option<&'a [content_reader_key_names::resources::Resource]>,
    pub max_railway_level: Option<i32>,
    pub equipment: Option<&'a content_reader_key_names::equipment::Database>,
    pub autonomy_states: Option<&'a content_reader_politics::autonomy::AutonomyStates>,
    pub state_categories: Option<&'a content_reader_state_categories::state_categories::Database>,
    pub division_names: Option<&'a content_reader_division_names::division_names::Database>,
    pub wargoals: Option<&'a content_reader_wargoals::wargoals::Database>,
    pub gaps: Option<&'a BTreeMap<&'static str, String>>,
}

pub struct CachedContent {
    politics: politics::PoliticsContent,
    characters: characters::CharacterContent,
    doctrines: Option<content_reader_military::doctrines::Database>,
    decisions: Option<content_reader_politics::decisions::Database>,
    ideas: Option<content_reader_politics::ideas::IdeaContent>,
    static_modifiers: Arc<content_reader_politics::static_modifiers::Descriptions>,
    dynamic_modifiers: Option<content_reader_politics::dynamic_modifiers::Database>,
    opinion_modifiers: Option<content_reader_politics::opinion_modifiers::Database>,
    power_balances: Option<content_reader_politics::power_balance::Database>,
    pub advisor_templates: Option<content_reader_key_names::characters::Characters>,
    name_lists: Option<content_reader_politics::names::NameLists>,
    key_names: Option<Arc<content_reader_key_names::key_name_products::KeyNameProducts>>,
    scripted_effects: Option<content_reader_key_names::scripted_effects::ScriptedEffects>,
    industrial_organisations: Option<content_reader_military::industrial_organisations::Database>,
    collections: Option<content_reader_politics::collections::Database>,
    country_scorers: Option<content_reader_politics::country_scorers::Database>,
    buildings: Option<content_reader_key_names::buildings::Buildings>,
    resources: Option<Vec<content_reader_key_names::resources::Resource>>,
    max_railway_level: Option<i32>,
    raids:
        content_reader_layers::content::diagnostics::Once<Result<content_reader_raids::raids::Catalog, String>>,
    equipment: Option<content_reader_key_names::equipment::Database>,
    autonomy_states: Option<content_reader_politics::autonomy::AutonomyStates>,
    state_categories: Option<content_reader_state_categories::state_categories::Database>,
    division_names: Option<content_reader_division_names::division_names::Database>,
    wargoals: Option<content_reader_wargoals::wargoals::Database>,
    gaps: BTreeMap<&'static str, String>,
    pub notes: Vec<String>,
}

fn alongside<'s, T: Send + 's>(
    threads: &'s std::thread::Scope<'s, '_>,
    read: impl FnOnce() -> T + Send + 's,
) -> std::thread::ScopedJoinHandle<'s, (T, crate::catalogue::OwnRefusals)> {
    content_reader_layers::parallel::spawn(threads, move || crate::catalogue::capture(read))
}

fn joined<T>(read: std::thread::ScopedJoinHandle<'_, (T, crate::catalogue::OwnRefusals)>) -> T {
    let (value, noted) = content_reader_layers::parallel::join(read);
    crate::catalogue::credit(&noted);
    value
}

impl CachedContent {
    pub(crate) fn read(
        layers: &content_reader_layers::ResolvedLayers,
        environment: &crate::scripted_text::Environment,
        static_modifiers: Arc<content_reader_politics::static_modifiers::Descriptions>,
    ) -> std::io::Result<Self> {
        std::thread::scope(|threads| Self::read_with(threads, layers, environment, static_modifiers))
    }

    fn read_with<'s>(
        threads: &'s std::thread::Scope<'s, '_>,
        layers: &'s content_reader_layers::ResolvedLayers,
        environment: &crate::scripted_text::Environment,
        static_modifiers: Arc<content_reader_politics::static_modifiers::Descriptions>,
    ) -> std::io::Result<Self> {
        fn keep<T>(
            read: std::io::Result<T>,
            key: &'static str,
            gaps: &mut BTreeMap<&'static str, String>,
        ) -> Option<T> {
            read.map_err(|why| {
                gaps.insert(key, why.to_string());
            })
            .ok()
        }
        let mut gaps = BTreeMap::new();
        let doctrines = alongside(threads, || content_reader_military::doctrines::read(layers));
        let decisions = alongside(threads, || content_reader_politics::decisions::read(layers));
        let division_names = alongside(threads, || {
            content_reader_division_names::division_names::read_directories::<String>(
                layers,
                &content_reader_division_names::division_names::DIRECTORIES,
            )
        });
        let politics = alongside(threads, || politics::PoliticsContent::read(layers));
        let dynamic_modifiers =
            alongside(threads, || content_reader_politics::dynamic_modifiers::read(layers));
        let power_balances = alongside(threads, || {
            content_reader_politics::power_balance::read_descriptions(layers)
        });
        let equipment = alongside(threads, || {
            content_reader_key_names::equipment::Database::load_complete(layers)
        });
        let autonomy_states = alongside(threads, || content_reader_politics::autonomy::read(layers));
        let state_categories = alongside(threads, || {
            content_reader_state_categories::state_categories::read(layers)
        });
        let wargoals = alongside(threads, || content_reader_wargoals::wargoals::read(layers));
        let name_lists = alongside(threads, || content_reader_politics::names::read(layers));
        let named = alongside(threads, || {
            let key_names = layers.key_names();
            let read = key_names.as_ref().ok().map(|names| {
                (
                    content_reader_key_names::scripted_effects::read(layers, names),
                    content_reader_military::industrial_organisations::read(layers, names),
                    content_reader_politics::collections::read(layers, names),
                    content_reader_politics::country_scorers::read(layers, names),
                )
            });
            (key_names, read)
        });
        let buildings = alongside(threads, || content_reader_key_names::buildings::read(layers));
        let resources = alongside(threads, || content_reader_key_names::resources::read(layers));
        let defines = environment.defines();
        let opinion_modifiers = keep(
            content_reader_politics::opinion_modifiers::read(layers, defines),
            "opinion modifiers",
            &mut gaps,
        );
        let read_ideas = || {
            let officers = defines.strings("NCharacter", "OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU")?;
            content_reader_politics::ideas::read(layers, &officers)
        };
        let ideas = keep(read_ideas(), "ideas", &mut gaps);
        let maximum = defines.int32("NSupply", "MAX_RAILWAY_LEVEL");
        let max_railway_level = keep(maximum, "MAX_RAILWAY_LEVEL", &mut gaps);
        let _ = environment.ideologies();
        let _ = environment.terrain();
        let _ = environment.leader_traits();
        let environment_notes = environment.catalogue_notes();
        let (ideas, advisor_templates, idea_warnings) = match ideas {
            Some((ideas, templates, warnings)) => (Some(ideas), Some(templates), warnings),
            None => (None, None, Vec::new()),
        };
        let characters = characters::CharacterContent::read(
            threads,
            layers,
            environment,
            advisor_templates
                .as_ref()
                .ok_or_else(|| gaps["ideas"].clone()),
        );
        let mut doctrines = keep(joined(doctrines), "doctrines", &mut gaps);
        if let Some(doctrines) = &mut doctrines {
            doctrines.tactic_sources.default_reward_mastery =
                defines.fixed5("NDoctrines", "DEFAULT_REWARD_MASTERY").ok();
        }
        let dynamic_modifiers = keep(joined(dynamic_modifiers), "dynamic modifiers", &mut gaps);
        let power_balances = keep(joined(power_balances), "power balances", &mut gaps);
        let equipment = keep(joined(equipment), "equipment", &mut gaps);
        let autonomy_states = keep(joined(autonomy_states), "autonomy states", &mut gaps);
        let state_categories = keep(joined(state_categories), "state categories", &mut gaps);
        let wargoals = keep(joined(wargoals), "Wargoal definitions", &mut gaps);
        let decisions = keep(joined(decisions), "decisions", &mut gaps);
        let division_names = keep(joined(division_names), "division name groups", &mut gaps);
        let mut notes = doctrines
            .as_ref()
            .map(|db| db.notes.clone())
            .unwrap_or_default();
        if let Some(database) = &dynamic_modifiers {
            notes.extend(database.notes.iter().cloned());
        }
        if let Some(database) = &opinion_modifiers {
            notes.extend(database.notes.iter().cloned());
        }
        if let Some(database) = &power_balances {
            notes.extend(database.notes.iter().cloned());
        }
        notes.extend(environment_notes);
        notes.extend(
            decisions
                .as_ref()
                .map(|db| db.notes.clone())
                .unwrap_or_default(),
        );
        if let Some(database) = &equipment {
            notes.extend(database.misreads.iter().cloned());
        }
        if let Some(database) = &state_categories {
            notes.extend(database.notes.iter().cloned());
        }
        if let Some(database) = &wargoals {
            notes.extend(database.notes.iter().cloned());
        }
        let division_names = division_names.map(|(database, warnings)| {
            notes.extend(warnings);
            database
        });
        let autonomy_states = autonomy_states.map(|(states, warnings)| {
            notes.extend(warnings.into_iter().map(|(_, message)| message));
            states
        });
        notes.extend(idea_warnings.into_iter().map(|(_, why)| why));
        let name_lists = keep(joined(name_lists), "name lists", &mut gaps);
        let (key_names, named) = joined(named);
        let key_names = keep(key_names, "effect key names", &mut gaps);
        let (scripted_effects, industrial_organisations, collections, country_scorers) = match named
        {
            Some((scripted, industrial, collections, scorers)) => (
                keep(scripted, "scripted effects", &mut gaps),
                keep(industrial, "industrial organisation visibility", &mut gaps),
                keep(collections, "collections", &mut gaps),
                keep(scorers, "country scorers", &mut gaps),
            ),
            None => (None, None, None, None),
        };
        if let Some(database) = &industrial_organisations {
            notes.extend(database.notes.clone());
        }
        if let Some(database) = &collections {
            notes.extend(database.notes.clone());
        }
        if let Some(database) = &country_scorers {
            notes.extend(database.notes.clone());
        }
        let buildings = keep(joined(buildings), "buildings", &mut gaps);
        let resources =
            keep(joined(resources), "resources", &mut gaps).and_then(|(resources, warnings)| {
                let refused = resources.gap().is_some()
                    || warnings.iter().any(|(kind, _)| {
                        matches!(
                            kind,
                            content_reader_parser::DiagnosticKind::ParseFailure
                                | content_reader_parser::DiagnosticKind::Unsettled
                                | content_reader_parser::DiagnosticKind::Unsupported
                                | content_reader_parser::DiagnosticKind::Refused
                        )
                    });
                if refused {
                    gaps.insert(
                        "resources",
                        warnings
                            .iter()
                            .map(|(_, why)| why.as_str())
                            .collect::<Vec<_>>()
                            .join("; "),
                    );
                }
                notes.extend(warnings.into_iter().map(|(_, why)| why));
                (!refused).then_some(resources.declarations)
            });
        let politics = joined(politics);
        notes.extend(politics.notes.iter().cloned());
        notes.extend(characters.notes.iter().cloned());
        Ok(Self {
            characters,
            politics,
            doctrines,
            decisions,
            ideas,
            static_modifiers,
            dynamic_modifiers,
            opinion_modifiers,
            power_balances,
            advisor_templates,
            name_lists,
            key_names,
            scripted_effects,
            industrial_organisations,
            collections,
            country_scorers,
            buildings,
            resources,
            max_railway_level,
            raids: content_reader_layers::content::diagnostics::Once::new(),
            equipment,
            autonomy_states,
            state_categories,
            division_names,
            wargoals,
            gaps,
            notes,
        })
    }

    pub fn technologies(&self) -> Result<&content_reader_military::technology::Catalog, String> {
        self.doctrines
            .as_ref()
            .ok_or_else(|| {
                self.gaps
                    .get("doctrines")
                    .cloned()
                    .unwrap_or_else(|| "doctrines were not read".into())
            })?
            .technologies
            .as_ref()
            .ok_or_else(|| "the technology catalogue was not read".into())
    }

    pub fn view<'a>(&'a self, events: Option<&'a EventLookup<'a>>) -> Content<'a> {
        Content {
            doctrines: self.doctrines.as_ref(),
            decisions: self.decisions.as_ref(),
            ideas: self.ideas.as_ref(),
            static_modifiers: Some(&self.static_modifiers),
            dynamic_modifiers: self.dynamic_modifiers.as_ref(),
            opinion_modifiers: self.opinion_modifiers.as_ref(),
            power_balances: self.power_balances.as_ref(),
            name_lists: self.name_lists.as_ref(),
            key_names: self.key_names.as_deref().map(|product| &product.names),
            scripted_effects: self.scripted_effects.as_ref(),
            industrial_organisations: self.industrial_organisations.as_ref(),
            collections: self.collections.as_ref(),
            country_scorers: self.country_scorers.as_ref(),
            buildings: self.buildings.as_ref(),
            resources: self.resources.as_deref(),
            max_railway_level: self.max_railway_level,
            equipment: self.equipment.as_ref(),
            autonomy_states: self.autonomy_states.as_ref(),
            state_categories: self.state_categories.as_ref(),
            division_names: self.division_names.as_ref(),
            wargoals: self.wargoals.as_ref(),
            gaps: Some(&self.gaps),
            events,
        }
    }
}

#[derive(Default)]
struct Describer {
    cache: content_reader_layers::content::diagnostics::Once<Result<CachedContent, String>>,
}

pub fn effect_describer() -> Box<dyn EffectDescriber> {
    Box::new(Describer::default())
}

impl EffectDescriber for Describer {
    fn describe<'f>(
        &self,
        text: &ScriptedText<'f>,
        source: &str,
        items: &[Item],
        expansion: &ExpansionScopes<'_, 'f>,
        graph: &mut EventScopes,
        refused: &mut Vec<String>,
    ) -> Result<String, String> {
        let content = text.content().effects()?;
        let mut context = Context::new(
            text,
            text.localisation(),
            text.viewing_country(),
            source,
            content.view(None),
        )
        .with_expansion_scopes(expansion);
        let session = expansion.variable_session();
        if let Some(session) = &session {
            context = context.with_variable_session(session);
        }
        let description = describe(items, &context, graph);
        if session.is_some() {
            expansion.replace_variable_session(context.variable_session());
        }
        for (reason, count) in description.refusals {
            refused.extend(std::iter::repeat_n(reason, count));
        }
        Ok(description.text)
    }

    fn prepare_static_modifiers(&self, content: &ScriptedContent) -> Result<(), String> {
        content.effects().map(|_| ())
    }
}

pub trait ScriptedEffects {
    fn effects(&self) -> Result<&CachedContent, String>;

    #[cfg(any(test, feature = "test-support"))]
    fn effects_read(&self) -> bool;
}

impl ScriptedEffects for ScriptedContent {
    fn effects(&self) -> Result<&CachedContent, String> {
        let describer = self
            .effect_describer::<Describer>()
            .ok_or("effect description content: effect describer is unavailable")?;
        describer.cache
            .get_or_init(|| {
                CachedContent::read(
                    &self.layers.layers,
                    self.environment()?,
                    Arc::clone(self.static_modifiers()),
                )
                .map_err(|why| format!("effect description content: {why}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    #[cfg(any(test, feature = "test-support"))]
    fn effects_read(&self) -> bool {
        self.effect_describer::<Describer>()
            .is_some_and(|describer| describer.cache.get().is_some())
    }
}

pub struct Context<'c, 'f> {
    text: &'c ScriptedText<'f>,
    localisation: &'c Localisation,
    session: RefCell<Option<crate::triggers::VariableSession>>,
    nested_session: bool,
    expansion: Option<&'c ExpansionScopes<'c, 'f>>,
    perspective: u32,
    source: &'c str,
    content: Content<'c>,
    entity_links: bool,
}

impl<'c, 'f> Context<'c, 'f> {
    pub fn new(
        text: &'c ScriptedText<'f>,
        localisation: &'c Localisation,
        perspective: u32,
        source: &'c str,
        content: Content<'c>,
    ) -> Self {
        Self {
            text,
            localisation,
            session: RefCell::new(None),
            nested_session: false,
            expansion: None,
            perspective,
            source,
            content,
            entity_links: false,
        }
    }

    pub fn with_entity_links(mut self) -> Self {
        self.entity_links = true;
        self
    }

    fn world(&self) -> Result<&'c World<'f>, String> {
        match self.expansion.and_then(ExpansionScopes::world) {
            Some(world) => Ok(world),
            None => self.text.trigger_world(),
        }
    }

    pub(crate) fn with_variable_session(
        mut self,
        session: &crate::triggers::VariableSession,
    ) -> Self {
        self.session.replace(Some(session.clone()));
        self.nested_session = true;
        self
    }

    pub(crate) fn with_expansion_scopes(mut self, scopes: &'c ExpansionScopes<'c, 'f>) -> Self {
        self.expansion = Some(scopes);
        self
    }

    pub(crate) fn variable_session(&self) -> Option<crate::triggers::VariableSession> {
        self.session.borrow().clone()
    }
}

pub fn describe(
    items: &[Item],
    context: &Context<'_, '_>,
    scopes: &mut EventScopes,
) -> Description {
    if !context.nested_session {
        context.session.replace(None);
    }
    let graph = Rc::new(RefCell::new(scopes.clone()));
    let mut out = Renderer {
        context,
        graph,
        evaluator: OnceCell::new(),
        out: Description::default(),
        scripted_stack: Vec::new(),
        concise: None,
    };
    let projected = crate::triggers::event_scopes::project(&out.graph.borrow(), 0);
    match projected {
        Ok(scopes) => {
            out.out.text = out.group(items, scopes, 0);
            if let Some(Ok(evaluator)) = out.evaluator.get() {
                let result = evaluator.end_description(scopes);
                context.session.replace(Some(evaluator.variable_session()));
                if let Err(why) = result {
                    out.refuse("description cleanup", why.to_string());
                }
            } else if !context.nested_session {
                let result = crate::triggers::event_scopes::cleanup_description(
                    &mut out.graph.borrow_mut(),
                    0,
                );
                if let Err(why) = result {
                    out.refuse("description cleanup", why.to_string());
                }
            }
        }
        Err(why) => out.refuse("event scope", why.to_string()),
    }
    *scopes = out.graph.borrow().clone();
    out.out
}

struct Renderer<'r, 'c, 'f> {
    context: &'r Context<'c, 'f>,
    graph: Rc<RefCell<EventScopes>>,
    evaluator: OnceCell<Result<Evaluator<'c, 'f>, String>>,
    out: Description,
    scripted_stack: Vec<(Vec<u8>, Scope)>,
    concise: Option<(u32, u32)>,
}

pub enum MioText<'a> {
    Bonuses(&'a [content_reader_military::mio_traits::Bonus]),
    Modifiers(&'a content_reader_script::modifiers::Modifiers),
    TraitModifiers(&'a content_reader_military::mio_traits::TraitName),
}

pub fn mio_text(
    text: MioText<'_>,
    context: &Context<'_, '_>,
    scopes: &EventScopes,
) -> Vec<Result<String, String>> {
    context.session.replace(None);
    let graph = Rc::new(RefCell::new(scopes.clone()));
    let mut out = Renderer {
        context,
        graph,
        evaluator: OnceCell::new(),
        out: Description::default(),
        scripted_stack: Vec::new(),
        concise: None,
    };
    let projected = crate::triggers::event_scopes::project(&out.graph.borrow(), 0);
    match projected {
        Ok(projected) => out.char_mio_lines(text, projected),
        Err(why) => vec![Err(why.to_string())],
    }
}

pub fn trim_description(text: &str) -> &str {
    text.trim_matches(['\n', '\t', '\r', ' '])
}

pub(super) fn indent_nonempty_lines(text: &str, indent: usize) -> String {
    let prefix = " ".repeat(indent);
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        if line != "\n" {
            out.push_str(&prefix);
        }
        out.push_str(line);
    }
    out
}

pub(super) fn append_description(text: &mut String, child: &str) {
    if child.len() as i32 <= 1 {
        return;
    }
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(child);
}

fn field<'a>(items: &'a [Item], name: &str) -> Option<&'a Item> {
    items.iter().rev().find(|item| item.keyword_is(name))
}

pub(super) fn fixed_field(items: &[Item], name: &str, default: i64) -> i64 {
    items
        .iter()
        .filter(|item| item.keyword_is(name))
        .filter_map(|item| content_reader_parser::fixed5::script_fixed5(item.value.scalar()?))
        .next_back()
        .unwrap_or(default)
}

fn integer(text: &[u8]) -> i32 {
    match content_reader_parser::crt::scan_i32(text) {
        content_reader_parser::crt::Scanned::Stored(value) => value,
        _ => 0,
    }
}

fn without(items: &[Item], names: &[&str]) -> Vec<Item> {
    items
        .iter()
        .filter(|item| !names.iter().any(|name| item.keyword_is(name)))
        .cloned()
        .collect()
}

fn children(item: &Item) -> &[Item] {
    item.value.items().unwrap_or_default()
}
fn key(item: &Item) -> String {
    String::from_utf8_lossy(item.key.as_deref().unwrap_or_default()).into_owned()
}

fn selector_scope(selector: Selector) -> Option<Parsed> {
    Some(Parsed::Keyword(match selector {
        Selector::This => Keyword::This,
        Selector::Root => Keyword::Root,
        Selector::Prev => Keyword::Prev,
        Selector::From => Keyword::From,
        _ => return None,
    }))
}

enum EffectSavedTarget<'a> {
    Graph(EventObject),
    Recorded(Cow<'a, GlobalSavedEventTargetRow>),
}

fn scalar_operand(item: &Item) -> Result<&[u8], String> {
    item.value
        .scalar()
        .ok_or_else(|| format!("{} requires a scalar operand", key(item)))
}

impl<'r, 'c, 'f> Renderer<'r, 'c, 'f> {
    pub(super) fn tag(&self, country: u32) -> Result<&str, String> {
        self.context.text.country_tag(country)
    }

    pub(super) fn country_name(&self, country: u32, definite: bool) -> Result<String, String> {
        self.context.text.country_name(country, definite)
    }

    pub(super) fn current_country(&self, scopes: Scopes) -> Result<u32, String> {
        match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => Ok(country),
            _ => Err("current scope has no country".into()),
        }
    }

    pub(super) fn selected_party_name(
        &self,
        country: u32,
        ideology: &[u8],
    ) -> Result<String, String> {
        let parties = self.complete::<tables::PoliticalParties>(PARTY_SELECTION)?;
        let mut party = None;
        for row in parties.keyed(|key| {
            self.context.text.recorded().slot((key >> 32) as u32) == Some(country)
        }) {
            let row = row.map_err(built)?;
            if row.token.eq_ignore_ascii_case(ideology) {
                party = Some(row);
                break;
            }
        }
        let party = party.ok_or("political parties lacks the party selected by ideology")?;
        self.context
            .text
            .party_name(&party, self.tag(country)?, false)
    }

    pub(super) fn country_target(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<Option<u32>, String> {
        let item = if item.value.scalar().is_some() {
            item
        } else {
            let effect = item.keyword().map(|key| key.folded()).unwrap_or_default();
            if !content_reader_script::effect_keys::is_own_field(&effect, b"target")? {
                scalar_operand(item)?;
            }
            let Some(target) = self.country_target_field(item, "target")? else {
                return Ok(None);
            };
            target
        };
        let country = match scope_selector(item) {
            Some(Selected::Keyword(Selector::StateOwner)) => {
                self.state_country_selector(Keyword::Owner, scopes)?
            }
            Some(Selected::Keyword(Selector::StateController)) => {
                self.state_country_selector(Keyword::Controller, scopes)?
            }
            Some(Selected::Keyword(Selector::StateOccupier)) => {
                let state = match scopes.this {
                    Scope::State(state) => state,
                    Scope::Country(_) | Scope::CountryLeader { .. } => return Ok(None),
                    _ => return Err("occupied selector needs the event scope's state ID; this projected scope does not expose that field".into()),
                };
                if self.valid_state(state)?.is_none() {
                    return Ok(None);
                }
                let rows = self.complete::<tables::StateResistance>(RESISTANCE_COUNTRY)?;
                let raw = keyed(&rows, u128::from(state))?
                    .ok_or_else(|| format!("state resistance lacks country for state {state}"))?
                    .country;
                if raw == 0 {
                    None
                } else {
                    Some(
                        self.context
                            .text
                            .recorded()
                            .slot(raw)
                            .ok_or_else(|| format!("country links lacks occupied country {raw}"))?,
                    )
                }
            }
            Some(Selected::Keyword(selector)) => {
                if let Some(target) = selector_scope(selector) {
                    match self
                        .evaluator()?
                        .enter(&target, scopes)
                        .map_err(|why| why.to_string())?
                    {
                        Some(Scope::Country(country) | Scope::CountryLeader { country, .. }) => {
                            Some(country)
                        }
                        Some(Scope::State(id)) if id != NULL_STATE => {
                            let state = self.valid_state(id)?.ok_or_else(|| {
                                format!("invalid nonzero state {id}: country selection needs the missing state's owner")
                            })?;
                            if state.owner == 0 {
                                None
                            } else {
                                Some(self.context.text.recorded().slot(state.owner).ok_or_else(
                                    || {
                                        format!(
                                            "country links lacks state {id} owner {}",
                                            state.owner
                                        )
                                    },
                                )?)
                            }
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }
            Some(Selected::EventTarget(name)) => match self.effect_saved_target(name, scopes)? {
                Some(EffectSavedTarget::Graph(EventObject::Country(country))) => Some(country),
                Some(EffectSavedTarget::Recorded(row)) => self
                    .context
                    .text
                    .recorded()
                    .saved_country(row.country, content_reader_parser::parser::c_string(name))
                    .map_err(|why| why.to_string())?,
                _ => None,
            },
            None => self
                .evaluator()?
                .scoped_country(scalar_operand(item)?, scopes)
                .map_err(|why| why.to_string())?,
        };
        Ok(country.filter(|country| *country > 0))
    }

    pub(super) fn country_target_field<'a>(
        &self,
        item: &'a Item,
        name: &str,
    ) -> Result<Option<&'a Item>, String> {
        let mut fields = children(item).iter().filter(|field| field.keyword_is(name));
        if fields.clone().nth(1).is_some()
            && fields.clone().any(|field| {
                scope_selector(field).is_some()
                    || field.value.scalar().is_some_and(|value| value.contains(&b'.'))
                        && item.keyword().is_some_and(|key|
                            content_reader_script::effect_keys::reads_scope_chain(&key.folded(), name.as_bytes()))
            })
        {
            return Err(format!(
                "repeated {name} fields retain selector flags or appended scope links; auxiliary reader replay is unavailable"
            ));
        }
        Ok(fields.next_back())
    }

    fn state_country_selector(
        &self,
        keyword: Keyword,
        scopes: Scopes,
    ) -> Result<Option<u32>, String> {
        match scopes.this {
            Scope::Country(_) | Scope::CountryLeader { .. } | Scope::State(NULL_STATE) => Ok(None),
            Scope::State(_) => Ok(match self.evaluator()?.enter(&Parsed::Keyword(keyword), scopes).map_err(|why| why.to_string())? {
                Some(Scope::Country(country) | Scope::CountryLeader { country, .. }) => Some(country),
                _ => None,
            }),
            _ => Err("state owner/controller selector needs the event scope's state ID; this projected scope does not expose that field".into()),
        }
    }

    fn effect_saved_target(
        &self,
        name: &[u8],
        scopes: Scopes,
    ) -> Result<Option<EffectSavedTarget<'_>>, String> {
        let name = content_reader_parser::parser::c_string(name);
        if let Some(mut at) = scopes.cursor {
            let graph = self.graph.borrow();
            let mut steps = 0;
            loop {
                if steps == graph.scopes.len() {
                    return Err("effect saved-target FROM storage cycle has no terminal".into());
                }
                steps += 1;
                let scope = graph
                    .scopes
                    .get(at)
                    .ok_or("effect target scope is not given")?;
                let next = match scope.from {
                    Link::Itself => at,
                    Link::Scope(to) if to < graph.scopes.len() => to,
                    _ => {
                        return Err("effect saved-target FROM storage link is not known".into());
                    }
                };
                if next == at {
                    break;
                }
                at = next;
            }
            let lookup = |rows: &[(Vec<u8>, EventObject)]| {
                rows.iter()
                    .find(|(key, _)| key == name)
                    .map(|(_, object)| *object)
            };
            for target in [
                graph.tooltip_targets.get(&at).and_then(|rows| lookup(rows)),
                graph.saved_targets.get(&at).and_then(|rows| lookup(rows)),
                lookup(&graph.global_tooltip_targets),
            ]
            .into_iter()
            .flatten()
            {
                if name.is_empty() {
                    continue;
                }
                match target {
                    EventObject::Country(0)
                    | EventObject::State(0)
                    | EventObject::UnitLeader(0) => continue,
                    EventObject::Country(_) | EventObject::State(_) => {
                        return Ok(Some(EffectSavedTarget::Graph(target)));
                    }
                    EventObject::Project { owner, id } => {
                        self.context
                            .world()?
                            .project(owner, id)
                            .map_err(|why| why.to_string())?;
                        return Ok(Some(EffectSavedTarget::Graph(target)));
                    }
                    EventObject::Raid(reference) => {
                        self.context
                            .world()?
                            .raid(reference)
                            .map_err(|why| why.to_string())?;
                        return Ok(Some(EffectSavedTarget::Graph(target)));
                    }
                    EventObject::Unit(reference) => {
                        self.context
                            .world()?
                            .unit(reference)
                            .map_err(|why| why.to_string())?;
                        return Ok(Some(EffectSavedTarget::Graph(target)));
                    }
                    EventObject::Character(_)
                    | EventObject::UnitLeader(_)
                    | EventObject::Province(_)
                    | EventObject::CountryState { .. }
                    | EventObject::CountryUnitLeader { .. }
                    | EventObject::IndustrialOrganisation(_)
                    | EventObject::Combat => {
                        return Err(format!(
                            "saved effect target {}: projected {target:?} lacks liveness for the independent state/country probe; target projection support is unavailable",
                            String::from_utf8_lossy(name)
                        ));
                    }
                }
            }
        }
        let recorded = self.context.text.recorded();
        match recorded.saved_target(name).map_err(|why| why.to_string())? {
            Some(row) if crate::scope::valid_saved_target(&row) => {
                Ok(Some(EffectSavedTarget::Recorded(row)))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn state_target(&self, item: &Item, scopes: Scopes) -> Result<u32, String> {
        let target = match scope_selector(item) {
            Some(Selected::Keyword(selector)) => {
                if let Some(target) = selector_scope(selector) {
                    target
                } else if item.value_keyword().as_deref() == Some("capital") {
                    match scopes.this {
                        Scope::State(id) if id != NULL_STATE => {
                            let state = self.valid_state(id)?.ok_or_else(|| {
                                "capital selector has an invalid state; country selection needs the missing state owner".to_string()
                            })?;
                            if state.owner == 0 {
                                return Err("capital selector needs the reserved country zero's capital state; no recording writer captures it".into());
                            }
                            Parsed::Chain {
                                head: Keyword::Owner,
                                then: Box::new(Parsed::Keyword(Keyword::CapitalScope)),
                            }
                        }
                        Scope::Country(_) | Scope::CountryLeader { .. } | Scope::State(NULL_STATE) => return Ok(0),
                        _ => return Err("capital selector needs the event scope's state/country fields; the projected scope does not expose them".into()),
                    }
                } else {
                    return Ok(0);
                }
            }
            Some(Selected::EventTarget(name)) => {
                return Ok(match self.effect_saved_target(name, scopes)? {
                    Some(EffectSavedTarget::Graph(EventObject::State(state))) => state,
                    Some(EffectSavedTarget::Recorded(row)) => row.state as u32,
                    _ => 0,
                });
            }
            None => {
                let operand = scalar_operand(item)?;
                return self
                    .evaluator()?
                    .scoped_state_id(operand, scopes)
                    .map(|id| id as u32)
                    .map_err(|why| why.to_string());
            }
        };
        Ok(
            match self
                .evaluator()?
                .enter(&target, scopes)
                .map_err(|why| why.to_string())?
            {
                Some(Scope::State(id)) => id,
                _ => 0,
            },
        )
    }

    pub(super) fn tables(&self) -> HourFrame<'f> {
        self.context.text.tables()
    }

    pub(super) fn complete<T: TableSchema>(&self, mask: u64) -> Result<Rowset<'f, T::Row>, String> {
        self.tables()
            .complete::<T>(mask)
            .map_err(|why| why.to_string())
    }

    pub(super) fn features(
        &self,
        mask: u64,
    ) -> Result<Option<Cow<'f, replay_data::rows::SimulationFeatureRow>>, String> {
        let rows = self.complete::<tables::SimulationFeatures>(mask)?;
        if rows.len() != 1 {
            return Ok(None);
        }
        each(rows).next().transpose()
    }

    pub(super) fn clock_seed(&self) -> Result<Option<i32>, String> {
        const SEED: u64 = mask::<replay_data::rows::ClockRow>(&["seed"]);
        match self.complete::<tables::Clock>(SEED)?.iter().next() {
            Some(row) => Ok(Some(row.map_err(built)?.seed)),
            None => Ok(None),
        }
    }

    fn valid_state(&self, id: u32) -> Result<Option<Cow<'f, StateRow>>, String> {
        let states = self.complete::<tables::States>(STATE_COUNTRIES)?;
        if (id as i32) > 0 {
            keyed(&states, u128::from(id))
        } else {
            Ok(None)
        }
    }

    pub(super) fn line(
        &self,
        key: &[u8],
        args: &[(&str, Argument<'_>)],
        scopes: Scopes,
        expand: bool,
    ) -> Result<String, String> {
        let mut text = self.localised(key, args, scopes, expand)?;
        text.push('\n');
        Ok(text)
    }

    pub(super) fn state_name(&self, state: u32) -> Result<String, String> {
        self.context.text.state_name(state)
    }

    pub(super) fn scoped_value(&self, operand: &[u8], scopes: Scopes) -> Result<i64, String> {
        self.evaluator()?
            .scoped_value(operand, scopes)
            .map_err(|why| why.to_string())
    }

    pub(super) fn evaluator(&self) -> Result<&Evaluator<'c, 'f>, String> {
        self.evaluator
            .get_or_init(|| {
                let mut evaluator = Evaluator::new(
                    self.context.world()?,
                    self.context.expansion.and_then(ExpansionScopes::combat),
                )
                .with_event_scopes(Rc::clone(&self.graph));
                if let Some(session) = self.context.session.borrow().as_ref() {
                    evaluator = evaluator.with_variable_session(session);
                }
                evaluator.begin_description_in(
                    evaluator.event_scopes(0).map_err(|why| why.to_string())?,
                );
                Ok(evaluator)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    pub(super) fn refuse(&mut self, key: &str, why: impl Into<String>) {
        let why = why.into();
        *self
            .out
            .refusals
            .entry(format!("{} effect {key}: {why}", self.context.source))
            .or_default() += 1;
    }

    pub(super) fn expanded(&self, text: &str, scopes: Scopes) -> Result<String, String> {
        self.expanded_with_faction_environment(text, scopes, None)
    }

    pub(super) fn expanded_with_faction_environment(
        &self,
        text: &str,
        scopes: Scopes,
        faction: Option<Option<u64>>,
    ) -> Result<String, String> {
        let session = self
            .evaluator
            .get()
            .and_then(|result| result.as_ref().ok())
            .map(Evaluator::variable_session)
            .or_else(|| self.context.variable_session());
        let (result, session) = self.context.text.expand_in_description(
            text,
            &self.graph.borrow(),
            scopes.cursor.unwrap_or(0),
            session,
            self.context.expansion,
            faction,
        );
        if let Some(session) = &session
            && let Some(Ok(evaluator)) = self.evaluator.get()
        {
            evaluator.restore_variable_session(session);
        }
        self.context.session.replace(session);
        result
    }

    fn content_gap(&self, name: &'static str) -> String {
        self.context
            .content
            .gaps
            .and_then(|gaps| gaps.get(name))
            .map(|why| format!("{name} content unavailable: {why}"))
            .unwrap_or_else(|| format!("{name} content unavailable"))
    }

    pub(super) fn localised(
        &self,
        key: &[u8],
        args: &[(&str, Argument<'_>)],
        scopes: Scopes,
        expand: bool,
    ) -> Result<String, String> {
        let text = self
            .context
            .localisation
            .localise_with(key, args)
            .map_err(|why| why.to_string())?;
        if expand {
            self.expanded(&text, scopes)
        } else {
            Ok(text)
        }
    }

    pub(super) fn event_title(
        &self,
        title: content_reader_events::events::TitleKey<'_>,
        scopes: Scopes,
    ) -> Result<String, String> {
        let Some(key) = title.key else {
            return Ok(String::new());
        };
        if let Some(why) = title.unsettled {
            let text = self.localised(key, &[], scopes, false)?;
            if text
                .find('[')
                .is_some_and(|open| text[open..].contains(']'))
            {
                return Err(why);
            }
        }
        self.localised(key, &[], scopes, true)
    }

    fn own(&mut self, item: &Item, scopes: Scopes) -> Option<String> {
        match self.leaf(item, scopes) {
            Some(Ok(text)) => Some(text),
            result => {
                let effect_key = key(item);
                let why = match &result {
                    Some(Err(why)) => why.clone(),
                    _ => self
                        .effect_class(item)
                        .map(|_| format!("effect key {effect_key} description is unsupported"))
                        .unwrap_or_else(|| format!("unknown effect key {effect_key}")),
                };
                self.refuse(&effect_key, format!("key {effect_key}: {why}"));
                result.map(|_| String::new())
            }
        }
    }

    fn builder(&mut self, item: &Item, scopes: Scopes, indent: usize) -> String {
        if let Some(text) = self.scope_builder(item, scopes, indent) {
            return text;
        }
        if let Some(text) = self.list_builder(item, scopes, indent) {
            return text;
        }
        match self.effect_class(item).map(|class| class.vtable) {
            Some(effect_identity::CONTEXT) => return self.context_effect(item, scopes, indent),
            Some(0x142A31E48) => return self.tooltip_group(item, scopes, indent),
            Some(0x142A31D78) => return self.hidden_group(item, scopes, indent),
            _ => {}
        }
        match self.concise_leaf(item, scopes) {
            Some(Ok(text)) if self.indents(item) => return indent_nonempty_lines(&text, 3 * indent),
            Some(Ok(text)) => return text,
            Some(Err(why)) => {
                let effect_key = key(item);
                self.refuse(&effect_key, format!("key {effect_key}: {why}"));
                return String::new();
            }
            None => {}
        }
        let Some(own) = self.own(item, scopes) else {
            return String::new();
        };
        let mut text = if own.len() as i32 <= 1 {
            String::new()
        } else if self.indents(item) {
            format!("{}{own}", "   ".repeat(indent))
        } else {
            own
        };
        if !self.empty_build_override(item) {
            let body = self.effect_children(item);
            append_description(&mut text, &self.group(&body, scopes, indent));
        }
        text
    }

    pub(super) fn effect_children(&mut self, item: &Item) -> Vec<Item> {
        match content_reader_script::effect_keys::description_fields(item) {
            Ok(body) => body,
            Err(why) => {
                self.refuse(&key(item), why);
                Vec::new()
            }
        }
    }

    fn empty_build_override(&self, item: &Item) -> bool {
        self.effect_class(item).is_some_and(|class| {
            matches!(
                class.vtable,
                effect_identity::EMPTY_OVERRIDE | effect_identity::EMPTY_COUNTED_LOOP | effect_identity::EMPTY_CONDITIONAL_LOOP | 0x1427E4808 | 0x1427FC050 | 0x1427FBF80
            )
        })
    }

    fn indents(&self, item: &Item) -> bool {
        !(self.is_context(item)
            || item.keyword_is("country_event")
                && field(children(item), "desc")
                    .and_then(|item| item.value.read_string())
                    .is_some_and(|desc| !desc.is_empty()))
    }

    pub(super) fn group(&mut self, items: &[Item], scopes: Scopes, indent: usize) -> String {
        let mut text = String::new();
        for (item, branches) in self.attached(items, true) {
            let line = if item.keyword_is("if") {
                self.if_chain(item, &branches, scopes, indent, false)
                    .unwrap_or_default()
            } else {
                self.builder(item, scopes, indent)
            };
            append_description(&mut text, &line);
        }
        text
    }
}

pub fn technology_description(token: &str, database_index: Option<i32>, context: &Context<'_, '_>, country: u32) -> Description {
    let mut renderer = Renderer {
        context,
        graph: Rc::new(RefCell::new(EventScopes::new(vec![
            crate::scripted_text::EventScope::of(EventObject::Country(country)),
        ]))),
        evaluator: OnceCell::new(),
        out: Description::default(),
        scripted_stack: Vec::new(),
        concise: None,
    };
    match renderer.bound_technology_effect(token, database_index, Scopes::of(Scope::Country(country))) {
        Ok(text) => renderer.out.text = text,
        Err(why) => renderer.refuse("technology effects", why),
    }
    renderer.out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IterationScopes {
    pub country_seed: Option<u32>,
    pub scientist: Result<Option<u32>, String>,
    pub scientist_seed: Option<u32>,
    pub facility: Result<Option<(u32, u32)>, String>,
    pub state_seed: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputSection {
    pub heading: Option<String>,
    pub text: String,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct IterationDescription {
    pub heading: Option<String>,
    pub sections: Vec<OutputSection>,
    pub no_changes: bool,
    pub refusals: BTreeMap<String, usize>,
}

pub fn project_iteration_description(
    output: &content_reader_military::special_projects::Output,
    context: &Context<'_, '_>,
    owner: u32,
    id: u32,
    iteration: IterationScopes,
) -> IterationDescription {
    if concise::changes_nothing(output) {
        return IterationDescription {
            no_changes: true,
            ..IterationDescription::default()
        };
    }
    let mut renderer = Renderer {
        context,
        graph: Rc::new(RefCell::new(EventScopes::new(vec![
            crate::scripted_text::EventScope::of(EventObject::Project { owner, id }),
        ]))),
        evaluator: OnceCell::new(),
        out: Description::default(),
        scripted_stack: Vec::new(),
        concise: Some((owner, id)),
    };
    let mut out = IterationDescription::default();
    match renderer.project_iteration_body(
        output,
        Scopes::of(Scope::Project { owner, id }),
        owner,
        id,
        iteration,
    ) {
        Ok((heading, sections)) => {
            out.heading = heading;
            out.sections = sections;
        }
        Err(why) => renderer.refuse("special-project iteration", why),
    }
    out.refusals = renderer.out.refusals;
    out
}

pub fn project_output_description(
    output: &content_reader_military::special_projects::Output,
    context: &Context<'_, '_>,
    owner: u32,
    id: u32,
    scientist: Result<Option<u32>, String>,
    facility: Result<Option<(u32, u32)>, String>,
) -> Description {
    let mut renderer = Renderer {
        context,
        graph: Rc::new(RefCell::new(EventScopes::new(vec![
            crate::scripted_text::EventScope::of(EventObject::Project { owner, id }),
        ]))),
        evaluator: OnceCell::new(),
        out: Description::default(),
        scripted_stack: Vec::new(),
        concise: None,
    };
    let item = Item {
        key: None,
        key_quoted: false,
        operator: Default::default(),
        value: content_reader_parser::parser::Value::Block(Vec::new()),
        kind: content_reader_parser::parser::Kind::Open,
    };
    match renderer.project_output_body(
        output,
        &item,
        Scopes::of(Scope::Project { owner, id }),
        owner,
        id,
        scientist,
        facility,
    ) {
        Ok(text) => renderer.out.text = text,
        Err(why) => renderer.refuse("special-project output", why),
    }
    renderer.out
}
