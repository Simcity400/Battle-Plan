//! Ordered scripted entries and content used by recorded triggers.
use super::*;
use crate::triggers::{
    ContentFacts, Eval, GeographyInputs, LeaderRoles, Scope, Scopes, Temps, World,
};
use std::collections::{BTreeMap, BTreeSet};
mod prepared;

#[cfg(test)]
mod tests;

pub type StateNames = BTreeMap<u32, Result<String, String>>;

#[derive(Clone)]
pub(super) struct ActiveEntry {
    name: String,
    at: usize,
    scopes_len: usize,
    bare: bool,
    variables: ScopeVariables,
}

struct EntryGuard<'s, 'e, 'a>(&'s ExpansionScopes<'e, 'a>, usize);

impl Drop for EntryGuard<'_, '_, '_> {
    fn drop(&mut self) {
        self.0.active.borrow_mut().pop();
        self.0.scopes.borrow_mut().truncate(self.1);
        self.0.prev_distance.borrow_mut().truncate(self.1);
    }
}

impl ActiveEntry {
    fn matches(&self, name: &str, scopes: &ExpansionScopes, at: usize, bare: bool) -> bool {
        if self.name != name || self.bare != bare || self.variables != scopes.variables.get() {
            return false;
        }
        // Equivalent graphs have the same PREV distance to a different object or unknown link.
        // This skips full graph walks for a growing PREV history.
        if scopes.valid_links.get() {
            let distance = scopes.prev_distance.borrow();
            if distance[self.at] != distance[at] {
                return false;
            }
        }
        let valid_links = scopes.valid_links.get();
        let scopes = scopes.scopes.borrow();
        if scopes[self.at].random_seed != scopes[at].random_seed
            || scopes[self.at].random_seed_refusal != scopes[at].random_seed_refusal
        {
            return false;
        }
        // Compare reachable values and links, not allocation indices.
        // ROOT/FROM/PREV can themselves be promoted; only THIS's seed
        // is read here, since promoted entries receive a fresh seed.
        let mut pending = vec![(self.at, at)];
        let mut seen = BTreeSet::new();
        while let Some((left_at, right_at)) = pending.pop() {
            if (valid_links && left_at == right_at) || !seen.insert((left_at, right_at)) {
                continue;
            }
            let (left, right) = (scopes[left_at], scopes[right_at]);
            if left.this != right.this {
                return false;
            }
            for (a, b) in [
                (left.root, right.root),
                (left.from, right.from),
                (left.prev, right.prev),
            ] {
                let linked = |at, link, len| match link {
                    Link::Itself => Some(at),
                    Link::Scope(to) if to < len => Some(to),
                    _ => None,
                };
                match (
                    linked(left_at, a, self.scopes_len),
                    linked(right_at, b, scopes.len()),
                ) {
                    (Some(a), Some(b)) => pending.push((a, b)),
                    (None, None) if a == b => {}
                    _ => return false,
                }
            }
        }
        true
    }
}

pub struct StateContent {
    pub defines: installed_content_core::defines::Defines,
    pub states: installed_content_states::states::States,
    pub names: StateNames,
    pub warnings: Vec<installed_content_states::states::Diagnostic>,
}

pub fn live_state_warnings(
    notes: &[installed_content_states::states::Diagnostic],
) -> impl Iterator<Item = &str> {
    use installed_content_states::states::MisreadCause;
    // States' Warning channel contains nonpositive-ID and duplicate-ID
    // skips. Both keep exactly the templates the game keeps.
    // Only the listed state-field recoveries preserve the game's result;
    // shared reader notes and unsupported readers still warn.
    notes.iter().filter_map(|note| {
        let exact = note.kind == installed_content_core::DiagnosticKind::Misread
            && matches!(
                note.cause,
                Some(
                    MisreadCause::InvalidImpassable
                        | MisreadCause::OffMapProvince
                        | MisreadCause::NonStrategicResource
                        | MisreadCause::InvalidVictoryPoints
                        | MisreadCause::NonBlockBuildings
                )
            );
        (note.kind != installed_content_core::DiagnosticKind::Warning && !exact)
            .then_some(note.message.as_str())
    })
}

/// Shared state templates and name keys, resolved through the recorded stack.
pub fn read_state_content(
    layers: &installed_content_core::ResolvedLayers,
) -> std::io::Result<StateContent> {
    let defines = installed_content_core::defines::read(layers)?;
    let (states, names, warnings) = read_state_templates(layers, &defines)?;
    Ok(StateContent {
        defines,
        states,
        names,
        warnings,
    })
}

