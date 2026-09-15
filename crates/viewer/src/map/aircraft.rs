//! Flying aircraft, a729 `gfxairplanes.cpp` (not the three parked aircraft).
//!
//! `14122efd0` visits strategic regions every two wall seconds. `14122eb00`
//! partitions eligible missions (fighter mask 0x85, bomber 0x1f7fa, transport
//! bit 10, scout bit 14). The wing's mission-effective count, divided by 30
//! and floored to one when positive, selects single/triple scenarios. The
//! regional limit (two patrol/bombing/transport/scout, three dogfight) is
//! checked by exact scenario kind AND outcome BEFORE a wing's batch, so a
//! batch can exceed it. It is not a fixed number of models per wing.
//!
//! `140f4b590 -> 140ffac80` takes the largest WingEquipment amount (first on
//! ties). `141236ed0` tries the designer model and variant entity, then
//! `141236950/141237140`: graphical equipment override, cosmetic tag, country
//! tag, bare; each prefix tries equipment name, archetype name, sprite, with
//! `_entity`. No graphical-culture or invented fighter-role fallback.
//!
//! `141239810` picks a province from the mission region, not its centre.
//! `141238530` anchors the scenario at that province's integer map position
//! and sampled terrain height. `141235260` faces it relative to the base,
//! perturbed by a signed 10..40 degrees (the random variate is sqrt-shaped).
//! `14122cb70` reads formation positions, orientation, duration and initial
//! speed from the scenario rig's animated bones; `14123b7a0` follows those
//! bones, not a synthetic circle. `1412363c0/14117b800` join the base to the
//! animation with a five-control-point nested interpolation, sampled at
//! distance/2 + 3 points, handle length min(.9*distance,20), and raise the
//! interior samples to terrain+4. `141235ee0` advances by polyline arc length.
//! Takeoff/landing shrink to .1 in the first/last tenth; the group synchronises
//! arrivals. Constants are the shipped NAirGfx defaults (division factors 30,
//! formation thresholds 1/3, scenario limits 2/3); modded Lua overrides are
//! UNVERIFIED. `14123a670/141238090` advance a separate presentation clock even
//! paused: .22 seconds of animation per wall second (.28.. .50 at game speeds).
//!
//! # Generated state, explicitly UNVERIFIED
//!
//! The replay does not record mission eligibility/current sorties, target
//! province, hostile air-intel estimates, designer/graphical/cosmetic overrides,
//! scenario roster/outcome or clock. We use positive recorded effective_planes,
//! recorded region/mask, largest recorded variant, and default scenario outcome.
//! Seed 0x41495250, XOR wing ID/region/scenario ordinal, supplies the engine's
//! integer mixer, province, orientation and phase. This is a stable per-hour
//! presentation, not recovered game state. Opposing fighter/bomber pairing,
//! current CAS target and bombing-strength outcome cannot be recovered; those
//! wings use their independent default mission scenarios. Training without a
//! mission region uses its base region. Missiles and carrier basing need other
//! inputs and are skipped. Exact eligibility, count and dogfight agreement
//! require a recorded scenario/mission-effective census, not a pixel refit.
//! Country admission starts from the viewing country's own war/volunteer
//! context, not from unit-vision sharing. At both reference hours Germany has
//! no such context and no assigned own missions: exactly zero flight slots.
//! The native pairK census confirms an empty German roster at heights 50..310,
//! with air map mode both off and on, after allowing simulation to advance.
//! Nonempty wartime/air-volunteer roster reconstruction remains UNVERIFIED:
//! the replay does not store the manager's native participant arrays.
//!
//! The airborne manager has no dedicated height define or gate in its update.
//! We retain the shared DRAW_MAP_OBJECTS_CUTOFF=1100. Its airborne-specific
//! camera/fade behaviour remains UNVERIFIED pending a draw-bearing height sweep.
//! The supplied frame33840 has ZERO airborne PDX draws (all conventional mesh
//! diffuse resources match buildings/bridges/trains/map frame), so it cannot
//! settle altitude/scale/formation. No captured transform is an input here.
//! Frame-rate smoothing, propeller animation, gun/bomb/death particles and
//! success/failure transitions are not reconstructed. Authored rig transforms
//! are sampled at their frame rate and interpolated; the route banking uses
//! its tangent, with the unrecorded smoothing history omitted.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::{
    ResolvedLayers,
    mesh::{self, Transform},
    parser,
};
use replay::{
    frame::DecodedFrame,
    rows::{AirWingRow, tables},
};

use super::{
    borders::HeightField,
    counters::CounterContent,
    models::{DRAW_MAP_OBJECTS_CUTOFF, ModelField, ModelInstance, ModelLibrary},
    orders::ProvinceGeometry,
    perspective::Vision,
    tables::{core, rows},
};

pub const SEED: u32 = 0x4149_5250;
pub const CLOCK_SPEED: f32 = 0.22;

