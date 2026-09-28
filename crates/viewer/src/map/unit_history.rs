//! Viewer-owned unit entity lifetimes: which land figures and ship hulls
//! exist, at what popup scale, and since when their animation runs. The
//! viewer is the game's client here, so this runs on the viewer's frame clock.
//!
//! **Popup scale, not alpha.** `FadeOrDestroyUnitSlotEntity@141279F50`
//! shrinks the uniform transform scale, `max(0, scale - dt *
//! MESH_POPUP_SCALE_DOWN_SPEED)` into all three axes (`14127A047..14127A062`
//! -> `1424E7450` -> `14227BA20`), and destroys an entity whose scale was
//! already zero when visited. Formations shrink at
//! `SHIP_POPUP_SCALE_DOWN_SPEED`, their root scale carrying the hull offsets
//! (`141652B90`). Both grow at `MESH_POPUP_SCALE_UP_SPEED` (`14127F000`).
//!
//! **Visits.** `UpdateUnitStackGraphics@14127C910` visits a kind that just
//! lost its show bit in its gate-loss loop and again in
//! `FadeInactiveUnitStackModels@14127A090`; later frames visit once.
//! Provinces in view update every frame. `CGraphicalMap::update@140B49340`
//! advances a phase `(+0x130 + 1) % 100` per frame before `140B4C120`
//! updates an off-screen province, only when its index modulo 100 equals the
//! phase. That update destroys at once (`141277FC0` clears the show bits,
//! `14127D260` passes camera admission false); until then the entity, its
//! transform and its animation clock survive, so a quick pan back resumes
//! them. Hidden entities' clocks advance (`14227CE00` -> `14226EF30`). A
//! selected unit bypasses the phase (`province+0x1764`); the viewer has no
//! unit selection.
//!
//! Divisions (transports included) and railway guns share the land path,
//! naval formations the ship path. Naval combat graphics are destroyed on
//! land-gate loss (`14127CAA9`); aircraft and trains have their own
//! managers. Not modelled: the per-frame creation budget
//! `MAX_MESHES_LOADED_PER_FRAME` (see the viewer README).

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use super::{
    camera::Camera,
    map_buildings::province_view,
    models::{ModelField, ModelInstance},
    unit_models::Gates,
};

/// The model path an entity follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Land,
    Ship,
}

/// One figure or hull an hour places.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Target {
    pub province: u32,
    /// The stack's oldest division, or the formation.
    pub identity: u64,
    /// The hull within a formation; 0 for a land figure.
    pub part: u32,
    pub kind: Kind,
    pub mesh: usize,
    pub instance: ModelInstance,
    /// Formation root; shrinking also moves hull attachments toward it.
    pub origin: [f32; 3],
}

/// What one native unit slot holds: a land stack or a naval formation.
type Slot = (Kind, u64);

impl Target {
    fn slot(self) -> Slot {
        (self.kind, self.identity)
    }
}

/// An hour's models: the field keeps their meshes resident, the targets
/// feed the [`History`].
#[derive(Clone, Default)]
pub struct Field {
    pub models: ModelField,
    pub targets: Vec<Target>,
}

/// One slot's targets, by part.
type Parts = BTreeMap<u32, Target>;

/// One figure, or one hull attached under a formation's root.
struct Part {
    target: Target,
    epoch: u32,
}

/// A slot's entity. Its popup scale belongs to the slot (`+0xD38`), so the
/// hulls of a formation share their root's.
struct Entity {
    kind: Kind,
    scale: f32,
    /// Shrinking through the inactive path, which drops the transport scale.
    shrinking: bool,
    parts: BTreeMap<u32, Part>,
}

impl Entity {
    /// One `141279F50` visit: it tests the OLD scale, then subtracts, so zero
    /// lasts one visit. False means the entity is destroyed.
    fn shrink(&mut self, delta: f32, gates: Gates) -> bool {
        if self.scale <= 0.0 {
            return false;
        }
        let speed = match self.kind {
            Kind::Land => gates.scale_down,
            Kind::Ship => gates.ship_scale_down,
        };
        self.scale = (self.scale - delta * speed).max(0.0);
        self.shrinking = true;
        true
    }

