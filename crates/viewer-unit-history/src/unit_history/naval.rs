
use std::collections::{BTreeMap, BTreeSet};

use super::{Budget, Entity, Frame, Kind, ModelInstance, Part, Target};
use viewer_runtime::{unit_class::CounterClass, unit_handle::Unit};

#[derive(Clone, Default)]
pub struct Sources {
    pub candidates: Vec<Candidate>,
    pub formations: Vec<Formation>,
    pub combats: super::naval_combat::Sources,
    pub positions: Vec<FormationPlacement>,
}

#[derive(Clone)]
pub struct FormationPlacement {
    pub force: u64,
    pub position: viewer_unit_reads::counter_move::Position,
    pub rotation: f32,
    pub scale: f32,
}

impl Sources {
    #[cfg(any(test, feature = "test-support"))]
    pub fn settled_targets(&self, gates: super::Gates) -> Vec<Target> {
        let provinces: BTreeSet<_> = self.candidates.iter().map(|c| c.province).collect();
        let mut history = History::default();
        history.install(self.clone());
        let frame = Frame {
            delta: 0.0,
            gates,
            shown: [true; 2],
            arrows: true,
            steady: true,
        };
        let mut targets = Vec::new();
        let mut epoch = 0;
        for province in provinces {
            for formation in history.representatives(province, frame).0 {
                let mut entity = Entity {
                    kind: Kind::Ship,
                    scale: 1.0,
                    shrinking: false,
                    parts: BTreeMap::new(),
                };
                formation.update(
                    &mut entity,
                    frame,
                    &mut epoch,
                    &mut Budget::new(i32::MAX, true),
                );
                targets.extend(entity.parts.into_values().map(|part| part.target));
            }
        }
        targets
    }
}

#[derive(Clone)]
pub struct Candidate {
    pub province: u32,
    pub identity: u64,
    pub ordinal: u32,
    pub slot: u32,
    pub priority: u8,
    pub ours: bool,
    pub class: CounterClass,
    pub land_eligible: bool,
    pub sample_count: usize,
    pub next_arc: f32,
    pub route: Vec<u32>,
    pub speed: f32,
    pub min_arc: f32,
    pub arrival_allowed: bool,
    pub day: i32,
}

impl Candidate {
    fn unit(&self) -> Unit {
        match self.class {
            CounterClass::Division => Unit::Division(self.identity),
            CounterClass::RailwayGun => Unit::RailwayGun(self.identity),
            CounterClass::TaskForce => Unit::TaskForce(self.identity),
        }
    }

    fn naval(&self) -> bool {
        self.class == CounterClass::TaskForce
    }
}

#[derive(Clone)]
pub struct Hull {
    pub flags: u64,
    pub graphics: Vec<u8>,
    pub targets: [Option<Target>; 4],
}

impl Hull {
    fn occupant(&self) -> Option<u64> {
        self.targets.iter().flatten().next().map(|t| t.occupant)
    }
}

#[derive(Clone)]
pub struct Formation {
    pub identity: u64,
    pub province: u32,
    pub slot: u32,
    pub roster: Vec<Hull>,
}

impl Formation {
    pub fn targets(&self) -> impl Iterator<Item = Target> + '_ {
        self.roster
            .iter()
            .flat_map(|h| h.targets.iter().copied().flatten())
    }

    pub(super) fn update(
        &self,
        entity: &mut Entity,
        frame: Frame,
        epoch: &mut u32,
        budget: &mut Budget,
    ) {
        if let Some(hull) = self.roster.first().filter(|h| h.flags & 0x400040 != 0) {
            attach(hull, 0, entity, frame, epoch, budget);
        } else {
            entity.parts.remove(&0);
        }
        for slot in 1..=3 {
            let retained = entity.parts.get(&slot).and_then(|part| {
                self.roster.iter().find(|hull| {
                    hull.occupant() == Some(part.target.occupant) && hull.graphics == part.graphics
                })
            });
            if let Some(hull) = retained {
                refresh(hull, slot, entity);
                continue;
            }
            entity.parts.remove(&slot);
            let unused = |hull: &&Hull| {
                hull.occupant().is_some_and(|handle| {
                    !entity
                        .parts
                        .range(1..=3)
                        .any(|(_, part)| part.target.occupant == handle)
                })
            };
            let next = self
                .roster
                .iter()
                .filter(|h| h.flags & (1 << 8) != 0)
                .find(unused)
                .or_else(|| {
                    self.roster
                        .iter()
                        .filter(|h| h.flags & ((1 << 7) | (1 << 39)) != 0)
                        .find(unused)
                });
            if let Some(hull) = next {
                attach(hull, slot, entity, frame, epoch, budget);
            }
        }
    }
}

