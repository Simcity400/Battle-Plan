//! Ordered scripted entries and content used by recorded triggers.
use super::*;
use crate::triggers::{ContentFacts, Eval, Evaluator, LeaderRoles, Scope, Scopes, Temps, World};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type StateNames = BTreeMap<u32, Result<String, String>>;

pub(crate) struct StateContent {
    pub defines: installed_content::defines::Defines,
    pub states: installed_content::states::States,
    pub names: StateNames,
    pub warnings: Vec<String>,
}

/// Shared state templates and name keys, resolved through the recorded stack.
pub(crate) fn read_state_content(
    layers: &installed_content::ResolvedLayers,
) -> std::io::Result<StateContent> {
    let defines = installed_content::defines::read(layers)?;
    let map = installed_content::terrain::read_default_map(layers)?;
    let provinces = installed_content::map::parse_definitions(&layers.read(&map.file("definitions")?)?)?;
    let (states, warnings) = installed_content::states::read(layers, &defines, &provinces)?;
    let names = states
        .definitions()
        .iter()
        .map(|(&id, state)| (id, state.name_key.clone()))
        .collect();
    Ok(StateContent {
        defines,
        states,
        names,
        warnings,
    })
}

pub(crate) struct Environment {
    pub warnings: Vec<String>,
    pub(super) state_names: BTreeMap<u32, Result<String, String>>,
    state_provinces: BTreeMap<u32, Vec<u32>>,
    impassable: BTreeSet<u32>,
    neighbors: Result<BTreeMap<u32, Vec<u32>>, String>,
    terrain: Option<installed_content::movement::MovementMap>,
    terrain_names: Result<Vec<Vec<u8>>, String>,
    dlcs: BTreeSet<String>,
    triggers: installed_content::trigger_keys::TriggerNames,
    aliases: std::sync::Arc<installed_content::tag_aliases::TagAliases>,
    buildings: installed_content::buildings::Buildings,
    bonuses: [i64; 2],
    roles: Result<LeaderRoles, String>,
    invasion_days: Result<i32, String>,
    constants: Result<installed_content::script_constants::Constants, String>,
    /// The unit-leader and country-leader trait databases a character's
    /// `has_trait` reads.
    traits: Result<
        (
            installed_content::unit_leader::Traits,
            installed_content::leader_traits::LeaderTraits,
        ),
        String,
    >,
}

impl Environment {
    fn read(layers: &installed_content::ResolvedLayers) -> std::io::Result<Self> {
        let StateContent {
            defines,
            states,
            names: state_names,
            warnings,
        } = read_state_content(layers)?;
        let states = states.definitions();
        let state_provinces = states
            .iter()
            .map(|(&id, s)| (id, s.provinces.clone()))
            .collect();
        let buildings = installed_content::buildings::read(layers)?;
        let neighbors = installed_content::province_graph::ProvinceGraph::read(layers)
            .and_then(|(graph, _)| graph.neighbor_states(&state_provinces))
            .map_err(|e| e.to_string());
        Ok(Self {
            warnings,
            state_provinces,
            impassable: states
                .iter()
                .filter_map(|(&id, s)| s.impassable.then_some(id))
                .collect(),
            state_names,
            neighbors,
            terrain: installed_content::movement::MovementMap::read(layers, &defines).ok(),
            terrain_names: installed_content::terrain::read_definitions(layers)
                .map(|definitions| {
                    definitions
                        .categories
                        .into_iter()
                        .map(|category| category.name)
                        .collect()
                })
                .map_err(|e| e.to_string()),
            dlcs: layers.dlc_names()?,
            triggers: installed_content::trigger_keys::TriggerNames::new(
                installed_content::scripted_triggers::read(layers)?,
                installed_content::trigger_keys::CountTriggerNames::new(
                    &buildings,
                    &installed_content::ideologies::read(layers)?.0,
                    &installed_content::resources::read(layers)?.0,
                ),
            ),
            aliases: std::sync::Arc::new(installed_content::tag_aliases::read(layers)?),
            buildings,
            bonuses: crate::state_resources::resource_bonuses(&defines)?,
            roles: installed_content::characters::leader_templates(layers)
                .map(LeaderRoles::new)
                .map_err(|e| e.to_string()),
            invasion_days: defines
                .int32("NNavy", "NAVAL_INVASION_PREPARE_DAYS")
                .map_err(|e| e.to_string()),
            constants: installed_content::script_constants::read(layers).map_err(|e| e.to_string()),
            traits: installed_content::unit_leader::Traits::registered(layers)
                .and_then(|units| Ok((units, installed_content::leader_traits::read(layers)?.0)))
                .map_err(|e| e.to_string()),
        })
    }

