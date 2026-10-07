//! CEffect::BuildDescription (140532780), shared by event and balance tooltips.
//!
//! Stable domain entry points (core changes go through lead I):
//! - `Renderer::{line, localised, expanded}` localise keys/arguments and expand in
//!   the caller's scope/session (1410CF560); `line` appends LF.
//! - `Renderer::expanded_with_faction_environment` selects the copied scope's
//!   kind-0 environment and optional faction (+0xC8, 1410CF4A0).
//! - `Renderer::{country_target, state_target}` resolve native effect selectors
//!   (141384830/1405336C0/1405302E0 and 1403A1950/140533BE0/14052FA10),
//!   sharing `evaluator` for scoped variables. State zero stays zero.
//! - `Renderer::{current_country, country_name, tag, state_name}` resolve names;
//!   `state_name` uses GetStateName (1409CC380).
//! - `Renderer::selected_party_name` shares selected-ideology party lookup
//!   and `ScriptedText::party_name` (140B980C0/14118F860).
//! - `ScriptedText::country_marked_name` shares `GetNameWithFlag` (140BA59E0).
//! - `Renderer::equipment_list_description` resolves seller-scoped entries and
//!   formats the quantity pool (140306B70/1413A8A20), reusing equipment names.
//! - `Scope` / `EventObject` carry `Project { owner, id }`, `Raid(u64)` and
//!   `Unit(u64)`; `Evaluator::{enter_scope, with_this, event_scopes}` keeps their
//!   graph links (14146BB30/1405296C0). `sp:` and `mio:` use that selector path.
//! - `World::{project(owner, id), raid(reference), unit(reference)}` joins those
//!   recorded identities. `Renderer::{project_name(owner, id), raid_name(reference),
//!   unit_name(reference), organisation_name(reference)}` returns native names
//!   or a named missing-input error (140FCDDF0/140FD6260/140C72F00/140DA44C0).
//! - `World::unit_status(reference)` supplies the retained `DivisionStatus.name`
//!   and historical generator operands. `unit_names::army_name` shares the
//!   native cached getter (140C72F00), exact literal provenance and stored-text
//!   language guard with the viewer and recorded-unit products. Only historical
//!   cache absence uses group/ordinal selection (1409B5640/1409B57C0), retaining
//!   named legacy-allocation refusals (1409B4CE0/140AE3750).
//!   `Content::division_names` caches the recorded stack's name groups.
//! - `Renderer::unit_name(&self, reference: u64) -> Result<String, String>` is
//!   `pub(super)`, the single entry point for all effect domains, including raid
//!   divisions (pass `RaidRow.unit`) and full wing handles. Recorded AirWings
//!   stored names use `scripted_text::stored_text`'s recording-language rule
//!   (141454480/140F4B8F0/140F51FB0). An old format's absent name retains
//!   `World::raid_wing_name`'s active-raid WING_NAME projection.
//! - `World::has_template(country, name)` shares the exact recorded template
//!   display-name comparison and content fallback (1406C29B0/140B91170).
//! - `Content::{buildings, resources, max_railway_level}` supplies cached declared
//!   building labels (1414A4470), resource frames (140BBD150), and NSupply's
//!   MAX_RAILWAY_LEVEL (14037A240), read once per stack by `CachedContent`.
//! - `Content::{equipment, autonomy_states}` supplies the complete equipment
//!   database (1409EB8B0/14139B350) and declared autonomy states (140669480),
//!   each read once per stack by `CachedContent`, with reader notes retained.
//! - `Content::state_categories` supplies declared category identities
//!   (1413AFB60/1413AF3B0/1413AFAC0), cached once with reader notes retained.
//! - `Content::wargoals` supplies declared wargoal-type identities
//!   (140A2C690/140A2C5D0), cached once with reader notes retained.
//! - `ScriptedText::{province_display_name, country_adjective}` shares effect
//!   province naming (140E69320) and country adjective naming (140BA5560).
//! - `Renderer::evaluator` is the shared trigger/variable/saved-target evaluator;
//!   `scoped_value` evaluates fixed5 operands (1405382B0).
//! - `Renderer::modifier_description_with` formats raw modifier text (14054E050);
//!   `modifier_list_with` expands it in the supplied scope (14054E520/1410CF560).
//!   `ModifierListOptions` maps the native formatting arguments; `modifier_list`
//!   uses its defaults. The numeric prefix is a separate `&str` argument.
//!   `cached_modifier_description_with(entries: &[CachedModifier], prefix: &str,
//!   options: ModifierListOptions) -> Result<String, String>` takes recorded
//!   definition IDs in native cache order, sharing that formatter (14128C8E0).
//! - `Content::{static_modifiers, dynamic_modifiers, opinion_modifiers,
//!   power_balances}` supplies description catalogues from the same stack;
//!   static descriptions resolve each requested name once (140550940).
//! - `Renderer::units_equipment_reference_name(&self, fields: &[Item],
//!   scopes: Scopes) -> Result<String, String>` shares CEquipmentReference naming
//!   for production and licences (141411150). `character_content().catalogue`
//!   `char_country_traits` and `char_equipment_bonuses` are shared inside effect_description for
//!   advisor-backed idea lookup and equipment fragments (1410D69C0/140FBB140).
//! - Numeric leaves share `numeric_leaf(&self, item: &Item, scopes: Scopes,
//!   key: &[u8], format: numeric::NumericFormat) -> Result<String, String>`;
//!   `numeric_value` keeps its existing fixed5 signature (1405382B0).
//!   The native modifier/number formatters below remain unchanged.
//!   `crate::modifier_text::value_text`, localisation
//!   `Argument`, and `installed_content::fixed5::{fixed5_to_string, fixed5_percent}` retain
//!   native modifier/number formatting (14054FE50, 142230F20).
//! - `Renderer::group`, `append_description`, `indent_nonempty_lines`,
//!   `trim_description`, and `Context::{with_variable_session,
//!   with_expansion_scopes, variable_session}` retain indentation/session rules.
//! - `Renderer::refuse(class, reason)` counts a named failure; the class-keyed
//!   dispatch is in `dispatch.rs`. Each lead edits only its own domain and rows.
//! - `Renderer::effect_children(item)` projects slot-4 own fields out of the
//!   child array, shared by builders and probes (140534280/1405345D0).
//! - `Renderer::fresh_country_state_scope(country, state)` creates a localisation
//!   environment without caller ROOT/FROM/PREV (1404A5070/1404A5160).
//! - Variable descriptions use `Evaluator::description_variable` for shared
//!   operands/arithmetic (14039A7D0/140379020); builder writes use
//!   `description_temp_destination_fields` and `store_description_temp`
//!   for repeated fields/defaults and index overrides (140535A10/14047AD60);
//!   `encode_description_scope` retains shared object encoding (140537310).
//!   `refuse_description_writer` invalidates unknown TLS mutations.
//!   `Evaluator::begin_description_in(scopes)` initializes the outer TLS seed
//!   from the recorded store and immediate THIS/FROM (1405354A0); nested calls
//!   retain it. `description_random_temp(item, scopes)` consumes 140537810 RNG;
//!   `description_supply_temp(item, scopes, minimum_trains)` lazily reads the
//!   content minimum only for required trains (14047B0B0).
//! - `Evaluator::{script_target_candidates, fresh_country_scope}` share native
//!   CScriptTargets selection and fresh FROM-linked scorer scopes
//!   (140C1C820/14047BB20); saved global tooltip targets use
//!   `save_global_tooltip_target` (14038D6B0/1401ED290).
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
use installed_content::{
    localisation::{Argument, Localisation},
    parser::Item,
    scope::{Keyword, Scope as Parsed},
};
use replay::rows::{GlobalSavedEventTargetRow, StateRow, tables};

