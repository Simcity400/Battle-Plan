//! Ordered scripted entries and content used by recorded triggers.
use super::*;
use crate::triggers::{ContentFacts, Eval, LeaderRoles, Scope, Scopes, Temps, World};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
mod tests;

pub(crate) type StateNames = BTreeMap<u32, Result<String, String>>;

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
        // ROOT/FROM/PREV can themselves be promoted (141A81250); only
        // THIS's seed is read here (1410CDDF0 gives entries a fresh seed).
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

pub(crate) struct StateContent {
    pub defines: installed_content::defines::Defines,
    pub states: installed_content::states::States,
    pub names: StateNames,
    pub warnings: Vec<installed_content::states::Diagnostic>,
}

pub(crate) fn live_state_warnings(
    notes: &[installed_content::states::Diagnostic],
) -> impl Iterator<Item = &str> {
    use installed_content::states::MisreadCause;
    // States' Warning channel contains 140AAD450's nonpositive-ID and
    // duplicate-ID skips. Both keep exactly the templates the game keeps.
    // Only the explicitly cited state-field recoveries are proven exact;
    // shared reader notes and unported readers still warn.
    notes.iter().filter_map(|note| {
        let exact = note.kind == installed_content::DiagnosticKind::Misread
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
        (note.kind != installed_content::DiagnosticKind::Warning && !exact).then_some(note.message.as_str())
    })
}

/// Shared state templates and name keys, resolved through the recorded stack.
pub(crate) fn read_state_content(
    layers: &installed_content::ResolvedLayers,
) -> std::io::Result<StateContent> {
    let defines = installed_content::defines::read(layers)?;
    let (states, names, warnings) = read_state_templates(layers, &defines)?;
    Ok(StateContent {
        defines,
        states,
        names,
        warnings,
    })
}

fn read_state_templates(
    layers: &installed_content::ResolvedLayers,
    defines: &installed_content::defines::Defines,
) -> std::io::Result<(
    installed_content::states::States,
    StateNames,
    Vec<installed_content::states::Diagnostic>,
)> {
    let map = installed_content::terrain::read_default_map(layers)?;
    let provinces = installed_content::map::parse_definitions(&layers.read(&map.file("definitions")?)?)?;
    let (states, warnings) = installed_content::states::read_with_diagnostics(layers, defines, &provinces)?;
    let names = states
        .definitions()
        .iter()
        .map(|(&id, state)| (id, state.name_key.clone()))
        .collect();
    Ok((states, names, warnings))
}

pub(crate) struct Environment {
    // LoadGameDefines 14073DDA0; shared content stays independent of lazy map inputs.
    defines: installed_content::defines::Defines,
    ideologies: Result<installed_content::ideologies::Ideologies, String>,
    terrain_definitions: Result<installed_content::terrain::TerrainDefinitions, String>,
    leader_traits: Result<installed_content::leader_traits::LeaderTraits, String>,
    pub(crate) catalogue_notes: Vec<String>,
    pub(super) inputs: std::sync::OnceLock<Result<EnvironmentInputs, String>>,
}

impl Environment {
    pub(super) fn read(layers: &installed_content::ResolvedLayers) -> std::io::Result<Self> {
        let defines = installed_content::defines::read(layers)?;
        let mut catalogue_notes = Vec::new();
        let ideologies = installed_content::ideologies::read(layers)
            .map(|(db, notes)| {
                catalogue_notes.extend(notes.into_iter().map(|(_, why)| why));
                db
            })
            .map_err(|why| why.to_string());
        let terrain_definitions = installed_content::terrain::read_definitions(layers)
            .inspect(|db| catalogue_notes.extend(db.skipped.iter().cloned()))
            .map_err(|why| why.to_string());
        let leader_traits = installed_content::leader_traits::read(layers)
            .map(|(db, notes)| {
                catalogue_notes.extend(notes.into_iter().map(|(_, why)| why));
                db
            })
            .map_err(|why| why.to_string());
        Ok(Self {
            defines,
            ideologies,
            terrain_definitions,
            leader_traits,
            catalogue_notes,
            inputs: std::sync::OnceLock::new(),
        })
    }

