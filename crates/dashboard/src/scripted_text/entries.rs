//! Ordered scripted entries and content used by recorded triggers.
use super::*;
use crate::triggers::{ContentFacts, Eval, LeaderRoles, Scope, Scopes, World};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct Environment {
    pub warnings: Vec<String>,
    pub(super) state_names: BTreeMap<u32, Result<String, String>>,
    state_provinces: BTreeMap<u32, Vec<u32>>,
    impassable: BTreeSet<u32>,
    neighbors: Result<BTreeMap<u32, Vec<u32>>, String>,
    terrain: Option<installed_content::movement::MovementMap>,
    dlcs: BTreeSet<String>,
    scripted: BTreeMap<String, installed_content::triggers::Triggers>,
    buildings: installed_content::buildings::Buildings,
    bonuses: [i64; 2],
    roles: Result<LeaderRoles, String>,
    invasion_days: Result<i32, String>,
}

impl Environment {
    fn read(layers: &installed_content::ResolvedLayers) -> std::io::Result<Self> {
        let defines = installed_content::defines::read(layers)?;
        let map = installed_content::terrain::read_default_map(layers)?;
        let provinces =
            installed_content::map::parse_definitions(&layers.read(&map.file("definitions")?)?)?;
        let (states, warnings) = installed_content::states::read(layers, &defines, &provinces)?;
        let states = states.definitions();
        let state_provinces = states
            .iter()
            .map(|(&id, s)| (id, s.provinces.clone()))
            .collect();
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
            state_names: states
                .iter()
                .map(|(&id, state)| (id, state.name_key.clone()))
                .collect(),
            neighbors,
            terrain: installed_content::movement::MovementMap::read(layers, &defines).ok(),
            dlcs: layers.dlc_names()?,
            scripted: installed_content::triggers::read_scripted(layers)?,
            buildings: installed_content::buildings::read(layers)?,
            bonuses: crate::state_resources::resource_bonuses(&defines)?,
            roles: installed_content::characters::leader_templates(layers)
                .map(LeaderRoles::new)
                .map_err(|e| e.to_string()),
            invasion_days: defines
                .int32("NNavy", "NAVAL_INVASION_PREPARE_DAYS")
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
            scripted: &self.scripted,
            buildings: &self.buildings,
            resource_bonuses: self.bonuses,
            leader_roles: self.roles.as_ref().map_err(String::as_str),
            invasion_prepare_days: self.invasion_days.as_ref().copied().map_err(String::as_str),
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
        for text in &entry.texts {
            // 140A9CC20 skips a null value without checking its trigger.
            let Some(key) = &text.key else { continue };
            let trigger = text.trigger.as_ref().map_err(|e| format!("{name}: {e}"))?;
            if !trigger.0.is_empty() {
                let scopes = entry_scopes(&bound, at, bare, name)?;
                let world = self
                    .world
                    .get_or_init(|| World::new(self.frame, self.environment()?.facts()))
                    .as_ref()
                    .map_err(Clone::clone)?;
                match world.evaluate(trigger, scopes) {
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
            let key = key
                .as_ref()
                .map_err(|e| format!("scripted localisation {name}: {e}"))?;
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
            let expanded = self.process(&text, 0, &bound, at);
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
                return self.stored_text(bytes, &format!("GetName of state {id}: {what}"));
            }
        }
        let environment = self.environment()?;
        let definition = environment
            .state_names
            .get(&id)
            .ok_or_else(|| format!("GetName: state {id} has no recorded-content template"))?;
        let key = definition.as_ref().map_err(Clone::clone)?;
        self.localise(key)
    }
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
    });
    let at = bound.scopes.len() - 1;
    (bound, at)
}

/// The trigger scopes of event scope `at`, `bare` when its THIS was set with
/// clearing ([`bound_scope`]). A link the caller cannot
/// describe, or one naming a character (the evaluator reads unit-leader
/// character scopes only), is unknown to the evaluator, so only a
/// condition that reads it is unknown (`always` reads none, 1403CE520).
fn entry_scopes(scopes: &EventScopes, at: usize, bare: bool, name: &str) -> Result<Scopes, String> {
    let scope = |object| match object {
        EventObject::Country(slot) => Some(Scope::Country(slot)),
        EventObject::State(id) => Some(Scope::State(id)),
        EventObject::Province(id) => Some(Scope::Province(id)),
        EventObject::UnitLeader(reference) => Some(Scope::Leader(reference)),
        EventObject::Character(_) => None,
    };
    let current = scopes.scopes[at];
    let this = scope(current.this).ok_or_else(|| {
        format!(
            "{name}: THIS is a character, and event-scope triggers require a character scope (1410CDDF0); the shared evaluator currently accepts unit-leader scopes only"
        )
    })?;
    let linked = |link| {
        scopes
            .linked(at, link, "")
            .ok()
            .and_then(|to| scope(scopes.scopes[to].this))
    };
    let (root, from, prev) = (
        linked(current.root),
        linked(current.from),
        linked(current.prev),
    );
    Ok(Scopes {
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
    })
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
            dlcs: BTreeSet::new(),
            scripted: BTreeMap::new(),
            buildings: Default::default(),
            bonuses: [0; 2],
            roles: Ok(LeaderRoles::new(Vec::new())),
            invasion_days: Ok(0),
        }
    }
}