fn read_state_templates(
    layers: &installed_content_core::ResolvedLayers,
    defines: &installed_content_core::defines::Defines,
) -> std::io::Result<(
    installed_content_states::states::States,
    StateNames,
    Vec<installed_content_states::states::Diagnostic>,
)> {
    let map = installed_content_core::terrain::read_default_map(layers)?;
    let provinces =
        installed_content_core::map::parse_definitions(&layers.read(&map.file("definitions")?)?)?;
    let (states, warnings) =
        installed_content_states::states::read_with_diagnostics(layers, defines, &provinces)?;
    let names = states
        .definitions()
        .iter()
        .map(|(&id, state)| (id, state.name_key.clone()))
        .collect();
    Ok((states, names, warnings))
}

type Database<T> = installed_content_core::content::diagnostics::Once<(Result<T, String>, Vec<String>)>;

pub struct Environment {
    prepared: prepared::PreparedInputs,
    // Shared defines stay independent of lazy map inputs.
    defines: installed_content_core::defines::Defines,
    layers: installed_content_core::ResolvedLayers,
    ideologies: Database<installed_content_core::ideologies::Ideologies>,
    terrain_definitions: Database<installed_content_core::terrain::TerrainDefinitions>,
    leader_traits: Database<installed_content_core::leader_traits::LeaderTraits>,
    ideas: Database<installed_content_core::ideas::IdeaContent>,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) inputs: std::sync::OnceLock<Result<EnvironmentInputs, String>>,
}

impl Environment {
    pub(super) fn read(layers: &installed_content_core::ResolvedLayers) -> std::io::Result<Self> {
        let defines = installed_content_core::defines::read(layers)?;
        Ok(Self {
            prepared: Default::default(),
            defines,
            layers: layers.clone(),
            ideologies: Default::default(),
            terrain_definitions: Default::default(),
            leader_traits: Default::default(),
            ideas: Default::default(),
            #[cfg(any(test, feature = "test-support"))]
            inputs: std::sync::OnceLock::new(),
        })
    }

    pub(crate) fn catalogue_notes(&self) -> Vec<String> {
        [
            self.ideologies.get().map(|db| &db.1),
            self.terrain_definitions.get().map(|db| &db.1),
            self.leader_traits.get().map(|db| &db.1),
        ]
        .into_iter()
        .flatten()
        .flatten()
        .cloned()
        .collect()
    }

    fn ideas(&self) -> Result<&installed_content_core::ideas::IdeaContent, &str> {
        self.ideas
            .get_or_init(|| {
                let loaded = (|| {
                    let officers = self
                        .defines
                        .strings("NCharacter", "OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU")?;
                    installed_content_core::ideas::read(&self.layers, &officers)
                })();
                match loaded {
                    Ok((ideas, _, notes)) => {
                        (Ok(ideas), notes.into_iter().map(|(_, why)| why).collect())
                    }
                    Err(error) => (Err(error.to_string()), Vec::new()),
                }
            })
            .0
            .as_ref()
            .map_err(String::as_str)
    }

    pub(crate) fn defines(&self) -> &installed_content_core::defines::Defines {
        &self.defines
    }

    pub(crate) fn ideologies(&self) -> Result<&installed_content_core::ideologies::Ideologies, String> {
        self.ideologies_result().as_ref().map_err(Clone::clone)
    }

    pub(crate) fn terrain(&self) -> Result<&installed_content_core::terrain::TerrainDefinitions, String> {
        self.terrain_result().as_ref().map_err(Clone::clone)
    }

    pub(crate) fn leader_traits(
        &self,
    ) -> Result<&installed_content_core::leader_traits::LeaderTraits, String> {
        self.leader_traits_result().as_ref().map_err(Clone::clone)
    }

    fn ideologies_result(&self) -> &Result<installed_content_core::ideologies::Ideologies, String> {
        &self
            .ideologies
            .get_or_init(|| match installed_content_core::ideologies::read(&self.layers) {
                Ok((db, notes)) => (Ok(db), notes.into_iter().map(|(_, why)| why).collect()),
                Err(why) => (Err(why.to_string()), Vec::new()),
            })
            .0
    }

    fn terrain_result(&self) -> &Result<installed_content_core::terrain::TerrainDefinitions, String> {
        &self
            .terrain_definitions
            .get_or_init(
                || match installed_content_core::terrain::read_definitions(&self.layers) {
                    Ok(db) => {
                        let notes = db.skipped.clone();
                        (Ok(db), notes)
                    }
                    Err(why) => (Err(why.to_string()), Vec::new()),
                },
            )
            .0
    }

    fn leader_traits_result(&self) -> &Result<installed_content_core::leader_traits::LeaderTraits, String> {
        &self
            .leader_traits
            .get_or_init(|| match installed_content_core::leader_traits::read(&self.layers) {
                Ok((db, notes)) => (Ok(db), notes.into_iter().map(|(_, why)| why).collect()),
                Err(why) => (Err(why.to_string()), Vec::new()),
            })
            .0
    }

