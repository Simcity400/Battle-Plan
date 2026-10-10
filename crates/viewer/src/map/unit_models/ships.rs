//! Ships at sea: each task force's formation of hull models, standing where its counter stands.
//!
//! The game attaches up to four hulls to the formation template's locators: slot 0 the
//! first roster ship when it is a capital, slots 1 to 3 the supports. Each hull
//! entity comes from the game's ladder over the variant's hull sprite, then the ship's own
//! fallback sprite. The game hides a docked formation (visual slot 19). The template and
//! its scale are the ones `gfx/naval_combat.txt` names, so a mod's formation draws.

use super::*;
mod combat;
mod combat_content;
mod selection;
use crate::map::{
    counters::SHIP_IN_PORT,
    task_force_visual::{MissionSkips, TaskForceMotions},
    unit_history::naval::{Candidate, Formation, Hull, Sources},
};

/// The file the formation template and its scale are named in.
pub const NAVAL_COMBAT_FILE: &str = "gfx/naval_combat.txt";

/// Literal capital attachment name.
const CAPITAL_NODE: &str = "root";

/// The formation every task force at sea draws with.
#[derive(Debug)]
pub(super) struct ShipAssets {
    /// The capital attaches to the literal "root".
    root: Option<mesh::Transform>,
    /// The template's hull locators, posed.
    nodes: Vec<mesh::Transform>,
    /// `naval_formation_scale`, applied to the template and its hulls.
    scale: f32,
    combat: Result<combat_content::CombatAssets, String>,
    /// The game reads the authored slot-zero yaw; an absent record stays zero.
    combat_rotations: BTreeMap<u32, f32>,
}

impl Default for ShipAssets {
    fn default() -> Self {
        Self {
            root: None,
            nodes: Vec::new(),
            scale: 0.0,
            combat: Err("naval combat assets were not loaded".into()),
            combat_rotations: BTreeMap::new(),
        }
    }
}

impl ShipAssets {
    /// Reads the formation `gfx/naval_combat.txt` names, with its native scale default.
    pub(super) fn load(
        layers: &ResolvedLayers,
        roster: &mesh::Roster,
        warnings: &mut Vec<installed_content::Diagnostic>,
    ) -> io::Result<Self> {
        let config =
            combat_content::Config::read(&layers.read_uncovered(NAVAL_COMBAT_FILE)?, warnings);
        let (formation, scale) = config.formation()?;
        if let Some(reason) = entity_refusal(roster, &formation) {
            return Err(invalid(reason));
        }
        let descriptors = &roster.descriptors;
        let (root, nodes) = formation_nodes(layers, descriptors, &formation)?;
        let combat = combat_content::CombatAssets::load(descriptors, &config)
            .map_err(|error| error.to_string());
        let combat_rotations = installed_content::positions::read_unit_stacks(layers)?
            .into_iter()
            .filter(|stack| stack.kind == 0)
            .map(|stack| (stack.province, stack.rotation))
            .collect();
        Ok(Self {
            root,
            nodes,
            scale,
            combat,
            combat_rotations,
        })
    }
}

/// Parse combat-template content with empty names and initial scale 1.
/// Strings and floats overwrite in reader order; effect lists append.
/// Unknown fields are reported and skipped. Combat-only
/// fields still consume their native forms, so their tokens cannot become formation fields.
#[cfg(test)]
fn read_formation(
    bytes: &[u8],
    warnings: &mut Vec<installed_content::Diagnostic>,
) -> io::Result<(String, f32)> {
    combat_content::Config::read(bytes, warnings).formation()
}

