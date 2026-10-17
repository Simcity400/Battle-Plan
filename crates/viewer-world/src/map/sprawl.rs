//! City placement and damage-driven particle states.
//!
//! The authored stride is in full map pixels. Each eligible urban sample selects a mesh by its
//! Chebyshev edge ring and a mixer seeded by province*x*z. The mixer also selects one of four
//! quarter turns. `density` displaces samples along the normalized, ring-weighted sum of eight
//! neighbours divided by the sample's own ring. Rivers reject samples with water in their
//! [-step, step) rectangle; displaced positions are checked again. Heights use half-pixel bilinear
//! sampling. Infrastructure and civilian-factory health select particle state separately from
//! placement.

use std::collections::BTreeSet;
use std::{collections::BTreeMap, io, sync::Arc};
#[cfg(test)]
use viewer_foundation::map::camera::MapDefines;
#[cfg(test)]
use viewer_foundation::map::camera::MapSize;
#[cfg(test)]
use viewer_foundation::map::camera::Placement;
use viewer_foundation::map::tables::core;
use viewer_foundation::table_access::TokenNames;

use installed_content::{
    ResolvedLayers,
    cities::Cities,
    image::{IndexedImage, native::Decode},
};

use installed_content::buildings::BuildingType;
use replay::{
    frame::DecodedFrame,
    rows::{BuildingRow, ProvinceRow, tables},
};
use {
    super::{
        entity_animation::{BUILDING_IDLE, Clock, Request, STOPPED},
        models::{ModelField, ModelInstance, ModelLibrary},
    },
    viewer_foundation::map::{
        camera::Camera,
        province_admission::{Admission, province_boxes},
        province_map::{HeightField, ProvinceMap},
    },
};

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Side of the sprawl cull grid, in world units. The engine buckets its trees
/// on 256-unit tiles; nothing ties the sprawl to a size, and a
/// city blob is tens of pixels across, so 64 keeps a group inside one city.
pub const SPRAWL_TILE: f32 = 64.0;

/// The distance ring a pixel sits in, capped at the native search limit.
pub type DistanceField = Vec<u16>;

/// Chebyshev distance from every urban pixel to the nearest non-urban one, in
/// map pixels; 0 outside the urban mask.
///
/// The walk is a breadth-first sweep seeded at the pixels that touch a
/// non-urban neighbour, which is where distance 1 is defined. Off-map
/// neighbours count as non-urban, so a blob running off the north edge is
/// bounded there. `cap` stops the sweep at the native search limit; rings also
/// supply the displacement weights, independently of mesh thresholds.
pub fn distance_to_edge(urban: &[bool], width: usize, height: usize, cap: u16) -> DistanceField {
    let mut distance = vec![0u16; urban.len()];
    let mut frontier = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            if !urban[index] {
                continue;
            }
            let edge = (-1i64..=1).any(|dy| {
                (-1i64..=1).any(|dx| {
                    if dx == 0 && dy == 0 {
                        return false;
                    }
                    let nx = x as i64 + dx;
                    let ny = y as i64 + dy;
                    if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                        return true;
                    }
                    !urban[ny as usize * width + nx as usize]
                })
            });
            if edge {
                distance[index] = 1;
                frontier.push(index);
            }
        }
    }
    let mut step = 1u16;
    while !frontier.is_empty() && step < cap {
        step += 1;
        let mut next = Vec::new();
        for index in frontier {
            let x = index % width;
            let y = index / width;
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nx = x as i64 + dx;
                    let ny = y as i64 + dy;
                    if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                        continue;
                    }
                    let neighbour = ny as usize * width + nx as usize;
                    if urban[neighbour] && distance[neighbour] == 0 {
                        distance[neighbour] = step;
                        next.push(neighbour);
                    }
                }
            }
        }
        frontier = next;
    }
    // Anything still unvisited is deeper than the cap, which is the last ring.
    for (index, value) in distance.iter_mut().enumerate() {
        if urban[index] && *value == 0 {
            *value = cap;
        }
    }
    distance
}

/// Full-map-pixel stride.
pub fn sample_stride(pixel_step: u32) -> usize {
    pixel_step.max(1) as usize
}

/// Province/x/z integer mixer, with native wrapping at every multiplication.
fn city_roll(province: u32, x: u32, z: u32) -> u32 {
    fn mix(seed: u32, a: u32, b: u32, c: u32) -> u32 {
        let value = seed.wrapping_mul(a);
        let value = (value ^ (value >> 8)).wrapping_add(b);
        let value = (value ^ (value << 8)).wrapping_mul(c);
        value ^ (value >> 8)
    }
    let seed = province.wrapping_mul(x).wrapping_mul(z);
    let a = mix(seed, 0x7eb8_da44, 0x10d8_365e, 0x6d22_f31b);
    let b = mix(seed, 0x205a_d647, 0xb4f3_c315, 0x387a_9f3a);
    let value = a.wrapping_sub(b.wrapping_mul(0x4ad6_85b3));
    let value = (value ^ (value >> 8)).wrapping_add(0x68e3_1da4);
    let value = (value ^ (value << 8)).wrapping_mul(0x1b56_c4e9);
    let value = (value ^ (value >> 8)).wrapping_mul(0x92d6_8ca2);
    value ^ (value >> 8)
}