    pub(crate) fn warnings(&self) -> &[String] {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(Ok(inputs)) = self.inputs.get() {
            return &inputs.warnings;
        }
        self.prepared_warnings()
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(super) struct EnvironmentInputs {
    pub warnings: Vec<String>,
    pub(super) state_names: BTreeMap<u32, Result<String, String>>,
    state_provinces: BTreeMap<u32, Vec<u32>>,
    state_error: Option<String>,
    continents: Result<Vec<String>, String>,
    impassable: BTreeSet<u32>,
    neighbors: installed_content_core::content::diagnostics::Once<Result<BTreeMap<u32, Vec<u32>>, String>>,
    terrain:
        installed_content_core::content::diagnostics::Once<Option<installed_content_core::movement::MovementMap>>,
    terrain_names: Result<Vec<Vec<u8>>, String>,
    dlcs: BTreeSet<String>,
    triggers: Result<installed_content_core::trigger_keys::TriggerNames, String>,
    aliases: std::sync::Arc<installed_content_core::tag_aliases::TagAliases>,
    buildings: Result<installed_content_core::buildings::Buildings, String>,
    bonuses: Result<[i64; 2], String>,
    roles: Result<LeaderRoles, String>,
    invasion_days: Result<i32, String>,
    constants: Result<installed_content_core::script_constants::Constants, String>,
    /// Registered unit-leader traits; country-leader traits stay in Environment.
    traits: Result<installed_content_core::unit_leader::Traits, String>,
}

#[cfg(any(test, feature = "test-support"))]
impl EnvironmentInputs {
    fn facts<'a>(&'a self, environment: &'a Environment) -> ContentFacts<'a> {
        ContentFacts {
            state_provinces: &self.state_provinces,
            state_content: self.state_error.as_deref().map_or(Ok(()), Err),
            continents: self.continents.as_deref().map_err(String::as_str),
            ideologies: environment
                .ideologies_result()
                .as_ref()
                .map_err(String::as_str),
            impassable: &self.impassable,
            neighbor_states: self.neighbors.get().map_or(
                Err("recorded-content neighbor states have not been requested"),
                |neighbors| neighbors.as_ref().map_err(String::as_str),
            ),
            terrain: self.terrain.get().and_then(Option::as_ref),
            dlcs: &self.dlcs,
            triggers: self.triggers.as_ref().map_err(String::as_str),
            tag_aliases: &self.aliases,
            buildings: self.buildings.as_ref().map_err(String::as_str),
            resource_bonuses: self.bonuses.as_ref().copied().map_err(String::as_str),
            leader_roles: self.roles.as_ref().map_err(String::as_str),
            invasion_prepare_days: self.invasion_days.as_ref().copied().map_err(String::as_str),
            script_constants: self.constants.as_ref().map_err(String::as_str),
            character_traits: self
                .traits
                .as_ref()
                .and_then(|units| {
                    environment
                        .leader_traits_result()
                        .as_ref()
                        .map(|country| (units, country))
                })
                .map_err(String::as_str),
        }
    }

    fn geography<'a>(
        &'a self,
        content: &'a ScriptedContent,
        environment: &'a Environment,
    ) -> GeographyInputs<'a> {
        GeographyInputs {
            neighbors: &self.neighbors,
            terrain: &self.terrain,
            layers: &content.layers.layers,
            state_provinces: &self.state_provinces,
            state_error: self.state_error.as_deref(),
            defines: environment.defines(),
            terrain_definitions: environment.terrain_result().as_ref().ok(),
        }
    }

    fn world<'a>(
        &'a self,
        content: &'a ScriptedContent,
        environment: &'a Environment,
        frame: replay_frame::frame::HourFrame<'a>,
    ) -> World<'a> {
        World::read_in(frame, self.facts(environment))
            .with_geography(self.geography(content, environment))
            .with_template_names(
                &content.template_names,
                &content.layers.layers,
                &content.recorded_language,
            )
    }
}

impl ScriptedContent {
    /// Content-load callers use a fresh empty scope, without a game
    /// hour. Reuse this cached content environment; runtime operands remain
    /// unavailable instead of borrowing a later recording read.
    pub fn unrecorded_trigger_world(&self) -> Result<World<'_>, String> {
        let environment = self.environment()?;
        let dlcs = environment
            .prepared
            .dlcs
            .get_or_init(|| self.layers.layers.dlc_names().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(Clone::clone)?;
        static TRIGGERS: std::sync::OnceLock<installed_content_core::trigger_keys::TriggerNames> =
            std::sync::OnceLock::new();
        Ok(World::unrecorded(
            dlcs,
            TRIGGERS.get_or_init(Default::default),
            &self.tag_aliases,
        )
        .with_prepared_content(self))
    }