mod ability;
mod characters;
pub use ability::ability_description;
mod diplomacy;
mod doctrine_sources;
pub use doctrine_sources::doctrine_tactic_sources;
mod dispatch;
mod empty;
mod events;
mod lists;
mod modifiers;
mod numeric;
mod politics;
mod scope_names;
mod scopes;
mod states;
mod technology;
#[cfg(test)]
mod tests;
mod units;
mod variables;

/// Native formatter arguments, at the caller's RSP before CALL. 14054E050
/// has 11 arguments; 14054E520 has 12, inserting an event scope at arg 3
/// (14054E53A forwards arg 12; 14054E59F/14054E5AE format then expand).
///
/// | 14054E050 | 14054E520 | Meaning / port argument |
/// |---|---|---|
/// | 1 RCX | 1 RCX | Accumulator / `modifiers`; wrapper adds receiver +0x10 |
/// | 2 RDX | 2 RDX | Output MSVC string / returned `String` |
/// | — | 3 R8 | Event scope / `scopes` on `modifier_list_with` |
/// | 3 R8D | 4 R9D | Signed requested `precision`, capped by definition +0x74 |
/// | 4 R9B | 5 [rsp+20] byte | `outer_colour`; zero omits the outer controls |
/// | 5 [rsp+20] | 6 [rsp+28] | C-string `separator`; null means LF (142702E68) |
/// | 6 [rsp+28] u32 | 7 [rsp+30] u32 | Include `category_mask` |
/// | 7 [rsp+30] u32 | 8 [rsp+38] u32 | `excluded_category_mask` |
/// | 8 [rsp+38] i64 | 9 [rsp+40] i64 | Fixed5 `scale`, not a pointer |
/// | 9 [rsp+40] | 10 [rsp+48] | Nullable MSVC-string numeric `prefix`; null is empty |
/// | 10 [rsp+48] byte | 11 [rsp+50] byte | `force_plus` (nonzero) |
/// | 11 [rsp+50] i32 | 12 [rsp+58] i32 | `invert` colour polarity (nonzero) |
///
/// Verified callers; omitted fields have the defaults below. Direct 14054E050
/// callers use `modifier_description_with`, expanding only when their caller does.
///
/// Unit-leader trait helper 140AD9060:
///
/// | Call | Port value |
/// |---|---|
/// | 140AD929B and 140AD9346 | `modifier_description_with` |
/// | Options | `ModifierListOptions { category_mask: u32::MAX, scale: trait_scale, ..Default::default() }` |
/// | Prefix | `""` |
/// | Scale source | Helper R9 copied to RBX at 140AD9089; both calls write inversion 0 (140AD926C/140AD9310) |
///
/// Targeted trait helper 140712850:
///
/// | Call | Port value |
/// |---|---|
/// | 140712BE0 | `modifier_description_with` |
/// | Options | `ModifierListOptions { separator: caller_separator, ..Default::default() }` |
/// | Prefix | Localised `EFFECT_CONTEXT_COUNTRY` at [rbp-70], passed by address at 140712BAF |
/// | Separator source | Helper R8's MSVC string (R14); its SSO/heap bytes become RAX at 140712B99/140712BA3 |
///
/// Advisor helper 14036F000:
///
/// | Call | Port value |
/// |---|---|
/// | 14036F0A2 | `modifier_description_with` |
/// | Options | `ModifierListOptions { category_mask: u32::MAX, ..Default::default() }` |
/// | Prefix | `""`; expand the eventual `EFFECT_ASSIGN_ADVISOR` wrapper (1410CF560) |
///
/// Scientist helper 1404938C0:
///
/// | Call | Port value |
/// |---|---|
/// | 140493BF5 | `modifier_list_with` |
/// | Options | `ModifierListOptions { category_mask: u32::MAX, ..Default::default() }` |
/// | Prefix / scope | `""` / caller R15; 140493BB9 also writes arg 12 (inversion 0) |
#[derive(Clone, Copy, Debug)]
pub(super) struct ModifierListOptions<'a> {
    /// 14054E050 arg 3 -> 14054F6D0 arg 6 -> 14054FE50 arg 5.
    pub(super) precision: i32,
    /// 14054E050 arg 4 wraps the entire list (14054E0CC/14054E3DE).
    pub(super) outer_colour: u8,
    /// Include mask: 14054E520 arg 7 -> 14054E050 arg 6 -> 14054F6D0 arg 9 (+0x64).
    pub(super) category_mask: u32,
    /// 14054E050 arg 7 -> 14054F6D0 arg 10; an independent intersection (+0x64).
    pub(super) excluded_category_mask: u32,
    /// Numeric separator: 14054E520 arg 6 -> 14054E050 arg 5 -> 14054F6D0 arg 8.
    pub(super) separator: &'a str,
    /// 14054E050 arg 8: effective value times scale / 100000 (14054E195–1B4).
    pub(super) scale: i64,
    /// 14054E050 arg 10 -> 14054FE50 arg 6, overriding definition NO_PLUS.
    pub(super) force_plus: bool,
    /// 14054E050 arg 11 -> 14054FE50 arg 7; zero/nonzero changes good/bad polarity.
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
pub type EventLookup<'a> = dyn Fn(&[u8]) -> Result<Arc<installed_content::events::Event>, String> + 'a;