/// texel centres at half units, clamped at shore.
fn city_height(heights: &HeightField<'_>, x: f32, z: f32) -> f32 {
    let x = x - 0.5;
    let z = z - 0.5;
    let ix = (x as i32).clamp(0, heights.width as i32 - 2) as usize;
    let iz = (z as i32).clamp(0, heights.height as i32 - 2) as usize;
    let tx = x - ix as f32;
    let tz = z - iz as f32;
    let at = |x: usize, z: usize| {
        f32::from(heights.values[(heights.height as usize - 1 - z) * heights.width as usize + x])
    };
    let low = at(ix, iz) + (at(ix + 1, iz) - at(ix, iz)) * tx;
    let high = at(ix, iz + 1) + (at(ix + 1, iz + 1) - at(ix, iz + 1)) * tx;
    ((low + (high - low) * tz) * 0.1).max(9.5)
}

/// One placed house, before the mesh library resolves its entity.
#[derive(Clone, Debug, PartialEq)]
pub struct SprawlBuilding {
    /// The entity name `map/cities.txt` named.
    pub entity: String,
    pub position: [f32; 3],
    pub rotation: f32,
    /// The city object's lattice province, before displacement.
    pub province: u32,
    /// The displaced bitmap province owns the resulting city instance.
    pub owner: u32,
}

/// What the generator produced, with the counts the viewer reports.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SprawlPlan {
    pub buildings: Vec<SprawlBuilding>,
    /// Urban pixels of `map/terrain.bmp`.
    pub urban_pixels: usize,
    /// Samples the `pixel_step` grid took inside the mask.
    pub samples: usize,
    /// Entity names used, in first-use order.
    pub entities: Vec<String>,
    pub bounds: BTreeMap<u32, [f32; 4]>,
}

/// Native two-stage placement: build the eligible lattice, then displace it.
/// Inputs share the bitmap's top-down row order; the native lattice walks +Z.
pub fn generate(
    cities: &Cities,
    style: &IndexedImage,
    urban: &[bool],
    heights: &HeightField<'_>,
    provinces: &[u32],
    rivers: &IndexedImage,
) -> io::Result<SprawlPlan> {
    let width = style.width as usize;
    let height = style.height as usize;
    if width < 2
        || height < 2
        || urban.len() != width * height
        || provinces.len() != urban.len()
        || rivers.indices.len() != urban.len()
        || rivers.width != style.width
        || rivers.height != style.height
        || heights.width != style.width
        || heights.height != style.height
        || heights.values.len() != urban.len()
    {
        return Err(invalid(
            "city style, terrain, rivers, provinces and heights must share a grid",
        ));
    }
    // The generator scans out to ring 100 even if the last mesh starts earlier:
    // the uncapped ring also determines displacement and neighbour weights.
    let distance = distance_to_edge(urban, width, height, 100);
    let step_x = sample_stride(cities.pixel_step_x);
    let step_z = sample_stride(cities.pixel_step_y);
    let index = |x: usize, z: usize| (height - 1 - z) * width + x;
    let eligible = |x: usize, z: usize| {
        if !urban[index(x, z)] {
            return false;
        }
        // Skip river rectangles at the map margins.
        if step_x < x
            && step_z < z
            && x < width.saturating_sub(step_x)
            && z < height.saturating_sub(step_z)
        {
            for nz in z - step_z..z + step_z {
                for nx in x - step_x..x + step_x {
                    if rivers.indices[index(nx, nz)] < 254 {
                        return false;
                    }
                }
            }
        }
        true
    };
    let mut plan = SprawlPlan {
        urban_pixels: urban.iter().filter(|flag| **flag).count(),
        bounds: province_boxes(&ProvinceMap {
            width: style.width,
            height: style.height,
            ids: provinces,
        }),
        ..SprawlPlan::default()
    };
    // Only actual mesh candidates contribute to the eight-neighbour sum.
    let mut candidates = BTreeMap::new();
    for z in (0..height).step_by(step_z) {
        for x in (0..width).step_by(step_x) {
            if !eligible(x, z) {
                continue;
            }
            plan.samples += 1;
            let i = index(x, z);
            let Some(group) = cities.group_for(style.indices[i]) else {
                continue;
            };
            let Some(building) = group
                .building_for(u32::from(distance[i]))
                .filter(|building| !building.meshes.is_empty())
            else {
                continue;
            };
            let roll = city_roll(provinces[i], x as u32, z as u32);
            let entity = &building.meshes[((roll & 0x7fff_ffff) as usize) % building.meshes.len()];
            candidates.insert((x, z), (entity, roll, group.density as f32, distance[i]));
        }
    }
    let mut seen = BTreeMap::new();
    for (&(x, z), &(entity, roll, density, ring)) in &candidates {
        let mut dx = 0.0;
        let mut dz = 0.0;
        if density != 0.0 {
            for oz in [-1isize, 0, 1] {
                for ox in [-1isize, 0, 1] {
                    if ox == 0 && oz == 0 {
                        continue;
                    }
                    let nz = z as isize + oz * step_z as isize;
                    if nz < 0 || nz >= height as isize {
                        continue;
                    }
                    let nx = (x as isize + ox * step_x as isize).rem_euclid(width as isize);
                    if let Some(&(_, _, _, weight)) = candidates.get(&(nx as usize, nz as usize)) {
                        dx += (ox * step_x as isize) as f32 * f32::from(weight);
                        dz += (oz * step_z as isize) as f32 * f32::from(weight);
                    }
                }
            }
            if dx != 0.0 || dz != 0.0 {
                let length = (dx * dx + dz * dz).sqrt();
                dx /= length;
                dz /= length;
            }
            let amount = density / f32::from(ring.max(1));
            dx *= amount;
            dz *= amount;
        }
        let world_x = x as f32 + dx;
        let world_z = z as f32 + dz;
        if world_x < 0.0
            || world_z < 0.0
            || world_x >= width as f32
            || world_z >= height as f32
            || !eligible(world_x as usize, world_z as usize)
        {
            continue;
        }
        if seen.insert(entity.clone(), ()).is_none() {
            plan.entities.push(entity.clone());
        }
        plan.buildings.push(SprawlBuilding {
            entity: entity.clone(),
            position: [world_x, city_height(heights, world_x, world_z), world_z],
            rotation: (roll & 3) as f32 * std::f32::consts::FRAC_PI_2,
            province: provinces[index(x, z)],
            owner: match provinces[index(world_x as usize, world_z as usize)] {
                0 => provinces[index(x, z)],
                owner => owner,
            },
        });
    }
    Ok(plan)
}