    pub(crate) fn cached_states(&self) -> Result<&StateContent, &str> {
        crate::catalogue::used(crate::catalogue::Reader::States);
        self.state_content
            .get_or_init(|| {
                read_state_content(&self.layers.layers).map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(String::as_str)
    }
}

/// Preserve each caller's established diagnostics without registering a second refusal.
#[derive(Clone, Copy)]
pub enum StateNameUsage {
    Place,
    Occupation,
    Raid,
}

/// State tables and place labels share stored-language and template precedence.
pub fn state_display_name(
    state: &replay_data::rows::StateRow,
    cat: &crate::catalogue::Catalogue,
    names: Result<&StateNames, &str>,
    usage: StateNameUsage,
) -> Result<String, String> {
    state_name_from(
        state,
        |bytes, what| super::stored_text(&cat.localisation, &cat.recorded_language, bytes, what),
        |id| {
            let key = names
                .map_err(str::to_owned)?
                .get(&id)
                .ok_or_else(|| match usage {
                    StateNameUsage::Place => {
                        format!("state {id}: No recorded-content state template")
                    }
                    StateNameUsage::Occupation => "No recorded-content state template".into(),
                    StateNameUsage::Raid => format!("state {id} has no content name"),
                })?
                .as_ref()
                .map_err(Clone::clone)?;
            if matches!(usage, StateNameUsage::Raid) {
                crate::catalogue::localised(&cat.localisation, key, crate::catalogue::View::Plain)
                    .map_err(|_| format!("state {id} name {key} is refused"))
            } else {
                cat.localisation
                    .localise(key.as_bytes())
                    .map_err(|error| error.to_string())
            }
        },
    )
    .and_then(|text| crate::catalogue::plain_text(&text))
}

impl ScriptedContent {
    /// The same lazy content inputs as scripted text, retaining recorded table gaps.
    pub fn recorded_world<'a>(
        &'a self,
        frame: &'a dyn replay_frame::frame::DecodedSource,
    ) -> Result<World<'a>, String> {
        self.lazy_world(replay_frame::frame::HourFrame::Decoded(frame))
    }

    /// [`Self::recorded_world`] of any frame's tables, a walk's views among them.
    pub fn recorded_world_in<'a>(
        &'a self,
        frame: replay_frame::frame::HourFrame<'a>,
    ) -> Result<World<'a>, String> {
        self.lazy_world(frame)
    }
}

