//! Native-only animation calls retained in source order and grouped by player.
//! Palette consumers keep the general path; this layout never holds borrowed players.

use super::{ModelField, ModelGroup, ModelInstance, Rig, entity_animation};
use std::{
    collections::BTreeMap,
    ops::Range,
    sync::{Arc, Mutex},
};
use viewer_foundation::parallel::Workers;

type Key = (usize, u32, u32);

#[derive(Default)]
pub(super) struct Dispatch {
    layout: Option<Layout>,
}

struct Layout {
    instances: usize,
    groups: Vec<Group>,
    sources: Vec<Source>,
    tasks: Vec<Task>,
    supported: bool,
}

struct Group {
    mesh: usize,
    range: Range<u32>,
    rig: Option<Arc<Rig>>,
    sources: Range<usize>,
}

#[derive(Clone, Copy)]
struct Source {
    index: usize,
    key: Key,
    kind: u8,
    call: Option<(usize, usize)>,
}

struct Task {
    key: Key,
    rig: Arc<Rig>,
    calls: Vec<Call>,
    alive: bool,
    ordinary: bool,
}

#[derive(Default)]
struct Call {
    active: bool,
    clocked: bool,
    instance: ModelInstance,
    events: Vec<entity_animation::Event>,
}

fn kind(instance: ModelInstance) -> u8 {
    match instance.animation {
        entity_animation::STOPPED => 0,
        entity_animation::DEFAULT_STATE => 1,
        _ => 2,
    }
}

fn key(mesh: usize, instance: ModelInstance) -> Key {
    let (seed, identity) = instance.animation_key();
    (mesh, seed, identity)
}

impl Layout {
    fn matches(&self, rigs: &[Arc<Rig>], field: &ModelField) -> bool {
        if self.instances != field.instances.len() || self.groups.len() != field.groups.len() {
            return false;
        }
        for (cached, group) in self.groups.iter().zip(&field.groups) {
            if cached.mesh != group.mesh || cached.range != group.range {
                return false;
            }
            match (cached.rig.as_ref(), rigs.get(group.mesh)) {
                (Some(before), Some(after)) if Arc::ptr_eq(before, after) => {}
                (None, None) => {}
                _ => return false,
            }
        }
        self.sources.iter().all(|source| {
            field.instances.get(source.index).is_some_and(|&instance| {
                key(source.key.0, instance) == source.key && kind(instance) == source.kind
            })
        })
    }

    fn build(rigs: &[Arc<Rig>], field: &ModelField) -> Self {
        let mut layout = Self {
            instances: field.instances.len(),
            groups: Vec::with_capacity(field.groups.len()),
            sources: Vec::new(),
            tasks: Vec::new(),
            supported: true,
        };
        for group in &field.groups {
            let rig = rigs.get(group.mesh);
            let start = layout.sources.len();
            if let Some(rig) = rig
                && rig.nodes.iter().any(|node| !node.clips.is_empty())
            {
                // A mixed rig needs real palette slices. Keep its whole pass on the
                // general path rather than change the order or ownership of those slices.
                layout.supported &= rig.nodes.iter().all(|node| node.native.is_some());
                for index in group.range.clone() {
                    let Some(&instance) = field.instances.get(index as usize) else {
                        // Let the original demand path decide whether an invalid range is
                        // reached. Validation must not add a panic for an unseen group.
                        layout.supported = false;
                        break;
                    };
                    layout.sources.push(Source {
                        index: index as usize,
                        key: key(group.mesh, instance),
                        kind: kind(instance),
                        call: None,
                    });
                }
            }
            layout.groups.push(Group {
                mesh: group.mesh,
                range: group.range.clone(),
                rig: rig.map(Arc::clone),
                sources: start..layout.sources.len(),
            });
        }
        if !layout.supported {
            return layout;
        }
        let mut by_player = BTreeMap::<Key, Vec<usize>>::new();
        for (at, source) in layout.sources.iter().enumerate() {
            if source.kind != 0 {
                by_player.entry(source.key).or_default().push(at);
            }
        }
        for (key, sources) in by_player {
            let task = layout.tasks.len();
            let mut calls = Vec::with_capacity(sources.len());
            for source in sources {
                layout.sources[source].call = Some((task, calls.len()));
                calls.push(Call::default());
            }
            layout.tasks.push(Task {
                key,
                rig: Arc::clone(&rigs[key.0]),
                calls,
                alive: false,
                ordinary: false,
            });
        }
        layout
    }

    fn alive(&self, key: &Key) -> bool {
        self.tasks
            .binary_search_by_key(key, |task| task.key)
            .is_ok_and(|at| self.tasks[at].alive)
    }
}

