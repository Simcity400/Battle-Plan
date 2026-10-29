//! Effect descriptions, shared by event and balance tooltips.

//! Shared domain entry points:
//! `Renderer::{line, localised, expanded}` localise keys/arguments and expand in
//!   the caller's scope/session; `line` appends LF.
//! `Renderer::expanded_with_faction_environment` selects the copied scope's
//!   kind-0 environment and optional faction.
//! `Renderer::{country_target, state_target}` resolve native effect selectors

//!   sharing `evaluator` for scoped variables. State zero stays zero.
//! `Renderer::{current_country, country_name, tag, state_name}` resolve names;
//!   `state_name` shares the state-name formatter.
//! `Renderer::selected_party_name` shares selected-ideology party lookup
//!   and `ScriptedText::party_name`.
//! `ScriptedText::country_marked_name` shares marked country-name formatting.
//! `Renderer::equipment_list_description` resolves seller-scoped entries and
//!   formats the quantity pool, reusing equipment names.
//! `Scope` `EventObject` carry `Project { owner, id }`, `Raid(u64)` and
//!   `Unit(u64)`; `Evaluator::{enter_scope, with_this, event_scopes}` keeps their
//!   graph links. `sp:` and `mio:` use that selector path.
//! `World::{project(owner, id), raid(reference), unit(reference)}` joins those
//!   recorded identities. `Renderer::{project_name(owner, id), raid_name(reference),
//!   unit_name(reference), organisation_name(reference)}` returns native names
//!   or a named missing-input error.
//! `World::unit_status(reference)` supplies the retained `DivisionStatus.name`
//!   and historical generator operands. `unit_names::army_name` shares the
//!   cached getter, exact literal identity and stored-text
//!   language guard with the viewer and recorded-unit products. Only historical
//!   cache absence uses group/ordinal selection, retaining
//!   named legacy-allocation refusals.
//!   `Content::division_names` caches the recorded stack's name groups.
//! `Renderer::unit_name(&self, reference: u64) -> Result<String, String>` is
//!   `pub(super)`, the single entry point for all effect domains, including raid
//!   divisions (pass `RaidRow.unit`) and full wing handles. Recorded AirWings
//!   stored names use `scripted_text::stored_text`'s recording-language rule
//!   An old format's absent name retains
//!   `World::raid_wing_name`'s active-raid WING_NAME projection.
//! `World::has_template(country, name)` shares the exact recorded template
//!   display-name comparison and content fallback.
//! `Content::{buildings, resources, max_railway_level}` supplies cached declared
//!   building labels, resource frames, and NSupply's
//!   MAX_RAILWAY_LEVEL, read once per stack by `CachedContent`.
//! `Content::{equipment, autonomy_states}` supplies the complete equipment
//!   database and declared autonomy states,
//!   each read once per stack by `CachedContent`, with reader notes retained.
//! `Content::state_categories` supplies declared category identities
//!   cached once with reader notes retained.
//! `Content::wargoals` supplies declared wargoal-type identities
//!   cached once with reader notes retained.
//! `ScriptedText::{province_display_name, country_adjective}` shares effect
//!   province naming and country adjective naming.
//! `Renderer::evaluator` is the shared trigger/variable/saved-target evaluator;
//!   `scoped_value` evaluates fixed5 operands.
//! `Renderer::modifier_description_with` formats raw modifier text;
//!   `modifier_list_with` expands it in the supplied scope.
//!   `ModifierListOptions` maps the native formatting arguments; `modifier_list`
//!   uses its defaults. The numeric prefix is a separate `&str` argument.
//!   `cached_modifier_description_with(entries: &[CachedModifier], prefix: &str,
//!   options: ModifierListOptions) -> Result<String, String>` takes recorded
//!   definition IDs in native cache order, sharing that formatter.
//! `scaled_static_modifiers(context, token, prefix, scale)` lists a static
//!   modifier's raw text with its values scaled, sharing that formatter.
//! `Content::{static_modifiers, dynamic_modifiers, opinion_modifiers,
//!   power_balances}` supplies description catalogues from the same stack;
//!   static descriptions resolve each requested name once.
//! `Renderer::units_equipment_reference_name(&self, fields: &[Item],
//!   scopes: Scopes) -> Result<String, String>` shares equipment reference naming
//!   for production and licences. `character_content.catalogue`
//!   `char_country_traits` and `char_equipment_bonuses` are shared inside effect_description for
//!   advisor-backed idea lookup and equipment fragments.
//! Numeric leaves share `numeric_leaf(&self, item: &Item, scopes: Scopes,
//!   key: &[u8], format: numeric::NumericFormat) -> Result<String, String>`;
//!   `numeric_value` keeps its existing fixed5 signature.
//!   The modifier/number formatters share localisation arguments
//!   and `crate::modifier_text::value_text`.
//!   `Argument` and `installed_content_core::fixed5::{fixed5_to_string, fixed5_percent}` retain
//!   native modifier/number formatting.
//! `Renderer::group`, `append_description`, `indent_nonempty_lines`,
//!   `trim_description`, and `Context::{with_variable_session,
//!   with_expansion_scopes, variable_session}` retain indentation/session rules.
//! `Renderer::refuse(key, reason)` counts a named failure; the class-keyed
//!   dispatch is in `dispatch.rs`.
//! `Renderer::effect_children(item)` projects each reader's own fields out of the
//!   child array, shared by builders and probes.
//! `Renderer::fresh_country_state_scope(country, state)` creates a localisation
//!   environment without caller ROOT/FROM/PREV.
//! Variable descriptions use `Evaluator::description_variable` for shared
//!   operands/arithmetic; builder writes use
//!   `description_temp_destination_fields` and `store_description_temp`
//!   for repeated fields/defaults and index overrides;
//!   `encode_description_scope` retains shared object encoding.
//!   `refuse_description_writer` invalidates unknown TLS mutations.
//!   `Evaluator::begin_description_in(scopes)` initializes the outer TLS seed
//!   from the recorded store and immediate THIS/FROM; nested calls
//!   retain it. `description_random_temp(item, scopes)` consumes RNG;
//!   `description_supply_temp(item, scopes, minimum_trains)` lazily reads the
//!   content minimum only for required trains.
//! `Evaluator::{script_target_candidates, fresh_country_scope}` share native
//!   script targets selection and fresh FROM-linked scorer scopes;
//!   saved global tooltip targets use
//!   `save_global_tooltip_target`.
use std::{
    cell::{OnceCell, RefCell},
    collections::BTreeMap,
    rc::Rc,
    sync::Arc,
};