pub fn draws(height: f32) -> bool {
    height < DRAW_MAP_OBJECTS_CUTOFF
}

#[derive(Clone, Default)]
struct Equipment {
    archetype: String,
    sprite: String,
    parent: String,
}

/// Authored track shared by every scenario of this kind, prepared once.
struct Rig {
    tracks: Vec<Vec<Transform>>,
    fps: f32,
    duration: f32,
}

pub struct AircraftAssets {
    pub library: ModelLibrary,
    equipment: BTreeMap<String, Equipment>,
    rigs: BTreeMap<String, Rig>,
}

fn scalar(items: &[parser::Item], key: &str) -> Option<String> {
    items
        .iter()
        .find(|i| i.key.as_deref() == Some(key.as_bytes()))?
        .value
        .scalar_str()
        .map(str::to_owned)
}

/// The exact suffix ladder under each available prefix. Missing recorded
/// override levels are reported, never substituted with a graphical culture.
fn candidates(tag: &str, name: &str, archetype: &str, sprite: &str) -> Vec<String> {
    let mut out = Vec::new();
    for prefix in [tag, ""] {
        for key in [name, archetype, sprite] {
            if key.is_empty() {
                continue;
            }
            let key = if prefix.is_empty() {
                format!("{key}_entity")
            } else {
                format!("{prefix}_{key}_entity")
            };
            if !out.contains(&key) {
                out.push(key);
            }
        }
    }
    out
}

// Several shipped aircraft descriptors have stray top-level braces. Keep
// strings/comments untouched; only repair structural braces on parser failure.
fn balanced_blocks(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let (mut depth, mut quoted, mut comment, mut escaped) = (0usize, false, false, false);
    for &b in bytes {
        if comment {
            out.push(b);
            if b == b'\n' {
                comment = false;
            }
            continue;
        }
        if quoted {
            out.push(b);
            if b == b'"' && !escaped {
                quoted = false;
            }
            escaped = b == b'\\' && !escaped;
            continue;
        }
        match b {
            b'#' => comment = true,
            b'"' => quoted = true,
            b'{' => depth += 1,
            b'}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
            }
            _ => {}
        }
        out.push(b);
    }
    out.push(b'\n');
    out.extend(std::iter::repeat_n(b'}', depth));
    out
}