    fn placed(&self) -> impl Iterator<Item = (usize, ModelInstance)> + '_ {
        self.parts.values().map(|part| {
            let mut instance = part.target.instance;
            instance.animation_epoch = part.epoch;
            // FadeInactiveUnitStackModels passes land multiplier 1.0, including
            // transports; the admitted path uses the authored unit multiplier.
            instance.scale = if self.shrinking && self.kind == Kind::Land {
                self.scale
            } else {
                instance.scale * self.scale
            };
            if self.kind == Kind::Ship {
                instance.position = std::array::from_fn(|axis| {
                    let root = part.target.origin[axis];
                    root + (instance.position[axis] - root) * self.scale
                });
            }
            (part.target.mesh, instance)
        })
    }
}

/// A new incarnation's animation epoch. Explicit dump clocks keep their
/// sampling convention (epoch 0); only live incarnations start now.
fn incarnation(epoch: &mut u32, steady: bool) -> u32 {
    *epoch = epoch.wrapping_add(1);
    if steady { 0 } else { *epoch }
}

/// What one viewer frame applies to each province it updates.
#[derive(Clone, Copy)]
struct Frame {
    delta: f32,
    gates: Gates,
    /// Whether the camera shows land models and ships.
    shown: [bool; 2],
    /// Whether it shows unit arrows.
    arrows: bool,
    steady: bool,
}

/// One province's unit graphics.
#[derive(Default)]
struct Province {
    /// The land and ship show bits of its last update (`+0x175D`).
    shown: [bool; 2],
    /// The arrow flag of its last update (`+0x175E`).
    arrows: bool,
    entities: BTreeMap<Slot, Entity>,
}

impl Province {
    /// The update of a province in view.
    fn visit(&mut self, targets: Option<&BTreeMap<Slot, Parts>>, frame: Frame, epoch: &mut u32) {
        let placed = |slot: &Slot| targets.is_some_and(|targets| targets.contains_key(slot));
        let was = self.shown;
        self.entities.retain(|slot, entity| {
            let kind = slot.0 as usize;
            if frame.steady {
                return frame.shown[kind] && placed(slot);
            }
            if !frame.shown[kind] {
                // The gate-loss loop precedes 14127A090, so the first frame
                // out of range visits twice.
                return (!was[kind] || entity.shrink(frame.delta, frame.gates))
                    && entity.shrink(frame.delta, frame.gates);
            }
            // A missing stack of a kind shown now and before is destroyed at
            // once (14127A090 param 7); otherwise it shrinks.
            placed(slot) || (!was[kind] && entity.shrink(frame.delta, frame.gates))
        });
        for (&slot, parts) in targets.into_iter().flatten() {
            let kind = slot.0 as usize;
            if !frame.shown[kind] {
                continue;
            }
            // A stack whose figure changes gets a new entity (1412785B0).
            if slot.0 == Kind::Land
                && self.entities.get(&slot).is_some_and(|entity| {
                    entity.parts.iter().any(|(part, old)| {
                        parts
                            .get(part)
                            .is_some_and(|new| new.mesh != old.target.mesh)
                    })
                })
            {
                self.entities.remove(&slot);
            }
            let created = !self.entities.contains_key(&slot);
            let entity = self.entities.entry(slot).or_insert_with(|| Entity {
                kind: slot.0,
                // 1412785B0 and 14127F000: full size where the kind was
                // already shown.
                scale: if frame.steady || was[kind] { 1.0 } else { 0.0 },
                shrinking: false,
                parts: BTreeMap::new(),
            });
            // Hulls attach and detach under the formation's root without
            // touching its scale (141653F00 -> 1416542A0).
            entity.parts.retain(|part, _| parts.contains_key(part));
            for (&index, &target) in parts {
                let part = entity.parts.entry(index).or_insert_with(|| Part {
                    target,
                    epoch: incarnation(epoch, frame.steady),
                });
                if part.target.mesh != target.mesh {
                    part.epoch = incarnation(epoch, frame.steady);
                }
                part.target = target;
            }
            entity.shrinking = false;
            if frame.steady {
                entity.scale = 1.0;
            } else if !created || slot.0 == Kind::Ship {
                // Land creation is deferred until after 14127F000's growth
                // pass; formations are created within it and grow at once.
                entity.scale = (entity.scale + frame.delta * frame.gates.scale_up).min(1.0);
            }
        }
        self.shown = frame.shown;
        self.arrows = frame.arrows;
    }