/// Content already cached by the caller. Referenced events remain lazy.
#[derive(Default, Clone, Copy)]
pub struct Content<'a> {
    pub doctrines: Option<&'a installed_content::doctrines::Database>,
    pub decisions: Option<&'a installed_content::decisions::Database>,
    pub ideas: Option<&'a installed_content::ideas::IdeaContent>,
    pub static_modifiers: Option<&'a installed_content::static_modifiers::Descriptions>,
    pub dynamic_modifiers: Option<&'a installed_content::dynamic_modifiers::Database>,
    pub opinion_modifiers: Option<&'a installed_content::opinion_modifiers::Database>,
    pub power_balances: Option<&'a installed_content::power_balance::Database>,
    pub name_lists: Option<&'a installed_content::names::NameLists>,
    pub events: Option<&'a EventLookup<'a>>,
    pub key_names: Option<&'a installed_content::key_names::KeyNames>,
    pub scripted_effects: Option<&'a installed_content::scripted_effects::ScriptedEffects>,
    pub industrial_organisations: Option<&'a installed_content::industrial_organisations::Database>,
    pub collections: Option<&'a installed_content::collections::Database>,
    pub country_scorers: Option<&'a installed_content::country_scorers::Database>,
    pub buildings: Option<&'a installed_content::buildings::Buildings>,
    pub resources: Option<&'a [installed_content::resources::Resource]>,
    pub max_railway_level: Option<i32>,
    pub equipment: Option<&'a installed_content::equipment::Database>,
    pub autonomy_states: Option<&'a installed_content::autonomy::AutonomyStates>,
    pub state_categories: Option<&'a installed_content::state_categories::Database>,
    pub division_names: Option<&'a installed_content::division_names::Database>,
    pub wargoals: Option<&'a installed_content::wargoals::Database>,
    pub gaps: Option<&'a BTreeMap<&'static str, String>>,
}