/// The formation template's `l_position_*` and `r_position_*` locators, posed by the template's
/// animation when it has one.
fn formation_nodes(
    layers: &ResolvedLayers,
    descriptors: &mesh::Descriptors,
    formation: &str,
) -> io::Result<(Option<mesh::Transform>, Vec<mesh::Transform>)> {
    let entity = descriptors
        .resolve_entity(formation)
        .ok_or_else(|| invalid(format!("missing naval formation entity {formation}")))?;
    let bytes = layers.read_uncovered(&entity.definition.file)?;
    let posed = mesh::read(&bytes)?;
    let tree = mesh::parse(&bytes)?;
    let animation = entity
        .animation
        .and_then(|animation| layers.read_uncovered(&animation.file).ok())
        .and_then(|bytes| mesh::read_animation(&bytes).ok());
    // These are locator children of the animated root, not skeleton joints.
    let locators: Vec<_> = tree
        .child("locator")
        .ok_or_else(|| invalid("ship formation has no locators"))?
        .children
        .iter()
        .filter(|j| {
            j.name == CAPITAL_NODE
                || j.name.starts_with("l_position_")
                || j.name.starts_with("r_position_")
        })
        .map(|node| {
            let position: [f32; 3] = node
                .property("p")
                .and_then(mesh::Value::floats)
                .and_then(|v| v.try_into().ok())
                .ok_or_else(|| invalid("ship locator has no position"))?;
            let rotation: [f32; 4] = node
                .property("q")
                .and_then(mesh::Value::floats)
                .and_then(|v| v.try_into().ok())
                .ok_or_else(|| invalid("ship locator has no rotation"))?;
            let parent = node
                .property("pa")
                .and_then(mesh::Value::strings)
                .and_then(|v| v.first())
                .ok_or_else(|| invalid("ship locator has no parent joint"))?;
            let parent = animation
                .as_ref()
                .and_then(|a| posed.pose(a, mesh::FIRST_FRAME, parent))
                .or_else(|| posed.bind_pose(parent))
                .ok_or_else(|| invalid("ship locator parent has no transform"))?;
            Ok((
                node.name.as_str(),
                parent.compose(&mesh::Transform::from_trs(position, rotation, 1.0)),
            ))
        })
        .collect::<io::Result<_>>()?;
    // Child attachment also resolves skeleton joints. The ordinary template's capital point is a joint, not a locator.
    let root = locators
        .iter()
        .find(|(name, _)| *name == CAPITAL_NODE)
        .map(|(_, transform)| *transform)
        .or_else(|| capital_joint(&posed, animation.as_ref()));
    let nodes: Vec<_> = locators
        .into_iter()
        .filter(|(name, _)| *name != CAPITAL_NODE)
        .map(|(_, transform)| transform)
        .collect();
    if nodes.is_empty() {
        return Err(invalid("ship formation has no support positions"));
    }
    Ok((root, nodes))
}

/// Slot 0 attaches to the literal "root".
fn capital_joint(
    posed: &mesh::Mesh,
    animation: Option<&mesh::Animation>,
) -> Option<mesh::Transform> {
    animation
        .and_then(|animation| posed.pose(animation, mesh::FIRST_FRAME, CAPITAL_NODE))
        .or_else(|| posed.bind_pose(CAPITAL_NODE))
}

/// The game's ladder for one sprite: the country's numbered model, the country's, the
/// numbered bare one and the bare one.
fn ship_candidates(tag: &str, sprite: &str, number: i32) -> [String; 4] {
    [
        format!("{tag}_{sprite}_{number}_entity"),
        format!("{tag}_{sprite}_entity"),
        format!("{sprite}_{number}_entity"),
        format!("{sprite}_entity"),
    ]
}

/// What places a task force's formation: its motion rows and the content its route runs over.
pub struct ShipPlaces<'a> {
    pub motions: &'a TaskForceMotions,
    pub movement: Option<&'a crate::map::division_eta::MovementContent>,
    /// Whether a province is land, which picks a task force's slot 19.
    pub land: &'a dyn Fn(u32) -> bool,
    /// The game's mission skip, shared with the counter pass: a skipped task force has no
    /// representative, so the game removes its formation.
    pub missions: &'a MissionSkips,
    /// A province's strategic region, which a patrol's skip compares with its current region.
    pub region: &'a dyn Fn(u32) -> Option<u32>,
    /// Authored standing-slot scale, initialized to 1.
    pub scale: &'a dyn Fn(u32, u32) -> f32,
}