impl<'a> ScriptedText<'a> {
    /// The hour's trigger inputs; missing tables refuse only predicates that use them.
    pub fn trigger_world(&self) -> Result<&World<'a>, String> {
        self.world
            .get_or_init(|| {
                crate::triggers::inputs::unrecorded(|| {
                    self.content.lazy_world(self.frame)
                })
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Text expansion requires complete core scope tables.
    pub(crate) fn world(&self) -> Result<&World<'a>, String> {
        let world = self.trigger_world()?;
        match world.core_gap() {
            Some(gap) => Err(gap.to_string()),
            None => Ok(world),
        }
    }

    pub(super) fn entry(
        &self,
        object: Object,
        name: &str,
        scopes: &ExpansionScopes<'_, 'a>,
        active: usize,
        event_active: bool,
    ) -> Result<String, String> {
        if scopes.needs_large_stack() {
            return self.on_large_stack(scopes, |scopes| {
                self.entry(object, name, scopes, active, event_active)
            });
        }
        if let Some(unknown) = self.content.names.uncertain(name.as_bytes()) {
            return Err(format!(
                "{name}: scripted localisation definitions are uncertain: {unknown}"
            ));
        }
        let Some(entry) = self.content.names.entries.get(name.as_bytes()) else {
            return Ok(String::new());
        };
        let previous_len = scopes.scopes.borrow().len();
        let at = bound_scope(object, scopes, active, event_active);
        let bare = object != Object::Event;
        let repeated = scopes
            .active
            .borrow()
            .iter()
            .any(|entry| entry.matches(name, scopes, at, bare));
        scopes.active.borrow_mut().push(ActiveEntry {
            name: name.to_owned(),
            at,
            scopes_len: scopes.scopes.borrow().len(),
            bare,
            variables: scopes.variables.get(),
        });
        let _guard = EntryGuard(scopes, previous_len);
        if repeated {
            return Err(format!(
                "scripted localisation {name}: cyclic entry invocation"
            ));
        }
        if scopes.active.borrow().len() + scopes.fresh_depth.get() > MAX_SCRIPTED_NESTING {
            return Err(nesting_refusal(name));
        }
        // Check alternatives in order; closing the variable context
        // leaves the checked trigger's temps for a later value.
        let mut checked_trigger = false;
        let mut evaluator = None;
        for text in &entry.texts {
            // Skip a null value without checking its trigger.
            let Some(value) = &text.value else { continue };
            let trigger = &text.trigger;
            if text.trigger_present {
                checked_trigger = true;
                // Even an empty trigger opens the next context.
                let projected = entry_scopes(scopes, at, bare);
                evaluator = Some(scopes.evaluator(self, Temps::Fresh)?);
                let checked = evaluator.as_ref().unwrap().evaluate(trigger, projected);
                scopes
                    .session
                    .replace(Some(evaluator.as_ref().unwrap().variable_session()));
                match checked {
                    Eval::False => continue,
                    Eval::True => {}
                    Eval::Unknown(reasons) => {
                        return Err(format!(
                            "scripted localisation {name}: {}",
                            reasons.into_iter().collect::<Vec<_>>().join("; ")
                        ));
                    }
                }
            }
            let key = match value {
                installed_content_scripted_localisation::scripted_localisation::TextValue::Key(key) => {
                    key.as_slice()
                }
                installed_content_scripted_localisation::scripted_localisation::TextValue::RandomList(
                    list,
                ) => {
                    let key = self
                        .random_key(
                            list,
                            scopes,
                            at,
                            entry_scopes(scopes, at, bare),
                            evaluator.as_ref(),
                            if checked_trigger {
                                Temps::Fresh
                            } else {
                                Temps::Stale
                            },
                        )
                        .map_err(|e| format!("scripted localisation {name}: {e}"))?;
                    let Some(key) = key.filter(|key| !key.is_empty()) else {
                        return Ok(String::new());
                    };
                    key
                }
            };
            // The key's commands are recorded under it, apart from those of the text selecting it.
            let outer = scopes.select(key);
            let expanded = self
                .localisation
                .localise(key)
                .map_err(|e| e.to_string())
                // A new text context in the evaluation scope starts at depth
                // zero; the text processor's own depth rule still applies.
                .and_then(|text| self.process(&text, 0, scopes, at, Object::Event));
            scopes.leave(outer);
            return expanded;
        }
        // No alternative selected: return empty localisation.
        Ok(String::new())
    }

    pub(crate) fn state_name(&self, id: u32) -> Result<String, String> {
        self.state_name_for(id, false)
    }

    /// War naming copies the already-localised display name without
    /// scripted-text processing.
    pub fn plain_state_name(&self, id: u32) -> Result<String, String> {
        self.state_name_for(id, true)
    }

    fn state_name_for(&self, id: u32, plain: bool) -> Result<String, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::StateNameFor(id, plain)),
            || self.state_name_for_unrecorded(id, plain),
        )
    }

    pub(crate) fn state_name_for_unrecorded(&self, id: u32, plain: bool) -> Result<String, String> {
        let (states, gap) = rows::<tables::States>(self.frame, STATE);
        let state = keyed(&states, u128::from(id))?.ok_or_else(|| format!("state {id}: states: {gap}"))?;
        state_name_from(
            &state,
            |bytes, what| self.stored_text(bytes, what),
            |id| {
                let key = state_template_key(self.content, id)?;
                if plain {
                    crate::catalogue::localised(
                        self.localisation,
                        key,
                        crate::catalogue::View::Plain,
                    )
                } else {
                    self.localise(key)
                }
            },
        )
    }
}

/// The name key of state `id`'s recorded-content template.
fn state_template_key(content: &ScriptedContent, id: u32) -> Result<&String, String> {
    static EMPTY_STATE_NAMES: StateNames = BTreeMap::new();
    content.environment()?;
    #[cfg(any(test, feature = "test-support"))]
    let names = match content.environment()?.inputs.get() {
        Some(Ok(inputs)) => &inputs.state_names,
        _ => content
            .cached_states()
            .map(|states| &states.names)
            .unwrap_or(&EMPTY_STATE_NAMES),
    };
    #[cfg(not(any(test, feature = "test-support")))]
    let names = content
        .cached_states()
        .map(|states| &states.names)
        .unwrap_or(&EMPTY_STATE_NAMES);
    let definition = names
        .get(&id)
        .ok_or_else(|| format!("GetName: state {id} has no recorded-content template"))?;
    definition.as_ref().map_err(Clone::clone)
}