/// Per recorded content stack. Event bodies use the caller's lazy ID cache.
pub struct CachedContent {
    politics: politics::PoliticsContent,
    characters: characters::CharacterContent,
    doctrines: Option<installed_content::doctrines::Database>,
    decisions: Option<installed_content::decisions::Database>,
    ideas: Option<installed_content::ideas::IdeaContent>,
    static_modifiers: installed_content::static_modifiers::Descriptions,
    dynamic_modifiers: Option<installed_content::dynamic_modifiers::Database>,
    opinion_modifiers: Option<installed_content::opinion_modifiers::Database>,
    power_balances: Option<installed_content::power_balance::Database>,
    /// Advisor templates from the same ideas read (140FC0B30 -> 1406B0490).
    pub advisor_templates: Option<installed_content::characters::Characters>,
    name_lists: Option<installed_content::names::NameLists>,
    key_names: Option<installed_content::key_names::KeyNames>,
    scripted_effects: Option<installed_content::scripted_effects::ScriptedEffects>,
    industrial_organisations: Option<installed_content::industrial_organisations::Database>,
    collections: Option<installed_content::collections::Database>,
    country_scorers: Option<installed_content::country_scorers::Database>,
    buildings: Option<installed_content::buildings::Buildings>,
    resources: Option<Vec<installed_content::resources::Resource>>,
    max_railway_level: Option<i32>,
    /// Only a raid name needs the raid database; keep it lazy per content stack.
    raids: std::sync::OnceLock<Result<installed_content::raids::Catalog, String>>,
    equipment: Option<installed_content::equipment::Database>,
    autonomy_states: Option<installed_content::autonomy::AutonomyStates>,
    state_categories: Option<installed_content::state_categories::Database>,
    division_names: Option<installed_content::division_names::Database>,
    wargoals: Option<installed_content::wargoals::Database>,
    gaps: BTreeMap<&'static str, String>,
    pub notes: Vec<String>,
}