    pub(crate) fn defines(&self) -> &installed_content::defines::Defines {
        &self.defines
    }

    pub(crate) fn ideologies(&self) -> Result<&installed_content::ideologies::Ideologies, String> {
        self.ideologies.as_ref().map_err(Clone::clone)
    }

    pub(crate) fn terrain(&self) -> Result<&installed_content::terrain::TerrainDefinitions, String> {
        self.terrain_definitions.as_ref().map_err(Clone::clone)
    }

    pub(crate) fn leader_traits(&self) -> Result<&installed_content::leader_traits::LeaderTraits, String> {
        self.leader_traits.as_ref().map_err(Clone::clone)
    }

    fn inputs(&self, content: &ScriptedContent) -> Result<&EnvironmentInputs, String> {
        self.inputs
            .get_or_init(|| {
                EnvironmentInputs::read(content, self)
                    .map_err(|why| format!("scripted localisation content: {why}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    pub(crate) fn warnings(&self) -> &[String] {
        self.inputs
            .get()
            .and_then(|inputs| inputs.as_ref().ok())
            .map_or(&[], |inputs| inputs.warnings.as_slice())
    }
}

pub(super) struct EnvironmentInputs {
    pub warnings: Vec<String>,
    pub(super) state_names: BTreeMap<u32, Result<String, String>>,
    state_provinces: BTreeMap<u32, Vec<u32>>,
    state_error: Option<String>,
    continents: Result<Vec<String>, String>,
    impassable: BTreeSet<u32>,
    neighbors: Result<BTreeMap<u32, Vec<u32>>, String>,
    terrain: Option<installed_content::movement::MovementMap>,
    terrain_names: Result<Vec<Vec<u8>>, String>,
    dlcs: BTreeSet<String>,
    triggers: Result<installed_content::trigger_keys::TriggerNames, String>,
    aliases: std::sync::Arc<installed_content::tag_aliases::TagAliases>,
    buildings: Result<installed_content::buildings::Buildings, String>,
    bonuses: Result<[i64; 2], String>,
    roles: Result<LeaderRoles, String>,
    invasion_days: Result<i32, String>,
    constants: Result<installed_content::script_constants::Constants, String>,
    /// Registered unit-leader traits; country-leader traits stay in Environment.
    traits: Result<installed_content::unit_leader::Traits, String>,
}

impl EnvironmentInputs {
    fn read(content: &ScriptedContent, environment: &Environment) -> std::io::Result<Self> {
        let layers = &content.layers.layers;
        let defines = environment.defines();
        // Map and count content failures belong to the predicates that read
        // them. A recorded political predicate needs none of those assets.
        let mut warnings = Vec::new();
        let mut state_provinces = BTreeMap::new();
        let mut state_names = BTreeMap::new();
        let mut impassable = BTreeSet::new();
        let state_error = match content.cached_states() {
            Ok(content) => {
                warnings.extend(live_state_warnings(&content.warnings).map(str::to_owned));
                state_names.clone_from(&content.names);
                for (&id, state) in content.states.definitions() {
                    state_provinces.insert(id, state.provinces.clone());
                    if state.impassable {
                        impassable.insert(id);
                    }
                }
                None
            }
            Err(why) => Some(why.to_owned()),
        };
        let buildings = installed_content::buildings::read(layers).map_err(|why| why.to_string());
        let triggers = installed_content::scripted_triggers::read(layers)
            .map(|scripted| {
                let counts = buildings.as_ref().ok().and_then(|buildings| {
                    let ideologies = environment.ideologies().ok()?;
                    let resources = installed_content::resources::read(layers).ok()?;
                    Some(installed_content::trigger_keys::CountTriggerNames::new(
                        buildings,
                        ideologies,
                        &resources.0,
                    ))
                });
                installed_content::trigger_keys::TriggerNames { scripted, counts }
            })
            .map_err(|why| why.to_string());
        let neighbors = match &state_error {
            Some(why) => Err(why.clone()),
            None => installed_content::province_graph::ProvinceGraph::read(layers)
                .and_then(|(graph, _)| graph.neighbor_states(&state_provinces))
                .map_err(|e| e.to_string()),
        };
        let continents = installed_content::terrain::read_default_map(layers)
            .and_then(|map| layers.read(&map.file("continent")?))
            .and_then(|bytes| installed_content::equipment_graphics::context::continent_names(&bytes))
            .map_err(|why| why.to_string());
        Ok(Self {
            warnings,
            state_provinces,
            state_error,
            continents,
            impassable,
            state_names,
            neighbors,
            terrain: environment.terrain().ok().and_then(|terrain| {
                installed_content::movement::MovementMap::read_with_terrain(layers, defines, terrain).ok()
            }),
            terrain_names: environment.terrain().map(|definitions| {
                definitions
                    .categories
                    .iter()
                    .map(|category| category.name.clone())
                    .collect()
            }),
            dlcs: layers.dlc_names()?,
            triggers,
            aliases: std::sync::Arc::new(installed_content::tag_aliases::read(layers)?),
            buildings,
            bonuses: crate::state_resources::resource_bonuses(defines)
                .map_err(|why| why.to_string()),
            // Visible-trigger templates differ from the full character role/name projections.
            roles: installed_content::characters::leader_templates(layers)
                .map(LeaderRoles::new)
                .map_err(|e| e.to_string()),
            invasion_days: defines
                .int32("NNavy", "NAVAL_INVASION_PREPARE_DAYS")
                .map_err(|e| e.to_string()),
            constants: installed_content::script_constants::read(layers).map_err(|e| e.to_string()),
            // Registration alone decides has_trait; the full description/skills read stays separate.
            traits: installed_content::unit_leader::Traits::registered(layers).map_err(|e| e.to_string()),
        })
    }

    fn facts<'a>(&'a self, environment: &'a Environment) -> ContentFacts<'a> {
        ContentFacts {
            state_provinces: &self.state_provinces,
            state_content: self.state_error.as_deref().map_or(Ok(()), Err),
            continents: self.continents.as_deref().map_err(String::as_str),
            ideologies: environment.ideologies.as_ref().map_err(String::as_str),
            impassable: &self.impassable,
            neighbor_states: self.neighbors.as_ref().map_err(String::as_str),
            terrain: self.terrain.as_ref(),
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
                        .leader_traits
                        .as_ref()
                        .map(|country| (units, country))
                })
                .map_err(String::as_str),
        }
    }
}

impl ScriptedContent {
    /// Native content-load callers use a fresh empty scope, without a game
    /// hour. Reuse this cached content environment; runtime operands remain
    /// unavailable instead of borrowing a later recording read.
    pub(crate) fn unrecorded_trigger_world(&self) -> Result<World<'_>, String> {
        let environment = self.environment()?;
        let facts = environment.inputs(self)?.facts(environment);
        Ok(World::unrecorded(
            facts.dlcs,
            facts.triggers.map_err(str::to_owned)?,
            facts.tag_aliases,
        ))
    }

    pub(crate) fn cached_states(&self) -> Result<&StateContent, &str> {
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
pub(crate) enum StateNameUsage {
    Place,
    Occupation,
    Raid,
}

/// State tables and place labels share stored-language and template precedence.
pub(crate) fn state_display_name(
    state: &replay::rows::StateRow,
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
    pub(crate) fn recorded_world<'a>(
        &'a self,
        frame: &'a DecodedFrame,
    ) -> Result<World<'a>, String> {
        let environment = self.environment()?;
        let facts = environment.inputs(self)?.facts(environment);
        Ok(World::read(frame, facts).with_template_names(
            &self.template_names,
            &self.layers.layers,
            &self.recorded_language,
        ))
    }
}

impl<'a> ScriptedText<'a> {
    fn environment(&self) -> Result<&'a EnvironmentInputs, String> {
        self.content.environment()?.inputs(self.content)
    }