impl AircraftAssets {
    pub fn load(layers: &ResolvedLayers, warnings: &mut Vec<String>) -> io::Result<Self> {
        let mut equipment: BTreeMap<String, Equipment> = BTreeMap::new();
        for path in layers.paths().filter(|p| {
            p.starts_with("common/units/equipment/")
                && p.ends_with(".txt")
                && !p[23..].contains('/')
        }) {
            let document = parser::parse(&layers.read(path)?)?;
            for block in &document.items {
                if block.key.as_deref() == Some(b"duplicate_archetypes") {
                    for item in block.value.items().unwrap_or_default() {
                        let (Some(key), Some(fields)) = (item.key.as_deref(), item.value.items())
                        else {
                            continue;
                        };
                        let name = String::from_utf8_lossy(key).into_owned();
                        let Some(base) = scalar(fields, "archetype") else {
                            continue;
                        };
                        let mut copies = Vec::new();
                        for (old, eq) in &equipment {
                            if old == &base || eq.archetype == base {
                                let mut copy = eq.clone();
                                let derived = old.replacen(&base, &name, 1);
                                if derived == *old {
                                    continue;
                                }
                                copy.archetype = if old == &base {
                                    String::new()
                                } else {
                                    name.clone()
                                };
                                copy.parent = old.clone();
                                if let Some(sprite) = scalar(fields, "sprite") {
                                    copy.sprite = sprite;
                                }
                                copies.push((derived, copy));
                            }
                        }
                        equipment.extend(copies);
                    }
                    continue;
                }
                if block.key.as_deref() != Some(b"equipments") {
                    continue;
                }
                for item in block.value.items().unwrap_or_default() {
                    let (Some(key), Some(fields)) = (item.key.as_deref(), item.value.items())
                    else {
                        continue;
                    };
                    let name = String::from_utf8_lossy(key).into_owned();
                    equipment.insert(
                        name,
                        Equipment {
                            archetype: scalar(fields, "archetype").unwrap_or_default(),
                            sprite: scalar(fields, "sprite").unwrap_or_default(),
                            parent: scalar(fields, "parent").unwrap_or_default(),
                        },
                    );
                }
            }
        }
        // Definition inheritance, bounded so malformed cycles terminate.
        for _ in 0..equipment.len() {
            let mut changes = Vec::new();
            for (name, eq) in &equipment {
                let parent = if eq.parent.is_empty() {
                    &eq.archetype
                } else {
                    &eq.parent
                };
                if let Some(base) = equipment.get(parent) {
                    let sprite = if eq.sprite.is_empty() {
                        base.sprite.clone()
                    } else {
                        eq.sprite.clone()
                    };
                    let archetype = if eq.archetype.is_empty() {
                        base.archetype.clone()
                    } else {
                        eq.archetype.clone()
                    };
                    if sprite != eq.sprite || archetype != eq.archetype {
                        changes.push((name.clone(), sprite, archetype));
                    }
                }
            }
            if changes.is_empty() {
                break;
            }
            for (name, sprite, archetype) in changes {
                let eq = equipment.get_mut(&name).unwrap();
                eq.sprite = sprite;
                eq.archetype = archetype;
            }
        }
        let mut descriptors = mesh::Descriptors::default();
        for path in DESCRIPTORS {
            let bytes = match layers.read_uncovered(path) {
                Ok(b) => b,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            let parsed = match mesh::parse_descriptors(&bytes, path) {
                Ok(p) => p,
                Err(_) => {
                    let fixed = balanced_blocks(&bytes);
                    warnings.push(format!("Aircraft descriptor {path}: tolerated unmatched top-level closing braces or open blocks at EOF, as shipped"));
                    mesh::parse_descriptors(&fixed, path)?
                }
            };
            descriptors.meshes.extend(parsed.meshes);
            descriptors.entities.extend(parsed.entities);
            descriptors.animations.extend(parsed.animations);
        }
        let mut rigs = BTreeMap::new();
        for family in ["fighter", "scout", "bomber", "transport"] {
            for count in [1, 3] {
                let missions: &[&str] = match family {
                    "bomber" => &["CAS", "bombing"],
                    "transport" => &["supplying"],
                    _ => &["patrol"],
                };
                for mission in missions {
                    let key = format!("{family}_{count}_{mission}");
                    let entity = format!("{key}_entity");
                    // Scenario states are explicitly "default", not the PDX
                    // implicit idle used by ordinary model entities.
                    if let Some(e) = descriptors.entities.get_mut(&entity) {
                        e.default_state = Some("default".into());
                    }
                    let Some(resolved) = descriptors.resolve_entity(&entity) else {
                        continue;
                    };
                    let Some(animation) = resolved.animation else {
                        continue;
                    };
                    let mesh = mesh::read(&layers.read_uncovered(&resolved.definition.file)?)?;
                    let mut anim = mesh::read_animation(&layers.read_uncovered(&animation.file)?)?;
                    // The shipped single CAS mesh calls joint 3 `fighter`,
                    // whereas its animation calls the same slot `fighter_1`.
                    // UNVERIFIED engine alias: preserve that animated slot;
                    // a live single-CAS attachment matrix would settle it.
                    if key == "bomber_1_CAS"
                        && let Some(joint) = anim.joints.get_mut(3)
                        && joint.name == "fighter_1"
                    {
                        joint.name = "fighter".into();
                    }
                    let bone_family = if family == "scout" || *mission == "CAS" {
                        "fighter"
                    } else if family == "transport" {
                        "bomber"
                    } else {
                        family
                    };
                    let bones: Vec<_> = (1..=count)
                        .map(|n| {
                            if count == 1 {
                                bone_family.to_owned()
                            } else {
                                format!("{bone_family}_{n}")
                            }
                        })
                        .collect();
                    let tracks: Vec<Vec<_>> = bones
                        .iter()
                        .map(|bone| {
                            (0..anim.samples)
                                .filter_map(|frame| mesh.pose(&anim, frame, bone))
                                .collect()
                        })
                        .collect();
                    if tracks
                        .iter()
                        .any(|t| t.len() != anim.samples as usize || t.len() < 2)
                    {
                        warnings.push(format!("Aircraft scenario {entity} lacks a complete animated attachment track; skipped"));
                        continue;
                    }
                    rigs.insert(
                        key,
                        Rig {
                            tracks,
                            fps: anim.fps,
                            duration: (anim.samples - 1) as f32 / anim.fps,
                        },
                    );
                }
            }
        }
        let names: Vec<_> = descriptors
            .entities
            .keys()
            .filter(|key| {
                equipment.iter().any(|(name, eq)| {
                    [name.as_str(), eq.archetype.as_str(), eq.sprite.as_str()]
                        .iter()
                        .any(|s| {
                            !s.is_empty()
                                && (key.as_str() == format!("{s}_entity")
                                    || key.ends_with(&format!("_{s}_entity")))
                        })
                })
            })
            .cloned()
            .collect();
        let library = ModelLibrary::load(layers, &descriptors, &names, warnings)?;
        Ok(Self {
            library,
            equipment,
            rigs,
        })
    }

    fn resolve(&self, tag: &str, name: &str) -> Option<usize> {
        let eq = self.equipment.get(name)?;
        candidates(tag, name, &eq.archetype, &eq.sprite)
            .iter()
            .find_map(|name| self.library.slot(name))
    }
}

/// The game's unsigned mixer, with a specified replacement seed.
struct Random {
    seed: u32,
    index: u32,
}
impl Random {
    fn new(seed: u32) -> Self {
        Self { seed, index: 1 }
    }
    fn next(&mut self) -> u32 {
        let mut n = self.seed.wrapping_add(self.index.wrapping_mul(0xb529_7a4d));
        self.index = self.index.wrapping_add(1);
        n = (n ^ (n >> 8)).wrapping_add(0x68e3_1da4);
        n = (n ^ (n << 8)).wrapping_mul(0x1b56_c4e9);
        n = (n ^ (n >> 8)).wrapping_mul(0x92d6_8ca2);
        (n ^ (n >> 8)) & 0x7fff_ffff
    }
    fn unit(&mut self) -> f32 {
        self.next() as f32 / 2147483648.0
    }
}

#[derive(Clone)]
struct Route {
    points: Vec<[f32; 3]>,
    arcs: Vec<f32>,
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn mul(a: [f32; 3], n: f32) -> [f32; 3] {
    a.map(|v| v * n)
}
fn norm(a: [f32; 3]) -> f32 {
    a.iter().map(|v| v * v).sum::<f32>().sqrt()
}
fn unit(a: [f32; 3]) -> [f32; 3] {
    mul(a, 1.0 / norm(a).max(0.00001))
}
fn lerp(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    add(a, mul(sub(b, a), t))
}
fn rotate(a: [f32; 3], yaw: f32) -> [f32; 3] {
    let (s, c) = yaw.sin_cos();
    [c * a[0] - s * a[2], a[1], s * a[0] + c * a[2]]
}

impl Route {
    /// Literal five-point nested interpolation in 14117b800 (not a cubic
    /// Bezier and not the land counter's Catmull-Rom spline).
    fn build(
        start: [f32; 3],
        end: [f32; 3],
        back: [f32; 3],
        takeoff: bool,
        heights: &HeightField<'_>,
    ) -> Self {
        let distance = norm(sub(end, start));
        let h = (distance * 0.9).min(20.0);
        let toward = unit(if takeoff {
            sub(end, start)
        } else {
            sub(start, end)
        });
        let cross = toward[2] * back[0] - back[2] * toward[0];
        let side = if cross > 0.0 { 1.0 } else { -1.0 };
        let right = [-back[2], 0.0, back[0]];
        let (p1, p2, p3) = if takeoff {
            let p3 = add(end, mul(back, h));
            let offset = add(mul(back, h), mul(right, h * side));
            (add(start, offset), add(p3, offset), p3)
        } else {
            let p1 = sub(start, mul(back, h));
            let offset = sub(mul(back, -h), mul(right, h * side));
            (p1, add(p1, offset), add(end, offset))
        };
        let count = (distance / 2.0) as usize + 3;
        let mut points = Vec::with_capacity(count);
        let mut arcs = Vec::with_capacity(count);
        for i in 0..count {
            let t = i as f32 / (count - 1) as f32;
            let mut p = lerp(start, lerp(p1, lerp(p2, lerp(p3, end, t), t), t), t);
            if i > 0 && i + 1 < count {
                p[1] = p[1].max(heights.height_at_world(p[0], p[2]) + 4.0);
            }
            arcs.push(points.last().map_or(0.0, |last| {
                arcs.last().copied().unwrap_or(0.0) + norm(sub(p, *last))
            }));
            points.push(p);
        }
        Self { points, arcs }
    }
    fn length(&self) -> f32 {
        *self.arcs.last().unwrap_or(&0.0)
    }
    fn at(&self, t: f32) -> Transform {
        let arc = t.clamp(0.0, 1.0) * self.length();
        let i = self
            .arcs
            .partition_point(|v| *v < arc)
            .max(1)
            .min(self.points.len() - 1);
        let t = (arc - self.arcs[i - 1]) / (self.arcs[i] - self.arcs[i - 1]).max(0.0001);
        let back = unit(sub(self.points[i - 1], self.points[i]));
        let right = unit([back[2], 0.0, -back[0]]);
        let up = [
            back[1] * right[2] - back[2] * right[1],
            back[2] * right[0] - back[0] * right[2],
            back[0] * right[1] - back[1] * right[0],
        ];
        Transform {
            translation: lerp(self.points[i - 1], self.points[i], t),
            basis: [right, up, back],
        }
    }
}

#[derive(Clone)]
struct Flight {
    mesh: usize,
    wing: u64,
    region: u32,
    rig: String,
    track: usize,
    origin: [f32; 3],
    yaw: f32,
    outgoing: Route,
    incoming: Route,
    travel: f32,
    delay: f32,
    group_travel: f32,
    phase: f32,
}

#[derive(Clone, Default)]
pub struct AircraftPlan {
    flights: Vec<Flight>,
    pub diagnostics: Vec<String>,
}

/// A mission category, following the mask priority at 14122eb00. Predicates
/// that require the unrecorded current target are conservatively named in the
/// registry; this function only classifies the recorded assignment.
fn family(mask: u32) -> Option<(&'static str, &'static str)> {
    if mask & (1 << 17) != 0 {
        return None;
    }
    if mask & (1 << 10) != 0 {
        return Some(("transport", "supplying"));
    }
    if mask & (1 << 14) != 0 {
        return Some(("scout", "patrol"));
    }
    if mask & 0x1f7fa != 0 && mask & 0x80 == 0 {
        return Some(("bomber", if mask & 0x202 != 0 { "CAS" } else { "bombing" }));
    }
    (mask & (0x85 | 0x800) != 0).then_some(("fighter", "patrol"))
}

/// A whole wing batch; the regional cap is checked before calling this.
fn formation_count(planes: i32) -> (usize, usize) {
    let n = (planes / 30).max(i32::from(planes > 0)) as usize;
    if n >= 3 { (3, n / 3) } else { (1, n) }
}

/// admission: a peaceful faction member must not inherit its partners' wars.
/// Native wartime allies and air-volunteer donor arrays are not recorded;
/// common enemies and VolunteerHosts reconstruct those connections. Their
/// exact equivalence outside the verified empty-roster frames is UNVERIFIED.
fn scenario_countries(ours: u32, wars: &[(u32, u32)], volunteers: &[(u32, u32)]) -> BTreeSet<u32> {
    let mut hosts = BTreeSet::from([ours]);
    hosts.extend(
        volunteers
            .iter()
            .filter_map(|&(sender, host)| (sender == ours).then_some(host)),
    );
    let enemies: BTreeSet<_> = wars
        .iter()
        .flat_map(|&(a, b)| [(a, b), (b, a)])
        .filter_map(|(a, b)| hosts.contains(&a).then_some(b))
        .collect();
    let mut friends = hosts;
    friends.extend(
        wars.iter()
            .flat_map(|&(a, b)| [(a, b), (b, a)])
            .filter_map(|(a, b)| enemies.contains(&b).then_some(a)),
    );
    let mut countries = friends.clone();
    countries.extend(enemies.iter().copied());
    countries.extend(volunteers.iter().filter_map(|&(sender, host)| {
        (friends.contains(&host) || enemies.contains(&host)).then_some(sender)
    }));
    countries
}

impl AircraftPlan {
    pub fn build(
        frame: &DecodedFrame,
        assets: &AircraftAssets,
        content: &CounterContent,
        geometry: &ProvinceGeometry,
        heights: &HeightField<'_>,
        vision: &Vision,
    ) -> io::Result<Self> {
        let mut out = Self::default();
        let links: BTreeMap<_, _> = core::<tables::CountryLinks>(frame)?
            .iter()
            .map(|r| (r.id, r.canonical_slot))
            .collect();
        let tags: BTreeMap<_, _> = core::<tables::Countries>(frame)?
            .iter()
            .map(|r| {
                (
                    r.canonical_slot,
                    String::from_utf8_lossy(&r.tag).into_owned(),
                )
            })
            .collect();
        let bases = rows::<tables::AirBases>(frame)?;
        let bases: BTreeMap<_, _> = bases.rows().iter().map(|r| (r.id, r)).collect();
        let variants = rows::<tables::VariantDefinitions>(frame)?;
        let variants: BTreeMap<_, _> = variants.rows().iter().map(|r| (r.id, r)).collect();
        let bank = rows::<tables::WingEquipment>(frame)?;
        let mut equipment: BTreeMap<u64, &replay::rows::WingEquipmentRow> = BTreeMap::new();
        for row in bank.rows() {
            if row.amount_raw > 0
                && equipment.get(&row.wing).is_none_or(|old| {
                    row.amount_raw > old.amount_raw
                        || (row.amount_raw == old.amount_raw && row.ordinal < old.ordinal)
                })
            {
                equipment.insert(row.wing, row);
            }
        }
        let mut regions: BTreeMap<u32, Vec<[f32; 3]>> = BTreeMap::new();
        for p in core::<tables::Provinces>(frame)? {
            if let Some(region) = content.region(p.id)
                && let Some([x, z]) = geometry.centroid(p.id)
            {
                // 141238530 reads province_definition +b8/+bc, the same
                // integer pixel means already prepared for order geometry.
                let anchor = [x, heights.height_at_world(x, z), z];
                regions.entry(region).or_default().push(anchor);
            }
        }
        let Some(ours) = vision.ours() else {
            out.diagnostics
                .push("aircraft require a viewing country (native manager +108)".into());
            return Ok(out);
        };
        let wars = rows::<tables::WarRelations>(frame)?;
        let wars: Vec<_> = wars
            .rows()
            .iter()
            .filter(|w| super::allegiance::ongoing(w.start_hour, w.end_hour, frame.hour))
            .filter_map(|w| {
                Some((
                    *links.get(&w.first_country)?,
                    *links.get(&w.second_country)?,
                ))
            })
            .collect();
        let volunteers = rows::<tables::VolunteerHosts>(frame)?;
        let volunteers: Vec<_> = volunteers
            .rows()
            .iter()
            .filter_map(|v| Some((*links.get(&v.country)?, *links.get(&v.host)?)))
            .collect();
        let countries = scenario_countries(ours, &wars, &volunteers);
        let wings = rows::<tables::AirWings>(frame)?;
        let mut ordered: Vec<&AirWingRow> = wings.rows().iter().collect();
        // Stable replacement for the manager's transient random wing order.
        ordered.sort_by_key(|w| Random::new(SEED ^ w.id as u32 ^ (w.id >> 32) as u32).next());
        let mut counts: BTreeMap<(u32, String), usize> = BTreeMap::new();
        for wing in ordered {
            if wing.effective_planes <= 0 {
                continue;
            }
            let Some((family, mission)) = family(wing.mission_mask) else {
                continue;
            };
            let owner = if wing.country_override > 0 {
                wing.country_override as u32
            } else {
                wing.owner
            };
            let Some(&slot) = links.get(&owner) else {
                continue;
            };
            if !countries.contains(&slot) {
                continue;
            }
            // 14122efd0 only scans our training fallback and passes no scout
            // destination list for allied/enemy countries. 140f66100 rejects
            // a resolved base override (a transfer), rather than flying from it.
            if (slot != ours && wing.mission_mask & (0x800 | 0x4000) != 0)
                || wing.base_override != 0
            {
                continue;
            }
            let base = wing.air_base;
            let Some(base) = bases.get(&base) else {
                out.diagnostics
                    .push(format!("wing {} missing base", wing.id));
                continue;
            };
            let Some(start) = base.state.and_then(|state| content.air_base(state)) else {
                out.diagnostics.push(format!(
                    "wing {} has no authored land-base slot (carrier/missing base)",
                    wing.id
                ));
                continue;
            };
            let region = wing.mission_region.or_else(|| {
                if wing.mission_mask & 0x800 != 0 {
                    base.province.and_then(|p| content.region(p))
                } else {
                    None
                }
            });
            let Some(region) = region else { continue };
            let Some(provinces) = regions.get(&region).filter(|p| !p.is_empty()) else {
                continue;
            };
            let Some(variant) = equipment
                .get(&wing.id)
                .and_then(|e| variants.get(&e.variant))
            else {
                out.diagnostics
                    .push(format!("wing {} missing equipment identity", wing.id));
                continue;
            };
            let name = String::from_utf8_lossy(&variant.equipment_name);
            let Some(mesh) = assets.resolve(tags.get(&slot).map_or("", String::as_str), &name)
            else {
                out.diagnostics.push(format!(
                    "wing {} unresolved aircraft equipment {name}",
                    wing.id
                ));
                continue;
            };
            let (formation, batches) = formation_count(wing.effective_planes.min(wing.planes));
            let key = format!("{family}_{formation}_{mission}");
            let Some(rig) = assets.rigs.get(&key) else {
                out.diagnostics.push(format!("missing scenario {key}"));
                continue;
            };
            let count = counts.entry((region, key.clone())).or_default();
            if *count >= 2 {
                continue;
            }
            *count += batches;
            for ordinal in 0..batches {
                let mut rng = Random::new(
                    SEED ^ wing.id as u32
                        ^ (wing.id >> 32) as u32
                        ^ region.rotate_left(16)
                        ^ ordinal as u32,
                );
                let origin = provinces[rng.next() as usize % provinces.len()];
                let direction = sub(origin, start);
                let yaw = -direction[0].atan2(direction[2])
                    + (10.0 + 30.0 * rng.unit().sqrt()).to_radians()
                        * if rng.next() & 1 == 0 { -1.0 } else { 1.0 };
                let phase = rng.unit();
                let mut group = Vec::new();
                let mut group_travel = 0.0f32;
                for (track, poses) in rig.tracks.iter().enumerate() {
                    let first = poses[0];
                    let last = *poses.last().unwrap();
                    let begin = add(origin, rotate(first.translation, yaw));
                    let end = add(origin, rotate(last.translation, yaw));
                    let outgoing =
                        Route::build(start, begin, rotate(first.basis[2], yaw), true, heights);
                    let incoming =
                        Route::build(end, start, rotate(last.basis[2], yaw), false, heights);
                    let frame = 0.1 * rig.fps;
                    let tenth = lerp(
                        poses[(frame as usize).min(poses.len() - 1)].translation,
                        poses[(frame as usize + 1).min(poses.len() - 1)].translation,
                        frame.fract(),
                    );
                    let speed = norm(sub(tenth, first.translation)) / 0.1;
                    let travel = if speed > 0.0 {
                        outgoing.length() / speed
                    } else {
                        1.0
                    };
                    group_travel = group_travel.max(travel);
                    group.push(Flight {
                        mesh,
                        wing: wing.id,
                        region,
                        rig: key.clone(),
                        track,
                        origin,
                        yaw,
                        outgoing,
                        incoming,
                        travel,
                        delay: 0.0,
                        group_travel: 0.0,
                        phase,
                    });
                }
                for f in &mut group {
                    f.delay = group_travel - f.travel;
                    f.group_travel = group_travel;
                }
                out.flights.extend(group);
            }
        }
        Ok(out)
    }

