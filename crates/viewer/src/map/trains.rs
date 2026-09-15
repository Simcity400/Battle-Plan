//! Supply trains (`gfx_railway_manager.cpp`, `gfx_train.cpp`, a729).
//!
//! `141598790` attempts a departure every TUNNELBANA_TIMETABLE (9200..12000
//! inclusive milliseconds). It uses the viewed country's supply companion,
//! not each railway independently. Eligible 0xe8-byte nodes have flag +0xc=0,
//! `(kind & !5)!=0` and positive `available(+0x38)-consumed(+0x40)` rounded
//! UP to whole fixed5 units. That is spare supply, not throughput or demand.
//! A weighted draw picks one node; `1411fc780` follows its parent routes,
//! `141598230` removes consecutive duplicates and REVERSES each land portion.
//! Trains therefore travel from the capital side towards the chosen hub.
//! The sum of the viewed country's +0x120 train-inventory counts caps the fleet.
//! `141c4f3c0` weights the equipment selection by those same counts, then
//! resolves country, continent, default train_gfx_database entries. Rail level
//! affects supply routing; it does not directly set a train's model or count.
//!
//! `141c503b0` advances distance by TRAIN_MAP_SPEED (3)*frame seconds even
//! when hidden. `141c4eb80` destroys a train on an invalid/access-denied route,
//! a cooling-down province, or after its tail clears the final spline. The
//! rail opacity gates visibility at camera height 300 (14159bce0); there is
//! also a per-province on-screen test, not a separate train height define.
//! `141c4f3c0` chooses min..max+1 entries, appends wagon_0/1/2, fills the
//! random list through count+(wagon_last!=null), then appends wagon_last.
//! Thus GER's authored list produces SIX TO NINE objects, not merely 3..6.
//! `141c4e350` links them using wagon_data.length, solves the chord up to ten
//! times on bends, and applies wagon_data.scale/rotation/shift. The shader's
//! TrainColor is white; it clips/fades at route endpoints over 2.5 units.
//!
//! # Recorded inputs and reconstruction (UNVERIFIED)
//!
//! There is NO SupplyNodes table in formats 14/17/18. RailEdges, RailCooldowns,
//! ProvinceSupply (hub and port levels, canonical controllers), Politics
//! (capital state), Countries and CountryLinks ARE recorded. HubMotorization
//! contains settings, not the simulation node's supply values or parent path.
//! StateBuildings cannot substitute for a node's province or spare supply.
//! Stockpiles omit the allocated-train inventory and equipment type dictionary.
//! This implementation reconstructs capital-connected land routes by shortest
//! distance on recorded edges, and weights hubs equally. It selects equipment_1
//! because the allocated equipment mix is missing. These are UNVERIFIED
//! substitutes, not the engine's supply solution; exact count agreement modulo
//! phase requires the missing companion nodes/parent paths and train inventory.
//! Naval legs and foreign-access routes require those same missing inputs.
//!
//! The unrecorded phase uses the engine's integer mixer (141598790), seed
//! 0x54524149 XOR the ordered route provinces. The global scheduler advances
//! by integer draws in 9200..12000 milliseconds, selecting one route per
//! departure. Its initial seed XORs all route seeds; each consist uses its
//! own route seed and a scheduler draw. Missing node weights and inventory
//! still prevent reproducing the engine's selected fleet. A seek is
//! independent of playback history; --dump uses time 0, window time advances.
//! The existing rail spline/anchors remain UNVERIFIED (railways.rs). Capturing
//! the engine's spline control points would settle their placement discrepancy.
//!
//! Ground truth: captures/trains_20260915/train_draws.json, frame32511: two
//! consists, 7 and 9 submitted objects, white TrainColor RGB, scales .175.. .3.
//! One trailing object has alpha 0; it and its adjacent wagon carry endpoint
//! planes. Thus 16 submitted draws do not mean 16 fully visible objects.
//! No captured transform is used as a rendering input.
//! Remaining presentation gaps: ModelLibrary retains the default skeleton pose;
//! wheel-cycle animation and locomotive exhaust are not advanced. The existing
//! railway camera gate switches immediately rather than fading over frame time.