    /// The hour's trigger inputs; missing tables refuse only predicates that use them.
    pub(crate) fn trigger_world(&self) -> Result<&World<'a>, String> {
        self.world
            .get_or_init(|| {
                let facts = self.environment()?.facts(self.content.environment()?);
                Ok(World::read(self.frame, facts).with_template_names(
                    &self.content.template_names,
                    &self.content.layers.layers,
                    &self.content.recorded_language,
                ))
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// Text expansion requires complete core scope tables.
    pub(crate) fn world(&self) -> Result<&World<'a>, String> {
        let world = self.trigger_world()?;
        match world.core_gap() {
            Some(gap) => Err(gap),
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
        if !self.content.names.unknown.is_empty() {
            return Err(format!(
                "{name}: scripted localisation definitions are uncertain: {}",
                self.content.names.unknown.join("; ")
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
                "scripted localisation {name}: cyclic entry invocation (1410CF560)"
            ));
        }
        if scopes.active.borrow().len() + scopes.fresh_depth.get() > MAX_SCRIPTED_NESTING {
            return Err(nesting_refusal(name));
        }
        // 140A9CC20 checks alternatives in order; EndScopedVariableContext
        // (140535830) leaves the checked trigger's temps for a later value.
        let mut checked_trigger = false;
        let mut evaluator = None;
        for text in &entry.texts {
            // 140A9CC20 skips a null value without checking its trigger.
            let Some(value) = &text.value else { continue };
            let trigger = &text.trigger;
            if text.trigger_present {
                checked_trigger = true;
                // Even an empty trigger opens the next context (140A9CC20).
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
                installed_content::scripted_localisation::TextValue::Key(key) => key.as_slice(),
                installed_content::scripted_localisation::TextValue::RandomList(list) => {
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
            let text = self.localisation.localise(key).map_err(|e| e.to_string())?;
            // SScopeLocalizer (1402F6340) -> ExpandLocalizationWithEventScope
            // (1410CF560): a new context in the evaluation scope, from depth
            // 0; ProcessString's own depth rule still applies.
            return self.process(&text, 0, scopes, at, Object::Event);
        }
        // The shared empty CBoundLocalization at 143098438.
        Ok(String::new())
    }

    pub(crate) fn state_name(&self, id: u32) -> Result<String, String> {
        self.state_name_for(id, false)
    }

    /// War naming copies display_name's already-localised string without
    /// running CTextBase::ProcessString (140D0BCA0, 140AAF580, 1409CC380).
    pub(crate) fn plain_state_name(&self, id: u32) -> Result<String, String> {
        self.state_name_for(id, true)
    }

    fn state_name_for(&self, id: u32, plain: bool) -> Result<String, String> {
        let (states, gap) = rows::<tables::States>(self.frame);
        let state = states
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| format!("state {id}: States {gap}"))?;
        state_name_from(
            state,
            |bytes, what| self.stored_text(bytes, what),
            |id| {
                let environment = self.environment()?;
                let definition = environment.state_names.get(&id).ok_or_else(|| {
                    format!("GetName: state {id} has no recorded-content template")
                })?;
                let key = definition.as_ref().map_err(Clone::clone)?;
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

    /// GetDisplayName (140E67930), as province GetName calls it (141A83DD0).
    pub(crate) fn province_name(&self, id: u32) -> Result<String, String> {
        if let Some(raw) = self.province_name_override(id)? {
            let text = self.localisation.localise(raw).map_err(|e| e.to_string())?;
            if !text.is_empty() {
                return Ok(text);
            }
        }
        self.province_victory_point_name(id)
    }

    /// First two rungs of 140E69320, also used by the combat-history location.
    pub(crate) fn province_display_name_primary(&self, id: u32) -> Result<Option<String>, String> {
        if let Some(raw) = self
            .province_name_override(id)?
            .filter(|raw| !raw.is_empty())
        {
            // Native tests raw length, then returns even an empty localisation.
            return self
                .localisation
                .localise(raw)
                .map(Some)
                .map_err(|e| e.to_string());
        }
        let text = self.province_victory_point_name(id)?;
        if !text.is_empty() {
            return Ok(Some(text));
        }
        Ok(None)
    }

    /// Effect/map display (140E69320), distinct from province GetName.
    pub(crate) fn province_display_name(&self, id: u32) -> Result<String, String> {
        if let Some(text) = self.province_display_name_primary(id)? {
            return Ok(text);
        }
        let (provinces, gap) = rows::<tables::Provinces>(self.frame);
        let province = provinces
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| format!("province {id}: Provinces {gap}"))?;
        if province.state != 0 {
            self.state_name(province.state)
        } else {
            Err(format!(
                "province {id} effect display fallback needs unrecorded CProvince+0xC8 strategic-region pointer/nullness and CStrategicRegion+0x38/+0x48 instance name (140E69320/140E0EB10; writers 140E0F2B0,141B50C80,141B4F0B0)"
            ))
        }
    }

    fn province_name_override(&self, id: u32) -> Result<Option<&[u8]>, String> {
        let (names, gap) = rows::<tables::ProvinceNames>(self.frame);
        if let Some(row) = names.iter().find(|row| row.province == id) {
            Ok(Some(&row.name))
        } else if complete::<tables::ProvinceNames>(self.frame) {
            Ok(None)
        } else {
            Err(format!("GetName of province {id}: ProvinceNames {gap}"))
        }
    }

    pub(crate) fn province_victory_point_name(&self, id: u32) -> Result<String, String> {
        let (provinces, gap) = rows::<tables::Provinces>(self.frame);
        let province = provinces
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| format!("province {id}: Provinces {gap}"))?;
        // 140E68E30 takes the STATE's owner original tag or tag, not the
        // province controller. A null state uses the country at slot zero.
        let owner = if province.state == 0 {
            0
        } else {
            let (states, gap) = rows::<tables::States>(self.frame);
            let state = states
                .iter()
                .find(|row| row.id == province.state)
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
        let tag = installed_content::country_names::original_or_own_tag(self.frame, owner)?;
        for key in [
            format!("{tag}_VICTORY_POINTS_{id}"),
            format!("VICTORY_POINTS_{id}"),
        ] {
            if self.localisation.get(&key).is_some() {
                return self.localise(&key);
            }
        }
        Ok(String::new())
    }

    pub(crate) fn province_terrain(&self, id: u32) -> Result<i32, String> {
        self.environment()?.terrain.as_ref().and_then(|map| map.terrain(id))
            .map(|terrain| terrain.index)
            .ok_or_else(|| format!("Terrain of province {id}: the recorded-content province template is unavailable (141A83F00)"))
    }

    /// CGameItem::id_copy@140616BC0, also used by raid target naming (140FD1D50).
    pub(crate) fn terrain_id_name(&self, ordinal: i32) -> Result<String, String> {
        // Null CTerrainType has an empty id (140AC6860 -> 1414297D0).
        if ordinal == 0 {
            return Ok(String::new());
        }
        let environment = self.environment()?;
        let names = environment.terrain_names.as_ref().map_err(Clone::clone)?;
        let name = ordinal.checked_sub(1).and_then(|at| usize::try_from(at).ok()).and_then(|at| names.get(at))
            .ok_or_else(|| format!("Terrain.GetName: native terrain ordinal {ordinal} has no recorded-content category"))?;
        self.localisation.localise(name).map_err(|e| e.to_string())
    }

    /// Terrain GetName@141A857D0 localises the already localised CGameItem id.
    pub(super) fn terrain_name(&self, ordinal: i32) -> Result<String, String> {
        let id = self.terrain_id_name(ordinal)?;
        self.localisation
            .localise(id.as_bytes())
            .map_err(|e| e.to_string())
    }
}

/// Shared with state tables: native name precedence without another ID scan.
pub(crate) fn state_name_from(
    state: &replay::rows::StateRow,
    stored: impl Fn(&[u8], &str) -> Result<String, String>,
    template: impl Fn(u32) -> Result<String, String>,
) -> Result<String, String> {
    let id = state.id;
    // CState::display_name (1409CC380): the custom name, else the owner
    // name, else the template's. set_state_name (140310B50) stores the
    // localised text of its argument as the custom name; SetOwner
    // (1409D1710, also on load) has 1409D3770 store the localised
    // `<owner original tag or tag>_<template name key>` as the owner name,
    // or clear it when the key is missing.
    for (bytes, what) in [
        (&state.custom_name, "the custom name (CState+0x38)"),
        (&state.owner_name, "the owner name (CState+0xE8)"),
    ] {
        if !bytes.is_empty() {
            return stored(bytes, &format!("GetName of state {id}: {what}"));
        }
    }
    template(id)
}

/// The event scope 1410CD900 evaluates an entry in, as an index into the
/// expansion's scope arena. With no promotion it is the active scope itself
/// (1410CE090 with +0x378); otherwise 1410CDDF0 binds the promoted object as
/// THIS in a new scope with the active scope's ROOT (+0x18) and FROM
/// (+0x20), and the active scope as PREV (+0x28); a country or state is set
/// with clearing (14052DD00, 14052ECE0 with 1 -> 14052F3F0), so that scope
/// carries no character.
fn bound_scope(
    object: Object,
    scopes: &ExpansionScopes,
    active: usize,
    event_active: bool,
) -> usize {
    let this = match object {
        Object::Event => return active,
        Object::Country(slot) => EventObject::Country(slot),
        Object::State(id) => EventObject::State(id.unwrap_or(crate::triggers::NULL_STATE)),
        Object::Character(id) => EventObject::Character(id),
        Object::Province(_)
        | Object::Terrain(_)
        | Object::FactionEnvironment(_)
        | Object::Faction(_) => {
            unreachable!("1410CD900 does not bind these text kinds")
        }
    };
    // 1410CD900 leaves the fresh 1410CDDF0 scope's links alone when +0x378
    // is null, including a country promoted from the faction environment.
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
        random_seed_refusal: None, // 1410CDDF0 constructs a known fresh seed.
    });
    scopes.prev_distance.borrow_mut().push(distance);
    at
}

/// The trigger scopes of event scope `at`, `bare` when its THIS was set with
/// clearing ([`bound_scope`]). A character is a character scope
/// (`crate::character_scope`). A link the caller cannot describe is unknown
/// to the evaluator, so only a condition that reads it is unknown (`always`
/// reads none, 1403CE520).
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

#[cfg(test)]
#[test]
fn entry_invocations_keep_the_original_bounds_of_unresolved_links() {
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

#[cfg(test)]
impl EnvironmentInputs {
    pub(super) fn synthetic() -> Self {
        Self {
            warnings: Vec::new(),
            state_names: BTreeMap::new(),
            state_provinces: BTreeMap::new(),
            state_error: None,
            continents: Err("no synthetic continent content is supplied".into()),
            impassable: BTreeSet::new(),
            neighbors: Ok(BTreeMap::new()),
            terrain: None,
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
    pub(super) fn with_aliases(
        self,
        aliases: std::sync::Arc<installed_content::tag_aliases::TagAliases>,
    ) -> Self {
        Self { aliases, ..self }
    }

    pub(super) fn with_terrain(
        mut self,
        terrain: installed_content::movement::MovementMap,
        names: Vec<Vec<u8>>,
    ) -> Self {
        self.terrain = Some(terrain);
        self.terrain_names = Ok(names);
        self
    }
}