    fn placed(&self, assets: &AircraftAssets, seconds: f32) -> Vec<(usize, ModelInstance)> {
        self.flights
            .iter()
            .enumerate()
            .filter_map(|(index, flight)| {
                let rig = &assets.rigs[&flight.rig];
                let total = 2.0 * flight.group_travel + rig.duration;
                let time =
                    (seconds.max(0.0) * CLOCK_SPEED + flight.phase * total).rem_euclid(total);
                let (pose, scale) = if time < flight.group_travel {
                    if time < flight.delay {
                        return None;
                    }
                    let t = (time - flight.delay) / flight.travel;
                    (flight.outgoing.at(t), (t / 0.1).clamp(0.1, 1.0))
                } else if time < flight.group_travel + rig.duration {
                    let frame = (time - flight.group_travel) * rig.fps;
                    let poses = &rig.tracks[flight.track];
                    let a = &poses[(frame as usize).min(poses.len() - 1)];
                    let b = &poses[(frame as usize + 1).min(poses.len() - 1)];
                    let t = frame.fract();
                    (
                        Transform {
                            translation: add(
                                flight.origin,
                                rotate(lerp(a.translation, b.translation, t), flight.yaw),
                            ),
                            basis: std::array::from_fn(|i| {
                                rotate(unit(lerp(a.basis[i], b.basis[i], t)), flight.yaw)
                            }),
                        },
                        1.0,
                    )
                } else {
                    let t = (time - flight.group_travel - rig.duration) / flight.travel;
                    if t >= 1.0 {
                        return None;
                    }
                    (flight.incoming.at(t), ((1.0 - t) / 0.1).clamp(0.1, 1.0))
                };
                let pitch = (-pose.basis[2][1]).clamp(-1.0, 1.0).asin();
                let rotation = (-pose.basis[2][0]).atan2(pose.basis[2][2]);
                let roll = pose.basis[0][1].atan2(pose.basis[1][1]);
                Some((
                    index,
                    ModelInstance {
                        position: pose.translation,
                        rotation,
                        pitch,
                        roll,
                        scale,
                        train_clip: [0.0; 4],
                    },
                ))
            })
            .collect()
    }

