//! Event scope copies and description-only target storage.
use crate::triggers::Failure;

use std::{cell::RefCell, rc::Rc};

use installed_content_core::{scope::Keyword, scope::Scope as Parsed};

use super::{Evaluator, Scope, Scopes, UnknownLinks, saved_event_targets::Reader};
use crate::scripted_text::{EventObject, EventScope, EventScopes, Link};

pub(super) fn object(value: EventObject) -> Scope {
    match value {
        EventObject::Country(id) | EventObject::CountryState { country: id, .. } => {
            Scope::Country(id)
        }
        EventObject::Combat => Scope::Combat,
        EventObject::State(id) => Scope::State(id),
        EventObject::Province(id) => Scope::Province(id),
        EventObject::UnitLeader(id) => Scope::Leader(id),
        EventObject::CountryUnitLeader { country, leader } => {
            Scope::CountryLeader { country, leader }
        }
        EventObject::Character(id) => Scope::Character(id),
        EventObject::IndustrialOrganisation(id) => Scope::IndustrialOrganisation(id),
        EventObject::Project { owner, id } => Scope::Project { owner, id },
        EventObject::Raid(id) => Scope::Raid(id),
        EventObject::Unit(id) => Scope::Unit(id),
    }
}

pub(super) fn event_object(value: Scope) -> Result<EventObject, Failure> {
    Ok(match value {
        Scope::Country(id) => EventObject::Country(id),
        Scope::State(id) => EventObject::State(id),
        Scope::Province(id) => EventObject::Province(id),
        Scope::Leader(id) => EventObject::UnitLeader(id),
        Scope::CountryLeader { country, leader } => {
            EventObject::CountryUnitLeader { country, leader }
        }
        Scope::Character(id) => EventObject::Character(id),
        Scope::IndustrialOrganisation(id) => EventObject::IndustrialOrganisation(id),
        Scope::Project { owner, id } => EventObject::Project { owner, id },
        Scope::Raid(id) => EventObject::Raid(id),
        Scope::Unit(id) => EventObject::Unit(id),
        Scope::Combat => EventObject::Combat,
    })
}

pub(super) fn linked(graph: &EventScopes, at: usize, link: Link) -> Result<usize, Failure> {
    match link {
        Link::Itself if at < graph.scopes.len() => Ok(at),
        Link::Scope(to) if to < graph.scopes.len() => Ok(to),
        Link::Unknown => Err("event scope link is not known here".into()),
        _ => Err("event scope link is outside the caller graph".into()),
    }
}

fn terminal(graph: &EventScopes, mut at: usize, root: bool) -> Result<usize, Failure> {
    // Both storage walks stop after 101 links.
    for _ in 0..101 {
        let current = graph.scopes.get(at).ok_or("event scope is not given")?;
        let next = linked(graph, at, if root { current.root } else { current.from })?;
        if at == next {
            return Ok(at);
        }
        at = next;
    }
    Err("event target storage chain exceeds native 101-link walk".into())
}

fn lookup(rows: &[(Vec<u8>, EventObject)], name: &[u8]) -> Option<EventObject> {
    rows.iter()
        .find(|(key, value)| key == name && *value != EventObject::State(super::NULL_STATE))
        .map(|(_, value)| *value)
}

/// Graph projection needs no World; unavailable operands are checked when read.
pub(crate) fn project(graph: &EventScopes, at: usize) -> Result<Scopes, Failure> {
    let scope = graph.scopes.get(at).ok_or("event scope is not given")?;
    let selected = |link| {
        linked(graph, at, link)
            .ok()
            .map(|to| object(graph.scopes[to].this))
    };
    let (root, from, prev) = (
        selected(scope.root),
        selected(scope.from),
        selected(scope.prev),
    );
    Ok(Scopes {
        this: object(scope.this),
        root: root.unwrap_or(object(scope.this)),
        from,
        prev,
        unknown: UnknownLinks {
            root: root.is_none(),
            from: from.is_none(),
            prev: prev.is_none(),
        },
        target_state: graph.variables.target_state,
        target_province: graph.variables.target_province,
        target_leader: graph.variables.target_leader,
        // Clearing country/state/combat setters
        // leave no character. Retained operands
        // use distinct combined carriers, including after link copies.
        bare: matches!(
            scope.this,
            EventObject::Country(_) | EventObject::State(_) | EventObject::Combat
        ),
        cursor: Some(at),
    })
}