/// [`ScriptedText::plain_state_name`] of state `id` from its recorded custom and owner names,
/// for a reader that holds those two cells rather than a frame's States row.
pub fn plain_state_name_of(
    localisation: &Localisation,
    content: &ScriptedContent,
    id: u32,
    custom_name: &[u8],
    owner_name: &[u8],
) -> Result<String, String> {
    let state = replay_data::rows::StateRow {
        id,
        custom_name: custom_name.to_vec(),
        owner_name: owner_name.to_vec(),
        ..Default::default()
    };
    state_name_from(
        &state,
        |bytes, what| super::stored_text(localisation, &content.recorded_language, bytes, what),
        |id| {
            crate::catalogue::localised(
                localisation,
                state_template_key(content, id)?,
                crate::catalogue::View::Plain,
            )
        },
    )
}

impl<'a> ScriptedText<'a> {

    /// Primary province display names, also used by the combat-history location.
    pub fn province_display_name_primary(&self, id: u32) -> Result<Option<String>, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::ProvinceDisplayNamePrimary(id)),
            || self.province_display_name_primary_unrecorded(id),
        )
    }

    pub(crate) fn province_display_name_primary_unrecorded(
        &self,
        id: u32,
    ) -> Result<Option<String>, String> {
        province_primary_name(self.localisation, self.province_name_override(id), || {
            self.province_victory_point_name(id)
        })
    }

    /// Effect/map display names, distinct from province GetName.
    pub(crate) fn province_display_name(&self, id: u32) -> Result<String, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::ProvinceDisplayName(id)),
            || self.province_display_name_unrecorded(id),
        )
    }

    pub(crate) fn province_display_name_unrecorded(&self, id: u32) -> Result<String, String> {
        if let Some(text) = self.province_display_name_primary(id)? {
            return Ok(text);
        }
        let (provinces, gap) = rows::<tables::Provinces>(self.frame, PROVINCE);
        let province = keyed(&provinces, u128::from(id))?
            .ok_or_else(|| format!("province {id}: provinces: {gap}"))?;
        if province.state != 0 {
            self.state_name(province.state)
        } else {
            Err(format!(
                "province {id} effect display fallback needs unrecorded province strategic-region pointer/nullness and region instance name"
            ))
        }
    }

    fn province_name_override(&self, id: u32) -> Result<Option<&[u8]>, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::ProvinceNameOverride(id)),
            || self.province_name_override_unrecorded(id),
        )
    }

    pub(crate) fn province_name_override_unrecorded(
        &self,
        id: u32,
    ) -> Result<Option<&[u8]>, String> {
        let (names, gap) = rows::<tables::ProvinceNames>(self.frame, PROVINCE_NAME);
        if let Some(name) = names
            .text(u128::from(id), |row| &row.name, PROVINCE_NAME_COLUMN)
            .map_err(built)?
        {
            Ok(Some(name))
        } else if complete::<tables::ProvinceNames>(self.frame) {
            Ok(None)
        } else {
            Err(format!("GetName of province {id}: province names: {gap}"))
        }
    }

    pub(crate) fn province_victory_point_name(&self, id: u32) -> Result<String, String> {
        crate::triggers::inputs::query(
            || crate::triggers::inputs::Key::Text(TextQuery::ProvinceVictoryPointName(id)),
            || self.province_victory_point_name_unrecorded(id),
        )
    }

    pub(crate) fn province_victory_point_name_unrecorded(&self, id: u32) -> Result<String, String> {
        let (provinces, gap) = rows::<tables::Provinces>(self.frame, PROVINCE);
        let province = keyed(&provinces, u128::from(id))?
            .ok_or_else(|| format!("province {id}: provinces: {gap}"))?;
        // Use the state's owner original tag or tag, not the
        // province controller. A null state uses the country at slot zero.
        let owner = if province.state == 0 {
            0
        } else {
            let (states, gap) = rows::<tables::States>(self.frame, STATE);
            let state = keyed(&states, u128::from(province.state))?
                .ok_or_else(|| {
                    format!(
                        "province {id}: state {} has no States row: {gap}",
                        province.state
                    )
                })?;
            if state.owner == 0 {
                0
            } else {
                self.scopes.slot(state.owner).ok_or_else(|| {
                    format!(
                        "province {id}: state owner's link {} resolves to no country",
                        state.owner
                    )
                })?
            }
        };
        let tag =
            installed_content_country_names::country_names::original_or_own_tag_in(&self.frame, owner)?;
        victory_point_name(self.localisation, id, &tag)
    }

    pub(crate) fn province_terrain(&self, id: u32) -> Result<i32, String> {
        crate::triggers::PreparedContent::geography(self.content).terrain().and_then(|map| map.terrain(id))
            .map(|terrain| terrain.index)
            .ok_or_else(|| format!("Terrain of province {id}: the recorded-content province template is unavailable"))
    }

    /// Localised terrain ID, also used by raid target naming.
    pub(crate) fn terrain_id_name(&self, ordinal: i32) -> Result<String, String> {
        // Null terrain has an empty ID.
        if ordinal == 0 {
            return Ok(String::new());
        }
        let environment = self.content.environment()?;
        let at = ordinal
            .checked_sub(1)
            .and_then(|at| usize::try_from(at).ok());
        let missing = || {
            format!(
                "Terrain.GetName: native terrain ordinal {ordinal} has no recorded-content category"
            )
        };
        #[cfg(any(test, feature = "test-support"))]
        if let Some(Ok(inputs)) = environment.inputs.get() {
            let names = inputs.terrain_names.as_ref().map_err(Clone::clone)?;
            let name = at.and_then(|at| names.get(at)).ok_or_else(missing)?;
            return self.localisation.localise(name).map_err(|e| e.to_string());
        }
        let terrain = environment.terrain()?;
        let name = at
            .and_then(|at| terrain.categories.get(at))
            .ok_or_else(missing)?;
        self.localisation
            .localise(&name.name)
            .map_err(|e| e.to_string())
    }
}