    /// The phase update of a province out of view. The gate-loss loops
    /// destroy each kind shown last update, and a changed arrow flag forces
    /// the inactive pass, which destroys the rest. Otherwise `14127C910`
    /// returns early and a kind already shrinking keeps its entity.
    fn leave(&mut self, steady: bool) {
        let (shown, arrows) = (self.shown, self.arrows);
        self.entities
            .retain(|slot, _| !steady && !shown[slot.0 as usize] && !arrows);
        self.shown = [false; 2];
        self.arrows = false;
    }
}

/// Every province's unit entities, advanced once per viewer frame.
#[derive(Default)]
pub struct History {
    /// The hour's targets by province and slot.
    targets: BTreeMap<u32, BTreeMap<Slot, Parts>>,
    provinces: BTreeMap<u32, Province>,
    /// Province boxes for camera admission.
    bounds: Arc<BTreeMap<u32, [f32; 4]>>,
    /// The previous frame's animation clock.
    last: Option<f32>,
    /// `CGraphicalMap+0x130`.
    phase: u32,
    /// The last incarnation handed out.
    epoch: u32,
    /// Offline frames settle for their camera instead of animating.
    steady: bool,
}

impl History {
    /// Installs an hour's targets. Entities carry over; the next frame
    /// admits, grows or retires them.
    pub fn set(
        &mut self,
        targets: Vec<Target>,
        bounds: Arc<BTreeMap<u32, [f32; 4]>>,
        steady: bool,
    ) {
        if !Arc::ptr_eq(&self.bounds, &bounds) {
            // Native updates the show bits of empty provinces too: a stack
            // arriving in a province already shown starts at full scale.
            for &id in bounds.keys() {
                self.provinces.entry(id).or_default();
            }
            self.bounds = bounds;
        }
        self.targets.clear();
        for target in targets {
            self.provinces.entry(target.province).or_default();
            self.targets
                .entry(target.province)
                .or_default()
                .entry(target.slot())
                .or_default()
                .insert(target.part, target);
        }
        self.steady = steady;
    }

    /// Clears the unit slots of `provinces`, or of every province, as a load
    /// [`super::presentation::UnitRestart`]): their entities go and their
    /// show bits drop, so what shows next grows in on a new clock.
    pub fn reset(&mut self, provinces: Option<&BTreeSet<u32>>) {
        for (id, province) in &mut self.provinces {
            if provinces.is_none_or(|provinces| provinces.contains(id)) {
                *province = Province::default();
            }
        }
    }

    /// Advances one frame for `camera` and returns the models to draw.
    pub fn update(
        &mut self,
        camera: Camera,
        viewport: [u32; 2],
        time: f32,
        gates: Gates,
    ) -> Vec<(usize, ModelInstance)> {
        let in_view = province_view(camera, viewport);
        let bounds = Arc::clone(&self.bounds);
        self.step(
            time,
            gates,
            [gates.units(camera.height), gates.ships(camera.height)],
            camera.height < gates.arrows,
            |id| bounds.get(&id).is_some_and(|bounds| in_view(*bounds)),
        )
    }