fn refresh(hull: &Hull, slot: u32, entity: &mut Entity) {
    if let (Some(target), Some(part)) = (hull.targets[slot as usize], entity.parts.get_mut(&slot)) {
        let mesh = part.target.mesh;
        let animation = part.target.instance.animation;
        part.target = target;
        part.target.mesh = mesh;
        part.target.instance.animation = animation;
    }
}

fn attach(
    hull: &Hull,
    slot: u32,
    entity: &mut Entity,
    frame: Frame,
    epoch: &mut u32,
    budget: &mut Budget,
) {
    let Some(target) = hull.targets[slot as usize] else {
        return;
    };
    if entity.parts.get(&slot).is_some_and(|part| {
        part.target.occupant == target.occupant && part.graphics == hull.graphics
    }) {
        refresh(hull, slot, entity);
    } else if budget.take() {
        let mut part = Part::new(target, epoch, frame.steady);
        part.graphics = hull.graphics.clone();
        entity.parts.insert(slot, part);
    }
}

pub(super) struct History {
    sources: Sources,
    candidates: BTreeMap<u32, Vec<usize>>,
    formations: BTreeMap<u64, usize>,
    arcs: BTreeMap<Unit, ArcState>,
    selected: BTreeSet<Unit>,
    representatives: [usize; 0x26],
}

impl Default for History {
    fn default() -> Self {
        Self {
            sources: Sources::default(),
            candidates: BTreeMap::new(),
            formations: BTreeMap::new(),
            arcs: BTreeMap::new(),
            selected: BTreeSet::new(),
            representatives: [0; 0x26],
        }
    }
}

#[cfg_attr(test, derive(Debug, PartialEq))]
struct ArcState {
    province: u32,
    route: Vec<u32>,
    pending_route: Vec<u32>,
    sample_count: usize,
    next_arc: f32,
    arrived: bool,
    arrival_deadline: Option<i32>,
    arc: f32,
}

impl ArcState {
    fn new(candidate: &Candidate) -> Self {
        Self {
            province: candidate.province,
            route: candidate.route.clone(),
            pending_route: Vec::new(),
            sample_count: candidate.sample_count,
            next_arc: candidate.next_arc,
            arrived: false,
            arrival_deadline: None,
            arc: 0.0,
        }
    }

    fn install(&mut self, candidate: &Candidate) {
        if self.province == candidate.province && self.route == candidate.route {
            self.arrived = false;
            self.arrival_deadline = None;
            self.sample_count = candidate.sample_count;
            self.next_arc = candidate.next_arc;
            return;
        }
        let arrived = self.sample_count != 0
            && self.route.first() == Some(&candidate.province)
            && self.route[1..] == candidate.route;
        if arrived
            && candidate.arrival_allowed
            && self
                .arrival_deadline
                .is_none_or(|deadline| candidate.day < deadline)
        {
            self.arrived = true;
            self.pending_route.clone_from(&candidate.route);
            return;
        }
        self.route.clone_from(&candidate.route);
        self.initialize(candidate);
    }

    fn initialize(&mut self, candidate: &Candidate) {
        self.province = candidate.province;
        self.sample_count = candidate.sample_count;
        self.next_arc = candidate.next_arc;
        self.arrived = false;
        self.arrival_deadline = None;
        self.arc = 0.0;
    }

    fn advance(&mut self, candidate: &Candidate, frame: Frame) {
        if self.arrived {
            self.arrival_deadline.get_or_insert(candidate.day + 6);
            self.arc = if frame.steady {
                self.next_arc
            } else {
                (self.arc + frame.delta * candidate.speed).clamp(0.0, self.next_arc)
            };
            if self.arc == self.next_arc {
                std::mem::swap(&mut self.route, &mut self.pending_route);
                self.initialize(candidate);
            }
        } else {
            self.arc = if self.route.is_empty() || self.sample_count == 0 {
                0.0
            } else if frame.steady {
                1.0
            } else {
                (self.arc + frame.delta * candidate.speed).clamp(candidate.min_arc, 1.0)
            };
        }
    }
}