/// Outer completion is independent of whether an operand needed World.
pub(crate) fn cleanup_description(graph: &mut EventScopes, at: usize) -> Result<(), Failure> {
    if !graph.tooltip_targets.is_empty() {
        let owner = terminal(graph, at, false)?;
        graph.tooltip_targets.remove(&owner);
    }
    graph.global_tooltip_targets.clear();
    Ok(())
}

/// Target lookup order: local tooltip, local saved, global tooltip.
pub(crate) fn event_tooltip_target(
    graph: &EventScopes,
    at: usize,
    name: &[u8],
) -> Result<Option<EventObject>, Failure> {
    let owner = terminal(graph, at, false)?;
    Ok(graph
        .tooltip_targets
        .get(&owner)
        .and_then(|rows| lookup(rows, name))
        .or_else(|| {
            graph
                .saved_targets
                .get(&owner)
                .and_then(|rows| lookup(rows, name))
        })
        .or_else(|| lookup(&graph.global_tooltip_targets, name)))
}

/// Restores a caller's graph binding after a shared description invocation.
pub struct EventGraphGuard<'g, 'e, 'a> {
    evaluator: &'g Evaluator<'e, 'a>,
    previous: Option<Rc<RefCell<EventScopes>>>,
}

impl Drop for EventGraphGuard<'_, '_, '_> {
    fn drop(&mut self) {
        self.evaluator.event_graph.replace(self.previous.take());
    }
}

impl<'e, 'a> Evaluator<'e, 'a> {
    pub fn with_event_scopes(self, graph: Rc<RefCell<EventScopes>>) -> Self {
        self.event_graph.replace(Some(graph));
        self
    }