use super::{
    borders::HeightField,
    models::{ModelField, ModelInstance, ModelLibrary},
    railways,
    tables::{core, rows},
};
use installed_content::{
    ResolvedLayers, mesh,
    parser::{self, Item, Value},
};
use replay::{frame::DecodedFrame, rows::tables};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    io,
};

pub const SPEED: f32 = 3.0;
pub const SEED: u32 = 0x5452_4149;

#[derive(Clone)]
struct Wagon {
    entity: String,
    scale: f32,
    rotation: f32,
    shift: [f32; 3],
    length: f32,
}

#[derive(Clone, Default)]
struct Consist {
    fixed: Vec<String>,
    random: Vec<String>,
    last: Option<String>,
}

pub struct TrainAssets {
    pub library: ModelLibrary,
    wagons: BTreeMap<String, Wagon>,
    lists: BTreeMap<(String, String), Consist>,
}

fn get<'a>(items: &'a [Item], key: &str) -> Option<&'a Value> {
    items
        .iter()
        .find(|i| i.key.as_deref() == Some(key.as_bytes()))
        .map(|i| &i.value)
}
fn scalar(items: &[Item], key: &str) -> Option<String> {
    get(items, key)?.scalar_str().map(str::to_owned)
}
fn number(items: &[Item], key: &str, fallback: f32) -> f32 {
    scalar(items, key)
        .and_then(|s| s.parse().ok())
        .filter(|v: &f32| v.is_finite())
        .unwrap_or(fallback)
}

impl TrainAssets {
    pub fn load(layers: &ResolvedLayers, warnings: &mut Vec<String>) -> io::Result<Self> {
        let mut wagons = BTreeMap::new();
        let mut lists = BTreeMap::new();
        for path in [
            "gfx/train_gfx_database/generic.txt",
            "gfx/train_gfx_database/NSB_generic.txt",
        ] {
            let bytes = match layers.read_uncovered(path) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            for item in parser::parse(&bytes)?.items {
                let (Some(key), Some(items)) = (item.key.as_deref(), item.value.items()) else {
                    continue;
                };
                if key == b"wagon_data" {
                    let Some(entity) = scalar(items, "entity") else {
                        continue;
                    };
                    let mut shift = [0.0; 3];
                    if let Some(values) = get(items, "shift").and_then(Value::items) {
                        for (out, value) in shift.iter_mut().zip(values) {
                            *out = value
                                .value
                                .scalar_str()
                                .and_then(|v| v.parse().ok())
                                .unwrap_or(0.0);
                        }
                    }
                    wagons.insert(
                        entity.clone(),
                        Wagon {
                            entity,
                            scale: number(items, "scale", 1.0),
                            rotation: number(items, "rotation", 0.0).to_radians(),
                            shift,
                            length: number(items, "length", 2.0),
                        },
                    );
                } else {
                    let country = String::from_utf8_lossy(key).into_owned();
                    for entry in items {
                        let (Some(kind), Some(parts)) = (entry.key.as_deref(), entry.value.items())
                        else {
                            continue;
                        };
                        let fixed = ["wagon_0", "wagon_1", "wagon_2"]
                            .iter()
                            .filter_map(|k| scalar(parts, k))
                            .collect();
                        let random = get(parts, "wagons")
                            .and_then(Value::items)
                            .unwrap_or(&[])
                            .iter()
                            .filter_map(|i| i.value.scalar_str().map(str::to_owned))
                            .collect();
                        lists.insert(
                            (country.clone(), String::from_utf8_lossy(kind).into_owned()),
                            Consist {
                                fixed,
                                random,
                                last: scalar(parts, "wagon_last"),
                            },
                        );
                    }
                }
            }
        }
        let mut paths = vec![
            "gfx/models/supply/trains/meshes.gfx".to_owned(),
            "gfx/entities/NSB_units_trains_meshes.gfx".to_owned(),
        ];
        for name in wagons.keys() {
            let stem = name.strip_suffix("_entity").unwrap_or(name);
            paths.push(format!("gfx/models/units/trains/{stem}.asset"));
            paths.push(format!("gfx/models/supply/trains/{stem}.asset"));
        }
        let mut descriptors = mesh::Descriptors::default();
        for path in paths {
            let mut bytes = match layers.read_uncovered(&path) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            let parsed = match mesh::parse_descriptors(&bytes, &path) {
                Ok(parsed) => parsed,
                Err(original) => {
                    // The game closes open blocks at EOF; several shipped NSB
                    // tender entities omit their final brace (as do unit assets).
                    let open = bytes.iter().filter(|b| **b == b'{').count();
                    let close = bytes.iter().filter(|b| **b == b'}').count();
                    if open <= close {
                        return Err(original);
                    }
                    bytes.extend(std::iter::repeat_n(b'}', open - close));
                    let parsed = mesh::parse_descriptors(&bytes, &path)?;
                    warnings.push(format!(
                        "Train descriptor {path}: closed {} open blocks at EOF as the game does",
                        open - close
                    ));
                    parsed
                }
            };
            descriptors.meshes.extend(parsed.meshes);
            descriptors.entities.extend(parsed.entities);
            descriptors.animations.extend(parsed.animations);
        }
        // Shipped european_train mesh paths use /gfx for content-root paths.
        for definition in descriptors.meshes.values_mut() {
            if definition.file.starts_with("/gfx/") {
                definition.file.remove(0);
            }
        }
        let library = ModelLibrary::load(
            layers,
            &descriptors,
            &wagons.keys().cloned().collect::<Vec<_>>(),
            warnings,
        )?;
        Ok(Self {
            library,
            wagons,
            lists,
        })
    }