impl UnitModelAssets {
    /// The game selects the first registered definition, then creates its
    /// actor. A failed bake of that winner cannot choose a later candidate.
    fn ship_slot<'t>(
        &self,
        tag: impl FnOnce() -> Result<&'t str, String>,
        row: &replay::rows::ShipModelRow,
        graphics: &[u8],
        definition: Option<(u32, u32)>,
        hour: Option<&Hour<'_>>,
    ) -> Result<Option<(usize, String)>, String> {
        let Some(name) = selection::name(self, tag, row, graphics, definition, hour)? else {
            return Ok(None);
        };
        let slot = self.bake_slot(&name).ok_or_else(|| {
            format!("naval_entity: selected registered entity {name} cannot be baked")
        })?;
        Ok(Some((slot, name)))
    }

    /// Native representative candidates and roster inputs. The game creates
    /// only the selected task force's root; unit_history advances its retained
    /// attachments. Cold standing heading is -pi/2, plus the mesh
    /// convention's quarter turn; moving roots take their sampled heading.
    // Keep the frame, vision, route and placement inputs explicit at the call site.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_naval(
        &self,
        frame: &DecodedFrame,
        vision: &Vision,
        anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
        world: Option<&counter_move::RouteWorld<'_>>,
        places: &ShipPlaces<'_>,
        hour: Option<&Hour<'_>>,
        report: &mut ModelReport,
    ) -> io::Result<Sources> {
        let models = rows::<tables::ShipModels>(frame)?;
        let ships = rows::<tables::Ships>(frame)?;
        let forces = rows::<tables::TaskForces>(frame)?;
        if !models.is_complete() || !ships.is_complete() || !forces.is_complete() {
            return Ok(Sources::default());
        }
        let order = crate::map::counter_order::RecordedOrder::read(frame)?;
        let variants = rows::<tables::VariantDefinitions>(frame)?;
        let variant_names: BTreeMap<_, _> = variants
            .rows()
            .iter()
            .map(|r| (r.id, &r.explicit_entity))
            .collect();
        let clock = rows::<tables::Clock>(frame)?;
        let clock = clock.rows().first().filter(|_| clock.is_complete());
        let models: BTreeMap<_, _> = models.rows().iter().map(|r| (r.ship, r)).collect();
        let links: BTreeMap<_, _> = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|r| (r.id, r.canonical_slot))
            .collect();
        let countries: BTreeMap<_, _> = core::<tables::Countries>(frame)?
            .iter()
            .map(|r| (r.canonical_slot, r))
            .collect();
        let mut roster: BTreeMap<_, Vec<_>> = BTreeMap::new();
        let classes: BTreeMap<_, _> = ships
            .rows()
            .iter()
            .map(|ship| (ship.id, ship.class_id as u32))
            .collect();
        for ship in ships.rows() {
            if let Some(model) = models.get(&ship.id) {
                roster.entry(ship.task_force).or_default().push(*model);
            }
        }
        let mut placed = Sources::default();
        for force in forces.rows() {
            if force.province == 0 {
                continue;
            }
            let placement =
                match places
                    .motions
                    .visual(force, places.movement, world, anchor, places.land)
                {
                    Ok(placement) => placement,
                    Err(reason) => {
                        report.error(format!("naval_formation_motion: {reason}"));
                        continue;
                    }
                };
            if !self.bounds.contains_key(&force.province) {
                report.error(format!(
                    "province {} has no content bounds for model admission",
                    force.province
                ));
                continue;
            }
            let Some(&owner) = links.get(&force.logical_owner) else {
                continue;
            };
            if vision.sees_navy(force.id, owner) == Sight::None {
                continue;
            }
            if placement.kind == SHIP_IN_PORT {
                continue;
            }
            // A task force on its mission here is left to the mission icon: no counter and no
            // formation. Unrecorded skip operands leave the formation undrawn and counted.
            match places
                .missions
                .skip(force, placement.kind, (places.region)(force.province))
            {
                Some(false) => {}
                Some(true) => continue,
                None => {
                    report.undecided_mission_formations += 1;
                    continue;
                }
            }
            if let Some(reason) = placement.unplaced_route {
                report.error(format!("naval_animator_route: {reason}"));
                continue;
            }
            let Some(ordinal) = order
                .as_ref()
                .and_then(|order| order.mixed(force.id, force.province))
            else {
                report.error("naval_representative_order: no complete ProvinceUnitOrder entry");
                continue;
            };
            let Some(&roster_owner) = links.get(&force.roster_owner) else {
                report.error("naval_representative_owner: roster owner has no CountryLinks row");
                continue;
            };
            // The frame history applies viewer selection to this base priority.
            let row = places.motions.row(force.id).map_err(invalid)?;
            let route = counter_move::UnitRoute::task_force(row);
            let path = route.display_path();
            let Some((ours, priority)) = representative_priority(vision, roster_owner, row.target)
            else {
                report.error(
                    "naval_representative_relations: country sharing/war operands are unavailable",
                );
                continue;
            };
            let Some(min_arc) = animator_min_arc(path, clock, report) else {
                continue;
            };
            placed.candidates.push(Candidate {
                province: force.province,
                identity: force.id,
                ordinal,
                slot: placement.kind,
                priority,
                ours,
                class: route.class,
                land_eligible: false,
                sample_count: placement.animator.map_or(0, |sample| sample.sample_count),
                next_arc: placement.animator.map_or(0.0, |sample| sample.next_arc),
                route: path.to_vec(),
                speed: self.gates.naval_movement_speed,
                min_arc,
                arrival_allowed: clock.is_some_and(|clock| clock.speed < 4)
                    && route.combat_count == 0,
                // Convert the current hour to its zero-based day.
                day: (frame.hour - 43_800_000) / 24,
            });
            // Native slots 9/10 still offer a representative, but create the
            // province's two-sided combat actor rather than an ordinary root.
            if matches!(placement.kind, 9 | 10) {
                continue;
            }
            let position = placement.position;
            let Some(roster) = roster.get(&force.id) else {
                continue;
            };
            let (root, turn) = match &position.motion {
                Some(motion) => {
                    let (at, heading) = motion.position_heading();
                    (at, heading + MESH_FORWARD_OFFSET)
                }
                None => (position.standing, 0.0),
            };
            // Reuse visual's viewer-owned route sample
            // for scale as well as admission and position; never sample twice.
            let standing_scale = |province| {
                (places.scale)(
                    province,
                    counter_move::CounterClass::TaskForce.standstill((places.land)(province)),
                )
            };
            let from_scale = standing_scale(force.province);
            let to_scale = path.first().map_or(1.0, |&next| standing_scale(next));
            let animator_scale = placement.animator.map_or(from_scale, |sample| {
                sample.interpolate_scale(from_scale, to_scale)
            });
            if roster.iter().any(|model| model.counter.is_none()) {
                report.error("naval_formation_roster: ShipModels lacks roster ordinals");
                continue;
            }
            let mut ordered = roster.clone();
            ordered.sort_by_key(|model| model.counter.as_ref().unwrap().roster_ordinal);
            let mut hulls = Vec::new();
            for model in &ordered {
                // Use the selected variant's explicit entity name, or an empty
                // string for an invalid reference. That exact string controls reuse.
                let graphics = if model.variant == 0 {
                    Vec::new()
                } else if let Some(name) = variant_names.get(&model.variant) {
                    (*name).clone()
                } else {
                    report.error("naval_formation_graphics: selected variant has no recorded explicit entity");
                    continue;
                };
                let model_slot = match self.ship_slot(
                    || {
                        // The game reads this country only after the primary
                        // model missed. The logical owner remains the visibility key above.
                        let owner =
                            model_owner(force.roster_owner, force.expeditionary_owner, true);
                        let country = links
                            .get(&owner)
                            .and_then(|slot| countries.get(slot))
                            .ok_or(
                                "naval_formation_model_country: native model owner is unavailable",
                            )?;
                        installed_content::parser::text(installed_content::parser::c_string(&country.tag))
                            .map_err(|error| format!("naval_formation_model_country: {error}"))
                    },
                    model,
                    &graphics,
                    classes
                        .get(&model.ship)
                        .map(|&class| (force.roster_owner, class)),
                    hour,
                ) {
                    Ok(Some(slot)) => Some(slot),
                    Ok(None) => {
                        report.error("naval_formation_entity: no hull candidate or placeholder is registered");
                        None
                    }
                    Err(reason) => {
                        report.error(&reason);
                        None
                    }
                };
                let targets = std::array::from_fn(|index| {
                    let (slot, _) = model_slot.as_ref()?;
                    let node = if index == 0 {
                        self.ships.root.as_ref()?
                    } else {
                        self.ships.nodes.get(
                            ((force.id >> 32) as usize + index) % self.ships.nodes.len().max(1),
                        )?
                    };
                    // Animator scale multiplies both
                    // locator offsets and hull size, before the shared popup scale.
                    let scale = self.ships.scale * animator_scale;
                    let [x, y, z] = node.translation.map(|v| v * scale);
                    let (sin, cos) = turn.sin_cos();
                    Some(Target {
                        province: force.province,
                        identity: force.id,
                        order: placement.kind,
                        occupant: model.ship,
                        part: index as u32,
                        kind: Kind::Ship,
                        mesh: *slot,
                        origin: root,
                        instance: ModelInstance {
                            position: [
                                root[0] + x * cos - z * sin,
                                root[1] + y,
                                root[2] + x * sin + z * cos,
                            ],
                            rotation: turn + node.basis[2][0].atan2(node.basis[2][2]),
                            scale: scale * node.uniform_scale(),
                            pitch: 0.0,
                            train_clip: [0.0; 4],
                            // A hull plays its entity's idle; the task force's handle keeps one
                            // formation on one phase across frames.
                            animation: crate::map::entity_animation::IDLE,
                            animation_seed: force.id as u32 ^ (force.id >> 32) as u32,
                            ..ModelInstance::default()
                        },
                    })
                });
                hulls.push(Hull {
                    flags: model.formation_flags,
                    graphics,
                    targets,
                });
                if let Some((_, name)) = model_slot {
                    *report.by_entity.entry(name).or_default() += 1;
                }
            }
            if self.ships.root.is_none()
                && ordered.first().is_some_and(|model| {
                    model.formation_flags & crate::map::counter_order::FORMATION_CAPITAL_MASK != 0
                })
            {
                report.error(
                    "naval_formation_capital_root: authored formation has no root attach point",
                );
            }
            let slots = formation_slots(&ordered);
            report.ship_hulls += slots
                .iter()
                .filter(|(index, _)| *index != 0 || self.ships.root.is_some())
                .count();
            if slots.iter().any(|(index, _)| *index != 0) {
                report.support_locator_rng += 1;
            }
            placed.formations.push(Formation {
                identity: force.id,
                province: force.province,
                slot: placement.kind,
                roster: hulls,
            });
        }
        self.add_land_competitors(
            frame,
            vision,
            anchor,
            world,
            places,
            &links,
            clock,
            &mut placed,
            report,
        )?;
        placed.combats = self.prepare_naval_combats(frame, anchor, &placed, hour, report)?;
        Ok(placed)
    }

    /// Land units and railway guns share the game's representative array
    /// with task forces, subject to their terrain and camera admission.
    #[allow(clippy::too_many_arguments)]
    fn add_land_competitors(
        &self,
        frame: &DecodedFrame,
        vision: &Vision,
        anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
        world: Option<&counter_move::RouteWorld<'_>>,
        places: &ShipPlaces<'_>,
        links: &BTreeMap<u32, u32>,
        clock: Option<&replay::rows::ClockRow>,
        sources: &mut Sources,
        report: &mut ModelReport,
    ) -> io::Result<()> {
        let provinces: BTreeSet<_> = sources
            .candidates
            .iter()
            .map(|candidate| candidate.province)
            .collect();
        let orders = rows::<tables::ProvinceUnitOrder>(frame)?;
        let divisions = rows::<tables::Divisions>(frame)?;
        let division_index: BTreeMap<_, _> =
            divisions.rows().iter().map(|row| (row.id, row)).collect();
        let movement = rows::<tables::DivisionMovement>(frame)?;
        let movements: BTreeMap<_, _> = movement
            .rows()
            .iter()
            .map(|row| (row.division, row))
            .collect();
        let guns = rows::<tables::RailwayGunCounterInputs>(frame)?;
        let gun_index: BTreeMap<_, _> = guns.rows().iter().map(|row| (row.gun, row)).collect();
        let known_forces: BTreeSet<_> = rows::<tables::TaskForces>(frame)?
            .rows()
            .iter()
            .map(|force| force.id)
            .collect();
        let mut refused = BTreeSet::new();
        for order in orders
            .rows()
            .iter()
            .filter(|order| provinces.contains(&order.province))
        {
            for (ordinal, &id) in order.mixed.iter().enumerate() {
                if known_forces.contains(&id) {
                    continue;
                }
                let inputs = if let Some(division) = division_index.get(&id) {
                    movements.get(&id).map(|row| {
                        (
                            division.logical_owner,
                            division.roster_owner,
                            counter_move::UnitRoute::division(row),
                        )
                    })
                } else {
                    gun_index.get(&id).map(|gun| {
                        (
                            gun.logical_owner,
                            gun.roster_owner,
                            counter_move::UnitRoute::railway_gun(gun),
                        )
                    })
                };
                let Some((logical, roster, route)) = inputs else {
                    report.error("naval_representative_competitor: mixed occupant lacks DivisionMovement or RailwayGunCounterInputs");
                    refused.insert(order.province);
                    continue;
                };
                let Some(&logical) = links.get(&logical) else {
                    report.error("naval_representative_competitor_owner: missing CountryLinks row");
                    refused.insert(order.province);
                    continue;
                };
                if vision.sees_land(id, logical, order.province) == Sight::None {
                    continue;
                }
                let Some(&owner) = links.get(&roster) else {
                    report.error(
                        "naval_representative_competitor_owner: missing roster CountryLinks row",
                    );
                    refused.insert(order.province);
                    continue;
                };
                let path = route.display_path();
                let sample =
                    if path.is_empty() && route.path_total == 0 && route.alternate_total == 0 {
                        Some(counter_move::RouteSample::default())
                    } else {
                        world.and_then(|world| {
                            counter_move::Motion::route_sample(
                                order.province,
                                path,
                                &|id| anchor(id, route.class.standstill((places.land)(id))),
                                world,
                            )
                        })
                    };
                let Some(sample) = sample else {
                    report.error(
                        "naval_representative_competitor_route: route geometry is unavailable",
                    );
                    refused.insert(order.province);
                    continue;
                };
                let kind = if let Some(world) = world {
                    route.stack_kind_from_animator(order.province, world.map, sample.sample_count)
                } else if path.is_empty() {
                    Some(counter_move::stack_kind(counter_move::StackKindInputs {
                        class: route.class,
                        combat_role: route.combat_role,
                        combat_count: route.combat_count,
                        ordinary_path_total: route.path_total,
                        movement_target: route.target,
                        naval_transfer: route.naval_transfer,
                        terrain_flags: u8::from((places.land)(order.province)),
                        ..Default::default()
                    }))
                } else {
                    None
                };
                let Some(slot) = kind else {
                    report.error("naval_representative_competitor_route: route or terrain operands are incomplete");
                    refused.insert(order.province);
                    continue;
                };
                let Some(flags) = world
                    .and_then(|world| world.map.terrain(order.province))
                    .map(|terrain| terrain.flags)
                else {
                    report.error("naval_representative_competitor_terrain: native province definition flags are unavailable");
                    refused.insert(order.province);
                    continue;
                };
                // The game excludes land occupants that fail this
                // predicate; eligible occupants also need the land camera gate.
                let land_eligible =
                    (flags & 3 != 0 || (11..19).contains(&slot) || (30..38).contains(&slot))
                        && (!route.naval_transfer || flags & 1 == 0);
                let Some((ours, priority)) = representative_priority(vision, owner, route.target)
                else {
                    report.error("naval_representative_relations: country sharing/war operands are unavailable");
                    refused.insert(order.province);
                    continue;
                };
                let Some(min_arc) = animator_min_arc(path, clock, report) else {
                    refused.insert(order.province);
                    continue;
                };
                sources.candidates.push(Candidate {
                    province: order.province,
                    identity: id,
                    ordinal: ordinal as u32,
                    slot,
                    priority,
                    ours,
                    class: route.class,
                    land_eligible,
                    sample_count: sample.sample_count,
                    next_arc: sample.next_arc,
                    route: path.to_vec(),
                    speed: if route.combat_count != 0 {
                        0.0
                    } else {
                        self.gates.land_movement_speed
                    },
                    min_arc,
                    arrival_allowed: clock.is_some_and(|clock| clock.speed < 4)
                        && route.combat_count == 0,
                    day: (frame.hour - 43_800_000) / 24,
                });
            }
        }
        sources
            .candidates
            .retain(|candidate| !refused.contains(&candidate.province));
        sources
            .formations
            .retain(|formation| !refused.contains(&formation.province));
        Ok(())
    }

    #[cfg(test)]
    fn prepare_ships(
        &self,
        frame: &DecodedFrame,
        vision: &Vision,
        anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
        world: Option<&counter_move::RouteWorld<'_>>,
        places: &ShipPlaces<'_>,
        report: &mut ModelReport,
    ) -> io::Result<Vec<Target>> {
        let sources = self.prepare_naval(frame, vision, anchor, world, places, None, report)?;
        Ok(sources.settled_targets(self.gates))
    }
}

