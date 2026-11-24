
use crate::{ModelField, ModelGroup, ModelInstance, Rig, entity_animation};
use std::{
    collections::BTreeMap,
    ops::Range,
    sync::Arc,
};

type Key = (usize, u32, u32);

#[derive(Default)]
pub(super) struct Dispatch {
    layout: Option<Layout>,
    created: Vec<Key>,
}

struct Layout {
    instances: usize,
    order: Vec<(usize, usize)>,
    groups: Vec<Group>,
    visible: Vec<bool>,
    sources: Vec<Source>,
    tasks: Vec<Task>,
    supported: bool,
}

struct Group {
    mesh: usize,
    range: Range<u32>,
    rig: Option<Arc<Rig>>,
}

#[derive(Clone, Copy)]
struct Source {
    group: usize,
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
        if self.instances != field.instances.len()
            || self.groups.len() != field.groups.len()
            || !self.order.iter().copied().eq(field.source_order())
        {
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
            order: Vec::new(),
            groups: Vec::with_capacity(field.groups.len()),
            visible: Vec::with_capacity(field.groups.len()),
            sources: Vec::new(),
            tasks: Vec::new(),
            supported: true,
        };
        layout.order.extend(field.source_order());
        for group in &field.groups {
            let rig = rigs.get(group.mesh);
            layout.groups.push(Group {
                mesh: group.mesh,
                range: group.range.clone(),
                rig: rig.map(Arc::clone),
            });
        }
        for &(group_at, index) in &layout.order {
            let Some(group) = field.groups.get(group_at) else {
                layout.supported = false;
                break;
            };
            let Some(rig) = rigs.get(group.mesh) else {
                continue;
            };
            if rig.nodes.iter().all(|node| node.clips.is_empty()) {
                continue;
            }
            layout.supported &= rig.nodes.iter().all(|node| node.native.is_some());
            let Some(&instance) = field.instances.get(index) else {
                layout.supported = false;
                break;
            };
            layout.sources.push(Source {
                group: group_at,
                index,
                key: key(group.mesh, instance),
                kind: kind(instance),
                call: None,
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
    #[expect(clippy::too_many_arguments, reason = "the players and rigs, the field it poses, the time and filter, and the events it records")]
    pub fn animate(
        &mut self,
        players: &mut BTreeMap<Key, entity_animation::Player>,
        rigs: &[Arc<Rig>],
        palette_start: usize,
        field: &mut ModelField,
        time: f32,
        shown: &dyn Fn(&ModelGroup) -> bool,
        events: &mut Vec<entity_animation::Event>,
        label: &'static str,
    ) -> bool {
        self.created.clear();
        let created = &mut self.created;
        let plan_span = viewer_runtime::profiling::span("animation_dispatch_plan");
        if self
            .layout
            .as_ref()
            .is_none_or(|layout| !layout.matches(rigs, field))
        {
            viewer_runtime::view_record::add(label, "dispatch_layout_built", 1);
            self.layout = Some(Layout::build(rigs, field));
        }
        let layout = self.layout.as_mut().unwrap();
        if !layout.supported {
            return false;
        }
        field.prepare_clock_order();
        layout.visible.clear();
        layout.visible.extend(field.groups.iter().map(shown));
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
        for source in &layout.sources {
            if !layout.visible[source.group] {
                continue;
            }
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
                created.push(source.key);
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
                player.evaluate_clock(&task.rig, call.instance, clock, &mut [], &mut call.events);
            } else {
                task.ordinary = true;
            }
            task.alive = true;
        }
        let run_span;
        {
            let mut tasks = layout.tasks.iter_mut().peekable();
            let mut bound: Vec<_> = players
                .iter_mut()
                .filter_map(|(key, player)| {
                    while tasks.peek().is_some_and(|task| task.key < *key) {
                        tasks.next();
                    }
                    let task = tasks.next_if(|task| task.key == *key)?;
                    task.ordinary.then_some((player, task))
                })
                .collect();
            drop(plan_span);
            run_span = viewer_runtime::profiling::span("animation_dispatch_run_commit");
            for (player, task) in &mut bound {
                let Task { rig, calls, .. } = &mut **task;
                for call in calls {
                    if call.active && !call.clocked {
                        player.evaluate(rig, call.instance, time, &mut [], &mut call.events);
                    }
                }
            }
        }
        for source in &layout.sources {
            if let Some((task, call)) = source.call {
                let call = &mut layout.tasks[task].calls[call];
                if call.active {
                    events.extend(call.events.drain(..).map(|mut event| {
                        event.key.model = field.groups[source.group].asset.clone();
                        event
                    }));
                }
            }
        }
        for key in field.clock_order() {
            let clock = &field.clocks[&key];
            if clock.phase.is_none() || layout.alive(&key) {
                continue;
            }
            let Some(rig) = rigs.get(key.0) else {
                continue;
            };
            players
                .entry(key)
                .or_insert_with(|| {
                    created.push(key);
                    entity_animation::Player::new(rig)
                })
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
        if viewer_runtime::view_record::on() {
            use viewer_runtime::view_record::add;
            add(label, "dispatch_sources", layout.sources.len() as u64);
            add(
                label,
                "dispatch_sources_shown",
                layout.sources.iter().filter(|source| layout.visible[source.group]).count() as u64,
            );
            let (mut alive, mut ordinary) = (0, 0);
            for task in &layout.tasks {
                alive += u64::from(task.alive);
                ordinary += u64::from(task.ordinary);
            }
            add(label, "dispatch_tasks_alive", alive);
            add(label, "dispatch_tasks_ordinary", ordinary);
        }
        drop(run_span);
        true
    }

    pub fn created(&self) -> &[Key] {
        &self.created
    }
}

#[cfg(test)]
mod tests;