/// The sprawl's mesh library and placed instances.
pub struct Sprawl {
    pub library: ModelLibrary,
    pub field: ModelField,
    pub plan_summary: SprawlSummary,
    presentation: Presentation,
}

/// City damage uses the lower average health of infrastructure and `industrial_complex` buildings.
/// Other factory types and bombing damage do not enter this value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Damage {
    /// None marks a known null province-state link rather than a missing recorded row.
    health: BTreeMap<u32, Option<i64>>,
    pub refused: Option<String>,
}

impl Damage {
    pub fn read(
        frame: &DecodedFrame,
        types: &[BuildingType],
        available: &BTreeMap<(bool, u32, usize), &BuildingRow>,
    ) -> Self {
        let read = || -> io::Result<Self> {
            // An absent row means no building only in a complete table.
            let states = frame.complete::<tables::StateBuildings>()?;
            let provinces = frame.complete::<tables::ProvinceBuildings>()?;
            frame.complete::<tables::LexicalTokens>()?;
            let names = TokenNames::new(frame)?;
            for row in states
                .iter()
                .chain(provinces)
                .filter(|row| row.building_type != 0)
            {
                names
                    .name(row.building_type)
                    .map_err(|error| invalid(error.to_string()))?;
            }
            let provinces = core::<tables::Provinces>(frame)?;
            Self::from_rows(provinces, types, available)
        };
        read().unwrap_or_else(|error| Self {
            refused: Some(error.to_string()),
            ..Self::default()
        })
    }

    fn from_rows(
        provinces: &[ProvinceRow],
        types: &[BuildingType],
        available: &BTreeMap<(bool, u32, usize), &BuildingRow>,
    ) -> io::Result<Self> {
        let infrastructure: Vec<_> = types
            .iter()
            .enumerate()
            .filter(|(_, kind)| kind.name.eq_ignore_ascii_case("infrastructure"))
            .map(|(index, _)| index)
            .collect();
        if infrastructure.len() > 1 {
            return Err(invalid(format!(
                "infrastructure: {} ambiguous city-damage templates",
                infrastructure.len()
            )));
        }
        let civilian: Vec<_> = types
            .iter()
            .enumerate()
            .filter(|(_, kind)| kind.name == "industrial_complex")
            .map(|(index, _)| index)
            .collect();
        if !civilian.is_empty() {
            let count = types
                .iter()
                .filter(|kind| kind.name.eq_ignore_ascii_case("industrial_complex"))
                .count();
            if count > 1 {
                return Err(invalid(format!(
                    "industrial_complex: {count} ambiguous city-damage templates"
                )));
            }
        }
        let selected: Vec<_> = infrastructure.into_iter().chain(civilian).collect();
        let mut health = BTreeMap::new();
        for province in provinces {
            // The province must resolve to a state even for provincial building types.
            let mut minimum = 100_000;
            if province.state != 0 {
                for &index in &selected {
                    // Provincial templates use the province's buildings; other templates use its state's buildings.
                    // Templates start enabled. The content ordinal identifies the corresponding recorded building.
                    let provincial = types[index].provincial();
                    let location = if provincial {
                        province.id
                    } else {
                        province.state
                    };
                    if let Some(row) = available.get(&(provincial, location, index))
                        && row.level > 0
                    {
                        minimum = minimum.min(average_health(
                            row.level,
                            row.healthy_level,
                            row.partial_health,
                        ));
                    }
                }
            }
            health.insert(province.id, (province.state != 0).then_some(minimum));
        }
        Ok(Self {
            health,
            refused: None,
        })
    }
}

/// Average building health from recorded level, healthy level and partial health, preserving
/// exceptional fixed5 values and wrapping multiplications.
pub fn average_health(level: i16, healthy_level: i16, partial_health: i64) -> i64 {
    let exceptional =
        |value: i64| (value as u64).wrapping_add(0x7fff_ffff_ffff_fffe) >= 0xffff_ffff_ffff_fffd;
    if level == 0 {
        return 0;
    }
    if level == healthy_level {
        return 100_000;
    }
    let partial = if exceptional(partial_health) {
        0xffff_ffff
    } else {
        partial_health.wrapping_mul(100_000) / 10_000_000
    };
    let total = partial.wrapping_add(i64::from(healthy_level) * 100_000);
    if exceptional(total) {
        0xffff_ffff
    } else {
        total.wrapping_mul(100_000) / (i64::from(level) * 100_000)
    }
}