    fn consist(&self, tag: &str, seed: u32) -> Vec<Wagon> {
        let kind = "train_equipment_1".to_owned();
        let Some(list) = self
            .lists
            .get(&(tag.to_owned(), kind.clone()))
            .or_else(|| self.lists.get(&("default".to_owned(), kind)))
        else {
            return Vec::new();
        };
        let mut rng = Random { seed, index: 0 };
        let count = 4 + rng.next() as usize % 4;
        let mut names = list.fixed.clone();
        let target = count.max(list.fixed.len() + usize::from(list.last.is_some()))
            + usize::from(list.last.is_some());
        if !list.random.is_empty() {
            while names.len() < target {
                names.push(list.random[rng.next() as usize % list.random.len()].clone());
            }
        }
        names.extend(list.last.clone());
        names
            .iter()
            .filter_map(|n| self.wagons.get(n).cloned())
            .collect()
    }
}

/// Integer presentation RNG in 141598790/141c4f3c0, wrapping at every operation.
#[derive(Clone, Default)]
struct Random {
    seed: u32,
    index: u32,
}
impl Random {
    fn next(&mut self) -> u32 {
        let mut x = self.seed.wrapping_add(self.index.wrapping_mul(0xb529_7a4d));
        self.index = self.index.wrapping_add(1);
        x = (x ^ (x >> 8)).wrapping_add(0x68e3_1da4);
        x = (x ^ (x << 8)).wrapping_mul(0x1b56_c4e9);
        x = (x ^ (x >> 8)).wrapping_mul(0x92d6_8ca2);
        (x ^ (x >> 8)) & 0x7fff_ffff
    }
}

#[derive(Clone, Default)]
pub struct TrainPlan {
    routes: Vec<Route>,
    tag: String,
    services: Vec<Service>,
    next_departure: f64,
    last_time: Option<f32>,
    random: Random,
}
#[derive(Clone)]
struct Service {
    route: usize,
    departed: f64,
    cars: Vec<Wagon>,
}
#[derive(Clone)]
struct Route {
    points: Vec<[f32; 3]>,
    distance: Vec<f32>,
    seed: u32,
}
impl Route {
    fn at(&self, distance: f32) -> ([f32; 3], [f32; 3]) {
        let i = self
            .distance
            .partition_point(|d| *d < distance)
            .clamp(1, self.points.len() - 1);
        let a = self.points[i - 1];
        let b = self.points[i];
        let t = (distance - self.distance[i - 1])
            / (self.distance[i] - self.distance[i - 1]).max(0.0001);
        (
            std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t),
            std::array::from_fn(|k| b[k] - a[k]),
        )
    }
}

