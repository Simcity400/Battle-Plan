//! CEventScope copies and description-only target storage (14146BB30).

use std::{cell::RefCell, rc::Rc};

use installed_content::scope::{Keyword, Scope as Parsed};

use super::{Evaluator, Scope, Scopes, UnknownLinks, saved_event_targets::Reader};
use crate::scripted_text::{EventObject, EventScope, EventScopes, Link};

pub(super) fn object(value: EventObject) -> Scope {
    match value {
        EventObject::Country(id) => Scope::Country(id),
        EventObject::State(id) => Scope::State(id),
        EventObject::Province(id) => Scope::Province(id),
        EventObject::UnitLeader(id) => Scope::Leader(id),
        EventObject::Character(id) => Scope::Character(id),
    }
}

fn event_object(value: Scope) -> Result<EventObject, String> {
    Ok(match value {
        Scope::Country(id) => EventObject::Country(id),
        Scope::State(id) => EventObject::State(id),
        Scope::Province(id) => EventObject::Province(id),
        Scope::Leader(id) => EventObject::UnitLeader(id),
        Scope::Character(id) => EventObject::Character(id),
        Scope::Combat => return Err("combat scope graph needs its caller's combat context".into()),
    })
}

pub(super) fn linked(graph: &EventScopes, at: usize, link: Link) -> Result<usize, String> {
    match link {
        Link::Itself if at < graph.scopes.len() => Ok(at),
        Link::Scope(to) if to < graph.scopes.len() => Ok(to),
        Link::Unknown => Err("event scope link is not known here (14146BB30)".into()),
        _ => Err("event scope link is outside the caller graph".into()),
    }
}

fn terminal(graph: &EventScopes, mut at: usize, root: bool) -> Result<usize, String> {
    // Both native storage walks stop after 101 links (140529520/14052D4F0).
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
pub(crate) fn project(graph: &EventScopes, at: usize) -> Result<Scopes, String> {
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
        bare: false,
        cursor: Some(at),
    })
}

/// Native outer completion is independent of whether an operand needed World.
pub(crate) fn cleanup_description(graph: &mut EventScopes, at: usize) -> Result<(), String> {
    if !graph.tooltip_targets.is_empty() {
        let owner = terminal(graph, at, false)?;
        graph.tooltip_targets.remove(&owner);
    }
    graph.global_tooltip_targets.clear();
    Ok(())
}

/// 14146BB30/1410CFF00: local tooltip, local saved, global tooltip.
pub(crate) fn event_tooltip_target(
    graph: &EventScopes,
    at: usize,
    name: &[u8],
) -> Result<Option<EventObject>, String> {
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
    pub fn event_scopes(&self, at: usize) -> Result<Scopes, String> {
        let binding = self.event_graph.borrow();
        let graph = binding
            .as_ref()
            .ok_or("no caller event scope graph")?
            .borrow();
        project(&graph, at)
    }

    /// BeginScopedVariableRandomSeed@1405354A0: nested checks share TLS temps.
    pub fn begin_description(&self) {
        if self.description_depth.get() == 0 && self.checking.get() == 0 {
            self.local_temps.borrow_mut().clear();
            self.clear_temp_writer();
        }
        self.description_depth.set(self.description_depth.get() + 1);
    }

    /// 140529520 clears only terminal FROM storage and global tooltip targets.
    pub fn end_description(&self, scopes: Scopes) -> Result<(), String> {
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

    /// 14038D670 -> 14052D4F0: save the current object on terminal ROOT.
    pub fn save_tooltip_target(&self, name: &[u8], scopes: Scopes) -> Result<(), String> {
        let at = scopes
            .cursor
            .ok_or("description target needs caller scope graph")?;
        let binding = self.event_graph.borrow();
        let mut graph = binding
            .as_ref()
            .ok_or("description target needs caller scope graph")?
            .borrow_mut();
        let owner = terminal(&graph, at, true)?;
        let target = graph.scopes[at].this;
        let rows = graph.tooltip_targets.entry(owner).or_default();
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
    ) -> Result<Option<Scope>, String> {
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
        Ok(target.map(object))
    }

    pub(super) fn local_saved_exists(&self, name: &[u8], scopes: Scopes) -> Result<bool, String> {
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
                EventObject::Country(_) => true,
                EventObject::State(id) | EventObject::Province(id) | EventObject::Character(id) => {
                    id > 0
                }
                EventObject::UnitLeader(id) => id != 0,
            }))
    }

    pub(super) fn enter_graph(
        &self,
        parsed: &Parsed,
        scopes: Scopes,
    ) -> Result<Option<Scopes>, String> {
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
            // Dotted-tail exit restores the original PREV (14146D64E).
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
                Parsed::Keyword(Keyword::Root) => Some(
                    linked(&graph, at, graph.scopes[at].root)
                        .map_err(|why| format!("ROOT is not known here: {why}"))?,
                ),
                Parsed::Keyword(Keyword::From) => Some(
                    linked(&graph, at, graph.scopes[at].from)
                        .map_err(|why| format!("FROM is not known here: {why}"))?,
                ),
                Parsed::Keyword(Keyword::Prev) => Some(
                    linked(&graph, at, graph.scopes[at].prev)
                        .map_err(|why| format!("PREV is not known here: {why}"))?,
                ),
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

    fn copy_scope(&self, scopes: Scopes, source: usize, this: Scope) -> Result<usize, String> {
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
        // copy_members@1405296C0 carries links; only following one reads it.
        copied.root = match graph.scopes[caller].root {
            Link::Itself => Link::Scope(caller),
            root => root,
        };
        copied.prev = Link::Scope(caller);
        copied.this = event_object(this)?;
        let index = graph.scopes.len();
        graph.scopes.push(copied);
        // copy_members@1405296C0 copies storage rather than sharing it.
        if let Some(targets) = graph.tooltip_targets.get(&source).cloned() {
            graph.tooltip_targets.insert(index, targets);
        }
        if let Some(targets) = graph.saved_targets.get(&source).cloned() {
            graph.saved_targets.insert(index, targets);
        }
        Ok(index)
    }

    /// Collection setters keep caller ROOT/FROM and install PREV (1413AE780).
    pub fn with_this(&self, scopes: Scopes, this: Scope) -> Result<Scopes, String> {
        let Some(at) = scopes.cursor else {
            return Ok(scopes.with_this(this));
        };
        let index = self.copy_scope(scopes, at, this)?;
        self.event_scopes(index).map(|mut scopes| {
            scopes.bare = true;
            scopes
        })
    }
}