#[derive(Clone)]
struct City {
    province: u32,
    bounds: [f32; 4],
    mesh: usize,
    instance: ModelInstance,
}

#[derive(Clone)]
pub struct Presentation {
    cities: Arc<[City]>,
    damaged: i64,
    destroyed: i64,
    damage: Damage,
    view: Option<(Camera, [u32; 2], bool)>,
    actors: BTreeMap<u32, Actor>,
    last_time: Option<f32>,
}

#[derive(Clone)]
struct Actor {
    start: f32,
    state: &'static str,
    requests: Arc<Vec<Request>>,
    visible: bool,
}

impl Presentation {
    pub fn set_damage(&mut self, damage: &Damage) {
        if self.damage != *damage {
            self.damage = damage.clone();
        }
        self.view = None;
    }

    /// Preserve crossed hourly state requests at the last presentation time,
    /// just as the individual-building history does for skipped hours.
    pub fn advance(&mut self, damage: &Damage) {
        if let (Some((camera, size, enabled)), Some(time)) = (self.view, self.last_time) {
            self.set_damage(damage);
            self.update(camera, size, enabled, time);
        }
    }

    fn state(&self, province: u32) -> Option<&'static str> {
        // Leave target zero when the province has no state,
        // bypassing both thresholds (which a mod may set above full health).
        let Some(&health) = self.damage.health.get(&province)?.as_ref() else {
            return Some("idle");
        };
        // both thresholds are strict; damaged is tested first.
        if health >= self.damaged {
            Some("idle")
        } else if health < self.destroyed {
            Some("burning") // damaged is .
        } else {
            Some("damaged")
        }
    }

    pub fn update(
        &mut self,
        camera: Camera,
        size: [u32; 2],
        enabled: bool,
        time: f32,
    ) -> Option<Vec<(usize, ModelInstance)>> {
        if self.last_time.is_some_and(|last| time < last) {
            self.reset();
        }
        self.last_time = Some(time);
        let view = (camera, size, enabled);
        if self.view == Some(view) {
            return None;
        }
        self.view = Some(view);
        let admission = Admission::new(camera, size);
        let mut placed = Vec::new();
        let mut active = BTreeSet::new();
        for city in self
            .cities
            .iter()
            .filter(|city| enabled && admission.admits(city.bounds))
        {
            let mut instance = city.instance;
            active.insert(instance.animation_seed);
            if let Some(state) = self.state(city.province) {
                let actor = self
                    .actors
                    .entry(instance.animation_seed)
                    .or_insert_with(|| Actor {
                        start: time,
                        state,
                        visible: true,
                        // Initialize the city-state cache to zero. A first healthy show preserves the entity default.
                        requests: Arc::new(if state == "idle" {
                            Vec::new()
                        } else {
                            vec![Request {
                                time: 0.0,
                                phase_time: None,
                                state: state.into(),
                            }]
                        }),
                    });
                if actor.state != state {
                    Arc::make_mut(&mut actor.requests).push(Request {
                        time: time - actor.start,
                        phase_time: None,
                        state: state.into(),
                    });
                    actor.state = state;
                }
                instance.animation = BUILDING_IDLE;
            }
            placed.push((city.mesh, instance));
        }
        // Request idle, reset the city-state cache to zero and deactivate the precreated entity. Retain
        // request history so re-entry preserves idle even when the authored default differs.
        for (id, actor) in &mut self.actors {
            let visible = active.contains(id);
            if actor.visible && !visible {
                Arc::make_mut(&mut actor.requests).push(Request {
                    time: time - actor.start,
                    phase_time: None,
                    state: "idle".into(),
                });
                actor.state = "idle";
            }
            actor.visible = visible;
        }
        Some(placed)
    }

    pub fn clocks(&self, time: f32) -> BTreeMap<(usize, u32, u32), Clock> {
        self.cities
            .iter()
            .filter_map(|city| {
                let instance = city.instance;
                let actor = self.actors.get(&instance.animation_seed)?;
                if !actor.visible || !self.damage.health.contains_key(&city.province) {
                    return None;
                }
                Some((
                    (
                        city.mesh,
                        instance.animation_seed,
                        instance.animation_identity,
                    ),
                    Clock {
                        time: time - actor.start,
                        phase: None,
                        requests: Arc::clone(&actor.requests),
                    },
                ))
            })
            .collect()
    }

    pub fn reset(&mut self) {
        self.actors.clear();
        self.view = None;
        self.last_time = None;
    }
}

/// Counts the viewer reports for the layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SprawlSummary {
    pub urban_pixels: usize,
    pub samples: usize,
    pub placed: usize,
    pub entities: usize,
    pub meshes: usize,
}