impl TrainPlan {
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    /// Dump diagnostics: production inputs/derived positions, never capture data.
    pub fn diagnostics(&self) -> Vec<String> {
        let time = self.last_time.unwrap_or(0.0);
        self.services.iter().map(|service| {
            let route = &self.routes[service.route];
            let distance = ((f64::from(time)-service.departed)*f64::from(SPEED)) as f32;
            let (head, _) = route.at(distance);
            format!("Train route seed {:08x}: {} cars; head {:.3},{:.3},{:.3}; distance {:.3}/{:.3}", route.seed, service.cars.len(), head[0], head[1], head[2], distance, route.distance.last().unwrap())
        }).collect()
    }

    /// UNVERIFIED substitute for the missing country-supply parent graph and
    /// spare-supply weights. One graph traversal per recorded hour, never per frame.
    pub fn build(
        frame: &DecodedFrame,
        ours: Option<u32>,
        anchors: &BTreeMap<u32, [f32; 3]>,
        heights: &HeightField<'_>,
    ) -> io::Result<Self> {
        let mut plan = Self::default();
        let Some(country) = core::<tables::Countries>(frame)?
            .iter()
            .find(|c| Some(c.canonical_slot) == ours)
        else {
            return Ok(plan);
        };
        plan.tag = String::from_utf8_lossy(&country.tag).into_owned();
        let supply = rows::<tables::ProvinceSupply>(frame)?;
        let edges = rows::<tables::RailEdges>(frame)?;
        let cooldown = rows::<tables::RailCooldowns>(frame)?;
        if !supply.is_complete() || !edges.is_complete() || !cooldown.is_complete() {
            return Ok(plan);
        }
        let closed: BTreeSet<_> = cooldown
            .rows()
            .iter()
            .filter(|r| r.cooldown_ticks > 0)
            .map(|r| r.province)
            .collect();
        let allowed: BTreeSet<_> = supply
            .rows()
            .iter()
            .filter(|r| {
                u32::from(r.controller_index) == country.canonical_slot
                    && !closed.contains(&r.province)
            })
            .map(|r| r.province)
            .collect();
        let capital_state = rows::<tables::Politics>(frame)?
            .rows()
            .iter()
            .find(|r| r.country == country.id)
            .map(|r| r.capital as u32);
        let provinces = core::<tables::Provinces>(frame)?;
        let hubs: BTreeSet<_> = supply
            .rows()
            .iter()
            .filter(|r| {
                allowed.contains(&r.province) && (r.supply_node_level > 0 || r.naval_base_level > 0)
            })
            .map(|r| r.province)
            .collect();
        let Some(start) = provinces
            .iter()
            .filter(|p| Some(p.state) == capital_state && hubs.contains(&p.id))
            .max_by_key(|p| (p.victory_points, Reverse(p.id)))
            .map(|p| p.id)
        else {
            return Ok(plan);
        };
        let mut graph: BTreeMap<u32, Vec<(u32, u64)>> = BTreeMap::new();
        for e in edges.rows() {
            if e.level <= 0 || !allowed.contains(&e.from) || !allowed.contains(&e.to) {
                continue;
            }
            let (Some(a), Some(b)) = (anchors.get(&e.from), anchors.get(&e.to)) else {
                continue;
            };
            let cost = (((a[0] - b[0]).hypot(a[2] - b[2]) * 1000.0) as u64).max(1);
            graph.entry(e.from).or_default().push((e.to, cost));
            graph.entry(e.to).or_default().push((e.from, cost));
        }
        let mut distances = BTreeMap::from([(start, 0)]);
        let mut parent = BTreeMap::new();
        let mut queue = BinaryHeap::from([Reverse((0u64, start))]);
        while let Some(Reverse((distance, province))) = queue.pop() {
            if distances.get(&province) != Some(&distance) {
                continue;
            }
            for &(next, cost) in graph.get(&province).into_iter().flatten() {
                let candidate = distance + cost;
                if distances.get(&next).is_none_or(|old| candidate < *old) {
                    distances.insert(next, candidate);
                    parent.insert(next, province);
                    queue.push(Reverse((candidate, next)));
                }
            }
        }
        for end in hubs {
            if end == start || !parent.contains_key(&end) {
                continue;
            }
            let mut route = vec![end];
            let mut p = end;
            while p != start {
                p = parent[&p];
                route.push(p);
            }
            route.reverse();
            let seed = route.iter().fold(SEED, |h, p| h.rotate_left(5) ^ p);
            let mut points = railways::spline(
                &route
                    .iter()
                    .filter_map(|p| anchors.get(p).copied())
                    .collect::<Vec<_>>(),
                railways::RAIL_STEP,
            );
            if points.len() < 2 {
                continue;
            }
            for p in &mut points {
                p[1] = heights.height_at_world(p[0], p[2]).max(9.5);
            }
            let mut distance = vec![0.0];
            for pair in points.windows(2) {
                distance.push(
                    distance.last().unwrap()
                        + (pair[0][0] - pair[1][0]).hypot(pair[0][2] - pair[1][2]),
                );
            }
            plan.routes.push(Route {
                points,
                distance,
                seed,
            });
        }
        Ok(plan)
    }