    pub fn bind_event_scopes(
        &self,
        graph: Rc<RefCell<EventScopes>>,
    ) -> EventGraphGuard<'_, 'e, 'a> {
        EventGraphGuard {
            evaluator: self,
            previous: self.event_graph.replace(Some(graph)),
        }
    }

    /// Project only the immediate links; the cursor keeps deeper links intact.
    pub fn event_scopes(&self, at: usize) -> Result<Scopes, Failure> {
        let binding = self.event_graph.borrow();
        let graph = binding
            .as_ref()
            .ok_or("no caller event scope graph")?
            .borrow();
        project(&graph, at)
    }

    /// Nested checks share TLS temps.
    pub fn begin_description(&self) {
        if self.description_depth.get() == 0 && self.checking.get() == 0 {
            self.local_temps.borrow_mut().clear();
            self.clear_temp_writer();
        }
        self.description_depth.set(self.description_depth.get() + 1);
    }

    /// Begin TLS in the outer caller, before child scope changes.
    /// Retain an unavailable seed as a failed temp, without refusing unrelated text.
    pub(crate) fn begin_description_in(&self, scopes: Scopes) {
        let outer = self.description_depth.get() == 0 && self.checking.get() == 0;
        self.begin_description();
        if outer {
            self.initialize_variable_random(scopes);
        }
    }

    /// Clear only terminal FROM storage and global tooltip targets.
    pub fn end_description(&self, scopes: Scopes) -> Result<(), Failure> {
        let depth = self.description_depth.get();
        if depth == 0 {
            return Err("description session was not opened".into());
        }
        self.description_depth.set(depth - 1);
        if depth != 1 {
            return Ok(());
        }
        if let Some(at) = scopes.cursor
            && let Some(graph) = self.event_graph.borrow().as_ref()
        {
            let mut graph = graph.borrow_mut();
            cleanup_description(&mut graph, at)?;
        }
        Ok(())
    }

    /// Save the current object on terminal ROOT.
    pub fn save_tooltip_target(&self, name: &[u8], scopes: Scopes) -> Result<(), Failure> {
        let at = scopes
            .cursor
            .ok_or("description target needs caller scope graph")?;
        let binding = self.event_graph.borrow();
        let mut graph = binding
            .as_ref()
            .ok_or("description target needs caller scope graph")?
            .borrow_mut();
        let owner = terminal(&graph, at, true)?;
        // Saved targets project UNIT/RAID, but have no PROJECT arm.
        let target = match graph.scopes[at].this {
            EventObject::Project { .. } => EventObject::State(super::NULL_STATE),
            // Check country before character and write one
            // saved-target arm; scope copies still retain both operands.
            EventObject::CountryUnitLeader { country, leader } => {
                if self.variable_scope_has_country(country)? {
                    EventObject::Country(country)
                } else {
                    EventObject::Character(
                        self.world()
                            .unit_leader(leader)?
                            .ok_or("tooltip target unit leader is not recorded")?
                            .character_id,
                    )
                }
            }
            target => target,
        };
        let rows = graph.tooltip_targets.entry(owner).or_default();
        match rows.iter_mut().find(|(key, _)| key == name) {
            Some((_, value)) => *value = target,
            None => rows.push((name.to_vec(), target)),
        }
        Ok(())
    }

    /// Write the description registry, not saved globals.
    /// State precedes country in a combined scope.
    pub(crate) fn save_global_tooltip_target(
        &self,
        name: &[u8],
        scopes: Scopes,
    ) -> Result<(), Failure> {
        let at = scopes
            .cursor
            .ok_or("global description target needs caller scope graph")?;
        let binding = self.event_graph.borrow();
        let mut graph = binding
            .as_ref()
            .ok_or("global description target needs caller scope graph")?
            .borrow_mut();
        let target = match graph
            .scopes
            .get(at)
            .ok_or("global target scope is absent")?
            .this
        {
            EventObject::CountryState { state, .. } if state != super::NULL_STATE => {
                EventObject::State(state)
            }
            EventObject::CountryState { country, .. } => EventObject::Country(country),
            // Choose positive country before ability caster.
            EventObject::CountryUnitLeader { country, leader } => {
                if self.variable_scope_has_country(country)? {
                    EventObject::Country(country)
                } else {
                    EventObject::UnitLeader(leader)
                }
            }
            EventObject::State(super::NULL_STATE) | EventObject::IndustrialOrganisation(_) => {
                return Ok(());
            }
            EventObject::Province(_) => {
                return Err("global target's province scope is not an event scope".into());
            }
            target => target,
        };
        let rows = &mut graph.global_tooltip_targets;
        match rows.iter_mut().find(|(key, _)| key == name) {
            Some((_, value)) => *value = target,
            None => rows.push((name.to_vec(), target)),
        }
        Ok(())
    }

    pub(super) fn graph_saved(
        &self,
        name: &[u8],
        reader: Reader,
        scopes: Scopes,
    ) -> Result<Option<Scope>, Failure> {
        let Some(at) = scopes.cursor else {
            return Ok(None);
        };
        let binding = self.event_graph.borrow();
        let Some(graph) = binding.as_ref() else {
            return Ok(None);
        };
        let graph = graph.borrow();
        let target = if matches!(reader, Reader::CountrySelector) {
            let owner = terminal(&graph, at, false)?;
            graph
                .saved_targets
                .get(&owner)
                .and_then(|rows| lookup(rows, name))
        } else {
            event_tooltip_target(&graph, at, name)?
        };
        Ok(target.map(|target| match (reader, target) {
            // Context and text have no saved RAID setter;
            // text also has no saved UNIT setter. Keep the caller's object.
            (Reader::Context | Reader::Text, EventObject::Raid(_))
            | (Reader::Text, EventObject::Unit(_)) => scopes.this,
            (_, target) => object(target),
        }))
    }

    pub(super) fn local_saved_exists(&self, name: &[u8], scopes: Scopes) -> Result<bool, Failure> {
        let Some(at) = scopes.cursor else {
            return Ok(false);
        };
        let binding = self.event_graph.borrow();
        let Some(graph) = binding.as_ref() else {
            return Ok(false);
        };
        let graph = graph.borrow();
        let owner = terminal(&graph, at, false)?;
        Ok(graph
            .saved_targets
            .get(&owner)
            .and_then(|rows| lookup(rows, name))
            .is_some_and(|target| match target {
                EventObject::Country(_)
                | EventObject::CountryState { .. }
                | EventObject::CountryUnitLeader { .. }
                | EventObject::Combat => true,
                EventObject::State(id) | EventObject::Province(id) | EventObject::Character(id) => {
                    id > 0
                }
                EventObject::UnitLeader(id)
                | EventObject::IndustrialOrganisation(id)
                | EventObject::Raid(id)
                | EventObject::Unit(id) => id != 0,
                EventObject::Project { id, .. } => id != 0,
            }))
    }

    pub(super) fn enter_graph(
        &self,
        parsed: &Parsed,
        scopes: Scopes,
    ) -> Result<Option<Scopes>, Failure> {
        let at = scopes.cursor.ok_or("scope graph cursor is missing")?;
        {
            let binding = self.event_graph.borrow();
            let graph = binding.as_ref().ok_or("scope graph is missing")?.borrow();
            graph
                .scopes
                .get(at)
                .ok_or("scope graph cursor is outside the caller graph")?;
        }
        if let Parsed::Chain { head, then } = parsed {
            let Some(head) = self.enter_graph(&Parsed::Keyword(*head), scopes)? else {
                return Ok(None);
            };
            let Some(result) = self.enter_graph(then, head)? else {
                return Ok(None);
            };
            // Dotted-tail exit restores the original PREV.
            let binding = self.event_graph.borrow();
            let mut graph = binding
                .as_ref()
                .ok_or("scope graph is missing")?
                .borrow_mut();
            graph.scopes[result.cursor.unwrap()].prev = Link::Scope(at);
            drop(graph);
            drop(binding);
            return self.event_scopes(result.cursor.unwrap()).map(Some);
        }
        if let Parsed::EventTarget {
            name,
            then: Some(then),
        } = parsed
        {
            let first = Parsed::EventTarget {
                name: name.clone(),
                then: None,
            };
            let Some(first) = self.enter_graph(&first, scopes)? else {
                return Ok(None);
            };
            let Some(result) = self.enter_graph(then, first)? else {
                return Ok(None);
            };
            let binding = self.event_graph.borrow();
            let mut graph = binding
                .as_ref()
                .ok_or("scope graph is missing")?
                .borrow_mut();
            graph.scopes[result.cursor.unwrap()].prev = Link::Scope(at);
            drop(graph);
            drop(binding);
            return self.event_scopes(result.cursor.unwrap()).map(Some);
        }
        let selected = {
            let binding = self.event_graph.borrow();
            let graph = binding.as_ref().ok_or("scope graph is missing")?.borrow();
            match parsed {
                Parsed::Keyword(Keyword::This) => Some(at),
                Parsed::Keyword(Keyword::Root) => {
                    Some(linked(&graph, at, graph.scopes[at].root).map_err(|why| {
                        why.context(|why| format!("ROOT is not known here: {why}"))
                    })?)
                }
                Parsed::Keyword(Keyword::From) => {
                    Some(linked(&graph, at, graph.scopes[at].from).map_err(|why| {
                        why.context(|why| format!("FROM is not known here: {why}"))
                    })?)
                }
                Parsed::Keyword(Keyword::Prev) => {
                    Some(linked(&graph, at, graph.scopes[at].prev).map_err(|why| {
                        why.context(|why| format!("PREV is not known here: {why}"))
                    })?)
                }
                _ => None,
            }
        };
        let value = if let Some(selected) = selected {
            let binding = self.event_graph.borrow();
            Some(object(
                binding
                    .as_ref()
                    .ok_or("scope graph is missing")?
                    .borrow()
                    .scopes[selected]
                    .this,
            ))
        } else if let Parsed::EventTarget { name, .. } = parsed {
            self.saved_scope(name.as_bytes(), Reader::Context, scopes)?
                .map(|s| s.this)
        } else {
            // All non-link selectors still use the existing resolver/evaluator.
            self.enter_flat(parsed, scopes)?
        };
        let Some(value) = value else {
            return Ok(None);
        };
        let index = self.copy_scope(scopes, selected.unwrap_or(at), value)?;
        if matches!(parsed, Parsed::Keyword(Keyword::Root | Keyword::Prev)) {
            let binding = self.event_graph.borrow();
            let mut graph = binding
                .as_ref()
                .ok_or("scope graph is missing")?
                .borrow_mut();
            graph.scopes[index].from = match linked(&graph, at, graph.scopes[at].root) {
                Ok(root) => match graph.scopes[root].from {
                    Link::Itself => Link::Scope(root),
                    from => from,
                },
                Err(_) => Link::Unknown,
            };
        }
        self.event_scopes(index).map(Some)
    }

    fn copy_scope(&self, scopes: Scopes, source: usize, this: Scope) -> Result<usize, Failure> {
        let caller = scopes.cursor.ok_or("scope graph cursor is missing")?;
        let binding = self.event_graph.borrow();
        let mut graph = binding
            .as_ref()
            .ok_or("scope graph is missing")?
            .borrow_mut();
        let mut copied: EventScope = *graph.scopes.get(source).ok_or("source scope is missing")?;
        copied.from = match copied.from {
            Link::Itself => Link::Scope(source),
            other => other,
        };
        // Copying members carries links; only following one reads it.
        copied.root = match graph.scopes[caller].root {
            Link::Itself => Link::Scope(caller),
            root => root,
        };
        copied.prev = Link::Scope(caller);
        copied.this = event_object(this)?;
        let index = graph.scopes.len();
        graph.scopes.push(copied);
        // Copying members copies storage rather than sharing it.
        if let Some(targets) = graph.tooltip_targets.get(&source).cloned() {
            graph.tooltip_targets.insert(index, targets);
        }
        if let Some(targets) = graph.saved_targets.get(&source).cloned() {
            graph.saved_targets.insert(index, targets);
        }
        Ok(index)
    }

    /// Fresh scorer scope with FROM copied independently.
    pub(crate) fn fresh_country_scope(
        &self,
        country: Option<u32>,
        from: Option<Scopes>,
    ) -> Result<Scopes, Failure> {
        let binding = self.event_graph.borrow();
        // Default construction keeps country zero.
        let this = EventObject::Country(country.unwrap_or(0));
        let Some(graph) = binding.as_ref() else {
            let this = object(this);
            let mut fresh = Scopes::of(this);
            fresh.from = Some(from.map_or(this, |scope| scope.this));
            fresh.prev = Some(this);
            return Ok(fresh);
        };
        let index = {
            let mut graph = graph.borrow_mut();
            let from = match from {
                Some(from) => {
                    let source = from
                        .cursor
                        .ok_or("scorer FROM copy needs its graph cursor")?;
                    let mut copied = *graph
                        .scopes
                        .get(source)
                        .ok_or("scorer FROM scope is absent")?;
                    let absolute = |link| {
                        if link == Link::Itself {
                            Link::Scope(source)
                        } else {
                            link
                        }
                    };
                    copied.root = absolute(copied.root);
                    copied.from = absolute(copied.from);
                    copied.prev = absolute(copied.prev);
                    let at = graph.scopes.len();
                    graph.scopes.push(copied);
                    if let Some(targets) = graph.tooltip_targets.get(&source).cloned() {
                        graph.tooltip_targets.insert(at, targets);
                    }
                    if let Some(targets) = graph.saved_targets.get(&source).cloned() {
                        graph.saved_targets.insert(at, targets);
                    }
                    Link::Scope(at)
                }
                None => Link::Itself,
            };
            let mut fresh = EventScope::of(this);
            fresh.from = from;
            let at = graph.scopes.len();
            graph.scopes.push(fresh);
            at
        };
        drop(binding);
        self.event_scopes(index)
    }

    /// Set each target on the same copied scope.
    pub(crate) fn repoint_this(&self, scopes: Scopes, this: Scope) -> Result<Scopes, Failure> {
        let Some(at) = scopes.cursor else {
            return Ok(scopes.with_this(this));
        };
        let binding = self.event_graph.borrow();
        let graph = binding.as_ref().ok_or("scope graph is missing")?;
        graph
            .borrow_mut()
            .scopes
            .get_mut(at)
            .ok_or("scope cursor is absent")?
            .this = event_object(this)?;
        self.event_scopes(at).map(|mut scopes| {
            scopes.bare = true;
            scopes
        })
    }

    /// Collection setters keep caller ROOT/FROM and install PREV.
    pub fn with_this(&self, scopes: Scopes, this: Scope) -> Result<Scopes, Failure> {
        let Some(at) = scopes.cursor else {
            return Ok(scopes.with_this(this));
        };
        let index = self.copy_scope(scopes, at, this)?;
        self.event_scopes(index).map(|mut scopes| {
            scopes.bare = true;
            scopes
        })
    }

    /// Fresh collection scopes use default seeds; terminal representatives
    /// follow the caller's ROOT environment.
    pub(crate) fn collection_scope(
        &self,
        scopes: Scopes,
        this: Scope,
        terminal: bool,
    ) -> Result<Scopes, Failure> {
        let Some(caller) = scopes.cursor else {
            return Err("collection description needs its event graph".into());
        };
        let index = {
            let binding = self.event_graph.borrow();
            let mut graph = binding
                .as_ref()
                .ok_or("scope graph is missing")?
                .borrow_mut();
            let caller_scope = *graph
                .scopes
                .get(caller)
                .ok_or("collection caller is absent")?;
            let source = if terminal {
                linked(&graph, caller, caller_scope.root)?
            } else {
                caller
            };
            let origin = *graph
                .scopes
                .get(source)
                .ok_or("collection caller is absent")?;
            let absolute = |link| match link {
                Link::Itself => Link::Scope(source),
                other => other,
            };
            let mut fresh = EventScope::of(event_object(this)?);
            fresh.root = absolute(origin.root);
            fresh.from = absolute(origin.from);
            fresh.prev = Link::Scope(source);
            let at = graph.scopes.len();
            graph.scopes.push(fresh);
            at
        };
        self.event_scopes(index).map(|mut scopes| {
            scopes.bare = true;
            scopes
        })
    }
}