impl Sprawl {
    /// Resolves the plan's entities and groups its buildings for drawing.
    /// `defines` give the city damage thresholds: the graphics defines while the visual mods
    /// show.
    pub fn load(
        layers: &ResolvedLayers,
        decode: Decode,
        plan: &SprawlPlan,
        defines: &installed_content::defines::Defines,
        warnings: &mut Vec<String>,
    ) -> io::Result<Self> {
        // The entities and meshes of every descriptor the engine registers
        // ( enumerates `.gfx` recursively, registers
        // them), as the shared roster holds them.
        let roster = layers.descriptor_roster()?;
        warnings.extend(
            roster
                .unreadable
                .iter()
                .map(|file| format!("City sprawl descriptor unavailable: {file}")),
        );
        let (refused, entities): (Vec<&String>, Vec<&String>) = plan
            .entities
            .iter()
            .partition(|entity| roster.unreadable_names.contains_key(*entity));
        if !refused.is_empty() {
            warnings.push(format!(
                "City sprawl entities a descriptor file that does not parse may declare are not drawn: {}",
                refused
                    .iter()
                    .map(|entity| format!("{entity} ({})", roster.unreadable_names[*entity]))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let entities: Vec<String> = entities.into_iter().cloned().collect();
        let mut model_warnings = Vec::new();
        let mut library = ModelLibrary::load(
            layers,
            decode,
            &roster.descriptors,
            &entities,
            &mut model_warnings,
        )?;
        // An entity only a visual mod's unreadable descriptor may declare is drawn from the
        // recording, whose own descriptors declare it readably; every other is recorded by name
        // for the map's fallback.
        let recorded = layers.recorded_descriptor_roster()?;
        let (from_recording, refused): (Vec<String>, Vec<String>) =
            refused.into_iter().cloned().partition(|entity| {
                recorded.as_ref().is_some_and(|recorded| {
                    !recorded.unreadable_names.contains_key(entity)
                        && recorded.descriptors.entities.contains_key(entity)
                })
            });
        if !from_recording.is_empty() {
            library.merge(ModelLibrary::load_recorded(
                layers,
                decode,
                &from_recording,
                |entity| format!("a descriptor that does not parse may declare {entity}"),
            )?);
        }
        for entity in refused.iter().chain(
            entities
                .iter()
                .filter(|entity| library.slot(entity).is_none()),
        ) {
            layers.note_refused(format!("model entity {entity}"));
        }
        warnings.extend(model_warnings.into_iter().map(|(_, message)| message));
        let mut placed = Vec::with_capacity(plan.buildings.len());
        let mut cities = Vec::with_capacity(plan.buildings.len());
        for (id, building) in plan.buildings.iter().enumerate() {
            let Some(slot) = library.slot(&building.entity) else {
                continue;
            };
            let instance = ModelInstance {
                position: building.position,
                rotation: building.rotation,
                // The entity and pdxmesh scales are baked into the library's
                // vertices, so an instance carries no further scale.
                scale: 1.0,
                pitch: 0.0,
                train_clip: [0.0; 4],
                animation: STOPPED,
                animation_seed: id as u32,
                animation_identity: 1,
                ..ModelInstance::default()
            };
            placed.push((slot, instance));
            if let Some(&bounds) = plan.bounds.get(&building.owner) {
                cities.push(City {
                    province: building.province,
                    bounds,
                    mesh: slot,
                    instance,
                });
            } else {
                warnings.push(format!("City sprawl entity {}: 1 particle actor refused; province {} has no bitmap box", building.entity, building.owner));
            }
        }
        let summary = SprawlSummary {
            urban_pixels: plan.urban_pixels,
            samples: plan.samples,
            placed: placed.len(),
            entities: plan.entities.len(),
            meshes: library.meshes.len(),
        };
        Ok(Self {
            field: ModelField::build(&library, placed, SPRAWL_TILE),
            library,
            plan_summary: summary,
            presentation: Presentation {
                cities: cities.into(),
                damaged: defines.fixed5("NGraphics", "CITY_DAMAGED_THRESHOLD")?,
                destroyed: defines.fixed5("NGraphics", "CITY_DESTROYED_THRESHOLD")?,
                damage: Damage::default(),
                view: None,
                actors: BTreeMap::new(),
                last_time: None,
            },
        })
    }

    pub fn presentation(&self) -> Presentation {
        self.presentation.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content::cities;

    fn damage_types() -> Vec<BuildingType> {
        ["infrastructure", "industrial_complex", "arms_factory"]
            .map(|name| BuildingType {
                name: name.into(),
                ..Default::default()
            })
            .into()
    }

    fn presentation(damage: Damage) -> Presentation {
        Presentation {
            cities: Arc::default(),
            damaged: 80_000,
            destroyed: 30_000,
            damage,
            view: None,
            actors: BTreeMap::new(),
            last_time: None,
        }
    }

    #[test]
    fn city_damage_uses_the_lower_civilian_health_and_content_scope() {
        let mut types = damage_types();
        types[0].province_max = Some(7);
        let provinces: Vec<_> = (1..=5_000)
            .map(|id| ProvinceRow {
                id,
                state: 20,
                ..Default::default()
            })
            .collect();
        let infra = BuildingRow {
            level: 5,
            healthy_level: 1,
            partial_health: 5_000_000,
            ..Default::default()
        };
        let factory = BuildingRow {
            level: 10,
            healthy_level: 7,
            partial_health: 5_000_000,
            ..Default::default()
        };
        let arms = BuildingRow {
            level: 10,
            healthy_level: 0,
            ..Default::default()
        };
        let available = BTreeMap::from([
            ((true, 2, 0), &infra),
            ((false, 20, 1), &factory),
            ((false, 20, 2), &arms),
        ]);
        let damage = Damage::from_rows(&provinces, &types, &available).unwrap();
        assert_eq!(
            damage.health[&1],
            Some(75_000),
            "arms factories never drive city damage"
        );
        assert_eq!(
            damage.health[&2],
            Some(30_000),
            "scope follows the authored provincial cap"
        );
        assert_eq!(damage.health.len(), 5_000);
        let mut player = presentation(damage);
        assert_eq!(player.state(1), Some("damaged"));
        assert_eq!(
            player.state(2),
            Some("damaged"),
            "destroyed threshold is strict"
        );
        player.damage.health.insert(2, Some(29_999));
        assert_eq!(player.state(2), Some("burning"));
        player.damage.health.insert(2, Some(80_000));
        assert_eq!(player.state(2), Some("idle"), "damaged threshold is strict");
        player.destroyed = 90_000;
        assert_eq!(
            player.state(2),
            Some("idle"),
            "native tests damaged before destroyed"
        );
        assert_eq!(player.state(6_000), None, "missing data is not healthy");
        let stateless = Damage::from_rows(
            &[ProvinceRow {
                id: 6_001,
                state: 0,
                ..Default::default()
            }],
            &types,
            &available,
        )
        .unwrap();
        player.damage.health.extend(stateless.health);
        player.damaged = 110_000;
        assert_eq!(
            player.state(6_001),
            Some("idle"),
            "a null state bypasses thresholds"
        );
    }

    #[test]
    fn city_damage_zero_levels_and_fixed5_sentinels_follow_native_health() {
        let types = damage_types();
        let province = ProvinceRow {
            id: 7,
            state: 9,
            ..Default::default()
        };
        let absent = BuildingRow {
            level: 0,
            healthy_level: -1,
            ..Default::default()
        };
        let rows = BTreeMap::from([((false, 9, 0), &absent)]);
        assert_eq!(
            Damage::from_rows(&[province], &types, &rows)
                .unwrap()
                .health[&7],
            Some(100_000)
        );
        let row = BuildingRow {
            level: 3,
            healthy_level: 1,
            partial_health: 9_999_999,
            ..Default::default()
        };
        assert_eq!(
            average_health(row.level, row.healthy_level, row.partial_health),
            66_666
        );
        let row = BuildingRow {
            level: 1,
            healthy_level: 0,
            partial_health: i64::MAX,
            ..Default::default()
        };
        assert_eq!(
            average_health(row.level, row.healthy_level, row.partial_health),
            0xffff_ffff
        );
        for word in [i64::MIN, i64::MIN + 1, i64::MAX] {
            assert_eq!(average_health(1, 0, word), 0xffff_ffff);
        }
        assert_eq!(average_health(1, 0, i64::MAX - 3), 0);
    }

    #[test]
    fn city_damage_refuses_incomplete_rows_and_ambiguous_templates() {
        use replay::frame::TableView;
        use replay::rows::{TableId, TableRows};
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 42,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "synthetic gap".into(),
                }
                .into()
            }),
            metrics: Default::default(),
        };
        let damage = Damage::read(&frame, &damage_types(), &BTreeMap::new());
        assert!(damage.health.is_empty());
        assert!(damage.refused.unwrap().contains("StateBuildings"));
        *frame.tables[TableId::StateBuildings.index()] =
            TableView::Complete(TableRows::StateBuildings(Vec::new()));
        *frame.tables[TableId::ProvinceBuildings.index()] = TableView::Truncated {
            rows: TableRows::ProvinceBuildings(Vec::new()),
            missing: 1,
        };
        let damage = Damage::read(&frame, &damage_types(), &BTreeMap::new());
        assert!(damage.health.is_empty());
        assert!(
            damage
                .refused
                .unwrap()
                .contains("ProvinceBuildings is truncated")
        );
        let mut types = damage_types();
        types.push(BuildingType {
            name: "Infrastructure".into(),
            ..Default::default()
        });
        let error = Damage::from_rows(&[], &types, &BTreeMap::new()).unwrap_err();
        assert!(error.to_string().contains("infrastructure: 2 ambiguous"));
    }