use crate::triggers::{NULL_STATE, Scope, Selected, Selector, scope_selector};
use crate::{
    scripted_text::{EventObject, EventScopes, ExpansionScopes, Link, ScriptedText},
    triggers::{Evaluator, Scopes, World},
};
use installed_content_core::{parser::Item, scope::Keyword, scope::Scope as Parsed};
use installed_content_localisation::{localisation::Argument, localisation::Localisation};
use replay_data::{rows::GlobalSavedEventTargetRow, rows::StateRow, rows::tables};

mod ability;
mod characters;
pub use ability::ability_description;
mod diplomacy;
mod doctrine_sources;
pub use doctrine_sources::{doctrine_benefits, doctrine_tactic_sources};
pub use static_lists::scaled_static_modifiers;
mod dispatch;
mod empty;
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

/// Recorded operands effect descriptions read beyond the trigger kit
/// ([`crate::triggers::RECORDED_TABLES`]).
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
    replay_data::rows::TableId::ScientistSkills,
    replay_data::rows::TableId::Scientists,
    replay_data::rows::TableId::TaskForces,
    replay_data::rows::TableId::TimedIdeas,
    replay_data::rows::TableId::TrainInventory,
    replay_data::rows::TableId::VariantDefinitions,
    replay_data::rows::TableId::VariantUpgrades,
];

/// Every recorded table an effect description reads at its hour: the trigger
/// kit and the effects' own operands. Every product describing effects selects
/// all of them, so an operand the hour recorded is never discarded locally.
pub fn recorded_tables() -> impl Iterator<Item = replay_data::rows::TableId> {
    crate::triggers::RECORDED_TABLES
        .iter()
        .chain(OPERAND_TABLES)
        .copied()
}