/// Nonempty routes clamp the arc to one at speed index >=4.
/// An empty route initializes at zero without reading the game clock.
fn animator_min_arc(
    path: &[u32],
    clock: Option<&replay::rows::ClockRow>,
    report: &mut ModelReport,
) -> Option<f32> {
    if path.is_empty() {
        return Some(0.0);
    }
    let Some(clock) = clock else {
        report.error("naval_animator_clock: Clock speed is unavailable");
        return None;
    };
    Some(if clock.speed >= 4 {
        counter_move::SETTLED_ARC
    } else {
        0.0
    })
}

/// Canonical roster owner and unselected base priority; foreign sharing
/// uses directed sharing and at-war relations, not a friendliness test.
fn representative_priority(vision: &Vision, owner: u32, target: u32) -> Option<(bool, u8)> {
    if owner == vision.ours() {
        return Some((true, 3 - u8::from(target != 0)));
    }
    Some((
        false,
        if vision.shares_vision_proven(owner).ok()? {
            2
        } else if vision.at_war_proven(owner).ok()? {
            1
        } else {
            0
        },
    ))
}

/// A formation with no retained slots: each hull with the slot index its
/// locator is taken from.
///
/// The recording stores each ship's roster order (`ShipCounterInputs.roster_ordinal`)
/// and complete flags word. Slot 0 uses the first roster ship when its flags
/// have bit 6 or 22 (`0x400040`), and slots 1 to 3 take the
/// supports. Live attachments retain their handles/graphics in unit_history::naval;
/// only support locators use client-specific RNG.
fn formation_slots<'a>(
    roster: &[&'a replay::rows::ShipModelRow],
) -> Vec<(usize, &'a replay::rows::ShipModelRow)> {
    let recorded = !roster.is_empty() && roster.iter().all(|row| row.counter.is_some());
    if !recorded {
        return Vec::new();
    }
    let mut ordered = roster.to_vec();
    ordered.sort_by_key(|row| row.counter.as_ref().map_or(0, |c| c.roster_ordinal));
    let mut slots = Vec::new();
    if let Some(first) = ordered.first()
        && first.formation_flags & crate::map::counter_order::FORMATION_CAPITAL_MASK != 0
    {
        slots.push((0, *first));
    }
    slots.extend(
        support_ships(&ordered)
            .into_iter()
            .enumerate()
            .map(|(index, row)| (index + 1, row)),
    );
    slots
}

fn support_ships<'a>(
    roster: &[&'a replay::rows::ShipModelRow],
) -> Vec<&'a replay::rows::ShipModelRow> {
    let mut selected = Vec::new();
    for mask in [1u64 << 8, (1u64 << 7) | (1u64 << 39)] {
        for &row in roster {
            if selected.len() == 3 {
                return selected;
            }
            if row.formation_flags & mask != 0
                && !selected
                    .iter()
                    .any(|r: &&replay::rows::ShipModelRow| r.ship == row.ship)
            {
                selected.push(row);
            }
        }
    }
    selected
}

#[cfg(test)]
mod tests;