    #[test]
    fn city_damage_drives_authored_fire_and_smoke_on_the_state_entry_clock() {
        use super::super::entity_animation::{Clip, Node, Player, Rig};
        use installed_content::mesh;
        let state = |name: &str, particle: &str| Clip {
            state: mesh::EntityState {
                name: name.into(),
                chance: 1.0,
                speed: 1.0,
                events: (!particle.is_empty())
                    .then(|| mesh::EntityEvent {
                        time: 0.1,
                        particle: Some(particle.into()),
                        trigger_once: true,
                        ..Default::default()
                    })
                    .into_iter()
                    .collect(),
                ..Default::default()
            },
            animation: None,
        };
        let rig = Rig {
            end: 1,
            nodes: vec![Node {
                mesh: Arc::new(mesh::Mesh::default()),
                native: None,
                clips: vec![
                    state("idle", ""),
                    state("damaged", "synthetic_smoke"),
                    state("burning", "synthetic_fire"),
                ],
                default: "idle".into(),
                follows_parent: false,
                parent: None,
                scale: 1.0,
                mesh_scale: 1.0,
                skin_base: None,
                rigid: 0,
                locators: BTreeMap::new(),
            }],
            ..Default::default()
        };
        let mut scene = presentation(Damage {
            health: BTreeMap::from([(7, Some(20_000))]),
            refused: None,
        });
        scene.cities = Arc::from([City {
            province: 7,
            bounds: [8.0, 8.0, 12.0, 12.0],
            mesh: 0,
            instance: ModelInstance {
                position: [10.0, 10.0, 10.0],
                animation_identity: 1,
                animation: STOPPED,
                ..Default::default()
            },
        }]);
        let mut player = Player::new(&rig);
        let mut events = Vec::new();
        let camera = Placement {
            center: [10.0, 10.0],
            height: 60.0,
            pose: viewer_foundation::map::camera::Pose::Map,
        }
        .on(MapSize::TEST, MapDefines::test());
        let instance = scene.cities[0].instance;
        let step = |scene: &mut Presentation,
                    player: &mut Player,
                    time: f32,
                    events: &mut Vec<super::super::entity_animation::Event>| {
            scene.update(camera, [1024, 768], true, time);
            let actor = &scene.actors[&0];
            player.evaluate_clock(
                &rig,
                instance,
                &Clock {
                    time: time - actor.start,
                    phase: None,
                    requests: Arc::clone(&actor.requests),
                },
                &mut [super::super::models::PaletteJoint::IDENTITY],
                events,
            );
        };
        step(&mut scene, &mut player, 50.0, &mut events);
        assert!(
            events.is_empty(),
            "a late first observation starts the city clock now"
        );
        step(&mut scene, &mut player, 50.2, &mut events);
        assert_eq!(
            events[0].definition.particle.as_deref(),
            Some("synthetic_fire")
        );
        assert!((events[0].age - 0.1).abs() < 0.0001);
        events.clear();
        scene.damage.health.insert(7, Some(60_000));
        scene.view = None;
        step(&mut scene, &mut player, 51.0, &mut events);
        assert!(events.is_empty());
        step(&mut scene, &mut player, 51.2, &mut events);
        assert_eq!(
            events[0].definition.particle.as_deref(),
            Some("synthetic_smoke")
        );
        assert_eq!(
            scene.actors[&0].requests.len(),
            2,
            "an unchanged city does not request again"
        );
        events.clear();
        // Ignore an absent authored state. In particular it must
        // not replace a mod's live smoke state with the entity default.
        let clock = Clock {
            time: 2.2,
            phase: None,
            requests: Arc::new(vec![
                Request {
                    time: 0.0,
                    phase_time: None,
                    state: "burning".into(),
                },
                Request {
                    time: 1.0,
                    phase_time: None,
                    state: "damaged".into(),
                },
                Request {
                    time: 2.0,
                    phase_time: None,
                    state: "unavailable_mod_state".into(),
                },
            ]),
        };
        player.evaluate_clock(
            &rig,
            instance,
            &clock,
            &mut [super::super::models::PaletteJoint::IDENTITY],
            &mut events,
        );
        assert_eq!(
            events[0].definition.particle.as_deref(),
            Some("synthetic_smoke")
        );
        scene.advance(&Damage {
            health: BTreeMap::from([(7, Some(20_000))]),
            refused: None,
        });
        scene.advance(&Damage {
            health: BTreeMap::from([(7, Some(60_000))]),
            refused: None,
        });
        assert_eq!(scene.actors[&0].requests.len(), 4);
        assert_eq!(
            scene.actors[&0].requests[2].time,
            scene.actors[&0].requests[3].time
        );
        scene.reset();
        scene.set_damage(&Damage {
            health: BTreeMap::from([(7, Some(100_000))]),
            refused: None,
        });
        let mut rig = rig;
        rig.nodes[0].default = "burning".into();
        let mut player = Player::new(&rig);
        let mut events = Vec::new();
        scene.update(camera, [1024, 768], true, 100.0);
        let clock = &scene.clocks(100.2)[&(0, 0, 1)];
        assert!(
            clock.requests.is_empty(),
            "the initial healthy city cache is zero"
        );
        player.evaluate_clock(
            &rig,
            instance,
            clock,
            &mut [super::super::models::PaletteJoint::IDENTITY],
            &mut events,
        );
        assert_eq!(
            events[0].definition.particle.as_deref(),
            Some("synthetic_fire"),
            "the authored default is preserved even when it differs from idle"
        );
        scene.update(camera, [1024, 768], false, 101.0);
        assert!(scene.clocks(101.0).is_empty());
        scene.update(camera, [1024, 768], true, 102.0);
        let clock = &scene.clocks(102.0)[&(0, 0, 1)];
        assert_eq!(clock.requests.len(), 1);
        assert_eq!(
            clock.requests[0].state, "idle",
            "requests idle even with cache zero"
        );
        let mut player = Player::new(&rig);
        events.clear();
        player.evaluate_clock(
            &rig,
            instance,
            clock,
            &mut [super::super::models::PaletteJoint::IDENTITY],
            &mut events,
        );
        assert!(
            events.is_empty(),
            "re-entry retains the hidden entity's idle state"
        );
    }