impl Dispatch {
    /// Returns false without changing presentation state if real palettes are needed.
    /// Exact validation covers placements, direct field installs and redirected rigs;
    /// time, transforms, clocks, defaults and group visibility are always read live.
    #[allow(clippy::too_many_arguments)]
    pub fn animate(
        &mut self,
        players: &mut BTreeMap<Key, entity_animation::Player>,
        rigs: &[Arc<Rig>],
        palette_start: usize,
        field: &mut ModelField,
        time: f32,
        shown: &dyn Fn(&ModelGroup) -> bool,
        events: &mut Vec<entity_animation::Event>,
        workers: &Workers,
    ) -> bool {
        let plan_span = viewer_foundation::profiling::span("animation_dispatch_plan");
        if self
            .layout
            .as_ref()
            .is_none_or(|layout| !layout.matches(rigs, field))
        {
            self.layout = Some(Layout::build(rigs, field));
        }
        let layout = self.layout.as_mut().unwrap();
        if !layout.supported {
            return false;
        }
        for task in &mut layout.tasks {
            task.alive = false;
            task.ordinary = false;
            for call in &mut task.calls {
                call.active = false;
                call.events.clear();
            }
        }
        let phase_only = time <= 0.0 && field.clocks.values().any(|clock| clock.phase.is_some());
        for instance in &mut field.instances {
            instance.palette_offset = 0;
        }
        let mut palette_end = palette_start;
        for (cached, group) in layout.groups.iter().zip(&field.groups) {
            // Keep even this callback in the original group order, before that group's
            // clock callbacks. The callback itself is not part of the retained layout.
            if !shown(group) {
                continue;
            }
            for source in &layout.sources[cached.sources.clone()] {
                let Some((task_at, call_at)) = source.call else {
                    continue;
                };
                let instance = &mut field.instances[source.index];
                if phase_only
                    && field
                        .clocks
                        .get(&source.key)
                        .is_none_or(|clock| clock.phase.is_none())
                {
                    continue;
                }
                let request = field.default_states.get(&source.key).copied();
                if source.kind == 1 && request.is_none() {
                    continue;
                }
                let task = &mut layout.tasks[task_at];
                let player = players.entry(source.key).or_insert_with(|| {
                    if request.is_some() {
                        entity_animation::Player::native_choice(&task.rig)
                    } else {
                        entity_animation::Player::new(&task.rig)
                    }
                });
                instance.palette_offset = (palette_end - task.rig.start) as u32;
                palette_end += task.rig.end - task.rig.start;
                let call = &mut task.calls[call_at];
                call.active = true;
                call.instance = ModelInstance {
                    animation: request.unwrap_or(instance.animation),
                    ..*instance
                };
                let clock = field.clocks.get(&source.key);
                call.clocked = clock.is_some();
                if let Some(clock) = clock {
                    player.evaluate_clock(
                        &task.rig,
                        call.instance,
                        clock,
                        &mut [],
                        &mut call.events,
                    );
                } else {
                    task.ordinary = true;
                }
                task.alive = true;
            }
        }
        let run_span;
        {
            // Keys are sorted in both stores. Only this frame's borrow bindings are
            // temporary; occurrence lists and event packet capacities stay retained.
            let mut tasks = layout.tasks.iter_mut().peekable();
            let bound: Vec<_> = players
                .iter_mut()
                .filter_map(|(key, player)| {
                    while tasks.peek().is_some_and(|task| task.key < *key) {
                        tasks.next();
                    }
                    let task = tasks.next_if(|task| task.key == *key)?;
                    task.ordinary.then(|| Mutex::new((player, task)))
                })
                .collect();
            drop(plan_span);
            run_span = viewer_foundation::profiling::span("animation_dispatch_run_commit");
            workers.run(bound.len(), &|at| {
                let mut bound = bound[at].lock().unwrap_or_else(|error| error.into_inner());
                let (player, task) = &mut *bound;
                let Task { rig, calls, .. } = &mut **task;
                for call in calls {
                    if call.active && !call.clocked {
                        player.evaluate(rig, call.instance, time, &mut [], &mut call.events);
                    }
                }
            });
        }
        for source in &layout.sources {
            if let Some((task, call)) = source.call {
                let call = &mut layout.tasks[task].calls[call];
                if call.active {
                    events.extend(call.events.drain(..).map(|mut event| {
                        event.key.model = source.key.0;
                        event
                    }));
                }
            }
        }
        for (key, clock) in &field.clocks {
            if clock.phase.is_none() || layout.alive(key) {
                continue;
            }
            let Some(rig) = rigs.get(key.0) else {
                continue;
            };
            players
                .entry(*key)
                .or_insert_with(|| entity_animation::Player::new(rig))
                .advance_hidden(rig, clock);
        }
        players.retain(|key, _| {
            layout.alive(key)
                || (rigs.get(key.0).is_some()
                    && field
                        .clocks
                        .get(key)
                        .is_some_and(|clock| clock.phase.is_some()))
        });
        drop(run_span);
        true
    }
}

#[cfg(test)]
mod tests;