    pub fn field(&self, assets: &AircraftAssets, seconds: f32) -> ModelField {
        ModelField::build(
            &assets.library,
            self.placed(assets, seconds)
                .into_iter()
                .map(|(index, instance)| (self.flights[index].mesh, instance))
                .collect(),
            128.0,
        )
    }

    pub fn report(&self, assets: &AircraftAssets) -> Vec<String> {
        let mut lines = vec![format!(
            "Aircraft: {} generated flight slots, {} at time zero, seed 0x{SEED:08x}; UNVERIFIED eligibility/scenario census",
            self.flights.len(),
            self.placed(assets, 0.0).len()
        )];
        let mut wings: BTreeMap<(u64, u32, &str, usize), usize> = BTreeMap::new();
        for f in &self.flights {
            *wings.entry((f.wing, f.region, &f.rig, f.mesh)).or_default() += 1;
        }
        for ((wing, region, rig, mesh), count) in wings {
            lines.push(format!(
                "Aircraft wing {wing}, region {region}: {count} slots, {rig}, {}, asset scale {}",
                assets.library.meshes[mesh].name, assets.library.meshes[mesh].scale
            ));
        }
        for (index, i) in self.placed(assets, 0.0) {
            let flight = &self.flights[index];
            lines.push(format!(
                "Aircraft t=0 wing {} track {} {} at {:.4},{:.4},{:.4} instance scale {:.4}",
                flight.wing,
                flight.track,
                assets.library.meshes[flight.mesh].name,
                i.position[0],
                i.position[1],
                i.position[2],
                i.scale
            ));
        }
        for key in self
            .flights
            .iter()
            .map(|f| &f.rig)
            .collect::<std::collections::BTreeSet<_>>()
        {
            let rig = &assets.rigs[key];
            let mut altitude = (f32::INFINITY, f32::NEG_INFINITY);
            let mut spacing = (f32::INFINITY, f32::NEG_INFINITY);
            let mut speed = (f32::INFINITY, f32::NEG_INFINITY);
            for (i, track) in rig.tracks.iter().enumerate() {
                for (frame, pose) in track.iter().enumerate() {
                    altitude.0 = altitude.0.min(pose.translation[1]);
                    altitude.1 = altitude.1.max(pose.translation[1]);
                    for other in &rig.tracks[i + 1..] {
                        let distance = norm(sub(pose.translation, other[frame].translation));
                        spacing.0 = spacing.0.min(distance);
                        spacing.1 = spacing.1.max(distance);
                    }
                    if let Some(next) = track.get(frame + 1) {
                        let v = norm(sub(next.translation, pose.translation)) * rig.fps;
                        speed.0 = speed.0.min(v);
                        speed.1 = speed.1.max(v);
                    }
                }
            }
            lines.push(format!(
                "Aircraft rig {key}: duration {:.4}s, authored height {:.4}..{:.4} above anchor, pair spacing {}, speed {:.4}..{:.4} units/animation-second",
                rig.duration, altitude.0, altitude.1,
                if spacing.0.is_finite() { format!("{:.4}..{:.4}", spacing.0, spacing.1) } else { "n/a (single)".into() },
                speed.0, speed.1
            ));
        }
        lines.extend(self.diagnostics.iter().cloned());
        lines
    }
}

const DESCRIPTORS: &[&str] = &[
    "gfx/entities/AAT_meshes_planes.gfx",
    "gfx/entities/AAT_units_planes.asset",
    "gfx/entities/BBA_meshes_planes.gfx",
    "gfx/entities/BBA_units_planes.asset",
    "gfx/entities/EP_meshes_seaplane_tenders_pack.gfx",
    "gfx/entities/EP_units_seaplane_tenders_pack.asset",
    "gfx/entities/GoE_meshes_planes.gfx",
    "gfx/entities/GoE_units_planes.asset",
    "gfx/entities/MTG_planes.asset",
    "gfx/entities/MTG_planes.gfx",
    "gfx/entities/SEA_meshes_planes.gfx",
    "gfx/entities/SEA_units_planes.asset",
    "gfx/entities/TOA_meshes_planes.gfx",
    "gfx/entities/TOA_units_planes.asset",
    "gfx/entities/TaoG_meshes_planes.gfx",
    "gfx/entities/TaoG_units_planes.asset",
    "gfx/entities/WW_meshes_planes.gfx",
    "gfx/entities/WW_units_planes.asset",
    "gfx/entities/_BfB_meshes_planes.gfx",
    "gfx/entities/_BfB_units_planes.asset",
    "gfx/entities/_LaR_meshes_planes.gfx",
    "gfx/entities/_LaR_units_planes.asset",
    "gfx/entities/dod_planes.gfx",
    "gfx/entities/dod_units_planes.asset",
    "gfx/entities/planes.gfx",
    "gfx/entities/pro_meshes_planes.gfx",
    "gfx/entities/pro_units_planes.asset",
    "gfx/entities/tfv_planes.gfx",
    "gfx/entities/tfv_units_planes.asset",
    "gfx/entities/units_planes.asset",
    "gfx/entities/wtt_planes.gfx",
    "gfx/entities/wtt_units_planes.asset",
    "gfx/models/units/planes/BBA_animation_planes.asset",
    "gfx/models/units/planes/animation_planes.asset",
    "gfx/models/units/planes/pro_animation_planes.asset",
    "gfx/models/units/planes/tfv_animation_planes.asset",
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unrelated_wars_do_not_admit_aircraft_to_a_peaceful_viewer() {
        let wars = [(2, 3), (4, 3), (5, 6)];
        assert_eq!(scenario_countries(1, &wars, &[]), BTreeSet::from([1]));
        assert_eq!(scenario_countries(2, &wars, &[]), BTreeSet::from([2, 3, 4]));
        assert_eq!(
            scenario_countries(1, &wars, &[(1, 2)]),
            BTreeSet::from([1, 2, 3, 4])
        );
        assert!(!scenario_countries(2, &wars, &[(7, 5)]).contains(&7));
    }
    #[test]
    fn country_keys_precede_bare_equipment_and_do_not_add_a_culture() {
        assert_eq!(
            candidates("GER", "small_airframe_2", "small_airframe", "light_plane"),
            [
                "GER_small_airframe_2_entity",
                "GER_small_airframe_entity",
                "GER_light_plane_entity",
                "small_airframe_2_entity",
                "small_airframe_entity",
                "light_plane_entity"
            ]
        );
    }
    #[test]
    fn wing_batch_discards_remainder_after_choosing_triples() {
        assert_eq!(formation_count(0), (1, 0));
        assert_eq!(formation_count(29), (1, 1));
        assert_eq!(formation_count(60), (1, 2));
        assert_eq!(formation_count(100), (3, 1));
        assert_eq!(formation_count(400), (3, 4));
    }
    #[test]
    fn supported_missions_follow_priority_and_training_is_separate() {
        assert_eq!(family(4), Some(("fighter", "patrol")));
        assert_eq!(family(2 | 8), Some(("bomber", "CAS")));
        assert_eq!(family(1 << 14), Some(("scout", "patrol")));
        assert_eq!(family(1 << 10), Some(("transport", "supplying")));
        assert_eq!(family(0), None);
    }
}