    fn mask(rows: &[&str]) -> (Vec<bool>, usize, usize) {
        let width = rows[0].len();
        let mut urban = Vec::new();
        for row in rows {
            assert_eq!(row.len(), width);
            urban.extend(row.bytes().map(|byte| byte == b'#'));
        }
        (urban, width, rows.len())
    }

    #[test]
    fn the_edge_ring_is_distance_one_and_the_core_counts_up() {
        let (urban, width, height) = mask(&[
            ".......", ".#####.", ".#####.", ".#####.", ".#####.", ".......",
        ]);
        let distance = distance_to_edge(&urban, width, height, 4);
        let at = |x: usize, y: usize| distance[y * width + x];
        assert_eq!(at(0, 0), 0, "a non-urban pixel has no distance");
        assert_eq!(at(1, 1), 1, "the corner touches the outside");
        assert_eq!(at(2, 2), 2, "one ring in");
        assert_eq!(at(3, 2), 2);
        // The 5x4 block is only two rings deep, so nothing reaches 3.
        assert!(distance.iter().all(|value| *value <= 2));
    }

    #[test]
    fn the_map_edge_bounds_a_blob() {
        let (urban, width, height) = mask(&["###", "###", "###"]);
        let distance = distance_to_edge(&urban, width, height, 4);
        assert_eq!(distance[0], 1);
        assert_eq!(distance[width + 1], 2, "the centre is one ring in");
    }