/// A province's primary display name: its nonempty recorded rename localised, even to empty
/// text; else its nonempty victory-point name; else none.
pub fn province_primary_name(
    localisation: &Localisation,
    rename: Result<Option<&[u8]>, String>,
    victory_point: impl FnOnce() -> Result<String, String>,
) -> Result<Option<String>, String> {
    if let Some(raw) = rename?.filter(|raw| !raw.is_empty()) {
        return localisation
            .localise(raw)
            .map(Some)
            .map_err(|e| e.to_string());
    }
    let text = victory_point()?;
    Ok((!text.is_empty()).then_some(text))
}

/// Province `id`'s victory-point name under its state owner's original-or-own `tag`: the
/// owner's key, else the plain one, whichever is defined first; empty when neither is.
pub fn victory_point_name(localisation: &Localisation, id: u32, tag: &str) -> Result<String, String> {
    for key in [
        format!("{tag}_VICTORY_POINTS_{id}"),
        format!("VICTORY_POINTS_{id}"),
    ] {
        if localisation.get(&key).is_some() {
            return localisation
                .localise(key.as_bytes())
                .map_err(|refusal| refusal.to_string());
        }
    }
    Ok(String::new())
}

/// Shared with state tables: name precedence without another ID scan.
pub(crate) fn state_name_from(
    state: &replay_data::rows::StateRow,
    stored: impl Fn(&[u8], &str) -> Result<String, String>,
    template: impl Fn(u32) -> Result<String, String>,
) -> Result<String, String> {
    let id = state.id;
    // The custom name, else the owner name, else the template's. Setting
    // a state name stores the localised text as the custom name. Setting
    // the owner, also on load, stores the localised
    // `<owner original tag or tag>_<template name key>` as the owner name,
    // or clears it when the key is missing.
    for (bytes, what) in [
        (&state.custom_name, "the custom name"),
        (&state.owner_name, "the owner name"),
    ] {
        if !bytes.is_empty() {
            return stored(bytes, &format!("GetName of state {id}: {what}"));
        }
    }
    template(id)
}

/// The event scope an entry is evaluated in, as an index into the
/// expansion's scope arena. With no promotion it is the active scope itself;
/// otherwise bind the promoted object as THIS in a new scope with the
/// active scope's ROOT and FROM, and the active scope as PREV. A country
/// or state is set with clearing, so that scope carries no character.
fn bound_scope(
    object: Object,
    scopes: &ExpansionScopes,
    active: usize,
    event_active: bool,
) -> usize {
    let this = match object {
        Object::Event => return active,
        // Bind this text object's own country with clearing. Actual
        // Country0 stays a country text object, but binding its raw zero
        // country reference leaves this event scope empty.
        Object::Country(0) => EventObject::State(crate::triggers::NULL_STATE),
        Object::Country(slot) => EventObject::Country(slot),
        Object::State(id) => EventObject::State(id.unwrap_or(crate::triggers::NULL_STATE)),
        Object::Character(id) => EventObject::Character(id),
        Object::FactionEnvironment(_) | Object::Faction(_) => {
            unreachable!("scripted entries do not bind these text kinds")
        }
    };
    // Leave the fresh scope's links alone when the active event is null,
    // including a country promoted from the faction environment.
    let current = if event_active {
        scopes.scope(active)
    } else {
        EventScope::of(this)
    };
    let carried = |link| match link {
        Link::Itself if event_active => Link::Scope(active),
        other => other,
    };
    let distance = if !event_active {
        None
    } else if this != current.this {
        Some(1)
    } else {
        scopes.prev_distance.borrow()[active].map(|distance| distance + 1)
    };
    let mut arena = scopes.scopes.borrow_mut();
    let at = arena.len();
    arena.push(EventScope {
        this,
        root: carried(current.root),
        from: carried(current.from),
        prev: if event_active {
            Link::Scope(active)
        } else {
            Link::Itself
        },
        random_seed: super::random::scope_seed(424242),
        random_counter: super::random::scope_counter(424242),
        random_seed_refusal: None, // Construct a known fresh seed.
        clock_seed: None,
    });
    scopes.prev_distance.borrow_mut().push(distance);
    at
}

