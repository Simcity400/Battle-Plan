
use super::*;
use viewer_foundation::movement_timing::MovementContent;
mod combat;
mod combat_content;
mod selection;
use viewer_unit_history::unit_history::naval::{Candidate, Formation, FormationPlacement, Hull, Sources};
use viewer_unit_reads::task_force_visual::{MissionSkips, TaskForceMotions};
use viewer_foundation::unit_rules::SHIP_IN_PORT;

pub const NAVAL_COMBAT_FILE: &str = "gfx/naval_combat.txt";

const CAPITAL_NODE: &str = "root";

#[derive(Debug)]
pub(super) struct ShipAssets {
    root: Option<mesh::Transform>,
    nodes: Vec<mesh::Transform>,
    scale: f32,
    combat: Result<combat_content::CombatAssets, String>,
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
    #[cfg(test)]
    pub(super) fn test_formation() -> Self {
        Self {
            root: Some(mesh::Transform::from_trs([0.; 3], [0., 0., 0., 1.], 1.)),
            nodes: vec![mesh::Transform::from_trs([1., 0., 2.], [0., 0., 0., 1.], 1.)],
            scale: 1.,
            ..Self::default()
        }
    }

    pub(super) fn load(
        layers: &ResolvedLayers,
        roster: &mesh::Roster,
        warnings: &mut Vec<content_reader_parser::Diagnostic>,
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
        let combat_rotations = content_reader_map::positions::read_unit_stacks(layers)?
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

#[cfg(test)]
fn read_formation(
    bytes: &[u8],
    warnings: &mut Vec<content_reader_parser::Diagnostic>,
) -> io::Result<(String, f32)> {
    combat_content::Config::read(bytes, warnings).formation()
}

fn formation_nodes(
    layers: &ResolvedLayers,
    descriptors: &mesh::Descriptors,
    formation: &str,
) -> io::Result<(Option<mesh::Transform>, Vec<mesh::Transform>)> {
    let entity = descriptors
        .resolve_entity(formation)
        .ok_or_else(|| invalid(format!("missing naval formation entity {formation}")))?;
    let tree = mesh::parse_in(layers, &entity.definition.file)?;
    let posed = mesh::read_tree(&tree)?;
    let animation = entity.animation.and_then(|animation| {
        let read = |stack: &ResolvedLayers| {
            mesh::read_animation_in(stack, &animation.file)
        };
        match read(layers) {
            Ok(posed) => Some(posed),
            Err(why) => match layers.without_overlays().map(|recorded| read(&recorded)) {
                Some(Ok(posed)) => {
                    layers.note_overlay_refusal(format!(
                        "naval formation animation {} ({why})",
                        animation.file
                    ));
                    Some(posed)
                }
                _ => {
                    layers.note_refused(format!("model animation {}", animation.file));
                    None
                }
            },
        }
    });
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
    let root = locators
        .iter()
        .find(|(name, _)| *name == CAPITAL_NODE)
        .map(|(_, transform)| *transform)
        .or_else(|| capital_joint(&posed, animation.as_deref()));
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

fn capital_joint(
    posed: &mesh::Mesh,
    animation: Option<&mesh::Animation>,
) -> Option<mesh::Transform> {
    animation
        .and_then(|animation| posed.pose(animation, mesh::FIRST_FRAME, CAPITAL_NODE))
        .or_else(|| posed.bind_pose(CAPITAL_NODE))
}

fn ship_candidates(tag: &str, sprite: &str, number: i32) -> [String; 4] {
    [
        format!("{tag}_{sprite}_{number}_entity"),
        format!("{tag}_{sprite}_entity"),
        format!("{sprite}_{number}_entity"),
        format!("{sprite}_entity"),
    ]
}

pub struct ShipPlaces<'a> {
    pub motions: &'a TaskForceMotions,
    pub movement: Option<&'a MovementContent>,
    pub land: &'a dyn Fn(u32) -> bool,
    pub missions: &'a MissionSkips,
    pub region: &'a dyn Fn(u32) -> Option<u32>,
    pub scale: &'a dyn Fn(u32, u32) -> f32,
}

impl UnitModelAssets {
    fn ship_slot<'t>(
        &self,
        tag: impl FnOnce() -> Result<&'t str, String>,
        row: &replay_data::rows::ShipModelRow,
        graphics: &[u8],
        definition: Option<(u32, u32)>,
        hour: Option<&Hour<'_>>,
    ) -> Result<Option<(usize, String)>, String> {
        let selected = selection::name(self, tag, row, graphics, definition, hour)?;
        self.ship_slot_selected(selected, hour)
    }

    fn ship_slot_selected(
        &self,
        selected: Option<String>,
        hour: Option<&Hour<'_>>,
    ) -> Result<Option<(usize, String)>, String> {
        let Some(name) = selected else {
            return Ok(None);
        };
        let slot = self.bake_slot_for_hour(&name, hour).ok_or_else(|| {
            format!("naval_entity: selected registered entity {name} cannot be baked")
        })?;
        Ok(Some((slot, name)))
    }

    #[cfg(test)]
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
        self.prepare_naval_with_spans(
            frame,
            vision,
            anchor,
            world,
            places,
            hour,
            report,
            &mut BuilderSpans::new(false),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare_naval_with_spans(
        &self,
        frame: &DecodedFrame,
        vision: &Vision,
        anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
        world: Option<&counter_move::RouteWorld<'_>>,
        places: &ShipPlaces<'_>,
        hour: Option<&Hour<'_>>,
        report: &mut ModelReport,
        spans: &mut BuilderSpans,
    ) -> io::Result<Sources> {
        let mut names = selection::Pass::new(self, frame, hour);
        self.prepare_naval_with_names(
            frame, vision, anchor, world, places, hour, report, spans, &mut names,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn prepare_naval_with_names<'rows>(
        &self,
        frame: &'rows DecodedFrame,
        vision: &Vision,
        anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
        world: Option<&counter_move::RouteWorld<'_>>,
        places: &ShipPlaces<'_>,
        hour: Option<&Hour<'_>>,
        report: &mut ModelReport,
        spans: &mut BuilderSpans,
        names: &mut impl selection::Names<'rows>,
    ) -> io::Result<Sources> {
        let inputs = spans.measure("units.naval.inputs", |_| -> io::Result<_> {
            let models = rows::<tables::ShipModels>(frame)?;
            let ships = rows::<tables::Ships>(frame)?;
            let forces = rows::<tables::TaskForces>(frame)?;
            if !models.is_complete() || !ships.is_complete() || !forces.is_complete() {
                return Ok(None);
            }
            let order = viewer_unit_reads::counter_order::RecordedOrder::read(frame)?;
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
            Ok(Some((
                forces,
                order,
                variant_names,
                clock,
                links,
                countries,
                roster,
                classes,
            )))
        })?;
        let Some((forces, order, variant_names, clock, links, countries, roster, classes)) = inputs
        else {
            return Ok(Sources::default());
        };
        let mut placed = Sources::default();
        spans.measure("units.naval.formations", |_| -> io::Result<()> {
            let ground = ShipGround { movement: places.movement, world, anchor, land: places.land, scale: places.scale };
            let link = |raw: u32| links.get(&raw).copied();
            let day = (frame.hour - 43_800_000) / 24;
            for force in forces.rows() {
                if force.province == 0 {
                    continue;
                }
                let values = Force { id: force.id, province: force.province, logical_owner: force.logical_owner, roster_owner: force.roster_owner, expeditionary_owner: force.expeditionary_owner };
                let route = places.motions.row(force.id).map(counter_move::UnitRoute::task_force);
                let skip = |kind| places.missions.skip(force, kind, (places.region)(force.province));
                let ordinal = order.as_ref().and_then(|order| order.mixed(force.id, force.province));
                let tag_of = |owner: u32| -> Result<String, String> {
                    let country = links.get(&owner).and_then(|slot| countries.get(slot)).ok_or("naval_formation_model_country: native model owner is unavailable")?;
                    content_reader_parser::parser::text(content_reader_parser::parser::c_string(&country.tag)).map(str::to_owned).map_err(|error| format!("naval_formation_model_country: {error}"))
                };
                let part = self.naval_force(&values, route, &ground, &link, vision, &skip, ordinal, clock.map(|clock| clock.speed), day, || {
                    let models = roster.get(&force.id)?;
                    Some(self.resolve_roster(&values, models, &|variant| variant_names.get(&variant).copied(), &|ship| classes.get(&ship).copied(), &tag_of, names, hour))
                });
                part.into(&mut placed, report);
            }
            Ok(())
        })?;
        spans.measure("units.naval.competitors", |_| {
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
            )
        })?;
        placed.combats = spans.measure("units.naval.combats", |_| {
            self.prepare_naval_combats(frame, anchor, &placed, hour, report)
        })?;
        Ok(placed)
    }

    #[allow(clippy::too_many_arguments)]
    fn add_land_competitors(
        &self,
        frame: &DecodedFrame,
        vision: &Vision,
        anchor: &dyn Fn(u32, u32) -> Option<[f32; 3]>,
        world: Option<&counter_move::RouteWorld<'_>>,
        places: &ShipPlaces<'_>,
        links: &BTreeMap<u32, u32>,
        clock: Option<&replay_data::rows::ClockRow>,
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
                let land_eligible =
                    (flags & 3 != 0 || (11..19).contains(&slot) || (30..38).contains(&slot))
                        && (!route.naval_transfer || flags & 1 == 0);
                let Some((ours, priority)) = representative_priority(vision, owner, route.target)
                else {
                    report.error("naval_representative_relations: country sharing/war operands are unavailable");
                    refused.insert(order.province);
                    continue;
                };
                let Some(min_arc) = animator_min_arc(path, clock.map(|clock| clock.speed), report) else {
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

pub struct ShipGround<'a> {
    pub movement: Option<&'a MovementContent>,
    pub world: Option<&'a counter_move::RouteWorld<'a>>,
    pub anchor: &'a dyn Fn(u32, u32) -> Option<[f32; 3]>,
    pub land: &'a dyn Fn(u32) -> bool,
    pub scale: &'a dyn Fn(u32, u32) -> f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Force {
    pub id: u64,
    pub province: u32,
    pub logical_owner: u32,
    pub roster_owner: u32,
    pub expeditionary_owner: u32,
}

#[derive(Clone, Default)]
pub struct ForcePart {
    pub candidate: Option<Candidate>,
    pub position: Option<FormationPlacement>,
    pub formation: Option<Formation>,
    pub report: ModelReport,
}

impl ForcePart {
    pub fn into(self, sources: &mut Sources, report: &mut ModelReport) {
        sources.candidates.extend(self.candidate);
        sources.positions.extend(self.position);
        sources.formations.extend(self.formation);
        report.absorb(&self.report);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ResolvedHull {
    Skipped(String),
    Hull { ship: u64, flags: u64, graphics: Vec<u8>, slot: Result<Option<(usize, String)>, String> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedRoster {
    pub first_flags: Option<u64>,
    pub slots: Vec<usize>,
    pub hulls: Vec<ResolvedHull>,
}

impl UnitModelAssets {
    #[expect(clippy::too_many_arguments, reason = "one formation's every input, each named at the call")]
    pub fn naval_force(
        &self,
        force: &Force,
        route: Result<counter_move::UnitRoute<'_>, &'static str>,
        ground: &ShipGround<'_>,
        link: &dyn Fn(u32) -> Option<u32>,
        vision: &Vision,
        skip: &dyn Fn(u32) -> Option<bool>,
        ordinal: Option<u32>,
        clock_speed: Option<i32>,
        day: i32,
        roster: impl FnOnce() -> Option<Result<ResolvedRoster, String>>,
    ) -> ForcePart {
        let mut part = ForcePart::default();
        let report = &mut part.report;
        let placement = match route.and_then(|route| viewer_unit_reads::task_force_visual::visual_of(force.province, &route, ground.movement, ground.world, ground.anchor, ground.land, ground.scale)) {
            Ok(placement) => placement,
            Err(reason) => {
                report.error(format!("naval_formation_motion: {reason}"));
                return part;
            }
        };
        let route = route.expect("a placed task force has its route");
        if !self.bounds.contains_key(&force.province) {
            report.error(format!("province {} has no content bounds for model admission", force.province));
            return part;
        }
        let Some(owner) = link(force.logical_owner) else { return part };
        if vision.sees_navy(force.id, owner) == Sight::None || placement.kind == SHIP_IN_PORT {
            return part;
        }
        match skip(placement.kind) {
            Some(false) => {}
            Some(true) => return part,
            None => {
                report.undecided_mission_formations += 1;
                return part;
            }
        }
        if let Some(reason) = placement.unplaced_route {
            report.error(format!("naval_animator_route: {reason}"));
            return part;
        }
        let Some(ordinal) = ordinal else {
            report.error("naval_representative_order: no complete ProvinceUnitOrder entry");
            return part;
        };
        let Some(roster_owner) = link(force.roster_owner) else {
            report.error("naval_representative_owner: roster owner has no CountryLinks row");
            return part;
        };
        let path = route.display_path();
        let Some((ours, priority)) = representative_priority(vision, roster_owner, route.target) else {
            report.error("naval_representative_relations: country sharing/war operands are unavailable");
            return part;
        };
        let Some(min_arc) = animator_min_arc(path, clock_speed, report) else { return part };
        part.candidate = Some(Candidate {
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
            arrival_allowed: clock_speed.is_some_and(|speed| speed < 4) && route.combat_count == 0,
            day,
        });
        let position = placement.position;
        let (root, turn) = match &position.motion {
            Some(motion) => {
                let (at, heading) = motion.position_heading();
                (at, heading + MESH_FORWARD_OFFSET)
            }
            None => (position.standing, 0.0),
        };
        let standing_scale = |province| (ground.scale)(province, viewer_runtime::unit_class::CounterClass::TaskForce.standstill((ground.land)(province)));
        let from_scale = standing_scale(force.province);
        let to_scale = path.first().map_or(1.0, |&next| standing_scale(next));
        let animator_scale = placement.animator.map_or(from_scale, |sample| sample.interpolate_scale(from_scale, to_scale));
        part.position = Some(FormationPlacement { force: force.id, position, rotation: turn, scale: animator_scale });
        if matches!(placement.kind, 9 | 10) {
            return part;
        }
        let roster = match roster() {
            None => return part,
            Some(Err(reason)) => {
                part.report.error(reason);
                return part;
            }
            Some(Ok(roster)) => roster,
        };
        let report = &mut part.report;
        let mut hulls = Vec::with_capacity(roster.hulls.len());
        for hull in &roster.hulls {
            let (ship, flags, graphics, slot) = match hull {
                ResolvedHull::Skipped(reason) => {
                    report.error(reason.clone());
                    continue;
                }
                ResolvedHull::Hull { ship, flags, graphics, slot } => (*ship, *flags, graphics, slot),
            };
            let model_slot = match slot {
                Ok(Some(slot)) => Some(slot),
                Ok(None) => {
                    report.error("naval_formation_entity: no hull candidate or placeholder is registered");
                    None
                }
                Err(reason) => {
                    report.error(reason);
                    None
                }
            };
            let targets = std::array::from_fn(|index| {
                let (slot, _) = model_slot?;
                let node = if index == 0 { self.ships.root.as_ref()? } else { self.ships.nodes.get(((force.id >> 32) as usize + index) % self.ships.nodes.len().max(1))? };
                let scale = self.ships.scale * animator_scale;
                let [x, y, z] = node.translation.map(|v| v * scale);
                let (sin, cos) = turn.sin_cos();
                Some(Target {
                    province: force.province,
                    identity: force.id,
                    order: placement.kind,
                    occupant: ship,
                    part: index as u32,
                    kind: Kind::Ship,
                    mesh: *slot,
                    origin: root,
                    instance: ModelInstance {
                        position: [root[0] + x * cos - z * sin, root[1] + y, root[2] + x * sin + z * cos],
                        rotation: turn + node.basis[2][0].atan2(node.basis[2][2]),
                        scale: scale * node.uniform_scale(),
                        pitch: 0.0,
                        train_clip: [0.0; 4],
                        animation: viewer_models::entity_animation::IDLE,
                        animation_seed: force.id as u32 ^ (force.id >> 32) as u32,
                        ..ModelInstance::default()
                    },
                })
            });
            hulls.push(Hull { flags, graphics: graphics.clone(), targets });
            if let Some((_, name)) = model_slot {
                *report.by_entity.entry(name.clone()).or_default() += 1;
            }
        }
        if self.ships.root.is_none() && roster.first_flags.is_some_and(|flags| flags & viewer_unit_reads::counter_order::FORMATION_CAPITAL_MASK != 0) {
            report.error("naval_formation_capital_root: authored formation has no root attach point");
        }
        report.ship_hulls += roster.slots.iter().filter(|&&index| index != 0 || self.ships.root.is_some()).count();
        if roster.slots.iter().any(|&index| index != 0) {
            report.support_locator_rng += 1;
        }
        part.formation = Some(Formation { identity: force.id, province: force.province, slot: placement.kind, roster: hulls });
        part
    }

    fn resolve_roster<'rows>(
        &self,
        force: &Force,
        roster: &[&'rows replay_data::rows::ShipModelRow],
        graphics: &dyn Fn(u64) -> Option<&'rows Vec<u8>>,
        class: &dyn Fn(u64) -> Option<u32>,
        tag: &dyn Fn(u32) -> Result<String, String>,
        names: &mut impl selection::Names<'rows>,
        hour: Option<&Hour<'_>>,
    ) -> Result<ResolvedRoster, String> {
        if roster.iter().any(|model| model.counter.is_none()) {
            return Err("naval_formation_roster: ShipModels lacks roster ordinals".into());
        }
        let mut ordered = roster.to_vec();
        ordered.sort_by_key(|model| model.counter.as_ref().map_or(0, |counter| counter.roster_ordinal));
        let owner = model_owner(force.roster_owner, force.expeditionary_owner, true);
        let told = std::cell::OnceCell::new();
        let mut hulls = Vec::with_capacity(ordered.len());
        for model in &ordered {
            let explicit = if model.variant == 0 {
                Vec::new()
            } else if let Some(name) = graphics(model.variant) {
                name.clone()
            } else {
                hulls.push(ResolvedHull::Skipped("naval_formation_graphics: selected variant has no recorded explicit entity".into()));
                continue;
            };
            let definition = class(model.ship).map(|class| (force.roster_owner, class));
            let selected = names.name(|| told.get_or_init(|| tag(owner)).as_deref().map_err(Clone::clone), model, &explicit, definition);
            let slot = selected.and_then(|name| self.ship_slot_selected(name, hour));
            hulls.push(ResolvedHull::Hull { ship: model.ship, flags: model.formation_flags, graphics: explicit, slot });
        }
        Ok(ResolvedRoster { first_flags: ordered.first().map(|model| model.formation_flags), slots: formation_slots(&ordered).iter().map(|(index, _)| *index).collect(), hulls })
    }
}

impl UnitModelAssets {
    pub fn resolve_rosters(
        &self,
        frame: &DecodedFrame,
        limits: equipment_graphics::Limits<'_>,
        terrain: &BTreeMap<u32, String>,
        localisation: &Localisation,
        forces: &[u64],
    ) -> io::Result<Vec<(u64, Option<Result<ResolvedRoster, String>>)>> {
        let hour = Hour::read_naval(self, frame, terrain, limits, localisation)?;
        let variants = rows::<tables::VariantDefinitions>(frame)?;
        let variant_names: BTreeMap<_, _> = variants.rows().iter().map(|row| (row.id, &row.explicit_entity)).collect();
        let links: BTreeMap<_, _> = core::<tables::CountryLinks>(frame)?.iter().map(|row| (row.id, row.canonical_slot)).collect();
        let countries: BTreeMap<_, _> = core::<tables::Countries>(frame)?.iter().map(|row| (row.canonical_slot, row)).collect();
        let ships = rows::<tables::Ships>(frame)?;
        let classes: BTreeMap<_, _> = ships.rows().iter().map(|ship| (ship.id, ship.class_id as u32)).collect();
        let models: BTreeMap<_, _> = rows::<tables::ShipModels>(frame)?.rows().iter().map(|row| (row.ship, row)).collect();
        let mut roster: BTreeMap<u64, Vec<&replay_data::rows::ShipModelRow>> = BTreeMap::new();
        for ship in ships.rows() {
            if let Some(model) = models.get(&ship.id) {
                roster.entry(ship.task_force).or_default().push(*model);
            }
        }
        let task_forces = rows::<tables::TaskForces>(frame)?;
        let by_id: BTreeMap<_, _> = task_forces.rows().iter().map(|force| (force.id, force)).collect();
        let mut names = selection::Pass::new(self, frame, Some(&hour));
        let tag_of = |owner: u32| -> Result<String, String> {
            let country = links.get(&owner).and_then(|slot| countries.get(slot)).ok_or("naval_formation_model_country: native model owner is unavailable")?;
            content_reader_parser::parser::text(content_reader_parser::parser::c_string(&country.tag)).map(str::to_owned).map_err(|error| format!("naval_formation_model_country: {error}"))
        };
        let mut out = Vec::with_capacity(forces.len());
        for &id in forces {
            let resolved = by_id.get(&id).and_then(|force| {
                let values = Force { id, province: force.province, logical_owner: force.logical_owner, roster_owner: force.roster_owner, expeditionary_owner: force.expeditionary_owner };
                let models = roster.get(&id)?;
                Some(self.resolve_roster(&values, models, &|variant| variant_names.get(&variant).copied(), &|ship| classes.get(&ship).copied(), &tag_of, &mut names, Some(&hour)))
            });
            out.push((id, resolved));
        }
        Ok(out)
    }
}

fn animator_min_arc(
    path: &[u32],
    clock_speed: Option<i32>,
    report: &mut ModelReport,
) -> Option<f32> {
    if path.is_empty() {
        return Some(0.0);
    }
    let Some(speed) = clock_speed else {
        report.error("naval_animator_clock: Clock speed is unavailable");
        return None;
    };
    Some(if speed >= 4 {
        counter_move::SETTLED_ARC
    } else {
        0.0
    })
}

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

fn formation_slots<'a>(
    roster: &[&'a replay_data::rows::ShipModelRow],
) -> Vec<(usize, &'a replay_data::rows::ShipModelRow)> {
    let recorded = !roster.is_empty() && roster.iter().all(|row| row.counter.is_some());
    if !recorded {
        return Vec::new();
    }
    let mut ordered = roster.to_vec();
    ordered.sort_by_key(|row| row.counter.as_ref().map_or(0, |c| c.roster_ordinal));
    let mut slots = Vec::new();
    if let Some(first) = ordered.first()
        && first.formation_flags & viewer_unit_reads::counter_order::FORMATION_CAPITAL_MASK
            != 0
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
    roster: &[&'a replay_data::rows::ShipModelRow],
) -> Vec<&'a replay_data::rows::ShipModelRow> {
    let mut selected = Vec::new();
    for mask in [1u64 << 8, (1u64 << 7) | (1u64 << 39)] {
        for &row in roster {
            if selected.len() == 3 {
                return selected;
            }
            if row.formation_flags & mask != 0
                && !selected
                    .iter()
                    .any(|r: &&replay_data::rows::ShipModelRow| r.ship == row.ship)
            {
                selected.push(row);
            }
        }
    }
    selected
}

pub fn naval_differ(mine: &Sources, theirs: &Sources) -> (u64, u64, Option<String>) {
    fn candidate(c: &Candidate) -> String {
        format!(
            "candidate {} at {} ordinal {} slot {} priority {} ours {} class {:?} land {} samples {} arc {:?} route {:?} speed {:?} min {:?} arrival {} day {}",
            c.identity, c.province, c.ordinal, c.slot, c.priority, c.ours, c.class, c.land_eligible, c.sample_count, c.next_arc, c.route, c.speed, c.min_arc, c.arrival_allowed, c.day
        )
    }
    fn placement(p: &FormationPlacement) -> String {
        format!("placement {} {:?} rotation {:?} scale {:?}", p.force, p.position, p.rotation, p.scale)
    }
    fn formation(f: &Formation) -> String {
        let hulls: Vec<String> = f.roster.iter().map(|hull| format!("{:#x} {:?} {:?}", hull.flags, hull.graphics, hull.targets)).collect();
        format!("formation {} at {} slot {}: {hulls:?}", f.identity, f.province, f.slot)
    }
    fn battle(b: &viewer_unit_history::unit_history::naval_combat::Battle) -> String {
        let health: Vec<_> = b.health.iter().map(|h| (h.capital, h.screens, h.sunk_fraction)).collect();
        let impacts: Vec<bool> = b.impacts.iter().map(Option::is_some).collect();
        format!(
            "battle {} stamp {} root {:?} locators {:?} hulls {:?} offered {:?} impacts {impacts:?} effects {:?} health {health:?} death {:?}",
            b.id, b.stamp, b.root, b.locators, b.hulls, b.offered, b.effects, b.death_time
        )
    }
    let lines = |sources: &Sources| -> Vec<String> {
        let combats = &sources.combats;
        sources
            .candidates
            .iter()
            .map(candidate)
            .chain(sources.positions.iter().map(placement))
            .chain(sources.formations.iter().map(formation))
            .chain(combats.battles.values().map(battle))
            .chain(combats.links.iter().map(|(id, links)| format!("link {id} {links:?}")))
            .chain(combats.roots.iter().map(|(province, root)| format!("root {province} {root:?}")))
            .collect()
    };
    let (mine, theirs) = (lines(mine), lines(theirs));
    let mut differing = mine.len().abs_diff(theirs.len()) as u64;
    let mut first = (mine.len() != theirs.len()).then(|| format!("{} entries against {}", mine.len(), theirs.len()));
    for (a, b) in mine.iter().zip(&theirs) {
        if a != b {
            differing += 1;
            first.get_or_insert_with(|| format!("{a} against {b}"));
        }
    }
    (mine.len().max(theirs.len()) as u64, differing, first)
}

#[cfg(test)]
mod tests;