/// Modifier list formatting controls shared by raw and scoped descriptions.
///
/// Precision is capped by each definition. Category masks use separate include
/// and exclude intersections. Scale multiplies the value before fixed-point
/// division. A null separator selects LF; an empty prefix adds no numeric text.
/// Colour, plus signs and inversion apply independently of scope expansion.
#[derive(Clone, Copy, Debug)]
pub(super) struct ModifierListOptions<'a> {
    /// Requested numeric precision, capped by each definition.
    pub(super) precision: i32,
    /// Colour marker surrounding the entire list.
    pub(super) outer_colour: u8,
    /// A definition must include a category in this mask.
    pub(super) category_mask: u32,
    /// A definition must include a category outside this mask.
    pub(super) excluded_category_mask: u32,
    /// Separator appended to numeric modifier text.
    pub(super) separator: &'a str,
    /// Fixed-point multiplier applied before division by 100_000.
    pub(super) scale: i64,
    /// Print a plus sign even when a definition disables it.
    pub(super) force_plus: bool,
    /// Reverse good/bad numeric colour polarity.
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

/// Resolves an event id to its parsed event, or why it cannot be.
pub type EventLookup<'a> =
    dyn Fn(&[u8]) -> Result<Arc<installed_content_events::events::Event>, String> + 'a;

/// Content already cached by the caller. Referenced events remain lazy.
#[derive(Default, Clone, Copy)]
pub struct Content<'a> {
    pub doctrines: Option<&'a installed_content_core::doctrines::Database>,
    pub decisions: Option<&'a installed_content_core::decisions::Database>,
    pub ideas: Option<&'a installed_content_core::ideas::IdeaContent>,
    pub static_modifiers: Option<&'a installed_content_core::static_modifiers::Descriptions>,
    pub dynamic_modifiers: Option<&'a installed_content_core::dynamic_modifiers::Database>,
    pub opinion_modifiers: Option<&'a installed_content_core::opinion_modifiers::Database>,
    pub power_balances: Option<&'a installed_content_core::power_balance::Database>,
    pub name_lists: Option<&'a installed_content_core::names::NameLists>,
    pub events: Option<&'a EventLookup<'a>>,
    pub key_names: Option<&'a installed_content_core::key_names::KeyNames>,
    pub scripted_effects: Option<&'a installed_content_core::scripted_effects::ScriptedEffects>,
    pub industrial_organisations: Option<&'a installed_content_core::industrial_organisations::Database>,
    pub collections: Option<&'a installed_content_core::collections::Database>,
    pub country_scorers: Option<&'a installed_content_core::country_scorers::Database>,
    pub buildings: Option<&'a installed_content_core::buildings::Buildings>,
    pub resources: Option<&'a [installed_content_core::resources::Resource]>,
    pub max_railway_level: Option<i32>,
    pub equipment: Option<&'a installed_content_core::equipment::Database>,
    pub autonomy_states: Option<&'a installed_content_core::autonomy::AutonomyStates>,
    pub state_categories: Option<&'a installed_content_state_categories::state_categories::Database>,
    pub division_names: Option<&'a installed_content_division_names::division_names::Database>,
    pub wargoals: Option<&'a installed_content_wargoals::wargoals::Database>,
    pub gaps: Option<&'a BTreeMap<&'static str, String>>,
}

/// Per recorded content stack. Event bodies use the caller's lazy ID cache.
pub struct CachedContent {
    politics: politics::PoliticsContent,
    characters: characters::CharacterContent,
    doctrines: Option<installed_content_core::doctrines::Database>,
    decisions: Option<installed_content_core::decisions::Database>,
    ideas: Option<installed_content_core::ideas::IdeaContent>,
    static_modifiers: installed_content_core::static_modifiers::Descriptions,
    dynamic_modifiers: Option<installed_content_core::dynamic_modifiers::Database>,
    opinion_modifiers: Option<installed_content_core::opinion_modifiers::Database>,
    power_balances: Option<installed_content_core::power_balance::Database>,
    /// Advisor templates from the same ideas read.
    pub advisor_templates: Option<installed_content_core::characters::Characters>,
    name_lists: Option<installed_content_core::names::NameLists>,
    key_names: Option<std::sync::Arc<installed_content_core::key_names::KeyNames>>,
    scripted_effects: Option<installed_content_core::scripted_effects::ScriptedEffects>,
    industrial_organisations: Option<installed_content_core::industrial_organisations::Database>,
    collections: Option<installed_content_core::collections::Database>,
    country_scorers: Option<installed_content_core::country_scorers::Database>,
    buildings: Option<installed_content_core::buildings::Buildings>,
    resources: Option<Vec<installed_content_core::resources::Resource>>,
    max_railway_level: Option<i32>,
    /// Only a raid name needs the raid database; keep it lazy per content stack.
    raids: std::sync::OnceLock<Result<installed_content_raids::raids::Catalog, String>>,
    equipment: Option<installed_content_core::equipment::Database>,
    autonomy_states: Option<installed_content_core::autonomy::AutonomyStates>,
    state_categories: Option<installed_content_state_categories::state_categories::Database>,
    division_names: Option<installed_content_division_names::division_names::Database>,
    wargoals: Option<installed_content_wargoals::wargoals::Database>,
    gaps: BTreeMap<&'static str, String>,
    pub notes: Vec<String>,
}