impl CachedContent {
    pub(crate) fn read(
        layers: &installed_content::ResolvedLayers,
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
        let mut doctrines = keep(installed_content::doctrines::read(layers), "doctrines", &mut gaps);
        let decisions = keep(installed_content::decisions::read(layers), "decisions", &mut gaps);
        let defines = environment.defines();
        if let Some(doctrines) = &mut doctrines {
            doctrines.tactic_sources.default_reward_mastery =
                defines.fixed5("NDoctrines", "DEFAULT_REWARD_MASTERY").ok();
        }
        let dynamic_modifiers = keep(
            installed_content::dynamic_modifiers::read(layers),
            "dynamic modifiers",
            &mut gaps,
        );
        let opinion_modifiers = keep(
            installed_content::opinion_modifiers::read(layers, defines),
            "opinion modifiers",
            &mut gaps,
        );
        let power_balances = keep(
            installed_content::power_balance::read_descriptions(layers),
            "power balances",
            &mut gaps,
        );
        let read_ideas = || {
            let officers = defines.strings("NCharacter", "OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU")?;
            installed_content::ideas::read(layers, &officers)
        };
        let ideas = keep(read_ideas(), "ideas", &mut gaps);
        let maximum = defines.int32("NSupply", "MAX_RAILWAY_LEVEL");
        let max_railway_level = keep(maximum, "MAX_RAILWAY_LEVEL", &mut gaps);
        let equipment = keep(
            installed_content::equipment::Database::load_complete(layers),
            "equipment",
            &mut gaps,
        );
        let autonomy_states = keep(
            installed_content::autonomy::read(layers),
            "autonomy states",
            &mut gaps,
        );
        let state_categories = keep(
            installed_content::state_categories::read(layers),
            "state categories",
            &mut gaps,
        );
        let wargoals = keep(
            installed_content::wargoals::read(layers),
            "Wargoal definitions",
            &mut gaps,
        );
        let division_names = keep(
            installed_content::division_names::read_directories::<String>(
                layers,
                &installed_content::division_names::DIRECTORIES,
            ),
            "division name groups",
            &mut gaps,
        );
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
        notes.extend(environment.catalogue_notes.iter().cloned());
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
        let name_lists = keep(installed_content::names::read(layers), "name lists", &mut gaps);
        let key_names = keep(
            installed_content::key_names::KeyNames::read(layers),
            "effect key names",
            &mut gaps,
        );
        let scripted_effects = key_names.as_ref().and_then(|names| {
            keep(
                installed_content::scripted_effects::read(layers, names),
                "scripted effects",
                &mut gaps,
            )
        });
        let industrial_organisations = key_names.as_ref().and_then(|names| {
            keep(
                installed_content::industrial_organisations::read(layers, names),
                "industrial organisation visibility",
                &mut gaps,
            )
        });
        if let Some(database) = &industrial_organisations {
            notes.extend(database.notes.clone());
        }
        let collections = key_names.as_ref().and_then(|names| {
            keep(
                installed_content::collections::read(layers, names),
                "collections",
                &mut gaps,
            )
        });
        if let Some(database) = &collections {
            notes.extend(database.notes.clone());
        }
        let country_scorers = key_names.as_ref().and_then(|names| {
            keep(
                installed_content::country_scorers::read(layers, names),
                "country scorers",
                &mut gaps,
            )
        });
        if let Some(database) = &country_scorers {
            notes.extend(database.notes.clone());
        }
        let buildings = keep(installed_content::buildings::read(layers), "buildings", &mut gaps);
        let resources = keep(installed_content::resources::read(layers), "resources", &mut gaps).and_then(
            |(resources, warnings)| {
                let refused = warnings.iter().any(|(kind, _)| {
                    matches!(
                        kind,
                        installed_content::DiagnosticKind::ParseFailure
                            | installed_content::DiagnosticKind::Unsettled
                            | installed_content::DiagnosticKind::Unsupported
                            | installed_content::DiagnosticKind::Refused
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
            },
        );
        let politics = politics::PoliticsContent::read(layers);
        notes.extend(politics.notes.iter().cloned());
        let characters = characters::CharacterContent::read(
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
            static_modifiers: installed_content::static_modifiers::Descriptions::default(),
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
            key_names: self.key_names.as_ref(),
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
            None => self.text.world(),
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
/// targets on a distinct ROOT endpoint may survive into the next call (140529520).
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
                    out.refuse("description cleanup", why);
                }
            } else if !context.nested_session {
                let result = crate::triggers::event_scopes::cleanup_description(
                    &mut out.graph.borrow_mut(),
                    0,
                );
                if let Err(why) = result {
                    out.refuse("description cleanup", why);
                }
            }
        }
        Err(why) => out.refuse("event scope", why),
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

/// 1424B7400 trims exactly these four ASCII bytes.
pub fn trim_description(text: &str) -> &str {
    text.trim_matches(['\n', '\t', '\r', ' '])
}

/// 1424B7850 prefixes one space per level, leaving empty lines untouched.
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
        .filter_map(|item| installed_content::fixed5::script_fixed5(item.value.scalar()?))
        .next_back()
        .unwrap_or(default) // ReadFixed5@1424ABB10 retains the previous value on failure.
}

fn integer(text: &[u8]) -> i32 {
    match installed_content::crt::scan_i32(text) {
        installed_content::crt::Scanned::Stored(value) => value,
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
    item.value.scalar().ok_or_else(|| {
        format!(
            "{} requires a scalar operand (ReadValue 141385570 / CScopedVariable reader 140539440)",
            key(item)
        )
    })
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
            _ => Err("current scope has no country (1405336C0)".into()),
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
            .ok_or("PoliticalParties lacks FindPartyByIdeology result (140B980C0)")?;
        self.context
            .text
            .party_name(party, self.tag(country)?, false)
    }

    pub(super) fn country_target(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<Option<u32>, String> {
        // vf25@141384830 evaluates parsed operands through 140536ED0;
        // otherwise vf15@1405336C0 / 1405302E0 resolves the selector.
        // Block target fields belong only to the class's own native reader.
        let item = if item.value.scalar().is_some() {
            item
        } else {
            let token = item
                .keyword()
                .and_then(|key| installed_content::effect_keys::registered(&key.folded()));
            // These custom value/key readers own target outside KEY_READERS.
            if !token.is_some_and(|token| {
                installed_content::effect_keys::is_own_field(token, b"target")
                    || installed_content::effect_keys::class(token).is_some_and(
                        |(value_reader, key_reader)| {
                            value_reader == 0x1403A9C70 // Make puppet's value reader.
                                || matches!(
                                    key_reader,
                                    0x1403B1FE0 // Declare war.
                                        | 0x1403B4B00 // Remove wargoal.
                                        | 0x1403B4480 // Release autonomy.
                                        | 0x1403B4CC0 // Set autonomy.
                                        | 0x14049E600 // Send equipment.
                                        | 0x14049E7D0 // Send equipment fraction.
                                        | 0x1403B6280 // Transfer units fraction.
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
                    _ => return Err("occupied selector needs native CEventScope.state_id +0xA8 (1405336C0); this projected scope does not expose that field".into()),
                };
                if self.valid_state(state)?.is_none() {
                    return Ok(None);
                }
                // CState+268 / CResistance+50 is recorded unchanged, even inactive.
                let rows = self
                    .context
                    .text
                    .frame()
                    .complete::<tables::StateResistance>()
                    .map_err(|why| why.to_string())?;
                let raw = rows.iter().find(|row| row.state == state)
                    .ok_or_else(|| format!("StateResistance lacks country for state {state} (CResistance+0x50; 1405336C0)"))?.country;
                if raw == 0 {
                    None
                } else {
                    Some(self.context.text.recorded().slot(raw).ok_or_else(|| {
                        format!("CountryLinks lacks occupied country {raw} (140BA6240)")
                    })?)
                }
            }
            Some(Selected::Keyword(selector)) => {
                if let Some(target) = selector_scope(selector) {
                    match self.evaluator()?.enter(&target, scopes)? {
                        Some(Scope::Country(country) | Scope::CountryLeader { country, .. }) => {
                            Some(country)
                        }
                        Some(Scope::State(id)) if id != NULL_STATE => {
                            // 14052B6A0 returns the selected state's owner tag (+0xC8).
                            let state = self.valid_state(id)?.ok_or_else(|| {
                                format!("invalid nonzero state {id}: native GetCountryTagOrStateOwner@14052B6A0 dereferences the missing state's owner (+0xC8)")
                            })?;
                            if state.owner == 0 {
                                None
                            } else {
                                Some(self.context.text.recorded().slot(state.owner).ok_or_else(|| {
                                    format!("CountryLinks lacks state {id} owner {} (14052B6A0, state+0xC8)", state.owner)
                                })?)
                            }
                        }
                        _ => None,
                    }
                } else {
                    None // ResolveCountryScope does not test capital/random.
                }
            }
            Some(Selected::EventTarget(name)) => match self.effect_saved_target(name, scopes)? {
                Some(EffectSavedTarget::Graph(EventObject::Country(country))) => Some(country),
                Some(EffectSavedTarget::Recorded(row)) => self
                    .context
                    .text
                    .recorded()
                    .saved_country(row.country, installed_content::parser::c_string(name))?,
                _ => None,
            },
            None => self
                .evaluator()?
                .scoped_country(scalar_operand(item)?, scopes)?,
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
            .and_then(|key| installed_content::effect_keys::registered(&key.folded()))
            .and_then(installed_content::effect_keys::class)
            .is_some_and(|(_, reader)| reader == 0x1403AF5F0);
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
                "repeated {name} fields retain CEffect +0x24 flags or appended scope-link selectors (140530BB0/1403B2390/1403AD690); auxiliary reader replay is unavailable"
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
            // 1405336C0 falls back to zero when no current state exists.
            Scope::Country(_) | Scope::CountryLeader { .. } | Scope::State(NULL_STATE) => Ok(None),
            Scope::State(_) => Ok(match self.evaluator()?.enter(&Parsed::Keyword(keyword), scopes)? {
                Some(Scope::Country(country) | Scope::CountryLeader { country, .. }) => Some(country),
                _ => None,
            }),
            _ => Err("state owner/controller selector needs native CEventScope.state_id +0xA8 (1405336C0); this projected scope does not expose that field".into()),
        }
    }

    fn effect_saved_target(
        &self,
        name: &[u8],
        scopes: Scopes,
    ) -> Result<Option<EffectSavedTarget<'_>>, String> {
        let name = installed_content::parser::c_string(name);
        if let Some(mut at) = scopes.cursor {
            let graph = self.graph.borrow();
            // 14052B3B0/14052B410 use terminal FROM storage (14052AED0).
            let mut steps = 0;
            loop {
                if steps == graph.scopes.len() {
                    return Err(
                        "effect saved-target FROM storage cycle has no native terminal (14052AED0)"
                            .into(),
                    );
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
                        return Err(
                            "effect saved-target FROM storage link is not known (14052AED0)".into(),
                        );
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
            // 1405302E0/14052FA10: Find first, then vf9@14052BE50; an invalid first
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
                    // 14052DE40 clears both members before storing a state;
                    // its nonzero state is live, while country +0x0C stays zero.
                    EventObject::Country(_) | EventObject::State(_) => {
                        return Ok(Some(EffectSavedTarget::Graph(target)));
                    }
                    EventObject::Project { owner, id } => {
                        self.context.world()?.project(owner, id)?;
                        return Ok(Some(EffectSavedTarget::Graph(target)));
                    }
                    EventObject::Raid(reference) => {
                        self.context.world()?.raid(reference)?;
                        return Ok(Some(EffectSavedTarget::Graph(target)));
                    }
                    EventObject::Unit(reference) => {
                        self.context.world()?.unit(reference)?;
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
                            "saved effect target {}: projected {target:?} lacks CSavedEventTarget vf9 liveness for the state +0x08 / country +0x0C probe (14052FA10/1405302E0/14052BE50); core target projection plumbing pending",
                            String::from_utf8_lossy(name)
                        ));
                    }
                }
            }
        }
        // Read only the requested member: +0x08 state and +0x0C country are
        // independent, unlike a context projection (14052FA10/1405302E0).
        let recorded = self.context.text.recorded();
        match recorded.saved_target(name)? {
            Some(row) if crate::scope::valid_saved_target(row) => {
                Ok(Some(EffectSavedTarget::Recorded(row)))
            }
            _ => Ok(None),
        }
    }

    pub(super) fn state_target(&self, item: &Item, scopes: Scopes) -> Result<u32, String> {
        // vf6@14137DCD0 parses selectors before assigning the scoped value;
        // vf25@1403A1950 falls back to vf16@140533BE0 with state zero.
        let target = match scope_selector(item) {
            Some(Selected::Keyword(selector)) => {
                if let Some(target) = selector_scope(selector) {
                    target
                } else if item.value_keyword().as_deref() == Some("capital") {
                    match scopes.this {
                        Scope::State(id) if id != NULL_STATE => {
                            let state = self.valid_state(id)?.ok_or_else(|| {
                                "capital selector has an invalid state; native 140533D1E dereferences the missing state owner".to_string()
                            })?;
                            if state.owner == 0 {
                                return Err("capital selector needs reserved CCountry[0].capital_state at +0x1018 (140533BE0/1406E02D0); no recording writer captures it".into());
                            }
                            Parsed::Chain {
                                head: Keyword::Owner,
                                then: Box::new(Parsed::Keyword(Keyword::CapitalScope)),
                            }
                        }
                        // 140533D3C calls the capital getter, then falls through
                        // to 14052FA10 with zero; only the state branch returns it.
                        Scope::Country(_) | Scope::CountryLeader { .. } | Scope::State(NULL_STATE) => return Ok(0),
                        _ => return Err("capital selector needs the native event scope's state/country fields (140533BE0); the projected scope does not expose them".into()),
                    }
                } else {
                    return Ok(0); // 14052FA10 does not test these selector bits.
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
                    .map(|id| id as u32);
            }
        };
        Ok(match self.evaluator()?.enter(&target, scopes)? {
            Some(Scope::State(id)) => id,
            _ => 0,
        })
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
        self.context.text.state_name(state) // GetStateName@1409CC380.
    }

    pub(super) fn scoped_value(&self, operand: &[u8], scopes: Scopes) -> Result<i64, String> {
        self.evaluator()?.scoped_value(operand, scopes) // CScopedVariable::Evaluate@1405382B0.
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
                evaluator.begin_description_in(evaluator.event_scopes(0)?);
                Ok(evaluator)
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    pub(super) fn refuse(&mut self, key: &str, why: String) {
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

    fn own(&mut self, item: &Item, scopes: Scopes) -> Option<String> {
        match self.leaf(item, scopes) {
            Some(Ok(text)) => Some(text),
            result => {
                let class = self
                    .effect_class(item)
                    .map(|class| class.name)
                    .unwrap_or("unknown key");
                let why = match &result {
                    Some(Err(why)) => why.clone(),
                    _ => self.effect_class(item).map(|class| format!(
                        "{}@{:X} description is not ported (CEffect::BuildDescription 140532780)", class.name, class.vtable
                    )).unwrap_or_else(|| format!("unknown effect key {} (ReadEffect 1405345D0)", key(item))),
                };
                self.refuse(class, format!("key {}: {why}", key(item)));
                // Known builders can still describe their independent children.
                // An unported class may override vf8, so do not assume base traversal.
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
            Some(0x14299F6F8) /* CContextEffect */ => return self.context_effect(item, scopes, indent),
            Some(0x14299FD88) /* CEffectTooltipEffect */ => return self.tooltip_group(item, scopes, indent),
            Some(0x14299FCB8) /* CHiddenEffect */ => return self.hidden_group(item, scopes, indent),
            _ => {}
        }
        let Some(own) = self.own(item, scopes) else {
            return String::new();
        };
        // vf14 and generic own-line prefix, 140532780. Custom event desc differs.
        let mut text = if own.len() as i32 <= 1 {
            String::new()
        } else if self.indents(item) {
            format!("{}{own}", "   ".repeat(indent))
        } else {
            own
        };
        // 140532780 always walks +8/count +0x14, even when own length <= 1.
        if !self.empty_build_override(item) {
            let body = self.effect_children(item);
            append_description(&mut text, &self.group(&body, scopes, indent));
        }
        text
    }

    pub(super) fn effect_children(&mut self, item: &Item) -> Vec<Item> {
        match installed_content::effect_keys::description_fields(item) {
            Ok(body) => body,
            Err(why) => {
                let class = self
                    .effect_class(item)
                    .map(|class| class.name)
                    .unwrap_or("unknown key");
                self.refuse(class, why);
                Vec::new()
            }
        }
    }

    fn empty_build_override(&self, item: &Item) -> bool {
        // These classes override vf8 with 1402E9B40, discarding all children.
        self.effect_class(item).is_some_and(|class| {
            matches!(
                class.name,
                "CForEachEffect"
                    | "CForEffect"
                    | "CWhileEffect"
                    | "CSetPoliticalPartyEffect"
                    | "CFindHighestLowestInArrayEffect<>"
            )
        })
    }

    fn indents(&self, item: &Item) -> bool {
        // Context vf14 is needed here for its parent's first-child header clear.
        // The builder returns context descriptions before its own-line prefix.
        // CIfEffect vf14 is ReturnTrue@1401802A0 (1427BE590 + 0x70).
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