/// The trigger scopes of event scope `at`, `bare` when its THIS was set with
/// clearing ([`bound_scope`]). A character is a character scope
/// (`crate::character_scope`). A link the caller cannot describe is unknown
/// to the evaluator, so only a condition that reads it is unknown (`always`
/// reads none).
pub(super) fn entry_scopes(scopes: &ExpansionScopes, at: usize, bare: bool) -> Scopes {
    let scope = |object| match object {
        EventObject::Country(slot) | EventObject::CountryState { country: slot, .. } => {
            Scope::Country(slot)
        }
        EventObject::Combat => Scope::Combat,
        EventObject::State(id) => Scope::State(id),
        EventObject::Province(id) => Scope::Province(id),
        EventObject::UnitLeader(reference) => Scope::Leader(reference),
        EventObject::CountryUnitLeader { country, leader } => {
            Scope::CountryLeader { country, leader }
        }
        EventObject::Character(id) => Scope::Character(id),
        EventObject::IndustrialOrganisation(id) => Scope::IndustrialOrganisation(id),
        EventObject::Project { owner, id } => Scope::Project { owner, id },
        EventObject::Raid(id) => Scope::Raid(id),
        EventObject::Unit(id) => Scope::Unit(id),
    };
    let current = scopes.scope(at);
    let this = scope(current.this);
    let linked = |link| {
        scopes
            .linked(at, link, "")
            .ok()
            .map(|to| scope(scopes.scope(to).this))
    };
    let (root, from, prev) = (
        linked(current.root),
        linked(current.from),
        linked(current.prev),
    );
    Scopes {
        root: root.unwrap_or(this),
        this,
        prev,
        from,
        unknown: crate::triggers::UnknownLinks {
            root: root.is_none(),
            from: from.is_none(),
            prev: prev.is_none(),
        },
        bare,
        target_state: scopes.variables.get().target_state,
        target_province: scopes.variables.get().target_province,
        target_leader: scopes.variables.get().target_leader,
        cursor: Some(at),
    }
}

#[cfg(any(test, feature = "test-support"))]
#[test]
fn entry_invocations_keep_the_original_bounds_of_unresolved_links() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let scopes = ExpansionScopes::new(&EventScopes::new(vec![EventScope {
        prev: Link::Scope(1),
        ..EventScope::of(EventObject::Country(0))
    }]));
    let entry = ActiveEntry {
        name: "X".into(),
        at: 0,
        scopes_len: 1,
        bare: false,
        variables: scopes.variables.get(),
    };
    bound_scope(Object::Country(1), &scopes, 0, true);
    assert!(!entry.matches("X", &scopes, 0, false));
}

#[cfg(any(test, feature = "test-support"))]
impl EnvironmentInputs {
    pub(super) fn synthetic() -> Self {
        Self {
            warnings: Vec::new(),
            state_names: BTreeMap::new(),
            state_provinces: BTreeMap::new(),
            state_error: None,
            continents: Err("no synthetic continent content is supplied".into()),
            impassable: BTreeSet::new(),
            neighbors: installed_content_core::content::diagnostics::Once::from(Ok(BTreeMap::new())),
            terrain: installed_content_core::content::diagnostics::Once::from(None),
            terrain_names: Ok(Vec::new()),
            dlcs: BTreeSet::new(),
            triggers: Ok(Default::default()),
            aliases: Default::default(),
            buildings: Ok(Default::default()),
            bonuses: Ok([0; 2]),
            roles: Ok(LeaderRoles::new(Vec::new())),
            invasion_days: Ok(0),
            constants: Ok(Default::default()),
            traits: Ok(Default::default()),
        }
    }

    /// [`Self::synthetic`] with the content's tag aliases.
    #[cfg(test)]
    pub(super) fn with_aliases(
        self,
        aliases: std::sync::Arc<installed_content_core::tag_aliases::TagAliases>,
    ) -> Self {
        Self { aliases, ..self }
    }

    #[cfg(test)]
    pub(super) fn with_triggers(
        self,
        triggers: installed_content_core::trigger_keys::TriggerNames,
    ) -> Self {
        Self {
            triggers: Ok(triggers),
            ..self
        }
    }
}