impl History {
    #[cfg(test)]
    pub(super) fn assert_same_state(&self, other: &Self) {
        assert_eq!(self.candidates, other.candidates);
        assert_eq!(self.formations, other.formations);
        assert_eq!(self.arcs, other.arcs);
        assert_eq!(self.selected, other.selected);
        assert_eq!(self.representatives, other.representatives);
    }

    #[cfg(test)]
    pub(super) fn has_candidates(&self, province: u32) -> bool {
        self.candidates.get(&province).is_some_and(|c| !c.is_empty())
    }

    pub(super) fn candidate_provinces(&self) -> impl Iterator<Item = u32> + '_ {
        self.candidates
            .iter()
            .filter(|(_, c)| !c.is_empty())
            .map(|(&province, _)| province)
    }

    #[cfg(test)]
    pub(super) fn empty_candidates_for_test(&mut self, province: u32) {
        self.candidates.insert(province, Vec::new());
    }

    pub(super) fn set_selection(&mut self, selected: &BTreeSet<Unit>) {
        if self.selected != *selected {
            self.selected.clone_from(selected);
        }
    }

    pub(super) fn install(&mut self, sources: Sources) {
        self.candidates.clear();
        for (index, candidate) in sources.candidates.iter().enumerate() {
            self.candidates
                .entry(candidate.province)
                .or_default()
                .push(index);
        }
        for indices in self.candidates.values_mut() {
            indices.sort_by_key(|&index| sources.candidates[index].ordinal);
        }
        self.formations = sources
            .formations
            .iter()
            .enumerate()
            .map(|(i, f)| (f.identity, i))
            .collect();
        let current: BTreeMap<_, _> = sources
            .candidates
            .iter()
            .map(|candidate| (candidate.unit(), candidate))
            .collect();
        self.arcs.retain(|key, _| current.contains_key(key));
        for (key, candidate) in current {
            self.arcs
                .entry(key)
                .and_modify(|state| state.install(candidate))
                .or_insert_with(|| ArcState::new(candidate));
        }
        self.sources = sources;
    }

    pub(super) fn reset(&mut self, provinces: Option<&BTreeSet<u32>>) {
        if provinces.is_none() {
            for candidate in &self.sources.candidates {
                let state = self.arcs.get_mut(&candidate.unit()).unwrap();
                if state.arrived {
                    std::mem::swap(&mut state.route, &mut state.pending_route);
                }
                state.initialize(candidate);
            }
        }
    }

    pub(super) fn representatives(
        &mut self,
        province: u32,
        frame: Frame,
    ) -> (impl Clone + Iterator<Item = &Formation>, Option<(u64, u64)>) {
        let mut slots: [Option<(usize, u8)>; 0x26] = [None; 0x26];
        let mut counts = [0u16; 0x26];
        for &index in self.candidates.get(&province).into_iter().flatten() {
            let candidate = &self.sources.candidates[index];
            let count = &mut counts[candidate.slot as usize];
            if *count == 0x400 {
                continue;
            }
            *count += 1;
            let state = self.arcs.get_mut(&candidate.unit()).unwrap();
            state.advance(candidate, frame);
            let candidate_arc = state.arc;
            let moving = state.sample_count != 0;
            if (candidate.naval() && !frame.shown[Kind::Ship as usize])
                || (!candidate.naval()
                    && (!candidate.land_eligible || !frame.shown[Kind::Land as usize]))
            {
                continue;
            }
            let candidate_priority = if candidate.ours && self.selected.contains(&candidate.unit())
            {
                4
            } else {
                candidate.priority
            };
            let slot = &mut slots[candidate.slot as usize];
            let Some((current, priority)) = slot else {
                *slot = Some((index, candidate_priority));
                continue;
            };
            let held = &self.sources.candidates[*current];
            let held_arc = self.arcs[&held.unit()].arc;
            if moving && candidate_arc > held_arc {
                *current = index;
            } else if candidate_priority > *priority
                || (candidate_priority == *priority && candidate.ours)
            {
                if !held.ours || candidate.ours {
                    *current = index;
                }
            } else {
                continue;
            }
            *priority = candidate_priority;
        }
        let combat_slot = if slots[9].map_or(0, |(_, priority)| priority)
            < slots[10].map_or(0, |(_, priority)| priority)
        {
            10
        } else {
            9
        };
        let combat = slots[combat_slot].and_then(|(index, _)| {
            let candidate = &self.sources.candidates[index];
            candidate
                .naval()
                .then_some(candidate)
                .and_then(|candidate| {
                    self.sources
                        .combats
                        .links
                        .get(&candidate.identity)
                        .map(|links| (links[1], links[0]))
                })
        });
        let mut count = 0;
        for (index, _) in slots.into_iter().flatten() {
            let candidate = &self.sources.candidates[index];
            if !candidate.naval() || matches!(candidate.slot, 9 | 10 | 19) {
                continue;
            }
            if let Some(&i) = self.formations.get(&candidate.identity) {
                let formation = &self.sources.formations[i];
                if formation.slot == candidate.slot && formation.province == province {
                    self.representatives[count] = i;
                    count += 1;
                }
            }
        }
        let formations = &self.sources.formations;
        (
            self.representatives[..count]
                .iter()
                .map(move |&i| &formations[i]),
            combat,
        )
    }

    pub(super) fn combat(
        &self,
        province: u32,
        selected: Option<(u64, u64)>,
    ) -> Option<super::naval_combat::Selection<'_>> {
        let (id, first) = selected?;
        let battle = self.sources.combats.battles.get(&id)?;
        let geometry = self.sources.combats.roots.get(&province)?;
        Some(super::naval_combat::Selection {
            battle,
            first,
            root: ModelInstance {
                position: geometry.position,
                scale: geometry.scale,
                rotation: geometry.rotation,
                ..battle.root.instance
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(delta: f32) -> Frame {
        Frame {
            delta,
            gates: super::super::test_support::GATES,
            shown: [true; 2],
            arrows: false,
            steady: false,
        }
    }

    fn candidate(identity: u64, priority: u8, ours: bool) -> Candidate {
        Candidate {
            province: 9206,
            identity,
            ordinal: identity as u32,
            slot: 0,
            priority,
            ours,
            class: CounterClass::TaskForce,
            land_eligible: false,
            sample_count: 0,
            next_arc: 0.0,
            route: Vec::new(),
            speed: 1.0,
            min_arc: 0.0,
            arrival_allowed: true,
            day: 10,
        }
    }

    fn sources(candidates: Vec<Candidate>) -> Sources {
        let formations = candidates
            .iter()
            .filter(|c| c.naval())
            .map(|c| Formation {
                identity: c.identity,
                province: c.province,
                slot: c.slot,
                roster: Vec::new(),
            })
            .collect();
        Sources {
            candidates,
            formations,
            ..Default::default()
        }
    }

    fn selected(history: &mut History, delta: f32) -> Vec<u64> {
        history
            .representatives(9206, frame(delta)).0
            .map(|f| f.identity)
            .collect()
    }

    #[test]
    fn a_province_with_no_candidate_offers_no_formation_and_names_no_combat() {
        let mut history = History {
            representatives: [27; 0x26],
            ..Default::default()
        };
        assert!(!history.has_candidates(42));
        let (formations, combat) = history.representatives(42, frame(0.25));
        assert_eq!(formations.count(), 0);
        assert_eq!(combat, None);
    }

    #[test]
    fn candidate_provinces_ascend_under_the_point_lookup_predicate() {
        let mut history = History::default();
        let at = |province, identity, ordinal| Candidate {
            province,
            ordinal,
            ..candidate(identity, 3, true)
        };
        history.install(sources(vec![
            at(u32::MAX, 1, 0),
            at(9206, 2, 1),
            at(0, 3, 2),
            at(9206, 4, 3),
            at(u32::MAX - 1, 5, 4),
            at(17, 6, 5),
        ]));
        history.empty_candidates_for_test(17);
        history.empty_candidates_for_test(40);
        let provinces: Vec<_> = history.candidate_provinces().collect();
        assert_eq!(provinces, [0, 9206, u32::MAX - 1, u32::MAX]);
        let probes = [0, 1, 17, 40, 9205, 9206, 9207, u32::MAX - 1, u32::MAX];
        assert_eq!(
            probes
                .into_iter()
                .filter(|&province| history.has_candidates(province))
                .collect::<Vec<_>>(),
            provinces
        );
    }

    #[test]
    fn viewer_selection_changes_representatives_without_reinstalling_the_hour() {
        let mut history = History::default();
        history.install(sources(vec![candidate(1, 3, true), candidate(2, 3, true)]));
        assert_eq!(selected(&mut history, 0.0), [2]);
        history.set_selection(&BTreeSet::from([Unit::TaskForce(1)]));
        assert_eq!(selected(&mut history, 0.0), [1]);
        history.set_selection(&BTreeSet::from([Unit::TaskForce(1), Unit::TaskForce(2)]));
        assert_eq!(
            selected(&mut history, 0.0),
            [2],
            "selected ties keep own last"
        );
        history.set_selection(&BTreeSet::new());
        assert_eq!(selected(&mut history, 0.0), [2]);

        history.install(sources(vec![
            candidate(1, 2, false),
            candidate(2, 1, false),
        ]));
        history.set_selection(&BTreeSet::from([Unit::TaskForce(2)]));
        assert_eq!(
            selected(&mut history, 0.0),
            [1],
            "foreign selection adds no priority"
        );
    }

    #[test]
    fn viewer_selection_distinguishes_land_classes_and_keeps_camera_gates() {
        let mut history = History::default();
        let mut land = candidate(2, 3, true);
        land.class = CounterClass::RailwayGun;
        land.land_eligible = true;
        history.install(sources(vec![candidate(1, 3, true), land]));
        history.set_selection(&BTreeSet::from([Unit::TaskForce(1), Unit::Division(2)]));
        assert_eq!(
            selected(&mut history, 0.0),
            [1],
            "a division handle cannot select a gun"
        );
        history.set_selection(&BTreeSet::from([Unit::TaskForce(1), Unit::RailwayGun(2)]));
        assert!(
            selected(&mut history, 0.0).is_empty(),
            "own selected land wins its slot"
        );
        let mut closed = frame(0.0);
        closed.shown[Kind::Land as usize] = false;
        assert_eq!(
            history
                .representatives(9206, closed).0
                .next()
                .unwrap()
                .identity,
            1
        );
    }

    #[test]
    fn viewer_selection_preserves_moving_arc_precedence_and_animation() {
        let mut history = History::default();
        let mut moving = candidate(2, 0, false);
        moving.sample_count = 8;
        moving.route = vec![1434];
        history.install(sources(vec![candidate(1, 3, true), moving]));
        history.set_selection(&BTreeSet::from([Unit::TaskForce(1)]));
        assert_eq!(selected(&mut history, 0.0), [1]);
        assert_eq!(selected(&mut history, 0.25), [2]);
        history.set_selection(&BTreeSet::new());
        assert_eq!(selected(&mut history, 0.0), [2]);
        assert_eq!(history.arcs[&Unit::TaskForce(2)].arc, 0.25);
    }

    #[test]
    fn land_classes_with_equal_handles_keep_separate_arc_state() {
        let mut history = History::default();
        let mut division = candidate(2, 3, true);
        division.class = CounterClass::Division;
        division.route = vec![1434];
        division.sample_count = 8;
        let mut gun = division.clone();
        gun.class = CounterClass::RailwayGun;
        gun.route = vec![1660];
        history.install(sources(vec![division, gun]));
        assert_eq!(history.arcs.len(), 2);
        assert_eq!(history.arcs[&Unit::Division(2)].route, [1434]);
        assert_eq!(history.arcs[&Unit::RailwayGun(2)].route, [1660]);
    }

    #[test]
    fn naval_representative_ties_keep_foreign_first_but_ours_last() {
        let mut history = History::default();
        history.install(sources(vec![
            candidate(1, 2, false),
            candidate(2, 2, false),
        ]));
        assert_eq!(selected(&mut history, 0.0), [1]);
        history.install(sources(vec![candidate(1, 3, true), candidate(2, 3, true)]));
        assert_eq!(selected(&mut history, 0.0), [2]);
        history.install(sources(vec![
            candidate(1, 2, true),
            candidate(2, 3, false),
            candidate(3, 2, true),
        ]));
        assert_eq!(
            selected(&mut history, 0.0),
            [1],
            "foreign pointer held, but priority becomes 3"
        );
    }

    #[test]
    fn representative_stops_at_the_native_bucket_capacity() {
        let mut history = History::default();
        history.install(sources(
            (1..=0x401).map(|id| candidate(id, 3, true)).collect(),
        ));
        assert_eq!(selected(&mut history, 0.0), [0x400]);
    }

    #[test]
    fn free_arc_can_override_ownership_only_with_nonzero_samples() {
        let mut history = History::default();
        let mut moving = candidate(2, 0, false);
        moving.route = vec![9206, 1434];
        history.install(sources(vec![candidate(1, 3, true), moving.clone()]));
        assert_eq!(selected(&mut history, 0.25), [1]);
        moving.sample_count = 8;
        history.install(sources(vec![candidate(1, 3, true), moving]));
        assert_eq!(selected(&mut history, 0.25), [2]);
        assert_eq!(history.arcs[&Unit::TaskForce(2)].arc, 0.25);
        history.reset(None);
        assert_eq!(selected(&mut history, 0.0), [1], "zero arc is a strict tie");
        assert_eq!(selected(&mut history, 0.25), [2]);
    }

    #[test]
    fn animator_resets_on_route_change_and_fast_speed_forces_one() {
        let mut history = History::default();
        let mut moving = candidate(2, 0, false);
        moving.sample_count = 8;
        moving.route = vec![9206, 1434];
        history.install(sources(vec![candidate(1, 3, true), moving.clone()]));
        assert_eq!(selected(&mut history, 0.5), [2]);
        moving.route[1] = 1660;
        history.install(sources(vec![candidate(1, 3, true), moving.clone()]));
        assert_eq!(selected(&mut history, 0.0), [1]);
        moving.min_arc = 1.0;
        history.install(sources(vec![candidate(1, 3, true), moving]));
        assert_eq!(selected(&mut history, 0.0), [2]);
        assert_eq!(history.arcs[&Unit::TaskForce(2)].arc, 1.0);
    }

    #[test]
    fn representatives_prepare_arc_storage_and_reuse_native_slot_buffer() {
        let mut history = History::default();
        let candidates = (0..0x26)
            .rev()
            .map(|slot| {
                let mut moving = candidate(u64::from(slot + 1), 3, true);
                moving.slot = slot;
                moving.route = vec![9206, 1434];
                moving.sample_count = 8;
                moving.next_arc = 5.0;
                moving
            })
            .collect();
        history.install(sources(candidates));
        assert_eq!(history.arcs.len(), 0x26);
        let route = history.arcs[&Unit::TaskForce(1)].route.as_ptr();
        let buffer = history.representatives.as_ptr();
        for _ in 0..3 {
            let mut previous = None;
            let mut count = 0;
            for formation in history.representatives(9206, frame(0.1)).0 {
                assert!(previous.is_none_or(|slot| slot < formation.slot));
                assert!(!matches!(formation.slot, 9 | 10 | 19));
                previous = Some(formation.slot);
                count += 1;
            }
            assert_eq!(count, 0x26 - 3);
            assert_eq!(history.arcs.len(), 0x26);
            assert_eq!(history.arcs[&Unit::TaskForce(1)].route.as_ptr(), route);
            assert_eq!(history.representatives.as_ptr(), buffer);
        }
    }

    #[test]
    fn perspective_entity_reset_keeps_animator_but_seek_restarts_it() {
        let mut history = History::default();
        let mut moving = candidate(2, 0, false);
        moving.sample_count = 8;
        moving.route = vec![1434];
        history.install(sources(vec![candidate(1, 3, true), moving]));
        assert_eq!(selected(&mut history, 0.25), [2]);
        history.reset(Some(&[9206].into_iter().collect()));
        assert_eq!(selected(&mut history, 0.0), [2]);
        assert_eq!(history.arcs[&Unit::TaskForce(2)].arc, 0.25);
        history.reset(None);
        assert_eq!(selected(&mut history, 0.0), [1]);
        assert_eq!(history.arcs[&Unit::TaskForce(2)].arc, 0.0);
    }

    #[test]
    fn arrival_glides_to_old_next_anchor_then_rebuilds_at_zero() {
        let mut history = History::default();
        let mut moving = candidate(2, 0, false);
        moving.sample_count = 8;
        moving.next_arc = 1.5;
        moving.route = vec![1434, 1660];
        history.install(sources(vec![moving.clone()]));
        assert_eq!(selected(&mut history, 0.75), [2]);
        let old_route = history.arcs[&Unit::TaskForce(2)].route.as_ptr();
        moving.province = 1434;
        moving.route = vec![1660];
        moving.next_arc = 8.0;
        let mut standing = candidate(1, 3, true);
        standing.province = 1434;
        history.install(sources(vec![standing, moving.clone()]));
        assert_eq!(history.arcs[&Unit::TaskForce(2)].arc, 0.75);
        assert_eq!(history.arcs[&Unit::TaskForce(2)].route.as_ptr(), old_route);
        let winner = |history: &mut History, delta| {
            history
                .representatives(1434, frame(delta)).0
                .next()
                .unwrap()
                .identity
        };
        assert_eq!(winner(&mut history, 0.5), 2);
        assert_eq!(history.arcs[&Unit::TaskForce(2)].arc, 1.25);
        assert_eq!(history.arcs[&Unit::TaskForce(2)].arrival_deadline, Some(16));
        assert_eq!(winner(&mut history, 0.5), 1);
        let state = &history.arcs[&Unit::TaskForce(2)];
        assert_eq!(state.arc, 0.0, "rebuild consumes this frame's advance");
        assert_eq!(state.province, 1434);
        assert_eq!(state.route, [1660]);
        assert_eq!(state.next_arc, 8.0);
        assert_eq!(state.arrival_deadline, None);
        assert_eq!(winner(&mut history, 0.25), 2);

        moving.province = 1660;
        moving.route.clear();
        moving.sample_count = 0;
        history.install(sources(vec![moving]));
        assert_eq!(history.arcs[&Unit::TaskForce(2)].arc, 0.25);
        assert_eq!(history.arcs[&Unit::TaskForce(2)].sample_count, 8);
        assert_eq!(
            history
                .representatives(1660, frame(1.0)).0
                .next()
                .unwrap()
                .identity,
            2
        );
        assert_eq!(history.arcs[&Unit::TaskForce(2)].arc, 1.25);
        history.reset(None);
        assert_eq!(history.arcs[&Unit::TaskForce(2)].arc, 0.0);
        assert!(history.arcs[&Unit::TaskForce(2)].route.is_empty());
    }

    #[test]
    fn arrival_glide_expires_after_six_days_or_snaps_for_speed_or_combat() {
        for (allowed, day, route) in [
            (true, 16, vec![1660]),
            (false, 10, vec![1660]),
            (true, 10, vec![9999]),
        ] {
            let mut history = History::default();
            let mut moving = candidate(2, 0, false);
            moving.sample_count = 8;
            moving.next_arc = 10.0;
            moving.route = vec![1434, 1660];
            history.install(sources(vec![moving.clone()]));
            selected(&mut history, 0.5);
            moving.province = 1434;
            moving.route = vec![1660];
            history.install(sources(vec![moving.clone()]));
            history.representatives(1434, frame(0.0)).0.count();
            assert_eq!(history.arcs[&Unit::TaskForce(2)].arc, 0.5);
            moving.day = day;
            moving.arrival_allowed = allowed;
            moving.route = route;
            history.install(sources(vec![moving]));
            assert_eq!(history.arcs[&Unit::TaskForce(2)].arc, 0.0);
            assert_eq!(history.arcs[&Unit::TaskForce(2)].arrival_deadline, None);
        }
    }

    #[test]
    fn naval_gate_and_port_or_combat_slot_never_offer_ordinary_models() {
        let mut history = History::default();
        for slot in [9, 10, 19] {
            let mut c = candidate(1, 3, true);
            c.slot = slot;
            history.install(sources(vec![c]));
            assert!(selected(&mut history, 0.0).is_empty());
        }
        history.install(sources(vec![candidate(1, 3, true)]));
        let mut closed = frame(0.0);
        closed.shown[1] = false;
        assert!(history.representatives(9206, closed).0.next().is_none());
    }

    #[test]
    fn carrier_plate_only_when_another_naval_representative_wins_the_slot() {
        let mut history = History::default();
        let mut cv = candidate((944 << 32) | 61, 3, true);
        cv.ordinal = 0;
        let mut escort = candidate((7 << 32) | 61, 3, true);
        escort.ordinal = 1;
        let escort_id = escort.identity;
        history.install(sources(vec![cv, escort]));
        for height in [
            163.710_052_490_234_38_f64 as f32,
            162.928_390_502_929_7_f64 as f32,
        ] {
            let mut frame = frame(1.0 / 120.0);
            frame.shown = [height < 120.0, height < 240.0];
            assert_eq!(
                history
                    .representatives(9206, frame).0
                    .map(|f| f.identity)
                    .collect::<Vec<_>>(),
                [escort_id],
                "Choosing another representative leaves the carrier plate without its own formation"
            );
        }
    }

    #[test]
    fn land_representatives_require_both_native_terrain_and_camera_gates() {
        let mut history = History::default();
        let ship = candidate(1, 3, true);
        let mut land = candidate(2, 3, true);
        land.class = CounterClass::Division;
        history.install(sources(vec![ship.clone(), land.clone()]));
        assert_eq!(
            selected(&mut history, 0.0),
            [1],
            "sea stationary land is ineligible"
        );
        land.land_eligible = true;
        history.install(sources(vec![ship, land]));
        assert!(
            selected(&mut history, 0.0).is_empty(),
            "eligible land wins the slot"
        );
        let mut above_land = frame(0.0);
        above_land.shown[Kind::Land as usize] = false;
        assert_eq!(
            history
                .representatives(9206, above_land).0
                .next()
                .unwrap()
                .identity,
            1
        );
    }

    fn hull(handle: u64, flags: u64, graphics: &[u8]) -> Hull {
        Hull {
            flags,
            graphics: graphics.to_vec(),
            targets: std::array::from_fn(|part| {
                Some(Target {
                    identity: 944,
                    occupant: handle,
                    part: part as u32,
                    mesh: handle as usize,
                    ..super::super::test_support::target(9206, Kind::Ship)
                })
            }),
        }
    }

    fn entity() -> Entity {
        Entity {
            kind: Kind::Ship,
            scale: 1.0,
            shrinking: false,
            parts: BTreeMap::new(),
        }
    }

    fn formation(roster: Vec<Hull>) -> Formation {
        Formation {
            identity: 944,
            province: 9206,
            slot: 0,
            roster,
        }
    }

    #[test]
    fn supports_retain_handles_graphics_and_mesh_after_roster_reorders() {
        let mut entity = entity();
        let mut epoch = 0;
        let roster = vec![
            hull(271, 0x400040, b"cv"),
            hull(11, 1 << 8, b"cl"),
            hull(12, 1 << 8, b"cl"),
            hull(13, 1 << 7, b"dd"),
        ];
        formation(roster.clone()).update(
            &mut entity,
            frame(0.0),
            &mut epoch,
            &mut Budget::new(4, false),
        );
        assert_eq!(entity.parts[&1].target.occupant, 11);
        let old_epoch = entity.parts[&1].epoch;
        let mut reordered = vec![
            roster[0].clone(),
            roster[2].clone(),
            roster[3].clone(),
            roster[1].clone(),
        ];
        reordered[3].flags = 0;
        reordered[3].targets[1].as_mut().unwrap().mesh = 99;
        reordered[3].targets[1].as_mut().unwrap().instance.position = [4.0; 3];
        formation(reordered).update(
            &mut entity,
            frame(0.1),
            &mut epoch,
            &mut Budget::new(0, false),
        );
        assert_eq!(entity.parts[&1].target.occupant, 11);
        assert_eq!(entity.parts[&1].target.mesh, 11);
        assert_eq!(entity.parts[&1].target.instance.position, [4.0; 3]);
        assert_eq!(entity.parts[&1].epoch, old_epoch);
    }

    #[test]
    fn mesh_budget_yields_one_hull_then_a_full_formation() {
        let mut entity = entity();
        let mut epoch = 0;
        let formation = formation(vec![
            hull(271, 0x400040, b"cv"),
            hull(11, 1 << 8, b"cl"),
            hull(12, 1 << 7, b"dd"),
            hull(13, 1 << 39, b"ss"),
        ]);
        formation.update(
            &mut entity,
            frame(0.0),
            &mut epoch,
            &mut Budget::new(1, false),
        );
        assert_eq!(entity.parts.keys().copied().collect::<Vec<_>>(), [0]);
        assert_eq!(epoch, 1);
        formation.update(
            &mut entity,
            frame(0.1),
            &mut epoch,
            &mut Budget::new(3, false),
        );
        assert_eq!(
            entity.parts.keys().copied().collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        assert_eq!(
            entity
                .parts
                .values()
                .map(|p| p.target.occupant)
                .collect::<Vec<_>>(),
            [271, 11, 12, 13]
        );
    }

    #[test]
    fn capital_holds_but_stale_support_drops_when_budget_is_spent() {
        let mut entity = entity();
        let mut epoch = 0;
        formation(vec![hull(271, 0x400040, b"cv"), hull(11, 1 << 8, b"cl")]).update(
            &mut entity,
            frame(0.0),
            &mut epoch,
            &mut Budget::new(2, false),
        );
        formation(vec![
            hull(272, 0x400040, b"cv"),
            hull(11, 1 << 8, b"new_cl"),
        ])
        .update(
            &mut entity,
            frame(0.1),
            &mut epoch,
            &mut Budget::new(0, false),
        );
        assert_eq!(entity.parts.len(), 1);
        assert_eq!(entity.parts[&0].target.occupant, 271);
    }
}