    #[test]
    fn the_cap_stops_the_sweep_and_the_rest_takes_it() {
        let (urban, width, height) = mask(&[
            ".........",
            ".#######.",
            ".#######.",
            ".#######.",
            ".#######.",
            ".#######.",
            ".#######.",
            ".........",
        ]);
        let distance = distance_to_edge(&urban, width, height, 2);
        let at = |x: usize, y: usize| distance[y * width + x];
        assert_eq!(at(1, 1), 1);
        assert_eq!(at(2, 2), 2);
        assert_eq!(at(4, 3), 2, "deeper than the cap stays at the cap");
    }

    const CITIES: &[u8] = br#"
types_source = "map/cities.bmp"
pixel_step_x = 2
pixel_step_y = 2
city_group = {
    color_index = 1
    density = 0.0
    building = { distance = 1 mesh = { "small" } }
    building = { distance = 2 mesh = { "large" } }
}
"#;

    fn indexed(rows: &[&str], urban_index: u8) -> IndexedImage {
        let width = rows[0].len() as u32;
        let mut indices = Vec::new();
        for row in rows {
            indices.extend(
                row.bytes()
                    .map(|byte| if byte == b'#' { urban_index } else { 0 }),
            );
        }
        IndexedImage {
            width,
            height: rows.len() as u32,
            indices,
            palette: [[0; 4]; 256],
        }
    }

    #[test]
    fn every_kept_sample_places_one_entity_on_its_own_pixel() {
        let rows = [
            "........", "..####..", "..####..", "..####..", "..####..", "........",
        ];
        let (urban, _, _) = mask(&rows);
        let style = indexed(&rows, 1);
        let cities = cities::parse(CITIES).unwrap();
        let values = vec![100u8; urban.len()];
        let heights = HeightField {
            width: style.width,
            height: style.height,
            values: &values,
        };
        let mut rivers = style.clone();
        rivers.indices.fill(254);
        let plan = generate(
            &cities,
            &style,
            &urban,
            &heights,
            &vec![1; urban.len()],
            &rivers,
        )
        .unwrap();
        assert_eq!(plan.urban_pixels, 16);
        // Full-pixel stride 2 samples one of every four urban pixels.
        assert_eq!(plan.samples, 4);
        assert_eq!(
            plan.buildings.len(),
            4,
            "density zero means no displacement"
        );
        for building in &plan.buildings {
            assert!(building.position[0] >= 2.0 && building.position[0] <= 5.0);
            assert_eq!(building.position[1], 10.0, "heightmap byte 100 is y 10.0");
            assert!(building.rotation >= 0.0);
        }
        // The blob is four pixels across, so every pixel is in ring 1 or 2.
        assert!(
            plan.entities
                .iter()
                .all(|name| name == "small" || name == "large")
        );
    }

    #[test]
    fn the_stride_is_in_full_map_pixels() {
        assert_eq!(sample_stride(2), 2);
        assert_eq!(sample_stride(4), 4);
        assert_eq!(sample_stride(1), 1);
        assert_eq!(sample_stride(0), 1, "a zero step would never advance");
    }

    #[test]
    fn a_style_index_with_no_group_places_nothing() {
        let rows = ["....", ".##.", ".##.", "...."];
        let (urban, _, _) = mask(&rows);
        // Style index 7 is declared by no `city_group`.
        let style = indexed(&rows, 7);
        let cities = cities::parse(CITIES).unwrap();
        let values = vec![100u8; urban.len()];
        let heights = HeightField {
            width: 4,
            height: 4,
            values: &values,
        };
        let mut rivers = style.clone();
        rivers.indices.fill(254);
        let plan = generate(
            &cities,
            &style,
            &urban,
            &heights,
            &vec![1; urban.len()],
            &rivers,
        )
        .unwrap();
        assert!(plan.samples > 0);
        assert!(plan.buildings.is_empty());
    }

    #[test]
    fn a_mask_of_the_wrong_size_is_refused() {
        let cities = cities::parse(CITIES).unwrap();
        let values = vec![100u8; 16];
        let heights = HeightField {
            width: 4,
            height: 4,
            values: &values,
        };
        let error = generate(
            &cities,
            &indexed(&["##", "##"], 1),
            &[false; 16],
            &heights,
            &[1; 16],
            &indexed(&["##", "##"], 254),
        )
        .unwrap_err();
        assert!(error.to_string().contains("share a grid"), "{error}");
    }
}