    fn facts(&self) -> ContentFacts<'_> {
        ContentFacts {
            state_provinces: &self.state_provinces,
            impassable: &self.impassable,
            neighbor_states: self.neighbors.as_ref().map_err(String::as_str),
            terrain: self.terrain.as_ref(),
            dlcs: &self.dlcs,
            triggers: &self.triggers,
            tag_aliases: &self.aliases,
            buildings: &self.buildings,
            resource_bonuses: Ok(self.bonuses),
            leader_roles: self.roles.as_ref().map_err(String::as_str),
            invasion_prepare_days: self.invasion_days.as_ref().copied().map_err(String::as_str),
            script_constants: self.constants.as_ref().map_err(String::as_str),
            character_traits: self
                .traits
                .as_ref()
                .map(|(units, country_leaders)| (units, country_leaders))
                .map_err(String::as_str),
        }
    }
}

impl<'a> ScriptedText<'a> {
    fn environment(&self) -> Result<&'a Environment, String> {
        self.content
            .environment
            .get_or_init(|| {
                Environment::read(&self.content.layers.layers)
                    .map_err(|e| format!("scripted localisation content: {e}"))
            })
            .as_ref()
            .map_err(Clone::clone)
    }

    /// The hour's trigger inputs, read on first use.
    pub(super) fn world(&self) -> Result<&World<'a>, String> {
        self.world
            .get_or_init(|| World::new(self.frame, self.environment()?.facts()))
            .as_ref()
            .map_err(Clone::clone)
    }

    pub(super) fn entry(
        &self,
        object: Object,
        name: &str,
        scopes: &EventScopes,
        active: usize,
    ) -> Result<String, String> {
        if !self.content.names.unknown.is_empty() {
            return Err(format!(
                "{name}: scripted localisation definitions are uncertain: {}",
                self.content.names.unknown.join("; ")
            ));
        }
        let Some(entry) = self.content.names.entries.get(name.as_bytes()) else {
            return Ok(String::new());
        };
        let (bound, at) = bound_scope(object, scopes, active);
        let bare = object != Object::Event;
        // 140A9CC20 checks alternatives in order; EndScopedVariableContext
        // (140535830) leaves the checked trigger's temps for a later value.
        let mut checked_trigger = false;
        let mut evaluator = None;
        for text in &entry.texts {
            // 140A9CC20 skips a null value without checking its trigger.
            let Some(value) = &text.value else { continue };
            if text.trigger_present {
                checked_trigger = true;
                // The next outer check starts a new context (140544190).
                evaluator = None;
            }
            let trigger = &text.trigger;
            if !trigger.is_empty() {
                let scopes = entry_scopes(&bound, at, bare);
                evaluator = Some(Evaluator::new(self.world()?, None));
                match evaluator.as_ref().unwrap().evaluate(trigger, scopes) {
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
                            &bound,
                            at,
                            entry_scopes(&bound, at, bare),
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
            // 0. An entry that reaches itself would not end, so nesting as
            // deep as MAX_SCRIPTED_LOC_RECURSION is refused.
            let depth = self.nested.get();
            if depth >= self.content.recursion {
                return Err(format!(
                    "scripted localisation {name} is nested {depth} entries deep, and each restarts at depth 0 (1410CF560)"
                ));
            }
            self.nested.set(depth + 1);
            let expanded = self.process(&text, 0, &bound, at, Object::Event);
            self.nested.set(depth);
            return expanded;
        }
        // The shared empty CBoundLocalization at 143098438.
        Ok(String::new())
    }

    pub(super) fn state_name(&self, id: u32) -> Result<String, String> {
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
                self.localise(definition.as_ref().map_err(Clone::clone)?)
            },
        )
    }

    /// GetDisplayName (140E67930), as province GetName calls it (141A83DD0).
    pub(super) fn province_name(&self, id: u32) -> Result<String, String> {
        let (names, gap) = rows::<tables::ProvinceNames>(self.frame);
        if let Some(row) = names.iter().find(|row| row.province == id) {
            let text = self
                .localisation
                .localise(&row.name)
                .map_err(|e| e.to_string())?;
            if !text.is_empty() {
                return Ok(text);
            }
        } else if !complete::<tables::ProvinceNames>(self.frame) {
            return Err(format!("GetName of province {id}: ProvinceNames {gap}"));
        }
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
            self.scopes.slot(state.owner).ok_or_else(|| {
                format!(
                    "province {id}: state owner's link {} resolves to no country",
                    state.owner
                )
            })?
        };
        let appearance = self
            .country_row::<tables::CountryAppearance>(owner, |row| row.country)?
            .ok_or_else(|| {
                format!("province {id}: owner slot {owner} has no CountryAppearance row")
            })?;
        let tag = if appearance.name_tag.is_empty() {
            self.scopes
                .tag(owner)
                .ok_or_else(|| format!("province {id}: owner slot {owner} has no country tag"))?
        } else {
            std::str::from_utf8(&appearance.name_tag)
                .map_err(|_| format!("province {id}: owner's original tag is not UTF-8"))?
        };
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

    pub(super) fn province_terrain(&self, id: u32) -> Result<i32, String> {
        self.environment()?.terrain.as_ref().and_then(|map| map.terrain(id))
            .map(|terrain| terrain.index)
            .ok_or_else(|| format!("Terrain of province {id}: the recorded-content province template is unavailable (141A83F00)"))
    }

    /// Terrain GetName (141A857D0) localises its CGameItem id.
    pub(super) fn terrain_name(&self, ordinal: i32) -> Result<String, String> {
        // Null CTerrainType has an empty id (140AC6860 -> 1414297D0).
        if ordinal == 0 {
            return Ok(String::new());
        }
        let environment = self.environment()?;
        let names = environment.terrain_names.as_ref().map_err(Clone::clone)?;
        let name = ordinal.checked_sub(1).and_then(|at| usize::try_from(at).ok()).and_then(|at| names.get(at))
            .ok_or_else(|| format!("Terrain.GetName: native terrain ordinal {ordinal} has no recorded-content category"))?;
        let id = self
            .localisation
            .localise(name)
            .map_err(|e| e.to_string())?;
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
/// returned scopes. With no promotion it is the active scope itself
/// (1410CE090 with +0x378); otherwise 1410CDDF0 binds the promoted object as
/// THIS in a new scope with the active scope's ROOT (+0x18) and FROM
/// (+0x20), and the active scope as PREV (+0x28); a country or state is set
/// with clearing (14052DD00, 14052ECE0 with 1 -> 14052F3F0), so that scope
/// carries no character.
fn bound_scope(object: Object, scopes: &EventScopes, active: usize) -> (EventScopes, usize) {
    let this = match object {
        Object::Event => return (scopes.clone(), active),
        Object::Country(slot) => EventObject::Country(slot),
        Object::State(id) => EventObject::State(id.unwrap_or(crate::triggers::NULL_STATE)),
        Object::Character(id) => EventObject::Character(id),
        Object::Province(_) | Object::Terrain(_) => {
            unreachable!("1410CD900 does not bind these text kinds")
        }
    };
    let current = scopes.scopes[active];
    let carried = |link| match link {
        Link::Itself => Link::Scope(active),
        other => other,
    };
    let mut bound = scopes.clone();
    bound.scopes.push(EventScope {
        this,
        root: carried(current.root),
        from: carried(current.from),
        prev: Link::Scope(active),
        random_seed: super::random::scope_seed(424242),
    });
    let at = bound.scopes.len() - 1;
    (bound, at)
}

/// The trigger scopes of event scope `at`, `bare` when its THIS was set with
/// clearing ([`bound_scope`]). A character is a character scope
/// (`crate::character_scope`). A link the caller cannot describe is unknown
/// to the evaluator, so only a condition that reads it is unknown (`always`
/// reads none, 1403CE520).
fn entry_scopes(scopes: &EventScopes, at: usize, bare: bool) -> Scopes {
    let scope = |object| match object {
        EventObject::Country(slot) => Scope::Country(slot),
        EventObject::State(id) => Scope::State(id),
        EventObject::Province(id) => Scope::Province(id),
        EventObject::UnitLeader(reference) => Scope::Leader(reference),
        EventObject::Character(id) => Scope::Character(id),
    };
    let current = scopes.scopes[at];
    let this = scope(current.this);
    let linked = |link| {
        scopes
            .linked(at, link, "")
            .ok()
            .map(|to| scope(scopes.scopes[to].this))
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
        target_state: scopes.variables.target_state,
        target_province: scopes.variables.target_province,
        target_leader: scopes.variables.target_leader,
    }
}

#[cfg(test)]
impl Environment {
    pub(super) fn synthetic() -> Self {
        Self {
            warnings: Vec::new(),
            state_names: BTreeMap::new(),
            state_provinces: BTreeMap::new(),
            impassable: BTreeSet::new(),
            neighbors: Ok(BTreeMap::new()),
            terrain: None,
            terrain_names: Ok(Vec::new()),
            dlcs: BTreeSet::new(),
            triggers: Default::default(),
            aliases: Default::default(),
            buildings: Default::default(),
            bonuses: [0; 2],
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