    pub fn field(&mut self, assets: &TrainAssets, time: f32) -> ModelField {
        if self.routes.is_empty() {
            return ModelField::default();
        }
        let time = time.max(0.0);
        if self.last_time.is_none_or(|old| time < old) {
            self.services.clear();
            self.random = Random {
                seed: self.routes.iter().fold(SEED, |s, r| s ^ r.seed),
                index: 0,
            };
            // Warm from an empty fleet farther back than any train's lifetime.
            // This is presentation history, not game state or an inventory estimate.
            let longest = self
                .routes
                .iter()
                .map(|r| *r.distance.last().unwrap())
                .fold(0.0, f32::max);
            let car_length = assets.wagons.values().map(|w| w.length).fold(0.0, f32::max);
            self.next_departure = -f64::from((longest + car_length * 12.0) / SPEED);
        }
        while self.next_departure <= f64::from(time) {
            // 141598790: one global timer; choose an eligible hub by weight.
            // Missing node spare supply currently gives each hub weight one.
            let route = self.random.next() as usize % self.routes.len();
            let cars = assets.consist(&self.tag, self.routes[route].seed ^ self.random.next());
            let duration = (*self.routes[route].distance.last().unwrap()
                + cars.iter().map(|w| w.length).sum::<f32>())
                / SPEED;
            if self.next_departure + f64::from(duration) >= f64::from(time) {
                self.services.push(Service {
                    route,
                    departed: self.next_departure,
                    cars,
                });
            }
            self.next_departure += f64::from(9200 + self.random.next() % 2801) / 1000.0;
        }
        self.last_time = Some(time);
        self.services.retain(|service| {
            let length = *self.routes[service.route].distance.last().unwrap()
                + service.cars.iter().map(|w| w.length).sum::<f32>();
            (f64::from(time) - service.departed) * f64::from(SPEED) <= f64::from(length)
        });
        let mut placed = Vec::new();
        for service in &self.services {
            let route = &self.routes[service.route];
            let length = *route.distance.last().unwrap();
            let head = ((f64::from(time) - service.departed) * f64::from(SPEED)) as f32;
            let cars = &service.cars;
            let mut offset = 0.0;
            for car in cars {
                let front = head - offset;
                offset += car.length;
                let mut back = head - offset;
                let (front_position, _) = route.at(front);
                // 141c4e350: solve the link's straight chord, at most ten steps.
                for _ in 0..10 {
                    let (b, _) = route.at(back);
                    let error = ((front_position[0] - b[0]).powi(2)
                        + (front_position[1] - b[1]).powi(2)
                        + (front_position[2] - b[2]).powi(2))
                    .sqrt()
                        - car.length;
                    if error.abs() < 0.001 {
                        break;
                    }
                    back += error;
                }
                offset = head - back;
                if front < 0.0 || back > length {
                    continue;
                }
                let (a, _) = route.at(front);
                let (b, tangent) = route.at(back);
                let dx = a[0] - b[0];
                let dz = a[2] - b[2];
                let (dx, dz) = if dx.hypot(dz) < 0.0001 {
                    (tangent[0], tangent[2])
                } else {
                    (dx, dz)
                };
                if let Some(mesh) = assets.library.slot(&car.entity) {
                    placed.push((
                        mesh,
                        ModelInstance {
                            position: [
                                b[0] + car.shift[0],
                                b[1] + car.shift[1],
                                b[2] + car.shift[2],
                            ],
                            rotation: -(dz.atan2(dx) - std::f32::consts::FRAC_PI_2 + car.rotation)
                                + std::f32::consts::FRAC_PI_2,
                            scale: car.scale,
                            pitch: (a[1] - b[1]).atan2(dx.hypot(dz)),
                            roll: 0.0,
                            train_clip: if back < 2.5 {
                                let (p, t) = route.at(0.0);
                                let n = t[0].hypot(t[2]).max(0.0001);
                                [p[0], p[2], t[0] / n, t[2] / n]
                            } else if front > length - 2.5 {
                                let (p, t) = route.at(length);
                                let n = t[0].hypot(t[2]).max(0.0001);
                                [p[0], p[2], -t[0] / n, -t[2] / n]
                            } else {
                                [0.0; 4]
                            },
                        },
                    ));
                }
            }
        }
        ModelField::build(&assets.library, placed, 128.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> TrainAssets {
        let wagon = Wagon {
            entity: "car".into(),
            scale: 1.0,
            rotation: 0.0,
            shift: [0.0; 3],
            length: 2.0,
        };
        TrainAssets {
            library: ModelLibrary::default(),
            wagons: BTreeMap::from([("car".into(), wagon)]),
            lists: BTreeMap::from([(
                ("default".into(), "train_equipment_1".into()),
                Consist {
                    fixed: vec!["car".into(), "car".into()],
                    random: vec!["car".into()],
                    last: Some("car".into()),
                },
            )]),
        }
    }

    fn plan() -> TrainPlan {
        TrainPlan {
            routes: vec![Route {
                points: vec![[0.0, 10.0, 0.0], [600.0, 10.0, 0.0]],
                distance: vec![0.0, 600.0],
                seed: SEED,
            }],
            ..TrainPlan::default()
        }
    }

    #[test]
    fn last_wagon_is_appended_after_random_fill() {
        let assets = assets();
        let counts: BTreeSet<_> = (0..100)
            .map(|seed| assets.consist("GER", seed).len())
            .collect();
        assert_eq!(counts, BTreeSet::from([6, 7, 8, 9]));
    }

    #[test]
    fn frame_partition_and_backward_seek_do_not_change_departures() {
        let assets = assets();
        let mut direct = plan();
        direct.field(&assets, 45.0);
        let departures = |p: &TrainPlan| {
            p.services
                .iter()
                .map(|s| (s.route, s.departed, s.cars.len()))
                .collect::<Vec<_>>()
        };
        let expected = departures(&direct);
        let mut stepped = plan();
        for time in 0..=450 {
            stepped.field(&assets, time as f32 / 10.0);
        }
        assert_eq!(departures(&stepped), expected);
        stepped.field(&assets, 0.0);
        stepped.field(&assets, 45.0);
        assert_eq!(departures(&stepped), expected);
        for pair in expected.windows(2) {
            assert!((9.2 - 1e-8..=12.0 + 1e-8).contains(&(pair[1].1 - pair[0].1)));
        }
    }

    #[test]
    fn endpoint_extrapolation_keeps_cars_from_piling_up() {
        let plan = plan();
        let route = &plan.routes[0];
        assert_eq!(route.at(-2.0).0, [-2.0, 10.0, 0.0]);
        assert!((route.at(603.0).0[0] - 603.0).abs() < 0.001);
        assert_eq!(route.at(3.0 * SPEED).0, [9.0, 10.0, 0.0]);
    }
}