    pub(super) fn step(
        &mut self,
        time: f32,
        gates: Gates,
        shown: [bool; 2],
        arrows: bool,
        in_view: impl Fn(u32) -> bool,
    ) -> Vec<(usize, ModelInstance)> {
        if self.last.is_some_and(|previous| time < previous) {
            self.reset(None);
        }
        let frame = Frame {
            delta: self.last.map_or(0.0, |previous| (time - previous).max(0.0)),
            gates,
            shown,
            arrows,
            steady: self.steady,
        };
        self.last = Some(time);
        // The native literal, advanced before the province updates; never
        // indexed by the replay hour.
        self.phase = (self.phase + 1) % 100;
        for (&id, province) in &mut self.provinces {
            if in_view(id) {
                province.visit(self.targets.get(&id), frame, &mut self.epoch);
            } else if self.steady || id % 100 == self.phase {
                province.leave(self.steady);
            }
        }
        self.provinces
            .values()
            .flat_map(|province| province.entities.values().flat_map(Entity::placed))
            .collect()
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub const GATES: Gates = Gates {
        units: 100.0,
        ships: 200.0,
        arrows: 300.0,
        transport_scale: 0.3,
        scale_up: 2.0,
        scale_down: 1.0,
        ship_scale_down: 0.5,
    };

    pub fn target(province: u32, kind: Kind) -> Target {
        Target {
            province,
            kind,
            identity: u64::from(province),
            part: 0,
            mesh: 0,
            instance: ModelInstance {
                scale: 1.0,
                animation_seed: province,
                ..Default::default()
            },
            origin: [0.0; 3],
        }
    }

    fn history(targets: Vec<Target>, steady: bool) -> History {
        let mut h = History::default();
        h.set(targets, Arc::default(), steady);
        h
    }

    fn near(h: &mut History, time: f32) -> Vec<(usize, ModelInstance)> {
        h.step(time, GATES, [true; 2], true, |_| true)
    }

    #[test]
    fn zoom_out_shrinks_twice_on_gate_loss_then_once_and_destroys_after_zero() {
        let mut h = history(vec![target(40, Kind::Land), target(40, Kind::Ship)], false);
        assert_eq!(near(&mut h, 0.0)[0].1.scale, 0.0);
        let full = near(&mut h, 1.0);
        assert!(full.iter().all(|(_, i)| i.scale == 1.0));
        let faded = h.step(1.25, GATES, [false; 2], true, |_| true);
        assert_eq!(faded[0].1.scale, 0.5);
        assert_eq!(faded[1].1.scale, 0.75);
        let zero = h.step(1.75, GATES, [false; 2], true, |_| true);
        assert_eq!(zero[0].1.scale, 0.0);
        assert_eq!(zero[1].1.scale, 0.5);
        let gone = h.step(2.0, GATES, [false; 2], true, |_| true);
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].1.scale, 0.375);
        let back = near(&mut h, 2.125);
        assert_eq!(
            back[0].1.scale, 0.0,
            "new land entity is created after the growth pass"
        );
        assert_ne!(back[0].1.animation_epoch, full[0].1.animation_epoch);
        assert_eq!(back[1].1.animation_epoch, full[1].1.animation_epoch);
        assert_eq!(back[1].1.scale, 0.625, "existing ship reverses its shrink");
    }

    #[test]
    fn pan_out_keeps_scale_and_entity_until_the_province_id_slot() {
        // Order and sparse IDs are immaterial; these are native ID array indexes.
        let mut h = history(vec![target(104, Kind::Land), target(99, Kind::Ship)], false);
        near(&mut h, 0.0); // phase 1
        let full = near(&mut h, 1.0); // phase 2
        let hidden = h.step(1.1, GATES, [true; 2], true, |_| false); // phase 3
        assert_eq!(hidden, full, "pan-out does not fade");
        let hidden = h.step(1.2, GATES, [true; 2], true, |_| false); // phase 4
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].1.animation_seed, 99);
        let back = near(&mut h, 1.3);
        assert_eq!(back[0].1.animation_epoch, full[0].1.animation_epoch);
        assert_ne!(back[1].1.animation_epoch, full[1].1.animation_epoch);
        assert_eq!(back[1].1.scale, 0.0);
    }

    #[test]
    fn quick_pan_back_reuses_but_after_a_scheduled_visit_recreates() {
        for kind in [Kind::Land, Kind::Ship] {
            let mut h = history(vec![target(50, kind)], false);
            near(&mut h, 0.0);
            let epoch = near(&mut h, 1.0)[0].1.animation_epoch;
            for _ in 0..20 {
                h.step(1.0, GATES, [true; 2], true, |_| false);
            }
            assert_eq!(near(&mut h, 1.0)[0].1.animation_epoch, epoch);
            for _ in 0..100 {
                h.step(1.0, GATES, [true; 2], true, |_| false);
            }
            assert!(h.provinces[&50].entities.is_empty());
            assert_ne!(near(&mut h, 1.0)[0].1.animation_epoch, epoch);
        }
    }

    #[test]
    fn hour_changes_wait_offscreen_and_resets_invalidate_even_the_same_target() {
        let old = target(70, Kind::Land);
        let mut h = history(vec![old], false);
        near(&mut h, 0.0);
        let full = near(&mut h, 1.0);
        let mut replacement = old;
        replacement.mesh = 2;
        h.set(vec![replacement], Arc::default(), false);
        assert_eq!(h.step(1.1, GATES, [true; 2], true, |_| false), full);
        let new = near(&mut h, 1.2);
        assert_eq!(new[0].0, 2);
        assert_eq!(
            new[0].1.scale, 1.0,
            "already shown province replaces at full scale"
        );
        assert_ne!(new[0].1.animation_epoch, full[0].1.animation_epoch);
        h.reset(None);
        let reset = near(&mut h, 1.2);
        assert_ne!(reset[0].1.animation_epoch, new[0].1.animation_epoch);
        assert_eq!(reset[0].1.scale, 0.0);
        assert_ne!(
            near(&mut h, 0.0)[0].1.animation_epoch,
            reset[0].1.animation_epoch
        );
        h.set(Vec::new(), Arc::default(), false);
        assert!(
            near(&mut h, 1.0).is_empty(),
            "removed visible stack destroys immediately"
        );
    }

    #[test]
    fn a_tag_switch_recreates_only_the_listed_provinces() {
        let mut h = history(vec![target(1, Kind::Land), target(2, Kind::Ship)], false);
        near(&mut h, 0.0);
        let full = near(&mut h, 1.0);
        h.reset(Some(&BTreeSet::from([1])));
        let after = near(&mut h, 1.1);
        assert_ne!(after[0].1.animation_epoch, full[0].1.animation_epoch);
        assert_eq!(after[0].1.scale, 0.0, "cleared show bit: grows in again");
        assert_eq!(after[1], full[1], "other provinces keep their entities");
    }

    #[test]
    fn ship_hull_offsets_shrink_about_the_formation_root() {
        let mut ship = target(30, Kind::Ship);
        ship.origin = [10.0, 20.0, 30.0];
        ship.instance.position = [14.0, 22.0, 38.0];
        ship.instance.scale = 0.6;
        let mut h = history(vec![ship], false);
        near(&mut h, 0.0);
        let half = near(&mut h, 0.25);
        assert_eq!(half[0].1.position, [12.0, 21.0, 34.0]);
        assert_eq!(half[0].1.scale, 0.3);
    }

    #[test]
    fn hulls_joining_a_growing_formation_take_its_root_scale() {
        let hull = |part, mesh| Target {
            part,
            mesh,
            ..target(30, Kind::Ship)
        };
        let mut h = history(vec![hull(0, 0)], false);
        near(&mut h, 0.0);
        let growing = near(&mut h, 0.25);
        assert_eq!(growing[0].1.scale, 0.5);
        h.set(vec![hull(0, 3), hull(1, 0)], Arc::default(), false);
        let joined = near(&mut h, 0.375);
        assert_eq!(joined.len(), 2);
        assert!(joined.iter().all(|(_, instance)| instance.scale == 0.75));
        assert_ne!(
            joined[0].1.animation_epoch, growing[0].1.animation_epoch,
            "a replaced hull is a new attachment"
        );
        assert_ne!(joined[1].1.animation_epoch, joined[0].1.animation_epoch);
    }

    #[test]
    fn an_empty_visible_province_keeps_its_show_bit_for_arriving_stacks() {
        let bounds = Arc::new(BTreeMap::from([(9, [0.0; 4])]));
        let mut h = History::default();
        h.set(Vec::new(), bounds.clone(), false);
        assert!(near(&mut h, 0.0).is_empty());
        h.set(vec![target(9, Kind::Land)], bounds, false);
        assert_eq!(near(&mut h, 0.1)[0].1.scale, 1.0);
    }

    #[test]
    fn already_shrinking_offscreen_slot_follows_the_native_early_return() {
        let mut h = history(vec![target(5, Kind::Land)], false);
        near(&mut h, 0.0);
        let epoch = near(&mut h, 1.0)[0].1.animation_epoch;
        h.step(1.1, GATES, [false; 2], false, |_| true); // also beyond arrow cutoff
        h.step(1.2, GATES, [false; 2], false, |_| false);
        let held = h.step(1.3, GATES, [false; 2], false, |_| false); // phase 5
        assert_eq!(held[0].1.animation_epoch, epoch);
        assert_eq!(near(&mut h, 1.4)[0].1.animation_epoch, epoch);
    }

    #[test]
    fn dumps_settle_for_their_camera_even_with_an_explicit_animation_time() {
        use super::super::camera::{MapDefines, MapSize, Placement};
        let mut h = history(vec![target(7, Kind::Land), target(8, Kind::Ship)], true);
        h.bounds = Arc::new(BTreeMap::from([
            (7, [0.0, 0.0, 2.0, 2.0]),
            (8, [3000.0, 1000.0, 3002.0, 1002.0]),
        ]));
        let camera = Placement {
            center: [0.0, 0.0],
            height: 50.0,
        }
        .on(MapSize::TEST, MapDefines::test());
        let full = h.update(camera, [1024, 768], 120.0, GATES);
        assert_eq!(full.len(), 1);
        assert_eq!(full[0].1.scale, 1.0);
        assert!(
            h.update(
                Camera {
                    height: 250.0,
                    ..camera
                },
                [1024, 768],
                120.0,
                GATES
            )
            .is_empty()
        );
    }

    #[test]
    fn popup_rates_come_from_evaluated_content_defines() {
        let defines = installed_content::defines::Defines::new().unwrap();
        defines.load("synthetic.lua", br#"NDefines = { NGraphics = {
            UNITS_DISTANCE_CUTOFF=11, SHIPS_DISTANCE_CUTOFF=22, UNIT_ARROW_DISTANCE_CUTOFF=33, SHIP_IN_PORT_SCALE=.125,
            MESH_POPUP_SCALE_UP_SPEED=7, MESH_POPUP_SCALE_DOWN_SPEED=9, SHIP_POPUP_SCALE_DOWN_SPEED=13
        }}"#).unwrap();
        let gates = Gates::read(&defines).unwrap();
        assert_eq!(
            (gates.scale_up, gates.scale_down, gates.ship_scale_down),
            (7.0, 9.0, 13.0)
        );
        defines
            .load(
                "override.lua",
                b"NDefines.NGraphics.MESH_POPUP_SCALE_DOWN_SPEED = 3 * 5",
            )
            .unwrap();
        assert_eq!(Gates::read(&defines).unwrap().scale_down, 15.0);
        defines
            .load(
                "missing.lua",
                b"NDefines.NGraphics.SHIP_POPUP_SCALE_DOWN_SPEED = nil",
            )
            .unwrap();
        assert!(
            Gates::read(&defines)
                .unwrap_err()
                .to_string()
                .contains("SHIP_POPUP_SCALE_DOWN_SPEED")
        );
    }

    #[test]
    #[ignore = "release viewer-frame lifecycle measurement"]
    fn unit_lifetime_cost() {
        for count in [3_000u32, 10_000, 50_000] {
            let targets: Vec<_> = (1..=count).map(|id| target(id, Kind::Land)).collect();
            let mut h = history(targets.clone(), false);
            near(&mut h, 0.0);
            near(&mut h, 1.0);
            let mut frames = Vec::new();
            let mut builders = Vec::new();
            for frame in 0..300 {
                if frame % 30 == 0 {
                    let start = std::time::Instant::now();
                    h.set(targets.clone(), Arc::default(), false);
                    builders.push(start.elapsed().as_secs_f64() * 1e3);
                }
                let start = std::time::Instant::now();
                std::hint::black_box(h.step(
                    2.0 + frame as f32 / 120.0,
                    GATES,
                    [frame % 120 < 60; 2],
                    true,
                    |id| (id + frame / 30) % 3 != 0,
                ));
                frames.push(start.elapsed().as_secs_f64() * 1e3);
            }
            frames.sort_by(f64::total_cmp);
            builders.sort_by(f64::total_cmp);
            println!(
                "UNIT_LIFETIME count={count} frame_p50_ms={:.4} frame_p95_ms={:.4} frame_max_ms={:.4} target_replace_p50_ms={:.4} target_replace_max_ms={:.4}",
                frames[150], frames[285], frames[299], builders[5], builders[9]
            );
        }
    }
}