/// Runs `read` on a helper thread working for this one ([`installed_content_core::parallel::spawn`]),
/// with what the catalogues note there credited to this thread's consumers once [`joined`].
fn alongside<'s, T: Send + 's>(
    threads: &'s std::thread::Scope<'s, '_>,
    read: impl FnOnce() -> T + Send + 's,
) -> std::thread::ScopedJoinHandle<'s, (T, crate::catalogue::OwnRefusals)> {
    installed_content_core::parallel::spawn(threads, move || crate::catalogue::capture(read))
}

/// What [`alongside`] read, its notes credited here as if read on this thread.
fn joined<T>(read: std::thread::ScopedJoinHandle<'_, (T, crate::catalogue::OwnRefusals)>) -> T {
    let (value, noted) = installed_content_core::parallel::join(read);
    crate::catalogue::credit(&noted);
    value
}

impl CachedContent {
    pub(crate) fn read(
        layers: &installed_content_core::ResolvedLayers,
        environment: &crate::scripted_text::Environment,
    ) -> std::io::Result<Self> {
        std::thread::scope(|threads| Self::read_with(threads, layers, environment))
    }

    fn read_with<'s>(
        threads: &'s std::thread::Scope<'s, '_>,
        layers: &'s installed_content_core::ResolvedLayers,
        environment: &crate::scripted_text::Environment,
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
        // The largest reads that need only the layers run alongside the rest.
        let doctrines = alongside(threads, || installed_content_core::doctrines::read(layers));
        let decisions = alongside(threads, || installed_content_core::decisions::read(layers));
        let division_names = alongside(threads, || {
            installed_content_division_names::division_names::read_directories::<String>(
                layers,
                &installed_content_division_names::division_names::DIRECTORIES,
            )
        });
        let politics = alongside(threads, || politics::PoliticsContent::read(layers));
        let defines = environment.defines();
        let dynamic_modifiers = keep(
            installed_content_core::dynamic_modifiers::read(layers),
            "dynamic modifiers",
            &mut gaps,
        );
        let opinion_modifiers = keep(
            installed_content_core::opinion_modifiers::read(layers, defines),
            "opinion modifiers",
            &mut gaps,
        );
        let power_balances = keep(
            installed_content_core::power_balance::read_descriptions(layers),
            "power balances",
            &mut gaps,
        );
        let read_ideas = || {
            let officers = defines.strings("NCharacter", "OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU")?;
            installed_content_core::ideas::read(layers, &officers)
        };
        let ideas = keep(read_ideas(), "ideas", &mut gaps);
        let maximum = defines.int32("NSupply", "MAX_RAILWAY_LEVEL");
        let max_railway_level = keep(maximum, "MAX_RAILWAY_LEVEL", &mut gaps);
        let equipment = keep(
            installed_content_core::equipment::Database::load_complete(layers),
            "equipment",
            &mut gaps,
        );
        let autonomy_states = keep(
            installed_content_core::autonomy::read(layers),
            "autonomy states",
            &mut gaps,
        );
        let state_categories = keep(
            installed_content_state_categories::state_categories::read(layers),
            "state categories",
            &mut gaps,
        );
        let wargoals = keep(
            installed_content_wargoals::wargoals::read(layers),
            "Wargoal definitions",
            &mut gaps,
        );
        let mut doctrines = keep(joined(doctrines), "doctrines", &mut gaps);
        if let Some(doctrines) = &mut doctrines {
            doctrines.tactic_sources.default_reward_mastery =
                defines.fixed5("NDoctrines", "DEFAULT_REWARD_MASTERY").ok();
        }
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
        // Effect descriptions share these catalogues and their diagnostics.
        let _ = environment.ideologies();
        let _ = environment.terrain();
        let _ = environment.leader_traits();
        notes.extend(environment.catalogue_notes());
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
        let (ideas, advisor_templates) = match ideas {
            Some((ideas, templates, warnings)) => {
                notes.extend(warnings.into_iter().map(|(_, why)| why));
                (Some(ideas), Some(templates))
            }
            None => (None, None),
        };
        let name_lists = keep(installed_content_core::names::read(layers), "name lists", &mut gaps);
        let key_names = keep(layers.key_names(), "effect key names", &mut gaps);
        let scripted_effects = key_names.as_ref().and_then(|names| {
            keep(
                installed_content_core::scripted_effects::read(layers, names),
                "scripted effects",
                &mut gaps,
            )
        });
        let industrial_organisations = key_names.as_ref().and_then(|names| {
            keep(
                installed_content_core::industrial_organisations::read(layers, names),
                "industrial organisation visibility",
                &mut gaps,
            )
        });
        if let Some(database) = &industrial_organisations {
            notes.extend(database.notes.clone());
        }
        let collections = key_names.as_ref().and_then(|names| {
            keep(
                installed_content_core::collections::read(layers, names),
                "collections",
                &mut gaps,
            )
        });
        if let Some(database) = &collections {
            notes.extend(database.notes.clone());
        }
        let country_scorers = key_names.as_ref().and_then(|names| {
            keep(
                installed_content_core::country_scorers::read(layers, names),
                "country scorers",
                &mut gaps,
            )
        });
        if let Some(database) = &country_scorers {
            notes.extend(database.notes.clone());
        }
        let buildings = keep(
            installed_content_core::buildings::read(layers),
            "buildings",
            &mut gaps,
        );
        let resources = keep(
            installed_content_core::resources::read(layers),
            "resources",
            &mut gaps,
        )
        .and_then(|(resources, warnings)| {
            let refused = warnings.iter().any(|(kind, _)| {
                matches!(
                    kind,
                    installed_content_core::DiagnosticKind::ParseFailure
                        | installed_content_core::DiagnosticKind::Unsettled
                        | installed_content_core::DiagnosticKind::Unsupported
                        | installed_content_core::DiagnosticKind::Refused
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
            (!refused).then_some(resources)
        });
        let politics = joined(politics);
        notes.extend(politics.notes.iter().cloned());
        let characters = characters::CharacterContent::read(
            threads,
            layers,
            environment,
            advisor_templates
                .as_ref()
                .ok_or_else(|| gaps["ideas"].clone()),
        );
        notes.extend(characters.notes.iter().cloned());
        Ok(Self {
            characters,
            politics,
            doctrines,
            decisions,
            ideas,
            static_modifiers: installed_content_core::static_modifiers::Descriptions::default(),
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
            raids: std::sync::OnceLock::new(),
            equipment,
            autonomy_states,
            state_categories,
            division_names,
            wargoals,
            gaps,
            notes,
        })
    }

    /// The technology catalogue read with the doctrines, or why it is unavailable.
    pub fn technologies(&self) -> Result<&installed_content_core::technology::Catalog, String> {
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
            key_names: self.key_names.as_deref(),
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

impl crate::scripted_text::ScriptedContent {
    pub fn effects(&self) -> Result<&CachedContent, String> {
        self.effects
            .get_or_init(|| {
                CachedContent::read(&self.layers.layers, self.environment()?)
                    .map_err(|why| format!("effect description content: {why}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// One hour's immutable inputs; the mutable event graph belongs to its caller.
pub struct Context<'c, 'f> {
    text: &'c ScriptedText<'f>,
    localisation: &'c Localisation,
    session: RefCell<Option<crate::triggers::VariableSession>>,
    nested_session: bool,
    expansion: Option<&'c ExpansionScopes<'c, 'f>>,
    perspective: u32,
    source: &'c str,
    content: Content<'c>,
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
        }
    }

    fn world(&self) -> Result<&'c World<'f>, String> {
        match self.expansion.and_then(|scopes| scopes.world) {
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

/// Each call is its own native variable/description session. Cleanup follows FROM;
/// targets on a distinct ROOT endpoint may survive into the next call.
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
}

/// trims exactly these four ASCII bytes.
pub fn trim_description(text: &str) -> &str {
    text.trim_matches(['\n', '\t', '\r', ' '])
}

/// prefixes one space per level, leaving empty lines untouched.
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
        .filter_map(|item| installed_content_core::fixed5::script_fixed5(item.value.scalar()?))
        .next_back()
        .unwrap_or(default) // fixed-point parsing retains the previous value on failure.
}

fn integer(text: &[u8]) -> i32 {
    match installed_content_core::crt::scan_i32(text) {
        installed_content_core::crt::Scanned::Stored(value) => value,
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
    Recorded(&'a GlobalSavedEventTargetRow),
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
        let party = self
            .context
            .text
            .frame()
            .complete::<tables::PoliticalParties>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| {
                self.context.text.recorded().slot(row.country) == Some(country)
                    && row.token.eq_ignore_ascii_case(ideology)
            })
            .ok_or("PoliticalParties lacks the party selected by ideology")?;
        self.context
            .text
            .party_name(party, self.tag(country)?, false)
    }

    pub(super) fn country_target(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<Option<u32>, String> {
        // Evaluate parsed scoped operands;
        // otherwise resolves the selector.
        // Block target fields belong only to the class's own native reader.
        let item = if item.value.scalar().is_some() {
            item
        } else {
            let token = item
                .keyword()
                .and_then(|key| installed_content_core::effect_keys::registered(&key.folded()));
            // These custom value/key readers own target outside KEY_READERS.
            if !token.is_some_and(|token| {
                installed_content_core::effect_keys::is_own_field(token, b"target")
                    || installed_content_core::effect_keys::class(token).is_some_and(
                        |(value_reader, key_reader)| {
                            value_reader == 0x1403C2810 // Make puppet's value reader.
                                || matches!(
                                    key_reader,
                                    0x1403CABB0 // Declare war.
                                        | 0x1403CD6E0 // Remove wargoal.
                                        | 0x1403CD060 // Release autonomy.
                                        | 0x1403CD8A0 // Set autonomy.
                                        | 0x1404BE080 // Send equipment.
                                        | 0x1404BE250 // Send equipment fraction.
                                        | 0x1403CEE80 // Transfer units fraction.
                                )
                        },
                    )
            }) {
                scalar_operand(item)?;
            }
            let Some(target) = self.country_target_field(item, "target")? else {
                return Ok(None); // Constructor's zero country/selector.
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
                // state resistance is recorded unchanged, even inactive.
                let rows = self
                    .context
                    .text
                    .frame()
                    .complete::<tables::StateResistance>()
                    .map_err(|why| why.to_string())?;
                let raw = rows
                    .iter()
                    .find(|row| row.state == state)
                    .ok_or_else(|| format!("StateResistance lacks country for state {state}"))?
                    .country;
                if raw == 0 {
                    None
                } else {
                    Some(
                        self.context
                            .text
                            .recorded()
                            .slot(raw)
                            .ok_or_else(|| format!("CountryLinks lacks occupied country {raw}"))?,
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
                            // returns the selected state's owner tag.
                            let state = self.valid_state(id)?.ok_or_else(|| {
                                format!("invalid nonzero state {id}: country selection needs the missing state's owner")
                            })?;
                            if state.owner == 0 {
                                None
                            } else {
                                Some(self.context.text.recorded().slot(state.owner).ok_or_else(
                                    || {
                                        format!(
                                            "CountryLinks lacks state {id} owner {}",
                                            state.owner
                                        )
                                    },
                                )?)
                            }
                        }
                        _ => None,
                    }
                } else {
                    None // Country-scope resolution does not test capital/random.
                }
            }
            Some(Selected::EventTarget(name)) => match self.effect_saved_target(name, scopes)? {
                Some(EffectSavedTarget::Graph(EventObject::Country(country))) => Some(country),
                Some(EffectSavedTarget::Recorded(row)) => self
                    .context
                    .text
                    .recorded()
                    .saved_country(row.country, installed_content_core::parser::c_string(name))
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
        let chains = item
            .keyword()
            .and_then(|key| installed_content_core::effect_keys::registered(&key.folded()))
            .and_then(installed_content_core::effect_keys::class)
            .is_some_and(|(_, reader)| reader == 0x1403C81B0);
        if fields.clone().nth(1).is_some()
            && fields.clone().any(|field| {
                scope_selector(field).is_some()
                    || chains
                        && field
                            .value
                            .scalar()
                            .is_some_and(|value| value.contains(&b'.'))
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
            // falls back to zero when no current state exists.
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
        let name = installed_content_core::parser::c_string(name);
        if let Some(mut at) = scopes.cursor {
            let graph = self.graph.borrow();
            // use terminal FROM storage.
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
            // Find the first name match, then test its validity; an invalid first
            // duplicate tries the next store, never another duplicate.
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
                    // clears both members before storing a state;
                    // its nonzero state is live, while country stays zero.
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
        // Read only the requested member: state and country are
        // independent, unlike a context projection.
        let recorded = self.context.text.recorded();
        match recorded.saved_target(name).map_err(|why| why.to_string())? {
            Some(row) if crate::scope::valid_saved_target(row) => {
                Ok(Some(EffectSavedTarget::Recorded(row)))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn state_target(&self, item: &Item, scopes: Scopes) -> Result<u32, String> {
        // parses selectors before assigning the scoped value;
        // falls back to the carried country with state zero.
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
                        // calls the capital getter, then falls through
                        // to the country branch with zero; only the state branch returns it.
                        Scope::Country(_) | Scope::CountryLeader { .. } | Scope::State(NULL_STATE) => return Ok(0),
                        _ => return Err("capital selector needs the event scope's state/country fields; the projected scope does not expose them".into()),
                    }
                } else {
                    return Ok(0); // does not test these selector bits.
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

    fn valid_state(&self, id: u32) -> Result<Option<&StateRow>, String> {
        Ok(self
            .context
            .text
            .frame()
            .complete::<tables::States>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| (id as i32) > 0 && row.id == id))
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
                    self.context.expansion.and_then(|scopes| scopes.combat),
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

    /// None: event text; Some: kind-0 environment, including a null faction.
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

    /// Bracketed text reads the scope state the title triggers leave, so it
    /// refuses when their walk stopped short; other text does not depend on it.
    pub(super) fn event_title(
        &self,
        title: installed_content_events::events::TitleKey<'_>,
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
                // Known builders can still describe their independent children.
                // An unsupported class may override traversal, so do not assume the base projection.
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
            Some(0x142A310F0) => return self.context_effect(item, scopes, indent),
            Some(0x142A31708) => return self.tooltip_group(item, scopes, indent),
            Some(0x142A31638) => return self.hidden_group(item, scopes, indent),
            _ => {}
        }
        let Some(own) = self.own(item, scopes) else {
            return String::new();
        };
        // Prefix the generic own line with LF. Custom event descriptions differ.
        let mut text = if own.len() as i32 <= 1 {
            String::new()
        } else if self.indents(item) {
            format!("{}{own}", "   ".repeat(indent))
        } else {
            own
        };
        // Always traverse the children, even when own length <= 1.
        if !self.empty_build_override(item) {
            let body = self.effect_children(item);
            append_description(&mut text, &self.group(&body, scopes, indent));
        }
        text
    }

    pub(super) fn effect_children(&mut self, item: &Item) -> Vec<Item> {
        match installed_content_core::effect_keys::description_fields(item) {
            Ok(body) => body,
            Err(why) => {
                self.refuse(&key(item), why);
                Vec::new()
            }
        }
    }

    fn empty_build_override(&self, item: &Item) -> bool {
        // These builders discard their children.
        self.effect_class(item).is_some_and(|class| {
            matches!(
                class.vtable,
                0x1427f3960 | 0x1427f3cf0 | 0x1427f3bf8 | 0x1427E47C8 | 0x1427FC010 | 0x1427FBF40
            )
        })
    }

    fn indents(&self, item: &Item) -> bool {
        // Context is needed here for its parent's first-child header clear.
        // The builder returns context descriptions before its own-line prefix.
        // The if effect tests its embedded condition.
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

/// Describe an authored output in the selected project's actual assignment scopes.
/// A missing assignment stays missing instead of selecting another scientist or facility.
pub fn project_output_description(
    output: &installed_content_core::special_projects::Output,
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
    };
    let item = Item {
        key: None,
        key_quoted: false,
        operator: Default::default(),
        value: installed_content_core::parser::Value::Block(Vec::new()),
        kind: installed_content_core::parser::Kind::Open,
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
